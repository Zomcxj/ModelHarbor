use super::*;

impl super::App {
    /// Remember a page's manually specified config path (empty string = clear the override, back to auto-detection).
    pub(in crate::app) fn remember_page_path(&mut self, format: ConfigFormat, path: &str) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            let default = ConfigPaths::default_local_path(format);
            self.config_paths.set_local_path(format, &default);
        } else {
            self.config_paths.set_local_path(format, trimmed);
        }
    }

    /// Re-apply styling when theme / shape / acrylic tier changes (first startup included).
    pub(in crate::app) fn apply_theme_if_changed(&mut self, ctx: &egui::Context) {
        let shape = self.ui_style;
        let glass = self.glass;
        if crate::theme::needs_apply_style(&mut self.applied_theme, self.theme, shape, glass) {
            self.theme.apply_style(ctx, shape, glass);
            // Window layer: DWM backdrop and corners, toggling only "whether DWM paints the blur".
            crate::windowfx::set_backdrop(if glass {
                crate::windowfx::Backdrop::Acrylic
            } else {
                crate::windowfx::Backdrop::None
            });
        }
    }

    /// Persist settings when they change (home's `.modelharbor/settings.json`).
    pub(in crate::app) fn persist_prefs_if_changed(&mut self) {
        let current = self.current_prefs();
        if current == self.prefs_saved {
            return;
        }
        // Update the snapshot even when the write fails.
        if let Err(err) = current.save() {
            self.status = format!("界面偏好保存失败：{err}");
        }
        self.prefs_saved = current;
    }

    /// Resolve every save target's availability and actual path.
    pub(in crate::app) fn refresh_targets(&mut self) {
        // Default targets are Windows local paths; the WSL side writes only via the "WSL sync" checkbox.
        // Paths go through `resolve_local_path`: when the default name is missing but an equivalent .jsonc exists, use that.
        self.targets = backends::BACKENDS
            .iter()
            .map(|b| {
                let id = b.id();
                let local = backends::resolve_local_path(id, &self.config_paths.local_path(id));
                SaveTarget {
                    backend: id,
                    available: self.config_paths.validate_target(id),
                    path: local,
                }
            })
            .collect();
    }

    /// Load the current config_path per source_format; on failure, clear the data and record load_error.
    pub(in crate::app) fn apply_load(&mut self) {
        let path = self.config_path.clone();
        self.loaded_path = path.clone();
        // File content changed: the diff view's cache is invalid.
        self.preview_diff_cache = None;
        self.preview_diff_pending = None;
        let result = backends::load_backend(self.source_format, &path);
        match result {
            Ok(load) => {
                self.root = load.root;
                self.agents = load.agents;
                self.providers = load.providers;
                self.pi_extras = load.extras;
                self.load_error = None;
                // Seed this page's agent model view into the memory; other pages' memories are keyed by (config_id, page)
                // and unaffected.
                if self.source_format.is_opencode_family() {
                    let view = self
                        .agents
                        .iter()
                        .map(|a| (a.key.trim().to_string(), a.model.clone()))
                        .collect();
                    self.agent_models_by_page
                        .insert((self.config_id(), self.source_format), view);
                }
                self.status = format!(
                    "已加载 ({}): {} agents, {} providers",
                    self.source_format.label(),
                    self.agents.len(),
                    self.providers.len()
                );
            }
            Err(e) => {
                self.root = Value::Object(Map::new());
                self.agents = Vec::new();
                self.providers = Vec::new();
                self.pi_extras = Value::Object(Map::new());
                self.load_error = Some(e.clone());
                self.status = format!("加载失败: {}", e);
            }
        }
        // The WorkBuddy page: multiple enabled entries per id converge to "only the first enabled"
        // (see `normalize_workbuddy_enable_flags`).
        if self.source_format == ConfigFormat::WorkBuddy {
            self.normalize_workbuddy_enable_flags();
        }
        // baseUrl health check: tally suspicious URLs (like `//v1` doubled slashes) and note in the status bar;
        // details live on the cards' ⚠ labels.
        let suspicious = self
            .providers
            .iter()
            .filter(|p| !crate::util::url_suspicions(&p.base_url).is_empty())
            .count();
        if suspicious > 0 {
            self.status
                .push_str(&format!("（{} 个 baseUrl 可疑，见卡片提示）", suspicious));
        }
        // Reload discards old test states
        self.latency.clear();
        // Per-page agent model view memories aren't cleared: keyed by (config_id, page), they survive a
        // same-file reload.
        // User-data query results follow the config too — re-queried after reload.
        self.balance.clear();
        self.balance_batch = false;
        // Discarded probes never return results: release the global serialization slot.
        self.probe.release(None);
        // After loading, jump to the page matching the source format
        self.current_page = self.source_format;
        // Collapse records clear only on a successful load: an unreadable file leaves providers/agents empty.
        if self.load_error.is_none() {
            self.migrate_legacy_collapsed();
            self.prune_collapsed();
        }
        self.reset_preview_draft();
        self.refresh_targets();
    }

    pub(in crate::app) fn reload(&mut self) {
        self.reload_for_page(self.current_page, true);
    }

    /// Reload path: the file's **content** decides the parse and display format (source_format), while the path
    /// is recorded under the **current page** (owner) for continued parsing / saving within this session.
    ///
    /// Content detection answers only "which backend parses this file", never "which page owns the path" —
    /// otherwise opening an opencode-content file from the pi page would register the path under opencode,
    /// losing the user's in-page switch (a reported bug).
    ///
    /// The remembered path is **not written to settings.json** (see `current_prefs`): every page loads its
    /// official default at startup, and files swapped within a session don't survive restart (user decision).
    /// The path is remembered only after the file is successfully read and parsed.
    pub(in crate::app) fn reload_for_page(&mut self, owner: ConfigFormat, remember: bool) {
        if !crate::util::config_exists(&self.config_path) {
            self.source_format = owner;
            self.apply_load();
            // A nonexistent path carries no format evidence: never add or rewrite any persisted override on that basis.
            self.current_page = owner;
            self.load_error = Some("配置文件不存在".into());
            self.status = format!("加载失败: 配置文件不存在 ({})", self.config_path);
            return;
        }

        // Content detection: decides which backend's format parses and displays this file.
        let (detected, _) = ConfigPaths::detect_for_path(&self.config_path);
        self.source_format = detected;
        self.apply_load();

        if self.load_error.is_some() {
            // On failure there's no trustworthy format evidence: stay on the user's page.
            self.current_page = owner;
            return;
        }

        // The page stays on the owner page the user operated (no content-based page switching).
        self.current_page = owner;
        let path = self.config_path.trim().to_string();
        if remember && !path.is_empty() {
            // The path registers under the current page (owner), effective for this session only (never persisted).
            self.remember_page_path(owner, &path);
        }
    }

    /// Clear a page's path override, switch back to the auto-detected default path and load it
    /// (the UI's "leave empty + Enter" semantics).
    pub(in crate::app) fn reset_page_path(&mut self, format: ConfigFormat) {
        self.remember_page_path(format, "");
        self.config_path = self.config_paths.target_path(format);
        self.reload_for_page(format, false);
    }
}
