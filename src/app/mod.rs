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
mod diff;
mod fetch;
mod health;
mod preview;
mod providers;
mod providers_form;
mod save;
mod serialize;
mod syntax;

use fetch::{FreeModelsState, LatencyState, ModelFetchState, ProbeGate};
use save::SaveTarget;

pub use save::{load_opencode_result, load_or_empty, strip_cross_format_containers};
pub(crate) use serialize::{compact_json, pretty_json};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SaveFormat {
    Current,
    #[default]
    Compact,
}

impl SaveFormat {
    /// 切到另一种保存格式（滚轮与点击共用同一翻转，别处不得另写一份）。
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
    /// 正在拖动的顶栏页面（已安装的那一段内换位）。
    tab_drag_src: Option<ConfigFormat>,
    /// 每个 provider 的模型获取状态（key → 状态）。
    model_fetch: HashMap<String, ModelFetchState>,
    /// 已展开的模型获取面板（provider key）。
    model_fetch_open: HashSet<String>,
    /// 各后端内置网关的免费模型（后端 → 状态）。
    ///
    /// 动态拉取而非写死：免费层会随上游上下架，写死的 id 迟早变成「选了却跑不起来」
    /// 的过期项（见 [`crate::opencode_models`]）。opencode 与 kilocode 各有自己的
    /// 网关与列表，mimocode 没有免费层（不进这张表）。启动先用落盘缓存填充，
    /// 缓存缺失或过期才在后台重新拉取。
    free_models: HashMap<ConfigFormat, FreeModelsState>,
    /// 各 opencode 系页面**各自**的 agent model 视图（页面 → agent key → model）。
    ///
    /// 为什么需要按页记忆、切页时怎么归一，见 [`crate::app::agents`] 的切页归一说明；
    /// 这里只记字面语义：只在**离开**页面时写入，当前页的权威值永远是 `agents` 本身
    /// （用户可能正在编辑）。
    agent_models_by_page: HashMap<ConfigFormat, HashMap<String, String>>,
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
    /// 配置体检悬浮窗是否打开（见 [`crate::app::health`]）。
    show_health: bool,
    /// 界面形状预设（圆角默认值 + 描边宽度）。
    ui_style: crate::theme::UiStyle,
    /// 顶栏已安装页面的拖动顺序（后端标识；未列出的按名字首字母补在其后）。
    tab_order: Vec<String>,
    /// 上次网络守卫检测时刻（egui 秒）。
    net_guard_at: f64,
    theme: Theme,
    /// 已应用到 egui 的主题：egui 0.33 的 `set_style` 是**每个主题各存一份 style**，
    /// 所以主题一变就必须显式再 `apply` 一次，否则只有按钮文字变、界面颜色不跟着变。
    applied_theme: Option<(Theme, crate::theme::UiStyle)>,
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
    /// 对比视图显示「磁盘上的目标文件 → 待保存文档」的逐行改动。
    ///
    /// 只读视图：对比模式下不渲染文本框，因此不存在「手改被对比覆盖」的问题，
    /// 切回编辑模式时草稿仍是原样。
    preview_diff_mode: bool,
    /// 对比结果缓存（签名 → 显示行与统计），避免每帧重算 LCS。
    preview_diff_cache: Option<(u64, Vec<diff::DiffLine>, diff::DiffSummary)>,
    /// 待重算的对比签名与它首次出现的时刻（见 `ui_preview_diff` 的防抖说明）。
    preview_diff_pending: Option<(u64, f64)>,
    /// 成功写盘的次数。对比视图把它并进缓存签名：写盘后磁盘内容变了，
    /// 缓存必须作废，否则会继续显示一份已经落盘的「改动」。
    save_serial: u64,
    /// 光标所在行（1-based；失焦时保留最后位置）。
    preview_cursor_line: usize,
    load_error: Option<String>,
    pi_extras: Value,
    /// 各后端官方图标纹理（与 BACKENDS 顺序对齐，首帧惰性加载）。
    backend_icons: Vec<Option<egui::TextureHandle>>,
}

/// 启动时的方言判定：以**文件内容**为准，路径所属页面只作回退。
///
/// `detect_preferring_overrides` 只回答「哪一页填了覆盖路径且文件存在」，
/// 不看内容。用户把 opencode.json 填到 pi 页时，用 pi 方言去读会解析出
/// 0 条 provider——表现为「写了路径却不自动加载」。这里按内容纠正，
/// 与手动加载（`reload_for_page`）用同一套判定。
///
/// **文件读不出内容时保持页面推断**：`detect_for_path` 在无内容时按扩展名
/// 回落（`.json` → opencode），据此改判会在文件缺失 / 无权限时把页面
/// 误判成 opencode，并可能把错误路径记进持久化覆盖。
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
        // 路径：先套用 prefs 里用户手动指定过的覆盖，再定位要打开的页面
        // （覆盖过且文件存在的页面优先，其次才按默认路径自动探测）。
        let mut paths = ConfigPaths::default();
        paths.apply_overrides(&prefs.config_paths);
        let (format, path) = paths
            .detect_preferring_overrides(&prefs.config_paths)
            .unwrap_or((ConfigFormat::Opencode, String::new()));
        // 启动探测只按「哪一页有覆盖路径」选后端；方言以文件内容为准
        // （为什么见 `startup_format` 文档）。
        let format = startup_format(format, &path);
        // 各后端内置网关的免费模型：逐后端读一次落盘缓存，界面先用它渲染；
        // 缓存缺失或过期的后端由首帧自动在后台重取（不阻塞启动）。
        // 只对确实有免费层的后端建状态（见 `opencode_models::source_for`）。
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
            model_fetch: HashMap::new(),
            model_fetch_open: HashSet::new(),
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
            ui_style: crate::theme::UiStyle::from_key(&prefs.ui_style),
            tab_order: prefs.tab_order.clone(),
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
            preview_diff_mode: false,
            preview_diff_cache: None,
            preview_diff_pending: None,
            save_serial: 0,
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
        self.load_backend_icons(ctx);
        self.poll_model_fetch();
        self.poll_latency();
        self.poll_balance();
        self.poll_free_models();
        // 首次启动 / 缓存过期：只自动拉一次（拉过即清空，失败也不反复重试，
        // 用户可在 Agents 的模型下拉旁点「刷新」手动重取）。
        for format in std::mem::take(&mut self.free_models_auto) {
            self.start_free_models_fetch(format);
        }
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
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: false,
                    ..egui::scroll_area::ScrollSource::ALL
                })
                .show(ui, |ui| {
                    // 顶部不留空白：Providers 吸顶条一滚就会贴到滚动区可视顶。
                    // 多出 4px 时，标题会从内容流位置跳到可视顶，看起来整行往上抬。
                    // （egui 还会把裁剪顶上扩 3px，见 bars::sticky_y 的说明。）
                    // Providers 在上、Agents 在下：Agents 只属于 opencode 页面，
                    // 且按需求放在 Providers 下方（只影响界面顺序，不动配置文件里的字段顺序）。
                    self.ui_providers_section(ui);
                    ui.add_space(crate::theme::SPACE_2);
                    if self.current_page.is_opencode_family() {
                        self.ui_agents_section(ui);
                        ui.add_space(crate::theme::SPACE_2);
                    }
                });
        });
        // 令牌管理：独立悬浮窗（可拖动 / 可关闭），不占正文布局。
        self.ui_tokens_window(ctx);
        // 配置体检：同样是独立悬浮窗，保存前想核对一遍时打开。
        self.ui_health_window(ctx);
        self.paint_drag_ghost(ctx);
        // 抓取光标：控件在各自绘制时只「提出请求」（悬停=手掌、按住=拳头），
        // 这里帧末统一提交，同一帧只碰一次系统光标。
        //
        // 兜底：拖动中即便没有任何热区报状态（指针已拖离原把手、又悬在空白处），
        // 也必须保持拳头——否则光标会退回箭头，看着像「拖丢了」。
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

impl App {
    /// 是否有任意拖动源正在拖拽。
    ///
    /// 页签（后端图标）也是拖动源：它此前漏在这一组之外，于是拖卡片是抓取光标、
    /// 拖图标却退回系统手型。自定义抓取光标是整窗生效的（子类过程拦 WM_SETCURSOR），
    /// 只要拖拽期间置位，对所有控件一视同仁。
    fn is_dragging_anything(&self) -> bool {
        self.agent_drag_src.is_some()
            || self.provider_drag_src.is_some()
            || self.model_drag_src.is_some()
            || self.tab_drag_src.is_some()
    }

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
            },
            collapsed,
            allow_model_test_with_proxy: self.allow_model_test_with_proxy,
            guide_dismissed: self.guide_dismissed,
            ui_style: self.ui_style.key().to_string(),
            tab_order: self.tab_order.clone(),
        }
    }

    /// 某一页「相对默认路径」的覆盖值：与默认相同（或没改过）返回空串，
    /// 这样 prefs 里只留真正手动指定过的路径，默认路径永远跟着自动探测走。
    ///
    /// 比较基准用**解析后的默认路径**（`resolve_local_path`）：自动落到 `.jsonc`
    /// 变体上只是探测结果，不是用户的选择，不该被当成手动覆盖记进 settings.json——
    /// 否则 CLI 之后把文件改名成 `.json`，这条覆盖就指向一个不存在的路径了。
    fn path_override(&self, format: ConfigFormat) -> String {
        let current = self.config_paths.local_path(format);
        let default =
            backends::resolve_local_path(format, &ConfigPaths::default_local_path(format));
        if current.trim() == default.trim() {
            String::new()
        } else {
            current
        }
    }

    /// 当前配置文件身份。各页面共享同一份加载数据，故只按路径分区，不按页面分区。
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

    /// 主题 / 形状任一变化（含首次启动）时重新套用样式。
    fn apply_theme_if_changed(&mut self, ctx: &egui::Context) {
        let shape = self.ui_style;
        if crate::theme::needs_apply_style(&mut self.applied_theme, self.theme, shape) {
            self.theme.apply_style(ctx, shape);
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
    fn apply_load(&mut self) {
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
        // 各页的 agent model 视图是「上一个文件」的，键（agent 名）可能已经不存在，
        // 留着会让下次切页把陈旧的值覆盖到新文件上。
        self.agent_models_by_page.clear();
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

    /// 站点面板令牌悬浮窗所在的层级。
    ///
    /// 用 `Foreground`：高于预览分隔条所在的 `Middle`，分割线不会横穿悬浮窗；
    /// 又低于 `Tooltip`，悬停提示仍显示在窗上面。
    const TOKENS_WINDOW_ORDER: egui::Order = egui::Order::Foreground;

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
        // 始终居中：`default_pos` 只在首次生效（之后记住用户拖过的位置），
        // 要每帧都居中得用 `current_pos` 指定。窗口尺寸固定，居中可以
        // 直接由内容区算出左上角。
        let size = egui::vec2(620.0, height);
        let centered = area.center() - size / 2.0;
        egui::Window::new("站点面板令牌")
            // 提到 Foreground：预览分隔条在 Middle 层，窗在它上面，
            // 分割线不会横穿悬浮窗。
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .fixed_size(size)
            // 不允许拖到主窗口外：拖出去后标题栏可能落到屏幕外，窗口就找不回来了。
            .constrain_to(area)
            .current_pos(centered)
            .frame(self.floating_window_frame(ctx))
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

    /// 配置体检悬浮窗：把各类检查汇总成一张清单（内容见 [`crate::app::health`]）。
    ///
    /// 只读清单，**不提供一键修复**：这些问题的正确修法取决于用户意图
    /// （重复的 model id 该留哪条、可疑的 baseUrl 该不该改），自动改就是替用户做决定。
    fn ui_health_window(&mut self, ctx: &egui::Context) {
        if !self.show_health {
            return;
        }
        let area = ctx.content_rect();
        let height = (area.height() - 140.0).clamp(240.0, 560.0);
        let mut open = true;
        let size = egui::vec2(680.0, height);
        let centered = area.center() - size / 2.0;
        egui::Window::new("配置体检")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .fixed_size(size)
            .constrain_to(area)
            .current_pos(centered)
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.ui_health_panel(ui);
                    });
            });
        if !open {
            self.show_health = false;
        }
    }

    /// 悬浮窗统一的边框样式：比卡片更大的圆角与内边距，与主界面分层。
    /// 圆角在形状预设基础上加一档（上限 20）。
    fn floating_window_frame(&self, ctx: &egui::Context) -> egui::Frame {
        let mut frame = egui::Frame::window(&ctx.style());
        let extra = crate::theme::RADIUS_LG - crate::theme::RADIUS_MD;
        frame.corner_radius = self.ui_style.radius().saturating_add(extra).min(20).into();
        frame.inner_margin = egui::Margin::same(crate::theme::SPACE_4 as i8);
        frame
    }

    /// 体检清单的内容。每帧重算：它只遍历内存里的 providers / agents，
    /// 不读文件、不发请求，比维护一份失效逻辑更省心。
    fn ui_health_panel(&mut self, ui: &mut egui::Ui) {
        let input = health::HealthInput {
            page: self.current_page,
            providers: &self.providers,
            agents: &self.agents,
            source_is_opencode: self.source_format.is_opencode_family(),
        };
        let issues = health::collect(&input);
        let semantics = crate::theme::semantics(ui);
        if issues.is_empty() {
            ui.colored_label(semantics.ok, "没有发现需要处理的问题。");
            ui.add_space(crate::theme::SPACE_2);
            ui.label(
                egui::RichText::new(
                    "体检只检查结构与字段写法（重名、非法数字、可疑 URL、跨网关引用等），\
                     不代表配置一定能在上游跑通。",
                )
                .small()
                .weak(),
            );
            return;
        }
        let (blockers, warnings) = health::count_by_severity(&issues);
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{} 项", issues.len()));
            if blockers > 0 {
                ui.colored_label(semantics.err, format!("{} 项会阻止保存", blockers));
            }
            if warnings > 0 {
                ui.colored_label(semantics.warn, format!("{} 项需要注意", warnings));
            }
        });
        ui.add_space(crate::theme::SPACE_2);
        for issue in &issues {
            let color = match issue.severity {
                health::Severity::Blocker => semantics.err,
                health::Severity::Warn => semantics.warn,
                health::Severity::Info => semantics.info,
            };
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(color, egui::RichText::new("●").small());
                ui.strong(&issue.title);
                if !issue.where_.is_empty() {
                    ui.label(egui::RichText::new(&issue.where_).monospace().weak());
                }
                ui.label(
                    egui::RichText::new(issue.severity.label())
                        .small()
                        .color(color),
                );
            });
            // 详情缩进一行，与标题分开，长文本自动换行。
            ui.horizontal_wrapped(|ui| {
                ui.add_space(crate::theme::SPACE_4);
                ui.add(egui::Label::new(egui::RichText::new(&issue.detail).small().weak()).wrap());
            });
            ui.add_space(crate::theme::SPACE_2);
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
            egui::Stroke::new(1.0f32, stroke_color),
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
