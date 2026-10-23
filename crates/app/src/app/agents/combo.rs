use crate::app::fetch::FreeModelsState;
use crate::format::ConfigFormat;
use crate::model::ProviderRow;
use eframe::egui;
use std::collections::{HashMap, HashSet};

/// The Agents `model` dropdown's candidates, ordered "own gateway → user-configured providers → current value".
///
/// Three segments concatenated in order:
///
/// 1. `gateway`: this page gateway's models (preferred first — see [`crate::opencode_models::gateway_models`]);
/// 2. user-configured providers' models (provider order, then model order);
/// 3. the current value (when neither segment above holds it).
///
/// The current value is always kept, sorted last.
pub(super) fn model_options(
    providers: &[ProviderRow],
    gateway: &[String],
    current: &str,
) -> Vec<String> {
    let mut options: Vec<String> = Vec::new();
    options.extend(gateway.iter().cloned());
    options.extend(
        providers
            .iter()
            .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.key, m.id))),
    );
    let current = current.trim();
    if !current.is_empty() && !options.iter().any(|option| option == current) {
        options.push(current.to_string());
    }
    // Dedup only, preserving original order.
    let mut seen = HashSet::new();
    options.retain(|option| seen.insert(option.clone()));
    options
}

/// The free-model status note and "refresh" button in the Agents title row (right of "expand all cards").
///
/// Refreshes this page backend's built-in free-model list — the page has exactly one.
///
/// Returns `true` when the user clicked refresh; the caller then starts the background fetch.
///
/// Fetching / failed / empty-list each carry their own wording, visible in place.
pub(super) fn model_options_hint(
    ui: &mut egui::Ui,
    backend: &str,
    free_count: usize,
    error: Option<&str>,
    fetching: bool,
) -> bool {
    let mut clicked = false;
    if fetching {
        ui.add(egui::Spinner::new().size(14.0));
        ui.label(egui::RichText::new("正在获取免费模型…").small().weak());
    } else {
        if let Some(err) = error {
            ui.label(
                egui::RichText::new("免费模型获取失败")
                    .small()
                    .color(crate::theme::semantics(ui).err),
            )
            .on_hover_text(err);
        } else if free_count == 0 {
            ui.label(
                egui::RichText::new(format!("未获取到 {} 免费模型", backend))
                    .small()
                    .weak(),
            );
        } else {
            ui.label(
                egui::RichText::new(format!("{} 免费模型 {} 个", backend, free_count))
                    .small()
                    .weak(),
            );
        }
        clicked = ui
            .button("刷新")
            .on_hover_text(format!(
                "重新拉取 {} 的免费模型列表\n（默认每 {} 小时自动更新一次）",
                backend,
                crate::opencode_models::CACHE_TTL_SECS / 3600
            ))
            .clicked();
    }
    clicked
}

/// The current page backend's free models `(reference prefix, bare id list)`; the prefix is `None` without a free tier.
///
/// The prefix comes from [`crate::opencode_models::source_for`], the same source as the fetch's provider id.
///
/// Used only by the free-tier note and refresh button (mimocode has no free tier and shows neither).
/// For dropdown candidates use [`current_gateway_options`].
pub(super) fn current_free_models(
    page: ConfigFormat,
    free_models: &HashMap<ConfigFormat, FreeModelsState>,
) -> (Option<&'static str>, &[String]) {
    let Some(source) = crate::opencode_models::source_for(page) else {
        return (None, &[]);
    };
    let models = free_models
        .get(&page)
        .map(|state| state.models.as_slice())
        .unwrap_or(&[]);
    (Some(source.provider_id), models)
}

/// The current page's own-gateway model candidates (prefixed, preferred first).
///
/// Covers every opencode-family page, including tier-less mimocode
/// (the fallback list is [`crate::opencode_models::gateway_models`]).
pub(super) fn current_gateway_options(
    page: ConfigFormat,
    free_models: &HashMap<ConfigFormat, FreeModelsState>,
) -> Vec<String> {
    let free = free_models
        .get(&page)
        .map(|state| state.models.as_slice())
        .unwrap_or(&[]);
    crate::opencode_models::gateway_models(page, free)
}

/// The current page backend's free-model fetch state: `(in flight, failure reason)`.
pub(super) fn current_free_status(
    page: ConfigFormat,
    free_models: &HashMap<ConfigFormat, FreeModelsState>,
) -> (bool, Option<&str>) {
    match free_models.get(&page) {
        Some(state) => (state.fetching(), state.error.as_deref()),
        None => (false, None),
    }
}

/// The read-only sources the agent dropdown needs.
///
/// The `model` dropdown needs the page, the provider list, and the free-model state together.
pub(super) struct AgentComboCtx<'a> {
    pub(super) page: ConfigFormat,
    pub(super) providers: &'a [ProviderRow],
    pub(super) free_models: &'a HashMap<ConfigFormat, FreeModelsState>,
}

impl AgentComboCtx<'_> {
    /// The `model` dropdown: own-gateway models first, free models included.
    ///
    /// The free-model status note and "refresh" button live in the Agents title row — see `ui_agents_section`.
    pub(super) fn model_combo(&self, ui: &mut egui::Ui, salt: String, current: &str) -> String {
        let gateway = current_gateway_options(self.page, self.free_models);
        let options = model_options(self.providers, &gateway, current);
        let mut selected = options.iter().position(|m| m == current);
        egui::ComboBox::from_id_salt(salt)
            .selected_text(if current.is_empty() {
                "选择模型..."
            } else {
                current
            })
            .width(180.0)
            .show_ui(ui, |ui| {
                for (i, model) in options.iter().enumerate() {
                    if ui
                        .selectable_label(selected == Some(i), model.as_str())
                        .clicked()
                    {
                        selected = Some(i);
                    }
                }
            });
        selected
            .map(|i| options[i].clone())
            .unwrap_or_else(|| current.to_string())
    }
}

/// The `variant` dropdown. An empty value displays as "(none)" in the menu.
///
/// Both forms share one rendering; only the id salt differs.
pub(super) fn agent_variant_combo(ui: &mut egui::Ui, salt: String, current: &str) -> String {
    let options = ["", "low", "medium", "high", "xhigh", "max", "ultra"];
    let mut selected = options.iter().position(|v| *v == current);
    egui::ComboBox::from_id_salt(salt)
        .selected_text(if current.is_empty() {
            "选择..."
        } else {
            current
        })
        .width(100.0)
        .show_ui(ui, |ui| {
            for (i, v) in options.iter().enumerate() {
                let label = if v.is_empty() { "(空)" } else { v };
                if ui.selectable_label(selected == Some(i), label).clicked() {
                    selected = Some(i);
                }
            }
        });
    selected
        .map(|i| options[i].to_string())
        .unwrap_or_else(|| current.to_string())
}
