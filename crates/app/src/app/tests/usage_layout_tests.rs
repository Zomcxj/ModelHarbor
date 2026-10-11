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

/// 第二列三张表（Agent / 模型 / 会话）对齐同列、按序从上到下。
///
/// 时间模块已删（热力图承担按天视角），不再断言它。
#[test]
fn tables_are_laid_out_in_two_columns() {
    let texts = render(1600.0);
    let agent = heading_of(&texts, "Agent", "宽窗口");
    let model = heading_of(&texts, "模型", "宽窗口");
    let session = heading_of(&texts, "会话", "宽窗口");

    // 第二列三张表同一 x，从上到下按 Agent / 模型 / 会话。
    let xs = [agent.left(), model.left(), session.left()];
    let spread =
        xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min);
    assert!(spread < 1.0, "第二列三张表没对齐（x 相差 {spread}）");
    assert!(agent.top() < model.top(), "Agent 表应在模型表上面");
    assert!(model.top() < session.top(), "模型表应在会话表上面");

    // 第二列在总览（趋势图标题）右边。
    let chart = texts
        .iter()
        .find(|t| t.text == "用量趋势")
        .expect("没渲染出趋势图标题")
        .rect;
    assert!(
        agent.left() > chart.left(),
        "第二列应在总览右边：{} vs {}",
        agent.left(),
        chart.left()
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

/// 窄窗口下第二列三张表仍对齐同列，不与总览重叠。
#[test]
fn two_columns_fit_a_narrow_window() {
    let texts = render(900.0);
    let agent = heading_of(&texts, "Agent", "窄窗口");
    let model = heading_of(&texts, "模型", "窄窗口");
    let session = heading_of(&texts, "会话", "窄窗口");
    let chart = texts
        .iter()
        .find(|t| t.text == "用量趋势")
        .expect("窄窗口下没渲染出趋势图标题")
        .rect;
    // 第二列三张表对齐同一列，且在总览右边。
    let spread = (agent.left() - model.left())
        .abs()
        .max((agent.left() - session.left()).abs());
    assert!(spread < 1.0, "窄窗口下三张表没对齐（x 相差 {spread}）");
    assert!(
        agent.left() > chart.left(),
        "窄窗口下第二列仍应在总览右边：{} vs {}",
        agent.left(),
        chart.left()
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

/// 会话卡底边应与热力图卡底边对齐（用户反馈：没对齐）。
#[test]
fn session_card_bottom_aligns_with_heatmap_card_bottom() {
    let (sessions, daily) = realistic_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    let heatmap_card = cards
        .iter()
        .find(|r| r.contains(heading_of(&texts, "热力图", "热力图").center()))
        .copied()
        .expect("应找到热力图卡矩形");
    let session_card = cards
        .iter()
        .find(|r| r.contains(heading_of(&texts, "会话", "会话").center()))
        .copied()
        .expect("应找到会话卡矩形");
    assert!(
        (heatmap_card.bottom() - session_card.bottom()).abs() < 1.0,
        "会话卡底边应与热力图卡底边对齐：热力图 {:.1} vs 会话 {:.1}",
        heatmap_card.bottom(),
        session_card.bottom()
    );
    // 会话没显示全（用户反馈①）：整行裁切不许发生 —— 十二个会话在预算内
    // 应全部渲染出来。
    for index in 0..12 {
        let id = format!("session-{index:02}");
        assert!(
            texts.iter().any(|t| t.text == id),
            "会话 {id} 应完整可见（未裁切）"
        );
    }
}

/// 行数远超左列高度时（用户反馈：底要对齐）：卡底仍与热力图平齐，
/// 装不下的行在表体内滚动，预算内的前若干行完整可见。
#[test]
fn session_card_stays_aligned_with_many_rows() {
    let (sessions, daily) = realistic_data_with(30);
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    // 对齐预算装得下的前十几行必须完整可见。
    for index in 0..15 {
        let id = format!("session-{index:02}");
        assert!(
            texts.iter().any(|t| t.text == id),
            "会话 {id} 应渲染出来（预算内要铺满可见）"
        );
    }
    let heatmap_card = cards
        .iter()
        .find(|r| r.contains(heading_of(&texts, "热力图", "热力图").center()))
        .copied()
        .expect("应找到热力图卡矩形");
    let session_card = cards
        .iter()
        .find(|r| r.contains(heading_of(&texts, "会话", "会话").center()))
        .copied()
        .expect("应找到会话卡矩形");
    assert!(
        (session_card.bottom() - heatmap_card.bottom()).abs() < 1.0,
        "30 行时卡底仍应与热力图平齐：会话 {:.1} vs 热力图 {:.1}",
        session_card.bottom(),
        heatmap_card.bottom()
    );
}

/// 区间按钮横跨窗口单独一行（用户反馈：今日 / 本周 / 本月 / 全部单一行），
/// 双栏从同一 y 开始 —— Agent 卡与左列总览卡顶部平齐（用户反馈）。
#[test]
fn range_chips_single_row_and_agent_top_aligns_with_summary() {
    let (sessions, daily) = standard_data();
    let out = render_output(1400.0, sessions, daily);
    let texts = texts_of(&out);
    let cards = card_rects_of(&out);
    // 四个区间按钮在同一行（容忍 1px）；「今日」可能也出现在表格里，
    // 所以只要存在一个 y 让四个按钮都在附近即可。
    let ys = |label: &str| -> Vec<f32> {
        texts
            .iter()
            .filter(|t| t.text == label)
            .map(|t| t.rect.top())
            .collect()
    };
    let today_ys = ys("今日");
    assert!(!today_ys.is_empty(), "应渲染出「今日」按钮");
    let same_row = today_ys.iter().any(|&y| {
        ["本周", "本月", "全部"]
            .iter()
            .all(|label| ys(label).iter().any(|&other| (other - y).abs() < 1.0))
    });
    assert!(same_row, "四个区间按钮应在同一行");
    // Agent 卡与总览卡顶部平齐（总览卡没有标题，用「连续天数」统计项定位）。
    let stat_text = texts
        .iter()
        .find(|t| t.text == "连续天数")
        .expect("总览卡应有统计项");
    let summary_card = cards
        .iter()
        .find(|r| r.contains(stat_text.rect.center()))
        .copied()
        .expect("应找到总览卡矩形");
    let agent_card = cards
        .iter()
        .find(|r| r.contains(heading_of(&texts, "Agent", "Agent 表").center()))
        .copied()
        .expect("应找到 Agent 卡矩形");
    assert!(
        (summary_card.top() - agent_card.top()).abs() < 1.0,
        "Agent 卡顶应与总览卡顶平齐：总览 {:.1} vs Agent {:.1}",
        summary_card.top(),
        agent_card.top()
    );
}
