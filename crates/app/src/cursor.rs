#![cfg(target_os = "windows")]

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

#[cfg(debug_assertions)]
use std::io::Write;

#[cfg(debug_assertions)]
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, DIB_RGB_COLORS,
    RGBQUAD,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, SUBCLASSPROC};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, LoadCursorW, SetCursor, ICONINFO, IDC_HAND,
};

#[cfg(debug_assertions)]
static DEBUG_FILE: OnceLock<std::sync::Mutex<Option<std::fs::File>>> = OnceLock::new();

/// Debug builds write a debug log (temp directory); release builds no-op.
#[cfg(debug_assertions)]
fn debug_log(msg: &str) {
    let guard = DEBUG_FILE.get_or_init(|| std::sync::Mutex::new(None));
    let mut lock = guard.lock().unwrap();
    if lock.is_none() {
        let path = std::env::temp_dir().join("model_harbor_cursor_debug.log");
        *lock = std::fs::File::create(path).ok();
    }
    if let Some(ref mut f) = *lock {
        let _ = f.write_all(msg.as_bytes());
        let _ = f.flush();
    }
}

#[cfg(not(debug_assertions))]
fn debug_log(_msg: &str) {}

/// The custom cursor to use now: an open palm on hover, a closed fist while held.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CustomCursor {
    /// Don't manage the cursor; hand it back to egui / the system.
    None,
    /// Hover state: the system's own hand cursor (`IDC_HAND`).
    Palm,
    /// Held mid-drag: the closed fist.
    Fist,
}

static CUSTOM_CURSOR: AtomicU8 = AtomicU8::new(CustomCursor::None as u8);
/// The system hand-cursor handle (`LoadCursorW(null, IDC_HAND)`).
///
/// A shared resource owned by the system, not this process — never `DestroyIcon`.
static PALM_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static FIST_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Set the current custom cursor, applying it once immediately (`None` hands it back to egui / the system).
///
/// State lives in an atomic; the subclass proc picks the handle by it on every `WM_SETCURSOR`.
pub fn set_custom_cursor(cursor: CustomCursor) {
    CUSTOM_CURSOR.store(cursor as u8, Ordering::Relaxed);
    let ptr = match cursor {
        CustomCursor::None => return,
        CustomCursor::Palm => PALM_HANDLE.load(Ordering::Relaxed),
        CustomCursor::Fist => FIST_HANDLE.load(Ordering::Relaxed),
    };
    if !ptr.is_null() {
        // SAFETY: `ptr` is written only by `init_cursors` from `CreateIconIndirect`'s return value;
        // nulls are excluded here; non-null means an HICON this process owns, which `SetCursor` only reads.
        unsafe {
            debug_log(&format!("Win32 SetCursor custom {:?}\r\n", ptr));
            SetCursor(ptr);
        }
    }
}

const CURSOR_W: i32 = 20;
const CURSOR_H: i32 = 20;
const CURSOR_HX: i32 = 10;
const CURSOR_HY: i32 = 10;

/// The fist (held mid-drag)'s RGBA pixel data.
const FIST_RGBA: &[u8] = include_bytes!("../assets/grab_rgba.bin");

/// Create a cursor from BGRA pixel data using Win32 GDI.
///
/// # Safety
///
/// `rgba` must hold at least `w * h * 4` bytes; short buffers copy only their actual length (the rest of
/// GDI's buffer stays zero-initialized). The returned HICON belongs to the caller — `DestroyIcon` when
/// done.
unsafe fn create_icon_from_rgba(rgba: &[u8], w: i32, h: i32, hx: i32, hy: i32) -> *mut c_void {
    // SAFETY: the inputs guarantee the pixel buffer's length; every GDI handle is released in pairs inside
    // the function (`DeleteObject` / `ReleaseDC`), and `GetDC(null)` takes the screen DC, so the paired
    // `ReleaseDC` also passes a null window.
    unsafe {
        let hdc = GetDC(std::ptr::null_mut());

        let bmi = windows_sys::Win32::Graphics::Gdi::BITMAPINFO {
            bmiHeader: windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER {
                biSize: std::mem::size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>()
                    as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbReserved: 0,
            }; 1],
        };

        let mut pixels: *mut c_void = std::ptr::null_mut();
        let hbm_color = CreateDIBSection(
            hdc,
            &bmi,
            DIB_RGB_COLORS,
            &mut pixels,
            std::ptr::null_mut(),
            0,
        );

        if !hbm_color.is_null() && !pixels.is_null() {
            let copy_len = ((w * h * 4) as usize).min(rgba.len());
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), pixels as *mut u8, copy_len);
        }

        let hbm_mask = CreateCompatibleBitmap(hdc, w, h);

        let icon_info = ICONINFO {
            fIcon: 0,
            xHotspot: hx as u32,
            yHotspot: hy as u32,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };

        let hicon = CreateIconIndirect(&icon_info);

        DeleteObject(hbm_mask as _);
        DeleteObject(hbm_color as _);
        ReleaseDC(std::ptr::null_mut(), hdc);

        hicon
    }
}

unsafe extern "system" fn cursor_subclass_proc(
    hwnd: HWND,
    u_msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
    _uid_subclass: usize,
    _dw_ref_data: usize,
) -> LRESULT {
    // SAFETY: this function is called only by `SetWindowSubclass` inside `hwnd`'s message loop;
    // `hwnd` and `l_param` arrive from the system per the window-message contract.
    // On WM_SETCURSOR while a custom cursor is active, pick palm / fist by state.
    unsafe {
        if u_msg == 0x0020 && (l_param as u32 & 0xffff) == 1 {
            let cursor_handle = match CUSTOM_CURSOR.load(Ordering::Relaxed) {
                x if x == CustomCursor::Palm as u8 => PALM_HANDLE.load(Ordering::Relaxed),
                x if x == CustomCursor::Fist as u8 => FIST_HANDLE.load(Ordering::Relaxed),
                _ => std::ptr::null_mut(),
            };
            if !cursor_handle.is_null() {
                SetCursor(cursor_handle);
                return 1;
            }
        }
        DefSubclassProc(hwnd, u_msg, w_param, l_param)
    }
}

/// # Safety
///
/// `hwnd` must be a valid Win32 window handle owned by the current process, callable once per window;
/// internally `SetWindowSubclass` installs the subclass proc — the caller must not install twice while
/// the window's message loop lives.
///
/// Public API for the binary target (`src/main.rs` calls it after window creation).
pub unsafe fn init_cursors(hwnd: HWND) {
    debug_log(&format!("init_cursors hwnd={:?}\r\n", hwnd));
    // The palm uses the system's own IDC_HAND: system-owned, shared across processes; this process neither creates nor destroys it.
    let palm = LoadCursorW(std::ptr::null_mut(), IDC_HAND);
    let fist = create_icon_from_rgba(FIST_RGBA, CURSOR_W, CURSOR_H, CURSOR_HX, CURSOR_HY);
    PALM_HANDLE.store(palm, Ordering::Relaxed);
    FIST_HANDLE.store(fist, Ordering::Relaxed);
    debug_log(&format!("cursors loaded palm={palm:?} fist={fist:?}\r\n"));

    let subclass_proc: SUBCLASSPROC = Some(cursor_subclass_proc);
    let ret = SetWindowSubclass(hwnd, subclass_proc, 1, 0);
    debug_log(&format!("SetWindowSubclass ret={ret}\r\n"));
}
