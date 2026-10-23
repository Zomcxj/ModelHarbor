//! Billing shapes and result structures ([`Shape`] / [`Source`] / [`Billing`]).
use super::*;

/// The billing shape the panel returned.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Shape {
    /// OpenAI-compatible billing: `hard_limit_usd` / `soft_limit_usd` + `total_usage` (cents).
    Subscription,
    /// The non-standard `credit_summary`: three raw fields, unit unlabeled.
    CreditSummary,
    /// Token quota (`/api/usage/token/`): this token is unlimited.
    TokenUnlimited,
    /// Token quota (`/api/usage/token/`): a concrete quota and balance.
    TokenQuota,
    /// Call logs only (the station offers no `/api/usage/token/`): today / last-7-days available, the three quota fields empty.
    TokenLogsOnly,
    /// Unrecognized (the station doesn't support it, or returned some other structure).
    #[default]
    Unknown,
}

/// Data source: decides how numbers are read and converted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Source {
    /// The OpenAI-compatible billing endpoint (`/dashboard/billing/*`): quotas are mostly placeholders.
    #[default]
    Compat,
    /// The token-quota endpoint (`/api/usage/token/`, only needs the `sk-` key): even unlimited stations yield real usage.
    Token,
    /// A Sub2API station's `/v1/usage` (only needs the `sk-` key): station-level remaining / used / total.
    Sub2api,
}

/// Sub2API `/v1/usage` readings (all three may be missing; versions expose different fields).
///
/// Unit defaults to USD; the remaining-quota fallback is `total - used`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sub2apiUsage {
    /// Remaining quota.
    pub remaining: Option<f64>,
    /// Used quota.
    pub used: Option<f64>,
    /// The quota cap.
    pub total: Option<f64>,
    /// The unit (as the station gives it; defaults to USD).
    pub unit: String,
}

/// Panel account quota (`/api/user/self`, present only with a panel access token): account-level, not token-level.
///
/// All `sk-` tokens under one station share it; token-level quota is [`Source::Token`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountInfo {
    /// The account's remaining quota (USD).
    pub balance_usd: Option<f64>,
    /// The account's lifetime used (USD).
    pub used_usd: Option<f64>,
    /// Historical request count.
    pub requests: Option<u64>,
    /// The owning group (empty when the station doesn't say).
    pub group: String,
}

/// Result of one usage query.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Billing {
    /// The panel name (`system_name` + version), possibly empty.
    pub panel: String,
    pub shape: Shape,
    /// Used quota (USD).
    pub used_usd: Option<f64>,
    /// The quota cap the station reports (USD).
    pub limit_usd: Option<f64>,
    /// Balance (USD) = quota − used.
    pub balance_usd: Option<f64>,
    /// The quota is a placeholder: no computing, no balance display — only usage is stated.
    ///
    /// The UI shows only "used" based on this.
    pub placeholder_limit: bool,
    /// This token is "quota-free": only usage is meaningful; there is no balance concept.
    pub unlimited: bool,
    /// `credit_summary`'s raw-value note (unit unlabeled).
    pub raw_credit: Option<String>,
    /// The data source (compatible billing / token quota / panel account).
    pub source: Source,
    /// Today's usage (USD, since local midnight).
    pub today_usd: Option<f64>,
    /// Today's request count.
    pub today_calls: Option<u64>,
    /// Today's data has gone stale across midnight; lifetime, balance, and last-7-days keep showing.
    pub today_stale: bool,
    /// Today's per-model spend (USD, descending, at most 3).
    pub today_models: Vec<(String, f64)>,
    /// Last-7-days usage (USD, today included).
    pub week_usd: Option<f64>,
    /// The station gave no `quota_per_unit`; conversion assumes the default 500000 (noted in the hover).
    pub unit_assumed: bool,
    /// Check-in status (`GET /api/user/checkin`, read-only, requires the panel token).
    ///
    /// Unrelated to quota; when the station has none, no panel token is set, or the endpoint can't be read,
    /// it's `None` and the card omits the line.
    pub checkin: Option<CheckinStatus>,
    /// The reason for degradation / missing data (call logs unavailable, panel token invalid, etc.).
    ///
    /// The card's small window doesn't render this.
    pub note: Option<String>,
    /// Panel account quota (`/api/user/self`): present only with a panel access token; when present, the balance follows it.
    pub account: Option<AccountInfo>,
    /// Panel per-day usage (`/api/data/self`, requires the panel access token): recent days first.
    ///
    /// Panel aggregates cover all sources (non-local included); shown only in the card hover, never merged
    /// into the ledger, avoiding double counting with the local scan.
    pub panel_days: Vec<PanelDay>,
    /// Token / subscription expiry (`/api/usage/token`'s `expires_at`, compatible billing's
    /// `access_until`); 0 / negative (no expiry) is `None`.
    pub expires: Option<KeyExpiry>,
}

/// Token / subscription expiry info.
///
/// `ts` is parsed and filled by core; `label` / `days_left` are completed by the app layer in the local
/// timezone (the hover omits the line when unset).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyExpiry {
    /// The expiry moment (Unix seconds).
    pub ts: i64,
    /// The local date label (`YYYY-MM-DD`), filled by the app layer.
    pub label: String,
    /// Local calendar days until expiry; negative when expired. Filled by the app layer.
    pub days_left: Option<i64>,
}

/// The panel side's per-day usage aggregate (`/api/data/self`'s `data` rows merged per day).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelDay {
    /// The station's per-day midnight timestamp (seconds).
    pub day_ts: i64,
    /// The display label (today / yesterday / N days ago / MM-DD), filled by the app layer.
    pub date: String,
    /// That day's spend (USD, converted by the station's quota_per_unit).
    pub usd: f64,
    /// That day's request count.
    pub calls: u64,
    /// That day's token volume.
    pub tokens: i64,
    /// That day's heaviest model's name.
    pub top_model: String,
}
