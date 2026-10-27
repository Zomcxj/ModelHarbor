use eframe::egui;

/// A form field's label: left-aligned, width fitted to its text (capped at `max_width`), hugging the input that follows;
/// over-long truncates with the full text on hover.
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

/// Render a card list in order (adding row spacing between cards).
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

/// A centered empty-state block: a title line, an optional hint line, and an optional action button.
///
/// Empty lists used to leave bare left-aligned weak text on the panel; this gives them one consistent
/// treatment — centered, the title at body size, the hint faint — so an empty page still reads as a
/// designed state rather than a rendering failure. `title_color` emphasizes the title when the empty
/// state signals a problem (e.g. the usage page's warning shade); `None` keeps it quiet. Returns
/// whether the action button was clicked.
pub fn empty_state(
    ui: &mut egui::Ui,
    title: &str,
    hint: &str,
    title_color: Option<egui::Color32>,
    action: Option<&str>,
) -> bool {
    let weak = ui.visuals().weak_text_color();
    let title_color = title_color.unwrap_or(weak);
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(
            crate::theme::SPACE_3 as i8,
            crate::theme::SPACE_5 as i8,
        ))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .size(crate::theme::TEXT_BODY)
                        .color(title_color),
                );
                if !hint.is_empty() {
                    ui.add_space(crate::theme::SPACE_1);
                    ui.label(
                        egui::RichText::new(hint)
                            .size(crate::theme::TEXT_CAPTION)
                            .color(weak),
                    );
                }
                action
                    .map(|label| {
                        ui.add_space(crate::theme::SPACE_3);
                        ui.button(label).clicked()
                    })
                    .unwrap_or(false)
            })
            .inner
        })
        .inner
}

/// The key input box: masked (dots) when `show` is false, content retained.
/// Visibility toggles via the toolbar's global "show keys / hide keys" button.
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

/// A numeric text edit: non-empty unparseable content highlights red with a hover note.
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

/// The toggle's geometry: track size and knob diameter.
pub const TOGGLE_WIDTH: f32 = 34.0;
pub const TOGGLE_HEIGHT: f32 = 18.0;
/// The gap between knob and track edge.
pub const TOGGLE_INSET: f32 = 2.0;
/// Knob diameter = track height − 2×padding.
pub const TOGGLE_KNOB: f32 = TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET;

/// The geometry constants must be self-consistent: the knob smaller than the track, the track wider than tall.
const _: () = {
    assert!(TOGGLE_KNOB < TOGGLE_HEIGHT, "滑块要小于轨道高，留出描边");
    assert!(TOGGLE_WIDTH > TOGGLE_HEIGHT, "轨道应横向长于纵向");
};

/// The toggle knob's center: interpolated by progress — `t = 0` hugs left, `t = 1` hugs right, vertically centered.
pub fn toggle_knob_center_at(rect: egui::Rect, t: f32) -> egui::Pos2 {
    let radius = TOGGLE_KNOB / 2.0;
    let left = rect.left() + TOGGLE_INSET + radius;
    let right = rect.right() - TOGGLE_INSET - radius;
    let t = t.clamp(0.0, 1.0);
    egui::pos2(left + (right - left) * t, rect.center().y)
}

/// The toggle knob's center (two-state version: `on` is progress 1, `off` is progress 0).
pub fn toggle_knob_center(rect: egui::Rect, on: bool) -> egui::Pos2 {
    toggle_knob_center_at(rect, on as u8 as f32)
}

/// The toggle: a "click to flip" control expressed by track and knob, with the whole track as the click hot zone.
///
/// `id` is a stable value from the caller (like "card key + field name"); animation state hangs on it.
///
/// Visual rules: state is expressed by fill + knob position; the radius takes the theme's widget radius;
/// no hover tip.
pub fn toggle_switch(ui: &mut egui::Ui, id: egui::Id, on: &mut bool) -> egui::Response {
    let desired = egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT);
    let (rect, mut resp) = ui.allocate_exact_size(desired, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    // Take the progress first: even when this frame paints nothing (clipped out), the animation keeps advancing.
    let t = crate::motion::toggle_progress(ui.ctx(), id, *on);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&resp);
        let colors = crate::theme::semantics(ui);
        let radius = TOGGLE_HEIGHT / 2.0;
        // Track: green when on, neutral gray when off. Hover brightens each a step, then interpolates by progress.
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
        // Knob: a white dot with a thin border.
        let center = toggle_knob_center_at(rect, t);
        ui.painter()
            .circle_filled(center, TOGGLE_KNOB / 2.0, egui::Color32::WHITE);
        ui.painter().circle_stroke(
            center,
            TOGGLE_KNOB / 2.0,
            // The width literal carries an `_f32` suffix to avoid `float_literal_f32_fallback`.
            egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }
    resp
}
