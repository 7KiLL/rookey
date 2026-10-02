//! `rookey overlay`: the pill on screen while rookey listens, transcribes, types or fails.
//! It is its own process that follows the status file (see status.rs), so it never slows the
//! recording down, and a crash in it costs nothing but the pill. It quits once rookey is idle.
//!
//! The pill is drawn here into premultiplied BGRA pixels, the byte order a Wayland ARGB8888
//! buffer, a Windows layered window and a macOS CGImage all take; only showing them differs.

use std::fs;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use fontdue::{Font, FontSettings};
use serde_json::Value;

use crate::status;

/// The surface, in logical pixels: the pill is centred in it, the rest is transparent and
/// lets clicks through.
pub const W: u32 = 480;
pub const H: u32 = 36;
/// Gap between the pill and the screen's bottom edge (or a bar there).
pub const MARGIN: i32 = 28;
/// How long a failure stays on screen. The bar and `rookey status` keep it until the next
/// recording; a pill over your work shouldn't.
pub const FAILED_MS: u64 = 5000;

const INK: [f32; 3] = rgb(0x17231E);
const PAPER: [f32; 3] = rgb(0xF6F9EF);
const RED: [f32; 3] = rgb(0xE5584C);
const ALARM: [f32; 3] = rgb(0xA32A22);

const fn rgb(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 255) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0]
}

// The page's own fonts, cut down to what the pill says. fontdue reads no woff2 and no font
// variations, so each is decompressed, pinned to one instance and subset:
//   woff2_decompress src/ui/commissioner-latin.woff2
//   fonttools varLib.instancer commissioner-latin.ttf wght=560 FLAR=70 VOLM=0 slnt=0 -o cl.ttf
//   pyftsubset cl.ttf --unicodes=U+0020-007E,U+2019,U+2014,U+2026 --layout-features='' --no-hinting
// likewise the Cyrillic half (U+0400-045F,U+0490-0491), and Martian Mono at wght=450 wdth=87
// with only 0-9 and the colon (U+0030-003A).
static WORDS: LazyLock<[Font; 2]> = LazyLock::new(|| {
    [font(include_bytes!("overlay/commissioner-latin.ttf")), font(include_bytes!("overlay/commissioner-cyrillic.ttf"))]
});
static DIGITS: LazyLock<[Font; 1]> = LazyLock::new(|| [font(include_bytes!("overlay/martian-mono-digits.ttf"))]);

fn font(bytes: &'static [u8]) -> Font {
    Font::from_bytes(bytes, FontSettings::default()).expect("fonts are compiled in")
}

/// Where the pill goes when nothing says otherwise: the bottom centre.
pub const DEFAULT_AT: (f64, f64) = (50.0, 100.0);

/// ROOKEY_PILL_AT: "x,y", each 0 to 100, in percent of the room the pill has on the screen's
/// usable area, from the top left. Percent, so one setting fits every screen it lands on.
pub fn parse_at(value: &str) -> Option<(f64, f64)> {
    let (x, y) = value.split_once(',')?;
    let percent = |s: &str| s.trim().parse::<f64>().ok().filter(|n| (0.0..=100.0).contains(n));
    Some((percent(x)?, percent(y)?))
}

/// Where this pill goes, from the settings.
pub fn at() -> (f64, f64) {
    crate::setting("ROOKEY_PILL_AT").and_then(|v| parse_at(&v)).unwrap_or(DEFAULT_AT)
}

/// The pill's top-left corner in an area `w` by `h` (origin at its top left), in pixels that
/// are `scale` times the pill's own: it keeps MARGIN from every edge, and `at` picks within
/// the room that leaves. The default is the bottom centre the pill always had.
pub fn spot(w: f64, h: f64, scale: f64, at: (f64, f64)) -> (f64, f64) {
    let margin = MARGIN as f64 * scale;
    let room = |size: f64, pill: u32| (size - pill as f64 * scale - 2.0 * margin).max(0.0);
    (margin + room(w, W) * at.0 / 100.0, margin + room(h, H) * at.1 / 100.0)
}

/// Whether a recording should show the pill: it's on, and this desktop can show it.
pub fn wanted() -> bool {
    if crate::setting("ROOKEY_NO_OVERLAY").is_some() {
        return false;
    }
    // ponytail: no layer shell (GNOME, X11), no pill: the notification and sounds carry it there
    #[cfg(target_os = "linux")]
    return crate::linux::layer_shell();
    #[cfg(any(windows, target_os = "macos"))]
    return true;
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    return false;
}

fn pidfile() -> std::path::PathBuf {
    crate::pidfile().with_file_name("rookey-overlay.pid")
}

fn showing() -> bool {
    fs::read_to_string(pidfile()).ok().and_then(|p| p.trim().parse().ok()).is_some_and(crate::alive)
}

/// Starts `rookey overlay` unless one is up already (it outlives a recording by a moment).
pub fn show() {
    if showing() {
        return;
    }
    let exe = crate::exe().unwrap_or_else(|_| "rookey".into());
    let mut cmd = Command::new(exe);
    cmd.arg("overlay").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    crate::no_window(&mut cmd);
    if let Err(e) = cmd.spawn() {
        vlog!(1, "overlay: couldn't start: {e}");
    }
}

/// `rookey overlay`
pub fn run() -> Res {
    if showing() {
        return Ok(());
    }
    fs::write(pidfile(), std::process::id().to_string())?;
    #[cfg(target_os = "linux")]
    let done = wayland::run();
    #[cfg(windows)]
    let done = crate::overlay_win::run();
    #[cfg(target_os = "macos")]
    let done = crate::overlay_mac::run();
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    let done: Res = Err(crate::t!("overlay.unsupported").into());
    let _ = fs::remove_file(pidfile());
    done
}

type Res = crate::Res<()>;

/// The looks to pick from with ROOKEY_PILL: the first is the default.
pub const STYLES: [&str; 3] = ["full", "compact", "dot"];

/// What the pill shows, and the bits that move.
pub struct Pill {
    style: &'static str,
    raw: Value,
    shown: Value,
    history: [f32; 7], // the last levels, oldest first, drawn as bars
    bars: [f32; 7],    // what is drawn, eased toward `history`
    at: u64,           // when this state began, for the clock and the transcribing dots
    pushed: u64,
    polled: u64,
    /// Where in its surface the pill sits, 0 left to 1 right: the way the surface sits on the
    /// screen, so a pill in a corner is in the corner, and grows away from its edge.
    lean: f32,
}

impl Pill {
    pub fn new() -> Pill {
        let raw = status::read().unwrap_or(Value::Null);
        let style = crate::setting("ROOKEY_PILL");
        let style = STYLES.into_iter().find(|&s| style.as_deref() == Some(s)).unwrap_or(STYLES[0]);
        let lean = (at().0 / 100.0) as f32;
        Pill { style, raw, shown: Value::Null, history: [0.0; 7], bars: [0.0; 7], at: 0, pushed: 0, polled: 0, lean }
    }

    /// Reads the status now and then and moves the animation. `false` once there is nothing
    /// left to show.
    pub fn tick(&mut self, now: u64) -> bool {
        if now.saturating_sub(self.polled) >= 50 {
            self.polled = now;
            self.raw = status::read().unwrap_or(self.raw.take());
        }
        let shown = status::current(&self.raw, now, running);
        let state = shown["state"].as_str().unwrap_or("idle").to_string();
        if state != self.shown["state"].as_str().unwrap_or("") {
            self.at = self.raw["at"].as_u64().unwrap_or(now);
            self.history = [0.0; 7];
        }
        if let Some(level) = shown["level"].as_f64() {
            // the meter scrolls at a steady pace, whatever the level does
            if now.saturating_sub(self.pushed) >= 125 {
                self.pushed = now;
                self.history.rotate_left(1);
                self.history[6] = level as f32;
            }
        }
        for (bar, target) in self.bars.iter_mut().zip(self.history) {
            *bar += (target - *bar) * 0.25;
        }
        self.shown = shown;
        match state.as_str() {
            "idle" => false,
            "failed" => now.saturating_sub(self.raw["at"].as_u64().unwrap_or(0)) < FAILED_MS,
            _ => true,
        }
    }

    /// Draws into `px` (premultiplied BGRA, `W*scale` by `H*scale`, cleared here).
    pub fn draw(&self, px: &mut [u8], scale: u32, now: u64) {
        let mut c = Canvas { px, w: (W * scale) as usize, h: (H * scale) as usize, s: scale as f32 };
        c.px.fill(0);
        let state = self.shown["state"].as_str().unwrap_or("idle");
        let words = |key: &str| say(key, &self.shown);
        let full = self.style == "full";
        // a failure is always the whole pill: the reason is the point
        if self.style == "dot" && state != "failed" {
            let r = H as f32 / 2.0;
            let (color, size) = match state {
                "listening" => (RED, 6.0 + 2.0 * self.bars[6].sqrt().min(1.0)),
                "transcribing" => (INK, 6.0),
                _ => return, // typed: the words on screen say it
            };
            // by its widest, so it stays put while it swells, and a corner dot is in the corner
            let outer = 8.0 + 3.0;
            let x = outer + (W as f32 - 2.0 * outer) * self.lean;
            c.capsule((x, r), (x, r), size + 3.0, PAPER, 1.0);
            c.capsule((x, r), (x, r), size, color, 1.0);
            return;
        }
        let (fill, content): (_, Vec<Part>) = match state {
            "listening" if full => {
                let clock = status::clock(now.saturating_sub(self.at) / 1000);
                (INK, vec![Part::Dot, Part::Bars, Part::Digits(clock)])
            }
            "listening" => (INK, vec![Part::Dot, Part::Bars]),
            "transcribing" if full => (INK, vec![Part::Dots, Part::Words(words("transcribing"))]),
            "transcribing" => (INK, vec![Part::Dots]),
            "typed" if full => (INK, vec![Part::Tick, Part::Words(words("typed"))]),
            "typed" => (INK, vec![Part::Tick]),
            "failed" => (ALARM, vec![Part::Mark, Part::Words(self.shown["reason"].as_str().unwrap_or("").into())]),
            _ => return,
        };
        let (pad_l, pad_r, gap) = (14.0, 16.0, 10.0);
        let room = W as f32 - pad_l - pad_r - 2.0; // the hairline stays inside the surface
        let gaps = gap * (content.len() - 1) as f32;
        // one part alone sits in a circle-ish pill, as wide as it is tall at least
        let (pad_l, pad_r) = if content.len() == 1 {
            let p = (H as f32 - content[0].width(0.0)) / 2.0;
            (p, p)
        } else {
            (pad_l, pad_r)
        };
        let fixed = content.iter().filter(|p| !p.is_text()).map(|p| p.width(0.0)).sum::<f32>() + gaps;
        let widths: Vec<f32> = content.iter().map(|p| p.width(room - fixed)).collect();
        let pill_w = pad_l + widths.iter().sum::<f32>() + gaps + pad_r;
        let x0 = (W as f32 - pill_w) * self.lean;
        let r = H as f32 / 2.0;
        // a hairline of paper around the ink, so it reads on a dark wallpaper too
        c.capsule((x0 + r, r), (x0 + pill_w - r, r), r, PAPER, 0.18);
        c.capsule((x0 + r, r), (x0 + pill_w - r, r), r - 1.0, fill, 1.0);

        let mut x = x0 + pad_l;
        let t = now.saturating_sub(self.at) as f32 / 1000.0;
        for (part, w) in content.iter().zip(widths) {
            match part {
                Part::Dot => c.capsule((x + 4.0, r), (x + 4.0, r), 4.0, RED, 1.0),
                Part::Bars => {
                    for (i, level) in self.bars.iter().enumerate() {
                        let h = 4.0 + 16.0 * (level.sqrt() * 1.3).min(1.0);
                        let bx = x + 1.5 + i as f32 * 6.0;
                        c.capsule((bx, r - h / 2.0 + 1.5), (bx, r + h / 2.0 - 1.5), 1.5, PAPER, 1.0);
                    }
                }
                Part::Dots => {
                    for i in 0..3 {
                        // a wave walks along the three dots
                        let a = 0.3 + 0.7 * (0.5 + 0.5 * (t * 5.0 - i as f32 * 0.9).cos());
                        c.capsule((x + 2.5 + i as f32 * 9.0, r), (x + 2.5 + i as f32 * 9.0, r), 2.5, PAPER, a);
                    }
                }
                Part::Tick => {
                    let p = |dx: f32, dy: f32| (x + dx, r - 8.0 + dy);
                    c.capsule(p(3.0, 8.5), p(6.2, 11.7), 1.0, PAPER, 1.0);
                    c.capsule(p(6.2, 11.7), p(13.0, 5.0), 1.0, PAPER, 1.0);
                }
                Part::Mark => {
                    c.capsule((x + 8.0, r - 4.5), (x + 8.0, r + 1.0), 1.0, PAPER, 1.0);
                    c.capsule((x + 8.0, r + 4.2), (x + 8.0, r + 4.3), 1.0, PAPER, 1.0);
                }
                Part::Words(s) => c.text(&*WORDS, s, 13.0, x, w),
                Part::Digits(s) => c.text(&*DIGITS, s, 12.5, x, w),
            }
            x += w + gap;
        }
    }
}

fn running(pid: u32) -> bool {
    crate::alive(pid)
}

enum Part {
    Dot,
    Bars,
    Dots,
    Tick,
    Mark,
    Words(String),
    Digits(String),
}

impl Part {
    fn is_text(&self) -> bool {
        matches!(self, Part::Words(_) | Part::Digits(_))
    }

    /// Logical width; text takes at most `room` and is cut with an ellipsis past it.
    fn width(&self, room: f32) -> f32 {
        match self {
            Part::Dot => 8.0,
            Part::Bars => 39.0,
            Part::Dots => 23.0,
            Part::Tick | Part::Mark => 16.0,
            Part::Words(s) => fit(&*WORDS, s, 13.0, room).1,
            Part::Digits(s) => fit(&*DIGITS, s, 12.5, room).1,
        }
    }
}

/// The part of `s` that fits in `room` at `size`, with "…" if it was cut, and its width.
fn fit(fonts: &[Font], s: &str, size: f32, room: f32) -> (String, f32) {
    let advance = |c: char| glyph_font(fonts, c).metrics(c, size).advance_width;
    let full: f32 = s.chars().map(advance).sum();
    if full <= room {
        return (s.to_string(), full);
    }
    let (mut out, mut w) = (String::new(), advance('…'));
    for c in s.chars() {
        if w + advance(c) > room {
            break;
        }
        w += advance(c);
        out.push(c);
    }
    (out.trim_end().to_string() + "…", w)
}

fn glyph_font(fonts: &[Font], c: char) -> &Font {
    fonts.iter().find(|f| f.lookup_glyph_index(c) != 0).unwrap_or(&fonts[0])
}

/// The words on the pill, in rookey's language.
fn say(key: &str, shown: &Value) -> String {
    let n = shown["words"].as_u64().unwrap_or(0);
    match key {
        "transcribing" => crate::i18n::t("pill.transcribing", &[]),
        "typed" => crate::i18n::t("pill.typed", &[("n", &n)]),
        _ => String::new(),
    }
}

struct Canvas<'a> {
    px: &'a mut [u8],
    w: usize,
    h: usize,
    s: f32,
}

impl Canvas<'_> {
    /// Paints `color` over one pixel with coverage `a` (premultiplied "over").
    fn blend(&mut self, x: usize, y: usize, color: [f32; 3], a: f32) {
        if a <= 0.0 || x >= self.w || y >= self.h {
            return;
        }
        let i = (y * self.w + x) * 4;
        let p = &mut self.px[i..i + 4];
        let keep = 1.0 - a;
        // BGRA
        for (ch, c) in [(0, color[2]), (1, color[1]), (2, color[0])] {
            p[ch] = (c * a * 255.0 + p[ch] as f32 * keep).round() as u8;
        }
        p[3] = (a * 255.0 + p[3] as f32 * keep).round() as u8;
    }

    /// A segment from `a` to `b` with round ends of radius `r`, anti-aliased. With a == b it's
    /// a dot; long and fat it's the pill. Logical coordinates.
    fn capsule(&mut self, a: (f32, f32), b: (f32, f32), r: f32, color: [f32; 3], alpha: f32) {
        let s = self.s;
        let (ax, ay, bx, by, r) = (a.0 * s, a.1 * s, b.0 * s, b.1 * s, r * s);
        let (x0, x1) = ((ax.min(bx) - r - 1.0).max(0.0) as usize, (ax.max(bx) + r + 1.0) as usize);
        let (y0, y1) = ((ay.min(by) - r - 1.0).max(0.0) as usize, (ay.max(by) + r + 1.0) as usize);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for y in y0..y1.min(self.h) {
            for x in x0..x1.min(self.w) {
                let (px, py) = (x as f32 + 0.5 - ax, y as f32 + 0.5 - ay);
                let t = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
                let d = ((px - t * dx).powi(2) + (py - t * dy).powi(2)).sqrt() - r;
                self.blend(x, y, color, (0.5 - d).clamp(0.0, 1.0) * alpha);
            }
        }
    }

    /// Text starting at `x`, vertically centred on the pill, cut to `room`.
    fn text(&mut self, fonts: &[Font], s: &str, size: f32, x: f32, room: f32) {
        let (s, _) = fit(fonts, s, size, room + 0.5);
        let px_size = size * self.s;
        let line = fonts[0].horizontal_line_metrics(px_size);
        let (ascent, descent) = line.map_or((px_size * 0.8, -px_size * 0.2), |l| (l.ascent, l.descent));
        let baseline = (self.h as f32 + ascent + descent) / 2.0;
        let mut pen = x * self.s;
        for c in s.chars() {
            let (m, cover) = glyph_font(fonts, c).rasterize(c, px_size);
            let left = pen.round() as i32 + m.xmin;
            let top = (baseline.round() as i32) - m.ymin - m.height as i32;
            for (i, &a) in cover.iter().enumerate() {
                let (gx, gy) = (left + (i % m.width.max(1)) as i32, top + (i / m.width.max(1)) as i32);
                if gx >= 0 && gy >= 0 {
                    self.blend(gx as usize, gy as usize, PAPER, a as f32 / 255.0);
                }
            }
            pen += m.advance_width;
        }
    }
}

#[cfg(target_os = "linux")]
mod wayland {
    //! The pill as a layer-shell surface: above every window, tiled with none, never focused,
    //! clicks pass through. niri, Hyprland, sway and KDE all speak layer shell; GNOME doesn't.

    use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region};
    use smithay_client_toolkit::output::{OutputHandler, OutputState};
    use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
    use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
    use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
    use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
    use smithay_client_toolkit::shell::WaylandSurface;
    use smithay_client_toolkit::shell::wlr_layer::{
        Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
    };
    use smithay_client_toolkit::shm::slot::SlotPool;
    use smithay_client_toolkit::shm::{Shm, ShmHandler};
    use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};

    use super::{DEFAULT_AT, H, MARGIN, Pill, Res, W, spot};
    use crate::status::now_ms;

    struct App {
        registry: RegistryState,
        outputs: OutputState,
        shm: Shm,
        pool: SlotPool,
        layer: LayerSurface,
        pill: Pill,
        scale: u32,
        /// Somewhere other than the default: the first configure is the usable area's size,
        /// asked for by stretching to every edge, and the pill is placed in it from there.
        at: Option<(f64, f64)>,
        configured: bool,
        drawn: u64,
        exit: bool,
    }

    pub fn run() -> Res {
        let conn = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init(&conn)?;
        let qh = queue.handle();
        let compositor = CompositorState::bind(&globals, &qh)?;
        let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| crate::t!("overlay.no-layer-shell"))?;
        let shm = Shm::bind(&globals, &qh)?;

        let surface = compositor.create_surface(&qh);
        // an empty input region: the pill never takes a click
        surface.set_input_region(Some(Region::new(&compositor)?.wl_region()));
        let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("rookey"), None);
        let at = Some(super::at()).filter(|&at| at != DEFAULT_AT);
        if at.is_some() {
            // the layer shell anchors to edges and never says how big the screen is: a surface
            // on all four edges is told the size of what bars leave free, which is the area
            layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            layer.set_size(0, 0);
        } else {
            layer.set_anchor(Anchor::BOTTOM);
            layer.set_margin(0, 0, MARGIN, 0);
            layer.set_size(W, H);
        }
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();

        let pool = SlotPool::new((W * H * 4) as usize, &shm)?;
        let mut app = App {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &qh),
            shm,
            pool,
            layer,
            pill: Pill::new(),
            scale: 1,
            at,
            configured: false,
            drawn: 0,
            exit: false,
        };
        // A pill that isn't on any screen gets no frame callbacks and would wait forever:
        // a watcher ends it once rookey has been idle a while.
        std::thread::spawn(|| {
            let mut pill = Pill::new();
            let mut idle_since = None;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let now = now_ms();
                match (pill.tick(now), idle_since) {
                    (true, _) => idle_since = None,
                    (false, None) => idle_since = Some(now),
                    (false, Some(t)) if now - t > 3000 => std::process::exit(0),
                    _ => {}
                }
            }
        });
        while !app.exit {
            queue.blocking_dispatch(&mut app)?;
        }
        Ok(())
    }

    impl App {
        fn draw(&mut self, qh: &QueueHandle<Self>) {
            let now = now_ms();
            let surface = self.layer.wl_surface().clone();
            if !self.pill.tick(now) {
                self.exit = true;
                return;
            }
            // ~30 frames a second is plenty for a meter; skip the rest of a 180 Hz screen's
            if now.saturating_sub(self.drawn) >= 30 {
                self.drawn = now;
                let (w, h) = ((W * self.scale) as i32, (H * self.scale) as i32);
                let Ok((buffer, canvas)) = self.pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else {
                    self.exit = true;
                    return;
                };
                self.pill.draw(canvas, self.scale, now);
                surface.set_buffer_scale(self.scale as i32);
                surface.damage_buffer(0, 0, w, h);
                if buffer.attach_to(&surface).is_err() {
                    self.exit = true;
                    return;
                }
            }
            surface.frame(qh, FrameCallbackData(surface.clone()));
            self.layer.commit();
        }
    }

    impl CompositorHandler for App {
        // ponytail: whole-number scales; wp_fractional_scale if 1.25x screens look soft
        fn scale_factor_changed(
            &mut self,
            _: &Connection,
            qh: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            factor: i32,
        ) {
            self.scale = factor.max(1) as u32;
            self.drawn = 0;
            if self.configured {
                self.draw(qh);
            }
        }
        fn transform_changed(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: wl_output::Transform,
        ) {
        }
        fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
            self.draw(qh);
        }
        fn surface_enter(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: &wl_output::WlOutput,
        ) {
        }
        fn surface_leave(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: &wl_surface::WlSurface,
            _: &wl_output::WlOutput,
        ) {
        }
    }

    impl LayerShellHandler for App {
        fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
            self.exit = true;
        }
        fn configure(
            &mut self,
            _: &Connection,
            qh: &QueueHandle<Self>,
            _: &LayerSurface,
            configure: LayerSurfaceConfigure,
            _: u32,
        ) {
            if let Some(at) = self.at.take() {
                let (w, h) = configure.new_size;
                if w > 0 && h > 0 {
                    let (x, y) = spot(w as f64, h as f64, 1.0, at);
                    self.layer.set_anchor(Anchor::TOP | Anchor::LEFT);
                    self.layer.set_margin(y.round() as i32, 0, 0, x.round() as i32);
                } else {
                    self.layer.set_anchor(Anchor::BOTTOM);
                    self.layer.set_margin(0, 0, MARGIN, 0);
                }
                self.layer.set_size(W, H);
                // drawn once the compositor answers this one
                self.layer.commit();
                return;
            }
            if !self.configured {
                self.configured = true;
                self.draw(qh);
            }
        }
    }

    impl OutputHandler for App {
        fn output_state(&mut self) -> &mut OutputState {
            &mut self.outputs
        }
        fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
        fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
        fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    }

    impl ShmHandler for App {
        fn shm_state(&mut self) -> &mut Shm {
            &mut self.shm
        }
    }

    impl ProvidesRegistryState for App {
        fn registry(&mut self) -> &mut RegistryState {
            &mut self.registry
        }
        registry_handlers![OutputState];
    }

    delegate_registry!(App);
    delegate_dispatch2!(App);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pill_in_a_corner_is_in_the_corner() {
        // the first and last columns with anything drawn, at 1x
        let drawn = |lean: f32, style: &'static str| {
            let mut pill = Pill::new();
            pill.shown = serde_json::json!({"state": "listening", "level": 0.5});
            (pill.style, pill.lean) = (style, lean);
            let mut px = vec![0u8; (W * H * 4) as usize];
            pill.draw(&mut px, 1, 0);
            let lit: Vec<u32> = (0..W).filter(|&x| (0..H).any(|y| px[((y * W + x) * 4 + 3) as usize] > 0)).collect();
            (*lit.first().unwrap(), *lit.last().unwrap())
        };
        for style in STYLES {
            let (left, _) = drawn(0.0, style);
            let (_, right) = drawn(1.0, style);
            let (l, r) = drawn(0.5, style);
            // the dot keeps room to swell into, a few pixels at its quietest
            assert!(left <= 3 && right >= W - 4, "{style}: {left}..{right}");
            assert!(l.abs_diff(W - 1 - r) <= 2, "{style} centred: {l}..{r}");
        }
    }

    #[test]
    fn where_the_pill_goes() {
        assert_eq!(parse_at("50,100"), Some((50.0, 100.0)));
        assert_eq!(parse_at(" 12.5 , 0 "), Some((12.5, 0.0)));
        for bad in ["", "50", "50,", "a,b", "-1,50", "50,101", "50;100", "NaN,1"] {
            assert_eq!(parse_at(bad), None, "{bad}");
        }
        // the default is where the pill always was: centred, MARGIN above the bottom
        let (x, y) = spot(1512.0, 900.0, 1.0, DEFAULT_AT);
        assert_eq!((x, y), ((1512.0 - W as f64) / 2.0, 900.0 - H as f64 - MARGIN as f64));
        // the corners keep the margin, at 2x too
        assert_eq!(spot(3024.0, 1800.0, 2.0, (0.0, 0.0)), (2.0 * MARGIN as f64, 2.0 * MARGIN as f64));
        let (x, y) = spot(3024.0, 1800.0, 2.0, (100.0, 100.0));
        assert_eq!(
            (x + 2.0 * W as f64, y + 2.0 * H as f64),
            (3024.0 - 2.0 * MARGIN as f64, 1800.0 - 2.0 * MARGIN as f64)
        );
        // a screen too small for the margins puts it at the margin, not off screen
        assert_eq!(spot(100.0, 20.0, 1.0, (100.0, 100.0)), (MARGIN as f64, MARGIN as f64));
    }
    use serde_json::json;

    #[test]
    fn draws_each_state() {
        let mut px = vec![0u8; (W * H * 4 * 4) as usize];
        for shown in [
            json!({"state": "listening", "seconds": 4, "level": 0.4}),
            json!({"state": "transcribing"}),
            json!({"state": "typed", "words": 12}),
            json!({"state": "failed", "reason": "x".repeat(300)}), // too long: cut, not a panic
        ] {
            for style in STYLES {
                let pill = Pill { style, shown: shown.clone(), ..Pill::new() };
                pill.draw(&mut px, 2, 4_000);
                let centre = ((H as usize) * (W * 2) as usize + W as usize) * 4; // middle of the 2x buffer
                // the dot draws nothing once typed: the typed words say it
                let drawn = !(style == "dot" && shown["state"] == "typed");
                assert_eq!(px[centre + 3] == 255, drawn, "{style} {shown}: the middle");
                assert_eq!(&px[0..4], [0, 0, 0, 0], "the corner stays see-through");
            }
        }
        // a failure keeps its reason in every style: the pill is wider than the dot
        let failed = json!({"state": "failed", "reason": "mic is silent"});
        let pill = Pill { style: "dot", shown: failed, ..Pill::new() };
        pill.draw(&mut px, 2, 4_000);
        let row = (H as usize) * (W * 2) as usize * 4;
        let left = (W as usize - 60) * 4; // 30 logical pixels left of centre, at 2x
        assert_eq!(px[row + left + 3], 255);
    }

    #[test]
    fn cuts_long_words() {
        let (cut, w) = fit(&*WORDS, &"ElevenLabs has no key ".repeat(20), 13.0, 200.0);
        assert!(cut.ends_with('…') && w <= 200.0);
        assert_eq!(fit(&*WORDS, "12 words", 13.0, 200.0).0, "12 words");
    }
}
