use super::*;

/// The protocol-package dropdown (opencode only); `id_salt` distinguishes a page's multiple form instances.
///
/// Options uniformly display the package name's **last segment** (`openai-compatible` etc.): v1's
/// `@ai-sdk/*` and v2's `@opencode/ai/providers/*` agree there, so one vocabulary serves both. On selection,
/// the full name for the page's version assembles per `p.schema`; at write time `to_value_as` translates
/// once more per the target file's version as a backstop.
pub(super) fn provider_npm_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    flags: &ProviderFormFlags,
    id_salt: &str,
    empty_has_label: bool,
) {
    /// Official protocol packages' last segments (without the `@ai-sdk/` / `@opencode/ai/providers/` prefixes).
    /// One-to-one with core's `PACKAGE_VERSION_PAIRS` (mistral appears only in the translation table, not in
    /// the dropdown, matching the pre-change behavior).
    const PACKAGE_SUFFIXES: [&str; 4] = ["openai", "anthropic", "google", "openai-compatible"];
    field_label(ui, 120.0, flags.package_label);
    // The current value compares by last segment: v1 / v2 full names differ but segments agree, so both
    // versions can be selected. Uncollected custom packages aren't in the vocabulary — their full names
    // display as-is, with nothing preselected.
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
    // `None` = nothing selected; `Some(None)` = "(none)" was picked.
    let mut picked: Option<Option<usize>> = selected.map(Some);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if current.is_empty() {
            "选择协议包...".to_string()
        } else {
            current_suffix.clone()
        })
        .width(220.0)
        .show_ui(ui, |ui| {
            // The first item "(none)" = no package specified (the default compatibility layer applies at write).
            if empty_has_label && ui.selectable_label(current.is_empty(), "(空)").clicked() {
                picked = Some(None);
            }
            for (i, suffix) in PACKAGE_SUFFIXES.iter().enumerate() {
                if ui.selectable_label(selected == Some(i), *suffix).clicked() {
                    picked = Some(Some(i));
                }
            }
        });
    // Assemble the full name per the current page's version; "(none)" clears; untouched values don't write back (preserving custom package names).
    if let Some(choice) = picked {
        p.npm = match choice {
            Some(i) => crate::model::package_full_name(PACKAGE_SUFFIXES[i], p.schema),
            None => String::new(),
        };
    }
}

/// The "(none)" label in the api / npm dropdowns: no protocol specified.
pub(super) const EMPTY_API_LABEL: &str = "(空)";

/// The official presets dropdown: fills only key / baseUrl / protocol (npm on the opencode page) — never
/// keys, never the model list.
///
/// The first item "(custom / don't apply)" changes nothing; after applying, every field stays hand-editable.
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

/// The api dropdown: omp's official 9 values / pi KnownApi's 10; the first "(none)" matches the opencode page's npm empty option.
pub(super) fn provider_api_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    page: ConfigFormat,
    id_salt: &str,
) {
    // Each backend's own protocol vocabulary: omp 9 / pi 10 / ZCode 3 / WorkBuddy 3 /
    // QwenCode 3 / KimiCode 6 (verbatim, the `type` field's values).
    let options: &[&str] = match page {
        ConfigFormat::OhMyPi => &convert::OMP_APIS,
        ConfigFormat::ZCode => &convert::ZCODE_APIS,
        ConfigFormat::WorkBuddy => &convert::WORKBUDDY_APIS,
        ConfigFormat::QwenCode => &convert::QWEN_APIS,
        ConfigFormat::KimiCode => &convert::KIMI_APIS,
        _ => &convert::PI_APIS,
    };
    // ZCode's api.type vocabulary differs from the internal representation by one `chat`; both display and write-back convert.
    let zcode = page == ConfigFormat::ZCode;
    let to_display = |api: &str| {
        if zcode {
            convert::api_to_zcode_api(api)
        } else {
            api.to_string()
        }
    };
    // "(none)" = protocol unset. Judged by npm / pi_api / raw.api all being empty; the displayed value goes
    // through effective_api() uniformly, sharing the disk-write and latency-test accounting.
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
                // Selection compares by the converted value: the ZCode page stores openai-completions while the option
                // text reads openai-chat-completions.
                let shown = to_display(api);
                if ui
                    .selectable_label(explicit && to_display(&current) == shown, shown)
                    .clicked()
                {
                    // Write back to the internal representation: ZCode's chat value maps back to the pi vocabulary first.
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

/// Reasoning-tier multi-select: buttons expand, checks write back as comma-separated text.
/// With `normalize` true, write-back follows the canonical tier order.
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
