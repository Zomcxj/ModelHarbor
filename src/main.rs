#![windows_subsystem = "windows"]

use eframe::egui;
use model_harbor::app::App;

const ICON_BYTES: &[u8] = include_bytes!("../assets/icon_rgba.bin");
const ICON_W: u32 = 256;
const ICON_H: u32 = 256;

/// 把 panic 的位置与消息追加到 `<配置目录>/crash.log`。
///
/// release 构建是 `panic = "abort"`（无回溯、无控制台），界面又用了
/// `windows_subsystem = "windows"`，启动即崩时用户看不到任何线索；
/// 这条记录是唯一的事后证据。写不进去就安静放弃。
fn install_crash_logger() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<未知位置>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<无消息>".to_string());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        model_harbor::prefs::append_crash_log(&format!(
            "[unix {stamp}] panic at {location}\n{payload}\n\n"
        ));
    }));
}

fn main() -> eframe::Result {
    install_crash_logger();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1250.0, 820.0])
            .with_min_inner_size([970.0, 660.0])
            .with_title("ModelHarbor")
            .with_icon(egui::IconData {
                rgba: ICON_BYTES.to_vec(),
                width: ICON_W,
                height: ICON_H,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "ModelHarbor",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_fonts(build_cjk_fonts());
            // 主题不在这里写死：App 第一帧会按 prefs 里保存的主题套用
            // （这里若先套一个默认值，会和保存的主题打架，出现「按钮文字变了、界面没变」）。
            #[cfg(target_os = "windows")]
            {
                use raw_window_handle::HasWindowHandle;
                if let Ok(handle) = cc.window_handle() {
                    if let raw_window_handle::RawWindowHandle::Win32(w) = handle.as_raw() {
                        unsafe {
                            let hwnd = w.hwnd.get() as *mut core::ffi::c_void;
                            model_harbor::cursor::init_cursors(hwnd);
                        }
                    }
                }
            }
            Ok(Box::new(App::default()))
        }),
    )
}

fn build_cjk_fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for path in [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\msjh.ttc",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cjk".to_owned(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .push("cjk".to_owned());
            }
            break;
        }
    }
    fonts
}
