use eframe::egui::{self, Color32, Stroke, Visuals};

/// 淡化禁用控件时用的 alpha（即 `Visuals::disabled_alpha` 的默认值）。
///
/// 这里写常量而不是去问 egui：主题构造时手上只有调色板。单测会拿
/// `Visuals::disabled_alpha()` 核对，egui 改了默认值就会红。
const DISABLED_ALPHA: f32 = 0.5;

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
/// 徽标 / 极短标记。egui 默认 `Small` 只有 9px，中文在这个尺寸下吃力，抬到 11。
pub const TEXT_CAPTION: f32 = 11.0;
/// 副信息：来源、请求次数、站点名。
pub const TEXT_SMALL: f32 = 12.0;
/// 卡片标题（provider key）。
pub const TEXT_TITLE: f32 = 16.0;
/// 正文 / 控件文字。egui 默认就是 13，这里显式写出来当刻度基准。
pub const TEXT_BODY: f32 = 13.0;
/// 区块标题。
pub const TEXT_HEADING: f32 = 18.0;

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

/// 提示 / 占位文字的最低对比度。
///
/// 比正文低一档：提示色是推导值（见 [`Palette::hint_color`]），上限由控件底色决定 ——
/// 要再高就得把按钮做成中灰，控件反而比面板抢眼。
pub const CONTRAST_HINT_MIN: f32 = 3.0;

/// 非文字 UI 组件（控件描边、边框）的最低对比度。
pub const CONTRAST_UI_MIN: f32 = 3.0;

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

/// 界面形状预设：圆角、描边粗细、内嵌亮暗边、投影的组合。
///
/// 与主题正交 —— 主题管颜色，形状管「控件长什么样」。8 档覆盖常见观感。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum UiStyle {
    /// 圆润：默认档，圆角 10、1px 描边。
    #[default]
    Soft,
    /// 精致：圆角 6 + 0.5px 细边，轻量感。
    Fine,
    /// 极简：全直角 + 0px 无边框，纯色块。
    Minimal,
    /// 标签：胶囊 20 + 0.5px 细边，标签样式。
    Tag,
    /// 云朵：大圆角 16 + 软投影，卡片浮在底上。
    Cloud,
    /// 浮雕：中圆角 8 + 双向内嵌边，浅底也能看出凹凸。
    Emboss,
    /// 石板：小圆角 + 亮暗内嵌边，做出轻微立体感。
    Slab,
    /// 棱镜：小圆角 4 + 2px 粗边 + 内光影，强几何感。
    Prism,
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
        UiStyle::Prism,
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
            UiStyle::Prism => "棱镜",
        }
    }

    /// 持久化用的稳定标识（与界面文案解耦）。
    pub fn key(&self) -> &'static str {
        match self {
            UiStyle::Soft => "soft",
            UiStyle::Fine => "fine",
            UiStyle::Minimal => "minimal",
            UiStyle::Tag => "tag",
            UiStyle::Cloud => "cloud",
            UiStyle::Emboss => "emboss",
            UiStyle::Slab => "slab",
            UiStyle::Prism => "prism",
        }
    }

    /// 由持久化标识还原；未知 / 空值回落默认档。
    pub fn from_key(key: &str) -> UiStyle {
        UiStyle::ALL
            .into_iter()
            .find(|style| style.key() == key)
            .unwrap_or_default()
    }

    /// 该预设的默认圆角（滑块可在此基础上继续调）。
    pub fn radius(&self) -> u8 {
        match self {
            UiStyle::Soft => RADIUS_MD,
            UiStyle::Fine => 6,
            UiStyle::Minimal => 0,
            UiStyle::Tag => RADIUS_SLIDER_MAX,
            UiStyle::Cloud => 16,
            UiStyle::Emboss => 8,
            UiStyle::Slab => RADIUS_SM,
            UiStyle::Prism => 4,
        }
    }

    /// 控件描边宽度（像素）。
    pub fn border_width(&self) -> f32 {
        match self {
            UiStyle::Soft | UiStyle::Slab | UiStyle::Emboss => 1.0,
            UiStyle::Prism => 2.0,
            UiStyle::Minimal => 0.0,
            UiStyle::Fine | UiStyle::Tag => 0.5,
            // 云朵的卡片本身不描边（靠投影成形），输入框 / 按钮仍要 1px，否则浅底上看不见框。
            UiStyle::Cloud => 1.0,
        }
    }

    /// 是否画亮暗内嵌边（左上偏亮、右下偏暗的立体感）。
    pub fn has_bevel(&self) -> bool {
        matches!(self, UiStyle::Slab | UiStyle::Emboss | UiStyle::Prism)
    }

    /// 是否给卡片画一层软投影（云朵）。
    pub fn has_card_shadow(&self) -> bool {
        matches!(self, UiStyle::Cloud)
    }

    /// 写进 egui `WidgetVisuals` 的描边宽度。
    ///
    /// 浮雕 / 石板的控件描边要换成内嵌暗边（1px）；棱镜靠粗边立骨架，保留原宽。
    pub fn widget_stroke_width(&self) -> f32 {
        if self.has_bevel() && !matches!(self, UiStyle::Prism) {
            1.0
        } else {
            self.border_width()
        }
    }

    /// 卡片软投影：深色用黑、浅色用更深的灰，浅底上也能看出浮起。
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

    /// 内嵌边颜色。
    ///
    /// 浅底上白高光隐形（白上白），**质感全靠深色暗边**：浅色主题的暗边
    /// 拉到近实色（石板 190 / 浮雕 230 / 棱镜 255）——1px 的半透明线在
    /// 浅底上会化掉，近实色才有「刻出来」的凹凸。深底走反方向：亮边负责
    /// 立体感。
    pub fn bevel_colors(&self, dark: bool) -> (Color32, Color32) {
        let style = match self {
            UiStyle::Emboss => 2, // 浮雕：强
            UiStyle::Prism => 3,  // 棱镜：最强
            _ => 1,               // 石板：标准
        };
        if dark {
            match style {
                3 => (
                    Color32::from_white_alpha(80),
                    Color32::from_black_alpha(200),
                ),
                2 => (
                    Color32::from_white_alpha(55),
                    Color32::from_black_alpha(160),
                ),
                _ => (
                    Color32::from_white_alpha(30),
                    Color32::from_black_alpha(120),
                ),
            }
        } else {
            match style {
                3 => (
                    Color32::from_white_alpha(255),
                    Color32::from_black_alpha(255),
                ),
                2 => (
                    Color32::from_white_alpha(255),
                    Color32::from_black_alpha(230),
                ),
                _ => (
                    Color32::from_white_alpha(255),
                    Color32::from_black_alpha(190),
                ),
            }
        }
    }
}

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
                false, 0xF0FAF5, 0xE6F7ED, 0xFFFFFF, 0xD0F0DE, 0xBFEBD3, 0x2D8659, 0x1A4D33,
                0x7A9B88, 0xFFFFFF,
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
        if shape.has_bevel() {
            // 石板 / 浮雕 / 棱镜：控件描边换成内嵌暗边（棱镜保留自己的粗边），
            // 悬浮窗给偏右下的投影。
            let (_, edge) = shape.bevel_colors(dark);
            style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(
                if matches!(shape, UiStyle::Prism) {
                    shape.border_width()
                } else {
                    1.0
                },
                edge,
            );
            style.visuals.widgets.inactive.bg_stroke =
                style.visuals.widgets.noninteractive.bg_stroke;
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

/// 语义色（绿 = 正常 / 黄 = 注意 / 红 = 异常 / 蓝 = 信息）。
///
/// **跨主题统一**：所有主题共用同一套（黑白灰随主题变，彩色不变），
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

/// 对比度（WCAG，1.0 ~ 21.0）。只在单测里用来锁定「每个主题都看得清」。
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

/// 单测用：其他模块锁定配色对比度时复用同一套公式。
#[cfg(test)]
pub(crate) fn contrast_for_tests(a: Color32, b: Color32) -> f32 {
    contrast(a, b)
}

/// 单测用：RGB 欧氏距离，判断两块颜色是否看得出区别。
#[cfg(test)]
pub(crate) fn distance_for_tests(a: Color32, b: Color32) -> f32 {
    distance(a, b)
}

/// 当前形状预设存在上下文里的键。
const ACTIVE_STYLE_ID: &str = "modelharbor_active_ui_style";

/// 当前生效的形状预设（还没套用样式时是默认档）。
pub fn active_style(ctx: &egui::Context) -> UiStyle {
    ctx.data(|data| data.get_temp::<UiStyle>(egui::Id::new(ACTIVE_STYLE_ID)))
        .unwrap_or_default()
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

/// 主题或圆角变了都要重套样式。
///
/// `applied` 记录已套用的（主题, 圆角），会被就地更新。
pub fn needs_apply_style(
    applied: &mut Option<(Theme, UiStyle)>,
    theme: Theme,
    shape: UiStyle,
) -> bool {
    if *applied == Some((theme, shape)) {
        return false;
    }
    *applied = Some((theme, shape));
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
        // 同一主题不重复应用（每帧重设会白白丢掉 egui 的样式缓存）。
        // 用 `ALL` 的末项而不是写死某个主题：主题表增删时这里不该跟着改。
        let last = *Theme::ALL.last().expect("主题表非空");
        assert!(!needs_apply(&mut applied, last));
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
                hint >= CONTRAST_HINT_MIN,
                "{} 的提示色太淡，读不清：{hint:.1}:1（下限 {CONTRAST_HINT_MIN}）",
                theme.key()
            );
        }
    }

    #[test]
    fn every_theme_meets_the_text_and_border_contrast_floor() {
        // 设计系统的硬指标：正文与强调色是文字（4.5:1），描边是非文字 UI（3:1）。
        for theme in Theme::ALL {
            let palette = theme.palette();
            for (what, color, floor) in [
                ("正文", palette.text, CONTRAST_TEXT_MIN),
                ("强调色", palette.accent, CONTRAST_TEXT_MIN),
                ("描边", palette.border, CONTRAST_UI_MIN),
            ] {
                for (background_name, background) in [
                    ("panel", palette.panel),
                    ("faint", palette.faint),
                    ("extreme", palette.extreme),
                ] {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= floor,
                        "{} 的{what} / {background_name} 对比度 {ratio:.2}:1 低于 {floor}:1",
                        theme.key()
                    );
                }
            }
        }
    }

    #[test]
    fn accent_text_reads_on_the_accent_fill() {
        // 强调色是选中态行与主按钮的**底色**，所以它上面的文字要单独校验。
        for theme in Theme::ALL {
            let palette = theme.palette();
            let ratio = contrast(palette.accent_text, palette.accent);
            assert!(
                ratio >= CONTRAST_TEXT_MIN,
                "{} 的强调色文字对比度只有 {ratio:.2}:1（下限 {CONTRAST_TEXT_MIN}）",
                theme.key()
            );
        }
    }

    #[test]
    fn tap_targets_are_tall_enough_to_hit() {
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let spacing = ctx.style().spacing.clone();
        assert!(
            spacing.interact_size.y >= TAP_TARGET_MIN,
            "点击区高度 {} 低于 {TAP_TARGET_MIN}",
            spacing.interact_size.y
        );
    }

    #[test]
    fn spacing_scale_is_used_by_the_style() {
        // 刻度常量必须真的进了 Style，否则改 token 不影响界面。
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let spacing = ctx.style().spacing.clone();
        assert_eq!(spacing.item_spacing.y, SPACE_2);
        assert_eq!(spacing.button_padding.x, SPACE_4 - 2.0);
        assert_eq!(spacing.button_padding.y, SPACE_1 - 1.0);
    }

    #[test]
    fn text_scale_is_used_by_the_style() {
        // egui 默认 Small 是 9px，中文读不清；这条钉住它走我们的刻度。
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let styles = ctx.style().text_styles.clone();
        assert_eq!(styles[&egui::TextStyle::Small].size, TEXT_CAPTION);
        assert_eq!(styles[&egui::TextStyle::Body].size, TEXT_BODY);
        assert_eq!(styles[&egui::TextStyle::Button].size, TEXT_BODY);
        assert_eq!(styles[&egui::TextStyle::Heading].size, TEXT_HEADING);
        // 副信息字号必须明显高于 egui 默认的 9px，否则中文读不清。
        let default_small = egui::TextStyle::Small.resolve(&egui::Style::default()).size;
        assert!(
            TEXT_CAPTION > default_small,
            "副信息字号 {TEXT_CAPTION} 不应退回 egui 默认的 {default_small}"
        );
        assert!(
            TEXT_CAPTION < TEXT_SMALL
                && TEXT_SMALL < TEXT_BODY
                && TEXT_BODY < TEXT_TITLE
                && TEXT_TITLE < TEXT_HEADING,
            "字阶必须严格递增：caption {TEXT_CAPTION} < small {TEXT_SMALL} < body {TEXT_BODY} < title {TEXT_TITLE} < heading {TEXT_HEADING}"
        );
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
    fn new(
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
        v.selection.bg_fill = self.accent;
        v.widgets.inactive.weak_bg_fill = self.widget;
        v.widgets.inactive.bg_fill = self.widget;
        v.widgets.hovered.weak_bg_fill = self.hover;
        v.widgets.hovered.bg_fill = self.hover;
        v.widgets.active.weak_bg_fill = self.accent;
        v.widgets.active.bg_fill = self.accent;
        // 控件描边：控件底色与面板底色只差 1.2–1.5:1，不给描边就靠这点色差分边界。
        // 输入框用 `extreme` 底，同样靠这条线成形。
        let edge = Stroke::new(1.0, self.border);
        v.widgets.noninteractive.bg_stroke = edge;
        v.widgets.inactive.bg_stroke = edge;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, self.accent);
        v.widgets.active.bg_stroke = Stroke::new(1.0, self.accent);
        // 强调色底上的文字：选中态行 / 主按钮要看得清（浅底主题用白、深底主题用黑）。
        v.selection.stroke = Stroke::new(1.0, self.accent_text);
        v
    }
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
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
        for removed in ["heavy", "sharp", "frosted", "compact", "neon"] {
            assert!(!UiStyle::ALL.iter().any(|s| s.key() == removed));
            assert_eq!(UiStyle::from_key(removed), UiStyle::default());
        }
        assert_eq!(UiStyle::from_key("cloud").key(), "cloud");
        assert_eq!(UiStyle::from_key("emboss").key(), "emboss");
        assert_eq!(UiStyle::from_key("prism").key(), "prism");
        assert_eq!(UiStyle::from_key("minimal").key(), "minimal");
        assert_eq!(UiStyle::from_key("fine").key(), "fine");
    }

    #[test]
    fn light_slab_bevel_is_stronger_than_dark() {
        let (light_hi, light_lo) = UiStyle::Slab.bevel_colors(false);
        let (dark_hi, dark_lo) = UiStyle::Slab.bevel_colors(true);
        assert!(
            light_lo.a() > dark_hi.a(),
            "浅色石板的暗边必须比深色石板的亮边更实，否则浅底上看不见"
        );
        assert!(light_hi.a() >= dark_hi.a());
        assert!(light_lo.a() > 0 && dark_lo.a() > 0);
        let (emboss_hi, emboss_lo) = UiStyle::Emboss.bevel_colors(false);
        assert!(emboss_lo.a() >= light_lo.a());
        assert!(emboss_hi.a() >= light_hi.a());
        // 浅色主题的暗边必须近实色：半透明线在浅底上会化掉，用户已两次反馈。
        assert!(
            light_lo.a() >= 190,
            "浅色石板暗边 alpha {} 不够实",
            light_lo.a()
        );
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
