//! `rookey listen`: the hotkey without the compositor. Reads the keyboards through evdev,
//! which gets what niri's binds can't: the key going up again. Hold the keys to talk and let
//! go to stop; a quick tap keeps it recording until the next press. Each start and stop is a
//! `rookey toggle`, so the recording itself works the same as from any other hotkey.
//!
//! Runs as a systemd user service tied to the graphical session, which carries the Wayland
//! variables typing needs. The page installs, restarts and removes it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use std::{fs, thread};

use evdev::{EventSummary, KeyCode};

use crate::{Res, t};
use crate::desktop::Chord;
use crate::hold::{Hold, recording, toggle};

const UNIT: &str = "rookey-listen.service";

/// XKB names (what compositors write) that differ from the kernel's, which is "KEY_" + the rest.
const NAMES: [(&str, &str); 27] = [
    ("space", "SPACE"),
    ("Return", "ENTER"),
    ("grave", "GRAVE"),
    ("minus", "MINUS"),
    ("equal", "EQUAL"),
    ("bracketleft", "LEFTBRACE"),
    ("bracketright", "RIGHTBRACE"),
    ("backslash", "BACKSLASH"),
    ("semicolon", "SEMICOLON"),
    ("apostrophe", "APOSTROPHE"),
    ("comma", "COMMA"),
    ("period", "DOT"),
    ("slash", "SLASH"),
    ("Page_Up", "PAGEUP"),
    ("Page_Down", "PAGEDOWN"),
    ("Scroll_Lock", "SCROLLLOCK"),
    ("Caps_Lock", "CAPSLOCK"),
    ("Print", "SYSRQ"),
    ("Menu", "COMPOSE"),
    ("Control_L", "LEFTCTRL"),
    ("Control_R", "RIGHTCTRL"),
    ("Alt_L", "LEFTALT"),
    ("Alt_R", "RIGHTALT"),
    ("Shift_L", "LEFTSHIFT"),
    ("Shift_R", "RIGHTSHIFT"),
    ("Super_L", "LEFTMETA"),
    ("Super_R", "RIGHTMETA"),
];

/// The key by the name compositors give it (XKB), as the kernel numbers it.
pub fn key_code(name: &str) -> Option<KeyCode> {
    let kernel = match NAMES.iter().find(|(xkb, _)| *xkb == name) {
        Some((_, kernel)) => kernel.to_string(),
        None => match name.to_ascii_uppercase().strip_prefix("KP_") {
            Some(digit) => format!("KP{digit}"),
            None => name.to_ascii_uppercase(),
        },
    };
    KeyCode::from_str(&format!("KEY_{kernel}")).ok()
}

/// The other way round: the name a compositor gives the key.
fn key_name(code: KeyCode) -> Option<String> {
    let kernel = format!("{code:?}");
    let kernel = kernel.strip_prefix("KEY_")?;
    if let Some((xkb, _)) = NAMES.iter().find(|(_, k)| *k == kernel) {
        return Some(xkb.to_string());
    }
    if let Some(digit) = kernel.strip_prefix("KP").filter(|d| d.len() == 1 && d.chars().all(|c| c.is_ascii_digit())) {
        return Some(format!("KP_{digit}"));
    }
    let plain = kernel.len() == 1 || (kernel.starts_with('F') && kernel[1..].chars().all(|c| c.is_ascii_digit()));
    plain.then(|| kernel.to_string())
}

/// Which modifier a key is, by the name `Chord` gives it.
fn modifier(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        KeyCode::KEY_LEFTMETA | KeyCode::KEY_RIGHTMETA => "Super",
        KeyCode::KEY_LEFTCTRL | KeyCode::KEY_RIGHTCTRL => "Ctrl",
        KeyCode::KEY_LEFTALT | KeyCode::KEY_RIGHTALT => "Alt",
        KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT => "Shift",
        _ => return None,
    })
}

/// Why the keyboards can't be read, if they can't.
pub fn access() -> Option<String> {
    let readable = fs::read_dir("/dev/input")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
        .any(|e| fs::File::open(e.path()).is_ok());
    (!readable).then(|| t!("listen.input-group"))
}

pub fn run() -> Res<()> {
    let chord = crate::setting("ROOKEY_HOTKEY").ok_or_else(|| t!("listen.no-hotkey"))?;
    let chord = Chord::parse(&chord)?;
    let key = key_code(chord.key()).ok_or_else(|| t!("server.key-unknown", key = chord.key()))?;
    if let Some(why) = access() {
        return Err(why.into());
    }
    eprintln!("{}", t!("listen.started", chord = chord));
    crate::update::in_background(true);

    let (tx, rx) = mpsc::channel();
    let open = Arc::new(Mutex::new(HashSet::new()));
    // keyboards come and go (a wireless one sleeps), so look again every few seconds
    thread::spawn(move || {
        loop {
            for (path, device) in evdev::enumerate() {
                if !device.supported_keys().is_some_and(|keys| keys.contains(key)) {
                    continue;
                }
                if !open.lock().unwrap().insert(path.clone()) {
                    continue;
                }
                vlog!(2, "listen: reading {} ({})", path.display(), device.name().unwrap_or("?"));
                let (tx, open) = (tx.clone(), open.clone());
                thread::spawn(move || {
                    read(device, &tx);
                    open.lock().unwrap().remove(&path);
                });
            }
            thread::sleep(Duration::from_secs(3));
        }
    });

    let mut hold = Hold::new(chord, key, modifier);
    for (code, value) in rx {
        if hold.event(code, value, Instant::now(), recording) {
            toggle();
        }
    }
    Ok(())
}

/// Waits for keys to be pressed, the way the page's "Press keys" does, but from the keyboards
/// themselves: keys the compositor keeps from the browser come through too. A modifier alone
/// counts as it goes up with nothing pressed in between. None for Esc or once `wait` is over.
pub fn capture(wait: Duration) -> Res<Option<String>> {
    if let Some(why) = access() {
        return Err(why.into());
    }
    let (tx, rx) = mpsc::channel();
    for (_, device) in evdev::enumerate() {
        if device.supported_keys().is_some_and(|keys| keys.contains(KeyCode::KEY_A)) {
            let tx = tx.clone();
            // ends at the first key after this is done, as its send finds nobody listening
            thread::spawn(move || read(device, &tx));
        }
    }
    let until = Instant::now() + wait;
    let mut held: HashSet<KeyCode> = HashSet::new();
    let mut lone = None;
    while let Ok((code, value)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        match value {
            1 => {
                held.insert(code);
                if code == KeyCode::KEY_ESC {
                    return Ok(None);
                }
                if modifier(code).is_some() {
                    lone = Some(code);
                    continue;
                }
                lone = None;
                let Some(key) = key_name(code) else { continue };
                let mut mods: Vec<&str> = held.iter().filter_map(|&k| modifier(k)).collect();
                mods.sort_by_key(|m| crate::desktop::MODS.iter().position(|x| x == m));
                mods.dedup();
                return Ok(Some(mods.into_iter().chain([key.as_str()]).collect::<Vec<_>>().join("+")));
            }
            0 => {
                held.remove(&code);
                if lone == Some(code) {
                    return Ok(key_name(code));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Whether the key is a modifier, which on its own types nothing and no compositor binds.
pub fn is_modifier(name: &str) -> bool {
    key_code(name).and_then(modifier).is_some()
}

/// Passes the key events of one keyboard on, until it goes away.
fn read(mut device: evdev::Device, tx: &mpsc::Sender<(KeyCode, i32)>) {
    while let Ok(events) = device.fetch_events() {
        for event in events {
            if let EventSummary::Key(_, code, value) = event.destructure() {
                if tx.send((code, value)).is_err() {
                    return;
                }
            }
        }
    }
}

fn unit_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("systemd/user").join(UNIT))
}

fn systemctl(args: &[&str]) -> Res<()> {
    let out = Command::new("systemctl").arg("--user").args(args).output()?;
    if !out.status.success() {
        return Err(t!("listen.tool-failed", tool = format!("systemctl {}", args.join(" ")), why = String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    Ok(())
}

/// Installs the service and (re)starts it, so it picks up the keys just saved.
pub fn start() -> Res<()> {
    let exe = crate::exe()?;
    let unit = format!(
        "# Written by `rookey ui`, which rewrites or removes it: change the hotkey there.\n\
         [Unit]\n\
         Description=rookey hotkey: hold to talk\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart=\"{}\" listen\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        exe.display()
    );
    let path = unit_path().ok_or_else(|| t!("listen.no-service-dir"))?;
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&path, unit)?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", UNIT])?;
    systemctl(&["restart", UNIT])
}

/// Stops the service and takes it away.
pub fn stop() -> Res<()> {
    let Some(path) = unit_path().filter(|p| p.is_file()) else { return Ok(()) };
    let _ = systemctl(&["disable", "--now", UNIT]);
    fs::remove_file(path)?;
    systemctl(&["daemon-reload"])
}

/// Starts the service again on the binary on disk now, if it runs and runs `exe`: another
/// rookey's service is left alone. Whether it was restarted.
pub fn restart(exe: &std::path::Path) -> Res<bool> {
    let unit = unit_path().and_then(|p| fs::read_to_string(p).ok()).unwrap_or_default();
    if !unit.contains(&format!("ExecStart=\"{}\" listen", exe.display())) || !running() {
        return Ok(false);
    }
    systemctl(&["try-restart", UNIT])?;
    Ok(true)
}

pub fn running() -> bool {
    Command::new("systemctl").args(["--user", "is-active", "--quiet", UNIT]).status().is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(key_code("D"), Some(KeyCode::KEY_D));
        assert_eq!(key_code("d"), Some(KeyCode::KEY_D));
        assert_eq!(key_code("7"), Some(KeyCode::KEY_7));
        assert_eq!(key_code("F13"), Some(KeyCode::KEY_F13));
        assert_eq!(key_code("grave"), Some(KeyCode::KEY_GRAVE));
        assert_eq!(key_code("Control_R"), Some(KeyCode::KEY_RIGHTCTRL));
        assert_eq!(key_code("KP_5"), Some(KeyCode::KEY_KP5));
        assert_eq!(key_code("Page_Down"), Some(KeyCode::KEY_PAGEDOWN));
        assert_eq!(key_code("Nope"), None);
        for name in ["D", "7", "F13", "grave", "Control_R", "KP_5", "Page_Down", "period"] {
            assert_eq!(key_name(key_code(name).unwrap()).as_deref(), Some(name));
        }
        assert!(is_modifier("Alt_R") && !is_modifier("grave"));
    }
}
