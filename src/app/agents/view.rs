use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::AgentRow;
use std::collections::HashMap;

impl App {
    /// 某个 opencode 系页面的 agent model 视图：该页上用户真正配过的那份值。
    ///
    /// 目标页是当前页时以 `agents` 为准；别的页面以离开该页时存下的记忆覆盖对应
    /// key，其余沿用当前值。
    fn page_agent_view(&self, page: ConfigFormat) -> Vec<AgentRow> {
        let mut view = self.agents.clone();
        if page == self.current_page {
            return view;
        }
        self.overlay_remembered_models(page, &mut view);
        view
    }

    /// 用某页的记忆覆盖 `view` 里对应 agent 的 model（key 已被删的条目自然跳过）。
    fn overlay_remembered_models(&self, page: ConfigFormat, view: &mut [AgentRow]) {
        let key = (self.config_id(), page);
        let Some(saved) = self.agent_models_by_page.get(&key) else {
            return;
        };
        for agent in view.iter_mut() {
            if let Some(model) = saved.get(agent.key.trim()) {
                agent.model = model.clone();
            }
        }
    }

    /// 切页时的 agent model 处理：先还原该页自己的视图，再替换仍然无效的引用。
    ///
    /// `leaving` 是刚刚离开的页面（没有则传 `None`），它的当前 `agents` 值会先被记下。
    ///
    /// 还原时不看 `current_page`。
    ///
    /// 返回被替换的条数（供调用方在状态栏提示），0 表示无需改动。
    pub(in crate::app) fn normalize_agent_models_for_page(
        &mut self,
        page: ConfigFormat,
        leaving: Option<ConfigFormat>,
    ) -> usize {
        if let Some(prev) = leaving {
            self.remember_agent_models(prev);
        }
        if !page.is_opencode_family() {
            return 0;
        }
        // 还原：把该页上次离开时的视图取回来。
        let mut restored = self.agents.clone();
        self.overlay_remembered_models(page, &mut restored);
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        let free = self.page_gateway_free_models(page);
        let (agents, replaced) = agents_for_page(&restored, page, &keys, &free);
        // 无论有没有发生替换都要写回。
        self.agents = agents;
        replaced
    }

    /// 记下某页当前的 agent model 视图（（文件身份， 页面） → agent key → model）。
    ///
    /// 键用 `key.trim()`，外层带文件身份。
    fn remember_agent_models(&mut self, page: ConfigFormat) {
        if !page.is_opencode_family() {
            return;
        }
        let view: HashMap<String, String> = self
            .agents
            .iter()
            .map(|a| (a.key.trim().to_string(), a.model.clone()))
            .collect();
        let key = (self.config_id(), page);
        self.agent_models_by_page.insert(key, view);
    }

    /// 本页动态拉到的网关免费模型裸 id；没有免费层（mimocode）时为空。
    fn page_gateway_free_models(&self, page: ConfigFormat) -> Vec<String> {
        self.free_models
            .get(&page)
            .map(|state| state.models.clone())
            .unwrap_or_default()
    }

    /// 写入某一页时要落盘的 agents：先取该页自己的视图，再替换其中无效的引用。
    ///
    /// 逐页归一，每个文件都拿到自己网关认的值。
    pub(in crate::app) fn agents_for_page(&self, page: ConfigFormat) -> Vec<AgentRow> {
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        let free = self.page_gateway_free_models(page);
        agents_for_page(&self.page_agent_view(page), page, &keys, &free).0
    }
}

/// 把 `agents` 里指向别家网关的 `model` 换成 `page` 自家网关的首选模型。
///
/// 返回 `(归一后的列表, 替换条数)`；无需改动时原样克隆返回、条数为 0。
///
/// 无效的判据见 [`crate::opencode_models::model_is_valid_on`]：`model` 的前缀既不是
/// 本页网关，也不是用户自己配的 provider key。
///
/// 空 `model` 不动；拿不到任何自家模型时也一律不动。
pub(super) fn agents_for_page(
    agents: &[AgentRow],
    page: ConfigFormat,
    configured_keys: &[String],
    free: &[String],
) -> (Vec<AgentRow>, usize) {
    let mut out = agents.to_vec();
    if !page.is_opencode_family() {
        return (out, 0);
    }
    let invalid: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            let model = a.model.trim();
            !model.is_empty()
                && !crate::opencode_models::model_is_valid_on(page, configured_keys, model)
        })
        .map(|(idx, _)| idx)
        .collect();
    if invalid.is_empty() {
        return (out, 0);
    }
    let Some(replacement) = crate::opencode_models::default_gateway_model(page, free) else {
        return (out, 0);
    };
    for idx in &invalid {
        out[*idx].model = replacement.clone();
    }
    let replaced = invalid.len();
    (out, replaced)
}
