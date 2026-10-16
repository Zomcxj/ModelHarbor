use super::*;

/// 协议包下拉（opencode 专用）；`id_salt` 区分同一页面内的多个表单实例。
///
/// 选项统一显示包名**末段**（`openai-compatible` 等）：v1 的 `@ai-sdk/*` 与 v2 的
/// `@opencode/ai/providers/*` 末段一致，一套词表即可。选中后按 `p.schema`
/// 拼回当前页版本的全名；落盘时 `to_value_as` 还会再按目标文件版本兜底翻译。
pub(super) fn provider_npm_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    flags: &ProviderFormFlags,
    id_salt: &str,
    empty_has_label: bool,
) {
    /// 官方协议包的末段（不含 `@ai-sdk/` / `@opencode/ai/providers/` 前缀）。
    /// 与 core 的 `PACKAGE_VERSION_PAIRS` 一一对应（mistral 仅出现在译表里，
    /// 下拉不列，与改动前一致）。
    const PACKAGE_SUFFIXES: [&str; 4] = ["openai", "anthropic", "google", "openai-compatible"];
    field_label(ui, 120.0, flags.package_label);
    // 当前值按末段比较：v1 / v2 全名不同但末段相同，两版都能选中。
    // 未收录的自定义包不在词表里，原样显示全名且不预选任何项。
    let known = PACKAGE_SUFFIXES.contains(&crate::model::package_suffix(&p.npm));
    let current = p.npm.clone();
    let current_suffix = crate::model::package_suffix(&current).to_string();
    let selected = if known {
        PACKAGE_SUFFIXES
            .iter()
            .position(|n| *n == current_suffix.as_str())
    } else {
        None
    };
    // `None` = 未选中任何项；`Some(None)` = 选了「(空)」。
    let mut picked: Option<Option<usize>> = selected.map(Some);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if current.is_empty() {
            "选择协议包...".to_string()
        } else {
            current_suffix.clone()
        })
        .width(220.0)
        .show_ui(ui, |ui| {
            // 首项「(空)」= 不指定包（写盘时走默认兼容层）。
            if empty_has_label && ui.selectable_label(current.is_empty(), "(空)").clicked() {
                picked = Some(None);
            }
            for (i, suffix) in PACKAGE_SUFFIXES.iter().enumerate() {
                if ui.selectable_label(selected == Some(i), *suffix).clicked() {
                    picked = Some(Some(i));
                }
            }
        });
    // 按当前页版本拼回全名；选「(空)」则清空；未动过不写回（保留自定义包名）。
    if let Some(choice) = picked {
        p.npm = match choice {
            Some(i) => crate::model::package_full_name(PACKAGE_SUFFIXES[i], p.schema),
            None => String::new(),
        };
    }
}

/// api / npm 下拉里的「(空)」标签：表示未指定协议。
pub(super) const EMPTY_API_LABEL: &str = "(空)";

/// 官方预设下拉：只填 key / baseUrl / 协议（opencode 页填 npm），不写密钥、不动模型列表。
///
/// 首项「(自定义 / 不套用)」不修改任何字段；套用后所有字段仍可继续手改。
pub(super) fn provider_preset_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    dialect: crate::presets::PresetDialect,
    id_salt: &str,
) {
    field_label(ui, 120.0, "官方预设");
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text("(自定义 / 不套用)")
        .width(240.0)
        .show_ui(ui, |ui| {
            ui.label(
                egui::RichText::new(
                    "套用后仅覆盖 key / baseUrl / 协议，其余字段可照旧手填；\n\
                     不写密钥、不改动模型列表。第三方 / 中转站 / 自建端点请直接在下方手填",
                )
                .small()
                .weak(),
            );
            ui.separator();
            for preset in crate::presets::PRESETS {
                let text = format!("{}  ·  {}", preset.label, preset.key);
                if ui.selectable_label(false, text).clicked() {
                    crate::presets::apply(p, preset, dialect);
                }
            }
        });
}

/// api 下拉：omp 官方 9 值 / pi KnownApi 10 值；首项「(空)」与 opencode 页 npm 的空选项同义。
pub(super) fn provider_api_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    page: ConfigFormat,
    id_salt: &str,
) {
    // 每个后端自己的协议词表：omp 9 值 / pi 10 值 / ZCode 3 值 / WorkBuddy 3 值 /
    // QwenCode 3 值 / KimiCode 6 值（逐字就是 `type` 字段）。
    let options: &[&str] = match page {
        ConfigFormat::OhMyPi => &convert::OMP_APIS,
        ConfigFormat::ZCode => &convert::ZCODE_APIS,
        ConfigFormat::WorkBuddy => &convert::WORKBUDDY_APIS,
        ConfigFormat::QwenCode => &convert::QWEN_APIS,
        ConfigFormat::KimiCode => &convert::KIMI_APIS,
        _ => &convert::PI_APIS,
    };
    // ZCode 的 api.type 词表与内部表示差一个 `chat`，显示与写回都要转换。
    let zcode = page == ConfigFormat::ZCode;
    let to_display = |api: &str| {
        if zcode {
            convert::api_to_zcode_api(api)
        } else {
            api.to_string()
        }
    };
    // 「(空)」= 未指定协议。以 npm / pi_api / raw.api 是否都为空判定，
    // 显示值统一走 effective_api()，与写盘、延迟测试同口径。
    let explicit = p.has_explicit_api();
    let current = p.effective_api();
    field_label(ui, 120.0, "api");
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if explicit {
            to_display(&current)
        } else {
            EMPTY_API_LABEL.to_string()
        })
        .width(180.0)
        .show_ui(ui, |ui| {
            ui.label(
                egui::RichText::new("(空) = 不指定协议：写盘时按兼容层 openai-completions 处理")
                    .small()
                    .weak(),
            );
            ui.separator();
            if ui.selectable_label(!explicit, EMPTY_API_LABEL).clicked() {
                p.clear_api();
            }
            for &api in options {
                // 选中态按转换后的值比较：ZCode 页存的是 openai-completions，
                // 而选项文本是 openai-chat-completions。
                let shown = to_display(api);
                if ui
                    .selectable_label(explicit && to_display(&current) == shown, shown)
                    .clicked()
                {
                    // 写回内部表示：ZCode 的 chat 值先映射回 pi 词表。
                    p.pi_api = if zcode {
                        convert::zcode_api_to_api(api)
                    } else {
                        api.to_string()
                    };
                    p.npm = convert::api_to_npm(api);
                }
            }
        });
}

/// 思考档位多选：按钮展开、勾选写回逗号分隔文本。
/// `normalize` 为 true 时按规范档位顺序写回。
pub(super) fn variant_selector(
    ui: &mut egui::Ui,
    variants: &mut String,
    names: &[&'static str],
    open_key: String,
    open_set: &mut HashSet<String>,
    normalize: bool,
) {
    let current = variants.clone();
    let mut selected: Vec<String> = current
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let display = if selected.is_empty() {
        "选择...".to_string()
    } else {
        current.clone()
    };
    let is_open = open_set.contains(&open_key);
    if ui.button(display).clicked() {
        if is_open {
            open_set.remove(&open_key);
        } else {
            open_set.insert(open_key.clone());
        }
    }
    if !is_open {
        return;
    }
    for name in names {
        let mut checked = selected.iter().any(|s| s == name);
        if ui.checkbox(&mut checked, *name).changed() {
            if checked {
                if !selected.iter().any(|s| s == name) {
                    selected.push((*name).to_string());
                }
            } else {
                selected.retain(|s| s != name);
            }
            *variants = if normalize {
                crate::model::ordered_variants(selected.iter().map(String::as_str)).join(", ")
            } else {
                selected.join(", ")
            };
        }
    }
}
