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

/// 第二列（表）从上到下排的维度：Agent / 模型 / 会话。
///
/// 时间表单独放在第一列热力图下方（用户要求的布局）——它跟日历一样是
/// 「按天看」的视角，放在一起读起来顺。
const TABLES_COLUMN: [Dimension; 3] = [Dimension::Agent, Dimension::Model, Dimension::Session];

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

    /// 用量视图主体：状态行 + 两列（左：总览 / 右：四张表）。
    ///
    /// 时间 / Agent / 模型 / 会话四张表放第二列（用户要求的布局），总览卡、
    /// 趋势图与热力图留在第一列。两列都从顶部开始，互不挤占。
    pub(in crate::app) fn ui_usage_section(&mut self, ui: &mut egui::Ui) {
        self.ui_usage_status(ui);
        ui.add_space(crate::theme::SPACE_3);
        // 区间按钮横跨整个窗口宽度单独一行（用户反馈：今日 / 本周 / 本月 /
        // 全部单一行）；双栏从同一 y 开始，Agent 卡与左列总览卡顶部平齐。
        self.ui_usage_controls(ui);
        ui.add_space(crate::theme::SPACE_4);
        let sessions: Vec<SessionSnapshot> = self.usage_sessions().to_vec();
        let daily = self.usage_daily_filtered();
        if sessions.is_empty() && daily.is_empty() {
            self.ui_usage_empty(ui);
            return;
        }
        // 左右列随页面缩放、始终保持 4:3（用户反馈）：间距不计入比例，
        // 也不再夹上下限 —— 比例在任何窗口宽度下都不变。
        let gap = crate::theme::SPACE_4;
        let usable = (ui.available_width() - gap).max(1.0);
        let overview_width = usable * 4.0 / 7.0;
        let tables_width = usable * 3.0 / 7.0;

        let now_ms = unix_now_ms();
        let today = self.usage_today();
        let range = self.usage_range;
        let sort = self.usage_sort;
        let pending_delete = self.usage_delete_pending.clone();

        // 两列各是独立闭包，`self` 借不进来：交互（排序 / 删除）用 `RefCell`
        // 收集，画完统一处理。不同交互不可能同帧同时发生，逐个处理即可。
        let collected: std::cell::RefCell<Vec<TableAction>> = std::cell::RefCell::new(Vec::new());
        let record = |result: Option<TableAction>| {
            if let Some(action) = result {
                collected.borrow_mut().push(action);
            }
        };

        // 左列的底部 y：热力图是左列最后一件，会话卡要跟它对齐底边。
        let mut left_bottom = 0.0_f32;
        ui.horizontal_top(|ui| {
            // 第一列：总览（控件 + 总览卡 + 趋势图 + 热力图）。
            // 时间维度由热力图承担，不再单独列表（用户反馈：删除时间模块）。
            ui.allocate_ui_with_layout(
                egui::vec2(overview_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(overview_width);
                    self.ui_usage_overview(ui);
                    left_bottom = ui.min_rect().bottom();
                },
            );
            ui.add_space(gap);
            // 第二列：Agent / 模型 / 会话三张表从上到下。
            ui.allocate_ui_with_layout(
                egui::vec2(tables_width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(tables_width);
                    // 解除列高限制：allocate_ui_with_layout 的 0 高期望会把内部
                    // 所有 ScrollArea 的视口压到「剩余可用高度」，会话表装不满
                    // 卡内空间（用户反馈：列表要铺满组件）。
                    ui.set_max_height(f32::INFINITY);
                    for (index, dimension) in TABLES_COLUMN.iter().copied().enumerate() {
                        if index > 0 {
                            ui.add_space(crate::theme::SPACE_5);
                        }
                        // 会话卡期望高 = 左列底部 - 卡片顶部；其余表不参与对齐。
                        let card_height = (dimension == Dimension::Session)
                            .then(|| left_bottom - ui.cursor().top());
                        record(self.ui_dimension_table(
                            ui,
                            dimension,
                            &sessions,
                            &daily,
                            now_ms,
                            today,
                            range,
                            sort,
                            pending_delete.as_deref(),
                            card_height,
                        ));
                    }
                },
            );
        });

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

    /// 顶部状态行：合计、扫描进度、数据新鲜度与错误。
    /// 顶部状态行：筛选、扫描进度、数据新鲜度与错误。
    ///
    /// 压成两行（原本三行）：用量视图下面是表格，上面多占一行就少一行数据。
    fn ui_usage_status(&mut self, ui: &mut egui::Ui) {
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
        let ledger_len = self.usage.ledger_len();
        let scanning = self.usage.scanning();
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

    /// 画一个维度的表（带小节标题），返回它的交互结果。
    ///
    /// 标题用 `Dimension::label`：四个表同屏后，不标就分不清哪张是哪张。
    /// `pending_delete` 只对会话表有意义：标着哪一行正等二次确认。
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn ui_dimension_table(
        &self,
        ui: &mut egui::Ui,
        dimension: Dimension,
        sessions: &[SessionSnapshot],
        daily: &crate::usage::DailyMap,
        now_ms: i64,
        today: i64,
        range: Range,
        sort: SortState,
        pending_delete: Option<&str>,
        card_height: Option<f32>,
    ) -> Option<TableAction> {
        // 标题画在卡片**里面**：与趋势图 / 热力图的组件外壳一致。
        super::component::card(ui, |ui| {
            super::component::heading(ui, dimension.label());
            ui.add_space(crate::theme::SPACE_2);
            match dimension {
                Dimension::Agent => {
                    let rows = aggregate::by_agent_in_range(daily, range, today);
                    agent_table(ui, &rows, sort, super::component::BODY_MAX_HEIGHT)
                }
                Dimension::Model => {
                    let rows = aggregate::by_model_in_range(daily, range, today);
                    model_table(ui, &rows, sort, super::component::BODY_MAX_HEIGHT)
                }
                Dimension::Session => {
                    // 会话维度也必须走区间：拿会话总量当「本月」会虚高
                    // （一个会话可以横跨几十天）。
                    let rows = aggregate::session_rows_in_range(sessions, daily, range, today);
                    // 卡底要与左列平齐（用户反馈）：卡内装得下几整行就画几整行
                    // （末行不裁半），装不下的行在表体内滚动；内容比左列矮时由
                    // 下面的底部填充补齐，底边仍与热力图平齐。
                    let card_height = card_height.unwrap_or(0.0);
                    let consumed = ui.cursor().top() - ui.min_rect().top();
                    let rest = card_height
                        - crate::theme::SPACE_3 * 2.0
                        - CARD_STROKE_OVERHEAD
                        - consumed
                        - TABLE_HEADER_HEIGHT;
                    let rows_fit = (rest / ROW_PITCH).floor();
                    let max_height = if card_height > 0.0 && rows_fit >= MIN_ALIGNED_ROWS {
                        rows_fit * ROW_PITCH
                    } else {
                        super::component::BODY_MAX_HEIGHT
                    };
                    let action =
                        session_table(ui, &rows, now_ms, range, sort, max_height, pending_delete);
                    if card_height > 0.0 {
                        // 底部填充：把卡内高度精确补到目标值。add_space 是纯光标推进
                        // （不叠加 item_spacing），min_rect 会包含它 —— 卡片总高即期望高，
                        // 底边与热力图平齐。
                        let target = ui.min_rect().top() + card_height
                            - crate::theme::SPACE_3 * 2.0
                            - CARD_STROKE_OVERHEAD;
                        let fill = target - ui.cursor().top();
                        if fill > 0.0 {
                            ui.add_space(fill);
                        }
                    }
                    action
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

/// 卡片描边（1px）在两侧各外扩约 1px（取整后）：实测外框高 = 内容
/// + 2×内边距 + 2。会话卡对齐预算与底部填充都按外框目标反推内容高。
const CARD_STROKE_OVERHEAD: f32 = 2.0;

/// 一行占的高（行高 + 行距）：会话卡的对齐预算按整行数取整用。
const ROW_PITCH: f32 = CELL_HEIGHT + crate::theme::SPACE_1;

/// 对齐生效所需的至少整行数：再矮的卡对不齐也难看，不如按 5 行滚动。
const MIN_ALIGNED_ROWS: f32 = 2.0;

/// 表头行占的高（行高 + 行距）：会话卡对齐预算要从卡高里扣掉 ——
/// 不扣的话卡底边比热力图低出一截（用户反馈）。
const TABLE_HEADER_HEIGHT: f32 = CELL_HEIGHT + crate::theme::SPACE_1;

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
    max_height: f32,
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
    super::component::scroll_body(ui, id, max_height, |ui| {
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
    max_height: f32,
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
    let clicked = table(ui, "agent-table", &columns, sort, max_height, items);
    table_action(clicked)
}

/// 模型维度表：模型 / 合计 / 输入 / 输出 / 缓存读 / 占比。
fn model_table(
    ui: &mut egui::Ui,
    rows: &[ModelUsage],
    sort: SortState,
    max_height: f32,
) -> Option<TableAction> {
    let columns = fit_columns(
        &[
            Column::sorted_text("模型", 210.0, SortKey::Name),
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
    let clicked = table(ui, "model-table", &columns, sort, max_height, items);
    table_action(clicked)
}

/// 会话维度表：会话 / Agent / 模型 / 项目 / 区间内用量 / 状态 / 操作。
///
/// 区间不是「全部」时，用量列是该会话在区间内的部分，不是它的一生总量。
/// 项目列来自 tokscale 解析出的工作区，旧账本条目没有，画「—」。
/// 操作列是两段式删除：第一击标待确认，再击同一行才真删（见 [`delete_cell`]）。
#[allow(clippy::too_many_arguments)]
fn session_table(
    ui: &mut egui::Ui,
    rows: &[SessionSnapshot],
    now_ms: i64,
    range: Range,
    sort: SortState,
    max_height: f32,
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
            Column::text("会话", 170.0),
            Column::sorted_text("Agent", 110.0, SortKey::Name),
            Column::text("模型", 190.0),
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
    // 行闭包拿不到可变借用：删除事件用 `RefCell` 收集，画完取第一个。
    let deletions: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let items: Vec<_> = rows
        .iter()
        .map(|row| {
            let short = aggregate::short_session_id(&row.session_id, SESSION_ID_CHARS);
            let client = row.client;
            let model_id = row.model_id.clone();
            let project = row.project.clone().unwrap_or_else(|| "—".to_string());
            let total = row.total();
            let last_seen_ms = row.last_seen_ms;
            let archived = row.archived;
            let key = SessionSnapshot::key(client, &row.session_id);
            let confirming = pending_delete == Some(key.as_str());
            let deletions = &deletions;
            move |ui: &mut egui::Ui| {
                cell_text(ui, &columns, 0, &short, false);
                cell_text(ui, &columns, 1, client.label(), false);
                cell_text(ui, &columns, 2, &model_id, false);
                cell_text(ui, &columns, 3, &project, false);
                cell_number(ui, &columns, 4, &aggregate::format_tokens(total), true);
                // 归档会话优先标状态（它比「最近活动」更值得注意：
                // 用户可能以为统计读错了，得说清用量来自账本）。
                if archived {
                    cell_text(ui, &columns, 5, "已归档", false);
                } else {
                    let when = aggregate::relative_time(last_seen_ms, now_ms);
                    cell_text(ui, &columns, 5, &when, false);
                }
                delete_cell(ui, &columns, 6, &key, confirming, deletions);
            }
        })
        .collect();
    let clicked = table(ui, "session-table", &columns, sort, max_height, items);
    let deleted = deletions.into_inner().into_iter().next();
    if let Some(key) = deleted {
        // 删除优先：同一个单元格同帧不可能又排序又删除。
        return Some(TableAction::DeleteSession(key));
    }
    table_action(clicked)
}

/// 会话表的删除单元格：两段式确认，防在密集表格里误触把账本数据删了。
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
            "再点一次确认删除：该会话将从用量统计中永久移除（不影响磁盘文件）"
        } else {
            "从用量统计中删除该会话"
        });
}
