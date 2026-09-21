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

/// 仅 debug 构建写调试日志（临时目录），release 构建为空操作。
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

/// 当前要用的自定义光标。
///
/// 两态是为了让拖动把手 / 页签的反馈和真实桌面软件一致：**悬停是张开的手掌，
/// 按住才握成拳头**。只做拳头（旧实现）时，鼠标一进热区就显示「已抓住」，
/// 但此时还没按下，语义提前了。
///
/// 悬停态的「手掌」直接用**系统自带的 `IDC_HAND`**，不再自绘。
/// 自绘过两版都不行：20×20 是光标的真实尺寸，四指的指缝在那个尺寸下只有
/// 亚像素宽，降采样后指缝糊掉、整只手化成一坨；把手指加粗又变成连指手套。
/// 系统光标是按这个尺寸专门手工调过像素的，任何缩放的位图都比不过它。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CustomCursor {
    /// 不接管光标，交给 egui / 系统。
    None,
    /// 悬停在可拖动处：系统自带的手形光标（`IDC_HAND`）。
    Palm,
    /// 已按住拖动中：握起的拳头。
    Fist,
}

static CUSTOM_CURSOR: AtomicU8 = AtomicU8::new(CustomCursor::None as u8);
/// 系统手形光标句柄（`LoadCursorW(null, IDC_HAND)`）。
///
/// 这是**共享资源**，不属于本进程，绝不能 `DestroyIcon`——销毁它会破坏
/// 整个系统的手形光标，其他程序也跟着坏。
static PALM_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static FIST_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// 设置当前自定义光标，并立即应用一次（`None` 时交回 egui / 系统）。
///
/// 状态存在原子量里，子类过程每次 `WM_SETCURSOR` 都按它挑句柄，因此拖动中
/// 即使指针移到别的控件上，光标也不会被那个控件改回系统样式。
pub fn set_custom_cursor(cursor: CustomCursor) {
    CUSTOM_CURSOR.store(cursor as u8, Ordering::Relaxed);
    let ptr = match cursor {
        CustomCursor::None => return,
        CustomCursor::Palm => PALM_HANDLE.load(Ordering::Relaxed),
        CustomCursor::Fist => FIST_HANDLE.load(Ordering::Relaxed),
    };
    if !ptr.is_null() {
        // SAFETY: `ptr` 只在 `init_cursors` 里由 `CreateIconIndirect` 的返回值写入，
        // 此处已排除空指针；非空即为本进程拥有的 HICON，`SetCursor` 只读取它。
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

/// 拳头（按下拖动中）。手指短、握起。
const FIST_RGBA: &[u8] = include_bytes!("../assets/grab_rgba.bin");

/// Create a cursor from BGRA pixel data using Win32 GDI.
///
/// # Safety
///
/// `rgba` 必须有至少 `w * h * 4` 字节；不足时只拷贝实际长度（GDI 缓冲区剩余部分保持零初始化）。
/// 返回的 HICON 归调用方所有，不再使用时需 `DestroyIcon`。
unsafe fn create_icon_from_rgba(rgba: &[u8], w: i32, h: i32, hx: i32, hy: i32) -> *mut c_void {
    // SAFETY: 入参已保证像素缓冲区长度；每个 GDI 句柄在函数内成对释放（`DeleteObject` /
    // `ReleaseDC`），`GetDC(null)` 取的是屏幕 DC，因此配对的 `ReleaseDC` 也传空窗口。
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
    // SAFETY: 本函数只由 `SetWindowSubclass` 在 `hwnd` 的消息循环里调用，`hwnd` 与
    // `l_param` 由系统按窗口消息约定传入。命中 WM_SETCURSOR 且当前处于自定义
    // 光标态时，按状态挑手掌/拳头（不再用 `_dw_ref_data` 里固定的那一个，
    // 因为同一个热区要在「悬停」和「按住」两种光标间切换）。
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
/// `hwnd` 必须是当前进程拥有的有效 Win32 窗口句柄，且每个窗口仅可调用一次；
/// 内部通过 `SetWindowSubclass` 安装子类过程，调用方需保证窗口消息循环存活期间不重复安装。
///
/// 对二进制目标是公开 API（`src/main.rs` 在窗口创建后调用），因此不能收窄可见性。
pub unsafe fn init_cursors(hwnd: HWND) {
    debug_log(&format!("init_cursors hwnd={:?}\r\n", hwnd));
    // 手掌直接用系统自带的 IDC_HAND：这个句柄归系统所有、跨进程共享，
    // 所以本进程既不创建也不销毁它（销毁会连带弄坏别处的手形光标）。
    let palm = LoadCursorW(std::ptr::null_mut(), IDC_HAND);
    let fist = create_icon_from_rgba(FIST_RGBA, CURSOR_W, CURSOR_H, CURSOR_HX, CURSOR_HY);
    PALM_HANDLE.store(palm, Ordering::Relaxed);
    FIST_HANDLE.store(fist, Ordering::Relaxed);
    debug_log(&format!("cursors loaded palm={palm:?} fist={fist:?}\r\n"));

    let subclass_proc: SUBCLASSPROC = Some(cursor_subclass_proc);
    let ret = SetWindowSubclass(hwnd, subclass_proc, 1, 0);
    debug_log(&format!("SetWindowSubclass ret={ret}\r\n"));
}
