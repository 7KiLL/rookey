//! The macOS privacy switches rookey needs, and asking for them: the microphone, recording the
//! screen (screen terms), Accessibility and control of System Events (typing). macOS grants
//! them to the app responsible for rookey, not to rookey: the terminal `rookey ui` runs in,
//! or the app that runs `rookey toggle` from a hotkey. Each call here answers for that app.

use std::ffi::{c_char, c_void};

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
    }
}
