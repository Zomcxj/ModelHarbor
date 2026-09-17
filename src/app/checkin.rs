//! 站点签到：读状态（**只读**）+ 显式执行（**写操作**）。
//!
//! 签到会改变账号额度并让站点记一条系统日志，所以它**完全不参与**「查询用量」的
//! 批处理流程：只有用户为某个 provider 勾选了「签到」并点了按钮，才会发请求。
//!
//! 接口（new-api）：
//! - `GET  /api/user/checkin`：今天是否已签、本月 / 累计次数、累计获得额度；
//! - `POST /api/user/checkin`：执行签到。
//!
//! 部署方若开了 Cloudflare Turnstile，`POST` 必须带浏览器解题产生的
//! `?turnstile=` token。本工具**不做人机验证**，遇到这种情况只把站点原话加上
//! 一句解释报给用户（见 [`billing::checkin_error_hint`]）。
//!
//! 鉴权与用量查询同一套：`Authorization: Bearer <面板令牌>`（旧版站点再带
//! `New-Api-User`）。凭证只进请求头，不进日志与状态栏文本。

use super::balance::{body_requests_user_id, user_id_header, ACCOUNT_NEEDS_USER_ID};
use super::fetch::{apply_auth, latency_agent, AuthKind};
use super::App;
use crate::billing;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

/// 一次签到的输入（拥有所有权，便于 move 进后台线程）。
#[derive(Clone, Default)]
pub(super) struct Query {
    pub(super) key: String,
    pub(super) base_url: String,
    /// 面板访问令牌：签到**必须**用它，`sk-` key 不行（站点要的是用户身份）。
    pub(super) pat: String,
    /// 旧版站点要的用户 ID（空串 = 不发 `New-Api-User`）。
    pub(super) user_id: String,
}

/// 单个 provider 的签到状态。
#[derive(Default)]
pub(super) struct CheckinState {
    pub(super) rx: Option<Receiver<Result<String, String>>>,
    /// 最近一次的结果：成功是可展示文案，失败是原因。
    pub(super) result: Option<Result<String, String>>,
}

impl App {
    /// 发起签到。同一 provider 同时只允许一个在飞（签到是写操作，不并发）。
    pub(super) fn start_checkin(&mut self, query: Query) -> Option<String> {
        let key = query.key.clone();
        let state = self.checkin.entry(key.clone()).or_default();
        if state.rx.is_some() {
            return Some(format!("{} 的签到还在进行中", key));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(run(&query));
        });
        state.result = None;
        state.rx = Some(rx);
        None
    }

    /// 收割签到结果（在 `update` 里每帧调用，与 `poll_balance` 同批）。
    pub(super) fn poll_checkin(&mut self) {
        for state in self.checkin.values_mut() {
            let Some(rx) = state.rx.as_ref() else {
                continue;
            };
            match rx.try_recv() {
                Ok(result) => {
                    self.status = match &result {
                        Ok(text) => text.clone(),
                        Err(err) => format!("签到失败：{err}"),
                    };
                    state.result = Some(result);
                    state.rx = None;
                }
                Err(TryRecvError::Empty) => {}
                // 线程异常退出：结束等待，避免 Spinner 永久转圈。
                Err(TryRecvError::Disconnected) => state.rx = None,
            }
        }
    }

}

/// 签到全流程：先读状态，今天没签才执行。
///
/// 先读后写是刻意的：站点对「今天已签到」的 POST 会报错（`success:false`），
/// 但这不是失败，而是**没必要再签**——用户看到「今日已签到」比看到一句报错准确。
fn run(query: &Query) -> Result<String, String> {
    let origin = billing::endpoints(&query.base_url).origin;
    if origin.trim().is_empty() {
        return Err("该 provider 没有可用的 baseUrl".to_string());
    }
    if query.pat.trim().is_empty() {
        return Err("签到需要面板访问令牌：请在「令牌」面板为这个站点填上".to_string());
    }
    // 换算比：拿不到就按默认（是否“按默认估算”由 billing 的 `unit_assumed` 说明）。
    let units = billing::parse_units(fetch_status(&origin).as_deref());

    let endpoint = format!("{origin}/api/user/checkin");
    let status_body = call(signed(&endpoint, query, false))?;
    let status = billing::parse_checkin_status(&status_body, &units)
        .map_err(|err| billing::checkin_error_hint(&err))?;
    if !status.enabled {
        return Err("该站点未启用签到".to_string());
    }
    if status.today {
        return Ok(billing::checkin_message(&status, None));
    }
    let done_body = call(signed(&endpoint, query, true))?;
    let outcome = billing::parse_checkin_result(&done_body, &units)
        .map_err(|err| billing::checkin_error_hint(&err))?;
    Ok(billing::checkin_message(&status, Some(&outcome)))
}

/// `/api/status`：只用来取 `quota_per_unit`，失败可容忍（回落默认换算比）。
fn fetch_status(origin: &str) -> Option<String> {
    signed_get(&format!("{origin}/api/status")).ok()
}

/// 带鉴权的请求构造：`Authorization: Bearer <PAT>` +（旧版站点）`New-Api-User`。
fn signed(url: &str, query: &Query, post: bool) -> ureq::Request {
    let agent = latency_agent();
    let request = if post {
        agent.post(url)
    } else {
        agent.get(url)
    };
    sign(request, &query.pat, &query.user_id)
}

/// 无凭证的 GET（站点信息接口不需要鉴权）。
fn signed_get(url: &str) -> Result<String, String> {
    call(latency_agent().get(url))
}

/// 统一加鉴权头：管理接口按浏览器身份请求（ureq 默认 UA 会被部分站点 WAF 拒掉）。
fn sign(request: ureq::Request, pat: &str, user_id: &str) -> ureq::Request {
    let request = request
        .set("Accept", "application/json")
        .set("User-Agent", "Mozilla/5.0");
    let mut request = apply_auth(request, AuthKind::Bearer, pat);
    if let Some(id) = user_id_header(user_id) {
        request = request.set("New-Api-User", id);
    }
    request
}

/// 发请求并取正文。
///
/// 非 2xx **不把正文并进错误文本**（与账号接口同一策略）：正文只在本地用来判断
/// 「是不是缺 `New-Api-User`」，避免把站点回显的内容带到状态栏。
fn call(request: ureq::Request) -> Result<String, String> {
    match request.call() {
        Ok(response) => response
            .into_string()
            .map_err(|err| format!("读取响应失败：{}", err)),
        Err(ureq::Error::Status(code, response)) => {
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

/// 各 provider 的签到状态表（与 [`App::balance`] 同为「按 provider key」索引）。
pub(super) type States = HashMap<String, CheckinState>;
