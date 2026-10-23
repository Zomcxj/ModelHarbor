use eframe::egui;

/// The sticky title's placeholder: reserves the title row's height in the content flow, returning the paint anchor.
/// Paired with [`sticky_end`], called after the section's content renders.
pub(in crate::app) fn sticky_begin(ui: &mut egui::Ui, height: f32) -> (f32, f32, f32, f32) {
    let avail = ui.available_rect_before_wrap();
    ui.allocate_exact_size(egui::vec2(avail.width(), height), egui::Sense::hover());
    (avail.top(), avail.left(), avail.right(), height)
}

/// The sticky bar's Y: pinned at the content-flow position before scrolling past; at the scroll area's visible top after.
pub(in crate::app) fn sticky_y(content_top: f32, clip_top: f32, clip_margin: f32) -> f32 {
    content_top.max(clip_top + clip_margin).round()
}

/// Paint the sticky title: it stays in the content flow until scrolled past, then sticks to the scroll area's top.
pub(in crate::app) fn sticky_end(
    ui: &mut egui::Ui,
    anchor: (f32, f32, f32, f32),
    paint: impl FnOnce(&mut egui::Ui),
) {
    let (top, left, right, height) = anchor;
    let clip_top = ui.clip_rect().top();
    let y = sticky_y(top, clip_top, ui.visuals().clip_rect_margin);
    let target = egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(right, y + height));
    if !ui.clip_rect().intersects(target) {
        return;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(target));
    // The interception layer: clicks / drags on the sticky bar's blank areas are consumed here, never falling through to the card widgets below.
    child.interact(
        target,
        child.id().with("sticky-block"),
        egui::Sense::click_and_drag(),
    );
    // The background fills from the clip top, avoiding a gap at the top after sticking.
    let fill_top = if y > top { clip_top } else { target.top() };
    child.painter().rect_filled(
        egui::Rect::from_min_max(egui::pos2(target.left(), fill_top), target.max),
        0.0,
        ui.visuals().panel_fill,
    );
    paint(&mut child);
    // The title's bottom rule.
    child.painter().hline(
        target.x_range(),
        target.bottom() - 1.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}
