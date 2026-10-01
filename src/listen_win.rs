//! `rookey listen` on Windows: the same hold to talk as on Linux, from the keys' state.
//!
//! Runs from the user's Run key, so it starts at login. The page adds, restarts and removes it.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use std::{fs, thread};

use windows_sys::Win32::System::Console::FreeConsole;

use crate::desktop::Chord;
use crate::hold::{Hold, recording, toggle};
use crate::win;
use crate::{Res, t};

// ponytail: polls the key state 100 times a second; a low-level keyboard hook if that ever
// shows up in the battery or misses a quick tap.
const POLL: Duration = Duration::from_millis(10);
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "rookey-listen";

/// Virtual-key codes by the names compositors give the keys (XKB), which is what
/// `ROOKEY_HOTKEY` holds on every system. Letters, digits, F-keys and KP_ digits are worked out.
const NAMES: [(&str, u16); 29] = [
    ("space", 0x20),
    ("Return", 0x0D),
    ("Escape", 0x1B),
    ("grave", 0xC0),
    ("minus", 0xBD),
    ("equal", 0xBB),
    ("bracketleft", 0xDB),
    ("bracketright", 0xDD),
    ("backslash", 0xDC),
    ("semicolon", 0xBA),
    ("apostrophe", 0xDE),
    ("comma", 0xBC),
    ("period", 0xBE),
    ("slash", 0xBF),
    ("Page_Up", 0x21),
    ("Page_Down", 0x22),
    ("Scroll_Lock", 0x91),
    ("Caps_Lock", 0x14),
    ("Print", 0x2C),
    ("Menu", 0x5D),
    ("Pause", 0x13),
    ("Control_L", 0xA2),
    ("Control_R", 0xA3),
    ("Alt_L", 0xA4),
    ("Alt_R", 0xA5),
    ("Shift_L", 0xA0),
    ("Shift_R", 0xA1),
    ("Super_L", 0x5B),
    ("Super_R", 0x5C),
];

const ESCAPE: u16 = 0x1B;

pub fn key_code(name: &str) -> Option<u16> {
    if let Some(&(_, vk)) = NAMES.iter().find(|(xkb, _)| *xkb == name) {
        return Some(vk);
    }
    let upper = name.to_ascii_uppercase();
    if upper.len() == 1 && upper.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(upper.as_bytes()[0] as u16);
    }
    let number = |rest: &str| rest.parse::<u16>().ok();
    if let Some(n) = upper.strip_prefix('F').and_then(number).filter(|n| (1..=24).contains(n)) {
        return Some(0x70 + n - 1);
    }
    upper.strip_prefix("KP_").and_then(number).filter(|&n| n <= 9).map(|n| 0x60 + n)
}

fn key_name(vk: u16) -> Option<String> {
    if let Some((xkb, _)) = NAMES.iter().find(|(_, code)| *code == vk) {
        return Some(xkb.to_string());
    }
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some((vk as u8 as char).to_string()),
        0x60..=0x69 => Some(format!("KP_{}", vk - 0x60)),
        0x70..=0x87 => Some(format!("F{}", vk - 0x70 + 1)),
        _ => None,
    }
}

fn modifier(vk: u16) -> Option<&'static str> {
    Some(match vk {
        0x5B | 0x5C => "Super",
        0xA2 | 0xA3 => "Ctrl",
        0xA4 | 0xA5 => "Alt",
        0xA0 | 0xA1 => "Shift",
        _ => return None,
    })
}

pub fn is_modifier(name: &str) -> bool {
    key_code(name).and_then(modifier).is_some()
}

/// Any user can read the key state on Windows.
pub fn access() -> Option<String> {
    None
}

/// Calls `on` with each key that went down (1) or up (0) among `keys`, until it returns false.
fn watch(keys: &[u16], mut on: impl FnMut(u16, i32) -> bool) {
    // a key already down when this starts (the click on the page's button) is no press
    let mut was: Vec<bool> = keys.iter().map(|&k| win::down(k)).collect();
    loop {
        for (i, &vk) in keys.iter().enumerate() {
            let now = win::down(vk);
            if now != was[i] {
                was[i] = now;
                if !on(vk, now as i32) {
                    return;
                }
            }
        }
        thread::sleep(POLL);
    }
}

pub fn run() -> Res<()> {
    let chord = crate::setting("ROOKEY_HOTKEY").ok_or_else(|| t!("listen.no-hotkey"))?;
    let chord = Chord::parse(&chord)?;
    let key = key_code(chord.key()).ok_or_else(|| t!("server.key-unknown", key = chord.key()))?;
    eprintln!("{}", t!("listen.started", chord = chord));
    fs::write(pidfile(), std::process::id().to_string())?;
    // started from the Run key it got a console window of its own, which this closes
    unsafe { FreeConsole() };
    crate::update::in_background(true);

    let mut keys = vec![key, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
    keys.dedup();
    let mut hold = Hold::new(chord, key, modifier);
    watch(&keys, |vk, value| {
        if hold.event(vk, value, Instant::now(), recording) {
            toggle();
        }
        true
    });
    Ok(())
}

/// Waits for keys to be pressed, like the Linux one: a modifier alone counts as it goes up
/// with nothing pressed in between. None for Esc or once `wait` is over.
pub fn capture(wait: Duration) -> Res<Option<String>> {
    // every key but the mouse buttons and the Shift, Ctrl and Alt that stand for either side
    let keys: Vec<u16> = (0x08..=0xFE).filter(|k| !(0x10..=0x12).contains(k)).collect();
    let (tx, rx) = mpsc::channel();
    // ends at the first key after this is done, as its send finds nobody listening
    thread::spawn(move || watch(&keys, |vk, value| tx.send((vk, value)).is_ok()));

    let until = Instant::now() + wait;
    let mut held = Vec::new();
    let mut lone = None;
    while let Ok((vk, value)) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        if value == 0 {
            held.retain(|&k| k != vk);
            if lone == Some(vk) {
                return Ok(key_name(vk));
            }
            continue;
        }
        held.push(vk);
        if vk == ESCAPE {
            return Ok(None);
        }
        if modifier(vk).is_some() {
            lone = Some(vk);
            continue;
        }
        lone = None;
        let Some(key) = key_name(vk) else { continue };
        let mut mods: Vec<&str> = held.iter().filter_map(|&k| modifier(k)).collect();
        mods.sort_by_key(|m| crate::desktop::MODS.iter().position(|x| x == m));
        mods.dedup();
        return Ok(Some(mods.into_iter().chain([key.as_str()]).collect::<Vec<_>>().join("+")));
    }
    Ok(None)
}

fn pidfile() -> PathBuf {
    crate::pidfile().with_file_name("rookey-listen.pid")
}

fn reg(args: &[&str]) -> Res<()> {
    let mut cmd = Command::new("reg");
    cmd.args(args);
    crate::no_window(&mut cmd);
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(t!(
            "listen.tool-failed",
            tool = format!("reg {}", args[0]),
            why = String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(())
}

/// Starts at every login from now on, and (re)starts now, so it picks up the keys just saved.
pub fn start() -> Res<()> {
    let exe = crate::exe()?;
    reg(&["add", RUN_KEY, "/v", RUN_NAME, "/t", "REG_SZ", "/d", &format!("\"{}\" listen", exe.display()), "/f"])?;
    end();
    let mut cmd = Command::new(exe);
    cmd.arg("listen");
    crate::no_window(&mut cmd);
    cmd.spawn()?;
    Ok(())
}

/// Stops the listener and takes it out of the login.
pub fn stop() -> Res<()> {
    end();
    // not there is fine: nothing to take out
    let _ = reg(&["delete", RUN_KEY, "/v", RUN_NAME, "/f"]);
    Ok(())
}

/// Leaves the listener running: it picks a new binary up at the next login.
// ponytail: which rookey.exe the running listener is isn't checked here, so it isn't ended;
// read the Run key's value and compare it with `exe` to restart it like on Linux.
pub fn restart(_exe: &std::path::Path) -> Res<bool> {
    Ok(false)
}

pub fn running() -> bool {
    fs::read_to_string(pidfile()).ok().and_then(|p| p.trim().parse().ok()).is_some_and(win::alive)
}

/// Ends the listener that runs now, if one does.
fn end() {
    let Some(pid) = fs::read_to_string(pidfile()).ok().filter(|_| running()) else { return };
    let mut cmd = Command::new("taskkill");
    cmd.args(["/PID", pid.trim(), "/F"]);
    crate::no_window(&mut cmd);
    let _ = cmd.output();
    let _ = fs::remove_file(pidfile());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(key_code("D"), Some(0x44));
        assert_eq!(key_code("d"), Some(0x44));
        assert_eq!(key_code("7"), Some(0x37));
        assert_eq!(key_code("F13"), Some(0x7C));
        assert_eq!(key_code("Control_R"), Some(0xA3));
        assert_eq!(key_code("KP_5"), Some(0x65));
        assert_eq!(key_code("Nope"), None);
        for name in ["D", "7", "F13", "grave", "Control_R", "KP_5", "Page_Down", "period"] {
            assert_eq!(key_name(key_code(name).unwrap()).as_deref(), Some(name));
        }
        assert!(is_modifier("Alt_R") && !is_modifier("grave"));
    }
}
