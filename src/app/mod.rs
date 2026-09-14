use crate::backends;
use crate::format::{ConfigFormat, ConfigPaths};
use crate::model::{AgentRow, ProviderRow};
use crate::theme::Theme;
use eframe::egui;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

mod serialize;

pub(crate) use serialize::{compact_json, pretty_json};

mod syntax;

use syntax::{apply_find_background, syntax_tokens, PreviewSyntax};

mod fetch;

use fetch::{
    LatencyState, ModelFetchState, ProbeGate,
};

mod bars;

mod agents;

mod providers;

mod providers_form;

mod save;

use save::{PageTarget, SaveTarget};

pub use save::{load_opencode_result, load_or_empty, load_pi_result, strip_cross_format_containers};

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

/// 预览编辑框的固定 id（切页时需要主动释放焦点，见 reset_preview_draft）。
const PREVIEW_EDITOR_ID: &str = "preview_editor";
/// 预览框内停止输入多久后，允许用组件状态重建草稿（秒）。
const PREVIEW_EDIT_IDLE_SECS: f64 = 2.0;

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

#[cfg(test)]
mod tests;
