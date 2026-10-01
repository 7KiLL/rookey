//! `rookey ui`: settings in the browser. A small HTTP server on localhost serves one embedded
//! page that edits the settings and the keys. Every API call needs the token from the link
//! `rookey ui` opens, so no other page in the browser can reach it, and a saved API key is
//! never sent back.

use crate::t;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use serde_json::{Map, Value, json};

use crate::{DEFAULT_MODEL, Mode, Res, config_path, desktop, keys_path, models, parse_config, reader, sound, update};

/// The settings the page may change. Keys are not among them, they have a file of their own.
const SETTINGS: [&str; 25] = [
    "ROOKEY_BACKEND",
    "ROOKEY_LANG",
    "ROOKEY_MODEL",
    "ROOKEY_SANITIZE",
    "ROOKEY_EDIT",
    "ROOKEY_EDIT_CUSTOM", // the page's own: your instruction, kept while a preset is on
    "ROOKEY_CONTEXT",
    "ROOKEY_READER",
    "ROOKEY_QUIET",
    "ROOKEY_SOUNDS",
    // a file of your own in place of a cue
    "ROOKEY_SOUND_START",
    "ROOKEY_SOUND_STOP",
    "ROOKEY_SOUND_TYPED",
    "ROOKEY_SOUND_FAILED",
    "ROOKEY_NO_NOTIFICATIONS",
    "ROOKEY_NO_OVERLAY",
    "ROOKEY_PILL",           // its look: full, compact or dot
    "ROOKEY_PILL_AT",        // where it goes: "x,y" in percent, empty for the bottom centre
    "ROOKEY_HISTORY",        // 0 keeps no history, empty keeps it
    "ROOKEY_WORDS",          // your own names and jargon, comma-separated
    "ROOKEY_KEEP_CLIPBOARD", // macOS: puts the clipboard back after a paste; on unless 0
    "ROOKEY_AUTOUPDATE",     // installs new releases by itself; on unless 0, which only checks
    // the settings page's own look; empty follows the system and the browser
    "ROOKEY_UI_THEME",
    "ROOKEY_UI_LANG",
    "ROOKEY_UI_CLOSED", // the sections folded shut, comma-separated; empty opens all
];
// kept in the config, not the browser: every `rookey ui` gets a new port, so a new origin
const UI_THEMES: [&str; 2] = ["light", "dark"];
const UI_SECTIONS: [&str; 5] = ["hotkey", "engine", "typed", "talk", "system"]; // app.js Settings()
const BACKENDS: [&str; 3] = ["local", "elevenlabs", "elevenlabs-realtime"];

/// (id, name, the setting its key goes by, where keys are made); what it's for is `provider.<id>`
const PROVIDERS: [(&str, &str, &str, &str); 3] = [
    ("elevenlabs", "ElevenLabs", "ELEVENLABS_API_KEY", "https://elevenlabs.io/app/developers/api-keys"),
    ("openai", "OpenAI", "OPENAI_API_KEY", "https://platform.openai.com/api-keys"),
    ("anthropic", "Claude", "ANTHROPIC_API_KEY", "https://platform.claude.com/settings/keys"),
];
const MAX_REQUEST: u64 = 64 * 1024;
const MAX_EDIT: usize = 2000; // ElevenLabs' limit for transcript_edit
// ElevenLabs takes keyterms under 50 characters and up to 5 words each; past 100 terms it
// bills a 20 s minimum, and realtime takes only 50 of 20 characters (longer ones are left out)
const MAX_WORDS: usize = 100;
const MAX_WORD: usize = 49;
const WOFF2: &str = "font/woff2";

/// (path, content type, body), all baked into the binary: the page looks the same anywhere.
const ASSETS: [(&str, &str, &[u8]); 10] = [
    ("/", "text/html; charset=utf-8", include_bytes!("ui/index.html")),
    ("/app.css", "text/css; charset=utf-8", include_bytes!("ui/app.css")),
    ("/app.js", "text/javascript; charset=utf-8", include_bytes!("ui/app.js")),
    ("/i18n.js", "text/javascript; charset=utf-8", include_bytes!("ui/i18n.js")),
    ("/arrow.js", "text/javascript; charset=utf-8", include_bytes!("ui/arrow.js")),
    ("/icon.svg", "image/svg+xml", include_bytes!("ui/icon.svg")),
    ("/font/commissioner-latin.woff2", WOFF2, include_bytes!("ui/commissioner-latin.woff2")),
    ("/font/commissioner-cyrillic.woff2", WOFF2, include_bytes!("ui/commissioner-cyrillic.woff2")),
    ("/font/martian-mono-latin.woff2", WOFF2, include_bytes!("ui/martian-mono-latin.woff2")),
    ("/font/martian-mono-cyrillic.woff2", WOFF2, include_bytes!("ui/martian-mono-cyrillic.woff2")),
];

/// Open pages. The server stops once the last one is closed.
static PAGES: AtomicUsize = AtomicUsize::new(0);

/// Keys an older rookey saved among the settings move to the keys file, out of ~/.config.
fn move_keys() {
    let settings = read(config_path());
    let (Some(from), Some(to)) = (config_path(), keys_path()) else { return };
    for var in PROVIDERS.iter().map(|p| p.2) {
        let Some(key) = settings.get(var) else { continue };
        let kept = read(Some(to.clone())).get(var).cloned().unwrap_or_else(|| key.clone());
        // written to its new place before it leaves the old one
        let moved =
            write(&to, &[(var.to_string(), kept)]).and_then(|()| write(&from, &[(var.to_string(), String::new())]));
        match moved {
            Ok(()) => eprintln!("rookey ui: moved {var} from {} to {}", tilde(&from), tilde(&to)),
            Err(e) => eprintln!("rookey ui: {var} is still in {}: {e}", tilde(&from)),
        }
    }
}

pub fn run(open: bool, browser: bool) -> Res<()> {
    // now, while the terminal that started this is sure to be among its parents (Rookey needs
    // no looking up)
    let _ = app();
    move_keys();
    models::clear_leftovers();
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let token = token()?;
    let url = format!("http://{}/?t={token}", listener.local_addr()?);
    eprintln!("{}", t!("server.url", url = url));
    eprintln!("{}", t!("server.stops"));
    crate::update::in_background(false);
    if open {
        if let Err(e) = open_browser(&url, browser) {
            eprintln!("{}", t!("server.no-browser", why = e));
        }
    }
    for stream in listener.incoming().flatten() {
        let token = token.clone();
        thread::spawn(move || {
            if let Err(e) = serve(&stream, &token) {
                vlog!(2, "ui: {e}");
            }
        });
    }
    Ok(())
}

fn token() -> Res<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness for the page's token: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The page in rookey's own window (`rookey-window`, next to this binary), or in the default
/// browser when that is missing, can't start, or `browser` asks for it.
fn open_browser(url: &str, browser: bool) -> std::io::Result<()> {
    if !browser {
        // Rookey is a copy on its own: its window is next to the rookey it came from
        let helper =
            crate::installed()?.with_file_name(if cfg!(windows) { "rookey-window.exe" } else { "rookey-window" });
        let mut cmd = Command::new(&helper);
        cmd.arg(url).stdin(Stdio::null()).stdout(Stdio::null());
        // WebKitGTK's DMABUF renderer flickers on scroll with NVIDIA's own driver; a value the
        // user set wins
        const DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
        if cfg!(target_os = "linux") && Path::new("/proc/driver/nvidia").exists() && env::var_os(DMABUF).is_none() {
            cmd.env(DMABUF, "1");
        }
        if crate::verbosity() < 2 {
            cmd.stderr(Stdio::null());
        }
        match cmd.spawn().and_then(|child| started(child, Duration::from_secs(1))) {
            Ok(()) => return Ok(()),
            Err(e) => vlog!(1, "ui: no window ({}: {e}), opening the browser", helper.display()),
        }
    }
    // rundll32 rather than `cmd /c start`, which would split the link at its &
    let (opener, args): (&str, &[&str]) = if cfg!(windows) {
        ("rundll32", &["url.dll,FileProtocolHandler", url])
    } else if cfg!(target_os = "macos") {
        ("open", &[url])
    } else {
        ("xdg-open", &[url])
    };
    let quiet = Stdio::null;
    Command::new(opener).args(args).stdin(quiet()).stdout(quiet()).stderr(quiet()).spawn().map(drop)
}

/// A window that fails does so at once (no WebKitGTK, no WebView2): one still running after
/// `wait`, or that ended well, is taken as open.
fn started(mut child: std::process::Child, wait: Duration) -> std::io::Result<()> {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        match child.try_wait()? {
            Some(status) if !status.success() => return Err(std::io::Error::other(format!("it ended with {status}"))),
            Some(_) => return Ok(()),
            None => thread::sleep(Duration::from_millis(50)),
        }
    }
    Ok(())
}

struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

// ponytail: one request per connection and only what the page sends; take tiny_http if
// this ever has to speak more HTTP than that.
fn read_request(stream: &TcpStream) -> Res<Request> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err("not an HTTP request".into());
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim_end().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse()?;
            }
        }
    }
    if length as u64 > MAX_REQUEST {
        return Err("request too large".into());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request { method: method.into(), path: path.into(), query: query.into(), body })
}

fn serve(stream: &TcpStream, token: &str) -> Res<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let req = read_request(stream)?;
    if req.method == "GET" {
        if let Some((_, kind, body)) = ASSETS.iter().find(|a| a.0 == req.path) {
            return respond(stream, "200 OK", kind, body);
        }
        if req.path == "/locales.json" {
            return respond(stream, "200 OK", "application/json; charset=utf-8", crate::i18n::all_json().as_bytes());
        }
    }
    if !req.path.starts_with("/api/") {
        return respond(stream, "404 Not Found", "text/plain", b"not found");
    }
    let sent = req.query.split('&').find_map(|p| p.strip_prefix("t=")).unwrap_or_default();
    if !same(sent, token) {
        return error(stream, "403 Forbidden", &t!("server.expired"));
    }
    let done = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/alive") => return alive(stream),
        ("GET", "/api/state") => Ok(state_after_listening()),
        // what changes by itself, asked for often while it does
        ("GET", "/api/history") => Ok(history()),
        ("GET", "/api/progress") => {
            Ok(json!({ "download": models::download_state(), "trial": trial(), "update": update::state() }))
        }
        ("GET", "/api/update") => Ok(update::state()),
        ("POST", "/api/save") => match changes(&req.body) {
            Ok(changes) => config_path()
                .ok_or_else(|| t!("server.no-settings-file").into())
                .and_then(|path| write(&path, &changes))
                .map(|()| state()),
            Err(e) => return error(stream, "400 Bad Request", &e.to_string()),
        },
        ("POST", path) => match serde_json::from_slice::<Value>(&req.body) {
            Ok(asked) => match path {
                "/api/key" => save_key(&asked).map(|()| state()),
                "/api/model" => model(&asked).map(|()| state()),
                "/api/hotkey" => hotkey(&asked),
                "/api/try" => try_it(&asked).map(|()| json!({ "trial": trial() })),
                "/api/sound" => play(&asked).map(|()| json!({})),
                "/api/open" => open_pane(&asked).map(|()| json!({})),
                "/api/update" if asked["check"] == true || asked["install"] == true => {
                    update::start(asked["install"] == true).map(|()| update::state())
                }
                "/api/history" if asked["clear"] == true => crate::history::path()
                    .map_or(Ok(()), |p| crate::history::clear(&p))
                    .map_err(|e| t!("server.history-clear", why = e).into())
                    .map(|()| history()),
                _ => return error(stream, "404 Not Found", &t!("server.not-found")),
            },
            Err(e) => return error(stream, "400 Bad Request", &e.to_string()),
        },
        _ => return error(stream, "404 Not Found", &t!("server.not-found")),
    };
    match done {
        Ok(body) => respond_json(stream, "200 OK", &body),
        // asked for properly, and it didn't work out: the message says why
        Err(e) => error(stream, "422 Unprocessable Content", &e.to_string()),
    }
}

/// Plays a cue the way a recording would, with the settings as saved.
fn play(asked: &Value) -> Res<()> {
    let name = asked["cue"].as_str().unwrap_or_default();
    let cue = sound::Cue::ALL.into_iter().find(|c| c.name() == name).ok_or_else(|| t!("server.no-sound"))?;
    let settings = read(config_path());
    let get = |k: &str| env::var(k).ok().or_else(|| settings.get(k).cloned()).filter(|v| !v.is_empty());
    sound::play(cue, get);
    sound::wait();
    Ok(())
}

/// The transcripts kept on this computer, newest first, and where they are.
fn history() -> Value {
    let path = crate::history::path();
    let mut entries = path.as_deref().map(crate::history::read).unwrap_or_default();
    entries.reverse();
    json!({ "entries": entries, "path": path.as_deref().map(tilde), "keep": crate::history::KEEP })
}

/// Compares in constant time, so the token can't be guessed a character at a time.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

fn respond(mut stream: &TcpStream, status: &str, kind: &str, body: &[u8]) -> Res<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\n\
         Content-Type: {kind}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'self'; frame-ancestors 'none'; \
         base-uri 'none'; form-action 'none'\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}

fn respond_json(stream: &TcpStream, status: &str, body: &Value) -> Res<()> {
    respond(stream, status, "application/json", body.to_string().as_bytes())
}

fn error(stream: &TcpStream, status: &str, message: &str) -> Res<()> {
    respond_json(stream, status, &json!({ "error": message }))
}

/// Held open by the page for as long as it lives: when the last one goes, so does the server.
fn alive(mut stream: &TcpStream) -> Res<()> {
    PAGES.fetch_add(1, Ordering::SeqCst);
    let held = (|| -> Res<()> {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
             Cache-Control: no-store\r\n\r\n: rookey\n\n"
        )?;
        stream.set_read_timeout(None)?;
        // the page never sends anything here, so the read returns when it hangs up
        while stream.read(&mut [0; 64])? > 0 {}
        Ok(())
    })();
    if PAGES.fetch_sub(1, Ordering::SeqCst) == 1 {
        thread::sleep(Duration::from_secs(3)); // a reload comes back within this
        if PAGES.load(Ordering::SeqCst) == 0 {
            eprintln!("{}", t!("server.closed"));
            std::process::exit(0);
        }
    }
    held
}

fn read(path: Option<std::path::PathBuf>) -> std::collections::HashMap<String, String> {
    path.and_then(|p| fs::read_to_string(p).ok()).map(|text| parse_config(&text)).unwrap_or_default()
}

/// Whether the mic sent any sound when last listened to; None before the first time or when
/// it couldn't be opened (the mic check says why then).
static HEARD: Mutex<Option<bool>> = Mutex::new(None);

/// The state, after half a second from the mic: what the page asks for when it opens and
/// on "Check again". Saves answer with `state()` and keep the last result.
fn state_after_listening() -> Value {
    let heard = default_mic().0 && crate::mic_hears(Duration::from_millis(500)).unwrap_or(false);
    *HEARD.lock().unwrap() = Some(heard);
    state()
}

/// Everything the page shows. Of an API key only the last characters leave this process.
fn state() -> Value {
    let settings = read(config_path());
    let mut keys = read(keys_path());
    // a key saved before keys had their own file still counts
    for (name, value) in &settings {
        keys.entry(name.clone()).or_insert_with(|| value.clone());
    }
    let get = |key: &str| settings.get(key).cloned().unwrap_or_default();

    let values: Map<String, Value> = SETTINGS.iter().map(|&k| (k.to_string(), get(k).into())).collect();
    let from_env: Map<String, Value> =
        SETTINGS.iter().filter_map(|&k| Some((k.to_string(), env::var(k).ok()?.into()))).collect();

    let providers: Vec<Value> = PROVIDERS
        .iter()
        .map(|&(id, name, var, site)| {
            let key = keys.get(var).cloned().unwrap_or_default();
            let hint = masked(&key);
            json!({
                "id": id, "name": name, "var": var, "does": t!(&format!("provider.{id}")), "site": site,
                "saved": !key.is_empty(), "hint": hint, "env": env::var_os(var).is_some(),
                // still in the settings file, where dotfile managers would pick it up
                "exposed": settings.contains_key(var),
            })
        })
        .collect();

    let default_model = models::dir().join(DEFAULT_MODEL);
    let model = env::var("ROOKEY_MODEL")
        .ok()
        .or_else(|| settings.get("ROOKEY_MODEL").cloned())
        .filter(|m| !m.is_empty())
        .map_or_else(|| default_model.clone(), PathBuf::from);
    let installed = models::installed(Some(&model));
    let known = |path: &Path| {
        let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let named = models::CATALOG.iter().find(|k| k.0 == file).map(|k| k.1);
        (file, named)
    };
    let on_disk: Vec<Value> = installed
        .iter()
        .map(|path| {
            let (file, name) = known(path);
            let mb = fs::metadata(path).map(|m| m.len() / (1024 * 1024)).unwrap_or(0);
            json!({
                "path": path, "shown": tilde(path), "file": file, "name": name, "mb": mb,
                "default": *path == default_model,
            })
        })
        .collect();
    let catalog: Vec<Value> = models::CATALOG
        .iter()
        .map(|&(file, name, mb, about)| {
            json!({
                "file": file, "name": name, "mb": mb, "about": about,
                "installed": models::dir().join(file).is_file(),
            })
        })
        .collect();

    json!({
        "path": config_path().as_deref().map(tilde),
        "keys_path": keys_path().as_deref().map(tilde),
        "values": values,
        "env": from_env,
        "providers": providers,
        "models": {
            "dir": tilde(&models::dir()),
            "in_use": model,
            "found": model.is_file(),
            "installed": on_disk,
            "catalog": catalog,
            "download": models::download_state(),
        },
        "tools": { "grim": on_path("grim"), "tesseract": on_path("tesseract") },
        "checks": checks(
            &get,
            default_mic(),
            *HEARD.lock().unwrap(),
            model.is_file(),
            keys.get("ELEVENLABS_API_KEY").is_some_and(|k| !k.is_empty()) || env::var_os("ELEVENLABS_API_KEY").is_some(),
            &access(),
        ),
        // the app macOS asks about, when it can be named
        "app": app(),
        // Rookey on macOS: the app that records when the hotkey does, with words of its own
        "rookey": cfg!(target_os = "macos") && rookey_app(),
        "hotkey": desktop::hotkey(),
        "listen": listening(&get),
        "trial": trial(),
        "update": update::state(),
        "os": env::consts::OS,
    })
}

/// A key the way the page shows it: its kind (the "sk_" every such key starts with), dots,
/// and the last four. Empty for a key too short to show that much of.
fn masked(key: &str) -> String {
    let n = key.chars().count();
    if n < 12 {
        return String::new();
    }
    // the prefix up to the first _ or -, if there is a short one: it names the kind, not the key
    let prefix = key
        .char_indices()
        .take(5)
        .find(|&(_, c)| c == '_' || c == '-')
        .filter(|&(i, _)| key[..i].chars().all(|c| c.is_ascii_alphabetic()))
        .map_or("", |(i, _)| &key[..=i]);
    let last: String = key.chars().skip(n - 4).collect();
    format!("{prefix}••••••••{last}")
}

/// What the system lets the app responsible for rookey do; None where it can't tell or
/// doesn't ask. Only macOS asks (see mac.rs).
#[derive(Default)]
struct Access {
    mic: Option<bool>,
    screen: Option<bool>,
    typing: Option<bool>,
    automation: Option<bool>,
}

/// What macOS lets this process do, as JSON on stdout: `rookey __access`.
#[cfg(target_os = "macos")]
pub fn access_here() -> Res<()> {
    use crate::mac;
    let answer = json!({
        "mic": mac::mic(), "screen": mac::screen(), "typing": mac::typing(),
        "automation": mac::automation(),
    });
    mac::answer(&answer.to_string())
}

/// Asks macOS for one permission from this process: `rookey __ask <what>`, run as Rookey.
#[cfg(target_os = "macos")]
pub fn ask_here(args: &[String]) -> Res<()> {
    use crate::mac;
    match args.first().map(String::as_str) {
        Some("mic-privacy") => drop(crate::mic_hears(Duration::from_millis(300))),
        Some("screen-privacy") => mac::ask_screen(),
        Some("accessibility") => mac::ask_typing(),
        // any event at all brings up the question, and this one changes nothing
        Some("automation") => {
            drop(Command::new("osascript").args(["-e", r#"tell application "System Events" to get name"#]).output())
        }
        _ => return Err("ask for what?".into()),
    }
    Ok(())
}

/// macOS answers Accessibility once per process and keeps that answer, so a check again from
/// the same process never turns green. A child asks afresh every time, and answers for the
/// same app as this process: Rookey, when the page runs as Rookey.
#[cfg(target_os = "macos")]
fn access() -> Access {
    use crate::mac;
    let printed = crate::exe()
        .and_then(|exe| Command::new(exe).arg("__access").stdin(Stdio::null()).stderr(Stdio::null()).output())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
    let answer: Value = match printed.map(|p| serde_json::from_str(p.trim())) {
        Ok(Ok(answer)) => answer,
        _ => {
            vlog!(1, "ui: no fresh answer about the permissions, using this process's");
            json!({ "mic": mac::mic(), "screen": mac::screen(), "typing": mac::typing(), "automation": mac::automation() })
        }
    };
    Access {
        mic: answer["mic"].as_bool(),
        screen: answer["screen"].as_bool(),
        typing: answer["typing"].as_bool(),
        automation: answer["automation"].as_bool(),
    }
}

#[cfg(not(target_os = "macos"))]
fn access() -> Access {
    Access::default()
}

/// The settings pane /api/open offers for a failing check, where this system has one.
fn opens(id: &str, os: &str) -> Option<&'static str> {
    Some(match id {
        "mic" => "sound",
        // Windows' privacy switch hands apps silence rather than an error
        "mic-silent" if os == "windows" => "mic-privacy",
        "mic-silent" => "sound",
        "mic-access" => "mic-privacy",
        "screen-access" => "screen-privacy",
        "typing-access" => "accessibility",
        "automation" => "automation",
        _ => return None,
    })
}

/// What `/api/open` may open, by name: a fixed program and arguments per system. Nothing in
/// the request reaches the command line but the name picking one of these.
fn pane(what: &str, os: &str, on_path: &dyn Fn(&str) -> bool) -> Option<(&'static str, &'static [&'static str])> {
    Some(match (os, what) {
        // the older pane ids, which System Settings still answers to since Ventura
        ("macos", "mic-privacy") => {
            ("open", &["x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"])
        }
        ("macos", "screen-privacy") => {
            ("open", &["x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"])
        }
        ("macos", "accessibility") => {
            ("open", &["x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"])
        }
        ("macos", "automation") => {
            ("open", &["x-apple.systempreferences:com.apple.preference.security?Privacy_Automation"])
        }
        ("macos", "sound") => ("open", &["x-apple.systempreferences:com.apple.Sound-Settings.extension"]),
        ("windows", "mic-privacy") => ("explorer", &["ms-settings:privacy-microphone"]),
        ("windows", "sound") => ("explorer", &["ms-settings:sound"]),
        // ponytail: the usual mixers and GNOME; other desktops get the words and no button
        ("linux", "sound") => [("pavucontrol", &[][..]), ("pwvucontrol", &[]), ("gnome-control-center", &["sound"])]
            .into_iter()
            .find(|(program, _)| on_path(program))?,
        _ => return None,
    })
}

/// Opens the settings pane a check asked for. On macOS it asks for the permission first:
/// that puts the app in the pane's list, where the switch is.
fn open_pane(asked: &Value) -> Res<()> {
    let what = asked["what"].as_str().unwrap_or_default();
    let (program, args) = pane(what, env::consts::OS, &on_path).ok_or_else(|| t!("server.no-pane"))?;
    #[cfg(target_os = "macos")]
    match what {
        "screen-privacy" => crate::mac::ask_screen(),
        "accessibility" => crate::mac::ask_typing(),
        _ => {}
    }
    let quiet = Stdio::null;
    Command::new(program).args(args).stdin(quiet()).stdout(quiet()).stderr(quiet()).spawn()?;
    Ok(())
}

/// The app macOS files permissions under: the first app bundle among rookey's parents, by
/// `ps -A -o pid=,ppid=,comm=`. None when there is none, over ssh or in tmux.
// ponytail: tmux's server hangs off launchd, so the terminal behind it goes unnamed and the
// page says "the app you started rookey ui from"; the way up is responsibility_get_pid_responsible_for_pid.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn responsible_app(table: &str, mut pid: u32) -> Option<String> {
    let rows: Vec<(u32, u32, &str)> = table
        .lines()
        .filter_map(|line| {
            // the name may have spaces in it, the numbers never do
            let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
            let (ppid, comm) = rest.trim_start().split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, ppid.parse().ok()?, comm.trim()))
        })
        .collect();
    for _ in 0..32 {
        let &(_, ppid, comm) = rows.iter().find(|r| r.0 == pid)?;
        // the outermost bundle: a helper inside Visual Studio Code.app is Visual Studio Code
        if let Some(at) = comm.find(".app/") {
            let name = comm[..at].rsplit('/').next()?;
            return (!name.is_empty()).then(|| name.to_string());
        }
        if ppid <= 1 {
            return None;
        }
        pid = ppid;
    }
    None
}

#[cfg(target_os = "macos")]
fn app() -> Option<String> {
    if crate::mac::is_app() {
        return Some("Rookey".into());
    }
    static APP: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    APP.get_or_init(|| {
        let out = Command::new("ps").args(["-A", "-o", "pid=,ppid=,comm="]).output().ok()?;
        responsible_app(&String::from_utf8_lossy(&out.stdout), std::process::id())
    })
    .clone()
}

#[cfg(not(target_os = "macos"))]
fn app() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn rookey_app() -> bool {
    crate::mac::is_app()
}

#[cfg(not(target_os = "macos"))]
fn rookey_app() -> bool {
    false
}

/// What recording needs and is missing on this machine, for the settings as they are, with
/// the command that installs it, or the settings pane that turns it on.
/// `mic` is `default_mic()`'s answer, passed in: tests never touch the machine's audio.
fn checks(
    get: &dyn Fn(&str) -> String,
    mic: (bool, Option<String>),
    heard: Option<bool>,
    model_found: bool,
    has_key: bool,
    access: &Access,
) -> Vec<Value> {
    let mut checks = Vec::new();
    let mut check = |id: &str, ok: bool, title: &str, missing: &str, fix: Option<String>| {
        let open = opens(id, env::consts::OS).filter(|what| pane(what, env::consts::OS, &on_path).is_some());
        checks.push(json!({
            "id": id, "ok": ok, "title": title, "missing": missing,
            "fix": if ok { None } else { fix },
            "open": if ok { None } else { open },
        }));
    };
    let (mic, name) = mic;
    // the page's own words, filled in here as plain text: {app} and {engine} are links there
    let app = t!("check.app.unknown");
    let engine = t!("engine.title");
    let title = name.map_or(t!("check.mic.title"), |n| t!("check.mic.named", name = n));
    check("mic", mic, &title, &t!("check.mic.missing"), None);
    let allowed = access.mic != Some(false);
    if !allowed {
        check("mic-access", false, &t!("check.mic-access.title"), &t!("check.mic-access.missing", app = app), None);
    }
    // A wireless headset that is off still has its dongle plugged in: only listening tells.
    // ponytail: a headset with a noise gate sends exact zeros while you're quiet too; the
    // advice says to speak and check again rather than guessing which one it is
    if mic && allowed && heard == Some(false) {
        check("mic-silent", false, &t!("check.mic-silent.title"), &t!("check.mic-silent.missing"), None);
    }

    let backend = env::var("ROOKEY_BACKEND").unwrap_or_else(|_| get("ROOKEY_BACKEND"));
    if backend.is_empty() || backend == "local" {
        check("model", model_found, &t!("check.model.title"), &t!("check.model.missing"), None);
    } else {
        check("key", has_key, &t!("check.key.title"), &t!("check.key.missing", engine = engine), None);
    }
    if cfg!(target_os = "linux") {
        check("wtype", on_path("wtype"), &t!("check.wtype.title"), &t!("check.wtype.missing"), install("wtype"));
    }
    // macOS pastes through System Events: Accessibility for the keys, Automation for the events
    if let Some(ok) = access.typing {
        check(
            "typing-access",
            ok,
            &t!("check.typing-access.title"),
            &t!("check.typing-access.missing", app = app),
            None,
        );
    }
    if access.automation == Some(false) {
        check("automation", false, &t!("check.automation.title"), &t!("check.automation.missing", app = app), None);
    }
    let context = env::var("ROOKEY_CONTEXT").unwrap_or_else(|_| get("ROOKEY_CONTEXT"));
    if context == "1" || context == "true" {
        if let Some(ok) = access.screen {
            check(
                "screen-access",
                ok,
                &t!("check.screen-access.title"),
                &t!("check.screen-access.missing", app = app),
                None,
            );
        }
        let reader = env::var("ROOKEY_READER").unwrap_or_else(|_| get("ROOKEY_READER"));
        let ocr = reader.is_empty() || reader == "ocr";
        // macOS has screencapture built in; Windows has ROOKEY_SCREENSHOT, and no tesseract
        // on PATH by that name
        let mut tools = if cfg!(target_os = "linux") { vec!["grim"] } else { vec![] };
        if ocr && cfg!(any(target_os = "linux", target_os = "macos")) {
            tools.push("tesseract");
        }
        let missing: Vec<&str> = tools.iter().copied().filter(|t| !on_path(t)).collect();
        // the picked languages tesseract has no pack for: their text is read as junk
        let picked = crate::language_list(&env::var("ROOKEY_LANG").unwrap_or_else(|_| get("ROOKEY_LANG")));
        let installed = if ocr && !picked.is_empty() { reader::installed_langs() } else { vec![] };
        let (_, mut no_pack) = reader::ocr_langs(&picked, &installed);
        // no tesseract on Windows: nothing to add packs to, and no row for it there
        if !ocr || (cfg!(windows) && installed.is_empty()) {
            no_pack.clear();
        }
        if tools.is_empty() && no_pack.is_empty() {
            return checks;
        }
        let packs: Vec<&str> = no_pack.iter().filter_map(|l| reader::tesseract_lang(l)).collect();
        let mut needed: Vec<Option<String>> = missing.iter().map(|t| packages(t)).collect();
        if !packs.is_empty() {
            needed.push(installer().and_then(|i| lang_packages(i, &packs)));
        }
        let fix = needed
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .and_then(|p| Some(format!("{} {}", installer()?, p.join(" "))));
        let mut words = Vec::new();
        if !missing.is_empty() {
            words.push(t!("check.screen.missing", tools = missing.join(", ")));
        }
        if !no_pack.is_empty() {
            words.push(t!("check.screen.langs", langs = no_pack.join(", ")));
            if cfg!(windows) {
                words.push(t!("check.screen.langs.windows"));
            }
        }
        check(
            "screen",
            // a missing language pack is optional: the page notes it under the switch, not as a failure
            missing.is_empty(),
            &t!("check.screen.title"),
            &words.join(" "),
            fix.clone(),
        );
        // by name too, for the page to word in its own language
        if let Some(last) = checks.last_mut() {
            last["tools"] = json!(missing);
            last["langs"] = json!(no_pack);
            last["fix"] = json!(fix); // kept when only packs are missing: the page offers it under the switch
        }
    }
    checks
}

/// The package manager's install command here, by /etc/os-release. None where it's unknown.
// ponytail: the three big families and Homebrew; others get the tool's name and no command.
fn installer() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        return Some("brew install");
    }
    let release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let ids: String = release
        .lines()
        .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
        .collect::<Vec<_>>()
        .join(" ");
    let family = |name: &str| ids.split(|c: char| c == ' ' || c == '"').any(|id| id == name);
    if family("arch") {
        Some("sudo pacman -S --needed")
    } else if family("debian") || family("ubuntu") {
        Some("sudo apt install")
    } else if family("fedora") {
        Some("sudo dnf install")
    } else {
        None
    }
}

/// What a tool's package is called by the installer above.
fn packages(tool: &str) -> Option<String> {
    let debian = installer() == Some("sudo apt install");
    Some(match tool {
        "tesseract" if installer() == Some("sudo pacman -S --needed") => "tesseract tesseract-data-eng".into(),
        "tesseract" if debian => "tesseract-ocr".into(),
        "wtype" | "grim" | "tesseract" => tool.into(),
        _ => return None,
    })
}

/// The packages of tesseract's language packs (its own names: ukr, chi_sim), by the installer
/// above. Homebrew has them all in one.
fn lang_packages(installer: &str, packs: &[&str]) -> Option<String> {
    let each = |prefix: &str| packs.iter().map(|p| format!("{prefix}{p}")).collect::<Vec<_>>().join(" ");
    Some(match installer {
        "brew install" => "tesseract-lang".into(),
        "sudo pacman -S --needed" => each("tesseract-data-"),
        // Debian writes chi_sim as chi-sim
        "sudo apt install" => each("tesseract-ocr-").replace('_', "-"),
        "sudo dnf install" => each("tesseract-langpack-"),
        _ => return None,
    })
}

fn install(tool: &str) -> Option<String> {
    Some(format!("{} {}", installer()?, packages(tool)?))
}

/// A path the way people write it, with ~ for the home directory.
pub fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => Path::new("~").join(rest).display().to_string(),
        None => path.display().to_string(),
    }
}

/// The default input as the sound server sees it: whether it is a real, plugged-in microphone,
/// and its name. A wireless headset's dongle counts as plugged in with no mic on the headset;
/// the sound server can't tell.
// ponytail: asks pactl (PulseAudio or pipewire-pulse); without it, any input device counts.
fn default_mic() -> (bool, Option<String>) {
    let run = |args: &[&str]| {
        std::process::Command::new("pactl").args(args).output().ok().filter(|o| o.status.success()).map(|o| o.stdout)
    };
    let (Some(default), Some(list)) = (run(&["get-default-source"]), run(&["--format=json", "list", "sources"])) else {
        return (cpal::traits::HostTrait::default_input_device(&cpal::default_host()).is_some(), None);
    };
    let default = String::from_utf8_lossy(&default).trim().to_string();
    let sources: Vec<Value> = serde_json::from_slice(&list).unwrap_or_default();
    let Some(source) = sources.iter().find(|s| s["name"] == default.as_str()) else { return (false, None) };
    let port = source["active_port"].as_str();
    let unplugged = source["ports"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|p| p["name"].as_str() == port && p["availability"] == "not available");
    let ok = !default.ends_with(".monitor") && !unplugged;
    (ok, source["description"].as_str().map(str::to_string))
}

fn on_path(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d.join(program).is_file()))
}

/// Checks what the page sent: known settings only, one line each. An empty value unsets.
fn changes(body: &[u8]) -> Res<Vec<(String, String)>> {
    let sent: Map<String, Value> = serde_json::from_slice(body)?;
    let mut changes = Vec::new();
    for (key, value) in sent {
        if !SETTINGS.contains(&key.as_str()) {
            return Err(t!("server.not-setting", key = key).into());
        }
        let value = value.as_str().ok_or_else(|| t!("server.text"))?;
        // the file is one setting per line
        let value = value.replace(|c: char| c.is_control(), " ");
        let mut value = value.trim().to_string();
        match key.as_str() {
            "ROOKEY_BACKEND" if !value.is_empty() && !BACKENDS.contains(&value.as_str()) => {
                return Err(t!("server.no-engine", value = value).into());
            }
            "ROOKEY_READER" if !value.is_empty() && !reader::READERS.contains(&value.as_str()) => {
                return Err(t!("server.no-reader", value = value).into());
            }
            // one language, or several to choose between: "en" or "en,uk"
            "ROOKEY_LANG" => {
                value = value
                    .to_lowercase()
                    .split(',')
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .collect::<Vec<_>>()
                    .join(",");
                let code = |c: &str| (2..=3).contains(&c.len()) && c.chars().all(|c| c.is_ascii_lowercase());
                if !value.split(',').all(|c| c.is_empty() || c == "auto" || code(c)) {
                    return Err(t!("server.lang-code", value = value).into());
                }
            }
            // switches: on or unset, nothing else
            "ROOKEY_QUIET" | "ROOKEY_NO_NOTIFICATIONS" | "ROOKEY_NO_OVERLAY" if !matches!(value.as_str(), "" | "1") => {
                return Err(t!("server.switch", key = key).into());
            }
            "ROOKEY_HISTORY" if !matches!(value.as_str(), "" | "0") => {
                return Err(t!("server.history").into());
            }
            "ROOKEY_PILL" if !value.is_empty() && !crate::overlay::STYLES.contains(&value.as_str()) => {
                return Err(t!("server.pill-style", styles = crate::overlay::STYLES.join(", "), value = value).into());
            }
            "ROOKEY_PILL_AT" if !value.is_empty() && crate::overlay::parse_at(&value).is_none() => {
                return Err(t!("server.pill-at").into());
            }
            "ROOKEY_SOUNDS" if !value.is_empty() && !sound::SETS.contains(&value.as_str()) => {
                return Err(t!("server.sounds", value = value, sets = sound::SETS.join(", ")).into());
            }
            key if key.starts_with("ROOKEY_SOUND_") && !value.is_empty() => {
                if let Some(rest) = value.strip_prefix("~/") {
                    let home = dirs::home_dir().ok_or_else(|| t!("server.home"))?;
                    value = home.join(rest).display().to_string();
                }
                if !Path::new(&value).is_file() {
                    return Err(t!("server.no-sound-file", value = value).into());
                }
            }
            "ROOKEY_UI_THEME" if !value.is_empty() && !UI_THEMES.contains(&value.as_str()) => {
                return Err(t!("server.theme", value = value).into());
            }
            "ROOKEY_UI_LANG" if !value.is_empty() && !crate::i18n::known(&value) => {
                return Err(t!("server.ui-lang", value = value).into());
            }
            "ROOKEY_UI_CLOSED" if value.split(',').any(|id| !id.is_empty() && !UI_SECTIONS.contains(&id)) => {
                return Err(t!("server.sections", value = value).into());
            }
            "ROOKEY_EDIT" | "ROOKEY_EDIT_CUSTOM" if value.chars().count() > MAX_EDIT => {
                return Err(t!("server.edit-long", n = value.chars().count(), max = MAX_EDIT).into());
            }
            "ROOKEY_WORDS" => {
                let mut words: Vec<&str> = Vec::new();
                for word in value.split(',').map(str::trim).filter(|w| !w.is_empty()) {
                    if word.chars().count() > MAX_WORD || word.split_whitespace().count() > 5 {
                        return Err(t!("server.word-long", word = word, max = MAX_WORD).into());
                    }
                    if word.contains(['<', '>', '{', '}', '[', ']', '\\']) {
                        return Err(t!("server.word-bracket", word = word).into());
                    }
                    if !words.contains(&word) {
                        words.push(word);
                    }
                }
                if words.len() > MAX_WORDS {
                    return Err(t!("server.words-many", n = words.len(), max = MAX_WORDS).into());
                }
                value = words.join(",");
            }
            "ROOKEY_KEEP_CLIPBOARD" | "ROOKEY_AUTOUPDATE" if !matches!(value.as_str(), "" | "0" | "1") => {
                return Err(t!("server.on-default", key = key).into());
            }
            // rookey opens this path as it is, and only a shell knows what ~ means
            "ROOKEY_MODEL" if value.starts_with("~/") => {
                let home = dirs::home_dir().ok_or_else(|| t!("server.home"))?;
                value = home.join(&value[2..]).display().to_string();
            }
            _ => {}
        }
        changes.push((key, value));
    }
    Ok(changes)
}

/// Saves a provider's key into the keys file, and takes it out of the settings file if an
/// older rookey left it there.
fn save_key(asked: &Value) -> Res<()> {
    let id = asked["provider"].as_str().unwrap_or_default();
    let var = PROVIDERS.iter().find(|p| p.0 == id).ok_or_else(|| t!("server.no-provider"))?.2;
    let key = asked["key"].as_str().ok_or_else(|| t!("server.key-text"))?.trim();
    if key.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(t!("server.key-spaces").into());
    }
    let path = keys_path().ok_or_else(|| t!("server.no-keys-file"))?;
    write(&path, &[(var.to_string(), key.to_string())])?;
    match config_path() {
        Some(settings) if read(Some(settings.clone())).contains_key(var) => {
            write(&settings, &[(var.to_string(), String::new())])
        }
        _ => Ok(()),
    }
}

fn model(asked: &Value) -> Res<()> {
    if asked["cancel"] == true {
        models::cancel();
        return Ok(());
    }
    models::download(asked["download"].as_str().ok_or_else(|| t!("server.which-model"))?)
}

/// The hotkey rookey listens for itself: its keys, whether the service runs, and why it
/// can't here, if it can't.
fn listening(get: &dyn Fn(&str) -> String) -> Value {
    json!({
        "chord": get("ROOKEY_HOTKEY"),
        "running": crate::listen::running(),
        "blocked": crate::listen::access(),
        // while rookey listens, its only bind is the one that keeps the keys from the windows;
        // on macOS Rookey keeps them itself, all but a modifier on its own
        "swallowed": if cfg!(target_os = "macos") {
            desktop::Chord::parse(&get("ROOKEY_HOTKEY")).is_ok_and(|c| !crate::listen::is_modifier(c.key()))
        } else {
            desktop::is_bound()
        },
    })
}

/// One hotkey at a time: the compositor's bind and rookey's own listening both run
/// `rookey toggle`, and one press would start and stop it at once.
fn hotkey(asked: &Value) -> Res<Value> {
    let settings = config_path().ok_or_else(|| t!("server.no-config-dir"))?;
    if asked["unbind"] == true {
        desktop::unbind()?;
        return Ok(state());
    }
    if asked["capture"] == true {
        return Ok(json!({ "captured": crate::listen::capture(Duration::from_secs(10))? }));
    }
    if asked["unlisten"] == true {
        crate::listen::stop()?;
        if desktop::is_bound() {
            desktop::unbind()?;
        }
        write(&settings, &[("ROOKEY_HOTKEY".into(), String::new())])?;
        return Ok(state());
    }
    let chord = asked["chord"].as_str().ok_or_else(|| t!("server.which-keys"))?;
    if asked["listen"] == true {
        return listen(&settings, chord, asked["replace"] == true);
    }
    let done = desktop::bind(chord, asked["file"].as_str(), asked["replace"] == true, false)?;
    // taken keys come back as a question, and nothing was written
    if done.get("taken").is_some() {
        return Ok(done);
    }
    crate::listen::stop()?;
    write(&settings, &[("ROOKEY_HOTKEY".into(), String::new())])?;
    Ok(state())
}

fn listen(settings: &Path, chord: &str, replace: bool) -> Res<Value> {
    let chord = desktop::Chord::parse(chord)?;
    chord.check()?;
    // alone, these go down with every capital letter and every shortcut
    let busy = ["Shift_L", "Shift_R", "Control_L", "Alt_L", "Super_L"];
    if chord.mods().is_empty() && busy.iter().any(|k| k.eq_ignore_ascii_case(chord.key())) {
        return Err(t!("server.key-busy", chord = chord).into());
    }
    if crate::listen::key_code(chord.key()).is_none() {
        return Err(t!("server.key-unknown", key = chord.key()).into());
    }
    if let Some(why) = crate::listen::access() {
        return Err(why.into());
    }
    // the keys still reach the compositor, so what it does with them would happen too
    if !replace {
        if let Some(taken) = desktop::taken(&chord) {
            return Ok(json!({ "taken": taken }));
        }
    }
    if desktop::is_bound() {
        desktop::unbind()?;
    }
    write(settings, &[("ROOKEY_HOTKEY".into(), chord.to_string())])?;
    crate::listen::start()?;
    // A bind that does nothing keeps the keys from the window you are in, or they type there.
    // A modifier alone types nothing, and compositors can't bind one anyway.
    if !crate::listen::is_modifier(chord.key()) && desktop::writable() {
        if let Err(e) = desktop::bind(&chord.to_string(), None, true, true) {
            eprintln!("{}", t!("server.keys-reach", why = e));
        }
    }
    Ok(state())
}

/// A test recording from the page: the same road a dictation takes, with the text shown
/// instead of typed.
struct Trial {
    phase: &'static str, // idle, listening, working, done, failed
    text: String,
    error: String,
    waited_ms: u128,
    stop: Option<mpsc::Sender<()>>,
    stopped: Option<Instant>,
}

impl Trial {
    const fn new(phase: &'static str) -> Trial {
        Trial { phase, text: String::new(), error: String::new(), waited_ms: 0, stop: None, stopped: None }
    }
}

static TRIAL: Mutex<Trial> = Mutex::new(Trial::new("idle"));
const TRIAL_MAX: Duration = Duration::from_secs(60);

fn trial() -> Value {
    let trial = TRIAL.lock().unwrap();
    json!({ "phase": trial.phase, "text": trial.text, "error": trial.error, "waited_ms": trial.waited_ms as u64 })
}

fn try_it(asked: &Value) -> Res<()> {
    let mut trial = TRIAL.lock().unwrap();
    if asked["stop"] == true {
        if let Some(stop) = trial.stop.take() {
            let _ = stop.send(());
            trial.phase = "working";
            trial.stopped = Some(Instant::now());
        }
        return Ok(());
    }
    if matches!(trial.phase, "listening" | "working") {
        return Err(t!("server.test-running").into());
    }
    let (stop, stopped) = mpsc::channel();
    *trial = Trial { stop: Some(stop.clone()), ..Trial::new("listening") };
    drop(trial);

    thread::spawn(move || {
        crate::load_config(); // what was just changed on the page is what gets tested
        let result = crate::run(Mode::Page, stopped);
        crate::finished(Mode::Page, &result); // the pill and the sounds, as from the hotkey
        let result = result.map_err(|e| e.to_string());
        let mut trial = TRIAL.lock().unwrap();
        trial.stop = None;
        trial.waited_ms = trial.stopped.map_or(0, |t| t.elapsed().as_millis());
        match result {
            Ok(text) => (trial.phase, trial.text) = ("done", text),
            Err(e) => (trial.phase, trial.error) = ("failed", e),
        }
    });
    // a forgotten test doesn't record for ever
    thread::spawn(move || {
        thread::sleep(TRIAL_MAX);
        let _ = stop.send(());
    });
    Ok(())
}

/// Applies `changes` to a settings or keys file, leaving the rest of it as it is.
fn write(path: &Path, changes: &[(String, String)]) -> Res<()> {
    // a symlinked file (dotfiles) is written through, not replaced
    let path = fs::canonicalize(path).unwrap_or(path.to_path_buf());
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        // unreadable is not empty: writing now would wipe what is in there
        Err(e) => return Err(t!("server.cant-read", path = tilde(&path), why = e).into()),
    };
    write_private(&path, &update_config(&text, changes))
        .map_err(|e| t!("server.cant-write", path = tilde(&path), why = e).into())
}

/// Replaces the file in one step, readable by its owner only: it may hold API keys.
pub fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    // ponytail: on Windows the profile's own ACL is what keeps it the user's; no ACL of its own
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, path)
}

/// The config text with `changes` applied. A changed setting keeps its line's place, an
/// emptied one loses its line, a new one goes to the end. Comments and the rest stay.
fn update_config(text: &str, changes: &[(String, String)]) -> String {
    let mut lines = Vec::new();
    let mut seen = Vec::new();
    for line in text.lines() {
        let key = line.split_once('=').map(|(key, _)| key.trim());
        let change = key.and_then(|key| changes.iter().find(|(k, _)| k == key));
        match change {
            Some((key, value)) => {
                // a repeated key is written once: on read the last one would win
                if !seen.contains(&key) && !value.is_empty() {
                    lines.push(config_line(key, value));
                }
                seen.push(key);
            }
            None => lines.push(line.to_string()),
        }
    }
    for (key, value) in changes {
        if !seen.contains(&key) && !value.is_empty() {
            lines.push(config_line(key, value));
        }
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn config_line(key: &str, value: &str) -> String {
    let line = format!("{key}={value}");
    // one pair of quotes is dropped on read, so a value that has its own gets a pair to lose
    match parse_config(&line).get(key) {
        Some(read) if read == value => line,
        _ => format!("{key}=\"{value}\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(key: &str, value: &str) -> (String, String) {
        (key.to_string(), value.to_string())
    }

    #[cfg(unix)]
    #[test]
    fn a_window_that_fails_falls_back() {
        let run = |script: &str| Command::new("sh").args(["-c", script]).spawn().unwrap();
        let wait = Duration::from_millis(500);
        assert!(started(run("exit 1"), wait).is_err()); // no WebKitGTK: the browser instead
        assert!(started(run("exit 0"), wait).is_ok()); // closed at once, but it opened
        let t = Instant::now();
        assert!(started(run("sleep 2"), wait).is_ok()); // still up: the window is open
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn update_keeps_the_rest() {
        let text = "# mine\nROOKEY_LANG=uk\nOTHER=1\n\nROOKEY_EDIT=old\nROOKEY_LANG=en\n";
        let changes =
            [change("ROOKEY_LANG", "de"), change("ROOKEY_EDIT", ""), change("ROOKEY_CONTEXT", "cat \"my terms.txt\"")];
        assert_eq!(
            update_config(text, &changes),
            "# mine\nROOKEY_LANG=de\nOTHER=1\n\nROOKEY_CONTEXT=cat \"my terms.txt\"\n"
        );
        assert_eq!(update_config("", &[change("ROOKEY_SANITIZE", "1")]), "ROOKEY_SANITIZE=1\n");
    }

    #[test]
    fn written_values_read_back() {
        for value in ["plain", "\"quoted\"", "'it'", "say \"um\"", "a=b # c", "drop \"ну\""] {
            let text = update_config("", &[change("ROOKEY_EDIT", value)]);
            assert_eq!(parse_config(&text)["ROOKEY_EDIT"], value, "{text}");
        }
    }

    #[test]
    fn words_and_clipboard_are_checked() {
        let words =
            |v: &str| changes(format!(r#"{{"ROOKEY_WORDS": {}}}"#, serde_json::to_string(v).unwrap()).as_bytes());
        assert_eq!(words(" rookey, Kyiv\nOblast ,,rookey").unwrap(), [change("ROOKEY_WORDS", "rookey,Kyiv Oblast")]);
        assert_eq!(words("").unwrap(), [change("ROOKEY_WORDS", "")]);
        assert!(words(&"x".repeat(50)).is_err());
        assert!(words("one two three four five six").is_err());
        assert!(words("a[b]").is_err());
        assert!(words(&(0..101).map(|n| format!("w{n}")).collect::<Vec<_>>().join(",")).is_err());
        assert!(words(&(0..100).map(|n| format!("w{n}")).collect::<Vec<_>>().join(",")).is_ok());
        assert_eq!(changes(br#"{"ROOKEY_KEEP_CLIPBOARD": "0"}"#).unwrap(), [change("ROOKEY_KEEP_CLIPBOARD", "0")]);
        assert!(changes(br#"{"ROOKEY_KEEP_CLIPBOARD": "yes"}"#).is_err());
        assert_eq!(changes(br#"{"ROOKEY_AUTOUPDATE": "0"}"#).unwrap(), [change("ROOKEY_AUTOUPDATE", "0")]);
        assert_eq!(changes(br#"{"ROOKEY_AUTOUPDATE": ""}"#).unwrap(), [change("ROOKEY_AUTOUPDATE", "")]);
        assert!(changes(br#"{"ROOKEY_AUTOUPDATE": "weekly"}"#).is_err());
    }

    #[test]
    fn changes_are_checked() {
        assert!(changes(br#"{"PATH": "/tmp"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_BACKEND": "cloud"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_LANG": "en;rm -rf"}"#).is_err());
        assert_eq!(changes(br#"{"ROOKEY_LANG": "EN, uk,"}"#).unwrap(), [change("ROOKEY_LANG", "en,uk")]);
        assert_eq!(masked("sk_0123456789abcdefa1b2"), "sk_••••••••a1b2");
        assert_eq!(masked("0123456789abcdefa1b2"), "••••••••a1b2");
        assert_eq!(masked("short-key"), "");
        assert_eq!(masked("a1b2-0123456789abcdef"), "••••••••cdef"); // not a kind, part of the key
        assert!(changes(br#"{"ROOKEY_SANITIZE": 1}"#).is_err());
        assert!(changes(br#"{"ROOKEY_UI_THEME": "neon"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_NO_OVERLAY": "yes please"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_SOUNDS": "kazoo"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_PILL": "hexagon"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_HISTORY": "1"}"#).is_err()); // on is the default, and empty
        assert_eq!(changes(br#"{"ROOKEY_HISTORY": " 0 "}"#).unwrap(), [change("ROOKEY_HISTORY", "0")]);
        assert_eq!(changes(br#"{"ROOKEY_PILL": "dot"}"#).unwrap(), [change("ROOKEY_PILL", "dot")]);
        assert_eq!(changes(br#"{"ROOKEY_PILL_AT": "12,0"}"#).unwrap(), [change("ROOKEY_PILL_AT", "12,0")]);
        assert_eq!(changes(br#"{"ROOKEY_PILL_AT": ""}"#).unwrap(), [change("ROOKEY_PILL_AT", "")]);
        for bad in [
            r#"{"ROOKEY_PILL_AT": "120,5"}"#,
            r#"{"ROOKEY_PILL_AT": "top"}"#,
            r#"{"ROOKEY_PILL_AT": "5,5
PATH=/tmp"}"#,
        ] {
            assert!(changes(bad.as_bytes()).is_err(), "{bad}");
        }
        assert_eq!(changes(br#"{"ROOKEY_SOUNDS": "pencil"}"#).unwrap(), [change("ROOKEY_SOUNDS", "pencil")]);
        assert!(changes(br#"{"ROOKEY_SOUND_START": "/no/such/caw.wav"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_SOUND_START": "/tmp"}"#).is_err()); // a folder isn't a sound
        let here = env!("CARGO_MANIFEST_DIR").to_string() + "/Cargo.toml";
        let sent = json!({ "ROOKEY_SOUND_STOP": here }).to_string(); // escapes a Windows path's backslashes
        assert_eq!(changes(sent.as_bytes()).unwrap(), [change("ROOKEY_SOUND_STOP", &here)]);
        assert_eq!(changes(br#"{"ROOKEY_NO_OVERLAY": "1"}"#).unwrap(), [change("ROOKEY_NO_OVERLAY", "1")]);
        assert!(changes(br#"{"ROOKEY_UI_LANG": "xx"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_UI_CLOSED": "engine,nope"}"#).is_err());
        assert!(changes(br#"{"ROOKEY_UI_CLOSED": "engine,typed"}"#).is_ok());
        assert!(changes(br#"{"ROOKEY_UI_CLOSED": ""}"#).is_ok());
        assert_eq!(changes(br#"{"ROOKEY_UI_THEME": "dark"}"#).unwrap(), [change("ROOKEY_UI_THEME", "dark")]);
        assert!(changes(format!(r#"{{"ROOKEY_EDIT": "{}"}}"#, "x".repeat(2001)).as_bytes()).is_err());
        let ok = changes(br#"{"ROOKEY_EDIT": " one\ntwo ", "ROOKEY_BACKEND": ""}"#).unwrap();
        assert!(ok.contains(&change("ROOKEY_EDIT", "one two")));
        assert!(ok.contains(&change("ROOKEY_BACKEND", "")));
        // the kept instruction of your own goes by the same rules as the one in use
        assert!(changes(format!(r#"{{"ROOKEY_EDIT_CUSTOM": "{}"}}"#, "x".repeat(2001)).as_bytes()).is_err());
        let both = changes(br#"{"ROOKEY_EDIT": "", "ROOKEY_EDIT_CUSTOM": " mine\nhere "}"#).unwrap();
        assert!(both.contains(&change("ROOKEY_EDIT_CUSTOM", "mine here")) && both.contains(&change("ROOKEY_EDIT", "")));
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }

    #[test]
    fn silent_mic_is_its_own_check() {
        let none = |_: &str| String::new();
        let silent = |there: bool, heard| {
            checks(&none, (there, None), heard, true, true, &Access::default()).iter().any(|c| c["id"] == "mic-silent")
        };
        assert!(silent(true, Some(false)));
        // only for a mic that is there: a missing one already says so
        assert!(!silent(false, Some(false)));
        assert!(!silent(true, Some(true)));
        assert!(!silent(true, None)); // not listened to yet
    }

    #[test]
    fn open_takes_only_its_names() {
        let all = |_: &str| true;
        for os in ["macos", "windows", "linux"] {
            for what in [
                "",
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera",
                "sound; rm -rf ~",
                "ms-settings:",
                "../sound",
                "SOUND",
            ] {
                assert!(pane(what, os, &all).is_none(), "{os} {what}");
            }
        }
        assert!(open_pane(&json!({ "what": "calc.exe" })).is_err());
        assert!(open_pane(&json!({ "what": ["sound"] })).is_err());
        assert!(open_pane(&json!({})).is_err());
        assert_eq!(
            pane("screen-privacy", "macos", &all),
            Some(("open", &["x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"][..]))
        );
        assert_eq!(pane("mic-privacy", "windows", &all), Some(("explorer", &["ms-settings:privacy-microphone"][..])));
        // Linux opens a mixer only when one is installed, the first there is
        assert_eq!(
            pane("sound", "linux", &|p| p == "gnome-control-center"),
            Some(("gnome-control-center", &["sound"][..]))
        );
        assert_eq!(pane("sound", "linux", &|_| false), None);
        assert_eq!(pane("mic-privacy", "linux", &all), None);
    }

    #[test]
    fn every_failing_check_has_its_fix() {
        let all = |_: &str| true;
        // (check, macOS, Windows, Linux with a mixer); the rest are fixed on the page itself
        let expected = [
            ("mic", Some("sound"), Some("sound"), Some("sound")),
            ("mic-silent", Some("sound"), Some("mic-privacy"), Some("sound")),
            ("mic-access", Some("mic-privacy"), Some("mic-privacy"), None), // a row macOS alone has
            ("screen-access", Some("screen-privacy"), None, None),
            ("typing-access", Some("accessibility"), None, None),
            ("automation", Some("automation"), None, None),
            ("model", None, None, None),
            ("key", None, None, None),
        ];
        for (id, macos, windows, linux) in expected {
            for (os, want) in [("macos", macos), ("windows", windows), ("linux", linux)] {
                let got = opens(id, os).filter(|w| pane(w, os, &all).is_some());
                assert_eq!(got, want, "{id} on {os}");
            }
        }
    }

    #[test]
    fn macos_permissions_are_checks() {
        let screen = |k: &str| if k == "ROOKEY_CONTEXT" { "1".to_string() } else { String::new() };
        let denied = Access { mic: Some(false), screen: Some(false), typing: Some(false), automation: Some(false) };
        let ids = |access: &Access, heard| -> Vec<(String, bool)> {
            checks(&screen, (true, None), heard, true, true, access)
                .iter()
                .map(|c| (c["id"].as_str().unwrap().to_string(), c["ok"] == true))
                .collect()
        };
        let got = ids(&denied, Some(false));
        for id in ["mic-access", "typing-access", "automation", "screen-access"] {
            assert!(got.contains(&(id.to_string(), false)), "{id} in {got:?}");
        }
        // silence from a mic macOS keeps closed is that, not a muted mic
        assert!(!got.iter().any(|c| c.0 == "mic-silent"));
        let allowed = Access { mic: Some(true), screen: Some(true), typing: Some(true), automation: None };
        let got = ids(&allowed, Some(true));
        assert!(
            got.contains(&("typing-access".to_string(), true)) && got.contains(&("screen-access".to_string(), true))
        );
        assert!(!got.iter().any(|c| c.0 == "mic-access" || c.0 == "automation"));
        // elsewhere nothing is asked, so there are no such rows
        let got = ids(&Access::default(), None);
        assert!(!got.iter().any(|c| c.0.ends_with("-access") || c.0 == "automation"));
    }

    #[test]
    fn language_pack_names() {
        let packs = ["ukr", "chi_sim"];
        assert_eq!(
            lang_packages("sudo pacman -S --needed", &packs).unwrap(),
            "tesseract-data-ukr tesseract-data-chi_sim"
        );
        assert_eq!(lang_packages("sudo apt install", &packs).unwrap(), "tesseract-ocr-ukr tesseract-ocr-chi-sim");
        assert_eq!(
            lang_packages("sudo dnf install", &packs).unwrap(),
            "tesseract-langpack-ukr tesseract-langpack-chi_sim"
        );
        assert_eq!(lang_packages("brew install", &packs).unwrap(), "tesseract-lang");
        assert_eq!(lang_packages("zypper in", &packs), None);
    }

    #[test]
    fn the_app_macos_asks_about() {
        let table = "    1     0 /sbin/launchd\n\
                     400     1 /System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal\n\
                     401   400 login\n\
                     402   401 -zsh\n\
                     500   402 /usr/local/bin/rookey\n\
                     600     1 /Applications/Visual Studio Code.app/Contents/MacOS/Electron\n\
                     601   600 /Applications/Visual Studio Code.app/Contents/Frameworks/Code Helper (Plugin).app/Contents/MacOS/Code Helper (Plugin)\n\
                     602   601 /bin/zsh\n\
                     603   602 rookey\n\
                     700     1 tmux\n\
                     701   700 -zsh\n\
                     702   701 rookey\n";
        assert_eq!(responsible_app(table, 500).as_deref(), Some("Terminal"));
        assert_eq!(responsible_app(table, 603).as_deref(), Some("Visual Studio Code"));
        assert_eq!(responsible_app(table, 702), None);
        assert_eq!(responsible_app(table, 999), None);
        assert_eq!(responsible_app("garbage\n\n", 1), None);
    }
}
