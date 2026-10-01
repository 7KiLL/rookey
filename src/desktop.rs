//! What differs from one desktop to the next: which output has the focus, and where a
//! hotkey is declared. A hotkey is written into the compositor's own config, checked with
//! the compositor's own validator, and put back as it was if that finds a fault.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use serde_json::{Value, json};

use crate::{Res, t};

#[derive(Clone, Copy, PartialEq)]
pub enum Desktop {
    Niri,
    Hyprland,
    MacOs,
    Other,
}

pub fn detect() -> Desktop {
    if cfg!(target_os = "macos") {
        Desktop::MacOs
    } else if env::var_os("NIRI_SOCKET").is_some() {
        Desktop::Niri
    } else if env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        Desktop::Hyprland
    } else {
        Desktop::Other
    }
}

impl Desktop {
    pub fn id(self) -> &'static str {
        match self {
            Desktop::Niri => "niri",
            Desktop::Hyprland => "hyprland",
            Desktop::MacOs => "macos",
            Desktop::Other => "other",
        }
    }

    /// The name of the output with the focus, for a screenshot of that one alone.
    pub fn focused_output(self) -> Option<String> {
        let (program, args): (_, &[&str]) = match self {
            Desktop::Niri => ("niri", &["msg", "-j", "focused-output"]),
            Desktop::Hyprland => ("hyprctl", &["monitors", "-j"]),
            _ => return None,
        };
        let out = Command::new(program).args(args).output().ok()?;
        let json: Value = serde_json::from_slice(&out.stdout).ok()?;
        let output = match self {
            Desktop::Niri => &json,
            _ => json.as_array()?.iter().find(|monitor| monitor["focused"] == true)?,
        };
        Some(output["name"].as_str()?.to_string())
    }
}

/// Modifiers and one key, the key by its XKB name: what both niri and Hyprland bind.
#[derive(Debug, PartialEq)]
pub struct Chord {
    mods: Vec<&'static str>, // in the order of MODS
    key: String,
}

pub const MODS: [&str; 4] = ["Super", "Ctrl", "Alt", "Shift"];

impl Chord {
    /// Reads "Super+Shift+D", and the spellings configs use: "Mod+Shift+D", "SUPER SHIFT, D".
    pub fn parse(text: &str) -> Res<Chord> {
        let mut mods = Vec::new();
        let mut key = None;
        for part in text.split(['+', ',', ' ']).filter(|p| !p.is_empty()) {
            let named = match part.to_ascii_lowercase().as_str() {
                "super" | "mod" | "win" | "mod4" | "meta" => Some("Super"),
                "ctrl" | "control" => Some("Ctrl"),
                "alt" | "mod1" => Some("Alt"),
                "shift" => Some("Shift"),
                _ => None,
            };
            let is_key = part.len() <= 32 && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            match named {
                Some(name) if !mods.contains(&name) => mods.push(name),
                Some(_) => {}
                // this goes into a config file, so nothing but a plain key name gets through
                None if key.is_none() && is_key => key = Some(part.to_string()),
                None => return Err(t!("desktop.not-chord", text = format!("{text:?}")).into()),
            }
        }
        let key = key.ok_or_else(|| t!("desktop.only-mods", text = format!("{text:?}")))?;
        mods.sort_by_key(|name| MODS.iter().position(|m| m == name));
        Ok(Chord { mods, key })
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn mods(&self) -> &[&'static str] {
        &self.mods
    }

    /// A key that types, with at most Shift, would swallow ordinary typing.
    pub fn check(&self) -> Res<()> {
        const TYPING: [&str; 14] = [
            "grave",
            "minus",
            "equal",
            "bracketleft",
            "bracketright",
            "backslash",
            "semicolon",
            "apostrophe",
            "comma",
            "period",
            "slash",
            "space",
            "Return",
            "Tab",
        ];
        let bare = self.mods.iter().all(|&m| m == "Shift");
        let types = self.key.chars().count() == 1
            || self.key.starts_with("KP_")
            || TYPING.iter().any(|k| k.eq_ignore_ascii_case(&self.key));
        if bare && types {
            return Err(t!("desktop.types", chord = self).into());
        }
        Ok(())
    }

    fn same(&self, other: &Chord) -> bool {
        self.mods == other.mods && self.key.eq_ignore_ascii_case(&other.key)
    }

    fn joined(&self, sep: &str, upper: bool) -> String {
        let mods = self.mods.iter().map(|m| if upper { m.to_uppercase() } else { m.to_string() });
        mods.chain([self.key.clone()]).collect::<Vec<_>>().join(sep)
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.joined("+", false))
    }
}

/// Where the hotkey can go on this desktop.
struct Config {
    desktop: Desktop,
    main: PathBuf,
    /// Files the hotkey may be written into: the main config and what it pulls in.
    files: Vec<PathBuf>,
    /// Why this desktop's config can't be written, if it can't.
    blocked: Option<String>,
}

const MARK: &str = "rookey hotkey";

impl Config {
    fn find() -> Option<Config> {
        let config = dirs::config_dir()?;
        match detect() {
            Desktop::Niri => {
                let main = env::var_os("NIRI_CONFIG")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| config.join("niri").join("config.kdl"));
                let mut files = Vec::new();
                niri_includes(&main, &mut files);
                // rookey's own file is written whole, never a place to put the include
                files.retain(|file| *file != main.with_file_name("rookey.kdl"));
                // An include of a missing file stops niri from loading its config. 26.04
                // can mark one optional, and nothing older gets an include from here.
                let version = niri_version();
                let blocked = match version {
                    _ if !main.is_file() => Some(t!("desktop.niri-defaults", path = tilde(&main))),
                    Some(v) if v >= (26, 4) => None,
                    Some((major, minor)) => Some(t!("desktop.niri-old", version = format!("{major}.{minor:02}"))),
                    None => Some(t!("desktop.niri-no-version")),
                };
                Some(Config { desktop: Desktop::Niri, main, files, blocked })
            }
            Desktop::Hyprland => {
                let dir = config.join("hypr");
                let main = ["hyprland.lua", "hyprland.conf"].iter().map(|f| dir.join(f)).find(|p| p.is_file())?;
                let kind = main.extension()?.to_owned();
                let mut files: Vec<PathBuf> = fs::read_dir(&dir)
                    .ok()?
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_file() && path.extension() == Some(&kind))
                    .collect();
                files.sort();
                Some(Config { desktop: Desktop::Hyprland, main, files, blocked: None })
            }
            _ => None,
        }
    }

    /// The file that is the user's own by its name, else the main one.
    fn default_file(&self) -> &Path {
        let own = self.files.iter().find(|f| f.file_stem().is_some_and(|stem| stem == "user"));
        own.unwrap_or(&self.main)
    }

    fn is_lua(&self) -> bool {
        self.main.extension().is_some_and(|e| e == "lua")
    }

    /// niri keeps the bind in a file of its own, next to the main config.
    fn niri_file(&self) -> PathBuf {
        self.main.with_file_name("rookey.kdl")
    }

    /// The hotkey as it stands: (chord, the file that holds or includes it).
    fn bound(&self) -> Option<(String, PathBuf)> {
        match self.desktop {
            Desktop::Niri => {
                let text = fs::read_to_string(self.niri_file()).ok()?;
                let chord = text.lines().find_map(|l| l.strip_prefix("// chord: "))?;
                let file = self
                    .files
                    .iter()
                    .find(|f| fs::read_to_string(f).is_ok_and(|text| text.lines().any(is_niri_include)))?;
                Some((chord.to_string(), file.clone()))
            }
            _ => self.files.iter().find_map(|file| {
                let text = fs::read_to_string(file).ok()?;
                let line = text.lines().find(|l| is_mark(l) && !l.contains("end"))?;
                let chord = line.split_once(MARK)?.1.split(':').next()?.trim();
                Some((chord.to_string(), file.clone()))
            }),
        }
    }

    /// Another bind of the same keys: (file, line number, the line). With `noop_too` false,
    /// a bind that only keeps the keys from the windows (`spawn "true"`) doesn't count.
    fn taken(&self, chord: &Chord, noop_too: bool) -> Option<(PathBuf, usize, String)> {
        for file in &self.files {
            let Ok(text) = fs::read_to_string(file) else { continue };
            let mut ours = false;
            for (n, line) in text.lines().enumerate() {
                if is_mark(line) {
                    ours = !line.contains("end");
                    continue;
                }
                let line = line.trim();
                // the keys of a bind, in each config's own notation
                let keys: Option<String> = match self.desktop {
                    Desktop::Niri if line.starts_with("//") => None,
                    // Mod+Shift+D { ... }
                    Desktop::Niri => line.split([' ', '\t', '{']).next().map(str::to_string),
                    // hl.bind("SUPER + SHIFT + D", ...)
                    _ if self.is_lua() => line
                        .split_once("hl.bind(\"")
                        .filter(|(before, _)| !before.contains("--"))
                        .and_then(|(_, rest)| rest.split('"').next())
                        .map(str::to_string),
                    // bind = SUPER SHIFT, D, exec, ...
                    _ => line
                        .strip_prefix("bind")
                        .and_then(|rest| rest.split_once('='))
                        .map(|(_, keys)| keys.split(',').take(2).collect::<Vec<_>>().join(" ")),
                };
                let found = keys.and_then(|keys| Chord::parse(&keys).ok());
                let noop = ["spawn \"true\"", "exec_cmd(\"true\")", "exec, true"].iter().any(|n| line.contains(n));
                if !ours && found.is_some_and(|found| found.same(chord)) && (noop_too || !noop) {
                    return Some((file.clone(), n + 1, line.to_string()));
                }
            }
        }
        None
    }

    fn validate(&self, file: &Path) -> Res<()> {
        let (program, args): (_, &[&str]) = match self.desktop {
            Desktop::Niri => ("niri", &["validate", "-c"]),
            _ => ("Hyprland", &["--verify-config", "-c"]),
        };
        let out = Command::new(program)
            .args(args)
            .arg(file)
            .output()
            .map_err(|e| t!("desktop.no-check", program = program, why = e))?;
        if out.status.success() {
            return Ok(());
        }
        let said = [out.stdout, out.stderr].concat();
        let said = strip_colors(&String::from_utf8_lossy(&said));
        Err(t!("desktop.refused", program = program, why = reason(&said)).into())
    }
}

/// The part of a validator's report that says what is wrong.
fn reason(report: &str) -> String {
    let lines: Vec<&str> = report.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    // niri chains causes with arrows, the last one is the root
    if let Some(cause) = lines.iter().rev().find_map(|l| l.split_once("▶ ")) {
        return cause.1.to_string();
    }
    // Hyprland lists the faults under a heading
    let faults = lines.iter().skip_while(|l| !l.contains("parsing result")).skip(1);
    let faults: Vec<&str> = faults.copied().collect();
    if faults.is_empty() { lines.join(" ") } else { faults.join(" ") }
}

fn is_ours(line: &str) -> bool {
    is_mark(line) || is_niri_include(line)
}

fn is_mark(line: &str) -> bool {
    let line = line.trim_start();
    ["//", "--", "#"].iter().any(|c| line.strip_prefix(c).is_some_and(|l| l.trim_start().starts_with(MARK)))
}

fn is_niri_include(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("include ") && included(line).is_some_and(|path| path.ends_with("rookey.kdl"))
}

/// The path in an `include "path"` line.
fn included(line: &str) -> Option<&str> {
    line.trim().strip_prefix("include")?.split('"').nth(1)
}

/// `main` and everything it includes, in the order niri reads them.
fn niri_includes(file: &Path, seen: &mut Vec<PathBuf>) {
    if seen.iter().any(|s| s == file) || !file.is_file() {
        return;
    }
    seen.push(file.to_path_buf());
    let Ok(text) = fs::read_to_string(file) else { return };
    for path in text.lines().filter_map(included) {
        let path = match path.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().unwrap_or_default().join(rest),
            None => file.with_file_name(path), // relative to the including file; absolute wins
        };
        niri_includes(&path, seen);
    }
}

/// From "niri 26.04 (8ed0da4)".
fn niri_version() -> Option<(u32, u32)> {
    let out = Command::new("niri").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (major, minor) = text.split_whitespace().nth(1)?.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

fn strip_colors(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // an escape sequence ends with its first letter
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// This binary, by a path that doesn't lean on the compositor's PATH.
fn program() -> Res<String> {
    let exe = crate::exe()?;
    let path = match dirs::home_dir().and_then(|home| exe.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => exe.display().to_string(),
    };
    let plain = path.chars().all(|c| c.is_ascii_alphanumeric() || "_-./~".contains(c));
    if !plain {
        return Err(t!("desktop.odd-path", path = path).into());
    }
    Ok(path)
}

fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// What the page shows under Hotkey.
pub fn hotkey() -> Value {
    let Some(config) = Config::find() else {
        return json!({ "desktop": detect().id(), "writable": false });
    };
    let bound = config.bound().map(|(chord, file)| json!({ "chord": chord, "file": tilde(&file) }));
    json!({
        "desktop": config.desktop.id(),
        "writable": config.blocked.is_none(),
        "blocked": config.blocked,
        "bound": bound,
        "files": config.files.iter().map(|f| tilde(f)).collect::<Vec<_>>(),
        "file": tilde(config.default_file()),
        "runs": program().map(|p| format!("{p} toggle")).ok(),
    })
}

/// Binds `rookey toggle` to `chord`, or with `swallow` a bind that does nothing, which keeps
/// the keys `rookey listen` hears from reaching the windows. With the keys taken by something
/// else it says by what and leaves everything alone, unless `replace`.
pub fn bind(chord: &str, file: Option<&str>, replace: bool, swallow: bool) -> Res<Value> {
    let config = Config::find().ok_or_else(|| t!("desktop.out-of-reach"))?;
    if let Some(why) = &config.blocked {
        return Err(why.clone().into());
    }
    let chord = Chord::parse(chord)?;
    chord.check()?;
    let target = match file {
        // only the config's own files, whatever the request says
        Some(asked) => config
            .files
            .iter()
            .find(|f| tilde(f) == asked || f.display().to_string() == asked)
            .ok_or_else(|| t!("desktop.not-part", file = asked, desktop = config.desktop.id()))?
            .clone(),
        None => config.default_file().to_path_buf(),
    };
    if !replace {
        if let Some((file, line, text)) = config.taken(&chord, !swallow) {
            return Ok(
                json!({ "taken": { "chord": chord.to_string(), "file": tilde(&file), "line": line, "text": text } }),
            );
        }
    }

    let command = if swallow { "true".to_string() } else { format!("{} toggle", program()?) };
    let before = Saved::of(&config)?;
    let written = match config.desktop {
        Desktop::Niri => bind_niri(&config, &chord, &target, &command),
        _ => bind_hyprland(&config, &chord, &target, &command),
    };
    if let Err(e) = written.and_then(|()| config.validate(&config.main)) {
        before.restore();
        return Err(e);
    }
    Ok(json!({ "bound": { "chord": chord.to_string(), "file": tilde(&target) } }))
}

/// Another bind of the same keys in the compositor's config, the way `bind` reports it. A bind
/// that does nothing isn't one: it keeps the keys from the windows, as `rookey listen` wants.
pub fn taken(chord: &Chord) -> Option<Value> {
    let (file, line, text) = Config::find()?.taken(chord, false)?;
    Some(json!({ "chord": chord.to_string(), "file": tilde(&file), "line": line, "text": text }))
}

/// Whether the compositor holds a hotkey of rookey's.
pub fn is_bound() -> bool {
    Config::find().is_some_and(|c| c.bound().is_some())
}

/// Whether rookey can write binds into this desktop's config.
pub fn writable() -> bool {
    Config::find().is_some_and(|c| c.blocked.is_none())
}

pub fn unbind() -> Res<()> {
    let config = Config::find().ok_or_else(|| t!("desktop.out-of-reach"))?;
    let before = Saved::of(&config)?;
    let removed = (|| -> Res<()> {
        for file in &config.files {
            let text = fs::read_to_string(file)?;
            let without = without_hotkey(&text);
            if without != text {
                fs::write(file, without)?;
            }
        }
        if config.desktop == Desktop::Niri {
            // the include went first, the file it named goes after
            match fs::remove_file(config.niri_file()) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    })();
    if let Err(e) = removed.and_then(|()| config.validate(&config.main)) {
        before.restore();
        return Err(e);
    }
    Ok(())
}

/// The config files as they were, to put back when a change doesn't pass.
struct Saved(Vec<(PathBuf, Option<String>)>);

impl Saved {
    fn of(config: &Config) -> Res<Saved> {
        let mut files = config.files.clone();
        if config.desktop == Desktop::Niri {
            files.push(config.niri_file());
        }
        let mut saved = Vec::new();
        for file in files {
            let text = match fs::read_to_string(&file) {
                Ok(text) => Some(text),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(t!("server.cant-read", path = tilde(&file), why = e).into()),
            };
            saved.push((file, text));
        }
        Ok(Saved(saved))
    }

    fn restore(&self) {
        for (file, text) in &self.0 {
            let now = fs::read_to_string(file).ok();
            if now == *text {
                continue;
            }
            let put_back = match text {
                Some(text) => fs::write(file, text),
                None => fs::remove_file(file),
            };
            if let Err(e) = put_back {
                eprintln!("{}", t!("desktop.put-back", file = file.display(), why = e));
            }
        }
    }
}

/// The text without the hotkey: the marked block or the include of rookey.kdl, and the blank
/// line that set it apart. A text with no hotkey in it comes back untouched.
fn without_hotkey(text: &str) -> String {
    if !text.lines().any(is_ours) {
        return text.to_string();
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut ours = false;
    for line in text.split_inclusive('\n') {
        let starts = is_ours(line) && !ours;
        if starts && kept.last().is_some_and(|last| last.trim().is_empty()) {
            kept.pop();
        }
        if is_mark(line) {
            ours = !line.contains("end");
        } else if !ours && !is_niri_include(line) {
            kept.push(line);
        }
    }
    kept.concat()
}

/// `text` with `addition` after it, on lines of its own and set apart by a blank one.
fn with_added(text: &str, addition: &str) -> String {
    let gap = if text.is_empty() || text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{text}{gap}{addition}\n")
}

fn bind_niri(config: &Config, chord: &Chord, target: &Path, command: &str) -> Res<()> {
    let own = config.niri_file();
    let spawn = command.split(' ').map(|part| format!("\"{part}\"")).collect::<Vec<_>>().join(" ");
    let bind = format!(
        "// The hotkey of rookey, written by `rookey ui`. Change it there: this file is rewritten.\n\
         // chord: {chord}\n\
         binds {{\n    {chord} repeat=false hotkey-overlay-title=\"Dictate with rookey\" {{ spawn {spawn}; }}\n}}\n"
    );
    // checked on its own before niri gets to see it under its real name
    let draft = own.with_extension("kdl.new");
    fs::write(&draft, &bind)?;
    let checked = config.validate(&draft);
    if let Err(e) = checked {
        let _ = fs::remove_file(&draft);
        return Err(e);
    }
    fs::rename(&draft, &own)?;

    for file in &config.files {
        let text = fs::read_to_string(file)?;
        let mut new = without_hotkey(&text);
        if file == target {
            let path = if own.parent() == file.parent() { "rookey.kdl".to_string() } else { tilde(&own) };
            new = with_added(&new, &format!("include \"{path}\" optional=true // {MARK}, set in `rookey ui`"));
        }
        if new != text {
            fs::write(file, new)?;
        }
    }
    Ok(())
}

fn bind_hyprland(config: &Config, chord: &Chord, target: &Path, command: &str) -> Res<()> {
    let (comment, bind) = if config.is_lua() {
        ("--", format!("hl.bind(\"{}\", hl.dsp.exec_cmd(\"{command}\"))", chord.joined(" + ", true)))
    } else {
        let mods = chord.mods.iter().map(|m| m.to_uppercase()).collect::<Vec<_>>().join(" ");
        ("#", format!("bind = {mods}, {}, exec, {command}", chord.key))
    };
    for file in &config.files {
        let text = fs::read_to_string(file)?;
        let mut new = without_hotkey(&text);
        if file == target {
            new = with_added(
                &new,
                &format!(
                    "{comment} {MARK} {chord}: managed by `rookey ui`, change or remove it there\n{bind}\n{comment} {MARK} end"
                ),
            );
        }
        if new != text {
            fs::write(file, new)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        let chord = Chord::parse("Shift+super+D").unwrap();
        assert_eq!(chord.to_string(), "Super+Shift+D");
        assert_eq!(chord.joined(" + ", true), "SUPER + SHIFT + D");
        assert!(chord.same(&Chord::parse("Mod+Shift+d").unwrap()));
        assert!(chord.same(&Chord::parse("SUPER SHIFT, D").unwrap()));
        assert!(!chord.same(&Chord::parse("Mod+D").unwrap()));
        assert!(chord.check().is_ok());

        assert!(Chord::parse("Super+Shift").is_err());
        assert!(Chord::parse("Super+D\" { spawn \"rm\"; }").is_err());
        assert!(Chord::parse("Super+D+E").is_err());
        assert!(Chord::parse("Shift+D").unwrap().check().is_err());
        assert!(Chord::parse("F13").unwrap().check().is_ok());
        // keys that type, alone or with Shift, stay with typing
        assert!(Chord::parse("grave").unwrap().check().is_err());
        assert!(Chord::parse("Shift+grave").unwrap().check().is_err());
        assert!(Chord::parse("Super+Shift+grave").unwrap().check().is_ok());
        assert!(Chord::parse("Control_R").unwrap().check().is_ok());
    }

    #[test]
    fn hotkey_comes_out_clean() {
        let lua =
            "a = 1\n\n-- rookey hotkey Super+D: managed\nhl.bind(\"SUPER + D\", x)\n-- rookey hotkey end\nb = 2\n";
        assert_eq!(without_hotkey(lua), "a = 1\nb = 2\n");
        let kdl = "binds {\n}\n\ninclude \"rookey.kdl\" optional=true // rookey hotkey, set in `rookey ui`\n";
        assert_eq!(without_hotkey(kdl), "binds {\n}\n");
        // what has no hotkey in it is nobody's to tidy
        for untouched in ["include \"user.kdl\"\n", "a\r\n\n\n", "no newline at the end"] {
            assert_eq!(without_hotkey(untouched), untouched);
        }
        // adding and taking away again leaves a file as it was
        for text in ["binds {\n}\n", "binds {\n}", "a\n\n", ""] {
            let added = with_added(text, "include \"rookey.kdl\" optional=true // rookey hotkey, x");
            assert!(added.ends_with("x\n") && added.starts_with(text));
            let back = without_hotkey(&added);
            assert_eq!(back.trim_end_matches('\n'), text.trim_end_matches('\n'), "{text:?}");
        }
        assert_eq!(
            reason("Error: x\n  ├─▶ invalid keybind\n  ╰─▶ invalid key: Nope\n 3 │ binds {"),
            "invalid key: Nope"
        );
        assert_eq!(reason("==== Config parsing result:\n\nuser.lua:24: Unknown keysym"), "user.lua:24: Unknown keysym");
        assert_eq!(included("include \"~/x/rookey.kdl\" optional=true"), Some("~/x/rookey.kdl"));
        assert_eq!(strip_colors("\x1b[2mniri\x1b[0m ok"), "niri ok");
    }
}
