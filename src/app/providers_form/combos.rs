use super::*;

/// npm 包下拉（opencode 专用）；`id_salt` 区分同一页面内的多个表单实例。
pub(super) fn provider_npm_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    id_salt: &str,
    empty_has_label: bool,
) {
    pub(super) const NPM_OPTIONS: [&str; 5] = [
        "",
        "@ai-sdk/openai",
        "@ai-sdk/anthropic",
        "@ai-sdk/google",
        "@ai-sdk/openai-compatible",
    ];
    field_label(ui, 120.0, "npm");
    let current = p.npm.clone();
    let mut selected = NPM_OPTIONS.iter().position(|n| *n == current.as_str());
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if current.is_empty() {
            "选择 npm 包..."
        } else {
            &current
        })
        .width(220.0)
        .show_ui(ui, |ui| {
            for (i, npm) in NPM_OPTIONS.iter().enumerate() {
                let label = if npm.is_empty() && empty_has_label {
                    "(空)"
                } else {
                    *npm
                };
                if ui.selectable_label(selected == Some(i), label).clicked() {
                    selected = Some(i);
                }
            }
        });
    if let Some(i) = selected {
        p.npm = NPM_OPTIONS[i].to_string();
    }
}

/// api / npm 下拉里的「(空)」标签：表示未指定协议。
pub(super) const EMPTY_API_LABEL: &str = "(空)";

/// 官方预设下拉：只填 key / baseUrl / 协议（opencode 页填 npm），不写密钥、不动模型列表。
///
/// 首项就是「(自定义 / 不套用)」——不选预设时表单与以前完全一样，第三方 / 中转站 / 自建
/// 端点照旧手填；套用后所有字段仍可继续手改。
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
    // 每个后端自己的协议词表：omp 9 值 / pi 10 值 / ZCode 3 值（多一个 chat）/
    // WorkBuddy 3 值（协议落到 URL 后缀，见 workbuddy 后端）/
    // QwenCode 3 值（协议落到 pid + wireApi，见 qwen_code 后端）/
    // KimiCode 6 值（逐字就是 `type` 字段，见 kimi_code 后端）。
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
    // 「(空)」= 未指定协议。协议在 opencode 系 / pi / omp / DSH 之间共用一份数据，
    // 故以 npm / pi_api / raw.api 是否都为空判定，显示值统一走 effective_api()，
    // 与写盘、延迟测试同口径。
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
                // 选中态按「转换后」的值比较：ZCode 页存的是 openai-completions，
                // 但选项文本是 openai-chat-completions。
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
/// `normalize` 为 true 时按规范档位顺序写回，避免重新勾选后被追加到末尾。
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
