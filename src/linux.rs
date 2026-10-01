//! Linux: what this desktop lets rookey do, asked of the Wayland compositor once, and typing.

use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, KeyCode, KeyEvent};
use smithay_client_toolkit::reexports::client::globals::{GlobalListContents, registry_queue_init};
use smithay_client_toolkit::reexports::client::protocol::wl_registry::{self, WlRegistry};
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, QueueHandle};

use crate::{Res, t};

/// How text gets into the focused window here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Typing {
    /// wtype, through the virtual-keyboard protocol: Hyprland, niri, sway and the other wlroots
    /// desktops, COSMIC
    Wtype,
    /// a Wayland desktop that keeps that protocol to itself, GNOME and KDE: pasted
    Paste,
    /// no Wayland session to type into
    None,
}

/// The interfaces the Wayland compositor offers, asked once; None without a Wayland session.
fn globals() -> Option<&'static [String]> {
    static GLOBALS: OnceLock<Option<Vec<String>>> = OnceLock::new();
    GLOBALS.get_or_init(wayland_globals).as_deref()
}

fn wayland_globals() -> Option<Vec<String>> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, _queue) = registry_queue_init::<Probe>(&conn).ok()?;
    Some(globals.contents().clone_list().into_iter().map(|g| g.interface).collect())
}

/// registry_queue_init fills in the list of globals by itself: nothing to handle here.
struct Probe;

impl Dispatch<WlRegistry, GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn offers(interface: &str) -> bool {
    globals().is_some_and(|g| g.iter().any(|i| i == interface))
}

/// Whether the compositor can show the pill: niri, Hyprland, sway and KDE can, GNOME can't.
pub fn layer_shell() -> bool {
    offers("zwlr_layer_shell_v1")
}

pub fn typing() -> Typing {
    pick(globals())
}

fn pick(globals: Option<&[String]>) -> Typing {
    match globals {
        Some(g) if g.iter().any(|i| i == "zwp_virtual_keyboard_manager_v1") => Typing::Wtype,
        Some(_) => Typing::Paste,
        None => Typing::None,
    }
}

/// Types text into the focused window, or says why it can't.
pub fn type_text(text: &str) -> Res<()> {
    let how = typing();
    vlog!(2, "typing: {how:?}");
    match how {
        Typing::Wtype => run("wtype", Command::new("wtype").args(["--", text])),
        Typing::Paste => paste(text),
        Typing::None => Err(t!("cli.linux-no-wayland").into()),
    }
}

/// Runs a tool that types. When it fails, its own first words say why.
fn run(tool: &str, cmd: &mut Command) -> Res<()> {
    let out = match cmd.stdin(Stdio::null()).output() {
        Ok(out) => out,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(t!("cli.linux-no-tool", tool = tool).into()),
        Err(e) => return Err(e.into()),
    };
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let why =
        stderr.lines().map(str::trim).find(|l| !l.is_empty()).map_or_else(|| out.status.to_string(), str::to_string);
    Err(t!("cli.linux-tool-failed", tool = tool, why = why).into())
}

/// One udev rule opens /dev/uinput (the paste keys) and the keyboards (the hotkey) to whoever
/// sits at this computer, as the `input` group would but for the active session only, and
/// without logging out. It has to sort before systemd's 73-seat-late.rules, which hands out
/// what it tags.
pub const ACCESS_FIX: &str = r#"printf '%s\n' 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' 'SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_KEYBOARD}=="1", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/70-rookey.rules >/dev/null && sudo modprobe uinput && sudo udevadm control --reload && sudo udevadm trigger --settle --subsystem-match=misc --subsystem-match=input"#;

/// Whether rookey may make the virtual keyboard that presses the paste keys.
pub fn uinput_ok() -> bool {
    OpenOptions::new().write(true).open("/dev/uinput").is_ok()
}

/// The virtual keyboard, made as the recording starts so the compositor has taken it in by
/// the time there is text; with when it was made.
static KEYBOARD: Mutex<Option<(VirtualDevice, Instant)>> = Mutex::new(None);

/// Where rookey pastes, makes the virtual keyboard now, on a thread: typing won't wait for it.
pub fn prepare() {
    thread::spawn(|| {
        if typing() != Typing::Paste {
            return;
        }
        match make_keyboard() {
            Ok(keyboard) => *KEYBOARD.lock().unwrap() = Some((keyboard, Instant::now())),
            Err(e) => vlog!(1, "typing: no virtual keyboard: {e}"),
        }
    });
}

fn make_keyboard() -> io::Result<VirtualDevice> {
    // udev calls a device a keyboard only if it has every key from Esc to S; one that isn't
    // a keyboard may be left alone. They're never pressed.
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in 1..=31 {
        keys.insert(KeyCode::new(code));
    }
    keys.insert(KeyCode::KEY_LEFTSHIFT);
    keys.insert(KeyCode::KEY_INSERT);
    VirtualDevice::builder()?.name("rookey").with_keys(&keys)?.build()
}

/// The prepared virtual keyboard, or a new one.
fn keyboard() -> io::Result<VirtualDevice> {
    let (keyboard, made) = match KEYBOARD.lock().unwrap().take() {
        Some(prepared) => prepared,
        None => (make_keyboard()?, Instant::now()),
    };
    // ponytail: the compositor ignores a new device until it has opened it; 300 ms covers
    // that. Way up: one kept open by `rookey listen`.
    thread::sleep(Duration::from_millis(300).saturating_sub(made.elapsed()));
    Ok(keyboard)
}

/// Waits, at most `within`, until no keyboard holds Shift, Ctrl, Alt or Super down: with the
/// hotkey's modifiers still held, the paste would be another shortcut. A keyboard rookey can't
/// read doesn't count.
fn modifiers_up(within: Duration) {
    use KeyCode as K;
    const HELD: [KeyCode; 8] = [
        K::KEY_LEFTSHIFT,
        K::KEY_RIGHTSHIFT,
        K::KEY_LEFTCTRL,
        K::KEY_RIGHTCTRL,
        K::KEY_LEFTALT,
        K::KEY_RIGHTALT,
        K::KEY_LEFTMETA,
        K::KEY_RIGHTMETA,
    ];
    let keyboards: Vec<_> = evdev::enumerate()
        .map(|(_, device)| device)
        .filter(|d| d.name() != Some("rookey") && d.supported_keys().is_some_and(|k| k.contains(K::KEY_LEFTSHIFT)))
        .collect();
    let held = || keyboards.iter().any(|d| d.get_key_state().is_ok_and(|down| HELD.iter().any(|&k| down.contains(k))));
    let until = Instant::now() + within;
    while held() && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
}

/// Shift+Insert: the paste key in every layout, and in terminals, which paste the primary
/// selection on it.
fn shift_insert(keyboard: &mut VirtualDevice) -> io::Result<()> {
    for (key, value) in
        [(KeyCode::KEY_LEFTSHIFT, 1), (KeyCode::KEY_INSERT, 1), (KeyCode::KEY_INSERT, 0), (KeyCode::KEY_LEFTSHIFT, 0)]
    {
        keyboard.emit(&[*KeyEvent::new(key, value)])?;
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

/// Waits for a child, at most `within`. On a desktop that won't hand it the clipboard (GNOME
/// has no protocol for that), wl-clipboard can wait for the focus forever: it is stopped.
fn finish(mut child: Child, tool: &str, within: Duration) -> Res<ExitStatus> {
    let until = Instant::now() + within;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err(t!("cli.linux-clipboard-stuck", tool = tool).into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Puts text on the clipboard, or on the primary selection.
fn copy(primary: bool, text: &[u8]) -> Res<()> {
    let mut cmd = Command::new("wl-copy");
    if primary {
        cmd.arg("--primary");
    }
    // wl-copy leaves a server behind to hand the text out, and that keeps whatever it was given:
    // nothing to read here, and no pipe to wait on
    let mut child = match cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(t!("cli.linux-no-tool", tool = "wl-copy").into()),
        Err(e) => return Err(e.into()),
    };
    child.stdin.take().unwrap().write_all(text)?;
    let status = finish(child, "wl-copy", Duration::from_secs(3))?;
    if !status.success() {
        return Err(t!("cli.linux-tool-failed", tool = "wl-copy", why = status).into());
    }
    Ok(())
}

/// The clipboard's text, to put back after a paste. None for no text, or none to be had.
fn clipboard() -> Option<Vec<u8>> {
    let mut child = Command::new("wl-paste")
        .args(["--no-newline", "--type", "text"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reading = thread::spawn(move || {
        let mut text = Vec::new();
        out.read_to_end(&mut text).map(|_| text)
    });
    let status = finish(child, "wl-paste", Duration::from_secs(3)).ok()?;
    let text = reading.join().ok()?.ok()?;
    (status.success() && !text.is_empty()).then_some(text)
}

/// GNOME and KDE: the text goes on the clipboard and a virtual keyboard presses Shift+Insert,
/// the way macOS pastes. The primary selection gets it too, for terminals.
fn paste(text: &str) -> Res<()> {
    let saved = if crate::keep_clipboard() { clipboard() } else { None };
    copy(false, text.as_bytes())?;
    // only terminals want it; a desktop without one still takes the paste
    if let Err(e) = copy(true, text.as_bytes()) {
        vlog!(1, "typing: no primary selection here: {e}");
    }
    let mut keyboard = keyboard().map_err(|e| t!("cli.linux-no-uinput", why = e))?;
    if !offers("ext_data_control_manager_v1") && !offers("zwlr_data_control_manager_v1") {
        // ponytail: without a clipboard protocol for tools (GNOME), wl-copy takes the focus for
        // a moment to set it; this lets it come back first. Way up: own the selection through
        // Xwayland, which needs no focus.
        thread::sleep(Duration::from_millis(150));
    }
    modifiers_up(Duration::from_millis(500));
    shift_insert(&mut keyboard).map_err(|e| t!("cli.linux-no-uinput", why = e))?;
    if let Some(saved) = saved {
        // ponytail: the app reads the paste on its own time; 300 ms covers the usual ones, as
        // on macOS. Only text comes back, and the primary selection keeps the dictation.
        thread::sleep(Duration::from_millis(300));
        copy(false, &saved)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_follows_the_globals() {
        let offered = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(pick(Some(&offered(&["wl_seat", "zwp_virtual_keyboard_manager_v1"]))), Typing::Wtype);
        // KDE: a layer shell, but no virtual keyboard for others
        assert_eq!(pick(Some(&offered(&["wl_seat", "zwlr_layer_shell_v1"]))), Typing::Paste);
        assert_eq!(pick(Some(&[])), Typing::Paste);
        assert_eq!(pick(None), Typing::None);
    }

    #[test]
    fn a_failed_tool_says_why() {
        assert!(run("true", &mut Command::new("true")).is_ok());
        let failed = run("wtype", Command::new("sh").args(["-c", "echo; echo '  no keyboard here ' >&2; exit 3"]));
        let failed = failed.unwrap_err().to_string();
        assert!(failed.contains("wtype") && failed.contains("no keyboard here"), "{failed}");
        // a tool that says nothing: its exit status is the reason
        let quiet = run("sh", Command::new("sh").args(["-c", "exit 4"])).unwrap_err().to_string();
        assert!(quiet.contains('4'), "{quiet}");
        let missing = run("no-such-tool", &mut Command::new("rookey-no-such-tool")).unwrap_err().to_string();
        assert!(missing.contains("no-such-tool"), "{missing}");
    }

    /// Makes a real input device for a moment, though it presses nothing:
    /// `cargo test --release --features cuda -- --ignored virtual_keyboard`
    #[test]
    #[ignore]
    fn the_virtual_keyboard_is_a_keyboard() {
        let mut keyboard = make_keyboard().unwrap();
        let node = keyboard.enumerate_dev_nodes_blocking().unwrap().flatten().next().unwrap();
        Command::new("udevadm").arg("settle").status().unwrap();
        let info = Command::new("udevadm").args(["info", "--query=property", "--name"]).arg(&node).output().unwrap();
        let info = String::from_utf8_lossy(&info.stdout);
        assert!(info.contains("ID_INPUT_KEYBOARD=1"), "{}: {info}", node.display());
    }

    #[test]
    fn a_stuck_tool_is_stopped() {
        let started = Instant::now();
        let stuck = finish(Command::new("sleep").arg("5").spawn().unwrap(), "wl-copy", Duration::from_millis(100));
        assert!(stuck.unwrap_err().to_string().contains("wl-copy"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(finish(Command::new("true").spawn().unwrap(), "true", Duration::from_secs(2)).unwrap().success());
    }

    #[test]
    fn one_rule_for_the_paste_keys_and_the_hotkey() {
        let file = ACCESS_FIX.split_whitespace().find(|w| w.starts_with("/etc/udev/rules.d/")).unwrap();
        // systemd's 73-seat-late.rules hands out what was tagged before it, and only that
        assert!(file.rsplit('/').next().unwrap() < "73-seat-late.rules", "{file}");
        for part in
            ["static_node=uinput", "ENV{ID_INPUT_KEYBOARD}==\"1\"", "TAG+=\"uaccess\"", "modprobe uinput", "--settle"]
        {
            assert!(ACCESS_FIX.contains(part), "{part}");
        }
    }
}
