use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::AgentRow;
use std::collections::HashMap;

impl App {
    /// An opencode-family page's agent model view.
    ///
    /// When the target page is the current page, `agents` rules; other pages overlay their leave-time memories
    /// onto the matching keys, with the current value serving the rest.
    fn page_agent_view(&self, page: ConfigFormat) -> Vec<AgentRow> {
        let mut view = self.agents.clone();
        if page == self.current_page {
            return view;
        }
        self.overlay_remembered_models(page, &mut view);
        view
    }

    /// Overlay a page's memory onto `view`'s matching agents' models; keys absent from the memory are skipped.
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

    /// Agent model handling on page switches: first restore that page's own view, then replace still-invalid references.
    ///
    /// `leaving` is the page just left (`None` when none); its current `agents` values are recorded first.
    /// Restoration ignores `current_page`.
    ///
    /// Returns the replaced count; 0 means nothing needed changing.
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
        // Restore the page's view from when it was last left.
        let mut restored = self.agents.clone();
        self.overlay_remembered_models(page, &mut restored);
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        let free = self.page_gateway_free_models(page);
        let (agents, replaced) = agents_for_page(&restored, page, &keys, &free);
        // Write back whether or not anything was replaced.
        self.agents = agents;
        replaced
    }

    /// Record a page's current agent model view: `(file identity, page) → agent key → model`.
    ///
    /// Agent keys go through `key.trim()`.
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

    /// This page's dynamically fetched gateway free-model bare ids; empty without a free tier.
    fn page_gateway_free_models(&self, page: ConfigFormat) -> Vec<String> {
        self.free_models
            .get(&page)
            .map(|state| state.models.clone())
            .unwrap_or_default()
    }

    /// The agents to write when saving to a page: that page's own view, with invalid references replaced.
    pub(in crate::app) fn agents_for_page(&self, page: ConfigFormat) -> Vec<AgentRow> {
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        let free = self.page_gateway_free_models(page);
        agents_for_page(&self.page_agent_view(page), page, &keys, &free).0
    }
}

/// Swap `agents` models pointing at other gateways to `page`'s own preferred gateway model.
///
/// Returns `(the normalized list, replaced count)`; with nothing to change, a clone returns as-is with count 0.
///
/// Validity follows [`crate::opencode_models::model_is_valid_on`]: `model`'s prefix is neither this page's
/// gateway nor a configured provider key.
///
/// An empty `model` is untouched; with no own-gateway models available, nothing moves either.
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
