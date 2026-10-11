//! 用量趋势图：按天柱状，可按模型堆叠。
//!
//! 数据源是 `DailyBucket`（按本地日历日分桶），不是会话级汇总 —— 一个会话
//! 可以横跨几十天，按它的 `last_seen_ms` 归日会把整月的量堆到最后一天。
//!
//! 堆叠只取**用量前几名的模型**，其余并成「其他」：颜色数量有限，全画出来
//! 图例会挤成一团，而且长尾模型各自的量在图上看不出差别。
//!
//! 配色一律由当前主题的强调色按黄金角散开（见 [`series_color`]），不硬编码
//! 颜色 —— 八个主题（深色 / 亮色 / 海洋 / 极地 / 苔藓 / 薄荷 / 薰衣草 / 玫瑰）
//! 都要能看。

use super::aggregate;
use super::range::{self};
use crate::app::App;
use crate::usage::DailyBucket;
use eframe::egui;
use egui_plot::{Bar, BarChart, Legend, Plot};
use std::collections::BTreeMap;

/// 图表高度。
const PLOT_HEIGHT: f32 = 190.0;

/// 堆叠最多画几个模型，其余并成「其他」。
const MAX_SERIES: usize = 6;

/// 「其他」在图例与堆叠里的名字。
const OTHER_LABEL: &str = "其他";

/// 堆叠柱的间隙：柱宽略小于相邻日索引的距离，让相邻两天看得出分隔。
const BAR_GAP: f64 = 0.15;

/// 柱宽：只画一天时给固定宽度，否则「1 天」的柱子会宽到填满整幅图。
fn bar_width(days: usize) -> f64 {
    if days <= 1 {
        0.6
    } else {
        1.0 - BAR_GAP
    }
}

/// 模型用量的一个系列（一个颜色一摞柱子）。
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
    /// 趋势图：按天柱状，按模型堆叠。
    ///
    /// 只在区间内有数据时画；没有数据时整块不占高度（总览卡已经给了空状态提示）。
    pub(in crate::app) fn ui_usage_chart(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let range = self.usage_range;
        let buckets = range::buckets_in_range(&daily, range, today);
        if buckets.is_empty() {
            return;
        }

        let stacked = self.usage_chart_stacked;
        let series = build_series(
            &buckets,
            stacked,
            self.theme.accent_color(),
            ui.visuals().dark_mode,
        );

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("每日用量").strong());
            ui.label(
                egui::RichText::new(range::totals_label(range))
                    .size(crate::theme::TEXT_CAPTION)
                    .color(ui.visuals().weak_text_color()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if super::chip(ui, stacked, "按模型堆叠", 96.0) {
                    self.usage_chart_stacked = !stacked;
                }
            });
        });
        ui.add_space(crate::theme::SPACE_2);

        let days: Vec<i64> = buckets.keys().copied().collect();
        let width = bar_width(days.len());
        let mut plot = Plot::new("usage_daily_plot")
            .height(PLOT_HEIGHT)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_drag(false)
            .allow_boxed_zoom(false)
            .allow_axis_zoom_drag(false)
            .allow_double_click_reset(false)
            .show_background(false)
            .show_grid(true)
            // 柱子贴边会被裁掉一半：留一点横向余量。
            .set_margin_fraction(egui::vec2(0.02, 0.08))
            .x_axis_formatter(axis_day_label)
            .y_axis_formatter(|mark, _| aggregate::format_tokens(mark.value as i64));
        if stacked {
            plot = plot.legend(Legend::default().background_alpha(0.0));
        }

        plot.show(ui, |plot_ui| {
            // 累积高度按天维护、**跨系列**累加：堆叠要把每根柱子摞在
            // 前面几摞之上，所以这个游标不能每个系列重置。
            let mut cumulative = vec![0.0f64; days.len()];
            for item in &series {
                let mut bars: Vec<Bar> = Vec::with_capacity(days.len());
                for (index, day) in days.iter().enumerate() {
                    let value = item.by_day.get(day).copied().unwrap_or(0) as f64;
                    let base = if stacked {
                        let base = cumulative[index];
                        cumulative[index] += value;
                        base
                    } else {
                        0.0
                    };
                    bars.push(
                        Bar::new(*day as f64, value)
                            .base_offset(base)
                            .width(width)
                            .fill(item.color),
                    );
                }
                plot_ui.bar_chart(BarChart::new(item.label.clone(), bars).vertical());
            }
        });
    }
}

/// 轴标签：日索引 → `M/D`。
///
/// 轴上不写年份（区间最长也就一年），也不写「今天 / 昨天」—— 刻度是等距的，
/// 夹一个「今天」进去会读不出节奏。
fn axis_day_label(mark: egui_plot::GridMark, _range: &std::ops::RangeInclusive<f64>) -> String {
    let (_, month, day) = aggregate::civil_from_days(mark.value as i64);
    format!("{month}/{day}")
}

/// 把区间内的桶整理成堆叠用的系列。
///
/// 非堆叠模式只返回一个系列（每天的总量），堆叠模式按模型拆开、取前
/// [`MAX_SERIES`] 名，其余并成「其他」。
fn build_series(
    buckets: &BTreeMap<i64, &DailyBucket>,
    stacked: bool,
    accent: egui::Color32,
    dark: bool,
) -> Vec<Series> {
    if !stacked {
        let by_day = buckets
            .iter()
            .map(|(day, bucket)| (*day, bucket.totals.total()))
            .collect();
        let total = buckets.values().map(|b| b.totals.total()).sum();
        return vec![Series {
            label: "合计".to_string(),
            color: accent,
            by_day,
            total,
        }];
    }

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
    fn unstacked_has_one_series_of_daily_totals() {
        let data = [
            (0i64, bucket("1970-01-01", &[("a", 10), ("b", 5)])),
            (1, bucket("1970-01-02", &[("a", 20)])),
        ];
        let series = build_series(&buckets_of(&data), false, egui::Color32::RED, true);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].label, "合计");
        assert_eq!(series[0].by_day[&0], 15);
        assert_eq!(series[0].by_day[&1], 20);
        assert_eq!(series[0].total, 35);
    }

    #[test]
    fn stacked_splits_by_model() {
        let data = [
            (0i64, bucket("1970-01-01", &[("a", 10), ("b", 5)])),
            (1, bucket("1970-01-02", &[("a", 20)])),
        ];
        let series = build_series(&buckets_of(&data), true, egui::Color32::RED, true);
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
    fn stacked_folds_the_tail_into_other() {
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
        let series = build_series(&buckets_of(&data), true, egui::Color32::RED, true);
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
        let label = |day: i64| {
            axis_day_label(
                egui_plot::GridMark {
                    value: day as f64,
                    step_size: 1.0,
                },
                &(0.0..=1.0),
            )
        };
        assert_eq!(label(0), "1/1");
        assert_eq!(label(aggregate::days_from_civil(2026, 3, 15)), "3/15");
    }
}
