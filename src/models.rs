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

/// What a download left behind when rookey was stopped in the middle of it.
pub fn clear_leftovers() {
    let Ok(entries) = fs::read_dir(dir()) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if name.starts_with("ggml-") && name.ends_with(".bin.part") {
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
    let mut places = places();
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
        return Err(format!("{file} is not a model rookey knows where to get.").into());
    }
    let mut slot = DOWNLOAD.lock().unwrap();
    if let Some(running) = slot.as_ref().filter(|d| d.running) {
        return Err(format!("{} is still downloading.", running.file).into());
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

// ponytail: starts over after a failure and trusts HTTPS plus the byte count; add range
// requests and the sha256 Hugging Face publishes if downloads prove flaky.
fn fetch(file: &str) -> Res<()> {
    let target = dir().join(file);
    let part = dir().join(format!("{file}.part"));
    fs::create_dir_all(dir())?;

    let result = (|| -> Res<()> {
        let mut res = ureq::get(format!("{SOURCE}/{file}")).call()?;
        let total = res
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()?.parse().ok())
            .unwrap_or(0);
        if let Some(d) = DOWNLOAD.lock().unwrap().as_mut() {
            d.total = total;
        }
        let mut body = res.body_mut().as_reader();
        let mut out = fs::File::create(&part)?;
        let mut chunk = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            if CANCEL.load(Ordering::SeqCst) {
                return Err("Stopped.".into());
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
        if total != 0 && done != total {
            return Err(format!("The download broke off at {done} of {total} bytes.").into());
        }
        if !is_model(&part) {
            return Err("What came down is not a whisper model.".into());
        }
        Ok(())
    })();

    match result {
        // no half-downloaded model where a whole one is expected
        Ok(()) => Ok(fs::rename(&part, &target)?),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}
