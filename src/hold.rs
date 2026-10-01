//! Hold to talk, tap to keep talking: the part of `rookey listen` that doesn't care where the
//! key events come from (evdev on Linux, the key state on Windows).

use std::collections::HashSet;
use std::hash::Hash;
use std::process::Command;
use std::time::{Duration, Instant};
use std::{fs, thread};

use crate::desktop::Chord;

/// A press shorter than this is a tap: it leaves the recording running.
const TAP: Duration = Duration::from_millis(300);

/// Which key events start or stop a recording. `K` is the platform's key code.
pub struct Hold<K> {
    chord: Chord,
    key: K,
    /// Which modifier a key is, by the name `Chord` gives it.
    modifier: fn(K) -> Option<&'static str>,
    // ponytail: a key whose release got lost with its keyboard stays in here; restart the listener if that ever bites
    held: HashSet<K>,
    /// When the press that started the recording went down.
    pressed: Option<Instant>,
}

impl<K: Copy + Eq + Hash> Hold<K> {
    pub fn new(chord: Chord, key: K, modifier: fn(K) -> Option<&'static str>) -> Hold<K> {
        Hold { chord, key, modifier, held: HashSet::new(), pressed: None }
    }

    /// Whether this event (1 down, 0 up, 2 repeat) should run `rookey toggle`.
    pub fn event(&mut self, code: K, value: i32, now: Instant, recording: impl Fn() -> bool) -> bool {
        match value {
            1 => self.held.insert(code),
            0 => self.held.remove(&code),
            _ => return false, // auto-repeat
        };
        if code != self.key {
            return false;
        }
        if value == 0 {
            // a hold ends the recording as it lets go, a tap leaves it running
            return self.pressed.take().is_some_and(|at| now - at >= TAP) && recording();
        }
        // the modifiers held besides the key itself, all of them and nothing else
        let mut mods: Vec<&str> = self.held.iter().filter(|&&k| k != self.key).filter_map(|&k| (self.modifier)(k)).collect();
        mods.sort_by_key(|m| crate::desktop::MODS.iter().position(|x| x == m));
        mods.dedup();
        if mods != self.chord.mods() {
            return false;
        }
        // running already, from a tap: this press ends it, and its release does nothing
        self.pressed = (!recording()).then_some(now);
        true
    }
}

/// Whether a `rookey toggle` recording is running now.
pub fn recording() -> bool {
    let Ok(pid) = fs::read_to_string(crate::pidfile()) else { return false };
    pid.trim().parse().is_ok_and(crate::alive)
}

pub fn toggle() {
    let exe = crate::exe().unwrap_or_else(|_| "rookey".into());
    let mut cmd = Command::new(exe);
    cmd.arg("toggle");
    crate::no_window(&mut cmd);
    match cmd.spawn() {
        // reaped by a thread of its own, so none are left as zombies
        Ok(mut child) => drop(thread::spawn(move || child.wait())),
        Err(e) => eprintln!("{}", crate::t!("listen.toggle-failed", why = e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_and_tap() {
        const D: u16 = 1;
        const CTRL: u16 = 2;
        const X: u16 = 3;
        const SHIFT: u16 = 4;
        const CTRL_R: u16 = 5;
        let modifier = |k| match k {
            CTRL | CTRL_R => Some("Ctrl"),
            SHIFT => Some("Shift"),
            _ => None,
        };
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut hold = Hold::new(Chord::parse("Ctrl+D").unwrap(), D, modifier);

        // D alone, or with a key that isn't Ctrl, is ordinary typing
        assert!(!hold.event(D, 1, ms(0), || false));
        assert!(!hold.event(D, 0, ms(10), || false));

        // held: starts on the way down, stops on the way up
        assert!(!hold.event(CTRL, 1, ms(0), || false));
        assert!(hold.event(D, 1, ms(10), || false));
        assert!(!hold.event(D, 2, ms(300), || true)); // auto-repeat
        assert!(hold.event(D, 0, ms(900), || true));

        // tapped: starts, keeps going past the release, the next press stops it
        assert!(hold.event(D, 1, ms(1000), || false));
        assert!(!hold.event(D, 0, ms(1100), || true));
        assert!(hold.event(D, 1, ms(5000), || true));
        assert!(!hold.event(D, 0, ms(6000), || false));

        // Ctrl+Shift+D is another combination
        assert!(!hold.event(SHIFT, 1, ms(7000), || false));
        assert!(!hold.event(D, 1, ms(7010), || false));
        assert!(!hold.event(X, 1, ms(7020), || false));

        // a key on its own, like the right Ctrl, is not a modifier of itself
        let mut hold = Hold::new(Chord::parse("Control_R").unwrap(), CTRL_R, modifier);
        assert!(hold.event(CTRL_R, 1, ms(0), || false));
        assert!(hold.event(CTRL_R, 0, ms(800), || true));
    }
}
