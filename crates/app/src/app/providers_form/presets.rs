use super::*;

/// Common preset values for context (input), in units of **k (1000)**.
///
/// Values are decimal: `262k` → `262000`, `1024k` → `1024000`.
pub(super) const CONTEXT_PRESETS: [u32; 7] = [128, 200, 262, 300, 400, 500, 1024];

/// Common preset values for max output, same units (k = 1000).
///
/// `131` / `262` match vendors' stated `131072` / `262144`, landing as `131000` / `262000` decimal.
pub(super) const OUTPUT_PRESETS: [u32; 4] = [32, 64, 131, 262];

/// A preset value's display text (`128k`).
pub(super) fn preset_label(k: u32) -> String {
    format!("{}k", k)
}

/// A preset value's actual written value (`128000`).
pub(super) fn preset_value(k: u32) -> String {
    (k as u64 * 1000).to_string()
}

/// The current value's index in the preset table; `None` when absent (hand-typed numbers, empty, invalid).
pub(super) fn preset_index(value: &str, presets: &[u32]) -> Option<usize> {
    presets
        .iter()
        .position(|k| preset_value(*k) == value.trim())
}

/// The preset dropdown beside a numeric field: picking one fills the value in.
///
/// Only **fills** — never locks; the field stays hand-editable after. The selected item reverse-looks-up
/// from the current value; hand-typed non-preset values show the unselected placeholder.
///
/// With `sync_tooltip` set, the dropdown's first item adds "sync": clicking doesn't change this field but
/// returns `true`, letting the caller brush the value onto every model on the page.
pub(super) fn preset_combo(
    ui: &mut egui::Ui,
    value: &mut String,
    presets: &[u32],
    id_salt: &str,
    tooltip: &str,
    sync_tooltip: Option<&str>,
) -> bool {
    // When the current value exactly equals a preset, the dropdown shows that item; otherwise the placeholder.
    let selected = preset_index(value, presets);
    let text = match selected {
        Some(i) => preset_label(presets[i]),
        None => "选择...".to_string(),
    };
    // Write back only on a real pick: initial `None`, so an unselection never triggers an assignment.
    let mut picked: Option<usize> = None;
    let mut sync_clicked = false;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(text)
        .width(72.0)
        .show_ui(ui, |ui| {
            // The first item, "sync": brush this model's field value onto every model on the page.
            if let Some(st) = sync_tooltip {
                if ui
                    .selectable_label(false, "同步")
                    .on_hover_text(st)
                    .clicked()
                {
                    sync_clicked = true;
                }
                ui.separator();
            }
            for (i, k) in presets.iter().enumerate() {
                if ui
                    .selectable_label(selected == Some(i), preset_label(*k))
                    .clicked()
                {
                    picked = Some(i);
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    if let Some(i) = picked {
        *value = preset_value(presets[i]);
    }
    sync_clicked
}
