use eframe::egui::Color32;

/// 淡化禁用控件时用的 alpha（与 egui 的 `Visuals::disabled_alpha` 默认值一致）。
pub(crate) const DISABLED_ALPHA: f32 = 0.5;

/// 亚克力档：面板底色保留的不透明度（其余透出 DWM 亚克力模糊）。
///
/// 卡片 / 输入框比它实，见 [`GLASS_SURFACE_ALPHA`]；两者保持差距，且卡片始终比面板**实**。
pub const GLASS_PANEL_ALPHA: f32 = 0.50;

/// 亚克力档：卡片 / 输入框 / 控件底色的不透明度，高于 [`GLASS_PANEL_ALPHA`]。
pub const GLASS_SURFACE_ALPHA: f32 = 0.55;

// ── 间距刻度（4 的倍数）────────────────────────────────────────────
/// 行内元素之间。
pub const SPACE_1: f32 = 4.0;
/// 同一分组内。
pub const SPACE_2: f32 = 8.0;
/// 控件与标签之间。
pub const SPACE_3: f32 = 12.0;
/// 卡片内边距。
pub const SPACE_4: f32 = 16.0;
/// 卡片之间。
pub const SPACE_5: f32 = 24.0;
/// 区块之间。
pub const SPACE_6: f32 = 32.0;
/// 页面级留白。
pub const SPACE_7: f32 = 48.0;

// ── 字号刻度 ──────────────────────────────────────────────────
/// 徽标 / 极短标记，11px（egui 默认 `Small` 为 9px）。
pub const TEXT_CAPTION: f32 = 11.0;
/// 副信息：来源、请求次数、站点名。
pub const TEXT_SMALL: f32 = 12.0;
/// 卡片标题（provider key）。
pub const TEXT_TITLE: f32 = 16.0;
/// 正文 / 控件文字，13px（egui 默认值）。
pub const TEXT_BODY: f32 = 13.0;
/// 区块标题。
pub const TEXT_HEADING: f32 = 18.0;

/// 字阶严格递增（caption < small < body < title < heading）的编译期断言。
const _: () = {
    assert!(TEXT_CAPTION < TEXT_SMALL, "字阶错位：caption 不小于 small");
    assert!(TEXT_SMALL < TEXT_BODY, "字阶错位：small 不小于 body");
    assert!(TEXT_BODY < TEXT_TITLE, "字阶错位：body 不小于 title");
    assert!(TEXT_TITLE < TEXT_HEADING, "字阶错位：title 不小于 heading");
};

// ── 圆角 ──────────────────────────────────────────────────
/// 圆角滑块的上限（「外观」面板里连续调的范围是 0..=这个值）。
pub const RADIUS_SLIDER_MAX: u8 = 20;
/// 徽标 / 小按钮。
pub const RADIUS_SM: u8 = 4;
/// 卡片与控件。
pub const RADIUS_MD: u8 = 10;
/// 悬浮窗。
pub const RADIUS_LG: u8 = 14;

/// 可点击目标的最小高度（触控与高 DPI 下的下限）。
pub const TAP_TARGET_MIN: f32 = 24.0;

/// 正文 / 语义色对底色要求的最低对比度（WCAG AA）。
pub const CONTRAST_TEXT_MIN: f32 = 4.5;

/// 提示 / 占位文字的最低对比度，比正文低一档。
pub const CONTRAST_HINT_MIN: f32 = 3.0;

/// 非文字 UI 组件（控件描边、边框）的最低对比度。
pub const CONTRAST_UI_MIN: f32 = 3.0;

/// 把一个**预乘 alpha** 的颜色合成到不透明底色上：`结果 = 源 + 底 × (1 − α)`。
pub(crate) fn over(dst: Color32, src: Color32) -> Color32 {
    let keep = 1.0 - f32::from(src.a()) / 255.0;
    let mix = |s: u8, d: u8| {
        (f32::from(s) + f32::from(d) * keep)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(
        mix(src.r(), dst.r()),
        mix(src.g(), dst.g()),
        mix(src.b(), dst.b()),
    )
}
