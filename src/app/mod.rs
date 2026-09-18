use crate::backends;
use crate::format::{ConfigFormat, ConfigPaths};
use crate::model::{AgentRow, ProviderRow};
use crate::theme::Theme;
use eframe::egui;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

mod agents;
mod balance;
mod bars;
mod fetch;
mod preview;
mod providers;
mod providers_form;
mod save;
mod serialize;
mod syntax;

use fetch::{LatencyState, ModelFetchState, ProbeGate};
use save::SaveTarget;

pub use save::{
    load_opencode_result, load_or_empty, load_pi_result, strip_cross_format_containers,
};
pub(crate) use serialize::{compact_json, pretty_json};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SaveFormat {
    Current,
    #[default]
    Compact,
}

impl SaveFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Current => "默认格式",
            Self::Compact => "压缩格式",
        }
    }

    /// 持久化用的稳定标识。
    fn key(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Compact => "compact",
        }
    }

    /// 由持久化标识还原；未知 / 空值回落默认格式。
    fn from_key(key: &str) -> SaveFormat {
        match key {
            "current" => Self::Current,
            _ => Self::default(),
        }
    }
}

pub struct App {
    root: Value,
    agents: Vec<AgentRow>,
    providers: Vec<ProviderRow>,
    new_agent: AgentRow,
    new_provider: ProviderRow,
    config_path: String,
    /// 最近一次实际加载的路径（config_path 与之不等时按“未加载”处理，防止误覆盖）。
    loaded_path: String,
    status: String,
    show_new_agent: bool,
    show_new_provider: bool,
    /// 已折叠的卡片（`页面/类别/名字`，见 `prefs::collapsed_id`）。
    /// 用「折叠集合」而不是「展开集合」：新加载进来的卡片默认是展开的，
    /// 而且这份状态能原样落盘、不会被「重新加载」清空。
    collapsed: HashSet<String>,
    variant_open: HashSet<String>,
    agent_drag_src: Option<String>,
    agent_drag_target: Option<String>,
    provider_drag_src: Option<String>,
    provider_drag_target: Option<String>,
    model_drag_src: Option<String>,
    model_drag_target: Option<String>,
    /// 每个 provider 的模型获取状态（key → 状态）。
    model_fetch: HashMap<String, ModelFetchState>,
    /// 已展开的模型获取面板（provider key）。
    model_fetch_open: HashSet<String>,
    /// 每个 provider 的延迟测试状态（key → 状态）。
    latency: HashMap<String, LatencyState>,
    /// 每个 provider 的「已用 / 余额」查询状态（key → 状态）。
    balance: HashMap<String, balance::BalanceState>,
    /// 正在跑「一键查询用户数据」批次：全部结束后在状态栏给一条汇总。
    balance_batch: bool,
    /// 站点面板访问令牌（PAT，`.modelharbor/tokens.json`）：站点级，多个 provider 共用。
    tokens: crate::tokens::StationTokens,
    /// 「令牌」管理面板是否展开。
    show_tokens: bool,
    /// 令牌面板里正在编辑的文本（键 = 站点 origin），未保存的草稿。
    token_draft: HashMap<String, String>,
    /// 令牌面板里「用户 ID」的草稿（旧版 new-api 的 `New-Api-User` 头，可留空）。
    token_uid_draft: HashMap<String, String>,
    /// 已点「显示」的站点（单条掩码开关，默认跟随全局「显示密钥」）。
    token_reveal: HashSet<String>,
    /// 模型延迟探测的节流与串行状态（纯内存，重启清零）。
    probe: ProbeGate,
    /// 网络守卫结论：`Some(reason)` 表示检测到系统代理 / VPN，模型延迟测试被禁用。
    net_guard: Option<String>,
    /// 检测到代理时是否仍允许模型延迟测试（来自 settings.json，默认拦截）。
    allow_model_test_with_proxy: bool,
    /// 首次使用引导条是否已被关掉（来自 settings.json）。
    guide_dismissed: bool,
    /// 上次网络守卫检测时刻（egui 秒）。
    net_guard_at: f64,
    theme: Theme,
    /// 已应用到 egui 的主题：egui 0.33 的 `set_style` 是**每个主题各存一份 style**，
    /// 所以主题一变就必须显式再 `apply` 一次，否则只有按钮文字变、界面颜色不跟着变。
    applied_theme: Option<Theme>,
    save_format: SaveFormat,
    /// 滚轮切换保存格式的门门：一次连续滚动手势只切换一次。
    save_format_wheel_latch: bool,
    source_format: ConfigFormat,
    config_paths: ConfigPaths,
    targets: Vec<SaveTarget>,
    current_page: ConfigFormat,
    sync_wsl: bool,
    /// 全局 API Key 显隐：一键控制所有密钥输入框的明文/掩码显示。
    show_api_keys: bool,
    /// 已落盘的界面偏好快照：与当前值不同就写盘（避免每帧重复写文件）。
    prefs_saved: crate::prefs::Prefs,
    /// 右侧配置预览/编辑面板是否打开。
    show_preview: bool,
    /// 预览面板宽度占窗口宽度的比例（拖动分隔条调整；窗口缩放时按此比例适配）。
    preview_ratio: f32,
    /// 预览文本框是否持有焦点（编辑中以文本为准，失焦后以组件状态为准）。
    preview_focused: bool,
    /// 预览文本框缓冲（待保存文档；失焦时由组件状态实时重写）。
    preview_draft: String,
    /// 最近一次文本编辑的时间点（ctx 时间），用于防抖自动保存。
    preview_dirty_at: Option<f64>,
    /// 用户最近一次在预览框内输入的帧时间（用于判断「正在手改预览」）。
    preview_edit_at: Option<f64>,
    /// 最近一次文本解析是否成功（解析失败不写盘、不覆盖文本）。
    preview_parse_ok: bool,
    /// 最近一次预览文本解析失败的报错（成功时为 None），用于面板内红字提示。
    preview_parse_error: Option<String>,
    /// 预览 Ctrl+F 查找：查询词、是否打开、当前命中下标。
    preview_find: String,
    preview_find_active: bool,
    preview_find_index: usize,
    /// 下一帧需要给查找框抢焦点。
    preview_find_focus: bool,
    /// 待跳转的命中字节偏移（Enter/按钮跳转后用光标滚动到该处）。
    preview_find_jump: Option<usize>,
    /// 光标所在行（1-based；失焦时保留最后位置）。
    preview_cursor_line: usize,
    load_error: Option<String>,
    pi_extras: Value,
    /// 各后端官方图标纹理（与 BACKENDS 顺序对齐，首帧惰性加载）。
    backend_icons: Vec<Option<egui::TextureHandle>>,
}

impl Default for App {
    fn default() -> Self {
        let prefs = crate::prefs::Prefs::load();
        // 路径：先套用 prefs 里用户手动指定过的覆盖，再定位要打开的页面
        // （覆盖过且文件存在的页面优先，其次才按默认路径自动探测）。
        let mut paths = ConfigPaths::default();
        paths.apply_overrides(&prefs.config_paths);
        let (format, path) = paths
            .detect_preferring_overrides(&prefs.config_paths)
            .unwrap_or((ConfigFormat::Opencode, String::new()));
        let mut app = Self {
            root: Value::Object(Map::new()),
            agents: Vec::new(),
            providers: Vec::new(),
            new_agent: AgentRow::new(),
            new_provider: ProviderRow::new(),
            config_path: path,
            loaded_path: String::new(),
            status: String::new(),
            show_new_agent: false,
            show_new_provider: false,
            collapsed: prefs.collapsed.iter().cloned().collect(),
            variant_open: HashSet::new(),
            agent_drag_src: None,
            agent_drag_target: None,
            provider_drag_src: None,
            provider_drag_target: None,
            model_drag_src: None,
            model_drag_target: None,
            model_fetch: HashMap::new(),
            model_fetch_open: HashSet::new(),
            latency: HashMap::new(),
            balance: HashMap::new(),
            balance_batch: false,
            tokens: crate::tokens::StationTokens::load(),
            show_tokens: false,
            token_draft: HashMap::new(),
            token_uid_draft: HashMap::new(),
            token_reveal: HashSet::new(),
            probe: ProbeGate::default(),
            net_guard: crate::netguard::detect(),
            allow_model_test_with_proxy: prefs.allow_model_test_with_proxy,
            guide_dismissed: prefs.guide_dismissed,
            net_guard_at: 0.0,
            // 界面设置来自家目录 .modelharbor/settings.json（缺省即 App 默认）。
            theme: Theme::from_key(&prefs.theme),
            // 交给第一帧的 apply_theme_if_changed 应用（保证 prefs 里的主题真正生效）
            applied_theme: None,
            save_format: SaveFormat::from_key(&prefs.save_format),
            prefs_saved: prefs.clone(),
            save_format_wheel_latch: false,
            source_format: format,
            config_paths: paths,
            targets: Vec::new(),
            current_page: format,
            sync_wsl: prefs.sync_wsl,
            show_api_keys: prefs.show_api_keys,
            show_preview: false,
            preview_ratio: 0.38,
            preview_focused: false,
            preview_draft: String::new(),
            preview_dirty_at: None,
            preview_edit_at: None,
            preview_parse_ok: true,
            preview_parse_error: None,
            preview_find: String::new(),
            preview_find_active: false,
            preview_find_index: 0,
            preview_find_focus: false,
            preview_find_jump: None,
            preview_cursor_line: 1,
            load_error: None,
            pi_extras: Value::Object(Map::new()),
            backend_icons: Vec::new(),
        };
        app.apply_load();
        app
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 第一件事就是套用主题：启动时（applied_theme == None）与切换主题后都必须走这里，
        // 否则会出现「按钮文字是浅色、界面还是深色」的错配。
        self.apply_theme_if_changed(ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .first()
                .and_then(|f| f.path.as_ref().map(|p| p.to_string_lossy().into_owned()))
        });
        if let Some(path) = dropped {
            self.config_path = path;
            self.reload();
        }
        // 每 5 秒复查一次系统代理 / VPN：用户可能在运行期间开关 Clash 等，
        // 守卫结论用于禁用模型延迟测试（见 netguard 模块说明）。
        let frame_time = ctx.input(|i| i.time);
        if frame_time - self.net_guard_at >= 5.0 {
            self.net_guard = crate::netguard::detect();
            self.net_guard_at = frame_time;
        }
        // 首帧惰性加载各后端官方图标
        if self.backend_icons.is_empty() {
            self.backend_icons = backends::BACKENDS
                .iter()
                .map(|b| {
                    b.icon_rgba().map(|(rgba, w, h)| {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [w as usize, h as usize],
                            rgba,
                        );
                        ctx.load_texture(
                            format!("backend_icon_{}", b.id().label()),
                            image,
                            egui::TextureOptions::LINEAR,
                        )
                    })
                })
                .collect();
        }
        self.poll_model_fetch();
        self.poll_latency();
        self.poll_balance();
        self.persist_prefs_if_changed();
        self.ui_top_bar(ctx);
        self.ui_status_bar(ctx);
        // 右侧配置预览/编辑面板：宽度由 preview_ratio 控制（拖动左边缘分隔条调整），
        // 窗口缩放时按该比例适配；窄窗口下限 220px，并保证组件区至少 320px。
        if self.show_preview {
            let screen_w = ctx.content_rect().width().max(1.0);
            let max_w = (screen_w - 320.0).max(220.0);
            let preview_w = (screen_w * self.preview_ratio).clamp(220.0, max_w);
            let side = egui::SidePanel::right("preview_panel")
                .exact_width(preview_w)
                .show(ctx, |ui| {
                    self.ui_preview_panel(ui);
                });
            // 分隔条用 Foreground 层的独立热区：同层注册会被占满面板的文本框抢走拖拽。
            self.ui_preview_resizer(ctx, side.response.rect, screen_w);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.ui_page_header(ui);
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: false,
                    ..egui::scroll_area::ScrollSource::ALL
                })
                .show(ui, |ui| {
                    ui.add_space(crate::theme::SPACE_1);
                    // Providers 在上、Agents 在下：Agents 只属于 opencode 页面，
                    // 且按需求放在 Providers 下方（只影响界面顺序，不动配置文件里的字段顺序）。
                    self.ui_providers_section(ui);
                    ui.add_space(crate::theme::SPACE_2);
                    if self.current_page == ConfigFormat::Opencode {
                        self.ui_agents_section(ui);
                        ui.add_space(crate::theme::SPACE_2);
                    }
                });
        });
        // 令牌管理：独立悬浮窗（可拖动 / 可关闭），不占正文布局。
        self.ui_tokens_window(ctx);
        self.paint_drag_ghost(ctx);
        // 仅拖拽中显示抓取光标（避免任意控件按下时全局变光标）
        let dragging = self.agent_drag_src.is_some()
            || self.provider_drag_src.is_some()
            || self.model_drag_src.is_some();
        #[cfg(target_os = "windows")]
        crate::cursor::set_custom_cursor_active(dragging);
    }
}

impl App {
    /// 当前界面偏好。
    fn current_prefs(&self) -> crate::prefs::Prefs {
        let mut collapsed: Vec<String> = self.collapsed.iter().cloned().collect();
        collapsed.sort();
        crate::prefs::Prefs {
            show_api_keys: self.show_api_keys,
            save_format: self.save_format.key().to_string(),
            theme: self.theme.key().to_string(),
            sync_wsl: self.sync_wsl,
            config_paths: crate::prefs::ConfigPathPrefs {
                opencode: self.path_override(ConfigFormat::Opencode),
                pi: self.path_override(ConfigFormat::Pi),
                oh_my_pi: self.path_override(ConfigFormat::OhMyPi),
                deepseek_harness: self.path_override(ConfigFormat::DeepSeekHarness),
            },
            collapsed,
            allow_model_test_with_proxy: self.allow_model_test_with_proxy,
            guide_dismissed: self.guide_dismissed,
        }
    }

    /// 某一页「相对默认路径」的覆盖值：与默认相同（或没改过）返回空串，
    /// 这样 prefs 里只留真正手动指定过的路径，默认路径永远跟着自动探测走。
    fn path_override(&self, format: ConfigFormat) -> String {
        let current = self.config_paths.local_path(format);
        let default = ConfigPaths::default_local_path(format);
        if current.trim() == default.trim() {
            String::new()
        } else {
            current
        }
    }

    /// 当前配置文件身份。四个页面共享同一份加载数据，故只按路径分区，不按页面分区。
    fn config_id(&self) -> String {
        let path = if self.loaded_path.trim().is_empty() {
            &self.config_path
        } else {
            &self.loaded_path
        };
        crate::prefs::config_identity(path)
    }

    /// 卡片折叠状态的持久化键（按配置与类别区分）。
    fn card_id(&self, kind: &str, key: &str) -> String {
        crate::prefs::collapsed_id(&self.config_id(), kind, key)
    }

    /// 某 baseUrl 所属站点的面板令牌（没设置则空串）。
    ///
    /// 键是规范化 origin：同一站点的多个 provider 共用一份令牌。
    pub(super) fn station_pat(&self, base_url: &str) -> String {
        let key = crate::tokens::station_key(base_url);
        self.tokens.get(&key).to_string()
    }

    /// 某 baseUrl 所属站点的用户 ID（没设置则空串；空串 = 不发 `New-Api-User`）。
    pub(super) fn station_user_id(&self, base_url: &str) -> String {
        let key = crate::tokens::station_key(base_url);
        self.tokens.user_id(&key).to_string()
    }

    /// provider 卡片是否处于折叠状态。
    pub(super) fn provider_collapsed(&self, key: &str) -> bool {
        self.collapsed.contains(&self.card_id("providers", key))
    }
    /// 设置 provider 卡片折叠状态。
    pub(super) fn set_provider_collapsed(&mut self, key: &str, collapsed: bool) {
        let id = self.card_id("providers", key);
        if collapsed {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
        }
    }

    /// 当前页所有 provider 卡片：折叠或展开。
    pub(super) fn set_all_providers_collapsed(&mut self, collapsed: bool) {
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        for key in keys {
            self.set_provider_collapsed(&key, collapsed);
        }
    }

    /// agent 卡片是否处于折叠状态。
    pub(super) fn agent_collapsed(&self, key: &str) -> bool {
        self.collapsed.contains(&self.card_id("agents", key))
    }

    /// 设置 agent 卡片折叠状态。
    pub(super) fn set_agent_collapsed(&mut self, key: &str, collapsed: bool) {
        let id = self.card_id("agents", key);
        if collapsed {
            self.collapsed.insert(id);
        } else {
            self.collapsed.remove(&id);
        }
    }

    /// 当前页所有 agent 卡片：折叠或展开。
    pub(super) fn set_all_agents_collapsed(&mut self, collapsed: bool) {
        let keys: Vec<String> = self.agents.iter().map(|a| a.key.clone()).collect();
        for key in keys {
            self.set_agent_collapsed(&key, collapsed);
        }
    }

    /// 卡片改名后同步折叠状态（否则改完名卡片会跳回展开）。
    pub(super) fn rename_collapsed_card(&mut self, kind: &str, old: &str, new: &str) {
        let from = self.card_id(kind, old);
        if self.collapsed.remove(&from) {
            self.collapsed.insert(self.card_id(kind, new));
        }
    }

    /// 把 v2 全局折叠键迁到当前首次成功加载的配置身份。
    fn migrate_legacy_collapsed(&mut self) {
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
    fn prune_collapsed(&mut self) {
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

    /// 记住某一页手动指定过的配置路径（空串 = 清除覆盖，回到自动探测值）。
    pub(super) fn remember_page_path(&mut self, format: ConfigFormat, path: &str) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            let default = ConfigPaths::default_local_path(format);
            self.config_paths.set_local_path(format, &default);
        } else {
            self.config_paths.set_local_path(format, trimmed);
        }
    }

    /// 主题变化（含首次启动）时重新套用样式。
    fn apply_theme_if_changed(&mut self, ctx: &egui::Context) {
        if crate::theme::needs_apply(&mut self.applied_theme, self.theme) {
            self.theme.apply(ctx);
        }
    }

    /// 界面设置变了就落盘（家目录 `.modelharbor/settings.json`）。
    fn persist_prefs_if_changed(&mut self) {
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
    fn refresh_targets(&mut self) {
        // 默认目标固定为 Windows 本地路径；WSL 侧仅通过“WSL同步”勾选写入，
        // 且写入前按页面检测对应 agent 是否已安装。
        self.targets = backends::BACKENDS
            .iter()
            .map(|b| {
                let id = b.id();
                let local = self.config_paths.local_path(id);
                SaveTarget {
                    backend: id,
                    available: self.config_paths.validate_target(id),
                    path: local,
                }
            })
            .collect();
    }

    /// 按 source_format 加载当前 config_path；失败时置空数据并记录 load_error。
    fn apply_load(&mut self) {
        let path = self.config_path.clone();
        self.loaded_path = path.clone();
        let result = backends::load_backend(self.source_format, &path);
        match result {
            Ok(load) => {
                self.root = load.root;
                self.agents = load.agents;
                self.providers = load.providers;
                self.pi_extras = load.extras;
                self.load_error = None;
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

    fn reload(&mut self) {
        self.reload_for_page(self.current_page, true);
    }

    /// 重新加载路径，同时把“路径属于哪个页面”和“文件实际是什么格式”分开。
    ///
    /// 只有成功读出并解析文件后才记住路径；不存在的 `.json` 即使探测回落到
    /// opencode，也不会污染任何页面的持久化覆盖。实际格式与发起页面不同时，
    /// 页面跟随文件格式切换，路径只记到检测出的格式。
    fn reload_for_page(&mut self, owner: ConfigFormat, remember: bool) {
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
    pub(super) fn reset_page_path(&mut self, format: ConfigFormat) {
        self.remember_page_path(format, "");
        self.config_path = self.config_paths.target_path(format);
        self.reload_for_page(format, false);
    }

    /// 站点面板令牌的悬浮窗外壳（内容见 `ui_tokens_panel`）。
    ///
    /// 用独立窗口而不是内联面板：它只在配置令牌时用一下，没必要长期占着正文空间。
    /// 窗口被限制在主窗口内（不允许拖出去后看不到），高度固定、内容超出用滚轮。
    fn ui_tokens_window(&mut self, ctx: &egui::Context) {
        if !self.show_tokens {
            return;
        }
        // 固定高度：站点多了也不让窗口无限撑高，内容交给内部滚动区。
        // 上限取当前窗口高度减去边距，避免在矮窗口下把按钮顶到屏幕外。
        let area = ctx.content_rect();
        let height = (area.height() - 140.0).clamp(240.0, 520.0);
        // `.open()` 要借一个局部变量：直接传 `&mut self.show_tokens`
        // 会与闭包里的 `&mut self` 冲突。
        let mut open = true;
        egui::Window::new("站点面板令牌")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fixed_size([620.0, height])
            // 不允许拖到主窗口外：拖出去后标题栏可能落到屏幕外，窗口就找不回来了。
            .constrain_to(area)
            .default_pos(egui::pos2(area.left() + 90.0, area.top() + 90.0))
            .frame({
                // 悬浮窗用比卡片更大的圆角与内边距，与主界面分层。
                let mut frame = egui::Frame::window(&ctx.style());
                frame.corner_radius = crate::theme::RADIUS_LG.into();
                frame.inner_margin = egui::Margin::same(crate::theme::SPACE_4 as i8);
                frame
            })
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    // 撑满固定高度（不随内容缩），滚动条才是“内容超出才出现”。
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.ui_tokens_panel(ui);
                    });
            });
        if !open {
            self.show_tokens = false;
        }
    }

    fn paint_drag_ghost(&self, ctx: &egui::Context) {
        let label = if let Some(k) = &self.agent_drag_src {
            self.agents
                .iter()
                .find(|a| &a.key == k)
                .map(|a| a.key.as_str())
                .unwrap_or("")
        } else if let Some(k) = &self.provider_drag_src {
            self.providers
                .iter()
                .find(|p| &p.key == k)
                .map(|p| p.key.as_str())
                .unwrap_or("")
        } else if let Some(k) = &self.model_drag_src {
            k.split_once('\u{1f}')
                .map(|(_, model)| model)
                .unwrap_or(k.as_str())
        } else {
            return;
        };
        if label.is_empty() {
            return;
        }
        let Some(pointer) = ctx.pointer_hover_pos() else {
            return;
        };
        let layer_id = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
        let painter = ctx.layer_painter(layer_id);
        let visuals = &ctx.style().visuals;
        let font_id = egui::FontId::proportional(14.0);
        let text_color = visuals.text_color();
        let bg = visuals.faint_bg_color;
        let stroke_color = visuals.widgets.noninteractive.bg_stroke.color;
        let galley = painter.layout(
            label.to_string(),
            font_id.clone(),
            text_color,
            f32::INFINITY,
        );
        let label_w = galley.size().x;
        let size = egui::vec2(label_w + 24.0, 30.0);
        let ghost_rect = egui::Rect::from_min_size(pointer + egui::vec2(20.0, -15.0), size);
        painter.rect_filled(ghost_rect, 8.0, bg);
        painter.rect_stroke(
            ghost_rect,
            8.0,
            egui::Stroke::new(1.0, stroke_color),
            egui::StrokeKind::Inside,
        );
        painter.text(
            ghost_rect.left_center() + egui::vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            font_id,
            text_color,
        );
    }

    /// 按格式取官方图标纹理（图标未加载时返回 None）。
    fn icon_for(&self, fmt: ConfigFormat) -> Option<&egui::TextureHandle> {
        let idx = backends::BACKENDS.iter().position(|b| b.id() == fmt)?;
        self.backend_icons.get(idx).and_then(|o| o.as_ref())
    }
}

// ---------- 兼容再导出：保持既有外部路径（测试与 backends）可用 ----------
pub use crate::backends::opencode::merge_opencode_root;
pub use crate::util::parse_config_content;

#[cfg(test)]
mod tests;
