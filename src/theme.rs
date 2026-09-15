use eframe::egui::{self, Color32, Stroke, Visuals};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Ocean,
    Nord,
    Rose,
}

impl Theme {
    pub const ALL: [Theme; 5] = [
        Theme::Dark,
        Theme::Light,
        Theme::Ocean,
        Theme::Nord,
        Theme::Rose,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Theme::Dark => "深色",
            Theme::Light => "浅色",
            Theme::Ocean => "海洋",
            Theme::Nord => "极地",
            Theme::Rose => "玫瑰",
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
        }
    }

    /// 由持久化标识还原；未知 / 空值回落默认主题。
    pub fn from_key(key: &str) -> Theme {
        Theme::ALL
            .into_iter()
            .find(|theme| theme.key() == key)
            .unwrap_or_default()
    }

    fn palette(&self) -> Palette {
        match self {
            // dark, panel, faint, extreme, widget, hover, accent, text
            Theme::Dark => Palette::new(
                true, 0x1E1E1E, 0x242424, 0x101010, 0x2D2D2D, 0x383838, 0x3B82F6, 0xE4E4E4,
            ),
            Theme::Light => Palette::new(
                false, 0xF5F5F5, 0xECECEC, 0xFFFFFF, 0xE2E2E2, 0xD5D5D5, 0x2563EB, 0x1E1E1E,
            ),
            Theme::Ocean => Palette::new(
                true, 0x0D1B2A, 0x1B263B, 0x0A1622, 0x22384F, 0x2C4A66, 0x48CAE4, 0xE0E8F0,
            ),
            Theme::Nord => Palette::new(
                true, 0x2E3440, 0x3B4252, 0x272C36, 0x434C5E, 0x4C566A, 0x88C0D0, 0xD8DEE9,
            ),
            Theme::Rose => Palette::new(
                false, 0xFBEEF0, 0xF5DCE1, 0xFFFFFF, 0xF0CDD4, 0xE8B9C2, 0xB5838D, 0x4A2E33,
            ),
        }
    }

    /// Builds a full Style (theme visuals + shared spacing/rounding) and applies it.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut style = egui::Style::default();
        style.spacing.item_spacing = egui::vec2(5.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 3.0);
        style.spacing.interact_size.y = 18.0;
        style.spacing.scroll.bar_outer_margin = 0.0;
        style.spacing.scroll.floating = true;
        style.visuals = self.palette().into_visuals();
        let r = 10u8;
        for w in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            w.corner_radius = r.into();
        }
        ctx.set_style(style);
    }
}

struct Palette {
    dark: bool,
    panel: Color32,
    faint: Color32,
    extreme: Color32,
    widget: Color32,
    hover: Color32,
    accent: Color32,
    text: Color32,
}

impl Palette {
    #[allow(clippy::too_many_arguments)]
    fn new(
        dark: bool,
        panel: u32,
        faint: u32,
        extreme: u32,
        widget: u32,
        hover: u32,
        accent: u32,
        text: u32,
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
        }
    }

    fn into_visuals(self) -> Visuals {
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.faint_bg_color = self.faint;
        v.extreme_bg_color = self.extreme;
        v.override_text_color = Some(self.text);
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent.gamma_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.widgets.inactive.weak_bg_fill = self.widget;
        v.widgets.inactive.bg_fill = self.widget;
        v.widgets.hovered.weak_bg_fill = self.hover;
        v.widgets.hovered.bg_fill = self.hover;
        v.widgets.active.weak_bg_fill = self.accent;
        v.widgets.active.bg_fill = self.accent;
        v
    }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

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
