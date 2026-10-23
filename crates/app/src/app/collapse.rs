use super::*;

impl super::App {
    /// Whether a provider card is collapsed.
    pub(in crate::app) fn provider_collapsed(&self, key: &str) -> bool {
        self.collapsed.contains(&self.card_id("providers", key))
    }

    /// Set a provider card's collapsed state.
    pub(in crate::app) fn set_provider_collapsed(&mut self, key: &str, collapsed: bool) {
        let id = self.card_id("providers", key);
        if collapsed {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
        }
    }

    /// Every provider card on the current page: collapsed or expanded.
    pub(in crate::app) fn set_all_providers_collapsed(&mut self, collapsed: bool) {
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        for key in keys {
            self.set_provider_collapsed(&key, collapsed);
        }
    }

    /// Whether an agent card is collapsed.
    pub(in crate::app) fn agent_collapsed(&self, key: &str) -> bool {
        self.collapsed.contains(&self.card_id("agents", key))
    }

    /// Set an agent card's collapsed state.
    pub(in crate::app) fn set_agent_collapsed(&mut self, key: &str, collapsed: bool) {
        let id = self.card_id("agents", key);
        if collapsed {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
        }
    }

    /// Every agent card on the current page: collapsed or expanded.
    pub(in crate::app) fn set_all_agents_collapsed(&mut self, collapsed: bool) {
        let keys: Vec<String> = self.agents.iter().map(|a| a.key.clone()).collect();
        for key in keys {
            self.set_agent_collapsed(&key, collapsed);
        }
    }

    /// Sync collapse state after a card rename.
    pub(in crate::app) fn rename_collapsed_card(&mut self, kind: &str, old: &str, new: &str) {
        let from = self.card_id(kind, old);
        if self.collapsed.remove(&from) {
            self.collapsed.insert(self.card_id(kind, new));
        }
    }

    /// Migrate the v2 global collapse keys onto the first successfully loaded config's identity.
    pub(in crate::app) fn migrate_legacy_collapsed(&mut self) {
        let config_id = self.config_id();
        for (kind, key) in self
            .providers
            .iter()
            .map(|row| ("providers", row.key.as_str()))
            .chain(self.agents.iter().map(|row| ("agents", row.key.as_str())))
        {
            let legacy = crate::prefs::legacy_collapsed_id(kind, key);
            if self.collapsed.remove(&legacy) {
                self.collapsed
                    .insert(crate::prefs::collapsed_id(&config_id, kind, key));
            }
        }
    }

    /// Prune only collapse records under the current config identity that no longer exist; other configs are unaffected.
    pub(in crate::app) fn prune_collapsed(&mut self) {
        let config_id = self.config_id();
        let prefix = format!("{config_id}/");
        let mut alive: HashSet<String> = self
            .providers
            .iter()
            .map(|p| crate::prefs::collapsed_id(&config_id, "providers", &p.key))
            .collect();
        alive.extend(
            self.agents
                .iter()
                .map(|a| crate::prefs::collapsed_id(&config_id, "agents", &a.key)),
        );
        self.collapsed
            .retain(|id| !id.starts_with(&prefix) || alive.contains(id));
    }
}
