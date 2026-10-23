use super::tokens::{over, DISABLED_ALPHA};
use eframe::egui::{Color32, Stroke, Visuals};

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
