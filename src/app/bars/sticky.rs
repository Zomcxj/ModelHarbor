use eframe::egui;

/// 吸顶标题占位：在内容流中预留标题行高度，返回绘制锚点。
/// 必须与 [`sticky_end`] 配对，并在 section 内容渲染完成后调用 sticky_end，
/// 以保证标题最后绘制（否则会被下方滚动内容覆盖）。
pub(in crate::app) fn sticky_begin(ui: &mut egui::Ui, height: f32) -> (f32, f32, f32, f32) {
    let avail = ui.available_rect_before_wrap();
    ui.allocate_exact_size(egui::vec2(avail.width(), height), egui::Sense::hover());
    (avail.top(), avail.left(), avail.right(), height)
}

/// 吸顶条的 Y：还没滚过标题时钉在内容流位置，滚过之后钉在滚动区**可视顶**。
///
/// egui 会把滚动区裁剪顶向上扩 `clip_rect_margin`（默认 3px，
/// `content_clip_rect.min.y = inner_rect.min.y - margin`）。如果直接用
/// `clip_rect().top()`，标题要多滚 3px 才停住——看起来整行先往上抬一下。
/// 可视顶 = clip_top + margin，吸顶钉在它上面就纹丝不动。
/// 抽成纯函数是为了能直接断言这两条性质；`round` 收掉亚像素差。
pub(in crate::app) fn sticky_y(content_top: f32, clip_top: f32, clip_margin: f32) -> f32 {
    content_top.max(clip_top + clip_margin).round()
}

/// 绘制吸顶标题：未滚过时留在内容流中；滚动越过视口顶部后吸附在滚动区顶部。
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
    // 用 new_child 而非 scope_builder：后者会推进父 cursor 到吸顶位置，
    // 破坏内容流导致滚动区滚轮失效。
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(target));
    // 拦截层：吸顶条空白区域（标题文字间隙等）的点击/拖拽先被此层消费，
    // 不再穿透到其正下方的卡片控件（误删/误展开/误拖放目标）。
    // 先注册，后续的按钮仍在其上层优先响应。
    child.interact(
        target,
        child.id().with("sticky-block"),
        egui::Sense::click_and_drag(),
    );
    // 背景填充要从**裁剪顶**开始，不能只填 target：吸附后 target.top() =
    // clip_top + clip_margin，与裁剪顶还差那 3px；只填 target 会在顶上留一条缝，
    // 滚动内容从缝里透出来，看起来像整行透明。未吸附时（target.top() 还在
    // 内容流里）不需要补——缝里本来就是空白。
    let fill_top = if y > top { clip_top } else { target.top() };
    child.painter().rect_filled(
        egui::Rect::from_min_max(egui::pos2(target.left(), fill_top), target.max),
        0.0,
        ui.visuals().panel_fill,
    );
    paint(&mut child);
    // 标题下边线：吸顶时也能与内容分隔
    child.painter().hline(
        target.x_range(),
        target.bottom() - 1.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}
