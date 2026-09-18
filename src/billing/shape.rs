//! 账单形状与结果结构（[`Shape`] / [`Source`] / [`Billing`]）。
use super::*;

/// 面板返回的账单形状。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Shape {
    /// OpenAI 兼容账单：`hard_limit_usd` / `soft_limit_usd` + `total_usage`（美分）。
    Subscription,
    /// 非标准 `credit_summary`：三个原值字段，单位未标注。
    CreditSummary,
    /// 令牌额度（`/api/usage/token/`）：该令牌不限额度（公益站常见）。
    TokenUnlimited,
    /// 令牌额度（`/api/usage/token/`）：有具体额度与余额。
    TokenQuota,
    /// 只有调用日志（站点未提供 `/api/usage/token/`，如把 baseUrl 指向中转域名）。
    /// 今日 / 近 7 天仍可用；额度三项为空，不编数字。
    TokenLogsOnly,
    /// 无法识别（站点不支持，或返回了别的结构）。
    #[default]
    Unknown,
}

/// 数据来源：决定数字怎么读、怎么换算。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Source {
    /// OpenAI 兼容账单接口（`/dashboard/billing/*`）：额度多为占位值。
    #[default]
    Compat,
    /// 令牌额度接口（`/api/usage/token/`，只需 `sk-` key）：无限制站也能读到真实已用。
    Token,
}

/// 面板账号额度（`/api/user/self`，填了面板访问令牌才有）：**账号级**，不是令牌级。
///
/// 与令牌额度（`/api/usage/token/`）的区别：这里的数字属于**整个账号**，
/// 同一个站点下的所有 `sk-` 令牌共用一份。因此它回答「我还剩多少钱」，
/// 而令牌额度回答「这个 key 还能用多少」。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountInfo {
    /// 账号剩余额度（美元）。
    pub balance_usd: Option<f64>,
    /// 账号累计已用（美元）。
    pub used_usd: Option<f64>,
    /// 历史请求次数。
    pub requests: Option<u64>,
    /// 所属分组（站点未给时为空串）。
    pub group: String,
}

/// 一次用量查询的结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Billing {
    /// 面板名（`system_name` + 版本），可能为空。
    pub panel: String,
    pub shape: Shape,
    /// 已用额度（美元）。
    pub used_usd: Option<f64>,
    /// 站点给出的额度上限（美元）。
    pub limit_usd: Option<f64>,
    /// 余额（美元）= 额度 − 已用。
    pub balance_usd: Option<f64>,
    /// 额度是占位值（公益站常见 1e8）：**不算、不显示余额**，只说清已用。
    ///
    /// 界面据此只显示「已用」；标记本身是解析结果的一部分，供调用方与测试判断。
    pub placeholder_limit: bool,
    /// 该令牌是「不计额度」的（公益站常见）：只有已用有意义，没有余额概念。
    pub unlimited: bool,
    /// `credit_summary` 的原值说明（单位未标注）。
    pub raw_credit: Option<String>,
    /// 数据来源（兼容账单 / 令牌额度 / 面板账户）。
    pub source: Source,
    /// 今日已用（美元，本地时区 0 点起）。
    pub today_usd: Option<f64>,
    /// 今日请求数。
    pub today_calls: Option<u64>,
    /// 今日数据已跨日失效；累计、余额和近 7 天继续展示。
    pub today_stale: bool,
    /// 今日各模型消耗（美元，降序，最多 3 项）。
    pub today_models: Vec<(String, f64)>,
    /// 近 7 天已用（美元，含今日）。
    pub week_usd: Option<f64>,
    /// 站点没给 `quota_per_unit`，换算按默认 500000 假设（悬停里说明）。
    pub unit_assumed: bool,
    /// 签到状态（`GET /api/user/checkin`，只读，需面板令牌）。
    ///
    /// 与额度无关，是「今天领没领」这类信息：站点没开签到、没填面板令牌、
    /// 或接口读不到时都是 `None`——卡片上就**不写这一项**（有就输出，没有就不输出）。
    pub checkin: Option<CheckinStatus>,
    /// 降级 / 缺数据时的原因（调用日志不可用、面板令牌失效等）。
    ///
    /// 卡片小窗只显示数字，不渲染这一项；原因本身是解析结果的一部分，
    /// 供调用方与测试判断某次查询为什么少了数据。
    pub note: Option<String>,
    /// 面板账号额度（`/api/user/self`）：填了面板访问令牌才有；有它时余额以它为准。
    pub account: Option<AccountInfo>,
}
