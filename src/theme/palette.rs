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
    /// 控件描边：把控件从面板底色里分出来。
    pub border: Color32,
    /// 强调色底上的文字色（选中态 / 主按钮）。
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

    /// 提示 / 淡色小字（输入框占位提示、`.weak()` 小字）的颜色。
    ///
    /// 按 [`egui::Ui::disable`] 的淡化算式一路算到不透明：先把 `widget` 淡化叠到
    /// 面板底上，再把 `text` 淡化叠到那层底上。
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
        // 玻璃档：底色整体缩放（`gamma_multiply` 连 alpha 一起乘）。
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
        // 正文色写进各状态的 `fg_stroke`，不用 `override_text_color`：
        // 后者会连纯文本 `WidgetText` 的颜色一起钉死，`TextEdit::hint_text`
        // 与 `.weak()` 小字就拿不到淡色。`Visuals::text_color` 读的也是 `fg_stroke`。
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
        // 控件描边：控件底色与面板底色只差 1.2–1.5:1；输入框用 `extreme` 底，
        // 同样靠这条线成形。
        let edge = Stroke::new(1.0f32, self.border);
        v.widgets.noninteractive.bg_stroke = edge;
        v.widgets.inactive.bg_stroke = edge;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0f32, self.accent);
        v.widgets.active.bg_stroke = Stroke::new(1.0f32, self.accent);
        // 强调色底上的文字：选中态行 / 主按钮（浅底主题用白、深底主题用黑）。
        v.selection.stroke = Stroke::new(1.0f32, self.accent_text);
        v
    }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
