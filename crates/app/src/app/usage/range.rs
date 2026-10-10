//! 时间区间筛选：今日 / 本周 / 本月 / 全部。
//!
//! 数据源是 [`DailyMap`]（按本地日历日分桶），不是会话级汇总 —— 一个会话可以
//! 横跨几十天，拿它的总量当「本月用量」会虚高。
//!
//! 区间边界用**日索引**（1970-01-01 为 0）表达，与 [`aggregate::local_day_index`]
//! 同一坐标系；`DailyMap` 的键是 `YYYY-MM-DD`，两者用
//! [`aggregate::days_from_civil`] 换算。

use super::aggregate;
use crate::usage::{DailyBucket, DailyMap, DayTotals};
use std::collections::BTreeMap;

/// 时间区间。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum Range {
    /// 今天。
    Today,
    /// 本周（周一为一周之始）。
    Week,
    /// 本月。
    #[default]
    Month,
    /// 全部历史。
    All,
}

impl Range {
    /// 全部区间，按界面上的展示顺序。
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

    /// 区间包含的日索引范围（闭区间）；`All` 返回 `None`（不设下界）。
    ///
    /// 上界固定为 `today`：账本里可能有未来日期的记录（系统时钟被改过、
    /// agent 写了错误的日期），它们不该混进「今日」。
    pub(in crate::app) fn day_bounds(self, today: i64) -> Option<(i64, i64)> {
        match self {
            Self::Today => Some((today, today)),
            // 周一为一周之始：`weekday_mon0` 给出 0=周一…6=周日。
            Self::Week => Some((today - i64::from(aggregate::weekday_mon0(today)), today)),
            Self::Month => {
                let (year, month, _) = aggregate::civil_from_days(today);
                Some((aggregate::days_from_civil(year, month, 1), today))
            }
            Self::All => None,
        }
    }

    /// 区间内是否包含某一天。
    ///
    /// 生产路径用 [`Range::day_bounds`] 直接枚举区间内的日子（连续区间比逐天
    /// 判断更快也更直白）；这个留给测试当断言工具，以及未来需要「判断单天」的场景。
    #[cfg(test)]
    pub(in crate::app) fn contains(self, day: i64, today: i64) -> bool {
        match self.day_bounds(today) {
            None => day <= today,
            Some((start, end)) => day >= start && day <= end,
        }
    }
}

/// 区间名 + 「用量」，用在总览卡的标题上（如「本月用量」）。
pub(in crate::app) fn totals_label(range: Range) -> &'static str {
    match range {
        Range::Today => "今日用量",
        Range::Week => "本周用量",
        Range::Month => "本月用量",
        Range::All => "总计用量",
    }
}

/// 某一天在 `DailyMap` 里的桶。
///
/// `DailyMap` 用 `YYYY-MM-DD` 字符串做键（tokscale 给的就是这个格式），
/// 而区间判断用日索引；这里按需换算，避免把 42 个键全转一遍。
fn bucket_for(daily: &DailyMap, day: i64) -> Option<&DailyBucket> {
    let (year, month, day_of_month) = aggregate::civil_from_days(day);
    let key = format!("{year:04}-{month:02}-{day_of_month:02}");
    daily.get(&key)
}

/// 把 `DailyMap` 按区间过滤成「日索引 → 桶」。
///
/// 返回 `BTreeMap` 是为了让调用方能按时间序遍历（热力图、日维度表格）。
pub(in crate::app) fn buckets_in_range(
    daily: &DailyMap,
    range: Range,
    today: i64,
) -> BTreeMap<i64, &DailyBucket> {
    let mut out = BTreeMap::new();
    match range.day_bounds(today) {
        Some((start, end)) => {
            // 区间是连续的日子，逐天取桶（区间最多一个月，开销可忽略）。
            for day in start..=end {
                if let Some(bucket) = bucket_for(daily, day) {
                    out.insert(day, bucket);
                }
            }
        }
        None => {
            // 全部：直接遍历账本里有的日子，不按区间枚举。
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

/// `YYYY-MM-DD` → 日索引；格式不对返回 `None`。
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

/// 区间内的合计（跨所有 agent）。
pub(in crate::app) fn totals_in_range(daily: &DailyMap, range: Range, today: i64) -> DayTotals {
    let mut out = DayTotals::default();
    for bucket in buckets_in_range(daily, range, today).values() {
        out.add_assign(&bucket.totals);
    }
    out
}

/// 区间内的汇总事实（总览卡用）。
///
/// 跟参考实现（tokscale 的 Token Usage 面板）一样把「总量」与「日均 / 活跃天数 /
/// 最高一天」分开：单看总量看不出频率 —— 一天猛跑 10 亿和一个月每天跑一点，
/// 总量可能一样，但使用习惯完全不同。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct RangeStats {
    /// 区间内合计。
    pub totals: DayTotals,
    /// 有量的天数（0 用量的日子不算）。
    pub active_days: usize,
    /// 最高的一天：`(日索引, 当天合计)`。区间内没有量时为 `None`。
    pub best_day: Option<(i64, i64)>,
}

impl RangeStats {
    /// 日均（按活跃天数算，不是按自然天数）。
    ///
    /// 用活跃天数而不是区间长度：后者会把没用的日子也算进去，
    /// 得到一个偏小、没有意义的平均数。
    pub(in crate::app) fn average_per_active_day(&self) -> i64 {
        if self.active_days == 0 {
            return 0;
        }
        self.totals.total() / self.active_days as i64
    }
}

/// 统计区间内的汇总事实。
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

/// 区间内按某个分解维度汇总（agent / 模型 / 会话）。
///
/// 分解表在 [`DailyBucket`] 里，键的含义由 `pick` 决定；跨天求和即可得到
/// 该区间内的正确用量 —— 这正是不能用会话总量代替的地方。
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

/// 热力图的一个格子。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct HeatCell {
    /// 日索引。
    pub day: i64,
    /// 当天合计 token。
    pub total: i64,
    /// 账本里有没有这一天的记录（没有 = 空格子，不是 0 用量）。
    pub has_data: bool,
}

/// 热力图网格：`weeks` 列 × 7 行，最后一列是本周。
///
/// 每列是一周（周一到周日），行是星期几 —— GitHub 贡献图的排法。
/// 返回的格子按列优先顺序（先第一周的 7 天，再第二周…），方便直接按行列画。
pub(in crate::app) fn heatmap_grid(daily: &DailyMap, today: i64, weeks: usize) -> Vec<HeatCell> {
    // 最后一列所在的周一；往回推 `weeks - 1` 周就是第一列的周一。
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

/// 热力图的强度分级（0-4），照 GitHub 的做法按分位数定阈。
///
/// 返回 **3 个分界值** `[t0, t1, t2]`：比 0 个阈值大是 1 级，比 1 个大是 2 级，
/// 比 2 个大是 3 级，比 3 个都大是 4 级（见 [`heat_level`]）。
///
/// 为什么不是 4 个阈值：4 级需要 3 个分界，第 4 个阈值只能是最大值本身，
/// 而「比最大值还大」永远为假 —— 最忙的那天就永远拿不到最深的一档。
///
/// 用分位数而不是固定阈值：不同 agent 的量级差几个数量级（本机 dsh 是几万、
/// opencode 是十几亿），固定阈值会让一边全白、一边全绿。
///
/// 取**最近秩**（`ceil(q * n) - 1`）而不是 `round((n - 1) * q)`：后者在样本少时
/// 会产生重复阈值（n = 4 时 q = 0.75 与 q = 0.5 都落在下标 2），而重复的阈值
/// 会让中间某一级永远取不到。
pub(in crate::app) fn heat_thresholds(values: &[i64]) -> [i64; 3] {
    let mut nonzero: Vec<i64> = values.iter().copied().filter(|v| *v > 0).collect();
    if nonzero.is_empty() {
        return [0; 3];
    }
    nonzero.sort_unstable();
    let n = nonzero.len();
    let quantile = |q: f64| -> i64 {
        // 最近秩：至少取第 1 个，至多取最后一个。
        let rank = (q * n as f64).ceil().max(1.0) as usize;
        nonzero[rank.min(n) - 1]
    };
    let mut out = [quantile(0.25), quantile(0.50), quantile(0.75)];
    // 样本很少时几个分位数会撞在一起，把重复的往后推，保证严格递增
    // （否则某一级永远是空的）。
    for index in 1..out.len() {
        if out[index] <= out[index - 1] {
            out[index] = out[index - 1].saturating_add(1);
        }
    }
    out
}

/// 某天的用量落在哪一级（0 = 无数据，1-4 逐级加深）。
///
/// 分级规则是「严格比多少个分界值大」：最大值必然比 3 个都大（阈值来自样本
/// 自身，不会超过最大值），所以最忙的那天一定是最深的一档。
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

    /// 造一个只有合计的桶。
    fn day(input: i64) -> DailyBucket {
        let mut bucket = DailyBucket::new("x");
        bucket.totals.input = input;
        bucket.totals.messages = 1;
        bucket
    }

    /// 2026-03-15 是周日；用固定日期避免测试依赖「今天」。
    fn today() -> i64 {
        aggregate::days_from_civil(2026, 3, 15)
    }

    fn daily_with(days: &[(&str, i64)]) -> DailyMap {
        days.iter()
            .map(|(key, value)| ((*key).to_string(), day(*value)))
            .collect()
    }

    /// 今日只含今天。
    #[test]
    fn today_range_covers_only_today() {
        let today = today();
        assert_eq!(Range::Today.day_bounds(today), Some((today, today)));
        assert!(Range::Today.contains(today, today));
        assert!(!Range::Today.contains(today - 1, today));
    }

    /// 本周从周一起算：2026-03-15 是周日，所以本周一是 03-09。
    #[test]
    fn week_starts_on_monday() {
        let today = today();
        let (start, end) = Range::Week.day_bounds(today).unwrap();
        assert_eq!(end, today);
        assert_eq!(aggregate::civil_from_days(start), (2026, 3, 9));
        // 周日在周内（不是下一周的开始）。
        assert!(Range::Week.contains(today, today));
        // 上周日不在。
        assert!(!Range::Week.contains(today - 7, today));
    }

    /// 本月从 1 号起算。
    #[test]
    fn month_starts_on_the_first() {
        let today = today();
        let (start, end) = Range::Month.day_bounds(today).unwrap();
        assert_eq!(end, today);
        assert_eq!(aggregate::civil_from_days(start), (2026, 3, 1));
        assert!(Range::Month.contains(start, today));
        assert!(!Range::Month.contains(start - 1, today));
    }

    /// 全部不设下界，但未来日期仍被排除。
    #[test]
    fn all_has_no_lower_bound_but_excludes_future() {
        let today = today();
        assert_eq!(Range::All.day_bounds(today), None);
        assert!(Range::All.contains(today - 10_000, today));
        assert!(!Range::All.contains(today + 1, today), "未来日期不算");
    }

    /// 区间过滤按日索引取桶。
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

    /// 区间合计只算区间内的天数。
    #[test]
    fn totals_in_range_sums_only_that_range() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 100), ("2026-03-14", 50), ("2026-02-28", 20)]);
        assert_eq!(totals_in_range(&daily, Range::Today, today).input, 100);
        assert_eq!(totals_in_range(&daily, Range::Month, today).input, 150);
        assert_eq!(totals_in_range(&daily, Range::All, today).input, 170);
    }

    /// 分解跨天求和：这是「本月 × pi」能算对的原因。
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

    /// 未来日期的记录不混进今日（系统时钟被改过时会出现）。
    #[test]
    fn future_dated_records_are_ignored() {
        let today = today();
        let daily = daily_with(&[("2026-12-31", 999)]);
        assert_eq!(totals_in_range(&daily, Range::All, today).input, 0);
        assert_eq!(totals_in_range(&daily, Range::Today, today).input, 0);
    }

    /// 日期键解析：正常、补零、非法格式。
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

    /// 网格是 7 的整数倍，且最后一格是今天所在周的周日。
    #[test]
    fn heatmap_grid_shape_and_alignment() {
        let today = today();
        let grid = heatmap_grid(&DailyMap::new(), today, 53);
        assert_eq!(grid.len(), 53 * 7);
        // 第一格是周一（53 周前的那个周一）。
        assert_eq!(aggregate::weekday_mon0(grid[0].day), 0);
        // 最后一列的周一到周日：今天是周日，所以最后一格就是今天。
        assert_eq!(grid[grid.len() - 1].day, today);
    }

    /// 未来的格子标记为无数据，不显示成 0 用量。
    ///
    /// 用一个周三做「今天」（2026-03-11）：这样本周里今天之后还有周四到周日
    /// 四格。若拿周日当今天，本周最后一格就是今天，测不到「未来格子」。
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

    /// 有数据的天数被标出来。
    #[test]
    fn heatmap_marks_days_with_data() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 100)]);
        let grid = heatmap_grid(&daily, today, 1);
        let cell = grid.iter().find(|c| c.day == today).unwrap();
        assert!(cell.has_data);
        assert_eq!(cell.total, 100);
    }

    /// 分位数阈值：0 值不参与，避免「一个高用量日」把阈值拉飞。
    #[test]
    fn thresholds_ignore_zero_days() {
        let values = [0, 0, 0, 10, 20, 30, 40];
        let t = heat_thresholds(&values);
        assert_eq!(t[0], 10, "最小值作第 1 个分界");
        assert_eq!(t[2], 30, "上四分位作第 3 个分界");
        assert!(t[0] < t[1] && t[1] < t[2], "分界值严格递增：{t:?}");
        // 最大值一定比 3 个分界都大（分界来自样本自身）。
        let max = *values.iter().max().unwrap();
        assert_eq!(heat_level(max, &t), 4, "最忙的一天要拿到最深的一档");
    }

    /// 样本很少时阈值也不能重复：重复会让中间某一级永远取不到。
    ///
    /// 这是实际踩到的坑：`round((n-1)*q)` 在 n = 4 时给出 [20,30,30,40]，
    /// 于是「3 级」要求 `> 30 && <= 30`，无解。
    #[test]
    fn thresholds_are_strictly_increasing_even_for_tiny_samples() {
        for values in [vec![5], vec![5, 9], vec![10, 20, 30, 40], vec![1, 1, 1]] {
            let t = heat_thresholds(&values);
            assert!(t[0] < t[1], "{values:?} → {t:?}");
            assert!(t[1] < t[2], "{values:?} → {t:?}");
            // 最低一级永远有样本（任何非 0 值都比 0 个分界大）。
            assert!(
                values.iter().any(|v| heat_level(*v, &t) == 1),
                "{values:?} → {t:?} 里没有 1 级的样本"
            );
            // 能填满的档数取决于取值范围：只有 1 个不同取值时只能分出 1 档，
            // 有多个取值时至少能分出 2 档。上界永远是 4。
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

    /// 全 0 时阈值全 0，不会除零。
    #[test]
    fn thresholds_of_all_zero_are_zero() {
        assert_eq!(heat_thresholds(&[0, 0, 0]), [0; 3]);
        assert_eq!(heat_thresholds(&[]), [0; 3]);
    }

    /// 分级：0 是无数据，1-4 逐级；规则是「比多少个阈值大」。
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

    /// 最忙的那天必须能拿到 4 级。
    ///
    /// 这是实际踩到的坑：早先按「逐个 `<=` 比较」分级，而 `t3` 恰好是最大值，
    /// `max > max` 永远为假，最深的一档永远空着。
    #[test]
    fn the_largest_value_reaches_level_four() {
        let values = [1, 2, 3, 4, 5, 100];
        let t = heat_thresholds(&values);
        let max = *values.iter().max().unwrap();
        assert_eq!(heat_level(max, &t), 4, "阈值 {t:?}");
    }

    /// 区间标题文案。
    #[test]
    fn totals_label_names_every_range() {
        assert_eq!(totals_label(Range::Today), "今日用量");
        assert_eq!(totals_label(Range::Week), "本周用量");
        assert_eq!(totals_label(Range::Month), "本月用量");
        assert_eq!(totals_label(Range::All), "总计用量");
    }

    // ---- 汇总事实 ----

    /// 活跃天数只数有量的天，最高一天取最大。
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

    /// 日均按活跃天数算，不是按区间长度。
    #[test]
    fn average_divides_by_active_days() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 300), ("2026-03-14", 100)]);
        let stats = range_stats(&daily, Range::All, today);
        assert_eq!(stats.average_per_active_day(), 200, "400 / 2 天");
    }

    /// 区间内没有任何量时：活跃 0 天、没有最高一天、日均 0（不除零）。
    #[test]
    fn empty_range_stats_are_zero() {
        let stats = range_stats(&DailyMap::new(), Range::Month, today());
        assert_eq!(stats.active_days, 0);
        assert_eq!(stats.best_day, None);
        assert_eq!(stats.average_per_active_day(), 0);
    }

    /// 汇总事实也受区间限制。
    #[test]
    fn range_stats_respect_the_range() {
        let today = today();
        let daily = daily_with(&[("2026-03-15", 100), ("2026-02-20", 999)]);
        let stats = range_stats(&daily, Range::Today, today);
        assert_eq!(stats.totals.input, 100, "上月的不算进来");
        assert_eq!(stats.active_days, 1);
        assert_eq!(stats.best_day.map(|(_, t)| t), Some(100));
    }
}
