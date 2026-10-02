use super::tokens::{RADIUS_MD, RADIUS_SLIDER_MAX, RADIUS_SM};
use super::Theme;
use eframe::egui::{self, Color32};

/// 界面形状预设：圆角、描边粗细、内嵌亮暗边、投影、色带的组合。
///
/// 与主题正交 —— 主题管颜色，形状管「控件长什么样」。8 档各有一套
/// **不同的机制**（描边 / 无边框 / 凸起光线 / 软投影 / 接触影 / 顶部色带），
/// 而不是同一效果调强度。
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
    /// 浮雕：凸起受光线 + 接触影，卡面「立」在底上。
    Emboss,
    /// 石板：平放石板——深色描边 + 接触影，无受光线。
    Slab,
    /// 色带：卡片顶部一条主题强调色色带。
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
            UiStyle::Band => "band",
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
            UiStyle::Band => 8,
        }
    }

    /// 控件描边宽度（像素）。
    pub fn border_width(&self) -> f32 {
        match self {
            UiStyle::Soft | UiStyle::Slab | UiStyle::Emboss | UiStyle::Band => 1.0,
            UiStyle::Minimal => 0.0,
            UiStyle::Fine | UiStyle::Tag => 0.5,
            // 云朵的卡片本身不描边（靠投影成形），输入框 / 按钮仍要 1px，否则浅底上看不见框。
            UiStyle::Cloud => 1.0,
        }
    }

    /// 是否画凸起受光线（卡内左上亮、右下暗）。
    pub fn has_bevel(&self) -> bool {
        matches!(self, UiStyle::Emboss)
    }

    /// 是否画接触影（向右下偏移的整圈暗色描边，让卡片「坐」在底上）。
    pub fn has_contact_shadow(&self) -> bool {
        matches!(self, UiStyle::Slab | UiStyle::Emboss)
    }

    /// 是否给卡片画一层软投影（云朵）。
    pub fn has_card_shadow(&self) -> bool {
        matches!(self, UiStyle::Cloud)
    }

    /// 是否在卡片顶部画主题强调色色带。
    pub fn has_accent_bar(&self) -> bool {
        matches!(self, UiStyle::Band)
    }

    /// 写进 egui `WidgetVisuals` 的描边宽度。
    ///
    /// 浮雕的控件描边要换成内嵌暗边（1px），让整卡质感统一；石板的
    /// 深色描边本身就是边框，直接用原宽。
    pub fn widget_stroke_width(&self) -> f32 {
        if self.has_bevel() {
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

    /// 凸起受光线的颜色（浮雕专用），返回 `(亮边, 暗边)`。
    ///
    /// 浅底上白高光隐形（白上白），**立体感全靠深色暗边**：浅色主题的暗边
    /// 拉到近实色（alpha 230）——1px 的半透明线在浅底上会化掉。
    /// 深底走反方向：亮边负责立体感。
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

    /// 接触影颜色：向右下偏移的整圈暗色描边用。石板 / 浮雕共用，
    /// 浅底更实、深底更透。
    pub fn contact_shadow_color(&self, dark: bool) -> Color32 {
        if dark {
            Color32::from_black_alpha(120)
        } else {
            Color32::from_black_alpha(140)
        }
    }
}

/// 当前形状预设存在上下文里的键。
pub(super) const ACTIVE_STYLE_ID: &str = "modelharbor_active_ui_style";

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
