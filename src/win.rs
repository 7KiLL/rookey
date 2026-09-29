//! The few Windows calls rookey needs: typing text, reading a key's state, telling whether a
//! process still runs.

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
};

use crate::Res;

/// What GetExitCodeProcess says of a process that hasn't ended.
const STILL_ACTIVE: u32 = 259;

/// Types text into the focused window as Unicode key presses, so any language comes through
/// whatever the keyboard layout.
pub fn type_text(text: &str) -> Res<()> {
    let key = |unit: u16, flags: u32| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs: Vec<INPUT> = text
        .encode_utf16()
        .flat_map(|unit| [key(unit, KEYEVENTF_UNICODE), key(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)])
        .collect();
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        // a window running as administrator takes no input from one that isn't
        return Err("Windows took only part of the text; is the window running as administrator?".into());
    }
    Ok(())
}

/// Whether the key with this virtual-key code is down now.
pub fn down(vk: u16) -> bool {
    let state = unsafe { GetAsyncKeyState(vk as i32) };
    state < 0 // the high bit: down now
}

pub fn alive(pid: u32) -> bool {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(process, &mut code) != 0;
        CloseHandle(process);
        ok && code == STILL_ACTIVE
    }
}
