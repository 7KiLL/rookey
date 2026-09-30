//! What rookey is doing now, for bars and the on-screen indicator. A recording keeps one small
//! JSON file next to the pidfile up to date; `rookey status` reads it, once or on every change.
//!
//! The file holds `{"state", "at", "pid", ...}`, with `at` in Unix milliseconds. Readers turn
//! it into what is true now: a recording whose process is gone is idle, "typed" is shown for
//! a moment, "failed" stays until the next recording.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, thread};

use serde_json::{Value, json};

use crate::Res;

/// How long "typed" is shown after the text went out.
pub const TYPED_MS: u64 = 1200;

pub fn path() -> PathBuf {
    crate::pidfile().with_extension("status")
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The last state this process wrote, so a level update keeps its `at`.
static LAST: Mutex<Option<Value>> = Mutex::new(None);

/// Records a new state. `extra` is merged in (level, words, reason...).
pub fn set(state: &str, extra: Value) {
    let mut v = json!({"state": state, "at": now_ms(), "pid": std::process::id()});
    if let (Some(v), Value::Object(extra)) = (v.as_object_mut(), extra) {
        v.extend(extra);
    }
    write(v);
}

/// The mic's peak level while listening, 0 to 1.
pub fn level(peak: f32) {
    let last = LAST.lock().unwrap().clone();
    if let Some(mut v) = last.filter(|v| v["state"] == "listening") {
        v["level"] = json!((peak.clamp(0.0, 1.0) * 100.0).round() / 100.0);
        write(v);
    }
}

/// The text went out: how many words, and how long the wait after the stop was.
pub fn typed(text: &str) {
    let last = LAST.lock().unwrap().clone();
    let waited = last.filter(|v| v["state"] == "transcribing").and_then(|v| v["at"].as_u64());
    let waited = waited.map_or(0, |at| now_ms().saturating_sub(at));
    set("typed", json!({"words": text.split_whitespace().count(), "waited_ms": waited}));
}

pub fn failed(reason: &str) {
    let reason = reason.lines().next().unwrap_or("").trim();
    set("failed", json!({"reason": reason.chars().take(120).collect::<String>()}));
}

// ponytail: write-then-rename per change (4 a second while listening, on tmpfs); a socket if
// readers ever need more than that
fn write(v: Value) {
    let path = path();
    let tmp = path.with_extension(format!("status.{}", std::process::id()));
    if fs::write(&tmp, v.to_string()).and_then(|_| fs::rename(&tmp, &path)).is_err() {
        let _ = fs::remove_file(&tmp);
    }
    *LAST.lock().unwrap() = Some(v);
}

/// The file as written: `Some(idle)` when there is none, `None` when it can't be read right
/// now (mid-rename on Windows), so a reader keeps what it had.
pub fn read() -> Option<Value> {
    match fs::read_to_string(path()) {
        Ok(text) => serde_json::from_str(&text).ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(json!({"state": "idle"})),
        Err(_) => None,
    }
}

fn running(pid: u32) -> bool {
    crate::alive(pid)
}

/// What is true at `now`, from the file: `{"state": "listening", "seconds": 4, "level": 0.41}`.
pub fn current(raw: &Value, now: u64, running: impl Fn(u32) -> bool) -> Value {
    let at = raw["at"].as_u64().unwrap_or(0);
    let alive = raw["pid"].as_u64().is_some_and(|p| running(p as u32));
    match raw["state"].as_str().unwrap_or("idle") {
        "listening" if alive => json!({
            "state": "listening",
            "seconds": now.saturating_sub(at) / 1000,
            "level": raw["level"].as_f64().unwrap_or(0.0),
        }),
        "transcribing" if alive => json!({"state": "transcribing"}),
        "typed" if now.saturating_sub(at) < TYPED_MS => {
            json!({"state": "typed", "words": raw["words"], "waited_ms": raw["waited_ms"]})
        }
        "failed" => json!({"state": "failed", "reason": raw["reason"]}),
        _ => json!({"state": "idle"}),
    }
}

pub fn clock(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// A line for people: `listening 0:04`.
pub fn text(now: &Value) -> String {
    match now["state"].as_str().unwrap_or("idle") {
        "listening" => format!("listening {}", clock(now["seconds"].as_u64().unwrap_or(0))),
        "typed" => format!("typed {} words", now["words"]),
        "failed" => format!("failed: {}", now["reason"].as_str().unwrap_or("")),
        state => state.to_string(),
    }
}

/// Waybar's custom module format. `alt` and `class` are the state, for `format-icons` and CSS.
pub fn waybar(now: &Value) -> Value {
    let state = now["state"].as_str().unwrap_or("idle");
    let shown = match state {
        "listening" => clock(now["seconds"].as_u64().unwrap_or(0)),
        "typed" => format!("{} words", now["words"]),
        "failed" => "failed".into(),
        s => s.into(),
    };
    json!({"text": shown, "alt": state, "class": state, "tooltip": format!("rookey: {}", text(now))})
}

/// `rookey status [--json | --waybar] [--follow]`
pub fn run(args: &[String]) -> Res<()> {
    let (mut json, mut waybar_out, mut follow) = (false, false, false);
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            "--waybar" => waybar_out = true,
            "--follow" | "-f" => follow = true,
            _ => return Err(format!("status takes --json, --waybar and --follow, not {a}").into()),
        }
    }
    let line = |now: &Value| match () {
        _ if waybar_out => waybar(now).to_string(),
        _ if json => now.to_string(),
        _ => text(now),
    };
    let mut raw = read().unwrap_or(json!({"state": "idle"}));
    let mut last = String::new();
    loop {
        let shown = line(&current(&raw, now_ms(), running));
        if shown != last {
            let mut out = std::io::stdout().lock();
            // a bar that went away closed the pipe: that's the end, not an error
            if writeln!(out, "{shown}").and_then(|_| out.flush()).is_err() {
                return Ok(());
            }
            last = shown;
        }
        if !follow {
            return Ok(());
        }
        // ponytail: polls a tmpfs file 10 times a second; inotify if that ever shows up in a profile
        thread::sleep(Duration::from_millis(100));
        raw = read().unwrap_or(raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_is_true_now() {
        let up = |_| true;
        let down = |_| false;
        let listening = json!({"state": "listening", "at": 1_000, "pid": 7, "level": 0.41});
        assert_eq!(current(&listening, 5_200, up), json!({"state": "listening", "seconds": 4, "level": 0.41}));
        // the recording crashed or was killed: idle, not listening forever
        assert_eq!(current(&listening, 5_200, down)["state"], "idle");
        let typed = json!({"state": "typed", "at": 1_000, "pid": 7, "words": 12, "waited_ms": 310});
        assert_eq!(current(&typed, 1_500, down)["words"], 12);
        assert_eq!(current(&typed, 1_000 + TYPED_MS, down)["state"], "idle");
        // failed stays, with the process long gone
        let failed = json!({"state": "failed", "at": 1_000, "pid": 7, "reason": "no key"});
        assert_eq!(current(&failed, 99_000_000, down), json!({"state": "failed", "reason": "no key"}));
        assert_eq!(current(&json!({"state": "nonsense"}), 0, up)["state"], "idle");
        assert_eq!(current(&json!("garbage"), 0, up)["state"], "idle");
    }

    #[test]
    fn prints_for_people_and_bars() {
        let now = json!({"state": "listening", "seconds": 64, "level": 0.2});
        assert_eq!(text(&now), "listening 1:04");
        let bar = waybar(&now);
        assert_eq!((bar["text"].as_str(), bar["class"].as_str()), (Some("1:04"), Some("listening")));
        assert_eq!(waybar(&json!({"state": "typed", "words": 3}))["text"], "3 words");
        assert_eq!(text(&json!({"state": "failed", "reason": "no key"})), "failed: no key");
    }
}
