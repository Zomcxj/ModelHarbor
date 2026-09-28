use super::*;

impl super::App {
    /// provider 卡片是否处于折叠状态。
    pub(in crate::app) fn provider_collapsed(&self, key: &str) -> bool {
        self.collapsed.contains(&self.card_id("providers", key))
    }

    /// 设置 provider 卡片折叠状态。
    pub(in crate::app) fn set_provider_collapsed(&mut self, key: &str, collapsed: bool) {
        let id = self.card_id("providers", key);
        if collapsed {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
        }
    }

    /// 当前页所有 provider 卡片：折叠或展开。
    pub(in crate::app) fn set_all_providers_collapsed(&mut self, collapsed: bool) {
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        for key in keys {
            self.set_provider_collapsed(&key, collapsed);
        }
    }

    /// agent 卡片是否处于折叠状态。
    pub(in crate::app) fn agent_collapsed(&self, key: &str) -> bool {
        self.collapsed.contains(&self.card_id("agents", key))
    }

    /// 设置 agent 卡片折叠状态。
    pub(in crate::app) fn set_agent_collapsed(&mut self, key: &str, collapsed: bool) {
        let id = self.card_id("agents", key);
        if collapsed {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
        }
    }

    /// 当前页所有 agent 卡片：折叠或展开。
    pub(in crate::app) fn set_all_agents_collapsed(&mut self, collapsed: bool) {
        let keys: Vec<String> = self.agents.iter().map(|a| a.key.clone()).collect();
        for key in keys {
            self.set_agent_collapsed(&key, collapsed);
        }
    }

    /// 卡片改名后同步折叠状态（否则改完名卡片会跳回展开）。
    pub(in crate::app) fn rename_collapsed_card(&mut self, kind: &str, old: &str, new: &str) {
        let from = self.card_id(kind, old);
        if self.collapsed.remove(&from) {
            self.collapsed.insert(self.card_id(kind, new));
        }
    }

    /// 把 v2 全局折叠键迁到当前首次成功加载的配置身份。
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

    /// 只清理当前配置身份下已经不存在的折叠记录；其他配置不受影响。
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
