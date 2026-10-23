//! Endpoint assembly ([`Endpoints`]).
use super::*;

/// Endpoints derived from a baseUrl (the origin serves panel management APIs and `/api/status`).
pub struct Endpoints {
    /// Keep only scheme + host (panel management APIs all live at the station root).
    pub origin: String,
    pub status: String,
    pub subscription: String,
    pub usage: String,
}

impl Endpoints {
    /// Token quota info (only needs the `sk-` key): `total_granted` / `total_used` / `total_available`.
    pub fn token_usage(&self) -> String {
        format!("{}/api/usage/token/", self.origin)
    }

    /// Check-in status (read-only, requires the panel access token): `GET /api/user/checkin`.
    ///
    /// Read-only: never performs the check-in, only reads the status for display.
    pub fn checkin(&self) -> String {
        format!("{}/api/user/checkin", self.origin)
    }

    /// This token's call logs (for today / last-7-days usage; only needs the `sk-` key).
    /// Only the first large page is requested; later pages aren't walked.
    pub fn token_logs(&self) -> String {
        format!(
            "{}/api/log/token?p=0&page_size={LOG_PAGE_LIMIT}",
            self.origin
        )
    }

    /// User-level per-day usage statistics (`GET /api/data/self`, requires the panel access token).
    ///
    /// Returns per-day × model `{model_name, count, quota, token_used, created_at}`; panel aggregates cover
    /// usage from all sources (non-local included), complementing the local scan.
    pub fn data_self(&self, start: i64, end: i64) -> String {
        format!(
            "{}/api/data/self?start_timestamp={start}&end_timestamp={end}",
            self.origin
        )
    }
}

/// Derive endpoints: `base` loses trailing slashes; `origin` keeps only scheme + host.
///
/// Without `://`, the whole string counts as the origin.
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

/// The alternate endpoint: when baseUrl holds only the origin, insert a `/v1` after it.
///
/// The caller falls back here only on 404; a baseUrl already carrying a path (`/v1` included) returns as-is.
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

/// The Sub2API usage endpoint: `{base}/v1/usage` (a Bearer sk- key suffices; no panel token needed).
///
/// Users fill the base inconsistently (`https://host` / `https://host/v1` / the whole path) — the same
/// normalization as other endpoints: an already-`v1/usage` tail is untouched, an existing `/v1` only gets
/// `usage`, otherwise `v1/usage` completes it.
pub fn sub2api_usage_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/v1/usage") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{base}/usage")
    } else {
        format!("{base}/v1/usage")
    }
}
