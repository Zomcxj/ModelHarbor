//! The usage view's two-column layout and top-bar button placement.
//!
//! Two user-feedback constraints:
//!
//! 1. **Top bar**: `[Provider Management] [Usage]` sits right next to the agent tabs (the two buttons
//!    were swapped in order), not flung to the far right.
//! 2. **Two-column body**: the left column is the overview (overview card / trend chart / heatmap); the
//!    right column stacks the time / Agent / model / session tables top to bottom; the session table
//!    has "Project" and "Actions" columns.
//!
//! Only geometry and element presence are asserted, never pixel values — theme or font changes won't
//! cause false failures.

use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DayTotals, SessionSnapshot};
use eframe::egui;

/// A rendered piece of text with its rect and font size.
struct TextAt {
    text: String,
    rect: egui::Rect,
    /// Font sizes: section titles are `TEXT_TITLE`; table column headers are `TEXT_CAPTION`.
    /// "Sessions" and "Models" are both section titles and column headers — only font size tells them apart.
    size: f32,
}

/// Collect all text shapes (including nested).
fn texts_of(out: &egui::FullOutput) -> Vec<TextAt> {
    let mut texts = Vec::new();
    fn collect(shape: &egui::Shape, out: &mut Vec<TextAt>) {
        match shape {
            egui::Shape::Text(t) => out.push(TextAt {
                text: t.galley.text().to_string(),
                rect: egui::Rect::from_min_size(t.pos, t.galley.size()),
                size: t
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| section.format.font_id.size)
                    .unwrap_or_default(),
            }),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    for cs in &out.shapes {
        collect(&cs.shape, &mut texts);
    }
    texts
}

/// Build one session snapshot (with project).
fn session(id: &str, model: &str, input: i64) -> SessionSnapshot {
    SessionSnapshot {
        client: ConfigFormat::Pi,
        session_id: id.to_string(),
        model_id: model.to_string(),
        project: Some("ModelHarbor".to_string()),
        title: None,
        input,
        output: input / 10,
        cache_read: input * 4,
        cache_write: 0,
        reasoning: 0,
        message_count: 3,
        cost: 0.0,
        first_seen_ms: 1_700_000_000_000,
        last_seen_ms: 1_700_000_000_000,
        archived: false,
    }
}

/// Build one day's bucket: given model and usage, and record the day's session into `by_session`
/// (the session table's rows come from `session_rows_in_range`, which only reads `by_session`).
fn bucket_for(date: &str, models: &[(&str, i64)], active: &[(&str, i64)]) -> DailyBucket {
    let mut bucket = DailyBucket::new(date);
    for (model, input) in models {
        let totals = DayTotals {
            input: *input,
            output: input / 10,
            cache_read: input * 4,
            messages: 1,
            ..DayTotals::default()
        };
        bucket.totals.add_assign(&totals);
        bucket.by_model.insert((*model).to_string(), totals);
    }
    for (session_id, input) in active {
        let totals = DayTotals {
            input: *input,
            output: input / 10,
            cache_read: input * 4,
            messages: 1,
            ..DayTotals::default()
        };
        bucket.by_session.insert(format!("pi:{session_id}"), totals);
    }
    bucket.by_client.insert("pi".into(), bucket.totals);
    bucket.by_client_model.insert(
        "pi".into(),
        [("gpt-5".to_string(), bucket.totals)].into_iter().collect(),
    );
    bucket
}

/// Offscreen-render the usage view (default data); returns all text shapes.
fn render(width: f32) -> Vec<TextAt> {
    let (sessions, daily) = standard_data();
    texts_of(&render_output(width, sessions, daily))
}

/// Default test data: two sessions, two days of usage.
fn standard_data() -> (Vec<SessionSnapshot>, crate::usage::DailyMap) {
    let sessions = vec![
        session("session-aaaa", "gpt-5", 1_000_000),
        session("session-bbbb", "claude-opus-5", 500_000),
    ];
    let daily: crate::usage::DailyMap = [
        (
            "2026-01-01".to_string(),
            bucket_for(
                "2026-01-01",
                &[("gpt-5", 1_000_000)],
                &[("session-aaaa", 1_000_000)],
            ),
        ),
        (
            "2026-01-02".to_string(),
            bucket_for(
                "2026-01-02",
                &[("claude-opus-5", 500_000)],
                &[("session-bbbb", 500_000)],
            ),
        ),
    ]
    .into_iter()
    .collect();
    (sessions, daily)
}

/// Offscreen-render the usage view (custom data); returns the full output (texts and card rects).
fn render_output(
    width: f32,
    sessions: Vec<SessionSnapshot>,
    daily: crate::usage::DailyMap,
) -> egui::FullOutput {
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        usage_range: crate::app::usage::range::Range::All,
        // The test data uses fixed dates outside "last 30 days": the chart's range is opened to all.
        usage_chart_range: crate::app::usage::range::ChartRange::All,
        guide_dismissed: true,
        ..App::default()
    };
    app.set_usage_result_for_test(sessions, daily);

    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 1600.0));
    ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Copied from [`App::update`]'s usage branch: the status line (with the range switch) paints
                // **outside** the scroll area, pinned to the top, never scrolling.
                app.ui_usage_status(ui);
                ui.add_space(crate::theme::SPACE_4);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        app.ui_usage_section(ui);
                    });
            });
        },
    )
}

/// Find a section title (`TEXT_TITLE` font size).
fn heading_of(texts: &[TextAt], label: &str, what: &str) -> egui::Rect {
    texts
        .iter()
        .find(|t| t.text == label && (t.size - crate::theme::TEXT_TITLE).abs() < 0.01)
        .unwrap_or_else(|| panic!("{what}没渲染出小节标题「{label}」"))
        .rect
}

/// Single-column vertical flow: overview on top, Agent and Models side by side, session table full-width at the bottom.
#[test]
fn tables_are_laid_out_in_a_single_column() {
    let texts = render(1600.0);
    let agent = heading_of(&texts, "Agent", "宽窗口");
    let model = heading_of(&texts, "模型", "宽窗口");
    let session = heading_of(&texts, "会话", "宽窗口");
    let chart = texts
        .iter()
        .find(|t| t.text == "用量趋势")
        .expect("没渲染出趋势图标题")
        .rect;

    // Agent and Models side by side: top-aligned, Agent on the left.
    assert!(
        (agent.top() - model.top()).abs() < 1.0,
        "Agent 与模型应顶对齐：{:.1} vs {:.1}",
        agent.top(),
        model.top()
    );
    assert!(agent.left() < model.left(), "Agent 应在模型左边");
    // The session table sits below both, taking a full row.
    assert!(
        session.top() > agent.top(),
        "会话表应在 Agent / 模型下面：{:.1} vs {:.1}",
        session.top(),
        agent.top()
    );
    // The overview sits above the tables, sharing the Agent card's column (single-column flow: same left edge).
    assert!(chart.top() < agent.top(), "总览应在表格上面");
    assert!(
        (chart.left() - agent.left()).abs() < 1.0,
        "单列流下总览与 Agent 卡应左对齐：{:.1} vs {:.1}",
        chart.left(),
        agent.left()
    );
}

/// The session table has "Project" and "Actions" headers, and data rows render project names.
#[test]
fn session_table_shows_project_and_delete_columns() {
    let texts = render(1600.0);
    for header in ["项目", "操作"] {
        assert!(
            texts.iter().any(|t| t.text == header),
            "会话表缺「{header}」列头"
        );
    }
    assert!(
        texts.iter().any(|t| t.text == "ModelHarbor"),
        "会话行的项目名没画出来"
    );
    // One "delete" entry per session row.
    let deletes = texts.iter().filter(|t| t.text == "删").count();
    assert_eq!(deletes, 2, "两个会话应各有一个删除入口");
}

/// In a narrow window, Agent and Models stay side by side and the session table stays full-width.
#[test]
fn narrow_window_keeps_agent_and_model_side_by_side() {
    let texts = render(900.0);
    let agent = heading_of(&texts, "Agent", "窄窗口");
    let model = heading_of(&texts, "模型", "窄窗口");
    let session = heading_of(&texts, "会话", "窄窗口");
    assert!(
        (agent.top() - model.top()).abs() < 1.0,
        "窄窗口下 Agent 与模型仍应顶对齐：{:.1} vs {:.1}",
        agent.top(),
        model.top()
    );
    assert!(agent.left() < model.left(), "窄窗口下 Agent 仍应在模型左边");
    assert!(
        session.top() > model.top(),
        "窄窗口下会话表仍应在 Agent / 模型下面"
    );
}

/// Top bar: `[Provider Management] [Usage]` right next to the tabs — order was swapped, and it's not far right.
#[test]
fn view_switch_keeps_swapped_order_next_to_the_tabs() {
    let mut app = App {
        guide_dismissed: true,
        ..App::default()
    };
    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    app.load_backend_icons(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 800.0));
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| app.ui_top_bar(ctx),
    );
    let texts = texts_of(&out);
    let usage = texts
        .iter()
        .find(|t| t.text == crate::app::MainView::Usage.label())
        .expect("顶栏应渲染出「用量」按钮");
    let providers = texts
        .iter()
        .find(|t| t.text == crate::app::MainView::Providers.label())
        .expect("顶栏应渲染出「提供商管理」按钮");

    // The swapped order: provider management first, usage second.
    assert!(
        providers.rect.left() < usage.rect.left(),
        "「提供商管理」应在「用量」前面：{} vs {}",
        providers.rect.left(),
        usage.rect.left()
    );
    // Not at the far right: the switch strip stays in the left half beside the tabs.
    assert!(
        usage.rect.left() < screen.width() / 2.0,
        "「用量」按钮不该在最右边：x={}",
        usage.rect.left()
    );
}

/// The Agent / Models tables' last column is "share".
#[test]
fn agent_and_model_tables_have_a_share_column() {
    let texts = render(1600.0);
    let shares: Vec<_> = texts.iter().filter(|t| t.text == "占比").collect();
    assert_eq!(shares.len(), 2, "两张表各一个占比列头");
}

/// Collect card rects: rounded rects with a 1px border (component card shells).
fn card_rects_of(out: &egui::FullOutput) -> Vec<egui::Rect> {
    let mut rects = Vec::new();
    fn collect(shape: &egui::Shape, rects: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) => {
                if (r.stroke.width - 1.0).abs() < 0.01 {
                    rects.push(r.rect);
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, rects)),
            _ => {}
        }
    }
    for cs in &out.shapes {
        collect(&cs.shape, &mut rects);
    }
    rects
}

/// Realistic data shape: `count` sessions, each day carrying cost (cost stays in the data layer; the UI no longer shows it).
fn realistic_data_with(count: usize) -> (Vec<SessionSnapshot>, crate::usage::DailyMap) {
    let mut sessions = Vec::new();
    let mut daily: crate::usage::DailyMap = Default::default();
    for index in 0..count {
        let id = format!("session-{index:02}");
        sessions.push(session(&id, "gpt-5", 100_000 + index as i64 * 10_000));
        let date = if index % 2 == 0 {
            "2026-01-01"
        } else {
            "2026-01-02"
        };
        let bucket = daily.entry(date.to_string()).or_insert_with(|| {
            let mut bucket = DailyBucket::new(date);
            let totals = DayTotals {
                input: 1_000_000,
                output: 100_000,
                cache_read: 4_000_000,
                messages: 6,
                cost: 1.5,
                ..DayTotals::default()
            };
            bucket.totals = totals;
            bucket.by_model.insert("gpt-5".to_string(), totals);
            bucket.by_client.insert("pi".into(), totals);
            bucket.by_client_model.insert(
                "pi".into(),
                [("gpt-5".to_string(), totals)].into_iter().collect(),
            );
            bucket
        });
        bucket.by_session.insert(
            format!("pi:{id}"),
            DayTotals {
                input: 100_000 + index as i64 * 10_000,
                output: 10_000,
                cache_read: 400_000,
                messages: 3,
                cost: 0.25,
                ..DayTotals::default()
            },
        );
    }
    (sessions, daily)
}

/// Default shape with 12 sessions (for alignment tests: 12 rows is under budget so card bottoms can be padded equal).
fn realistic_data() -> (Vec<SessionSnapshot>, crate::usage::DailyMap) {
    realistic_data_with(12)
}

/// Data that saturates all three tables' height budgets: 6 agents, 2 models, 16 sessions.
///
/// Each number serves one assertion:
///
/// * **6 agents** (more than the 5-row budget): the Agent table body caps at `BODY_MAX_HEIGHT`.
/// * **2 models** (fewer than the 5-row budget): the model table body would only be 2 rows tall. The
///   two row counts are intentionally unequal so that "lock height to the taller side" (user request:
///   the Agent card matches the Models card height) is genuinely exercised, not an empty assertion
///   that only holds when counts happen to be equal.
/// * **16 sessions** (more than the 15-row budget): the session table body caps at `SESSION_BODY_HEIGHT`,
///   so the two cards' height difference is exactly the two budgets' difference, unpolluted by `auto_shrink`.
fn saturated_data() -> (Vec<SessionSnapshot>, crate::usage::DailyMap) {
    saturated_data_with_prefix("")
}

/// Isomorphic to [`saturated_data`] but session ids get a prefix: repeated rounds stack, and keys stay
/// consistent with [`SessionSnapshot::key`] (renaming inside the key's client segment would desync the ledger).
fn saturated_data_with_prefix(prefix: &str) -> (Vec<SessionSnapshot>, crate::usage::DailyMap) {
    let agents = [
        ConfigFormat::Pi,
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
        ConfigFormat::ZCode,
        ConfigFormat::WorkBuddy,
    ];
    let models = ["gpt-5", "claude-opus-5"];
    let mut sessions = Vec::new();
    let mut daily: crate::usage::DailyMap = Default::default();
    for index in 0..16usize {
        let client = agents[index % agents.len()];
        let model = models[index % models.len()];
        let id = format!("{prefix}session-{index:02}");
        let date = if index % 2 == 0 {
            "2026-01-01"
        } else {
            "2026-01-02"
        };
        let totals = DayTotals {
            input: 100_000 + index as i64 * 10_000,
            output: 10_000,
            cache_read: 400_000,
            messages: 3,
            cost: 0.25,
            ..DayTotals::default()
        };
        let mut snapshot = session(&id, model, totals.input);
        snapshot.client = client;
        sessions.push(snapshot);

        let bucket = daily
            .entry(date.to_string())
            .or_insert_with(|| DailyBucket::new(date));
        bucket.totals.add_assign(&totals);
        bucket
            .by_client
            .entry(client.label().to_string())
            .or_default()
            .add_assign(&totals);
        bucket
            .by_model
            .entry(model.to_string())
            .or_default()
            .add_assign(&totals);
        bucket
            .by_client_model
            .entry(client.label().to_string())
            .or_default()
            .entry(model.to_string())
            .or_default()
            .add_assign(&totals);
        bucket
            .by_session
            .insert(SessionSnapshot::key(client, &id), totals);
    }
    (sessions, daily)
}

/// Find the card rect containing a given section title.
fn card_of(cards: &[egui::Rect], texts: &[TextAt], label: &str) -> egui::Rect {
    let heading = heading_of(texts, label, label);
    cards
        .iter()
        .find(|rect| rect.contains(heading.center()))
        .copied()
        .unwrap_or_else(|| panic!("应找到{label}卡矩形"))
}

/// The session card takes a full row: same left edge and width as the heatmap card (both fill content width), placed below it.
#[test]
fn session_card_spans_the_full_width() {
    let (sessions, daily) = realistic_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    let heatmap = card_of(&cards, &texts, "热力图");
    let agent = card_of(&cards, &texts, "Agent");
    let session = card_of(&cards, &texts, "会话");
    assert!(
        (session.left() - heatmap.left()).abs() < 1.0,
        "会话卡应与热力图卡左对齐：{:.1} vs {:.1}",
        session.left(),
        heatmap.left()
    );
    // Fills the row: width comparable to the heatmap card. Tolerance widened to 8px — the two cards' content
    // each size independently (heatmap cells vs table columns), never exactly equal.
    assert!(
        (session.width() - heatmap.width()).abs() < 8.0,
        "会话卡应占满整行宽度：{:.1} vs 热力图 {:.1}",
        session.width(),
        heatmap.width()
    );
    // Not half-width: clearly wider than the side-by-side Agent card, proving it shares a row with no one.
    assert!(
        session.width() > agent.width() * 1.8,
        "会话卡应远宽于半宽的 Agent 卡：{:.1} vs {:.1}",
        session.width(),
        agent.width()
    );
    assert!(
        session.top() >= heatmap.bottom() - 1.0,
        "会话卡应在热力图下面：{:.1} vs {:.1}",
        session.top(),
        heatmap.bottom()
    );
}

/// The Agent and Models cards each take half the width: same top, equal width, Agent on the left, no overlap.
#[test]
fn agent_and_model_cards_split_the_row() {
    let (sessions, daily) = realistic_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    let agent = card_of(&cards, &texts, "Agent");
    let model = card_of(&cards, &texts, "模型");
    assert!(
        (agent.top() - model.top()).abs() < 1.0,
        "两张卡应顶对齐：{:.1} vs {:.1}",
        agent.top(),
        model.top()
    );
    assert!(
        (agent.width() - model.width()).abs() < 1.0,
        "两张卡应等宽（各占一半）：{:.1} vs {:.1}",
        agent.width(),
        model.width()
    );
    assert!(agent.left() < model.left(), "Agent 卡应在模型卡左边");
    assert!(
        agent.right() <= model.left() + 1.0,
        "两张卡不该重叠：Agent 右 {:.1} vs 模型左 {:.1}",
        agent.right(),
        model.left()
    );
}

/// The Agent and Models cards are equal height: with unequal row counts, bottoms align to the taller side (user request).
///
/// The data has 6 agents vs 2 models: unequal rows — if each table body shrank to its own row count,
/// the cards would differ by three rows; locked, both bottoms land on the same line.
#[test]
fn agent_and_model_cards_have_the_same_height() {
    let (sessions, daily) = saturated_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    let agent = card_of(&cards, &texts, "Agent");
    let model = card_of(&cards, &texts, "模型");
    assert!(
        (agent.height() - model.height()).abs() < 1.0,
        "两张卡应等高：Agent {:.1} vs 模型 {:.1}",
        agent.height(),
        model.height()
    );
    assert!(
        (agent.bottom() - model.bottom()).abs() < 1.0,
        "两张卡底边应对齐：Agent {:.1} vs 模型 {:.1}",
        agent.bottom(),
        model.bottom()
    );
    // The lock genuinely works: the two tables' row counts differ (6 agents vs 2 models); unlocked, the
    // model card would stand three rows shorter. Equal height can only come from "locking to the taller side".
    assert!(
        agent.height() > 2.0 * crate::app::usage::ROW_PITCH,
        "两张卡都该拉到 5 行预算高，而不是只装自己的行数：{:.1}",
        agent.height()
    );
}

/// The session table's visible height is 3x the Agent / Models tables' (user request).
///
/// The data pushes all three tables past budget (6 agents / 16 sessions), so the cards' height
/// difference is exactly the budgets' difference (3x - 1x), with no row-count interference.
#[test]
fn session_card_body_is_three_times_taller() {
    let (sessions, daily) = saturated_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    let agent = card_of(&cards, &texts, "Agent");
    let session = card_of(&cards, &texts, "会话");
    let extra = session.height() - agent.height();
    // Everything besides the table bodies (card margins + section title + header) is identical, so the
    // difference is 3x budget − 1x budget = 2 × `BODY_MAX_HEIGHT`.
    let expected = 2.0 * 5.0 * crate::app::usage::ROW_PITCH;
    assert!(
        (extra - expected).abs() < 2.0,
        "会话卡应比普通卡高出两倍表体预算：{:.1} vs 预期 {:.1}",
        extra,
        expected
    );
}

/// The range-switch control hugs the status row's far right (user request) and sits above the page body (fixed, non-scrolling).
#[test]
fn range_chips_sit_at_the_right_of_the_status_row() {
    let (sessions, daily) = standard_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let chip_top = |label: &str| -> f32 {
        texts
            .iter()
            .filter(|t| t.text == label)
            .map(|t| t.rect.top())
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or_else(|| panic!("应渲染出区间按钮「{label}」"))
    };
    // All four range buttons on one line (a segmented control).
    let today = chip_top("今日");
    for label in ["本周", "本月", "全部"] {
        assert!(
            (chip_top(label) - today).abs() < 1.0,
            "四个区间按钮应在同一行（分段控件）：{label} {} vs 今日 {today}",
            chip_top(label)
        );
    }
    // Right-hugging: this button group belongs in the screen's right half, apart from the left filter text.
    let chip_left = texts
        .iter()
        .filter(|t| {
            matches!(t.text.as_str(), "今日" | "本周" | "本月" | "全部")
                && (t.rect.top() - today).abs() < 1.0
        })
        .map(|t| t.rect.left())
        .fold(f32::MAX, f32::min);
    assert!(
        chip_left > 700.0,
        "区间切换应贴右（1400 宽下靠右半区）：left={chip_left}"
    );
    // Fixed, non-scrolling: the status line sits above the overview title (the overview is the scroll area's first body element).
    let overview_top = texts
        .iter()
        .filter(|t| t.text == "用量趋势")
        .map(|t| t.rect.top())
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .expect("应渲染出趋势图标题");
    assert!(
        today < overview_top,
        "状态行（区间切换）应固定在滚动主体之上：{} vs 趋势图 {}",
        today,
        overview_top
    );
}

/// The session table has a "session name" column: names render when present, dashes when absent.
#[test]
fn session_table_shows_the_session_title() {
    let (mut sessions, daily) = standard_data();
    sessions[0].title = Some("重构删除链路".to_string());
    let out = render_output(1600.0, sessions, daily);
    let texts = texts_of(&out);
    assert!(
        texts.iter().any(|t| t.text == "会话名"),
        "会话表应有「会话名」列头"
    );
    assert!(
        texts.iter().any(|t| t.text == "重构删除链路"),
        "有名字的会话应把名字画出来"
    );
    assert!(
        texts.iter().any(|t| t.text == "—"),
        "没名字的会话应画「—」占位，不留空白"
    );
}

/// The status row must actually claim its height. It draws **outside** the scroll area; if only
/// `min_rect` grows without advancing the placer cursor, the following `ScrollArea` starts painting at
/// the status row's top and the first column gets covered by the overview card (reproduced from a
/// live screenshot: the archived hint overlapped the big numbers).
///
/// Assertion: compare "bottom of the status row's first text line" with "top of the first content
/// card" — if the cursor didn't advance, the card top lands below the status row text.
#[test]
fn status_row_reserves_its_height_above_the_scrolled_body() {
    let (sessions, daily) = saturated_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);

    // The status line's first text is the filter label ("all agents …"), at the very top of the page.
    let status_line = texts
        .iter()
        .find(|t| t.text.starts_with("全部 agent"))
        .expect("状态行应渲染出筛选标签");
    let status_bottom = status_line.rect.bottom();

    // Component card shells are 1px-bordered rounded rects; the range switch's shell is narrow (~244),
    // excluded by width so only genuine content cards remain.
    let first_card_top = card_rects_of(&out)
        .iter()
        .filter(|r| r.width() > 400.0)
        .map(|r| r.top())
        .fold(f32::MAX, f32::min);

    assert!(
        first_card_top >= status_bottom,
        "首栏被滚动内容盖住：首张内容卡顶 {first_card_top} 落在状态行首行下沿 {status_bottom} 之上"
    );
}

/// All text of the overview card's legend (input / output / cache read / cache write) must sit on one horizontal line.
///
/// Historical defect: prefixes went through `label` (vertically centered in the line) while values
/// were painted by per-digit rolling (flush to the row top), differing by half a line height (measured
/// 5.5px). After the fix the whole legend goes through a single `rolling_number` draw: non-digits
/// stay static, digits roll, all characters share the same slot top.
#[test]
fn composition_legend_text_shares_one_baseline() {
    let (sessions, daily) = saturated_data();
    let out = render_output(1600.0, sessions, daily);
    let texts = texts_of(&out);
    let card = card_rects_of(&out)
        .iter()
        .filter(|r| r.width() > 400.0)
        .min_by(|a, b| a.top().partial_cmp(&b.top()).unwrap())
        .copied()
        .expect("应渲染出总览卡");
    // The overview card also holds the trend chart's axis annotations (same small font, positioned lower):
    // take only the topmost line — the legend row. Under the old defect (centered prefixes, top-painted
    // values) that line held only digits, so the "same line must also carry a Chinese prefix" assertion failed.
    let captions: Vec<&TextAt> = texts
        .iter()
        .filter(|t| (t.size - crate::theme::TEXT_CAPTION).abs() < 0.01)
        .filter(|t| card.contains(t.rect.center()))
        .collect();
    let min_y = captions
        .iter()
        .map(|t| t.rect.min.y)
        .fold(f32::MAX, |a, b| a.min(b));
    let row: Vec<&&TextAt> = captions
        .iter()
        .filter(|t| t.rect.min.y < min_y + 0.5)
        .collect();
    assert!(row.len() >= 8, "图例行文字太少：{}", row.len());
    assert!(
        row.iter().any(|t| t.text == "输"),
        "图例行缺汉字前缀「输」：前缀与数值不在同一水平线"
    );
    assert!(
        row.iter().any(|t| {
            t.text
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '.' || c == 'M' || c == 'B' || c == 'K')
        }),
        "图例行缺数值字符：前缀与数值不在同一水平线"
    );

    // Swatch and text share the line's center: the swatch goes through layout, vertically centered in the
    // line; text painted flush to the line top would stand half a line higher (user feedback: "not aligned
    // with the block on the left").
    let row_height = row.iter().map(|t| t.rect.height()).fold(0.0f32, f32::max);
    let text_center = min_y + row_height / 2.0;
    let mut swatches: Vec<egui::Rect> = Vec::new();
    fn collect_swatches(shape: &egui::Shape, card: egui::Rect, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(r) => {
                let rect = r.rect;
                if (rect.width() - 8.0).abs() < 0.5
                    && (rect.height() - 8.0).abs() < 0.5
                    && r.fill != egui::Color32::TRANSPARENT
                    && card.contains(rect.center())
                {
                    out.push(rect);
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect_swatches(s, card, out)),
            _ => {}
        }
    }
    for cs in &out.shapes {
        collect_swatches(&cs.shape, card, &mut swatches);
    }
    let row_swatches: Vec<&egui::Rect> = swatches
        .iter()
        .filter(|r| (r.center().y - text_center).abs() < 12.0)
        .collect();
    assert!(
        row_swatches.len() >= 4,
        "图例行附近应至少有 4 个色块，实际 {}",
        row_swatches.len()
    );
    for swatch in &row_swatches {
        assert!(
            (swatch.center().y - text_center).abs() < 1.0,
            "色块中心 {:.1} 与文字行中心 {text_center:.1} 未对齐",
            swatch.center().y
        );
    }
}

/// When the session table's content overflows, hovering the body should paint an internal scrollbar (track + handle).
///
/// egui's floating scrollbar is fully transparent when idle (`dormant_handle_opacity = 0`) and fades
/// in on hover; the fade relies on per-frame animation advancing, and starved repaints (the historical
/// defect where [`super::super::usage`]'s idle polling branch requested no frames) would leave the
/// scrollbar stuck at transparent — perceived by users as "the session table's internal scrollbar
/// disappeared".
#[test]
fn session_table_shows_inner_scrollbar_on_hover() {
    // Three rounds of saturated_data (session ids prefixed for uniqueness): 48 content rows far exceed the
    // 375 viewport — guaranteed overflow.
    let mut sessions = Vec::new();
    let mut daily: crate::usage::DailyMap = Default::default();
    for prefix in ["", "r1-", "r2-"] {
        let (s, d) = saturated_data_with_prefix(prefix);
        sessions.extend(s);
        for (date, bucket) in d {
            let entry = daily
                .entry(date.clone())
                .or_insert_with(|| DailyBucket::new(&date));
            entry.totals.add_assign(&bucket.totals);
            for (k, v) in bucket.by_client {
                entry.by_client.entry(k).or_default().add_assign(&v);
            }
            for (k, v) in bucket.by_model {
                entry.by_model.entry(k).or_default().add_assign(&v);
            }
            for (k, models) in bucket.by_client_model {
                for (m, v) in models {
                    entry
                        .by_client_model
                        .entry(k.clone())
                        .or_default()
                        .entry(m)
                        .or_default()
                        .add_assign(&v);
                }
            }
            for (k, v) in bucket.by_session {
                entry.by_session.entry(k).or_default().add_assign(&v);
            }
        }
    }
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        usage_range: crate::app::usage::range::Range::All,
        usage_chart_range: crate::app::usage::range::ChartRange::All,
        guide_dismissed: true,
        ..App::default()
    };
    app.set_usage_result_for_test(sessions, daily);
    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1600.0));
    fn count_bars(out: &egui::FullOutput) -> usize {
        fn collect(shape: &egui::Shape, hits: &mut Vec<(f32, f32, f32, f32)>) {
            match shape {
                egui::Shape::Rect(r) => {
                    let rect = r.rect;
                    if rect.width() < 14.0
                        && rect.height() > 40.0
                        && r.fill != egui::Color32::TRANSPARENT
                    {
                        hits.push((rect.min.x, rect.min.y, rect.width(), rect.height()));
                    }
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, hits)),
                _ => {}
            }
        }
        let mut hits = Vec::new();
        for cs in &out.shapes {
            collect(&cs.shape, &mut hits);
        }
        hits.len()
    }
    let mut bars_seen = 0usize;
    for frame in 0..10 {
        // From frame 3, the pointer sits inside the session table body: the floating scrollbar's handle is fully
        // transparent at rest (dormant_handle_opacity=0), fading in on hover (active 0.6).
        let events = if frame >= 3 {
            vec![egui::Event::PointerMoved(egui::pos2(700.0, 1550.0))]
        } else {
            Vec::new()
        };
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                predicted_dt: 1.0 / 60.0,
                time: Some(0.05 * (frame as f64 + 1.0)),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.ui_usage_status(ui);
                    ui.add_space(crate::theme::SPACE_4);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            app.ui_usage_section(ui);
                        });
                });
            },
        );
        bars_seen = bars_seen.max(count_bars(&out));
    }
    assert!(
        bars_seen >= 2,
        "悬停多帧后会话表内部滚动条（轨道 + 手柄）应被画出，实际细高矩形 {bars_seen} 个"
    );
}
