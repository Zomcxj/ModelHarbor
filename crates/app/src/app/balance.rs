//! Relay-station "used / balance / today's usage" queries: background thread + mpsc channel.
//!
//! Two paths (by priority):
//! - **Token quota** (only needs the `sk-` key): `/api/usage/token/` + `/api/log/token`;
//! - **Compatible billing** (the `sk-` key): `/dashboard/billing/subscription` + `/usage`.
//!
//! Constraints: direct connections (a fresh Agent per query); all endpoints are read-only management
//! APIs; two queries to the same provider are spaced at least [`BALANCE_COOLDOWN_MS`] apart;
//! credentials live only in request headers — never in logs or status-bar text.

use super::fetch::{apply_auth, latency_agent, AuthKind};
use super::App;
use crate::billing;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Minimum interval between two queries to the same provider (milliseconds).
pub(super) const BALANCE_COOLDOWN_MS: f64 = 5_000.0;

/// Input for one query (owned, so it can be moved into the background thread).
#[derive(Clone, Default)]
pub(super) struct Query {
    pub(super) key: String,
    pub(super) base_url: String,
    /// The `sk-`-style API key (used by the compatible billing endpoint).
    pub(super) secret: String,
    /// The panel access token (PAT, optional): required for account-level quota (`/api/user/self`).
    pub(super) pat: String,
    /// The user ID required by older new-api (the `New-Api-User` header); empty = the header isn't sent.
    pub(super) user_id: String,
    /// The station marked sub2api: query `{base}/v1/usage` directly with the `sk-` key,
    /// skipping the NewAPI panel system (token quota / compatible billing / panel account).
    pub(super) sub2api: bool,
}

/// Per-provider user-data query state.
#[derive(Default)]
pub(super) struct BalanceState {
    pub(super) result: Option<Result<billing::Billing, String>>,
    pub(super) rx: Option<Receiver<Result<billing::Billing, String>>>,
    /// When the last query was issued (egui timeline, seconds).
    pub(super) last_at: Option<f64>,
    /// This successful result's local-midnight marker; crossing to the next day hides `today_*`.
    pub(super) snapshot_midnight: Option<i64>,
}

impl App {
    /// Issue one user-data query.
    ///
    /// Returns `Some(note)` when no request was issued (already querying, or still cooling down).
    pub(super) fn start_balance_query(
        balance: &mut HashMap<String, BalanceState>,
        query: Query,
        now: f64,
    ) -> Option<String> {
        let key = query.key.clone();
        let state = balance.entry(key.clone()).or_default();
        if state.rx.is_some() {
            return Some(format!("{} 的用户数据查询还在进行中", key));
        }
        if let Some(last) = state.last_at {
            let remain = BALANCE_COOLDOWN_MS / 1000.0 - (now - last);
            if remain > 0.0 {
                return Some(format!("{} 刚查过用量，请 {:.1} 秒后再试", key, remain));
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_billing(&query));
        });
        state.result = None;
        state.snapshot_midnight = None;
        state.rx = Some(rx);
        state.last_at = Some(now);
        None
    }

    /// Harvest user-data query results (called every frame in `update`).
    pub(super) fn poll_balance(&mut self) {
        let mut finished = 0usize;
        for state in self.balance.values_mut() {
            let Some(rx) = state.rx.as_ref() else {
                continue;
            };
            match rx.try_recv() {
                Ok(result) => {
                    state.snapshot_midnight = result
                        .as_ref()
                        .ok()
                        .filter(|billing| billing.source == billing::Source::Token)
                        .and_then(|_| local_midnight_unix());
                    state.result = Some(result);
                    state.rx = None;
                    finished += 1;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    // The thread died abnormally: end the wait.
                    state.rx = None;
                }
            }
        }
        if finished == 0 || !self.balance_batch {
            return;
        }
        // Query all: one summary lands after everything finishes.
        if self.balance.values().any(|state| state.rx.is_some()) {
            return;
        }
        let total = self.balance.len();
        let ok = self
            .balance
            .values()
            .filter(|state| matches!(state.result, Some(Ok(_))))
            .count();
        self.balance_batch = false;
        // Report counts only, never reasons.
        self.status = format!("用户数据查询完成：{} 个查到，{} 个没查到", ok, total - ok);
    }
}

impl BalanceState {
    /// Return a display copy per the current local date; crossing midnight only invalidates "today", keeping lifetime / balance / last-7-days.
    pub(super) fn display_result(
        &self,
        current_midnight: Option<i64>,
    ) -> Option<Result<billing::Billing, String>> {
        let mut result = self.result.clone()?;
        if let Ok(info) = &mut result {
            let stale = self
                .snapshot_midnight
                .zip(current_midnight)
                .is_some_and(|(snapshot, current)| current > snapshot);
            if stale {
                info.expire_today();
            }
        }
        Some(result)
    }
}

/// Query a station's usage: token-quota endpoints first, falling back to compatible billing on failure;
/// with a panel token filled in, account-level quota is fetched on top.
/// When the station is marked sub2api, only `{base}/v1/usage` is queried (no panel token needed).
fn fetch_billing(query: &Query) -> Result<billing::Billing, String> {
    // sub2api stations have no NewAPI panel system; their own usage endpoint is used directly.
    if query.sub2api {
        return fetch_sub2api(&query.base_url, &query.secret);
    }
    let endpoints = billing::endpoints(&query.base_url);
    // Station info (panel name) and the conversion ratio; failure is tolerated.
    let status = http_get(&endpoints.status, None).ok();
    let units = billing::parse_units(status.as_deref());
    let mut result = match fetch_token(&endpoints, &units, &query.secret, status.as_deref()) {
        Some(info) => Ok(info),
        None => fetch_compat(&query.base_url, &query.secret, status.as_deref()),
    };
    // The panel token is an optional enhancement: failure only appends one explanatory line to the details.
    let pat = query.pat.trim();
    if !pat.is_empty() {
        let account = fetch_account(&endpoints, &units, pat, &query.user_id);
        let checkin = fetch_checkin(&endpoints, &units, pat, &query.user_id);
        let panel_days = fetch_panel_days(&endpoints, &units, pat, &query.user_id);
        let panel = billing::parse_panel(status.as_deref());
        merge_account(&mut result, account, panel.clone());
        merge_checkin(&mut result, checkin, panel);
        merge_panel_days(&mut result, panel_days);
    }
    // Expiry labels and urgency: both the token path and the compatible-billing path attach them.
    fill_expiry_labels(&mut result);
    result
}

/// Sub2API station usage: `GET {base}/v1/usage` (only the `sk-` key, Bearer auth).
///
/// Returns station-level remaining / used / total; there is no panel-account or check-in concept, and
/// `/api/status` is not queried.
fn fetch_sub2api(base_url: &str, secret: &str) -> Result<billing::Billing, String> {
    let secret = secret.trim();
    if secret.is_empty() {
        return Err("sub2api 站点需要 provider 的 API Key（sk-…）才能查余额".to_string());
    }
    let url = billing::sub2api_usage_url(base_url);
    let text = http_get(&url, Some(secret))?;
    let usage = billing::parse_sub2api_usage(&text)
        .ok_or_else(|| "返回内容不是可识别的 sub2api 用量".to_string())?;
    Ok(billing::Billing {
        shape: billing::Shape::TokenQuota,
        source: billing::Source::Sub2api,
        used_usd: usage.used,
        limit_usd: usage.total,
        balance_usd: usage.remaining,
        ..Default::default()
    })
}

/// Hint shown when a station requires the `New-Api-User` (user ID) header.
pub(super) const ACCOUNT_NEEDS_USER_ID: &str =
    "该站点要求 New-Api-User（用户 ID）头：请在「令牌」面板为它补填用户 ID（可在站点面板 F12 看 /api/user/self 请求的 New-Api-User 值）";

/// Marker included in the error text when an account query fails for lacking the `New-Api-User` header.
pub(super) const NEEDS_USER_ID_MARK: &str = "New-Api-User";

/// Panel account quota: `GET {origin}/api/user/self` (requires the panel access token PAT).
///
/// Auth is `Authorization: Bearer <pat>` only; the `New-Api-User` header is sent only when the user
/// has filled in a user ID.
fn fetch_account(
    endpoints: &billing::Endpoints,
    units: &billing::Units,
    pat: &str,
    user_id: &str,
) -> Result<billing::AccountInfo, String> {
    let url = format!("{}/api/user/self", endpoints.origin);
    let text = account_get(&url, pat, user_id)?;
    billing::parse_account_self(&text, units)
        .ok_or_else(|| "返回内容不是可识别的账号额度".to_string())
}

/// Check-in status: `GET /api/user/checkin` (read-only, requires the panel access token).
///
/// Never performs the check-in; returns `None` when the station has none, no token is set, or the
/// endpoint can't be read.
fn fetch_checkin(
    endpoints: &billing::Endpoints,
    units: &billing::Units,
    pat: &str,
    user_id: &str,
) -> Option<billing::CheckinStatus> {
    let body = account_get(&endpoints.checkin(), pat, user_id).ok()?;
    billing::parse_checkin_status(&body, units).ok()
}

/// Merge check-in status into the result.
///
/// Unreadable status is not written; when the result is `Err`, a presentable result is assembled from
/// the check-in data alone.
fn merge_checkin(
    result: &mut Result<billing::Billing, String>,
    checkin: Option<billing::CheckinStatus>,
    panel: String,
) {
    let Some(status) = checkin else {
        return;
    };
    match result.as_mut() {
        Ok(info) => info.checkin = Some(status),
        Err(_) => {
            *result = Ok(billing::Billing {
                panel,
                source: billing::Source::Token,
                checkin: Some(status),
                ..Default::default()
            });
        }
    }
}

/// Panel per-day usage: `GET {origin}/api/data/self` (last 7 days; requires the panel access token).
///
/// Panel aggregates cover usage from all sources (non-local machines, cleaned-up sessions included),
/// complementing the local scan; shown only in the card hover, never merged into the ledger (to avoid
/// double counting with the local scan).
fn fetch_panel_days(
    endpoints: &billing::Endpoints,
    units: &billing::Units,
    pat: &str,
    user_id: &str,
) -> Option<Vec<billing::PanelDay>> {
    let end = unix_now();
    let start = end - 7 * 86_400;
    let body = account_get(&endpoints.data_self(start, end), pat, user_id).ok()?;
    billing::parse_data_self(&body, units)
}

/// Merge panel per-day usage into the result: recent days first, date labels localized; unreadable data is not written.
fn merge_panel_days(
    result: &mut Result<billing::Billing, String>,
    days: Option<Vec<billing::PanelDay>>,
) {
    let Some(mut days) = days else {
        return;
    };
    days.reverse();
    let offset = local_utc_offset_secs();
    let now_idx = crate::app::usage::aggregate::local_day_index(unix_now(), offset);
    for day in &mut days {
        day.date = panel_day_label(
            crate::app::usage::aggregate::local_day_index(day.day_ts, offset) - now_idx,
            now_idx,
        );
    }
    if let Ok(info) = result.as_mut() {
        info.panel_days = days;
    }
}

/// Day offset (relative to today: 0 = today, negative = past) → label: today / yesterday / N days ago / MM-DD.
fn panel_day_label(day_delta: i64, now_idx: i64) -> String {
    match day_delta {
        0 => return "今天".to_string(),
        -1 => return "昨天".to_string(),
        -6..=-2 => return format!("{} 天前", -day_delta),
        _ => {}
    }
    let (_y, m, d) = crate::app::usage::aggregate::civil_from_days(now_idx + day_delta);
    format!("{m:02}-{d:02}")
}

/// Attach localized labels and urgency to expiry info (`ts` is parsed by core; labels are filled by the app layer).
fn fill_expiry_labels(result: &mut Result<billing::Billing, String>) {
    let Ok(info) = result.as_mut() else {
        return;
    };
    let Some(expiry) = info.expires.as_mut() else {
        return;
    };
    let offset = local_utc_offset_secs();
    let now_idx = crate::app::usage::aggregate::local_day_index(unix_now(), offset);
    let exp_idx = crate::app::usage::aggregate::local_day_index(expiry.ts, offset);
    let (y, m, d) = crate::app::usage::aggregate::civil_from_days(exp_idx);
    expiry.label = format!("{y:04}-{m:02}-{d:02}");
    expiry.days_left = Some(exp_idx - now_idx);
}

/// Value of the `New-Api-User` header: an empty string (unset) means the header is not sent.
pub(super) fn user_id_header(user_id: &str) -> Option<&str> {
    let id = user_id.trim();
    (!id.is_empty()).then_some(id)
}

/// Account-endpoint-specific request: differs from [`http_get`] in that the error response body is
/// kept for classification.
///
/// The body is used locally for classification only and never enters error text.
fn account_get(url: &str, pat: &str, user_id: &str) -> Result<String, String> {
    let request = latency_agent()
        .get(url)
        .set("Accept", "application/json")
        // Same as http_get: carries a browser UA.
        .set("User-Agent", "Mozilla/5.0");
    let mut request = apply_auth(request, AuthKind::Bearer, pat);
    // The header is sent only when the user filled a user ID.
    if let Some(id) = user_id_header(user_id) {
        request = request.set("New-Api-User", id);
    }
    match request.call() {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("读取响应失败：{}", err)),
        Err(ureq::Error::Status(code, response)) => {
            // Body-read failures report by status code.
            let body = response.into_string().unwrap_or_default();
            if body_requests_user_id(&body) {
                return Err(ACCOUNT_NEEDS_USER_ID.to_string());
            }
            Err(crate::http_status::label(code))
        }
        Err(ureq::Error::Transport(transport)) => Err(format!(
            "网络错误：{}",
            crate::app::bars::sanitize_network_error(&transport.to_string())
        )),
    }
}

/// Whether the station demanded `New-Api-User` in an error body (header-name match, case-insensitive).
pub(super) fn body_requests_user_id(body: &str) -> bool {
    body.to_lowercase().contains("new-api-user")
}

/// Merge an account-query result into an existing result:
/// - success: writes `account`; when the result was `Err`, a presentable result is assembled from the
///   account data;
/// - failure: the existing result is kept as-is, with only one reason line appended to `note`.
fn merge_account(
    result: &mut Result<billing::Billing, String>,
    account: Result<billing::AccountInfo, String>,
    panel: String,
) {
    match account {
        Ok(account) => match result.as_mut() {
            Ok(info) => info.account = Some(account),
            Err(_) => {
                *result = Ok(billing::Billing {
                    panel,
                    source: billing::Source::Token,
                    account: Some(account),
                    ..Default::default()
                });
            }
        },
        Err(err) => {
            if let Ok(info) = result.as_mut() {
                let message = format!("账号令牌查询失败：{}", account_error_message(&err));
                info.note = Some(match info.note.take() {
                    Some(existing) => format!("{existing}；{message}"),
                    None => message,
                });
            }
        }
    }
}

/// Account-endpoint error text: 401/403 become "panel token invalid or revoked"; everything else passes through.
fn account_error_message(err: &str) -> String {
    match crate::app::bars::http_status_code(err) {
        Some(401) | Some(403) => "面板令牌无效或已撤销".to_string(),
        _ => err.to_string(),
    }
}

/// Token quota: `/api/usage/token/` (quota) + `/api/log/token` (today / last-7-days usage).
///
/// Returns `None` when neither side yields data (caller continues degrading).
fn fetch_token(
    endpoints: &billing::Endpoints,
    units: &billing::Units,
    secret: &str,
    status_json: Option<&str>,
) -> Option<billing::Billing> {
    let now = unix_now();
    let fallback_from = now - 86_400;
    let today_from = local_midnight_unix().unwrap_or(fallback_from);
    let mut note = (today_from == fallback_from)
        .then(|| "非 Windows 或读不到本地时间：今日按近 24 小时统计".to_string());

    // The two endpoints are independent; each parses on its own.
    let (usage, usage_error) = match http_get(&endpoints.token_usage(), Some(secret)) {
        Ok(text) => (billing::parse_token_usage(&text), None),
        Err(err) => (None, Some(err)),
    };
    let logs = match http_get(&endpoints.token_logs(), Some(secret)) {
        Ok(text) => billing::parse_token_logs(&text),
        Err(err) => {
            // Quota arrived but logs unavailable: keep the quota.
            note = Some(format!("调用日志不可用（{err}），今日用量取不到"));
            Vec::new()
        }
    };

    // With neither side yielding, return `None` and let the caller degrade to compatible billing.
    if usage.is_none() && logs.is_empty() {
        return None;
    }
    if usage.is_none() {
        if let Some(err) = usage_error {
            let reason = format!("令牌额度接口不可用（{err}）");
            note = Some(match note {
                Some(existing) => format!("{reason}；{existing}"),
                None => reason,
            });
        }
    }
    Some(billing::parse_token_billing(billing::TokenInputs {
        usage: usage.as_ref(),
        logs: &logs,
        units,
        status_json,
        now,
        today_from,
        note,
    }))
}

/// Compatible billing: the quota endpoint must succeed; usage-endpoint failure is tolerated.
fn fetch_compat(
    base_url: &str,
    secret: &str,
    status_json: Option<&str>,
) -> Result<billing::Billing, String> {
    let mut endpoints = billing::endpoints(base_url);
    let mut subscription = http_get(&endpoints.subscription, Some(secret));
    // Only 404 retries once on the `/v1` path; every other error returns immediately.
    if matches!(&subscription, Err(err) if crate::app::bars::http_status_code(err) == Some(404)) {
        let fallback = billing::endpoints_v1(base_url);
        if fallback.subscription != endpoints.subscription {
            if let Ok(text) = http_get(&fallback.subscription, Some(secret)) {
                endpoints = fallback;
                subscription = Ok(text);
            }
        }
    }
    let subscription = subscription?;
    let usage = http_get(&endpoints.usage, Some(secret)).ok();
    Ok(billing::parse(&subscription, usage.as_deref(), status_json))
}

/// Read-only GET: returns the body, with a human-readable reason on failure (reusing HTTP status explanations).
fn http_get(url: &str, secret: Option<&str>) -> Result<String, String> {
    let mut request = latency_agent()
        .get(url)
        .set("Accept", "application/json")
        // Management endpoints carry a browser UA.
        .set("User-Agent", "Mozilla/5.0");
    if let Some(secret) = secret {
        request = apply_auth(request, AuthKind::Bearer, secret);
    }
    match request.call() {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("读取响应失败：{}", err)),
        Err(ureq::Error::Status(code, _)) => Err(crate::http_status::label(code)),
        Err(ureq::Error::Transport(transport)) => Err(format!(
            "网络错误：{}",
            crate::app::bars::sanitize_network_error(&transport.to_string())
        )),
    }
}

/// Current time (second-resolution Unix timestamp).
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Local timezone offset from UTC (seconds; UTC+8 = 28800).
///
/// Computed as the difference between local wall time and UTC. The `usage` "time" dimension needs it
/// to floor timestamps onto local calendar days (see `crate::app::usage::aggregate::local_day_index`).
///
/// Limitation: the **current** offset is read, so historical timestamps crossing a DST boundary are off
/// by an hour. Same trade as [`local_midnight_unix`] — not worth pulling in a timezone database.
#[cfg(windows)]
pub(super) fn local_utc_offset_secs() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut local = SYSTEMTIME::default();
    // SAFETY: `GetLocalTime` writes only this one struct.
    unsafe { GetLocalTime(&mut local) };
    let wall_now = wall_clock_unix(
        i64::from(local.wYear),
        i64::from(local.wMonth),
        i64::from(local.wDay),
        i64::from(local.wHour),
        i64::from(local.wMinute),
        i64::from(local.wSecond),
    );
    wall_now - unix_now()
}

/// Non-Windows: no local timezone available; treated as UTC.
#[cfg(not(windows))]
pub(super) fn local_utc_offset_secs() -> i64 {
    0
}

/// Unix timestamp (seconds) of local timezone "today 00:00".
///
/// Takes local wall time and subtracts its offset from UTC.
#[cfg(windows)]
pub(super) fn local_midnight_unix() -> Option<i64> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut local = SYSTEMTIME::default();
    // SAFETY: `GetLocalTime` writes only this one struct.
    unsafe { GetLocalTime(&mut local) };
    let (year, month, day) = (
        i64::from(local.wYear),
        i64::from(local.wMonth),
        i64::from(local.wDay),
    );
    if year < 1970 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let wall_now = wall_clock_unix(
        year,
        month,
        day,
        i64::from(local.wHour),
        i64::from(local.wMinute),
        i64::from(local.wSecond),
    );
    let offset = wall_now - unix_now();
    Some(wall_clock_unix(year, month, day, 0, 0, 0) - offset)
}

/// Non-Windows: no local timezone available; returns `None`.
#[cfg(not(windows))]
pub(super) fn local_midnight_unix() -> Option<i64> {
    None
}

/// Convert Y/M/D h:m:s (interpreted as UTC) into a second-resolution Unix timestamp.
///
/// Uses Howard Hinnant's `days_from_civil` algorithm.
pub(super) fn wall_clock_unix(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    days * 86_400 + hour * 3_600 + minute * 60 + second
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_daily_snapshot_hides_today_but_keeps_other_totals() {
        let billing = billing::Billing {
            source: billing::Source::Token,
            shape: billing::Shape::TokenQuota,
            used_usd: Some(20.0),
            balance_usd: Some(80.0),
            today_usd: Some(3.0),
            today_calls: Some(4),
            today_models: vec![("m".into(), 3.0)],
            week_usd: Some(9.0),
            ..Default::default()
        };
        let state = BalanceState {
            result: Some(Ok(billing)),
            snapshot_midnight: Some(1_000),
            ..Default::default()
        };

        let current = state
            .display_result(Some(1_000))
            .expect("有结果")
            .expect("成功结果");
        assert_eq!(current.today_usd, Some(3.0));

        let stale = state
            .display_result(Some(2_000))
            .expect("有结果")
            .expect("成功结果");
        assert_eq!(stale.today_usd, None);
        assert_eq!(stale.today_calls, None);
        assert!(stale.today_models.is_empty());
        assert_eq!(stale.used_usd, Some(20.0));
        assert_eq!(stale.balance_usd, Some(80.0));
        assert_eq!(stale.week_usd, Some(9.0));
        assert!(stale.detail().contains("今日数据已跨日，请重新查询"));
    }
    #[test]
    fn wall_clock_matches_known_epochs() {
        assert_eq!(wall_clock_unix(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(wall_clock_unix(1970, 1, 2, 0, 0, 0), 86_400);
        // 2024-03-01 00:00:00 UTC
        assert_eq!(wall_clock_unix(2024, 3, 1, 0, 0, 0), 1_709_251_200);
        // 2024-02-29 (leap day)
        assert_eq!(wall_clock_unix(2024, 2, 29, 0, 0, 0), 1_709_164_800);
        // 2000-03-01 (century leap rule)
        assert_eq!(wall_clock_unix(2000, 3, 1, 0, 0, 0), 951_868_800);
        assert_eq!(wall_clock_unix(1999, 12, 31, 23, 59, 59), 946_684_799);
        assert_eq!(
            wall_clock_unix(2024, 3, 1, 12, 34, 56) - wall_clock_unix(2024, 3, 1, 0, 0, 0),
            12 * 3_600 + 34 * 60 + 56
        );
    }

    #[cfg(windows)]
    #[test]
    fn local_midnight_is_today_and_within_a_day() {
        let midnight = local_midnight_unix().expect("Windows 上应能取到本地 0 点");
        let now = unix_now();
        assert!(midnight <= now, "0 点不应晚于现在");
        assert!(now - midnight < 86_400 + 3_600, "0 点应落在过去 25 小时内");
    }

    /// A presentable token-side result.
    fn token_side_result() -> billing::Billing {
        billing::Billing {
            source: billing::Source::Token,
            shape: billing::Shape::TokenQuota,
            used_usd: Some(5.0),
            balance_usd: Some(15.0),
            ..Default::default()
        }
    }

    fn sample_account() -> billing::AccountInfo {
        billing::AccountInfo {
            balance_usd: Some(12.3),
            used_usd: Some(20.5),
            requests: Some(321),
            group: "default".to_string(),
        }
    }

    #[test]
    fn account_401_message_is_plain_language() {
        for code in [401, 403] {
            let err = crate::http_status::label(code);
            assert_eq!(account_error_message(&err), "面板令牌无效或已撤销", "{err}");
        }
        let not_found = crate::http_status::label(404);
        assert_eq!(account_error_message(&not_found), not_found);
        assert_eq!(
            account_error_message("网络错误：连接被重置"),
            "网络错误：连接被重置"
        );
    }

    #[test]
    fn user_id_requirement_is_not_reported_as_a_bad_token() {
        // Both vendors' wordings must be recognized.
        for body in [
            r#"{"message":"Unauthorized, New-Api-User header not provided","success":false}"#,
            r#"{"message":"无权进行此操作，未提供 New-Api-User","success":false}"#,
            r#"{"message":"Unauthorized, New-Api-User does not match logged in user"}"#,
            // Case-insensitive.
            r#"{"message":"new-api-user missing"}"#,
        ] {
            assert!(body_requests_user_id(body), "应识别：{body}");
        }
        for body in [
            r#"{"message":"Invalid token","success":false}"#,
            r#"{"message":"无权进行此操作，未登录且未提供 access token"}"#,
            "",
        ] {
            assert!(!body_requests_user_id(body), "不应误判：{body}");
        }
    }

    #[test]
    fn user_id_message_survives_the_error_mapping() {
        let message = account_error_message(ACCOUNT_NEEDS_USER_ID);
        assert_eq!(message, ACCOUNT_NEEDS_USER_ID);
        assert!(
            !message.contains("令牌无效"),
            "不能把缺用户 ID 说成令牌无效：{message}"
        );
        assert!(
            message.contains("New-Api-User") && message.contains("用户 ID"),
            "要说清缺什么、怎么补：{message}"
        );
    }

    #[test]
    fn user_id_header_is_sent_only_when_filled_in() {
        // Empty and whitespace-only strings = don't send the header.
        assert_eq!(user_id_header(""), None);
        assert_eq!(user_id_header("   "), None);
        assert_eq!(user_id_header(" 777 "), Some("777"));
    }

    #[test]
    fn account_success_fills_account_and_keeps_token_numbers() {
        let mut result: Result<billing::Billing, String> = Ok(token_side_result());
        merge_account(&mut result, Ok(sample_account()), "TestPanel".to_string());
        let info = result.expect("仍是成功结果");
        assert_eq!(info.account, Some(sample_account()));
        assert_eq!(info.used_usd, Some(5.0), "令牌侧数字必须保留");
        assert_eq!(info.balance_usd, Some(15.0));
        assert!(info.note.is_none(), "成功不应该产生备注");
    }

    #[test]
    fn account_failure_keeps_existing_result_and_appends_note() {
        let mut result: Result<billing::Billing, String> = Ok(billing::Billing {
            note: Some("调用日志不可用".to_string()),
            ..token_side_result()
        });
        merge_account(
            &mut result,
            Err(crate::http_status::label(401)),
            "TestPanel".to_string(),
        );
        let info = result.expect("账号查询失败不得影响已有结果");
        assert_eq!(info.used_usd, Some(5.0), "已有数字必须原样保留");
        assert!(info.account.is_none());
        let note = info.note.clone().unwrap_or_default();
        assert!(note.contains("调用日志不可用"), "原有说明不能丢：{note}");
        assert!(note.contains("账号令牌查询失败"), "{note}");
        assert!(note.contains("面板令牌无效或已撤销"), "{note}");
    }

    #[test]
    fn account_success_rescues_a_failed_result() {
        let mut result: Result<billing::Billing, String> = Err("HTTP 404 Not Found".to_string());
        merge_account(&mut result, Ok(sample_account()), "TestPanel".to_string());
        let info = result.expect("有账号数据就应救回成功结果");
        assert_eq!(info.account, Some(sample_account()));
        assert_eq!(info.panel, "TestPanel");
        assert!(info.is_displayable(), "仅账号数据也要能显示");
        assert!(
            info.inline().contains("账号余额 $12.30"),
            "{}",
            info.inline()
        );
    }

    #[test]
    fn account_failure_on_a_failed_result_stays_failed() {
        let mut result: Result<billing::Billing, String> = Err("HTTP 404 Not Found".to_string());
        merge_account(
            &mut result,
            Err("返回内容不是可识别的账号额度".to_string()),
            "TestPanel".to_string(),
        );
        assert!(result.is_err(), "不该把失败改写成成功");
    }

    fn sample_checkin() -> billing::CheckinStatus {
        billing::CheckinStatus {
            enabled: true,
            today: true,
            today_usd: Some(0.3),
            month_count: 14,
            total_usd: Some(13.0),
            ..Default::default()
        }
    }

    #[test]
    fn checkin_rides_along_with_the_token_result() {
        let mut result = Ok(token_side_result());
        merge_checkin(&mut result, Some(sample_checkin()), "TestPanel".to_string());
        let info = result.expect("应保持成功");
        assert_eq!(info.used_usd, Some(5.0), "令牌侧数据要留着");
        assert!(info.checkin.is_some(), "签到要挂上去");
        assert!(info.inline().contains("签到"), "{}", info.inline());
    }

    #[test]
    fn checkin_rescues_a_failed_result() {
        let mut result: Result<billing::Billing, String> = Err("HTTP 401 Unauthorized".to_string());
        merge_checkin(&mut result, Some(sample_checkin()), "TestPanel".to_string());
        let info = result.expect("签到读到了就不算全失败");
        assert!(info.is_displayable());
        assert_eq!(info.panel, "TestPanel", "面板名照旧要带上");
        assert!(info.inline().contains("签到 今日已签"), "{}", info.inline());
    }

    #[test]
    fn panel_days_rides_along_and_gets_local_labels() {
        let mut result = Ok(token_side_result());
        let today = local_midnight_unix().unwrap_or(1_800_000_000);
        let days = vec![
            billing::PanelDay {
                day_ts: today - 86_400,
                ..Default::default()
            },
            billing::PanelDay {
                day_ts: today,
                ..Default::default()
            },
        ];
        merge_panel_days(&mut result, Some(days));
        let info = result.expect("应保持成功");
        assert_eq!(info.panel_days.len(), 2);
        assert_eq!(info.panel_days[0].date, "今天");
        assert_eq!(info.panel_days[1].date, "昨天");
        assert!(info.detail().contains("面板按天"), "{}", info.detail());
    }

    #[test]
    fn missing_panel_days_changes_nothing() {
        let mut result = Ok(token_side_result());
        merge_panel_days(&mut result, None);
        let info = result.expect("应保持成功");
        assert!(info.panel_days.is_empty());
        assert!(!info.detail().contains("面板按天"));
    }

    #[test]
    fn expiry_labels_use_local_calendar() {
        let today = local_midnight_unix().unwrap_or(1_800_000_000);
        let mut result: Result<billing::Billing, String> = Ok(billing::Billing {
            expires: Some(billing::KeyExpiry {
                ts: today + 3 * 86_400,
                ..Default::default()
            }),
            ..Default::default()
        });
        fill_expiry_labels(&mut result);
        let info = result.expect("应保持成功");
        let expiry = info.expires.as_ref().expect("到期要保留");
        assert!(!expiry.label.is_empty(), "本地日期标签要填上");
        assert_eq!(expiry.days_left, Some(3), "本地日历日差");
    }

    #[test]
    fn missing_checkin_changes_nothing() {
        let mut result = Ok(token_side_result());
        merge_checkin(&mut result, None, "TestPanel".to_string());
        let info = result.expect("不该动");
        assert!(info.checkin.is_none());
        assert!(!info.inline().contains("签到"), "{}", info.inline());

        let mut failed: Result<billing::Billing, String> = Err("HTTP 404 Not Found".to_string());
        merge_checkin(&mut failed, None, String::new());
        assert!(failed.is_err(), "没签到数据就不该把失败改写成成功");
    }
}
