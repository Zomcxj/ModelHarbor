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
        super::chip(ui, self.main_view == view, view.label(), 72.0)
    }

    /// 用量视图主体：状态行 + 控件行 + 总览（含趋势图与热力图）+ 表格。
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
                Some((sessions, messages, _scan_ms, scanned_at)) => {
                    // 「更新于 N 秒前」：把 egui 时间轴上的扫描时刻换算成距今多久。
                    let age_ms = ((frame_time - scanned_at).max(0.0) * 1_000.0) as i64;
                    ui.label(
                        egui::RichText::new(format!(
                            "{} 个会话 · {} 条消息 · 更新于 {}",
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
        // 账本那句只说「历史比磁盘上多」：是不是「已删除会话」是实现细节，
        // 用户关心的是数字为何比源目录里看到的多。
        if ledger_len > 0 {
            ui.label(
                egui::RichText::new(format!("含 {ledger_len} 条已从磁盘消失的历史会话"))
                    .size(crate::theme::TEXT_CAPTION)
                    .color(weak),
            );
        }
    }

    /// 四个维度的切换按钮。
    pub(in crate::app) fn dimension_buttons(&mut self, ui: &mut egui::Ui) {
        for dimension in Dimension::ALL {
            if super::chip(
                ui,
                self.usage_dimension == dimension,
                dimension.label(),
                DIM_BUTTON_WIDTH,
            ) {
                self.usage_dimension = dimension;
            }
        }
    }

    /// 当前维度的表格。
    fn ui_usage_table(&mut self, ui: &mut egui::Ui) {
        let sessions: Vec<SessionSnapshot> = self.usage_sessions().to_vec();
        let daily = self.usage_daily_filtered();
        if sessions.is_empty() && daily.is_empty() {
            self.ui_usage_empty(ui);
            return;
        }
        let now_ms = unix_now_ms();
        let today = self.usage_today();
        let range = self.usage_range;
        let sort = self.usage_sort;
        let show_all = self.usage_show_all;
        let action = match self.usage_dimension {
            Dimension::Agent => {
                let rows = aggregate::by_agent_in_range(&daily, range, today);
                usage_table_shell(ui, |ui| {
                    agent_table(ui, &rows, now_ms, range, sort, show_all)
                })
            }
            Dimension::Model => {
                let rows = aggregate::by_model_in_range(&daily, range, today);
                usage_table_shell(ui, |ui| {
                    model_table(ui, &rows, now_ms, range, sort, show_all)
                })
            }
            Dimension::Session => {
                // 会话维度也必须走区间：拿会话总量当「本月」会虚高
                // （一个会话可以横跨几十天）。
                let rows = aggregate::session_rows_in_range(&sessions, &daily, range, today);
                usage_table_shell(ui, |ui| {
                    session_table(ui, &rows, now_ms, range, sort, show_all)
                })
            }
            Dimension::Day => {
                let rows = aggregate::by_day_in_range(&daily, range, today);
                usage_table_shell(ui, |ui| day_table(ui, &rows, today, sort, show_all))
            }
        };
        match action {
            Some(TableAction::Sort(key)) => self.usage_sort = self.usage_sort.toggled(key),
            Some(TableAction::ToggleShowAll) => self.usage_show_all = !self.usage_show_all,
            None => {}
        }
    }

    /// 空状态：一行说明 + 一行「怎么让数字出现」。
    fn ui_usage_empty(&self, ui: &mut egui::Ui) {
        let semantics = crate::theme::semantics(ui);
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new("没有找到本机用量数据。")
                        .size(crate::theme::TEXT_BODY)
                        .color(semantics.warn),
                );
                ui.add_space(crate::theme::SPACE_2);
                ui.label(
                    egui::RichText::new(
                        "统计的是各 agent 自己写在磁盘上的会话账本（pi / opencode / \
                         workbuddy / dsh 等）。先用其中任意一个跑一轮对话，数字就会出现在这里。",
                    )
                    .small()
                    .weak(),
                );
            });
    }
}

/// 当前时间（Unix 毫秒）。
fn unix_now_ms() -> i64 {
    super::unix_now_secs().saturating_mul(1_000)
}

/// 表格外壳：统一内边距，并撑满可用宽度；把闭包的返回值（表格交互）透出去。
fn usage_table_shell<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
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
// 关键在**每列都要精确占掉声明的宽度**。`allocate_ui_with_layout` 做不到：
// 它只把实际用掉的宽度（`min_rect`）还给父 `Ui`（见 egui `ui.rs` 的
// `allocate_ui_with_layout_dyn`："the amount of space actually used (min_rect)
// will be allocated"），于是窄内容后面的列整体左移，列与列对不上。
// 这里改用 `allocate_exact_size`：它按请求的尺寸预留空间，列宽是死的。

/// 行高（也是每个单元格的高度）。
const CELL_HEIGHT: f32 = 16.0;

/// 数值列的常用宽度：`123.4M` 这类最长 6-7 字符。
const NUM_W: f32 = 68.0;

/// 单元格左右内边距：列宽固定，内容两侧各留这么多，
/// 否则数字会顶到下一列的表头上（列宽是死的，贴边看起来像连在一起）。
const CELL_PAD_X: f32 = 6.0;

/// 一列的定义。
#[derive(Clone, Copy)]
struct Column {
    /// 表头文字。
    header: &'static str,
    /// 列宽（点）。
    width: f32,
    /// 数字列：右对齐 + 等宽字体。
    numeric: bool,
    /// 排序键：`None` = 该列不可排序（如会话 id、模型名这类文本列）。
    sort: Option<SortKey>,
}

impl Column {
    /// 文本列（左对齐，不可排序）。
    const fn text(header: &'static str, width: f32) -> Self {
        Self {
            header,
            width,
            numeric: false,
            sort: None,
        }
    }

    /// 数字列（右对齐 + 等宽），可点表头排序。
    const fn number(header: &'static str, width: f32, sort: SortKey) -> Self {
        Self {
            header,
            width,
            numeric: true,
            sort: Some(sort),
        }
    }

    /// 文本列 + 可排序（用于日期、Agent 名这类）。
    const fn sorted_text(header: &'static str, width: f32, sort: SortKey) -> Self {
        Self {
            header,
            width,
            numeric: false,
            sort: Some(sort),
        }
    }
}

/// 可排序的键。
///
/// 只列真正有意义的几个：每个维度都能按「用量」与「名称」排，加时间维度
/// 自己就是时间序。再多就是给用户出选择题。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum SortKey {
    /// 名称 / 模型 id / 日期。
    Name,
    /// 合计 token。
    Total,
    /// 输入 token。
    Input,
    /// 输出 token。
    Output,
    /// 缓存读 token。
    CacheRead,
}

/// 排序状态：当前键 + 是否降序。
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct SortState {
    pub key: SortKey,
    /// `true` = 大在前。数字列默认降序（看用量），名称列默认升序。
    pub descending: bool,
}

impl SortState {
    /// 默认：按合计降序（用户最关心「谁用得最多」）。
    pub(in crate::app) const fn by_total() -> Self {
        Self {
            key: SortKey::Total,
            descending: true,
        }
    }

    /// 点表头：同一个键则翻转方向，换键则取该键的默认方向。
    pub(in crate::app) fn toggled(self, key: SortKey) -> Self {
        if self.key == key {
            Self {
                key,
                descending: !self.descending,
            }
        } else {
            Self {
                key,
                // 名称升序（A→Z 更自然），其余降序（大在前）。
                descending: key != SortKey::Name,
            }
        }
    }

    /// 排序指示箭头。
    fn arrow(self, key: SortKey) -> &'static str {
        if self.key != key {
            return "";
        }
        if self.descending {
            " ▼"
        } else {
            " ▲"
        }
    }
}

/// 排序：按当前键与方向排，方向相同时用名称正序兜底，保证界面不跳动。
fn sort_rows<T>(
    rows: &mut [T],
    state: SortState,
    name: impl Fn(&T) -> String,
    value: impl Fn(&T, SortKey) -> i64,
) {
    let name_of = |row: &T| name(row);
    rows.sort_by(|a, b| {
        let ord = if state.key == SortKey::Name {
            name_of(a).cmp(&name_of(b))
        } else {
            value(a, state.key).cmp(&value(b, state.key))
        };
        let ord = if state.descending { ord.reverse() } else { ord };
        ord.then_with(|| name_of(a).cmp(&name_of(b)))
    });
}

/// 表格：表头 + 逐行，列按固定宽度排。
///
/// 每行是一个闭包，负责往当前行里塞单元格（用 [`cell_text`] / [`cell_number`]）；
/// 这样四个维度共用一套列宽与对齐规则，不用各写一遍排版。
///
/// 行背景：隔行加一层极淡的底色（斑马纹），鼠标所在行再叠一层悬浮色 ——
/// 列宽固定、行高只有 16 点，不给背景提示很容易看串行。
///
/// 返回被点的表头列（没有则 `None`）。
fn table<F>(
    ui: &mut egui::Ui,
    columns: &[Column],
    sort: SortState,
    rows: impl IntoIterator<Item = F>,
) -> Option<SortKey>
where
    F: FnOnce(&mut egui::Ui),
{
    let clicked = table_header(ui, columns, sort);
    ui.add_space(crate::theme::SPACE_2);
    let faint = ui.visuals().faint_bg_color;
    let hover = ui.visuals().widgets.hovered.weak_bg_fill;
    let row_width: f32 = columns.iter().map(|column| column.width).sum();
    for (index, row) in rows.into_iter().enumerate() {
        // 先占位拿到本行的矩形，再把背景画在内容下面。
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(row_width, CELL_HEIGHT), egui::Sense::hover());
        if index % 2 == 1 {
            ui.painter().rect_filled(rect, 0.0, faint);
        }
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, hover);
        }
        // 内容画在占位矩形内：新建一个子 `Ui`，游标从行首开始。
        let mut row_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        row_ui.spacing_mut().item_spacing.x = 0.0;
        row(&mut row_ui);
        ui.add_space(crate::theme::SPACE_1);
    }
    clicked
}

/// 表头：弱化 + 与数据列同宽同对齐，于是表头正好落在数字上方。
///
/// 可排序的列画成可点的按钮（悬停变手型），当前排序列带 ▲/▼ 指示。
fn table_header(ui: &mut egui::Ui, columns: &[Column], sort: SortState) -> Option<SortKey> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for column in columns {
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(column.width, CELL_HEIGHT),
                if column.sort.is_some() {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            let header = match column.sort {
                Some(key) => format!("{}{}", column.header, sort.arrow(key)),
                None => column.header.to_string(),
            };
            let color = if column.sort.is_some_and(|key| key == sort.key) {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            let galley = ui.painter().layout_no_wrap(
                header,
                egui::FontId::proportional(crate::theme::TEXT_CAPTION),
                color,
            );
            paint_in_cell(ui, rect, galley, column.numeric);
            if let Some(key) = column.sort {
                if response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    clicked = Some(key);
                }
            }
        }
    });
    clicked
}

/// 把 galley 按列的对齐方式放进单元格：数字贴右内边距、文本贴左内边距，纵向居中。
fn paint_in_cell(
    ui: &egui::Ui,
    rect: egui::Rect,
    galley: std::sync::Arc<egui::Galley>,
    right: bool,
) {
    let x = if right {
        rect.right() - CELL_PAD_X - galley.size().x
    } else {
        rect.left() + CELL_PAD_X
    };
    let y = rect.center().y - galley.size().y / 2.0;
    ui.painter()
        .galley(egui::pos2(x, y), galley, egui::Color32::PLACEHOLDER);
}

/// 文本单元格（左对齐，过长截断 + 悬停看全文）。
fn cell_text(ui: &mut egui::Ui, columns: &[Column], index: usize, text: &str, strong: bool) {
    let column = columns[index];
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(column.width, CELL_HEIGHT), egui::Sense::hover());
    let color = if strong {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    // 超宽就省略号截断（`max_rows = 1`），全文走悬停。
    let mut job = egui::text::LayoutJob::simple(
        text.to_string(),
        egui::FontId::proportional(crate::theme::TEXT_SMALL),
        color,
        column.width - CELL_PAD_X * 2.0,
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let elided = galley.elided;
    paint_in_cell(ui, rect, galley, false);
    if elided || strong {
        response.on_hover_text(text);
    }
}

/// 数字单元格（右对齐 + 等宽，位数不同的数也能对齐）。
fn cell_number(ui: &mut egui::Ui, columns: &[Column], index: usize, text: &str, strong: bool) {
    let column = columns[index];
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(column.width, CELL_HEIGHT), egui::Sense::hover());
    // 强调态换个颜色区分「合计」列，**不加粗**：等宽体加粗会改变字宽，
    // 同一列里粗体与常规体的数字右边缘就不齐了。
    let color = if strong {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::monospace(crate::theme::TEXT_SMALL),
        color,
    );
    paint_in_cell(ui, rect, galley, true);
}

/// 一页最多先画多少行，其余折叠。
///
/// 模型动辄几百个，全画出来要滚很久，而前几名就占了九成以上的量。
const PAGE_ROWS: usize = 20;

/// 行是否折叠（超过 [`PAGE_ROWS`] 时只看前几行）。
///
/// 返回「要画的行数」与「总共多少行」，调用方据此决定要不要画「显示全部」开关。
fn visible_rows(total: usize, show_all: bool) -> usize {
    if show_all {
        total
    } else {
        total.min(PAGE_ROWS)
    }
}

/// 「显示全部 / 只看前 N」开关：只有真的被截断时才画。
fn show_all_toggle(ui: &mut egui::Ui, total: usize, show_all: bool) -> bool {
    if total <= PAGE_ROWS {
        return false;
    }
    let label = if show_all {
        format!("只看前 {PAGE_ROWS}")
    } else {
        format!("显示全部 {total} 行")
    };
    ui.add_space(crate::theme::SPACE_2);
    ui.add(egui::Button::new(
        egui::RichText::new(label).size(crate::theme::TEXT_CAPTION),
    ))
    .clicked()
}

/// 表格交互的结果。
enum TableAction {
    /// 点了某个表头：切换排序。
    Sort(SortKey),
    /// 点了「显示全部 / 只看前 N」。
    ToggleShowAll,
}

/// 把两个交互信号合成一个动作（显示全部优先：它不在表头里，不可能同时发生）。
fn table_action(clicked: Option<SortKey>, toggled: bool) -> Option<TableAction> {
    match (clicked, toggled) {
        (_, true) => Some(TableAction::ToggleShowAll),
        (Some(key), false) => Some(TableAction::Sort(key)),
        (None, false) => None,
    }
}

/// Agent 维度表：Agent / 合计 / 输入 / 输出 / 缓存读 / 最近活动。
///
/// 不列会话数：一个会话可以横跨多天，按区间求和时「会话数」相加会重复计数。
fn agent_table(
    ui: &mut egui::Ui,
    rows: &[AgentUsage],
    now_ms: i64,
    range: Range,
    sort: SortState,
    show_all: bool,
) -> Option<TableAction> {
    let columns = [
        Column::sorted_text("Agent", 130.0, SortKey::Name),
        Column::number("合计", NUM_W, SortKey::Total),
        Column::number("输入", NUM_W, SortKey::Input),
        Column::number("输出", NUM_W, SortKey::Output),
        Column::number("缓存读", NUM_W, SortKey::CacheRead),
        Column::text("最近活动", 90.0),
    ];
    let mut rows: Vec<AgentUsage> = rows.to_vec();
    sort_rows(
        &mut rows,
        sort,
        |row| row.client.label().to_string(),
        |row, key| match key {
            SortKey::Input => row.totals.input,
            SortKey::Output => row.totals.output,
            SortKey::CacheRead => row.totals.cache_read,
            _ => row.totals.total(),
        },
    );
    let visible = visible_rows(rows.len(), show_all);
    let items: Vec<_> = rows[..visible]
        .iter()
        .map(|row| {
            let client = row.client;
            let totals = row.totals;
            let last_seen_ms = row.last_seen_ms;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns, 0, client.label(), true);
                cell_number(
                    ui,
                    &columns,
                    1,
                    &aggregate::format_tokens(totals.total()),
                    true,
                );
                cell_number(
                    ui,
                    &columns,
                    2,
                    &aggregate::format_tokens(totals.input),
                    false,
                );
                cell_number(
                    ui,
                    &columns,
                    3,
                    &aggregate::format_tokens(totals.output),
                    false,
                );
                cell_number(
                    ui,
                    &columns,
                    4,
                    &aggregate::format_tokens(totals.cache_read),
                    false,
                );
                let when = if range == Range::All && last_seen_ms > 0 {
                    aggregate::relative_time(last_seen_ms, now_ms)
                } else {
                    String::new()
                };
                cell_text(ui, &columns, 5, &when, false);
            }
        })
        .collect();
    let clicked = table(ui, &columns, sort, items);
    table_action(clicked, show_all_toggle(ui, rows.len(), show_all))
}

/// 模型维度表：模型 / 合计 / 输入 / 输出 / 缓存读 / 来源 agent。
fn model_table(
    ui: &mut egui::Ui,
    rows: &[ModelUsage],
    now_ms: i64,
    range: Range,
    sort: SortState,
    show_all: bool,
) -> Option<TableAction> {
    let columns = [
        Column::sorted_text("模型", 210.0, SortKey::Name),
        Column::number("合计", NUM_W, SortKey::Total),
        Column::number("输入", NUM_W, SortKey::Input),
        Column::number("输出", NUM_W, SortKey::Output),
        Column::number("缓存读", NUM_W, SortKey::CacheRead),
        Column::text("来源 agent", 150.0),
    ];
    let mut rows: Vec<ModelUsage> = rows.to_vec();
    sort_rows(
        &mut rows,
        sort,
        |row| row.model_id.clone(),
        |row, key| match key {
            SortKey::Input => row.totals.input,
            SortKey::Output => row.totals.output,
            SortKey::CacheRead => row.totals.cache_read,
            _ => row.totals.total(),
        },
    );
    let visible = visible_rows(rows.len(), show_all);
    let items: Vec<_> = rows[..visible]
        .iter()
        .map(|row| {
            let model_id = row.model_id.clone();
            let totals = row.totals;
            let agents = row.agents.join(" / ");
            let last_seen_ms = row.last_seen_ms;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns, 0, &model_id, true);
                cell_number(
                    ui,
                    &columns,
                    1,
                    &aggregate::format_tokens(totals.total()),
                    true,
                );
                cell_number(
                    ui,
                    &columns,
                    2,
                    &aggregate::format_tokens(totals.input),
                    false,
                );
                cell_number(
                    ui,
                    &columns,
                    3,
                    &aggregate::format_tokens(totals.output),
                    false,
                );
                cell_number(
                    ui,
                    &columns,
                    4,
                    &aggregate::format_tokens(totals.cache_read),
                    false,
                );
                let source = if range == Range::All && last_seen_ms > 0 {
                    format!(
                        "{agents} · {}",
                        aggregate::relative_time(last_seen_ms, now_ms)
                    )
                } else {
                    agents.clone()
                };
                cell_text(ui, &columns, 5, &source, false);
            }
        })
        .collect();
    let clicked = table(ui, &columns, sort, items);
    table_action(clicked, show_all_toggle(ui, rows.len(), show_all))
}

/// 会话维度表：会话 / Agent / 模型 / 区间内用量 / 状态。
///
/// 区间不是「全部」时，用量列是该会话在区间内的部分，不是它的一生总量。
fn session_table(
    ui: &mut egui::Ui,
    rows: &[SessionSnapshot],
    now_ms: i64,
    range: Range,
    sort: SortState,
    show_all: bool,
) -> Option<TableAction> {
    // 表头是 `&'static str`：按区间二选一，不动态拼字符串。
    let total_column = if range == Range::All {
        Column::number("合计", NUM_W, SortKey::Total)
    } else {
        Column::number("区间内", NUM_W, SortKey::Total)
    };
    let columns = [
        Column::text("会话", 180.0),
        Column::sorted_text("Agent", 120.0, SortKey::Name),
        Column::text("模型", 210.0),
        total_column,
        Column::text("状态", 90.0),
    ];
    let mut rows: Vec<SessionSnapshot> = rows.to_vec();
    sort_rows(
        &mut rows,
        sort,
        // 会话没有天然的名字：用「Agent + 会话 id」当排序兜底键，
        // 保证同一个 agent 的会话能聚在一起。
        |row| format!("{}:{}", row.client.label(), row.session_id),
        |row, key| match key {
            SortKey::Input => row.input,
            SortKey::Output => row.output,
            SortKey::CacheRead => row.cache_read,
            _ => row.total(),
        },
    );
    let visible = visible_rows(rows.len(), show_all);
    let items: Vec<_> = rows[..visible]
        .iter()
        .map(|row| {
            let short = aggregate::short_session_id(&row.session_id, SESSION_ID_CHARS);
            let client = row.client;
            let model_id = row.model_id.clone();
            let total = row.total();
            let last_seen_ms = row.last_seen_ms;
            let archived = row.archived;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns, 0, &short, false);
                cell_text(ui, &columns, 1, client.label(), false);
                cell_text(ui, &columns, 2, &model_id, false);
                cell_number(ui, &columns, 3, &aggregate::format_tokens(total), true);
                // 归档会话优先标状态（它比「最近活动」更值得注意：
                // 用户可能以为统计读错了，得说清用量来自账本）。
                if archived {
                    cell_text(ui, &columns, 4, "已归档", false);
                } else {
                    let when = aggregate::relative_time(last_seen_ms, now_ms);
                    cell_text(ui, &columns, 4, &when, false);
                }
            }
        })
        .collect();
    let clicked = table(ui, &columns, sort, items);
    table_action(clicked, show_all_toggle(ui, rows.len(), show_all))
}

/// 时间维度表：日期 / 合计 / 输入 / 输出 / 缓存读 / 消息。
///
/// 不列会话数：跨天会话会在多天各计一次，列出来会让人以为总量对不上。
fn day_table(
    ui: &mut egui::Ui,
    rows: &[DayUsage],
    today: i64,
    sort: SortState,
    show_all: bool,
) -> Option<TableAction> {
    let columns = [
        Column::sorted_text("日期", 120.0, SortKey::Name),
        Column::number("合计", NUM_W, SortKey::Total),
        Column::number("输入", NUM_W, SortKey::Input),
        Column::number("输出", NUM_W, SortKey::Output),
        Column::number("缓存读", NUM_W, SortKey::CacheRead),
        Column::number("消息", 56.0, SortKey::Name),
    ];
    let mut rows: Vec<DayUsage> = rows.to_vec();
    sort_rows(
        &mut rows,
        sort,
        // 日期用**定宽**写法当排序键：`今天 / 昨天` 这种文案直接比字符串
        // 会把「今天」排到「2026-03-15」前面，而按 `YYYY-MM-DD` 比就对了。
        |row| {
            let (year, month, day) = aggregate::civil_from_days(row.day);
            format!("{year:04}-{month:02}-{day:02}")
        },
        |row, key| match key {
            SortKey::Input => row.totals.input,
            SortKey::Output => row.totals.output,
            SortKey::CacheRead => row.totals.cache_read,
            _ => row.totals.total(),
        },
    );
    let visible = visible_rows(rows.len(), show_all);
    let items: Vec<_> = rows[..visible]
        .iter()
        .map(|row| {
            let label = aggregate::day_label(row.day, today);
            let totals = row.totals;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns, 0, &label, true);
                cell_number(
                    ui,
                    &columns,
                    1,
                    &aggregate::format_tokens(totals.total()),
                    true,
                );
                cell_number(
                    ui,
                    &columns,
                    2,
                    &aggregate::format_tokens(totals.input),
                    false,
                );
                cell_number(
                    ui,
                    &columns,
                    3,
                    &aggregate::format_tokens(totals.output),
                    false,
                );
                cell_number(
                    ui,
                    &columns,
                    4,
                    &aggregate::format_tokens(totals.cache_read),
                    false,
                );
                cell_number(ui, &columns, 5, &totals.sessions.to_string(), false);
            }
        })
        .collect();
    let clicked = table(ui, &columns, sort, items);
    table_action(clicked, show_all_toggle(ui, rows.len(), show_all))
}
