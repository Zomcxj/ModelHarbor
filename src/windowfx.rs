//! Windows 窗口外观：DWM 亚克力背景与圆角。
//!
//! 玻璃效果分两层：窗口层（本模块）让 DWM 在窗口背后画模糊背景并把窗口切成圆角；
//! 绘制层（`theme::apply_style` + `App::clear_color`）用带 alpha 的底色把模糊透出来。
//! 窗口恒为透明窗口，玻璃开关只切 DWM 背景与绘制层 alpha。
//! 非 Windows 平台这些函数是空实现。

/// 窗口背景档位：DWM 在窗口背后画什么。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backdrop {
    /// 不透明：DWM 不画背景，由 egui 的清屏色盖满（默认档）。
    None,
    /// 亚克力：半透明 + 高斯模糊，桌面内容透出来（Win11 与 Win10 1803+ 可用）。
    Acrylic,
}

#[cfg(target_os = "windows")]
mod imp {
    use super::Backdrop;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };

    /// 主窗口句柄：`main.rs` 建窗后写入一次，之后所有外观调用都走它。
    ///
    /// 用 `OnceLock` 而不是 `App` 字段（与 `cursor` 模块同一套）。
    static MAIN_HWND: OnceLock<isize> = OnceLock::new();

    /// 记住主窗口句柄（`main.rs` 在窗口创建后调用一次）。
    pub fn set_main_hwnd(hwnd: isize) {
        let _ = MAIN_HWND.set(hwnd);
    }

    /// 按档位设置窗口背景。
    ///
    /// Win11（build 22000+）用 `DWMWA_SYSTEMBACKDROP_TYPE`；旧系统该属性被忽略，
    /// 静默降级为不透明。
    pub fn set_backdrop(backdrop: Backdrop) {
        let Some(&hwnd) = MAIN_HWND.get() else {
            return;
        };
        let hwnd = hwnd as HWND;
        let value = match backdrop {
            Backdrop::None => DWMSBT_NONE,
            // 亚克力（高斯模糊 + 半透明），不是云母（壁纸染色）。
            Backdrop::Acrylic => DWMSBT_TRANSIENTWINDOW,
        };
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &value as *const _ as *const core::ffi::c_void,
                std::mem::size_of_val(&value) as u32,
            );
        }
    }

    /// 把窗口切成系统圆角（Win11）；旧系统忽略。
    pub fn set_rounded_corners() {
        let Some(&hwnd) = MAIN_HWND.get() else {
            return;
        };
        let hwnd = hwnd as HWND;
        let value = DWMWCP_ROUND;
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &value as *const _ as *const core::ffi::c_void,
                std::mem::size_of_val(&value) as u32,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::Backdrop;

    pub fn set_main_hwnd(_hwnd: isize) {}

    pub fn set_backdrop(_backdrop: Backdrop) {}

    pub fn set_rounded_corners() {}
}

pub use imp::{set_backdrop, set_main_hwnd, set_rounded_corners};
