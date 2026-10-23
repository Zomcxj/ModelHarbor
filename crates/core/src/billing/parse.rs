//! Response parsing: compatible billing, panel status, account, check-in, call logs.
use super::*;
use serde_json::Value;

/// Parse `/api/status`'s panel name (`system_name` + version); an empty string on failure.
pub fn parse_panel(status_json: Option<&str>) -> String {
    let Some(text) = status_json else {
        return String::new();
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return String::new();
    };
    let data = root.get("data").unwrap_or(&root);
    let name = data
        .get("system_name")
        .or_else(|| data.get("systemName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = data.get("version").and_then(Value::as_str).unwrap_or("");
    match (name.is_empty(), version.is_empty()) {
        (true, true) => String::new(),
        (false, true) => name.to_string(),
        (true, false) => version.to_string(),
        (false, false) => format!("{} {}", name, version),
    }
}

/// Aggregate parsing of subscription + usage + station info.
pub fn parse(
    subscription_json: &str,
    usage_json: Option<&str>,
    status_json: Option<&str>,
) -> Billing {
    let mut out = Billing {
        panel: parse_panel(status_json),
        ..Default::default()
    };
    let Ok(sub) = serde_json::from_str::<Value>(subscription_json) else {
        return out;
    };

    // Non-standard shape: display the raw value directly, no unit conversion.
    if sub.get("object").and_then(Value::as_str) == Some("credit_summary")
        || sub.get("total_granted").is_some()
    {
        out.shape = Shape::CreditSummary;
        out.raw_credit = Some(format!(
            "总额度 {} / 已用 {} / 可用 {}（单位未标注，按原值展示）",
            raw_num(sub.get("total_granted")),
            raw_num(sub.get("total_used")),
            raw_num(sub.get("total_available")),
        ));
        return out;
    }

    // The regular OpenAI-compatible billing.
    // `hard_limit_usd` of 0 / missing means no hard cap, so `limit_usd` is `None`.
    out.limit_usd = sub
        .get("hard_limit_usd")
        .and_then(Value::as_f64)
        .or_else(|| sub.get("soft_limit_usd").and_then(Value::as_f64))
        .filter(|limit| *limit > 0.0);
    out.placeholder_limit = out
        .limit_usd
        .is_some_and(|limit| limit >= PLACEHOLDER_LIMIT_USD);
    // Placeholder quotas produce no balance number.
    if out.placeholder_limit {
        out.limit_usd = None;
    }
    if let Some(text) = usage_json {
        // total_usage's unit is cents.
        out.used_usd = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|usage| usage.get("total_usage").and_then(Value::as_f64))
            .map(|cents| cents / 100.0);
    }
    if let (Some(limit), Some(used)) = (out.limit_usd, out.used_usd) {
        // Quota smaller than usage (inconsistent data or overspent) yields no balance.
        let balance = limit - used;
        if balance >= 0.0 {
            out.balance_usd = Some(balance);
        }
    }
    // Subscription expiry (`access_until`, -1 / 0 = no expiry).
    out.expires = sub
        .get("access_until")
        .and_then(Value::as_f64)
        .filter(|ts| *ts > 0.0)
        .map(|ts| KeyExpiry {
            ts: ts as i64,
            ..Default::default()
        });
    out.shape = if out.limit_usd.is_some() || out.used_usd.is_some() {
        Shape::Subscription
    } else {
        Shape::Unknown
    };
    out
}

/// Parse `/api/user/self` (requires the panel access token / PAT).
///
/// `data.quota` is remaining quota and `used_quota` is lifetime used; both are quota points, converted to
/// money via the [`Units`] given by `/api/status`.
///
/// Returns `None` when: not JSON / root isn't an object; `success` explicitly false; or not a single usable
/// field exists (non-numeric fields like `group` don't count as data).
pub fn parse_account_self(json: &str, units: &Units) -> Option<AccountInfo> {
    let root = serde_json::from_str::<Value>(json).ok()?;
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    // Stations return fields under `data`; a few variants put them at the root.
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&root);
    if !data.is_object() {
        return None;
    }
    let quota = data.get("quota").and_then(Value::as_f64);
    let used = data.get("used_quota").and_then(Value::as_f64);
    // Counts tolerate both integer and float spellings.
    let requests = data.get("request_count").and_then(value_as_u64);
    // All None means this isn't usable account data.
    if quota.is_none() && used.is_none() && requests.is_none() {
        return None;
    }
    let to_money = |points: f64| points / units.quota_per_unit;
    Some(AccountInfo {
        balance_usd: quota.map(to_money),
        used_usd: used.map(to_money),
        requests,
        group: data
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// Read one non-negative integer (tolerating both integer and float spellings).
fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
}

/// Check-in status (`GET /api/user/checkin`; only the panel token / PAT, read-only).
///
/// Disabled stations answer `200 + success:false`; [`parse_checkin_status`] returns the station's own words as the error.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckinStatus {
    /// Whether the station enables check-in.
    pub enabled: bool,
    /// Whether today is already checked in.
    pub today: bool,
    /// Today's check-in amount (USD): the station gives per-day entries only in `records`; take the greatest date.
    /// `None` when today isn't checked in (or the station gives no details).
    pub today_usd: Option<f64>,
    /// Check-ins this month.
    pub month_count: u64,
    /// Lifetime check-ins.
    pub total_count: u64,
    /// Lifetime amount earned (USD); `None` when the station doesn't say.
    pub total_usd: Option<f64>,
    /// Per-check-in amount range (USD); `None` when unconfigured (0).
    pub min_usd: Option<f64>,
    pub max_usd: Option<f64>,
}

/// Take the station's own error message (`message` field), with a fallback line when absent.
fn site_message(root: &Value, fallback: &str) -> String {
    root.get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// Parse `GET /api/user/checkin`.
///
/// All failure modes return `Err` (not `None`): disabled, rejected, unrecognized structure.
pub fn parse_checkin_status(json: &str, units: &Units) -> Result<CheckinStatus, String> {
    let root = serde_json::from_str::<Value>(json).map_err(|_| "返回内容不是 JSON".to_string())?;
    if !root.is_object() {
        return Err("返回内容不是可识别的签到状态".to_string());
    }
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(site_message(&root, "站点拒绝了签到状态查询"));
    }
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| "返回内容不是可识别的签到状态".to_string())?;
    let stats = data.get("stats").filter(|value| value.is_object());
    let count = |key: &str| {
        stats
            .and_then(|stats| stats.get(key))
            .and_then(value_as_u64)
            .unwrap_or(0)
    };
    let to_money = |points: f64| points / units.quota_per_unit;
    let today = stats
        .and_then(|stats| stats.get("checked_in_today"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Today's amount: the `records` entry with the greatest date (`YYYY-MM-DD` lexicographic = chronological),
    // and "today" counts only when `checked_in_today` is true.
    let today_usd = if today {
        stats
            .and_then(|stats| stats.get("records"))
            .and_then(Value::as_array)
            .and_then(|records| {
                records
                    .iter()
                    .filter_map(|record| {
                        let date = record.get("checkin_date").and_then(Value::as_str)?;
                        let quota = record.get("quota_awarded").and_then(Value::as_f64)?;
                        Some((date, quota))
                    })
                    .max_by_key(|(date, _)| *date)
                    .map(|(_, quota)| quota)
            })
            .map(to_money)
    } else {
        None
    };
    // 0 means the station didn't configure this — not an amount.
    let positive_usd = |key: &str| {
        data.get(key)
            .and_then(Value::as_f64)
            .filter(|points| *points > 0.0)
            .map(to_money)
    };
    Ok(CheckinStatus {
        enabled: data
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        today,
        today_usd,
        month_count: count("checkin_count"),
        total_count: count("total_checkins"),
        total_usd: stats
            .and_then(|stats| stats.get("total_quota"))
            .and_then(Value::as_f64)
            .filter(|points| *points > 0.0)
            .map(to_money),
        min_usd: positive_usd("min_quota"),
        max_usd: positive_usd("max_quota"),
    })
}

/// Short description of check-in status (for the card's line).
///
/// Numbers the station didn't provide are never mentioned.
pub fn checkin_short(status: &CheckinStatus) -> String {
    if !status.enabled {
        return "未启用".to_string();
    }
    if !status.today {
        return "未签".to_string();
    }
    match status.today_usd {
        Some(today) => format!("今日已签 {}", money(today)),
        None => "今日已签".to_string(),
    }
}

/// Full description of check-in status (for the hover note).
pub fn checkin_long(status: &CheckinStatus) -> String {
    if !status.enabled {
        return "该站点未启用签到".to_string();
    }
    let mut text = checkin_short(status);
    let mut extra: Vec<String> = Vec::new();
    if status.month_count > 0 {
        extra.push(format!("本月 {} 次", status.month_count));
    }
    if let Some(total) = status.total_usd {
        extra.push(format!("累计获得 {}", money(total)));
    }
    if !extra.is_empty() {
        text.push_str(&format!("（{}）", extra.join("，")));
    }
    text
}

/// One call-log entry (only the fields statistics need).#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    /// When it happened (second-resolution Unix timestamp).
    pub at: i64,
    /// The spend (quota points).
    pub quota: f64,
    pub model: String,
}

/// Parse `/api/log/token`'s log list (`{success, data:[...]}`; missing `data` is an empty table).
pub fn parse_token_logs(json: &str) -> Vec<LogEntry> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(items) = root.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let at = item.get("created_at").and_then(Value::as_i64)?;
            Some(LogEntry {
                at,
                quota: item.get("quota").and_then(Value::as_f64).unwrap_or(0.0),
                model: item
                    .get("model_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// Log aggregation result (`from` of `None` = everything).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogSummary {
    pub count: usize,
    pub calls: u64,
    /// In-range spend (quota points).
    pub quota: f64,
    /// Per-model summary (quota descending, at most [`MODEL_TOP`] entries).
    pub models: Vec<(String, f64)>,
}

/// Max models listed in the hover.
pub const MODEL_TOP: usize = 3;

/// Aggregate logs by a time floor (all timestamps in seconds).
pub fn summarize_logs(entries: &[LogEntry], from: Option<i64>) -> LogSummary {
    let mut summary = LogSummary::default();
    let mut per_model: Vec<(String, f64)> = Vec::new();
    for entry in entries {
        if from.is_some_and(|from| entry.at < from) {
            continue;
        }
        summary.count += 1;
        summary.calls += 1;
        summary.quota += entry.quota;
        match per_model.iter_mut().find(|(name, _)| name == &entry.model) {
            Some((_, quota)) => *quota += entry.quota,
            None => per_model.push((entry.model.clone(), entry.quota)),
        }
    }
    per_model.sort_by(|a, b| b.1.total_cmp(&a.1));
    per_model.truncate(MODEL_TOP);
    summary.models = per_model;
    summary
}

/// Parse `GET /api/data/self`: the per-day × model usage array.
///
/// Returns single-day aggregates in ascending date order (model rows merged into the day's total + the
/// heaviest model's name); `success:false`, missing / empty / unrecognized `data` returns `None`.
/// `created_at` is the station's per-day midnight timestamp; date labels are filled by the app layer.
pub fn parse_data_self(json: &str, units: &Units) -> Option<Vec<PanelDay>> {
    let root = serde_json::from_str::<Value>(json).ok()?;
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let rows = root.get("data")?.as_array()?;
    if rows.is_empty() {
        return None;
    }
    let to_money = |points: f64| points / units.quota_per_unit;
    // (day timestamp) → (USD, count, tokens, model → quota)
    let mut acc: std::collections::BTreeMap<
        i64,
        (f64, u64, i64, std::collections::BTreeMap<String, f64>),
    > = std::collections::BTreeMap::new();
    for row in rows {
        let day_ts = row.get("created_at").and_then(Value::as_i64)?;
        let quota = row.get("quota").and_then(Value::as_f64).unwrap_or(0.0);
        let calls = row.get("count").and_then(value_as_u64).unwrap_or(0);
        let tokens = row.get("token_used").and_then(Value::as_i64).unwrap_or(0);
        let model = row
            .get("model_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let entry = acc.entry(day_ts).or_default();
        entry.0 += to_money(quota);
        entry.1 += calls;
        entry.2 += tokens;
        entry
            .3
            .entry(model)
            .and_modify(|sum| *sum += quota)
            .or_insert(quota);
    }
    Some(
        acc.into_iter()
            .map(|(day_ts, (usd, calls, tokens, models))| {
                let (top_model, _) = models
                    .into_iter()
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap_or_default();
                PanelDay {
                    day_ts,
                    usd,
                    calls,
                    tokens,
                    top_model,
                    ..PanelDay::default()
                }
            })
            .collect(),
    )
}

/// Parse Sub2API's `/v1/usage` response: `{ quota: {limit, used}, remaining|balance, unit }`.
///
/// All three readings may be missing (sub2api versions differ); any one suffices for success;
/// a missing remaining falls back to `total - used`. Unrecognized structures return `None`.
pub fn parse_sub2api_usage(text: &str) -> Option<Sub2apiUsage> {
    let root = serde_json::from_str::<Value>(text).ok()?;
    let quota = root.get("quota");
    let total = quota.and_then(|q| q.get("limit")).and_then(Value::as_f64);
    let used = quota.and_then(|q| q.get("used")).and_then(Value::as_f64);
    let remaining = root
        .get("remaining")
        .and_then(Value::as_f64)
        .or_else(|| root.get("balance").and_then(Value::as_f64))
        .or_else(|| {
            quota
                .and_then(|q| q.get("remaining"))
                .and_then(Value::as_f64)
        })
        .or(match (total, used) {
            (Some(total), Some(used)) => Some(total - used),
            _ => None,
        });
    if remaining.is_none() && total.is_none() && used.is_none() {
        return None;
    }
    let unit = root
        .get("unit")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("USD")
        .to_string();
    Some(Sub2apiUsage {
        remaining,
        used,
        total,
        unit,
    })
}
