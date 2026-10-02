use crate::app::fetch::FreeModelsState;
use crate::format::ConfigFormat;
use crate::model::ProviderRow;
use eframe::egui;
use std::collections::{HashMap, HashSet};

/// Agents 的 `model` 下拉候选，按「自家网关 → 自配 provider → 当前值」排序。
///
/// 写成自由函数（而不是 `&self` 方法）是为了能在表单里调用：那里
/// `self.agents[idx]` 已被可变借用，整结构借用会编译不过，只能按字段拆分。
///
/// **顺序就是需求**：用户要的是「自家网关排最前」，所以**不能再整体 `sort()`**——
/// 那会把网关模型和自配 provider 混在一起按字典序排，`kilo-auto/free` 这种
/// 网关里的关键项会被 `kilo/~anthropic/...` 挤到后面。分三段拼接：
///
/// 1. `gateway`：本页网关的模型（首选在最前，见 [`crate::opencode_models::gateway_models`]）；
/// 2. 自配 provider 的模型（按 provider 配置顺序、模型配置顺序，保持用户自己的排列）；
/// 3. 当前值（若前两段都没有它）。
///
/// **当前值一定保留**：远程列表随时会变，若某个已配置的模型被下架，直接从候选里
/// 抹掉会让用户看到「下拉是空的 / 选中项不见了」，误以为配置坏了。保留它并排在
/// 最后，用户能看见自己配的是什么，想换再换。
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
    // 只去重、**不排序**：顺序承载「自家网关排最前」的语义。
    let mut seen = HashSet::new();
    options.retain(|option| seen.insert(option.clone()));
    options
}

/// Agents 标题行里（「展开全部卡片」右侧）的免费模型状态提示与「刷新」按钮。
///
/// 刷新的是**本页后端的内置免费模型清单**，与具体哪个 agent 无关，所以整页只有
/// 这一份，不随每个 model 下拉重复。
///
/// 返回 `true` 表示用户点了刷新；调用方随即发起后台拉取（标题行是 `&mut self`
/// 上下文，可以直接调 `start_free_models_fetch`）。
///
/// 拉取中 / 失败 / 空列表三种状态各有文案，就地可见，用户不必去状态栏找原因。
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
/// 前缀来自 [`crate::opencode_models::source_for`]，与拉取时用的 provider id 同源，
/// 避免两处各写一份字符串而漂移。
///
/// 只用于**免费层的提示与刷新按钮**（mimocode 没有免费层，不显示这些）。
/// 下拉候选请用 [`current_gateway_options`]——那才是「自家网关模型」的完整口径。
///
/// 写成自由函数（不是 `&self` 方法）：表单里 `self.agents[idx]` 已被可变借用，
/// 只能按字段拆分借用，整结构借用会编译不过。
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

/// 当前页面**自家网关**的模型候选（已带前缀，首选在最前），供下拉排序与替换使用。
///
/// 与 [`current_free_models`] 的区别：这里覆盖**所有** opencode 系页面，包括没有免费层
/// 的 mimocode（它的兜底清单见 [`crate::opencode_models::gateway_models`]）。下拉的
/// 「自家网关排最前」与「切页即替换」都以此为准。
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
/// 打包成一个结构而不是逐个传参：`model` 下拉要同时用到页面、provider 列表与免费模型
/// 状态，摊开就是 5 个以上参数。写成 `&self` 方法也不行——调用处 `self.agents[idx]`
/// 已被可变借用，只能按字段拆分。
pub(super) struct AgentComboCtx<'a> {
    pub(super) page: ConfigFormat,
    pub(super) providers: &'a [ProviderRow],
    pub(super) free_models: &'a HashMap<ConfigFormat, FreeModelsState>,
}

impl AgentComboCtx<'_> {
    /// `model` 下拉：自家网关模型排最前，免费模型一并入选。
    ///
    /// 免费模型的**状态提示与「刷新」按钮不在这里**——每个下拉旁各放一份是重复，
    /// 统一放在 Agents 标题行（「展开全部卡片」右侧），见 `ui_agents_section`。
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
/// 与 [`AgentComboCtx::model_combo`] 同理：两个表单共用一套渲染，只差 id salt。
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
