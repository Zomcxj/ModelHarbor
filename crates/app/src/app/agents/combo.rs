use crate::app::fetch::FreeModelsState;
use crate::format::ConfigFormat;
use crate::model::ProviderRow;
use eframe::egui;
use std::collections::{HashMap, HashSet};

/// Agents 的 `model` 下拉候选，按「自家网关 → 自配 provider → 当前值」排序。
///
/// 分三段按顺序拼接：
///
/// 1. `gateway`：本页网关的模型（首选在最前，见 [`crate::opencode_models::gateway_models`]）；
/// 2. 自配 provider 的模型（按 provider 配置顺序、模型配置顺序）；
/// 3. 当前值（若前两段都没有它）。
///
/// 当前值一定保留，排在最后。
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
    // 只去重，保持原有顺序。
    let mut seen = HashSet::new();
    options.retain(|option| seen.insert(option.clone()));
    options
}

/// Agents 标题行里（「展开全部卡片」右侧）的免费模型状态提示与「刷新」按钮。
///
/// 刷新的是本页后端的内置免费模型清单，整页只有这一份。
///
/// 返回 `true` 表示用户点了刷新；调用方随即发起后台拉取。
///
/// 拉取中 / 失败 / 空列表三种状态各有文案，就地可见。
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

/// 当前页面后端的免费模型 `(引用前缀, 裸 id 列表)`；没有免费层时前缀为 `None`。
///
/// 前缀来自 [`crate::opencode_models::source_for`]，与拉取时用的 provider id 同源。
///
/// 只用于免费层的提示与刷新按钮（mimocode 没有免费层，不显示这些）。
/// 下拉候选请用 [`current_gateway_options`]。
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

/// 当前页面自家网关的模型候选（已带前缀，首选在最前）。
///
/// 覆盖所有 opencode 系页面，包括没有免费层的 mimocode
/// （兜底清单见 [`crate::opencode_models::gateway_models`]）。
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

/// 当前页面后端的免费模型拉取状态：`(是否在飞, 失败原因)`。
pub(super) fn current_free_status(
    page: ConfigFormat,
    free_models: &HashMap<ConfigFormat, FreeModelsState>,
) -> (bool, Option<&str>) {
    match free_models.get(&page) {
        Some(state) => (state.fetching(), state.error.as_deref()),
        None => (false, None),
    }
}

/// agent 下拉所需的只读来源。
///
/// `model` 下拉要同时用到页面、provider 列表与免费模型状态。
pub(super) struct AgentComboCtx<'a> {
    pub(super) page: ConfigFormat,
    pub(super) providers: &'a [ProviderRow],
    pub(super) free_models: &'a HashMap<ConfigFormat, FreeModelsState>,
}

impl AgentComboCtx<'_> {
    /// `model` 下拉：自家网关模型排最前，免费模型一并入选。
    ///
    /// 免费模型的状态提示与「刷新」按钮在 Agents 标题行，见 `ui_agents_section`。
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

/// `variant` 下拉。空值在菜单里显示为「(空)」。
///
/// 两个表单共用一套渲染，只差 id salt。
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
