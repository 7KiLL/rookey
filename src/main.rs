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

    // Load the model while we record, so stopping feels instant.
    whisper_rs::install_logging_hooks(); // silences whisper.cpp stderr spam
    let loader = thread::spawn(move || {
        WhisperContext::new_with_params(&model, WhisperContextParameters::default())
    });

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

    let ctx = loader.join().map_err(|_| "model loader panicked")??;
    notify(toggle, "transcribing");
    transcribe(&ctx, &audio)
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
    use super::resample;

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
