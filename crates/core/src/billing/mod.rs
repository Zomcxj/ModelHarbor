//! 中转站「已用 / 余额」解析（只读）。
//!
//! 端点面向 New-API / one-api 系站点，分令牌额度（`GET /api/usage/token/`、
//! `GET /api/log/token`、`GET /api/status`）与兼容账单（`/dashboard/billing/*`）两条路径；
//! 另支持 sub2api 站点的 `{base}/v1/usage`（只需 `sk-` key，见 [`sub2api_usage_url`]）。
//! 余额口径：`额度 − 已用`；占位额度与 `unlimited_quota` 的令牌不显示余额。

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
    parse_panel, parse_pricing, parse_sub2api_usage, parse_token_logs, summarize_logs,
    CheckinStatus, LogEntry, LogSummary, MODEL_TOP,
};
pub use render::{parse_token_billing, TokenInputs};
pub use shape::{
    AccountInfo, Billing, KeyExpiry, ModelUnitPrice, PanelDay, PriceKind, Pricing, Shape, Source,
    Sub2apiUsage,
};
pub use units::{
    parse_token_usage, parse_units, TokenUsage, Units, LOG_PAGE_LIMIT, PLACEHOLDER_LIMIT_USD,
    QUOTA_PER_USD,
};

pub(crate) use money::raw_num;
