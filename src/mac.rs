//! The macOS privacy switches rookey needs, and asking for them: the microphone, recording the
//! screen (screen terms), Accessibility (typing, and the hotkey Rookey hears) and control of
//! System Events (typing). macOS grants them to the app responsible for rookey, not to
//! rookey: the terminal `rookey ui` runs in, the app that runs `rookey toggle` from a hotkey,
//! or Rookey, the small app `rookey listen` runs from (see `app()`). Each call here answers
//! for the app responsible for this process.

use std::ffi::{c_char, c_void};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use std::{env, fs};

use crate::Res;

type Id = *mut c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, user: *mut c_void) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
    fn CGEventGetFlags(event: *mut c_void) -> u64;
}

type TapCallback = extern "C" fn(proxy: *mut c_void, kind: u32, event: *mut c_void, user: *mut c_void) -> *mut c_void;

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn getuid() -> u32;
}

/// Whether the process with this pid still runs (signal 0 checks without sending).
pub fn alive(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(|pid| pid > 0 && unsafe { kill(pid, 0) } == 0)
}

/// The user's id, which launchd's per-user domain is named after.
pub fn uid() -> u32 {
    unsafe { getuid() }
}

/// A key going down (1), up (0) or repeating (2), by its macOS virtual key code.
pub struct Key {
    pub code: u16,
    pub value: i32,
}

/// What the tap does with a key: true keeps it from the app in front.
type OnKey = Box<dyn FnMut(Key) -> bool>;

const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const KEYCODE_FIELD: u32 = 9;
const AUTOREPEAT_FIELD: u32 = 8;

/// The tap, to turn back on when macOS turns it off for being slow.
static TAP: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Which bit of the flags says a modifier key is down, by its key code: each side has its own.
fn side_mask(code: u16) -> Option<u64> {
    Some(match code {
        0x3B => 0x0000_0001, // left Control
        0x38 => 0x0000_0002, // left Shift
        0x3C => 0x0000_0004, // right Shift
        0x37 => 0x0000_0008, // left Command
        0x36 => 0x0000_0010, // right Command
        0x3A => 0x0000_0020, // left Option
        0x3D => 0x0000_0040, // right Option
        0x3E => 0x0000_2000, // right Control
        _ => return None,
    })
}

extern "C" fn on_event(_: *mut c_void, kind: u32, event: *mut c_void, user: *mut c_void) -> *mut c_void {
    if kind == TAP_DISABLED_BY_TIMEOUT {
        unsafe { CGEventTapEnable(TAP.load(std::sync::atomic::Ordering::Relaxed), true) };
        return event;
    }
    let code = unsafe { CGEventGetIntegerValueField(event, KEYCODE_FIELD) } as u16;
    let value = match kind {
        KEY_DOWN if unsafe { CGEventGetIntegerValueField(event, AUTOREPEAT_FIELD) } != 0 => 2,
        KEY_DOWN => 1,
        KEY_UP => 0,
        // a modifier went up or down: its own bit in the flags says which
        FLAGS_CHANGED => match side_mask(code) {
            Some(mask) => (unsafe { CGEventGetFlags(event) } & mask != 0) as i32,
            None => return event,
        },
        _ => return event,
    };
    let on = unsafe { &mut *(user as *mut OnKey) };
    if on(Key { code, value }) { std::ptr::null_mut() } else { event }
}

/// Hears every key, on this thread, for as long as the process runs: `on` says for each
/// whether to keep it from the app in front. An active tap needs Accessibility, which Rookey
/// needs to type anyway; without it this fails at once.
pub fn tap(on: impl FnMut(Key) -> bool + 'static) -> Res<()> {
    const SESSION: u32 = 1;
    const HEAD: u32 = 0;
    const ACTIVE: u32 = 0;
    let mask = (1u64 << KEY_DOWN) | (1 << KEY_UP) | (1 << FLAGS_CHANGED);
    // lives as long as the tap, which is as long as this process
    let user = Box::into_raw(Box::new(Box::new(on) as OnKey)) as *mut c_void;
    let tap = unsafe { CGEventTapCreate(SESSION, HEAD, ACTIVE, mask, on_event, user) };
    if tap.is_null() {
        return Err("macOS doesn't let Rookey see the keys: allow it under Privacy & Security > Accessibility".into());
    }
    TAP.store(tap, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        CFRunLoopRun();
    }
    Ok(())
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const c_void;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const u8,
        value_callbacks: *const u8,
    ) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFMachPortCreateRunLoopSource(allocator: *const c_void, port: *mut c_void, order: isize) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRun();
    static kCFRunLoopCommonModes: *const c_void;
}

#[repr(C)]
struct AeDesc {
    kind: u32,
    data: *mut c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(kind: u32, data: *const c_void, size: isize, out: *mut AeDesc) -> i16;
    fn AEDisposeDesc(desc: *mut AeDesc) -> i16;
    fn AEDeterminePermissionToAutomateTarget(target: *const AeDesc, class: u32, id: u32, ask: u8) -> i32;
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {
    static AVMediaTypeAudio: Id;
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    fn objc_msgSend();
}

/// Whether the microphone is allowed: None while macOS hasn't asked yet (it asks the first
/// time rookey listens, which the setup check does).
pub fn mic() -> Option<bool> {
    // +[AVCaptureDevice authorizationStatusForMediaType:], through the runtime: no crate for one call
    let status = unsafe {
        let send: unsafe extern "C" fn(Id, Id, Id) -> isize = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let class = objc_getClass(c"AVCaptureDevice".as_ptr());
        if class.is_null() {
            return None;
        }
        send(class, sel_registerName(c"authorizationStatusForMediaType:".as_ptr()), AVMediaTypeAudio)
    };
    authorized(status)
}

/// AVAuthorizationStatus: 0 not asked yet, 1 restricted, 2 denied, 3 allowed.
fn authorized(status: isize) -> Option<bool> {
    match status {
        3 => Some(true),
        1 | 2 => Some(false),
        _ => None,
    }
}

/// Whether the screen may be recorded. macOS may keep answering no until the app restarts.
pub fn screen() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Asks for screen recording: macOS shows its dialog the first time only, and lists the app
/// under Screen & System Audio Recording from then on, so there is a switch to turn on.
pub fn ask_screen() {
    unsafe { CGRequestScreenCaptureAccess() };
}

/// Whether rookey may type into other apps (Accessibility).
pub fn typing() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Asks for Accessibility: macOS shows its dialog, which leads to the switch.
pub fn ask_typing() {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        );
        AXIsProcessTrustedWithOptions(options);
        if !options.is_null() {
            CFRelease(options);
        }
    }
}

/// Whether rookey may send System Events the paste (Automation). None when macOS hasn't asked
/// yet, or System Events isn't running to ask about; the first paste asks.
pub fn automation() -> Option<bool> {
    let bundle = b"com.apple.systemevents";
    let mut target = AeDesc { kind: 0, data: std::ptr::null_mut() };
    let any = u32::from_be_bytes(*b"****");
    let status = unsafe {
        if AECreateDesc(u32::from_be_bytes(*b"bund"), bundle.as_ptr().cast(), bundle.len() as isize, &mut target) != 0 {
            return None;
        }
        let status = AEDeterminePermissionToAutomateTarget(&target, any, any, 0);
        AEDisposeDesc(&mut target);
        status
    };
    automated(status)
}

/// noErr, errAEEventNotPermitted; errAEEventWouldRequireUserConsent and procNotFound are unknowns.
fn automated(status: i32) -> Option<bool> {
    match status {
        0 => Some(true),
        -1743 => Some(false),
        _ => None,
    }
}

/// Rookey, the app `rookey listen` runs from under launchd. It is a copy of this binary in a
/// bundle of its own, so macOS asks for the microphone and typing (which covers the keys) for "Rookey", and
/// the recordings it starts (its children) are covered by the same answers. Signed with
/// rookey's own certificate, a new copy after an update keeps them; an ad-hoc build is a new
/// app to macOS each time and asks again.
pub const APP_ID: &str = "io.github.7kill.rookey";
const ICON: &[u8] = include_bytes!("../window/macos/rookey.icns");

/// Where Rookey lives: next to the models, not in the caches, which macOS may clear.
pub fn app() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("rookey").join("Rookey.app"))
}

/// The binary inside Rookey.
pub fn app_exe(app: &Path) -> PathBuf {
    app.join("Contents/MacOS/rookey")
}

/// Whether this process is Rookey itself.
pub fn is_app() -> bool {
    env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains("/Rookey.app/Contents/MacOS/"))
}

/// Puts Rookey together from `from` (a rookey binary), or brings its copy up to date. Written
/// only when something changed, so an app that runs from it keeps running the same file.
pub fn place_app(from: &Path) -> Res<PathBuf> {
    let app = app().ok_or("no data directory for Rookey")?;
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
    let write = |path: PathBuf, bytes: &[u8]| -> Res<()> {
        if fs::read(&path).is_ok_and(|there| there == bytes) {
            return Ok(());
        }
        // a new file then a rename: the running app keeps the one it started from
        let new = path.with_extension("new");
        fs::write(&new, bytes)?;
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
        fs::rename(&new, &path)?;
        Ok(())
    };
    write(contents.join("Info.plist"), info().as_bytes())?;
    write(contents.join("Resources/rookey.icns"), ICON)?;
    let exe = app_exe(&app);
    // itself when Rookey is what runs: the copy is already this binary
    if fs::canonicalize(from).ok() != fs::canonicalize(&exe).ok() {
        write(exe, &fs::read(from)?)?;
        let from = fs::canonicalize(from)?;
        write(contents.join("Resources/source"), from.to_string_lossy().as_bytes())?;
    }
    Ok(app)
}

/// The rookey Rookey was copied from, where its window and updates are.
pub fn source() -> Option<PathBuf> {
    let app = app()?;
    Some(PathBuf::from(fs::read_to_string(app.join("Contents/Resources/source")).ok()?.trim()))
}

/// Rookey's Info.plist. macOS ends an app that uses the microphone or sends Apple events
/// without saying why, so the reasons are in here; the dialogs show them.
fn info() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>{APP_ID}</string>
  <key>CFBundleName</key><string>Rookey</string>
  <key>CFBundleDisplayName</key><string>Rookey</string>
  <key>CFBundleExecutable</key><string>rookey</string>
  <key>CFBundleIconFile</key><string>rookey</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{}</string>
  <key>LSUIElement</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Rookey listens while you hold your hotkey, and writes down what you say.</string>
  <key>NSAppleEventsUsageDescription</key><string>Rookey pastes what you said through System Events.</string>
</dict>
</plist>
"#,
        crate::update::VERSION
    )
}

/// Runs rookey as Rookey, through LaunchServices, so what it asks and answers is Rookey's
/// and not the terminal's. With `wait`, waits that long for its answer (see `answer`) and
/// hands it back; without, leaves it running: a question macOS puts on screen takes as long
/// as it takes.
pub fn as_app(args: &[&str], wait: Option<Duration>) -> Res<String> {
    let app = place_app(&crate::exe()?)?;
    let mut open = Command::new("open");
    open.args(["-n", "-g"]).stdin(Stdio::null()).stdout(Stdio::null());
    let Some(wait) = wait else {
        open.arg(&app).arg("--args").args(args).status()?;
        return Ok(String::new());
    };
    // not `open -W`: Rookey may be done before open gets to watch it, and open fails then
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = env::temp_dir().join(format!("rookey-as-app-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let (reply, err) = (dir.join("answer"), dir.join("err"));
    let done = (|| -> Res<String> {
        let status = open.arg("--env").arg(format!("{ANSWER}={}", reply.display())).arg("--stderr").arg(&err).arg(&app).arg("--args").args(args).status()?;
        if !status.success() {
            return Err("Rookey didn't start".into());
        }
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if let Ok(text) = fs::read_to_string(&reply) {
                return Ok(text);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let why = fs::read_to_string(&err).unwrap_or_default();
        Err(format!("Rookey didn't answer ({})", why.trim()).into())
    })();
    let _ = fs::remove_dir_all(&dir);
    done
}

/// Runs this rookey command again as Rookey, through LaunchServices: `rookey ui` waits for it
/// and shows what it prints in this terminal; `rookey toggle` from a hotkey app returns at once.
/// The settings and PATH (tesseract is in Homebrew's folder) go along, as `open` passes none.
pub fn relaunch(args: &[String], wait: bool) -> Res<()> {
    let app = place_app(&crate::exe()?)?;
    let mut open = Command::new("open");
    open.args(["-n", "-g"]);
    // no terminal (a script, a pipe): Rookey writes to a file, and this passes it on
    let relay = (wait && tty().is_none()).then(|| env::temp_dir().join(format!("rookey-relay-{}", std::process::id())));
    if wait {
        open.arg("-W");
        let out = tty().or_else(|| relay.clone()).unwrap();
        open.arg("--stdout").arg(&out).arg("--stderr").arg(&out);
    }
    for (key, value) in env::vars() {
        if key.starts_with("ROOKEY_") || key.starts_with("XDG_") || key == "PATH" {
            open.arg("--env").arg(format!("{key}={value}"));
        }
    }
    let mut child = open.arg(&app).arg("--args").args(args).stdin(Stdio::null()).spawn()?;
    let mut passed = 0;
    let status = loop {
        if let Some(relay) = &relay {
            if let Ok(text) = fs::read(relay) {
                if text.len() > passed {
                    let _ = std::io::Write::write_all(&mut std::io::stderr(), &text[passed..]);
                    passed = text.len();
                }
            }
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if let Some(relay) = relay {
        let _ = fs::remove_file(relay);
    }
    if !status.success() {
        return Err(format!("couldn't start Rookey ({}); ROOKEY_IN_TERMINAL=1 runs it here instead", app.display()).into());
    }
    Ok(())
}

/// This terminal, for Rookey to print into.
fn tty() -> Option<PathBuf> {
    unsafe extern "C" {
        fn ttyname(fd: i32) -> *const c_char;
    }
    let name = unsafe { ttyname(2) };
    (!name.is_null()).then(|| PathBuf::from(unsafe { std::ffi::CStr::from_ptr(name) }.to_string_lossy().into_owned()))
}

/// Where `as_app` wants the answer, when it asks.
const ANSWER: &str = "ROOKEY_ANSWER";

/// What a command run for the page says back: into the file `as_app` waits on, whole or not
/// at all, or on stdout when it runs any other way.
pub fn answer(text: &str) -> Res<()> {
    match env::var_os(ANSWER) {
        Some(path) => {
            let path = PathBuf::from(path);
            let new = path.with_extension("new");
            fs::write(&new, text)?;
            fs::rename(new, path)?;
        }
        None => println!("{text}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_read_right() {
        assert_eq!(authorized(3), Some(true));
        assert_eq!(authorized(2), Some(false));
        assert_eq!(authorized(0), None);
        assert_eq!(automated(0), Some(true));
        assert_eq!(automated(-1743), Some(false));
        assert_eq!(automated(-1744), None);
        assert_eq!(automated(-600), None);
    }

    #[test]
    fn the_calls_answer() {
        // on CI nothing is granted; what matters is that every framework links and returns
        let _ = (mic(), screen(), typing(), automation());
        assert!(alive(std::process::id()) && !alive(0) && !alive(u32::MAX));
    }

    #[test]
    fn rookey_says_why() {
        let info = info();
        for key in ["NSMicrophoneUsageDescription", "NSAppleEventsUsageDescription", "LSUIElement", APP_ID] {
            assert!(info.contains(key), "{key}");
        }
    }
}
