//! The pill on macOS: a borderless window with no background, above other windows and on
//! every Space, full-screen apps too; never focused, not in the Dock, clicks pass through. Fed
//! the same pixels as on Wayland and Windows, as the image of its view's layer.

use std::ffi::{c_char, c_void};

use crate::overlay::{H, MARGIN, Pill, W};
use crate::status::now_ms;

type Id = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSApplicationLoad() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(space: *mut c_void);
    fn CGDataProviderCreateWithCFData(data: *const c_void) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    #[allow(clippy::too_many_arguments)]
    fn CGImageCreate(
        w: usize,
        h: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut c_void,
        info: u32,
        provider: *mut c_void,
        decode: *const f64,
        interpolate: bool,
        intent: i32,
    ) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
    static kCFRunLoopDefaultMode: *const c_void;
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

/// `[obj sel:args…]` through the runtime, typed per call: no crate for a dozen messages.
macro_rules! send {
    ($obj:expr, $sel:literal $(, $arg:expr => $ty:ty)* ; $ret:ty) => {{
        let f: unsafe extern "C" fn(Id, Id $(, $ty)*) -> $ret = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        f($obj, sel_registerName(concat!($sel, "\0").as_ptr().cast()) $(, $arg)*)
    }};
}

unsafe fn class(name: &std::ffi::CStr) -> Id {
    unsafe { objc_getClass(name.as_ptr()) }
}

/// The part of the screen windows go in: not under the menu bar or the Dock.
unsafe fn visible_frame(screen: Id) -> Rect {
    unsafe {
        // a struct this big comes back through memory on Intel, in registers on Apple silicon
        #[cfg(target_arch = "x86_64")]
        {
            let mut rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
            let f: unsafe extern "C" fn(*mut Rect, Id, Id) = std::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
            f(&mut rect, screen, sel_registerName(c"visibleFrame".as_ptr()));
            rect
        }
        #[cfg(not(target_arch = "x86_64"))]
        send!(screen, "visibleFrame"; Rect)
    }
}

/// Where the pill goes: centred at the bottom of the usable screen, like on the other systems.
fn place(area: Rect) -> Rect {
    Rect { x: area.x + (area.w - W as f64) / 2.0, y: area.y + MARGIN as f64, w: W as f64, h: H as f64 }
}

pub fn run() -> crate::Res<()> {
    unsafe {
        NSApplicationLoad();
        let app: Id = send!(class(c"NSApplication"), "sharedApplication"; Id);
        // an accessory: no Dock icon and no menu bar of its own
        send!(app, "setActivationPolicy:", 1isize => isize; bool);
        // ponytail: the screen with the keyboard's focus as it starts; it stays there if you
        // move to another screen mid-sentence
        let screen: Id = send!(class(c"NSScreen"), "mainScreen"; Id);
        if screen.is_null() {
            return Err("no screen to show the pill on".into());
        }
        let scale = (send!(screen, "backingScaleFactor"; f64)).round().max(1.0) as u32;
        let rect = place(visible_frame(screen));

        let window: Id = send!(class(c"NSWindow"), "alloc"; Id);
        const BORDERLESS: usize = 0;
        const BUFFERED: usize = 2;
        let window: Id = send!(window, "initWithContentRect:styleMask:backing:defer:",
            rect => Rect, BORDERLESS => usize, BUFFERED => usize, false => bool; Id);
        if window.is_null() {
            return Err("couldn't make the pill's window".into());
        }
        send!(window, "setReleasedWhenClosed:", false => bool; ());
        send!(window, "setOpaque:", false => bool; ());
        let clear: Id = send!(class(c"NSColor"), "clearColor"; Id);
        send!(window, "setBackgroundColor:", clear => Id; ());
        send!(window, "setHasShadow:", false => bool; ());
        send!(window, "setIgnoresMouseEvents:", true => bool; ());
        // the status bar's level: over ordinary windows and full-screen apps
        send!(window, "setLevel:", 25isize => isize; ());
        // every Space, full-screen ones too, left where it is by Mission Control and ⌘`
        const ALL_SPACES: usize = 1;
        const STATIONARY: usize = 16;
        const IGNORES_CYCLE: usize = 64;
        const FULL_SCREEN_AUXILIARY: usize = 256;
        send!(window, "setCollectionBehavior:", ALL_SPACES | STATIONARY | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY => usize; ());
        let view: Id = send!(window, "contentView"; Id);
        send!(view, "setWantsLayer:", true => bool; ());
        let layer: Id = send!(view, "layer"; Id);
        send!(layer, "setContentsScale:", scale as f64 => f64; ());

        let (w, h) = ((W * scale) as usize, (H * scale) as usize);
        let mut px = vec![0u8; w * h * 4];
        let space = CGColorSpaceCreateDeviceRGB();
        // premultiplied BGRA bytes: alpha first in a little-endian 32-bit word
        const PREMULTIPLIED_FIRST: u32 = 2;
        const LITTLE_32: u32 = 2 << 12;
        let transaction = class(c"CATransaction");

        let mut pill = Pill::new();
        let mut shown = false;
        loop {
            let pool = objc_autoreleasePoolPush();
            let now = now_ms();
            if !pill.tick(now) {
                objc_autoreleasePoolPop(pool);
                break;
            }
            pill.draw(&mut px, scale, now);
            let data = CFDataCreate(std::ptr::null(), px.as_ptr(), px.len() as isize);
            let provider = CGDataProviderCreateWithCFData(data);
            let image = CGImageCreate(w, h, 8, 32, w * 4, space, PREMULTIPLIED_FIRST | LITTLE_32, provider, std::ptr::null(), false, 0);
            // no fade from the last frame to this one: that's a layer's habit
            send!(transaction, "begin"; ());
            send!(transaction, "setDisableActions:", true => bool; ());
            send!(layer, "setContents:", image => Id; ());
            send!(transaction, "commit"; ());
            CGImageRelease(image);
            CGDataProviderRelease(provider);
            CFRelease(data);
            if !shown {
                send!(window, "orderFrontRegardless"; ());
                shown = true;
            }
            objc_autoreleasePoolPop(pool);
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.033, 0);
        }
        send!(window, "orderOut:", std::ptr::null_mut::<c_void>() => Id; ());
        send!(window, "close"; ());
        CGColorSpaceRelease(space);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centred_above_the_dock() {
        // a 1512x982 screen with the Dock taking 70 points at the bottom and the menu bar 33
        let at = place(Rect { x: 0.0, y: 70.0, w: 1512.0, h: 879.0 });
        assert_eq!((at.x, at.y, at.w, at.h), ((1512.0 - W as f64) / 2.0, 70.0 + MARGIN as f64, W as f64, H as f64));
        // a second screen to the left of the main one starts at a negative x
        let at = place(Rect { x: -1920.0, y: 0.0, w: 1920.0, h: 1080.0 });
        assert!(at.x < 0.0 && at.x + at.w < 0.0);
    }
}
