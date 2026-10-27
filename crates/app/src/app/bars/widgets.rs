use eframe::egui;

/// An icon button using an SVG texture: active, hover, and pressed states all give clear visual feedback.
pub(in crate::app) fn toolbar_icon_button(
    ui: &mut egui::Ui,
    texture: Option<&egui::TextureHandle>,
    active: bool,
    tint: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    let feedback_rect = rect.shrink(1.0);
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered();
    let visuals = ui.visuals();
    // The corner radius follows the current theme's shape (consistent with other widgets).
    let corner = visuals.widgets.inactive.corner_radius;
    let fill = if active {
        visuals.selection.bg_fill
    } else if pressed {
        visuals.widgets.active.bg_fill.gamma_multiply(0.35)
    } else if hovered {
        visuals.widgets.hovered.bg_fill.gamma_multiply(0.35)
    } else {
        egui::Color32::TRANSPARENT
    };
    if fill != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(feedback_rect, corner, fill);
    }
    ui.painter().rect_stroke(
        feedback_rect,
        corner,
        if pressed {
            visuals.widgets.active.bg_stroke
        } else if hovered {
            visuals.widgets.hovered.bg_stroke
        } else {
            visuals.widgets.inactive.bg_stroke
        },
        egui::StrokeKind::Inside,
    );
    if let Some(texture) = texture {
        ui.painter().image(
            texture.id(),
            rect.shrink(3.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    // Click acknowledgment: an accent ring expanding from the button.
    crate::motion::click_pulse(ui, &response, visuals.widgets.hovered.bg_fill, corner);
    response
}
