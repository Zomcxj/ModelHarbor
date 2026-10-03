//! 端点拼装（[`Endpoints`]）。
use super::*;

/// 由 baseUrl 推导出的端点（origin 用于面板管理接口与 `/api/status`）。
pub struct Endpoints {
    /// 只保留 scheme + host（面板管理接口都在站根下）。
    pub origin: String,
    pub status: String,
    pub subscription: String,
    pub usage: String,
}

impl Endpoints {
    /// 令牌额度信息（只需 `sk-` key）：`total_granted` / `total_used` / `total_available`。
    pub fn token_usage(&self) -> String {
        format!("{}/api/usage/token/", self.origin)
    }

    /// 签到状态（只读，需面板访问令牌）：`GET /api/user/checkin`。
    ///
    /// 只读：不代签，只查回状态展示。
    pub fn checkin(&self) -> String {
        format!("{}/api/user/checkin", self.origin)
    }

    /// 该令牌的调用日志（用于算今日 / 近 7 天用量，只需 `sk-` key）。
    /// 只请求首个大分页，未遍历后续页。
    pub fn token_logs(&self) -> String {
        format!(
            "{}/api/log/token?p=0&page_size={LOG_PAGE_LIMIT}",
            self.origin
        )
    }
}

/// 推导端点：`base` 去掉末尾斜杠；`origin` 只保留 scheme + host。
///
/// 缺少 `://` 时按整体当 origin。
pub fn endpoints(base_url: &str) -> Endpoints {
    let base = base_url.trim().trim_end_matches('/').to_string();
    let origin = match base.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split('/').next().unwrap_or("");
            format!("{}://{}", scheme, host)
        }
        None => base.clone(),
    };
    Endpoints {
        origin: origin.clone(),
        status: format!("{}/api/status", origin),
        subscription: format!("{}/dashboard/billing/subscription", base),
        usage: format!("{}/dashboard/billing/usage", base),
    }
}

/// 备选端点：baseUrl 只有 origin 时，在 origin 后插一个 `/v1`。
///
/// 调用方仅在 404 时才回退到这里；baseUrl 已带路径（含 `/v1`）时原样返回。
pub fn endpoints_v1(base_url: &str) -> Endpoints {
    let base = base_url.trim().trim_end_matches('/');
    match base.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = match rest.split_once('/') {
                Some((host, path)) => (host, path),
                None => (rest, ""),
            };
            if path.is_empty() {
                endpoints(&format!("{}://{}/v1", scheme, host))
            } else {
                endpoints(base)
            }
        }
        None => endpoints(base),
    }
}
