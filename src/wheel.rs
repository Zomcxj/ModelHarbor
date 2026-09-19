//! 卡片列表的两种展示模式：平面 / 圆柱转轮。
//!
//! 「圆柱转轮」用**等比缩放 + 向内收 + 淡出**近似 iOS 滚轮的纵深感：
//! egui 的 `TSTransform` 只有等比缩放与平移，做不出真正的绕轴旋转，
//! 但「中间最大最清晰、上下逐渐缩小淡出」这套视觉语言已经能传达
//! 「转着选」的感觉。
//!
//! 关键约束：变换必须走 [`egui::Ui::with_visual_transform`]（**只改视觉**），
//! 不能用 `Context::set_transform_layer`——后者会连带变换输入坐标，
//! 卡片里的输入框、按钮会整体偏移，点不准。

use eframe::egui;

/// 卡片列表展示模式。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum ListStyle {
    /// 平面：卡片等大平铺（默认）。
    #[default]
    Flat,
    /// 转轮：卡片按到选中卡的距离缩放淡出，形成纵深。
    Wheel,
}

impl ListStyle {
    pub const ALL: [ListStyle; 2] = [ListStyle::Flat, ListStyle::Wheel];

    pub fn label(&self) -> &'static str {
        match self {
            ListStyle::Flat => "平面",
            ListStyle::Wheel => "转轮",
        }
    }

    /// 持久化用的稳定标识（与界面文案解耦）。
    pub fn key(&self) -> &'static str {
        match self {
            ListStyle::Flat => "flat",
            ListStyle::Wheel => "wheel",
        }
    }

    /// 由持久化标识还原；未知 / 空值回落平面。
    pub fn from_key(key: &str) -> ListStyle {
        Self::ALL
            .into_iter()
            .find(|s| s.key() == key)
            .unwrap_or_default()
    }

    pub fn is_wheel(&self) -> bool {
        matches!(self, ListStyle::Wheel)
    }
}

/// 选中卡（距离 0）的缩放。
///
/// 固定为 1.0（**不放大**）：放大焦点卡会让它超出列表宽度、被窗口边缘裁掉。
/// 纵深感靠「其余卡片缩小」来表达——这样焦点卡正好占满可用宽度，与平面
/// 模式观感一致，也不会溢出。
pub const WHEEL_FOCUS_SCALE: f32 = 1.0;

/// 离选中卡最远的可见卡的最小缩放。
///
/// 焦点卡固定 1.0（不放大），纵深全靠这一侧缩小表达——所以要拉得够开，
/// 否则看起来只是「卡片大小不齐」。0.72 时最远卡明显退后，又没小到看不清。
pub const WHEEL_EDGE_SCALE: f32 = 0.72;

/// 最远卡的透明度下限。
pub const WHEEL_EDGE_ALPHA: f32 = 0.35;

/// 超出这个距离的卡片不再可见（转轮只显示选中卡附近的几张）。
pub const WHEEL_VISIBLE_RADIUS: usize = 3;

/// 焦点卡上方的预留行数：让焦点卡固定落在列表区靠上的位置。
///
/// 焦点卡前面有几张卡就占几行，不足的用空白补——焦点卡因此始终停在
/// 同一个高度（而不是随它在列表里的位置上下浮动），配合上下相邻卡片
/// 的缩小淡出，形成「绕圆柱转」的观感。
pub const WHEEL_LEAD_ROWS: usize = 2;

/// 转轮模式下每张卡占的竖直步长（像素），用于把滚动位移换算成卡片张数。
///
/// 取收起态卡片高度加行距的近似值；展开的卡片更高，但步长固定才能让
/// 滚动手感稳定（否则展开一张卡后滚动速度突然变化）。
pub const WHEEL_ROW_PITCH: f32 = 48.0;

/// 某张卡相对选中卡的视觉参数：`(缩放, 透明度)`。
///
/// `distance` 是到选中卡的卡片数距离（0 = 选中卡）。抽成纯函数是为了
/// 能直接断言「中间最大最实、两端最小最淡」这条单调性。
pub fn card_visual(distance: usize) -> (f32, f32) {
    let last = WHEEL_VISIBLE_RADIUS.max(1) as f32;
    let t = (distance as f32 / last).clamp(0.0, 1.0);
    let scale = WHEEL_FOCUS_SCALE + (WHEEL_EDGE_SCALE - WHEEL_FOCUS_SCALE) * t;
    let alpha = 1.0 + (WHEEL_EDGE_ALPHA - 1.0) * t;
    (scale, alpha)
}

/// 转轮模式下该卡是否可见（超出半径的整卡跳过，省掉无谓的布局与绘制）。
pub fn card_visible(selected: usize, index: usize) -> bool {
    selected.abs_diff(index) <= WHEEL_VISIBLE_RADIUS
}

/// 转轮模式的焦点推进：把滚轮 / 拖动位移换算成焦点移动。
///
/// `delta` 是本次输入的纵向位移（像素，向下为正）；每滚过 `row_step`
/// 像素就把焦点往下挪一张。返回新的焦点下标（已夹在 `0..len` 内）。
///
/// 抽成纯函数是为了能直接断言三条边界：向上越界夹到 0、向下越界夹到
/// 最后一张、位移不够一张时不移动（避免抖动）。
pub fn advance_focus(focus: usize, delta: f32, row_step: f32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    // 步长无效（非正 / 非有限）时不移动：比兜底成 1px 更安全——那会让
    // 一次 48px 的滚动跳掉几十张卡。
    if !row_step.is_finite() || row_step < 1.0 {
        return focus.min(len - 1);
    }
    let moved = (delta / row_step).round() as i64;
    if moved == 0 {
        return focus.min(len - 1);
    }
    (focus as i64 + moved).clamp(0, len as i64 - 1) as usize
}

/// 转轮模式下渲染一张卡片：按到焦点卡的距离做视觉缩放。
///
/// 缩放锚点取**左边缘中点**：卡片缩小后左侧贴齐（与列表左边缘对齐）、
/// 垂直方向仍居中于原行位置，看起来像沿圆柱面退远，而不是往中间挤。
/// 锚点用左边缘而非中心，是因为放大时以中心为锚会向两侧溢出屏幕；
/// 焦点卡不放大（`WHEEL_FOCUS_SCALE = 1.0`），所以只有缩小这一侧。
pub fn render_card_in_wheel<R>(
    ui: &mut egui::Ui,
    distance: usize,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let (scale, _alpha) = card_visual(distance);
    if (scale - 1.0).abs() < f32::EPSILON {
        return add(ui);
    }
    // 锚点：光标处的左边缘中点（用当前行高的一半估中点，渲染前不知道真实高度）。
    // `mul_pos(p) = scaling * p + translation`，让锚点不动
    // （`scale * anchor + t == anchor`）→ `t = anchor * (1 - scale)`。
    let row_h = ui.spacing().interact_size.y.max(WHEEL_ROW_PITCH / 2.0);
    let anchor = ui.cursor().min.to_vec2() + egui::vec2(0.0, row_h / 2.0);
    let transform = egui::emath::TSTransform::new(anchor * (1.0 - scale), scale);
    ui.with_visual_transform(transform, add).inner
}

/// 转轮模式的上下渐变淡出遮罩：把列表上下边缘的卡片融进面板底色。
///
/// egui 没有内置的渐变淡出，用带顶点色的 mesh 自绘（与滚动区那套同思路）：
/// 每侧一个从 `panel_fill` 不透明到全透明的矩形，覆盖在卡片之上。
/// 只在转轮模式调用——平面模式没有纵深，盖上去只会糊掉首末行。
pub fn wheel_fade(ui: &egui::Ui, area: egui::Rect) {
    let base = ui.visuals().panel_fill;
    let clear = egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 0);
    let painter = ui.painter().with_clip_rect(area);
    let mut mesh = egui::Mesh::default();
    let top = egui::Rect::from_min_max(
        area.min,
        egui::pos2(area.right(), area.top() + WHEEL_FADE_HEIGHT),
    );
    add_fade_quad(&mut mesh, top, base, clear);
    let bottom = egui::Rect::from_min_max(
        egui::pos2(area.left(), area.bottom() - WHEEL_FADE_HEIGHT),
        area.max,
    );
    add_fade_quad(&mut mesh, bottom, clear, base);
    painter.add(egui::Shape::mesh(mesh));
}

/// 上下渐变过渡带高度（像素）。
pub const WHEEL_FADE_HEIGHT: f32 = 48.0;

/// 往 mesh 里加一个竖向渐变的矩形：`top_color` 在上、`bottom_color` 在下。
fn add_fade_quad(
    mesh: &mut egui::Mesh,
    rect: egui::Rect,
    top_color: egui::Color32,
    bottom_color: egui::Color32,
) {
    let idx = mesh.vertices.len() as u32;
    for (pos, color) in [
        (rect.left_top(), top_color),
        (rect.right_top(), top_color),
        (rect.right_bottom(), bottom_color),
        (rect.left_bottom(), bottom_color),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: egui::epaint::WHITE_UV,
            color,
        });
    }
    mesh.indices
        .extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 焦点卡不能放大：放大后会超出列表宽度被窗口裁掉（曾经 1.06 就溢出屏幕）。
    #[test]
    fn focus_card_never_scales_above_one() {
        assert!(
            WHEEL_FOCUS_SCALE <= 1.0,
            "焦点卡缩放 {} 大于 1，会溢出屏幕",
            WHEEL_FOCUS_SCALE
        );
        let (scale, _) = card_visual(0);
        assert!(scale <= 1.0);
        // 所有档位都不得放大
        for d in 0..=WHEEL_VISIBLE_RADIUS {
            let (s, _) = card_visual(d);
            assert!(s <= 1.0, "距离 {d} 的缩放 {s} 大于 1");
        }
    }

    /// 中间卡最大最实，越远越小越淡，且单调。
    #[test]
    fn card_visual_is_monotonic_from_focus_to_edge() {
        let (focus_scale, focus_alpha) = card_visual(0);
        assert!((focus_scale - WHEEL_FOCUS_SCALE).abs() < 1e-6);
        assert!((focus_alpha - 1.0).abs() < 1e-6);

        let (edge_scale, edge_alpha) = card_visual(WHEEL_VISIBLE_RADIUS);
        assert!((edge_scale - WHEEL_EDGE_SCALE).abs() < 1e-6);
        assert!((edge_alpha - WHEEL_EDGE_ALPHA).abs() < 1e-6);

        let mut prev = (focus_scale, focus_alpha);
        for d in 1..=WHEEL_VISIBLE_RADIUS {
            let cur = card_visual(d);
            assert!(cur.0 < prev.0, "距离 {d} 缩放没变小");
            assert!(cur.1 < prev.1, "距离 {d} 透明度没变小");
            prev = cur;
        }
    }

    /// 超出可见半径的距离要夹到边缘档，不能算出更小的缩放。
    #[test]
    fn card_visual_clamps_beyond_the_visible_radius() {
        let edge = card_visual(WHEEL_VISIBLE_RADIUS);
        for d in [WHEEL_VISIBLE_RADIUS + 1, 99] {
            let (scale, alpha) = card_visual(d);
            assert!(scale > 0.0, "缩放不能为负");
            assert!((scale - edge.0).abs() < 1e-6);
            assert!((alpha - edge.1).abs() < 1e-6);
        }
    }

    /// 可见性：选中卡附近可见，远处不可见；边界对称。
    #[test]
    fn card_visibility_is_a_symmetric_window() {
        let sel = 10;
        assert!(card_visible(sel, 10));
        assert!(card_visible(sel, 10 - WHEEL_VISIBLE_RADIUS));
        assert!(card_visible(sel, 10 + WHEEL_VISIBLE_RADIUS));
        assert!(!card_visible(sel, 10 - WHEEL_VISIBLE_RADIUS - 1));
        assert!(!card_visible(sel, 10 + WHEEL_VISIBLE_RADIUS + 1));
        // 边界处不能下溢（selected < index 与 selected > index 都要安全）
        assert!(card_visible(0, 0));
        assert!(!card_visible(0, WHEEL_VISIBLE_RADIUS + 1));
    }

    /// 模式标识往返。
    #[test]
    fn list_style_round_trips_and_falls_back() {
        for style in ListStyle::ALL {
            assert_eq!(ListStyle::from_key(style.key()), style);
        }
        assert_eq!(ListStyle::from_key(""), ListStyle::Flat);
        assert_eq!(ListStyle::from_key("nonsense"), ListStyle::Flat);
    }

    /// 焦点推进：向下滚焦点后移、向上滚前移，两端夹住不越界。
    #[test]
    fn advance_focus_moves_and_clamps() {
        // 向下滚一张（48px 步长）：焦点 +1
        assert_eq!(advance_focus(0, 48.0, 48.0, 10), 1);
        // 向上滚：焦点 -1
        assert_eq!(advance_focus(5, -48.0, 48.0, 10), 4);
        // 位移不够一张：不动（避免抖动）
        assert_eq!(advance_focus(3, 10.0, 48.0, 10), 3);
        // 向上越界夹到 0
        assert_eq!(advance_focus(0, -999.0, 48.0, 10), 0);
        // 向下越界夹到最后一张
        assert_eq!(advance_focus(8, 999.0, 48.0, 10), 9);
        // 空列表安全返回 0
        assert_eq!(advance_focus(0, 48.0, 48.0, 0), 0);
        // 步长非正时不移动（不能兜底成 1px，否则一次滚动跳几十张）
        assert_eq!(advance_focus(2, 48.0, 0.0, 10), 2);
        assert_eq!(advance_focus(2, 48.0, -5.0, 10), 2);
    }
}
