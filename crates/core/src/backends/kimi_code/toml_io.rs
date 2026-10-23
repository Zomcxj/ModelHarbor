use serde_json::{Map, Value};

/// TOML text → Value. Empty text counts as an empty object.
pub(crate) fn parse_toml(content: &str) -> Result<Value, String> {
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    toml::from_str(content).map_err(|e| format!("解析失败: {}", e))
}

/// Value → TOML text.
///
/// Recursively strips `null` before writing: TOML has no null type.
pub(crate) fn to_toml_string(root: &Value) -> Result<String, String> {
    let cleaned = strip_nulls(root);
    toml::to_string(&cleaned).map_err(|e| format!("序列化失败: {}", e))
}

/// Recursively delete every `null` value (keys in objects, elements in arrays).
pub(crate) fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter(|v| !v.is_null())
                .map(strip_nulls)
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub(crate) fn providers_map(root: &Value) -> Option<&Map<String, Value>> {
    root.get("providers").and_then(Value::as_object)
}

pub(crate) fn models_map(root: &Value) -> Option<&Map<String, Value>> {
    root.get("models").and_then(Value::as_object)
}

/// A provider entry's `type` (required by Kimi).
pub(crate) fn provider_type(entry: &Value) -> &str {
    entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
}

/// A non-empty string: `Some` means the key was genuinely set.
///
/// Empty strings count as unset.
pub(crate) fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Written only when it parses to an integer ≥1; otherwise the key is deleted.
pub(crate) fn set_num_min1(obj: &mut Map<String, Value>, key: &str, text: &str) {
    match text.parse::<i64>() {
        Ok(n) if n >= 1 => {
            obj.insert(key.to_string(), Value::Number(n.into()));
        }
        _ => {
            obj.shift_remove(key);
        }
    }
}

/// Empty tables aren't written out.
pub(crate) fn set_or_drop_table(
    root: &mut Map<String, Value>,
    key: &str,
    table: Map<String, Value>,
) {
    if table.is_empty() {
        root.shift_remove(key);
    } else {
        root.insert(key.to_string(), Value::Object(table));
    }
}
