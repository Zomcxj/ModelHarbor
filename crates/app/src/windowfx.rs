//! Windows window appearance: DWM acrylic backdrop and rounded corners.
//!
//! The acrylic effect has two layers: the window layer (this module) has DWM paint a blurred backdrop
//! behind the window and round the window's corners; the paint layer (`theme::apply_style` + `App::clear_color`)
//! uses alpha-bearing fills to let the blur show through. The window is always transparent; the acrylic
//! toggle switches only the DWM backdrop and the paint layer's alpha.
//! Non-Windows platforms make these functions no-ops.

/// The window backdrop tier: what DWM paints behind the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backdrop {
    /// Opaque: DWM paints no backdrop; egui's clear color fills it (the default tier).
    None,
    /// Acrylic: translucent + Gaussian blur, desktop content showing through (Win11 and Win10 1803+).
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

    /// The main window's handle: written once by `main.rs` after window creation; every appearance call uses it after.
    ///
    /// A `OnceLock` rather than an `App` field (the same approach as the `cursor` module).
    static MAIN_HWND: OnceLock<isize> = OnceLock::new();

    /// Remember the main window's handle (`main.rs` calls once after window creation).
    pub fn set_main_hwnd(hwnd: isize) {
        let _ = MAIN_HWND.set(hwnd);
    }

    /// The main window's handle (`None` before registration).
    pub fn main_hwnd() -> Option<isize> {
        MAIN_HWND.get().copied()
    }

    /// Set the window backdrop per tier.
    ///
    /// Win11 (build 22000+) uses `DWMWA_SYSTEMBACKDROP_TYPE`; older systems ignore the attribute and silently
    /// degrade to opaque.
    pub fn set_backdrop(backdrop: Backdrop) {
        let Some(&hwnd) = MAIN_HWND.get() else {
            return;
        };
        let hwnd = hwnd as HWND;
        let value = match backdrop {
            Backdrop::None => DWMSBT_NONE,
            // Acrylic (Gaussian blur + translucency), not mica (wallpaper-tinted).
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

    /// Round the window with system corners (Win11); older systems ignore.
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

    pub fn main_hwnd() -> Option<isize> {
        None
    }

    pub fn set_backdrop(_backdrop: Backdrop) {}

    pub fn set_rounded_corners() {}
}

pub use imp::{main_hwnd, set_backdrop, set_main_hwnd, set_rounded_corners};
