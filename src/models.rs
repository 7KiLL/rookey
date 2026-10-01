//! Whisper models for the local engine: which ones are on disk, and fetching new ones.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs, thread};

use serde_json::{Value, json};

use crate::Res;

const SOURCE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// (file, name, size in MB, what to expect). The models of whisper.cpp worth offering;
/// any other ggml file on disk is listed too.
pub const CATALOG: [(&str, &str, u32, &str); 5] = [
    (
        "ggml-large-v3-turbo.bin",
        "Large v3 turbo",
        1549,
        "The one rookey starts with. Close to the largest model in accuracy, several times faster.",
    ),
    (
        "ggml-large-v3-turbo-q5_0.bin",
        "Large v3 turbo, compressed",
        547,
        "The same model at a third of the size, a little less exact. For less memory.",
    ),
    ("ggml-large-v3.bin", "Large v3", 2952, "The largest and the slowest. Wants a GPU."),
    ("ggml-small.bin", "Small", 465, "Quick without a GPU. More mistakes, above all outside English."),
    ("ggml-base.bin", "Base", 141, "The quickest here, and the least exact."),
];

/// Where rookey keeps its models, and where else whisper.cpp models tend to be.
fn places() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let data = dirs::data_dir().unwrap_or_default();
    vec![
        dir(),
        data.join("pywhispercpp").join("models"),
        home.join(".cache").join("whisper.cpp"),
        home.join("whisper.cpp").join("models"),
        PathBuf::from("/usr/share/whisper.cpp/models"),
    ]
}

pub fn dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("rookey")
}

/// What a download left behind when rookey was stopped in the middle of it. A part of a
/// catalog model stays a week, so the next download of it goes on from there.
pub fn clear_leftovers() {
    leftovers(&dir());
}

fn leftovers(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let week = std::time::Duration::from_secs(7 * 24 * 3600);
    for path in entries.flatten().map(|e| e.path()) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let Some(file) = name.strip_suffix(".part").filter(|f| f.starts_with("ggml-") && f.ends_with(".bin"))
        else {
            continue;
        };
        let age = fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
        if !CATALOG.iter().any(|k| k.0 == file) || age.is_none_or(|a| a > week) {
            let _ = fs::remove_file(path);
        }
    }
}

/// A whisper.cpp model starts with the ggml magic. A failed download saved as one doesn't.
fn is_model(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == b"lmgg"
}

/// Models on disk: rookey's own, the usual places of other tools, and next to `also`.
pub fn installed(also: Option<&Path>) -> Vec<PathBuf> {
    scan(places(), also)
}

fn scan(mut places: Vec<PathBuf>, also: Option<&Path>) -> Vec<PathBuf> {
    places.extend(also.and_then(Path::parent).map(Path::to_path_buf));
    let mut found: Vec<PathBuf> = Vec::new();
    for place in places {
        let Ok(entries) = fs::read_dir(place) else { continue };
        for path in entries.flatten().map(|e| e.path()) {
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let named = name.starts_with("ggml-") && name.ends_with(".bin");
            if named && !found.contains(&path) && is_model(&path) {
                found.push(path);
            }
        }
    }
    found.extend(also.filter(|p| !found.iter().any(|f| f == p) && is_model(p)).map(Path::to_path_buf));
    found.sort();
    found
}

#[derive(Default)]
struct Download {
    file: String,
    done: u64,
    total: u64,
    error: Option<String>,
    running: bool,
}

static DOWNLOAD: Mutex<Option<Download>> = Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);

/// For the page: the download in progress or the last one that failed.
pub fn download_state() -> Value {
    match &*DOWNLOAD.lock().unwrap() {
        Some(d) => json!({
            "file": d.file, "done": d.done, "total": d.total, "error": d.error, "running": d.running,
        }),
        None => Value::Null,
    }
}

pub fn cancel() {
    CANCEL.store(true, Ordering::SeqCst);
}

/// Starts fetching a model from the catalog into rookey's directory. One at a time.
pub fn download(file: &str) -> Res<()> {
    if !CATALOG.iter().any(|known| known.0 == file) {
        return Err(crate::t!("models.unknown", file = file).into());
    }
    let mut slot = DOWNLOAD.lock().unwrap();
    if let Some(running) = slot.as_ref().filter(|d| d.running) {
        return Err(crate::t!("models.busy", file = running.file).into());
    }
    *slot = Some(Download { file: file.into(), running: true, ..Default::default() });
    CANCEL.store(false, Ordering::SeqCst);

    let file = file.to_string();
    thread::spawn(move || {
        let result = fetch(&file);
        let mut slot = DOWNLOAD.lock().unwrap();
        match result {
            Ok(()) => *slot = None,
            // stopped on request is not a failure to report
            Err(_) if CANCEL.load(Ordering::SeqCst) => *slot = None,
            Err(e) => {
                if let Some(d) = slot.as_mut() {
                    d.running = false;
                    d.error = Some(e.to_string());
                }
            }
        }
    });
    Ok(())
}

/// How the answer to `Range: bytes=<have>-` continues the part on disk: `Some(full size)`,
/// 0 when the server doesn't say, to append to it; `None` to start over.
fn continues(status: u16, have: u64, content_range: Option<&str>) -> Option<u64> {
    if status != 206 {
        return None;
    }
    // "bytes 100-199/200", or "bytes 100-199/*" without the size
    let (span, total) = content_range?.strip_prefix("bytes ")?.split_once('/')?;
    let start: u64 = span.split_once('-')?.0.parse().ok()?;
    (start == have).then(|| total.parse().unwrap_or(0))
}

// ponytail: goes on from the .part of a broken download with a range request, and trusts
// HTTPS plus the byte count; check the sha256 Hugging Face publishes if a model ever
// turns out damaged.
fn fetch(file: &str) -> Res<()> {
    let target = dir().join(file);
    let part = dir().join(format!("{file}.part"));
    fs::create_dir_all(dir())?;

    let result = (|| -> Res<()> {
        let (mut res, kept, total) = loop {
            let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            let mut req = ureq::get(format!("{SOURCE}/{file}"));
            if have > 0 {
                req = req.header("Range", format!("bytes={have}-"));
            }
            let res = match req.call() {
                // the part is as long as the model or longer: nothing to go on from
                Err(ureq::Error::StatusCode(416)) if have > 0 => {
                    fs::remove_file(&part)?;
                    continue;
                }
                r => r?,
            };
            let header = |name| res.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
            let len: u64 = header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
            let status = res.status().as_u16();
            match continues(status, have, header("content-range").as_deref()) {
                Some(0) => break (res, have, if len == 0 { 0 } else { have + len }),
                Some(total) => break (res, have, total),
                // a range other than the one asked for: ask for the whole file
                None if status == 206 && have > 0 => fs::remove_file(&part)?,
                None => break (res, 0, len),
            }
        };
        if let Some(d) = DOWNLOAD.lock().unwrap().as_mut() {
            d.total = total;
            d.done = kept;
        }
        let mut body = res.body_mut().as_reader();
        let mut out = if kept > 0 {
            fs::OpenOptions::new().append(true).open(&part)?
        } else {
            fs::File::create(&part)?
        };
        let mut chunk = vec![0u8; 256 * 1024];
        let mut done = kept;
        loop {
            if CANCEL.load(Ordering::SeqCst) {
                return Err(crate::t!("models.stopped").into());
            }
            let n = body.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            out.write_all(&chunk[..n])?;
            done += n as u64;
            if let Some(d) = DOWNLOAD.lock().unwrap().as_mut() {
                d.done = done;
            }
        }
        out.sync_all()?;
        if total != 0 && done < total {
            return Err(crate::t!("models.broke-off", done = done, total = total).into());
        }
        if (total != 0 && done > total) || !is_model(&part) {
            fs::remove_file(&part)?;
            return Err(crate::t!("models.not-model").into());
        }
        Ok(())
    })();

    match result {
        // no half-downloaded model where a whole one is expected
        Ok(()) => Ok(fs::rename(&part, &target)?),
        // stopped on request: nothing to go on from later
        Err(e) if CANCEL.load(Ordering::SeqCst) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
        // a broken connection leaves the part for the next try
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory of its own under the system temp dir, never rookey's data dir.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rookey-models-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn model(path: &Path) {
        fs::write(path, b"lmgg and the rest").unwrap();
    }

    #[test]
    fn the_catalog_names_ggml_files_once() {
        for (i, (file, name, mb, about)) in CATALOG.iter().enumerate() {
            assert!(file.starts_with("ggml-") && file.ends_with(".bin"), "{file}");
            assert!(!name.is_empty() && !about.is_empty() && *mb > 0, "{file}");
            assert!(CATALOG[..i].iter().all(|k| k.0 != *file), "{file} twice");
        }
    }

    #[test]
    fn only_catalog_models_download() {
        let err = download("ggml-evil.bin").unwrap_err().to_string();
        assert!(err.contains("not a model rookey knows"), "{err}");
        assert!(download("../ggml-base.bin").is_err());
    }

    #[test]
    fn a_model_starts_with_the_magic() {
        let dir = scratch("magic");
        model(&dir.join("ok"));
        fs::write(dir.join("page"), b"<!doctype html>").unwrap();
        fs::write(dir.join("short"), b"lm").unwrap();
        assert!(is_model(&dir.join("ok")));
        assert!(!is_model(&dir.join("page")));
        assert!(!is_model(&dir.join("short")));
        assert!(!is_model(&dir.join("missing")));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn scan_lists_models_once_and_sorted() {
        let (a, b) = (scratch("scan-a"), scratch("scan-b"));
        model(&a.join("ggml-small.bin"));
        model(&b.join("ggml-base.bin"));
        model(&a.join("ggml-base.bin.part")); // unfinished
        model(&a.join("notes.bin")); // not named like a model
        fs::write(a.join("ggml-large-v3.bin"), b"<html>").unwrap(); // a failed download
        let mine = b.join("mine.bin");
        model(&mine);

        let found = scan(vec![a.clone(), b.clone(), a.clone()], Some(&mine));
        let mut want = vec![a.join("ggml-small.bin"), b.join("ggml-base.bin"), mine.clone()];
        want.sort();
        assert_eq!(found, want);

        // `also` in a place already scanned is not listed twice, and one that isn't a model is left out
        assert_eq!(scan(vec![b.clone()], Some(&b.join("ggml-base.bin"))), vec![b.join("ggml-base.bin")]);
        // the folder of `also` is looked through too
        assert_eq!(scan(vec![], Some(&a.join("ggml-large-v3.bin"))), vec![a.join("ggml-small.bin")]);
        let _ = (fs::remove_dir_all(a), fs::remove_dir_all(b));
    }

    #[test]
    fn leftovers_keep_a_fresh_part_of_a_catalog_model() {
        let dir = scratch("leftovers");
        let fresh = dir.join("ggml-base.bin.part");
        let old = dir.join("ggml-small.bin.part");
        let unknown = dir.join("ggml-other.bin.part");
        let other = dir.join("keep.part");
        for p in [&fresh, &old, &unknown, &other] {
            fs::write(p, b"x").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 3600);
        fs::File::options().write(true).open(&old).unwrap().set_modified(long_ago).unwrap();
        model(&dir.join("ggml-base.bin"));

        leftovers(&dir);
        assert!(fresh.exists() && other.exists() && dir.join("ggml-base.bin").exists());
        assert!(!old.exists() && !unknown.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_ranged_answer_goes_on_or_starts_over() {
        assert_eq!(continues(206, 100, Some("bytes 100-199/200")), Some(200));
        assert_eq!(continues(206, 100, Some("bytes 100-199/*")), Some(0));
        // the whole file after all, or another range than the one asked for
        assert_eq!(continues(200, 100, Some("bytes 100-199/200")), None);
        assert_eq!(continues(206, 100, Some("bytes 0-199/200")), None);
        assert_eq!(continues(206, 100, None), None);
        assert_eq!(continues(206, 100, Some("items 100-199/200")), None);
        assert_eq!(continues(206, 100, Some("bytes x-199/200")), None);
    }
}
