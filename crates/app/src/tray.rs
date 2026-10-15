//! 常驻任务栏（系统托盘）：仅 Windows。
//!
//! 「常驻」模式下点 X 只把主窗口隐藏进托盘，托盘右键菜单可唤回或退出。
//!
//! 托盘菜单回调运行在 trayicon 内部线程，而窗口隐藏后 egui 不再画帧——
//! 事件不能走 UI 队列（永远没人消费，唤回与退出都会石沉大海）。这里直接
//! 用跨线程安全的 Win32 调用：`ShowWindowAsync` 把显示操作投回窗口自己的
//! 消息队列；退出则 `PostMessageW(WM_CLOSE)` 走正常关闭流程，
//! `raw_input_hook` 见到 [`exit_requested`] 标记后放行。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// 菜单项标识（trayicon 要求 `PartialEq + Clone + Send + Sync`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    Show,
    Exit,
}

/// 退出标记：托盘菜单点了「退出」之后，关闭请求不再被常驻模式拦截。
static FORCE_EXIT: AtomicBool = AtomicBool::new(false);
static TRAY: OnceLock<Mutex<Option<trayicon::TrayIcon<Item>>>> = OnceLock::new();

/// 托盘是否已建好（常驻模式开关的当前态）。
pub fn is_active() -> bool {
    TRAY.get()
        .is_some_and(|slot| slot.lock().map(|g| g.is_some()).unwrap_or(false))
}

/// 托盘菜单是否点过「退出」：`raw_input_hook` 据此决定放行关闭。
pub fn exit_requested() -> bool {
    FORCE_EXIT.load(Ordering::Relaxed)
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
        // 左键单击直接唤回（Windows / KDE 生效）；右键保持默认弹菜单。
        .on_click(Item::Show)
        // 回调在托盘线程：窗口隐藏时 egui 不画帧，不能走 UI 队列，
        // 直接用跨线程安全的 Win32 调用。
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

/// 撤掉托盘图标（关闭常驻时调用）。
pub fn teardown() {
    if let Ok(mut slot) = TRAY.get_or_init(|| Mutex::new(None)).lock() {
        *slot = None;
    }
}

/// 给主窗口投一条 `WM_CLOSE`：走正常的 winit / eframe 关闭流程，
/// `raw_input_hook` 见到 [`exit_requested`] 后放行。拿不到句柄时直接退出。
fn post_close() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    match crate::windowfx::main_hwnd() {
        Some(hwnd) => unsafe {
            PostMessageW(hwnd as *mut core::ffi::c_void, WM_CLOSE, 0, 0);
        },
        None => std::process::exit(0),
    }
}

/// 主窗口收进托盘（隐藏窗口；托盘图标仍在）。主窗口线程调用。
///
/// 句柄由 `main.rs` 经 `windowfx::set_main_hwnd` 登记；拿不到就什么也不做。
pub fn hide_main_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if let Some(hwnd) = crate::windowfx::main_hwnd() {
        unsafe {
            ShowWindow(hwnd as *mut core::ffi::c_void, SW_HIDE);
        }
    }
}

/// 把主窗口从托盘唤回。托盘线程调用：`ShowWindowAsync` 是跨线程版本
/// （把显示操作投回窗口自己的消息队列），随后置前。
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
