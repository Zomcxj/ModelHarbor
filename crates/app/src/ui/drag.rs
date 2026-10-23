use eframe::egui;

use super::cursor::{grab_cursor_for, request_grab_cursor, GrabCursor};

/// The drag handle's dot-matrix pitch (pixels).
pub const DRAG_HANDLE_GAP: f32 = 5.0;

/// The drag **source** (the item currently grabbed)'s highlight: shared by the card border and the handle's dots.
pub const DRAG_SOURCE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 180, 50);

/// The drag-source handle's fill: the same color darkened, sitting beneath the dots.
pub const DRAG_SOURCE_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(70, 49, 13, 90);

/// The drag **drop target** (the item under the pointer)'s highlight: green.
///
/// Card borders and tab selection share this value.
pub const DROP_TARGET_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 200, 100);

/// The drop/selected color's darkened variant, beneath icons or dots.
pub const DROP_TARGET_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(27, 55, 27, 90);

/// The drag handle's dot centers: laid out **centered** in the given rect.
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
        // Mid-drag lays a highlight fill beneath the raised dots.
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
        // The dot radius grows another step while dragging.
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
        // Cursor: an open palm on hover, a closed fist while held (see `crate::cursor`).
        // egui's `CursorIcon::Grab` isn't used.
        // The custom cursor takes effect window-wide; here only the request is raised, committed centrally at
        // `App::update`'s frame end; `PointingHand` is the fallback for non-Windows or failed handle creation.
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

/// Merge one card's drag drop target into this frame's aggregate.
///
/// Add-only: when `found` is `None`, the landing already in `acc` is kept.
pub fn merge_drag_target(acc: &mut Option<String>, found: Option<String>) {
    if acc.is_none() {
        *acc = found;
    }
}
