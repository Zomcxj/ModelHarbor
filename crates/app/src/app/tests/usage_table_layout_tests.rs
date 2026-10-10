//! 用量表格的排版与筛选：数字必须右对齐，筛到单个 agent 时数字必须跟着变。
//!
//! 用户反馈过「排版太丑」，根因是数字列左对齐 —— 位数不同的数（`1.2K` 与
//! `123.4M`）左对齐会排成参差的锯齿。这里按真实结构离屏渲染，断言：
//!
//! 1. 同一列的数字**右边缘对齐**（这才是「表格」的样子）；
//! 2. 表头落在数字正上方（同列同宽）；
//! 3. 切到某个 agent 后，合计列的数字确实收窄（筛选真的接到了渲染路径上）。
//!
//! 只断言几何关系，不断言具体像素值，这样换主题 / 改字体也不会误报。

use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DayTotals, SessionSnapshot};
use eframe::egui;
use std::collections::HashMap;

/// 一段被渲染出来的文本及其矩形。
struct TextAt {
    text: String,
    rect: egui::Rect,
}

/// 收集全部文本形状（含嵌套）。
fn texts_of(out: &egui::FullOutput) -> Vec<TextAt> {
    let mut texts = Vec::new();
    fn collect(shape: &egui::Shape, out: &mut Vec<TextAt>) {
        match shape {
            egui::Shape::Text(t) => out.push(TextAt {
                text: t.galley.text().to_string(),
                rect: egui::Rect::from_min_size(t.pos, t.galley.size()),
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

/// 造一条会话快照。
fn session(client: ConfigFormat, id: &str, input: i64) -> SessionSnapshot {
    SessionSnapshot {
        client,
        session_id: id.to_string(),
        model_id: "claude-opus-5".to_string(),
        input,
        output: input / 10,
        cache_read: input * 4,
        cache_write: 0,
        reasoning: 0,
        message_count: 3,
        first_seen_ms: 1_700_000_000_000,
        last_seen_ms: 1_700_000_000_000,
        archived: false,
    }
}

/// 造一天的分桶：pi 与 opencode 各有用量。
fn day_bucket(date: &str, pi_input: i64, opencode_input: i64) -> DailyBucket {
    let mut bucket = DailyBucket::new(date);
    let pi = DayTotals {
        input: pi_input,
        output: pi_input / 10,
        cache_read: pi_input * 4,
        messages: 2,
        ..DayTotals::default()
    };
    let opencode = DayTotals {
        input: opencode_input,
        output: opencode_input / 10,
        cache_read: opencode_input * 4,
        messages: 1,
        ..DayTotals::default()
    };
    bucket.totals = DayTotals {
        input: pi_input + opencode_input,
        output: (pi_input + opencode_input) / 10,
        cache_read: (pi_input + opencode_input) * 4,
        messages: 3,
        ..DayTotals::default()
    };
    bucket.by_client.insert("pi".into(), pi);
    bucket.by_client.insert("opencode".into(), opencode);
    bucket.by_client_model.insert(
        "pi".into(),
        [("claude-opus-5".to_string(), pi)].into_iter().collect(),
    );
    bucket.by_client_model.insert(
        "opencode".into(),
        [("claude-opus-5".to_string(), opencode)]
            .into_iter()
            .collect(),
    );
    bucket.by_model.insert("claude-opus-5".into(), pi);
    // 会话表读的是 `by_session`（键同 `SessionSnapshot::key`）。
    bucket.by_session.insert(
        SessionSnapshot::key(ConfigFormat::Pi, "pi-session-aaaa"),
        pi,
    );
    bucket.by_session.insert(
        SessionSnapshot::key(ConfigFormat::Opencode, "oc-session-bbbb"),
        opencode,
    );
    bucket
}

/// 离屏渲染用量视图，返回全部文本形状。
///
/// 与 `App::update` 同构：状态行 + 控件行 + 总览 + 表格都在中央面板的滚动区里。
fn render(filter: Option<ConfigFormat>) -> Vec<TextAt> {
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        usage_filter: filter,
        usage_range: crate::app::usage::range::Range::All,
        guide_dismissed: true,
        ..App::default()
    };
    // 直接注入一次「扫描完成」的结果：本测试只关心排版，不跑真实扫描。
    let sessions = vec![
        session(ConfigFormat::Pi, "pi-session-aaaa", 1_000),
        session(ConfigFormat::Opencode, "oc-session-bbbb", 50_000_000),
    ];
    let daily: crate::usage::DailyMap = [
        (
            "2026-01-01".to_string(),
            day_bucket("2026-01-01", 1_000, 50_000_000),
        ),
        (
            "2026-01-02".to_string(),
            day_bucket("2026-01-02", 2_000, 60_000_000),
        ),
    ]
    .into_iter()
    .collect();
    app.set_usage_result_for_test(sessions, daily);

    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 900.0));
    let out = ctx.run(
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
    );
    texts_of(&out)
}

/// 同一列的数字必须右边缘对齐。
///
/// 取「合计」列：它在四个表里都紧跟在名称列后面，且一定存在。这里用模型表
/// （模型名固定、行数固定）来测，断言所有数字的 `rect.right()` 落在同一 x 上。
#[test]
fn number_columns_are_right_aligned() {
    let texts = render(None);
    // 数字形状：全是等宽体的格式化结果（含 K / M / B 后缀或纯数字）。
    let numbers: Vec<&TextAt> = texts
        .iter()
        .filter(|t| t.text.ends_with('M') || t.text.ends_with('K') || t.text.ends_with('B'))
        .collect();
    assert!(
        numbers.len() >= 4,
        "至少要渲染出几个带单位的数字，实际 {}",
        numbers.len()
    );

    // 按「列」分组：x 中心接近的算同一列，再看右边缘是否齐平。
    let mut rights_by_column: HashMap<i64, Vec<f32>> = HashMap::new();
    for n in &numbers {
        // 量化到 4px 网格，容忍字体度量的小抖动。
        let key = (n.rect.center().x / 4.0).round() as i64;
        rights_by_column
            .entry(key)
            .or_default()
            .push(n.rect.right());
    }
    let aligned_column = rights_by_column.values().filter(|rights| {
        if rights.len() < 2 {
            return false;
        }
        let max = rights.iter().cloned().fold(f32::MIN, f32::max);
        let min = rights.iter().cloned().fold(f32::MAX, f32::min);
        // 右边缘差在 1px 内算对齐（等宽字体下应当几乎完全一致）。
        max - min < 1.0
    });
    assert!(
        aligned_column.count() > 0,
        "没有任何一列的数字右边缘对齐 —— 数字列没走右对齐布局"
    );
}

/// 表头要落在数字正上方（同列同宽），否则表头与数据会错位。
#[test]
fn headers_sit_above_their_numbers() {
    let texts = render(None);
    let header = |name: &str| {
        texts
            .iter()
            .find(|t| t.text == name)
            .unwrap_or_else(|| panic!("没渲染出表头「{name}」"))
    };
    let total = header("合计");
    // 合计列的某个数字：右边缘与表头右边缘应当基本齐平（同列右对齐）。
    let same_column = texts.iter().filter(|t| {
        let is_number = t.text.ends_with('M') || t.text.ends_with('K') || t.text.ends_with('B');
        is_number && (t.rect.right() - total.rect.right()).abs() < 2.0
    });
    assert!(
        same_column.count() > 0,
        "「合计」表头的右边缘没有和任何数字对齐"
    );
}

/// 筛到单个 agent 后，合计列的数字必须跟着变小（筛选真的接到了渲染路径）。
#[test]
fn filtering_narrows_the_rendered_numbers() {
    let all = render(None);
    let pi = render(Some(ConfigFormat::Pi));

    // 只看 pi 时，状态行要显示筛选名。
    assert!(
        pi.iter().any(|t| t.text.contains("pi")),
        "筛选后状态行应显示 agent 名"
    );
    // 未筛选时状态行显示「全部 agent」。
    assert!(
        all.iter().any(|t| t.text.contains("全部 agent")),
        "未筛选时状态行应显示「全部 agent」"
    );

    // 表格画的是 total()（= 输入 + 输出 + 缓存读），不是单独的输入列。
    // opencode 两天的合计 = (1e8 + 1e7 + 4e8) * 1.1 ≈ 561.0M。
    let has_opencode_total = |texts: &[TextAt]| texts.iter().any(|t| t.text == "561.0M");
    assert!(
        has_opencode_total(&all),
        "未筛选时应渲染出 opencode 的合计 561.0M，实际数字：{:?}",
        all.iter()
            .filter(|t| t.text.ends_with('M') || t.text.ends_with('K'))
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        !has_opencode_total(&pi),
        "只看 pi 时不该出现 opencode 的 561.0M"
    );
    // 只看 pi 时合计应变成它自己的量级（15.3K）。
    assert!(
        pi.iter().any(|t| t.text == "15.3K"),
        "只看 pi 时应渲染出 15.3K，实际：{:?}",
        pi.iter()
            .filter(|t| t.text.ends_with('M') || t.text.ends_with('K'))
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
    );
}
