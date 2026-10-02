use eframe::egui;

use super::cursor::{grab_cursor_for, request_grab_cursor, GrabCursor};

/// 拖动把手的点阵间距（像素）。
pub const DRAG_HANDLE_GAP: f32 = 5.0;

/// 拖动**源**（正被抓着的那一项）的高亮色：卡片描边与把手点阵共用同一个值，
/// 两处各写一份常量时改了一边就会「把手是橙色、卡片是别的颜色」。
pub const DRAG_SOURCE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 180, 50);

/// 拖动源把手的底色：同色压暗，垫在点阵下面，避免高饱和色块盖过点阵。
pub const DRAG_SOURCE_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(70, 49, 13, 90);

/// 拖动**落点**（当前指针所在的目标项）的高亮色：绿色。
///
/// 与 [`DRAG_SOURCE_COLOR`] 同源：卡片描边、页签选中态都用这两个值，
/// 各写一份常量就会出现「卡片是绿、页签是别的颜色」。
pub const DROP_TARGET_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 200, 100);

/// 落点/选中态色底的压暗版，垫在图标或点阵下面。
pub const DROP_TARGET_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(27, 55, 27, 90);

/// 拖动把手点阵的圆心：按给定矩形**居中**排布。
///
/// 抽成纯函数是为了能直接断言「点阵中心与控件中心重合」——写死偏移时，
/// 控件被 `interact_size` 撑高后点阵会留在偏上的位置，与同一行其他控件对不齐。
pub fn drag_handle_dots(rect: egui::Rect) -> Vec<egui::Pos2> {
    let center = rect.center();
    let mut dots = Vec::with_capacity(6);
    for row in 0..3 {
        for col in 0..2 {
            dots.push(egui::pos2(
                center.x - DRAG_HANDLE_GAP / 2.0 + col as f32 * DRAG_HANDLE_GAP,
                center.y - DRAG_HANDLE_GAP + row as f32 * DRAG_HANDLE_GAP,
            ));
        }
    }
    dots
}

pub struct DragHandle;

impl egui::Widget for DragHandle {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let button = egui::Button::new("")
            .frame(false)
            .sense(egui::Sense::click_and_drag())
            .min_size(egui::vec2(14.0, 18.0));
        let resp = ui.add(button);
        let dragging = resp.dragged();
        let active = resp.hovered() || dragging;
        // 拖动中除了提亮点阵，还铺一层高亮底色：卡片被拖时整卡会亮起橙色边框，
        // 把手作为「抓取点」也得有同源的选中态，否则光标一走开就看不出抓着谁。
        if dragging {
            ui.painter().rect_filled(resp.rect, 3.0, DRAG_SOURCE_FILL);
        }
        let color = if dragging {
            DRAG_SOURCE_COLOR
        } else if active {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        // 点阵半径在拖动时再放大一档，与边框一起构成「已抓起」的观感。
        let radius = if dragging {
            1.75
        } else if active {
            1.5
        } else {
            1.0
        };
        let painter = ui.painter();
        for c in drag_handle_dots(resp.rect) {
            painter.circle_filled(c, radius, color);
        }
        // 光标：悬停是张开的手掌、按住才是握起的拳头，与真实桌面软件一致
        // （见 `crate::cursor`）。**不要用 egui 的 `CursorIcon::Grab`**：它在 Windows 上
        // 经 winit 映射成 `IDC_SIZEALL`（四向箭头），看着像「可移动」而不是「抓住」。
        //
        // 自定义光标是整窗生效的（子类过程拦 WM_SETCURSOR），因此这里只提出请求，
        // 由 `App::update` 帧末统一提交；`PointingHand` 作为非 Windows 或光标
        // 句柄创建失败时的兜底。
        let want = grab_cursor_for(&resp);
        if want != GrabCursor::None {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            request_grab_cursor(ui.ctx(), want);
        }
        resp
    }
}

pub fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= items.len() || to >= items.len() {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

/// 把一张卡片的拖拽落点并入本帧的聚合结果。
///
/// 语义是「只增不减」：`found` 为 `None` 时必须保留 `acc` 中已有的落点。卡片是逐张渲染的，
/// 若每张卡片各自写入落点，后面渲染的卡片会把前面命中的落点清成 `None`，被拖到的卡片就拿不到
/// 落点边框（模型卡片的绿色边框曾因此消失）。
pub fn merge_drag_target(acc: &mut Option<String>, found: Option<String>) {
    if acc.is_none() {
        *acc = found;
    }
}
