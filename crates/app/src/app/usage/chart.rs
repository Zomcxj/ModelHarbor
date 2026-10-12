//! 用量趋势图：按天柱状（按模型堆叠），可切换成曲线。
//!
//! 数据源是 `DailyBucket`（按本地日历日分桶），不是会话级汇总 —— 一个会话
//! 可以横跨几十天，按它的 `last_seen_ms` 归日会把整月的量堆到最后一天。
//!
//! 堆叠只取**用量前几名的模型**，其余并成「其他」：颜色数量有限，全画出来
//! 图例会挤成一团，而且长尾模型各自的量在图上看不出差别。堆叠是唯一画法
//! （用户要求去掉开关）：量花在哪个模型上一眼可见。
//!
//! 配色一律由当前主题的强调色按黄金角散开（见 [`series_color`]），不硬编码
//! 颜色 —— 八个主题（深色 / 亮色 / 海洋 / 极地 / 苔藓 / 薄荷 / 薰衣草 / 玫瑰）
//! 都要能看。

use super::aggregate;
use super::range::{self};
use crate::app::App;
use crate::usage::DailyBucket;
use eframe::egui;
use egui_plot::{Bar, BarChart, Line, Plot, PlotPoints};
use std::collections::BTreeMap;

/// 图表高度。
const PLOT_HEIGHT: f32 = 190.0;

/// 堆叠最多画几个模型，其余并成「其他」。
const MAX_SERIES: usize = 6;

/// 「其他」在图例与堆叠里的名字。
const OTHER_LABEL: &str = "其他";

/// 堆叠柱的间隙：柱宽略小于相邻日索引的距离，让相邻两天看得出分隔。
const BAR_GAP: f64 = 0.15;

/// 趋势图的画法：柱状（按模型堆叠）或曲线（每模型一条线）。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum ChartKind {
    /// 柱状图（默认）：堆叠柱，总量与构成一眼可见。
    #[default]
    Bars,
    /// 曲线图：每个模型一条线，看单模型的走势更清楚。
    Line,
}

impl ChartKind {
    /// 全部画法，按界面上的展示顺序（右对齐布局里倒着加）。
    pub(in crate::app) const ALL: [ChartKind; 2] = [ChartKind::Line, ChartKind::Bars];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Bars => "柱状",
            Self::Line => "曲线",
        }
    }
}

/// 柱宽：只画一天时给固定宽度，否则「1 天」的柱子会宽到填满整幅图。
fn bar_width(days: usize) -> f64 {
    if days <= 1 {
        0.6
    } else {
        1.0 - BAR_GAP
    }
}

/// 模型用量的一个系列（一个颜色一摞柱子 / 一条线）。
struct Series {
    /// 图例名（模型 id 或「其他」）。
    label: String,
    /// 颜色。
    color: egui::Color32,
    /// 日索引 → 该模型当天的合计（缺的日按 0）。
    by_day: BTreeMap<i64, i64>,
    /// 区间内合计，用于图例里的量。
    total: i64,
}

/// 系列颜色：见 [`super::series_color`]。
fn series_color(index: usize, accent: egui::Color32, dark: bool) -> egui::Color32 {
    super::series_color(index, accent, dark)
}

impl App {
    /// 趋势图：按天柱状（按模型堆叠），可切换曲线。
    ///
    /// 区间是图表自己的档位（7 天 / 30 天 / 90 天 / 一年 / 全部），不随全局
    /// 区间走：总览卡与表格看的是「今日 / 本周 / 本月」，趋势图看的是走势。
    /// 只在区间内有数据时画；没有数据时整块不占高度（总览卡已经给了空状态提示）。
    pub(in crate::app) fn ui_usage_chart(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let chart_range = self.usage_chart_range;
        let buckets = range::buckets_in_chart_range(&daily, chart_range, today);
        if buckets.is_empty() {
            return;
        }

        let kind = self.usage_chart_kind;
        // 柱状拆模型看构成；曲线只画**总量**一条线（用户要求：直接显示总量）。
        let series = match kind {
            ChartKind::Bars => {
                build_series(&buckets, self.theme.accent_color(), ui.visuals().dark_mode)
            }
            ChartKind::Line => build_total_series(&buckets, self.theme.accent_color()),
        };

        super::component::card(ui, |ui| {
            ui.horizontal(|ui| {
                super::component::heading(ui, "用量趋势");
                for option in range::ChartRange::ALL {
                    if super::chip(ui, chart_range == option, option.label(), 44.0) {
                        self.usage_chart_range = option;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 右对齐布局里倒着加：视觉顺序 = [柱状] [曲线]。
                    for option in ChartKind::ALL {
                        if super::chip(ui, kind == option, option.label(), 44.0) {
                            self.usage_chart_kind = option;
                        }
                    }
                });
            });
            ui.add_space(crate::theme::SPACE_2);

            let days: Vec<i64> = buckets.keys().copied().collect();
            // x 从 0 起数（区间首日 = 0）。egui_plot 的刻度取「步长的绝对
            // 倍数」，绝对日索引（~2 万）在步长变大后首个刻度会落在起始日
            // 之后，30 天以上的档位左端就没刻度了（用户反馈：左侧缺少尺度）。
            let first_day = days[0];
            let width = bar_width(days.len());
            let plot = Plot::new("usage_daily_plot")
                .height(PLOT_HEIGHT)
                .allow_zoom(false)
                .allow_scroll(false)
                .allow_drag(false)
                .allow_boxed_zoom(false)
                .allow_axis_zoom_drag(false)
                .allow_double_click_reset(false)
                .show_background(false)
                // 内部网格线把图切成格子，柱子已经能对位读数（用户反馈：去掉）。
                .show_grid(false)
                // 柱子贴边会被裁掉一半：留一点横向余量。4% 而不是 2%：
                // 末档的日期标签（如「10/12」）以刻度为中心画，一半宽度落在
                // 刻度右侧 —— 2% 余量盖不住，标签伸出卡片外（用户反馈）。
                .set_margin_fraction(egui::vec2(0.04, 0.08))
                // 悬停不再显示 x/y 坐标读数（用户要求：不要显示xy）。
                .show_x(false)
                .show_y(false)
                .x_axis_formatter(move |mark, _| {
                    day_axis_label(mark.value.round() as i64, first_day)
                })
                .y_axis_formatter(|mark, _| aggregate::format_m(mark.value as i64))
                // 刻度自己取好数（1/2/5×10^k，约 5 条）：默认按 10 的幂量化，
                // 区间刚跨过 10 的幂时细刻度间距不足全被跳过、粗刻度只剩 0
                // 在界内 —— 30 天档整段没尺度（用户反馈）。
                .x_grid_spacer(nice_grid_marks)
                .y_grid_spacer(nice_grid_marks);

            let plot_response = plot.show(ui, |plot_ui| match kind {
                ChartKind::Bars => {
                    // 累积高度按天维护、**跨系列**累加：堆叠要把每根柱子摞在
                    // 前面几摞之上，所以这个游标不能每个系列重置。
                    let mut cumulative = vec![0.0f64; days.len()];
                    for item in &series {
                        let mut bars: Vec<Bar> = Vec::with_capacity(days.len());
                        for (index, day) in days.iter().enumerate() {
                            let value = item.by_day.get(day).copied().unwrap_or(0) as f64;
                            let base = cumulative[index];
                            cumulative[index] += value;
                            bars.push(
                                Bar::new((*day - first_day) as f64, value)
                                    // 悬停提示要报出是哪个模型：名字挂在段上。
                                    .name(item.label.clone())
                                    .base_offset(base)
                                    .width(width)
                                    .fill(item.color),
                            );
                        }
                        // 量与模型名的对应由图下方的图例与自绘悬停提示给出
                        //（egui_plot 在 show_x/show_y 都关时不会弹内置标签）。
                        plot_ui.bar_chart(BarChart::new(item.label.clone(), bars).vertical());
                    }
                }
                ChartKind::Line => {
                    // 一条总量曲线：日与日之间用样条过渡，锯齿读不出趋势。
                    let item = &series[0];
                    let points: Vec<[f64; 2]> = days
                        .iter()
                        .map(|day| {
                            [
                                (*day - first_day) as f64,
                                item.by_day.get(day).copied().unwrap_or(0) as f64,
                            ]
                        })
                        .collect();
                    plot_ui.line(
                        Line::new(item.label.clone(), PlotPoints::from(smooth_curve(&points)))
                            .color(item.color)
                            .width(2.0),
                    );
                }
            });

            // 悬停读数（用户反馈：要看到当天各模型用量，且提示要**跟着鼠标**）：
            // egui_plot 在 show_x/show_y 都关时内置悬停机制直接返回；而 egui 的
            // tooltip 会钉在首次出现的位置不动。所以自绘：每帧重算日索引，
            // 浮层固定在指针旁 —— 指针在图右半边时盒子往左翻，不探出窗口右缘。
            if plot_response.response.hovered() {
                if let Some(pointer) = ui.ctx().pointer_hover_pos() {
                    let offset = plot_response.transform.value_from_position(pointer).x;
                    let day = first_day + offset.round() as i64;
                    let rows: Vec<&Series> = series
                        .iter()
                        .filter(|item| item.by_day.get(&day).copied().unwrap_or(0) > 0)
                        .collect();
                    if !rows.is_empty() {
                        let on_right = pointer.x > plot_response.response.rect.center().x;
                        egui::Area::new(egui::Id::new("usage_chart_tooltip"))
                            .order(egui::Order::Tooltip)
                            .fixed_pos(pointer + egui::vec2(12.0, 12.0))
                            .pivot(if on_right {
                                egui::Align2::RIGHT_TOP
                            } else {
                                egui::Align2::LEFT_TOP
                            })
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                    let (_, month, mday) = aggregate::civil_from_days(day);
                                    ui.label(
                                        egui::RichText::new(format!("{month}/{mday}"))
                                            .size(crate::theme::TEXT_CAPTION)
                                            .strong(),
                                    );
                                    for item in &rows {
                                        let value = item.by_day.get(&day).copied().unwrap_or(0);
                                        ui.horizontal(|ui| {
                                            let (swatch, _) = ui.allocate_exact_size(
                                                egui::vec2(8.0, 8.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(swatch, 2.0, item.color);
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{} {}",
                                                    item.label,
                                                    aggregate::format_m(value)
                                                ))
                                                .size(crate::theme::TEXT_CAPTION),
                                            );
                                        });
                                    }
                                    if rows.len() > 1 {
                                        let total: i64 = rows
                                            .iter()
                                            .map(|item| item.by_day.get(&day).copied().unwrap_or(0))
                                            .sum();
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "合计 {}",
                                                aggregate::format_m(total)
                                            ))
                                            .size(crate::theme::TEXT_CAPTION),
                                        );
                                    }
                                });
                            });
                    }
                }
            }

            // 图例画在图**外面**（下方）：`egui_plot` 的内置图例固定在坐标区
            // 内部右上角，条目一多就盖住柱子。画在外面还能顺便带上各系列的用量。
            ui.add_space(crate::theme::SPACE_2);
            // 曲线模式在图例最左（卡片左下角）带出单日峰值。
            let peak = (kind == ChartKind::Line)
                .then(|| series[0].by_day.values().copied().max().unwrap_or(0));
            legend_row(ui, &series, peak);
        });
    }
}

/// 图例：每项一个色块 + 模型名 + 用量，排不下就换行。
///
/// 名字用**比例字体**而不是等宽体：模型名可以很长，等宽体下占宽明显更宽，
/// 会过早折行；名字过长时省略号截断，全文走悬停。
fn legend_row(ui: &mut egui::Ui, series: &[Series], peak: Option<i64>) {
    let weak = ui.visuals().weak_text_color();
    ui.horizontal_wrapped(|ui| {
        if let Some(peak) = peak {
            ui.label(
                egui::RichText::new(format!("峰值 {}", aggregate::format_m(peak)))
                    .size(crate::theme::TEXT_CAPTION)
                    .color(weak),
            );
            ui.add_space(crate::theme::SPACE_2);
        }
        // 对齐 tokenmonitor 的仪表盘行：每条系列带占比（分母 = 图表区间
        // 内全部系列的总量）。
        let grand: i64 = series.iter().map(|item| item.total).sum();
        for item in series {
            let (swatch, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter().rect_filled(swatch, 2.0, item.color);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&item.label)
                        .size(crate::theme::TEXT_CAPTION)
                        .color(weak),
                )
                .truncate(),
            )
            .on_hover_text(&item.label);
            ui.label(
                egui::RichText::new(format!(
                    "{} · {}",
                    super::overview::percent_of(item.total, grand),
                    aggregate::format_m(item.total)
                ))
                .size(crate::theme::TEXT_CAPTION)
                .color(weak),
            );
            // 项间留白：色块紧跟上一个数字会读成「两个数字」。
            ui.add_space(crate::theme::SPACE_2);
        }
    });
}

/// 轴标签：日索引 → `M/D`。
///
/// 轴上不写年份（区间最长也就一年），也不写「今天 / 昨天」—— 刻度是等距的，
/// 夹一个「今天」进去会读不出节奏。
/// x 轴刻度文案：x 是「区间首日起的第几天」，加回首日还原成日期。
fn day_axis_label(offset_days: i64, first_day: i64) -> String {
    let (_, month, day) = aggregate::civil_from_days(first_day + offset_days);
    format!("{month}/{day}")
}

/// 网格刻度：自己取「好数」（1/2/5×10^k），约 5 条铺满量程。
///
/// 不看 `base_step_size`、只按 bounds 均分：任何档位都有 3 条以上刻度。
fn nice_grid_marks(input: egui_plot::GridInput) -> Vec<egui_plot::GridMark> {
    let (min, max) = input.bounds;
    let span = max - min;
    if span <= 0.0 {
        return Vec::new();
    }
    let raw = span / 5.0;
    let power = 10f64.powf(raw.abs().log10().floor());
    let step = match raw / power {
        m if m <= 1.0 => power,
        m if m <= 2.0 => 2.0 * power,
        m if m <= 5.0 => 5.0 * power,
        _ => 10.0 * power,
    };
    // 两个轴的数据都从 0 起（x 是首日偏移，y 是非负用量）：负半轴不出刻度。
    let first = (min / step).ceil().max(0.0);
    let last = (max / step).floor();
    (first as i64..=last as i64)
        .map(|k| egui_plot::GridMark {
            value: k as f64 * step,
            step_size: step,
        })
        .collect()
}

/// 曲线图的系列：只有一条「合计」，直接显示每天的总量。
fn build_total_series(buckets: &BTreeMap<i64, &DailyBucket>, accent: egui::Color32) -> Vec<Series> {
    let by_day = buckets
        .iter()
        .map(|(day, bucket)| (*day, bucket.totals.total()))
        .collect();
    let total = buckets.values().map(|b| b.totals.total()).sum();
    vec![Series {
        label: "合计".to_string(),
        color: accent,
        by_day,
        total,
    }]
}

/// Catmull-Rom 样条：折线的每个线段细分出平滑过渡（用户要求曲线过渡）。
///
/// x 保持线性推进、只对 y 做样条：日与日之间有缺天（间距不均），对 x 一起
/// 做样条可能回折。端点没有邻居时用自身补位，曲线在两端自然变直。
fn smooth_curve(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    const STEPS: usize = 8;
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let at = |i: i64| points[i.clamp(0, n as i64 - 1) as usize];
    // 单个坐标的 Catmull-Rom 插值。
    let cr = |a: f64, b: f64, c: f64, d: f64, t: f64| {
        let t2 = t * t;
        let t3 = t2 * t;
        0.5 * ((2.0 * b)
            + (-a + c) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
            + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    let mut out = Vec::with_capacity((n - 1) * STEPS + 1);
    for i in 0..n - 1 {
        let (p0, p1, p2, p3) = (
            at(i as i64 - 1),
            at(i as i64),
            at(i as i64 + 1),
            at(i as i64 + 2),
        );
        for step in 0..STEPS {
            let t = step as f64 / STEPS as f64;
            out.push([
                p1[0] + (p2[0] - p1[0]) * t,
                cr(p0[1], p1[1], p2[1], p3[1], t),
            ]);
        }
    }
    out.push(points[n - 1]);
    out
}

/// 把区间内的桶整理成系列：按模型拆开、取前 [`MAX_SERIES`] 名，其余并成「其他」。
///
/// 柱状与曲线共用同一份系列拆分，只有画法不同。
fn build_series(
    buckets: &BTreeMap<i64, &DailyBucket>,
    accent: egui::Color32,
    dark: bool,
) -> Vec<Series> {
    // 先按模型汇总区间内的总量，再排序取前几名。
    let mut totals_by_model: BTreeMap<String, i64> = BTreeMap::new();
    for bucket in buckets.values() {
        for (model, totals) in &bucket.by_model {
            *totals_by_model.entry(model.clone()).or_default() += totals.total();
        }
    }
    let mut ranked: Vec<(String, i64)> = totals_by_model.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top: Vec<String> = ranked
        .iter()
        .take(MAX_SERIES)
        .map(|(model, _)| model.clone())
        .collect();

    // 区间内无任何模型分解时返回空系列；空系列上 `series.len() - 1`
    // 会发生 usize 下溢。
    if top.is_empty() {
        return Vec::new();
    }
    let mut series: Vec<Series> = top
        .iter()
        .enumerate()
        .map(|(index, model)| Series {
            label: model.clone(),
            color: series_color(index, accent, dark),
            by_day: BTreeMap::new(),
            total: 0,
        })
        .collect();
    // 「其他」只在真的被截断时才出现。
    let has_other = ranked.len() > top.len();
    if has_other {
        series.push(Series {
            label: OTHER_LABEL.to_string(),
            color: series_color(top.len(), accent, dark),
            by_day: BTreeMap::new(),
            total: 0,
        });
    }
    let other_index = series.len() - 1;

    for (day, bucket) in buckets {
        for (model, totals) in &bucket.by_model {
            let value = totals.total();
            match top.iter().position(|name| name == model) {
                Some(index) => {
                    *series[index].by_day.entry(*day).or_default() += value;
                    series[index].total += value;
                }
                None => {
                    *series[other_index].by_day.entry(*day).or_default() += value;
                    series[other_index].total += value;
                }
            }
        }
    }
    series
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::DayTotals;

    fn bucket(date: &str, models: &[(&str, i64)]) -> DailyBucket {
        let mut bucket = DailyBucket::new(date);
        for (model, value) in models {
            let totals = DayTotals {
                input: *value,
                messages: 1,
                ..Default::default()
            };
            bucket.totals.add_assign(&totals);
            bucket.by_model.insert((*model).to_string(), totals);
        }
        bucket
    }

    /// 把 `(日索引, 桶)` 排成 `build_series` 要的形状。
    fn buckets_of(rows: &[(i64, DailyBucket)]) -> BTreeMap<i64, &DailyBucket> {
        rows.iter().map(|(day, b)| (*day, b)).collect()
    }

    #[test]
    fn series_split_by_model() {
        let data = [
            (0i64, bucket("1970-01-01", &[("a", 10), ("b", 5)])),
            (1, bucket("1970-01-02", &[("a", 20)])),
        ];
        let series = build_series(&buckets_of(&data), egui::Color32::RED, true);
        assert_eq!(series.len(), 2, "没有长尾就不该出现「其他」");
        // 按总量倒序：a 30 > b 5。
        assert_eq!(series[0].label, "a");
        assert_eq!(series[0].by_day[&0], 10);
        assert_eq!(series[0].by_day[&1], 20);
        assert_eq!(series[0].total, 30);
        assert_eq!(series[1].label, "b");
        assert_eq!(series[1].by_day[&0], 5);
        assert!(!series[1].by_day.contains_key(&1), "b 在第 2 天没有量");
    }

    #[test]
    fn folds_the_tail_into_other() {
        // 8 个模型 → 前 6 名各自成系列，剩下 2 个并成「其他」。
        let rows: Vec<(&str, i64)> = vec![
            ("m1", 100),
            ("m2", 90),
            ("m3", 80),
            ("m4", 70),
            ("m5", 60),
            ("m6", 50),
            ("m7", 40),
            ("m8", 30),
        ];
        let data = [(0i64, bucket("1970-01-01", &rows))];
        let series = build_series(&buckets_of(&data), egui::Color32::RED, true);
        assert_eq!(series.len(), MAX_SERIES + 1);
        assert_eq!(series[MAX_SERIES].label, OTHER_LABEL);
        assert_eq!(series[MAX_SERIES].total, 70, "40 + 30 并进其他");
        assert_eq!(series[0].label, "m1");
    }

    #[test]
    fn series_colors_are_distinct() {
        let accent = egui::Color32::from_rgb(80, 140, 220);
        let colors: Vec<egui::Color32> = (0..MAX_SERIES)
            .map(|index| series_color(index, accent, true))
            .collect();
        for (index, color) in colors.iter().enumerate() {
            assert!(
                !colors[..index].contains(color),
                "第 {index} 个系列的颜色与前面重复"
            );
        }
        // 第 0 个系列就是强调色本身（往返 HSVA 会有舍入误差，容 2 个色阶）。
        let near = |a: u8, b: u8| a.abs_diff(b) <= 2;
        assert!(
            near(colors[0].r(), accent.r())
                && near(colors[0].g(), accent.g())
                && near(colors[0].b(), accent.b()),
            "{colors:?} 的第一个系列应等于强调色 {accent:?}"
        );
    }

    #[test]
    fn bar_width_shrinks_to_a_single_day() {
        assert_eq!(bar_width(1), 0.6, "一天的柱子不能填满整幅图");
        assert!(bar_width(30) > bar_width(1));
    }

    #[test]
    fn axis_labels_are_month_and_day() {
        // x 是「首日起的第几天」：0 = 首日本身，左端永远有刻度。
        assert_eq!(day_axis_label(0, 0), "1/1");
        let first = aggregate::days_from_civil(2026, 3, 10);
        assert_eq!(day_axis_label(5, first), "3/15");
    }

    /// 图内数字统一 M 单位：10M 以下一位小数，以上取整。
    #[test]
    fn chart_numbers_are_all_in_millions() {
        assert_eq!(aggregate::format_m(0), "0.0M");
        assert_eq!(aggregate::format_m(500_000), "0.5M");
        assert_eq!(aggregate::format_m(1_234_567), "1.2M");
        assert_eq!(aggregate::format_m(12_345_678), "12M");
        assert_eq!(aggregate::format_m(2_030_000_000), "2030M");
    }

    #[test]
    fn line_series_is_the_daily_total() {
        let data = [
            (0i64, bucket("1970-01-01", &[("a", 10), ("b", 5)])),
            (1, bucket("1970-01-02", &[("a", 20)])),
        ];
        let series = build_total_series(&buckets_of(&data), egui::Color32::RED);
        assert_eq!(series.len(), 1, "曲线只有一条总量线");
        assert_eq!(series[0].label, "合计");
        assert_eq!(series[0].by_day[&0], 15);
        assert_eq!(series[0].total, 35);
    }

    #[test]
    fn smooth_curve_keeps_endpoints_and_monotonic_x() {
        let points = vec![[0.0, 0.0], [1.0, 5.0], [3.0, 2.0], [4.0, 9.0]];
        let out = smooth_curve(&points);
        assert_eq!(out.first().copied(), Some([0.0, 0.0]), "起点不动");
        assert_eq!(out.last().copied(), Some([4.0, 9.0]), "终点不动");
        for pair in out.windows(2) {
            assert!(
                pair[1][0] >= pair[0][0],
                "x 必须单调不回折：{:?}",
                (pair[0], pair[1])
            );
        }
        // 不足三个点原样返回。
        assert_eq!(smooth_curve(&points[..2]), points[..2]);
    }

    #[test]
    fn grid_marks_are_nice_and_cover_the_range() {
        let marks = |max: f64| {
            nice_grid_marks(egui_plot::GridInput {
                bounds: (0.0, max),
                base_step_size: 1.0,
            })
        };
        // 30 天档的 y 量程（比如 2.5B）：5 亿一步，首尾都是整数条。
        let two_and_half_b = marks(2_500_000_000.0);
        assert_eq!(two_and_half_b.len(), 6);
        assert_eq!(two_and_half_b[0].value, 0.0);
        assert_eq!(two_and_half_b[5].value, 2_500_000_000.0);

        // 任何量程都至少 3 条刻度（默认实现会掉到只剩 0）。
        for max in [
            300_000_000.0f64,
            950_000_000.0,
            1_050_000_000.0,
            9_500_000_000.0,
        ] {
            let count = marks(max).len();
            assert!(count >= 3, "{max} 刻度太少：{count}");
        }
    }

    /// 画法切换的文案齐全。
    #[test]
    fn chart_kind_labels_are_complete() {
        assert_eq!(ChartKind::Bars.label(), "柱状");
        assert_eq!(ChartKind::Line.label(), "曲线");
        assert_eq!(ChartKind::default(), ChartKind::Bars, "默认柱状");
    }
}
