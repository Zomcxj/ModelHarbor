//! Units and quota: `quota` ↔ USD, the token-quota endpoint, the station's conversion ratio.
use serde_json::Value;

/// A quota at or above this counts as a placeholder.
pub const PLACEHOLDER_LIMIT_USD: f64 = 1_000_000.0;

/// The panel account's quota unit: `quota` ÷ this = USD (New-API / one-api fix it at 500000).
pub const QUOTA_PER_USD: f64 = 500_000.0;

/// The max entries the station's log endpoint returns per page; reaching it means stats may undercount.
pub const LOG_PAGE_LIMIT: usize = 1000;

/// The station's unit settings (`/api/status`): the basis for converting quota points ↔ currency.
#[derive(Clone, Debug, PartialEq)]
pub struct Units {
    /// How many quota points one currency unit is (station-configurable, default 500000).
    pub quota_per_unit: f64,
    /// The display currency (`USD` / `CNY` / empty).
    pub currency: String,
    /// `true` when the station gave no conversion ratio.
    pub assumed: bool,
}

/// Parse the conversion fields in `/api/status` (defaults + `assumed` when unavailable).
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

/// Token quota info (`/api/usage/token/`, **only needs the `sk-` key**); values are quota points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TokenUsage {
    /// The token's name (as shown in the panel).
    pub name: String,
    /// Quota-free; only `used` is meaningful then.
    pub unlimited: bool,
    pub granted: Option<f64>,
    pub used: Option<f64>,
    pub available: Option<f64>,
    /// The expiry time (second-resolution Unix timestamp, 0 = no expiry).
    pub expires_at: Option<f64>,
}

/// Parse the token quota; `None` when no usable field exists (the station doesn't support it).
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
