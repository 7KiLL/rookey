//! rookey: record the mic, transcribe locally with whisper.cpp, print or type the text.
//!
//!   rookey          record until Enter (or Ctrl-C), print transcript to stdout
//!   rookey toggle   first call starts recording, second call stops it and types the text
//!   rookey ui       settings in the browser (`rookey setup` too: what's missing comes first)
//!   rookey history  the last transcripts, kept on this computer (`--clear` deletes them)
//!   rookey update   installs a newer release (`--check` only says whether there is one)
//!
//! Settings are env vars, or KEY=value lines in <config_dir>/rookey/config (the environment wins):
//! ROOKEY_MODEL (ggml model path), ROOKEY_LANG (default "auto"), ROOKEY_BACKEND,
//! ROOKEY_SANITIZE, ROOKEY_EDIT, ROOKEY_WORDS, ROOKEY_CONTEXT, ROOKEY_READER, ROOKEY_HISTORY, ROOKEY_AUTOUPDATE (see README).
//! API keys are read the same way, from <data_dir>/rookey/keys.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const WHISPER_RATE: u32 = 16_000;
const DEFAULT_MODEL: &str = "ggml-large-v3-turbo.bin";

/// 0 quiet, 1 (-v) transcript text as it arrives, 2 (-vv) steps + timings, 3 (-vvv) every chunk.
static VERBOSE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn verbosity() -> u8 {
    VERBOSE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Log on stderr at the given verbosity level, timestamped from process start.
macro_rules! vlog {
    ($level:expr, $($arg:tt)*) => {
        if $crate::verbosity() >= $level {
            let t = $crate::START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64();
            eprintln!("[{t:7.3}s] {}", format!($($arg)*));
        }
    };
}

mod desktop;
mod history;
#[cfg(target_os = "macos")]
mod mac;
mod hold;
#[cfg(target_os = "linux")]
mod listen;
#[cfg(windows)]
#[path = "listen_win.rs"]
mod listen;
#[cfg(target_os = "macos")]
#[path = "listen_mac.rs"]
mod listen;
mod models;
mod overlay;
#[cfg(windows)]
mod overlay_win;
mod reader;
mod sound;
mod status;
mod ui;
mod update;
#[cfg(windows)]
mod win;

static CONFIG: LazyLock<RwLock<HashMap<String, String>>> = LazyLock::new(Default::default);

/// A setting by its env var name, from the environment or else the files.
/// Unset, "", "0" and "false" all mean off.
fn setting(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .or_else(|| CONFIG.read().unwrap().get(key).cloned())
        .filter(|v| !matches!(v.as_str(), "" | "0" | "false"))
}

/// Holds the pid of the `rookey toggle` recording while one runs.
fn pidfile() -> PathBuf {
    dirs::runtime_dir().unwrap_or_else(env::temp_dir).join("rookey.pid")
}

/// Asks the `rookey toggle` recording to stop, where there are no signals to send (Windows).
#[cfg(windows)]
fn stopfile() -> PathBuf {
    pidfile().with_extension("stop")
}

/// Whether the process with this pid still runs.
fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    return win::alive(pid);
    #[cfg(target_os = "linux")]
    return std::path::Path::new("/proc").join(pid.to_string()).exists();
    #[cfg(target_os = "macos")]
    return mac::alive(pid);
}

/// A program started from a hotkey or the page gets no console window of its own on Windows.
fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(cmd, 0x0800_0000); // CREATE_NO_WINDOW
    #[cfg(not(windows))]
    let _ = cmd;
}

/// This program's file, to start again. After an update put a new one in its place, that is
/// the new one: Linux names the running file "rookey (deleted)" then, and on Windows it was
/// moved aside to rookey.old.exe.
fn exe() -> std::io::Result<PathBuf> {
    Ok(on_disk(env::current_exe()?))
}

fn on_disk(exe: PathBuf) -> PathBuf {
    let name = exe.file_name().unwrap_or_default().to_string_lossy().into_owned();
    match name.strip_suffix(" (deleted)").map(str::to_string).or_else(|| Some(name.strip_suffix(".old.exe")?.to_string() + ".exe")) {
        Some(name) => exe.with_file_name(name),
        None => exe,
    }
}

fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("rookey").join("config"))
}

/// API keys are kept apart from the settings: ~/.config is what dotfile managers sync,
/// and what ends up in a public repo.
// ponytail: a file only its owner can read; the system keyring if that stops being enough.
fn keys_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("rookey").join("keys"))
}

/// The tool was called yap before: its settings, keys and models move over once.
// ponytail: drop this once nobody has a yap directory left.
fn move_from_yap() {
    for base in [dirs::config_dir(), dirs::data_dir()].into_iter().flatten() {
        let (old, new) = (base.join("yap"), base.join("rookey"));
        if !old.is_dir() || new.exists() {
            continue;
        }
        if let Err(e) = fs::rename(&old, &new) {
            eprintln!("rookey: couldn't move {} to {}: {e}", old.display(), new.display());
            continue;
        }
        eprintln!("rookey: moved {} to {}", old.display(), new.display());
        for file in [new.join("config"), new.join("keys")] {
            let Ok(text) = fs::read_to_string(&file) else { continue };
            let renamed = without_yap_names(&text);
            if renamed == text {
                continue;
            }
            // a new file beside it, then a rename: a crash halfway never leaves half a keys file
            let tmp = file.with_extension("new");
            let done = fs::metadata(&file).and_then(|meta| {
                fs::write(&tmp, &renamed)?;
                fs::set_permissions(&tmp, meta.permissions())?;
                fs::rename(&tmp, &file)
            });
            if let Err(e) = done {
                let _ = fs::remove_file(&tmp);
                eprintln!("rookey: {} still uses YAP_ names: {e}", file.display());
            }
        }
    }
}

fn without_yap_names(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| match line.trim_start().strip_prefix("YAP_") {
            Some(rest) => format!("ROOKEY_{rest}"),
            None => line.to_string(),
        })
        .collect()
}

/// Reads the settings, then the keys. A key still sitting in the settings file counts,
/// one in the keys file wins over it.
fn load_config() {
    let mut all = HashMap::new();
    for path in [config_path(), keys_path()].into_iter().flatten() {
        match fs::read_to_string(&path) {
            Ok(text) => {
                let part = parse_config(&text);
                vlog!(2, "config: {} ({} settings)", path.display(), part.len());
                all.extend(part);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                vlog!(2, "config: none at {}", path.display())
            }
            Err(e) => eprintln!("rookey: config {}: {e}", path.display()),
        }
    }
    *CONFIG.write().unwrap() = all;
}

/// KEY=value lines. Blank lines and # comments are skipped, one pair of quotes around a value
/// is dropped.
// ponytail: flat env-style file, no toml dependency; move to TOML if settings ever nest.
fn parse_config(text: &str) -> HashMap<String, String> {
    let mut config = HashMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            // not echoing the line: it may be a pasted secret
            eprintln!("rookey: config line {}: expected KEY=value, ignored", n + 1);
            continue;
        };
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|&q| value.strip_prefix(q)?.strip_suffix(q))
            .unwrap_or(value);
        config.insert(key.trim().to_string(), value.to_string());
    }
    config
}

fn main() {
    START.get_or_init(std::time::Instant::now);
    if let Err(e) = cli() {
        eprintln!("rookey: {e}");
        std::process::exit(1);
    }
}

fn cli() -> Res<()> {
    let (mut toggle, mut ui, mut open, mut listen, mut overlay) = (false, false, true, false, false);
    let mut browser = false;
    let args: Vec<String> = env::args().skip(1).collect();
    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            // the rest of the line is status's own flags
            "status" => return status::run(&args[i + 1..]),
            "history" => return history::run(&args[i + 1..]),
            "update" => return update::run(&args[i + 1..]),
            "--version" | "-V" => {
                println!("rookey {}", update::VERSION);
                return Ok(());
            }
            // what the page asks of a fresh process, or of Rookey (see mac.rs)
            #[cfg(target_os = "macos")]
            "__access" => return ui::access_here(),
            #[cfg(target_os = "macos")]
            "__ask" => return ui::ask_here(&args[i + 1..]),
            #[cfg(target_os = "macos")]
            "__capture" => return listen::capture_here(&args[i + 1..]),
            // `just install`: Rookey gets the new build, and its agent restarts on it
            #[cfg(target_os = "macos")]
            "__restart-listen" => return listen::restart(&exe()?).map(drop),
            "overlay" => overlay = true,
            "toggle" => toggle = true,
            "ui" | "setup" => ui = true,
            "listen" => listen = true,
            "--no-open" => open = false, // just print the link, for a browser somewhere else
            "--browser" => browser = true, // the browser, not rookey's own window
            // -v, -vv, -vvv (or repeated -v) raise the level
            v if v.len() > 1 && v.starts_with('-') && v[1..].chars().all(|c| c == 'v') => {
                VERBOSE.fetch_add(v.len() as u8 - 1, std::sync::atomic::Ordering::Relaxed);
            }
            "--verbose" => {
                VERBOSE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            _ => {
                eprintln!(
                    "usage: rookey [-v|-vv|-vvv] [toggle | listen | setup | ui [--no-open|--browser] | status [--json|--waybar] [--follow] | overlay | history [--clear] | update [--check] | --version]"
                );
                std::process::exit(2);
            }
        }
    }
    // Rookey opened by itself (a double-click, or macOS reopening it after a permission
    // changed) has nothing to record for: it shows the settings instead
    #[cfg(target_os = "macos")]
    if args.is_empty() && mac::is_app() {
        ui = true;
    }
    move_from_yap();
    #[cfg(windows)]
    update::clear_old();
    if ui {
        return ui::run(open, browser);
    }
    load_config();
    if overlay {
        return overlay::run();
    }
    if listen {
        return listen::run();
    }

    let pidfile = pidfile();
    if toggle {
        // A recording is running: tell it to stop, it does the rest.
        if let Ok(pid) = fs::read_to_string(&pidfile) {
            #[cfg(unix)]
            let stopped = Command::new("kill").args(["-TERM", pid.trim()]).status()?.success();
            #[cfg(windows)]
            let stopped = pid.trim().parse().is_ok_and(alive) && fs::write(stopfile(), "").is_ok();
            if stopped {
                return Ok(());
            }
            // stale pidfile, fall through and start a new recording
        }
        #[cfg(windows)]
        let _ = fs::remove_file(stopfile());
        fs::write(&pidfile, std::process::id().to_string())?;
    }

    let (stop_tx, stop_rx) = mpsc::channel();
    // Enter stops too: Ctrl-C would also kill the other side of `rookey | wl-copy`.
    if !toggle && std::io::stdin().is_terminal() {
        let tx = stop_tx.clone();
        thread::spawn(move || {
            let _ = std::io::stdin().read_line(&mut String::new());
            let _ = tx.send(());
        });
    }
    #[cfg(windows)]
    if toggle {
        let tx = stop_tx.clone();
        thread::spawn(move || {
            while fs::remove_file(stopfile()).is_err() {
                thread::sleep(Duration::from_millis(50));
            }
            let _ = tx.send(());
        });
    }
    // started from a hotkey with no console, Windows may have nothing to hand a Ctrl-C to
    if let Err(e) = ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    }) {
        vlog!(2, "no Ctrl-C handler: {e}");
    }

    let text = run(if toggle { Mode::Toggle } else { Mode::Terminal }, stop_rx);
    if toggle {
        let _ = fs::remove_file(&pidfile);
    }
    let done = text.and_then(|text| {
        if text.trim().is_empty() {
            return Err("heard no words".into());
        }
        // kept before it is typed: typed into the wrong window, or not at all, it is still here
        history::save(&text);
        if toggle {
            type_text(&text)?;
        } else {
            writeln!(std::io::stdout(), "{text}")?; // println! panics on a closed pipe
        }
        Ok(text)
    });
    finished(if toggle { Mode::Toggle } else { Mode::Terminal }, &done);
    done.map(drop)
}

/// What bars, the pill and the sounds say once a recording is over: the words, or why there
/// are none.
fn finished(mode: Mode, done: &Res<String>) {
    match done {
        Ok(text) => status::typed(text),
        Err(e) => {
            status::failed(&e.to_string());
            if mode != Mode::Terminal && overlay::wanted() {
                overlay::show(); // a hotkey has no terminal: the pill is where the reason shows
            }
        }
    }
    if mode != Mode::Terminal && setting("ROOKEY_QUIET").is_none() {
        sound::play(if done.is_ok() { sound::Cue::Typed } else { sound::Cue::Failed }, setting);
    }
    sound::wait();
}

/// Who asked for the recording, which decides where progress and the text go.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Terminal, // `rookey`: hints and live partials on stderr, the text on stdout
    Toggle,   // `rookey toggle` from a hotkey: desktop notifications, the text typed
    Page,     // the test on the settings page: quiet, the text handed back
}

enum Backend {
    Local(Loader),
    ElevenLabs,
    Realtime(Option<Realtime>), // connected on the first audio tick
}

/// Records until `stop` fires, then transcribes with the engine in the settings.
fn run(mode: Mode, stop_rx: mpsc::Receiver<()>) -> Res<String> {
    let mut backend = match setting("ROOKEY_BACKEND").as_deref() {
        // Load the model while we record, so stopping feels instant.
        None | Some("local") => Backend::Local(spawn_model_loader()?),
        Some("elevenlabs") => Backend::ElevenLabs,
        Some("elevenlabs-realtime") => Backend::Realtime(None),
        Some(b) => {
            return Err(format!(
                "unknown ROOKEY_BACKEND {b:?} (local, elevenlabs, elevenlabs-realtime)"
            )
            .into());
        }
    };
    // Verbose logs every partial as its own line instead of rewriting one.
    let show_partials =
        mode == Mode::Terminal && std::io::stderr().is_terminal() && verbosity() == 0;
    vlog!(2, "backend: {}", setting("ROOKEY_BACKEND").unwrap_or_else(|| "local".into()));
    // Grabbed now, while the window being dictated into is still the one on screen.
    let mut context = spawn_context();

    // Realtime streams each chunk as it's recorded; the others wait for the whole clip.
    let mut held = Vec::new(); // realtime: audio from before the connection
    let (samples, rate) = record_until(stop_rx, mode, |chunk, rate, last| {
        if let Backend::Realtime(rt) = &mut backend {
            held.extend(resample(chunk, rate, WHISPER_RATE));
            if rt.is_none() {
                // The terms go into the URL, so the connection waits for them, but never
                // blocks a tick while recording: that would hold up noticing the stop too.
                if !last && context.as_ref().is_some_and(|c| !c.is_finished()) {
                    return Ok(());
                }
                if last {
                    settle(&mut context);
                }
                // realtime takes up to 50 keyterms of 20 characters
                *rt = Some(Realtime::connect(&key_terms(&mut context, 50, 20))?);
            }
            let rt = rt.as_mut().unwrap();
            rt.send(&std::mem::take(&mut held), false)?;
            rt.poll(show_partials)?;
        }
        Ok(())
    })?;
    settle(&mut context);

    notify(mode, "transcribing");
    let audio = resample(&samples, rate, WHISPER_RATE);
    match backend {
        Backend::Local(loader) => transcribe(
            &loader.join().map_err(|_| "model loader panicked")??,
            &audio,
            // the prompt holds ~224 tokens and an identifier takes several
            &key_terms(&mut context, 30, 49),
        ),
        // up to 1000 keyterms under 50 characters, but past 100 a 20 s minimum is billed
        Backend::ElevenLabs => elevenlabs(&audio, &key_terms(&mut context, 100, 49)),
        Backend::Realtime(Some(rt)) => rt.finish(show_partials),
        Backend::Realtime(None) => Ok(String::new()), // stopped before the first tick
    }
}

type Context = thread::JoinHandle<reader::Context>;

/// Reads the screen, or runs the ROOKEY_CONTEXT command, while we record.
fn spawn_context() -> Option<Context> {
    let source = setting("ROOKEY_CONTEXT")?;
    Some(thread::spawn(move || {
        let read = match source.as_str() {
            "1" | "true" => reader::from_screen(),
            command => reader::from_command(command),
        };
        // Whatever goes wrong here costs the key terms, never the dictation.
        read.unwrap_or_else(|e| {
            eprintln!("rookey: no screen terms this time: {e}");
            Default::default()
        })
    }))
}

/// After the stop the text is waited for, so the screen read gets a little longer to finish
/// and is dropped past that: on a short clip the terms would cost more than they bring.
// ponytail: fixed budget; OCR of a 4K screen takes ~0.7 s here, so holds under ~0.45 s go without terms.
const CONTEXT_BUDGET: Duration = Duration::from_millis(250);

fn settle(context: &mut Option<Context>) {
    let Some(reading) = context else { return };
    let until = Instant::now() + CONTEXT_BUDGET;
    while !reading.is_finished() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    if !reading.is_finished() {
        vlog!(2, "context: not ready {} ms after the stop, going without terms", CONTEXT_BUDGET.as_millis());
        *context = None; // the thread finishes on its own, nobody waits for it
    }
}

/// Waits for the context and gives its key terms, within the engine's limits.
fn context_terms(context: &mut Option<Context>, max: usize, max_len: usize) -> Vec<String> {
    let Some(context) = context.take() else {
        return Vec::new();
    };
    let context = context.join().unwrap_or_default();
    let terms = match context.listed {
        true => listed_terms(&context.text, max, max_len),
        false => keyterms(&context.text, max, max_len),
    };
    vlog!(2, "context: {} terms: {}", terms.len(), terms.join(", "));
    terms
}

/// Your own words (ROOKEY_WORDS, comma-separated) first, then the context's, within the
/// engine's limits. A word longer than the engine takes is left out.
fn key_terms(context: &mut Option<Context>, max: usize, max_len: usize) -> Vec<String> {
    let terms = words_and(&setting("ROOKEY_WORDS").unwrap_or_default(), context_terms(context, max, max_len), max, max_len);
    vlog!(2, "key terms: {}", terms.join(", "));
    terms
}

fn words_and(words: &str, context: Vec<String>, max: usize, max_len: usize) -> Vec<String> {
    let mut terms = listed_terms(&words.replace(',', "\n"), max, max_len);
    for term in context {
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    terms.truncate(max);
    terms
}

/// Terms a model has picked, one per line, cut down to what an engine takes as key terms.
fn listed_terms(text: &str, max: usize, max_len: usize) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for line in text.lines() {
        let term = line.trim().trim_start_matches(['-', '*', '•']).trim();
        let fits = (2..=max_len).contains(&term.chars().count())
            && term.split_whitespace().count() <= 5
            && !term.contains(['<', '>', '{', '}', '[', ']', '\\']);
        if fits && !terms.iter().any(|t| t == term) {
            terms.push(term.to_string());
        }
    }
    terms.truncate(max);
    terms
}

/// Picks the words worth biasing the recognizer towards out of free text: identifiers
/// (snake_case, camelCase, CAPS) first, then Capitalized names, most frequent first.
// ponytail: crude heuristic over OCR output, lowercase jargon is missed; ROOKEY_READER hands
// the picking to a vision model.
fn keyterms(text: &str, max: usize, max_len: usize) -> Vec<String> {
    let mut count: HashMap<&str, usize> = HashMap::new();
    for word in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let word = word.trim_matches('_');
        if (3..=max_len).contains(&word.chars().count())
            && (word.contains('_') || word.chars().any(char::is_uppercase))
        {
            *count.entry(word).or_default() += 1;
        }
    }
    let is_name = |w: &str| !w.contains('_') && !w.chars().skip(1).any(char::is_uppercase);
    let mut terms: Vec<_> = count.into_iter().collect();
    terms.sort_by_key(|&(w, n)| (is_name(w), std::cmp::Reverse(n), w));
    terms.into_iter().take(max).map(|(w, _)| w.to_string()).collect()
}

type Loader = thread::JoinHandle<Result<WhisperContext, whisper_rs::WhisperError>>;

fn spawn_model_loader() -> Res<Loader> {
    let model = setting("ROOKEY_MODEL").map(PathBuf::from).unwrap_or_else(|| {
        dirs::data_dir().unwrap_or_default().join("rookey").join(DEFAULT_MODEL)
    });
    if !model.exists() {
        // nothing is downloaded behind anyone's back: the model is picked, or skipped, in setup
        return Err(format!(
            "no speech model at {}\nrun `rookey setup` to download one, or to use ElevenLabs instead",
            model.display()
        )
        .into());
    }
    whisper_rs::install_logging_hooks(); // silences whisper.cpp stderr spam
    vlog!(2, "whisper: loading {} in background", model.display());
    Ok(thread::spawn(move || {
        let ctx = WhisperContext::new_with_params(&model, WhisperContextParameters::default());
        vlog!(2, "whisper: model loaded");
        ctx
    }))
}

fn elevenlabs_key() -> Res<String> {
    Ok(setting("ELEVENLABS_API_KEY").ok_or("ElevenLabs backends need ELEVENLABS_API_KEY")?)
}

/// The languages in ROOKEY_LANG ("en" or "en,uk"); none means any.
fn languages() -> Vec<String> {
    setting("ROOKEY_LANG")
        .unwrap_or_default()
        .split(',')
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty() && l != "auto")
        .collect()
}

/// The language for the API, None = auto-detect.
// ponytail: ElevenLabs takes one language or none, so with several it detects among all.
fn lang_code() -> Option<String> {
    match &languages()[..] {
        [one] => Some(one.clone()),
        _ => None,
    }
}

/// The likeliest of `allowed`, by whisper's probability for each language code.
fn likeliest(allowed: &[String], prob: impl Fn(&str) -> Option<f32>) -> Option<String> {
    allowed
        .iter()
        .filter_map(|l| Some((l, prob(l)?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(l, _)| l.clone())
}

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// ElevenLabs Scribe realtime: audio streams over a websocket while you talk,
/// so on stop only a commit round-trip is left.
struct Realtime {
    ws: Ws,
    committed: String,
    edited: Option<String>, // Some with ROOKEY_EDIT on: the edited transcript so far
    sent: usize,            // samples streamed so far, for the log
}

impl Realtime {
    fn connect(terms: &[String]) -> Res<Self> {
        use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
        use tungstenite::client::IntoClientRequest;

        let mut url = format!(
            "wss://api.elevenlabs.io/v1/speech-to-text/realtime\
             ?model_id=scribe_v2_realtime&audio_format=pcm_{WHISPER_RATE}"
        );
        if let Some(lang) = lang_code() {
            url += &format!("&language_code={lang}");
        }
        if setting("ROOKEY_SANITIZE").is_some() {
            url += "&no_verbatim=true";
        }
        let edit = setting("ROOKEY_EDIT");
        if let Some(edit) = &edit {
            url += &format!("&transcript_edit={}", utf8_percent_encode(edit, NON_ALPHANUMERIC));
        }
        for term in terms {
            url += &format!("&keyterms={}", utf8_percent_encode(term, NON_ALPHANUMERIC));
        }
        vlog!(2, "ws: connecting {url}");
        let mut req = url.into_client_request()?;
        req.headers_mut().insert("xi-api-key", elevenlabs_key()?.parse()?);
        let (ws, res) = tungstenite::connect(req).map_err(|e| match e {
            tungstenite::Error::Http(res) => format!(
                "elevenlabs realtime {}: {}",
                res.status(),
                String::from_utf8_lossy(res.body().as_deref().unwrap_or_default())
            )
            .into(),
            e => Box::<dyn std::error::Error>::from(e),
        })?;
        vlog!(2, "ws: handshake done (HTTP {}), waiting for session_started", res.status());
        let edited = edit.map(|_| String::new());
        let mut rt = Realtime { ws, committed: String::new(), edited, sent: 0 };

        // The server opens with session_started; anything else (bad key, quota) is an error.
        rt.set_read_timeout(Duration::from_secs(10))?;
        let first = rt.next()?.ok_or("elevenlabs realtime: no session_started")?;
        if first["message_type"] != "session_started" {
            return Err(format!("elevenlabs realtime: {first}").into());
        }
        vlog!(2, "<- session_started {}", first["session_id"]);
        vlog!(3, "   config {}", first["config"]);
        rt.set_read_timeout(Duration::from_millis(1))?; // from here on, reads are polls
        Ok(rt)
    }

    /// Sends 16 kHz mono audio. `commit` asks the server to finalize what it has.
    fn send(&mut self, audio: &[f32], commit: bool) -> Res<()> {
        use base64::Engine;

        let pcm: Vec<u8> = audio
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes())
            .collect();
        let msg = serde_json::json!({
            "message_type": "input_audio_chunk",
            "audio_base_64": base64::engine::general_purpose::STANDARD.encode(pcm),
            "commit": commit,
            "sample_rate": WHISPER_RATE,
        });
        let msg = msg.to_string();
        self.sent += audio.len();
        vlog!(
            3,
            "-> input_audio_chunk {:4} ms audio, {:6} B json, commit={commit} (total {:.2} s)",
            audio.len() * 1000 / WHISPER_RATE as usize,
            msg.len(),
            self.sent as f64 / WHISPER_RATE as f64,
        );
        self.ws.send(tungstenite::Message::text(msg))?;
        Ok(())
    }

    /// Handles whatever the server has sent so far, without blocking.
    fn poll(&mut self, show_partials: bool) -> Res<()> {
        while let Some(msg) = self.next()? {
            self.handle(msg, show_partials)?;
        }
        Ok(())
    }

    /// Commits the rest and waits for the final transcript.
    fn finish(mut self, show_partials: bool) -> Res<String> {
        self.send(&[], true)?;
        let t = std::time::Instant::now();
        vlog!(2, "waiting for committed_transcript...");
        self.set_read_timeout(Duration::from_secs(10))?;
        if let Err(e) = self.wait_final(show_partials) {
            // A failed or slow edit must not cost the dictation: the raw transcript is here.
            if self.committed.trim().is_empty() {
                return Err(e);
            }
            eprintln!("rookey: {e}; using the transcript as it is");
        }
        vlog!(2, "commit round-trip {} ms", t.elapsed().as_millis());
        let _ = self.ws.close(None);
        vlog!(2, "ws: closed");
        if show_partials {
            eprint!("\r\x1b[K"); // clear the partial line
        }
        let text = self.edited.filter(|e| !e.trim().is_empty()).unwrap_or(self.committed);
        Ok(text.trim().to_string())
    }

    fn wait_final(&mut self, show_partials: bool) -> Res<()> {
        loop {
            let msg = self.next()?.ok_or("elevenlabs realtime: timed out waiting for transcript")?;
            if self.handle(msg, show_partials)? {
                return Ok(());
            }
        }
    }

    /// Returns true on the final transcript: the committed one, or its edit with ROOKEY_EDIT on.
    fn handle(&mut self, msg: serde_json::Value, show_partials: bool) -> Res<bool> {
        let text = msg["text"].as_str().unwrap_or_default();
        let kind = msg["message_type"].as_str().unwrap_or_default();
        match kind {
            "partial_transcript" => vlog!(1, "partial:   {text}"),
            "committed_transcript" => vlog!(1, "committed: {text}"),
            "edited_transcript" => vlog!(1, "edited:    {}", msg["edited_text"]),
            _ => vlog!(2, "<- {kind}: {text:?}"),
        }
        match kind {
            "partial_transcript" if show_partials => eprint!("\r\x1b[K{text}"),
            "partial_transcript" | "session_started" => {}
            "committed_transcript" => {
                self.committed.push(' ');
                self.committed.push_str(text);
                // with an edit on, its result follows; nothing said means nothing to edit
                return Ok(self.edited.is_none() || text.trim().is_empty());
            }
            "edited_transcript" => {
                let edited = self.edited.get_or_insert_default();
                edited.push(' ');
                edited.push_str(msg["edited_text"].as_str().unwrap_or(text));
                return Ok(true);
            }
            t if t.contains("error") || msg.get("error").is_some() => {
                return Err(format!("elevenlabs realtime: {msg}").into());
            }
            _ => eprintln!("elevenlabs realtime: unexpected {msg}"),
        }
        Ok(false)
    }

    /// Next JSON message, or None if nothing arrived within the read timeout.
    fn next(&mut self) -> Res<Option<serde_json::Value>> {
        use std::io::ErrorKind::{TimedOut, WouldBlock};
        match self.ws.read() {
            Ok(tungstenite::Message::Text(t)) => Ok(Some(serde_json::from_str(&t)?)),
            Ok(tungstenite::Message::Close(f)) => Err(format!("elevenlabs realtime closed: {f:?}").into()),
            Ok(_) => Ok(None), // ping/pong, answered by tungstenite
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), WouldBlock | TimedOut) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set_read_timeout(&mut self, d: Duration) -> Res<()> {
        use tungstenite::stream::MaybeTlsStream;
        match self.ws.get_mut() {
            MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(d))?,
            MaybeTlsStream::Rustls(s) => s.sock.set_read_timeout(Some(d))?,
            _ => {}
        }
        Ok(())
    }
}

/// ElevenLabs Scribe batch API: upload the whole clip once recording stops.
fn elevenlabs(audio: &[f32], terms: &[String]) -> Res<String> {
    use ureq::unversioned::multipart::{Form, Part};

    let key = elevenlabs_key()?;
    let wav = wav_bytes(audio);

    let mut form = Form::new()
        .text("model_id", "scribe_v2")
        .part("file", Part::bytes(&wav).file_name("rookey.wav"));
    let lang = lang_code();
    if let Some(lang) = &lang {
        form = form.text("language_code", lang);
    }
    if setting("ROOKEY_SANITIZE").is_some() {
        form = form.text("no_verbatim", "true");
    }
    let edit = setting("ROOKEY_EDIT");
    if let Some(edit) = &edit {
        form = form.text("transcript_edit", edit);
    }
    for term in terms {
        form = form.text("keyterms", term);
    }
    vlog!(2, "http: uploading {} KB wav to /v1/speech-to-text", wav.len() / 1024);
    let t = std::time::Instant::now();
    let mut res = ureq::post("https://api.elevenlabs.io/v1/speech-to-text")
        .header("xi-api-key", &key)
        .config()
        .http_status_as_error(false) // keep the error body, it says what went wrong
        .build()
        .send(form)?;
    let body = res.body_mut().read_to_string()?;
    vlog!(2, "http: {} in {} ms", res.status(), t.elapsed().as_millis());
    if !res.status().is_success() {
        return Err(format!("elevenlabs {}: {body}", res.status()).into());
    }
    let json: serde_json::Value = serde_json::from_str(&body)?;
    // The edit comes next to the raw text; a failed one has a message and no edited_text.
    let edited = &json["edited_transcript"];
    if let Some(e) = edited["message"].as_str().filter(|_| edited["edited_text"].is_null()) {
        eprintln!("rookey: transcript edit failed: {e}; using the transcript as it is");
    }
    let text = edited["edited_text"].as_str().or(json["text"].as_str());
    Ok(text.unwrap_or_default().trim().to_string())
}

/// 16-bit PCM mono WAV at WHISPER_RATE.
fn wav_bytes(audio: &[f32]) -> Vec<u8> {
    let data_len = (audio.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend(b"RIFF");
    w.extend((36 + data_len).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes()); // fmt chunk size
    w.extend(1u16.to_le_bytes()); // PCM
    w.extend(1u16.to_le_bytes()); // mono
    w.extend(WHISPER_RATE.to_le_bytes());
    w.extend((WHISPER_RATE * 2).to_le_bytes()); // byte rate
    w.extend(2u16.to_le_bytes()); // block align
    w.extend(16u16.to_le_bytes()); // bits per sample
    w.extend(b"data");
    w.extend(data_len.to_le_bytes());
    for s in audio {
        w.extend(((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    w
}

/// Records mono f32 from the default input until `stop` fires. Returns (samples, sample_rate).
/// `tick` gets each new chunk (~250 ms, at the device rate) while recording, then the tail
/// with `last` set.
fn record_until(
    stop: mpsc::Receiver<()>,
    mode: Mode,
    mut tick: impl FnMut(&[f32], u32, bool) -> Res<()>,
) -> Res<(Vec<f32>, u32)> {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (stream, rate) = open_mic(buf.clone())?;
    notify(mode, "recording");
    if mode != Mode::Page {
        eprintln!("recording... (Enter to stop)");
    }
    let mut sent = 0;
    while let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(Duration::from_millis(250)) {
        let chunk = buf.lock().unwrap()[sent..].to_vec();
        sent += chunk.len();
        // peak level tells you at a glance whether the mic is actually hearing you
        let peak = chunk.iter().fold(0f32, |m, s| m.max(s.abs()));
        vlog!(3, "mic: {:4} ms captured, peak {peak:.3}", chunk.len() * 1000 / rate as usize);
        status::level(peak);
        tick(&chunk, rate, false)?;
    }
    drop(stream);
    vlog!(2, "mic: stopped");

    let samples = std::mem::take(&mut *buf.lock().unwrap());
    if silent(&samples, rate) {
        return Err(SILENT.into());
    }
    tick(&samples[sent..], rate, true)?;
    vlog!(2, "recorded {:.2} s total", samples.len() as f64 / rate as f64);
    Ok((samples, rate))
}

/// Opens the default input and starts it, every sample going into `buf` as mono.
/// Opening it is also what asks for access, where the system asks (macOS, Windows).
fn open_mic(buf: Arc<Mutex<Vec<f32>>>) -> Res<(cpal::Stream, u32)> {
    let device = cpal::default_host().default_input_device().ok_or("no input device")?;
    let config = device.default_input_config()?;
    let rate = config.sample_rate();
    vlog!(
        2,
        "mic: {} ({} Hz, {} ch, {:?})",
        device.id().map(|id| id.to_string()).unwrap_or_else(|_| "?".into()),
        rate,
        config.channels(),
        config.sample_format()
    );
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, config.into(), buf),
        cpal::SampleFormat::I16 => build::<i16>(&device, config.into(), buf),
        cpal::SampleFormat::I32 => build::<i32>(&device, config.into(), buf),
        f => return Err(format!("unsupported sample format {f:?}").into()),
    }?;
    stream.play()?;
    Ok((stream, rate))
}

/// Listens to the default input for `time`: false when it sent nothing but silence.
pub fn mic_hears(time: Duration) -> Res<bool> {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (stream, _) = open_mic(buf.clone())?;
    thread::sleep(time);
    drop(stream);
    let heard = buf.lock().unwrap().iter().any(|s| s.abs() >= SILENCE);
    Ok(heard)
}

/// Below this a sample is silence: a real mic's own hiss is well above it.
const SILENCE: f32 = 1e-4;

/// Kept short: the pill shows one line.
const SILENT: &str = "mic is silent: is it on, unmuted and the default input?";

/// Half a second or more of dead zeros: a headset that is off but whose dongle is plugged
/// in, a muted source. A real mic in a quiet room still hears its own noise floor.
fn silent(samples: &[f32], rate: u32) -> bool {
    samples.len() >= rate as usize / 2 && samples.iter().all(|s| s.abs() < SILENCE)
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    buf: Arc<Mutex<Vec<f32>>>,
) -> Res<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &_| {
            // downmix interleaved frames to mono
            let mut buf = buf.lock().unwrap();
            buf.extend(data.chunks(channels).map(|frame| {
                frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / channels as f32
            }));
        },
        |e| eprintln!("audio error: {e}"),
        None,
    )?;
    Ok(stream)
}

/// Linear-interpolation resampler.
// ponytail: no anti-alias filter, fine for speech into whisper; swap for rubato if quality suffers.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * ratio;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = input[idx];
            let b = *input.get(idx + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

fn transcribe(ctx: &WhisperContext, audio: &[f32], terms: &[String]) -> Res<String> {
    let mut state = ctx.create_state()?;
    let allowed = languages();
    let lang = match &allowed[..] {
        [] => "auto".to_string(),
        [one] => one.clone(),
        // several: whisper guesses over every language, and the likeliest of these wins
        _ => {
            let threads = thread::available_parallelism().map_or(4, |n| n.get().min(8));
            state.pcm_to_mel(audio, threads)?;
            let (_, probs) = state.lang_detect(0, threads)?;
            let pick = likeliest(&allowed, |l| probs.get(whisper_rs::get_lang_id(l)? as usize).copied());
            vlog!(2, "whisper: language {pick:?} of {allowed:?}");
            pick.unwrap_or_else(|| "auto".into())
        }
    };
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(&lang));
    // ponytail: whisper already punctuates and skips most "um"s, so sanitize only mutes
    // non-speech tokens here; ROOKEY_EDIT needs an LLM pass this backend doesn't have.
    params.set_suppress_nst(setting("ROOKEY_SANITIZE").is_some());
    if setting("ROOKEY_EDIT").is_some() {
        vlog!(2, "whisper: ROOKEY_EDIT is ignored by the local backend");
    }
    if !terms.is_empty() {
        params.set_initial_prompt(&format!("Glossary: {}", terms.join(", ")));
    }
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);

    vlog!(2, "whisper: transcribing {:.2} s of audio", audio.len() as f64 / WHISPER_RATE as f64);
    state.full(params, audio)?;
    vlog!(2, "whisper: done");
    let mut text = String::new();
    for seg in state.as_iter() {
        text.push_str(&seg.to_str_lossy()?);
    }
    Ok(text.trim().to_string())
}

/// ROOKEY_KEEP_CLIPBOARD: on unless it is 0 or false, so typing on macOS puts your clipboard back.
#[cfg(target_os = "macos")]
fn keep_clipboard() -> bool {
    let key = "ROOKEY_KEEP_CLIPBOARD";
    let raw = env::var(key).ok().or_else(|| CONFIG.read().unwrap().get(key).cloned());
    !matches!(raw.as_deref(), Some("0" | "false"))
}

/// Types text into the focused window.
fn type_text(text: &str) -> Res<()> {
    if text.is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        // ponytail: paste via clipboard (keystroke mangles non-ASCII). Only text is put back:
        // pbpaste can't read an image or files, those are lost; NSPasteboard would keep them.
        let saved = keep_clipboard()
            .then(|| Command::new("pbpaste").output().ok())
            .flatten()
            .filter(|o| o.status.success() && !o.stdout.is_empty())
            .map(|o| o.stdout);
        let copy = |bytes: &[u8]| -> Res<()> {
            let mut pb = Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn()?;
            pb.stdin.take().unwrap().write_all(bytes)?;
            pb.wait()?;
            Ok(())
        };
        copy(text.as_bytes())?;
        let pasted = Command::new("osascript")
            .args(["-e", r#"tell application "System Events" to keystroke "v" using command down"#])
            .output()?;
        if !pasted.status.success() {
            // the text stays on the clipboard, so it can still be pasted by hand
            let why = String::from_utf8_lossy(&pasted.stderr);
            return Err(format!(
                "macOS didn't let rookey type ({}): allow {} under Privacy & Security > Accessibility and > Automation",
                why.trim(),
                if mac::is_app() { "Rookey" } else { "the app that started it" }
            )
            .into());
        }
        if let Some(saved) = saved {
            // ponytail: the app reads the paste after the keystroke returns, on its own time;
            // 300 ms covers the usual ones, a slow one pastes the old clipboard. Way up: wait
            // on NSPasteboard's changeCount, or a clipboard manager's own API.
            thread::sleep(Duration::from_millis(300));
            copy(&saved)?;
        }
    }
    #[cfg(windows)]
    win::type_text(text)?;
    #[cfg(not(any(target_os = "macos", windows)))]
    Command::new("wtype").args(["--", text]).status()?;
    Ok(())
}

/// Says what's happening: the status file for bars (see status.rs) on every recording; and
/// from a hotkey, which has no terminal, or the page's test, the pill on screen and a sound
/// as the recording starts and ends (a hotkey without the pill gets a desktop notification).
fn notify(mode: Mode, msg: &str) {
    let recording = msg == "recording";
    status::set(if recording { "listening" } else { msg }, serde_json::json!({}));
    if mode == Mode::Terminal {
        return;
    }
    if setting("ROOKEY_QUIET").is_none() {
        sound::play(if recording { sound::Cue::Start } else { sound::Cue::Stop }, setting);
    }
    let pill = overlay::wanted();
    if pill && recording {
        overlay::show();
    }
    // the page's test is watched on the page itself
    if pill || mode == Mode::Page || setting("ROOKEY_NO_NOTIFICATIONS").is_some() {
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = Command::new("osascript")
        .args(["-e", &format!(r#"display notification "{msg}" with title "rookey""#)])
        .status();
    // ponytail: no toast on Windows, the sounds say it; a toast needs an app id registered first
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = Command::new("notify-send").args(["-t", "1500", "rookey", msg]).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs network + key: ELEVENLABS_API_KEY=... cargo test -- --ignored
    /// Uses whisper.cpp's jfk.wav (16 kHz mono 16-bit), fetched to $TMPDIR.
    #[test]
    #[ignore]
    fn elevenlabs_jfk() {
        let text = elevenlabs(&jfk(), &["Americans".into()]).unwrap();
        println!("{text}");
        assert!(text.to_lowercase().contains("your country"), "{text}");
    }

    /// Streams in 250 ms chunks at real-time pace, like the mic loop does.
    #[test]
    #[ignore]
    fn elevenlabs_realtime_jfk() {
        let mut rt = Realtime::connect(&["Americans".into()]).unwrap();
        for chunk in jfk().chunks(WHISPER_RATE as usize / 4) {
            rt.send(chunk, false).unwrap();
            rt.poll(true).unwrap();
            thread::sleep(Duration::from_millis(250));
        }
        let t = std::time::Instant::now();
        let text = rt.finish(false).unwrap();
        println!("{text}\n(commit round-trip {:?})", t.elapsed());
        assert!(text.to_lowercase().contains("your country"), "{text}");
    }

    fn jfk() -> Vec<f32> {
        let path = env::temp_dir().join("jfk.wav");
        if !path.exists() {
            let ok = Command::new("curl")
                .args(["-fsSL", "-o"])
                .arg(&path)
                .arg("https://github.com/ggml-org/whisper.cpp/raw/master/samples/jfk.wav")
                .status()
                .unwrap()
                .success();
            assert!(ok, "download jfk.wav");
        }
        let bytes = fs::read(path).unwrap();
        bytes[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
            .collect()
    }

    #[test]
    fn the_file_on_disk_after_an_update() {
        assert_eq!(on_disk("/home/me/.local/bin/rookey (deleted)".into()), PathBuf::from("/home/me/.local/bin/rookey"));
        assert_eq!(on_disk("/home/me/.local/bin/rookey".into()), PathBuf::from("/home/me/.local/bin/rookey"));
        assert_eq!(on_disk("C:/rookey/rookey.old.exe".into()), PathBuf::from("C:/rookey/rookey.exe"));
    }

    #[test]
    fn config_lines() {
        let config = parse_config(
            "# comment\n\nROOKEY_LANG = uk\nROOKEY_EDIT=\"drop \"um\"\"\nKEY='a=b'\nnonsense\nROOKEY_LANG=en\n",
        );
        assert_eq!(config.len(), 3);
        assert_eq!(config["ROOKEY_LANG"], "en"); // the last one wins
        let probs = |l: &str| match l { "en" => Some(0.2), "uk" => Some(0.5), _ => None };
        assert_eq!(likeliest(&["en".into(), "uk".into(), "xx".into()], probs).as_deref(), Some("uk"));
        assert_eq!(
            without_yap_names("# YAP_X stays\nYAP_LANG=uk\nKEY=YAP_\nYAP_EDIT=x"),
            "# YAP_X stays\nROOKEY_LANG=uk\nKEY=YAP_\nROOKEY_EDIT=x"
        );
        assert_eq!(config["ROOKEY_EDIT"], "drop \"um\"");
        assert_eq!(config["KEY"], "a=b");
    }

    #[test]
    fn words_come_first() {
        let context = vec!["Realtime".to_string(), "rookey".to_string()];
        let terms = words_and(" rookey, Kyiv ,,a-very-long-product-name-here", context, 3, 20);
        assert_eq!(terms, ["rookey", "Kyiv", "Realtime"]);
        assert_eq!(words_and("", vec!["x_y".into()], 3, 20), ["x_y"]);
    }

    #[test]
    fn keyterms_pick_identifiers() {
        let text = "let rt = Realtime::connect()?; the MAX_BACKEND, MAX_BACKEND and \
                    spawn_model_loader(2024) isTerminal x The __ ok";
        let all = ["MAX_BACKEND", "isTerminal", "spawn_model_loader", "Realtime", "The"];
        assert_eq!(keyterms(text, 10, 20), all);
        assert_eq!(keyterms(text, 2, 20), all[..2]);
        assert_eq!(keyterms(text, 10, 11), ["MAX_BACKEND", "isTerminal", "Realtime", "The"]);
    }

    #[test]
    fn silence_is_caught() {
        assert!(silent(&[0.0; 8000], 16000));
        assert!(!silent(&[0.0; 100], 16000)); // too short to tell
        let mut quiet_room = vec![0.0; 8000];
        quiet_room[4000] = 0.002;
        assert!(!silent(&quiet_room, 16000));
    }

    #[test]
    fn wav_header() {
        let w = wav_bytes(&[0.0, 1.0]);
        assert_eq!(w.len(), 48);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[44..], &[0, 0, 0xff, 0x7f]);
    }

    #[test]
    fn resample_48k_to_16k() {
        let input: Vec<f32> = (0..48).map(|i| i as f32).collect();
        let out = resample(&input, 48_000, 16_000);
        assert_eq!(out.len(), 16);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[5], 15.0); // every 3rd sample
        assert_eq!(resample(&input, 16_000, 16_000), input);
    }
}
