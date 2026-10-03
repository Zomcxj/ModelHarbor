use eframe::egui;

use super::drag::{DRAG_SOURCE_COLOR, DROP_TARGET_COLOR};

pub fn card_frame<R>(
    ui: &mut egui::Ui,
    open: bool,
    highlight: u8,
    id: egui::Id,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let corner = ui.visuals().widgets.noninteractive.corner_radius;
    let shape = crate::theme::active_style(ui.ctx());
    let fill = if open {
        ui.visuals().faint_bg_color
    } else {
        ui.visuals().extreme_bg_color
    };
    // 悬停 id 按卡片 id 稳定派生，不用 auto_id；矩形画完才有，存进临时存储供
    // 下一帧判定指针是否悬停（悬停状态滞后一帧）。
    let hover_id = id.with("hover");
    let mut hover_t = 0.0f32;
    let (stroke_color, stroke_width) = match highlight {
        1 => (DRAG_SOURCE_COLOR, 2.0), // source: orange
        2 => (DROP_TARGET_COLOR, 2.0), // target: green
        // 无高亮时跟随形状预设的描边宽度（恒宽），颜色向强调色做悬停过渡；
        // 高亮环由 [`draw_hover_ring`] 用 painter 画在矩形内侧，不参与布局。
        _ => {
            let last_rect = ui
                .ctx()
                .data(|data| data.get_temp::<egui::Rect>(hover_id.with("rect")));
            let pointer = ui.input(|input| input.pointer.hover_pos());
            let hovered =
                matches!((last_rect, pointer), (Some(rect), Some(pos)) if rect.contains(pos));
            let t = crate::motion::hover_t(ui.ctx(), hover_id, hovered);
            hover_t = t;
            let base = ui.visuals().widgets.noninteractive.bg_stroke.color;
            let target = hover_target_color(ui);
            let rest_width = if shape.has_card_shadow() {
                0.0
            } else {
                shape.border_width()
            };
            (crate::motion::lerp_color(base, target, t), rest_width)
        }
    };
    let mut frame = egui::Frame::NONE
        .fill(fill)
        .corner_radius(corner)
        .stroke(egui::Stroke::new(stroke_width, stroke_color))
        .inner_margin(egui::Margin::symmetric(12, 6));
    if shape.has_card_shadow() {
        frame = frame.shadow(shape.card_shadow(ui.visuals().dark_mode));
    }
    let frame = frame.show(ui, |ui| {
        ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 2.0);
        add(ui);
    });
    if highlight == 0 {
        ui.ctx().data_mut(|data| {
            data.insert_temp(hover_id.with("rect"), frame.response.rect);
        });
    }
    draw_accent_bar(ui, frame.response.rect);
    draw_relief(ui, frame.response.rect);
    draw_hover_ring(ui, frame.response.rect, hover_t, stroke_width);
    frame.response
}

/// 石板 / 浮雕的「坐在底上」质感：接触影为向右下偏移 1px 的整圈暗色描边
/// （跟随圆角，一半落在卡外）；浮雕再叠加凸起受光线（卡内左上亮、右下暗）。
fn draw_relief(ui: &egui::Ui, rect: egui::Rect) {
    let style = crate::theme::active_style(ui.ctx());
    if !style.has_contact_shadow() {
        return;
    }
    let shadow = style.contact_shadow_color(ui.visuals().dark_mode);
    let radius = ui.visuals().widgets.noninteractive.corner_radius;
    let painter = ui.painter();
    painter.rect_stroke(
        rect.translate(egui::vec2(1.0, 1.0)),
        radius,
        egui::Stroke::new(1.0f32, shadow),
        egui::StrokeKind::Inside,
    );
    if style.has_bevel() {
        let (light, dark) = style.bevel_colors(ui.visuals().dark_mode);
        let (light_lines, dark_lines) = bevel_segments(rect, radius.nw as f32);
        for segment in light_lines {
            painter.line_segment(segment, egui::Stroke::new(1.0f32, light));
        }
        for segment in dark_lines {
            painter.line_segment(segment, egui::Stroke::new(1.0f32, dark));
        }
    }
}

/// 色带在卡片上的覆盖区：**只有顶部 3px**。
pub fn accent_bar_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.top() + ACCENT_BAR_HEIGHT),
    )
}

/// 色带高度（像素）。
pub const ACCENT_BAR_HEIGHT: f32 = 3.0;

/// 色带：卡片顶部一条 3px 主题强调色，随顶角圆弧收边。
///
/// 只画顶部条带（painter 裁剪区限制），条带 = 整卡圆角矩形裁剪到顶部 3px，
/// 顶角圆弧天然对齐。painter 绘制，不参与布局。
fn draw_accent_bar(ui: &egui::Ui, rect: egui::Rect) {
    let style = crate::theme::active_style(ui.ctx());
    if !style.has_accent_bar() {
        return;
    }
    let accent = ui.visuals().hyperlink_color;
    let corner = ui.visuals().widgets.noninteractive.corner_radius;
    let painter = ui.painter().with_clip_rect(accent_bar_rect(rect));
    painter.rect_filled(rect, corner, accent);
}

/// 悬停高亮的目标色：深色主题用强调色，浅色主题用中性深灰（`from_gray(110)`）。
fn hover_target_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        ui.visuals().hyperlink_color
    } else {
        egui::Color32::from_gray(110)
    }
}

/// 悬停高亮环：只给**平时无边框**的形状（极简 / 云朵卡片）画，
/// 沿卡片内侧 1px，宽度随悬停进度浮现；painter 绘制，不参与布局。
fn draw_hover_ring(ui: &egui::Ui, rect: egui::Rect, hover_t: f32, rest_stroke_width: f32) {
    if hover_t <= 0.01 || rest_stroke_width > 0.0 {
        return;
    }
    let color = hover_target_color(ui);
    let corner = ui.visuals().widgets.noninteractive.corner_radius;
    ui.painter().rect_stroke(
        rect,
        corner,
        egui::Stroke::new(hover_t, color),
        egui::StrokeKind::Inside,
    );
}

/// 凸起浮雕的内嵌线段：`(亮边, 暗边)`，各两条。
///
/// 亮边在内侧左上，暗边在内侧右下；线段落在矩形内侧，两端避开圆角。
pub fn bevel_segments(
    rect: egui::Rect,
    radius: f32,
) -> ([[egui::Pos2; 2]; 2], [[egui::Pos2; 2]; 2]) {
    let inner = rect.shrink(1.0);
    let r = radius.min(inner.width() / 2.0).min(inner.height() / 2.0);
    let light = [
        [
            egui::pos2(inner.left() + r, inner.top()),
            egui::pos2(inner.right() - r, inner.top()),
        ],
        [
            egui::pos2(inner.left(), inner.top() + r),
            egui::pos2(inner.left(), inner.bottom() - r),
        ],
    ];
    let dark = [
        [
            egui::pos2(inner.left() + r, inner.bottom()),
            egui::pos2(inner.right() - r, inner.bottom()),
        ],
        [
            egui::pos2(inner.right(), inner.top() + r),
            egui::pos2(inner.right(), inner.bottom() - r),
        ],
    ];
    (light, dark)
}
