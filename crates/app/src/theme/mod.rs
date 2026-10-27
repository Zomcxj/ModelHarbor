//! The design system: themes (palettes), shape presets (UiStyle), semantic colors, and design scales.
//!
//! Split by concern: `tokens` holds scale constants and color mixing, `style` the shape presets,
//! `palette` the palettes, `semantics` the semantic colors; the theme proper (`Theme`) lives here.

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
use style::{ACTIVE_STYLE_ID, GLASS_ID};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Ocean,
    Nord,
    Rose,
    /// Moss: low-saturation green.
    Moss,
    /// Mint: fresh green.
    Mint,
    /// Lavender: soft purple.
    Lavender,
}

impl Theme {
    pub const ALL: [Theme; 8] = [
        // Dark themes (4)
        Theme::Dark,
        Theme::Ocean,
        Theme::Nord,
        Theme::Moss,
        // Light themes (4)
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

    /// The stable identifier for persistence, decoupled from UI wording.
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

    /// The current theme's accent color.
    pub fn accent_color(&self) -> egui::Color32 {
        self.palette().accent
    }

    /// Restore from a persistence identifier; unknown / empty values fall back to the default theme.
    pub fn from_key(key: &str) -> Theme {
        Theme::ALL
            .into_iter()
            .find(|theme| theme.key() == key)
            .unwrap_or_default()
    }

    /// Whether this theme has a dark background.
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
            // The following four groups come from the low-saturation family.
            Theme::Moss => Palette::new(
                true, 0x18201A, 0x1E2620, 0x101812, 0x232B24, 0x2C342C, 0x6D9D82, 0xD8DED2,
                0x767676, 0x000000,
            ),
            Theme::Mint => Palette::new(
                false, 0xF0FAF5, 0xE6F7ED, 0xFFFFFF, 0xD0F0DE, 0xBFEBD3, 0x257A4E, 0x0F3624,
                0x6E8F7C, 0xFFFFFF,
            ),
            Theme::Lavender => Palette::new(
                false, 0xF5F2FA, 0xECE7F5, 0xFFFFFF, 0xD9CEEB, 0xCFC2E6, 0x6B4D9E, 0x3D2866,
                0x8E7FA3, 0xFFFFFF,
            ),
        }
    }

    /// Apply styling per the current theme + default shape.
    pub fn apply(&self, ctx: &egui::Context) {
        let style = UiStyle::default();
        self.apply_style(ctx, style, false);
    }
    /// Apply styling per theme + shape preset + acrylic tier.
    pub fn apply_style(&self, ctx: &egui::Context, shape: UiStyle, glass: bool) {
        self.apply_style_with_palette(ctx, shape, glass, self.palette());
    }

    /// [`Self::apply_style`] with an explicit palette.
    ///
    /// The theme cross-fade passes a blend of the previous and current palettes, so every frame of the
    /// transition re-applies the style instead of switching colors in one step.
    pub fn apply_style_with_palette(
        &self,
        ctx: &egui::Context,
        shape: UiStyle,
        glass: bool,
        palette: Palette,
    ) {
        let mut style = egui::Style::default();
        style.spacing.item_spacing = egui::vec2(SPACE_2 / 2.0, SPACE_2);
        style.spacing.button_padding = egui::vec2(SPACE_4 - 2.0, SPACE_1 - 1.0);
        style.spacing.interact_size.y = TAP_TARGET_MIN;
        style.spacing.scroll.bar_outer_margin = 0.0;
        style.spacing.scroll.floating = true;
        // A thin, quiet floating bar: no track while idle, a slim handle that strengthens on hover.
        let scroll = &mut style.spacing.scroll;
        scroll.bar_inner_margin = SPACE_1 / 2.0;
        scroll.bar_width = SPACE_1 + 2.0;
        scroll.floating_width = SPACE_1 + 2.0;
        scroll.dormant_background_opacity = 0.0;
        scroll.active_background_opacity = 0.18;
        scroll.interact_background_opacity = 0.3;
        scroll.dormant_handle_opacity = 0.3;
        scroll.active_handle_opacity = 0.55;
        scroll.interact_handle_opacity = 0.8;
        // Font-size scale.
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
        let dark = palette.dark;
        style.visuals = palette.into_visuals(glass);
        let radius = shape.radius();
        for w in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            w.corner_radius = radius.into();
        }
        // Tooltips follow the shape preset too: popup frames read these two fields.
        style.visuals.menu_corner_radius = radius.into();
        style.spacing.menu_margin = egui::Margin::symmetric(SPACE_2 as i8, SPACE_1 as i8 + 2);
        // Border widths follow the shape; colors come from the palette (hovered / active are the accent).
        for w in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
        ] {
            w.bg_stroke.width = shape.widget_stroke_width();
        }
        if shape.has_contact_shadow() {
            // Emboss: widget borders become inset dark edges; popovers get a lower-right drop shadow.
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
        ctx.set_theme(egui::Theme::from_dark_mode(palette.dark));
        ctx.set_style(style);
        // The shape goes into the context: cards in `ui.rs` read it for border width and inset bright lines.
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(ACTIVE_STYLE_ID), shape));
        // The acrylic tier goes into the context too: cards' / panels' Frames read it for fill opacity.
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(GLASS_ID), glass));
    }

    /// Whether the current tier is acrylic (`false` before any styling was applied).
    pub fn active_glass(ctx: &egui::Context) -> bool {
        ctx.data(|data| data.get_temp::<bool>(egui::Id::new(GLASS_ID)))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape presets' corner radii stay within the slider cap `RADIUS_SLIDER_MAX`.
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

    /// Presets don't repeat corner radius / border / emboss / shadow.
    #[test]
    fn shape_presets_differ_in_radius_or_border() {
        let seen: Vec<(u8, u32, bool, bool)> = UiStyle::ALL
            .iter()
            .map(|s| {
                (
                    s.radius(),
                    // The cloud card has no border while widgets keep 1px: uniqueness counts the card's look as 0.
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

    /// Slab and emboss differ in mechanism: emboss has raised light-catching edges; slab is only a border + contact shadow.
    #[test]
    fn slab_and_emboss_use_different_mechanisms() {
        assert!(UiStyle::Emboss.has_bevel(), "浮雕必须有凸起受光线");
        assert!(!UiStyle::Slab.has_bevel(), "石板是平放的板，不该有受光线");
        assert!(UiStyle::Slab.has_contact_shadow());
        assert!(UiStyle::Emboss.has_contact_shadow());
        // The ribbon: an accent strip at the top.
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
        // Light themes' dark edges must be near-solid.
        assert!(
            light_lo.a() >= 190,
            "浅色浮雕暗边 alpha {} 不够实",
            light_lo.a()
        );
        // Contact shadows must also be solid enough on pale backgrounds.
        assert!(UiStyle::Slab.contact_shadow_color(false).a() >= 120);
    }

    /// Corner radius / border genuinely land in the Style.
    #[test]
    fn shape_preset_reaches_the_style() {
        let ctx = egui::Context::default();
        for style in UiStyle::ALL {
            Theme::Dark.apply_style(&ctx, style, false);
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

    /// The scroll-bar and tooltip tuning genuinely land in the Style.
    #[test]
    fn scroll_and_menu_styling_reach_the_style() {
        let ctx = egui::Context::default();
        Theme::Dark.apply_style(&ctx, UiStyle::Fine, false);
        let style = ctx.style();
        let scroll = &style.spacing.scroll;
        assert!(scroll.floating, "floating 滚动条必须开启");
        assert_eq!(
            scroll.floating_width,
            SPACE_1 + 2.0,
            "滚动条把手宽度没进 Style"
        );
        assert_eq!(scroll.dormant_background_opacity, 0.0, "静止时轨道必须隐身");
        assert!(scroll.interact_handle_opacity > scroll.dormant_handle_opacity);
        // Tooltip geometry follows the shape preset.
        assert_eq!(
            style.visuals.menu_corner_radius.nw,
            UiStyle::Fine.radius(),
            "tooltip 圆角没跟随形状预设"
        );
        assert_eq!(
            style.spacing.menu_margin.left, SPACE_2 as i8,
            "tooltip 边距没进 Style"
        );
    }

    /// A shape-preset change re-applies styling via `needs_apply_style`.
    #[test]
    fn shape_change_reapplies_the_style() {
        let mut applied: Option<(Theme, UiStyle, bool)> = None;
        assert!(needs_apply_style(
            &mut applied,
            Theme::Dark,
            UiStyle::Soft,
            false
        ));
        assert!(!needs_apply_style(
            &mut applied,
            Theme::Dark,
            UiStyle::Soft,
            false
        ));
        assert!(needs_apply_style(
            &mut applied,
            Theme::Dark,
            UiStyle::Minimal,
            false
        ));
        assert!(needs_apply_style(
            &mut applied,
            Theme::Light,
            UiStyle::Minimal,
            false
        ));
        assert!(!needs_apply_style(
            &mut applied,
            Theme::Light,
            UiStyle::Minimal,
            false
        ));
        // The acrylic tier changing alone also re-applies (the fill's alpha lives in the style).
        assert!(needs_apply_style(
            &mut applied,
            Theme::Light,
            UiStyle::Minimal,
            true
        ));
        assert!(!needs_apply_style(
            &mut applied,
            Theme::Light,
            UiStyle::Minimal,
            true
        ));
    }

    /// The shape reads back from the context.
    #[test]
    fn active_style_round_trips_through_the_context() {
        let ctx = egui::Context::default();
        // Before any styling, it's the default tier.
        assert_eq!(active_style(&ctx), UiStyle::default());
        for style in UiStyle::ALL {
            Theme::Dark.apply_style(&ctx, style, false);
            assert_eq!(active_style(&ctx), style, "{} 没存进上下文", style.key());
        }
        // The acrylic tier reads back from the context too.
        assert!(!Theme::active_glass(&ctx));
        Theme::Dark.apply_style(&ctx, UiStyle::Soft, true);
        assert!(Theme::active_glass(&ctx));
    }

    #[test]
    fn labels_unique_and_visuals_build() {
        let mut labels = std::collections::HashSet::new();
        for t in Theme::ALL {
            assert!(labels.insert(t.label()), "duplicate label: {}", t.label());
            // Construction works without the acrylic tier as well.
            let _ = t.palette().into_visuals(false);
            // And with it (fill alpha scales; must not panic).
            let glass = t.palette().into_visuals(true);
            assert!(
                glass.panel_fill.a() < 255,
                "{} 亚克力档的面板底色没变透明",
                t.key()
            );
            // Hierarchy invariant: widget fills are more solid than the panel's.
            assert!(
                glass.widgets.inactive.bg_fill.a() > glass.panel_fill.a(),
                "{} 亚克力档的控件底比面板还透，层次反了",
                t.key()
            );
        }
        assert_eq!(labels.len(), Theme::ALL.len());
    }
}
