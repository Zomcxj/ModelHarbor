use eframe::egui::{self, Color32, Stroke, Visuals};

/// 淡化禁用控件时用的 alpha（即 `Visuals::disabled_alpha` 的默认值）。
///
/// 这里写常量而不是去问 egui：主题构造时手上只有调色板。单测会拿
/// `Visuals::disabled_alpha()` 核对，egui 改了默认值就会红。
const DISABLED_ALPHA: f32 = 0.5;

/// 把一个**预乘 alpha** 的颜色合成到不透明底色上。
///
/// egui 的 `Color32` 是预乘的，wgpu 侧的混合因子也是 `One / OneMinusSrcAlpha`，
/// 所以公式就是 `结果 = 源 + 底 × (1 − α)`。
fn over(dst: Color32, src: Color32) -> Color32 {
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
                true, 0x2E3440, 0x323846, 0x272C36, 0x434C5E, 0x4C566A, 0x88C0D0, 0xD8DEE9,
            ),
            Theme::Rose => Palette::new(
                false, 0xFBEEF0, 0xF8E5E8, 0xFFFFFF, 0xF0CDD4, 0xE8B9C2, 0xB5838D, 0x4A2E33,
            ),
        }
    }

    fn egui_theme(self) -> egui::Theme {
        egui::Theme::from_dark_mode(self.palette().dark)
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
        ctx.set_theme(self.egui_theme());
        ctx.set_style(style);
    }
}

/// 语义色（绿 = 正常 / 黄 = 注意 / 红 = 异常 / 蓝 = 信息）。
///
/// **跨主题统一**：五个主题共用同一套（黑白灰随主题变，彩色不变），
/// 取中间调色，让同一支颜色在浅底和深底上都看得清（单测锁定对比度 ≥3:1）。
/// 唯一的例外是「信息蓝」和主题强调色撞色时改用青蓝，见 [`semantics`]。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Semantics {
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub info: Color32,
}

/// 全主题共用的一套语义色。
pub const SEMANTICS: Semantics = Semantics {
    ok: Color32::from_rgb(0x19, 0x94, 0x4D),
    warn: Color32::from_rgb(0xA9, 0x7A, 0x00),
    err: Color32::from_rgb(0xD9, 0x59, 0x50),
    info: Color32::from_rgb(0x4F, 0x84, 0xCD),
};

/// 「信息蓝」与强调色撞色时的替代色（青蓝）。
const SEMANTICS_INFO_ALT: Color32 = Color32::from_rgb(0x15, 0x90, 0x9A);

/// 和强调色多近算「撞色」（RGB 空间欧氏距离）。
const ACCENT_CLASH: f32 = 45.0;

/// 取当前界面该用的语义色。
///
/// 正常就是 [`SEMANTICS`]（所有主题一致）；只有当前主题的强调色和信息蓝太接近时，
/// 信息蓝换成青蓝 —— 否则用量文字会和按钮 / 链接糊成一片。
pub fn semantics(ui: &egui::Ui) -> Semantics {
    semantics_with_accent(ui.visuals().hyperlink_color)
}

/// 按强调色取语义色（拆出来便于单测）。
fn semantics_with_accent(accent: Color32) -> Semantics {
    let mut colors = SEMANTICS;
    if distance(accent, colors.info) < ACCENT_CLASH {
        colors.info = SEMANTICS_INFO_ALT;
    }
    colors
}

/// RGB 欧氏距离。
fn distance(a: Color32, b: Color32) -> f32 {
    let [ar, ag, ab, _] = a.to_array();
    let [br, bg, bb, _] = b.to_array();
    let d = |x: u8, y: u8| {
        let diff = f32::from(x) - f32::from(y);
        diff * diff
    };
    (d(ar, br) + d(ag, bg) + d(ab, bb)).sqrt()
}

/// 对比度（WCAG，1.0 ~ 21.0）。只在单测里用来锁定「五个主题都看得清」。
#[cfg(test)]
fn contrast(a: Color32, b: Color32) -> f32 {
    let channel = |value: u8| {
        let v = f32::from(value) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let rel = |color: Color32| {
        let [r, g, b, _] = color.to_array();
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    };
    let (la, lb) = (rel(a), rel(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// 主题是否需要在本次应用（`applied` 记录已应用的主题，会被就地更新）。
///
/// 单独抽出来是为了能在单测里锁定「启动时必须应用一次」：
/// egui 0.33 的 `set_style` 是**每个主题各存一份 style**（dark / light 两份，
/// 按当前激活的主题取用），所以主题变了必须显式再调一次 `apply`，
/// 否则界面颜色不会跟着变。
pub fn needs_apply(applied: &mut Option<Theme>, theme: Theme) -> bool {
    if *applied == Some(theme) {
        return false;
    }
    *applied = Some(theme);
    true
}

#[cfg(test)]
mod semantics_tests {
    use super::*;

    #[test]
    fn every_semantic_color_reads_on_every_theme() {
        // 语义文字会同时出现在面板、展开卡片和折叠卡片上，三种真实底色都必须清楚。
        for theme in Theme::ALL {
            let palette = theme.palette();
            for (background_name, background) in [
                ("panel", palette.panel),
                ("faint", palette.faint),
                ("extreme", palette.extreme),
            ] {
                for (color_name, color) in [
                    ("ok", SEMANTICS.ok),
                    ("warn", SEMANTICS.warn),
                    ("err", SEMANTICS.err),
                    ("info", SEMANTICS.info),
                    ("info_alt", SEMANTICS_INFO_ALT),
                ] {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= 3.0,
                        "{} / {background_name} 的 {color_name} 对比度只有 {ratio:.2}（底色 {background:?}）",
                        theme.key()
                    );
                }
            }
        }
    }

    #[test]
    fn info_only_shifts_when_it_clashes_with_the_accent() {
        for theme in Theme::ALL {
            let colors = semantics_with_accent(theme.palette().accent);
            let expected = if distance(theme.palette().accent, SEMANTICS.info) < ACCENT_CLASH {
                SEMANTICS_INFO_ALT
            } else {
                SEMANTICS.info
            };
            assert_eq!(colors.info, expected, "{}", theme.key());
            assert_eq!(colors.ok, SEMANTICS.ok, "绿黄红不随主题变：{}", theme.key());
            assert_eq!(colors.warn, SEMANTICS.warn, "{}", theme.key());
            assert_eq!(colors.err, SEMANTICS.err, "{}", theme.key());
            // 换过之后必须真的拉开距离
            assert!(
                distance(colors.info, theme.palette().accent) >= ACCENT_CLASH,
                "{} 的信息色仍与强调色撞色",
                theme.key()
            );
        }
    }

    #[test]
    fn apply_switches_the_active_palette_and_pins_egui_theme() {
        let ctx = egui::Context::default();
        let mut applied: Option<Theme> = None;
        for theme in Theme::ALL {
            if needs_apply(&mut applied, theme) {
                theme.apply(&ctx);
            }
            let palette = theme.palette();
            let expected_egui_theme = egui::Theme::from_dark_mode(palette.dark);
            let expected_preference = egui::ThemePreference::from(expected_egui_theme);
            let visuals = ctx.style().visuals.clone();
            assert_eq!(visuals.panel_fill, palette.panel, "{}", theme.key());
            assert_eq!(visuals.dark_mode, palette.dark, "{}", theme.key());
            assert_eq!(ctx.theme(), expected_egui_theme, "{}", theme.key());
            assert_eq!(
                ctx.options(|options| options.theme_preference),
                expected_preference,
                "{} 不应继续跟随系统主题",
                theme.key()
            );
        }
        // 同一主题不重复应用（每帧重设会白白丢掉 egui 的样式缓存）
        assert!(!needs_apply(&mut applied, Theme::Rose));
        assert!(needs_apply(&mut applied, Theme::Dark));
    }

    #[test]
    fn hint_text_is_a_real_grey_not_a_dimmed_body_color() {
        // 要保证的性质不是「提示色比正文暗」——浅色主题的正文本来就是深色，
        // 提示比它**浅**才对——而是「提示对背景的对比度明显低于正文」，
        // 同时不能淡到读不清。下面按 WCAG 对比度查。
        let ctx = egui::Context::default();
        let mut applied: Option<Theme> = None;
        for theme in Theme::ALL {
            if needs_apply(&mut applied, theme) {
                theme.apply(&ctx);
            }
            let visuals = ctx.style().visuals.clone();
            let hint_color = theme.palette().hint_color();
            assert_eq!(visuals.weak_text_color(), hint_color, "{}", theme.key());

            let panel = visuals.panel_fill;
            let body = contrast(visuals.text_color(), panel);
            let hint = contrast(hint_color, panel);
            assert!(
                hint < body * 0.6,
                "{} 的提示色不够淡：正文 {body:.1}:1 vs 提示 {hint:.1}:1",
                theme.key()
            );
            assert!(
                hint >= 2.0,
                "{} 的提示色太淡，读不清：{hint:.1}:1",
                theme.key()
            );
        }
    }

    /// 一次丢弃式渲染：取回本帧所有顶点色（含数量）。
    ///
    /// 画三帧：egui 第一帧还没有字体，文字要等下一帧才画得出来。
    fn render_colors(theme: Theme, draw: impl Fn(&mut egui::Ui)) -> Vec<(Color32, usize)> {
        let ctx = egui::Context::default();
        theme.apply(&ctx);
        let mut out = None;
        for _ in 0..3 {
            out = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320.0, 120.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| draw(ui));
                },
            ));
        }
        // 用 `[r,g,b,a]` 当键：`Color32` 自己没有 `Ord`。
        let mut counts: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
        for prim in ctx.tessellate(out.expect("应有一帧").shapes, 1.0) {
            if let egui::epaint::Primitive::Mesh(mesh) = prim.primitive {
                for v in mesh.vertices {
                    *counts
                        .entry([v.color.r(), v.color.g(), v.color.b(), v.color.a()])
                        .or_default() += 1;
                }
            }
        }
        counts
            .into_iter()
            .map(|(k, n)| (Color32::from_rgba_premultiplied(k[0], k[1], k[2], k[3]), n))
            .collect()
    }

    #[test]
    fn hint_color_matches_a_disabled_widget_as_rendered() {
        // 提示色不是拍出来的常数，而是「禁用控件文字叠出来的观感」。
        // 那就不能只验算式：要拿 egui **真实画出来的顶点色**核对
        // 算式踩的是不是那两层色（按钮底、文字），否则 egui 换了
        // 禁用按钮用哪个 WidgetVisuals，算式再对也是错的。
        for theme in Theme::ALL {
            let palette = theme.palette();
            let faded = |color: Color32| color.gamma_multiply(DISABLED_ALPHA);
            let colors: Vec<Color32> = render_colors(theme, |ui| {
                ui.add_enabled(false, egui::Button::new("删除"));
            })
            .into_iter()
            .map(|(color, _)| color)
            .collect();
            assert!(
                colors.contains(&faded(palette.widget)),
                "{} 的禁用按钮底应是控件底色淡化",
                theme.key()
            );
            assert!(
                colors.contains(&faded(palette.text)),
                "{} 的禁用按钮文字应是正文色淡化",
                theme.key()
            );
            assert_eq!(
                over(
                    over(palette.panel, faded(palette.widget)),
                    faded(palette.text)
                ),
                palette.hint_color(),
                "{} 的提示色与禁用按钮文字不一致",
                theme.key()
            );
        }
    }

    #[test]
    fn a_text_edit_hint_actually_renders_in_the_hint_color() {
        // 钉住渲染结果：占位提示画出来的必须是提示色，不能是正文色。
        // （纯文本取色是 `override_text_color.unwrap_or(PLACEHOLDER)`，
        // 正文色若走 override，就会连 `hint_text` 的兜底色一起挡掉。）
        for theme in Theme::ALL {
            let palette = theme.palette();
            let colors = render_colors(theme, |ui| {
                let mut text = String::new();
                ui.add(egui::TextEdit::singleline(&mut text).hint_text("粘贴面板访问令牌"));
            });
            assert!(
                colors
                    .iter()
                    .any(|(color, _)| *color == palette.hint_color()),
                "{} 的占位提示没有用提示色",
                theme.key()
            );
            assert!(
                !colors.iter().any(|(color, _)| *color == palette.text),
                "{} 的占位提示被正文色盖住了",
                theme.key()
            );
        }
    }

    #[test]
    fn system_theme_change_cannot_replace_the_applied_palette() {
        let ctx = egui::Context::default();
        Theme::Nord.apply(&ctx);
        let expected_panel = Theme::Nord.palette().panel;

        let _ = ctx.run(
            egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            },
            |_| {},
        );

        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(ctx.style().visuals.panel_fill, expected_panel);
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
    fn hint_color(&self) -> Color32 {
        let faded = |color: Color32| color.gamma_multiply(DISABLED_ALPHA);
        over(over(self.panel, faded(self.widget)), faded(self.text))
    }

    fn into_visuals(self) -> Visuals {
        let hint = self.hint_color();
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.faint_bg_color = self.faint;
        v.extreme_bg_color = self.extreme;
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
