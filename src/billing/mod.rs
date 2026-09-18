//! 中转站「已用 / 余额」解析（只读）。
//!
//! 端点面向 New-API / one-api 系站点，分两条路径（见 [`Source`]）：
//!
//! **令牌额度（首选，只需 `sk-` key）**
//! - `GET {origin}/api/usage/token/`：`total_granted` / `total_used` / `total_available` /
//!   `unlimited_quota`，单位是 **quota 点**；
//! - `GET {origin}/api/log/token`：该 key 的调用日志，用来算今日 / 近 7 天用量与模型拆分
//!   （站点通常最多给 1000 条，见 [`LOG_PAGE_LIMIT`]）；
//! - `GET {origin}/api/status`：无需鉴权，`data.quota_per_unit` 给出 quota ↔ 货币的换算比
//!   （实测站点都是 500000，但它是站点设置，所以以它为准、拿不到才标注「按默认估算」）。
//!
//! **兼容账单（兜底）**
//! - `GET {base}/dashboard/billing/subscription`：`hard_limit_usd` / `soft_limit_usd`；
//! - `GET {base}/dashboard/billing/usage`：`total_usage`，单位是**美分**；
//!   日期参数被服务端忽略（实测 2000-01-01 / 近 14 天 / 不带参数结果完全相同），是累计值。
//!
//! 余额口径：`额度 − 已用`。两种情况下**不显示余额**（宁可不显示，也不给一个没意义的数字）：
//! - 公益站把额度写成占位值（`100000000`）→ `placeholder_limit`，只说清已用；
//! - `unlimited_quota` 的令牌 → 它本来就不计额度。

mod endpoints;
mod money;
mod parse;
mod render;
mod shape;
mod units;

#[cfg(test)]
mod tests;

pub use endpoints::{endpoints, endpoints_v1, Endpoints};
pub use money::money;
pub use parse::{
    checkin_long, checkin_short, parse, parse_account_self, parse_checkin_status, parse_panel,
    parse_token_logs, summarize_logs, CheckinStatus, LogEntry, LogSummary, MODEL_TOP,
};
pub use render::{parse_token_billing, TokenInputs};
pub use shape::{AccountInfo, Billing, Shape, Source};
pub use units::{
    parse_token_usage, parse_units, TokenUsage, Units, LOG_PAGE_LIMIT, PLACEHOLDER_LIMIT_USD,
    QUOTA_PER_USD,
};

pub(crate) use money::raw_num;
