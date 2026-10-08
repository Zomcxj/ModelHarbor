//! 右侧配置预览 / 编辑面板：草稿生成、语法高亮、查找与实时保存。
use super::{App, SaveFormat};
use crate::app::diff;
use crate::app::save::{strip_cross_format_containers, PageTarget};
use crate::app::syntax::{apply_find_background, syntax_tokens_with, PreviewSyntax, SyntaxPalette};
use crate::backends;
use crate::format::ConfigFormat;
use eframe::egui;

/// 预览编辑框的固定 id（切页时主动释放焦点，见 `reset_preview_draft`）。
pub(super) const PREVIEW_EDITOR_ID: &str = "preview_editor";

/// 预览框内停止输入多久后，允许用组件状态重建草稿（秒）。
pub(super) const PREVIEW_EDIT_IDLE_SECS: f64 = 2.0;

/// 是否用组件状态重建预览草稿：解析失败时不重建，编辑后静置满 `PREVIEW_EDIT_IDLE_SECS` 才重建。
pub(super) fn preview_should_rebuild(parse_failed: bool, edited_at: Option<f64>, now: f64) -> bool {
    if parse_failed {
        return false;
    }
    edited_at.is_none_or(|at| now - at >= PREVIEW_EDIT_IDLE_SECS)
}

/// 预览查找：大小写不敏感的字符级匹配，返回不重叠的字节区间。
pub(super) fn find_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
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

/// 把字节偏移向下钳到字符边界（并钳到长度内）。
pub(super) fn floor_char_boundary(text: &str, mut byte: usize) -> usize {
    byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// 预览分隔条所在的层级：高于预览面板的 `background` 层，低于令牌悬浮窗的 `Foreground` 层。
pub(super) const PREVIEW_RESIZER_ORDER: egui::Order = egui::Order::Middle;

impl App {
    /// 预览面板左边缘的拖动分隔条：拖拽调整预览宽度比例（窗口缩放时按比例适配）。
    /// 热区由独立 `Area` 承载。
    pub(super) fn ui_preview_resizer(
        &mut self,
        ctx: &egui::Context,
        panel_rect: egui::Rect,
        screen_w: f32,
    ) {
        let strip = egui::Rect::from_min_max(
            egui::pos2(panel_rect.left() - 4.0, panel_rect.top()),
            egui::pos2(panel_rect.left() + 4.0, panel_rect.bottom()),
        );
        let resp = egui::Area::new(egui::Id::new("preview_resizer"))
            .order(PREVIEW_RESIZER_ORDER)
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
                    egui::Stroke::new(1.0f32, color),
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

    /// 预览文本语法：opencode / pi 为 JSON(C)，omp / DSH 为 YAML，kimi-code 为 TOML；
    /// 首字符为 `{` / `[` 时按 JSON。
    pub(super) fn preview_syntax(&self, text: &str) -> PreviewSyntax {
        // 内容兜底：以 `{` / `[` 开头按 JSON 处理。
        if matches!(
            text.trim_start().as_bytes().first(),
            Some(b'{') | Some(b'[')
        ) {
            return PreviewSyntax::Json;
        }
        // opencode 系: opencode.json / kilo.json / mimocode.json（均 JSONC）
        // pi: ~/.pi/agent/models.json（JSONC）
        // omp: models.yml / DSH: settings.yaml（YAML）
        // zcode: provider_config.json / workbuddy: models.json / qwen-code: settings.json（JSON）
        // kimi-code: ~/.kimi-code/config.toml（TOML）
        match self.current_page {
            ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
                PreviewSyntax::Json
            }
            ConfigFormat::Pi
            | ConfigFormat::ZCode
            | ConfigFormat::WorkBuddy
            | ConfigFormat::QwenCode => PreviewSyntax::Json,
            ConfigFormat::OhMyPi | ConfigFormat::DeepSeekHarness => PreviewSyntax::Yaml,
            ConfigFormat::KimiCode => PreviewSyntax::Toml,
        }
    }

    /// 重置预览编辑状态，让草稿在下一帧按当前组件状态重建。
    /// 切页 / 重新加载 / 打开预览时调用。
    pub(super) fn reset_preview_draft(&mut self) {
        self.preview_focused = false;
        self.preview_parse_ok = true;
        self.preview_parse_error = None;
        self.preview_dirty_at = None;
        self.preview_edit_at = None;
    }

    /// 预览面板：右侧实时展示「待保存文档」（与保存按钮同路径、同合并语义）。
    /// 文本框始终可编辑：编辑内容实时解析并应用回组件，停止输入后自动写盘。
    pub(super) fn ui_preview_panel(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        // 待保存文档：与 page_save_path / save_backend_to 相同路径与合并逻辑。
        let doc = self.preview_document();
        // 组件状态是「待保存文档」的唯一来源：用户没在预览框里手改（停止输入超过
        // PREVIEW_EDIT_IDLE_SECS）且上次解析没失败时，按组件状态重建草稿。
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
            ui.strong(if self.preview_diff_mode {
                "预览对比"
            } else {
                "预览编辑"
            });
            // 「对比」切换：显示自加载以来「磁盘原文件 → 待保存文档」的改动。
            if ui
                .selectable_label(self.preview_diff_mode, "对比")
                .on_hover_text(
                    "显示自加载以来「原文件 → 待保存文档」的逐行改动。\n\
                     基线是**加载时**的文件内容，不是磁盘当前内容——预览有自动保存，\
                     拿磁盘当前内容比会永远显示无改动。",
                )
                .clicked()
            {
                self.preview_diff_mode = !self.preview_diff_mode;
            }
            if !self.preview_diff_mode {
                ui.label(
                    egui::RichText::new(format!(
                        "{} / {} 行",
                        self.preview_cursor_line, total_lines
                    ))
                    .small()
                    .weak(),
                )
                .on_hover_text("光标所在行 / 待保存文档总行数");
            }
            if let Some(e) = &self.preview_parse_error {
                let palette = SyntaxPalette::for_dark(ui.visuals().dark_mode);
                ui.colored_label(palette.error, "⚠ 格式错误");
                ui.add(
                    egui::Label::new(egui::RichText::new(e).small().color(palette.error)).wrap(),
                )
                .on_hover_text("继续编辑修正，或点「重新生成」/ 切走再切回以撤销文本修改");
                regenerate = ui.button("重新生成").clicked();
            } else if let Err(e) = &doc {
                ui.colored_label(crate::theme::semantics(ui).err, "生成失败")
                    .on_hover_text(e);
            }
        });
        if regenerate {
            self.reset_preview_draft();
        }
        ui.separator();
        // 对比模式是只读视图：不渲染文本框，无查找栏与自动保存。
        if self.preview_diff_mode {
            self.ui_preview_diff(ui);
            return;
        }
        // Ctrl+F：激活查找（读原始按键事件）。
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
                    ui.colored_label(crate::theme::semantics(ui).err, "0 处");
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
        // 文本框：常规自上而下布局的最后一个元素，占满剩余高度，滚轮/滚动条正常。
        let text_width = (ui.available_width() - 14.0).max(120.0);
        let mut edited = false;
        let mut cursor_line: Option<usize> = None;
        // 查找高亮：命中段加底色，当前命中用更亮的底色。
        let find_query = self.preview_find.clone();
        let find_active = self.preview_find_active && !find_query.is_empty();
        // 可变：layouter 里按当前文本过滤过期偏移。
        let mut find_matches = if find_active {
            find_matches(&self.preview_draft, &find_query)
        } else {
            Vec::new()
        };
        let find_current = self
            .preview_find_index
            .min(find_matches.len().saturating_sub(1));
        let find_jump = self.preview_find_jump.take();
        let syntax = self.preview_syntax(&self.preview_draft);
        // 语法配色按当前主题明暗选。
        let palette = SyntaxPalette::for_dark(self.theme.is_dark());
        let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = text.as_str();
            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let mut job = egui::text::LayoutJob::default();
            // 自适应换行：使用 TextEdit 传入的换行宽度。
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
            // 1) 语法着色（opencode=JSON / 其余=YAML；配色随主题明暗）
            let mut pos = 0;
            for (start, end, color) in syntax_tokens_with(text, syntax, palette) {
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
                // 只保留两端都是字符边界的命中；内容级刷新等下一帧。
                find_matches.retain(|&(s, e)| text.is_char_boundary(s) && text.is_char_boundary(e));
                apply_find_background(&mut job, &find_matches, find_current, palette);
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
                // 用 show：需要 output.cursor_range 计算光标所在行。
                let output = edit.show(ui);
                let resp = output.response;
                // 查找命中跳转：把光标移到命中处并写回状态，滚动区随之滚动。
                if let Some(byte) = find_jump {
                    // 跳转偏移可能过期：先钳回字符边界。
                    let bounded = floor_char_boundary(&self.preview_draft, byte);
                    let char_idx = self.preview_draft[..bounded].chars().count();
                    let ccursor = egui::text::CCursor::new(char_idx);
                    let mut state = output.state.clone();
                    state
                        .cursor
                        .set_char_range(Some(egui::text_selection::CCursorRange::two(
                            ccursor, ccursor,
                        )));
                    state.store(ui.ctx(), resp.id);
                    // 主动把命中位置滚入视野。
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
        // 编辑 → 实时解析并应用回组件状态（解析失败不写盘）。
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
    pub(super) fn preview_document(&self) -> Result<(String, String), String> {
        let fmt = self.current_page;
        let backend = backends::backend(fmt);
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        let is_current = self.source_format == fmt && path == self.loaded_path;
        // 与 save_backend_to 同一套语义：跨格式目标做干净转换（provider 容器由界面接管），
        // agent 的 model 按目标页网关归一。
        let agents = if self.source_format != fmt {
            self.agents_for_page(fmt)
        } else {
            self.agents.clone()
        };
        let target_root = if is_current {
            None
        } else {
            // 与保存同一语义：目标文件读不出即报错。
            let mut target = backend.load_target_root(&path)?;
            if self.source_format != fmt {
                strip_cross_format_containers(fmt, &mut target, !agents.is_empty());
            }
            Some(target)
        };
        let root = backend.serialize_root(
            &agents,
            &self.providers,
            self.extras_for(fmt),
            target_root.as_ref(),
        );
        let content = backend.render(&root, self.save_format == SaveFormat::Compact)?;
        Ok((path, content))
    }

    /// 把预览编辑内容解析并写回左侧组件状态；成功返回 true。
    /// 仅更新内存状态，不落盘（落盘由自动保存/立即保存负责）。
    pub(super) fn apply_preview_draft(&mut self) -> bool {
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
                // 存活卡片保留原折叠状态；新卡片不在集合中，天然展开。
                self.prune_collapsed();
                self.model_fetch.clear();
                self.model_fetch_open.clear();
                self.latency.clear();
                // agent 的 key 集合可能被预览内容换掉，各页的 model 记忆保留。
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

    /// 实时保存：把当前待保存文档写入目标文件（仅本地，不触发 WSL 同步）。
    /// 解析失败、目标不可用时跳过并提示。
    pub(super) fn preview_autosave(&mut self) {
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
                    Some(backup) => {
                        format!("{}: 已实时保存（原文件已备份为 {}）", fmt.label(), backup)
                    }
                    None => format!("{}: 已实时保存", fmt.label()),
                }
            }
            Err(e) => self.status = format!("{}: 实时保存失败({})", fmt.label(), e),
        }
    }

    /// 对比视图：逐行显示「目标文件当前内容 → 待保存文档」的改动。
    ///
    /// 基线是**此刻磁盘上该目标文件的内容**。结果按 `(路径, 草稿, 写盘次数)` 签名缓存，
    /// 签名变化后静置 [`DIFF_DEBOUNCE_SECS`] 才重算，期间沿用旧结果。
    pub(super) fn ui_preview_diff(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        let doc = self.preview_document();
        let path = match &doc {
            Ok((path, _)) => path.clone(),
            Err(e) => {
                ui.colored_label(crate::theme::semantics(ui).err, "生成失败")
                    .on_hover_text(e);
                return;
            }
        };
        let signature = diff_signature(&path, &self.preview_draft, self.save_serial);
        let cached = self.preview_diff_cache.as_ref().map(|(s, _, _)| *s);
        let (due, pending) = diff_step(cached, self.preview_diff_pending, signature, now);
        self.preview_diff_pending = pending;
        if due {
            // 读不出目标文件（还不存在 / 无权限）按空内容比。
            let baseline = crate::util::read_config_content(&path).unwrap_or_default();
            let (lines, summary) =
                diff::diff_hunks(&baseline, &self.preview_draft, diff::CONTEXT_LINES);
            self.preview_diff_cache = Some((signature, lines, summary));
        } else if cached != Some(signature) {
            // 显式要求到点重绘一次，保证防抖到期时有帧。
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(DIFF_DEBOUNCE_SECS));
        }
        let Some((_, lines, summary)) = &self.preview_diff_cache else {
            // 还没算出结果（刚打开对比）：给一行提示。
            ui.label(egui::RichText::new("正在计算差异…").small().weak());
            return;
        };
        let semantics = crate::theme::semantics(ui);
        ui.horizontal(|ui| {
            if summary.is_empty() {
                ui.colored_label(semantics.ok, "与磁盘上的文件一致，保存不会改动内容");
            } else {
                ui.colored_label(
                    semantics.ok,
                    egui::RichText::new(format!("+{}", summary.added)).monospace(),
                )
                .on_hover_text("本次保存会新增的行数");
                ui.colored_label(
                    semantics.err,
                    egui::RichText::new(format!("-{}", summary.removed)).monospace(),
                )
                .on_hover_text("本次保存会删除的行数");
                if lines.iter().all(|l| l.kind != diff::LineKind::Context) {
                    // 全是改动、没有上下文：通常是新建文件或整份替换。
                    ui.label(egui::RichText::new("（整份变更）").small().weak())
                        .on_hover_text("两侧没有公共行，说明是新建文件或整体重写");
                }
            }
        });
        if summary.is_empty() {
            return;
        }
        let weak = ui.visuals().weak_text_color();
        egui::ScrollArea::vertical()
            .id_salt("preview_diff_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for line in lines {
                    let (color, text) = match line.kind {
                        diff::LineKind::Added => (semantics.ok, format!("+{}", line.text)),
                        diff::LineKind::Removed => (semantics.err, format!("-{}", line.text)),
                        diff::LineKind::Hunk => (semantics.info, line.text.clone()),
                        diff::LineKind::Context => (weak, format!(" {}", line.text)),
                    };
                    ui.label(egui::RichText::new(text).monospace().color(color));
                }
            });
    }
}

/// 对比重算的防抖时长（秒）。
const DIFF_DEBOUNCE_SECS: f64 = 0.5;

/// 对比重算是否到点：同一签名静置满 [`DIFF_DEBOUNCE_SECS`] 才算。
/// `pending` 是上一帧记下的 `(签名, 首次见到它的时刻)`。
pub(super) fn diff_recompute_due(pending: Option<(u64, f64)>, signature: u64, now: f64) -> bool {
    match pending {
        Some((p, at)) if p == signature => now - at >= DIFF_DEBOUNCE_SECS,
        _ => false,
    }
}

/// 防抖的一步状态转移：返回 `(本帧是否重算, 下一步的 pending)`。
///
/// `cached` 是当前缓存结果的签名（`None` = 还没有任何结果），无结果可显示时立刻算。
pub(super) fn diff_step(
    cached: Option<u64>,
    pending: Option<(u64, f64)>,
    signature: u64,
    now: f64,
) -> (bool, Option<(u64, f64)>) {
    if cached == Some(signature) {
        // 缓存已是最新：清掉计时状态。
        return (false, None);
    }
    if cached.is_none() || diff_recompute_due(pending, signature, now) {
        return (true, None);
    }
    // 只在该签名**首次出现**时记时刻，后续帧沿用。
    match pending {
        Some((p, at)) if p == signature => (false, Some((p, at))),
        _ => (false, Some((signature, now))),
    }
}

/// 对比缓存的签名：目标路径 + 待保存文档 + 写盘次数。
/// 用标准库默认哈希，只用于判断「要不要重算」。
pub(super) fn diff_signature(path: &str, draft: &str, save_serial: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    draft.hash(&mut hasher);
    save_serial.hash(&mut hasher);
    hasher.finish()
}
