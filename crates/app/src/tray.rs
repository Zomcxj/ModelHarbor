//! 常驻任务栏（系统托盘）：仅 Windows。
//!
//! 「常驻」模式下主窗口最小化到托盘而不退出：点 X 只是隐藏窗口，托盘
//! 图标双击或右键「显示主窗口」即可唤回；右键「退出」才真正关闭。
//!
//! 托盘菜单的回调运行在系统托盘线程：只往全局队列里塞事件，主窗口
//! 每帧在 `raw_input_hook` 里排空（见 [`drain_events`]）。

use std::sync::{Mutex, OnceLock};

/// 托盘菜单事件：由托盘线程写入，主窗口线程消费。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// 显示主窗口（托盘双击 / 右键「显示主窗口」）。
    Show,
    /// 退出程序（右键「退出」）。
    Exit,
}

/// 菜单项标识（trayicon 要求 `PartialEq + Clone + Send + Sync`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Show,
    Exit,
}

static EVENTS: OnceLock<Mutex<Vec<TrayEvent>>> = OnceLock::new();
static TRAY: OnceLock<Mutex<Option<trayicon::TrayIcon<Item>>>> = OnceLock::new();

fn events() -> &'static Mutex<Vec<TrayEvent>> {
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// 托盘是否已建好（常驻模式开关的当前态）。
pub fn is_active() -> bool {
    TRAY.get()
        .is_some_and(|slot| slot.lock().map(|g| g.is_some()).unwrap_or(false))
}

/// 排空托盘事件队列（每帧调用一次，主窗口线程）。
pub fn drain_events() -> Vec<TrayEvent> {
    events()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// 建托盘图标（幂等）：图标取 `assets/icon.ico`，菜单两项。
///
/// 建失败（极少数环境无托盘）静默放弃：常驻退化为普通最小化。
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
        .sender(|item| {
            let event = match item {
                Item::Show => TrayEvent::Show,
                Item::Exit => TrayEvent::Exit,
            };
            if let Ok(mut queue) = events().lock() {
                queue.push(event);
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

/// 撤掉托盘图标（关闭常驻时调用）。
pub fn teardown() {
    if let Ok(mut slot) = TRAY.get_or_init(|| Mutex::new(None)).lock() {
        *slot = None;
    }
    if let Ok(mut queue) = events().lock() {
        queue.clear();
    }
}

/// 主窗口最小化到托盘（隐藏窗口；托盘图标仍在）。
///
/// 句柄由 `main.rs` 经 `windowfx::set_main_hwnd` 登记；拿不到就退回
/// 普通最小化，行为不至于退化。
pub fn hide_main_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if let Some(hwnd) = crate::windowfx::main_hwnd() {
        unsafe {
            ShowWindow(hwnd as *mut core::ffi::c_void, SW_HIDE);
        }
    }
}

/// 把主窗口从托盘唤回（显示并还原）。
pub fn show_main_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_RESTORE};
    if let Some(hwnd) = crate::windowfx::main_hwnd() {
        unsafe {
            ShowWindow(hwnd as *mut core::ffi::c_void, SW_RESTORE);
        }
    }
}
