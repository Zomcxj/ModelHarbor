impl super::App {
    /// The current config file's identity. All pages share one loaded dataset, so partitioning goes by path, never by page.
    pub(in crate::app) fn config_id(&self) -> String {
        let path = if self.loaded_path.trim().is_empty() {
            &self.config_path
        } else {
            &self.loaded_path
        };
        crate::prefs::config_identity(path)
    }

    /// The persistence key for card collapse state (per config and category).
    pub(in crate::app) fn card_id(&self, kind: &str, key: &str) -> String {
        crate::prefs::collapsed_id(&self.config_id(), kind, key)
    }

    /// A baseUrl's station's panel token (empty string when unset).
    ///
    /// Keys are normalized origins: several providers on one station share one token.
    pub(in crate::app) fn station_pat(&self, base_url: &str) -> String {
        let key = crate::tokens::station_key(base_url);
        self.tokens.get(&key).to_string()
    }

    /// A baseUrl's station's user ID (empty string when unset; empty = don't send `New-Api-User`).
    pub(in crate::app) fn station_user_id(&self, base_url: &str) -> String {
        let key = crate::tokens::station_key(base_url);
        self.tokens.user_id(&key).to_string()
    }

    /// Whether a baseUrl's station is marked sub2api (balances query `/v1/usage` directly, no panel token needed).
    pub(in crate::app) fn station_is_sub2api(&self, base_url: &str) -> bool {
        self.tokens
            .is_sub2api(&crate::tokens::station_key(base_url))
    }
}
