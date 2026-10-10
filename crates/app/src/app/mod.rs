use crate::backends;
use crate::format::{ConfigFormat, ConfigPaths};
use crate::model::{AgentRow, ProviderRow};
use crate::theme::Theme;
use eframe::egui;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

mod agents;
mod balance;
pub(crate) mod bars;
mod collapse;
mod diff;
mod drag;
mod fetch;
mod health;
mod ids;
mod lifecycle;
mod preview;
mod providers;
mod providers_form;
mod save;
mod syntax;
pub(in crate::app) mod usage;
mod windows;

use fetch::{FreeModelsState, LatencyState, ProbeGate};
use save::SaveTarget;

pub use save::{load_opencode_result, load_or_empty, strip_cross_format_containers};

/// 选择配置文件（JSON / YAML / TOML）。依赖 rfd，属 UI，故在 app 而非 core。
pub(crate) fn show_file_dialog() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("选择配置文件")
        .add_filter(
            "配置文件（JSON / YAML / TOML）",
            crate::util::CONFIG_FILE_EXTENSIONS,
        )
        .add_filter("JSON", &["json", "jsonc"])
        .add_filter("YAML", &["yml", "yaml"])
        .add_filter("TOML", &["toml"])
        .add_filter("所有文件", &["*"])
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SaveFormat {
    Current,
    #[default]
    Compact,
}

impl SaveFormat {
    /// 切到另一种保存格式。
    fn toggled(self) -> Self {
        match self {
            SaveFormat::Current => SaveFormat::Compact,
            SaveFormat::Compact => SaveFormat::Current,
        }
    }

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

/// 中央区域显示哪一块：用量统计还是提供商管理。
///
/// 这是**视图**切换，与顶栏的 `current_page`（agent 页签）正交：
/// 换页签不改变当前视图，换视图也不改变当前页签。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum MainView {
    /// 提供商管理（既有界面，默认）。
    #[default]
    Providers,
    /// 本机用量四维统计。
    Usage,
}

impl MainView {
    fn label(self) -> &'static str {
        match self {
            Self::Providers => "提供商管理",
            Self::Usage => "用量",
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
    /// 最近一次实际加载的路径；与 `config_path` 不等时按“未加载”处理。
    loaded_path: String,
    status: String,
    show_new_agent: bool,
    show_new_provider: bool,
    /// 已折叠的卡片（`页面/类别/名字`，见 `prefs::collapsed_id`）。
    /// 新加载进来的卡片默认展开。
    collapsed: HashSet<String>,
    variant_open: HashSet<String>,
    agent_drag_src: Option<String>,
    agent_drag_target: Option<String>,
    provider_drag_src: Option<String>,
    provider_drag_target: Option<String>,
    model_drag_src: Option<String>,
    model_drag_target: Option<String>,
    /// 正在拖动的顶栏页面（已安装的那一段内换位）。
    tab_drag_src: Option<ConfigFormat>,
    /// 每个 provider 的「探测模型」状态（key → 状态；存在即结果面板开着）。
    discovery: HashMap<String, fetch::DiscoveryState>,
    /// 探测结果缓存：24 小时内同 base_url 直接命中，不再发请求（不含任何凭据）。
    discovery_cache: model_harbor_core::discovery::DiscoveryCache,
    /// 各后端内置网关的免费模型（后端 → 状态）。
    ///
    /// opencode 与 kilocode 各有自己的网关与列表，mimocode 没有免费层（不进这张表）。
    /// 启动先用落盘缓存填充，缓存缺失或过期才在后台重新拉取。
    free_models: HashMap<ConfigFormat, FreeModelsState>,
    /// 各 opencode 系页面**各自**的 agent model 视图记忆：
    /// **（文件身份，页面） → agent key → model**。
    ///
    /// 只在**离开**页面时写入；当前页的权威值永远是 `agents` 本身。
    /// 键带 [`App::config_id`]（文件身份），换文件后旧记忆失配即跳过。
    agent_models_by_page: HashMap<(String, ConfigFormat), HashMap<String, String>>,
    /// 首帧需要自动后台拉取的后端（缓存缺失 / 过期）；拉过即清空。
    free_models_auto: Vec<ConfigFormat>,
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
    /// 令牌面板里「用户 ID」的草稿（`New-Api-User` 头，可留空）。
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
    /// 配置体检悬浮窗是否打开（见 [`crate::app::health`]）。
    show_health: bool,
    /// 中央区域当前显示哪一块（用量 / 提供商管理）。
    main_view: MainView,
    /// 用量视图内的维度页签（Agent / 模型 / 会话 / 时间）。
    usage_dimension: usage::aggregate::Dimension,
    /// 用量视图的时间区间（今日 / 本周 / 本月 / 全部）。
    usage_range: usage::range::Range,
    /// 本机用量扫描状态（见 [`crate::app::usage`]）。
    usage: usage::UsageState,
    /// 界面形状预设（圆角默认值 + 描边宽度）。
    ui_style: crate::theme::UiStyle,
    /// 亚克力：面板 / 卡片半透明 + DWM 亚克力模糊（正交于主题与形状）。
    glass: bool,
    /// 顶栏已安装页面的拖动顺序（后端标识；未列出的按名字首字母补在其后）。
    tab_order: Vec<String>,
    /// 上次网络守卫检测时刻（egui 秒）。
    net_guard_at: f64,
    theme: Theme,
    /// 已应用到 egui 的主题（主题 + 形状 + 亚克力）。
    applied_theme: Option<(Theme, crate::theme::UiStyle, bool)>,
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
    /// 已落盘的界面偏好快照；与当前值不同就写盘。
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
    /// 用户最近一次在预览框内输入的帧时间。
    preview_edit_at: Option<f64>,
    /// 最近一次文本解析是否成功；失败时不写盘、不覆盖文本。
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
    /// 对比视图：显示「磁盘上的目标文件 → 待保存文档」的逐行改动。
    ///
    /// 只读视图：对比模式下不渲染文本框。
    preview_diff_mode: bool,
    /// 对比结果缓存（签名 → 显示行与统计）。
    preview_diff_cache: Option<(u64, Vec<diff::DiffLine>, diff::DiffSummary)>,
    /// 待重算的对比签名与它首次出现的时刻（见 `ui_preview_diff` 的防抖）。
    preview_diff_pending: Option<(u64, f64)>,
    /// 成功写盘的次数；并进对比缓存的签名，写盘后缓存作废。
    save_serial: u64,
    /// 光标所在行（1-based；失焦时保留最后位置）。
    preview_cursor_line: usize,
    load_error: Option<String>,
    pi_extras: Value,
    /// 各后端官方图标纹理（与 BACKENDS 顺序对齐，首帧惰性加载）。
    backend_icons: Vec<Option<egui::TextureHandle>>,
    /// 顶部工具栏图标纹理（Lucide SVG 栅格化，首帧惰性加载）。
    toolbar_icons: ToolbarIcons,
    /// 各目标文件最近一次自动快照的内容 hash（路径 → hash），供「外部改动」判定。
    snapshot_hashes: HashMap<String, String>,
    /// 自动快照目录覆盖（单测注入 tempdir 用；None = 配置目录下的 backups/）。
    snapshots_root: Option<std::path::PathBuf>,
}

/// 启动时的方言判定：以**文件内容**为准，路径所属页面只作回退。
///
/// 文件读不出内容时保持页面推断。
fn startup_format(owner: ConfigFormat, path: &str) -> ConfigFormat {
    let path = path.trim();
    if path.is_empty() {
        return owner;
    }
    let readable = if crate::util::is_wsl_path(path) {
        crate::util::read_wsl_file(path).is_ok()
    } else {
        std::fs::read_to_string(path).is_ok()
    };
    if !readable {
        return owner;
    }
    ConfigPaths::detect_for_path(path).0
}

impl Default for App {
    fn default() -> Self {
        let prefs = crate::prefs::Prefs::load();
        // 路径：先套用 prefs 里用户手动指定过的覆盖，再定位要打开的页面。
        let mut paths = ConfigPaths::default();
        paths.apply_overrides(&prefs.config_paths);
        let (format, path) = paths
            .detect_preferring_overrides(&prefs.config_paths)
            .unwrap_or((ConfigFormat::Opencode, String::new()));
        // 启动探测只按「哪一页有覆盖路径」选后端；方言以文件内容为准。
        let format = startup_format(format, &path);
        // 各后端内置网关的免费模型：逐后端读一次落盘缓存；缺失或过期的后端
        // 由首帧在后台重取。只对确实有免费层的后端建状态。
        let mut free_models: HashMap<ConfigFormat, FreeModelsState> = HashMap::new();
        let mut free_models_auto: Vec<ConfigFormat> = Vec::new();
        for format in [
            ConfigFormat::Opencode,
            ConfigFormat::Kilocode,
            ConfigFormat::Mimocode,
        ] {
            if crate::opencode_models::source_for(format).is_none() {
                continue;
            }
            match crate::opencode_models::load_cache(format) {
                Some(cache) => {
                    if !cache.fresh {
                        free_models_auto.push(format);
                    }
                    free_models.insert(
                        format,
                        FreeModelsState {
                            models: cache.models,
                            ..FreeModelsState::default()
                        },
                    );
                }
                None => {
                    free_models_auto.push(format);
                    free_models.insert(format, FreeModelsState::default());
                }
            }
        }
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
            tab_drag_src: None,
            provider_drag_target: None,
            model_drag_src: None,
            model_drag_target: None,
            discovery: HashMap::new(),
            discovery_cache: model_harbor_core::discovery::DiscoveryCache::default(),
            // 免费模型：先用落盘缓存，缺失 / 过期由第一帧的后台刷新补上。
            free_models,
            free_models_auto,
            agent_models_by_page: HashMap::new(),
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
            show_health: false,
            main_view: MainView::default(),
            usage_dimension: usage::aggregate::Dimension::default(),
            usage_range: usage::range::Range::default(),
            usage: usage::UsageState::default(),
            ui_style: crate::theme::UiStyle::from_key(&prefs.ui_style),
            glass: prefs.glass,
            tab_order: prefs.tab_order.clone(),
            net_guard_at: 0.0,
            // 界面设置来自家目录 .modelharbor/settings.json。
            theme: Theme::from_key(&prefs.theme),
            // 交给第一帧的 apply_theme_if_changed 应用
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
            preview_diff_mode: false,
            preview_diff_cache: None,
            preview_diff_pending: None,
            save_serial: 0,
            preview_cursor_line: 1,
            load_error: None,
            pi_extras: Value::Object(Map::new()),
            backend_icons: Vec::new(),
            toolbar_icons: ToolbarIcons::default(),
            snapshot_hashes: HashMap::new(),
            snapshots_root: None,
        };
        // WSL 总闸与同步勾选同步初始化。
        crate::util::wsl_set_enabled(prefs.sync_wsl);
        app.apply_load();
        app
    }
}

impl eframe::App for App {
    /// 窗口清屏色：亚克力档返回透明，非亚克力档返回主题面板色。
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.glass {
            egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 第一件事就是套用主题：启动时与切换主题后都走这里。
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
        // 每 5 秒复查一次系统代理 / VPN，结论用于禁用模型延迟测试。
        let frame_time = ctx.input(|i| i.time);
        if frame_time - self.net_guard_at >= 5.0 {
            self.net_guard = crate::netguard::detect();
            self.net_guard_at = frame_time;
        }
        // 首帧惰性加载各后端官方图标和顶部工具栏图标
        self.load_backend_icons(ctx);
        self.load_toolbar_icons(ctx);
        self.poll_discovery();
        self.poll_latency();
        self.poll_balance();
        // 本机用量：后台扫描 + 3 秒增量检测（空闲时不扫）。
        self.poll_usage(ctx);
        self.poll_free_models();
        // 首次启动 / 缓存过期：只自动拉一次（拉过即清空），用户可手动重取。
        for format in std::mem::take(&mut self.free_models_auto) {
            self.start_free_models_fetch(format);
        }
        self.persist_prefs_if_changed();
        self.ui_top_bar(ctx);
        self.ui_status_bar(ctx);
        // 右侧配置预览/编辑面板：宽度由 preview_ratio 控制，窗口缩放时按该比例适配；
        // 下限 220px，组件区至少 320px。`show_animated` 在打开 / 隐藏期间保留侧栏
        // 占位并补间宽度；动画结束后才挂载真实预览内容。
        let screen_w = ctx.content_rect().width().max(1.0);
        let max_w = (screen_w - 320.0).max(220.0);
        let preview_w = (screen_w * self.preview_ratio).clamp(220.0, max_w);
        let side = egui::SidePanel::right("preview_panel")
            .exact_width(preview_w)
            .show_animated(ctx, self.show_preview, |ui| {
                self.ui_preview_panel(ui);
            });
        if let Some(side) = side {
            // 分隔条注册在 Foreground 层的独立热区。
            self.ui_preview_resizer(ctx, side.response.rect, screen_w);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.ui_page_header(ui);
            // 视图切换（用量 / 提供商管理）：两个视图二选一显示在同一位置。
            self.ui_view_switch(ui);
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: false,
                    ..egui::scroll_area::ScrollSource::ALL
                })
                .show(ui, |ui| match self.main_view {
                    MainView::Providers => {
                        // Providers 在上、Agents 在下；Agents 只属于 opencode
                        // 页面，且只影响界面顺序。
                        self.ui_providers_section(ui);
                        ui.add_space(crate::theme::SPACE_2);
                        if self.current_page.is_opencode_family() {
                            self.ui_agents_section(ui);
                            ui.add_space(crate::theme::SPACE_2);
                        }
                    }
                    // 用量视图与当前页签无关：它统计的是本机全部 agent。
                    MainView::Usage => self.ui_usage_section(ui),
                });
        });
        // 令牌管理：独立悬浮窗（可拖动 / 可关闭），不占正文布局。
        self.ui_tokens_window(ctx);
        // 配置体检：同样是独立悬浮窗，保存前想核对一遍时打开。
        self.ui_health_window(ctx);
        self.paint_drag_ghost(ctx);
        // 抓取光标：控件绘制时只提出请求，这里帧末统一提交。
        //
        // 拖动中即便没有任何热区报状态，也保持拳头。
        let mut want = crate::ui::take_grab_cursor(ctx);
        if self.is_dragging_anything() {
            want = crate::ui::GrabCursor::Fist;
        }
        #[cfg(target_os = "windows")]
        crate::cursor::set_custom_cursor(match want {
            crate::ui::GrabCursor::None => crate::cursor::CustomCursor::None,
            crate::ui::GrabCursor::Palm => crate::cursor::CustomCursor::Palm,
            crate::ui::GrabCursor::Fist => crate::cursor::CustomCursor::Fist,
        });
        #[cfg(not(target_os = "windows"))]
        let _ = want;
    }
}

#[derive(Default)]
struct ToolbarIcons {
    panel_right: Option<egui::TextureHandle>,
    eye: Option<egui::TextureHandle>,
    eye_off: Option<egui::TextureHandle>,
    key_round: Option<egui::TextureHandle>,
    activity: Option<egui::TextureHandle>,
    palette: Option<egui::TextureHandle>,
    folder_open: Option<egui::TextureHandle>,
    unfold: Option<egui::TextureHandle>,
    database: Option<egui::TextureHandle>,
    globe: Option<egui::TextureHandle>,
    save: Option<egui::TextureHandle>,
    layers: Option<egui::TextureHandle>,
    file_output: Option<egui::TextureHandle>,
    bot: Option<egui::TextureHandle>,
    server: Option<egui::TextureHandle>,
    zap: Option<egui::TextureHandle>,
}

impl App {
    /// 惰性加载各后端官方图标（首帧一次）。
    fn load_backend_icons(&mut self, ctx: &egui::Context) {
        if !self.backend_icons.is_empty() {
            return;
        }
        self.backend_icons = backends::BACKENDS
            .iter()
            .map(|b| {
                b.icon_rgba().map(|(rgba, w, h)| {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba);
                    ctx.load_texture(
                        format!("backend_icon_{}", b.id().label()),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                })
            })
            .collect();
    }

    /// 惰性加载顶部工具栏图标：SVG 资源栅格化后交给 egui 纹理系统。
    fn load_toolbar_icons(&mut self, ctx: &egui::Context) {
        if self.toolbar_icons.panel_right.is_some() {
            return;
        }
        let options = &Default::default();
        let load = |name: &str, svg: &[u8]| {
            let image = egui_extras::image::load_svg_bytes_with_size(
                svg,
                egui::SizeHint::Size {
                    width: 20,
                    height: 20,
                    maintain_aspect_ratio: true,
                },
                options,
            )
            .ok()?;
            Some(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
        };
        self.toolbar_icons.panel_right = load(
            "toolbar_icon_panel_right",
            include_bytes!("../../assets/icons/panel-right.svg"),
        );
        self.toolbar_icons.eye = load(
            "toolbar_icon_eye",
            include_bytes!("../../assets/icons/eye.svg"),
        );
        self.toolbar_icons.eye_off = load(
            "toolbar_icon_eye_off",
            include_bytes!("../../assets/icons/eye-off.svg"),
        );
        self.toolbar_icons.key_round = load(
            "toolbar_icon_key_round",
            include_bytes!("../../assets/icons/key-round.svg"),
        );
        self.toolbar_icons.activity = load(
            "toolbar_icon_activity",
            include_bytes!("../../assets/icons/activity.svg"),
        );
        self.toolbar_icons.palette = load(
            "toolbar_icon_palette",
            include_bytes!("../../assets/icons/palette.svg"),
        );
        self.toolbar_icons.folder_open = load(
            "toolbar_icon_folder_open",
            include_bytes!("../../assets/icons/folder-open.svg"),
        );
        self.toolbar_icons.save = load(
            "toolbar_icon_save",
            include_bytes!("../../assets/icons/save.svg"),
        );
        self.toolbar_icons.layers = load(
            "toolbar_icon_layers",
            include_bytes!("../../assets/icons/layers.svg"),
        );
        self.toolbar_icons.file_output = load(
            "toolbar_icon_file_output",
            include_bytes!("../../assets/icons/file-output.svg"),
        );
        self.toolbar_icons.bot = load(
            "toolbar_icon_bot",
            include_bytes!("../../assets/icons/bot.svg"),
        );
        self.toolbar_icons.server = load(
            "toolbar_icon_server",
            include_bytes!("../../assets/icons/server.svg"),
        );
        self.toolbar_icons.zap = load(
            "toolbar_icon_zap",
            include_bytes!("../../assets/icons/zap.svg"),
        );
        self.toolbar_icons.unfold = load(
            "toolbar_icon_unfold",
            include_bytes!("../../assets/icons/unfold.svg"),
        );
        self.toolbar_icons.database = load(
            "toolbar_icon_database",
            include_bytes!("../../assets/icons/database.svg"),
        );
        self.toolbar_icons.globe = load(
            "toolbar_icon_globe",
            include_bytes!("../../assets/icons/globe.svg"),
        );
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
                kilocode: self.path_override(ConfigFormat::Kilocode),
                mimocode: self.path_override(ConfigFormat::Mimocode),
                pi: self.path_override(ConfigFormat::Pi),
                oh_my_pi: self.path_override(ConfigFormat::OhMyPi),
                deepseek_harness: self.path_override(ConfigFormat::DeepSeekHarness),
                zcode: self.path_override(ConfigFormat::ZCode),
                workbuddy: self.path_override(ConfigFormat::WorkBuddy),
                qwen_code: self.path_override(ConfigFormat::QwenCode),
                kimi_code: self.path_override(ConfigFormat::KimiCode),
            },
            collapsed,
            allow_model_test_with_proxy: self.allow_model_test_with_proxy,
            guide_dismissed: self.guide_dismissed,
            ui_style: self.ui_style.key().to_string(),
            glass: self.glass,
            tab_order: self.tab_order.clone(),
        }
    }

    /// 站点面板令牌悬浮窗所在的层级。
    ///
    /// 用 `Foreground`：高于预览分隔条所在的 `Middle`，又低于 `Tooltip`。
    const TOKENS_WINDOW_ORDER: egui::Order = egui::Order::Foreground;

    /// 按格式取官方图标纹理（图标未加载时返回 None）。
    fn icon_for(&self, fmt: ConfigFormat) -> Option<&egui::TextureHandle> {
        let idx = backends::BACKENDS.iter().position(|b| b.id() == fmt)?;
        self.backend_icons.get(idx).and_then(|o| o.as_ref())
    }
}

// ---------- 兼容再导出：保持既有外部路径可用 ----------
pub use crate::backends::opencode::merge_opencode_root;
pub use crate::util::parse_config_content;

#[cfg(test)]
mod tests;
