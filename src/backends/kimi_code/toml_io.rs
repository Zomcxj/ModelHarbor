use serde_json::{Map, Value};

/// TOML 文本 → Value。空文本视为空对象。
///
/// 不用 `util::parse_config_content`：那是 JSONC 解析器，TOML 进不去。
pub(crate) fn parse_toml(content: &str) -> Result<Value, String> {
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    toml::from_str(content).map_err(|e| format!("解析失败: {}", e))
}

/// Value → TOML 文本。
///
/// **必须先剔除 `null`**：TOML 没有 null 类型，`toml` 遇到 `Value::Null` 会直接报
/// `unsupported unit type`，整次保存失败。JSONC 后端里 `null` 是合法值（serde_json
/// 也照写），跨格式从那些页面转过来时完全可能出现，所以在写之前统一清掉——
/// 「键不存在」与「键为 null」对 Kimi 是同一件事，删掉没有语义损失。
pub(crate) fn to_toml_string(root: &Value) -> Result<String, String> {
    let cleaned = strip_nulls(root);
    toml::to_string(&cleaned).map_err(|e| format!("序列化失败: {}", e))
}

/// 递归删掉所有 `null` 值（对象里删键，数组里删元素）。
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

/// provider 条目的 `type`（Kimi 必填字段）。
pub(crate) fn provider_type(entry: &Value) -> &str {
    entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
}

/// 非空字符串：`Some` 表示这个键**确实被设置了**。
///
/// 与 Kimi 源码的 `nonEmptyString` 同义：空串视同未设置。这是 XOR 判据的核心——
/// `api_key = ""` 与 `oauth` 并存是合法的（本机文件就是这样）。
pub(crate) fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// 能解析成**≥1 的整数**才写入；否则删键（`max_context_size` 要求 ≥1）。
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

/// 空表不写出来。
///
/// Kimi 自己的 `setRecordSection` 就是「转换后为空则删键」，照做才能让它读到的文件
/// 与自己写出的形态一致；而且 `[providers]` 这种空表在 TOML 里是**合法但多余**的，
/// 凭空多出来只会让文件看着像被改坏了。
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
