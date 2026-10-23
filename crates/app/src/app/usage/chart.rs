//! Usage trend chart: daily bars (stacked by model), switchable to lines.
//!
//! Data source is `DailyBucket` (per local calendar day), not session-level sums — a session spanning
//! dozens of days, bucketed by its `last_seen_ms`, would pile a whole month onto its last day.
//!
//! Stacking keeps only the **top models by usage** and folds the rest into "Other": colors are
//! limited, a full legend would cram together, and long-tail models are indistinguishable at chart
//! scale anyway. Stacking is the only mode (user asked to drop the toggle): where tokens went is
//! readable at a glance.
//!
//! Colors spread from the active theme's accent by the golden angle (see [`series_color`]); nothing
//! hard-coded — all eight themes (dark / light / ocean / polar / moss / mint / lavender / rose) must
//! read well.

use super::aggregate;
use super::range::{self};
use crate::app::App;
use crate::usage::DailyBucket;
use eframe::egui;
use egui_plot::{Bar, BarChart, Line, Plot, PlotPoints};
use std::collections::BTreeMap;

/// Chart height.
const PLOT_HEIGHT: f32 = 190.0;

/// Max models drawn in the stack; the rest fold into "Other".
const MAX_SERIES: usize = 6;

/// The "Other" name in legend and stack.
const OTHER_LABEL: &str = "其他";

/// Bar gap: bar width is slightly smaller than the distance between adjacent day indexes so consecutive days read as separate.
const BAR_GAP: f64 = 0.15;

/// Chart mode: bars (stacked by model) or lines (one line per model).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum ChartKind {
    /// Bars (default): stacked columns — total and composition at a glance.
    #[default]
    Bars,
    /// Lines: one line per model — clearer for a single model's trend.
    Line,
}

impl ChartKind {
    /// All modes, in the UI's display order (added reversed under the right-to-left layout).
    pub(in crate::app) const ALL: [ChartKind; 2] = [ChartKind::Line, ChartKind::Bars];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Bars => "柱状",
            Self::Line => "曲线",
        }
    }
}

/// Bar width when only one day is drawn: a fixed width, otherwise a "1 day" bar would span the whole chart.
fn bar_width(days: usize) -> f64 {
    if days <= 1 {
        0.6
    } else {
        1.0 - BAR_GAP
    }
}

/// One model-usage series (one color's stack of bars / one line).
struct Series {
    /// Legend name (model id or "Other").
    label: String,
    /// The color.
    color: egui::Color32,
    /// Day index → that model's total for the day (missing days count 0).
    by_day: BTreeMap<i64, i64>,
    /// In-range total, for the legend's quantity.
    total: i64,
}

/// Series color: see [`super::series_color`].
fn series_color(index: usize, accent: egui::Color32, dark: bool) -> egui::Color32 {
    super::series_color(index, accent, dark)
}

impl App {
    /// The trend chart: daily bars (stacked by model), switchable to lines.
    ///
    /// The range is the chart's own tier (7 / 30 / 90 days / a year / all), independent of the global
    /// range: the overview card and tables look at "today / week / month", the chart at the trend.
    /// Painted only when the range holds data; with none, the whole block takes no height (the overview
    /// card already shows the empty state).
    pub(in crate::app) fn ui_usage_chart(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let chart_range = self.usage_chart_range;
        let buckets = range::buckets_in_chart_range(&daily, chart_range, today);
        if buckets.is_empty() {
            return;
        }

        let kind = self.usage_chart_kind;
        // Bars split by model to see composition; lines draw a single **total** line (user request: show the total directly).
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
                    // Added reversed under the right-to-left layout: visual order = [Bars] [Lines].
                    for option in ChartKind::ALL {
                        if super::chip(ui, kind == option, option.label(), 44.0) {
                            self.usage_chart_kind = option;
                        }
                    }
                });
            });
            ui.add_space(crate::theme::SPACE_2);

            let days: Vec<i64> = buckets.keys().copied().collect();
            // x counts from 0 (the range's first day = 0). egui_plot ticks take "absolute multiples of the step":
            // with absolute day indexes (~20k), once the step grows the first tick lands after the start day and
            // tiers above 30 days lose their left-edge ticks (user feedback: missing scale on the left).
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
                // Internal grid lines slice the chart into cells while the bars already align for reading (user feedback: removed).
                .show_grid(false)
                // Bars flush to the edge get half-clipped: keep a little horizontal margin. 4% rather than 2%:
                // the last tier's date label (e.g. "10/12") paints centered on its tick, half its width falling to the
                // tick's right — 2% can't cover it and the label pokes outside the card (user feedback).
                .set_margin_fraction(egui::vec2(0.04, 0.08))
                // Hover no longer shows x/y coordinate readouts (user request: no xy display).
                .show_x(false)
                .show_y(false)
                .x_axis_formatter(move |mark, _| {
                    day_axis_label(mark.value.round() as i64, first_day)
                })
                .y_axis_formatter(|mark, _| aggregate::format_m(mark.value as i64))
                // Ticks pick nice numbers themselves (1/2/5×10^k, ~5 lines): the default quantizes by powers of 10,
                // so right after the range crosses a power of ten the fine ticks' spacing skips them all and only 0
                // remains in view — the 30-day tier had no scale at all (user feedback).
                .x_grid_spacer(nice_grid_marks)
                .y_grid_spacer(nice_grid_marks);

            let plot_response = plot.show(ui, |plot_ui| match kind {
                ChartKind::Bars => {
                    // Cumulative height is maintained per day and accumulated **across series**: stacking puts each bar on
                    // top of the previous stacks, so this cursor must not reset per series.
                    let mut cumulative = vec![0.0f64; days.len()];
                    for item in &series {
                        let mut bars: Vec<Bar> = Vec::with_capacity(days.len());
                        for (index, day) in days.iter().enumerate() {
                            let value = item.by_day.get(day).copied().unwrap_or(0) as f64;
                            let base = cumulative[index];
                            cumulative[index] += value;
                            bars.push(
                                Bar::new((*day - first_day) as f64, value)
                                    // The hover tip must name the model: the name hangs on the segment.
                                    .name(item.label.clone())
                                    .base_offset(base)
                                    .width(width)
                                    .fill(item.color),
                            );
                        }
                        // The quantity-to-model mapping comes from the legend below the chart and the hand-drawn hover tip
                        // (egui_plot pops no built-in label with show_x/show_y both off).
                        plot_ui.bar_chart(BarChart::new(item.label.clone(), bars).vertical());
                    }
                }
                ChartKind::Line => {
                    // One total line: spline transitions between days — jagged lines don't read as trends.
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
                            .width(2.0_f32),
                    );
                }
            });

            // Hover readout (user feedback: see each model's usage that day, and the tip must **follow the mouse**):
            // egui_plot's built-in hover returns nothing with show_x/show_y both off, while egui's tooltip pins
            // where it first appeared. So it's hand-drawn: recompute the day index each frame, with the overlay
            // fixed beside the pointer — when the pointer is on the chart's right half, the box flips left so it
            // never pokes out the window's right edge.
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

            // The legend paints **outside** the chart (below): egui_plot's built-in legend sits fixed in the plot
            // area's top-right corner and covers bars once entries multiply. Outside also lets each series carry its usage.
            ui.add_space(crate::theme::SPACE_2);
            // Line mode leads the legend (card's bottom-left) with the single-day peak.
            let peak = (kind == ChartKind::Line)
                .then(|| series[0].by_day.values().copied().max().unwrap_or(0));
            legend_row(ui, &series, peak);
        });
    }
}

/// Legend: one swatch + model name + usage per item, wrapping when out of width.
///
/// Names use a **proportional** face, not monospace: model names can be long, and monospace makes
/// them noticeably wider, wrapping earlier. Over-long names truncate with ellipses; full text on hover.
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
        // Align with tokenmonitor's dashboard row: each series carries its share (denominator = all series'
        // totals within the chart's range).
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
            // Gap between items: a swatch hugging the previous number reads as "two numbers".
            ui.add_space(crate::theme::SPACE_2);
        }
    });
}

/// Axis labels: day index → `M/D`.
///
/// No year on the axis (ranges span at most a year), and no "today / yesterday" either — ticks are
/// evenly spaced, and sneaking "today" in between ruins the rhythm.
/// x-axis tick text: x is "days since the range's first day"; adding the first day recovers the date.
fn day_axis_label(offset_days: i64, first_day: i64) -> String {
    let (_, month, day) = aggregate::civil_from_days(first_day + offset_days);
    format!("{month}/{day}")
}

/// Grid ticks: pick "nice numbers" (1/2/5×10^k) for roughly 5 ticks across the range.
///
/// Ignores `base_step_size` and splits the bounds evenly: every tier still yields 3+ ticks.
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
    // Both axes' data start at 0 (x is the first-day offset, y is non-negative usage): no ticks on the negative side.
    let first = (min / step).ceil().max(0.0);
    let last = (max / step).floor();
    (first as i64..=last as i64)
        .map(|k| egui_plot::GridMark {
            value: k as f64 * step,
            step_size: step,
        })
        .collect()
}

/// Line-chart series: a single "total", showing each day's total directly.
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

/// Catmull-Rom spline: subdivides each segment for smooth transitions (user-requested curve smoothing).
///
/// x advances linearly; only y is splined: missing days make spacing uneven, and splining x too could
/// double back. Endpoints without neighbors use themselves as stand-ins, so the curve straightens
/// naturally at both ends.
fn smooth_curve(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    const STEPS: usize = 8;
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let at = |i: i64| points[i.clamp(0, n as i64 - 1) as usize];
    // Catmull-Rom interpolation of a single coordinate.
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

/// Arrange in-range buckets into series: split by model, keep the top [`MAX_SERIES`], fold the rest into "Other".
///
/// Bars and lines share this exact series split; only the drawing differs.
fn build_series(
    buckets: &BTreeMap<i64, &DailyBucket>,
    accent: egui::Color32,
    dark: bool,
) -> Vec<Series> {
    // First sum each model's in-range total, then sort and take the top few.
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

    // With no model breakdown in the range, an empty series returns; `series.len() - 1` on it would
    // underflow usize.
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
    // "Other" appears only when something was actually truncated.
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

    /// Arrange `(day index, bucket)` into the shape `build_series` expects.
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
        // Descending by total: a 30 > b 5.
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
        // 8 models → the top 6 each become a series; the remaining 2 fold into "Other".
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
        // Series 0 IS the accent itself (an HSVA round trip has rounding error; tolerate 2 shade steps).
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
        // x is "days since the first day": 0 = the first day itself, so the left edge always has a tick.
        assert_eq!(day_axis_label(0, 0), "1/1");
        let first = aggregate::days_from_civil(2026, 3, 10);
        assert_eq!(day_axis_label(5, first), "3/15");
    }

    /// Numbers inside the chart use the unified M unit: one decimal below 10M, rounded above.
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
        // Fewer than three points returns as-is.
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
        // A 30-day tier's y range (say 2.5B): 500M per step, whole-number bars at both ends.
        let two_and_half_b = marks(2_500_000_000.0);
        assert_eq!(two_and_half_b.len(), 6);
        assert_eq!(two_and_half_b[0].value, 0.0);
        assert_eq!(two_and_half_b[5].value, 2_500_000_000.0);

        // Any range yields at least 3 ticks (the default implementation can collapse to just 0).
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

    /// Every mode-switch label is present.
    #[test]
    fn chart_kind_labels_are_complete() {
        assert_eq!(ChartKind::Bars.label(), "柱状");
        assert_eq!(ChartKind::Line.label(), "曲线");
        assert_eq!(ChartKind::default(), ChartKind::Bars, "默认柱状");
    }
}
