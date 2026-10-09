use eframe::egui;

/// 吸顶标题占位：在内容流中预留标题行高度，返回绘制锚点。
/// 与 [`sticky_end`] 配对使用，在 section 内容渲染完成后调用后者。
pub(in crate::app) fn sticky_begin(ui: &mut egui::Ui, height: f32) -> (f32, f32, f32, f32) {
    let avail = ui.available_rect_before_wrap();
    ui.allocate_exact_size(egui::vec2(avail.width(), height), egui::Sense::hover());
    (avail.top(), avail.left(), avail.right(), height)
}

/// 吸顶条的 Y：未滚过时钉在内容流位置，滚过后钉在滚动区可视顶。
pub(in crate::app) fn sticky_y(content_top: f32, clip_top: f32, clip_margin: f32) -> f32 {
    content_top.max(clip_top + clip_margin).round()
}

/// 绘制吸顶标题：未滚过时留在内容流中，滚过后吸附在滚动区顶部。
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
    // 拦截层：吸顶条空白区域的点击/拖拽在此消费，不穿透到下层的卡片控件。
    child.interact(
        target,
        child.id().with("sticky-block"),
        egui::Sense::click_and_drag(),
    );
    // 背景从裁剪顶开始填充，避免吸附后在顶上留缝。
    let fill_top = if y > top { clip_top } else { target.top() };
    child.painter().rect_filled(
        egui::Rect::from_min_max(egui::pos2(target.left(), fill_top), target.max),
        0.0,
        ui.visuals().panel_fill,
    );
    paint(&mut child);
    // 标题下边线。
    child.painter().hline(
        target.x_range(),
        target.bottom() - 1.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}
