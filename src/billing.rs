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
    /// 今日各模型消耗（美元，降序，最多 3 项）。
    pub today_models: Vec<(String, f64)>,
    /// 近 7 天已用（美元，含今日）。
    pub week_usd: Option<f64>,
    /// 日志条数达到站点上限：统计可能偏小（悬停里说明）。
    pub log_capped: bool,
    /// 站点没给 `quota_per_unit`，换算按默认 500000 假设（悬停里说明）。
    pub unit_assumed: bool,
    /// 降级 / 缺数据时的说明（日志接口不可用、换算比缺失等），用干提示而不是静默降级。
    pub note: Option<String>,
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

    /// 该令牌的调用日志（用于算今日 / 近 7 天用量，**只需 `sk-` key**）。
    pub fn token_logs(&self) -> String {
        format!("{}/api/log/token", self.origin)
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

/// 一条调用日志（只留统计要用的字段；`content` / `ip` 这类隐私字段不解析）。
#[derive(Clone, Debug, PartialEq)]
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
    pub usage: &'a TokenUsage,
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
    let capped = inputs.logs.len() >= LOG_PAGE_LIMIT;
    Billing {
        panel: parse_panel(inputs.status_json),
        source: Source::Token,
        shape: if inputs.usage.unlimited {
            Shape::TokenUnlimited
        } else {
            Shape::TokenQuota
        },
        used_usd: inputs.usage.used.map(to_money),
        // 不限额度时没有「余额 / 上限」可言，不编数字。
        balance_usd: (!inputs.usage.unlimited)
            .then(|| inputs.usage.available.map(to_money))
            .flatten(),
        limit_usd: (!inputs.usage.unlimited)
            .then(|| {
                inputs
                    .usage
                    .granted
                    .filter(|value| *value > 0.0)
                    .map(to_money)
            })
            .flatten(),
        unlimited: inputs.usage.unlimited,
        today_usd: (!today.models.is_empty() || today.count > 0).then(|| to_money(today.quota)),
        today_calls: (today.count > 0).then_some(today.calls),
        today_models: today
            .models
            .iter()
            .map(|(name, quota)| (name.clone(), to_money(*quota)))
            .collect(),
        week_usd: (week.count > 0).then(|| to_money(week.quota)),
        log_capped: capped,
        unit_assumed: inputs.units.assumed,
        note: inputs.note,
        ..Default::default()
    }
}

impl Billing {
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
    pub fn inline_full(&self) -> String {
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
        if self.source == Source::Token {
            return self.detail_token(lines);
        }
        if let Some(used) = self.used_usd {
            lines.push(format!("已用：{}", money(used)));
        }
        if let Some(limit) = self.limit_usd {
            lines.push(format!("额度：{}", money(limit)));
        }
        if self.placeholder_limit {
            lines.push("额度：站点给的是占位值（公益站常见），所以不显示余额".to_string());
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
        lines.push("已用为账号累计值（接口忽略日期参数，取不到日用量）".to_string());
        lines.push("来源：/dashboard/billing/subscription + /usage（只读管理接口）".to_string());
        lines.join("\n")
    }

    /// 令牌额度（`/api/usage/token/` + `/api/log/token`）的悬停详情。
    fn detail_token(&self, mut lines: Vec<String>) -> String {
        if self.unlimited {
            lines.push("额度：不限（该令牌不计额度，只有「已用」有意义，不存在余额）".to_string());
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
        if self.unit_assumed {
            lines.push(
                "换算：站点没给 quota_per_unit，按默认 1 美元 = 500,000 quota 估算".to_string(),
            );
        }
        if self.log_capped {
            lines.push(format!(
                "日志：已达站点上限 {} 条，超出的部分统计不到（数字会偏小）",
                LOG_PAGE_LIMIT
            ));
        }
        if let Some(note) = &self.note {
            lines.push(format!("备注：{}", note));
        }
        lines.push("来源：/api/usage/token/ + /api/log/token（只需 API key，只读）".to_string());
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
        assert!(detail.contains("占位值"), "detail 要说明占位：{detail}");
        assert!(
            !detail.contains("余额："),
            "占位额度下不该给余额行：{detail}"
        );
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
            usage: &usage,
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
            usage: &usage,
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
    fn log_cap_is_disclosed() {
        // 站点日志上限 1000 条：到顶了要说清楚，别让人以为算全了。
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
            usage: &usage,
            logs: &logs,
            units: &units,
            status_json: None,
            now: 1_789_430_000,
            today_from: 1_789_430_000,
            note: None,
        });
        assert!(info.log_capped);
        assert!(info.detail().contains("已达站点上限"), "{}", info.detail());
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
}
