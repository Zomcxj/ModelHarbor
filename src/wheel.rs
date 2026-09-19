//! 圆柱滚轮选择器：iOS 闹钟选时间那种 3D 转轮。
//!
//! 与平面下拉并列的另一种「选择器样式」，由「外观」面板切换。
//! 中间项最大最清晰，上下项按圆柱透视缩小、变淡、向内侧收，
//! 形成纵深；滚轮拖动 / 滚轮滚动切换选中项。
//!
//! 实现要点：egui 没有 3D 变换，圆柱效果靠**逐项缩放 + 透明度 + 位置**
//! 手工模拟——每一项的视觉参数都由它到选中项的「角度距离」推出来。

use eframe::egui;

/// 枚举选择器的展示样式。
///
/// 与形状预设正交：形状管「长什么样」，这里管「怎么选」。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum PickerStyle {
    /// 平面：下拉框 + 勾选列表（egui 默认观感）。
    #[default]
    Flat,
    /// 滚轮：iOS 闹钟式的圆柱转轮。
    Wheel,
}

impl PickerStyle {
    pub const ALL: [PickerStyle; 2] = [PickerStyle::Flat, PickerStyle::Wheel];

    pub fn label(&self) -> &'static str {
        match self {
            PickerStyle::Flat => "平面",
            PickerStyle::Wheel => "滚轮",
        }
    }

    /// 持久化用的稳定标识（与界面文案解耦）。
    pub fn key(&self) -> &'static str {
        match self {
            PickerStyle::Flat => "flat",
            PickerStyle::Wheel => "wheel",
        }
    }

    /// 由持久化标识还原；未知 / 空值回落平面（默认档）。
    pub fn from_key(key: &str) -> PickerStyle {
        Self::ALL
            .into_iter()
            .find(|s| s.key() == key)
            .unwrap_or_default()
    }

    pub fn is_wheel(&self) -> bool {
        matches!(self, PickerStyle::Wheel)
    }
}

/// 圆柱滚轮的可见行数（含选中项）。奇数保证选中项居中。
pub const WHEEL_VISIBLE_ROWS: usize = 5;

/// 单行基准高度（像素）。
pub const WHEEL_ROW_HEIGHT: f32 = 26.0;

/// 滚轮固定宽度（像素）。太窄文字放不下，太宽在表单里喧宾夺主。
pub const WHEEL_WIDTH: f32 = 160.0;

/// 选中项的放大倍率上限；离中心越远越小。
pub const WHEEL_MAX_SCALE: f32 = 1.12;

/// 边缘项的最小缩放（再小就糊了）。
pub const WHEEL_MIN_SCALE: f32 = 0.62;

/// 离中心最远项的透明度下限。
pub const WHEEL_MIN_ALPHA: f32 = 0.25;

/// 选中项的可选值集合与当前值。
pub struct WheelState<'a> {
    pub options: &'a [String],
    pub selected: usize,
}

/// 某一项相对选中项的视觉参数：`(缩放, 透明度)`。
///
/// `distance` 是到选中项的项数距离（0 = 选中项）。抽成纯函数是为了能直接
/// 断言「中间最大最实、两端最小最淡」这条单调性——写死查表容易在改行数时
/// 漏掉某一档。
pub fn row_visual(distance: usize) -> (f32, f32) {
    let last = (WHEEL_VISIBLE_ROWS / 2).max(1) as f32;
    let t = (distance as f32 / last).clamp(0.0, 1.0);
    let scale = WHEEL_MAX_SCALE + (WHEEL_MIN_SCALE - WHEEL_MAX_SCALE) * t;
    let alpha = 1.0 + (WHEEL_MIN_ALPHA - 1.0) * t;
    (scale, alpha)
}

/// 圆柱滚轮的交互结果。
pub struct WheelOutcome {
    /// 本帧选中项是否变化。
    pub changed: bool,
    /// 当前选中项下标（供调用方直接写回，不必再读存储）。
    pub selected: usize,
}

/// 圆柱滚轮：中间项最大最清晰，上下项缩小淡出。
///
/// `id_salt` 需在界面内唯一（同一卡片多个滚轮要区分）。
pub fn wheel_selector(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + Clone,
    state: WheelState<'_>,
) -> WheelOutcome {
    let id = ui.make_persistent_id(id_salt.clone());
    let total = state.options.len();
    if total == 0 {
        ui.label(egui::RichText::new("（无可选项）").weak());
        return WheelOutcome {
            changed: false,
            selected: 0,
        };
    }
    let mut changed = false;
    let mut selected = state.selected.min(total - 1);

    let half = WHEEL_VISIBLE_ROWS / 2;
    let height = WHEEL_ROW_HEIGHT * WHEEL_VISIBLE_ROWS as f32;
    // 固定宽度：调用点多在 `horizontal_wrapped` 里，`available_width()` 只剩
    // 行尾的零头，滚轮会被压成一条。宽度不够时让它换行（`allocate_exact_size`
    // 在水平布局里遇到放不下的宽度会自动另起一行）。
    let width = WHEEL_WIDTH.min(ui.available_width().max(WHEEL_WIDTH));
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click_and_drag());

    // 拖动 / 滚轮：按行高换算成项数增量。累积到 id 上，跨帧保留余量，
    // 否则慢速拖动会被逐帧取整抹成 0。
    let mut scroll_accum = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(id.with("accum")))
        .unwrap_or(0.0);
    if response.dragged() {
        scroll_accum -= response.drag_delta().y;
    }
    if response.hovered() {
        scroll_accum += ui.input(|i| i.smooth_scroll_delta.y);
    }
    let steps = (scroll_accum / WHEEL_ROW_HEIGHT).trunc();
    if steps != 0.0 {
        scroll_accum -= steps * WHEEL_ROW_HEIGHT;
        let next = (selected as i32 - steps as i32).clamp(0, total as i32 - 1) as usize;
        if next != selected {
            selected = next;
            changed = true;
        }
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(id.with("accum"), scroll_accum));

    // 逐项绘制。所有绘制都收在控件矩形内（`with_clip_rect`），
    // 否则会画到卡片外面被上层裁剪掉，看起来像什么都没画。
    let center_y = rect.center().y;
    let painter = ui.painter().with_clip_rect(rect);
    let text_color = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    // 由远及近：远的先画，近的后画盖在上面。
    for offset in (1..=half as i32).rev() {
        for sign in [-1i32, 1] {
            let idx = selected as i32 + sign * offset;
            if idx < 0 || idx >= total as i32 {
                continue;
            }
            let distance = offset as usize;
            let (scale, alpha) = row_visual(distance);
            // 圆柱透视：离中心越远，纵向压得越扁、颜色越淡。
            let row_center = center_y + sign as f32 * WHEEL_ROW_HEIGHT * offset as f32;
            let color = if sign < 0 { text_color } else { weak }.gamma_multiply(alpha);
            paint_row(
                &painter,
                rect,
                row_center,
                scale,
                &state.options[idx as usize],
                color,
            );
        }
    }
    // 选中底与上下指示线，再画选中项文字（最上层）。
    let accent = ui.visuals().selection.bg_fill;
    let sel_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, center_y),
        egui::vec2(rect.width(), WHEEL_ROW_HEIGHT),
    );
    painter.rect_filled(sel_rect, 4.0, accent.gamma_multiply(0.35));
    for y in [sel_rect.top(), sel_rect.bottom()] {
        painter.hline(
            sel_rect.x_range(),
            y,
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }
    let (scale, _) = row_visual(0);
    paint_row(
        &painter,
        rect,
        center_y,
        scale,
        &state.options[selected],
        ui.visuals().strong_text_color(),
    );

    // 点击上下项直接跳选。
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let delta = pos.y - center_y;
            let steps = (delta / WHEEL_ROW_HEIGHT).round() as i32;
            let next = (selected as i32 + steps).clamp(0, total as i32 - 1) as usize;
            if next != selected {
                selected = next;
                changed = true;
            }
        }
    }
    if changed {
        ui.ctx()
            .data_mut(|d| d.insert_temp(id.with("selected"), selected));
    } else {
        // 没变化时以存储为准（调用方可能只在 changed 时写回）。
        selected = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(id.with("selected")))
            .unwrap_or(selected);
    }
    WheelOutcome { changed, selected }
}

/// 画一行文字：以 `row_center_y` 为行中心、按 `scale` 缩放、水平居中。
///
/// 裁剪由调用方统一设置（整个控件矩形），这里不再各自收窄——
/// 逐行裁剪会把相邻行切掉一半，视觉上像被咬掉一块。
fn paint_row(
    painter: &egui::Painter,
    rect: egui::Rect,
    row_center_y: f32,
    scale: f32,
    text: &str,
    color: egui::Color32,
) {
    let font = egui::FontId::proportional(14.0 * scale);
    let galley = painter.layout_no_wrap(text.to_string(), font, color);
    let pos = egui::pos2(
        rect.center().x - galley.size().x / 2.0,
        row_center_y - galley.size().y / 2.0,
    );
    painter.galley(pos, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 中间项必须最大最实，两端最小最淡，且随距离单调变化。
    #[test]
    fn row_visual_is_monotonic_from_center_to_edge() {
        let (center_scale, center_alpha) = row_visual(0);
        assert!((center_scale - WHEEL_MAX_SCALE).abs() < 1e-6);
        assert!((center_alpha - 1.0).abs() < 1e-6);

        let last = WHEEL_VISIBLE_ROWS / 2;
        let (edge_scale, edge_alpha) = row_visual(last);
        assert!((edge_scale - WHEEL_MIN_SCALE).abs() < 1e-6);
        assert!((edge_alpha - WHEEL_MIN_ALPHA).abs() < 1e-6);

        let mut prev = (center_scale, center_alpha);
        for d in 1..=last {
            let cur = row_visual(d);
            assert!(
                cur.0 < prev.0,
                "距离 {d} 的缩放没有变小：{cur:?} vs {prev:?}"
            );
            assert!(
                cur.1 < prev.1,
                "距离 {d} 的透明度没有变小：{cur:?} vs {prev:?}"
            );
            prev = cur;
        }
    }

    /// 超出可见范围的项要被夹到边缘档，不能算出负数缩放。
    #[test]
    fn row_visual_clamps_beyond_the_visible_range() {
        let edge = row_visual(WHEEL_VISIBLE_ROWS / 2);
        for d in [WHEEL_VISIBLE_ROWS, 99] {
            let (scale, alpha) = row_visual(d);
            assert!(scale > 0.0, "缩放不能为负：{scale}");
            assert!((scale - edge.0).abs() < 1e-6);
            assert!((alpha - edge.1).abs() < 1e-6);
        }
    }

    /// 可见行数必须是奇数，选中项才能居中。
    #[test]
    fn visible_rows_is_odd() {
        assert_eq!(WHEEL_VISIBLE_ROWS % 2, 1, "偶数行数无法让选中项居中");
    }
}
