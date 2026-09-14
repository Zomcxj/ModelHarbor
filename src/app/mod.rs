use crate::backends;
use crate::convert;
use crate::credentials;
use crate::format::{ConfigFormat, ConfigPaths};
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::theme::Theme;
use crate::ui::{
    card_frame, card_list, field_label, merge_drag_target, move_item, numeric_text_edit,
    secret_text_edit, DragHandle,
};
use crate::util::{self, is_wsl_path, parse_number_text};
use eframe::egui;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

mod serialize;

pub(crate) use serialize::{compact_json, pretty_json};

mod syntax;

use syntax::{apply_find_background, syntax_tokens, PreviewSyntax};

mod fetch;

use fetch::{
    fetch_models_remote, latency_color, matrix_label, model_latency_label, model_probe_button,
    LatencyState, ModelFetchState, ProbeGate, LATENCY_RED, NEW_PROVIDER_FETCH_KEY,
};

mod bars;

use bars::{short_err, sticky_begin, sticky_end};

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
}

/// 一个保存目标的运行时状态（可用性 / 解析路径 / 勾选）。
struct SaveTarget {
    backend: ConfigFormat,
    available: bool,
    path: String,
}

/// 预览编辑框的固定 id（切页时需要主动释放焦点，见 reset_preview_draft）。
const PREVIEW_EDITOR_ID: &str = "preview_editor";
/// 预览框内停止输入多久后，允许用组件状态重建草稿（秒）。
const PREVIEW_EDIT_IDLE_SECS: f64 = 2.0;

/// 本页写入路径的解析结果。
#[derive(Clone)]
enum PageTarget {
    /// 当前文件（已加载）：整体替换 agent / provider。
    Current(String),
    /// 路径已修改但未重新加载：按“先读后合并”写入，不破坏目标文件已有配置。
    Modified(String),
    /// 该后端默认目标（Windows 本地路径；WSL 仅在勾选「WSL同步」后写入）。
    Default(String),
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
    agent_open: HashSet<String>,
    provider_open: HashSet<String>,
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
    /// 模型延迟探测的节流与串行状态（纯内存，重启清零）。
    probe: ProbeGate,
    /// 网络守卫结论：`Some(reason)` 表示检测到系统代理 / VPN，模型延迟测试被禁用。
    net_guard: Option<String>,
    /// 上次网络守卫检测时刻（egui 秒）。
    net_guard_at: f64,
    theme: Theme,
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
        let paths = ConfigPaths::default();
        let (format, path) =
            ConfigPaths::detect().unwrap_or((ConfigFormat::Opencode, String::new()));
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
            agent_open: HashSet::new(),
            provider_open: HashSet::new(),
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
            probe: ProbeGate::default(),
            net_guard: crate::netguard::detect(),
            net_guard_at: 0.0,
            theme: Theme::default(),
            save_format: SaveFormat::default(),
            save_format_wheel_latch: false,
            source_format: format,
            config_paths: paths,
            targets: Vec::new(),
            current_page: format,
            sync_wsl: false,
            show_api_keys: false,
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
                    ui.add_space(4.0);
                    // Agents 仅属于 opencode 页面；区块标题吸顶，滚动时始终显示在顶部。
                    if self.current_page == ConfigFormat::Opencode {
                        self.ui_agents_section(ui);
                        ui.add_space(8.0);
                    }
                    self.ui_providers_section(ui);
                    ui.add_space(8.0);
                });
        });
        self.paint_drag_ghost(ctx);
        // 仅拖拽中显示抓取光标（避免任意控件按下时全局变光标）
        let dragging = self.agent_drag_src.is_some()
            || self.provider_drag_src.is_some()
            || self.model_drag_src.is_some();
        #[cfg(target_os = "windows")]
        crate::cursor::set_custom_cursor_active(dragging);
    }
}

/// provider / model 表单的字段可见性与方言标签（opencode / pi / omp / DSH 共用）。
#[derive(Clone, Copy)]
struct ProviderFormFlags {
    show_oc: bool,
    show_omp: bool,
    show_dsh: bool,
    show_provider_base_url: bool,
    show_provider_timeout: bool,
    show_model_name: bool,
    show_model_context: bool,
    show_model_output: bool,
    show_model_input: bool,
    show_model_variants: bool,
    show_model_reasoning: bool,
    show_model_tool_call: bool,
    show_model_store: bool,
    base_label: &'static str,
    api_key_label: &'static str,
    context_label: &'static str,
    output_label: &'static str,
    input_label: &'static str,
}

impl ProviderFormFlags {
    fn new(app: &App) -> Self {
        let show_oc = app.current_page == ConfigFormat::Opencode;
        let show_dsh = app.current_page == ConfigFormat::DeepSeekHarness;
        Self {
            show_oc,
            show_omp: app.current_page == ConfigFormat::OhMyPi,
            show_dsh,
            show_provider_base_url: app.page_has_provider_field("base_url"),
            // opencode 的 options.timeout 始终显示（文件未写该字段时默认 180000ms）
            show_provider_timeout: show_oc || app.page_has_provider_field("timeout"),
            show_model_name: app.page_has_model_field("name"),
            show_model_context: app.page_has_model_field("context"),
            show_model_output: app.page_has_model_field("output"),
            show_model_input: app.page_has_model_field("input"),
            show_model_variants: app.page_has_model_field("variants"),
            show_model_reasoning: app.page_has_model_field("reasoning"),
            show_model_tool_call: app.page_has_model_field("tool_call"),
            show_model_store: app.page_has_model_field("store"),
            base_label: if show_oc {
                "options.baseURL"
            } else if show_dsh {
                "baseURL"
            } else {
                "baseUrl"
            },
            api_key_label: if show_oc {
                "options.apiKey"
            } else if show_dsh {
                "apiKeyEnv"
            } else {
                "apiKey"
            },
            context_label: if show_oc {
                "limit.context"
            } else {
                "contextWindow"
            },
            output_label: if show_oc { "limit.output" } else { "maxTokens" },
            input_label: if show_oc { "modalities.input" } else { "input" },
        }
    }
}

/// npm 包下拉（opencode 专用）；`id_salt` 区分同一页面内的多个表单实例。
fn provider_npm_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    id_salt: &str,
    empty_has_label: bool,
) {
    const NPM_OPTIONS: [&str; 5] = [
        "",
        "@ai-sdk/openai",
        "@ai-sdk/anthropic",
        "@ai-sdk/google",
        "@ai-sdk/openai-compatible",
    ];
    field_label(ui, 120.0, "npm");
    let current = p.npm.clone();
    let mut selected = NPM_OPTIONS.iter().position(|n| *n == current.as_str());
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if current.is_empty() {
            "选择 npm 包..."
        } else {
            &current
        })
        .width(220.0)
        .show_ui(ui, |ui| {
            for (i, npm) in NPM_OPTIONS.iter().enumerate() {
                let label = if npm.is_empty() && empty_has_label {
                    "(空)"
                } else {
                    *npm
                };
                if ui.selectable_label(selected == Some(i), label).clicked() {
                    selected = Some(i);
                }
            }
        });
    if let Some(i) = selected {
        p.npm = NPM_OPTIONS[i].to_string();
    }
}

/// api / npm 下拉里的「(空)」标签：表示未指定协议。
const EMPTY_API_LABEL: &str = "(空)";

/// api 下拉：omp 官方 9 值 / pi KnownApi 10 值；首项「(空)」与 opencode 页 npm 的空选项同义。
fn provider_api_combo(ui: &mut egui::Ui, p: &mut ProviderRow, show_omp: bool, id_salt: &str) {
    const OMP_APIS: [&str; 9] = [
        "openai-completions",
        "openai-responses",
        "openai-codex-responses",
        "azure-openai-responses",
        "anthropic-messages",
        "bedrock-converse-stream",
        "google-generative-ai",
        "google-gemini-cli",
        "google-vertex",
    ];
    const PI_APIS: [&str; 10] = [
        "openai-completions",
        "mistral-conversations",
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
        "anthropic-messages",
        "bedrock-converse-stream",
        "google-generative-ai",
        "google-vertex",
        "pi-messages",
    ];
    let options: &[&str] = if show_omp { &OMP_APIS } else { &PI_APIS };
    // 「(空)」= 未指定协议。四页共用同一份数据，故以 npm / pi_api / raw.api
    // 是否都为空判定，显示值统一走 effective_api()，与写盘、延迟测试同口径。
    let explicit = p.has_explicit_api();
    let current = p.effective_api();
    field_label(ui, 120.0, "api");
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if explicit {
            current.as_str()
        } else {
            EMPTY_API_LABEL
        })
        .width(180.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(!explicit, EMPTY_API_LABEL)
                .on_hover_text("不指定协议：写盘时按兼容层 openai-completions 处理")
                .clicked()
            {
                p.clear_api();
            }
            for &api in options {
                if ui
                    .selectable_label(explicit && current == api, api)
                    .clicked()
                {
                    p.pi_api = api.to_string();
                    p.npm = convert::api_to_npm(api);
                }
            }
        });
}

/// 思考档位多选：按钮展开、勾选写回逗号分隔文本。
/// `normalize` 为 true 时按规范档位顺序写回，避免重新勾选后被追加到末尾。
fn variant_selector(
    ui: &mut egui::Ui,
    variants: &mut String,
    names: &[&'static str],
    open_key: String,
    open_set: &mut HashSet<String>,
    normalize: bool,
) {
    let current = variants.clone();
    let mut selected: Vec<String> = current
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let display = if selected.is_empty() {
        "选择...".to_string()
    } else {
        current.clone()
    };
    let is_open = open_set.contains(&open_key);
    if ui.button(display).clicked() {
        if is_open {
            open_set.remove(&open_key);
        } else {
            open_set.insert(open_key.clone());
        }
    }
    if !is_open {
        return;
    }
    for name in names {
        let mut checked = selected.iter().any(|s| s == name);
        if ui.checkbox(&mut checked, *name).changed() {
            if checked {
                if !selected.iter().any(|s| s == name) {
                    selected.push((*name).to_string());
                }
            } else {
                selected.retain(|s| s != name);
            }
            *variants = if normalize {
                crate::model::ordered_variants(selected.iter().map(String::as_str)).join(", ")
            } else {
                selected.join(", ")
            };
        }
    }
}

/// 「获取模型」面板的分栏参数：列间水平间距、单列最小宽度、最大列数。
const FETCH_GRID_GAP_X: f32 = 28.0;
const FETCH_GRID_MIN_COL_W: f32 = 150.0;
const FETCH_GRID_MAX_COLS: usize = 5;

/// 计算模型勾选面板的横向分栏：列数由**可用宽度**推出来（而非写死 5 列），列宽再把总宽
/// 均分，所以 `列数 * 列宽 + 间距 * (列数 - 1)` 恒等于可用宽度，面板不会横向溢出。
/// 返回 `(列数, 每列宽度)`。
fn fetch_grid_columns(id_count: usize, avail_width: f32) -> (usize, f32) {
    if id_count == 0 {
        return (1, avail_width.max(1.0));
    }
    let max_cols = FETCH_GRID_MAX_COLS.min(id_count);
    // 浮点转整数在 Rust 中是饱和转换（NaN -> 0），因此不会 panic；clamp 也只是兜底。
    let cols = (((avail_width + FETCH_GRID_GAP_X) / (FETCH_GRID_MIN_COL_W + FETCH_GRID_GAP_X))
        .floor() as usize)
        .clamp(1, max_cols);
    let col_w = ((avail_width - FETCH_GRID_GAP_X * (cols - 1) as f32) / cols as f32).max(1.0);
    (cols, col_w)
}

/// 模型获取结果的勾选面板：勾选后把其中未配置的模型追加到 `models`。
fn model_fetch_popup<H: std::hash::Hash>(
    ui: &mut egui::Ui,
    state: Option<&ModelFetchState>,
    models: &mut Vec<ModelRow>,
    current_page: ConfigFormat,
    id_salt: H,
) {
    let Some(state) = state else {
        ui.label(egui::RichText::new("尚未获取，请先点击「获取模型」").weak());
        return;
    };
    if state.rx.is_some() {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(16.0));
            ui.label(egui::RichText::new("正在获取模型…").weak());
        });
        return;
    }
    let Some(result) = &state.result else {
        return;
    };
    match result {
        Ok(models_remote) if models_remote.is_empty() => {
            ui.label(egui::RichText::new("接口未返回任何模型").weak());
        }
        Ok(models_remote) => {
            let ids = models_remote.clone();
            ui.label(egui::RichText::new("勾选可新增未配置的模型：").weak());
            // 区域高度固定为 22 行，每列超出部分在区域内垂直滚动查看。
            // 注意 1：ScrollArea 内不能用 ui.columns —— columns 会把内容裁到当前可用高度，
            // 导致内容不进入滚动区、无法滚动。改为横向排布 + 纵向子列。
            // 注意 2：列数必须在滚动区**内部**按可用宽度计算，才能把「始终可见的滚动条」
            // 占用的那一条宽度也扣掉，否则整块内容会向右溢出界面。
            let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical()
                .id_salt(id_salt)
                .max_height(row_h * 22.5)
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    let (cols, col_w) = fetch_grid_columns(ids.len(), ui.available_width());
                    let per_col = ids.len().div_ceil(cols);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = FETCH_GRID_GAP_X;
                        for ci in 0..cols {
                            ui.vertical(|ui| {
                                // 定宽列 + 截断：超长模型名悬停看全名，而不是把列撑宽。
                                ui.set_width(col_w);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                for id in ids.iter().skip(ci * per_col).take(per_col) {
                                    let mut checked = models.iter().any(|m| m.id.trim() == id);
                                    if ui
                                        .checkbox(&mut checked, id)
                                        .on_hover_text(id.as_str())
                                        .changed()
                                        && checked
                                    {
                                        let mut row = ModelRow::new();
                                        row.id = id.clone();
                                        row.name = id.clone();
                                        row.source_format = Some(current_page);
                                        models.push(row);
                                    }
                                }
                            });
                        }
                    });
                });
        }
        Err(err) => {
            ui.label(
                egui::RichText::new(format!("获取失败：{}", err))
                    .color(egui::Color32::from_rgb(220, 90, 90)),
            );
        }
    }
}

impl App {
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
        self.agent_open = self.agents.iter().map(|a| a.key.clone()).collect();
        self.provider_open = self.providers.iter().map(|p| p.key.clone()).collect();
        // 重新加载后丢弃旧的模型获取状态
        self.model_fetch.clear();
        self.model_fetch_open.clear();
        self.latency.clear();
        // 被丢弃的探测不会再回传结果：释放全局串行位，否则门控会一直卡在 Busy。
        self.probe.release(None);
        // 加载后跳转到来源格式对应的页面
        self.current_page = self.source_format;
        self.reset_preview_draft();
        self.refresh_targets();
    }

    /// Agents 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
    fn ui_agents_section(&mut self, ui: &mut egui::Ui) {
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.agents.len()).collect();

        if self.agents.is_empty() && !self.show_new_agent {
            self.show_new_agent = true;
        }

        let mut to_remove: Option<usize> = None;
        let mut to_copy: Option<usize> = None;
        let mut hover_target: Option<String> = None;
        card_list(ui, &matched, 0.0, |ui, idx| {
            self.render_agent_card(ui, idx, &mut to_remove, &mut to_copy, &mut hover_target);
        });
        if let Some(idx) = to_remove {
            self.agents.remove(idx);
            self.status = "已删除 agent".into();
        }
        if let Some(idx) = to_copy {
            let mut a = self.agents[idx].clone();
            a.key = format!("{}_copy", a.key);
            self.agents.push(a);
            self.status = "已复制 agent".into();
        }
        if self.agent_drag_src.is_some() {
            self.agent_drag_target = hover_target;
        } else {
            self.agent_drag_target = None;
        }

        ui.add_space(6.0);
        if ui.button("新增 Agent").clicked() {
            self.show_new_agent = !self.show_new_agent;
        }
        if self.show_new_agent {
            self.ui_new_agent_form(ui);
        }
        sticky_end(ui, anchor, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Agents");
                // agents 只属于 opencode 页：来源不是 opencode 时界面没有 agents 数据，
                // 跨格式保存不会接管目标文件的 agent 容器（避免静默清空）。
                if self.source_format != ConfigFormat::Opencode {
                    let src = self.source_format.label();
                    ui.label(
                        egui::RichText::new(format!(
                            "（来源 {}：目标文件已有 agents 会保留，不被清空）",
                            src
                        ))
                        .small()
                        .weak(),
                    );
                }
                if !self.agents.is_empty() {
                    let all_open = self.agents.iter().all(|a| self.agent_open.contains(&a.key));
                    if ui
                        .button(if all_open {
                            "收起全部卡片"
                        } else {
                            "展开全部卡片"
                        })
                        .clicked()
                    {
                        if all_open {
                            self.agent_open.clear();
                        } else {
                            self.agent_open = self.agents.iter().map(|a| a.key.clone()).collect();
                        }
                    }
                }
            });
        });
    }

    fn render_agent_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        to_remove: &mut Option<usize>,
        to_copy: &mut Option<usize>,
        hover_target: &mut Option<String>,
    ) {
        let key = self.agents[idx].key.clone();
        let open = self.agent_open.contains(&key);
        let highlight = if self.agent_drag_target.as_deref() == Some(key.as_str()) {
            2
        } else if self.agent_drag_src.as_deref() == Some(key.as_str()) {
            1
        } else {
            0
        };
        let resp = card_frame(ui, open, highlight, |ui| {
            ui.horizontal(|ui| {
                let h = ui.add(DragHandle);
                if h.drag_started() {
                    self.agent_drag_src = Some(key.clone());
                    self.agent_drag_target = None;
                }
                if h.drag_stopped() {
                    if self.agent_drag_src == Some(key.clone()) {
                        if let Some(dst) = self.agent_drag_target.clone() {
                            let s = self.agents.iter().position(|a| a.key == key);
                            let d = self.agents.iter().position(|a| a.key == dst);
                            if let (Some(s), Some(d)) = (s, d) {
                                move_item(&mut self.agents, s, d);
                            }
                        }
                    }
                    self.agent_drag_src = None;
                    self.agent_drag_target = None;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if open { "▼" } else { "▶" }).size(14.0),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    if open {
                        self.agent_open.remove(&key);
                    } else {
                        self.agent_open.insert(key.clone());
                    }
                }
                ui.strong(&self.agents[idx].key);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("删除").clicked() {
                        *to_remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        *to_copy = Some(idx);
                    }
                });
            });
            if open {
                self.render_agent_form(ui, idx);
            }
        });
        if let Some(src_key) = &self.agent_drag_src {
            if src_key != &key && resp.contains_pointer() && hover_target.is_none() {
                *hover_target = Some(key.clone());
            }
        }
    }

    fn render_agent_form(&mut self, ui: &mut egui::Ui, idx: usize) {
        let prev_key = self.agents[idx].key.clone();
        let other_keys: HashSet<String> = self
            .agents
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, a)| a.key.trim().to_string())
            .collect();
        let a = &mut self.agents[idx];
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "key");
            let key_resp = ui.add(egui::TextEdit::singleline(&mut a.key).desired_width(120.0));
            if !a.key.trim().is_empty() && other_keys.contains(a.key.trim()) {
                key_resp.on_hover_text("key 与其他 agent 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(egui::Color32::from_rgb(220, 90, 90)),
                );
            }
            field_label(ui, 120.0, "mode");
            ui.add(egui::TextEdit::singleline(&mut a.mode).desired_width(120.0));
            field_label(ui, 120.0, "description");
            ui.add(egui::TextEdit::singleline(&mut a.description).desired_width(450.0));
        });
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "model");
            let mut model_options: Vec<String> = self
                .providers
                .iter()
                .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.key, m.id)))
                .collect();
            model_options.extend_from_slice(&[
                "opencode/mimo-v2.5-free".into(),
                "opencode/big-pickle".into(),
            ]);
            model_options.sort();
            model_options.dedup();
            let current = a.model.clone();
            let mut selected_idx = model_options.iter().position(|m| m == &current);
            egui::ComboBox::from_id_salt(format!("agent_model_{}", a.key))
                .selected_text(if current.is_empty() {
                    "选择模型..."
                } else {
                    &current
                })
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (i, model) in model_options.iter().enumerate() {
                        let is_selected = selected_idx == Some(i);
                        if ui.selectable_label(is_selected, model.as_str()).clicked() {
                            selected_idx = Some(i);
                        }
                    }
                });
            if let Some(idx) = selected_idx {
                a.model = model_options[idx].clone();
            }
            field_label(ui, 120.0, "variant");
            let variant_options = ["", "low", "medium", "high", "xhigh", "max", "ultra"];
            let current_variant = a.variant.clone();
            let mut selected_variant = variant_options
                .iter()
                .position(|v| *v == current_variant.as_str());
            egui::ComboBox::from_id_salt(format!("agent_variant_{}", a.key))
                .selected_text(if current_variant.is_empty() {
                    "选择..."
                } else {
                    &current_variant
                })
                .width(100.0)
                .show_ui(ui, |ui| {
                    for (i, v) in variant_options.iter().enumerate() {
                        let label = if v.is_empty() { "(空)" } else { v };
                        let is_selected = selected_variant == Some(i);
                        if ui.selectable_label(is_selected, label).clicked() {
                            selected_variant = Some(i);
                        }
                    }
                });
            if let Some(idx) = selected_variant {
                a.variant = variant_options[idx].to_string();
            }
        });
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "temperature");
            numeric_text_edit(ui, &mut a.temperature, 120.0, "");
            field_label(ui, 120.0, "color");
            ui.add(egui::TextEdit::singleline(&mut a.color).desired_width(120.0));
            field_label(ui, 120.0, "system");
            ui.add(egui::TextEdit::singleline(&mut a.system).desired_width(450.0));
        });
        // key 重命名后同步展开状态（避免改名导致卡片收起）
        let new_key = self.agents[idx].key.clone();
        if new_key != prev_key {
            self.sync_agent_rename(&prev_key, &new_key);
        }
    }

    fn ui_new_agent_form(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.key)
                        .hint_text("coding-assistant")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "mode");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.mode)
                        .hint_text("subagent")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "description");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.description)
                        .hint_text("简要描述此 agent 的用途")
                        .desired_width(450.0),
                );
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "model");
                let mut model_options: Vec<String> = self
                    .providers
                    .iter()
                    .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.key, m.id)))
                    .collect();
                model_options.extend_from_slice(&[
                    "opencode/mimo-v2.5-free".into(),
                    "opencode/big-pickle".into(),
                ]);
                model_options.sort();
                model_options.dedup();
                let current = self.new_agent.model.clone();
                let mut selected_idx = model_options.iter().position(|m| m == &current);
                let _response = egui::ComboBox::from_id_salt("new_agent_model")
                    .selected_text(if current.is_empty() {
                        "选择模型..."
                    } else {
                        &current
                    })
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (i, model) in model_options.iter().enumerate() {
                            let is_selected = selected_idx == Some(i);
                            if ui.selectable_label(is_selected, model.as_str()).clicked() {
                                selected_idx = Some(i);
                            }
                        }
                    });
                if let Some(idx) = selected_idx {
                    self.new_agent.model = model_options[idx].clone();
                }
                field_label(ui, 120.0, "variant");
                let variant_options = ["", "low", "medium", "high", "xhigh", "max", "ultra"];
                let current_variant = self.new_agent.variant.clone();
                let mut selected_variant = variant_options
                    .iter()
                    .position(|v| *v == current_variant.as_str());
                egui::ComboBox::from_id_salt("new_agent_variant")
                    .selected_text(if current_variant.is_empty() {
                        "选择..."
                    } else {
                        &current_variant
                    })
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for (i, v) in variant_options.iter().enumerate() {
                            let label = if v.is_empty() { "(空)" } else { v };
                            let is_selected = selected_variant == Some(i);
                            if ui.selectable_label(is_selected, label).clicked() {
                                selected_variant = Some(i);
                            }
                        }
                    });
                if let Some(idx) = selected_variant {
                    self.new_agent.variant = variant_options[idx].to_string();
                }
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "temperature");
                numeric_text_edit(ui, &mut self.new_agent.temperature, 120.0, "0.7");
                field_label(ui, 120.0, "color");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.color)
                        .hint_text("#00ccff")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "system");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.system)
                        .hint_text("系统提示词")
                        .desired_width(450.0),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                if ui.button("确认").clicked() {
                    let key = self.new_agent.key.trim().to_string();
                    if key.is_empty() {
                        self.status = "请填写 agent key".into();
                    } else if self.agents.iter().any(|a| a.key.trim() == key) {
                        self.status = format!("agent key \"{}\" 已存在", key);
                    } else {
                        let na = self.new_agent.clone();
                        self.agents.push(na);
                        self.new_agent = AgentRow::new();
                        self.show_new_agent = false;
                        self.status = "已添加 agent".into();
                    }
                }
                if ui.button("取消").clicked() {
                    self.new_agent = AgentRow::new();
                    self.show_new_agent = false;
                }
            });
        });
    }

    /// 当前 agent 文件是否使用某个 provider 级字段。
    /// 判断范围是整个文件，不是单个 provider；切换到尚未加载的目标页时
    /// 使用完整 schema，保证新增 provider 可以输入所有专属字段。
    fn page_has_provider_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers
            .iter()
            .any(|provider| match self.current_page {
                ConfigFormat::Opencode => match field {
                    "base_url" => provider
                        .raw
                        .get("options")
                        .and_then(|v| v.get("baseURL"))
                        .is_some(),
                    "timeout" => provider
                        .raw
                        .get("options")
                        .and_then(|v| v.get("timeout"))
                        .is_some(),
                    _ => false,
                },
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "base_url" => provider.raw.get("baseUrl").is_some(),
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "base_url" => provider.raw.get("baseURL").is_some(),
                    _ => false,
                },
            })
    }

    /// 当前 agent 文件是否使用某个 model 级字段。字段存在性按整个
    /// agent 文件判断，避免单个模型缺字段时导致同一页面布局跳变。
    fn page_has_model_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers.iter().any(|provider| {
            provider.models.iter().any(|model| match self.current_page {
                ConfigFormat::Opencode => match field {
                    "name" => model.raw.get("name").is_some(),
                    "reasoning" => model.raw.get("reasoning").is_some(),
                    "tool_call" => model.raw.get("tool_call").is_some(),
                    "store" => model
                        .raw
                        .get("options")
                        .and_then(|v| v.get("store"))
                        .is_some(),
                    "context" => model
                        .raw
                        .get("limit")
                        .and_then(|v| v.get("context"))
                        .is_some(),
                    "output" => model
                        .raw
                        .get("limit")
                        .and_then(|v| v.get("output"))
                        .is_some(),
                    "input" => model
                        .raw
                        .get("modalities")
                        .and_then(|v| v.get("input"))
                        .is_some(),
                    "variants" => model.raw.get("variants").is_some(),
                    _ => false,
                },
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "name" => model.raw.get("name").is_some(),
                    // reasoning 也可由 pi/omp 的 thinking 块 / thinkingLevelMap 表达，
                    // 只写了这些键时同样应显示（并勾选）reasoning。
                    "reasoning" => {
                        model.raw.get("reasoning").is_some()
                            || model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                            || model.raw.get("reasoningEfforts").is_some()
                    }
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => {
                        model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                    }
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "name" => model.raw.get("name").is_some(),
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => model.raw.get("reasoningEfforts").is_some(),
                    _ => false,
                },
            })
        })
    }

    /// 将已加载的公共 provider 凭据投影到 DSH 专属字段。
    /// 只在进入 DSH 页面时执行一次，避免用户在页面内主动清空后被立即回填。
    fn project_dsh_credentials(&mut self) {
        for provider in &mut self.providers {
            if provider.api_key_env.trim().is_empty() {
                provider.api_key_env = credentials::default_env_name(&provider.key);
            }
        }
    }

    /// 页面切换时保持 provider 密钥一致：DSH 页使用 api_key_secret（对应
    /// .credentials.yaml 的 refs），其他页面使用 api_key。切换时把非空值
    /// 同步到目标页字段；若用户在 DSH 页明确清空过密钥（原本有、当前空），
    /// 不再用其他页面的旧值覆盖。
    fn sync_provider_secrets(&mut self, target: ConfigFormat) {
        for provider in &mut self.providers {
            if target == ConfigFormat::DeepSeekHarness {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if !provider.api_key.trim().is_empty() && !cleared_on_dsh {
                    provider.api_key_secret = provider.api_key.clone();
                }
            } else {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if cleared_on_dsh {
                    // 与 DSH 方向对称：在 DSH 页明确清空过的密钥（原本有、当前空）
                    // 不再用旧值填充其他页面，避免已清空的密钥被写回 opencode 等配置。
                    provider.api_key = String::new();
                } else if !provider.api_key_secret.trim().is_empty() {
                    provider.api_key = provider.api_key_secret.clone();
                }
            }
        }
    }

    /// 当前页面的思考档位标签。
    fn dialect_variants(&self) -> (&'static str, &'static [&'static str]) {
        match self.current_page {
            ConfigFormat::Opencode => (
                "variants",
                &["none", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            ConfigFormat::Pi => (
                "thinkingLevelMap",
                &[
                    "off", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
                ],
            ),
            ConfigFormat::OhMyPi => (
                "thinking.efforts",
                &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            ConfigFormat::DeepSeekHarness => (
                "reasoningEfforts",
                &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
        }
    }

    /// Providers 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
    fn ui_providers_section(&mut self, ui: &mut egui::Ui) {
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.providers.len()).collect();

        if self.providers.is_empty() && !self.show_new_provider {
            self.show_new_provider = true;
        }

        let mut to_remove: Option<usize> = None;
        let mut to_copy: Option<usize> = None;
        // 拖拽落点必须在**所有卡片渲染完之后**统一聚合再写入 self：
        // 卡片各自赋值会被后渲染的卡片用 None 覆盖（模型卡片曾因此丢失绿色落点边框）。
        let mut hover_target: Option<String> = None;
        let mut model_hover_target: Option<String> = None;
        card_list(ui, &matched, 0.0, |ui, idx| {
            self.render_provider_card(
                ui,
                idx,
                &mut to_remove,
                &mut to_copy,
                &mut hover_target,
                &mut model_hover_target,
            );
        });
        if let Some(idx) = to_remove {
            self.providers.remove(idx);
            self.status = "已删除 provider".into();
        }
        if let Some(idx) = to_copy {
            let mut p = self.providers[idx].clone();
            p.key = format!("{}_copy", p.key);
            self.providers.push(p);
            self.status = "已复制 provider".into();
        }
        if self.provider_drag_src.is_some() {
            self.provider_drag_target = hover_target;
        } else {
            self.provider_drag_target = None;
        }
        // 模型拖拽落点：与 provider 同样在外层聚合，保证任意展开顺序下被拖到的
        // 模型卡片都能拿到绿色边框（见 render_provider_form 里的说明）。
        if self.model_drag_src.is_some() {
            self.model_drag_target = model_hover_target;
        } else {
            self.model_drag_target = None;
        }

        ui.add_space(10.0);
        if ui.button("新增 Provider").clicked() {
            self.show_new_provider = !self.show_new_provider;
        }
        if self.show_new_provider {
            self.ui_new_provider_form(ui);
        }
        sticky_end(ui, anchor, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Providers");
                if !self.providers.is_empty() {
                    let all_open = self
                        .providers
                        .iter()
                        .all(|p| self.provider_open.contains(&p.key));
                    if ui
                        .button(if all_open {
                            "收起全部卡片"
                        } else {
                            "展开全部卡片"
                        })
                        .clicked()
                    {
                        if all_open {
                            self.provider_open.clear();
                        } else {
                            self.provider_open =
                                self.providers.iter().map(|p| p.key.clone()).collect();
                        }
                    }
                }
                // 连通性测试：放在标题行右侧，收起全部卡片时也始终可见。
                if ui
                    .button("连通性测试")
                    .on_hover_text("并发测试当前页面全部厂商的接口连通性")
                    .clicked()
                {
                    let targets: Vec<(String, String, String, String)> = self
                        .providers
                        .iter()
                        .map(|p| {
                            let api = p.effective_api();
                            (
                                p.key.clone(),
                                p.base_url.clone(),
                                credentials::effective_secret(p),
                                api,
                            )
                        })
                        .collect();
                    let count = targets.len();
                    for (key, base, secret, api) in targets {
                        Self::start_provider_latency(&mut self.latency, &key, &base, &secret, &api);
                    }
                    self.status = format!("已开始连通性测试（{} 个厂商）", count);
                }
                // 网络守卫提示：检测到系统代理 / VPN 时禁用模型延迟测试
                //（中转站的「多 IP 检测 / 测活封号」可能因此触发）。
                if let Some(reason) = self.net_guard.clone() {
                    ui.label(
                        egui::RichText::new(format!("{}：{reason}", crate::netguard::BLOCK_PREFIX))
                            .small()
                            .color(LATENCY_RED),
                    )
                    .on_hover_text(
                        "中转站常见多 IP 检测 / 测活风控，经代理做推理探测可能被封号；\n\
                         因此已禁用模型延迟测试。请关闭系统代理 / VPN 后重试。\n\
                         （厂商「连通性测试」不受影响：它只拉模型列表，不做推理。）",
                    );
                }
                // 全局 API Key 显示/隐藏：一键切换全部密钥的明文/掩码。
                // 文案带「密钥」二字，与区块「隐藏/展开」、卡片 ▼/▶ 折叠按钮明确区分。
                if ui
                    .button(if self.show_api_keys {
                        "隐藏密钥"
                    } else {
                        "显示密钥"
                    })
                    .on_hover_text(if self.show_api_keys {
                        "点击掩码全部 API Key（默认状态）"
                    } else {
                        "点击显示全部 API Key 明文（注意防窥）"
                    })
                    .clicked()
                {
                    self.show_api_keys = !self.show_api_keys;
                }
                // 配置预览：右侧面板实时展示当前页面的序列化内容，可编辑并应用回组件。
                if ui
                    .button(if self.show_preview {
                        "关闭预览"
                    } else {
                        "预览"
                    })
                    .on_hover_text(
                        "在右侧打开当前页面「待保存文档」预览；可直接编辑，改动实时应用并自动保存",
                    )
                    .clicked()
                {
                    self.show_preview = !self.show_preview;
                    if self.show_preview {
                        // 打开时以组件状态重建待保存文档
                        self.reset_preview_draft();
                    }
                }
            });
        });
    }

    fn render_provider_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        to_remove: &mut Option<usize>,
        to_copy: &mut Option<usize>,
        hover_target: &mut Option<String>,
        model_hover_target: &mut Option<String>,
    ) {
        let key = self.providers[idx].key.clone();
        let open = self.provider_open.contains(&key);
        let highlight = if self.provider_drag_target.as_deref() == Some(key.as_str()) {
            2
        } else if self.provider_drag_src.as_deref() == Some(key.as_str()) {
            1
        } else {
            0
        };
        let resp = card_frame(ui, open, highlight, |ui| {
            ui.horizontal(|ui| {
                let h = ui.add(DragHandle);
                if h.drag_started() {
                    self.provider_drag_src = Some(key.clone());
                    self.provider_drag_target = None;
                }
                if h.drag_stopped() {
                    if self.provider_drag_src == Some(key.clone()) {
                        if let Some(dst) = self.provider_drag_target.clone() {
                            let s = self.providers.iter().position(|p| p.key == key);
                            let d = self.providers.iter().position(|p| p.key == dst);
                            if let (Some(s), Some(d)) = (s, d) {
                                move_item(&mut self.providers, s, d);
                            }
                        }
                    }
                    self.provider_drag_src = None;
                    self.provider_drag_target = None;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if open { "▼" } else { "▶" }).size(14.0),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    if open {
                        self.provider_open.remove(&key);
                    } else {
                        self.provider_open.insert(key.clone());
                    }
                }
                ui.strong(&self.providers[idx].key);
                // 连通性测试结果：显示在厂商名字右侧，卡片收起时也可见。
                if let Some(state) = self.latency.get(&key) {
                    if state.provider_rx.is_some() {
                        matrix_label(ui, &key);
                    } else if let Some(res) = &state.provider {
                        match res {
                            Ok(ms) => {
                                ui.label(
                                    egui::RichText::new(format!("{}ms", ms))
                                        .color(latency_color(*ms)),
                                );
                            }
                            Err(err) => {
                                ui.label(egui::RichText::new(short_err(err)).color(LATENCY_RED))
                                    .on_hover_text(err);
                            }
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("删除").clicked() {
                        *to_remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        *to_copy = Some(idx);
                    }
                });
            });
            if open {
                self.render_provider_form(ui, idx, model_hover_target);
            }
        });
        if let Some(src_key) = &self.provider_drag_src {
            if src_key != &key && resp.contains_pointer() && hover_target.is_none() {
                *hover_target = Some(key.clone());
            }
        }
    }

    fn render_provider_form(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        model_hover_target: &mut Option<String>,
    ) {
        let prev_key = self.providers[idx].key.clone();
        let other_keys: HashSet<String> = self
            .providers
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, p)| p.key.trim().to_string())
            .collect();
        let (variants_label, variant_names) = self.dialect_variants();
        let ProviderFormFlags {
            show_oc,
            show_omp,
            show_dsh,
            show_provider_base_url,
            show_provider_timeout,
            show_model_name,
            show_model_context,
            show_model_output,
            show_model_input,
            show_model_variants,
            show_model_reasoning,
            show_model_tool_call,
            show_model_store,
            base_label,
            api_key_label,
            context_label,
            output_label,
            input_label,
            ..
        } = ProviderFormFlags::new(self);
        let p = &mut self.providers[idx];
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "key");
            let key_resp = ui.add(egui::TextEdit::singleline(&mut p.key).desired_width(120.0));
            if !p.key.trim().is_empty() && other_keys.contains(p.key.trim()) {
                key_resp.on_hover_text("key 与其他 provider 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(egui::Color32::from_rgb(220, 90, 90)),
                );
            }
            if show_oc {
                let salt = format!("provider_npm_{}", p.key);
                provider_npm_combo(ui, p, &salt, true);
            }
            if !show_oc {
                let salt = format!("provider_api_{}", p.key);
                provider_api_combo(ui, p, show_omp, &salt);
            }
            // timeout / timeoutMs 与 npm/api 同排（第一行）。
            if show_oc && show_provider_timeout {
                field_label(ui, 120.0, "options.timeout");
                numeric_text_edit(ui, &mut p.timeout, 70.0, "180000");
            }
            if show_dsh {
                field_label(ui, 120.0, "timeoutMs");
                numeric_text_edit(ui, &mut p.dsh_timeout_ms, 70.0, "180000");
            }
            // pi / omp 的 compat 与 api 同排显示（紧跟 api 之后）。
            if !show_oc && !show_dsh {
                field_label(ui, 120.0, "compat");
                ui.checkbox(&mut p.compat, "supportsDeveloperRole");
                // pi / omp 相互映射字段：加载 opencode/dsh 时缺省不勾选。
                let requires_label = if show_omp {
                    "requiresReasoningContentForAllAssistantTurns"
                } else {
                    "requiresReasoningContentOnAssistantMessages"
                };
                ui.checkbox(&mut p.requires_reasoning_content, requires_label);
            }
            if show_dsh {
                field_label(ui, 120.0, "retryPolicy.mode");
                ui.add(egui::TextEdit::singleline(&mut p.dsh_retry_mode).desired_width(100.0));
                field_label(ui, 120.0, "maxRetries");
                numeric_text_edit(ui, &mut p.dsh_max_retries, 55.0, "3");
            }
        });
        ui.horizontal_wrapped(|ui| {
            if show_provider_base_url {
                field_label(ui, 120.0, base_label);
                ui.add(egui::TextEdit::singleline(&mut p.base_url).desired_width(200.0));
            }
            field_label(ui, 120.0, api_key_label);
            if show_dsh {
                ui.add(egui::TextEdit::singleline(&mut p.api_key_env).desired_width(192.0));
                field_label(ui, 120.0, "API Key");
                secret_text_edit(ui, &mut p.api_key_secret, self.show_api_keys, 408.0, "");
            } else {
                secret_text_edit(ui, &mut p.api_key, self.show_api_keys, 408.0, "");
            }
        });

        ui.add_space(6.0);
        let mut fetch_request: Option<(String, String, String, String)> = None;
        let mut close_fetch = false;
        // 本帧用户点下的探测请求（provider key, model id），UI 循环外统一发起。
        let mut probe_request: Option<(String, String)> = None;
        ui.horizontal(|ui| {
            ui.strong("Models");
            let fetch_api = p.effective_api();
            let fetch_secret = credentials::effective_secret(p);
            if ui.button("获取模型").clicked() {
                fetch_request = Some((
                    p.key.clone(),
                    p.base_url.clone(),
                    fetch_secret.clone(),
                    fetch_api.clone(),
                ));
            }
            if self.model_fetch_open.contains(&p.key) && ui.button("关闭").clicked() {
                close_fetch = true;
            }
            // 模型延迟已无批量入口：每个模型行右侧各有一个「测试」按钮，
            // 一次只测一个（同模型 60s、同厂商 10s 节流，规避测活风控）。
            if self
                .latency
                .get(&p.key)
                .is_some_and(|s| s.model_rx.is_some())
            {
                ui.label(egui::RichText::new("延迟测试中…").small());
            }
        });
        if let Some((key, base, secret, api)) = fetch_request {
            // 后台线程拉取模型列表（避免阻塞 UI），结果经通道回传。
            let url = Self::models_url(&base, &api);
            let secret = secret.trim().to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = fetch_models_remote(&url, &secret, &api);
                let _ = tx.send(result);
            });
            self.model_fetch.insert(
                key.clone(),
                ModelFetchState {
                    rx: Some(rx),
                    result: None,
                },
            );
            self.model_fetch_open.insert(key);
        }
        if close_fetch {
            self.model_fetch_open.remove(&p.key);
        }
        if self.model_fetch_open.contains(&p.key) {
            let fetch_key = p.key.clone();
            card_frame(ui, false, 0, |ui| {
                model_fetch_popup(
                    ui,
                    self.model_fetch.get(&fetch_key),
                    &mut p.models,
                    self.current_page,
                    ("model_fetch_scroll", fetch_key.as_str()),
                );
            });
        }
        let mut rm: Option<usize> = None;
        let mut model_hover_here: Option<String> = None;
        let mut model_drag_stopped = false;
        for j in 0..p.models.len() {
            let model_key = format!("{}\u{1f}{}", p.key, p.models[j].id);
            let model_highlight = if self.model_drag_target.as_deref() == Some(model_key.as_str()) {
                2
            } else if self.model_drag_src.as_deref() == Some(model_key.as_str()) {
                1
            } else {
                0
            };
            let other_ids: HashSet<String> = p
                .models
                .iter()
                .enumerate()
                .filter(|(j2, _)| *j2 != j)
                .map(|(_, m)| m.id.trim().to_string())
                .collect();
            let model_response = card_frame(ui, true, model_highlight, |ui| {
                ui.horizontal(|ui| {
                    let handle = ui.add(DragHandle);
                    if handle.drag_started() {
                        self.model_drag_src = Some(model_key.clone());
                        self.model_drag_target = None;
                    }
                    if handle.drag_stopped() {
                        model_drag_stopped = true;
                    }
                    // 单模型延迟测试：按钮在拖动按钮右侧，结果显示在按钮右侧。
                    let model_id = p.models[j].id.trim().to_string();
                    if model_probe_button(
                        ui,
                        &self.probe,
                        &p.key,
                        ui.input(|i| i.time),
                        self.net_guard.as_deref(),
                    ) {
                        probe_request = Some((p.key.clone(), model_id.clone()));
                    }
                    let latency = self.latency.get(&p.key);
                    model_latency_label(ui, latency, &model_id);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("删").clicked() {
                            rm = Some(j);
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, "id:");
                    let id_resp = ui
                        .add(egui::TextEdit::singleline(&mut p.models[j].id).desired_width(120.0));
                    if !p.models[j].id.trim().is_empty()
                        && other_ids.contains(p.models[j].id.trim())
                    {
                        id_resp.on_hover_text("id 与同 provider 内其他模型重复，保存将被阻止");
                        ui.label(
                            egui::RichText::new("⚠ 重复")
                                .small()
                                .color(egui::Color32::from_rgb(220, 90, 90)),
                        );
                    }
                    if show_model_name {
                        field_label(ui, 120.0, "name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut p.models[j].name).desired_width(120.0),
                        );
                    }
                    if show_model_reasoning && (show_oc || !show_dsh) {
                        ui.checkbox(&mut p.models[j].reasoning, "reasoning");
                    }
                    if show_model_tool_call && show_oc {
                        ui.checkbox(&mut p.models[j].tool_call, "tool_call");
                    }
                    if show_model_store && show_oc {
                        ui.checkbox(&mut p.models[j].store, "store");
                    }
                    if show_model_context {
                        field_label(ui, 120.0, context_label);
                        numeric_text_edit(ui, &mut p.models[j].context, 53.0, "");
                    }
                    if show_model_output {
                        field_label(ui, 120.0, output_label);
                        numeric_text_edit(ui, &mut p.models[j].output, 53.0, "");
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if show_model_input {
                        field_label(ui, 120.0, input_label);
                        ui.add(
                            egui::TextEdit::singleline(&mut p.models[j].modalities_input)
                                .desired_width(80.0),
                        );
                    }
                    if show_oc {
                        field_label(ui, 120.0, "modalities.output");
                        ui.add(
                            egui::TextEdit::singleline(&mut p.models[j].modalities_output)
                                .desired_width(80.0),
                        );
                    }
                    if show_model_variants {
                        field_label(ui, 120.0, variants_label);
                    }
                    let variant_key = format!("variant_open_{}_{}", p.key, j);
                    variant_selector(
                        ui,
                        &mut p.models[j].variants,
                        variant_names,
                        variant_key,
                        &mut self.variant_open,
                        true,
                    );
                });
            });
            if let Some(src) = &self.model_drag_src {
                if src != &model_key
                    && model_response.contains_pointer()
                    && model_hover_here.is_none()
                {
                    model_hover_here = Some(model_key.clone());
                }
            }
        }
        if let Some((provider_key, model_id)) = probe_request {
            let now = ui.input(|i| i.time);
            let base_url = p.base_url.clone();
            let secret = credentials::effective_secret(p);
            let api = p.effective_api();
            self.status = Self::run_model_probe(
                &mut self.probe,
                &mut self.latency,
                self.net_guard.as_deref(),
                &provider_key,
                &model_id,
                now,
                &base_url,
                &secret,
                &api,
            );
        }
        // 只登记「本 provider 内被拖到的模型」，跨卡片的聚合交给调用方
        // （ui_providers_section 在全部卡片渲染完之后统一写入 self.model_drag_target）。
        merge_drag_target(model_hover_target, model_hover_here);
        if model_drag_stopped {
            if let Some(src) = self.model_drag_src.take() {
                let target = self.model_drag_target.take();
                if let Some(dst) = target {
                    let source = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == src);
                    let destination = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == dst);
                    if let (Some(source), Some(destination)) = (source, destination) {
                        move_item(&mut p.models, source, destination);
                    }
                }
            }
        }
        if let Some(j) = rm {
            p.models.remove(j);
            // 删除后下标错位：关闭该 provider 的档位弹窗，避免状态串到其他模型
            let prefix = format!("variant_open_{}_", p.key);
            self.variant_open.retain(|k| !k.starts_with(&prefix));
        }
        ui.add_space(6.0);
        let show_new_model_key = format!("show_new_model_{}", p.key);
        let show_new_model = self.variant_open.contains(&show_new_model_key);
        let btn_text = if show_new_model {
            "收起"
        } else {
            "添加 Model"
        };
        if ui
            .add_sized([120.0, 20.0], egui::Button::new(btn_text))
            .clicked()
        {
            if show_new_model {
                self.variant_open.remove(&show_new_model_key);
            } else {
                self.variant_open.insert(show_new_model_key.clone());
            }
        }
        if show_new_model {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "id:");
                ui.add(egui::TextEdit::singleline(&mut p.new_model.id).desired_width(120.0));
                field_label(ui, 120.0, "name:");
                ui.add(egui::TextEdit::singleline(&mut p.new_model.name).desired_width(120.0));
                if show_oc || !show_dsh {
                    ui.checkbox(&mut p.new_model.reasoning, "reasoning");
                }
                if show_oc {
                    ui.checkbox(&mut p.new_model.tool_call, "tool_call");
                    ui.checkbox(&mut p.new_model.store, "store");
                }
                field_label(ui, 120.0, context_label);
                numeric_text_edit(ui, &mut p.new_model.context, 53.0, "");
                field_label(ui, 120.0, output_label);
                numeric_text_edit(ui, &mut p.new_model.output, 53.0, "");
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, input_label);
                ui.add(
                    egui::TextEdit::singleline(&mut p.new_model.modalities_input)
                        .desired_width(80.0),
                );
                if show_oc {
                    field_label(ui, 120.0, "modalities.output");
                    ui.add(
                        egui::TextEdit::singleline(&mut p.new_model.modalities_output)
                            .desired_width(80.0),
                    );
                }
                field_label(ui, 120.0, variants_label);
                let variant_key = format!("new_model_variant_{}", p.key);
                variant_selector(
                    ui,
                    &mut p.new_model.variants,
                    variant_names,
                    variant_key,
                    &mut self.variant_open,
                    false,
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                if ui.button("添加").clicked() && !p.new_model.id.trim().is_empty() {
                    p.models.push(p.new_model.clone());
                    p.new_model = ModelRow::new();
                    self.variant_open.remove(&show_new_model_key);
                }
            });
        }
        // key 重命名后同步展开状态与弹窗键
        let new_key = self.providers[idx].key.clone();
        if new_key != prev_key {
            self.sync_provider_rename(&prev_key, &new_key);
        }
    }

    fn ui_new_provider_form(&mut self, ui: &mut egui::Ui) {
        let (variants_label, variant_names) = self.dialect_variants();
        let ProviderFormFlags {
            show_oc,
            show_omp,
            show_dsh,
            base_label,
            api_key_label,
            context_label,
            output_label,
            input_label,
            ..
        } = ProviderFormFlags::new(self);
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_provider.key)
                        .hint_text("openai")
                        .desired_width(120.0),
                );
                if show_oc {
                    let p = &mut self.new_provider;
                    provider_npm_combo(ui, p, "new_provider_npm", false);
                }
                if !show_oc {
                    let p = &mut self.new_provider;
                    provider_api_combo(ui, p, show_omp, "new_provider_api");
                }
                // timeout 与 npm/api 同排（第一行）。
                if show_oc {
                    field_label(ui, 120.0, "options.timeout");
                    numeric_text_edit(ui, &mut self.new_provider.timeout, 70.0, "180000");
                }
                // pi / omp 的 compat 与 api 同排显示（紧跟 api 之后）。
                if !show_oc && !show_dsh {
                    field_label(ui, 120.0, "compat");
                    ui.checkbox(&mut self.new_provider.compat, "supportsDeveloperRole");
                    let requires_label = if show_omp {
                        "requiresReasoningContentForAllAssistantTurns"
                    } else {
                        "requiresReasoningContentOnAssistantMessages"
                    };
                    ui.checkbox(
                        &mut self.new_provider.requires_reasoning_content,
                        requires_label,
                    );
                }
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, base_label);
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_provider.base_url)
                        .hint_text("https://api.openai.com/v1")
                        .desired_width(200.0),
                );
                field_label(ui, 120.0, api_key_label);
                if show_dsh {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.api_key_env)
                            .hint_text("DEEPSEEK_API_KEY")
                            .desired_width(192.0),
                    );
                    field_label(ui, 120.0, "API Key");
                    secret_text_edit(
                        ui,
                        &mut self.new_provider.api_key_secret,
                        self.show_api_keys,
                        408.0,
                        "实际密钥",
                    );
                } else {
                    secret_text_edit(
                        ui,
                        &mut self.new_provider.api_key,
                        self.show_api_keys,
                        408.0,
                        "sk-xxx",
                    );
                }
            });
            ui.add_space(6.0);
            let mut fetch_request: Option<(String, String, String)> = None;
            let mut close_fetch = false;
            ui.horizontal(|ui| {
                ui.strong("Models");
                let fetch_api = self.new_provider.effective_api();
                let fetch_secret = credentials::effective_secret(&self.new_provider);
                if ui.button("获取模型").clicked() {
                    fetch_request = Some((
                        self.new_provider.base_url.clone(),
                        fetch_secret.clone(),
                        fetch_api.clone(),
                    ));
                }
                if self.model_fetch_open.contains(NEW_PROVIDER_FETCH_KEY)
                    && ui.button("关闭").clicked()
                {
                    close_fetch = true;
                }
                // 模型延迟已无批量入口：每个模型行右侧各有一个「测试」按钮。
                if self
                    .latency
                    .get(NEW_PROVIDER_FETCH_KEY)
                    .is_some_and(|s| s.model_rx.is_some())
                {
                    ui.label(egui::RichText::new("延迟测试中…").small());
                }
            });
            if let Some((base, secret, api)) = fetch_request {
                self.start_model_fetch(NEW_PROVIDER_FETCH_KEY, &base, &secret, &api);
                self.model_fetch_open
                    .insert(NEW_PROVIDER_FETCH_KEY.to_string());
            }
            if close_fetch {
                self.model_fetch_open.remove(NEW_PROVIDER_FETCH_KEY);
            }
            if self.model_fetch_open.contains(NEW_PROVIDER_FETCH_KEY) {
                card_frame(ui, false, 0, |ui| {
                    model_fetch_popup(
                        ui,
                        self.model_fetch.get(NEW_PROVIDER_FETCH_KEY),
                        &mut self.new_provider.models,
                        self.current_page,
                        "new_provider_fetch_scroll",
                    );
                });
            }
            let mut rm_new: Option<usize> = None;
            let mut move_new_request: Option<(usize, usize)> = None;
            // 本帧用户点下的探测请求（新 provider 固定用 NEW_PROVIDER_FETCH_KEY 做节流键）。
            let mut probe_request: Option<(String, String)> = None;
            for j in 0..self.new_provider.models.len() {
                let model_count = self.new_provider.models.len();
                card_frame(ui, true, 0, |ui| {
                    ui.horizontal(|ui| {
                        if j > 0 && ui.button("↑").clicked() {
                            move_new_request = Some((j, j - 1));
                        }
                        if j + 1 < model_count && ui.button("↓").clicked() {
                            move_new_request = Some((j, j + 1));
                        }
                        // 单模型延迟测试：按钮在调整按钮右侧，结果显示在按钮右侧。
                        let model_id = self.new_provider.models[j].id.trim().to_string();
                        if model_probe_button(
                            ui,
                            &self.probe,
                            NEW_PROVIDER_FETCH_KEY,
                            ui.input(|i| i.time),
                            self.net_guard.as_deref(),
                        ) {
                            probe_request =
                                Some((NEW_PROVIDER_FETCH_KEY.to_string(), model_id.clone()));
                        }
                        let latency = self.latency.get(NEW_PROVIDER_FETCH_KEY);
                        model_latency_label(ui, latency, &model_id);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("删").clicked() {
                                rm_new = Some(j);
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        field_label(ui, 120.0, "id:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_provider.models[j].id)
                                .desired_width(120.0),
                        );
                        field_label(ui, 120.0, "name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_provider.models[j].name)
                                .desired_width(120.0),
                        );
                        if show_oc || !show_dsh {
                            ui.checkbox(&mut self.new_provider.models[j].reasoning, "reasoning");
                        }
                        if show_oc {
                            ui.checkbox(&mut self.new_provider.models[j].tool_call, "tool_call");
                            ui.checkbox(&mut self.new_provider.models[j].store, "store");
                        }
                        field_label(ui, 120.0, context_label);
                        numeric_text_edit(ui, &mut self.new_provider.models[j].context, 53.0, "");
                        field_label(ui, 120.0, output_label);
                        numeric_text_edit(ui, &mut self.new_provider.models[j].output, 53.0, "");
                    });
                });
            }
            if let Some((provider_key, model_id)) = probe_request {
                let now = ui.input(|i| i.time);
                let base_url = self.new_provider.base_url.clone();
                let secret = credentials::effective_secret(&self.new_provider);
                let api = self.new_provider.effective_api();
                self.status = Self::run_model_probe(
                    &mut self.probe,
                    &mut self.latency,
                    self.net_guard.as_deref(),
                    &provider_key,
                    &model_id,
                    now,
                    &base_url,
                    &secret,
                    &api,
                );
            }
            if let Some((from, to)) = move_new_request {
                move_item(&mut self.new_provider.models, from, to);
            }
            if let Some(j) = rm_new {
                self.new_provider.models.remove(j);
            }
            ui.add_space(6.0);
            let show_new_model_key = format!("new_provider_show_model_{}", self.new_provider.key);
            let show_new_model = self.variant_open.contains(&show_new_model_key);
            if ui
                .button(if show_new_model {
                    "收起"
                } else {
                    "添加 Model"
                })
                .clicked()
            {
                if show_new_model {
                    self.variant_open.remove(&show_new_model_key);
                } else {
                    self.variant_open.insert(show_new_model_key.clone());
                }
            }
            if show_new_model {
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, "id:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.new_model.id)
                            .desired_width(120.0),
                    );
                    field_label(ui, 120.0, "name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.new_model.name)
                            .desired_width(120.0),
                    );
                    if show_oc || !show_dsh {
                        ui.checkbox(&mut self.new_provider.new_model.reasoning, "reasoning");
                    }
                    if show_oc {
                        ui.checkbox(&mut self.new_provider.new_model.tool_call, "tool_call");
                        ui.checkbox(&mut self.new_provider.new_model.store, "store");
                    }
                    field_label(ui, 120.0, context_label);
                    numeric_text_edit(ui, &mut self.new_provider.new_model.context, 53.0, "");
                    field_label(ui, 120.0, output_label);
                    numeric_text_edit(ui, &mut self.new_provider.new_model.output, 53.0, "");
                });
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, input_label);
                    ui.add(
                        egui::TextEdit::singleline(
                            &mut self.new_provider.new_model.modalities_input,
                        )
                        .desired_width(80.0),
                    );
                    if show_oc {
                        field_label(ui, 120.0, "modalities.output");
                        ui.add(
                            egui::TextEdit::singleline(
                                &mut self.new_provider.new_model.modalities_output,
                            )
                            .desired_width(80.0),
                        );
                    }
                    field_label(ui, 120.0, variants_label);
                    variant_selector(
                        ui,
                        &mut self.new_provider.new_model.variants,
                        variant_names,
                        "new_provider_new_model_variant".to_string(),
                        &mut self.variant_open,
                        false,
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(60.0);
                    if ui.button("添加").clicked()
                        && !self.new_provider.new_model.id.trim().is_empty()
                    {
                        self.new_provider
                            .models
                            .push(self.new_provider.new_model.clone());
                        self.new_provider.new_model = ModelRow::new();
                        self.variant_open.remove(&show_new_model_key);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                if ui.button("确认").clicked() {
                    let key = self.new_provider.key.trim().to_string();
                    if key.is_empty() {
                        self.status = "请填写 provider key".into();
                    } else if self.providers.iter().any(|p| p.key.trim() == key) {
                        self.status = format!("provider key \"{}\" 已存在", key);
                    } else {
                        let np = self.new_provider.clone();
                        self.providers.push(np);
                        self.new_provider = ProviderRow::new();
                        self.show_new_provider = false;
                        self.clear_new_provider_state();
                        self.status = "已添加 provider".into();
                    }
                }
                if ui.button("取消").clicked() {
                    self.new_provider = ProviderRow::new();
                    self.show_new_provider = false;
                    self.clear_new_provider_state();
                }
            });
        });
    }

    /// 关闭新增 provider 表单时清理其测试/获取状态，避免下次打开残留旧结果。
    fn clear_new_provider_state(&mut self) {
        self.latency.remove(NEW_PROVIDER_FETCH_KEY);
        self.model_fetch.remove(NEW_PROVIDER_FETCH_KEY);
        self.model_fetch_open.remove(NEW_PROVIDER_FETCH_KEY);
        // 表单关掉后探测结果无处显示：释放它的串行位。
        self.probe.release(Some(NEW_PROVIDER_FETCH_KEY));
    }

    fn reload(&mut self) {
        let (fmt, _) = ConfigPaths::detect_for_path(&self.config_path);
        self.source_format = fmt;
        self.apply_load();
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

    /// agent key 重命名后同步 UI 状态（卡片展开集合），避免改名后卡片收起。
    fn sync_agent_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        if self.agent_open.remove(old) {
            self.agent_open.insert(new.to_string());
        }
    }

    /// provider key 重命名后同步 UI 状态（展开集合 + 弹窗键）。
    fn sync_provider_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        if self.provider_open.remove(old) {
            self.provider_open.insert(new.to_string());
        }
        // 下标型弹窗键直接关闭（避免前缀歧义），需要时重新打开即可
        let variant_prefix = format!("variant_open_{}_", old);
        let show_key = format!("show_new_model_{}", old);
        let new_variant_key = format!("new_model_variant_{}", old);
        self.variant_open
            .retain(|k| !k.starts_with(&variant_prefix) && k != &show_key && k != &new_variant_key);
    }

    /// 统计非法数字字段数（非空且解析失败），保存后提示用户它们被忽略。
    fn count_invalid_numeric_fields(&self) -> usize {
        fn bad(s: &str) -> bool {
            !s.trim().is_empty() && parse_number_text(s).is_none()
        }
        let mut n = 0;
        for a in &self.agents {
            if bad(&a.temperature) {
                n += 1;
            }
        }
        for p in &self.providers {
            if bad(&p.timeout) {
                n += 1;
            }
            for m in &p.models {
                if bad(&m.context) || bad(&m.output) {
                    n += 1;
                }
            }
        }
        n
    }

    /// 校验 agent / provider / model key 唯一性，返回首个冲突描述。
    fn find_duplicate_keys(&self) -> Option<String> {
        let mut seen = HashSet::new();
        for a in &self.agents {
            let k = a.key.trim();
            if !k.is_empty() && !seen.insert(k.to_string()) {
                return Some(format!("agent \"{}\"", k));
            }
        }
        let mut seen_p = HashSet::new();
        for p in &self.providers {
            let k = p.key.trim();
            if k.is_empty() {
                continue;
            }
            if !seen_p.insert(k.to_string()) {
                return Some(format!("provider \"{}\"", k));
            }
            let mut seen_m = HashSet::new();
            for m in &p.models {
                let mk = m.id.trim();
                if !mk.is_empty() && !seen_m.insert(mk.to_string()) {
                    return Some(format!("provider \"{}\" 的 model \"{}\"", k, mk));
                }
            }
        }
        None
    }

    /// 本页写入路径：
    /// - 当前文件属于本页格式且已加载 → 当前文件（整体替换）；
    /// - 路径已修改但未加载 → 仍写该路径，但按“先读后合并”（防止覆盖目标文件已有配置）；
    /// - 其余 → 该后端默认目标（Windows 本地；WSL 需勾选「WSL同步」）。
    fn page_save_path(&self, fmt: ConfigFormat) -> PageTarget {
        if !self.config_path.is_empty() && self.config_path != self.loaded_path {
            // 用户已经在路径框中明确指定了目标文件，即使尚未点击“加载”，
            // 也必须使用该路径；保存流程会先读目标并按目标格式合并，不能静默回落默认路径。
            return PageTarget::Modified(self.config_path.clone());
        }
        if self.source_format == fmt && !self.config_path.is_empty() {
            return PageTarget::Current(self.config_path.clone());
        }
        let path = self
            .targets
            .iter()
            .find(|t| t.backend == fmt)
            .map(|t| t.path.clone())
            .unwrap_or_default();
        PageTarget::Default(path)
    }

    /// 保存当前页面：写入该页格式对应的路径，并按需同步 WSL。
    fn save_page(&mut self, fmt: ConfigFormat) {
        if let Some(dup) = self.find_duplicate_keys() {
            self.status = format!("key 重复: {}，已取消保存", dup);
            return;
        }
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        match &target {
            PageTarget::Current(_) => {
                if let Some(err) = self.load_error.clone() {
                    self.status = format!("当前文件: 加载失败({})，已跳过", err);
                    return;
                }
            }
            PageTarget::Default(_) => {
                if !self.targets.iter().any(|t| t.backend == fmt && t.available) {
                    self.status = format!("{}: 未安装（本地与 WSL 均未找到配置）", fmt.label());
                    return;
                }
            }
            PageTarget::Modified(_) => {}
        }
        let res = self.save_backend_to(fmt, &path);
        let ok = res.is_ok();
        self.status = match res {
            Ok(backup) => {
                let mut msg = format!("{}: 已保存", fmt.label());
                if let Some(backup) = backup {
                    msg.push_str(&format!("（跨格式转换，原文件已备份为 {}）", backup));
                }
                msg
            }
            Err(e) => format!("{}: 保存失败({})", fmt.label(), e),
        };
        if ok {
            // 该格式不支持 agents 时明确告知，避免误以为已写入
            if fmt != ConfigFormat::Opencode && !self.agents.is_empty() {
                self.status.push_str(&format!(
                    "（{} 个 agents 未写入：该格式不支持）",
                    self.agents.len()
                ));
            }
            let bad = self.count_invalid_numeric_fields();
            if bad > 0 {
                self.status
                    .push_str(&format!("（已忽略 {} 个无效数字字段）", bad));
            }
        }
        // WSL 同步：仅勾选“WSL同步”且写入路径为本地时，同步到 WSL 侧默认路径；
        // 写入前检测对应 agent 是否已安装（未安装则跳过并提示）。
        if self.sync_wsl && ok && !is_wsl_path(&path) {
            match backends::wsl_target(fmt) {
                Some(wsl_path) => match self.save_backend_to(fmt, &wsl_path) {
                    Ok(_) => self
                        .status
                        .push_str(&format!("; {}(WSL): 已同步", fmt.label())),
                    Err(e) => {
                        self.status
                            .push_str(&format!("; {}(WSL): 同步失败({})", fmt.label(), e))
                    }
                },
                None => self
                    .status
                    .push_str(&format!("; {}(WSL): 未安装，跳过同步", fmt.label())),
            }
        }
    }

    /// 通用保存：按后端构造 root、渲染内容并写入。
    fn save_backend_to(&mut self, fmt: ConfigFormat, path: &str) -> Result<Option<String>, String> {
        let backend = backends::backend(fmt);
        // 已加载的当前文件整体替换；同格式的其他路径（WSL 同步等）先读后合并；
        // 跨格式目标（来源格式不同）做「干净转换」：目标文件里由组件状态接管的
        // provider / agent 容器整体丢弃（条目与顺序都来自界面），其余顶层字段保留。
        let is_current = self.source_format == fmt && path == self.loaded_path;
        let cross_format = self.source_format != fmt;
        let target_root: Option<Value> = if is_current {
            None
        } else {
            let mut target = backend.load_target_root(path);
            if cross_format {
                strip_cross_format_containers(fmt, &mut target, !self.agents.is_empty());
            }
            Some(target)
        };
        let root = backend.serialize_root(
            &self.agents,
            &self.providers,
            self.extras_for(fmt),
            target_root.as_ref(),
        );
        let content = if fmt == ConfigFormat::DeepSeekHarness && is_current {
            // 未发生任何结构化修改时直接保留原始 YAML，避免无意义的
            // 缩进、引号、键顺序变化；实际修改后再使用稳定的 DSH 渲染器。
            match util::read_config_content(path) {
                Ok(original)
                    if util::parse_yaml_content(&original).ok().as_ref() == Some(&root) =>
                {
                    original
                }
                _ => backend.render(&root, self.save_format == SaveFormat::Compact)?,
            }
        } else {
            backend.render(&root, self.save_format == SaveFormat::Compact)?
        };
        let dsh_sidecar_backup = if fmt == ConfigFormat::DeepSeekHarness {
            let sidecar = credentials::sidecar_path(path);
            if util::config_exists(&sidecar) {
                Some((sidecar.clone(), util::read_config_content(&sidecar)?))
            } else {
                Some((sidecar, String::new()))
            }
        } else {
            None
        };
        // 跨格式转换会整体接管目标文件的 provider/agent：先把原文件滚动备份为 .bak，
        // 备份失败则取消保存（宁可不让存，也不能把旧配置静默抵掉）。
        let backup = if cross_format && !is_current {
            match util::read_config_content(path) {
                Ok(old) if !old.is_empty() && old != content => {
                    let backup_path = format!("{}.bak", path);
                    backends::write_config(&backup_path, &old).map_err(|e| {
                        format!("跨格式转换前备份失败（{}），已取消保存: {}", backup_path, e)
                    })?;
                    Some(backup_path)
                }
                _ => None,
            }
        } else {
            None
        };
        if fmt == ConfigFormat::DeepSeekHarness {
            backend.save_sidecars(path, &self.providers)?;
        }
        if let Err(error) = backends::write_config(path, &content) {
            if let Some((sidecar, old_content)) = dsh_sidecar_backup {
                let restore = if old_content.is_empty() {
                    if util::config_exists(&sidecar) {
                        util::remove_config(&sidecar)
                    } else {
                        Ok(())
                    }
                } else {
                    backends::write_config(&sidecar, &old_content)
                };
                if let Err(restore_error) = restore {
                    return Err(format!("{}；凭据回滚失败: {}", error, restore_error));
                }
            }
            return Err(error);
        }
        // 当前文件保存成功后，回填 opencode 的 extras 载体（self.root）保持与磁盘一致
        if is_current && fmt == ConfigFormat::Opencode {
            self.root = root;
        }
        Ok(backup)
    }

    /// 当前文件保存时使用的基底 extras（按后端取对应载体）。
    fn extras_for(&self, fmt: ConfigFormat) -> &Value {
        match fmt {
            ConfigFormat::Opencode => &self.root,
            // pi 系（pi / oh-my-pi）共用 extras 载体：providers 之外的顶层字段
            ConfigFormat::Pi | ConfigFormat::OhMyPi => &self.pi_extras,
            ConfigFormat::DeepSeekHarness => &self.root,
        }
    }

    /// 预览面板左边缘的拖动分隔条：拖拽调整预览宽度比例（窗口缩放时按比例适配）。
    /// 用 Foreground 层的 Area 承载热区，避免被同层的文本框/滚动区抢走拖拽。
    fn ui_preview_resizer(&mut self, ctx: &egui::Context, panel_rect: egui::Rect, screen_w: f32) {
        let strip = egui::Rect::from_min_max(
            egui::pos2(panel_rect.left() - 4.0, panel_rect.top()),
            egui::pos2(panel_rect.left() + 4.0, panel_rect.bottom()),
        );
        let resp = egui::Area::new(egui::Id::new("preview_resizer"))
            .order(egui::Order::Foreground)
            .fixed_pos(strip.min)
            .show(ctx, |ui| {
                let (rect, resp) =
                    ui.allocate_exact_size(strip.size(), egui::Sense::click_and_drag());
                let active = resp.hovered() || resp.dragged();
                if active {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                let color = if active {
                    ui.visuals().widgets.hovered.bg_stroke.color
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke.color
                };
                ui.painter().vline(
                    rect.center().x,
                    rect.y_range(),
                    egui::Stroke::new(1.0, color),
                );
                resp
            })
            .inner;
        if resp.dragged() {
            // 分隔条向右拖 → 预览变窄 → 比例减小。
            let dx = resp.drag_delta().x;
            self.preview_ratio = (self.preview_ratio - dx / screen_w.max(1.0)).clamp(0.15, 0.85);
        }
    }

    /// 预览文本语法：opencode / pi 为 JSON(C)，omp / DSH 为 YAML；
    /// 另按内容首字符兜底（`{` / `[` 视为 JSON），避免格式与页面不匹配时高亮错乱。
    fn preview_syntax(&self, text: &str) -> PreviewSyntax {
        // 内容兜底：以 `{` / `[` 开头一律按 JSON 处理（例如误把 JSON 当 YAML 页面导入）。
        if matches!(
            text.trim_start().as_bytes().first(),
            Some(b'{') | Some(b'[')
        ) {
            return PreviewSyntax::Json;
        }
        // opencode: opencode.json(c) / pi: ~/.pi/agent/models.json（JSONC）
        // omp: models.yml / DSH: settings.yaml（YAML）
        match self.current_page {
            ConfigFormat::Opencode | ConfigFormat::Pi => PreviewSyntax::Json,
            ConfigFormat::OhMyPi | ConfigFormat::DeepSeekHarness => PreviewSyntax::Yaml,
        }
    }

    /// 重置预览编辑状态，让草稿在下一帧按当前组件状态重建。
    /// 切页/重新加载/打开预览时调用：草稿只在「预览未聚焦且上次解析成功」时
    /// 才跟随组件状态，否则会停留在上一页的内容上。
    fn reset_preview_draft(&mut self) {
        self.preview_focused = false;
        self.preview_parse_ok = true;
        self.preview_parse_error = None;
        self.preview_dirty_at = None;
        self.preview_edit_at = None;
    }

    /// 预览面板：右侧实时展示「待保存文档」（与保存按钮同路径、同合并语义）；
    /// 文本框始终可编辑：编辑内容实时解析并应用回组件，停止输入后自动写盘。
    fn ui_preview_panel(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        // 待保存文档：与 page_save_path / save_backend_to 相同路径与合并逻辑。
        let doc = self.preview_document();
        // 组件状态是「待保存文档」的唯一来源：只要用户没在预览框里手改（停止输入
        // 超过 PREVIEW_EDIT_IDLE_SECS）且上次解析没失败，就按组件状态重建草稿。
        // 不再依赖「预览是否持有焦点」—— 焦点残留或解析失败会让预览停在旧内容上，
        // 用户再动一下预览还会把旧内容解析回组件，导致保存写回旧配置。
        if preview_should_rebuild(
            self.preview_parse_error.is_some(),
            self.preview_edit_at,
            now,
        ) {
            if let Ok((_, text)) = &doc {
                if text != &self.preview_draft {
                    self.preview_draft = text.clone();
                }
            }
        }
        // 顶部：标题 + 行数/总行数（不显示路径）；格式报错直接排在行数右侧。
        let total_lines = self.preview_draft.chars().filter(|c| *c == '\n').count() + 1;
        let mut regenerate = false;
        ui.horizontal(|ui| {
            ui.strong("预览编辑");
            ui.label(
                egui::RichText::new(format!("{} / {} 行", self.preview_cursor_line, total_lines))
                    .small()
                    .weak(),
            )
            .on_hover_text("光标所在行 / 待保存文档总行数");
            if let Some(e) = &self.preview_parse_error {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 120), "⚠ 格式错误");
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(e)
                            .small()
                            .color(egui::Color32::from_rgb(235, 170, 170)),
                    )
                    .wrap(),
                )
                .on_hover_text("继续编辑修正，或点「重新生成」/ 切走再切回以撤销文本修改");
                regenerate = ui
                    .button("重新生成")
                    .on_hover_text("放弃预览里的修改，按左侧组件状态重新生成待保存文档")
                    .clicked();
            } else if let Err(e) = &doc {
                ui.colored_label(egui::Color32::from_rgb(220, 90, 90), "生成失败")
                    .on_hover_text(e);
            }
        });
        if regenerate {
            self.reset_preview_draft();
        }
        ui.separator();
        // Ctrl+F：激活查找（读原始按键事件，避免被文本框消耗）。
        let ctrl_f = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::F,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command
                )
            })
        });
        if ctrl_f {
            self.preview_find_active = true;
            self.preview_find_focus = true;
        }
        // 查找栏：Enter 下一个 / Shift+Enter 上一个 / Esc 关闭。
        if self.preview_find_active {
            ui.horizontal(|ui| {
                let resp =
                    ui.add(egui::TextEdit::singleline(&mut self.preview_find).desired_width(150.0));
                if self.preview_find_focus {
                    resp.request_focus();
                    self.preview_find_focus = false;
                }
                let matches = find_matches(&self.preview_draft, &self.preview_find);
                let total = matches.len();
                if total == 0 {
                    ui.colored_label(egui::Color32::from_rgb(220, 90, 90), "0 处");
                } else {
                    if self.preview_find_index >= total {
                        self.preview_find_index = 0;
                    }
                    ui.label(
                        egui::RichText::new(format!("{} / {}", self.preview_find_index + 1, total))
                            .small()
                            .weak(),
                    );
                }
                let mut step: i32 = 0;
                if ui.button("⬆").clicked() {
                    step = -1;
                }
                if ui.button("⬇").clicked() {
                    step = 1;
                }
                if ui.button("×").clicked() {
                    self.preview_find_active = false;
                    self.preview_find.clear();
                    self.preview_find_index = 0;
                    self.preview_find_jump = None;
                }
                if resp.has_focus() {
                    let keys = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                            i.modifiers.shift,
                        )
                    });
                    if keys.0 {
                        step = if keys.2 { -1 } else { 1 };
                    }
                    if keys.1 {
                        self.preview_find_active = false;
                        self.preview_find.clear();
                        self.preview_find_index = 0;
                        self.preview_find_jump = None;
                    }
                }
                if step != 0 && total > 0 {
                    self.preview_find_index =
                        (self.preview_find_index as i32 + step).rem_euclid(total as i32) as usize;
                    if let Some((start, _)) = matches.get(self.preview_find_index) {
                        self.preview_find_jump = Some(*start);
                    }
                }
            });
        }
        // 文本框：常规自上而下布局的最后一个元素，占满剩余高度，
        // 滚轮/滚动条均正常（用 bottom_up 会把滚动错位到底部）。
        let text_width = (ui.available_width() - 14.0).max(120.0);
        let mut edited = false;
        let mut cursor_line: Option<usize> = None;
        // 查找高亮：命中段加底色，当前命中用更亮的底色。
        let find_query = self.preview_find.clone();
        let find_active = self.preview_find_active && !find_query.is_empty();
        let find_matches = if find_active {
            find_matches(&self.preview_draft, &find_query)
        } else {
            Vec::new()
        };
        let find_current = self
            .preview_find_index
            .min(find_matches.len().saturating_sub(1));
        let find_jump = self.preview_find_jump.take();
        let syntax = self.preview_syntax(&self.preview_draft);
        let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = text.as_str();
            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let mut job = egui::text::LayoutJob::default();
            // 自适应换行：使用 TextEdit 传入的换行宽度，长行不再溢出面板。
            job.wrap.max_width = wrap_width;
            let base = ui.visuals().text_color();
            let push = |job: &mut egui::text::LayoutJob, seg: &str, color: egui::Color32| {
                job.append(
                    seg,
                    0.0,
                    egui::TextFormat {
                        font_id: font_id.clone(),
                        color,
                        ..Default::default()
                    },
                );
            };
            // 1) 语法着色（VSCode Dark+ 配色，opencode=JSON / 其余=YAML）
            let mut pos = 0;
            for (start, end, color) in syntax_tokens(text, syntax) {
                if start > pos {
                    push(&mut job, &text[pos..start], base);
                }
                if end > start {
                    push(&mut job, &text[start..end], color);
                }
                pos = pos.max(end);
            }
            if pos < text.len() {
                push(&mut job, &text[pos..], base);
            }
            // 2) 查找命中底色叠加在语法色之上
            if !find_matches.is_empty() {
                apply_find_background(&mut job, &find_matches, find_current);
            }
            ui.painter().layout_job(job)
        };
        egui::ScrollArea::vertical()
            .id_salt("preview_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut self.preview_draft)
                    .id(egui::Id::new(PREVIEW_EDITOR_ID))
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(text_width)
                    .desired_rows(24)
                    .hint_text("在此直接编辑：改动实时应用到左侧组件，停止输入约 0.8s 后自动保存")
                    .layouter(&mut layouter);
                // 用 show 而非 add：需要 output.cursor_range 计算光标所在行。
                let output = edit.show(ui);
                let resp = output.response;
                // 查找命中跳转：把光标移到命中处并写回状态，滚动区随之滚动。
                if let Some(byte) = find_jump {
                    let bounded = byte.min(self.preview_draft.len());
                    let char_idx = self.preview_draft[..bounded].chars().count();
                    let ccursor = egui::text::CCursor::new(char_idx);
                    let mut state = output.state.clone();
                    state
                        .cursor
                        .set_char_range(Some(egui::text_selection::CCursorRange::two(
                            ccursor, ccursor,
                        )));
                    state.store(ui.ctx(), resp.id);
                    // 主动把命中位置滚入视野：egui 只在文本框内容变化时自动滚动到光标，
                    // 通过按钮/Enter 跳转时光标是外部设置的，需要自己请求滚动。
                    let rect = output
                        .galley
                        .pos_from_cursor(ccursor)
                        .translate(output.galley_pos.to_vec2());
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
                self.preview_focused = resp.has_focus();
                if let Some(range) = output.cursor_range {
                    let idx = range.primary.index;
                    let line = self
                        .preview_draft
                        .chars()
                        .take(idx)
                        .filter(|c| *c == '\n')
                        .count()
                        + 1;
                    cursor_line = Some(line);
                }
                if resp.changed() && self.preview_focused {
                    edited = true;
                }
            });
        if let Some(line) = cursor_line {
            self.preview_cursor_line = line;
        }
        // 编辑 → 实时解析并应用回组件状态（解析失败不写盘、不覆盖）。
        if edited {
            self.apply_preview_draft();
            self.preview_dirty_at = Some(now);
            self.preview_edit_at = Some(now);
        }
        // 防抖自动保存：解析成功且停止输入 0.8s 后写盘。
        if let Some(at) = self.preview_dirty_at {
            if now - at > 0.8 {
                self.preview_dirty_at = None;
                self.preview_autosave();
            }
        }
    }

    /// 生成当前页面「待保存文档」：目标路径 + 序列化内容。
    /// 与保存一致：非当前文件目标先读目标文件并按目标格式合并（upsert）。
    fn preview_document(&self) -> Result<(String, String), String> {
        let fmt = self.current_page;
        let backend = backends::backend(fmt);
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        let is_current = self.source_format == fmt && path == self.loaded_path;
        // 与 save_backend_to 同一套语义：跨格式目标做干净转换（provider 容器由界面接管），
        // 预览显示的内容就是保存将要写出的内容。
        let target_root = if is_current {
            None
        } else {
            let mut target = backend.load_target_root(&path);
            if self.source_format != fmt {
                strip_cross_format_containers(fmt, &mut target, !self.agents.is_empty());
            }
            Some(target)
        };
        let root = backend.serialize_root(
            &self.agents,
            &self.providers,
            self.extras_for(fmt),
            target_root.as_ref(),
        );
        let content = backend.render(&root, self.save_format == SaveFormat::Compact)?;
        Ok((path, content))
    }

    /// 把预览编辑内容解析并写回左侧组件状态；成功返回 true。
    /// 仅更新内存状态，不落盘（落盘由自动保存/立即保存负责）。
    fn apply_preview_draft(&mut self) -> bool {
        let fmt = self.current_page;
        let content = self.preview_draft.clone();
        match backends::backend(fmt).parse_at(&content, &self.config_path) {
            Ok(load) => {
                self.root = load.root;
                self.agents = load.agents;
                self.providers = load.providers;
                self.pi_extras = load.extras;
                self.load_error = None;
                self.source_format = fmt;
                self.preview_parse_ok = true;
                self.preview_parse_error = None;
                self.agent_open = self.agents.iter().map(|a| a.key.clone()).collect();
                self.provider_open = self.providers.iter().map(|p| p.key.clone()).collect();
                self.model_fetch.clear();
                self.model_fetch_open.clear();
                self.latency.clear();
                self.probe.release(None);
                true
            }
            Err(e) => {
                self.preview_parse_ok = false;
                self.preview_parse_error = Some(e.clone());
                self.status = format!("预览内容解析失败：{}（继续编辑或撤销）", e);
                false
            }
        }
    }

    /// 实时保存：把当前待保存文档写入目标文件（仅本地，不触发 WSL 同步；
    /// 解析失败、目标不可用时跳过并提示，绝不写坏文件）。
    fn preview_autosave(&mut self) {
        if !self.preview_parse_ok {
            self.status = "预览内容解析失败，未保存（修正文本后会自动保存）".into();
            return;
        }
        let fmt = self.current_page;
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        let usable = match &target {
            PageTarget::Default(_) => self.targets.iter().any(|t| t.backend == fmt && t.available),
            _ => true,
        };
        if !usable {
            self.status = format!(
                "{}: 目标不可用（{}），未实时保存——请用保存按钮",
                fmt.label(),
                path
            );
            return;
        }
        match self.save_backend_to(fmt, &path) {
            Ok(backup) => {
                self.status = match backup {
                    Some(backup) => format!(
                        "{}: 已实时保存（跨格式转换，原文件已备份为 {}）",
                        fmt.label(),
                        backup
                    ),
                    None => format!("{}: 已实时保存", fmt.label()),
                }
            }
            Err(e) => self.status = format!("{}: 实时保存失败({})", fmt.label(), e),
        }
    }

    /// 按格式取官方图标纹理（图标未加载时返回 None）。
    fn icon_for(&self, fmt: ConfigFormat) -> Option<&egui::TextureHandle> {
        let idx = backends::BACKENDS.iter().position(|b| b.id() == fmt)?;
        self.backend_icons.get(idx).and_then(|o| o.as_ref())
    }
}

/// 预览查找：大小写不敏感的字符级匹配，返回不重叠的字节区间。
fn preview_should_rebuild(parse_failed: bool, edited_at: Option<f64>, now: f64) -> bool {
    if parse_failed {
        return false;
    }
    edited_at.is_none_or(|at| now - at >= PREVIEW_EDIT_IDLE_SECS)
}

fn find_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if needle.is_empty() || chars.len() < needle.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        let matched = needle
            .iter()
            .enumerate()
            .all(|(k, qc)| chars[i + k].1.to_lowercase().next() == Some(*qc));
        if matched {
            let start = chars[i].0;
            let last = chars[i + needle.len() - 1];
            out.push((start, last.0 + last.1.len_utf8()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

// ---------- 配置加载 ----------

/// 读取配置文件内容；本地与 WSL 路径统一处理，文件不存在视为新建场景返回空串。
// —— 兼容再导出：实现迁移至 util / backends，保持既有测试路径可用 ——
pub use crate::backends::opencode::merge_opencode_root;
pub use crate::util::parse_config_content;

/// 跨格式转换前，从目标 root 中剔除由当前组件状态接管的容器：
/// provider（opencode 的 provider / pi 系与 DSH 的 providers）与 opencode 的 agent。
///
/// 目的：跨格式保存/预览时，provider 条目与顺序完全以界面为准（干净转换），
/// 同时目标文件的其他顶层字段（如 DSH 的 llm-pi-ai 下其他设置）原样保留。
/// 同格式目标（WSL 同步等）不走这里，仍用保守合并。
///
/// `agents_owned` 表示界面确实持有 agents 数据。agents 只属于 opencode 页：
/// 数据来自 pi / oh-my-pi / DSH（或空载启动）时界面无从表达 agents，
/// 此时必须保留目标文件里的 agent 容器，否则会把它们静默删掉。
pub fn strip_cross_format_containers(fmt: ConfigFormat, root: &mut Value, agents_owned: bool) {
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    match fmt {
        ConfigFormat::Opencode => {
            obj.remove("provider");
            if agents_owned {
                obj.remove("agent");
            }
        }
        ConfigFormat::Pi | ConfigFormat::OhMyPi => {
            obj.remove("providers");
        }
        ConfigFormat::DeepSeekHarness => {
            if let Some(llm) = obj.get_mut("llm-pi-ai").and_then(Value::as_object_mut) {
                llm.remove("providers");
            }
        }
    }
}

/// 加载 opencode 配置；读取/解析失败返回 Err。
pub fn load_opencode_result(
    path: &str,
) -> Result<(Value, Vec<AgentRow>, Vec<ProviderRow>), String> {
    let load = crate::backends::load_backend(ConfigFormat::Opencode, path)?;
    Ok((load.root, load.agents, load.providers))
}

/// 兼容包装：失败时回退空状态（供测试与旧调用方使用）。
pub fn load_or_empty(path: &str) -> (Value, Vec<AgentRow>, Vec<ProviderRow>) {
    load_opencode_result(path)
        .unwrap_or_else(|_| (Value::Object(Map::new()), Vec::new(), Vec::new()))
}

/// 加载 pi 配置（支持本地与 WSL 路径）；读取/解析失败返回 Err。
pub fn load_pi_result(path: &str) -> Result<(Value, Vec<ProviderRow>, Value), String> {
    let load = crate::backends::load_backend(ConfigFormat::Pi, path)?;
    Ok((load.root, load.providers, load.extras))
}

#[cfg(test)]
mod tests;
