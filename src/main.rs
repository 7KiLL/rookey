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
use std::{env, fs, thread};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const WHISPER_RATE: u32 = 16_000;
const DEFAULT_MODEL: &str = "ggml-large-v3-turbo.bin";

fn main() {
    if let Err(e) = cli() {
        eprintln!("yap: {e}");
        std::process::exit(1);
    }
}

fn cli() -> Res<()> {
    let toggle = match env::args().nth(1).as_deref() {
        None => false,
        Some("toggle") => true,
        Some(_) => {
            eprintln!("usage: yap [toggle]");
            std::process::exit(2);
        }
    };

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

fn run(toggle: bool) -> Res<String> {
    let cloud = match env::var("YAP_BACKEND").as_deref() {
        Err(_) | Ok("local") => false,
        Ok("elevenlabs") => true,
        Ok(b) => return Err(format!("unknown YAP_BACKEND {b:?} (local, elevenlabs)").into()),
    };
    // Load the model while we record, so stopping feels instant.
    let loader = if cloud { None } else { Some(spawn_model_loader()?) };

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

    let (samples, rate) = record_until(stop_rx, toggle)?;
    let audio = resample(&samples, rate, WHISPER_RATE);

    notify(toggle, "transcribing");
    match loader {
        None => elevenlabs(&audio),
        Some(loader) => transcribe(&loader.join().map_err(|_| "model loader panicked")??, &audio),
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
    Ok(thread::spawn(move || {
        WhisperContext::new_with_params(&model, WhisperContextParameters::default())
    }))
}

/// ElevenLabs Scribe batch API: upload the whole clip once recording stops.
// ponytail: batch, not the realtime websocket; switch if the post-stop wait matters.
fn elevenlabs(audio: &[f32]) -> Res<String> {
    use ureq::unversioned::multipart::{Form, Part};

    let key = env::var("ELEVENLABS_API_KEY")
        .map_err(|_| "YAP_BACKEND=elevenlabs needs ELEVENLABS_API_KEY")?;
    let lang = env::var("YAP_LANG").unwrap_or_default();
    let wav = wav_bytes(audio);

    let mut form = Form::new()
        .text("model_id", "scribe_v2")
        .part("file", Part::bytes(&wav).file_name("yap.wav"));
    if !lang.is_empty() && lang != "auto" {
        form = form.text("language_code", &lang);
    }
    let mut res = ureq::post("https://api.elevenlabs.io/v1/speech-to-text")
        .header("xi-api-key", &key)
        .config()
        .http_status_as_error(false) // keep the error body, it says what went wrong
        .build()
        .send(form)?;
    let body = res.body_mut().read_to_string()?;
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
fn record_until(stop: mpsc::Receiver<()>, toggle: bool) -> Res<(Vec<f32>, u32)> {
    let device = cpal::default_host().default_input_device().ok_or("no input device")?;
    let config = device.default_input_config()?;
    let rate = config.sample_rate();
    let buf = Arc::new(Mutex::new(Vec::new()));

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, config.into(), buf.clone()),
        cpal::SampleFormat::I16 => build::<i16>(&device, config.into(), buf.clone()),
        cpal::SampleFormat::I32 => build::<i32>(&device, config.into(), buf.clone()),
        f => return Err(format!("unsupported sample format {f:?}").into()),
    }?;
    stream.play()?;
    notify(toggle, "recording");
    eprintln!("recording... (Enter to stop)");
    stop.recv()?;
    drop(stream);

    let samples = std::mem::take(&mut *buf.lock().unwrap());
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
    state.full(params, audio)?;
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
        let audio: Vec<f32> = bytes[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
            .collect();
        let text = elevenlabs(&audio).unwrap();
        println!("{text}");
        assert!(text.to_lowercase().contains("your country"), "{text}");
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
