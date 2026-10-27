//! The usage view UI: top view switcher + four-dimension stat tables.
//!
//! All data comes from the [`super::super::usage`] background scan (`App::usage`); this module only
//! draws. Aggregation and formatting live in [`super::aggregate`] as unit-testable pure logic.
//!
//! The dimension-switch buttons reuse the top-bar tab selected style (`widgets.hovered.bg_fill` + 2px
//! border); colors always come from `crate::theme::semantics(ui)` — nothing hard-coded.

use super::aggregate::{self, AgentUsage, Dimension, ModelUsage};
use super::range::Range;
use crate::app::App;
use crate::usage::SessionSnapshot;
use eframe::egui;

/// Two side-by-side tables: Agents / Models.
///
/// The session table is not side-by-side: it has the most columns (name / id / Agent / model /
/// project / usage / status / actions) and squeezing it into half width turns every column into
/// ellipses, so it takes a full row (per user request).
const SIDE_BY_SIDE: [Dimension; 2] = [Dimension::Agent, Dimension::Model];

/// Max characters shown for a session id in the table.
const SESSION_ID_CHARS: usize = 20;

impl App {
    /// View switch: `[Provider Management] [Usage]`, one of two.
    ///
    /// The two buttons swapped positions (user request): provider management first. Painted right of the agent tabs.
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

    /// One view-switch button: selected = hover-tinted fill + 2px border (same set as the top-bar tabs).
    fn view_switch_button(&self, ui: &mut egui::Ui, view: super::super::MainView) -> bool {
        super::chip(ui, self.main_view == view, view.label(), 72.0)
    }

    /// The usage view's body: a single-column vertical flow.
    ///
    /// Layout (user request): the overview (trend chart + heatmap) takes full width → Agent and Models side
    /// by side → the session table takes a full row. No more left/right split; each table gets more column width.
    ///
    /// The status line (with the range switch) is not here: it's pinned above the scroll area, not scrolling
    /// with the page (user request), painted first by [`App::update`].
    pub(in crate::app) fn ui_usage_section(&mut self, ui: &mut egui::Ui) {
        let sessions: Vec<SessionSnapshot> = self.usage_sessions().to_vec();
        let daily = self.usage_daily_filtered();
        if sessions.is_empty() && daily.is_empty() {
            self.ui_usage_empty(ui);
            return;
        }
        // The overview (overview card + trend chart + heatmap) takes full width.
        self.ui_usage_overview(ui);
        ui.add_space(crate::theme::SPACE_5);

        let now_ms = unix_now_ms();
        let today = self.usage_today();
        let range = self.usage_range;
        let sort = self.usage_sort;
        let pending_delete = self.usage_delete_pending.clone();

        // Tables draw inside their own closures where `self` can't be borrowed: interactions (sort / delete) are
        // collected via `RefCell` and processed together after painting. Two interactions can't happen in the
        // same frame, so handling them one by one suffices.
        let collected: std::cell::RefCell<Vec<TableAction>> = std::cell::RefCell::new(Vec::new());
        let record = |result: Option<TableAction>| {
            if let Some(action) = result {
                collected.borrow_mut().push(action);
            }
        };

        // The side-by-side tables' row sets are computed first: body height locks to the taller side's row count,
        // so both cards bottom-align (user request: the Agent card matches the Models card height).
        let side: Vec<TableRows> = SIDE_BY_SIDE
            .iter()
            .copied()
            .map(|dimension| TableRows::of(dimension, &sessions, &daily, range, today))
            .collect();
        let side_rows = side.iter().map(TableRows::len).max().unwrap_or(0);
        let side_body = super::component::body_height(side_rows, super::component::BODY_MAX_HEIGHT);

        // Agent and Models side by side, half each: spacing stays out of the ratio.
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
                        // Lift the inner height limit: allocate_ui_with_layout's 0-height expectation squashes every internal
                        // ScrollArea's viewport to the "remaining available height", leaving the body unable to fill the card
                        // (user feedback: lists must fill the component).
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
        // The session table takes a full row: it has the most columns, and half width turns every column into ellipses.
        let session_rows = TableRows::of(Dimension::Session, &sessions, &daily, range, today);
        record(self.ui_dimension_table(
            ui,
            &session_rows,
            now_ms,
            range,
            sort,
            // The table body is three times the other tables' height (user request). Locked the same way: with only a
            // cap, the viewport gets squeezed by the page's remaining height, and a short window hides the 3x rows.
            super::component::body_height(
                session_rows.len(),
                super::component::SESSION_BODY_HEIGHT,
            ),
            pending_delete.as_deref(),
        ));

        let frame_time = ui.input(|input| input.time);
        self.handle_table_actions(collected.into_inner(), frame_time);
    }

    /// Handle this round's table interactions: sorts apply immediately; deletes go through two-step confirmation
    /// (first click arms, clicking the same row again really deletes).
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

    /// The top status line: filter / scan progress / data freshness / errors + the range switch on the far right.
    ///
    /// This line is painted by [`App::update`] above the scroll area, so it stays fixed (user request): even
    /// deep in a long table, the current range and filter remain visible.
    ///
    /// Layout: the left text fills "the row minus the range control's width", the range switch hugs the right
    /// edge; both cells top-align, and the row's height takes the taller of the two.
    pub(in crate::app) fn ui_usage_status(&mut self, ui: &mut egui::Ui) {
        let semantics = crate::theme::semantics(ui);
        let now_ms = unix_now_ms();
        let frame_time = ui.input(|input| input.time);
        // Extract the data to paint first: `self` still needs a mutable borrow later (dimension switching),
        // so ending the `self.usage` borrow early keeps things simple.
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

        // Layout: the left text fills the remaining width, the range switch hugs the right (user request).
        //
        // Each cell gets a dead-rect child `Ui`: `allocate_ui_with_layout` only hands the actually-used width
        // (`min_rect`) back to the parent, so short filter text would drag the whole range-button group leftward.
        // Pin both cells' positions first, then hand the row's height back to the parent (the taller cell's).
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
        // The text may wrap, or grow an error / ledger line: the child `Ui`'s remaining height can't serve as the
        // cap, or the second line would be squeezed to zero.
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
                // Keep `Align::Min` for horizontal alignment: `Max` in a vertical layout satisfies
                // `prefer_right_to_left()`, reversing the `ui.horizontal` inside `chip_group`,
                // and the four buttons would read "All … Today" right-to-left.
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        right.set_max_height(f32::INFINITY);
        self.ui_usage_range_chips(&mut right);
        let right_height = right.min_rect().height();

        // Hand the row's height back to the parent: `new_child` only paints, never reserves — the parent's cursor
        // still sits at the line start, and without advancing it the following `ScrollArea` paints from the status
        // line's top, covering the whole row (exactly how the covered-first-column defect arose).
        // `expand_to_include_rect` merges only `min_rect`/`max_rect` and never moves the cursor, so the
        // cursor-advancing variant is required here.
        ui.advance_cursor_after_rect(egui::Rect::from_min_max(
            row.min,
            egui::pos2(row.max.x, row.min.y + left_height.max(right_height)),
        ));
    }

    /// The status line's left text: filter / cache or scan freshness / errors / ledger-archive note.
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
                // Instant cache display: ledger data surfaces first (auto-replaced when the background scan finishes).
                // State the data's age clearly so minutes-old numbers aren't mistaken for live ones; past 5 minutes, warning color.
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
                // "Updated N seconds ago": convert the scan moment on egui's timeline into elapsed time.
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
                    // First open, scan still on the way: show a "scanning" indicator (user feedback: needed right after opening).
                    // Once data exists it stops painting, so it doesn't sit beside the numbers permanently
                    // (it was removed once before on user feedback).
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
        // Source-side deletion failure (process running / write failed): state clearly whether the statistics half
        // was also handled. A fully blocked deletion must not read "removed from statistics", or the user
        // would believe it succeeded.
        if let Some(err) = self.usage_source_error.clone() {
            ui.label(egui::RichText::new(err).color(semantics.err));
        }
        // The ledger line only says "history exceeds what's on disk": whether that means "vanished from the
        // sources" is an implementation detail — what the user cares about is why the numbers exceed what the
        // source directory shows.
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

    /// Draw one dimension's table (with section title); returns its interaction result.
    ///
    /// Titles use `Dimension::label`: with four tables on screen, unlabeled ones are indistinguishable.
    /// `body` comes from the caller: the side-by-side pair shares one budget so both cards stay equal height.
    /// `pending_delete` only matters for the session table: it marks the row awaiting second confirmation.
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
        // The title paints **inside** the card: consistent with the trend chart / heatmap component shells.
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

    /// Empty state: one explanatory line + one line on "how to make numbers appear".
    fn ui_usage_empty(&self, ui: &mut egui::Ui) {
        let semantics = crate::theme::semantics(ui);
        crate::ui::empty_state(
            ui,
            "没有找到本机用量数据。",
            "统计的是各 agent 自己写在磁盘上的会话账本（pi / opencode / workbuddy / dsh 等）。\
             先用其中任意一个跑一轮对话，数字就会出现在这里。",
            Some(semantics.warn),
            None,
        );
    }
}

/// Current time (Unix millis).
fn unix_now_ms() -> i64 {
    super::unix_now_secs().saturating_mul(1_000)
}

/// A table's row set.
///
/// The three dimensions have different row types, gathered into one enum: the two side-by-side tables
/// count their rows first and lock the table body height to the taller side so both cards bottom-align
/// (user request: the Agent card must match the Models card height). Row sets are computed before
/// drawing so height can't drift with paint order.
enum TableRows {
    Agent(Vec<AgentUsage>),
    Model(Vec<ModelUsage>),
    Session(Vec<SessionSnapshot>),
}

impl TableRows {
    /// Aggregate rows by dimension and range.
    ///
    /// The session dimension must also honor the range: using lifetime totals as "this month" overstates
    /// (one session can span dozens of days).
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

    /// Which dimension these rows belong to: the card title and interactions both follow it.
    fn dimension(&self) -> Dimension {
        match self {
            Self::Agent(_) => Dimension::Agent,
            Self::Model(_) => Dimension::Model,
            Self::Session(_) => Dimension::Session,
        }
    }

    /// Row count: the side-by-side pair takes the larger as their shared height budget.
    fn len(&self) -> usize {
        match self {
            Self::Agent(rows) => rows.len(),
            Self::Model(rows) => rows.len(),
            Self::Session(rows) => rows.len(),
        }
    }
}

/// Lower ratio bound for text columns: any narrower and only ellipses remain, carrying no information.
const MIN_TEXT_SCALE: f32 = 0.35;

/// Fit declared column widths into the available width.
///
/// Number columns keep their declared width (a truncated `123.4M` is harder to read than a truncated
/// name); text columns scale proportionally to their declared width. In the two-column layout each
/// table only gets the second column's width, so fixed widths would overflow. The scale cap is 1.0 —
/// text columns don't widen just because the container is wide; stable column widths are what keeps
/// columns aligned.
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
        // Wide windows (user feedback: table width should follow): all columns stretch proportionally and the
        // table fills the card — number columns also get slack, otherwise all spare width piles onto the name column.
        let scale = available / (fixed + text);
        for column in out.iter_mut() {
            column.width *= scale;
        }
    } else {
        // Narrow windows: only text columns compress (number columns keep readability), down to the floor.
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
// Table: column-aligned layout primitive
// ---------------------------------------------------------------------------
//
// Layout follows the reference (tokscale's `tui-daily.png`): de-emphasized headers,
// **right-aligned numbers**, fixed column widths. `egui::Grid` is not used because it lacks
// per-column alignment (stated in `grid.rs`: "Grid not yet available for right-to-left layouts"),
// so numbers would be left-aligned and mixed-digit values would stagger — exactly where the
// ugliness came from.
//
// The key is that **every column must occupy exactly its declared width**. `allocate_ui_with_layout`
// can't do that: it only hands back the actually-used width (`min_rect`) to the parent `Ui` (see
// egui `ui.rs`, `allocate_ui_with_layout_dyn`: "the amount of space actually used (min_rect) will be
// allocated"), so columns after narrow content shift left and stop aligning. This uses
// `allocate_exact_size` instead: it reserves exactly the requested size, making widths dead-strict.

/// Row height (also each cell's height).
/// Row height 21 + spacing 4 = 25px (user-set spacing): a 5-row budget = 125px.
const CELL_HEIGHT: f32 = 21.0;

/// Typical width for number columns: `123.4M` is at most 6-7 characters.
const NUM_W: f32 = 68.0;

/// Horizontal cell padding: with fixed column widths, leave this much on both sides of the content,
/// otherwise numbers touch the next column's header (a flush edge looks glued together).
const CELL_PAD_X: f32 = 6.0;

/// A column definition.
#[derive(Clone, Copy)]
struct Column {
    /// Header text.
    header: &'static str,
    /// Column width (points).
    width: f32,
    /// Number column: right-aligned + monospace font.
    numeric: bool,
    /// Sort key: `None` = not sortable (text columns like session id, model name).
    sort: Option<SortKey>,
}

impl Column {
    /// Text column (left-aligned, unsortable).
    const fn text(header: &'static str, width: f32) -> Self {
        Self {
            header,
            width,
            numeric: false,
            sort: None,
        }
    }

    /// Number column (right-aligned + monospace), clickable header to sort.
    const fn number(header: &'static str, width: f32, sort: SortKey) -> Self {
        Self {
            header,
            width,
            numeric: true,
            sort: Some(sort),
        }
    }

    /// Fixed-width column: excluded from stretching.
    ///
    /// Reuses the `numeric` flag: `fit_columns` treats it as "keep declared width". For interactive columns
    /// like the delete button — squashing them would make them unclickable.
    const fn fixed(header: &'static str, width: f32) -> Self {
        Self {
            header,
            width,
            numeric: true,
            sort: None,
        }
    }

    /// Text column + sortable (for dates, Agent names, etc.).
    const fn sorted_text(header: &'static str, width: f32, sort: SortKey) -> Self {
        Self {
            header,
            width,
            numeric: false,
            sort: Some(sort),
        }
    }
}

/// Sortable keys.
///
/// Only the genuinely meaningful ones: every dimension sorts by "usage" and "name"; the time
/// dimension is inherently time-ordered. More options would just be quiz questions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::app) enum SortKey {
    /// Name / model id / date.
    Name,
    /// Total tokens.
    Total,
    /// Input tokens.
    Input,
    /// Output tokens.
    Output,
    /// Cache-read tokens.
    CacheRead,
}

/// Sort state: current key + descending flag.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) struct SortState {
    pub key: SortKey,
    /// `true` = larger first. Number columns default descending (usage is what you scan for), name columns ascending.
    pub descending: bool,
}

impl SortState {
    /// Default: total descending ("who used the most" is what users care about).
    pub(in crate::app) const fn by_total() -> Self {
        Self {
            key: SortKey::Total,
            descending: true,
        }
    }

    /// Header click: same key flips direction, a new key takes that key's default direction.
    pub(in crate::app) fn toggled(self, key: SortKey) -> Self {
        if self.key == key {
            Self {
                key,
                descending: !self.descending,
            }
        } else {
            Self {
                key,
                // Names ascending (A→Z reads naturally), everything else descending (larger first).
                descending: key != SortKey::Name,
            }
        }
    }

    /// The sort direction arrow.
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

/// Sort by current key and direction; ties fall back to ascending name so the UI doesn't jump.
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

/// Table: header + rows, columns laid out at fixed widths.
///
/// Each row is a closure that fills cells into the current row (via [`cell_text`] / [`cell_number`]);
/// all four dimensions share one set of width/alignment rules instead of restating the layout.
///
/// Row background: a very faint fill on alternate rows (zebra stripes), plus a hover tint on the row
/// under the pointer — with fixed widths and 16pt rows, missing cues make rows easy to misread.
///
/// Returns the clicked header column (`None` if none).
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
    // Row spacing is self-managed from the **header** on: the default item_spacing stacks onto add_space,
    // leaving an ~11px dead zone between header and first row. "5 rows" means 5 **data rows**,
    // excluding the section title and header.
    ui.spacing_mut().item_spacing.y = 0.0;
    let clicked = table_header(ui, columns, sort);
    ui.add_space(crate::theme::SPACE_1);
    let faint = ui.visuals().faint_bg_color;
    let hover = ui.visuals().widgets.hovered.weak_bg_fill;
    let row_width: f32 = columns.iter().map(|column| column.width).sum();
    // The row region scrolls inside the component (header fixed): any row count leaves the page length alone.
    super::component::scroll_body(ui, id, body, |ui| {
        for (index, row) in rows.into_iter().enumerate() {
            // Reserve first to get this row's rect, then paint the background beneath the content.
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(row_width, CELL_HEIGHT), egui::Sense::hover());
            let corner = ui.visuals().widgets.inactive.corner_radius;
            let base = if index % 2 == 1 {
                faint
            } else {
                egui::Color32::TRANSPARENT
            };
            // The highlight fades in rather than snapping: crossing rows with the pointer used to strobe.
            // Keyed by position, so the highlight follows the row under the pointer across re-sorts.
            let t =
                crate::motion::hover_t(ui.ctx(), egui::Id::new((id, index)), response.hovered());
            let fill = crate::motion::lerp_color(base, hover, t);
            if fill != egui::Color32::TRANSPARENT {
                ui.painter().rect_filled(rect, corner, fill);
            }
            // Content paints inside the reserved rect: a fresh child `Ui`, cursor starting at the line's head.
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

/// Header: de-emphasized, same width and alignment as its data column so it sits exactly above the numbers.
///
/// Sortable columns render as clickable buttons (pointer cursor on hover); the active sort column
/// carries a ▲/▼ indicator.
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

/// Place a galley in its cell per the column's alignment: numbers flush right, text flush left, vertically centered.
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

/// Text cell (left-aligned; truncates long text, full text on hover).
fn cell_text(ui: &mut egui::Ui, columns: &[Column], index: usize, text: &str, strong: bool) {
    let column = columns[index];
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(column.width, CELL_HEIGHT), egui::Sense::hover());
    let color = if strong {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    // Over-wide truncates to ellipses (`max_rows = 1`); full text on hover.
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

/// Number cell (right-aligned + monospace so mixed-digit values still align).
fn cell_number(ui: &mut egui::Ui, columns: &[Column], index: usize, text: &str, strong: bool) {
    let column = columns[index];
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(column.width, CELL_HEIGHT), egui::Sense::hover());
    // The emphasized state gets a different color for the "total" column, **not bold**: bolding a monospace
    // face changes glyph widths, and within one column the bold and regular numbers' right edges misalign.
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

/// Result of table interaction.
enum TableAction {
    /// A header was clicked: toggle sorting.
    Sort(SortKey),
    /// A session row's delete was clicked (two-step: whether it's already armed is the caller's call).
    DeleteSession(String),
}

/// Combine header clicks into one action.
fn table_action(clicked: Option<SortKey>) -> Option<TableAction> {
    clicked.map(TableAction::Sort)
}

/// Agent dimension table: Agent / total / input / output / cache read / share.
///
/// No session count column: one session can span many days, and summing "session count" over a
/// range would double-count.
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
    // The share's denominator = all agents' total within the range.
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

/// Model dimension table: Model / total / input / output / cache read / share.
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
    // The share's denominator = all models' total within the range.
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

/// Session dimension table: name / session / Agent / model / project / in-range usage / status / actions.
///
/// When the range isn't "all", the usage column shows the session's in-range portion, not its
/// lifetime total. Name and project may be missing (sources may store no name; old ledger entries
/// lack projects) — missing values render as "—". The action column is two-step delete: first click
/// arms confirmation, clicking the same row again really deletes (see [`delete_cell`]).
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
    // The header is `&'static str`: one of two per range, never dynamically assembled.
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
        // Named sessions sort by name, unnamed fall back to session id; the prefix stays the Agent,
        // keeping one agent's sessions grouped together.
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
    // Row closures can't take the mutable borrow: delete events collect in a `RefCell`, first one wins after painting.
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
                // The session name bolds as the primary identifier; the session id demotes to a secondary column (user request to add names).
                cell_text(ui, &columns, 0, &title, true);
                cell_text(ui, &columns, 1, &short, false);
                cell_text(ui, &columns, 2, client.label(), false);
                cell_text(ui, &columns, 3, &model_id, false);
                cell_text(ui, &columns, 4, &project, false);
                cell_number(ui, &columns, 5, &aggregate::format_tokens(total), true);
                // The status column only shows the latest activity: sessions gone from the sources never enter the list (see
                // [`crate::usage::Ledger::view`]), so there is no "archived" status.
                let when = aggregate::relative_time(last_seen_ms, now_ms);
                cell_text(ui, &columns, 6, &when, false);
                delete_cell(ui, &columns, 7, &key, confirming, deletions);
            }
        })
        .collect();
    let clicked = table(ui, "session-table", &columns, sort, body, items);
    let deleted = deletions.into_inner().into_iter().next();
    if let Some(key) = deleted {
        // Delete wins: one cell can't sort and delete in the same frame.
        return Some(TableAction::DeleteSession(key));
    }
    table_action(clicked)
}

/// The session table's delete cell: two-step confirmation against mis-clicks in a dense table.
///
/// First click marks the row as pending (button turns "confirm", red); a second click on the same
/// row really deletes. Pending state lives in `App::usage_delete_pending` and moves if another
/// row is clicked.
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
        ui.painter().rect_filled(
            rect,
            ui.visuals().widgets.inactive.corner_radius,
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
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
