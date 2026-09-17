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

use serde_json::Value;

/// 额度 ≥ 该值即视为占位（公益站几乎都给 1e8）。
pub const PLACEHOLDER_LIMIT_USD: f64 = 1_000_000.0;

/// 面板账户的额度单位：`quota` ÷ 该值 = 美元（New-API / one-api 固定 500000）。
pub const QUOTA_PER_USD: f64 = 500_000.0;

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
    /// 卡片小窗不解释这一点（用户要求小窗只放数据），所以只看这一个标记的
    /// 话界面不会提它；它仍然是解析结果里的事实（调用方 / 测试靠它）。
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
    /// **卡片小窗不再显示它**：用户要小窗只放数据，不许出现「备注」这类行。
    /// 字段留着，因为「为什么降级」本身是解析结果的一部分：调用方（状态栏、
    /// 将来的提示）与测试靠它，也避免静默降级——数据缺了总得在某个层面说得清。
    pub note: Option<String>,
    /// 面板账号额度（`/api/user/self`）：填了面板访问令牌才有；有它时余额以它为准。
    pub account: Option<AccountInfo>,
}

/// 由 baseUrl 推导出的端点（origin 用于面板管理接口与 `/api/status`）。
pub struct Endpoints {
    /// 只保留 scheme + host（面板管理接口都在站根下）。
    pub origin: String,
    pub status: String,
    pub subscription: String,
    pub usage: String,
}

impl Endpoints {
    /// 令牌额度信息（**只需 `sk-` key**）：`total_granted` / `total_used` / `total_available`。
    pub fn token_usage(&self) -> String {
        format!("{}/api/usage/token/", self.origin)
    }

    /// 签到状态（**只读**，需面板访问令牌）：`GET /api/user/checkin`。
    ///
    /// 只读是有意的：签到会改账号额度，本工具不代签，只把状态查回来展示。
    pub fn checkin(&self) -> String {
        format!("{}/api/user/checkin", self.origin)
    }

    /// 该令牌的调用日志（用于算今日 / 近 7 天用量，**只需 `sk-` key**）。
    /// 显式请求首个大分页；未遍历后续页，因此展示层始终披露“统计可能不完整”。
    pub fn token_logs(&self) -> String {
        format!(
            "{}/api/log/token?p=0&page_size={LOG_PAGE_LIMIT}",
            self.origin
        )
    }
}

/// 推导端点：`base` 去掉末尾斜杠；`origin` 只保留 scheme + host。
///
/// 缺少 `://` 时按整体当 origin（这类地址本身已在 baseUrl 体检里报过「缺少协议头」）。
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

/// 备选端点：baseUrl 只有 origin（pi 页里 anthropic 系就是这样）时，在 origin 后插一个 `/v1`。
///
/// 实测：多数站点在 `/` 与 `/v1` 下都提供账单接口，但少数只在 `/v1` 下提供 ——
/// 调用方**仅在 404 时**才回退到这里；baseUrl 已带路径（含 `/v1`）时原样返回，
/// 避免拼出 `…/v1/v1/dashboard/…`（实测会 404）。
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

/// 解析 `/api/status` 的面板名（`system_name` + 版本），失败返回空串。
pub fn parse_panel(status_json: Option<&str>) -> String {
    let Some(text) = status_json else {
        return String::new();
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return String::new();
    };
    let data = root.get("data").unwrap_or(&root);
    let name = data
        .get("system_name")
        .or_else(|| data.get("systemName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = data.get("version").and_then(Value::as_str).unwrap_or("");
    match (name.is_empty(), version.is_empty()) {
        (true, true) => String::new(),
        (false, true) => name.to_string(),
        (true, false) => version.to_string(),
        (false, false) => format!("{} {}", name, version),
    }
}

/// 汇总解析订阅 + 用量 + 站点信息。
pub fn parse(
    subscription_json: &str,
    usage_json: Option<&str>,
    status_json: Option<&str>,
) -> Billing {
    let mut out = Billing {
        panel: parse_panel(status_json),
        ..Default::default()
    };
    let Ok(sub) = serde_json::from_str::<Value>(subscription_json) else {
        return out;
    };

    // 非标准形状：直接展示原值，不做单位换算（避免猜错量级）。
    if sub.get("object").and_then(Value::as_str) == Some("credit_summary")
        || sub.get("total_granted").is_some()
    {
        out.shape = Shape::CreditSummary;
        out.raw_credit = Some(format!(
            "总额度 {} / 已用 {} / 可用 {}（单位未标注，按原值展示）",
            raw_num(sub.get("total_granted")),
            raw_num(sub.get("total_used")),
            raw_num(sub.get("total_available")),
        ));
        return out;
    }

    // 常规 OpenAI 兼容账单。
    // `hard_limit_usd` 为 0 / 缺失表示「无硬上限」（不少站用 0 占位），
    // 此时算出来的「余额」会是负数，误导性强 —— 直接当成没有额度信息。
    out.limit_usd = sub
        .get("hard_limit_usd")
        .and_then(Value::as_f64)
        .or_else(|| sub.get("soft_limit_usd").and_then(Value::as_f64))
        .filter(|limit| *limit > 0.0);
    out.placeholder_limit = out
        .limit_usd
        .is_some_and(|limit| limit >= PLACEHOLDER_LIMIT_USD);
    // 占位额度算出来的「余额」没有意义（1e8 − 已用）：直接不产生这个数字，
    // 免得界面上出现一个看着像真实余额的量。
    if out.placeholder_limit {
        out.limit_usd = None;
    }
    if let Some(text) = usage_json {
        // total_usage 单位是美分。
        out.used_usd = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|usage| usage.get("total_usage").and_then(Value::as_f64))
            .map(|cents| cents / 100.0);
    }
    if let (Some(limit), Some(used)) = (out.limit_usd, out.used_usd) {
        // 额度比已用小（数据不一致或已超支）时不显示负数余额：只说清楚已用多少。
        let balance = limit - used;
        if balance >= 0.0 {
            out.balance_usd = Some(balance);
        }
    }
    out.shape = if out.limit_usd.is_some() || out.used_usd.is_some() {
        Shape::Subscription
    } else {
        Shape::Unknown
    };
    out
}

/// 站点日志接口通常最多返回这么多条（实测 1000）：到这个数就认为统计可能偏小。
pub const LOG_PAGE_LIMIT: usize = 1000;

/// 站点单位设置（`/api/status`）：quota 点 ↔ 货币的换算依据。
#[derive(Clone, Debug, PartialEq)]
pub struct Units {
    /// 1 单位货币 = 多少 quota 点（站点可改，默认 500000）。
    pub quota_per_unit: f64,
    /// 展示币种（`USD` / `CNY` / 空）。
    pub currency: String,
    /// 站点没给换算比：按默认值假设。
    pub assumed: bool,
}

/// 解析 `/api/status` 里的换算字段（拿不到就给默认值并标记 `assumed`）。
pub fn parse_units(status_json: Option<&str>) -> Units {
    let fallback = Units {
        quota_per_unit: QUOTA_PER_USD,
        currency: String::new(),
        assumed: true,
    };
    let Some(text) = status_json else {
        return fallback;
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return fallback;
    };
    let data = root.get("data").unwrap_or(&root);
    let unit = data
        .get("quota_per_unit")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0);
    Units {
        quota_per_unit: unit.unwrap_or(QUOTA_PER_USD),
        currency: data
            .get("quota_display_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_uppercase(),
        assumed: unit.is_none(),
    }
}

/// 令牌额度信息（`/api/usage/token/`，**只需 `sk-` key**）；数值单位是 quota 点。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenUsage {
    /// 令牌名（面板里显示的名字）。
    pub name: String,
    /// 不计额度（公益站常见）：只有已用有意义。
    pub unlimited: bool,
    pub granted: Option<f64>,
    pub used: Option<f64>,
    pub available: Option<f64>,
    /// 到期时间（秒级 Unix 时间戳，0 表示无期限）。
    pub expires_at: Option<f64>,
}

/// 解析令牌额度；没有可用字段时返回 `None`（视为该站不支持）。
pub fn parse_token_usage(json: &str) -> Option<TokenUsage> {
    let root = serde_json::from_str::<Value>(json).ok()?;
    let data = root.get("data")?;
    if !data.is_object() {
        return None;
    }
    let usage = TokenUsage {
        name: data
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        unlimited: data
            .get("unlimited_quota")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        granted: data.get("total_granted").and_then(Value::as_f64),
        used: data.get("total_used").and_then(Value::as_f64),
        available: data.get("total_available").and_then(Value::as_f64),
        expires_at: data.get("expires_at").and_then(Value::as_f64),
    };
    let empty = usage.granted.is_none()
        && usage.used.is_none()
        && usage.available.is_none()
        && usage.name.is_empty();
    (!empty).then_some(usage)
}

/// 解析 `/api/user/self`（**需要面板访问令牌/PAT**，普通登录用户即可）。
///
/// `data.quota` 是**剩余**额度、`used_quota` 是累计已用，两者都是 quota 点，
/// 按 `/api/status` 给出的 [`Units`] 换算成金额（与令牌额度同一套换算，不另猜汇率）。
///
/// 以下情况返回 `None`（视为「没拿到账号数据」，由调用方决定降级或提示）：
/// - 不是 JSON / 根不是对象；
/// - `success` 显式为 `false`（令牌无效、未提供令牌时站点用 200 + `success:false` 报错）；
/// - 一个可用字段都没有（只有 `group` 这类非数字字段不算数据）。
pub fn parse_account_self(json: &str, units: &Units) -> Option<AccountInfo> {
    let root = serde_json::from_str::<Value>(json).ok()?;
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    // 站点都在 `data` 下返回；少数变体把字段放在根上，一并兼容。
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&root);
    if !data.is_object() {
        return None;
    }
    let quota = data.get("quota").and_then(Value::as_f64);
    let used = data.get("used_quota").and_then(Value::as_f64);
    // 次数容忍整数 / 浮点两种写法（站点实现不完全一致）。
    let requests = data.get("request_count").and_then(value_as_u64);
    // 全是 None 说明这不是一份可用的账号数据（例如 success:true 但 data 为空）。
    if quota.is_none() && used.is_none() && requests.is_none() {
        return None;
    }
    let to_money = |points: f64| points / units.quota_per_unit;
    Some(AccountInfo {
        balance_usd: quota.map(to_money),
        used_usd: used.map(to_money),
        requests,
        group: data
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// 读一个非负整数（容忍整数 / 浮点两种写法：站点实现不完全一致）。
fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
}

/// 签到状态（`GET /api/user/checkin`，只需面板令牌/PAT，**只读**）。
///
/// 不是每个站点都开这个功能：未启用时站点用 `200 + success:false` 报
/// 「签到功能未启用」，[`parse_checkin_status`] 会把原话作为错误返回。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckinStatus {
    /// 站点是否启用签到。
    pub enabled: bool,
    /// 今天是否已签到。
    pub today: bool,
    /// **今天**签到的额度（美元）：站点只在 `records` 里逐日给，取最大日期那条。
    /// 今天没签（或站点没给明细）时为 `None`。
    pub today_usd: Option<f64>,
    /// 本月签到次数。
    pub month_count: u64,
    /// 累计签到次数。
    pub total_count: u64,
    /// 累计获得的额度（美元）；站点没给就是 `None`。
    pub total_usd: Option<f64>,
    /// 单次签到额度区间（美元）；站点未配置（0）时为 `None`。
    pub min_usd: Option<f64>,
    pub max_usd: Option<f64>,
}

/// 取站点自己给的错误消息（`message` 字段），取不到就用一句兜底。
///
/// 站点的话比我们猜的准：「签到功能未启用」「今日已签到」
/// 「Turnstile token 为空」都是它自己说的，要原样带给用户。
fn site_message(root: &Value, fallback: &str) -> String {
    root.get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// 解析 `GET /api/user/checkin`。
///
/// 失败情形一律返回 `Err`（而不是 `None`）：未启用、被拒绝、结构不认识
/// 都需要给用户一句具体的话，不能被当成「这个站点没有签到」而静默。
pub fn parse_checkin_status(json: &str, units: &Units) -> Result<CheckinStatus, String> {
    let root = serde_json::from_str::<Value>(json).map_err(|_| "返回内容不是 JSON".to_string())?;
    if !root.is_object() {
        return Err("返回内容不是可识别的签到状态".to_string());
    }
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(site_message(&root, "站点拒绝了签到状态查询"));
    }
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| "返回内容不是可识别的签到状态".to_string())?;
    let stats = data.get("stats").filter(|value| value.is_object());
    let count = |key: &str| {
        stats
            .and_then(|stats| stats.get(key))
            .and_then(value_as_u64)
            .unwrap_or(0)
    };
    let to_money = |points: f64| points / units.quota_per_unit;
    let today = stats
        .and_then(|stats| stats.get("checked_in_today"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // 今日金额：站点只在 `records` 里逐日给额度。取**最大日期**那条
    // （`YYYY-MM-DD` 的字典序就是时间序），不依赖站点给的先后顺序；
    // 且只在 `checked_in_today` 为真时才算「今天」——否则最大日期未必是今天，
    // 把别的日子当成今日会误导。
    let today_usd = if today {
        stats
            .and_then(|stats| stats.get("records"))
            .and_then(Value::as_array)
            .and_then(|records| {
                records
                    .iter()
                    .filter_map(|record| {
                        let date = record.get("checkin_date").and_then(Value::as_str)?;
                        let quota = record.get("quota_awarded").and_then(Value::as_f64)?;
                        Some((date, quota))
                    })
                    .max_by_key(|(date, _)| *date)
                    .map(|(_, quota)| quota)
            })
            .map(to_money)
    } else {
        None
    };
    // 0 表示站点没配置这一项：显示「额度 $0.00」只会误导。
    let positive_usd = |key: &str| {
        data.get(key)
            .and_then(Value::as_f64)
            .filter(|points| *points > 0.0)
            .map(to_money)
    };
    Ok(CheckinStatus {
        enabled: data
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        today,
        today_usd,
        month_count: count("checkin_count"),
        total_count: count("total_checkins"),
        total_usd: stats
            .and_then(|stats| stats.get("total_quota"))
            .and_then(Value::as_f64)
            .filter(|points| *points > 0.0)
            .map(to_money),
        min_usd: positive_usd("min_quota"),
        max_usd: positive_usd("max_quota"),
    })
}

/// 签到状态的**短**描述（卡片那一行里用，越短越好）。
///
/// 站点没给的数字一律不提：不编 `$0.00`、不编「本月 0 次」。
pub fn checkin_short(status: &CheckinStatus) -> String {
    if !status.enabled {
        return "未启用".to_string();
    }
    if !status.today {
        return "未签".to_string();
    }
    match status.today_usd {
        Some(today) => format!("今日已签 {}", money(today)),
        None => "今日已签".to_string(),
    }
}

/// 签到状态的**完整**描述（悬停说明里用）。
pub fn checkin_long(status: &CheckinStatus) -> String {
    if !status.enabled {
        return "该站点未启用签到".to_string();
    }
    let mut text = checkin_short(status);
    let mut extra: Vec<String> = Vec::new();
    if status.month_count > 0 {
        extra.push(format!("本月 {} 次", status.month_count));
    }
    if let Some(total) = status.total_usd {
        extra.push(format!("累计获得 {}", money(total)));
    }
    if !extra.is_empty() {
        text.push_str(&format!("（{}）", extra.join("，")));
    }
    text
}

/// 一条调用日志（只留统计要用的字段；`content` / `ip` 这类隐私字段不解析）。#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    /// 发生时间（秒级 Unix 时间戳）。
    pub at: i64,
    /// 消耗（quota 点）。
    pub quota: f64,
    pub model: String,
}

/// 解析 `/api/log/token` 的日志列表（`{success, data:[...]}`；`data` 缺失时为空表）。
pub fn parse_token_logs(json: &str) -> Vec<LogEntry> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(items) = root.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let at = item.get("created_at").and_then(Value::as_i64)?;
            Some(LogEntry {
                at,
                quota: item.get("quota").and_then(Value::as_f64).unwrap_or(0.0),
                model: item
                    .get("model_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// 日志聚合结果（`from` 为 `None` 表示全部）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogSummary {
    pub count: usize,
    pub calls: u64,
    /// 区间内消耗（quota 点）。
    pub quota: f64,
    /// 按模型汇总（quota 降序，最多 [`MODEL_TOP`] 项）。
    pub models: Vec<(String, f64)>,
}

/// 悬停里最多列几个模型。
pub const MODEL_TOP: usize = 3;

/// 按时间下限汇总日志（时间戳全为秒）。
pub fn summarize_logs(entries: &[LogEntry], from: Option<i64>) -> LogSummary {
    let mut summary = LogSummary::default();
    let mut per_model: Vec<(String, f64)> = Vec::new();
    for entry in entries {
        if from.is_some_and(|from| entry.at < from) {
            continue;
        }
        summary.count += 1;
        summary.calls += 1;
        summary.quota += entry.quota;
        match per_model.iter_mut().find(|(name, _)| name == &entry.model) {
            Some((_, quota)) => *quota += entry.quota,
            None => per_model.push((entry.model.clone(), entry.quota)),
        }
    }
    per_model.sort_by(|a, b| b.1.total_cmp(&a.1));
    per_model.truncate(MODEL_TOP);
    summary.models = per_model;
    summary
}

/// [`parse_token_billing`] 的输入（字段多，打包成一个结构）。
pub struct TokenInputs<'a> {
    /// 令牌额度接口的结果；该接口整个不存在时为 `None`（日志统计照常出）。
    pub usage: Option<&'a TokenUsage>,
    pub logs: &'a [LogEntry],
    pub units: &'a Units,
    pub status_json: Option<&'a str>,
    /// 当前时间（秒级 Unix 时间戳）。
    pub now: i64,
    /// 本地时区今天 0 点。
    pub today_from: i64,
    pub note: Option<String>,
}

/// 汇总「令牌额度 + 调用日志 + 站点单位」→ 卡片摘要（全程无需面板 token）。
pub fn parse_token_billing(inputs: TokenInputs<'_>) -> Billing {
    let unit = inputs.units.quota_per_unit;
    let to_money = |points: f64| points / unit;
    let today = summarize_logs(inputs.logs, Some(inputs.today_from));
    let week = summarize_logs(inputs.logs, Some(inputs.now - 7 * 86_400));
    // 只请求首个分页，无法证明服务端没有后续页（即今日 / 近 7 天统计可能偏小）。
    // 这一点不再写进悬停小窗：那里只放数字与影响读数的口径（换算比、跳日），
    // 接口来源与分页上限属于实现细节，用户看的是额度。
    // 额度接口可能整个不存在（把 baseUrl 指向中转域名的站点就没有这个面板路由）：
    // 此时日志统计照常出，额度三项留空，绝不编数字。
    let unlimited = inputs.usage.is_some_and(|usage| usage.unlimited);
    Billing {
        panel: parse_panel(inputs.status_json),
        source: Source::Token,
        shape: match inputs.usage {
            None => Shape::TokenLogsOnly,
            Some(usage) if usage.unlimited => Shape::TokenUnlimited,
            Some(_) => Shape::TokenQuota,
        },
        used_usd: inputs.usage.and_then(|usage| usage.used).map(to_money),
        // 不限额度时没有「余额 / 上限」可言，不编数字。
        balance_usd: inputs
            .usage
            .filter(|usage| !usage.unlimited)
            .and_then(|usage| usage.available)
            .map(to_money),
        limit_usd: inputs
            .usage
            .filter(|usage| !usage.unlimited)
            .and_then(|usage| usage.granted)
            .filter(|value| *value > 0.0)
            .map(to_money),
        unlimited,
        today_usd: (!today.models.is_empty() || today.count > 0).then(|| to_money(today.quota)),
        today_calls: (today.count > 0).then_some(today.calls),
        today_models: today
            .models
            .iter()
            .map(|(name, quota)| (name.clone(), to_money(*quota)))
            .collect(),
        week_usd: (week.count > 0).then(|| to_money(week.quota)),
        unit_assumed: inputs.units.assumed,
        note: inputs.note,
        ..Default::default()
    }
}

impl Billing {
    /// 跨日本地快照隐藏“今日”字段，保留累计、余额与近 7 天。
    pub fn expire_today(&mut self) {
        self.today_usd = None;
        self.today_calls = None;
        self.today_models.clear();
        self.today_stale = true;
    }

    /// 是否一个可用数字都没有（没余额、没已用、没今日、没原值）。
    ///
    /// 界面用它配合 [`Shape::Unknown`] 把「查不到」的站点隐掉。
    pub fn is_empty(&self) -> bool {
        self.used_usd.is_none()
            && self.balance_usd.is_none()
            && self.today_usd.is_none()
            && self.raw_credit.is_none()
    }

    /// 卡片上的一行摘要（尽量短，卡片收起时也显示）。
    pub fn inline(&self) -> String {
        // 账号级数据优先：它才是「我还剩多少钱」，令牌级降为补充。
        if let Some(account) = &self.account {
            return self.inline_account(account);
        }
        self.inline_token_or_compat()
    }

    /// 把签到段接到摘要行末尾（没数据就不接，绝不写占位）。
    fn push_checkin(&self, parts: &mut Vec<String>) {
        if let Some(status) = &self.checkin {
            parts.push(format!("签到 {}", checkin_short(status)));
        }
    }

    /// 有账号数据时的短行：账号余额 + 已用（没余额时才退而单说已用 / 请求数），其次今日。
    fn inline_account(&self, account: &AccountInfo) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(balance) = account.balance_usd {
            parts.push(format!("账号余额 {}", money(balance)));
            // 已用紧跟余额：只看余额看不出「用掉多少」，而对那些令牌额度接口缺失、
            // 调用日志又为空的中转站（如 ps.air-outer.com）来说，
            // 这个数字是唯一拿得到的用量。
            if let Some(used) = account.used_usd {
                parts.push(format!("已用 {}", money(used)));
            }
        } else if let Some(used) = account.used_usd {
            parts.push(format!("账号已用 {}", money(used)));
        } else if let Some(requests) = account.requests {
            parts.push(format!("账号请求 {} 次", requests));
        }
        if let Some(today) = self.today_usd {
            parts.push(match self.today_calls {
                Some(calls) => format!("今日 {}（{} 次）", money(today), calls),
                None => format!("今日 {}", money(today)),
            });
        }
        if let Some(week) = self.week_usd {
            parts.push(format!("近 7 天 {}", money(week)));
        }
        self.push_checkin(&mut parts);
        if parts.is_empty() {
            return "未返回额度信息".to_string();
        }
        parts.join(" · ")
    }

    /// 没有账号数据时的短行（令牌额度 / 兼容账单）。
    fn inline_token_or_compat(&self) -> String {
        // 面板账户 / 令牌额度：数字是真实的（不是占位额度），最多显示两段：
        // 有余额就先显余额，其次今日用量（没有今日就用累计已用）。
        if self.source == Source::Token {
            let mut parts: Vec<String> = Vec::new();
            match (self.balance_usd, self.used_usd) {
                (Some(balance), _) => parts.push(format!("余额 {}", money(balance))),
                // 不限额度站：「已用」才是重点（写「余额」会让人以为里面有钱）。
                (None, Some(used)) => parts.push(format!("已用 {}", money(used))),
                (None, None) => {}
            }
            match (self.today_usd, self.used_usd) {
                (Some(today), _) => parts.push(format!("今日 {}", money(today))),
                // 拿不到今日时用累计兼顾信息量（只在已有余额时补，避免重复）
                (None, Some(used)) if self.balance_usd.is_some() => {
                    parts.push(format!("累计 {}", money(used)))
                }
                (None, _) => {}
            }
            self.push_checkin(&mut parts);
            if parts.is_empty() {
                return "未返回额度信息".to_string();
            }
            return parts.join(" · ");
        }
        if self.raw_credit.is_some() {
            return "额度单位未标注（悬停看原值）".to_string();
        }
        match (self.used_usd, self.balance_usd) {
            (Some(used), Some(balance)) => {
                format!("已用 {} · 余额 {}", money(used), money(balance))
            }
            (Some(used), None) => format!("已用 {}", money(used)),
            (None, Some(balance)) => format!("余额 {}", money(balance)),
            (None, None) => "未返回额度信息".to_string(),
        }
    }

    /// 卡片上的一行完整用量（字段多，单独占一行；悬停看详情）。
    ///
    /// 与 [`Billing::inline`] 的区别：这里把能拿到的字段都列出来
    ///（余额 / 累计 / 今日 + 请求数 / 近 7 天），用于卡片正文那一行。
    /// 是否有可展示的用量结果（Unknown / 空数据隐藏）。
    ///
    /// 账号数据（面板令牌）单独就能让卡片显示：站点未开 `/api/usage/token/` 时，
    /// 只要拿到账号额度就不该整块隐掉。
    pub fn is_displayable(&self) -> bool {
        // 签到状态也算可展示数据：有些站点只有签到读得到（有就输出，没有就不输出）。
        self.has_token_side() || self.account.is_some() || self.checkin.is_some()
    }

    /// 令牌侧（令牌额度 / 兼容账单）是否真有可展示数据。
    fn has_token_side(&self) -> bool {
        self.shape != Shape::Unknown && !self.is_empty()
    }

    pub fn inline_full(&self) -> String {
        // 账号级余额优先，且不再把令牌级累计挤在同一行（降到 `detail`）。
        if let Some(account) = &self.account {
            return self.inline_account(account);
        }
        let mut parts: Vec<String> = Vec::new();
        match (self.balance_usd, self.used_usd) {
            (Some(balance), _) => {
                parts.push(format!("余额 {}", money(balance)));
                if let Some(used) = self.used_usd {
                    parts.push(format!("累计 {}", money(used)));
                }
            }
            (None, Some(used)) => parts.push(format!("已用 {}", money(used))),
            (None, None) => {}
        }
        if let Some(today) = self.today_usd {
            parts.push(match self.today_calls {
                Some(calls) => format!("今日 {}（{} 次）", money(today), calls),
                None => format!("今日 {}", money(today)),
            });
        }
        if let Some(week) = self.week_usd {
            parts.push(format!("近 7 天 {}", money(week)));
        }
        if let Some(raw) = &self.raw_credit {
            parts.push(raw.clone());
        }
        self.push_checkin(&mut parts);
        if parts.is_empty() {
            return self.inline();
        }
        parts.join(" · ")
    }

    /// 悬停提示：完整说明与口径来源。
    pub fn detail(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if !self.panel.is_empty() {
            lines.push(format!("面板：{}", self.panel));
        }
        // 账号数据（面板令牌）与令牌数据是两套口径：同时存在时分节列出，不混在一起。
        if let Some(account) = &self.account {
            lines.extend(Self::account_lines(account));
        }
        // 签到也是账号级信息（靠同一个面板令牌读），就跟账号那节排在一起。
        if let Some(status) = &self.checkin {
            lines.push(format!("签到：{}", checkin_long(status)));
        }
        let has_account_side = self.account.is_some() || self.checkin.is_some();
        if has_account_side && !self.has_token_side() {
            // 只有账号 / 签到数据：不写一个空的「本令牌」分节。
            return lines.join("\n");
        }
        if has_account_side {
            lines.push("—— 本令牌 ——".to_string());
        }
        if self.source == Source::Token {
            return self.detail_token(lines);
        }
        self.detail_compat(lines)
    }

    /// 账号级额度（`/api/user/self`）的悬停说明。
    ///
    /// 悬停小窗是「扫一眼」的地方：每项占一行、外加一句解释，会把数字挤到
    /// 视线之外。这里按「额度一行、计数与分组一行」合并；但「同站点共用」
    /// 这层含义要留个短标记——否则读者会以为这是某个 sk- 令牌的余额。
    fn account_lines(account: &AccountInfo) -> Vec<String> {
        let mut lines = vec!["账号级额度（面板访问令牌，同站点共用）".to_string()];
        let mut amounts: Vec<String> = Vec::new();
        if let Some(balance) = account.balance_usd {
            amounts.push(format!("余额 {}", money(balance)));
        }
        if let Some(used) = account.used_usd {
            amounts.push(format!("已用 {}", money(used)));
        }
        if !amounts.is_empty() {
            lines.push(amounts.join(" · "));
        }
        let mut meta: Vec<String> = Vec::new();
        if let Some(requests) = account.requests {
            meta.push(format!("请求 {} 次", requests));
        }
        if !account.group.is_empty() {
            meta.push(format!("分组 {}", account.group));
        }
        if !meta.is_empty() {
            lines.push(meta.join(" · "));
        }
        lines
    }

    /// 兼容账单（`/dashboard/billing/*`）的悬停说明。
    fn detail_compat(&self, mut lines: Vec<String>) -> String {
        if let Some(used) = self.used_usd {
            lines.push(format!("已用：{}", money(used)));
        }
        if let Some(limit) = self.limit_usd {
            lines.push(format!("额度：{}", money(limit)));
        }
        if let Some(balance) = self.balance_usd {
            lines.push(format!("余额：{}", money(balance)));
        }
        if let Some(raw) = &self.raw_credit {
            lines.push(format!("账单：{}", raw));
        }
        if lines.is_empty() {
            lines.push("该站没有返回可识别的账单信息".to_string());
        }
        // 口径只说一句：这是账号累计值（不是今日），不扯接口怎么实现。
        lines.push("已用为账号累计值".to_string());
        lines.join("\n")
    }

    /// 令牌额度（`/api/usage/token/` + `/api/log/token`）的悬停详情。
    fn detail_token(&self, mut lines: Vec<String>) -> String {
        // 额度接口缺失（`Shape::TokenLogsOnly`）时不写任何一行：
        // 缺数据本身不产生数字，写一句解释只是噪音。
        if self.unlimited {
            lines.push("额度：不限".to_string());
        }
        if let Some(used) = self.used_usd {
            lines.push(format!("累计已用：{}", money(used)));
        }
        if let Some(balance) = self.balance_usd {
            lines.push(format!("余额：{}", money(balance)));
        }
        if let Some(limit) = self.limit_usd {
            lines.push(format!("额度：{}", money(limit)));
        }
        match (self.today_usd, self.today_calls) {
            (Some(today), Some(calls)) => {
                lines.push(format!("今日已用：{}（{} 次请求）", money(today), calls));
            }
            (Some(today), None) => lines.push(format!("今日已用：{}", money(today))),
            (None, Some(calls)) => lines.push(format!("今日请求：{} 次", calls)),
            (None, None) => {}
        }
        if !self.today_models.is_empty() {
            let models: Vec<String> = self
                .today_models
                .iter()
                .map(|(name, usd)| format!("{} {}", name, money(*usd)))
                .collect();
            lines.push(format!("今日模型：{}", models.join("、")));
        }
        if let Some(week) = self.week_usd {
            lines.push(format!("近 7 天：{}", money(week)));
        }
        lines.push("今日 = 本机时区 0 点至今（按该 key 的调用日志统计）".to_string());
        if self.today_stale {
            lines.push("今日数据已跨日，请重新查询".to_string());
        }
        if self.unit_assumed {
            lines.push(
                "换算：站点没给 quota_per_unit，按默认 1 美元 = 500,000 quota 估算".to_string(),
            );
        }
        lines.join("\n")
    }
}

/// 金额格式化：`$1,234.50`。
pub fn money(value: f64) -> String {
    let sign = if value < 0.0 { "-$" } else { "$" };
    format!("{}{}", sign, grouped(value.abs(), 2))
}

/// 原值格式化（单位未知）：整数不带小数，非整数保留两位。
fn raw_num(value: Option<&Value>) -> String {
    let Some(v) = value else {
        return "?".to_string();
    };
    if let Some(n) = v.as_f64() {
        let decimals = if (n.fract()).abs() < f64::EPSILON {
            0
        } else {
            2
        };
        grouped(n, decimals)
    } else {
        v.to_string()
    }
}

/// 千分位分组；`decimals` 为小数位数。
fn grouped(value: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, value);
    let (int_part, frac) = match text.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (text, None),
    };
    let mut out = String::with_capacity(int_part.len() + int_part.len() / 3);
    // 内容来自 format!("{:.*}") 的浮点输出，必定是 ASCII，故字节下标与字符数一致。
    let total = int_part.len();
    for (idx, ch) in int_part.char_indices() {
        if idx > 0 && (total - idx) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    match frac {
        Some(f) => format!("{}.{}", out, f),
        None => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实响应：哈基米API站（公益站，额度是占位值 1e8，已用 $216.00）。
    const HAJIMI_SUB: &str = r#"{"object":"billing_subscription","has_payment_method":true,
        "soft_limit_usd":100000000,"hard_limit_usd":100000000,"system_hard_limit_usd":100000000,
        "access_until":0}"#;
    const HAJIMI_USAGE: &str = r#"{"object":"list","total_usage":21600}"#;
    const HAJIMI_STATUS: &str =
        r#"{"success":true,"data":{"system_name":"哈基米API站","version":"v1.0.0-rc.19.3518"}}"#;

    /// 真实响应：另一种形状 credit_summary（单位未标注）。
    const CREDIT_SUMMARY: &str = r#"{"object":"credit_summary","total_granted":-1300000,
        "total_used":0,"total_available":-1300000,"expires_at":0}"#;

    #[test]
    fn panel_name_combines_system_and_version() {
        assert_eq!(
            parse_panel(Some(HAJIMI_STATUS)),
            "哈基米API站 v1.0.0-rc.19.3518"
        );
        assert_eq!(parse_panel(None), "");
        assert_eq!(parse_panel(Some("not json")), "");
        assert_eq!(
            parse_panel(Some(r#"{"data":{"system_name":"SheApi"}}"#)),
            "SheApi"
        );
    }

    #[test]
    fn placeholder_limit_shows_used_only() {
        // 公益站额度是占位值 1e8：算出来的「余额」没意义，所以既不显示也不保留。
        let billing = parse(HAJIMI_SUB, Some(HAJIMI_USAGE), Some(HAJIMI_STATUS));
        assert_eq!(billing.shape, Shape::Subscription);
        assert_eq!(billing.used_usd, Some(216.0));
        assert!(billing.placeholder_limit);
        assert_eq!(billing.limit_usd, None, "占位额度不算额度");
        assert_eq!(billing.balance_usd, None, "占位额度不显示余额");
        let inline = billing.inline();
        assert_eq!(inline, "已用 $216.00");
        assert!(!inline.contains("余额"), "别出现没意义的余额：{inline}");
        assert!(
            !inline.contains('~'),
            "不要用 `~`（易被读成负号）：{inline}"
        );
        assert_eq!(billing.inline_full(), "已用 $216.00");
        let detail = billing.detail();
        // 小窗只放数据：占位额度既不显示数字，也不再写一行解释。
        assert!(!detail.contains("占位值"), "不再解释占位：{detail}");
        assert!(!detail.contains("额度"), "占位额度下不该有额度行：{detail}");
        assert!(!detail.contains("余额"), "占位额度下不该给余额行：{detail}");
        assert!(
            detail.contains("哈基米API站"),
            "detail 要带面板名：{detail}"
        );
    }

    #[test]
    fn real_limit_gives_plain_balance() {
        let sub = r#"{"object":"billing_subscription","hard_limit_usd":50.0}"#;
        let usage = r#"{"object":"list","total_usage":1234}"#;
        let billing = parse(sub, Some(usage), None);
        assert_eq!(billing.used_usd, Some(12.34), "total_usage 是美分");
        assert_eq!(billing.balance_usd, Some(37.66));
        assert!(!billing.placeholder_limit);
        assert_eq!(billing.inline(), "已用 $12.34 · 余额 $37.66");
        assert!(!billing.detail().contains("~"));
    }

    #[test]
    fn credit_summary_is_shown_raw() {
        let billing = parse(CREDIT_SUMMARY, None, None);
        assert_eq!(billing.shape, Shape::CreditSummary);
        assert!(billing.balance_usd.is_none(), "单位未知时不要算余额");
        assert_eq!(billing.inline(), "额度单位未标注（悬停看原值）");
        let raw = billing.raw_credit.clone().unwrap_or_default();
        assert!(raw.contains("-1,300,000"), "原值要带千分位：{raw}");
        assert!(billing.detail().contains("单位未标注"));
    }

    #[test]
    fn zero_hard_limit_is_treated_as_no_limit() {
        // 不少站用 hard_limit_usd: 0 表示「无硬上限」，直接算会得到负数余额。
        let sub = r#"{"object":"billing_subscription","hard_limit_usd":0,"soft_limit_usd":0}"#;
        let usage = r#"{"object":"list","total_usage":500}"#;
        let billing = parse(sub, Some(usage), None);
        assert_eq!(billing.used_usd, Some(5.0));
        assert_eq!(billing.limit_usd, None, "额度 0 不算额度");
        assert_eq!(billing.balance_usd, None);
        assert_eq!(billing.inline(), "已用 $5.00");
    }

    #[test]
    fn balance_never_shows_negative() {
        // 额度比已用小（数据不一致 / 已超支）：只说已用，不显示负数余额。
        let sub = r#"{"object":"billing_subscription","hard_limit_usd":10.0}"#;
        let usage = r#"{"object":"list","total_usage":1500}"#;
        let billing = parse(sub, Some(usage), None);
        assert_eq!(billing.balance_usd, None);
        assert_eq!(billing.inline(), "已用 $15.00");
        assert!(!billing.detail().contains("-$"), "不要出现负金额");
    }

    /// 真实际形状：`/api/status`（换算字段 + 面板名）。
    const STATUS_UNITS: &str = r#"{"success":true,"data":{"system_name":"哈基米API站",
        "version":"v1.0.0-rc.19.3518","quota_per_unit":500000,"quota_display_type":"CNY",
        "display_in_currency":true,"usd_exchange_rate":7.3}}"#;
    /// 真实际形状：`/api/usage/token/`（公益站：不限额度）。
    const USAGE_TOKEN_UNLIMITED: &str = r#"{"code":true,"message":"","data":{"name":"demo",
        "unlimited_quota":true,"total_granted":0,"total_used":137000000,
        "total_available":-137000000,"expires_at":0}}"#;
    /// 真实际形状：`/api/log/token`（字段取自实测；`content` / `ip` 等隐私字段不入测试）。
    const TOKEN_LOGS: &str = r#"{"success":true,"message":"","data":[
        {"created_at":1789436810,"type":2,"model_name":"deepseek-v4-flash","quota":1000000},
        {"created_at":1789436900,"type":2,"model_name":"grok-4.6","quota":500000},
        {"created_at":1789350000,"type":2,"model_name":"deepseek-v4-flash","quota":2000000}]}"#;

    #[test]
    fn units_come_from_status_and_fall_back_when_missing() {
        let units = parse_units(Some(STATUS_UNITS));
        assert_eq!(units.quota_per_unit, 500_000.0);
        assert_eq!(units.currency, "CNY");
        assert!(!units.assumed);

        for payload in [None, Some("not json"), Some(r#"{"data":{}}"#)] {
            let units = parse_units(payload);
            assert_eq!(units.quota_per_unit, QUOTA_PER_USD);
            assert!(units.assumed, "站点没给换算比要标记出来");
        }
    }

    #[test]
    fn token_usage_parses_and_rejects_empty() {
        let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
        assert_eq!(usage.name, "demo");
        assert!(usage.unlimited);
        assert_eq!(usage.used, Some(137_000_000.0));

        for payload in ["", "not json", "{}", r#"{"data":[]}"#, r#"{"data":{}}"#] {
            assert!(parse_token_usage(payload).is_none(), "{payload}");
        }
    }

    #[test]
    fn token_logs_summarize_by_window_and_model() {
        let logs = parse_token_logs(TOKEN_LOGS);
        assert_eq!(logs.len(), 3);

        // 今日：前两条（第三条在昨天的窗口里）
        let today = summarize_logs(&logs, Some(1_789_430_000));
        assert_eq!(today.count, 2);
        assert_eq!(today.quota, 1_500_000.0);
        assert_eq!(
            today.models[0],
            ("deepseek-v4-flash".to_string(), 1_000_000.0)
        );
        assert_eq!(today.models[1], ("grok-4.6".to_string(), 500_000.0));

        let all = summarize_logs(&logs, None);
        assert_eq!(all.count, 3);
        assert_eq!(all.quota, 3_500_000.0);
        assert_eq!(all.models[0].1, 3_000_000.0, "同模型要合并");

        // 空日志 / 坏日志都不要 panic
        assert_eq!(parse_token_logs("not json").len(), 0);
        assert_eq!(parse_token_logs(r#"{"success":false}"#).len(), 0);
        assert_eq!(summarize_logs(&[], Some(1)).count, 0);
    }

    #[test]
    fn token_billing_describes_unlimited_station() {
        let units = parse_units(Some(STATUS_UNITS));
        let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
        let logs = parse_token_logs(TOKEN_LOGS);
        let info = parse_token_billing(TokenInputs {
            usage: Some(&usage),
            logs: &logs,
            units: &units,
            status_json: Some(STATUS_UNITS),
            now: 1_789_437_000,
            today_from: 1_789_430_000,
            note: None,
        });
        assert_eq!(info.source, Source::Token);
        assert_eq!(info.shape, Shape::TokenUnlimited);
        assert!(info.unlimited);
        assert_eq!(info.used_usd, Some(274.0), "137,000,000 quota = $274");
        assert_eq!(info.balance_usd, None, "不限额度不该编出余额");
        assert_eq!(info.limit_usd, None);
        assert_eq!(info.today_usd, Some(3.0));
        assert_eq!(info.week_usd, Some(7.0), "近 7 天含今日");
        assert_eq!(info.today_calls, Some(2));
        assert_eq!(info.today_models.len(), 2);
        assert_eq!(info.inline(), "已用 $274.00 · 今日 $3.00");
        let detail = info.detail();
        assert!(detail.contains("不限"), "{detail}");
        assert!(detail.contains("deepseek-v4-flash"), "{detail}");
        assert!(detail.contains("今日已用：$3.00（2 次请求）"), "{detail}");
    }

    #[test]
    fn token_billing_shows_balance_when_quota_is_finite() {
        let usage = parse_token_usage(
            r#"{"data":{"name":"paid","unlimited_quota":false,"total_granted":10000000,
                "total_used":2500000,"total_available":7500000}}"#,
        )
        .expect("应能解析");
        // 站点没给换算比 → 按默认并标记假设
        let units = parse_units(None);
        let info = parse_token_billing(TokenInputs {
            usage: Some(&usage),
            logs: &[],
            units: &units,
            status_json: None,
            now: 1,
            today_from: 0,
            note: None,
        });
        assert_eq!(info.shape, Shape::TokenQuota);
        assert!(!info.unlimited);
        assert_eq!(info.balance_usd, Some(15.0));
        assert_eq!(info.limit_usd, Some(20.0));
        assert_eq!(info.used_usd, Some(5.0));
        assert_eq!(info.today_usd, None, "没有日志就不编今日数字");
        assert!(info.unit_assumed);
        assert_eq!(info.inline(), "余额 $15.00 · 累计 $5.00");
        assert!(info.detail().contains("默认 1 美元 = 500,000 quota"));
    }

    #[test]
    fn detail_keeps_numbers_and_drops_the_endpoint_footnotes() {
        // 悬停小窗只放「数字 + 会影响读数的口径」（换算比、跳日）。
        // 接口来源、分页上限、只读这类实现细节在卡片上只是噪音，一律不写。
        let items: Vec<String> = (0..LOG_PAGE_LIMIT)
            .map(|i| {
                format!(
                    r#"{{"created_at":{},"model_name":"m","quota":1000}}"#,
                    1_789_430_000 + i as i64
                )
            })
            .collect();
        let payload = format!(r#"{{"success":true,"data":[{}]}}"#, items.join(","));
        let logs = parse_token_logs(&payload);
        assert_eq!(logs.len(), LOG_PAGE_LIMIT);
        let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
        let units = parse_units(Some(STATUS_UNITS));
        let info = parse_token_billing(TokenInputs {
            usage: Some(&usage),
            logs: &logs,
            units: &units,
            status_json: None,
            now: 1_789_430_000,
            today_from: 1_789_430_000,
            note: None,
        });
        let detail = info.detail();
        for noise in ["来源", "/api/", "分页", "只读", "接口"] {
            assert!(
                !detail.contains(noise),
                "「{noise}」是实现细节，不该进小窗：{detail}"
            );
        }
        assert!(detail.contains("近 7 天"), "数字要留着：{detail}");
    }

    #[test]
    fn a_degraded_result_shows_no_note_line() {
        // 降级原因（调用日志不可用、面板令牌失效）不再写进小窗：
        // 小窗只放数据，不写「备注」。
        let units = parse_units(Some(STATUS_UNITS));
        let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
        let info = parse_token_billing(TokenInputs {
            usage: Some(&usage),
            logs: &[],
            units: &units,
            status_json: None,
            now: 1_789_430_000,
            today_from: 1_789_430_000,
            note: Some("调用日志不可用（HTTP 404 Not Found），今日用量取不到".to_string()),
        });
        assert!(info.note.is_some(), "原因本身要留在结果里，不当场丢掉");
        let detail = info.detail();
        assert!(!detail.contains("备注"), "{detail}");
        assert!(!detail.contains("不可用"), "{detail}");
    }

    #[test]
    fn unsupported_site_is_unknown() {
        // 站点不支持：拿到的可能是 404 页面或空对象
        for payload in ["", "{}", "<html>404</html>", r#"{"success":false}"#] {
            let billing = parse(payload, None, None);
            assert_eq!(billing.shape, Shape::Unknown, "{payload}");
            assert_eq!(billing.inline(), "未返回额度信息");
            assert!(billing.detail().contains("没有返回可识别的账单信息"));
        }
    }

    #[test]
    fn usage_card_visibility_hides_unknown_and_empty_results() {
        let unknown = Billing::default();
        assert!(!unknown.is_displayable());

        let empty_known = Billing {
            shape: Shape::Subscription,
            ..Default::default()
        };
        assert!(!empty_known.is_displayable());

        let visible = Billing {
            shape: Shape::Subscription,
            used_usd: Some(12.5),
            ..Default::default()
        };
        assert!(visible.is_displayable());
        assert_eq!(visible.inline_full(), "已用 $12.50");
        assert!(visible.detail().contains("已用：$12.50"));
    }
    #[test]
    fn usage_without_limit_still_reports_used() {
        let billing = parse(
            r#"{"object":"billing_subscription"}"#,
            Some(HAJIMI_USAGE),
            None,
        );
        assert_eq!(billing.used_usd, Some(216.0));
        assert!(billing.balance_usd.is_none());
        assert_eq!(billing.inline(), "已用 $216.00");
    }

    #[test]
    fn money_uses_thousands_separators() {
        assert_eq!(money(100_000_000.0), "$100,000,000.00");
        assert_eq!(money(216.0), "$216.00");
        assert_eq!(money(1234.5), "$1,234.50");
        assert_eq!(money(0.0), "$0.00");
        assert_eq!(money(-1300.0), "-$1,300.00");
        assert_eq!(money(999.999), "$1,000.00");
    }

    #[test]
    fn token_log_url_requests_a_bounded_page() {
        let endpoints = endpoints("https://example.test/v1");
        assert_eq!(
            endpoints.token_logs(),
            format!("https://example.test/api/log/token?p=0&page_size={LOG_PAGE_LIMIT}")
        );
    }
    #[test]
    fn endpoints_derive_origin_and_base() {
        let ep = endpoints("https://gemai.huchan.cn/v1");
        assert_eq!(ep.status, "https://gemai.huchan.cn/api/status");
        assert_eq!(
            ep.subscription,
            "https://gemai.huchan.cn/v1/dashboard/billing/subscription"
        );
        assert_eq!(
            ep.usage,
            "https://gemai.huchan.cn/v1/dashboard/billing/usage"
        );

        // anthropic 系 baseUrl 没有 /v1，末尾斜杠要去掉
        let ep = endpoints("https://kktoken.cc/");
        assert_eq!(ep.status, "https://kktoken.cc/api/status");
        assert_eq!(
            ep.subscription,
            "https://kktoken.cc/dashboard/billing/subscription"
        );

        // 无协议头：整体当 origin（这类地址已被 baseUrl 体检标为「缺少协议头」）
        let ep = endpoints("api.example.com/v1");
        assert_eq!(ep.status, "api.example.com/v1/api/status");
    }

    #[test]
    fn endpoints_v1_only_touches_origin_only_bases() {
        // pi 页里 anthropic 系的 baseUrl 只有 origin → 备选端点补上 /v1
        let ep = endpoints_v1("https://ps.air-outer.com");
        assert_eq!(
            ep.subscription,
            "https://ps.air-outer.com/v1/dashboard/billing/subscription"
        );
        assert_eq!(
            ep.status, "https://ps.air-outer.com/api/status",
            "站点信息仍在 origin"
        );
        assert_eq!(
            endpoints_v1("https://kktoken.cc/").subscription,
            "https://kktoken.cc/v1/dashboard/billing/subscription"
        );

        // 已带 /v1：必须原样返回，否则会拼出 /v1/v1/...（实测 404）
        let same = endpoints("https://gemai.huchan.cn/v1");
        let alt = endpoints_v1("https://gemai.huchan.cn/v1");
        assert_eq!(alt.subscription, same.subscription);

        // 带了其他路径：不猜该插在哪里，原样返回
        let same = endpoints("https://host/api");
        let alt = endpoints_v1("https://host/api");
        assert_eq!(alt.subscription, same.subscription);
    }

    /// 真实响应形状：`/api/user/self`（需要面板 PAT，普通登录用户即可）。
    /// 字段名取自 new-api `buildSelfUserData`（controller/user.go）。
    const ACCOUNT_SELF: &str = r#"{"success":true,"message":"","data":{
        "id":1,"username":"tester","display_name":"Tester","role":1,"status":1,
        "group":"default","quota":6150000,"used_quota":10250000,"request_count":321}}"#;

    /// 只有账号数据、没有令牌数据时的展示（站点未开 /api/usage/token/）。
    fn account_only_billing() -> Billing {
        let units = parse_units(Some(STATUS_UNITS));
        Billing {
            source: Source::Token,
            account: parse_account_self(ACCOUNT_SELF, &units),
            ..Default::default()
        }
    }

    #[test]
    fn account_self_parses_quota_used_and_requests() {
        let units = parse_units(Some(STATUS_UNITS));
        let account = parse_account_self(ACCOUNT_SELF, &units).expect("应能解析");
        assert_eq!(account.balance_usd, Some(12.30), "6,150,000 quota = $12.30");
        assert_eq!(account.used_usd, Some(20.50), "10,250,000 quota = $20.50");
        assert_eq!(account.requests, Some(321));
        assert_eq!(account.group, "default");
    }

    #[test]
    fn account_self_rejects_failures_and_useless_payloads() {
        let units = parse_units(Some(STATUS_UNITS));
        for payload in [
            "",
            "not json",
            "{}",
            "[]",
            // PAT 无效 / 未提供：new-api 把错误放在 200 响应的 success:false 里
            r#"{"success":false,"message":"Unauthorized, invalid access token"}"#,
            // 一个可用字段都没有
            r#"{"success":true,"data":{}}"#,
            // 只有分组，没有任何数字
            r#"{"success":true,"data":{"group":"default"}}"#,
            // 类型不符
            r#"{"success":true,"data":{"quota":"nope"}}"#,
        ] {
            assert!(parse_account_self(payload, &units).is_none(), "{payload}");
        }
    }

    #[test]
    fn account_zero_quota_is_a_real_zero_balance() {
        // 额度真为 0（用完了）必须显示 $0.00，而不是当成「没拿到数据」隐掉。
        let units = parse_units(None);
        let account = parse_account_self(
            r#"{"success":true,"data":{"quota":0,"used_quota":500000,"request_count":3}}"#,
            &units,
        )
        .expect("应能解析");
        assert_eq!(account.balance_usd, Some(0.0));
        assert_eq!(account.used_usd, Some(1.0));
        assert_eq!(account.requests, Some(3));
        assert!(account.group.is_empty(), "没给分组就是空串");
    }

    #[test]
    fn account_only_result_still_shows_how_much_was_used() {
        // 实测站点（ps.air-outer.com）：`/api/usage/token/` 是 404、
        // `/api/log/token` 回 200 但 `data:[]`，所以令牌侧整条都是空的，
        // 只有 `/api/user/self` 可用——它的 `used_quota` 是这一站**唯一**的
        // 用量数字。主行只报余额的话，用户会以为连使用量都读不到了。
        let info = account_only_billing();
        let line = info.inline();
        assert!(line.contains("账号余额 $12.30"), "{line}");
        assert!(
            line.contains("已用 $20.50"),
            "用量要跟着余额一起显示：{line}"
        );
    }

    /// 账号数据 + 签到状态同一条结果（「查询用户数据」的正常形态）。
    fn account_with_checkin_billing() -> Billing {
        let units = parse_units(Some(STATUS_UNITS));
        Billing {
            source: Source::Token,
            account: parse_account_self(ACCOUNT_SELF, &units),
            checkin: parse_checkin_status(CHECKIN_STATUS, &units).ok(),
            ..Default::default()
        }
    }

    #[test]
    fn checkin_state_joins_the_summary_line() {
        let line = account_with_checkin_billing().inline();
        assert!(line.contains("账号余额 $12.30"), "{line}");
        assert!(
            line.contains("签到 今日已签 $0.30"),
            "签到要和余额同一条报出来：{line}"
        );
    }

    #[test]
    fn checkin_state_gets_its_own_detail_line() {
        let text = account_with_checkin_billing().detail();
        assert!(
            text.contains("签到：今日已签 $0.30（本月 14 次，累计获得 $13.00）"),
            "{text}"
        );
    }

    #[test]
    fn checkin_alone_counts_as_displayable_data() {
        // 账号接口挂了（要 New-Api-User 的站点很常见）、只有签到读得到：
        // 「有就输出」——这一项也要能显示出来。
        let units = parse_units(Some(STATUS_UNITS));
        let info = Billing {
            source: Source::Token,
            checkin: parse_checkin_status(CHECKIN_STATUS, &units).ok(),
            ..Default::default()
        };
        assert!(info.is_displayable(), "只有签到数据也要显示");
        let line = info.inline();
        assert!(line.contains("签到 今日已签 $0.30"), "{line}");
        assert!(
            !line.contains("未返回额度信息"),
            "有签到就不该说没数据：{line}"
        );
        // 签到是账号级信息（同一个面板令牌读的），不能被塞进「本令牌」分节。
        let text = info.detail();
        assert!(text.contains("签到：今日已签"), "{text}");
        assert!(!text.contains("本令牌"), "{text}");
    }

    #[test]
    fn no_checkin_data_means_no_checkin_word() {
        // 站点没开签到、或用户没填面板令牌：卡片上不该出现「签到」这两个字。
        // 「有就输出，没有就不输出」——不写死占位。
        let info = account_only_billing();
        assert!(!info.inline().contains("签到"), "{}", info.inline());
        assert!(!info.detail().contains("签到"), "{}", info.detail());
    }

    #[test]
    fn account_balance_leads_the_summary_line() {
        let units = parse_units(Some(STATUS_UNITS));
        let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
        let logs = parse_token_logs(TOKEN_LOGS);
        let mut info = parse_token_billing(TokenInputs {
            usage: Some(&usage),
            logs: &logs,
            units: &units,
            status_json: Some(STATUS_UNITS),
            now: 1_789_437_000,
            today_from: 1_789_430_000,
            note: None,
        });
        info.account = parse_account_self(ACCOUNT_SELF, &units);

        let line = info.inline_full();
        assert!(line.starts_with("账号余额 $12.30"), "账号余额优先：{line}");
        assert!(
            line.contains("已用 $20.50"),
            "账号侧的用量要跟着余额：{line}"
        );
        assert!(line.contains("今日 $3.00"), "令牌侧凭据仍保留：{line}");
        assert!(
            !line.contains("已用 $274.00"),
            "令牌级累计不该挤在主行（降到详情）：{line}"
        );

        let detail = info.detail();
        // 悬停小窗求紧凑：解释长句去掉、数字合并成行，但「同站点共用」这层
        // 含义要留一个短标记（否则读者会以为这是某个 sk- 令牌的余额）。
        assert!(
            detail.contains("账号级额度（面板访问令牌，同站点共用）"),
            "{detail}"
        );
        assert!(detail.contains("余额 $12.30 · 已用 $20.50"), "{detail}");
        assert!(detail.contains("请求 321 次 · 分组 default"), "{detail}");
        assert!(detail.contains("本令牌"), "两套数据要分区：{detail}");
    }

    #[test]
    fn account_only_result_is_still_displayable() {
        let plain = Billing {
            source: Source::Token,
            ..Default::default()
        };
        assert!(!plain.is_displayable(), "没数据就不显示");

        let info = account_only_billing();
        assert!(info.is_displayable(), "有账号数据就必须显示");
        assert_eq!(info.inline(), "账号余额 $12.30 · 已用 $20.50");
        assert_eq!(info.inline_full(), "账号余额 $12.30 · 已用 $20.50");
        assert!(info.detail().contains("账号级额度（面板访问令牌"));
    }

    #[test]
    fn account_detail_is_disclosed_as_account_level() {
        // 小窗不再写接口路径（用户不要来源信息），但「账号级」与
        // 「面板访问令牌」两项口径披露必须留住：否则会被读成某个 sk- 令牌的余额。
        let info = account_only_billing();
        let detail = info.detail();
        assert!(
            detail.contains("面板访问令牌"),
            "要说明靠面板令牌拿到：{detail}"
        );
        assert!(detail.contains("账号级"), "口径要说清是账号级：{detail}");
    }

    #[test]
    fn logs_only_result_still_reports_today_and_week() {
        // 部分站点（如把 baseUrl 指向中转域名）没有 /api/usage/token/ 这个面板路由，
        // 但 /api/log/token 仍可用：今日 / 近 7 天用量必须单独拿出来，
        // 不能因为额度接口缺失就把日志统计一起丢掉。
        let units = parse_units(Some(STATUS_UNITS));
        let logs = parse_token_logs(TOKEN_LOGS);
        let info = parse_token_billing(TokenInputs {
            usage: None,
            logs: &logs,
            units: &units,
            status_json: Some(STATUS_UNITS),
            now: 1_789_437_000,
            today_from: 1_789_430_000,
            note: None,
        });
        assert_eq!(info.shape, Shape::TokenLogsOnly);
        assert_eq!(info.today_usd, Some(3.0), "今日仍要算出来");
        assert_eq!(info.today_calls, Some(2));
        assert_eq!(info.week_usd, Some(7.0), "近 7 天仍要算出来");
        assert_eq!(info.today_models.len(), 2);
        assert_eq!(info.used_usd, None, "没有额度接口就不编累计");
        assert_eq!(info.balance_usd, None, "更不能编余额");
        assert_eq!(info.limit_usd, None);
        assert!(info.is_displayable(), "只有日志数据也要能显示");

        let inline = info.inline();
        assert!(inline.contains("今日 $3.00"), "{inline}");
        assert!(!inline.contains("余额"), "没有额度就不该出现余额：{inline}");

        let detail = info.detail();
        assert!(detail.contains("今日已用：$3.00（2 次请求）"), "{detail}");
        // 额度缺失不再写一行解释（小窗只放数据）。
        assert!(!detail.contains("额度"), "没有额度就不该有额度行：{detail}");
        assert!(!detail.contains("接口"), "{detail}");
        assert!(!detail.contains("/api/"), "{detail}");
    }

    #[test]
    fn logs_only_result_without_logs_is_not_displayable() {
        // 两边都没拿到：保持“无数据”，不要凭空造出一个空结果。
        let units = parse_units(None);
        let info = parse_token_billing(TokenInputs {
            usage: None,
            logs: &[],
            units: &units,
            status_json: None,
            now: 1,
            today_from: 0,
            note: None,
        });
        assert_eq!(info.shape, Shape::TokenLogsOnly);
        assert!(!info.is_displayable(), "没有任何数据就不该显示");
    }

    /// 真实际形状：`GET /api/user/checkin`（字段取自实测；records 只留两条）。
    const CHECKIN_STATUS: &str = r#"{"success":true,"data":{"enabled":true,
        "max_quota":250000,"min_quota":50000,"stats":{"checked_in_today":true,
        "checkin_count":14,"total_checkins":45,"total_quota":6500000,
        "records":[{"checkin_date":"2026-09-17","quota_awarded":150000},
                   {"checkin_date":"2026-09-16","quota_awarded":100000}]}}}"#;
    #[test]
    fn checkin_status_reads_the_real_payload() {
        let units = parse_units(Some(STATUS_UNITS));
        let status = parse_checkin_status(CHECKIN_STATUS, &units).expect("应能解析");
        assert!(status.enabled);
        assert!(status.today, "今天已签到");
        assert_eq!(status.month_count, 14);
        assert_eq!(status.total_count, 45);
        assert_eq!(status.total_usd, Some(13.0), "6_500_000 / 500_000");
        assert_eq!(status.min_usd, Some(0.1), "50_000 / 500_000");
        assert_eq!(status.max_usd, Some(0.5), "250_000 / 500_000");
    }

    #[test]
    fn checkin_status_keeps_the_site_own_refusal_message() {
        // 站点没开签到：new-api 用 200 + success:false + 这句中文报错。
        let units = parse_units(Some(STATUS_UNITS));
        let err = parse_checkin_status(r#"{"message":"签到功能未启用","success":false}"#, &units)
            .expect_err("未启用应报错");
        assert!(err.contains("签到功能未启用"), "{err}");
        // 不是 JSON 也不能当成「已签到」。
        assert!(parse_checkin_status("not json", &units).is_err());
    }

    #[test]
    fn checkin_parsers_survive_missing_fields() {
        let units = parse_units(None);
        let status = parse_checkin_status(r#"{"success":true,"data":{"enabled":true}}"#, &units)
            .expect("缺 stats 也要能解析");
        assert!(status.enabled);
        assert!(!status.today);
        assert_eq!(status.month_count, 0);
        assert_eq!(status.total_usd, None, "没给累计就不编数字");
        assert_eq!(status.min_usd, None, "额度区间缺省不编");
        assert!(
            parse_checkin_status(r#"{"success":true}"#, &units).is_err(),
            "没有 data 不算签到状态"
        );
    }

    #[test]
    fn checkin_short_and_long_describe_the_same_state() {
        let units = parse_units(Some(STATUS_UNITS));
        let status = parse_checkin_status(CHECKIN_STATUS, &units).expect("应能解析");
        // 短描述给卡片那一行：只说状态与今日金额。
        let short = checkin_short(&status);
        assert!(short.contains("今日已签"), "{short}");
        assert!(short.contains("$0.30"), "今日金额要带上：{short}");
        assert!(!short.contains("累计"), "短描述不放累计：{short}");
        // 长描述给悬停说明：短描述的内容 + 本月 / 累计。
        let long = checkin_long(&status);
        assert!(long.contains("今日已签"), "{long}");
        assert!(long.contains("本月 14 次"), "{long}");
        assert!(long.contains("累计获得 $13.00"), "{long}");
    }

    #[test]
    fn checkin_descriptions_omit_numbers_the_site_did_not_give() {
        let bare = CheckinStatus {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(checkin_short(&bare), "未签");
        let text = checkin_long(&bare);
        assert_eq!(text, "未签", "没给本月 / 累计就不提：{text}");

        // 今天签了但站点没给金额：不能编一个 $0.00。
        let no_amount = CheckinStatus {
            enabled: true,
            today: true,
            ..Default::default()
        };
        assert_eq!(checkin_short(&no_amount), "今日已签");
        assert!(!checkin_long(&no_amount).contains('$'), "没给就不编金额");

        // 站点没开签到：明确说未启用，不显示成「未签」（那是两件事）。
        let disabled = CheckinStatus::default();
        assert_eq!(checkin_short(&disabled), "未启用");
        assert_eq!(checkin_long(&disabled), "该站点未启用签到");
    }

    #[test]
    fn checkin_status_reports_todays_amount_from_the_records() {
        let units = parse_units(Some(STATUS_UNITS));
        let status = parse_checkin_status(CHECKIN_STATUS, &units).expect("应能解析");
        // fixture 的最新一条是 2026-09-17 / 150000 → $0.30。
        assert_eq!(status.today_usd, Some(0.3), "今日签到的金额");
        let text = checkin_short(&status);
        assert!(text.contains("$0.30"), "要能说清今天签了多少：{text}");
    }

    #[test]
    fn checkin_status_has_no_today_amount_when_not_signed_today() {
        let units = parse_units(Some(STATUS_UNITS));
        // 今天没签：即使有历史记录，也不能把昨天/上一条的金额当成今天。
        let not_today = parse_checkin_status(
            r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":false,
                "checkin_count":3,"records":[{"checkin_date":"2026-09-16","quota_awarded":150000}]}}}"#,
            &units,
        )
        .expect("应能解析");
        assert_eq!(not_today.today_usd, None);
        let text = checkin_short(&not_today);
        assert_eq!(text, "未签", "今天没签就直说未签");
        assert!(!text.contains('$'), "没签就不该提金额：{text}");

        // 今天签了但记录为空（站点只给标记不给明细）：同样不编金额。
        let no_records = parse_checkin_status(
            r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":true,"checkin_count":1}}}"#,
            &units,
        )
        .expect("应能解析");
        assert_eq!(no_records.today_usd, None);
        assert_eq!(checkin_short(&no_records), "今日已签", "只有标记、没金额");

        // 记录顺序被打乱时按最大日期取，不依赖站点给的先后。
        let shuffled = parse_checkin_status(
            r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":true,
                "checkin_count":2,"records":[{"checkin_date":"2026-09-10","quota_awarded":50000},
                {"checkin_date":"2026-09-17","quota_awarded":250000}]}}}"#,
            &units,
        )
        .expect("应能解析");
        assert_eq!(shuffled.today_usd, Some(0.5), "取最大日期那条");
    }
}
