use eframe::egui;

pub fn card_frame<R>(
    ui: &mut egui::Ui,
    open: bool,
    highlight: u8,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let corner = ui.visuals().widgets.noninteractive.corner_radius;
    let fill = if open {
        ui.visuals().faint_bg_color
    } else {
        ui.visuals().extreme_bg_color
    };
    let (stroke_color, stroke_width) = match highlight {
        1 => (egui::Color32::from_rgb(255, 180, 50), 2.0), // source: orange
        2 => (egui::Color32::from_rgb(100, 200, 100), 2.0), // target: green
        _ => (ui.visuals().widgets.noninteractive.bg_stroke.color, 1.0),
    };
    egui::Frame::NONE
        .fill(fill)
        .corner_radius(corner)
        .stroke(egui::Stroke::new(stroke_width, stroke_color))
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 2.0);
            add(ui);
        })
        .response
}

/// 表单字段标签：**左对齐**且宽度按文本内容自适应（上限 `max_width`），
/// 既不在左侧留空白，也紧贴其后的输入框；超长时截断并悬停显示完整文本。
///
/// 注意：不能用 `add_sized` —— 它内部是 `Layout::centered_and_justified`，
/// 会把文本居中在定宽槽内。
pub fn field_label(ui: &mut egui::Ui, max_width: f32, text: impl Into<String>) -> egui::Response {
    let text = text.into();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_width = ui
        .painter()
        .layout_no_wrap(text.clone(), font, egui::Color32::WHITE)
        .size()
        .x;
    let width = text_width.min(max_width);
    let label = egui::Label::new(egui::RichText::new(text.as_str()).weak()).truncate();
    let resp = ui
        .allocate_ui_with_layout(
            egui::vec2(width, 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| ui.add(label),
        )
        .inner;
    resp.on_hover_text(text)
}

/// 依次渲染卡片列表（每卡片之间附加行间距）。
pub fn card_list(
    ui: &mut egui::Ui,
    keys: &[usize],
    row_gap: f32,
    mut f: impl FnMut(&mut egui::Ui, usize),
) {
    for &idx in keys {
        f(ui, idx);
        ui.add_space(row_gap);
    }
}

pub struct DragHandle;

impl egui::Widget for DragHandle {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let button = egui::Button::new("")
            .frame(false)
            .sense(egui::Sense::click_and_drag())
            .min_size(egui::vec2(14.0, 18.0));
        let resp = ui.add(button);
        let painter = ui.painter();
        let rect = resp.rect;
        let active = resp.hovered() || resp.dragged();
        let color = if active {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        for row in 0..3 {
            for col in 0..2 {
                let c = egui::pos2(
                    rect.left() + 2.5 + col as f32 * 5.0,
                    rect.top() + 3.0 + row as f32 * 5.0,
                );
                painter.circle_filled(c, if active { 1.5 } else { 1.0 }, color);
            }
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp
    }
}

pub fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= items.len() || to >= items.len() {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

/// 把一张卡片的拖拽落点并入本帧的聚合结果。
///
/// 语义是「只增不减」：`found` 为 `None` 时必须保留 `acc` 中已有的落点。卡片是逐张渲染的，
/// 若每张卡片各自写入落点，后面渲染的卡片会把前面命中的落点清成 `None`，被拖到的卡片就拿不到
/// 落点边框（模型卡片的绿色边框曾因此消失）。
pub fn merge_drag_target(acc: &mut Option<String>, found: Option<String>) {
    if acc.is_none() {
        *acc = found;
    }
}

/// 密钥输入框：`show` 为 false 时掩码显示（圆点），内容仍保留。
/// 显隐切换由工具栏「显示密钥/隐藏密钥」全局按钮控制。
pub fn secret_text_edit(
    ui: &mut egui::Ui,
    value: &mut String,
    show: bool,
    width: f32,
    hint: &str,
) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .password(!show)
            .desired_width(width)
            .hint_text(hint),
    )
}

/// 数字文本编辑框：内容非空且无法解析为数字时红色高亮并悬停提示。
/// （保存时非法数字字段会被丢弃——这里让用户在丢弃前就看到。）
pub fn numeric_text_edit(
    ui: &mut egui::Ui,
    s: &mut String,
    width: f32,
    hint: &str,
) -> egui::Response {
    let valid = s.trim().is_empty() || crate::util::parse_number_text(s).is_some();
    let edit = egui::TextEdit::singleline(s)
        .desired_width(width)
        .hint_text(hint);
    if valid {
        ui.add(edit)
    } else {
        ui.add(edit.text_color(crate::theme::semantics(ui).err))
            .on_hover_text("无效数字：保存时该字段将被忽略")
    }
}

#[cfg(test)]
mod tests {
    use super::merge_drag_target;

    #[test]
    fn merge_drag_target_keeps_earlier_hit() {
        let mut acc = None;
        // 先渲染的卡片未命中：聚合结果保持为空
        merge_drag_target(&mut acc, None);
        assert_eq!(acc, None);
        // 命中落点
        merge_drag_target(&mut acc, Some("p\u{1f}m".to_string()));
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // 后面还有卡片展开且未命中：不能把已命中的落点清掉
        merge_drag_target(&mut acc, None);
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // 已命中时后续命中不覆盖：保持首个落点，行为稳定
        merge_drag_target(&mut acc, Some("other".to_string()));
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
    }

    #[test]
    fn merge_drag_target_stays_none_without_hit() {
        let mut acc = None;
        merge_drag_target(&mut acc, None);
        merge_drag_target(&mut acc, None);
        assert_eq!(acc, None);
    }
}
