//! `rookey listen` on macOS: the same hold to talk as on Linux and Windows, from a keyboard
//! tap, which also keeps the hotkey from the app in front (Linux does that with a bind that
//! does nothing).
//!
//! Runs from Rookey (see `mac::app`) as a launchd agent of the user, so it starts at login and
//! macOS asks for the microphone and Accessibility (the tap and the typing) for Rookey, not
//! for the terminal. The recordings it starts are its children and share those answers. The
//! page installs, restarts and removes it.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use crate::{Res, t};
use crate::desktop::Chord;
use crate::hold::{Hold, recording, toggle};
use crate::mac;

const LABEL: &str = "io.github.7kill.rookey.listen";

/// macOS virtual key codes (kVK_*) by the names compositors give the keys (XKB), which is what
/// `ROOKEY_HOTKEY` holds on every system. They follow the keyboard's layout, not the alphabet,
/// so every key is listed.
const NAMES: [(&str, u16); 97] = [
    ("A", 0x00),
    ("S", 0x01),
    ("D", 0x02),
    ("F", 0x03),
    ("H", 0x04),
    ("G", 0x05),
    ("Z", 0x06),
    ("X", 0x07),
    ("C", 0x08),
    ("V", 0x09),
    ("B", 0x0B),
    ("Q", 0x0C),
    ("W", 0x0D),
    ("E", 0x0E),
    ("R", 0x0F),
    ("Y", 0x10),
    ("T", 0x11),
    ("1", 0x12),
    ("2", 0x13),
    ("3", 0x14),
    ("4", 0x15),
    ("6", 0x16),
    ("5", 0x17),
    ("equal", 0x18),
    ("9", 0x19),
    ("7", 0x1A),
    ("minus", 0x1B),
    ("8", 0x1C),
    ("0", 0x1D),
    ("bracketright", 0x1E),
    ("O", 0x1F),
    ("U", 0x20),
    ("bracketleft", 0x21),
    ("I", 0x22),
    ("P", 0x23),
    ("Return", 0x24),
    ("L", 0x25),
    ("J", 0x26),
    ("apostrophe", 0x27),
    ("K", 0x28),
    ("semicolon", 0x29),
    ("backslash", 0x2A),
    ("comma", 0x2B),
    ("slash", 0x2C),
    ("N", 0x2D),
    ("M", 0x2E),
    ("period", 0x2F),
    ("Tab", 0x30),
    ("space", 0x31),
    ("grave", 0x32),
    ("BackSpace", 0x33),
    ("Escape", 0x35),
    // Command is Super, Option is Alt
    ("Super_R", 0x36),
    ("Super_L", 0x37),
    ("Shift_L", 0x38),
    ("Caps_Lock", 0x39),
    ("Alt_L", 0x3A),
    ("Control_L", 0x3B),
    ("Shift_R", 0x3C),
    ("Alt_R", 0x3D),
    ("Control_R", 0x3E),
    ("F17", 0x40),
    ("F18", 0x4F),
    ("F19", 0x50),
    ("KP_0", 0x52),
    ("KP_1", 0x53),
    ("KP_2", 0x54),
    ("KP_3", 0x55),
    ("KP_4", 0x56),
    ("KP_5", 0x57),
    ("KP_6", 0x58),
    ("KP_7", 0x59),
    ("F20", 0x5A),
    ("KP_8", 0x5B),
    ("KP_9", 0x5C),
    ("F5", 0x60),
    ("F6", 0x61),
    ("F7", 0x62),
    ("F3", 0x63),
    ("F8", 0x64),
    ("F9", 0x65),
    ("F11", 0x67),
    ("F13", 0x69),
    ("F16", 0x6A),
    ("F14", 0x6B),
    ("F10", 0x6D),
    ("F12", 0x6F),
    ("F15", 0x71),
    ("Home", 0x73),
    ("Page_Up", 0x74),
    ("F4", 0x76),
    ("End", 0x77),
    ("F2", 0x78),
    ("Page_Down", 0x79),
    ("F1", 0x7A),
    ("Left", 0x7B),
    ("Right", 0x7C),
];

const ESCAPE: u16 = 0x35;

pub fn key_code(name: &str) -> Option<u16> {
    let exact = NAMES.iter().find(|(xkb, _)| *xkb == name);
    // letters are named in capitals, but "d" is D too
    let letter = || NAMES.iter().find(|(xkb, _)| xkb.len() == 1 && xkb.eq_ignore_ascii_case(name));
    exact.or_else(letter).map(|&(_, code)| code)
}

fn key_name(code: u16) -> Option<String> {
    NAMES.iter().find(|(_, c)| *c == code).map(|(xkb, _)| xkb.to_string())
}

fn modifier(code: u16) -> Option<&'static str> {
    Some(match code {
        0x36 | 0x37 => "Super",
        0x3B | 0x3E => "Ctrl",
        0x3A | 0x3D => "Alt",
        0x38 | 0x3C => "Shift",
        _ => return None,
    })
}

pub fn is_modifier(name: &str) -> bool {
    key_code(name).and_then(modifier).is_some()
}

/// Rookey asks for the keys itself as it starts; the page shows whether it may.
pub fn access() -> Option<String> {
    None
}

pub fn run() -> Res<()> {
    let chord = crate::setting("ROOKEY_HOTKEY").ok_or_else(|| t!("listen.no-hotkey"))?;
    let chord = Chord::parse(&chord)?;
    let key = key_code(chord.key()).ok_or_else(|| t!("server.key-unknown", key = chord.key()))?;
    // the tap needs Accessibility: ask, then wait for the switch rather than fail and restart
    if !mac::typing() {
        let who = if mac::is_app() { "Rookey".to_string() } else { t!("listen.this-terminal") };
        eprintln!("{}", t!("listen.mac-allow", who = who));
        mac::ask_typing();
        while !fresh_typing() {
            thread::sleep(Duration::from_secs(2));
        }
    }
    eprintln!("{}", t!("listen.started", chord = chord));
    // ponytail: no background updates here: this binary is Rookey's copy, and an update belongs
    // next to the rookey it was copied from, which puts a new copy here as it installs.

    let wanted: Vec<&str> = chord.mods().to_vec();
    let lone = modifier(key).is_some();
    let mut hold = Hold::new(chord, key, modifier);
    let mut held: Vec<u16> = Vec::new();
    // the key's own press, kept from the app in front up to its release (repeats too)
    let mut ours = false;
    mac::tap(move |k| {
        match k.value {
            1 => held.push(k.code),
            0 => held.retain(|&c| c != k.code),
            _ => {}
        }
        if k.code != key && modifier(k.code).is_none() {
            return false;
        }
        if hold.event(k.code, k.value, Instant::now(), recording) {
            toggle();
        }
        // a modifier on its own types nothing, so nothing to keep from the windows
        if lone || k.code != key {
            return false;
        }
        if k.value == 1 {
            let mut mods: Vec<&str> = held.iter().filter(|&&c| c != key).filter_map(|&c| modifier(c)).collect();
            mods.sort_by_key(|m| crate::desktop::MODS.iter().position(|x| x == m));
            mods.dedup();
            ours = mods == wanted;
        }
        let keep = ours;
        if k.value == 0 {
            ours = false;
        }
        keep
    })
}

/// Accessibility as a new process sees it: this one keeps the answer it got first.
fn fresh_typing() -> bool {
    let Ok(exe) = crate::exe() else { return false };
    Command::new(exe).arg("__access").output().is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(r#""typing":true"#))
}

/// Waits for keys to be pressed, like on Linux and Windows: a modifier alone counts as it goes
/// up with nothing pressed in between. None for Esc or once `wait` is over. Always in a new
/// process that is Rookey's: the page's server keeps the Accessibility answer it got first,
/// and asked forever after it was allowed.
pub fn capture(wait: Duration) -> Res<Option<String>> {
    let seconds = wait.as_secs().to_string();
    let printed = if mac::is_app() {
        let out = Command::new(crate::exe()?).args(["__capture", &seconds]).output()?;
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        // the page runs in the terminal (ROOKEY_IN_TERMINAL): Rookey is asked all the same
        mac::as_app(&["__capture", &seconds], Some(wait + Duration::from_secs(5)))?
    };
    let answer: serde_json::Value = serde_json::from_str(printed.trim()).map_err(|_| t!("listen.mac-no-answer"))?;
    if let Some(why) = answer["error"].as_str() {
        return Err(why.into());
    }
    Ok(answer["captured"].as_str().map(str::to_string))
}

/// `capture`, in the process that listens.
fn capture_now(wait: Duration) -> Res<Option<String>> {
    if !mac::typing() {
        mac::ask_typing();
        return Err(t!("listen.mac-no-keys").into());
    }
    let (tx, rx) = mpsc::channel();
    // the tap runs on a thread of its own until this process ends, which is right after
    thread::spawn(move || {
        let _ = mac::tap(move |k| {
            let _ = tx.send((k.code, k.value));
            false
        });
    });

    let until = Instant::now() + wait;
    let mut held = Vec::new();
    let mut lone = None;
    while let Ok((key, value)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        if value == 0 {
            held.retain(|&k| k != key);
            if lone == Some(key) {
                return Ok(key_name(key));
            }
            continue;
        }
        if value == 2 {
            continue;
        }
        held.push(key);
        if key == ESCAPE {
            return Ok(None);
        }
        if modifier(key).is_some() {
            lone = Some(key);
            continue;
        }
        lone = None;
        let Some(name) = key_name(key) else { continue };
        let mut mods: Vec<&str> = held.iter().filter_map(|&k| modifier(k)).collect();
        mods.sort_by_key(|m| crate::desktop::MODS.iter().position(|x| x == m));
        mods.dedup();
        return Ok(Some(mods.into_iter().chain([name.as_str()]).collect::<Vec<_>>().join("+")));
    }
    Ok(None)
}

/// `rookey __capture <seconds>`, run as Rookey: the keys as JSON on stdout for `capture`.
pub fn capture_here(args: &[String]) -> Res<()> {
    let wait = Duration::from_secs(args.first().and_then(|s| s.parse().ok()).unwrap_or(10));
    let answer = match capture_now(wait) {
        Ok(keys) => serde_json::json!({ "captured": keys }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    mac::answer(&answer.to_string())
}

fn plist_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
}

fn domain() -> String {
    format!("gui/{}", mac::uid())
}

fn launchctl(args: &[&str]) -> Res<String> {
    let out = Command::new("launchctl").args(args).output()?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(t!("listen.tool-failed", tool = format!("launchctl {}", args[0]), why = why.trim()).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Text for a plist: the five characters XML won't take as they are.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The launchd agent: Rookey's binary with `listen`, started at login and again if it fails.
/// launchd hands its programs a bare PATH, so this one gets the PATH of whoever installs it:
/// tesseract (screen terms) is usually in Homebrew's folder.
fn agent(exe: &std::path::Path, path: &str, log: &std::path::Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by `rookey ui`, which rewrites or removes it: change the hotkey there. -->
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string><string>listen</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>ProcessType</key><string>Interactive</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{}</string></dict>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        xml(&exe.to_string_lossy()),
        xml(path),
        xml(&log.to_string_lossy()),
        xml(&log.to_string_lossy()),
    )
}

/// Where the agent writes what it says: `tail -f` it to follow the hotkey.
fn log_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/Logs/rookey-listen.log"))
}

/// Puts Rookey together, installs the agent and (re)starts it, so it picks up the keys just
/// saved.
pub fn start() -> Res<()> {
    let app = mac::place_app(&crate::exe()?)?;
    let plist = plist_path().ok_or_else(|| t!("listen.no-home"))?;
    let log = log_path().ok_or_else(|| t!("listen.no-home"))?;
    let path = env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    fs::create_dir_all(plist.parent().unwrap())?;
    fs::write(&plist, agent(&mac::app_exe(&app), &path, &log))?;
    // not loaded yet is fine: there is nothing to take out
    let _ = launchctl(&["bootout", &format!("{}/{LABEL}", domain())]);
    launchctl(&["bootstrap", &domain(), &plist.to_string_lossy()]).map(drop)
}

/// Stops the agent and takes it out of the login.
pub fn stop() -> Res<()> {
    let _ = launchctl(&["bootout", &format!("{}/{LABEL}", domain())]);
    if let Some(plist) = plist_path().filter(|p| p.is_file()) {
        fs::remove_file(plist)?;
    }
    Ok(())
}

/// After an update: a new copy of `exe` into Rookey, and the agent started again on it, if it
/// runs. Whether it was restarted.
pub fn restart(exe: &std::path::Path) -> Res<bool> {
    if !running() {
        return Ok(false);
    }
    mac::place_app(exe)?;
    launchctl(&["kickstart", "-k", &format!("{}/{LABEL}", domain())])?;
    Ok(true)
}

pub fn running() -> bool {
    launchctl(&["print", &format!("{}/{LABEL}", domain())]).is_ok_and(|info| info.contains("state = running"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(key_code("D"), Some(0x02));
        assert_eq!(key_code("d"), Some(0x02));
        assert_eq!(key_code("7"), Some(0x1A));
        assert_eq!(key_code("F13"), Some(0x69));
        assert_eq!(key_code("Alt_R"), Some(0x3D));
        assert_eq!(key_code("KP_5"), Some(0x57));
        assert_eq!(key_code("Nope"), None);
        for name in ["D", "7", "F13", "grave", "Control_R", "Alt_R", "KP_5", "Page_Down", "period"] {
            assert_eq!(key_name(key_code(name).unwrap()).as_deref(), Some(name));
        }
        assert!(is_modifier("Alt_R") && is_modifier("Super_L") && !is_modifier("grave"));
        // no key twice, and no code twice
        for (i, (name, code)) in NAMES.iter().enumerate() {
            assert!(NAMES[i + 1..].iter().all(|(n, c)| n != name && c != code), "{name}");
        }
    }

    #[test]
    fn the_agent_is_one_plist() {
        let plist = agent(std::path::Path::new("/a b/Rookey.app/Contents/MacOS/rookey"), "/opt/homebrew/bin:/usr/bin", std::path::Path::new("/l"));
        assert!(plist.contains("<string>/a b/Rookey.app/Contents/MacOS/rookey</string><string>listen</string>"));
        assert!(plist.contains(LABEL) && plist.contains("/opt/homebrew/bin"));
        assert_eq!(xml("a<b>&'\""), "a&lt;b&gt;&amp;&apos;&quot;");
    }
}
