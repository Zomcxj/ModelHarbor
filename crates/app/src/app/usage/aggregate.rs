//! Pure-logic layer for "Local Usage": number formatting, relative time, local date conversion, four-dimension aggregation.
//!
//! This module **never touches egui or the filesystem** (the local timezone offset is injected by the
//! caller), so everything is directly unit-testable — UI code only draws the rows computed here.

use crate::format::ConfigFormat;
use crate::usage::{DailyMap, DayTotals, SessionSnapshot};

// ---------------------------------------------------------------------------
// Number formatting
// ---------------------------------------------------------------------------

/// Round to the given number of decimal places.
fn round_to(value: f64, decimals: usize) -> f64 {
    let factor = 10f64.powi(decimals as i32);
    (value * factor).round() / factor
}

/// Compact token counts: `999` / `12.3K` / `45.6M` / `1.23B`.
///
/// Unit thresholds are decimal (1000 / 1_000_000 / 1_000_000_000); K and M keep 1 decimal, B keeps 2.
///
/// Carry rule: if rounding hits the next unit (`999_999` → `1000.0K`), carry straight through to
/// `1.0M` — `1000.0K` is unreadable and looks like a bug.
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

/// Full number (thousands separators): `561,023,456`.
///
/// Used by the overview card: the compact form (K/M) sacrifices precision, and the user asked for the
/// full number in the first card.
pub(in crate::app) fn format_full(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if n < 0 {
        out.push('-');
    }
    for (index, byte) in digits.bytes().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(byte as char);
    }
    out
}

/// **Unified M unit** (millions of tokens) for charts and hover tips.
///
/// Axis ticks mixing `500.0K / 1.2M / 2.0B` can't be compared at a glance (user feedback); once
/// everything is in M the scale is linear and bar heights correspond directly to numbers. Below 10M
/// one decimal is kept, above that it's rounded — ticks like "2030.0M" are unreadable.
pub(in crate::app) fn format_m(n: i64) -> String {
    let millions = n.max(0) as f64 / 1_000_000.0;
    if millions >= 10.0 {
        format!("{millions:.0}M")
    } else {
        format!("{millions:.1}M")
    }
}

// ---------------------------------------------------------------------------
// Relative time
// ---------------------------------------------------------------------------

/// A cached display older than this is flagged "stale" (aligned with token-monitor's 5-minute threshold).
pub(in crate::app) const CACHED_STALE_MS: i64 = 5 * 60 * 1000;

/// Status-line text for the cached state (pure logic for easy unit tests):
/// `{sessions} sessions · cached · updated {age}[ · stale][ · updating…]`.
pub(in crate::app) fn cached_status_line(
    sessions: usize,
    saved_ms: Option<i64>,
    now_ms: i64,
    refreshing: bool,
) -> String {
    let mut line = match saved_ms {
        Some(saved_ms) => {
            let mut line = format!(
                "{sessions} 个会话 · 缓存 · 更新于 {}",
                relative_time(saved_ms, now_ms)
            );
            if now_ms - saved_ms > CACHED_STALE_MS {
                line.push_str(" · 已过期");
            }
            line
        }
        None => format!("{sessions} 个会话 · 缓存"),
    };
    if refreshing {
        line.push_str(" · 正在更新…");
    }
    line
}

/// The "N minutes ago" text for `last_seen_ms` relative to `now_ms`.
///
/// Times in the future (clock rollback, or a source timestamp ahead of this machine) are rendered as
/// "just now" — never as negative numbers.
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
// Local dates
// ---------------------------------------------------------------------------

/// Unix seconds → local "day number" (local midnight divided into whole days).
///
/// `offset_secs` is the local timezone's offset from UTC (UTC+8 = 28800). The offset is injected by
/// the caller rather than read from the system clock here, so the "day split" logic can be pinned by
/// unit tests in any timezone.
pub(in crate::app) fn local_day_index(unix_secs: i64, offset_secs: i64) -> i64 {
    unix_secs.saturating_add(offset_secs).div_euclid(86_400)
}

/// "Day number" (1970-01-01 = 0) → civil (year, month, day).
///
/// Howard Hinnant's `civil_from_days`, the inverse of `days_from_civil` in `core::billing`; `chrono`
/// is not in the dependency tree, so it's computed here.
pub(in crate::app) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // Era starts at 0000-03-01: leap day lands at year's end, so month lengths depend only on month_prime.
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

/// Day index → display date text.
///
/// Today / yesterday get plain words; anything earlier shows `YYYY-MM-DD`.
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

/// Civil (year, month, day) → day index (1970-01-01 = 0). Inverse of [`civil_from_days`].
///
/// Range filtering (today / week / month) needs month starts and Mondays — all "a civil date" →
/// "a day number". Same source as the one in `core::profiles`, but that one is private, and the usage
/// view shouldn't pry open `profiles`' API for five lines of arithmetic.
pub(in crate::app) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Era at 0000-03-01: leap day lands at year's end, so month lengths depend only on month_prime.
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Day index → weekday, **Monday = 0** through Sunday = 6.
///
/// 1970-01-01 was a Thursday, so the offset is 3 (`(0 + 3) % 7 == 3` → Thursday).
/// Uses `div_euclid` instead of `%`: with negative day indexes, `%` returns negative values.
pub(in crate::app) fn weekday_mon0(day_index: i64) -> u8 {
    (day_index + 3).rem_euclid(7) as u8
}

// ---------------------------------------------------------------------------
// Dimension switching
// ---------------------------------------------------------------------------

/// The usage statistics' dimensions.
///
/// The time dimension is absent here: the by-day heatmap (`range::heatmap_grid`) already covers it,
/// and a time table would duplicate it (user feedback: drop the time tab).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum Dimension {
    /// Group by agent (backend).
    #[default]
    Agent,
    /// Group by model.
    Model,
    /// One row per session.
    Session,
}

impl Dimension {
    /// All dimensions, in the UI's display order.
    ///
    /// Test-only: production draws the three tables on screen together and no longer enumerates dimensions.
    #[cfg(test)]
    pub(in crate::app) const ALL: [Dimension; 3] =
        [Dimension::Agent, Dimension::Model, Dimension::Session];

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Agent => "Agent",
            Self::Model => "模型",
            Self::Session => "会话",
        }
    }
}

// ---------------------------------------------------------------------------
// Four-dimension aggregation
// ---------------------------------------------------------------------------

/// Usage totals for a group of sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct Totals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    /// Session count.
    pub sessions: usize,
}

impl Totals {
    /// Total tokens. Same accounting as `SessionSnapshot::total`: `cache_write` is a subset and isn't added twice.
    pub(in crate::app) fn total(&self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }

    /// Absorb one session.
    ///
    /// Test-only: production goes through the daily buckets ([`totals_from_day`]), which has no "session
    /// count" to add — in per-day terms one session falls into several days and summing would double-count.
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

/// One row of the Agent dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct AgentUsage {
    pub client: ConfigFormat,
    pub totals: Totals,
    pub last_seen_ms: i64,
}

/// One row of the model dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct ModelUsage {
    pub model_id: String,
    pub totals: Totals,
    pub last_seen_ms: i64,
    /// The agents that used this model (deduped, name-sorted).
    pub agents: Vec<String>,
}

/// Descending by token total; ties sort by ascending name so the UI doesn't jump.
fn sort_by_total<T, F>(rows: &mut [T], total: F, name: impl Fn(&T) -> &str)
where
    F: Fn(&T) -> i64,
{
    rows.sort_by(|a, b| total(b).cmp(&total(a)).then_with(|| name(a).cmp(name(b))));
}

/// Display form of a session id: over-long ids truncate the middle, keeping head and tail recognizable.
pub(in crate::app) fn short_session_id(session_id: &str, max_chars: usize) -> String {
    let chars: Vec<char> = session_id.chars().collect();
    if chars.len() <= max_chars || max_chars < 5 {
        return session_id.to_string();
    }
    // Keep head and tail with an ellipsis between: `01968b50…4fb1db6e` matches up far more easily than a
    // head-only cut. The ellipsis takes 1 character; head and tail split the remaining budget (head takes
    // the smaller side, so short strings preferentially keep their tail differences).
    let budget = max_chars.saturating_sub(1);
    let head = budget / 2;
    let tail = budget - head;
    let prefix: String = chars[..head].iter().collect();
    let suffix: String = chars[chars.len() - tail..].iter().collect();
    format!("{prefix}…{suffix}")
}

// ---------------------------------------------------------------------------
// Aggregation by range
// ---------------------------------------------------------------------------
//
// The four dimensions above take `&[SessionSnapshot]` (full, timeless); the four below take
// `&DailyMap` + a range — the correct algorithm under time filtering. Keeping both is intentional:
// on the "all" range the results agree, but session-level sums cannot answer "this month × one agent" —
// a session may span dozens of days, and its lifetime total belongs to no single month.

/// [`DayTotals`] → [`Totals`].
///
/// The two types live in core and app respectively: core's daily buckets only track token counts,
/// while app's `Totals` also carries a "session count" field (in per-day terms one session falls into
/// several days and must not be summed), so session count stays 0 here, filled by the caller as needed.
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

/// Map `label` back to [`ConfigFormat`] (the ledger's agent breakdown keys on labels).
fn format_from_label(label: &str) -> Option<ConfigFormat> {
    crate::backends::BACKENDS
        .iter()
        .map(|backend| backend.id())
        .find(|format| format.label() == label)
}

/// Agent dimension (within range).
pub(in crate::app) fn by_agent_in_range(
    daily: &DailyMap,
    range: super::range::Range,
    today: i64,
) -> Vec<AgentUsage> {
    let rows = super::range::breakdown_in_range(daily, range, today, |bucket| &bucket.by_client);
    let mut out: Vec<AgentUsage> = rows
        .iter()
        .filter_map(|(label, totals)| {
            // Labels in the ledger come from ConfigFormat::label() and should always map back;
            // if a model/agent rename breaks the mapping, dropping the row beats guessing.
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

/// Model dimension (within range).
pub(in crate::app) fn by_model_in_range(
    daily: &DailyMap,
    range: super::range::Range,
    today: i64,
) -> Vec<ModelUsage> {
    // The agents behind each model come from the agent × model cross-breakdown.
    // Pairing "the day's agent set" with "the day's model set" would credit models used only by agent A
    // to agent B just because both appeared that day.
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

/// Session dimension (within range): only sessions active in the range, with their in-range usage.
pub(in crate::app) fn session_rows_in_range(
    sessions: &[SessionSnapshot],
    daily: &DailyMap,
    range: super::range::Range,
    today: i64,
) -> Vec<SessionSnapshot> {
    let rows = super::range::breakdown_in_range(daily, range, today, |bucket| &bucket.by_session);
    // Template from the ledger's snapshot (carrying model name and last activity), but with in-range usage —
    // exactly why session lifetime totals can't be used directly.
    //
    // Sessions gone from the sources (`archived`) build no template: they stay out of the list. Their
    // per-day `by_session` entries still exist (their usage stays in the aggregates), so an explicit guard
    // sits here — otherwise a different `view()` implementation would let them resurface.
    let by_key: std::collections::HashMap<String, &SessionSnapshot> = sessions
        .iter()
        .filter(|snapshot| !snapshot.archived)
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
            snapshot.cost = totals.cost;
            Some(snapshot)
        })
        .collect();
    // Descending by in-range usage (not recent activity): under time filtering the user cares about "how much in this period".
    out.sort_by(|a, b| {
        b.total()
            .cmp(&a.total())
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    out
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
            project: None,
            title: None,

            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 1,
            cost: 0.0,
            first_seen_ms: 0,
            last_seen_ms: 1_700_000_000_000,
            archived: false,
        }
    }

    // ---- format_full ----
    #[test]
    fn full_numbers_get_thousands_separators() {
        assert_eq!(format_full(0), "0");
        assert_eq!(format_full(561), "561");
        assert_eq!(format_full(123_456_789), "123,456,789");
        assert_eq!(format_full(-98_765), "-98,765");
    }

    // ---- format_tokens ----

    #[test]
    fn format_tokens_boundaries() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1_000), "1.0K");
        // 999_999 rounds to 1000.0K in K: carry to M rather than display an unreadable number.
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
        // Negative numbers from sources are dirty data; display as 0, never `-3`.
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

    // ---- Local dates ----

    #[test]
    fn local_day_index_shifts_by_the_timezone_offset() {
        // 1970-01-01 00:00:00 UTC
        assert_eq!(local_day_index(0, 0), 0);
        // UTC+8: UTC 16:00 is the local next-day midnight.
        assert_eq!(local_day_index(16 * 3_600 - 1, 8 * 3_600), 0);
        assert_eq!(local_day_index(16 * 3_600, 8 * 3_600), 1);
        // The same timestamp lands a day earlier in UTC+8 than in UTC.
        assert_eq!(local_day_index(15 * 3_600, 0), 0);
        assert_eq!(local_day_index(15 * 3_600, 8 * 3_600), 0);
        assert_eq!(local_day_index(17 * 3_600, 8 * 3_600), 1);
        // In UTC-5, 1969-12-31 20:00 UTC already falls on day -1.
        assert_eq!(local_day_index(0, -5 * 3_600), -1);
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        // 2024-02-29 (leap day)
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        // 2024-03-01
        assert_eq!(civil_from_days(19_783), (2024, 3, 1));
        // 2000-03-01 (century leap rule: 2000 is a leap year)
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        // Negative day index (before 1970)
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

    // ---- Aggregation by range ----

    /// Build a bucket: `(client, session, model, input, output, cache_read)`.
    ///
    /// All four breakdown tables get filled (matching core's `DailyBucket::add_message`) so the tests exercise
    /// "reading the breakdown tables", not the shape of my hand-rolled fake data.
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

    /// Data equivalent to the old `sample()`, but as daily buckets: three agents, two models, two days.
    ///
    /// 2026-03-15 is "today", 2026-03-14 is "yesterday".
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

    /// Today = 2026-03-15, aligned with `daily_sample`.
    fn sample_today() -> i64 {
        days_from_civil(2026, 3, 15)
    }

    #[test]
    fn by_agent_groups_and_sorts_by_total() {
        let rows = by_agent_in_range(&daily_sample(), Range::All, sample_today());
        assert_eq!(rows.len(), 3, "三个 agent 各一行");
        // opencode's total 2500 is the largest — first place.
        assert_eq!(rows[0].client, ConfigFormat::Opencode);
        assert_eq!(rows[0].totals.total(), 2_500);
        assert_eq!(rows[0].totals.cache_read, 300);
        // pi's two sessions merge into one row.
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
        // pi's two sessions plus workbuddy's all count.
        assert_eq!(gpt.totals.total(), 1_660);
        assert_eq!(gpt.agents, vec!["pi".to_string(), "workbuddy".to_string()]);
        // Only opencode touched claude-4.
        let claude = rows
            .iter()
            .find(|row| row.model_id == "claude-4")
            .expect("claude-4 应有一行");
        assert_eq!(claude.agents, vec!["opencode".to_string()]);
    }

    /// The model dimension's source agents come from the cross-breakdown, not "every agent that day".
    ///
    /// This day pi used gpt-5 and opencode used claude-4: gpt-5's source can only be pi. Pairing with "the
    /// day's agent set" would yield pi + opencode — wrong.
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

    /// The session dimension: only sessions active in the range, with their in-range usage.
    #[test]
    fn session_rows_in_range_scopes_usage_to_the_range() {
        // Today's active sessions are pi-a / pi-b / oc-a (see `daily_sample`).
        // Session keys embed the client, so the client here must match the bucket's.
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
        // Descending by in-range usage: opencode 2500 > pi-a 1100 > pi-b 550.
        assert_eq!(rows[0].session_id, "oc-a");
    }

    /// A multi-day session counts only its in-range share — why lifetime totals can't be used directly.
    #[test]
    fn session_usage_is_clipped_to_the_range() {
        // The same session was active both days: 100 today, 900 yesterday.
        let today = bucket_of("2026-03-15", &[(ConfigFormat::Pi, "long", "m", 100, 0, 0)]);
        let yesterday = bucket_of("2026-03-14", &[(ConfigFormat::Pi, "long", "m", 900, 0, 0)]);
        let daily: DailyMap = [
            ("2026-03-15".to_string(), today),
            ("2026-03-14".to_string(), yesterday),
        ]
        .into_iter()
        .collect();
        // The ledger's snapshot records the session's lifetime total (1000).
        let sessions = vec![snapshot("long", "m", 1_000, 0)];

        let rows = session_rows_in_range(&sessions, &daily, Range::Today, sample_today());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total(), 100, "只算今天的部分，不是一生的 1000");
    }

    /// Sessions gone from the sources stay out of the list (**the core requirement**):
    /// even with entries remaining in the range's per-day `by_session`.
    #[test]
    fn session_rows_in_range_hide_archived_sessions() {
        let mut gone = snapshot("gone", "gpt-5", 7, 3);
        gone.client = ConfigFormat::WorkBuddy;
        gone.archived = true;
        // Its usage exists in the range (the aggregates must keep it), but listing the session won't do.
        let rows = session_rows_in_range(&[gone], &daily_sample(), Range::All, sample_today());
        assert!(
            rows.is_empty(),
            "源里已消失的会话不该占列表行（用量仍在聚合里）"
        );
    }

    /// Sessions with no activity in the range don't appear in the list.
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
        assert!(session_rows_in_range(&[], &DailyMap::new(), Range::All, 0).is_empty());
    }

    #[test]
    fn every_dimension_is_listed_once() {
        assert_eq!(Dimension::ALL.len(), 3);
        let labels: Vec<&str> = Dimension::ALL.iter().map(|d| d.label()).collect();
        assert_eq!(labels, vec!["Agent", "模型", "会话"]);
        assert_eq!(Dimension::default(), Dimension::Agent);
    }
}
