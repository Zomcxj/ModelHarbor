//! 中转站「已用 / 余额 / 今日用量」查询：后台线程 + mpsc 通道。
//!
//! 两条路径（按优先级）：
//! - **令牌额度**（只需 `sk-` key）：`/api/usage/token/` + `/api/log/token`；
//! - **兼容账单**（`sk-` key）：`/dashboard/billing/subscription` + `/usage`。
//!
//! 约束：直连（每次查询新建 Agent）；全部为只读管理接口；同一 provider 两次查询
//! 间隔至少 [`BALANCE_COOLDOWN_MS`]；凭证只放在请求头里，不进日志与状态栏文本。

use super::fetch::{apply_auth, latency_agent, AuthKind};
use super::App;
use crate::billing;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

/// 同一 provider 两次查询的最小间隔（毫秒）。
pub(super) const BALANCE_COOLDOWN_MS: f64 = 5_000.0;

/// 一次查询的输入（拥有所有权，可 move 进后台线程）。
#[derive(Clone, Default)]
pub(super) struct Query {
    pub(super) key: String,
    pub(super) base_url: String,
    /// `sk-` 形式的 API key（兼容账单接口用）。
    pub(super) secret: String,
    /// 面板访问令牌（PAT，可选）：填了才能查账号级额度（`/api/user/self`）。
    pub(super) pat: String,
    /// 旧版 new-api 要求的用户 ID（`New-Api-User` 头）；空串 = 不发送该头。
    pub(super) user_id: String,
}

/// 单个 provider 的用户数据查询状态。
#[derive(Default)]
pub(super) struct BalanceState {
    pub(super) result: Option<Result<billing::Billing, String>>,
    pub(super) rx: Option<Receiver<Result<billing::Billing, String>>>,
    /// 上次发起查询的时间（egui 时间轴，秒）。
    pub(super) last_at: Option<f64>,
    /// 本次成功结果的本地午夜标识；跨到下一天后隐藏 `today_*`。
    pub(super) snapshot_midnight: Option<i64>,
}

impl App {
    /// 发起一次用户数据查询。
    ///
    /// 返回 `Some(提示文本)` 表示没有发起请求（正在查询中，或还在冷却期）。
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

    /// 收割用户数据查询结果（在 `update` 里每帧调用）。
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
                    // 线程异常退出：结束等待。
                    state.rx = None;
                }
            }
        }
        if finished == 0 || !self.balance_batch {
            return;
        }
        // 一键查询：全部跑完后给一条汇总。
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
        // 只报数，不报原因。
        self.status = format!("用户数据查询完成：{} 个查到，{} 个没查到", ok, total - ok);
    }
}

impl BalanceState {
    /// 按当前本地日期返回展示副本；跨日只失效「今日」，累计/余额/近 7 天保留。
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

/// 查询一个站点的用量：令牌额度接口优先，失败再退兼容账单；
/// 填了面板令牌则再补一份账号级额度。
fn fetch_billing(query: &Query) -> Result<billing::Billing, String> {
    let endpoints = billing::endpoints(&query.base_url);
    // 站点信息（面板名）与换算比，失败可容忍。
    let status = http_get(&endpoints.status, None).ok();
    let units = billing::parse_units(status.as_deref());
    let mut result = match fetch_token(&endpoints, &units, &query.secret, status.as_deref()) {
        Some(info) => Ok(info),
        None => fetch_compat(&query.base_url, &query.secret, status.as_deref()),
    };
    // 面板令牌为可选增强：失败只往详情里补一行说明。
    let pat = query.pat.trim();
    if !pat.is_empty() {
        let account = fetch_account(&endpoints, &units, pat, &query.user_id);
        let checkin = fetch_checkin(&endpoints, &units, pat, &query.user_id);
        let panel = billing::parse_panel(status.as_deref());
        merge_account(&mut result, account, panel.clone());
        merge_checkin(&mut result, checkin, panel);
    }
    result
}

/// 站点要求 `New-Api-User`（用户 ID）头时的提示。
pub(super) const ACCOUNT_NEEDS_USER_ID: &str =
    "该站点要求 New-Api-User（用户 ID）头：请在「令牌」面板为它补填用户 ID（可在站点面板 F12 看 /api/user/self 请求的 New-Api-User 值）";

/// 账号查询因缺 `New-Api-User` 头失败时，错误文本里包含的标记。
pub(super) const NEEDS_USER_ID_MARK: &str = "New-Api-User";

/// 面板账号额度：`GET {origin}/api/user/self`（需面板访问令牌 PAT）。
///
/// 鉴权只用 `Authorization: Bearer <pat>`；`New-Api-User` 头仅在用户填了用户 ID 时发送。
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

/// 签到状态：`GET /api/user/checkin`（只读，需面板访问令牌）。
///
/// 不代签；站点没开签到、没填令牌或接口读不到时返回 `None`。
fn fetch_checkin(
    endpoints: &billing::Endpoints,
    units: &billing::Units,
    pat: &str,
    user_id: &str,
) -> Option<billing::CheckinStatus> {
    let body = account_get(&endpoints.checkin(), pat, user_id).ok()?;
    billing::parse_checkin_status(&body, units).ok()
}

/// 把签到状态并进结果。
///
/// 读不到就不写；结果为 `Err` 时用签到数据拼一份可展示结果。
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

/// `New-Api-User` 头的取值：空串（没填）表示不发这个头。
pub(super) fn user_id_header(user_id: &str) -> Option<&str> {
    let id = user_id.trim();
    (!id.is_empty()).then_some(id)
}

/// 账号接口专用请求：与 [`http_get`] 的区别是保留错误响应正文用于分类。
///
/// 正文只在本地用于判断，不进入错误文本。
fn account_get(url: &str, pat: &str, user_id: &str) -> Result<String, String> {
    let request = latency_agent()
        .get(url)
        .set("Accept", "application/json")
        // 与 http_get 一致：带浏览器 UA。
        .set("User-Agent", "Mozilla/5.0");
    let mut request = apply_auth(request, AuthKind::Bearer, pat);
    // 用户填了用户 ID 时才发送该头。
    if let Some(id) = user_id_header(user_id) {
        request = request.set("New-Api-User", id);
    }
    match request.call() {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("读取响应失败：{}", err)),
        Err(ureq::Error::Status(code, response)) => {
            // 正文读失败时按状态码报。
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

/// 站点是否在错误正文里要求 `New-Api-User`（头名匹配，大小写不敏感）。
pub(super) fn body_requests_user_id(body: &str) -> bool {
    body.to_lowercase().contains("new-api-user")
}

/// 把账号查询结果并入已有结果：
/// - 成功：写入 `account`；结果为 `Err` 时用账号数据拼一份可展示结果；
/// - 失败：保留已有结果不动，只在 `note` 里追加一行原因。
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

/// 账号接口错误文本：401/403 转为「面板令牌无效或已撤销」，其余原样返回。
fn account_error_message(err: &str) -> String {
    match crate::app::bars::http_status_code(err) {
        Some(401) | Some(403) => "面板令牌无效或已撤销".to_string(),
        _ => err.to_string(),
    }
}

/// 令牌额度：`/api/usage/token/`（额度）+ `/api/log/token`（今日 / 近 7 天用量）。
///
/// 两边都没拿到时返回 `None`（交给调用方继续降级）。
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

    // 两个接口互相独立，各自解析。
    let (usage, usage_error) = match http_get(&endpoints.token_usage(), Some(secret)) {
        Ok(text) => (billing::parse_token_usage(&text), None),
        Err(err) => (None, Some(err)),
    };
    let logs = match http_get(&endpoints.token_logs(), Some(secret)) {
        Ok(text) => billing::parse_token_logs(&text),
        Err(err) => {
            // 额度读到了但日志不可用：保留额度。
            note = Some(format!("调用日志不可用（{err}），今日用量取不到"));
            Vec::new()
        }
    };

    // 两边都没拿到时返回 `None`，由调用方降级到兼容账单。
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

/// 兼容账单：额度接口必须成功，用量接口失败可容忍。
fn fetch_compat(
    base_url: &str,
    secret: &str,
    status_json: Option<&str>,
) -> Result<billing::Billing, String> {
    let mut endpoints = billing::endpoints(base_url);
    let mut subscription = http_get(&endpoints.subscription, Some(secret));
    // 仅 404 时换到 `/v1` 路径重试一次，其他错误直接返回。
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

/// 只读 GET：返回正文，失败给出人话原因（复用 HTTP 状态码解释）。
fn http_get(url: &str, secret: Option<&str>) -> Result<String, String> {
    let mut request = latency_agent()
        .get(url)
        .set("Accept", "application/json")
        // 管理接口带浏览器 UA。
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

/// 当前时间（秒级 Unix 时间戳）。
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 本地时区相对 UTC 的偏移（秒，东八区 = 28800）。
///
/// 取本地墙上时间与 UTC 之差。`usage` 的「时间」维度要用它把时间戳落到本地日历日
/// （见 `crate::app::usage::aggregate::local_day_index`）。
///
/// 局限：读的是**此刻**的偏移，所以跨夏令时的历史时间戳会差一小时。
/// 与 [`local_midnight_unix`] 同一个取舍 —— 不值得为此引入时区数据库。
#[cfg(windows)]
pub(super) fn local_utc_offset_secs() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut local = SYSTEMTIME::default();
    // SAFETY: `GetLocalTime` 只写这一个结构体。
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

/// 非 Windows：拿不到本地时区，按 UTC 处理。
#[cfg(not(windows))]
pub(super) fn local_utc_offset_secs() -> i64 {
    0
}

/// 本地时区「今天 0 点」的秒级 Unix 时间戳。
///
/// 取本地墙上时间，减去它与 UTC 的时区偏移。
#[cfg(windows)]
pub(super) fn local_midnight_unix() -> Option<i64> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut local = SYSTEMTIME::default();
    // SAFETY: `GetLocalTime` 只写这一个结构体。
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

/// 非 Windows：拿不到本地时区，返回 `None`。
#[cfg(not(windows))]
pub(super) fn local_midnight_unix() -> Option<i64> {
    None
}

/// 把年月日时分秒（按 UTC 理解）换算成秒级 Unix 时间戳。
///
/// 用 Howard Hinnant 的 `days_from_civil` 算法。
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
        // 2024-02-29（闰日）
        assert_eq!(wall_clock_unix(2024, 2, 29, 0, 0, 0), 1_709_164_800);
        // 2000-03-01（百年闰规则）
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

    /// 一份可展示的令牌侧结果。
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
        // 两家的文案都要识别出来。
        for body in [
            r#"{"message":"Unauthorized, New-Api-User header not provided","success":false}"#,
            r#"{"message":"无权进行此操作，未提供 New-Api-User","success":false}"#,
            r#"{"message":"Unauthorized, New-Api-User does not match logged in user"}"#,
            // 大小写不敏感。
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
        // 空串与纯空白 = 不发这个头。
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
