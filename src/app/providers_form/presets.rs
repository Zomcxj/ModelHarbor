use super::*;

/// 上下文（输入）的常用预设值，单位为 **k（1000）**——与仓库里既有的写法一致
/// （`ModelRow::new()` 的 `262000` 就是 262k）。
///
/// 取 `1000` 而不是 `1024`：厂商文档与网关界面普遍按 1000 报数（「上下文 128k」），
/// 而这些字段是**发给上游的声明**，少声明一点是安全的，多声明会被上游直接拒绝。
/// 所以 `262k` 落到 `262000` 而不是 `262144`、`1024k` 落到 `1024000` 而不是 `1048576`。
pub(super) const CONTEXT_PRESETS: [u32; 7] = [128, 200, 262, 300, 400, 500, 1024];

/// 最大输出（output）的常用预设值，单位同上。
///
/// `131` / `262` 看着不像整数是有意的：对应厂商常声明的 `131072` / `262144`，
/// 按 1000 进制落成 `131000` / `262000`（少声明，安全）。
pub(super) const OUTPUT_PRESETS: [u32; 4] = [32, 64, 131, 262];

/// 预设值的显示文本（`128k`）与实际写入配置的数字（`128000`）。
///
/// 两者**分开**给出：界面写「128k」是因为它一眼能读懂，而配置里必须是整数——
/// 上游不认 `128k` 这种写法。
pub(super) fn preset_label(k: u32) -> String {
    format!("{}k", k)
}

pub(super) fn preset_value(k: u32) -> String {
    (k as u64 * 1000).to_string()
}

/// 当前值在预设表里的下标；不在表里（手填的任意数字、空、非法）返回 `None`。
///
/// 抽成独立函数是为了能单测：它决定下拉收起时显示哪个标签，而「手填的值被误显示成
/// 某个预设」会让用户以为自己选过——这种错只在界面上看得出来，测试里看不见。
pub(super) fn preset_index(value: &str, presets: &[u32]) -> Option<usize> {
    presets
        .iter()
        .position(|k| preset_value(*k) == value.trim())
}

/// 数值字段右侧的预设下拉：选一项就把该值填进字段。
///
/// 用下拉而不是按钮组：上下文有 7 个预设，平铺会把整行挤爆（这一行本来就有
/// id / name / 三个勾选框），而下拉在收起时只占一个控件的宽度。
///
/// 只**填值**、不锁定：填完仍可继续手动编辑。选中项按当前值反查，所以手填的
/// 非预设值不会误显示成某个预设（下拉显示 `选择...`）。
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
    // 只有真正点了某一项才写回：初值是 `None` 而不是 `selected`，
    // 否则「打开下拉又点空白处关掉」也会走一次赋值（虽然写的是同一个数，
    // 但会把 `" 262000 "` 这类带空白的值静默改写，属于用户没要求的改动）。
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
