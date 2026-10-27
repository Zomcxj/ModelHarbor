//! Right-side config preview / editor panel: draft generation, syntax highlighting, find, and live save.
use super::{App, SaveFormat};
use crate::app::diff;
use crate::app::save::{strip_cross_format_containers, PageTarget};
use crate::app::syntax::{apply_find_background, syntax_tokens_with, PreviewSyntax, SyntaxPalette};
use crate::backends;
use crate::format::ConfigFormat;
use eframe::egui;

/// Fixed id of the preview editor (focus is released proactively on page switches; see `reset_preview_draft`).
pub(super) const PREVIEW_EDITOR_ID: &str = "preview_editor";

/// How long input must idle inside the preview before the draft may be rebuilt from component state (seconds).
pub(super) const PREVIEW_EDIT_IDLE_SECS: f64 = 2.0;

/// Whether the preview draft may be rebuilt from component state: not on parse failure, and only after
/// editing has idled for `PREVIEW_EDIT_IDLE_SECS`.
pub(super) fn preview_should_rebuild(parse_failed: bool, edited_at: Option<f64>, now: f64) -> bool {
    if parse_failed {
        return false;
    }
    edited_at.is_none_or(|at| now - at >= PREVIEW_EDIT_IDLE_SECS)
}

/// Preview find: case-insensitive character-level matching; returns non-overlapping byte ranges.
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

/// Clamp a byte offset down to a character boundary (and into range).
pub(super) fn floor_char_boundary(text: &str, mut byte: usize) -> usize {
    byte = byte.min(text.len());
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

/// Layer of the preview splitter: above the preview panel's `background` layer, below the token tooltip's `Foreground`.
pub(super) const PREVIEW_RESIZER_ORDER: egui::Order = egui::Order::Middle;

impl App {
    /// The drag splitter at the preview panel's left edge: drags adjust the preview width ratio (window resizes scale by it).
    /// The hot zone lives in its own `Area`.
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
            // Dragging the splitter right → narrower preview → smaller ratio.
            let dx = resp.drag_delta().x;
            self.preview_ratio = (self.preview_ratio - dx / screen_w.max(1.0)).clamp(0.15, 0.85);
        }
    }

    /// The preview text's syntax: opencode / pi are JSON(C), omp / DSH are YAML, kimi-code is TOML;
    /// a leading `{` / `[` counts as JSON.
    pub(super) fn preview_syntax(&self, text: &str) -> PreviewSyntax {
        // Content fallback: a leading `{` / `[` is treated as JSON.
        if matches!(
            text.trim_start().as_bytes().first(),
            Some(b'{') | Some(b'[')
        ) {
            return PreviewSyntax::Json;
        }
        // opencode family: opencode.json / kilo.json / mimocode.json (all JSONC)
        // pi: ~/.pi/agent/models.json (JSONC)
        // omp: models.yml / DSH: settings.yaml (YAML)
        // zcode: provider_config.json / workbuddy: models.json / qwen-code: settings.json (JSON)
        // kimi-code: ~/.kimi-code/config.toml (TOML)
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

    /// Reset the preview editing state so the draft rebuilds next frame from current component state.
    /// Called on page switch / reload / opening the preview.
    pub(super) fn reset_preview_draft(&mut self) {
        self.preview_focused = false;
        self.preview_parse_ok = true;
        self.preview_parse_error = None;
        self.preview_dirty_at = None;
        self.preview_edit_at = None;
    }

    /// The preview panel: the right side live-shows the "pending document" (same path and merge semantics as the save button).
    /// The text box stays editable: edits parse and apply back to components live; idling triggers an auto-save.
    pub(super) fn ui_preview_panel(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        // The pending document: same path and merge logic as page_save_path / save_backend_to.
        let doc = self.preview_document();
        // Component state is the pending document's single source: when the user hasn't hand-edited the preview box
        // (idle beyond PREVIEW_EDIT_IDLE_SECS) and the last parse didn't fail, the draft rebuilds from component state.
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
        // Top: title + line count / total (no path); format errors sit directly right of the line count.
        let total_lines = self.preview_draft.chars().filter(|c| *c == '\n').count() + 1;
        let mut regenerate = false;
        ui.horizontal(|ui| {
            ui.strong(if self.preview_diff_mode {
                "预览对比"
            } else {
                "预览编辑"
            });
            // The "diff" toggle: shows changes "original file on disk → pending document" since load.
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
        // Diff mode is a read-only view: no text box, no find bar, no auto-save.
        if self.preview_diff_mode {
            self.ui_preview_diff(ui);
            return;
        }
        // Ctrl+F: activates find (reads the raw key event).
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
        // Find bar: Enter next / Shift+Enter previous / Esc closes.
        if self.preview_find_active {
            ui.horizontal(|ui| {
                let resp = crate::ui::text_edit(
                    ui,
                    egui::TextEdit::singleline(&mut self.preview_find).desired_width(150.0),
                );
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
        // The text box: the last element of the ordinary top-down layout, filling remaining height; wheel and scrollbar behave.
        let text_width = (ui.available_width() - 14.0).max(120.0);
        let mut edited = false;
        let mut cursor_line: Option<usize> = None;
        // Find highlights: hits get a background; the current hit a brighter one.
        let find_query = self.preview_find.clone();
        let find_active = self.preview_find_active && !find_query.is_empty();
        // Mutable: the layouter filters stale offsets against the current text.
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
        // Syntax colors pick by the current theme's lightness.
        let palette = SyntaxPalette::for_dark(self.theme.is_dark());
        let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = text.as_str();
            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let mut job = egui::text::LayoutJob::default();
            // Adaptive wrap: uses the wrap width TextEdit passes in.
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
            // 1) Syntax coloring (opencode=JSON / rest=YAML; palette follows theme lightness)
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
            // 2) Find-hit backgrounds layered over the syntax colors
            if !find_matches.is_empty() {
                // Keep only hits whose both ends are character boundaries; content-level refreshes wait for the next frame.
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
                // Use show: needs output.cursor_range to compute the cursor's line.
                let output = edit.show(ui);
                let resp = output.response;
                // The editor builds its own frame (code editor layout), so the shared ring goes on here.
                crate::ui::focus_ring(ui, &resp);
                // Find-hit jump: move the cursor onto the hit and write the state back; the scroll area follows.
                if let Some(byte) = find_jump {
                    // The jump offset may be stale: clamp back to a character boundary first.
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
                    // Actively scroll the hit into view.
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
        // Edit → parse live and apply back to component state (parse failure writes nothing to disk).
        if edited {
            self.apply_preview_draft();
            self.preview_dirty_at = Some(now);
            self.preview_edit_at = Some(now);
        }
        // Debounced auto-save: on a successful parse, write 0.8s after input stops.
        if let Some(at) = self.preview_dirty_at {
            if now - at > 0.8 {
                self.preview_dirty_at = None;
                self.preview_autosave();
            }
        }
    }

    /// Build the current page's "pending document": target path + serialized content.
    /// Same as saving: non-current-file targets read the target file first and merge per its format (upsert).
    pub(super) fn preview_document(&self) -> Result<(String, String), String> {
        let fmt = self.current_page;
        let backend = backends::backend(fmt);
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        let is_current = self.source_format == fmt && path == self.loaded_path;
        // Same semantics as save_backend_to: cross-format targets do a clean conversion (the provider container is
        // UI-managed), and the agent's model normalizes per the target page's gateway.
        let agents = if self.source_format != fmt {
            self.agents_for_page(fmt)
        } else {
            self.agents.clone()
        };
        let target_root = if is_current {
            None
        } else {
            // Same semantics as saving: an unreadable target file is an error.
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

    /// Parse the preview's edited content and write it back to the left-side component state; true on success.
    /// Updates in-memory state only — no disk write (auto-save / explicit save handle that).
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
                // Surviving cards keep their collapse state; new cards aren't in the set and naturally expand.
                self.prune_collapsed();
                self.latency.clear();
                // The agents' key set may be replaced by the preview content; each page's model memory is kept.
                self.probe.release(None);
                true
            }
            Err(e) => {
                self.preview_parse_ok = false;
                self.preview_parse_error = Some(e.clone());
                self.set_status_err(format!("预览内容解析失败：{}（继续编辑或撤销）", e));
                false
            }
        }
    }

    /// Live save: write the current pending document to its target file (local only; no WSL sync triggered).
    /// Parse failures and unavailable targets skip with a note.
    pub(super) fn preview_autosave(&mut self) {
        if !self.preview_parse_ok {
            self.set_status_err("预览内容解析失败，未保存（修正文本后会自动保存）");
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
            self.set_status_err(format!(
                "{}: 目标不可用（{}），未实时保存——请用保存按钮",
                fmt.label(),
                path
            ));
            return;
        }
        match self.save_backend_to(fmt, &path) {
            Ok(backup) => {
                self.set_status_ok(match backup {
                    Some(backup) => {
                        format!("{}: 已实时保存（原文件已备份为 {}）", fmt.label(), backup)
                    }
                    None => format!("{}: 已实时保存", fmt.label()),
                });
            }
            Err(e) => self.set_status_err(format!("{}: 实时保存失败({})", fmt.label(), e)),
        }
    }

    /// Diff view: shows "the target file's current content → pending document" changes line by line.
    ///
    /// The baseline is **the target file's content on disk right now**. Results cache under the
    /// `(path, draft, write count)` signature; after a signature change, recomputation waits
    /// [`DIFF_DEBOUNCE_SECS`] of idle, with the old result serving meanwhile.
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
            // An unreadable target file (missing / no permission) diffs against empty content.
            let baseline = crate::util::read_config_content(&path).unwrap_or_default();
            let (lines, summary) =
                diff::diff_hunks(&baseline, &self.preview_draft, diff::CONTEXT_LINES);
            self.preview_diff_cache = Some((signature, lines, summary));
        } else if cached != Some(signature) {
            // Explicitly request one repaint when due, guaranteeing a frame exists when the debounce expires.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(DIFF_DEBOUNCE_SECS));
        }
        let Some((_, lines, summary)) = &self.preview_diff_cache else {
            // No result yet (diff just opened): show a hint line.
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
                    // All changes, no context: usually a new file or a whole-file replacement.
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

/// Debounce duration for diff recomputation (seconds).
const DIFF_DEBOUNCE_SECS: f64 = 0.5;

/// Whether diff recomputation is due: the same signature must idle for [`DIFF_DEBOUNCE_SECS`].
/// `pending` is the `(signature, first-seen moment)` recorded last frame.
pub(super) fn diff_recompute_due(pending: Option<(u64, f64)>, signature: u64, now: f64) -> bool {
    match pending {
        Some((p, at)) if p == signature => now - at >= DIFF_DEBOUNCE_SECS,
        _ => false,
    }
}

/// One step of the debounce state machine: returns `(recompute this frame, next pending)`.
///
/// `cached` is the cached result's signature (`None` = no result yet); with nothing to show, compute immediately.
pub(super) fn diff_step(
    cached: Option<u64>,
    pending: Option<(u64, f64)>,
    signature: u64,
    now: f64,
) -> (bool, Option<(u64, f64)>) {
    if cached == Some(signature) {
        // Cache already fresh: clear the timing state.
        return (false, None);
    }
    if cached.is_none() || diff_recompute_due(pending, signature, now) {
        return (true, None);
    }
    // Record the moment only when the signature **first appears**; later frames keep it.
    match pending {
        Some((p, at)) if p == signature => (false, Some((p, at))),
        _ => (false, Some((signature, now))),
    }
}

/// Diff cache signature: target path + pending document + write count.
/// Uses the std default hash; only for deciding "recompute or not".
pub(super) fn diff_signature(path: &str, draft: &str, save_serial: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    draft.hash(&mut hasher);
    save_serial.hash(&mut hasher);
    hasher.finish()
}
