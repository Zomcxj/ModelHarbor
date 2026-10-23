//! The resident taskbar (system tray): Windows only.
//!
//! In "resident" mode, clicking X only hides the main window into the tray; the tray's context menu can
//! restore or quit.
//!
//! Tray menu callbacks run on trayicon's internal thread, and once the window hides, egui stops painting
//! frames — events can't go through the UI queue (nobody would ever consume them; restore and quit would
//! sink without a trace). Cross-thread-safe Win32 calls are used directly: `ShowWindowAsync` posts the
//! show operation back to the window's own message queue; quitting posts `PostMessageW(WM_CLOSE)` through
//! the normal close flow, with `raw_input_hook` letting it through once [`exit_requested`] is flagged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Menu item identifiers (trayicon requires `PartialEq + Clone + Send + Sync`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Show,
    Exit,
}

/// The exit flag: after the tray menu's "quit", close requests are no longer intercepted by resident mode.
static FORCE_EXIT: AtomicBool = AtomicBool::new(false);
static TRAY: OnceLock<Mutex<Option<trayicon::TrayIcon<Item>>>> = OnceLock::new();

/// Whether the tray exists (the resident toggle's current state).
pub fn is_active() -> bool {
    TRAY.get()
        .is_some_and(|slot| slot.lock().map(|g| g.is_some()).unwrap_or(false))
}

/// Whether the tray menu's "quit" was clicked: `raw_input_hook` lets closes through on this.
pub fn exit_requested() -> bool {
    FORCE_EXIT.load(Ordering::Relaxed)
}

/// Build the tray icon (idempotent): the icon comes from `assets/icon.ico`, with two menu items.
///
/// A failed build (rare environments without a tray) gives up silently: residency degrades to plain minimize.
pub fn ensure() {
    if is_active() {
        return;
    }
    let menu = trayicon::MenuBuilder::new()
        .item("显示主窗口", Item::Show)
        .separator()
        .item("退出", Item::Exit);
    let Ok(icon) = trayicon::Icon::from_buffer(include_bytes!("../assets/icon.ico"), None, None)
    else {
        return;
    };
    let Ok(tray) = trayicon::TrayIconBuilder::new()
        .tooltip("ModelHarbor")
        .icon(icon)
        .menu(menu)
        // Left-click restores directly (works on Windows / KDE); right-click keeps the default menu.
        .on_click(Item::Show)
        // The callback runs on the tray thread: with the window hidden, egui paints no frames, so the UI queue
        // is out — cross-thread-safe Win32 calls are used directly.
        .sender(|item| match item {
            Item::Show => show_main_window(),
            Item::Exit => {
                FORCE_EXIT.store(true, Ordering::Relaxed);
                post_close();
            }
        })
        .build()
    else {
        return;
    };
    if let Ok(mut slot) = TRAY.get_or_init(|| Mutex::new(None)).lock() {
        *slot = Some(tray);
    }
}

/// Remove the tray icon (called when residency turns off).
pub fn teardown() {
    if let Ok(mut slot) = TRAY.get_or_init(|| Mutex::new(None)).lock() {
        *slot = None;
    }
}

/// Post a `WM_CLOSE` to the main window: the normal winit / eframe close flow, let through once
/// `raw_input_hook` sees [`exit_requested`]. Without the handle, exits directly.
fn post_close() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    match crate::windowfx::main_hwnd() {
        Some(hwnd) => unsafe {
            PostMessageW(hwnd as *mut core::ffi::c_void, WM_CLOSE, 0, 0);
        },
        None => std::process::exit(0),
    }
}

/// Tuck the main window into the tray (hide the window; the tray icon stays). Called on the main window's thread.
///
/// The handle registers via `main.rs` through `windowfx::set_main_hwnd`; without it, do nothing.
pub fn hide_main_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if let Some(hwnd) = crate::windowfx::main_hwnd() {
        unsafe {
            ShowWindow(hwnd as *mut core::ffi::c_void, SW_HIDE);
        }
    }
}

/// Restore the main window from the tray. Called on the tray thread: `ShowWindowAsync` is the
/// cross-thread variant (posting the show operation back to the window's own message queue), then brought to front.
pub fn show_main_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, ShowWindowAsync, SW_RESTORE,
    };
    if let Some(hwnd) = crate::windowfx::main_hwnd() {
        let hwnd = hwnd as *mut core::ffi::c_void;
        unsafe {
            ShowWindowAsync(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
}
