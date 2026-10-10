//! 「本机用量」的纯逻辑层：数字格式化、相对时间、本地日期换算、四维聚合。
//!
//! 这里**不碰 egui、不碰文件系统**（本地时区偏移由调用方注入），所以全部可以
//! 直接单测 —— 界面代码只负责把这里算出来的行画到表里。

use crate::format::ConfigFormat;
use crate::usage::{DailyMap, DayTotals, SessionSnapshot};

// ---------------------------------------------------------------------------
// 数字格式化
// ---------------------------------------------------------------------------

/// 四舍五入到指定小数位。
fn round_to(value: f64, decimals: usize) -> f64 {
    let factor = 10f64.powi(decimals as i32);
    (value * factor).round() / factor
}

/// token 数的紧凑写法：`999` / `12.3K` / `45.6M` / `1.23B`。
///
/// 单位阈值按 10 进制（1000 / 1_000_000 / 1_000_000_000），K 与 M 保留 1 位、
/// B 保留 2 位。
///
/// 进位规则：四舍五入后如果顶到了下一个单位（`999_999` → `1000.0K`），
/// 就直接进位成 `1.0M` —— `1000.0K` 这种数字读不出来，也容易让人以为出了 bug。
pub(in crate::app) fn format_tokens(n: i64) -> String {
    let n = n.max(0);
    if n < 1_000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        let k = round_to(n as f64 / 1e3, 1);
        if k < 1_000.0 {
            return format!("{k:.1}K");
        }
    }
    if n < 1_000_000_000 {
        let m = round_to(n as f64 / 1e6, 1);
        if m < 1_000.0 {
            return format!("{m:.1}M");
        }
    }
    format!("{:.2}B", round_to(n as f64 / 1e9, 2))
}

// ---------------------------------------------------------------------------
// 相对时间
// ---------------------------------------------------------------------------

/// `last_seen_ms` 相对 `now_ms` 的「N 分钟前」文案。
///
/// 时间在未来（时钟回拨、或源里的时间戳比本机快）时按「刚刚」处理，
/// 不显示负数。
pub(in crate::app) fn relative_time(last_seen_ms: i64, now_ms: i64) -> String {
    let delta_secs = (now_ms - last_seen_ms).max(0) / 1_000;
    if delta_secs < 60 {
        return "刚刚".to_string();
    }
    let minutes = delta_secs / 60;
    if minutes < 60 {
        return format!("{minutes} 分钟前");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} 小时前");
    }
    format!("{} 天前", hours / 24)
}

// ---------------------------------------------------------------------------
// 本地日期
// ---------------------------------------------------------------------------

/// 秒级时间戳 → 本地「第几天」（本地时区的 0 点整除一天）。
///
/// `offset_secs` 是本地时区相对 UTC 的偏移（东八区 = 28800）。偏移由调用方
/// 注入而不是在这里读系统时间，这样「分天」逻辑可以在任何时区下被单测钉住。
pub(in crate::app) fn local_day_index(unix_secs: i64, offset_secs: i64) -> i64 {
    unix_secs.saturating_add(offset_secs).div_euclid(86_400)
}

/// 「第几天」（1970-01-01 为 0）→ 公历年月日。
///
/// Howard Hinnant 的 `civil_from_days`，与 `core::billing` 里 `days_from_civil`
/// 的逆运算同源；`chrono` 不在依赖树里，所以自己算。
pub(in crate::app) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // 以 0000-03-01 为纪元起点：这样闰日落在年末，月份长度只跟 month_prime 有关。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year + i64::from(month <= 2), month, day)
}

/// 日索引 → 展示用日期文案。
///
/// 今天 / 昨天直接说人话，更早的给 `YYYY-MM-DD`。
pub(in crate::app) fn day_label(day_index: i64, today_index: i64) -> String {
    match today_index - day_index {
        0 => "今天".to_string(),
        1 => "昨天".to_string(),
        _ => {
            let (year, month, day) = civil_from_days(day_index);
            format!("{year:04}-{month:02}-{day:02}")
        }
    }
}

/// 公历年月日 → 日索引（1970-01-01 为 0）。[`civil_from_days`] 的逆运算。
///
/// 区间筛选（今日 / 本周 / 本月）要算月初和周一，都是「某个年月日」→
/// 「第几天」的方向。与 `core::profiles` 里那份同源，但那个是私有函数，
/// 而 usage 视图不该为一个 5 行的纯算术把 `profiles` 的接口撬开。
pub(in crate::app) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // 以 0000-03-01 为纪元：闰日落在年末，月份长度只跟 month_prime 有关。
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// 日索引 → 星期几，**周一为 0**、周日为 6。
///
/// 1970-01-01 是周四，所以偏移量是 3（`(0 + 3) % 7 == 3` → 周四）。
/// 用 `div_euclid` 而不是 `%`：负数日索引下 `%` 会返回负值。
pub(in crate::app) fn weekday_mon0(day_index: i64) -> u8 {
    (day_index + 3).rem_euclid(7) as u8
}

// ---------------------------------------------------------------------------
// 四维切换
// ---------------------------------------------------------------------------

/// 用量统计的四个维度。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum Dimension {
    /// 按 agent（后端）分组。
    #[default]
    Agent,
    /// 按模型分组。
    Model,
    /// 一条会话一行。
    Session,
    /// 按本地日期分组。
    Day,
}

impl Dimension {
    /// 全部维度，按界面上的展示顺序。
    pub(in crate::app) const ALL: [Dimension; 4] = [
        Dimension::Agent,
        Dimension::Model,
        Dimension::Session,
        Dimension::Day,
    ];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Model => "模型",
            Self::Session => "会话",
            Self::Day => "时间",
        }
    }
}

// ---------------------------------------------------------------------------
// 四维聚合
// ---------------------------------------------------------------------------

/// 一组会话的用量合计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct Totals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    /// 会话条数。
    pub sessions: usize,
}

impl Totals {
    /// 合计 token。口径与 `SessionSnapshot::total` 一致：`cache_write` 是子集，不重复加。
    pub(in crate::app) fn total(&self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }

    /// 并入一个会话。
    ///
    /// 只用于测试：生产路径走按天分桶（[`totals_from_day`]），那里没有
    /// 「会话数」可加 —— 按天分解时一个会话会落进多天，相加会重复计数。
    #[cfg(test)]
    pub(in crate::app) fn add(&mut self, snapshot: &SessionSnapshot) {
        self.input = self.input.saturating_add(snapshot.input);
        self.output = self.output.saturating_add(snapshot.output);
        self.cache_read = self.cache_read.saturating_add(snapshot.cache_read);
        self.cache_write = self.cache_write.saturating_add(snapshot.cache_write);
        self.reasoning = self.reasoning.saturating_add(snapshot.reasoning);
        self.sessions += 1;
    }
}

/// Agent 维度的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct AgentUsage {
    pub client: ConfigFormat,
    pub totals: Totals,
    pub last_seen_ms: i64,
}

/// 模型维度的一行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct ModelUsage {
    pub model_id: String,
    pub totals: Totals,
    pub last_seen_ms: i64,
    /// 用到这个模型的 agent（去重、按名字排序）。
    pub agents: Vec<String>,
}

/// 时间维度的一行（按本地日期分组）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct DayUsage {
    /// 本地日索引（[`local_day_index`]）。
    pub day: i64,
    pub totals: Totals,
    pub last_seen_ms: i64,
}

/// 按 token 合计倒序；合计相同按名字正序，保证界面不跳动。
fn sort_by_total<T, F>(rows: &mut [T], total: F, name: impl Fn(&T) -> &str)
where
    F: Fn(&T) -> i64,
{
    rows.sort_by(|a, b| total(b).cmp(&total(a)).then_with(|| name(a).cmp(name(b))));
}

/// 会话 id 的展示写法：太长时截断中间，保留头尾便于辨认。
pub(in crate::app) fn short_session_id(session_id: &str, max_chars: usize) -> String {
    let chars: Vec<char> = session_id.chars().collect();
    if chars.len() <= max_chars || max_chars < 5 {
        return session_id.to_string();
    }
    // 留头留尾，中间用省略号：`01968b50…4fb1db6e` 比只留头部更容易对上号。
    // 省略号占 1 个字符，头尾平分剩下的额度（头取少的一边，短串优先保留尾部差异）。
    let budget = max_chars.saturating_sub(1);
    let head = budget / 2;
    let tail = budget - head;
    let prefix: String = chars[..head].iter().collect();
    let suffix: String = chars[chars.len() - tail..].iter().collect();
    format!("{prefix}…{suffix}")
}

// ---------------------------------------------------------------------------
// 按区间聚合
// ---------------------------------------------------------------------------
//
// 上面四个维度都吃 `&[SessionSnapshot]`（全量、不分时间）；下面这四个吃
// `&DailyMap` + 区间，是时间筛选下的正确算法。两套并存是有意的：
// 「全部」区间下两者结果一致，但会话级汇总做不到「本月 × 某 agent」——
// 一个会话可以横跨几十天，它的总量不属于任何单独一个月。

/// [`DayTotals`] → [`Totals`]。
///
/// 两个类型分属 core 与 app：core 的按天桶只关心 token 计数，app 的 `Totals`
/// 还带一个「会话条数」字段（按天分解时一个会话会落进多天，不能相加），
/// 所以这里会话数留 0，由调用方按需填。
fn totals_from_day(day: &DayTotals) -> Totals {
    Totals {
        input: day.input,
        output: day.output,
        cache_read: day.cache_read,
        cache_write: day.cache_write,
        reasoning: day.reasoning,
        sessions: 0,
    }
}

/// 把 `label` 映射回 [`ConfigFormat`]（账本里 agent 分解用 label 做键）。
fn format_from_label(label: &str) -> Option<ConfigFormat> {
    crate::backends::BACKENDS
        .iter()
        .map(|backend| backend.id())
        .find(|format| format.label() == label)
}

/// Agent 维度（区间内）。
pub(in crate::app) fn by_agent_in_range(
    daily: &DailyMap,
    range: super::range::Range,
    today: i64,
) -> Vec<AgentUsage> {
    let rows = super::range::breakdown_in_range(daily, range, today, |bucket| &bucket.by_client);
    let mut out: Vec<AgentUsage> = rows
        .iter()
        .filter_map(|(label, totals)| {
            // 账本里的 label 来自 ConfigFormat::label()，理论上一定能映射回来；
            // 万一模型/agent 改名导致对不上，宁可丢掉这一行也不要瞎猜。
            let client = format_from_label(label)?;
            Some(AgentUsage {
                client,
                totals: totals_from_day(totals),
                last_seen_ms: 0,
            })
        })
        .collect();
    sort_by_total(&mut out, |row| row.totals.total(), |row| row.client.label());
    out
}

/// 模型维度（区间内）。
pub(in crate::app) fn by_model_in_range(
    daily: &DailyMap,
    range: super::range::Range,
    today: i64,
) -> Vec<ModelUsage> {
    // 用到每个模型的 agent：读 agent × 模型交叉分解。
    // 不能拿「当天的 agent 集合」去配「当天的模型集合」—— 那会把只在
    // agent A 用过的模型也算到同一天用过的 agent B 头上。
    let mut agents: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for bucket in super::range::buckets_in_range(daily, range, today).values() {
        for (client, models) in &bucket.by_client_model {
            for model in models.keys() {
                agents
                    .entry(model.clone())
                    .or_default()
                    .insert(client.clone());
            }
        }
    }

    let rows = super::range::breakdown_in_range(daily, range, today, |bucket| &bucket.by_model);
    let mut out: Vec<ModelUsage> = rows
        .into_iter()
        .map(|(model_id, totals)| ModelUsage {
            agents: agents
                .get(&model_id)
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default(),
            model_id,
            totals: totals_from_day(&totals),
            last_seen_ms: 0,
        })
        .collect();
    sort_by_total(&mut out, |row| row.totals.total(), |row| &row.model_id);
    out
}

/// 会话维度（区间内）：只列在区间内有活动的会话，用量取区间内的部分。
pub(in crate::app) fn session_rows_in_range(
    sessions: &[SessionSnapshot],
    daily: &DailyMap,
    range: super::range::Range,
    today: i64,
) -> Vec<SessionSnapshot> {
    let rows = super::range::breakdown_in_range(daily, range, today, |bucket| &bucket.by_session);
    // 以账本里的快照为模板（带 archived 标记、模型名、最近活动），
    // 但用量换成区间内的 —— 这正是不能直接用会话总量的原因。
    let by_key: std::collections::HashMap<String, &SessionSnapshot> = sessions
        .iter()
        .map(|snapshot| {
            (
                SessionSnapshot::key(snapshot.client, &snapshot.session_id),
                snapshot,
            )
        })
        .collect();

    let mut out: Vec<SessionSnapshot> = rows
        .into_iter()
        .filter_map(|(key, totals)| {
            let template = by_key.get(&key)?;
            let mut snapshot = (*template).clone();
            snapshot.input = totals.input;
            snapshot.output = totals.output;
            snapshot.cache_read = totals.cache_read;
            snapshot.cache_write = totals.cache_write;
            snapshot.reasoning = totals.reasoning;
            snapshot.message_count = totals.messages;
            Some(snapshot)
        })
        .collect();
    // 按区间内用量倒序（不是最近活动）：时间筛选下用户关心的是「这段时间用了多少」。
    out.sort_by(|a, b| {
        b.total()
            .cmp(&a.total())
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    out
}

/// 时间维度（区间内）：一天一行。
pub(in crate::app) fn by_day_in_range(
    daily: &DailyMap,
    range: super::range::Range,
    today: i64,
) -> Vec<DayUsage> {
    // 日期倒序：最近的在前（时间轴读起来才顺，不按 token 多少排）。
    super::range::buckets_in_range(daily, range, today)
        .into_iter()
        .rev()
        .map(|(day, bucket)| DayUsage {
            day,
            totals: totals_from_day(&bucket.totals),
            last_seen_ms: 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::usage::range::Range;
    use crate::usage::DailyBucket;

    fn snapshot(session: &str, model: &str, input: i64, output: i64) -> SessionSnapshot {
        SessionSnapshot {
            client: ConfigFormat::Pi,
            session_id: session.to_string(),
            model_id: model.to_string(),
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 1,
            first_seen_ms: 0,
            last_seen_ms: 1_700_000_000_000,
            archived: false,
        }
    }

    // ---- format_tokens ----

    #[test]
    fn format_tokens_boundaries() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1.0K");
        // 999_999 四舍五入到 K 是 1000.0K：进位到 M，不显示读不出来的数字。
        assert_eq!(format_tokens(999_999), "1.0M");
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(1_234_567_890), "1.23B");
    }

    #[test]
    fn format_tokens_keeps_the_middle_units_readable() {
        assert_eq!(format_tokens(1_500), "1.5K");
        assert_eq!(format_tokens(12_345), "12.3K");
        assert_eq!(format_tokens(45_600_000), "45.6M");
        assert_eq!(format_tokens(999_000), "999.0K");
        assert_eq!(format_tokens(1_500_000_000), "1.50B");
    }

    #[test]
    fn format_tokens_never_shows_a_negative() {
        // 源里出现负数是脏数据，界面上按 0 显示，不显示 `-3`。
        assert_eq!(format_tokens(-3), "0");
    }

    // ---- relative_time ----

    #[test]
    fn relative_time_boundaries() {
        let now = 1_700_000_000_000i64;
        assert_eq!(relative_time(now, now), "刚刚");
        assert_eq!(relative_time(now - 59_000, now), "刚刚");
        assert_eq!(relative_time(now - 60_000, now), "1 分钟前");
        assert_eq!(relative_time(now - 59 * 60_000, now), "59 分钟前");
        assert_eq!(relative_time(now - 60 * 60_000, now), "1 小时前");
        assert_eq!(relative_time(now - 23 * 3_600_000, now), "23 小时前");
        assert_eq!(relative_time(now - 24 * 3_600_000, now), "1 天前");
        assert_eq!(relative_time(now - 5 * 86_400_000, now), "5 天前");
    }

    #[test]
    fn relative_time_treats_the_future_as_just_now() {
        let now = 1_700_000_000_000i64;
        assert_eq!(relative_time(now + 600_000, now), "刚刚");
    }

    // ---- 本地日期 ----

    #[test]
    fn local_day_index_shifts_by_the_timezone_offset() {
        // 1970-01-01 00:00:00 UTC
        assert_eq!(local_day_index(0, 0), 0);
        // 东八区：UTC 16:00 才是本地次日 0 点。
        assert_eq!(local_day_index(16 * 3_600 - 1, 8 * 3_600), 0);
        assert_eq!(local_day_index(16 * 3_600, 8 * 3_600), 1);
        // 同一个时间戳在东八区比 UTC 早一天。
        assert_eq!(local_day_index(15 * 3_600, 0), 0);
        assert_eq!(local_day_index(15 * 3_600, 8 * 3_600), 0);
        assert_eq!(local_day_index(17 * 3_600, 8 * 3_600), 1);
        // 西五区的 1969-12-31 20:00 UTC 已经落在第 -1 天。
        assert_eq!(local_day_index(0, -5 * 3_600), -1);
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        // 2024-02-29（闰日）
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        // 2024-03-01
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
        // 2000-03-01（百年闰规则：2000 是闰年）
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        // 负日索引（1970 之前）
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn day_label_says_today_and_yesterday() {
        let today = civil_from_days(19_783).0;
        assert_eq!(today, 2024);
        assert_eq!(day_label(19_783, 19_783), "今天");
        assert_eq!(day_label(19_782, 19_783), "昨天");
        assert_eq!(day_label(19_781, 19_783), "2024-02-28");
    }

    // ---- 按区间聚合 ----

    /// 造一个桶：`(client, session, model, input, output, cache_read)`。
    ///
    /// 四个分解表都填上（与 core 的 `DailyBucket::add_message` 保持一致），
    /// 这样测的是「读分解表」的逻辑而不是我自己手搓的假数据形状。
    fn bucket_of(date: &str, rows: &[(ConfigFormat, &str, &str, i64, i64, i64)]) -> DailyBucket {
        let mut bucket = DailyBucket::new(date);
        for (client, session, model, input, output, cache_read) in rows {
            let totals = DayTotals {
                input: *input,
                output: *output,
                cache_read: *cache_read,
                messages: 1,
                ..Default::default()
            };
            bucket.totals.add_assign(&totals);
            bucket
                .by_client
                .entry(client.label().to_string())
                .or_default()
                .add_assign(&totals);
            bucket
                .by_model
                .entry((*model).to_string())
                .or_default()
                .add_assign(&totals);
            bucket
                .by_session
                .entry(SessionSnapshot::key(*client, session))
                .or_default()
                .add_assign(&totals);
            bucket
                .by_client_model
                .entry(client.label().to_string())
                .or_default()
                .entry((*model).to_string())
                .or_default()
                .add_assign(&totals);
        }
        bucket
    }

    /// 与旧 `sample()` 等价的数据，但按天分桶：三个 agent、两个模型、两天。
    ///
    /// 2026-03-15 是「今天」，2026-03-14 是「昨天」。
    fn daily_sample() -> DailyMap {
        let today = bucket_of(
            "2026-03-15",
            &[
                (ConfigFormat::Pi, "pi-a", "gpt-5", 1_000, 100, 0),
                (ConfigFormat::Pi, "pi-b", "gpt-5", 500, 50, 0),
                (ConfigFormat::Opencode, "oc-a", "claude-4", 2_000, 200, 300),
            ],
        );
        let yesterday = bucket_of(
            "2026-03-14",
            &[(ConfigFormat::WorkBuddy, "gone", "gpt-5", 7, 3, 0)],
        );
        [
            ("2026-03-15".to_string(), today),
            ("2026-03-14".to_string(), yesterday),
        ]
        .into_iter()
        .collect()
    }

    /// 今天 = 2026-03-15，与 `daily_sample` 对齐。
    fn sample_today() -> i64 {
        days_from_civil(2026, 3, 15)
    }

    #[test]
    fn by_agent_groups_and_sorts_by_total() {
        let rows = by_agent_in_range(&daily_sample(), Range::All, sample_today());
        assert_eq!(rows.len(), 3, "三个 agent 各一行");
        // opencode 合计 2500 最大，排第一。
        assert_eq!(rows[0].client, ConfigFormat::Opencode);
        assert_eq!(rows[0].totals.total(), 2_500);
        assert_eq!(rows[0].totals.cache_read, 300);
        // pi 两个会话合并成一行。
        assert_eq!(rows[1].client, ConfigFormat::Pi);
        assert_eq!(rows[1].totals.total(), 1_650);
        assert_eq!(rows[2].client, ConfigFormat::WorkBuddy);
        assert_eq!(rows[2].totals.total(), 10);
    }

    #[test]
    fn by_model_merges_the_same_model_across_agents() {
        let rows = by_model_in_range(&daily_sample(), Range::All, sample_today());
        let gpt = rows
            .iter()
            .find(|row| row.model_id == "gpt-5")
            .expect("gpt-5 应有一行");
        // pi 的两个会话 + workbuddy 的会话都算进来。
        assert_eq!(gpt.totals.total(), 1_660);
        assert_eq!(gpt.agents, vec!["pi".to_string(), "workbuddy".to_string()]);
        // claude-4 只有 opencode 用到。
        let claude = rows
            .iter()
            .find(|row| row.model_id == "claude-4")
            .expect("claude-4 应有一行");
        assert_eq!(claude.agents, vec!["opencode".to_string()]);
    }

    /// 模型维度的来源 agent 取自交叉分解，不是「当天所有 agent」。
    ///
    /// 这一天 pi 用 gpt-5、opencode 用 claude-4：gpt-5 的来源只能是 pi。
    /// 用「当天的 agent 集合」去配会得到 pi + opencode，那是错的。
    #[test]
    fn by_model_does_not_attribute_a_model_to_unrelated_agents() {
        let rows = by_model_in_range(&daily_sample(), Range::Today, sample_today());
        let gpt = rows.iter().find(|row| row.model_id == "gpt-5").unwrap();
        assert_eq!(
            gpt.agents,
            vec!["pi".to_string()],
            "opencode 当天没用 gpt-5"
        );
    }

    #[test]
    fn by_day_groups_on_the_calendar_day() {
        let rows = by_day_in_range(&daily_sample(), Range::All, sample_today());
        assert_eq!(rows.len(), 2, "两天各一行");
        // 最近的一天排前面。
        assert!(rows[0].day > rows[1].day);
        assert_eq!(rows[0].totals.total(), 4_150);
        assert_eq!(rows[1].totals.total(), 10);
    }

    /// 区间筛选真的把范围外的天排除了。
    #[test]
    fn by_day_respects_the_range() {
        let today = sample_today();
        let today_only = by_day_in_range(&daily_sample(), Range::Today, today);
        assert_eq!(today_only.len(), 1);
        assert_eq!(today_only[0].day, today);
        assert_eq!(today_only[0].totals.total(), 4_150, "昨天那 10 不算进来");
    }

    /// 会话维度：只列区间内有活动的会话，用量取区间内的部分。
    #[test]
    fn session_rows_in_range_scopes_usage_to_the_range() {
        // 今天有活动的是 pi-a / pi-b / oc-a（见 `daily_sample`）。
        // 会话键含 client，所以这里必须用与桶一致的 client。
        let mut oc = snapshot("oc-a", "claude-4", 2_000, 200);
        oc.client = ConfigFormat::Opencode;
        let sessions = vec![
            snapshot("pi-a", "gpt-5", 1_000, 100),
            snapshot("pi-b", "gpt-5", 500, 50),
            oc,
        ];
        let rows = session_rows_in_range(&sessions, &daily_sample(), Range::Today, sample_today());
        assert_eq!(rows.len(), 3, "今天三个会话都有活动");
        let pi_a = rows.iter().find(|r| r.session_id == "pi-a").unwrap();
        assert_eq!(pi_a.total(), 1_100, "区间内用量");
        let oc = rows.iter().find(|r| r.session_id == "oc-a").unwrap();
        assert_eq!(oc.total(), 2_500, "opencode 的 cache_read 也算进去");
        // 按区间内用量倒序：opencode 2500 > pi-a 1100 > pi-b 550。
        assert_eq!(rows[0].session_id, "oc-a");
    }

    /// 跨天会话在区间内只算区间那部分 —— 这是不能直接用会话总量的原因。
    #[test]
    fn session_usage_is_clipped_to_the_range() {
        // 同一个会话在两天都有活动：今天 100，昨天 900。
        let today = bucket_of("2026-03-15", &[(ConfigFormat::Pi, "long", "m", 100, 0, 0)]);
        let yesterday = bucket_of("2026-03-14", &[(ConfigFormat::Pi, "long", "m", 900, 0, 0)]);
        let daily: DailyMap = [
            ("2026-03-15".to_string(), today),
            ("2026-03-14".to_string(), yesterday),
        ]
        .into_iter()
        .collect();
        // 账本里的快照记的是这个会话的一生总量（1000）。
        let sessions = vec![snapshot("long", "m", 1_000, 0)];

        let rows = session_rows_in_range(&sessions, &daily, Range::Today, sample_today());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total(), 100, "只算今天的部分，不是一生的 1000");
    }

    /// 归档会话的标记要带出来给界面画角标。
    #[test]
    fn session_rows_in_range_keep_the_archived_mark() {
        let mut archived = snapshot("gone", "gpt-5", 7, 3);
        archived.client = ConfigFormat::WorkBuddy;
        archived.archived = true;
        let rows = session_rows_in_range(&[archived], &daily_sample(), Range::All, sample_today());
        let row = rows.iter().find(|r| r.session_id == "gone").unwrap();
        assert!(row.archived, "archived 标记要带出来");
    }

    /// 区间内没有活动的会话不出现在列表里。
    #[test]
    fn session_rows_in_range_hides_sessions_outside_the_range() {
        let sessions = vec![snapshot("gone", "gpt-5", 7, 3)];
        let rows = session_rows_in_range(&sessions, &daily_sample(), Range::Today, sample_today());
        assert!(
            rows.iter().all(|r| r.session_id != "gone"),
            "昨天才有活动的会话不该出现在「今日」里"
        );
    }

    #[test]
    fn short_session_id_truncates_the_middle_only_when_needed() {
        assert_eq!(short_session_id("abc", 8), "abc");
        assert_eq!(short_session_id("abcdefgh", 8), "abcdefgh");
        let short = short_session_id("01968b50-0179-4ba0-8eca-7c4a4fb1db6e", 12);
        assert_eq!(short.chars().count(), 12);
        assert!(short.contains('…'), "{short}");
        assert!(short.starts_with("0196"), "{short}");
        assert!(short.ends_with("db6e"), "{short}");
    }

    #[test]
    fn totals_total_ignores_nothing_but_sums_every_bucket() {
        let mut totals = Totals::default();
        let mut s = snapshot("s", "m", 1, 2);
        s.cache_read = 4;
        s.cache_write = 8;
        s.reasoning = 16;
        totals.add(&s);
        assert_eq!(totals.total(), 31);
        assert_eq!(totals.sessions, 1);
    }

    #[test]
    fn empty_input_produces_no_rows() {
        assert!(by_agent_in_range(&DailyMap::new(), Range::All, 0).is_empty());
        assert!(by_model_in_range(&DailyMap::new(), Range::All, 0).is_empty());
        assert!(by_day_in_range(&DailyMap::new(), Range::All, 0).is_empty());
        assert!(session_rows_in_range(&[], &DailyMap::new(), Range::All, 0).is_empty());
    }

    #[test]
    fn every_dimension_is_listed_once() {
        assert_eq!(Dimension::ALL.len(), 4);
        let labels: Vec<&str> = Dimension::ALL.iter().map(|d| d.label()).collect();
        assert_eq!(labels, vec!["Agent", "模型", "会话", "时间"]);
        assert_eq!(Dimension::default(), Dimension::Agent);
    }
}
