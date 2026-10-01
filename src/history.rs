//! The last transcripts, kept on this computer only: text typed into the wrong window, or not
//! typed at all, can be copied again. One JSON object a line, `{"at": <Unix ms>, "text": ...}`,
//! oldest first, in <data_dir>/rookey/history, readable by its owner only.

use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

use serde_json::{Value, json};

/// How many are kept.
// ponytail: the whole file is read and rewritten on every save, nothing at this size; append
// and trim now and then if it ever has to keep thousands.
pub const KEEP: usize = 500;

pub fn path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("rookey").join("history"))
}

/// On unless ROOKEY_HISTORY is 0 or false.
fn on(value: Option<&str>) -> bool {
    !matches!(value.map(str::trim), Some("0" | "false"))
}

/// Keeps a transcript, with history on. A failure here costs the line, never the dictation.
pub fn save(text: &str) {
    let setting =
        env::var("ROOKEY_HISTORY").ok().or_else(|| crate::CONFIG.read().unwrap().get("ROOKEY_HISTORY").cloned());
    let Some(path) = path() else { return };
    if let Err(e) = add(&path, text, setting.as_deref(), KEEP, crate::status::now_ms()) {
        eprintln!("{}", crate::t!("history.failed", path = path.display(), why = e));
    }
}

fn add(path: &Path, text: &str, setting: Option<&str>, keep: usize, at: u64) -> io::Result<()> {
    if !on(setting) || text.trim().is_empty() {
        return Ok(());
    }
    let old = match fs::read_to_string(path) {
        Ok(old) => old,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e), // unreadable is not empty: writing now would wipe it
    };
    let mut lines: Vec<&str> = old.lines().filter(|l| !l.trim().is_empty()).collect();
    let line = json!({ "at": at, "text": text }).to_string();
    lines.push(&line);
    let kept = &lines[lines.len().saturating_sub(keep)..];
    crate::ui::write_private(path, &(kept.join("\n") + "\n"))
}

/// Oldest first. A line that doesn't read as an entry is skipped.
pub fn read(path: &Path) -> Vec<Value> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|e| e["text"].is_string()).collect()
}

pub fn clear(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `rookey history` prints what is kept, newest last; `--clear` deletes it.
pub fn run(clear_all: bool) -> crate::Res<()> {
    let path = path().ok_or_else(|| crate::t!("history.no-data-dir"))?;
    if clear_all {
        clear(&path)?;
        eprintln!("{}", crate::t!("history.cleared", path = path.display()));
        return Ok(());
    }
    let mut out = io::stdout().lock(); // println! panics on a closed pipe
    for e in read(&path) {
        writeln!(out, "{}  {}", utc(e["at"].as_u64().unwrap_or(0)), e["text"].as_str().unwrap_or_default())?;
    }
    Ok(())
}

/// "2026-09-30 14:05 UTC".
// ponytail: UTC, there's no time zone database here without a dependency; the page shows local time.
fn utc(ms: u64) -> String {
    let (days, secs) = ((ms / 86_400_000) as i64, ms / 1000 % 86_400);
    // days since 1970 to a date, Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    format!("{y}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, secs % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_trimmed_and_off() {
        let dir = env::temp_dir().join(format!("rookey-history-{}", std::process::id()));
        let path = dir.join("history");
        add(&path, "first \"one\"\nand a line", None, 3, 1).unwrap();
        let read_back = read(&path);
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0]["text"], "first \"one\"\nand a line");
        assert_eq!(read_back[0]["at"], 1);
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&fs::metadata(&path).unwrap().permissions()) & 0o777, 0o600);

        for (i, text) in ["two", "three", "four", "  "].iter().enumerate() {
            add(&path, text, Some(""), 3, i as u64 + 2).unwrap();
        }
        let texts: Vec<_> = read(&path).iter().map(|e| e["text"].as_str().unwrap().to_string()).collect();
        assert_eq!(texts, ["two", "three", "four"]); // the oldest went, nothing empty came

        add(&path, "not kept", Some("0"), 3, 9).unwrap();
        add(&path, "not kept", Some("false"), 3, 9).unwrap();
        assert_eq!(read(&path).len(), 3);
        clear(&path).unwrap();
        clear(&path).unwrap(); // gone already is fine
        add(&path, "off", Some("0"), 3, 9).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn times_in_utc() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(951_782_400_000), "2000-02-29 00:00 UTC");
        assert_eq!(utc(1_790_000_000_000), "2026-09-21 14:13 UTC");
    }
}
