//! 用量视图的组件基元：小节标题、卡片容器、可滚动表体。
//!
//! 各模块（总览卡 / 趋势图 / 热力图 / 四张表）共用同一套外壳，保证
//! 内边距与标题层级一致；改这里 = 所有组件一起变。

use eframe::egui;

/// 组件小节标题（`TEXT_TITLE` 加粗）。
pub(in crate::app) fn heading(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(crate::theme::TEXT_TITLE)
            .strong(),
    );
}

/// 组件卡片：统一内边距 + 外框描边，撑满可用宽度。
///
/// 各模块的内容都画在这里面，视觉上是一张张「卡」。
pub(in crate::app) fn card<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let visuals = ui.visuals().clone();
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
        // 外框描边与圆角跟主题的非交互控件走（换主题一起变）。
        .stroke(egui::Stroke::new(
            1.0_f32,
            visuals.widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(visuals.widgets.noninteractive.corner_radius)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
}

/// 表体的最大可视高度：5 行 × 25px（行高 21 + 行距 4）。
///
/// 行数再多也在组件内部滚动，不把整页撑长；不足 5 行时按实际行数收缩
/// （`auto_shrink`），不留空行。
pub(in crate::app) const BODY_MAX_HEIGHT: f32 = 125.0;

/// 可滚动表体：内容超过 `max_height` 时组件内部出现竖向滚动条。
///
/// `id` 用于 egui 的滚动状态记忆（每张表一个，切排序 / 筛选不丢位置）。
pub(in crate::app) fn scroll_body<R>(
    ui: &mut egui::Ui,
    id: &str,
    max_height: f32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    // max_height 无穷大 = 不限高：直接画，不用内部滚动（会话表要铺满全部
    // 行，卡片随内容长高，页面滚动兑底 —— 用户反馈：不要只显示几行）。
    if max_height.is_infinite() {
        return body(ui);
    }
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, true])
        .max_height(max_height)
        .show(ui, body)
        .inner
}
