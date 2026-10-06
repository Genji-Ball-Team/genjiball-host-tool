//! The overlay window (#49): a transparent, always-on-top window over Overwatch, with no frame or
//! taskbar button, that lets every click through and never takes the focus from the game.
//!
//! **It never touches the game.** To know where Overwatch is, it asks Windows which window is in
//! front, and finds the game's by its window class (`config::GAME_WINDOW_CLASS`), as any window
//! manager does: it never opens the game's process, reads its memory, sends it input or draws
//! inside it. Windows draws the overlay over the game like any other window, so it shows over
//! borderless windowed and windowed Overwatch; exclusive fullscreen hides it.
//!
//! Its hotkeys (show or hide, edit layout, AFK) are global: Windows takes them before the game
//! sees them, so `settings::parse_hotkey` only takes keys with Ctrl, Alt or the Windows key.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::{config, Store};

/// The overlay window's label, and the page it shows (the stream page shows the same one).
pub const LABEL: &str = "overlay";
pub const PAGE: &str = "overlay.html";

/// Sent when the overlay's settings or the edit mode change: the overlay reads its feed again at
/// once rather than at its next poll, and Settings shows the change (a hotkey ended edit mode).
pub const CHANGED_EVENT: &str = "overlay-changed";

#[derive(Default)]
pub struct OverlayWindow {
    /// The host is placing the widgets: the window takes the mouse.
    editing: AtomicBool,
    /// Hidden with its hotkey. Memory only: it shows again after a restart.
    hidden: AtomicBool,
    /// The loop that follows the game window runs.
    tracking: AtomicBool,
    /// The hotkeys registered now, by action.
    registered: Mutex<Vec<(&'static str, Shortcut)>>,
    /// Hotkeys Windows wouldn't register (another app has them), for Settings.
    hotkey_errors: Mutex<Vec<String>>,
}

impl OverlayWindow {
    pub fn editing(&self) -> bool {
        self.editing.load(Ordering::Relaxed)
    }

    pub fn hidden(&self) -> bool {
        self.hidden.load(Ordering::Relaxed)
    }

    pub fn hotkey_errors(&self) -> Vec<String> {
        self.hotkey_errors.lock().unwrap().clone()
    }
}

/// Makes the overlay window and its hotkeys match the settings: call at startup and after every
/// change to them.
pub fn apply(app: &AppHandle) {
    let overlay = app.state::<OverlayWindow>();
    let on = app.state::<Store>().get().overlay.on();
    register_hotkeys(app, on);
    match (on, app.get_webview_window(LABEL)) {
        (true, None) => {
            if let Err(e) = create(app) {
                log::error!("Couldn't open the overlay: {e}");
                return;
            }
            log::info!("Overlay on");
            if !overlay.tracking.swap(true, Ordering::Relaxed) {
                track(app.clone());
            }
        }
        (false, Some(window)) => {
            overlay.editing.store(false, Ordering::Relaxed);
            // Not `close`: the app hides a window it's asked to close.
            let _ = window.destroy();
            log::info!("Overlay off");
        }
        _ => {}
    }
    let _ = app.emit(CHANGED_EVENT, ());
}

fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(PAGE.into()))
        .title("Genji Ball overlay")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        // After `transparent`, which asks for the focus.
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()?;
    window.set_ignore_cursor_events(true)?;
    #[cfg(windows)]
    win::set_passive(&window, true);
    Ok(())
}

/// Starts or ends edit mode: while it's on, the window takes the mouse so the host can drag the
/// widgets, and shows even when the game isn't in front.
pub fn set_editing(app: &AppHandle, on: bool) {
    let overlay = app.state::<OverlayWindow>();
    if !app.state::<Store>().get().overlay.on() {
        return;
    }
    overlay.editing.store(on, Ordering::Relaxed);
    if on {
        overlay.hidden.store(false, Ordering::Relaxed);
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_ignore_cursor_events(!on);
        #[cfg(windows)]
        win::set_passive(&window, !on);
    }
    let _ = app.emit(CHANGED_EVENT, ());
}

/// What a hotkey does.
fn hotkey(app: &AppHandle, shortcut: &Shortcut) {
    let action = {
        let overlay = app.state::<OverlayWindow>();
        let registered = overlay.registered.lock().unwrap();
        registered
            .iter()
            .find(|(_, s)| s.id() == shortcut.id())
            .map(|(a, _)| *a)
    };
    let overlay = app.state::<OverlayWindow>();
    match action {
        Some("toggle") => {
            let hidden = !overlay.hidden.load(Ordering::Relaxed);
            overlay.hidden.store(hidden, Ordering::Relaxed);
            if hidden {
                set_editing(app, false);
            }
        }
        Some("edit") => set_editing(app, !overlay.editing()),
        Some("afk") => {
            if let Err(e) = crate::toggle_afk(app) {
                log::warn!("AFK hotkey: {e}");
            }
        }
        _ => {}
    }
}

/// The global shortcut plugin, sending each hotkey here.
pub fn hotkeys_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                hotkey(app, shortcut);
            }
        })
        .build()
}

/// Registers the hotkeys in the settings while the overlay is on, and none while it's off.
fn register_hotkeys(app: &AppHandle, on: bool) {
    let overlay = app.state::<OverlayWindow>();
    let mut registered = overlay.registered.lock().unwrap();
    let shortcuts = app.global_shortcut();
    for (_, shortcut) in registered.drain(..) {
        let _ = shortcuts.unregister(shortcut);
    }
    let mut errors = Vec::new();
    if on {
        let settings = app.state::<Store>().get();
        for hotkey in &config::OVERLAY_HOTKEYS {
            let Some(keys) = settings.overlay.hotkey(hotkey.action) else {
                continue;
            };
            // Checked when saved: `settings::parse_hotkey`.
            let Ok(shortcut) = keys.parse::<Shortcut>() else {
                continue;
            };
            match shortcuts.register(shortcut) {
                Ok(()) => registered.push((hotkey.action, shortcut)),
                Err(e) => {
                    log::warn!("Couldn't register {keys} for \"{}\": {e}", hotkey.label);
                    errors.push(format!(
                        "{keys} ({}): another app has it. Pick other keys",
                        hotkey.label
                    ));
                }
            }
        }
    }
    *overlay.hotkey_errors.lock().unwrap() = errors;
}

/// Where the overlay goes: the game's window, else the primary screen. Physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Whether the overlay shows: not hidden with its hotkey, and while the game is in front unless
/// the host chose otherwise or is placing the widgets.
pub fn shows(hidden: bool, editing: bool, only_with_game: bool, game_in_front: bool) -> bool {
    !hidden && (editing || !only_with_game || game_in_front)
}

/// Follows the game window while the overlay is on: shows the overlay over it while it's in
/// front, and hides it otherwise. Ends once the overlay is off.
fn track(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last: Option<(Rect, bool)> = None;
        let mut ticks: u32 = 0;
        loop {
            std::thread::sleep(Duration::from_millis(config::OVERLAY_TRACK_MS));
            let overlay = app.state::<OverlayWindow>();
            let Some(window) = app.get_webview_window(LABEL) else {
                overlay.tracking.store(false, Ordering::Relaxed);
                return;
            };
            let only_with_game = app.state::<Store>().get().overlay.only_with_game();
            let game = game_window();
            let visible = shows(
                overlay.hidden(),
                overlay.editing(),
                only_with_game,
                game.is_some_and(|g| g.1),
            );
            let rect = game.map(|g| g.0).unwrap_or_else(primary_screen);
            ticks = ticks.wrapping_add(1);
            // Placed again now and then while it shows, to stay over windows that went on top.
            let refresh = visible && ticks.is_multiple_of(8);
            if last != Some((rect, visible)) || refresh {
                place(&window, rect, visible);
                last = Some((rect, visible));
            }
        }
    });
}

#[cfg(windows)]
fn game_window() -> Option<(Rect, bool)> {
    win::game_window()
}

#[cfg(windows)]
fn primary_screen() -> Rect {
    win::primary_screen()
}

#[cfg(windows)]
fn place(window: &tauri::WebviewWindow, rect: Rect, visible: bool) {
    win::place(window, rect, visible);
}

#[cfg(not(windows))]
fn game_window() -> Option<(Rect, bool)> {
    None
}

#[cfg(not(windows))]
fn primary_screen() -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }
}

#[cfg(not(windows))]
fn place(window: &tauri::WebviewWindow, rect: Rect, visible: bool) {
    let _ = window.set_position(tauri::PhysicalPosition::new(rect.x, rect.y));
    let _ = window.set_size(tauri::PhysicalSize::new(
        rect.width as u32,
        rect.height as u32,
    ));
    let _ = if visible {
        window.show()
    } else {
        window.hide()
    };
}

/// The Windows calls: only about windows, never the game's process.
#[cfg(windows)]
mod win {
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetClassNameW, GetClientRect, GetForegroundWindow, GetSystemMetrics,
        GetWindowLongPtrW, GetWindowTextW, IsIconic, IsWindowVisible, SetWindowLongPtrW,
        SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST, SM_CXSCREEN, SM_CYSCREEN, SWP_HIDEWINDOW,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    use super::Rect;
    use crate::config;

    /// Explorer's folder windows: a folder named Overwatch has the game's title.
    const EXPLORER_CLASS: &str = "CabinetWClass";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn class_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: the buffer and its length match; a gone window gives 0.
        let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn title_of(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        // SAFETY: as above.
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn is_game(hwnd: HWND) -> bool {
        if hwnd.is_null() {
            return false;
        }
        let class = class_of(hwnd);
        class == config::GAME_WINDOW_CLASS
            || (class != EXPLORER_CLASS && title_of(hwnd) == config::GAME_WINDOW_TITLE)
    }

    /// The game window's client area on screen, and whether it's in front. `None` while the game
    /// isn't running or is minimised.
    pub fn game_window() -> Option<(Rect, bool)> {
        // SAFETY: plain calls about windows; every handle is checked before use.
        unsafe {
            let front = GetForegroundWindow();
            let in_front = is_game(front);
            let hwnd = if in_front {
                front
            } else {
                let class = wide(config::GAME_WINDOW_CLASS);
                FindWindowW(class.as_ptr(), null())
            };
            if hwnd.is_null() || IsIconic(hwnd) != 0 || IsWindowVisible(hwnd) == 0 {
                return None;
            }
            let mut client = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            let mut origin = POINT { x: 0, y: 0 };
            if GetClientRect(hwnd, &mut client) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
                return None;
            }
            let (width, height) = (client.right - client.left, client.bottom - client.top);
            (width > 0 && height > 0).then_some((
                Rect {
                    x: origin.x,
                    y: origin.y,
                    width,
                    height,
                },
                in_front,
            ))
        }
    }

    pub fn primary_screen() -> Rect {
        // SAFETY: plain calls.
        let (width, height) =
            unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    fn hwnd(window: &tauri::WebviewWindow) -> Option<HWND> {
        window.hwnd().ok().map(|h| h.0 as HWND)
    }

    /// Places the overlay on top without taking the focus, shown or hidden.
    pub fn place(window: &tauri::WebviewWindow, rect: Rect, visible: bool) {
        let Some(hwnd) = hwnd(window) else { return };
        let show = if visible {
            SWP_SHOWWINDOW
        } else {
            SWP_HIDEWINDOW
        };
        // SAFETY: the overlay's own window handle.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOACTIVATE | show,
            );
        }
    }

    /// A passive window is never activated (clicking it, or showing it, leaves the focus where it
    /// was) and isn't in Alt+Tab. Off while the host edits the layout.
    pub fn set_passive(window: &tauri::WebviewWindow, passive: bool) {
        let Some(hwnd) = hwnd(window) else { return };
        // SAFETY: the overlay's own window handle; only its extended style changes.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let flags = (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize;
            let style = if passive {
                style | flags
            } else {
                (style & !(WS_EX_NOACTIVATE as isize)) | WS_EX_TOOLWINDOW as isize
            };
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_while_the_game_is_in_front() {
        assert!(shows(false, false, true, true));
        assert!(!shows(false, false, true, false));
        // The host chose to see it over everything.
        assert!(shows(false, false, false, false));
        // Placing the widgets, the host may have tabbed out.
        assert!(shows(false, true, true, false));
        // Hidden with its hotkey wins.
        assert!(!shows(true, false, true, true));
    }
}
