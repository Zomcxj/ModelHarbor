//! 「本机用量」的纯逻辑层：数字格式化、相对时间、本地日期换算、四维聚合。
//!
//! 这里**不碰 egui、不碰文件系统**（本地时区偏移由调用方注入），所以全部可以
//! 直接单测 —— 界面代码只负责把这里算出来的行画到表里。

use crate::format::ConfigFormat;
use crate::usage::SessionSnapshot;

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

/// Agent 维度：按 `snapshot.client` 分组。
pub(in crate::app) fn by_agent(sessions: &[SessionSnapshot]) -> Vec<AgentUsage> {
    let mut rows: Vec<AgentUsage> = Vec::new();
    for snapshot in sessions {
        match rows.iter_mut().find(|row| row.client == snapshot.client) {
            Some(row) => {
                row.totals.add(snapshot);
                row.last_seen_ms = row.last_seen_ms.max(snapshot.last_seen_ms);
            }
            None => {
                let mut totals = Totals::default();
                totals.add(snapshot);
                rows.push(AgentUsage {
                    client: snapshot.client,
                    totals,
                    last_seen_ms: snapshot.last_seen_ms,
                });
            }
        }
    }
    sort_by_total(
        &mut rows,
        |row| row.totals.total(),
        |row| row.client.label(),
    );
    rows
}

/// 模型维度：按 `snapshot.model_id` 分组。
///
/// 同一个模型可能被多个 agent 用到，所以每行额外记下「用到它的 agent」。
/// 模型名为空（源里没写）时归到 [`UNKNOWN_MODEL`]。
pub(in crate::app) fn by_model(sessions: &[SessionSnapshot]) -> Vec<ModelUsage> {
    /// 源里读不到模型名时的占位。
    const UNKNOWN_MODEL: &str = "(未知模型)";
    let mut rows: Vec<ModelUsage> = Vec::new();
    for snapshot in sessions {
        let model = if snapshot.model_id.trim().is_empty() {
            UNKNOWN_MODEL
        } else {
            snapshot.model_id.as_str()
        };
        match rows.iter_mut().find(|row| row.model_id == model) {
            Some(row) => {
                row.totals.add(snapshot);
                row.last_seen_ms = row.last_seen_ms.max(snapshot.last_seen_ms);
                let agent = snapshot.client.label();
                if !row.agents.iter().any(|name| name == agent) {
                    row.agents.push(agent.to_string());
                }
            }
            None => {
                let mut totals = Totals::default();
                totals.add(snapshot);
                rows.push(ModelUsage {
                    model_id: model.to_string(),
                    totals,
                    last_seen_ms: snapshot.last_seen_ms,
                    agents: vec![snapshot.client.label().to_string()],
                });
            }
        }
    }
    for row in &mut rows {
        row.agents.sort();
    }
    sort_by_total(
        &mut rows,
        |row| row.totals.total(),
        |row| row.model_id.as_str(),
    );
    rows
}

/// 时间维度：按 `last_seen_ms` 的本地日期分组。
///
/// `offset_secs` 是本地时区相对 UTC 的偏移（见 [`local_day_index`]）。
pub(in crate::app) fn by_day(sessions: &[SessionSnapshot], offset_secs: i64) -> Vec<DayUsage> {
    let mut rows: Vec<DayUsage> = Vec::new();
    for snapshot in sessions {
        let day = local_day_index(snapshot.last_seen_ms / 1_000, offset_secs);
        match rows.iter_mut().find(|row| row.day == day) {
            Some(row) => {
                row.totals.add(snapshot);
                row.last_seen_ms = row.last_seen_ms.max(snapshot.last_seen_ms);
            }
            None => {
                let mut totals = Totals::default();
                totals.add(snapshot);
                rows.push(DayUsage {
                    day,
                    totals,
                    last_seen_ms: snapshot.last_seen_ms,
                });
            }
        }
    }
    // 日期倒序：最近的在前，不按 token 多少排（时间轴读起来才顺）。
    rows.sort_by_key(|row| std::cmp::Reverse(row.day));
    rows
}

/// 会话维度：一条会话一行。
///
/// 直接用快照本身（`archived` 标记随行带到界面）。账本已经按最近活动排好序，
/// 这里再排一次，让「手工构造的输入」也有确定顺序。
pub(in crate::app) fn session_rows(sessions: &[SessionSnapshot]) -> Vec<SessionSnapshot> {
    let mut rows = sessions.to_vec();
    rows.sort_by(|a, b| {
        b.last_seen_ms
            .cmp(&a.last_seen_ms)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    rows
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

#[cfg(test)]
mod tests {
    use super::*;

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

    // ---- 聚合 ----

    /// 三个 agent、两个模型、两天的一组会话。
    fn sample() -> Vec<SessionSnapshot> {
        let mut pi_a = snapshot("pi-a", "gpt-5", 1_000, 100);
        pi_a.client = ConfigFormat::Pi;
        pi_a.last_seen_ms = 1_700_000_000_000;
        let mut pi_b = snapshot("pi-b", "gpt-5", 500, 50);
        pi_b.client = ConfigFormat::Pi;
        pi_b.last_seen_ms = 1_700_000_000_000;
        let mut oc = snapshot("oc-a", "claude-4", 2_000, 200);
        oc.client = ConfigFormat::Opencode;
        oc.cache_read = 300;
        oc.last_seen_ms = 1_700_000_100_000;
        let mut archived = snapshot("gone", "gpt-5", 7, 3);
        archived.client = ConfigFormat::WorkBuddy;
        archived.archived = true;
        archived.last_seen_ms = 1_600_000_000_000;
        vec![pi_a, pi_b, oc, archived]
    }

    #[test]
    fn by_agent_groups_and_sorts_by_total() {
        let rows = by_agent(&sample());
        assert_eq!(rows.len(), 3, "三个 agent 各一行");
        // opencode 合计 2500 最大，排第一。
        assert_eq!(rows[0].client, ConfigFormat::Opencode);
        assert_eq!(rows[0].totals.total(), 2_500);
        assert_eq!(rows[0].totals.sessions, 1);
        assert_eq!(rows[0].totals.cache_read, 300);
        // pi 两个会话合并成一行。
        assert_eq!(rows[1].client, ConfigFormat::Pi);
        assert_eq!(rows[1].totals.sessions, 2);
        assert_eq!(rows[1].totals.total(), 1_650);
        assert_eq!(rows[1].last_seen_ms, 1_700_000_000_000);
        assert_eq!(rows[2].client, ConfigFormat::WorkBuddy);
        assert_eq!(rows[2].totals.total(), 10);
    }

    #[test]
    fn by_model_merges_the_same_model_across_agents() {
        let rows = by_model(&sample());
        let gpt = rows
            .iter()
            .find(|row| row.model_id == "gpt-5")
            .expect("gpt-5 应有一行");
        // pi 的两个会话 + workbuddy 的归档会话都算进来。
        assert_eq!(gpt.totals.sessions, 3);
        assert_eq!(gpt.totals.total(), 1_660);
        assert_eq!(gpt.agents, vec!["pi".to_string(), "workbuddy".to_string()]);
        // claude-4 只有 opencode 用到。
        let claude = rows
            .iter()
            .find(|row| row.model_id == "claude-4")
            .expect("claude-4 应有一行");
        assert_eq!(claude.agents, vec!["opencode".to_string()]);
    }

    #[test]
    fn by_model_falls_back_to_a_placeholder_for_a_blank_name() {
        let mut s = snapshot("x", "   ", 10, 1);
        s.model_id = String::new();
        let rows = by_model(&[s]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, "(未知模型)");
    }

    #[test]
    fn by_day_groups_on_the_local_calendar_day() {
        let utc = by_day(&sample(), 0);
        assert_eq!(utc.len(), 2, "两组时间戳在 UTC 下是两天");
        // 最近的一天排前面，三个实时会话都在这一天。
        assert!(utc[0].day > utc[1].day);
        assert_eq!(utc[0].totals.sessions, 3);
        assert_eq!(utc[0].totals.total(), 4_150);
        // 归档的那个会话落在更早的一天。
        assert_eq!(utc[1].totals.sessions, 1);
        assert_eq!(utc[1].totals.total(), 10);
    }

    #[test]
    fn by_day_uses_the_offset_to_split_days() {
        // UTC 22:00：UTC 下仍是当天，东八区（+28800）已是次日 06:00。
        let mut s = snapshot("s", "m", 100, 0);
        s.last_seen_ms = 1_699_999_200_000;
        let utc = by_day(&[s.clone()], 0);
        let east8 = by_day(&[s], 8 * 3_600);
        assert_eq!(utc[0].day, local_day_index(1_699_999_200, 0));
        assert_eq!(east8[0].day, local_day_index(1_699_999_200, 8 * 3_600));
        assert_eq!(
            east8[0].day,
            utc[0].day + 1,
            "同一时间戳在东八区应落到后一天"
        );
    }

    #[test]
    fn session_rows_keep_the_archived_mark_and_sort_by_recency() {
        let rows = session_rows(&sample());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].session_id, "oc-a", "最近的排最前");
        let archived = rows
            .iter()
            .find(|row| row.session_id == "gone")
            .expect("归档会话必须出现在会话维度里");
        assert!(archived.archived, "archived 标记要带出来给界面画角标");
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
        assert!(by_agent(&[]).is_empty());
        assert!(by_model(&[]).is_empty());
        assert!(by_day(&[], 0).is_empty());
        assert!(session_rows(&[]).is_empty());
    }

    #[test]
    fn every_dimension_is_listed_once() {
        assert_eq!(Dimension::ALL.len(), 4);
        let labels: Vec<&str> = Dimension::ALL.iter().map(|d| d.label()).collect();
        assert_eq!(labels, vec!["Agent", "模型", "会话", "时间"]);
        assert_eq!(Dimension::default(), Dimension::Agent);
    }
}
