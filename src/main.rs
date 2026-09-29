//! yap: record the mic, transcribe locally with whisper.cpp, print or type the text.
//!
//!   yap          record until Enter (or Ctrl-C), print transcript to stdout
//!   yap toggle   first call starts recording, second call stops it and types the text
//!
//! Env: YAP_MODEL (ggml model path), YAP_LANG (default "auto").

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use std::{env, fs, thread};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const WHISPER_RATE: u32 = 16_000;
const DEFAULT_MODEL: &str = "ggml-large-v3-turbo.bin";

static VERBOSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// `-v` step log on stderr, timestamped from process start.
macro_rules! vlog {
    ($($arg:tt)*) => {
        if VERBOSE.load(std::sync::atomic::Ordering::Relaxed) {
            let t = START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64();
            eprintln!("[{t:7.3}s] {}", format!($($arg)*));
        }
    };
}

fn main() {
    START.get_or_init(std::time::Instant::now);
    if let Err(e) = cli() {
        eprintln!("yap: {e}");
        std::process::exit(1);
    }
}

fn cli() -> Res<()> {
    let mut toggle = false;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "toggle" => toggle = true,
            "-v" | "--verbose" => VERBOSE.store(true, std::sync::atomic::Ordering::Relaxed),
            _ => {
                eprintln!("usage: yap [-v] [toggle]");
                std::process::exit(2);
            }
        }
    }

    let pidfile = dirs::runtime_dir().unwrap_or_else(env::temp_dir).join("yap.pid");
    if toggle {
        // A recording is running: tell it to stop, it does the rest.
        if let Ok(pid) = fs::read_to_string(&pidfile) {
            let stopped = Command::new("kill").args(["-TERM", pid.trim()]).status()?.success();
            if stopped {
                return Ok(());
            }
            // stale pidfile, fall through and start a new recording
        }
        fs::write(&pidfile, std::process::id().to_string())?;
    }

    let text = run(toggle);
    if toggle {
        let _ = fs::remove_file(&pidfile);
    }
    let text = text?;

    if toggle {
        type_text(&text)?;
    } else {
        writeln!(std::io::stdout(), "{text}")?; // println! panics on a closed pipe
    }
    Ok(())
}

enum Backend {
    Local(Loader),
    ElevenLabs,
    Realtime(Option<Realtime>), // connected on the first audio tick
}

fn run(toggle: bool) -> Res<String> {
    let mut backend = match env::var("YAP_BACKEND").as_deref() {
        // Load the model while we record, so stopping feels instant.
        Err(_) | Ok("local") => Backend::Local(spawn_model_loader()?),
        Ok("elevenlabs") => Backend::ElevenLabs,
        Ok("elevenlabs-realtime") => Backend::Realtime(None),
        Ok(b) => {
            return Err(format!(
                "unknown YAP_BACKEND {b:?} (local, elevenlabs, elevenlabs-realtime)"
            )
            .into());
        }
    };
    // Verbose logs every partial as its own line instead of rewriting one.
    let show_partials =
        !toggle && std::io::stderr().is_terminal() && !VERBOSE.load(std::sync::atomic::Ordering::Relaxed);
    vlog!("backend: {}", env::var("YAP_BACKEND").unwrap_or_else(|_| "local".into()));

    let (stop_tx, stop_rx) = mpsc::channel();
    // Enter stops too: Ctrl-C would also kill the other side of `yap | wl-copy`.
    if !toggle && std::io::stdin().is_terminal() {
        let tx = stop_tx.clone();
        thread::spawn(move || {
            let _ = std::io::stdin().read_line(&mut String::new());
            let _ = tx.send(());
        });
    }
    ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    })?;

    // Realtime streams each chunk as it's recorded; the others wait for the whole clip.
    let (samples, rate) = record_until(stop_rx, toggle, |chunk, rate| {
        if let Backend::Realtime(rt) = &mut backend {
            let rt = match rt {
                Some(rt) => rt,
                None => rt.insert(Realtime::connect()?),
            };
            rt.send(&resample(chunk, rate, WHISPER_RATE), false)?;
            rt.poll(show_partials)?;
        }
        Ok(())
    })?;

    notify(toggle, "transcribing");
    let audio = resample(&samples, rate, WHISPER_RATE);
    match backend {
        Backend::Local(loader) => {
            transcribe(&loader.join().map_err(|_| "model loader panicked")??, &audio)
        }
        Backend::ElevenLabs => elevenlabs(&audio),
        Backend::Realtime(Some(rt)) => rt.finish(show_partials),
        Backend::Realtime(None) => Ok(String::new()), // stopped before the first tick
    }
}

type Loader = thread::JoinHandle<Result<WhisperContext, whisper_rs::WhisperError>>;

fn spawn_model_loader() -> Res<Loader> {
    let model = env::var_os("YAP_MODEL").map(PathBuf::from).unwrap_or_else(|| {
        dirs::data_dir().unwrap_or_default().join("yap").join(DEFAULT_MODEL)
    });
    if !model.exists() {
        return Err(format!(
            "model not found: {}\nget it with:\n  mkdir -p {dir} && curl -L -o {path} \
             https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{DEFAULT_MODEL}",
            model.display(),
            dir = model.parent().unwrap().display(),
            path = model.display(),
        )
        .into());
    }
    whisper_rs::install_logging_hooks(); // silences whisper.cpp stderr spam
    vlog!("whisper: loading {} in background", model.display());
    Ok(thread::spawn(move || {
        let ctx = WhisperContext::new_with_params(&model, WhisperContextParameters::default());
        vlog!("whisper: model loaded");
        ctx
    }))
}

fn elevenlabs_key() -> Res<String> {
    Ok(env::var("ELEVENLABS_API_KEY").map_err(|_| "ElevenLabs backends need ELEVENLABS_API_KEY")?)
}

/// YAP_LANG for the API, None = auto-detect.
fn lang_code() -> Option<String> {
    env::var("YAP_LANG").ok().filter(|l| !l.is_empty() && l != "auto")
}

type Ws = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

/// ElevenLabs Scribe realtime: audio streams over a websocket while you talk,
/// so on stop only a commit round-trip is left.
struct Realtime {
    ws: Ws,
    committed: String,
    sent: usize, // samples streamed so far, for the log
}

impl Realtime {
    fn connect() -> Res<Self> {
        use tungstenite::client::IntoClientRequest;

        let mut url = format!(
            "wss://api.elevenlabs.io/v1/speech-to-text/realtime\
             ?model_id=scribe_v2_realtime&audio_format=pcm_{WHISPER_RATE}"
        );
        if let Some(lang) = lang_code() {
            url += &format!("&language_code={lang}");
        }
        vlog!("ws: connecting {url}");
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
        vlog!("ws: handshake done (HTTP {}), waiting for session_started", res.status());
        let mut rt = Realtime { ws, committed: String::new(), sent: 0 };

        // The server opens with session_started; anything else (bad key, quota) is an error.
        rt.set_read_timeout(Duration::from_secs(10))?;
        let first = rt.next()?.ok_or("elevenlabs realtime: no session_started")?;
        if first["message_type"] != "session_started" {
            return Err(format!("elevenlabs realtime: {first}").into());
        }
        vlog!("<- session_started {}", first);
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
        vlog!("waiting for committed_transcript...");
        self.set_read_timeout(Duration::from_secs(10))?;
        loop {
            let msg = self.next()?.ok_or("elevenlabs realtime: timed out waiting for transcript")?;
            if self.handle(msg, show_partials)? {
                break;
            }
        }
        vlog!("commit round-trip {} ms", t.elapsed().as_millis());
        let _ = self.ws.close(None);
        vlog!("ws: closed");
        if show_partials {
            eprint!("\r\x1b[K"); // clear the partial line
        }
        Ok(self.committed.trim().to_string())
    }

    /// Returns true on a committed transcript.
    fn handle(&mut self, msg: serde_json::Value, show_partials: bool) -> Res<bool> {
        let text = msg["text"].as_str().unwrap_or_default();
        let kind = msg["message_type"].as_str().unwrap_or_default();
        vlog!("<- {kind}: {text:?}");
        match kind {
            "partial_transcript" if show_partials => eprint!("\r\x1b[K{text}"),
            "partial_transcript" | "session_started" => {}
            "committed_transcript" => {
                self.committed.push(' ');
                self.committed.push_str(text);
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
fn elevenlabs(audio: &[f32]) -> Res<String> {
    use ureq::unversioned::multipart::{Form, Part};

    let key = elevenlabs_key()?;
    let wav = wav_bytes(audio);

    let mut form = Form::new()
        .text("model_id", "scribe_v2")
        .part("file", Part::bytes(&wav).file_name("yap.wav"));
    let lang = lang_code();
    if let Some(lang) = &lang {
        form = form.text("language_code", lang);
    }
    vlog!("http: uploading {} KB wav to /v1/speech-to-text", wav.len() / 1024);
    let t = std::time::Instant::now();
    let mut res = ureq::post("https://api.elevenlabs.io/v1/speech-to-text")
        .header("xi-api-key", &key)
        .config()
        .http_status_as_error(false) // keep the error body, it says what went wrong
        .build()
        .send(form)?;
    let body = res.body_mut().read_to_string()?;
    vlog!("http: {} in {} ms", res.status(), t.elapsed().as_millis());
    if !res.status().is_success() {
        return Err(format!("elevenlabs {}: {body}", res.status()).into());
    }
    let json: serde_json::Value = serde_json::from_str(&body)?;
    Ok(json["text"].as_str().unwrap_or_default().trim().to_string())
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
/// `tick` gets each new chunk (~250 ms, at the device rate) while recording, then the tail.
fn record_until(
    stop: mpsc::Receiver<()>,
    toggle: bool,
    mut tick: impl FnMut(&[f32], u32) -> Res<()>,
) -> Res<(Vec<f32>, u32)> {
    let device = cpal::default_host().default_input_device().ok_or("no input device")?;
    let config = device.default_input_config()?;
    let rate = config.sample_rate();
    let buf = Arc::new(Mutex::new(Vec::new()));
    vlog!(
        "mic: {} ({} Hz, {} ch, {:?})",
        device.id().map(|id| id.to_string()).unwrap_or_else(|_| "?".into()),
        rate,
        config.channels(),
        config.sample_format()
    );

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, config.into(), buf.clone()),
        cpal::SampleFormat::I16 => build::<i16>(&device, config.into(), buf.clone()),
        cpal::SampleFormat::I32 => build::<i32>(&device, config.into(), buf.clone()),
        f => return Err(format!("unsupported sample format {f:?}").into()),
    }?;
    stream.play()?;
    notify(toggle, "recording");
    eprintln!("recording... (Enter to stop)");
    let mut sent = 0;
    while let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(Duration::from_millis(250)) {
        let chunk = buf.lock().unwrap()[sent..].to_vec();
        sent += chunk.len();
        // peak level tells you at a glance whether the mic is actually hearing you
        let peak = chunk.iter().fold(0f32, |m, s| m.max(s.abs()));
        vlog!("mic: {:4} ms captured, peak {peak:.3}", chunk.len() * 1000 / rate as usize);
        tick(&chunk, rate)?;
    }
    drop(stream);
    vlog!("mic: stopped");

    let samples = std::mem::take(&mut *buf.lock().unwrap());
    tick(&samples[sent..], rate)?;
    vlog!("recorded {:.2} s total", samples.len() as f64 / rate as f64);
    Ok((samples, rate))
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

fn transcribe(ctx: &WhisperContext, audio: &[f32]) -> Res<String> {
    let lang = env::var("YAP_LANG").unwrap_or_else(|_| "auto".into());
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some(&lang));
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);

    let mut state = ctx.create_state()?;
    vlog!("whisper: transcribing {:.2} s of audio", audio.len() as f64 / WHISPER_RATE as f64);
    state.full(params, audio)?;
    vlog!("whisper: done");
    let mut text = String::new();
    for seg in state.as_iter() {
        text.push_str(&seg.to_str_lossy()?);
    }
    Ok(text.trim().to_string())
}

/// Types text into the focused window.
fn type_text(text: &str) -> Res<()> {
    if text.is_empty() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        // ponytail: paste via clipboard (keystroke mangles non-ASCII); clobbers the clipboard.
        let mut pb = Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn()?;
        pb.stdin.take().unwrap().write_all(text.as_bytes())?;
        pb.wait()?;
        Command::new("osascript")
            .args(["-e", r#"tell application "System Events" to keystroke "v" using command down"#])
            .status()?;
    }
    #[cfg(not(target_os = "macos"))]
    Command::new("wtype").args(["--", text]).status()?;
    Ok(())
}

/// Desktop notification, only in toggle mode (a hotkey has no terminal to print to).
fn notify(toggle: bool, msg: &str) {
    if !toggle {
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = Command::new("osascript")
        .args(["-e", &format!(r#"display notification "{msg}" with title "yap""#)])
        .status();
    #[cfg(not(target_os = "macos"))]
    let _ = Command::new("notify-send").args(["-t", "1500", "yap", msg]).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs network + key: ELEVENLABS_API_KEY=... cargo test -- --ignored
    /// Uses whisper.cpp's jfk.wav (16 kHz mono 16-bit), fetched to $TMPDIR.
    #[test]
    #[ignore]
    fn elevenlabs_jfk() {
        let text = elevenlabs(&jfk()).unwrap();
        println!("{text}");
        assert!(text.to_lowercase().contains("your country"), "{text}");
    }

    /// Streams in 250 ms chunks at real-time pace, like the mic loop does.
    #[test]
    #[ignore]
    fn elevenlabs_realtime_jfk() {
        let mut rt = Realtime::connect().unwrap();
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
