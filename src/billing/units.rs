//! 单位与额度：`quota` ↔ 美元、令牌额度接口、站点换算比。
use serde_json::Value;

/// 额度 ≥ 该值即视为占位（公益站几乎都给 1e8）。
pub const PLACEHOLDER_LIMIT_USD: f64 = 1_000_000.0;

/// 面板账户的额度单位：`quota` ÷ 该值 = 美元（New-API / one-api 固定 500000）。
pub const QUOTA_PER_USD: f64 = 500_000.0;

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
