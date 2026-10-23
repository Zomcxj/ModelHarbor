//! Relay-station "used / balance" parsing (read-only).
//!
//! Endpoints target New-API / one-api family stations, split into token quota (`GET /api/usage/token/`,
//! `GET /api/log/token`, `GET /api/status`) and compatible billing (`/dashboard/billing/*`) paths;
//! sub2api stations' `{base}/v1/usage` is also supported (only the `sk-` key — see [`sub2api_usage_url`]).
//! Balance accounting: `quota − used`; placeholder quotas and `unlimited_quota` tokens show no balance.

mod endpoints;
mod money;
mod parse;
mod render;
mod shape;
mod units;

#[cfg(test)]
mod tests;

pub use endpoints::{endpoints, endpoints_v1, sub2api_usage_url, Endpoints};
pub use money::money;
pub use parse::{
    checkin_long, checkin_short, parse, parse_account_self, parse_checkin_status, parse_data_self,
    parse_panel, parse_sub2api_usage, parse_token_logs, summarize_logs, CheckinStatus, LogEntry,
    LogSummary, MODEL_TOP,
};
pub use render::{parse_token_billing, TokenInputs};
pub use shape::{AccountInfo, Billing, KeyExpiry, PanelDay, Shape, Source, Sub2apiUsage};
pub use units::{
    parse_token_usage, parse_units, TokenUsage, Units, LOG_PAGE_LIMIT, PLACEHOLDER_LIMIT_USD,
    QUOTA_PER_USD,
};

pub(crate) use money::raw_num;
