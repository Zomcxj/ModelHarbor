//! 用量视图的界面：顶部视图切换 + 四维统计表。
//!
//! 数据全部来自 [`super::super::usage`] 的后台扫描结果（`App::usage`），这里只负责画；
//! 聚合与格式化在 [`super::aggregate`] 里，是可单测的纯逻辑。
//!
//! 切维度按钮照顶栏页签的选中样式（`widgets.hovered.bg_fill` + 2px 描边），
//! 配色一律走 `crate::theme::semantics(ui)`，不硬编码颜色。

use super::aggregate::{self, AgentUsage, Dimension, ModelUsage};
use super::range::Range;
use crate::app::App;
use crate::usage::SessionSnapshot;
use eframe::egui;

/// 左右并排的两张表：Agent / 模型。
///
/// 会话表不并排：它列最多（会话名 / id / Agent / 模型 / 项目 / 用量 /
/// 状态 / 操作），挤半宽会把每列压成省略号，因此独占一行（用户要求）。
const SIDE_BY_SIDE: [Dimension; 2] = [Dimension::Agent, Dimension::Model];

/// 会话 id 在表里最多显示多少个字符。
const SESSION_ID_CHARS: usize = 20;

impl App {
    /// 视图切换：`[提供商管理] [用量]` 二选一。
    ///
    /// 两个按钮交换过位置（用户要求）：提供商管理在前。画在 agent 页签右侧。
    pub(in crate::app) fn ui_view_switch(&mut self, ui: &mut egui::Ui) {
        let views = [
            super::super::MainView::Providers,
            super::super::MainView::Usage,
        ];
        for view in views {
            if self.view_switch_button(ui, view) {
                self.main_view = view;
            }
        }
    }

    /// 单个视图切换按钮：选中态 = 悬浮色底 + 2px 描边（与顶栏页签同一套）。
    fn view_switch_button(&self, ui: &mut egui::Ui, view: super::super::MainView) -> bool {
        super::chip(ui, self.main_view == view, view.label(), 72.0)
    }

    /// 用量视图主体：单列纵向流。
    ///
    /// 布局（用户要求）：总览（含趋势图与热力图）占满宽 → Agent 与模型
    /// 左右并排 → 会话表独占一行。不再分左右两栏，每张表能拿到更多列宽。
    ///
    /// 状态行（含区间切换）不在这里：它固定在滚动区之上，不跟随页面滚动
    /// （用户要求），由 [`App::update`] 先画。
    pub(in crate::app) fn ui_usage_section(&mut self, ui: &mut egui::Ui) {
        let sessions: Vec<SessionSnapshot> = self.usage_sessions().to_vec();
        let daily = self.usage_daily_filtered();
        if sessions.is_empty() && daily.is_empty() {
            self.ui_usage_empty(ui);
            return;
        }
        // 总览（总览卡 + 趋势图 + 热力图）占满宽。
        self.ui_usage_overview(ui);
        ui.add_space(crate::theme::SPACE_5);

        let now_ms = unix_now_ms();
        let today = self.usage_today();
        let range = self.usage_range;
        let sort = self.usage_sort;
        let pending_delete = self.usage_delete_pending.clone();

        // 表格在独立闭包里画，`self` 借不进来：交互（排序 / 删除）用 `RefCell`
        // 收集，画完统一处理。不同交互不可能同帧同时发生，逐个处理即可。
        let collected: std::cell::RefCell<Vec<TableAction>> = std::cell::RefCell::new(Vec::new());
        let record = |result: Option<TableAction>| {
            if let Some(action) = result {
                collected.borrow_mut().push(action);
            }
        };

        // 并排两张表的行集先算出来：表体高度按较多一方的行数锁定，
        // 两卡底边才齐平（用户要求：Agent 卡高度与模型卡对齐）。
        let side: Vec<TableRows> = SIDE_BY_SIDE
            .iter()
            .copied()
            .map(|dimension| TableRows::of(dimension, &sessions, &daily, range, today))
            .collect();
        let side_rows = side.iter().map(TableRows::len).max().unwrap_or(0);
        let side_body = super::component::body_height(side_rows, super::component::BODY_MAX_HEIGHT);

        // Agent 与模型左右并排，各占一半：间距不计入比例。
        let gap = crate::theme::SPACE_4;
        let half = ((ui.available_width() - gap) / 2.0).max(1.0);
        ui.horizontal_top(|ui| {
            for (index, rows) in side.iter().enumerate() {
                if index > 0 {
                    ui.add_space(gap);
                }
                ui.allocate_ui_with_layout(
                    egui::vec2(half, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(half);
                        // 解除列高限制：allocate_ui_with_layout 的 0 高期望会把内部
                        // 所有 ScrollArea 的视口压到「剩余可用高度」，表体装不满
                        // 卡内空间（用户反馈：列表要铺满组件）。
                        ui.set_max_height(f32::INFINITY);
                        record(self.ui_dimension_table(
                            ui,
                            rows,
                            now_ms,
                            range,
                            sort,
                            side_body,
                            pending_delete.as_deref(),
                        ));
                    },
                );
            }
        });
        ui.add_space(crate::theme::SPACE_5);
        // 会话表独占一行（全宽）：它的列最多，挤半宽每列都只剩省略号。
        let session_rows = TableRows::of(Dimension::Session, &sessions, &daily, range, today);
        record(self.ui_dimension_table(
            ui,
            &session_rows,
            now_ms,
            range,
            sort,
            // 表体高度是其余两表的三倍（用户要求）。同样走锁定：只给上限
            // 时，视口会被页面剩余高度夹住，窗口矮一点就看不到三倍的行。
            super::component::body_height(
                session_rows.len(),
                super::component::SESSION_BODY_HEIGHT,
            ),
            pending_delete.as_deref(),
        ));

        let frame_time = ui.input(|input| input.time);
        self.handle_table_actions(collected.into_inner(), frame_time);
    }

    /// 处理本轮表格交互：排序立即生效；删除走两段式确认
    /// （第一击标待确认，再击同一行才真删）。
    fn handle_table_actions(&mut self, actions: Vec<TableAction>, frame_time: f64) {
        for action in actions {
            match action {
                TableAction::Sort(key) => self.usage_sort = self.usage_sort.toggled(key),
                TableAction::DeleteSession(key) => {
                    if self.usage_delete_pending.as_deref() == Some(key.as_str()) {
                        self.forget_usage_session(&key, frame_time);
                    } else {
                        self.usage_delete_pending = Some(key);
                    }
                }
            }
        }
    }

    /// 顶部状态行：筛选 / 扫描进度 / 数据新鲜度 / 错误 + 最右侧的区间切换。
    ///
    /// 这一行由 [`App::update`] 画在滚动区之上，因此固定不动
    /// （用户要求）：滚到长表里也能看到当前区间与筛选条件。
    ///
    /// 排版：左侧文字占满「整行减去区间组件宽度」，区间切换组件贴右边缘；
    /// 两格顶对齐，整行高度取两者较高的那个。
    pub(in crate::app) fn ui_usage_status(&mut self, ui: &mut egui::Ui) {
        let semantics = crate::theme::semantics(ui);
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
        let ledger_archived = self.usage.ledger_archived_len();
        let scanning = self.usage.scanning();
        let cached_saved_at = self.usage.cached_saved_at();
        let filter = self.usage_filter_label();
        let weak = ui.visuals().weak_text_color();

        // 排版：左侧文字铺满剩余宽度，区间切换组件贴右（用户要求）。
        //
        // 两格各自拿到一个死矩形的子 `Ui`：`allocate_ui_with_layout` 只把
        // 实际用掉的宽度（`min_rect`）还给父级，于是筛选文字一短，整组
        // 区间按钮就跟着往左跑。先把两格的位置定死，再把整行高度还给
        // 父级（两格里较高的那个）。
        let chips_width = Self::usage_range_chips_width();
        let row = ui.available_rect_before_wrap();
        let left_width = (row.width() - crate::theme::SPACE_3 - chips_width).max(1.0);

        let mut left = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    row.min,
                    egui::vec2(left_width, 0.0),
                ))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        // 文字可能折行，也可能多一行错误 / 账本提示：不能拿子 `Ui` 的
        // 剩余高度当上限，否则第二行就被压成 0 高。
        left.set_max_height(f32::INFINITY);
        self.usage_status_text(
            &mut left,
            &filter,
            cached_saved_at,
            report.as_ref(),
            error.as_deref(),
            now_ms,
            frame_time,
            scanning,
            ledger_archived,
            semantics,
            weak,
        );
        let left_height = left.min_rect().height();

        let mut right = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_size(
                    egui::pos2(row.max.x - chips_width, row.min.y),
                    egui::vec2(chips_width, 0.0),
                ))
                // 横向对齐留 `Align::Min`：竖向布局下的 `Max` 会让
                // `prefer_right_to_left()` 成立，`chip_group` 里的
                // `ui.horizontal` 跟着反向，四个按钮就排成「全部 … 今日」。
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        right.set_max_height(f32::INFINITY);
        self.ui_usage_range_chips(&mut right);
        let right_height = right.min_rect().height();

        // 把整行高度还给父级：`new_child` 只画不占位，父级游标还停在行首，
        // 不推进的话后面的 `ScrollArea` 会从状态行顶部起画，把整行盖住
        // （首栏被覆盖的缺陷就是这么来的）。`expand_to_include_rect` 只并
        // `min_rect`/`max_rect`、不动游标，所以这里必须用推进游标的版本。
        ui.advance_cursor_after_rect(egui::Rect::from_min_max(
            row.min,
            egui::pos2(row.max.x, row.min.y + left_height.max(right_height)),
        ));
    }

    /// 状态行左侧的文字：筛选 / 缓存或扫描新鲜度 / 错误 / 账本归档提示。
    #[allow(clippy::too_many_arguments)]
    fn usage_status_text(
        &mut self,
        ui: &mut egui::Ui,
        filter: &str,
        cached_saved_at: Option<i64>,
        report: Option<&(usize, usize, u32, f64)>,
        error: Option<&str>,
        now_ms: i64,
        frame_time: f64,
        scanning: bool,
        ledger_archived: usize,
        semantics: crate::theme::Semantics,
        weak: egui::Color32,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(filter)
                    .size(crate::theme::TEXT_BODY)
                    .strong(),
            );
            if let Some(saved_ms) = cached_saved_at {
                // 缓存秒显：账本数据先顶上（后台扫完自动替换）。标清数据新旧，
                // 避免把几分钟前的数字当成实时值；超过 5 分钟用警示色。
                let sessions = report.map_or(0, |(sessions, _, _, _)| *sessions);
                let stale = now_ms - saved_ms > aggregate::CACHED_STALE_MS;
                ui.label(
                    egui::RichText::new(aggregate::cached_status_line(
                        sessions,
                        Some(saved_ms),
                        now_ms,
                        scanning,
                    ))
                    .size(crate::theme::TEXT_CAPTION)
                    .color(if stale { semantics.warn } else { weak }),
                );
                if scanning {
                    ui.spinner();
                }
            } else if let Some((sessions, messages, _scan_ms, scanned_at)) = report {
                // 「更新于 N 秒前」：把 egui 时间轴上的扫描时刻换算成距今多久。
                let age_ms = ((frame_time - scanned_at).max(0.0) * 1_000.0) as i64;
                ui.label(
                    egui::RichText::new(format!(
                        "{sessions} 个会话 · {messages} 条消息 · 更新于 {}",
                        aggregate::relative_time(now_ms - age_ms, now_ms)
                    ))
                    .size(crate::theme::TEXT_CAPTION)
                    .color(weak),
                );
            } else {
                ui.label(
                    egui::RichText::new("还没有扫描结果")
                        .size(crate::theme::TEXT_CAPTION)
                        .color(semantics.warn),
                );
                if scanning {
                    // 首次打开、扫描还在路上：给个「扫描中」指示（用户反馈：
                    // 刚打开需要）。有了数据就不再画，免得在数字旁边常驻
                    //（此前按用户反馈删过一次）。
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("扫描中…")
                            .size(crate::theme::TEXT_CAPTION)
                            .color(weak),
                    );
                }
            }
        });
        if let Some(err) = error {
            ui.label(egui::RichText::new(format!("上次扫描失败：{err}")).color(semantics.err));
        }
        // 源侧删除失败（进程在跑 / 写库失败）：说清统计那半是否也已处理。
        // 拦下整次删除的情形不能写成「已从统计里移除」，否则用户会以为删成了。
        if let Some(err) = self.usage_source_error.clone() {
            ui.label(egui::RichText::new(err).color(semantics.err));
        }
        // 账本那句只说「历史比磁盘上多」：是不是「已从源里消失」是实现细节，
        // 用户关心的是数字为何比源目录里看到的多。
        if ledger_archived > 0 {
            ui.label(
                egui::RichText::new(format!(
                    "含 {ledger_archived} 条已从 agent 库里消失的历史会话（只在合计里，不占上表）"
                ))
                .size(crate::theme::TEXT_CAPTION)
                .color(weak),
            );
        }
    }

    /// 画一张维度的表（带小节标题），返回它的交互结果。
    ///
    /// 标题用 `Dimension::label`：四个表同屏后，不标就分不清哪张是哪张。
    /// `body` 由调用方给：并排的两张表共用一个预算，两卡才等高。
    /// `pending_delete` 只对会话表有意义：标着哪一行正等二次确认。
    #[allow(clippy::too_many_arguments)]
    fn ui_dimension_table(
        &self,
        ui: &mut egui::Ui,
        rows: &TableRows,
        now_ms: i64,
        range: Range,
        sort: SortState,
        body: f32,
        pending_delete: Option<&str>,
    ) -> Option<TableAction> {
        // 标题画在卡片**里面**：与趋势图 / 热力图的组件外壳一致。
        super::component::card(ui, |ui| {
            super::component::heading(ui, rows.dimension().label());
            ui.add_space(crate::theme::SPACE_2);
            match rows {
                TableRows::Agent(rows) => agent_table(ui, rows, sort, body),
                TableRows::Model(rows) => model_table(ui, rows, sort, body),
                TableRows::Session(rows) => {
                    session_table(ui, rows, now_ms, range, sort, body, pending_delete)
                }
            }
        })
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

/// 一张表的行集。
///
/// 三个维度的行类型不同，用枚举收拢：并排的两张表要先各自算出行数，
/// 按较多的一方锁定表体高度，两卡底边才齐平（用户要求：Agent 卡高度
/// 与模型卡对齐）。行集在画之前算好，高度就不会随绘制顺序漂移。
enum TableRows {
    Agent(Vec<AgentUsage>),
    Model(Vec<ModelUsage>),
    Session(Vec<SessionSnapshot>),
}

impl TableRows {
    /// 按维度与区间聚合出行集。
    ///
    /// 会话维度也必须走区间：拿会话总量当「本月」会虚高（一个会话可以
    /// 横跨几十天）。
    fn of(
        dimension: Dimension,
        sessions: &[SessionSnapshot],
        daily: &crate::usage::DailyMap,
        range: Range,
        today: i64,
    ) -> Self {
        match dimension {
            Dimension::Agent => Self::Agent(aggregate::by_agent_in_range(daily, range, today)),
            Dimension::Model => Self::Model(aggregate::by_model_in_range(daily, range, today)),
            Dimension::Session => Self::Session(aggregate::session_rows_in_range(
                sessions, daily, range, today,
            )),
        }
    }

    /// 这组行属于哪个维度：卡片标题与交互都按它走。
    fn dimension(&self) -> Dimension {
        match self {
            Self::Agent(_) => Dimension::Agent,
            Self::Model(_) => Dimension::Model,
            Self::Session(_) => Dimension::Session,
        }
    }

    /// 行数：并排两表取较大值当作共同的高度预算。
    fn len(&self) -> usize {
        match self {
            Self::Agent(rows) => rows.len(),
            Self::Model(rows) => rows.len(),
            Self::Session(rows) => rows.len(),
        }
    }
}

/// 文本列伸缩的下限比例：再窄就只剩省略号，没有信息量了。
const MIN_TEXT_SCALE: f32 = 0.35;

/// 把列宽收进可用宽度。
///
/// 数字列保持声明宽度（`123.4M` 被截断比名字被截断更难读），文本列按声明
/// 宽度的比例伸缩：两列布局下每张表只分到第二列的宽度，固定列宽会溢出。
/// 伸缩上限是 1.0 —— 文本列不因容器宽就跟着变宽，列宽稳定才好对位。
fn fit_columns<const N: usize>(columns: &[Column; N], available: f32) -> [Column; N] {
    let fixed: f32 = columns
        .iter()
        .filter(|column| column.numeric)
        .map(|column| column.width)
        .sum();
    let text: f32 = columns
        .iter()
        .filter(|column| !column.numeric)
        .map(|column| column.width)
        .sum();
    if text <= 0.0 {
        return *columns;
    }
    let mut out = *columns;
    if available >= fixed + text {
        // 宽窗口（用户反馈：表格宽度要跟随自适应）：全部列等比拉伸，
        // 表格铺满卡片 —— 数字列也分到余量，否则多余的空间全堆给名字列。
        let scale = available / (fixed + text);
        for column in out.iter_mut() {
            column.width *= scale;
        }
    } else {
        // 窄窗口：只压文本列（数字列保住可读性），压到下限为止。
        let scale = ((available - fixed) / text).clamp(MIN_TEXT_SCALE, 1.0);
        for column in out.iter_mut() {
            if !column.numeric {
                column.width *= scale;
            }
        }
    }
    out
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
/// 行高 21 + 行距 4 = 25px（用户定的行距）：5 行预算 = 125px。
const CELL_HEIGHT: f32 = 21.0;

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

    /// 固定宽度列：不参与伸缩。
    ///
    /// 复用 `numeric` 标志：`fit_columns` 把它当「保持声明宽度」。用于删除
    /// 按钮这类交互列 —— 压扁了点不中。
    const fn fixed(header: &'static str, width: f32) -> Self {
        Self {
            header,
            width,
            numeric: true,
            sort: None,
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
    id: &str,
    columns: &[Column],
    sort: SortState,
    body: f32,
    rows: impl IntoIterator<Item = F>,
) -> Option<SortKey>
where
    F: FnOnce(&mut egui::Ui),
{
    // 行距从**表头**起就自控：默认 item_spacing 会叠在 add_space 上，表头
    // 与首行之间多出 ~11px 死区。5 行指 5 个**数据行**，不含小标题与表头。
    ui.spacing_mut().item_spacing.y = 0.0;
    let clicked = table_header(ui, columns, sort);
    ui.add_space(crate::theme::SPACE_1);
    let faint = ui.visuals().faint_bg_color;
    let hover = ui.visuals().widgets.hovered.weak_bg_fill;
    let row_width: f32 = columns.iter().map(|column| column.width).sum();
    // 行区在组件内部滚动（表头固定）：行数再多也不把整页撑长。
    super::component::scroll_body(ui, id, body, |ui| {
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
    });
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

/// 表格交互的结果。
enum TableAction {
    /// 点了某个表头：切换排序。
    Sort(SortKey),
    /// 点了会话行的删除（两段式：是否已是待确认行由调用方判断）。
    DeleteSession(String),
}

/// 把表头的点击合成一个动作。
fn table_action(clicked: Option<SortKey>) -> Option<TableAction> {
    clicked.map(TableAction::Sort)
}

/// Agent 维度表：Agent / 合计 / 输入 / 输出 / 缓存读 / 占比。
///
/// 不列会话数：一个会话可以横跨多天，按区间求和时「会话数」相加会重复计数。
fn agent_table(
    ui: &mut egui::Ui,
    rows: &[AgentUsage],
    sort: SortState,
    body: f32,
) -> Option<TableAction> {
    let columns = fit_columns(
        &[
            Column::sorted_text("Agent", 130.0, SortKey::Name),
            Column::number("合计", NUM_W, SortKey::Total),
            Column::number("输入", NUM_W, SortKey::Input),
            Column::number("输出", NUM_W, SortKey::Output),
            Column::number("缓存读", NUM_W, SortKey::CacheRead),
            Column::text("占比", 52.0),
        ],
        ui.available_width(),
    );
    // 占比分母 = 区间内所有 agent 的合计。
    let grand: i64 = rows.iter().map(|row| row.totals.total()).sum();
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
    let items: Vec<_> = rows
        .iter()
        .map(|row| {
            let client = row.client;
            let totals = row.totals;
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
                cell_number(
                    ui,
                    &columns,
                    5,
                    &super::overview::percent_of(totals.total(), grand),
                    false,
                );
            }
        })
        .collect();
    let clicked = table(ui, "agent-table", &columns, sort, body, items);
    table_action(clicked)
}

/// 模型维度表：模型 / 合计 / 输入 / 输出 / 缓存读 / 占比。
fn model_table(
    ui: &mut egui::Ui,
    rows: &[ModelUsage],
    sort: SortState,
    body: f32,
) -> Option<TableAction> {
    let columns = fit_columns(
        &[
            Column::sorted_text("模型", 130.0, SortKey::Name),
            Column::number("合计", NUM_W, SortKey::Total),
            Column::number("输入", NUM_W, SortKey::Input),
            Column::number("输出", NUM_W, SortKey::Output),
            Column::number("缓存读", NUM_W, SortKey::CacheRead),
            Column::text("占比", 52.0),
        ],
        ui.available_width(),
    );
    // 占比分母 = 区间内所有模型的合计。
    let grand: i64 = rows.iter().map(|row| row.totals.total()).sum();
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
    let items: Vec<_> = rows
        .iter()
        .map(|row| {
            let model_id = row.model_id.clone();
            let totals = row.totals;
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
                cell_number(
                    ui,
                    &columns,
                    5,
                    &super::overview::percent_of(totals.total(), grand),
                    false,
                );
            }
        })
        .collect();
    let clicked = table(ui, "model-table", &columns, sort, body, items);
    table_action(clicked)
}

/// 会话维度表：会话名 / 会话 / Agent / 模型 / 项目 / 区间内用量 / 状态 / 操作。
///
/// 区间不是「全部」时，用量列是该会话在区间内的部分，不是它的一生总量。
/// 会话名与项目列都可能缺（源里没存名字、旧账本条目没有项目），缺了画「—」。
/// 操作列是两段式删除：第一击标待确认，再击同一行才真删（见 [`delete_cell`]）。
#[allow(clippy::too_many_arguments)]
fn session_table(
    ui: &mut egui::Ui,
    rows: &[SessionSnapshot],
    now_ms: i64,
    range: Range,
    sort: SortState,
    body: f32,
    pending_delete: Option<&str>,
) -> Option<TableAction> {
    // 表头是 `&'static str`：按区间二选一，不动态拼字符串。
    let total_column = if range == Range::All {
        Column::number("合计", NUM_W, SortKey::Total)
    } else {
        Column::number("区间内", NUM_W, SortKey::Total)
    };
    let columns = fit_columns(
        &[
            Column::text("会话名", 210.0),
            Column::text("会话", 150.0),
            Column::sorted_text("Agent", 100.0, SortKey::Name),
            Column::text("模型", 170.0),
            Column::text("项目", 110.0),
            total_column,
            Column::text("状态", 88.0),
            Column::fixed("操作", 52.0),
        ],
        ui.available_width(),
    );
    let mut rows: Vec<SessionSnapshot> = rows.to_vec();
    sort_rows(
        &mut rows,
        sort,
        // 有会话名的按名字排，没有的退回会话 id；前缀仍是 Agent，
        // 保证同一个 agent 的会话能聚在一起。
        |row| {
            format!(
                "{}:{}",
                row.client.label(),
                row.title.as_deref().unwrap_or(&row.session_id)
            )
        },
        |row, key| match key {
            SortKey::Input => row.input,
            SortKey::Output => row.output,
            SortKey::CacheRead => row.cache_read,
            _ => row.total(),
        },
    );
    // 行闭包拿不到可变借用：删除事件用 `RefCell` 收集，画完取第一个。
    let deletions: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let items: Vec<_> = rows
        .iter()
        .map(|row| {
            let title = row.title.clone().unwrap_or_else(|| "—".to_string());
            let short = aggregate::short_session_id(&row.session_id, SESSION_ID_CHARS);
            let client = row.client;
            let model_id = row.model_id.clone();
            let project = row.project.clone().unwrap_or_else(|| "—".to_string());
            let total = row.total();
            let last_seen_ms = row.last_seen_ms;
            let key = SessionSnapshot::key(client, &row.session_id);
            let confirming = pending_delete == Some(key.as_str());
            let deletions = &deletions;
            move |ui: &mut egui::Ui| {
                // 会话名加粗当主标识，会话 id 退为辅助列（用户要求增加会话名）。
                cell_text(ui, &columns, 0, &title, true);
                cell_text(ui, &columns, 1, &short, false);
                cell_text(ui, &columns, 2, client.label(), false);
                cell_text(ui, &columns, 3, &model_id, false);
                cell_text(ui, &columns, 4, &project, false);
                cell_number(ui, &columns, 5, &aggregate::format_tokens(total), true);
                // 状态列只画最近活动：源里已消失的会话不进列表（见
                // [`crate::usage::Ledger::view`]），不存在「已归档」这个状态。
                let when = aggregate::relative_time(last_seen_ms, now_ms);
                cell_text(ui, &columns, 6, &when, false);
                delete_cell(ui, &columns, 7, &key, confirming, deletions);
            }
        })
        .collect();
    let clicked = table(ui, "session-table", &columns, sort, body, items);
    let deleted = deletions.into_inner().into_iter().next();
    if let Some(key) = deleted {
        // 删除优先：同一个单元格同帧不可能又排序又删除。
        return Some(TableAction::DeleteSession(key));
    }
    table_action(clicked)
}

/// 会话表的删除单元格：两段式确认，防在密集表格里误触。
///
/// 第一击把该行标成待确认（按钮变「确认」，红色）；再击同一行才真删。
/// 待确认状态存在 `App::usage_delete_pending`，点别行会转移过去。
fn delete_cell(
    ui: &mut egui::Ui,
    columns: &[Column],
    index: usize,
    key: &str,
    confirming: bool,
    deletions: &std::cell::RefCell<Vec<String>>,
) {
    let column = columns[index];
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(column.width, CELL_HEIGHT), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let (label, color) = if confirming {
        ("确认", crate::theme::semantics(ui).err)
    } else {
        ("删", ui.visuals().weak_text_color())
    };
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(crate::theme::TEXT_CAPTION),
        color,
    );
    paint_in_cell(ui, rect, galley, false);
    if response.clicked() {
        deletions.borrow_mut().push(key.to_string());
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if confirming {
            "再点一次确认删除：会话从 agent 的库里真删掉，不能撤销"
        } else {
            "删除该会话（从 agent 的库里真删；已计入的用量保留在统计里）"
        });
}
