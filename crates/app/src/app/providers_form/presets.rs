use super::*;

/// 上下文（输入）的常用预设值，单位为 **k（1000）**。
///
/// 数值按 1000 进制：`262k` → `262000`，`1024k` → `1024000`。
pub(super) const CONTEXT_PRESETS: [u32; 7] = [128, 200, 262, 300, 400, 500, 1024];

/// 最大输出（output）的常用预设值，单位同上（k = 1000）。
///
/// `131` / `262` 对应厂商声明的 `131072` / `262144`，按 1000 进制落成
/// `131000` / `262000`。
pub(super) const OUTPUT_PRESETS: [u32; 4] = [32, 64, 131, 262];

/// 预设值的显示文本（`128k`）。
pub(super) fn preset_label(k: u32) -> String {
    format!("{}k", k)
}

/// 预设值的实际写入值（`128000`）。
pub(super) fn preset_value(k: u32) -> String {
    (k as u64 * 1000).to_string()
}

/// 当前值在预设表里的下标；不在表里（手填的任意数字、空、非法）返回 `None`。
pub(super) fn preset_index(value: &str, presets: &[u32]) -> Option<usize> {
    presets
        .iter()
        .position(|k| preset_value(*k) == value.trim())
}

/// 数值字段右侧的预设下拉：选一项就把该值填进字段。
///
/// 只**填值**、不锁定，填完仍可继续手动编辑。选中项按当前值反查，
/// 手填的非预设值显示为 `选择...`。
pub(super) fn preset_combo(
    ui: &mut egui::Ui,
    value: &mut String,
    presets: &[u32],
    id_salt: &str,
    tooltip: &str,
) {
    // 当前值恰好等于某个预设时，把下拉显示成那一项；否则显示占位文案。
    let selected = preset_index(value, presets);
    let text = match selected {
        Some(i) => preset_label(presets[i]),
        None => "选择...".to_string(),
    };
    // 只有真正点了某一项才写回：初值取 `None`，避免未选也走一次赋值。
    let mut picked: Option<usize> = None;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(text)
        .width(72.0)
        .show_ui(ui, |ui| {
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
}
