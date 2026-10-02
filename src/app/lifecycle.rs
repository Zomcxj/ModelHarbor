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

    /// 主题 / 形状任一变化（含首次启动）时重新套用样式。
    pub(in crate::app) fn apply_theme_if_changed(&mut self, ctx: &egui::Context) {
        let shape = self.ui_style;
        if crate::theme::needs_apply_style(&mut self.applied_theme, self.theme, shape) {
            self.theme.apply_style(ctx, shape);
        }
    }

    /// 界面设置变了就落盘（家目录 `.modelharbor/settings.json`）。
    pub(in crate::app) fn persist_prefs_if_changed(&mut self) {
        let current = self.current_prefs();
        if current == self.prefs_saved {
            return;
        }
        // 即使写失败也更新快照：否则每帧重试会刷屏（状态栏已提示一次）。
        if let Err(err) = current.save() {
            self.status = format!("界面偏好保存失败：{err}");
        }
        self.prefs_saved = current;
    }

    /// 解析各保存目标的可用性与实际路径（避免在渲染循环中频繁拉起 wsl 进程）。
    pub(in crate::app) fn refresh_targets(&mut self) {
        // 默认目标固定为 Windows 本地路径；WSL 侧仅通过“WSL同步”勾选写入，
        // 且写入前按页面检测对应 agent 是否已安装。
        // 路径走 `resolve_local_path`：默认名不存在但等价的 `.jsonc` 存在时，
        // 用后者——否则「已安装」判成 false，保存还会另建一个 `.json`。
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
                // 把「文件里这一页的 agent model 视图」播种进记忆：重载 = 文件为准，
                // 该页未保存的编辑随重载丢弃；其他页的记忆不受影响（它们按
                // (config_id, page) 键控，仍能还原各自视图）。
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
        // WorkBuddy 页：同一 id 多条启用收敛成「只启用第一条」（为什么见
        // `normalize_workbuddy_enable_flags`）；加载后立刻收敛，显示的状态才真实。
        if self.source_format == ConfigFormat::WorkBuddy {
            self.normalize_workbuddy_enable_flags();
        }
        // baseUrl 体检：加载后统计可疑 URL（如 `//v1` 重复斜杠），在状态栏提示，
        // 详情看 provider 卡片上的 ⚠ 标签（仅提示，不自动改写）。
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
        // 各页的 agent model 视图记忆**不清**：它按 (config_id, page) 键控，
        // 同文件重载后依然有效（还原切页视图靠它），换文件后旧键自然失配。
        // （曾在这里整表清空：同文件重载会把刚存下的记忆一起抹掉，切回原页
        // 时还原失效，指向别家网关的引用被判无效，全部被换成网关首选。）
        // 用户数据查询结果同样跟着配置走，重新加载后重查。
        self.balance.clear();
        self.balance_batch = false;
        // 被丢弃的探测不会再回传结果：释放全局串行位，否则门控会一直卡在 Busy。
        self.probe.release(None);
        // 加载后跳转到来源格式对应的页面
        self.current_page = self.source_format;
        // 只在加载成功时清理折叠记录：读不到文件（路径写错 / 临时不可用）时
        // providers/agents 是空的，照常清理会把用户存好的卡片状态抹掉。
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

    /// 重新加载路径，同时把“路径属于哪个页面”和“文件实际是什么格式”分开。
    ///
    /// 只有成功读出并解析文件后才记住路径；不存在的 `.json` 即使探测回落到
    /// opencode，也不会污染任何页面的持久化覆盖。实际格式与发起页面不同时，
    /// 页面跟随文件格式切换，路径只记到检测出的格式。
    pub(in crate::app) fn reload_for_page(&mut self, owner: ConfigFormat, remember: bool) {
        if !crate::util::config_exists(&self.config_path) {
            self.source_format = owner;
            self.apply_load();
            // 空内容在各后端可用于新建配置，因此 apply_load 会成功；但路径不存在时
            // 没有格式证据，也绝不能据此新增或改写任何持久化覆盖。
            self.current_page = owner;
            self.load_error = Some("配置文件不存在".into());
            self.status = format!("加载失败: 配置文件不存在 ({})", self.config_path);
            return;
        }

        let (detected, _) = ConfigPaths::detect_for_path(&self.config_path);
        self.source_format = detected;
        self.apply_load();

        if self.load_error.is_some() {
            // apply_load 会在成功时跟随来源切页；失败时没有可信的格式证据，留在用户页面。
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
