use eframe::egui;

/// 表单字段标签：左对齐且宽度按文本内容自适应（上限 `max_width`），紧贴其后的输入框；
/// 超长时截断并悬停显示完整文本。
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

/// 滑动开关（toggle）的几何：轨道尺寸与滑块直径。
pub const TOGGLE_WIDTH: f32 = 34.0;
pub const TOGGLE_HEIGHT: f32 = 18.0;
/// 滑块与轨道边缘的间距。
pub const TOGGLE_INSET: f32 = 2.0;
/// 滑块直径 = 轨道高 − 2×内边距。
pub const TOGGLE_KNOB: f32 = TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET;

/// 几何常量必须自洽：滑块小于轨道，轨道横向长于纵向。
const _: () = {
    assert!(TOGGLE_KNOB < TOGGLE_HEIGHT, "滑块要小于轨道高，留出描边");
    assert!(TOGGLE_WIDTH > TOGGLE_HEIGHT, "轨道应横向长于纵向");
};

/// 滑动开关的滑块圆心：按进度插值，`t = 0` 贴左、`t = 1` 贴右，垂直居中。
pub fn toggle_knob_center_at(rect: egui::Rect, t: f32) -> egui::Pos2 {
    let radius = TOGGLE_KNOB / 2.0;
    let left = rect.left() + TOGGLE_INSET + radius;
    let right = rect.right() - TOGGLE_INSET - radius;
    let t = t.clamp(0.0, 1.0);
    egui::pos2(left + (right - left) * t, rect.center().y)
}

/// 滑动开关的滑块圆心（两态版本：`on` 即进度 1，`off` 即进度 0）。
pub fn toggle_knob_center(rect: egui::Rect, on: bool) -> egui::Pos2 {
    toggle_knob_center_at(rect, on as u8 as f32)
}

/// 滑动开关：轨道与滑块表达的「点击即切换」控件，整块轨道都是点击热区。
///
/// `id` 由调用方给出稳定值（如「卡片键 + 字段名」），动画状态挂在它上面。
///
/// 视觉规则：状态靠填充 + 滑块位置表达；圆角取主题控件圆角；不挂悬停提示。
pub fn toggle_switch(ui: &mut egui::Ui, id: egui::Id, on: &mut bool) -> egui::Response {
    let desired = egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT);
    let (rect, mut resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    // 进度先取：即使本帧不画（被裁剪），动画也继续推进。
    let t = crate::motion::toggle_progress(ui.ctx(), id, *on);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&resp);
        let colors = crate::theme::semantics(ui);
        let radius = TOGGLE_HEIGHT / 2.0;
        // 轨道：开=绿色，关=中性灰。悬停各提亮一档，再按进度插值。
        let off = if resp.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        let on_fill = if resp.hovered() {
            colors.ok.gamma_multiply(1.15)
        } else {
            colors.ok
        };
        let track = crate::motion::lerp_color(off, on_fill, t);
        ui.painter().rect(
            rect,
            radius,
            track,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        // 滑块：白色圆点，带一圈细描边。
        let center = toggle_knob_center_at(rect, t);
        ui.painter()
            .circle_filled(center, TOGGLE_KNOB / 2.0, egui::Color32::WHITE);
        ui.painter().circle_stroke(
            center,
            TOGGLE_KNOB / 2.0,
            // 宽度带 `_f32` 后缀，避免 `float_literal_f32_fallback`。
            egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }
    resp
}
