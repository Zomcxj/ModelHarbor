//! Time-range filter: today / this week / this month / all.
//!
//! Data source is [`DailyMap`] (buckets per local calendar day), not session-level sums — one session
//! can span dozens of days, so using its lifetime total as "this month's usage" would overstate.
//!
//! Range bounds are expressed as **day indexes** (1970-01-01 = 0), the same coordinate system as
//! [`aggregate::local_day_index`]; `DailyMap` keys are `YYYY-MM-DD`, and the two are converted via
//! [`aggregate::days_from_civil`].

use super::aggregate;
use crate::usage::{DailyBucket, DailyMap, DayTotals};
use std::collections::BTreeMap;

/// A time range.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum Range {
    /// Today.
    Today,
    /// This week (Monday starts the week).
    Week,
    /// This month.
    #[default]
    Month,
    /// All history.
    All,
}

impl Range {
    /// All ranges, in the UI's display order.
    pub(in crate::app) const ALL: [Range; 4] =
        [Range::Today, Range::Week, Range::Month, Range::All];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Today => "今日",
            Self::Week => "本周",
            Self::Month => "本月",
            Self::All => "全部",
        }
    }

    /// Stable key for persistence (written into settings.json; decoupled from label so wording changes don't touch archives).
    pub(in crate::app) fn key(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Week => "week",
            Self::Month => "month",
            Self::All => "all",
        }
    }

    /// Restore from a persistence key; an empty or unknown key falls back to the default tier (this month).
    pub(in crate::app) fn from_key(key: &str) -> Range {
        match key {
            "today" => Range::Today,
            "week" => Range::Week,
            "month" => Range::Month,
            "all" => Range::All,
            _ => Range::default(),
        }
    }

    /// Day-index range the range covers (inclusive); `All` returns `None` (no lower bound).
    ///
    /// The upper bound is fixed at `today`: the ledger may hold future-dated records (a fiddled system clock,
    /// an agent writing wrong dates) and they must not leak into "today".
    pub(in crate::app) fn day_bounds(self, today: i64) -> Option<(i64, i64)> {
        match self {
            Self::Today => Some((today, today)),
            // Monday starts the week: `weekday_mon0` gives 0=Monday…6=Sunday.
            Self::Week => Some((today - i64::from(aggregate::weekday_mon0(today)), today)),
            Self::Month => {
                let (year, month, _) = aggregate::civil_from_days(today);
                Some((aggregate::days_from_civil(year, month, 1), today))
            }
            Self::All => None,
        }
    }

    /// Whether the range includes a given day.
    ///
    /// The production path enumerates in-range days via [`Range::day_bounds`] (a contiguous range beats
    /// per-day checks in both speed and clarity); this one stays for tests as an assertion helper, and for
    /// any future "judge a single day" need.
    #[cfg(test)]
    pub(in crate::app) fn contains(self, day: i64, today: i64) -> bool {
        match self.day_bounds(today) {
            None => day <= today,
            Some((start, end)) => day >= start && day <= end,
        }
    }
}

/// Range name + "usage", used for overview card titles (e.g. "this month's usage").
pub(in crate::app) fn totals_label(range: Range) -> &'static str {
    match range {
        Range::Today => "今日用量",
        Range::Week => "本周用量",
        Range::Month => "本月用量",
        Range::All => "总计用量",
    }
}

/// The trend chart's own time range (separate from the global [`Range`]).
///
/// The chart answers "how has the recent trend been", so a fixed window (last N days) fits better
/// than natural ranges like today / week / month; the five tiers mirror token-monitor's trend page.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum ChartRange {
    /// Last 7 days.
    D7,
    /// Last 30 days.
    #[default]
    D30,
    /// Last 90 days.
    D90,
    /// Last year.
    Y1,
    /// All history.
    All,
}

impl ChartRange {
    /// All tiers, in the UI's display order.
    pub(in crate::app) const ALL: [ChartRange; 5] = [
        ChartRange::D7,
        ChartRange::D30,
        ChartRange::D90,
        ChartRange::Y1,
        ChartRange::All,
    ];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::D7 => "7天",
            Self::D30 => "30天",
            Self::D90 => "90天",
            Self::Y1 => "一年",
            Self::All => "全部",
        }
    }

    /// Days the range covers; `All` returns `None`.
    pub(in crate::app) fn days(self) -> Option<i64> {
        match self {
            Self::D7 => Some(7),
            Self::D30 => Some(30),
            Self::D90 => Some(90),
            Self::Y1 => Some(365),
            Self::All => None,
        }
    }

    /// Day-index range the range covers (inclusive); `All` returns `None` (no lower bound).
    ///
    /// Upper bound fixed at `today`, same accounting as [`Range::day_bounds`]: future-dated records in the
    /// ledger must not leak into "last 7 days".
    pub(in crate::app) fn day_bounds(self, today: i64) -> Option<(i64, i64)> {
        self.days().map(|days| (today - days + 1, today))
    }
}

/// A day's bucket in `DailyMap`.
///
/// `DailyMap` keys are `YYYY-MM-DD` strings (that's the format tokscale emits) while range checks use
/// day indexes; conversion happens on demand instead of translating all 42 keys upfront.
fn bucket_for(daily: &DailyMap, day: i64) -> Option<&DailyBucket> {
    let (year, month, day_of_month) = aggregate::civil_from_days(day);
    let key = format!("{year:04}-{month:02}-{day_of_month:02}");
    daily.get(&key)
}

/// Filter `DailyMap` by range into "day index → bucket".
///
/// Returns a `BTreeMap` so callers can iterate in time order (heatmap, day-dimension table).
pub(in crate::app) fn buckets_in_range(
    daily: &DailyMap,
    range: Range,
    today: i64,
) -> BTreeMap<i64, &DailyBucket> {
    buckets_in_bounds(daily, range.day_bounds(today), today)
}

/// Buckets within a chart range: same logic as [`buckets_in_range`], bounds from [`ChartRange`].
pub(in crate::app) fn buckets_in_chart_range(
    daily: &DailyMap,
    chart_range: ChartRange,
    today: i64,
) -> BTreeMap<i64, &DailyBucket> {
    buckets_in_bounds(daily, chart_range.day_bounds(today), today)
}

/// Take buckets by day-index bounds (inclusive; `None` = all history).
fn buckets_in_bounds(
    daily: &DailyMap,
    bounds: Option<(i64, i64)>,
    today: i64,
) -> BTreeMap<i64, &DailyBucket> {
    let mut out = BTreeMap::new();
    match bounds {
        Some((start, end)) => {
            // A range is contiguous days; take buckets day by day (ranges span at most a month — negligible cost).
            for day in start..=end {
                if let Some(bucket) = bucket_for(daily, day) {
                    out.insert(day, bucket);
                }
            }
        }
        None => {
            // All: iterate the days the ledger actually has instead of enumerating a range.
            for (key, bucket) in daily {
                if let Some(day) = parse_day_key(key) {
                    if day <= today {
                        out.insert(day, bucket);
                    }
                }
            }
        }
    }
    out
}

/// `YYYY-MM-DD` → day index; returns `None` on malformed input.
pub(in crate::app) fn parse_day_key(key: &str) -> Option<i64> {
    let bytes = key.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year: i64 = key.get(0..4)?.parse().ok()?;
    let month: i64 = key.get(5..7)?.parse().ok()?;
    let day: i64 = key.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(aggregate::days_from_civil(year, month, day))
}

/// Totals within the range (across all agents).
pub(in crate::app) fn totals_in_range(daily: &DailyMap, range: Range, today: i64) -> DayTotals {
    let mut out = DayTotals::default();
    for bucket in buckets_in_range(daily, range, today).values() {
        out.add_assign(&bucket.totals);
    }
    out
}

/// Aggregate facts within the range (for the overview card).
///
/// Like the reference (tokscale's Token Usage panel), "total" is kept apart from "per-day average /
/// active days / peak day": totals alone say nothing about frequency — burning 1B in one day and
/// trickling the same total across a month are very different habits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::app) struct RangeStats {
    /// Totals within the range.
    pub totals: DayTotals,
    /// Days with usage (zero-usage days don't count).
    pub active_days: usize,
    /// The peak day: `(day index, that day's total)`. `None` when the range has no usage.
    pub best_day: Option<(i64, i64)>,
}

impl RangeStats {
    /// Per-day average (by active days, not calendar days).
    ///
    /// Active days instead of range length: the latter counts unused days, yielding a smaller, meaningless average.
    pub(in crate::app) fn average_per_active_day(&self) -> i64 {
        if self.active_days == 0 {
            return 0;
        }
        self.totals.total() / self.active_days as i64
    }
}

/// Aggregate facts within the range.
pub(in crate::app) fn range_stats(daily: &DailyMap, range: Range, today: i64) -> RangeStats {
    let mut stats = RangeStats::default();
    for (day, bucket) in buckets_in_range(daily, range, today) {
        let total = bucket.totals.total();
        stats.totals.add_assign(&bucket.totals);
        if total > 0 {
            stats.active_days += 1;
            if stats.best_day.is_none_or(|(_, best)| total > best) {
                stats.best_day = Some((day, total));
            }
        }
    }
    stats
}

/// The top-usage model within the range.
///
/// Returns `None` when no model was recorded in the range (never fabricates an "unknown model").
pub(in crate::app) fn top_model_in_range(
    daily: &DailyMap,
    range: Range,
    today: i64,
) -> Option<(String, i64)> {
    let mut best: Option<(String, i64)> = None;
    for (model, totals) in breakdown_in_range(daily, range, today, |bucket| &bucket.by_model) {
        let total = totals.total();
        if total <= 0 {
            continue;
        }
        if best.as_ref().is_none_or(|(_, current)| total > *current) {
            best = Some((model, total));
        }
    }
    best
}

/// Consecutive active days (counting back from today; stops at the first gap).
///
/// Follows the GitHub / tokscale convention: the streak only counts if "today or yesterday had
/// usage" — otherwise a week-long break would still show "1-day streak", a leftover of the previous
/// record rather than a current streak. Today having no usage yet doesn't break it (the day isn't over).
pub(in crate::app) fn current_streak(daily: &DailyMap, today: i64) -> usize {
    let has_usage =
        |day: i64| bucket_for(daily, day).is_some_and(|bucket| bucket.totals.total() > 0);
    // No usage today → count from yesterday (today may not have started yet).
    let mut cursor = if has_usage(today) {
        today
    } else if has_usage(today - 1) {
        today - 1
    } else {
        return 0;
    };
    let mut streak = 0usize;
    while has_usage(cursor) {
        streak += 1;
        cursor -= 1;
    }
    streak
}

/// Aggregate the range by one breakdown dimension (agent / model / session).
///
/// Breakdowns live in [`DailyBucket`]; `pick` decides what the keys mean. Summing across days yields
/// correct in-range usage — exactly where session lifetime totals would be wrong.
pub(in crate::app) fn breakdown_in_range<'a>(
    daily: &'a DailyMap,
    range: Range,
    today: i64,
    pick: impl Fn(&'a DailyBucket) -> &'a BTreeMap<String, DayTotals>,
) -> BTreeMap<String, DayTotals> {
    let mut out: BTreeMap<String, DayTotals> = BTreeMap::new();
    for bucket in buckets_in_range(daily, range, today).values() {
        for (key, totals) in pick(bucket) {
            out.entry(key.clone()).or_default().add_assign(totals);
        }
    }
    out
}

/// One heatmap cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct HeatCell {
    /// Day index.
    pub day: i64,
    /// That day's total tokens.
    pub total: i64,
    /// Whether the ledger has a record for this day (absent = empty cell, not zero usage).
    pub has_data: bool,
}

/// Heatmap grid: `weeks` columns × 7 rows, last column is the current week.
///
/// Each column is a week (Monday..Sunday), rows are weekdays — the GitHub contribution-graph layout.
/// Cells are returned column-major (first week's 7 days, then next week...) for straightforward
/// row-and-column painting.
pub(in crate::app) fn heatmap_grid(daily: &DailyMap, today: i64, weeks: usize) -> Vec<HeatCell> {
    // The Monday of the last column; stepping back `weeks - 1` weeks gives the first column's Monday.
    let last_monday = today - i64::from(aggregate::weekday_mon0(today));
    let first_monday = last_monday - (weeks as i64 - 1) * 7;

    let mut out = Vec::with_capacity(weeks * 7);
    for week in 0..weeks as i64 {
        for weekday in 0..7i64 {
            let day = first_monday + week * 7 + weekday;
            let bucket = if day <= today {
                bucket_for(daily, day)
            } else {
                None
            };
            out.push(HeatCell {
                day,
                total: bucket.map(|b| b.totals.total()).unwrap_or(0),
                has_data: bucket.is_some(),
            });
        }
    }
    out
}

/// Heatmap intensity levels (0-4), thresholds set by quantiles like GitHub.
///
/// Returns **3 boundary values** `[t0, t1, t2]`: greater than 0 boundaries → level 1, greater than
/// 1 → level 2, greater than 2 → level 3, greater than all 3 → level 4 (see [`heat_level`]).
///
/// Why not 4 thresholds: level 4 needs 3 boundaries; a 4th threshold could only be the maximum
/// itself, and "greater than the maximum" is never true — the busiest day would never reach the
/// deepest tier.
///
/// Quantiles instead of fixed thresholds: agent magnitudes differ by orders of magnitude (dsh runs
/// tens of thousands here, opencode over a billion); fixed thresholds would leave one side all white
/// and the other all green.
///
/// Uses the **nearest-rank** method (`ceil(q * n) - 1`) rather than `round((n - 1) * q)`: the latter
/// produces duplicate thresholds on small samples (with n = 4, q = 0.75 and q = 0.5 both land on
/// index 2), and duplicate thresholds make some middle level unreachable.
pub(in crate::app) fn heat_thresholds(values: &[i64]) -> [i64; 3] {
    let mut nonzero: Vec<i64> = values.iter().copied().filter(|v| *v > 0).collect();
    if nonzero.is_empty() {
        return [0; 3];
    }
    nonzero.sort_unstable();
    let n = nonzero.len();
    let quantile = |q: f64| -> i64 {
        // Nearest rank: at least the first, at most the last.
        let rank = (q * n as f64).ceil().max(1.0) as usize;
        nonzero[rank.min(n) - 1]
    };
    let mut out = [quantile(0.25), quantile(0.50), quantile(0.75)];
    // With few samples several quantiles collide; push duplicates back to keep strictly increasing
    // (otherwise some level stays empty forever).
    for index in 1..out.len() {
        if out[index] <= out[index - 1] {
            out[index] = out[index - 1].saturating_add(1);
        }
    }
    out
}

/// Which level a day's usage falls into (0 = no data, 1-4 progressively deeper).
///
/// Rule: "strictly greater than how many boundaries". The maximum is necessarily greater than all 3
/// (thresholds come from the sample and never exceed the max), so the busiest day always reaches the
/// deepest tier.
pub(in crate::app) fn heat_level(total: i64, thresholds: &[i64; 3]) -> u8 {
    if total <= 0 {
        return 0;
    }
    let below = thresholds.iter().filter(|bound| **bound < total).count();
    (below.min(3) + 1) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::DailyBucket;

    /// Build a bucket with totals only.
    fn day(input: i64) -> DailyBucket {
        let mut bucket = DailyBucket::new("x");
        bucket.totals.input = input;
        bucket.totals.messages = 1;
        bucket
    }

    /// 2026-03-15 is a Sunday; fixed dates keep tests independent of "today".
    fn today() -> i64 {
        aggregate::days_from_civil(2026, 3, 15)
    }

    fn daily_with(days: &[(&str, i64)]) -> DailyMap {
        days.iter()
            .map(|(key, value)| ((*key).to_string(), day(*value)))
            .collect()
    }

    /// Today contains only today.
    #[test]
    fn today_range_covers_only_today() {
        let today = today();
        assert_eq!(Range::Today.day_bounds(today), Some((today, today)));
        assert!(Range::Today.contains(today, today));
        assert!(!Range::Today.contains(today - 1, today));
    }

    /// The week starts Monday: 2026-03-15 is a Sunday, so this week's Monday is 03-09.
    #[test]
    fn week_starts_on_monday() {
        let today = today();
        let (start, end) = Range::Week.day_bounds(today).unwrap();
        assert_eq!(end, today);
        assert_eq!(aggregate::civil_from_days(start), (2026, 3, 9));
        // Sunday is inside the week (not the start of the next one).
        assert!(Range::Week.contains(today, today));
        // Last Sunday is not.
        assert!(!Range::Week.contains(today - 7, today));
    }

    /// The month starts on the 1st.
    #[test]
    fn month_starts_on_the_first() {
        let today = today();
        let (start, end) = Range::Month.day_bounds(today).unwrap();
        assert_eq!(end, today);
        assert_eq!(aggregate::civil_from_days(start), (2026, 3, 1));
        assert!(Range::Month.contains(start, today));
        assert!(!Range::Month.contains(start - 1, today));
    }

    /// All sets no lower bound, but future dates are still excluded.
    #[test]
    fn all_has_no_lower_bound_but_excludes_future() {
        let today = today();
        assert_eq!(Range::All.day_bounds(today), None);
        assert!(Range::All.contains(today - 10_000, today));
        assert!(!Range::All.contains(today + 1, today), "未来日期不算");
    }

    /// Range filtering takes buckets by day index.
    #[test]
    fn buckets_in_range_filters_by_day() {
        let today = today();
        let daily = daily_with(&[
            ("2026-03-15", 100),
            ("2026-03-14", 50),
            ("2026-03-01", 30),
            ("2026-02-28", 20),
        ]);
        assert_eq!(buckets_in_range(&daily, Range::Today, today).len(), 1);
        assert_eq!(buckets_in_range(&daily, Range::Month, today).len(), 3);
        assert_eq!(buckets_in_range(&daily, Range::All, today).len(), 4);
    }

    /// Range totals count only days inside the range.
    #[test]
    fn totals_in_range_sums_only_that_range() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 100), ("2026-03-14", 50), ("2026-02-28", 20)]);
        assert_eq!(totals_in_range(&daily, Range::Today, today).input, 100);
        assert_eq!(totals_in_range(&daily, Range::Month, today).input, 150);
        assert_eq!(totals_in_range(&daily, Range::All, today).input, 170);
    }

    /// Breakdowns sum across days: this is why "this month × pi" computes correctly.
    #[test]
    fn breakdown_sums_across_days() {
        let today = today();
        let mut first = day(100);
        first.by_client.insert("pi".to_string(), day(100).totals);
        let mut second = day(50);
        second.by_client.insert("pi".to_string(), day(50).totals);
        second
            .by_client
            .insert("opencode".to_string(), day(7).totals);
        let daily: DailyMap = [
            ("2026-03-15".to_string(), first),
            ("2026-03-14".to_string(), second),
        ]
        .into_iter()
        .collect();

        let rows = breakdown_in_range(&daily, Range::Month, today, |b| &b.by_client);
        assert_eq!(rows["pi"].input, 150);
        assert_eq!(rows["opencode"].input, 7);
    }

    /// Future-dated records don't leak into today (appears when the system clock was fiddled with).
    #[test]
    fn future_dated_records_are_ignored() {
        let today = today();
        let daily = daily_with(&[("2026-12-31", 999)]);
        assert_eq!(totals_in_range(&daily, Range::All, today).input, 0);
        assert_eq!(totals_in_range(&daily, Range::Today, today).input, 0);
    }

    /// Date-key parsing: normal, unpadded, malformed.
    #[test]
    fn parse_day_key_handles_edges() {
        assert_eq!(parse_day_key("1970-01-01"), Some(0));
        assert_eq!(
            parse_day_key("2026-03-15"),
            Some(aggregate::days_from_civil(2026, 3, 15))
        );
        assert_eq!(parse_day_key("2026-3-15"), None, "月份未补零");
        assert_eq!(parse_day_key("not-a-date"), None);
        assert_eq!(parse_day_key(""), None);
        assert_eq!(parse_day_key("2026-13-01"), None, "13 月不存在");
        assert_eq!(parse_day_key("2026-00-01"), None);
        assert_eq!(parse_day_key("2026-01-32"), None);
    }

    /// The grid is a multiple of 7, and the last cell is the Sunday of today's week.
    #[test]
    fn heatmap_grid_shape_and_alignment() {
        let today = today();
        let grid = heatmap_grid(&DailyMap::new(), today, 53);
        assert_eq!(grid.len(), 53 * 7);
        // The first cell is a Monday (53 weeks back).
        assert_eq!(aggregate::weekday_mon0(grid[0].day), 0);
        // The last column's Monday..Sunday: today is Sunday, so the last cell is today.
        assert_eq!(grid[grid.len() - 1].day, today);
    }

    /// Future cells are marked no-data, not shown as zero usage.
    ///
    /// Uses a Wednesday as "today" (2026-03-11): this leaves Thursday..Sunday — four cells — after today in
    /// the current week. Taking a Sunday as today would make the week's last cell today, untestable for
    /// "future cells".
    #[test]
    fn heatmap_future_cells_have_no_data() {
        let wednesday = aggregate::days_from_civil(2026, 3, 11);
        assert_eq!(aggregate::weekday_mon0(wednesday), 2, "2026-03-11 是周三");
        let grid = heatmap_grid(&DailyMap::new(), wednesday, 2);
        let future = grid.iter().filter(|cell| cell.day > wednesday).count();
        assert_eq!(future, 4, "周四到周日四格在未来");
        for cell in grid.iter().filter(|c| c.day > wednesday) {
            assert!(!cell.has_data, "未来格子不该有数据");
        }
    }

    /// Days with data get flagged.
    #[test]
    fn heatmap_marks_days_with_data() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 100)]);
        let grid = heatmap_grid(&daily, today, 1);
        let cell = grid.iter().find(|c| c.day == today).unwrap();
        assert!(cell.has_data);
        assert_eq!(cell.total, 100);
    }

    /// Quantile thresholds: zeros are excluded, so one heavy day can't yank the thresholds skyward.
    #[test]
    fn thresholds_ignore_zero_days() {
        let values = [0, 0, 0, 10, 20, 30, 40];
        let t = heat_thresholds(&values);
        assert_eq!(t[0], 10, "最小值作第 1 个分界");
        assert_eq!(t[2], 30, "上四分位作第 3 个分界");
        assert!(t[0] < t[1] && t[1] < t[2], "分界值严格递增：{t:?}");
        // The maximum is necessarily greater than all 3 boundaries (boundaries come from the sample itself).
        let max = *values.iter().max().unwrap();
        assert_eq!(heat_level(max, &t), 4, "最忙的一天要拿到最深的一档");
    }

    /// Even with few samples, thresholds must not repeat: duplicates leave some middle level unreachable.
    ///
    /// A pit actually hit: `round((n-1)*q)` with n = 4 yields [20,30,30,40], making level 3 require
    /// `> 30 && <= 30` — unsolvable.
    #[test]
    fn thresholds_are_strictly_increasing_even_for_tiny_samples() {
        for values in [vec![5], vec![5, 9], vec![10, 20, 30, 40], vec![1, 1, 1]] {
            let t = heat_thresholds(&values);
            assert!(t[0] < t[1], "{values:?} → {t:?}");
            assert!(t[1] < t[2], "{values:?} → {t:?}");
            // The lowest level always has samples (any non-zero value exceeds 0 boundaries).
            assert!(
                values.iter().any(|v| heat_level(*v, &t) == 1),
                "{values:?} → {t:?} 里没有 1 级的样本"
            );
            // How many tiers fill depends on the value range: one distinct value yields 1 tier; several yield at least 2.
            // The cap is always 4.
            let distinct = values
                .iter()
                .filter(|v| **v > 0)
                .collect::<std::collections::HashSet<_>>()
                .len();
            let reachable = values.iter().map(|v| heat_level(*v, &t)).max().unwrap_or(0);
            let expected_min = if distinct >= 2 { 2 } else { 1 };
            assert!(
                reachable >= expected_min,
                "{values:?} → {t:?} 至少能分出 {expected_min} 档（实际 {reachable}）"
            );
            assert!(reachable <= 4, "{values:?} → {t:?} 最多四档");
        }
    }

    /// All-zero data yields all-zero thresholds — no division by zero.
    #[test]
    fn thresholds_of_all_zero_are_zero() {
        assert_eq!(heat_thresholds(&[0, 0, 0]), [0; 3]);
        assert_eq!(heat_thresholds(&[]), [0; 3]);
    }

    /// Levels: 0 is no data, 1-4 progressive; the rule is "greater than how many thresholds".
    #[test]
    fn heat_level_maps_to_four_steps() {
        let t = [10, 20, 30];
        assert_eq!(heat_level(0, &t), 0, "0 是无数据");
        assert_eq!(heat_level(1, &t), 1, "比 0 个分界大");
        assert_eq!(heat_level(10, &t), 1, "等于 t0：不比它大");
        assert_eq!(heat_level(11, &t), 2, "比 t0 大");
        assert_eq!(heat_level(21, &t), 3, "比 t0/t1 大");
        assert_eq!(heat_level(31, &t), 4, "比 3 个分界都大");
        assert_eq!(heat_level(1000, &t), 4);
    }

    /// The busiest day must be able to reach level 4.
    ///
    /// A pit actually hit: levels were once assigned by chained `<=` comparisons, and `t3` happened to equal
    /// the maximum — `max > max` is never true, so the deepest tier stayed empty forever.
    #[test]
    fn the_largest_value_reaches_level_four() {
        let values = [1, 2, 3, 4, 5, 100];
        let t = heat_thresholds(&values);
        let max = *values.iter().max().unwrap();
        assert_eq!(heat_level(max, &t), 4, "阈值 {t:?}");
    }

    /// The range's title text.
    #[test]
    fn totals_label_names_every_range() {
        assert_eq!(totals_label(Range::Today), "今日用量");
        assert_eq!(totals_label(Range::Week), "本周用量");
        assert_eq!(totals_label(Range::Month), "本月用量");
        assert_eq!(totals_label(Range::All), "总计用量");
    }

    /// Persistence-key round trip: every range restores from its key; unknown / empty keys fall back to the default (this month).
    #[test]
    fn range_key_round_trips() {
        for range in Range::ALL {
            assert_eq!(Range::from_key(range.key()), range, "{range:?} 键往返");
        }
        assert_eq!(Range::from_key(""), Range::default(), "空键回退默认");
        assert_eq!(Range::from_key("bogus"), Range::default(), "未知键回退默认");
        assert_eq!(Range::default(), Range::Month, "默认档是本月");
    }

    // ---- The trend chart's own range ----

    /// A fixed window includes today, N days in all; All sets no lower bound but excludes the future.
    #[test]
    fn chart_range_bounds_cover_n_days_ending_today() {
        let today = today();
        assert_eq!(ChartRange::D7.day_bounds(today), Some((today - 6, today)));
        assert_eq!(ChartRange::D30.day_bounds(today), Some((today - 29, today)));
        assert_eq!(ChartRange::D90.day_bounds(today), Some((today - 89, today)));
        assert_eq!(ChartRange::Y1.day_bounds(today), Some((today - 364, today)));
        assert_eq!(ChartRange::All.day_bounds(today), None);
    }

    /// The chart takes buckets by its own range: the 7-day tier carries no last-month data.
    #[test]
    fn chart_buckets_respect_the_chart_range() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 100), ("2026-03-10", 50), ("2026-02-20", 999)]);
        let buckets = buckets_in_chart_range(&daily, ChartRange::D7, today);
        assert_eq!(buckets.len(), 2, "7 天档只要 03-10 与 03-15");
        let all = buckets_in_chart_range(&daily, ChartRange::All, today);
        assert_eq!(all.len(), 3);
    }

    // ---- Aggregate facts ----

    /// Active days count only days with usage; the peak day takes the maximum.
    #[test]
    fn range_stats_counts_only_active_days() {
        let today = today();
        let daily = daily_with(&[
            ("2026-03-15", 100),
            ("2026-03-14", 300),
            ("2026-03-13", 0),
            ("2026-03-12", 50),
        ]);
        let stats = range_stats(&daily, Range::All, today);
        assert_eq!(stats.totals.input, 450);
        assert_eq!(stats.active_days, 3, "0 用量的那天不算活跃");
        assert_eq!(stats.best_day.map(|(_, total)| total), Some(300));
        assert_eq!(
            stats.best_day.unwrap().0,
            aggregate::days_from_civil(2026, 3, 14)
        );
    }

    /// The daily average counts active days, not range length.
    #[test]
    fn average_divides_by_active_days() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 300), ("2026-03-14", 100)]);
        let stats = range_stats(&daily, Range::All, today);
        assert_eq!(stats.average_per_active_day(), 200, "400 / 2 天");
    }

    /// With no usage in the range: 0 active days, no peak day, 0 average (no division by zero).
    #[test]
    fn empty_range_stats_are_zero() {
        let stats = range_stats(&DailyMap::new(), Range::Month, today());
        assert_eq!(stats.active_days, 0);
        assert_eq!(stats.best_day, None);
        assert_eq!(stats.average_per_active_day(), 0);
    }

    /// Aggregate facts also respect the range.
    #[test]
    fn range_stats_respect_the_range() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 100), ("2026-02-20", 999)]);
        let stats = range_stats(&daily, Range::Today, today);
        assert_eq!(stats.totals.input, 100, "上月的不算进来");
        assert_eq!(stats.active_days, 1);
        assert_eq!(stats.best_day.map(|(_, t)| t), Some(100));
    }

    // ---- Consecutive days ----

    /// The streak counts back from today and stops at the first gap.
    #[test]
    fn streak_counts_back_from_today() {
        let today = today();
        let daily = daily_with(&[
            ("2026-03-15", 10),
            ("2026-03-14", 10),
            ("2026-03-13", 10),
            // 03-12 missing → stop here
            ("2026-03-11", 10),
        ]);
        assert_eq!(current_streak(&daily, today), 3);
    }

    /// Today having no usage yet doesn't break the streak (the day isn't over); counting starts from yesterday.
    #[test]
    fn streak_tolerates_a_quiet_today() {
        let today = today();
        let daily = daily_with(&[("2026-03-14", 10), ("2026-03-13", 10)]);
        assert_eq!(current_streak(&daily, today), 2);
    }

    /// Neither today nor yesterday having usage means broken — not "a leftover of the previous record".
    #[test]
    fn streak_is_zero_when_both_today_and_yesterday_are_empty() {
        let today = today();
        let daily = daily_with(&[("2026-03-13", 10), ("2026-03-12", 10)]);
        assert_eq!(current_streak(&daily, today), 0);
    }

    /// A zero-usage day is neither active nor streak-extending.
    #[test]
    fn streak_ignores_zero_usage_days() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 10), ("2026-03-14", 0), ("2026-03-13", 10)]);
        assert_eq!(current_streak(&daily, today), 1);
    }

    /// An empty ledger has no streak.
    #[test]
    fn streak_of_empty_ledger_is_zero() {
        assert_eq!(current_streak(&DailyMap::new(), today()), 0);
    }

    // ---- Favorite model ----

    /// The favorite model is the range's biggest user.
    #[test]
    fn top_model_picks_the_largest_in_range() {
        let today = today();
        let mut bucket = day(0);
        bucket.by_model.insert("small".into(), day(10).totals);
        bucket.by_model.insert("big".into(), day(90).totals);
        let daily: DailyMap = [("2026-03-15".to_string(), bucket)].into_iter().collect();
        let (model, total) = top_model_in_range(&daily, Range::Month, today).unwrap();
        assert_eq!(model, "big");
        assert_eq!(total, 90);
    }

    /// With no model records in the range, no answer — and never a fabricated "unknown model".
    #[test]
    fn top_model_is_none_without_data() {
        assert_eq!(
            top_model_in_range(&DailyMap::new(), Range::All, today()),
            None
        );
    }

    /// The favorite model also respects the range.
    #[test]
    fn top_model_respects_the_range() {
        let today = today();
        let mut recent = day(0);
        recent.by_model.insert("new".into(), day(10).totals);
        let mut old = day(0);
        old.by_model.insert("old".into(), day(999).totals);
        let daily: DailyMap = [
            ("2026-03-15".to_string(), recent),
            ("2026-02-20".to_string(), old),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            top_model_in_range(&daily, Range::Month, today).map(|(m, _)| m),
            Some("new".to_string()),
            "本月的量小，但上月的 999 不该算进来"
        );
    }
}
