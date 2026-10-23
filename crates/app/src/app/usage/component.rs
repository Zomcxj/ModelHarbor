//! The usage view's component primitives: section titles, card containers, scrollable table bodies.
//!
//! The modules (overview card / trend chart / heatmap / four tables) share one set of shells, keeping
//! padding and title hierarchy consistent; changing here changes every component together.

use eframe::egui;

/// A component section title (`TEXT_TITLE`, bold).
pub(in crate::app) fn heading(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(crate::theme::TEXT_TITLE)
            .strong(),
    );
}

/// A component card: uniform padding + outer border, filling the available width.
///
/// Every module's content paints inside; visually these are the "cards".
pub(in crate::app) fn card<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let visuals = ui.visuals().clone();
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
        // The border and radius follow the theme's non-interactive widgets (changing with the theme).
        .stroke(egui::Stroke::new(
            1.0_f32,
            visuals.widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(visuals.widgets.noninteractive.corner_radius)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
}

/// Row spacing: row height 21 + line gap 4 (the user-set spacing).
pub(in crate::app) const ROW_PITCH: f32 = 25.0;

/// The segmented control group's padding, also the gap between its buttons.
///
/// Button gaps tighten to this value (tighter than the global `item_spacing.x`): they belong to one
/// control, and a wide gap reads as several independent buttons.
pub(in crate::app) const CHIP_GROUP_PAD: f32 = crate::theme::SPACE_1;

/// The total width of `chips` equal-width buttons wrapped in a segmented control group.
///
/// The status line reserves this fixed width for the right-side button group, giving the rest to the left
/// text. Shares [`CHIP_GROUP_PAD`] with [`chip_group`], so changing button widths never skews the reservation.
pub(in crate::app) fn chip_group_width(chips: usize, button_width: f32) -> f32 {
    let n = chips as f32;
    n * button_width + (n - 1.0).max(0.0) * CHIP_GROUP_PAD + 2.0 * CHIP_GROUP_PAD
}

/// A segmented control group: wraps a row of mutually exclusive toggle buttons into one component
/// (user request: the range switch should be a wrapped component).
///
/// Its fill separates it from cards: cards are "raised" (border + panel color), this is "sunken"
/// (very dark fill + the same border), reading as a set of switches embedded in the status line.
///
/// Buttons lay out horizontally with gaps tightened to [`CHIP_GROUP_PAD`]: they belong to one control,
/// and the global `item_spacing.x` gap would read them as independent buttons. `body` only adds buttons
/// to `ui` one by one; the group's width is budgeted by [`chip_group_width`].
pub(in crate::app) fn chip_group<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let visuals = ui.visuals().clone();
    egui::Frame::NONE
        .fill(visuals.extreme_bg_color)
        .stroke(egui::Stroke::new(
            1.0_f32,
            visuals.widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(visuals.widgets.noninteractive.corner_radius)
        .inner_margin(egui::Margin::same(CHIP_GROUP_PAD as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = CHIP_GROUP_PAD;
                body(ui)
            })
            .inner
        })
        .inner
}

/// A table body's max visible height: 5 rows × 25px.
///
/// More rows scroll inside the component, never lengthening the page; fewer than 5 shrink to the actual
/// count (`auto_shrink`), leaving no blank rows.
pub(in crate::app) const BODY_MAX_HEIGHT: f32 = 5.0 * ROW_PITCH;

/// The session table body's height: three times an ordinary table (user request).
///
/// The session table takes a full row with the most columns — the main place to inspect entries one by
/// one; 5 rows a screen is too few.
pub(in crate::app) const SESSION_BODY_HEIGHT: f32 = BODY_MAX_HEIGHT * 3.0;

/// Table body height: rows × spacing, capped at `cap`.
///
/// Height locks instead of fitting content, for two reasons:
///
/// * When the side-by-side tables' row counts differ, budgeting by the taller side puts both cards'
///   bottoms on one line (user request: the Agent card matches the Models card height).
/// * Without a cap, `ScrollArea` takes the page's remaining height as its viewport: a slightly short
///   window hides the session table's 3x rows, and the user-requested "visible height is 3x an ordinary
///   table" becomes unreachable.
///
/// At least one row: a 0 height squashes the viewport to a line where even the header won't fit.
pub(in crate::app) fn body_height(rows: usize, cap: f32) -> f32 {
    (rows.max(1) as f32 * ROW_PITCH).clamp(ROW_PITCH, cap.max(ROW_PITCH))
}

/// A scrollable table body: content taller than `height` gets an internal vertical scrollbar.
///
/// The viewport height is fixed at `height` (`min_scrolled_height` + vertical shrinking off): the shorter
/// side leaves blank space at its bottom, buying the side-by-side cards' aligned bottoms.
///
/// `id` feeds egui's scroll-state memory (one per table; switching sort / filter never loses position).
pub(in crate::app) fn scroll_body<R>(
    ui: &mut egui::Ui,
    id: &str,
    height: f32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .max_height(height)
        .min_scrolled_height(height)
        .show(ui, body)
        .inner
}
