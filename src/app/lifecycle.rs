use super::*;

impl super::App {
    /// 记住某一页手动指定过的配置路径（空串 = 清除覆盖，回到自动探测值）。
    pub(in crate::app) fn remember_page_path(&mut self, format: ConfigFormat, path: &str) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            let default = ConfigPaths::default_local_path(format);
            self.config_paths.set_local_path(format, &default);
        } else {
            self.config_paths.set_local_path(format, trimmed);
        }
    }

    /// 主题 / 形状 / 玻璃档任一变化（含首次启动）时重新套用样式。
    pub(in crate::app) fn apply_theme_if_changed(&mut self, ctx: &egui::Context) {
        let shape = self.ui_style;
        let glass = self.glass;
        if crate::theme::needs_apply_style(&mut self.applied_theme, self.theme, shape, glass) {
            self.theme.apply_style(ctx, shape, glass);
            // 窗口层：DWM 背景与圆角，只切换「DWM 画不画模糊」。
            crate::windowfx::set_backdrop(if glass {
                crate::windowfx::Backdrop::Acrylic
            } else {
                crate::windowfx::Backdrop::None
            });
        }
    }

    /// 界面设置变了就落盘（家目录 `.modelharbor/settings.json`）。
    pub(in crate::app) fn persist_prefs_if_changed(&mut self) {
        let current = self.current_prefs();
        if current == self.prefs_saved {
            return;
        }
        // 写失败也更新快照。
        if let Err(err) = current.save() {
            self.status = format!("界面偏好保存失败：{err}");
        }
        self.prefs_saved = current;
    }

    /// 解析各保存目标的可用性与实际路径。
    pub(in crate::app) fn refresh_targets(&mut self) {
        // 默认目标为 Windows 本地路径；WSL 侧仅通过「WSL同步」勾选写入。
        // 路径走 `resolve_local_path`：默认名不存在但等价的 `.jsonc` 存在时用后者。
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

    /// 按 source_format 加载当前 config_path；失败时置空数据并记录 load_error。
    pub(in crate::app) fn apply_load(&mut self) {
        let path = self.config_path.clone();
        self.loaded_path = path.clone();
        // 文件内容变了，对比视图的缓存作废。
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
                // 把文件里这一页的 agent model 视图播种进记忆；其他页的记忆按
                // (config_id, page) 键控，不受影响。
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
        // WorkBuddy 页：同一 id 多条启用收敛成「只启用第一条」
        //（见 `normalize_workbuddy_enable_flags`）。
        if self.source_format == ConfigFormat::WorkBuddy {
            self.normalize_workbuddy_enable_flags();
        }
        // baseUrl 体检：统计可疑 URL（如 `//v1` 重复斜杠）并在状态栏提示，
        // 详情见卡片上的 ⚠ 标签。
        let suspicious = self
            .providers
            .iter()
            .filter(|p| !crate::util::url_suspicions(&p.base_url).is_empty())
            .count();
        if suspicious > 0 {
            self.status
                .push_str(&format!("（{} 个 baseUrl 可疑，见卡片提示）", suspicious));
        }
        // 重新加载后丢弃旧的模型获取状态
        self.model_fetch.clear();
        self.model_fetch_open.clear();
        self.latency.clear();
        // 各页的 agent model 视图记忆不清：它按 (config_id, page) 键控，
        // 同文件重载后依然有效。
        // 用户数据查询结果同样跟着配置走，重新加载后重查。
        self.balance.clear();
        self.balance_batch = false;
        // 被丢弃的探测不会再回传结果：释放全局串行位。
        self.probe.release(None);
        // 加载后跳转到来源格式对应的页面
        self.current_page = self.source_format;
        // 只在加载成功时清理折叠记录：读不到文件时 providers/agents 为空。
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

    /// 重新加载路径，同时把「路径属于哪个页面」和「文件实际是什么格式」分开。
    ///
    /// 只有成功读出并解析文件后才记住路径；实际格式与发起页面不同时，页面跟随
    /// 文件格式切换，路径只记到检测出的格式。
    pub(in crate::app) fn reload_for_page(&mut self, owner: ConfigFormat, remember: bool) {
        if !crate::util::config_exists(&self.config_path) {
            self.source_format = owner;
            self.apply_load();
            // 路径不存在时没有格式证据，不据此新增或改写任何持久化覆盖。
            self.current_page = owner;
            self.load_error = Some("配置文件不存在".into());
            self.status = format!("加载失败: 配置文件不存在 ({})", self.config_path);
            return;
        }

        let (detected, _) = ConfigPaths::detect_for_path(&self.config_path);
        self.source_format = detected;
        self.apply_load();

        if self.load_error.is_some() {
            // 失败时没有可信的格式证据，留在用户页面。
            self.current_page = owner;
            return;
        }

        let path = self.config_path.trim().to_string();
        if remember && !path.is_empty() {
            self.remember_page_path(detected, &path);
        }
        if detected != owner {
            self.status.push_str(&format!(
                "（检测为 {}，未记作 {} 页路径）",
                detected.label(),
                owner.label()
            ));
        }
    }

    /// 清除某一页的路径覆盖，切回自动探测到的默认路径并加载
    /// （界面「留空 + 回车」的语义）。
    pub(in crate::app) fn reset_page_path(&mut self, format: ConfigFormat) {
        self.remember_page_path(format, "");
        self.config_path = self.config_paths.target_path(format);
        self.reload_for_page(format, false);
    }
}
