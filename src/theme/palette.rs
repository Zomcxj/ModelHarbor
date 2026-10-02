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
    /// 控件描边：把控件从面板底色里分出来（控件底色与面板只差 1.2–1.5:1）。
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
    /// 取的是「禁用控件文字」的观感：egui 的禁用不是换一个灰，
    /// 而是 [`egui::Ui::disable`] 把整棵子树按 `disabled_alpha` 淡化——
    /// 颜色的 RGB 与 alpha 都乘上它（`Color32` 预乘，这正是「50% 不透明」的写法），
    /// 然后按 alpha 混合到底色上。所以这里把同一个算式一路算到**不透明**：
    /// 提示文字是画在别处（输入框底色）的，只有预先合成成同一个 RGB，
    /// 看上去才会真的是一个颜色。
    ///
    /// 顺序照抄渲染：按钮底（`widget`）先淡化叠到面板底，文字再淡化叠到那层底上。
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
        // 预乘色等比例缩放 = 「同一个颜色、更透」，直接改 alpha 会把颜色洗掉。
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
        // 正文色写进各状态的 `fg_stroke`，**不用** `override_text_color`：
        // 后者会连纯文本 `WidgetText` 的颜色一起钉死（`WidgetText::into_galley`
        // 对纯文本是 `override_text_color.unwrap_or(PLACEHOLDER)`），于是
        // `TextEdit::hint_text` 拿不到 `weak_text_color`——它的淡色是作为
        // 「galley 没给色时的兜底色」传进 `Painter::galley` 的，被 override 一挡，
        // 提示就画成了正文色，看上去像已经填好的值。
        // 写 `fg_stroke` 效果完全一样（`Visuals::text_color` 读的就是它），
        // 但不再拦住兜底色，占位提示与 `.weak()` 小字才能各自拿到淡色。
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
        // 控件描边：控件底色与面板底色只差 1.2–1.5:1，不给描边就靠这点色差分边界。
        // 输入框用 `extreme` 底，同样靠这条线成形。
        let edge = Stroke::new(1.0f32, self.border);
        v.widgets.noninteractive.bg_stroke = edge;
        v.widgets.inactive.bg_stroke = edge;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0f32, self.accent);
        v.widgets.active.bg_stroke = Stroke::new(1.0f32, self.accent);
        // 强调色底上的文字：选中态行 / 主按钮要看得清（浅底主题用白、深底主题用黑）。
        v.selection.stroke = Stroke::new(1.0f32, self.accent_text);
        v
    }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
