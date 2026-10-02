use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::AgentRow;
use std::collections::HashMap;

impl App {
    /// 某个 opencode 系页面的 agent model 视图：该页上用户真正配过的那份值。
    ///
    /// - 目标页就是**当前页**：以 `agents` 为准（用户可能正在编辑），记忆已过期。
    /// - 别的页面：以离开该页时存下的记忆覆盖对应 key，其余沿用当前值。
    ///
    /// 这样「一键保存」写到各页的才是**用户在该页配过的值**，而不是当前页那份被
    /// 归一过的值（否则用户在 kilo 页选的 `kilo-auto/balanced` 会被写成默认的
    /// `kilo-auto/free`，因为当前页看到的是 opencode 的引用）。
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
        let Some(saved) = self.agent_models_by_page.get(&page) else {
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
    /// ## 为什么需要「记忆」
    ///
    /// 三页共用同一份 `agents`，但每页网关不同，`model` 的前缀必须换成本页认的。
    /// 只在切页时单向替换是不够的——用户从 opencode 页切到 kilo 页（`opencode/…`
    /// 被换成 `kilo/kilo-auto/free`），再切回 opencode 页，**原来的 `opencode/…`
    /// 已经没了**，用户什么也没改却丢了配置。
    ///
    /// 所以离开一页时把该页的 model 视图存进 [`Self::agent_models_by_page`]，
    /// 回到该页先还原再归一：用户在各页配的、各自有效的那份值都留得住。
    ///
    /// `leaving` 是**刚刚离开**的页面（没有则传 `None`），它的当前 `agents` 值需要
    /// 先记下来——那是用户在该页真正编辑过的内容。
    ///
    /// 还原时**不看 `current_page`**：调用方通常已经把 `current_page` 设成了新页
    /// （见 `ui_top_bar`），若沿用 [`Self::page_agent_view`] 的当前页判定，记忆会被
    /// 当成过期而跳过，还原就失效了。
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
        // 还原：把该页上次离开时的视图取回来（key 已被删的条目自然跳过）。
        let mut restored = self.agents.clone();
        self.overlay_remembered_models(page, &mut restored);
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        let free = self.page_gateway_free_models(page);
        let (agents, replaced) = agents_for_page(&restored, page, &keys, &free);
        // 无论有没有发生替换都要写回：还原本身就是要让界面显示该页自己的值。
        self.agents = agents;
        replaced
    }

    /// 记下某页当前的 agent model 视图（页面 → agent key → model）。
    ///
    /// 用 `key.trim()` 作键：与保存时的重复判定同一口径，改名前后不会错位。
    fn remember_agent_models(&mut self, page: ConfigFormat) {
        if !page.is_opencode_family() {
            return;
        }
        let view: HashMap<String, String> = self
            .agents
            .iter()
            .map(|a| (a.key.trim().to_string(), a.model.clone()))
            .collect();
        self.agent_models_by_page.insert(page, view);
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
    /// **保存路径也必须按目标页归一**，不能只靠切页时改内存值。三页共用同一份
    /// agents 数据，而「一键保存」会把这同一份数据分别写进三个文件——若不按目标页
    /// 分别归一，最后访问过的那一页的模型会被写进所有文件，其余文件里就是无效引用
    /// （kilo 网关不认 `opencode/…`）。这里逐页归一，每个文件都拿到自己网关认的值。
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
/// ## 为什么需要
///
/// opencode 系三页共用同一份 agent 数据，但每页的**网关不同**：`model` 是
/// `provider/model`，前半段必须是本页网关认的 provider id。把 opencode 页配好的
/// `opencode/ling-3.0-flash-fin-free` 带到 kilo 页，kilo 网关不认这个前缀，agent
/// 直接跑不起来——而且界面看不出问题（下拉里就显示着那串字）。
///
/// ## 什么算「无效」
///
/// 判据见 [`crate::opencode_models::model_is_valid_on`]：前缀要么是本页网关，要么是
/// 用户自己配的 provider key（保存时 provider 容器一并写入，任何一页都有效）。
/// 只有**别家网关**的前缀才需要替换。
///
/// 空 model 不动：那是「还没选」，不是「选错了」，替换成默认值反而像擅自替用户做了决定。
/// 拿不到任何自家模型时也一律不动：宁可不替换，也不能把用户的配置清成空串。
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
