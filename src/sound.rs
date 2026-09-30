//! The cues: a sound as rookey starts listening, stops, types and fails. Three sets are made
//! from tones and noise as they play, so there is nothing to license and nothing to ship; any
//! cue can be swapped for a file of your own.

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

/// The sets, the first is the default. Their names are in ui/i18n.js.
pub const SETS: [&str; 3] = ["rook", "notes", "pencil"];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cue {
    Start,
    Stop,
    Typed,
    Failed,
}

impl Cue {
    pub const ALL: [Cue; 4] = [Cue::Start, Cue::Stop, Cue::Typed, Cue::Failed];

    pub fn name(self) -> &'static str {
        match self {
            Cue::Start => "start",
            Cue::Stop => "stop",
            Cue::Typed => "typed",
            Cue::Failed => "failed",
        }
    }

    /// The setting that swaps this cue for a file: `ROOKEY_SOUND_START`...
    pub fn setting(self) -> String {
        format!("ROOKEY_SOUND_{}", self.name().to_uppercase())
    }
}

/// Sounds still playing, waited for before the process ends (the typed cue comes last).
static PLAYING: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// Plays a cue without waiting, as the settings say: `setting` reads one by name.
pub fn play(cue: Cue, setting: impl Fn(&str) -> Option<String>) {
    if let Some(file) = setting(&cue.setting()) {
        return play_file(&file);
    }
    let set = setting("ROOKEY_SOUNDS").unwrap_or_default();
    let samples = move |rate| render(&voices(&set, cue), rate);
    PLAYING.lock().unwrap().push(thread::spawn(move || {
        if let Err(e) = play_samples(samples) {
            vlog!(1, "sound: {e}");
        }
    }));
}

/// Waits for the cues still playing.
pub fn wait() {
    for h in PLAYING.lock().unwrap().drain(..) {
        let _ = h.join();
    }
}

/// A file of the user's: handed to the system's player, which knows more formats than we do.
// ponytail: an external player per file (Windows: WAV only); decode in-house if people ask
fn play_file(file: &str) {
    #[cfg(target_os = "macos")]
    let players: &[&[&str]] = &[&["afplay"]];
    #[cfg(windows)]
    let script = format!("(New-Object Media.SoundPlayer '{}').PlaySync()", file.replace('\'', "''"));
    #[cfg(windows)]
    let players: &[&[&str]] = &[&["powershell", "-NoProfile", "-Command"]];
    #[cfg(not(any(target_os = "macos", windows)))]
    let players: &[&[&str]] = &[&["pw-play"], &["paplay"], &["aplay", "-q"]];
    for player in players {
        let mut cmd = Command::new(player[0]);
        cmd.args(&player[1..]);
        #[cfg(windows)]
        cmd.arg(&script);
        #[cfg(not(windows))]
        cmd.arg(file);
        crate::no_window(&mut cmd);
        if let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            thread::spawn(move || child.wait());
            return;
        }
    }
    vlog!(1, "sound: nothing here plays {file}");
}

fn play_samples(samples: impl FnOnce(u32) -> Vec<f32>) -> crate::Res<()> {
    let device = cpal::default_host().default_output_device().ok_or("no output device")?;
    let config = device.default_output_config()?;
    let (rate, format) = (config.sample_rate(), config.sample_format());
    let samples = samples(rate);
    let length = Duration::from_secs_f32(samples.len() as f32 / rate as f32);
    let stream = match format {
        cpal::SampleFormat::F32 => out::<f32>(&device, config.into(), samples),
        cpal::SampleFormat::I16 => out::<i16>(&device, config.into(), samples),
        cpal::SampleFormat::I32 => out::<i32>(&device, config.into(), samples),
        f => return Err(format!("unsupported output format {f:?}").into()),
    }?;
    stream.play()?;
    // the device buffers a little behind what we hand it
    thread::sleep(length + Duration::from_millis(120));
    Ok(())
}

fn out<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    samples: Vec<f32>,
) -> crate::Res<cpal::Stream> {
    let channels = config.channels as usize;
    let mut next = samples.into_iter();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &_| {
            for frame in data.chunks_mut(channels) {
                let s = T::from_sample(next.next().unwrap_or(0.0));
                frame.fill(s);
            }
        },
        crate::audio_error,
        None,
    )?;
    Ok(stream)
}

#[derive(Clone, Copy)]
enum Filter {
    Low,
    High,
    Band,
}

/// One sound in a cue: what, from when, for how long (seconds), how loud (0 to 1).
#[derive(Clone, Copy)]
enum Voice {
    /// a sine
    Tone { hz: f32, at: f32, len: f32, level: f32 },
    /// white noise through a filter
    Tap { filter: Filter, hz: f32, at: f32, len: f32, level: f32 },
    /// a buzzy saw falling from one pitch to another, through a band around 1400 Hz
    Caw { from: f32, to: f32, at: f32, len: f32, level: f32 },
}

fn voices(set: &str, cue: Cue) -> Vec<Voice> {
    use Cue::*;
    use Filter::*;
    use Voice::*;
    let tone = |hz, at, len, level| Tone { hz, at, len, level };
    let tap = |filter, hz, at, len, level| Tap { filter, hz, at, len, level };
    let caw = |from, to, at, len, level| Caw { from, to, at, len, level };
    match (set, cue) {
        // soft sine notes: up as it listens, down as it stops
        ("notes", Start) => vec![tone(587.33, 0.0, 0.09, 0.3), tone(880.0, 0.09, 0.16, 0.3)],
        ("notes", Stop) => vec![tone(880.0, 0.0, 0.09, 0.3), tone(587.33, 0.09, 0.16, 0.3)],
        ("notes", Typed) => vec![tone(1174.66, 0.0, 0.08, 0.18)],
        ("notes", Failed) => vec![tone(196.0, 0.0, 0.14, 0.4), tone(196.0, 0.2, 0.14, 0.4)],
        // a pencil on the pad: no pitch to get tired of
        ("pencil", Start) => vec![tap(Band, 2200.0, 0.0, 0.03, 0.8)],
        ("pencil", Stop) => vec![tap(Band, 2200.0, 0.0, 0.03, 0.8), tap(Band, 1700.0, 0.1, 0.03, 0.8)],
        ("pencil", Typed) => vec![tap(High, 3000.0, 0.0, 0.12, 0.35)],
        ("pencil", Failed) => vec![tap(Low, 220.0, 0.0, 0.16, 1.0)],
        // the rook: a small caw, beak clacks, a keycap knock, two low caws
        (_, Start) => vec![caw(620.0, 480.0, 0.0, 0.14, 0.5)],
        (_, Stop) => vec![tap(High, 3500.0, 0.0, 0.012, 0.9), tap(High, 3000.0, 0.06, 0.012, 0.9)],
        (_, Typed) => vec![tap(Low, 900.0, 0.0, 0.03, 0.9), tone(210.0, 0.0, 0.05, 0.25)],
        (_, Failed) => vec![caw(340.0, 250.0, 0.0, 0.16, 0.55), caw(330.0, 240.0, 0.22, 0.18, 0.55)],
    }
}

/// Overall loudness, the same as on the design board.
const VOLUME: f32 = 0.6;
const SILENT: f32 = 0.0001;

/// Mixes the voices into mono samples at `rate`.
fn render(voices: &[Voice], rate: u32) -> Vec<f32> {
    let fs = rate as f32;
    let end = voices.iter().map(|v| v.span().0 + v.span().1).fold(0.0, f32::max) + 0.03;
    let mut out = vec![0.0f32; (end * fs) as usize];
    let mut noise = 0x9E37_79B9_u32; // xorshift, the same every time
    for v in voices {
        let (at, len, level, attack) = v.span_full();
        let peak = level * VOLUME;
        let mut filter = match *v {
            Voice::Tap { filter, hz, .. } => Some(Biquad::new(filter, hz, 1.2, fs)),
            Voice::Caw { .. } => Some(Biquad::new(Filter::Band, 1400.0, 2.0, fs)),
            Voice::Tone { .. } => None,
        };
        let mut phase = 0.0f32;
        let first = (at * fs) as usize;
        for (i, s) in out.iter_mut().enumerate().skip(first).take((len * fs) as usize) {
            let t = (i - first) as f32 / fs;
            // exponential up over the attack, then down to silence at the end, like WebAudio's ramps
            let gain = if t < attack {
                peak * (SILENT / peak).powf(1.0 - t / attack)
            } else {
                peak * (SILENT / peak).powf((t - attack) / (len - attack).max(1e-4))
            };
            let raw = match *v {
                Voice::Tone { hz, .. } => (std::f32::consts::TAU * hz * t).sin(),
                Voice::Tap { .. } => {
                    noise ^= noise << 13;
                    noise ^= noise >> 17;
                    noise ^= noise << 5;
                    noise as f32 / u32::MAX as f32 * 2.0 - 1.0
                }
                Voice::Caw { from, to, .. } => {
                    phase = (phase + from * (to / from).powf(t / len) / fs).fract();
                    phase * 2.0 - 1.0
                }
            };
            let x = filter.as_mut().map_or(raw, |f| f.run(raw));
            *s += x * gain;
        }
    }
    out.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));
    out
}

impl Voice {
    fn span(&self) -> (f32, f32) {
        let (at, len, _, _) = self.span_full();
        (at, len)
    }

    /// (start, length, level, attack)
    fn span_full(&self) -> (f32, f32, f32, f32) {
        match *self {
            Voice::Tone { at, len, level, .. } => (at, len, level, 0.008),
            Voice::Tap { at, len, level, .. } => (at, len, level, 0.0),
            Voice::Caw { at, len, level, .. } => (at, len, level, 0.012),
        }
    }
}

/// The audio EQ cookbook's filters (R. Bristow-Johnson), the ones WebAudio has too.
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn new(kind: Filter, hz: f32, q: f32, fs: f32) -> Biquad {
        let w = std::f32::consts::TAU * hz.min(fs * 0.45) / fs;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        let b = match kind {
            Filter::Low => [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
            Filter::High => [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
            Filter::Band => [alpha, 0.0, -alpha],
        };
        let a0 = 1.0 + alpha;
        Biquad { b: b.map(|v| v / a0), a: [-2.0 * cos / a0, (1.0 - alpha) / a0], x: [0.0; 2], y: [0.0; 2] }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_makes_a_sound() {
        for set in SETS {
            for cue in Cue::ALL {
                let s = render(&voices(set, cue), 48_000);
                let peak = s.iter().fold(0f32, |m, x| m.max(x.abs()));
                assert!(s.len() > 480 && s.len() < 48_000, "{set} {cue:?}: {} samples", s.len());
                assert!(peak > 0.02 && peak <= 1.0 && s.iter().all(|x| x.is_finite()), "{set} {cue:?}: peak {peak}");
            }
        }
        // an unknown set falls back to the default rather than going quiet
        assert_eq!(render(&voices("nope", Cue::Start), 48_000), render(&voices("rook", Cue::Start), 48_000));
    }
}
