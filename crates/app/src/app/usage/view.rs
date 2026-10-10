//! 用量视图的界面：顶部视图切换 + 四维统计表。
//!
//! 数据全部来自 [`super::super::usage`] 的后台扫描结果（`App::usage`），这里只负责画；
//! 聚合与格式化在 [`super::aggregate`] 里，是可单测的纯逻辑。
//!
//! 切维度按钮照顶栏页签的选中样式（`widgets.hovered.bg_fill` + 2px 描边），
//! 配色一律走 `crate::theme::semantics(ui)`，不硬编码颜色。

use super::aggregate::{self, AgentUsage, DayUsage, Dimension, ModelUsage};
use super::range::Range;
use crate::app::App;
use crate::usage::SessionSnapshot;
use eframe::egui;

/// 切维度按钮的宽度：四个标签都能装下，且不随标签长短跳动。
const DIM_BUTTON_WIDTH: f32 = 68.0;

/// 会话 id 在表里最多显示多少个字符。
const SESSION_ID_CHARS: usize = 20;

impl App {
    /// 视图切换：`[用量] [提供商管理]` 二选一。
    ///
    /// 画在**顶栏第一行、agent 页签右侧**：它决定下面显示哪一块，与页签同层
    /// 但语义不同 —— 页签选 agent，这里选视图。
    ///
    /// 尺寸照页签（高 22），不另起一行、不加间距，避免把顶栏撑高。
    pub(in crate::app) fn ui_view_switch(&mut self, ui: &mut egui::Ui) {
        for view in [
            super::super::MainView::Usage,
            super::super::MainView::Providers,
        ] {
            if self.view_switch_button(ui, view) {
                self.main_view = view;
            }
        }
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
                .min_size(egui::vec2(72.0, 22.0)),
        )
        .clicked()
    }

    /// 用量视图主体：状态行 + 控件行 + 总览（含热力图）+ 表格。
    pub(in crate::app) fn ui_usage_section(&mut self, ui: &mut egui::Ui) {
        self.ui_usage_status(ui);
        ui.add_space(crate::theme::SPACE_3);
        self.ui_usage_overview(ui);
        ui.add_space(crate::theme::SPACE_5);
        self.ui_usage_table(ui);
    }

    /// 顶部状态行：合计、扫描进度、数据新鲜度与错误。
    /// 顶部状态行：筛选、扫描进度、数据新鲜度与错误。
    ///
    /// 压成两行（原本三行）：用量视图下面是表格，上面多占一行就少一行数据。
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
        let filter = self.usage_filter_label();
        let weak = ui.visuals().weak_text_color();

        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(&filter)
                    .size(crate::theme::TEXT_BODY)
                    .strong(),
            );
            match &report {
                Some((sessions, messages, scan_ms, scanned_at)) => {
                    // 「更新于 N 秒前」：把 egui 时间轴上的扫描时刻换算成距今多久。
                    let age_ms = ((frame_time - scanned_at).max(0.0) * 1_000.0) as i64;
                    ui.label(
                        egui::RichText::new(format!(
                            "{} 个会话 · {} 条消息 · 更新于 {} · 内核 {scan_ms} ms",
                            sessions,
                            messages,
                            aggregate::relative_time(now_ms - age_ms, now_ms)
                        ))
                        .size(crate::theme::TEXT_CAPTION)
                        .color(weak),
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new("还没有扫描结果")
                            .size(crate::theme::TEXT_CAPTION)
                            .color(semantics.warn),
                    );
                }
            }
            if scanning {
                ui.spinner();
                ui.label(
                    egui::RichText::new("扫描中…")
                        .size(crate::theme::TEXT_CAPTION)
                        .color(weak),
                );
            }
        });
        if let Some(err) = error {
            ui.label(egui::RichText::new(format!("上次扫描失败：{err}")).color(semantics.err));
        }
        ui.label(
            egui::RichText::new(format!("账本保留已删除会话的用量（共 {ledger_len} 条）"))
                .size(crate::theme::TEXT_CAPTION)
                .color(weak),
        );
    }

    /// 四个维度的切换按钮。
    pub(in crate::app) fn dimension_buttons(&mut self, ui: &mut egui::Ui) {
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
                    .min_size(egui::vec2(DIM_BUTTON_WIDTH, 22.0)),
            );
            if response.clicked() {
                self.usage_dimension = dimension;
            }
        }
    }

    /// 当前维度的表格。
    fn ui_usage_table(&mut self, ui: &mut egui::Ui) {
        let sessions: Vec<SessionSnapshot> = self.usage_sessions().to_vec();
        let daily = self.usage_daily_filtered();
        if sessions.is_empty() && daily.is_empty() {
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
        let _offset = crate::app::balance::local_utc_offset_secs();
        let now_ms = unix_now_ms();
        let today = self.usage_today();
        let range = self.usage_range;
        match self.usage_dimension {
            Dimension::Agent => {
                let rows = aggregate::by_agent_in_range(&daily, range, today);
                usage_table_shell(ui, |ui| agent_table(ui, &rows, now_ms, range, today));
            }
            Dimension::Model => {
                let rows = aggregate::by_model_in_range(&daily, range, today);
                usage_table_shell(ui, |ui| model_table(ui, &rows, now_ms, range, today));
            }
            Dimension::Session => {
                // 会话维度也必须走区间：拿会话总量当「本月」会虚高
                // （一个会话可以横跨几十天）。
                let rows = aggregate::session_rows_in_range(&sessions, &daily, range, today);
                usage_table_shell(ui, |ui| session_table(ui, &rows, now_ms, range, today));
            }
            Dimension::Day => {
                let rows = aggregate::by_day_in_range(&daily, range, today);
                usage_table_shell(ui, |ui| day_table(ui, &rows, today, now_ms));
            }
        }
    }
}

/// 当前时间（Unix 毫秒）。
fn unix_now_ms() -> i64 {
    super::unix_now_secs().saturating_mul(1_000)
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

// ---------------------------------------------------------------------------
// 表格：列对齐的排版基元
// ---------------------------------------------------------------------------
//
// 排版照参考实现（tokscale 的 `tui-daily.png`）：表头弱化、**数字右对齐**、
// 列宽固定。不用 `egui::Grid` 是因为它不支持逐列对齐（`grid.rs` 里明确写了
// "Grid not yet available for right-to-left layouts"），数字只能左对齐，
// 位数不同的数就排成参差的 —— 正是「丑」的来源。
//
// 做法：每个单元格用 `allocate_ui_with_layout` 分配固定宽度的子区域，
// 文字列左对齐、数字列右对齐，于是每一列的字形起点/终点都对齐。

/// 表头行高（也是每个单元格的高度）。
const CELL_HEIGHT: f32 = 16.0;

/// 数值列的常用宽度：`123.4M` 这类最长 6-7 字符。
const NUM_W: f32 = 68.0;

/// 一列的定义。
#[derive(Clone, Copy)]
struct Column {
    /// 表头文字。
    header: &'static str,
    /// 列宽（点）。
    width: f32,
    /// 数字列：右对齐 + 等宽字体。
    numeric: bool,
}

impl Column {
    /// 文本列（左对齐）。
    const fn text(header: &'static str, width: f32) -> Self {
        Self {
            header,
            width,
            numeric: false,
        }
    }

    /// 数字列（右对齐 + 等宽）。
    const fn number(header: &'static str, width: f32) -> Self {
        Self {
            header,
            width,
            numeric: true,
        }
    }
}

/// 表格：表头 + 逐行。
///
/// 每行是一个闭包，负责往当前行里塞单元格（用 [`cell_text`] / [`cell_number`]）；
/// 这样四个维度共用一套列宽与对齐规则，不用各写一遍排版。
fn table<F>(ui: &mut egui::Ui, columns: &[Column], rows: impl IntoIterator<Item = F>)
where
    F: FnOnce(&mut egui::Ui),
{
    table_header(ui, columns);
    ui.add_space(crate::theme::SPACE_2);
    for row in rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = crate::theme::SPACE_2;
            row(ui);
        });
        ui.add_space(crate::theme::SPACE_1);
    }
}

/// 表头：弱化 + 与数据列同宽同对齐，于是表头正好落在数字上方。
fn table_header(ui: &mut egui::Ui, columns: &[Column]) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::theme::SPACE_2;
        for column in columns {
            ui.allocate_ui_with_layout(
                egui::vec2(column.width, CELL_HEIGHT),
                align_layout(column.numeric),
                |ui| {
                    ui.label(
                        egui::RichText::new(column.header)
                            .size(crate::theme::TEXT_CAPTION)
                            .color(ui.visuals().weak_text_color()),
                    );
                },
            );
        }
    });
}

/// 数字列右对齐、文本列左对齐。
fn align_layout(numeric: bool) -> egui::Layout {
    if numeric {
        egui::Layout::right_to_left(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    }
}

/// 文本单元格（左对齐，过长截断 + 悬停看全文）。
fn cell_text(ui: &mut egui::Ui, column: &Column, text: &str, strong: bool, weak: bool) {
    ui.allocate_ui_with_layout(
        egui::vec2(column.width, CELL_HEIGHT),
        align_layout(false),
        |ui| {
            let mut rich = egui::RichText::new(text);
            if strong {
                rich = rich.strong();
            }
            if weak {
                rich = rich.small().color(ui.visuals().weak_text_color());
            }
            ui.add(egui::Label::new(rich).truncate())
                .on_hover_text(text);
        },
    );
}

/// 数字单元格（右对齐 + 等宽，位数不同的数也能对齐）。
fn cell_number(ui: &mut egui::Ui, column: &Column, text: &str, strong: bool, weak: bool) {
    ui.allocate_ui_with_layout(
        egui::vec2(column.width, CELL_HEIGHT),
        align_layout(true),
        |ui| {
            let mut rich = egui::RichText::new(text).monospace();
            if strong {
                rich = rich.strong();
            }
            if weak {
                rich = rich.small().color(ui.visuals().weak_text_color());
            }
            ui.label(rich);
        },
    );
}

/// Agent 维度表：agent / 合计 / 输入 / 输出 / 缓存读 / 最近活动。
///
/// 不列会话数：一个会话可以横跨多天，按区间求和时「会话数」相加会重复计数。
fn agent_table(ui: &mut egui::Ui, rows: &[AgentUsage], now_ms: i64, range: Range, _today: i64) {
    let columns = [
        Column::text("Agent", 130.0),
        Column::number("合计", NUM_W),
        Column::number("输入", NUM_W),
        Column::number("输出", NUM_W),
        Column::number("缓存读", NUM_W),
        Column::text("最近活动", 90.0),
    ];
    let rows: Vec<_> = rows
        .iter()
        .map(|row| {
            let client = row.client;
            let totals = row.totals;
            let last_seen_ms = row.last_seen_ms;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns[0], client.label(), true, false);
                cell_number(
                    ui,
                    &columns[1],
                    &aggregate::format_tokens(totals.total()),
                    true,
                    false,
                );
                cell_number(
                    ui,
                    &columns[2],
                    &aggregate::format_tokens(totals.input),
                    false,
                    true,
                );
                cell_number(
                    ui,
                    &columns[3],
                    &aggregate::format_tokens(totals.output),
                    false,
                    true,
                );
                cell_number(
                    ui,
                    &columns[4],
                    &aggregate::format_tokens(totals.cache_read),
                    false,
                    true,
                );
                let when = if range == Range::All && last_seen_ms > 0 {
                    aggregate::relative_time(last_seen_ms, now_ms)
                } else {
                    String::new()
                };
                cell_text(ui, &columns[5], &when, false, true);
            }
        })
        .collect();
    table(ui, &columns, rows);
}

/// 模型维度表：模型 / 合计 / 输入 / 输出 / 缓存读 / 来源 agent。
fn model_table(ui: &mut egui::Ui, rows: &[ModelUsage], now_ms: i64, range: Range, _today: i64) {
    let columns = [
        Column::text("模型", 210.0),
        Column::number("合计", NUM_W),
        Column::number("输入", NUM_W),
        Column::number("输出", NUM_W),
        Column::number("缓存读", NUM_W),
        Column::text("来源 agent", 150.0),
    ];
    let rows: Vec<_> = rows
        .iter()
        .map(|row| {
            let model_id = row.model_id.clone();
            let totals = row.totals;
            let agents = row.agents.join(" / ");
            let last_seen_ms = row.last_seen_ms;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns[0], &model_id, true, false);
                cell_number(
                    ui,
                    &columns[1],
                    &aggregate::format_tokens(totals.total()),
                    true,
                    false,
                );
                cell_number(
                    ui,
                    &columns[2],
                    &aggregate::format_tokens(totals.input),
                    false,
                    true,
                );
                cell_number(
                    ui,
                    &columns[3],
                    &aggregate::format_tokens(totals.output),
                    false,
                    true,
                );
                cell_number(
                    ui,
                    &columns[4],
                    &aggregate::format_tokens(totals.cache_read),
                    false,
                    true,
                );
                let source = if range == Range::All && last_seen_ms > 0 {
                    format!(
                        "{agents} · {}",
                        aggregate::relative_time(last_seen_ms, now_ms)
                    )
                } else {
                    agents.clone()
                };
                cell_text(ui, &columns[5], &source, false, true);
            }
        })
        .collect();
    table(ui, &columns, rows);
}

/// 会话维度表：会话 / Agent / 模型 / 区间内用量 / 最近活动。
///
/// 区间不是「全部」时，用量列是该会话在区间内的部分，不是它的一生总量。
fn session_table(
    ui: &mut egui::Ui,
    rows: &[SessionSnapshot],
    now_ms: i64,
    range: Range,
    _today: i64,
) {
    // 表头是 `&'static str`：按区间二选一，不动态拼字符串。
    let total_column = if range == Range::All {
        Column::number("合计", NUM_W)
    } else {
        Column::number("区间内", NUM_W)
    };
    let columns = [
        Column::text("会话", 180.0),
        Column::text("Agent", 120.0),
        Column::text("模型", 210.0),
        total_column,
        Column::text("状态", 90.0),
    ];
    let rows: Vec<_> = rows
        .iter()
        .map(|row| {
            let short = aggregate::short_session_id(&row.session_id, SESSION_ID_CHARS);
            let client = row.client;
            let model_id = row.model_id.clone();
            let total = row.total();
            let last_seen_ms = row.last_seen_ms;
            let archived = row.archived;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns[0], &short, false, false);
                cell_text(ui, &columns[1], client.label(), false, false);
                cell_text(ui, &columns[2], &model_id, false, true);
                cell_number(
                    ui,
                    &columns[3],
                    &aggregate::format_tokens(total),
                    true,
                    false,
                );
                // 归档会话优先标状态（它比「最近活动」更值得注意：
                // 用户可能以为统计读错了，得说清用量来自账本）。
                if archived {
                    cell_text(ui, &columns[4], "已归档", false, false);
                } else {
                    let when = aggregate::relative_time(last_seen_ms, now_ms);
                    cell_text(ui, &columns[4], &when, false, true);
                }
            }
        })
        .collect();
    table(ui, &columns, rows);
}

/// 时间维度表：日期 / 合计 / 输入 / 输出 / 缓存读 / 消息。
///
/// 不列会话数：跨天会话会在多天各计一次，列出来会让人以为总量对不上。
fn day_table(ui: &mut egui::Ui, rows: &[DayUsage], today: i64, _now_ms: i64) {
    let columns = [
        Column::text("日期", 120.0),
        Column::number("合计", NUM_W),
        Column::number("输入", NUM_W),
        Column::number("输出", NUM_W),
        Column::number("缓存读", NUM_W),
        Column::number("消息", 56.0),
    ];
    let rows: Vec<_> = rows
        .iter()
        .map(|row| {
            let label = aggregate::day_label(row.day, today);
            let totals = row.totals;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns[0], &label, true, false);
                cell_number(
                    ui,
                    &columns[1],
                    &aggregate::format_tokens(totals.total()),
                    true,
                    false,
                );
                cell_number(
                    ui,
                    &columns[2],
                    &aggregate::format_tokens(totals.input),
                    false,
                    true,
                );
                cell_number(
                    ui,
                    &columns[3],
                    &aggregate::format_tokens(totals.output),
                    false,
                    true,
                );
                cell_number(
                    ui,
                    &columns[4],
                    &aggregate::format_tokens(totals.cache_read),
                    false,
                    true,
                );
                cell_number(ui, &columns[5], &totals.sessions.to_string(), false, true);
            }
        })
        .collect();
    table(ui, &columns, rows);
}
