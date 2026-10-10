//! 用量视图的界面：顶部视图切换 + 四维统计表。
//!
//! 数据全部来自 [`super::super::usage`] 的后台扫描结果（`App::usage`），这里只负责画；
//! 聚合与格式化在 [`super::aggregate`] 里，是可单测的纯逻辑。
//!
//! 切维度按钮照顶栏页签的选中样式（`widgets.hovered.bg_fill` + 2px 描边），
//! 配色一律走 `crate::theme::semantics(ui)`，不硬编码颜色。

use super::aggregate::{self, AgentUsage, DayUsage, Dimension, ModelUsage, Totals};
use crate::app::App;
use crate::usage::SessionSnapshot;
use eframe::egui;

/// 切维度按钮的宽度：四个标签都能装下，且不随标签长短跳动。
const DIM_BUTTON_WIDTH: f32 = 68.0;

/// 会话 id 在表里最多显示多少个字符。
const SESSION_ID_CHARS: usize = 20;

impl App {
    /// 中央区域顶部的视图切换：`[用量] [提供商管理]` 二选一。
    ///
    /// 放在页头下方、内容区上方 —— 它决定下面显示哪一块，位置必须在两者之间。
    pub(in crate::app) fn ui_view_switch(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for view in [
                super::super::MainView::Usage,
                super::super::MainView::Providers,
            ] {
                if self.view_switch_button(ui, view) {
                    self.main_view = view;
                }
            }
        });
        ui.add_space(crate::theme::SPACE_2);
    }

    /// 单个视图切换按钮：选中态 = 悬浮色底 + 2px 描边（与顶栏页签同一套）。
    fn view_switch_button(&self, ui: &mut egui::Ui, view: super::super::MainView) -> bool {
        let selected = self.main_view == view;
        let visuals = ui.visuals();
        let (fill, stroke) = if selected {
            (
                visuals.widgets.hovered.bg_fill,
                egui::Stroke::new(2.0f32, visuals.widgets.hovered.bg_stroke.color),
            )
        } else {
            (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
        };
        let text = egui::RichText::new(view.label());
        let text = if selected { text.strong() } else { text };
        ui.add(
            egui::Button::new(text)
                .fill(fill)
                .stroke(stroke)
                .min_size(egui::vec2(88.0, 24.0)),
        )
        .clicked()
    }

    /// 用量视图主体：状态行 + 维度切换 + 表格。
    pub(in crate::app) fn ui_usage_section(&mut self, ui: &mut egui::Ui) {
        self.ui_usage_status(ui);
        ui.add_space(crate::theme::SPACE_3);
        self.ui_usage_dimensions(ui);
        ui.add_space(crate::theme::SPACE_2);
        self.ui_usage_table(ui);
    }

    /// 顶部状态行：合计、扫描进度、数据新鲜度与错误。
    fn ui_usage_status(&mut self, ui: &mut egui::Ui) {
        let semantics = crate::theme::semantics(ui);
        let scanning = self.usage.scanning();
        let now_ms = unix_now_ms();
        let frame_time = ui.input(|input| input.time);
        // 先把要画的数据取出来：后面 `self` 还要可变借用（维度切换），
        // 提前结束对 `self.usage` 的借用更省心。
        let report = match self.usage.result.as_ref() {
            Some(Ok(report)) => Some((
                report.sessions.len(),
                report.total(),
                report.message_count,
                report.scan_ms,
                report.scanned_at,
            )),
            _ => None,
        };
        let error = match self.usage.result.as_ref() {
            Some(Err(err)) => Some(err.clone()),
            _ => None,
        };
        let ledger_len = self.usage.ledger_len();
        let watched = super::scan_sources::watched_agent_count();

        ui.horizontal_wrapped(|ui| {
            ui.strong("本机用量");
            ui.label(
                egui::RichText::new(format!(
                    "监视 {watched} 个 agent 的本地会话账本，每 3 秒检测一次变化"
                ))
                .small()
                .weak(),
            );
        });
        ui.horizontal_wrapped(|ui| {
            match &report {
                Some((sessions, total, messages, scan_ms, scanned_at)) => {
                    ui.label(format!("合计 {}", aggregate::format_tokens(*total)));
                    ui.label(egui::RichText::new("·").weak());
                    ui.label(format!("{} 个会话", sessions));
                    ui.label(egui::RichText::new("·").weak());
                    ui.label(format!("{} 条消息", messages));
                    ui.label(egui::RichText::new("·").weak());
                    // 「更新于 N 秒前」：把 egui 时间轴上的扫描时刻换算成距今多久。
                    let age_ms = ((frame_time - scanned_at).max(0.0) * 1_000.0) as i64;
                    ui.label(
                        egui::RichText::new(format!(
                            "更新于 {}",
                            aggregate::relative_time(now_ms - age_ms, now_ms)
                        ))
                        .small()
                        .weak(),
                    );
                    ui.label(
                        egui::RichText::new(format!("（内核耗时 {scan_ms} ms）"))
                            .small()
                            .weak(),
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new("还没有扫描结果")
                            .small()
                            .color(semantics.warn),
                    );
                }
            }
            if scanning {
                ui.spinner();
                ui.label(egui::RichText::new("扫描中…").small().weak());
            }
        });
        if let Some(err) = error {
            ui.label(egui::RichText::new(format!("上次扫描失败：{err}")).color(semantics.err));
        }
        ui.label(
            egui::RichText::new(format!("账本保留已删除会话的用量（共 {ledger_len} 条）"))
                .small()
                .weak(),
        );
    }

    /// 四个维度的切换按钮。
    fn ui_usage_dimensions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for dimension in Dimension::ALL {
                let selected = self.usage_dimension == dimension;
                let visuals = ui.visuals();
                let (fill, stroke) = if selected {
                    (
                        visuals.widgets.hovered.bg_fill,
                        egui::Stroke::new(2.0f32, visuals.widgets.hovered.bg_stroke.color),
                    )
                } else {
                    (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
                };
                let text = egui::RichText::new(dimension.label());
                let text = if selected { text.strong() } else { text };
                let response = ui.add(
                    egui::Button::new(text)
                        .fill(fill)
                        .stroke(stroke)
                        .min_size(egui::vec2(DIM_BUTTON_WIDTH, 24.0)),
                );
                if response.clicked() {
                    self.usage_dimension = dimension;
                }
            }
        });
    }

    /// 当前维度的表格。
    fn ui_usage_table(&mut self, ui: &mut egui::Ui) {
        // 会话维度需要 `archived` 标记，所以直接用快照切片；其余维度在这里现算聚合。
        let sessions: Vec<SessionSnapshot> = self.usage_sessions().to_vec();
        if sessions.is_empty() {
            let semantics = crate::theme::semantics(ui);
            ui.label(egui::RichText::new("没有找到本机用量数据。").color(semantics.warn));
            ui.label(
                egui::RichText::new(
                    "统计的是各 agent 自己写在磁盘上的会话账本（pi / opencode / \
                     workbuddy / dsh 等）。先用其中任意一个跑一轮对话，数字就会出现在这里。",
                )
                .small()
                .weak(),
            );
            return;
        }
        let offset = crate::app::balance::local_utc_offset_secs();
        let now_ms = unix_now_ms();
        match self.usage_dimension {
            Dimension::Agent => {
                let rows = aggregate::by_agent(&sessions);
                usage_table_shell(ui, |ui| agent_table(ui, &rows, now_ms));
            }
            Dimension::Model => {
                let rows = aggregate::by_model(&sessions);
                usage_table_shell(ui, |ui| model_table(ui, &rows, now_ms));
            }
            Dimension::Session => {
                let rows = aggregate::session_rows(&sessions);
                usage_table_shell(ui, |ui| session_table(ui, &rows, now_ms));
            }
            Dimension::Day => {
                let rows = aggregate::by_day(&sessions, offset);
                let today = aggregate::local_day_index(unix_now_secs(), offset);
                usage_table_shell(ui, |ui| day_table(ui, &rows, today, now_ms));
            }
        }
    }
}

/// 当前时间（Unix 毫秒）。
fn unix_now_ms() -> i64 {
    unix_now_secs().saturating_mul(1_000)
}

/// 当前时间（Unix 秒）。
fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 表格外壳：统一内边距，并撑满可用宽度。
fn usage_table_shell(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}

/// 表头一行：列名弱化显示。
fn table_header(ui: &mut egui::Ui, columns: &[&str]) {
    ui.horizontal(|ui| {
        let mut first = true;
        for column in columns {
            if !first {
                ui.label(egui::RichText::new("·").weak());
            }
            first = false;
            ui.label(egui::RichText::new(*column).small().weak());
        }
    });
}

/// 用量数字块：合计 + 输入/输出/缓存读 明细。四维表格共用这一段。
fn usage_numbers(ui: &mut egui::Ui, totals: &Totals) {
    ui.label(
        egui::RichText::new(aggregate::format_tokens(totals.total()))
            .strong()
            .monospace(),
    );
    ui.label(
        egui::RichText::new(format!(
            "输入 {} · 输出 {} · 缓存读 {}",
            aggregate::format_tokens(totals.input),
            aggregate::format_tokens(totals.output),
            aggregate::format_tokens(totals.cache_read)
        ))
        .small()
        .weak(),
    );
}

/// Agent 维度表：agent 名 / 用量 / 会话数 / 最近活动。
fn agent_table(ui: &mut egui::Ui, rows: &[AgentUsage], now_ms: i64) {
    table_header(
        ui,
        &["Agent", "用量（输入 · 输出 · 缓存读）", "会话", "最近活动"],
    );
    for row in rows {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(row.client.label()).strong());
            usage_numbers(ui, &row.totals);
            ui.label(
                egui::RichText::new(format!("{} 个会话", row.totals.sessions))
                    .small()
                    .weak(),
            );
            ui.label(
                egui::RichText::new(aggregate::relative_time(row.last_seen_ms, now_ms))
                    .small()
                    .weak(),
            );
        });
        ui.add_space(crate::theme::SPACE_1);
    }
}

/// 模型维度表：模型名 / 用量 / 会话数 / 用到它的 agent。
fn model_table(ui: &mut egui::Ui, rows: &[ModelUsage], now_ms: i64) {
    table_header(
        ui,
        &["模型", "用量（输入 · 输出 · 缓存读）", "会话", "来源 agent"],
    );
    for row in rows {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(&row.model_id).strong().monospace());
            usage_numbers(ui, &row.totals);
            ui.label(
                egui::RichText::new(format!("{} 个会话", row.totals.sessions))
                    .small()
                    .weak(),
            );
            ui.label(egui::RichText::new(row.agents.join(" / ")).small().weak());
            ui.label(
                egui::RichText::new(aggregate::relative_time(row.last_seen_ms, now_ms))
                    .small()
                    .weak(),
            );
        });
        ui.add_space(crate::theme::SPACE_1);
    }
}

/// 会话维度表：会话 id / agent / 模型 / 合计 / 最近活动；归档的加标记。
fn session_table(ui: &mut egui::Ui, rows: &[SessionSnapshot], now_ms: i64) {
    let semantics = crate::theme::semantics(ui);
    table_header(ui, &["会话", "Agent", "模型", "合计", "最近活动"]);
    for row in rows {
        ui.horizontal(|ui| {
            // 会话 id 很长（UUID），截断中间显示；全文放悬停。
            let short = aggregate::short_session_id(&row.session_id, SESSION_ID_CHARS);
            ui.label(egui::RichText::new(short).monospace())
                .on_hover_text(&row.session_id);
            ui.label(egui::RichText::new(row.client.label()).small());
            // 模型名可能很长，弱化并截断，免得把合计挤出去。
            ui.add(egui::Label::new(egui::RichText::new(&row.model_id).small().weak()).truncate())
                .on_hover_text(&row.model_id);
            ui.label(
                egui::RichText::new(aggregate::format_tokens(row.total()))
                    .strong()
                    .monospace(),
            );
            ui.label(
                egui::RichText::new(aggregate::relative_time(row.last_seen_ms, now_ms))
                    .small()
                    .weak(),
            );
            if row.archived {
                // 已从源里消失、用量由账本保留：用「注意」色标出来，
                // 免得用户以为统计读错了。
                ui.label(egui::RichText::new("已归档").small().color(semantics.warn))
                    .on_hover_text("该会话已从源文件里删除，用量由本地账本保留");
            }
        });
        ui.add_space(crate::theme::SPACE_1);
    }
}

/// 时间维度表：日期 / 当天合计 / 当天会话数。
fn day_table(ui: &mut egui::Ui, rows: &[DayUsage], today: i64, now_ms: i64) {
    table_header(
        ui,
        &["日期", "用量（输入 · 输出 · 缓存读）", "会话", "最近活动"],
    );
    for row in rows {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(aggregate::day_label(row.day, today)).strong());
            usage_numbers(ui, &row.totals);
            ui.label(
                egui::RichText::new(format!("{} 个会话", row.totals.sessions))
                    .small()
                    .weak(),
            );
            ui.label(
                egui::RichText::new(aggregate::relative_time(row.last_seen_ms, now_ms))
                    .small()
                    .weak(),
            );
        });
        ui.add_space(crate::theme::SPACE_1);
    }
}
