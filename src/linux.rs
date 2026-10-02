//! Linux: what this desktop lets rookey do, asked of the Wayland compositor once, and typing.

use std::io::ErrorKind;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

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
    /// a Wayland desktop that keeps that protocol to itself: GNOME and KDE
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

/// Whether the compositor can show the pill: niri, Hyprland, sway and KDE can, GNOME can't.
pub fn layer_shell() -> bool {
    globals().is_some_and(|g| g.iter().any(|i| i == "zwlr_layer_shell_v1"))
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
        // ponytail: GNOME and KDE take the text only as a paste; pressing the paste keys
        // through /dev/uinput comes next. Until then the text waits in history.
        Typing::Paste => Err(t!("cli.linux-no-keyboard").into()),
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
}
