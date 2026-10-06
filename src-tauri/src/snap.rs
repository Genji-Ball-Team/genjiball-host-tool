//! Snap layouts for the custom title bar (#37). Windows shows its layouts when the pointer rests
//! on a native maximise button; the window draws its own, so it asks for them with the shortcut
//! Windows has for it, Win+Z, which works on the focused window. Only while this window has the
//! focus, so the keys never reach the game.

/// Win+Z, as key presses: Windows' shortcut for the focused window's snap layouts.
#[cfg(windows)]
fn send_win_z() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
        VK_LWIN,
    };
    const VK_Z: VIRTUAL_KEY = 0x5A;
    let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        key(VK_LWIN, false),
        key(VK_Z, false),
        key(VK_Z, true),
        key(VK_LWIN, true),
    ];
    // SAFETY: `inputs` is a valid array of `INPUT`s for the length and size given.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        log::debug!("Snap layouts: only {sent} of the keys were sent");
    }
}

/// Shows Windows' snap layouts for `window`, if it has the focus. Nothing on other systems.
pub fn show_layouts(window: &tauri::WebviewWindow) {
    if !window.is_focused().unwrap_or(false) {
        return;
    }
    #[cfg(windows)]
    send_win_z();
}
