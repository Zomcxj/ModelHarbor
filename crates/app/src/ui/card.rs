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
    // The hover id derives stably from the card's id, never auto_id; the rect exists only after painting,
    // stored in temp storage so the next frame can judge pointer hover (hover state lags one frame).
    let hover_id = id.with("hover");
    let mut hover_t = 0.0f32;
    let (stroke_color, stroke_width) = match highlight {
        1 => (DRAG_SOURCE_COLOR, 2.0), // source: orange
        2 => (DROP_TARGET_COLOR, 2.0), // target: green
        // Without a highlight, follow the shape preset's border width (constant), with the color transitioning
        // toward the accent on hover; the highlight ring is painted inside the rect by [`draw_hover_ring`],
        // taking no part in layout.
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

/// The slab / emboss "sitting on the surface" texture: the contact shadow is a full dark border ring offset
/// 1px toward the bottom-right (following the corners, half falling outside the card); emboss then adds
/// raised light-catching edges (bright top-left inside the card, dark bottom-right).
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

/// The ribbon's coverage on a card: **only the top 3px**.
pub fn accent_bar_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.top() + ACCENT_BAR_HEIGHT),
    )
}

/// The ribbon's height (pixels).
pub const ACCENT_BAR_HEIGHT: f32 = 3.0;

/// The ribbon: a 3px theme-accent strip along the card's top, trimmed by the top corners' arcs.
///
/// Only the top strip paints (painter clip limits): the strip is the card's full rounded rect clipped to
/// the top 3px, so the corner arcs align naturally. Painted — no layout participation.
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

/// The hover highlight's target color: the accent on dark themes, a neutral deep gray on light ones (`from_gray(110)`).
fn hover_target_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        ui.visuals().hyperlink_color
    } else {
        egui::Color32::from_gray(110)
    }
}

/// The hover highlight ring: painted only for **usually borderless** shapes (minimal / cloud cards),
/// along the card's inner 1px, its width emerging with hover progress; painter-painted, no layout.
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

/// The raised emboss's inset line segments: `(bright edge, dark edge)`, two of each.
///
/// Bright edges inset top-left, dark edges inset bottom-right; segments sit inside the rect, avoiding
/// the corners at both ends.
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
