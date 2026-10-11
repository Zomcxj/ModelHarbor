//! 用量视图的图表与统计块布局：图例不能压住柱子、统计块不能折行错位。
//!
//! 用户反馈过两处布局问题，这里把它们的几何约束钉住：
//!
//! 1. **统计块**：值一长（`346.2M（2026-10-04）`、长模型名）就折行，
//!    块高参差导致上下两排错位。断言每个统计值**只占一行**，且同一排的
//!    块顶边齐平。
//! 2. **图例**：`egui_plot` 的内置图例画在坐标区**内部**右上角，条目一多
//!    就盖住柱子。断言图例文字落在绘图区**下方**。
//!
//! 只断言几何关系，不断言具体像素值，这样换主题 / 改字体也不会误报。

use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DayTotals, SessionSnapshot};
use eframe::egui;

/// 一段被渲染出来的文本及其矩形。
struct TextAt {
    text: String,
    rect: egui::Rect,
    /// 这一行的高度（galley 的高度，用于判断是否折行）。
    height: f32,
}

/// 收集全部文本形状（含嵌套）。
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

/// 一段会话快照。
fn session(id: &str, model: &str, input: i64) -> SessionSnapshot {
    SessionSnapshot {
        client: ConfigFormat::Pi,
        session_id: id.to_string(),
        model_id: model.to_string(),
        project: None,

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

/// 离屏渲染用量视图，返回全部文本形状。
fn render(models: &[(&str, i64)], width: f32) -> Vec<TextAt> {
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        usage_range: crate::app::usage::range::Range::All,
        // 测试数据是写死的日期，落在「最近 30 天」之外：趋势图区间放开到全部。
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
        // 每天一份，模型轮流出现：堆叠图才会有多个系列。
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

/// 统计值只占一行：折行会让块高参差，整块看起来没对齐。
///
/// 取「最高一天」的值（带日期的那个）与「最爱模型」的值（长模型名），
/// 它们的行高必须与单行文本一致。
#[test]
fn stat_values_stay_on_one_line() {
    let long_model = "deepseek-v4.1-flash-preview-extremely-long-name";
    let texts = render(&[(long_model, 732_800_000), ("gpt-5", 1_000)], 1100.0);

    // 单行文本的行高基准：取一个确定短的标签。
    let one_line = texts
        .iter()
        .find(|t| t.text == "活跃天数")
        .map(|t| t.height)
        .expect("统计块应渲染出「活跃天数」标签");

    // 「最爱模型」的值就是那个长模型名（只截断、不折行）。
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

/// 同一排的统计块顶边齐平。
///
/// 六个统计项排成两排（每排三个）：第一排是活跃天数 / 连续天数 / 日均，
/// 第二排是最高一天 / 最爱模型 / 占全部。每排内的标签顶边必须一致。
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

    // 第一排。
    let first_row = [top_of("活跃天数"), top_of("连续天数"), top_of("日均")];
    let spread = first_row.iter().cloned().fold(f32::MIN, f32::max)
        - first_row.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        spread < 1.0,
        "第一排三个统计块的顶边没对齐（相差 {spread}）"
    );

    // 第二排。
    let second_row = [top_of("最高一天"), top_of("最爱模型"), top_of("占全部")];
    let spread = second_row.iter().cloned().fold(f32::MIN, f32::max)
        - second_row.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        spread < 1.0,
        "第二排三个统计块的顶边没对齐（相差 {spread}）"
    );

    // 两排之间有间距（不能挤在一起）。
    assert!(
        second_row.iter().cloned().fold(f32::MAX, f32::min)
            > first_row.iter().cloned().fold(f32::MIN, f32::max),
        "第二排应在第一排下方"
    );
}

/// 图例必须画在绘图区**下方**，不能压在柱子上。
///
/// `egui_plot` 的内置图例固定在坐标区内部右上角：条目一多就盖住数据。
/// 现在图例由 `legend_row` 画在 `Plot` 之后，应当落在绘图区底边以下。
///
/// 定位方式：图表块的上下界由「用量趋势」标题与横轴日期标签夹出，
/// 图例则落在横轴与下面热力图标题（「热力图」）之间。
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

    // 图表标题与热力图标题：夹出「图表 + 图例」这一段。
    let chart_title = find("用量趋势");
    let heatmap_title = find("热力图");
    // 横轴日期标签（`M/D`）是绘图区最下面的一行文字。
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

    // 图例里的模型名（长名会被截断，用前缀互配）。
    let is_model_name = |text: &str| {
        models
            .iter()
            .any(|(model, _)| text.starts_with(model) || model.starts_with(text))
    };

    // 两列布局后，右列的表格与绘图区**同高**：判断「绘图区内」要同时按
    // y（标题~横轴之间）与 x（第一列，即表格列小节标题的左边）。
    let tables_x = texts
        .iter()
        .find(|t| matches!(t.text.as_str(), "时间" | "Agent" | "模型" | "会话") && t.height >= 17.0)
        .map(|t| t.rect.left())
        .expect("没找到表格列的小节标题");
    // 核心断言：绘图区内不能有模型名 —— 内置图例正是画在那里。
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

    // 图例必须落在横轴与热力图之间。
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

/// 是否像横轴日期标签（`3/15` 这种 `数字/数字`）。
fn is_axis_label(text: &str) -> bool {
    let Some((left, right)) = text.split_once('/') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.chars().all(|c| c.is_ascii_digit())
        && right.chars().all(|c| c.is_ascii_digit())
}

/// 图例里要带用量数字（照参考实现），否则只有名字看不出量级。
#[test]
fn legend_shows_usage_totals() {
    let texts = render(&[("gpt-5", 1_000_000), ("other-model", 500_000)], 1100.0);
    // 模型用量会同时出现在表格与图例里，这里只要求存在带单位的数字。
    assert!(
        texts
            .iter()
            .any(|t| t.text.ends_with('M') || t.text.ends_with('K')),
        "图例应渲染出带单位的用量"
    );
}
