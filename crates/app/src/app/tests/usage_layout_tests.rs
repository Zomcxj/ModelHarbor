//! 用量视图的两列布局与顶栏按钮位置。
//!
//! 两处用户反馈的约束：
//!
//! 1. **顶栏**：`[提供商管理] [用量]` 紧接 agent 页签右侧（两个按钮交换过
//!    位置），不是甩到最右边。
//! 2. **主体两列**：左列是总览（总览卡 / 趋势图 / 热力图），右列从上到下
//!    是时间 / Agent / 模型 / 会话四张表；会话表带「项目」与「操作」列。
//!
//! 只断言几何关系与元素存在，不断言具体像素值，换主题 / 改字体也不会误报。

use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DayTotals, SessionSnapshot};
use eframe::egui;

/// 一段被渲染出来的文本及其矩形与字号。
struct TextAt {
    text: String,
    rect: egui::Rect,
    /// 字号：小节标题是 `TEXT_TITLE`，表格列头是 `TEXT_CAPTION`。
    /// 「会话」「模型」既是小节标题又是列头，只能靠字号区分。
    size: f32,
}

/// 收集全部文本形状（含嵌套）。
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

/// 造一条会话快照（带项目）。
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

/// 造一天的分桶：指定模型与用量，并把当天的会话记进 `by_session`
/// （会话表的行来自 `session_rows_in_range`，它只认 `by_session`）。
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

/// 离屏渲染用量视图（默认数据），返回全部文本形状。
fn render(width: f32) -> Vec<TextAt> {
    let (sessions, daily) = standard_data();
    texts_of(&render_output(width, sessions, daily))
}

/// 测试默认数据：两个会话、两天的量。
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

/// 离屏渲染用量视图（自定义数据），返回完整输出（可取文本与卡片矩形）。
fn render_output(
    width: f32,
    sessions: Vec<SessionSnapshot>,
    daily: crate::usage::DailyMap,
) -> egui::FullOutput {
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        usage_range: crate::app::usage::range::Range::All,
        // 测试数据是写死的日期，落在「最近 30 天」之外：趋势图区间放开到全部。
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
                // 照搬 [`App::update`] 的用量分支：状态行（含区间
                // 切换）画在滚动区**之外**，固定在顶部不跟随滚动。
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

/// 找小节标题（`TEXT_TITLE` 字号）。
fn heading_of(texts: &[TextAt], label: &str, what: &str) -> egui::Rect {
    texts
        .iter()
        .find(|t| t.text == label && (t.size - crate::theme::TEXT_TITLE).abs() < 0.01)
        .unwrap_or_else(|| panic!("{what}没渲染出小节标题「{label}」"))
        .rect
}

/// 单列纵向流：总览在上，Agent 与模型左右并排，会话表独占一行在最下。
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

    // Agent 与模型左右并排：顶对齐，Agent 在左。
    assert!(
        (agent.top() - model.top()).abs() < 1.0,
        "Agent 与模型应顶对齐：{:.1} vs {:.1}",
        agent.top(),
        model.top()
    );
    assert!(agent.left() < model.left(), "Agent 应在模型左边");
    // 会话表在两者之下，独占一行。
    assert!(
        session.top() > agent.top(),
        "会话表应在 Agent / 模型下面：{:.1} vs {:.1}",
        session.top(),
        agent.top()
    );
    // 总览在表格之上，且与 Agent 卡同列（单列流：左边缘一致）。
    assert!(chart.top() < agent.top(), "总览应在表格上面");
    assert!(
        (chart.left() - agent.left()).abs() < 1.0,
        "单列流下总览与 Agent 卡应左对齐：{:.1} vs {:.1}",
        chart.left(),
        agent.left()
    );
}

/// 会话表带「项目」与「操作」列头，且数据行的项目名画出来了。
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
    // 每个会话行一个「删」入口。
    let deletes = texts.iter().filter(|t| t.text == "删").count();
    assert_eq!(deletes, 2, "两个会话应各有一个删除入口");
}

/// 窄窗口下 Agent 与模型仍左右并排、会话表仍独占一行。
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

/// 顶栏：`[提供商管理] [用量]` 紧接页签右侧 —— 顺序交换过，且不在最右边。
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

    // 交换后的顺序：提供商管理在前，用量在后。
    assert!(
        providers.rect.left() < usage.rect.left(),
        "「提供商管理」应在「用量」前面：{} vs {}",
        providers.rect.left(),
        usage.rect.left()
    );
    // 不在最右边：切换条留在页签旁的左半区。
    assert!(
        usage.rect.left() < screen.width() / 2.0,
        "「用量」按钮不该在最右边：x={}",
        usage.rect.left()
    );
}

/// Agent / 模型两张表的最后一列是「占比」。
#[test]
fn agent_and_model_tables_have_a_share_column() {
    let texts = render(1600.0);
    let shares: Vec<_> = texts.iter().filter(|t| t.text == "占比").collect();
    assert_eq!(shares.len(), 2, "两张表各一个占比列头");
}

/// 收集卡片矩形：1px 描边的圆角矩形（组件卡外壳）。
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

/// 真实形状的数据：`count` 个会话、每天有费用（成本留在数据层，界面不再展示）。
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

/// 12 个会话的默认形状（对齐测试用：12 行矮于预算，卡底能补齐对齐）。
fn realistic_data() -> (Vec<SessionSnapshot>, crate::usage::DailyMap) {
    realistic_data_with(12)
}

/// 撑满三张表高度预算的数据：6 个 agent、2 个模型、16 条会话。
///
/// 三个数字各为一个断言服务：
///
/// * **6 个 agent**（多于 5 行预算）：Agent 表体封顶到 `BODY_MAX_HEIGHT`。
/// * **2 个模型**（少于 5 行预算）：模型表体本应只有 2 行高。两张表
///   行数故意不等，才能验证「按较多一方锁定高度」（用户要求：Agent 卡与
///   模型卡等高）真的生效，而不是行数恰好相同时的空断言。
/// * **16 条会话**（多于 15 行预算）：会话表体封顶到 `SESSION_BODY_HEIGHT`，
///   两张卡的差值就是两份预算的差，不掺 `auto_shrink` 的收缩量。
fn saturated_data() -> (Vec<SessionSnapshot>, crate::usage::DailyMap) {
    saturated_data_with_prefix("")
}

/// 与 [`saturated_data`] 同构，但会话 id 加前缀：多轮数据可叠加且键仍与
/// [`SessionSnapshot::key`] 一致（改名发生在键的客户段会导致账本对不上）。
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

/// 找到包含某个小节标题的卡片矩形。
fn card_of(cards: &[egui::Rect], texts: &[TextAt], label: &str) -> egui::Rect {
    let heading = heading_of(texts, label, label);
    cards
        .iter()
        .find(|rect| rect.contains(heading.center()))
        .copied()
        .unwrap_or_else(|| panic!("应找到{label}卡矩形"))
}

/// 会话卡独占一行：与热力图卡同左、同宽（都占满内容宽度），且排在它下面。
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
    // 占满整行：宽度与热力图卡相当。容差放到 8px —— 两张卡的内容各自定宽
    // （热力图是格子、会话表是列），不会正好等宽。
    assert!(
        (session.width() - heatmap.width()).abs() < 8.0,
        "会话卡应占满整行宽度：{:.1} vs 热力图 {:.1}",
        session.width(),
        heatmap.width()
    );
    // 不是半宽：明显宽于并排的 Agent 卡，证明它没与谁共享一行。
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

/// Agent 与模型两张卡左右各占一半：同顶、等宽、Agent 在左、互不重叠。
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

/// Agent 卡与模型卡等高：两边行数不同时也按较多的一方对齐底边（用户要求）。
///
/// 数据造了 6 个 agent 与 2 个模型：行数不等，若表体各按自己的行数
/// 收缩，两张卡会差三行高；锁定后两卡底边落在同一水平线。
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
    // 锁定真的生效：两张表行数不同（6 个 agent vs 2 个模型），若不锁定，
    // 模型卡会比 Agent 卡矮三行高。等高只能由「按较多一方锁定高度」得出。
    assert!(
        agent.height() > 2.0 * crate::app::usage::ROW_PITCH,
        "两张卡都该拉到 5 行预算高，而不是只装自己的行数：{:.1}",
        agent.height()
    );
}

/// 会话表的可视高度是 Agent / 模型表的三倍（用户要求）。
///
/// 数据把三张表都撑过了预算（6 个 agent / 16 条会话），卡高差值就是两份
/// 预算的差（3 倍 - 1 倍），不掺行数影响。
#[test]
fn session_card_body_is_three_times_taller() {
    let (sessions, daily) = saturated_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    let agent = card_of(&cards, &texts, "Agent");
    let session = card_of(&cards, &texts, "会话");
    let extra = session.height() - agent.height();
    // 两卡除表体外的部分（卡边距 + 小节标题 + 表头）完全相同，差值就是
    // 3 倍预算 − 1 倍预算 = 2 × `BODY_MAX_HEIGHT`。
    let expected = 2.0 * 5.0 * crate::app::usage::ROW_PITCH;
    assert!(
        (extra - expected).abs() < 2.0,
        "会话卡应比普通卡高出两倍表体预算：{:.1} vs 预期 {:.1}",
        extra,
        expected
    );
}

/// 区间切换组件贴状态行最右侧（用户要求），且位于页面主体之上（固定不滚动）。
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
    // 四个区间按钮同一行（分段控件）。
    let today = chip_top("今日");
    for label in ["本周", "本月", "全部"] {
        assert!(
            (chip_top(label) - today).abs() < 1.0,
            "四个区间按钮应在同一行（分段控件）：{label} {} vs 今日 {today}",
            chip_top(label)
        );
    }
    // 贴右：这组按钮应在屏幕右半区，与左侧筛选文字拉开。
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
    // 固定不滚动：状态行在总览标题之上（总览是滚动区的第一个主体元素）。
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

/// 会话表带「会话名」列：有名字的画名字，没名字的画破折号。
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

/// 状态行必须真的占住自己的高度。它画在滚动区**之外**，若只扩
/// `min_rect` 而不推进 placer 游标，后面的 `ScrollArea` 会从状态行顶部
/// 起画，首栏就被概览卡盖住（线上截图复现过：归档提示压在大数字上）。
///
/// 断言用「状态行首行文字的下沿」对比「首张内容卡的顶」：游标没推进时
/// 卡片顶会跑到状态行文字下面。
#[test]
fn status_row_reserves_its_height_above_the_scrolled_body() {
    let (sessions, daily) = saturated_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);

    // 状态行首行是筛选标签（「全部 agent …」），位于页面最顶部。
    let status_line = texts
        .iter()
        .find(|t| t.text.starts_with("全部 agent"))
        .expect("状态行应渲染出筛选标签");
    let status_bottom = status_line.rect.bottom();

    // 组件卡外壳是 1px 描边圆角矩形；区间切换的外壳窄（约 244），
    // 用宽度把它排除掉，只留真正的内容卡。
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

/// 总览卡图例（输入 / 输出 / 缓存读 / 缓存写）的所有文字须在同一水平线上。
///
/// 历史缺陷：前缀走 `label`（行内垂直居中）、数值走逐位滚动（贴行顶画），
/// 两者相差半个行高（实测 5.5px）。修复后整条图例交给同一个
/// `rolling_number` 画：非数字字符静态、数字滚动，全部字符共用槽顶。
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
    // 总览卡里还有趋势图的坐标注记（同为小字号、位置更低）：只取最上一行
    // ——图例行。旧缺陷（前缀居中、数值贴行顶）下这一行只剩数值字符，
    // 「同行须同时有汉字前缀」的断言即失败。
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

    // 色块与文字行共中心：色块经布局分配、行内垂直居中，文字若贴行顶
    // 直画就会高出半个行高（用户反馈「没和左侧的方块对齐」）。
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

/// 会话表内容溢出时，指针悬停表体应画出内部滚动条（轨道 + 手柄）。
///
/// egui 浮动滚动条静止时手柄全透明（`dormant_handle_opacity = 0`），
/// 悬停才淡入；淡入依赖逐帧动画推进，重绘饿死（[`super::super::usage`]
/// 轮询空闲分支不要帧的历史缺陷）会让滚动条永远停在透明档——用户
/// 观感即「会话表内部滚动条消失」。
#[test]
fn session_table_shows_inner_scrollbar_on_hover() {
    // 三轮 saturated_data（会话 id 加前缀保证唯一）：48 行内容远超
    // 375 视口，必然溢出。
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
        // 第 3 帧起把指针放进会话表体内：浮动滚动条静止时手柄全透明
        // （dormant_handle_opacity=0），悬停才淡入（active 0.6）。
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
