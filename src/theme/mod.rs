//! 设计系统：主题（调色板）、形状预设（UiStyle）、语义色与设计刻度。
//!
//! 按关注点拆分：`tokens` 刻度常量与混色、`style` 形状预设、
//! `palette` 调色板、`semantics` 语义色；主题本体（`Theme`）留在本文件。
//! 公共 API 一律从这里再导出，外部 `crate::theme::X` 路径不变。

mod palette;
mod semantics;
mod style;
mod tokens;

pub use palette::Palette;
#[cfg(test)]
pub(crate) use semantics::{contrast_for_tests, distance_for_tests};
pub use semantics::{semantics, Semantics, SEMANTICS};
pub use style::{active_style, needs_apply, needs_apply_style, UiStyle};
pub use tokens::{
    CONTRAST_HINT_MIN, CONTRAST_TEXT_MIN, CONTRAST_UI_MIN, RADIUS_LG, RADIUS_MD, RADIUS_SLIDER_MAX,
    RADIUS_SM, SPACE_1, SPACE_2, SPACE_3, SPACE_4, SPACE_5, SPACE_6, SPACE_7, TAP_TARGET_MIN,
    TEXT_BODY, TEXT_CAPTION, TEXT_HEADING, TEXT_SMALL, TEXT_TITLE,
};

use eframe::egui::{self, Stroke};
use style::ACTIVE_STYLE_ID;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Ocean,
    Nord,
    Rose,
    /// 苔藓：低饱和绿。
    Moss,
    /// 薄荷：清新绿。
    Mint,
    /// 薰衣草：柔和紫。
    Lavender,
}

impl Theme {
    pub const ALL: [Theme; 8] = [
        // 深色主题（4个）
        Theme::Dark,
        Theme::Ocean,
        Theme::Nord,
        Theme::Moss,
        // 浅色主题（4个）
        Theme::Light,
        Theme::Rose,
        Theme::Mint,
        Theme::Lavender,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Theme::Dark => "深色",
            Theme::Light => "亮色",
            Theme::Ocean => "海洋",
            Theme::Nord => "极地",
            Theme::Rose => "玫瑰",
            Theme::Moss => "苔藓",
            Theme::Mint => "薄荷",
            Theme::Lavender => "薰衣草",
        }
    }

    /// 持久化用的稳定标识（与界面文案解耦，文案改了不影响旧配置）。
    pub fn key(&self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
            Theme::Ocean => "ocean",
            Theme::Nord => "nord",
            Theme::Rose => "rose",
            Theme::Moss => "moss",
            Theme::Mint => "mint",
            Theme::Lavender => "lavender",
        }
    }

    /// 当前主题的强调色（用于主题选择器色块）。
    pub fn accent_color(&self) -> egui::Color32 {
        self.palette().accent
    }

    /// 由持久化标识还原；未知 / 空值回落默认主题。
    pub fn from_key(key: &str) -> Theme {
        Theme::ALL
            .into_iter()
            .find(|theme| theme.key() == key)
            .unwrap_or_default()
    }

    /// 该主题是否深色底。预览的语法配色按它选深浅两套。
    pub fn is_dark(&self) -> bool {
        self.palette().dark
    }

    pub fn palette(&self) -> Palette {
        match self {
            // dark, panel, faint, extreme, widget, hover, accent, text
            Theme::Dark => Palette::new(
                true, 0x1E1E1E, 0x242424, 0x101010, 0x2D2D2D, 0x383838, 0x4A8CF7, 0xE4E4E4,
                0x6F6F6F, 0x000000,
            ),
            Theme::Light => Palette::new(
                false, 0xF5F5F5, 0xECECEC, 0xFFFFFF, 0xE2E2E2, 0xD5D5D5, 0x1D4ED8, 0x1E1E1E,
                0x868686, 0xFFFFFF,
            ),
            Theme::Ocean => Palette::new(
                true, 0x0D1B2A, 0x1B263B, 0x0A1622, 0x22384F, 0x2C4A66, 0x48CAE4, 0xE0E8F0,
                0x67727E, 0x000000,
            ),
            Theme::Nord => Palette::new(
                true, 0x2E3440, 0x323846, 0x272C36, 0x434C5E, 0x4C566A, 0x88C0D0, 0xD8DEE9,
                0x7D838F, 0x000000,
            ),
            Theme::Rose => Palette::new(
                false, 0xFBEEF0, 0xF8E5E8, 0xFFFFFF, 0xEAC0C9, 0xE8B9C2, 0x8C5A66, 0x4A2E33,
                0x947F82, 0xFFFFFF,
            ),
            // 以下四组取自暖 / 冷两端的低饱和家族：面板底色偏暗、强调色压低，
            // 文字与描边按同一套阈值（正文 / 强调 4.5:1、描边 3:1）逐档校过。
            Theme::Moss => Palette::new(
                true, 0x18201A, 0x1E2620, 0x101812, 0x232B24, 0x2C342C, 0x6D9D82, 0xD8DED2,
                0x767676, 0x000000,
            ),
            Theme::Mint => Palette::new(
                // 原配色三处不达标：强调色 #2D8659 只有 4.22:1、提示色 2.70:1、
                // 正文 #1A4D33 在四个浅色主题里最淡（9.15:1，亮色 15.3 / 玫瑰 10.8 /
                // 薰衣草 11.2），提示色是正文淡化出来的，正文一淡它就跟着读不清。
                // 现在按 WCAG 下限校过：强调 #257A4E、正文 #0F3624、描边 #6E8F7C。
                false, 0xF0FAF5, 0xE6F7ED, 0xFFFFFF, 0xD0F0DE, 0xBFEBD3, 0x257A4E, 0x0F3624,
                0x6E8F7C, 0xFFFFFF,
            ),
            Theme::Lavender => Palette::new(
                false, 0xF5F2FA, 0xECE7F5, 0xFFFFFF, 0xD9CEEB, 0xCFC2E6, 0x6B4D9E, 0x3D2866,
                0x8E7FA3, 0xFFFFFF,
            ),
        }
    }

    fn egui_theme(self) -> egui::Theme {
        egui::Theme::from_dark_mode(self.palette().dark)
    }

    /// 按当前主题 + 默认形状套用样式。
    pub fn apply(&self, ctx: &egui::Context) {
        let style = UiStyle::default();
        self.apply_style(ctx, style);
    }
    /// 按主题 + 形状预设套用样式（顶部栏的「外观」面板用它）。
    pub fn apply_style(&self, ctx: &egui::Context, shape: UiStyle) {
        let mut style = egui::Style::default();
        style.spacing.item_spacing = egui::vec2(SPACE_2 / 2.0, SPACE_2);
        style.spacing.button_padding = egui::vec2(SPACE_4 - 2.0, SPACE_1 - 1.0);
        style.spacing.interact_size.y = TAP_TARGET_MIN;
        style.spacing.scroll.bar_outer_margin = 0.0;
        style.spacing.scroll.floating = true;
        // 字号刻度：egui 默认 Small 只有 9px，中文读不清；统一抬到刻度上。
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(TEXT_CAPTION, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(TEXT_BODY, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(TEXT_BODY, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(TEXT_HEADING, egui::FontFamily::Proportional),
        );
        let palette = self.palette();
        let dark = palette.dark;
        style.visuals = palette.into_visuals();
        let radius = shape.radius();
        for w in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            w.corner_radius = radius.into();
        }
        // 描边宽度随形状走；颜色沿用调色板（hovered / active 是强调色）。
        for w in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
        ] {
            w.bg_stroke.width = shape.widget_stroke_width();
        }
        if shape.has_contact_shadow() {
            // 浮雕：控件描边换成内嵌暗边；悬浮窗给偏右下的投影，与卡片接触影呼应。
            let (_, edge) = shape.bevel_colors(dark);
            style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0f32, edge);
            style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0f32, edge);
            style.visuals.window_shadow = egui::epaint::Shadow {
                offset: [2, 2],
                blur: 6,
                spread: 0,
                color: edge,
            };
            style.visuals.popup_shadow = style.visuals.window_shadow;
        }
        if shape.has_card_shadow() {
            let shadow = shape.card_shadow(dark);
            style.visuals.window_shadow = shadow;
            style.visuals.popup_shadow = shadow;
        }
        ctx.set_theme(self.egui_theme());
        ctx.set_style(style);
        // 形状存进上下文：`ui.rs` 里的卡片要读它决定描边宽度与内嵌亮线，
        // 而 `Frame` 只能拿到 `Ui`，拿不到 `App` 的字段。
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(ACTIVE_STYLE_ID), shape));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形状预设的圆角必须落在滑块能取到的范围里，否则用户切到预设后
    /// 再拖滑块会出现「值变了但样式没变」的错觉。
    #[test]
    fn every_shape_radius_is_reachable_by_the_slider() {
        for style in UiStyle::ALL {
            assert!(
                style.radius() <= RADIUS_SLIDER_MAX,
                "{} 的圆角 {} 超过滑块上限 {RADIUS_SLIDER_MAX}",
                style.key(),
                style.radius()
            );
        }
    }

    /// 预设要真的做出区别：圆角 / 描边 / 浮雕 / 投影不能撞车。
    #[test]
    fn shape_presets_differ_in_radius_or_border() {
        let seen: Vec<(u8, u32, bool, bool)> = UiStyle::ALL
            .iter()
            .map(|s| {
                (
                    s.radius(),
                    // 云朵卡片不描边，控件仍 1px： uniqueness 按卡片观感算 0。
                    if s.has_card_shadow() {
                        0
                    } else {
                        (s.border_width() * 10.0) as u32
                    },
                    s.has_bevel(),
                    s.has_card_shadow(),
                )
            })
            .collect();
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            seen.len(),
            "形状预设的（圆角, 描边, 浮雕, 投影）有重复：{seen:?}"
        );
    }

    #[test]
    fn heavy_is_gone_and_unknown_keys_fall_back() {
        for removed in ["heavy", "sharp", "frosted", "compact", "neon", "prism"] {
            assert!(!UiStyle::ALL.iter().any(|s| s.key() == removed));
            assert_eq!(UiStyle::from_key(removed), UiStyle::default());
        }
        assert_eq!(UiStyle::from_key("cloud").key(), "cloud");
        assert_eq!(UiStyle::from_key("emboss").key(), "emboss");
        assert_eq!(UiStyle::from_key("slab").key(), "slab");
        assert_eq!(UiStyle::from_key("band").key(), "band");
        assert_eq!(UiStyle::from_key("minimal").key(), "minimal");
        assert_eq!(UiStyle::from_key("fine").key(), "fine");
    }

    /// 石板与浮雕必须是**机制**不同，不是同一套光影调强度：
    /// 浮雕有凸起受光线，石板只有描边 + 接触影（平放的板）。
    #[test]
    fn slab_and_emboss_use_different_mechanisms() {
        assert!(UiStyle::Emboss.has_bevel(), "浮雕必须有凸起受光线");
        assert!(!UiStyle::Slab.has_bevel(), "石板是平放的板，不该有受光线");
        assert!(UiStyle::Slab.has_contact_shadow());
        assert!(UiStyle::Emboss.has_contact_shadow());
        // 色带是另一套机制：顶部强调条。
        assert!(UiStyle::Band.has_accent_bar());
        assert!(!UiStyle::Band.has_bevel());
        assert!(!UiStyle::Band.has_contact_shadow());
        assert!(!UiStyle::Band.has_card_shadow());
    }

    #[test]
    fn light_bevel_is_strong_enough_on_light_backgrounds() {
        let (light_hi, light_lo) = UiStyle::Emboss.bevel_colors(false);
        let (dark_hi, dark_lo) = UiStyle::Emboss.bevel_colors(true);
        assert!(
            light_lo.a() > dark_hi.a(),
            "浅色主题的暗边必须比深色主题的亮边更实，否则浅底上看不见"
        );
        assert!(light_hi.a() >= dark_hi.a());
        assert!(light_lo.a() > 0 && dark_lo.a() > 0);
        // 浅色主题的暗边必须近实色：半透明线在浅底上会化掉，用户已两次反馈。
        assert!(
            light_lo.a() >= 190,
            "浅色浮雕暗边 alpha {} 不够实",
            light_lo.a()
        );
        // 接触影在浅底上也要够实。
        assert!(UiStyle::Slab.contact_shadow_color(false).a() >= 120);
    }

    /// 圆角 / 描边真的写进了 Style。
    #[test]
    fn shape_preset_reaches_the_style() {
        let ctx = egui::Context::default();
        for style in UiStyle::ALL {
            Theme::Dark.apply_style(&ctx, style);
            let widgets = ctx.style().visuals.widgets.noninteractive;
            assert_eq!(
                widgets.corner_radius.nw,
                style.radius(),
                "{} 的圆角没进 Style",
                style.key()
            );
            let expected_width = style.widget_stroke_width();
            assert_eq!(
                widgets.bg_stroke.width,
                expected_width,
                "{} 的描边宽度没进 Style",
                style.key()
            );
        }
    }

    /// 形状预设变化时 `needs_apply_style` 要重套样式。
    #[test]
    fn shape_change_reapplies_the_style() {
        let mut applied: Option<(Theme, UiStyle)> = None;
        assert!(needs_apply_style(&mut applied, Theme::Dark, UiStyle::Soft));
        assert!(!needs_apply_style(&mut applied, Theme::Dark, UiStyle::Soft));
        assert!(needs_apply_style(
            &mut applied,
            Theme::Dark,
            UiStyle::Minimal
        ));
        assert!(needs_apply_style(
            &mut applied,
            Theme::Light,
            UiStyle::Minimal
        ));
        assert!(!needs_apply_style(
            &mut applied,
            Theme::Light,
            UiStyle::Minimal
        ));
    }

    /// 形状要能从上下文读回来：卡片靠它决定描边与内嵌边。
    #[test]
    fn active_style_round_trips_through_the_context() {
        let ctx = egui::Context::default();
        // 没套过样式时是默认档。
        assert_eq!(active_style(&ctx), UiStyle::default());
        for style in UiStyle::ALL {
            Theme::Dark.apply_style(&ctx, style);
            assert_eq!(active_style(&ctx), style, "{} 没存进上下文", style.key());
        }
    }

    #[test]
    fn labels_unique_and_visuals_build() {
        let mut labels = std::collections::HashSet::new();
        for t in Theme::ALL {
            assert!(labels.insert(t.label()), "duplicate label: {}", t.label());
            let _ = t.palette().into_visuals(); // must not panic
        }
        assert_eq!(labels.len(), Theme::ALL.len());
    }
}
