//! 中转站「已用 / 余额 / 今日用量」查询：后台线程 + mpsc 通道（与延迟测试同一套写法）。
//!
//! 两条路径（按优先级）：
//! - **令牌额度**（只需 `sk-` key）：`/api/usage/token/` + `/api/log/token`
//!   —— 额度、今日 / 近 7 天用量、按模型拆分；公益站多为「不限额度」，
//!   此时只有「已用」有意义（不存在余额）；
//! - **兼容账单**（`sk-` key）：`/dashboard/billing/subscription` + `/usage`，
//!   额度多为占位值，所以余额仅供参考。
//!
//! 共同约束：
//! - **直连**：复用探测用的 agent（ureq 默认不读系统代理）；
//! - **只读**：全是管理接口，不发推理请求；
//! - **节流**：同一 provider 两次查询至少间隔 [`BALANCE_COOLDOWN_MS`]；
//! - 凭证只在请求头里用，**不进日志、不进状态栏文本**。

use super::fetch::{apply_auth, latency_agent, AuthKind};
use super::App;
use crate::billing;
use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

/// 同一 provider 两次查询的最小间隔（毫秒）。
pub(super) const BALANCE_COOLDOWN_MS: f64 = 5_000.0;

/// 查询结果文本颜色（比正文稍亮的青蓝，和延迟结果的绿 / 红区分开）。
pub(super) const BALANCE_TEXT: egui::Color32 = egui::Color32::from_rgb(120, 170, 210);

/// 一次查询的输入（拥有所有权，便于 move 进后台线程）。
#[derive(Clone, Default)]
pub(super) struct Query {
    pub(super) key: String,
    pub(super) base_url: String,
    /// `sk-` 形式的 API key（兼容账单接口用）。
    pub(super) secret: String,
}

/// 单个 provider 的用量查询状态。
#[derive(Default)]
pub(super) struct BalanceState {
    pub(super) result: Option<Result<billing::Billing, String>>,
    pub(super) rx: Option<Receiver<Result<billing::Billing, String>>>,
    /// 上次发起查询的时间（egui 时间轴，秒）。
    pub(super) last_at: Option<f64>,
}

impl App {
    /// 发起一次用量查询（字段拆分借用，便于在卡片内部调用）。
    ///
    /// 返回 `Some(提示文本)` 表示**没有**发起请求（正在查询中，或还在冷却期）。
    pub(super) fn start_balance_query(
        balance: &mut HashMap<String, BalanceState>,
        query: Query,
        now: f64,
    ) -> Option<String> {
        let key = query.key.clone();
        let state = balance.entry(key.clone()).or_default();
        if state.rx.is_some() {
            return Some(format!("{} 的用量查询还在进行中", key));
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
        state.rx = Some(rx);
        state.last_at = Some(now);
        None
    }

    /// 收割用量查询结果（在 `update` 里每帧调用，与 `poll_latency` 同批）。
    pub(super) fn poll_balance(&mut self) {
        let mut finished = 0usize;
        for state in self.balance.values_mut() {
            let Some(rx) = state.rx.as_ref() else {
                continue;
            };
            match rx.try_recv() {
                Ok(result) => {
                    state.result = Some(result);
                    state.rx = None;
                    finished += 1;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    // 线程异常退出：结束等待，避免 Spinner 永久转圈。
                    state.rx = None;
                }
            }
        }
        if finished == 0 || !self.balance_batch {
            return;
        }
        // 一键查询：全部跑完后给一条汇总（查不到的站点不在卡片上显示，只在这里报数）。
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
        self.status = format!(
            "用量查询完成：{} 个查到，{} 个未开放该接口（已隐藏）",
            ok,
            total - ok
        );
    }
}

/// 查询一个站点的用量：令牌额度接口优先，失败再退兼容账单。
fn fetch_billing(query: &Query) -> Result<billing::Billing, String> {
    let endpoints = billing::endpoints(&query.base_url);
    // 站点信息（面板名）与换算比两条路径都用，失败可容忍。
    let status = http_get(&endpoints.status, None).ok();
    let units = billing::parse_units(status.as_deref());
    if let Some(info) = fetch_token(&endpoints, &units, &query.secret, status.as_deref()) {
        return Ok(info);
    }
    fetch_compat(&query.base_url, &query.secret, status.as_deref())
}

/// 令牌额度：`/api/usage/token/`（额度）+ `/api/log/token`（今日 / 近 7 天用量）。
///
/// 这两个接口直接用 `sk-` key 就能读，公益站（不限额度）也能拿到真实已用。
/// 额度接口不可用时返回 `None`（交给调用方继续降级）。
fn fetch_token(
    endpoints: &billing::Endpoints,
    units: &billing::Units,
    secret: &str,
    status_json: Option<&str>,
) -> Option<billing::Billing> {
    let usage_json = http_get(&endpoints.token_usage(), Some(secret)).ok()?;
    let usage = billing::parse_token_usage(&usage_json)?;
    let now = unix_now();
    let fallback_from = now - 86_400;
    let today_from = local_midnight_unix().unwrap_or(fallback_from);
    let mut note = (today_from == fallback_from)
        .then(|| "非 Windows 或读不到本地时间：今日按近 24 小时统计".to_string());
    let logs = match http_get(&endpoints.token_logs(), Some(secret)) {
        Ok(text) => billing::parse_token_logs(&text),
        Err(err) => {
            // 额度读到了但日志不可用：保留额度，说明今日用量缺原因。
            note = Some(format!("调用日志不可用（{err}），今日用量取不到"));
            Vec::new()
        }
    };
    Some(billing::parse_token_billing(billing::TokenInputs {
        usage: &usage,
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
    // 少数站点只在 `/v1` 下提供账单接口，而 pi 页里 anthropic 系的 baseUrl 只有 origin：
    // **仅在 404 时**换一次路径重试，其他错误（403 被 WAF 拦、401 等）直接如实报出。
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
        // 管理接口按浏览器身份请求：ureq 默认 UA 会被部分站点的 WAF 直接拒掉。
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
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 本地时区「今天 0 点」的秒级 Unix 时间戳。
///
/// 做法：取本地墙上时间，用「先当成 UTC 换算」的方式得到两个数（当前墙上时间、
/// 今天 0 点），差值就是时区偏移，再从今天 0 点里扣掉 —— 不需要时区数据库。
#[cfg(windows)]
fn local_midnight_unix() -> Option<i64> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;

    let mut local = SYSTEMTIME::default();
    // SAFETY: `GetLocalTime` 只写这一个结构体（无指针别名、无生命周期要求）。
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

/// 非 Windows：拿不到本地时区，调用方回退为「近 24 小时」。
#[cfg(not(windows))]
fn local_midnight_unix() -> Option<i64> {
    None
}

/// 把年月日时分秒（按 UTC 理解）换算成秒级 Unix 时间戳。
///
/// 纯函数，便于单测：用 Howard Hinnant 的 `days_from_civil` 算法，无闰年表。
fn wall_clock_unix(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
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
    fn wall_clock_matches_known_epochs() {
        assert_eq!(wall_clock_unix(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(wall_clock_unix(1970, 1, 2, 0, 0, 0), 86_400);
        // 2024-03-01 00:00:00 UTC
        assert_eq!(wall_clock_unix(2024, 3, 1, 0, 0, 0), 1_709_251_200);
        // 2024-02-29（闰日）
        assert_eq!(wall_clock_unix(2024, 2, 29, 0, 0, 0), 1_709_164_800);
        // 2000-03-01（百年闰规则：2000 是闰年）
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
}
