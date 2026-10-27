use super::tokens::{over, DISABLED_ALPHA};
use eframe::egui::{Color32, Stroke, Visuals};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Palette {
    pub dark: bool,
    pub panel: Color32,
    pub faint: Color32,
    pub extreme: Color32,
    pub widget: Color32,
    pub hover: Color32,
    pub accent: Color32,
    pub text: Color32,
    /// Widget borders: separating widgets from the panel's fill.
    pub border: Color32,
    /// Text color on accent fills (selected states / primary buttons).
    pub accent_text: Color32,
}

impl Palette {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        dark: bool,
        panel: u32,
        faint: u32,
        extreme: u32,
        widget: u32,
        hover: u32,
        accent: u32,
        text: u32,
        border: u32,
        accent_text: u32,
    ) -> Self {
        Self {
            dark,
            panel: rgb(panel),
            faint: rgb(faint),
            extreme: rgb(extreme),
            widget: rgb(widget),
            hover: rgb(hover),
            accent: rgb(accent),
            text: rgb(text),
            border: rgb(border),
            accent_text: rgb(accent_text),
        }
    }

    /// Channel-wise interpolation toward `other` (0 = `self`, 1 = `other`), driving the theme cross-fade.
    ///
    /// `dark` flips at the midpoint rather than blending: it selects egui's own dark / light base
    /// `Visuals`, which is a discrete choice, so the switch lands where the blend is already half way.
    pub fn lerp(&self, other: &Palette, t: f32) -> Palette {
        let mix = |a: Color32, b: Color32| crate::motion::lerp_color(a, b, t);
        Palette {
            dark: if t < 0.5 { self.dark } else { other.dark },
            panel: mix(self.panel, other.panel),
            faint: mix(self.faint, other.faint),
            extreme: mix(self.extreme, other.extreme),
            widget: mix(self.widget, other.widget),
            hover: mix(self.hover, other.hover),
            accent: mix(self.accent, other.accent),
            text: mix(self.text, other.text),
            border: mix(self.border, other.border),
            accent_text: mix(self.accent_text, other.accent_text),
        }
    }

    /// The color of hints / faint small text (input placeholders, `.weak()` small text).
    ///
    /// Computed through [`egui::Ui::disable`]'s dimming formula all the way to opaque: first dim `widget` over
    /// the panel fill, then dim `text` over that.
    pub(super) fn hint_color(&self) -> Color32 {
        let faded = |color: Color32| color.gamma_multiply(DISABLED_ALPHA);
        over(over(self.panel, faded(self.widget)), faded(self.text))
    }

    pub(super) fn into_visuals(self, glass: bool) -> Visuals {
        let hint = self.hint_color();
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        // The acrylic tier: fills scale wholesale (`gamma_multiply` multiplies alpha too).
        let surface = |c: Color32| {
            if glass {
                c.gamma_multiply(super::tokens::GLASS_SURFACE_ALPHA)
            } else {
                c
            }
        };
        let panel = surface(self.panel);
        // Disabled widgets dim by this factor. egui's own default happens to match, but pinning it here keeps
        // every theme on one deliberate value and survives an upstream default change.
        v.disabled_alpha = DISABLED_ALPHA;
        v.panel_fill = if glass {
            self.panel.gamma_multiply(super::tokens::GLASS_PANEL_ALPHA)
        } else {
            self.panel
        };
        let _ = panel;
        v.window_fill = v.panel_fill;
        v.faint_bg_color = surface(self.faint);
        v.extreme_bg_color = surface(self.extreme);
        // Body color goes into each state's `fg_stroke`, not `override_text_color`: the latter would pin plain
        // text `WidgetText` colors too, starving `TextEdit::hint_text` and `.weak()` small text of their faint
        // color. `Visuals::text_color` reads `fg_stroke` as well.
        for widget in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
        ] {
            widget.fg_stroke.color = self.text;
        }
        v.weak_text_color = Some(hint);
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent;
        v.widgets.inactive.weak_bg_fill = surface(self.widget);
        v.widgets.inactive.bg_fill = surface(self.widget);
        v.widgets.hovered.weak_bg_fill = surface(self.hover);
        v.widgets.hovered.bg_fill = surface(self.hover);
        v.widgets.active.weak_bg_fill = surface(self.accent);
        v.widgets.active.bg_fill = surface(self.accent);
        // Widget borders: widget fills sit only 1.2–1.5:1 above the panel fill; inputs use the `extreme` fill,
        // also shaped by this line.
        let edge = Stroke::new(1.0f32, self.border);
        v.widgets.noninteractive.bg_stroke = edge;
        v.widgets.inactive.bg_stroke = edge;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0f32, self.accent);
        v.widgets.active.bg_stroke = Stroke::new(1.0f32, self.accent);
        // Text on accent fills: selected rows / primary buttons (white on light themes, black on dark).
        v.selection.stroke = Stroke::new(1.0f32, self.accent_text);
        v
    }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[cfg(test)]
mod tests {
    use super::Palette;
    use crate::theme::{Theme, UiStyle};
    use eframe::egui;

    /// `lerp` returns the endpoints exactly and a channel-wise midpoint in between.
    #[test]
    fn lerp_reaches_both_endpoints_and_blends_between() {
        let dark: Palette = Theme::Dark.palette();
        let light = Theme::Light.palette();
        assert_eq!(dark.lerp(&light, 0.0), dark, "t=0 必须是起点调色板");
        assert_eq!(dark.lerp(&light, 1.0), light, "t=1 必须是终点调色板");
        // Past the ends the result clamps instead of overshooting.
        assert_eq!(dark.lerp(&light, -1.0), dark);
        assert_eq!(dark.lerp(&light, 2.0), light);
        let mid = dark.lerp(&light, 0.5);
        for (a, b, m) in [
            (dark.panel, light.panel, mid.panel),
            (dark.accent, light.accent, mid.accent),
            (dark.text, light.text, mid.text),
        ] {
            for (x, y, z) in [
                (a.r(), b.r(), m.r()),
                (a.g(), b.g(), m.g()),
                (a.b(), b.b(), m.b()),
            ] {
                let want = (f32::from(x) + (f32::from(y) - f32::from(x)) * 0.5).round() as u8;
                assert_eq!(z, want, "通道中点插值不对：{x} → {y} 得到 {z}");
            }
        }
    }

    /// `dark` is a discrete switch, so it flips at the midpoint rather than blending.
    #[test]
    fn lerp_flips_the_dark_flag_at_the_midpoint() {
        let dark: Palette = Theme::Dark.palette();
        let light: Palette = Theme::Light.palette();
        assert!(dark.lerp(&light, 0.49).dark, "中点之前仍是深色底");
        assert!(!dark.lerp(&light, 0.5).dark, "中点之后已翻到亮色底");
    }

    /// A half-way palette genuinely reaches the style (the cross-fade is not a no-op).
    #[test]
    fn blended_palette_reaches_the_style() {
        let ctx = egui::Context::default();
        let dark: Palette = Theme::Dark.palette();
        let light: Palette = Theme::Light.palette();
        let mid = dark.lerp(&light, 0.5);
        Theme::Dark.apply_style_with_palette(&ctx, UiStyle::Soft, false, mid);
        assert_eq!(
            ctx.style().visuals.panel_fill,
            mid.panel,
            "混合调色板没进 Style"
        );
        assert_ne!(mid.panel, dark.panel, "混合后不应仍等于起点");
        assert_ne!(mid.panel, light.panel, "混合后不应仍等于终点");
    }
}
