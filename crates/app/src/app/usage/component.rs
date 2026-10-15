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

/// 行距：行高 21 + 行间距 4（用户定的行距）。
pub(in crate::app) const ROW_PITCH: f32 = 25.0;

/// 分段控件组的内边距，同时也是组内按钮之间的间隙。
///
/// 按钮间距收到这个值（比全局 `item_spacing.x` 更紧）：它们同属一个控件，
/// 间隙大了就读成几个独立按钮。
pub(in crate::app) const CHIP_GROUP_PAD: f32 = crate::theme::SPACE_1;

/// `chips` 个等宽按钮包进分段控件组之后的总宽度。
///
/// 状态行要给右侧这组按钮预留固定宽度，剩下的才留给左侧文字。与
/// [`chip_group`] 共用 [`CHIP_GROUP_PAD`]，改按钮宽度不会把预留算错。
pub(in crate::app) fn chip_group_width(chips: usize, button_width: f32) -> f32 {
    let n = chips as f32;
    n * button_width + (n - 1.0).max(0.0) * CHIP_GROUP_PAD + 2.0 * CHIP_GROUP_PAD
}

/// 分段控件组：把一排互斥的切换按钮包成一个组件（用户要求：区间切换用
/// 组件包起来）。
///
/// 底色与卡片拉开层级：卡是「凸」的（描边 + 面板色），它是「凹」的
/// （极暗底 + 同一套描边），看上去像一组嵌在状态行里的开关。
///
/// 组内按钮横向排列，间隙收到 [`CHIP_GROUP_PAD`]：它们同属一个控件，
/// 全局 `item_spacing.x` 那个间隙会让它们读成几个独立按钮。`body` 只需
/// 逐个往 `ui` 里加按钮；组宽用 [`chip_group_width`] 预算。
pub(in crate::app) fn chip_group<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let visuals = ui.visuals().clone();
    egui::Frame::NONE
        .fill(visuals.extreme_bg_color)
        .stroke(egui::Stroke::new(
            1.0_f32,
            visuals.widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(visuals.widgets.noninteractive.corner_radius)
        .inner_margin(egui::Margin::same(CHIP_GROUP_PAD as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = CHIP_GROUP_PAD;
                body(ui)
            })
            .inner
        })
        .inner
}

/// 表体的最大可视高度：5 行 × 25px。
///
/// 行数再多也在组件内部滚动，不把整页撑长；不足 5 行时按实际行数收缩
/// （`auto_shrink`），不留空行。
pub(in crate::app) const BODY_MAX_HEIGHT: f32 = 5.0 * ROW_PITCH;

/// 会话表体的高度：普通表的三倍（用户要求）。
///
/// 会话表独占一行、列最多，是逐条核对的主要入口，一屏只给 5 行太少。
pub(in crate::app) const SESSION_BODY_HEIGHT: f32 = BODY_MAX_HEIGHT * 3.0;

/// 表体高度：行数 × 行距，封顶在 `cap`。
///
/// 高度锁死而不按内容伸缩，两个理由：
///
/// * 并排的两张表行数不同时，按较多一方的高度取预算，两卡底边才落在同
///   一水平线上（用户要求：Agent 卡高度与模型卡对齐）。
/// * 不给上限时，`ScrollArea` 会拿页面剩余高度当视口：窗口矮一点，会话表
///   就看不到三倍的行，用户要求的「可视高度是普通表的三倍」无从谈起。
///
/// 至少一行：高度为 0 会让视口塔成一条线，空表连表头都看不全。
pub(in crate::app) fn body_height(rows: usize, cap: f32) -> f32 {
    (rows.max(1) as f32 * ROW_PITCH).clamp(ROW_PITCH, cap.max(ROW_PITCH))
}

/// 可滚动表体：内容超过 `height` 时组件内部出现竖向滚动条。
///
/// 视口高度固定为 `height`（`min_scrolled_height` + 关掉纵向收缩）：行数少
/// 的一方底部留白，换取并排两卡底边齐平。
///
/// `id` 用于 egui 的滚动状态记忆（每张表一个，切排序 / 筛选不丢位置）。
pub(in crate::app) fn scroll_body<R>(
    ui: &mut egui::Ui,
    id: &str,
    height: f32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .max_height(height)
        .min_scrolled_height(height)
        .show(ui, body)
        .inner
}
