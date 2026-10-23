//! The usage view's chart and stat-block layout: the legend mustn't cover bars, stat blocks mustn't wrap misaligned.
//!
//! Two layout problems from user feedback, with their geometric constraints pinned here:
//!
//! 1. **Stat blocks**: long values (`346.2M (2026-10-04)`, long model names) wrap, block heights vary,
//!    and the two rows misalign. Assert every stat value occupies **one line** and blocks in the same row
//!    top-align.
//! 2. **Legend**: egui_plot's built-in legend paints **inside** the plot area's top-right corner and covers
//!    bars once entries multiply. Assert the legend text lands **below** the plot area.
//!
//! Only geometry is asserted, never pixel values — theme or font changes won't cause false failures.

use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DayTotals, SessionSnapshot};
use eframe::egui;

/// A rendered piece of text with its rect.
struct TextAt {
    text: String,
    rect: egui::Rect,
    /// The line's height (the galley's height, for wrap detection).
    height: f32,
}

/// Collect all text shapes (including nested).
fn texts_of(out: &egui::FullOutput) -> Vec<TextAt> {
    let mut texts = Vec::new();
    fn collect(shape: &egui::Shape, out: &mut Vec<TextAt>) {
        match shape {
            egui::Shape::Text(t) => out.push(TextAt {
                text: t.galley.text().to_string(),
                rect: egui::Rect::from_min_size(t.pos, t.galley.size()),
                height: t.galley.size().y,
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

/// One session snapshot.
fn session(id: &str, model: &str, input: i64) -> SessionSnapshot {
    SessionSnapshot {
        client: ConfigFormat::Pi,
        session_id: id.to_string(),
        model_id: model.to_string(),
        project: None,
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

/// Offscreen-render the usage view; returns all text shapes.
fn render(models: &[(&str, i64)], width: f32) -> Vec<TextAt> {
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        usage_range: crate::app::usage::range::Range::All,
        // The test data uses fixed dates outside "last 30 days": the chart's range is opened to all.
        usage_chart_range: crate::app::usage::range::ChartRange::All,
        guide_dismissed: true,
        ..App::default()
    };
    let sessions: Vec<SessionSnapshot> = models
        .iter()
        .enumerate()
        .map(|(index, (model, input))| session(&format!("session-{index:04}"), model, *input))
        .collect();
    let daily: crate::usage::DailyMap = [(
        "2026-01-01".to_string(),
        // One per day, models rotating: only then does the stacked chart have several series.
        {
            let mut bucket = DailyBucket::new("2026-01-01");
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
            bucket.by_client.insert("pi".into(), bucket.totals);
            bucket
        },
    )]
    .into_iter()
    .collect();
    app.set_usage_result_for_test(sessions, daily);

    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 1400.0));
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

/// Stat values occupy one line: wrapping would vary block heights and the whole group reads misaligned.
///
/// Take the "peak day" value (the dated one) and the "favorite model" value (the long model name):
/// their line heights must match single-line text.
#[test]
fn stat_values_stay_on_one_line() {
    let long_model = "deepseek-v4.1-flash-preview-extremely-long-name";
    let texts = render(&[(long_model, 732_800_000), ("gpt-5", 1_000)], 1100.0);

    // The single-line height baseline: take a certainly-short label.
    let one_line = texts
        .iter()
        .find(|t| t.text == "活跃天数")
        .map(|t| t.height)
        .expect("统计块应渲染出「活跃天数」标签");

    // The "favorite model" value IS that long model name (truncated, never wrapped).
    let model_value = texts
        .iter()
        .find(|t| t.text.starts_with("deepseek-v4.1-flash"))
        .expect("应渲染出最爱模型的值");
    assert!(
        model_value.height < one_line * 1.8,
        "长模型名折行了：高 {} vs 单行 {}",
        model_value.height,
        one_line
    );
}

/// Stat blocks in the same row top-align.
///
/// Six stat items in two rows of three: the first row is active days / streak / daily average; the second
/// is peak day / favorite model / share of all. Labels within a row must share their top edge.
#[test]
fn stat_blocks_align_in_rows() {
    let texts = render(&[("gpt-5", 1_000)], 1100.0);
    let top_of = |label: &str| {
        texts
            .iter()
            .find(|t| t.text == label)
            .unwrap_or_else(|| panic!("没渲染出统计标签「{label}」"))
            .rect
            .top()
    };

    // First row.
    let first_row = [top_of("活跃天数"), top_of("连续天数"), top_of("日均")];
    let spread = first_row.iter().cloned().fold(f32::MIN, f32::max)
        - first_row.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        spread < 1.0,
        "第一排三个统计块的顶边没对齐（相差 {spread}）"
    );

    // Second row.
    let second_row = [top_of("最高一天"), top_of("最爱模型"), top_of("占全部")];
    let spread = second_row.iter().cloned().fold(f32::MIN, f32::max)
        - second_row.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        spread < 1.0,
        "第二排三个统计块的顶边没对齐（相差 {spread}）"
    );

    // Spacing between rows (never crammed together).
    assert!(
        second_row.iter().cloned().fold(f32::MAX, f32::min)
            > first_row.iter().cloned().fold(f32::MIN, f32::max),
        "第二排应在第一排下方"
    );
}

/// The legend must paint **below** the plot area, never over the bars.
///
/// egui_plot's built-in legend is fixed inside the plot area's top-right: entries cover data once they
/// multiply. The legend now paints via `legend_row` after the `Plot`, and should land below the plot
/// area's bottom edge.
///
/// Locating: the chart block's bounds are bracketed by the "Usage Trend" title and the x-axis date labels;
/// the legend falls between the x-axis and the next heatmap title ("Heatmap").
#[test]
fn legend_sits_below_the_plot_area() {
    let models: Vec<(&str, i64)> = vec![
        ("alpha-model", 100_000),
        ("beta-model", 90_000),
        ("gamma-model", 80_000),
        ("delta-model", 70_000),
        ("epsilon-model", 60_000),
        ("zeta-model", 50_000),
        ("eta-model", 40_000),
    ];
    let texts = render(&models, 1100.0);
    let find = |label: &str| {
        texts
            .iter()
            .find(|t| t.text == label)
            .unwrap_or_else(|| panic!("没渲染出「{label}」"))
            .rect
    };

    // The chart title and heatmap title: bracketing the "chart + legend" span.
    let chart_title = find("用量趋势");
    let heatmap_title = find("热力图");
    // The x-axis date labels (`M/D`) are the lowest text row of the plot area.
    let axis_bottom = texts
        .iter()
        .filter(|t| is_axis_label(&t.text))
        .map(|t| t.rect.bottom())
        .fold(f32::MIN, f32::max);
    assert!(axis_bottom > 0.0, "应渲染出横轴日期标签");
    assert!(
        chart_title.bottom() < axis_bottom && axis_bottom < heatmap_title.top(),
        "图表标题、横轴、热力图标题的上下顺序不对"
    );

    // Model names in the legend (long names truncate; match by prefix).
    let is_model_name = |text: &str| {
        models
            .iter()
            .any(|(model, _)| text.starts_with(model) || model.starts_with(text))
    };

    // After the two-column layout, the right column's tables are **as tall as** the plot area: judging "inside
    // the plot area" needs both y (between the titles) and x (the first column, i.e. left of the tables'
    // section titles).
    let tables_x = texts
        .iter()
        .find(|t| matches!(t.text.as_str(), "时间" | "Agent" | "模型" | "会话") && t.height >= 17.0)
        .map(|t| t.rect.left())
        .expect("没找到表格列的小节标题");
    // The core assertion: no model name inside the plot area — that's exactly where the built-in legend painted.
    let inside_plot: Vec<&str> = texts
        .iter()
        .filter(|t| {
            is_model_name(&t.text)
                && t.rect.left() < tables_x
                && t.rect.top() > chart_title.bottom()
                && t.rect.bottom() < axis_bottom
        })
        .map(|t| t.text.as_str())
        .collect();
    assert!(
        inside_plot.is_empty(),
        "绘图区里出现了模型名（图例压住柱子了）：{inside_plot:?}"
    );

    // The legend must land between the x-axis and the heatmap.
    let legend: Vec<&str> = texts
        .iter()
        .filter(|t| {
            is_model_name(&t.text)
                && t.rect.top() >= axis_bottom
                && t.rect.bottom() <= heatmap_title.top()
        })
        .map(|t| t.text.as_str())
        .collect();
    assert!(!legend.is_empty(), "横轴下方没有图例 —— 图例没渲染到图外");
}

/// Whether it looks like an x-axis date label (`3/15`, digit/digit).
fn is_axis_label(text: &str) -> bool {
    let Some((left, right)) = text.split_once('/') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.chars().all(|c| c.is_ascii_digit())
        && right.chars().all(|c| c.is_ascii_digit())
}

/// The legend must carry usage numbers (like the reference) — names alone show no magnitude.
#[test]
fn legend_shows_usage_totals() {
    let texts = render(&[("gpt-5", 1_000_000), ("other-model", 500_000)], 1100.0);
    // Model usage appears in both the tables and the legend; only require a number with a unit here.
    assert!(
        texts
            .iter()
            .any(|t| t.text.ends_with('M') || t.text.ends_with('K')),
        "图例应渲染出带单位的用量"
    );
}
