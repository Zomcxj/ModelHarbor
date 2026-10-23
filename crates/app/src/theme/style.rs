use super::tokens::{RADIUS_MD, RADIUS_SLIDER_MAX, RADIUS_SM};
use super::Theme;
use eframe::egui::{self, Color32};

/// The UI's shape presets: a combination of corner radius, border weight, inset bright/dark edges, shadows, and the ribbon.
///
/// Orthogonal to the theme: themes govern color, shapes govern widget appearance. The 8 tiers each carry
/// one mechanism (bordered / borderless / raised light / soft shadow / contact shadow / top ribbon).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum UiStyle {
    /// Rounded: the default tier, radius 10 with a 1px border.
    #[default]
    Soft,
    /// Refined: radius 6 with a 0.5px hairline.
    Fine,
    /// Minimal: all square corners, 0px borderless — pure color blocks.
    Minimal,
    /// Pill: capsule 20 with a 0.5px hairline.
    Tag,
    /// Cloud: large radius 16 with a soft shadow.
    Cloud,
    /// Emboss: raised light-catching edges + contact shadow.
    Emboss,
    /// Slab: a flat stone — dark border + contact shadow, no light-catching edges.
    Slab,
    /// Ribbon: a theme-accent strip across the card's top.
    Band,
}

impl UiStyle {
    pub const ALL: [UiStyle; 8] = [
        UiStyle::Tag,
        UiStyle::Soft,
        UiStyle::Fine,
        UiStyle::Minimal,
        UiStyle::Cloud,
        UiStyle::Emboss,
        UiStyle::Slab,
        UiStyle::Band,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            UiStyle::Soft => "圆润",
            UiStyle::Fine => "精致",
            UiStyle::Minimal => "极简",
            UiStyle::Tag => "标签",
            UiStyle::Cloud => "云朵",
            UiStyle::Emboss => "浮雕",
            UiStyle::Slab => "石板",
            UiStyle::Band => "色带",
        }
    }

    /// The stable identifier for persistence, decoupled from UI wording.
    pub fn key(&self) -> &'static str {
        match self {
            UiStyle::Soft => "soft",
            UiStyle::Fine => "fine",
            UiStyle::Minimal => "minimal",
            UiStyle::Tag => "tag",
            UiStyle::Cloud => "cloud",
            UiStyle::Emboss => "emboss",
            UiStyle::Slab => "slab",
            UiStyle::Band => "band",
        }
    }

    /// Restore from a persistence identifier; unknown / empty values fall back to the default tier.
    pub fn from_key(key: &str) -> UiStyle {
        UiStyle::ALL
            .into_iter()
            .find(|style| style.key() == key)
            .unwrap_or_default()
    }

    /// This preset's default corner radius.
    pub fn radius(&self) -> u8 {
        match self {
            UiStyle::Soft => RADIUS_MD,
            UiStyle::Fine => 6,
            UiStyle::Minimal => 0,
            UiStyle::Tag => RADIUS_SLIDER_MAX,
            UiStyle::Cloud => 16,
            UiStyle::Emboss => 8,
            UiStyle::Slab => RADIUS_SM,
            UiStyle::Band => 8,
        }
    }

    /// Widget border width (pixels).
    pub fn border_width(&self) -> f32 {
        match self {
            UiStyle::Soft | UiStyle::Slab | UiStyle::Emboss | UiStyle::Band => 1.0,
            UiStyle::Minimal => 0.0,
            UiStyle::Fine | UiStyle::Tag => 0.5,
            // The cloud's card itself has no border (shaped by its shadow); inputs / buttons still need 1px.
            UiStyle::Cloud => 1.0,
        }
    }

    /// Whether to paint raised light-catching edges (bright top-left inside a card, dark bottom-right).
    pub fn has_bevel(&self) -> bool {
        matches!(self, UiStyle::Emboss)
    }

    /// Whether to paint the contact shadow (a full dark border ring offset toward the bottom-right).
    pub fn has_contact_shadow(&self) -> bool {
        matches!(self, UiStyle::Slab | UiStyle::Emboss)
    }

    /// Whether cards get a soft drop shadow (cloud).
    pub fn has_card_shadow(&self) -> bool {
        matches!(self, UiStyle::Cloud)
    }

    /// Whether to paint a theme-accent ribbon across the card's top.
    pub fn has_accent_bar(&self) -> bool {
        matches!(self, UiStyle::Band)
    }

    /// The border width written into egui's `WidgetVisuals`.
    ///
    /// Emboss swaps widget borders for inset dark edges (1px); slab keeps the original width.
    pub fn widget_stroke_width(&self) -> f32 {
        if self.has_bevel() {
            1.0
        } else {
            self.border_width()
        }
    }

    /// The card's soft shadow: black on dark themes, a deeper gray on light ones.
    pub fn card_shadow(&self, dark: bool) -> egui::epaint::Shadow {
        let color = if dark {
            Color32::from_black_alpha(110)
        } else {
            Color32::from_black_alpha(50)
        };
        egui::epaint::Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color,
        }
    }

    /// The raised light-catching edges' colors (emboss only), returning `(bright edge, dark edge)`.
    ///
    /// Light themes' dark edges are near-solid (alpha 230); dark themes' bright edges carry the dimensionality.
    pub fn bevel_colors(&self, dark: bool) -> (Color32, Color32) {
        if dark {
            (
                Color32::from_white_alpha(55),
                Color32::from_black_alpha(160),
            )
        } else {
            (
                Color32::from_white_alpha(255),
                Color32::from_black_alpha(230),
            )
        }
    }

    /// The contact shadow color: for the full dark ring offset bottom-right, shared by slab / emboss.
    pub fn contact_shadow_color(&self, dark: bool) -> Color32 {
        if dark {
            Color32::from_black_alpha(120)
        } else {
            Color32::from_black_alpha(140)
        }
    }
}

/// The context key holding the current shape preset.
pub(super) const ACTIVE_STYLE_ID: &str = "modelharbor_active_ui_style";

/// The context key holding the current acrylic tier.
pub(super) const GLASS_ID: &str = "modelharbor_active_glass";

/// The currently effective shape preset (the default tier before styling was applied).
pub fn active_style(ctx: &egui::Context) -> UiStyle {
    ctx.data(|data| data.get_temp::<UiStyle>(egui::Id::new(ACTIVE_STYLE_ID)))
        .unwrap_or_default()
}

/// Whether the theme needs applying this pass (`applied` records the applied theme and is updated in place).
///
/// egui's `set_style` keeps one style per theme; when the theme changes, `apply` must be called again explicitly.
pub fn needs_apply(applied: &mut Option<Theme>, theme: Theme) -> bool {
    if *applied == Some(theme) {
        return false;
    }
    *applied = Some(theme);
    true
}

/// Theme, shape, or acrylic tier changes all re-apply styling.
///
/// `applied` records the applied (theme, shape, acrylic) and is updated in place.
pub fn needs_apply_style(
    applied: &mut Option<(Theme, UiStyle, bool)>,
    theme: Theme,
    shape: UiStyle,
    glass: bool,
) -> bool {
    if *applied == Some((theme, shape, glass)) {
        return false;
    }
    *applied = Some((theme, shape, glass));
    true
}
