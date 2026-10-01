//! The pill on Windows: a layered window with per-pixel alpha, topmost, never focused, not in
//! the taskbar, clicks pass through. Fed the same pixels as on Wayland.

use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, RegisterClassW,
    SPI_GETWORKAREA, SW_SHOWNOACTIVATE, ShowWindow, SystemParametersInfoW, TranslateMessage, ULW_ALPHA,
    UpdateLayeredWindow, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::overlay::{H, Pill, W, spot};
use crate::status::now_ms;

pub fn run() -> crate::Res<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // ponytail: the system DPI, placed on the primary screen; per-monitor if people ask
        let scale = (GetDpiForSystem() as f32 / 96.0).round().max(1.0) as u32;
        let (w, h) = ((W * scale) as i32, (H * scale) as i32);
        let mut work = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&raw mut work).cast(), 0);
        let (x, y) =
            spot((work.right - work.left) as f64, (work.bottom - work.top) as f64, scale as f64, crate::overlay::at());
        let at = POINT { x: work.left + x.round() as i32, y: work.top + y.round() as i32 };

        let class: Vec<u16> = "rookey-overlay\0".encode_utf16().collect();
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let hwnd: HWND = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            at.x,
            at.y,
            w,
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(crate::t!("overlay.no-window").into());
        }

        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down, like the pixels are drawn
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            return Err(crate::t!("overlay.no-pixels").into());
        }
        let old = SelectObject(dc, bitmap);
        let px = std::slice::from_raw_parts_mut(bits.cast::<u8>(), (w * h * 4) as usize);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let (size, origin) = (SIZE { cx: w, cy: h }, POINT { x: 0, y: 0 });

        let mut pill = Pill::new();
        let mut shown = false;
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let now = now_ms();
            if !pill.tick(now) {
                break;
            }
            pill.draw(px, scale, now);
            UpdateLayeredWindow(hwnd, screen, &at, &size, dc, &origin, 0, &blend, ULW_ALPHA);
            if !shown {
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                shown = true;
            }
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(33));
        }

        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        ReleaseDC(std::ptr::null_mut(), screen);
        DestroyWindow(hwnd);
    }
    Ok(())
}
