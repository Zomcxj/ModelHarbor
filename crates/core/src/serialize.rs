//! Config serialization: the layout implementations of Compact / Pretty JSON output styles.

use serde_json::{Map, Value};

/// Join the accumulated merge groups into one line: each item loses its sub-indent, joined with ", ". Empty groups return an empty string.
fn joined_group(current: &[String], child_indent: &str) -> String {
    let Some((first, rest)) = current.split_first() else {
        return String::new();
    };
    let rest: Vec<&str> = rest
        .iter()
        .map(|f| f.strip_prefix(child_indent).unwrap_or(f.as_str()))
        .collect();
    let mut parts: Vec<&str> = Vec::with_capacity(1 + rest.len());
    parts.push(first);
    parts.extend(rest.iter());
    parts.join(", ")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompactRole {
    Normal,
    AgentContainer,
    ProviderContainer,
    ProviderEntry,
    ModelsContainer,
    Target,
}

pub fn compact_json(root: &Value) -> String {
    let mut lines = serialize_object(
        root.as_object().unwrap_or(&Map::new()),
        0,
        CompactRole::Normal,
    );
    lines.push('\n');
    lines
}

pub fn pretty_json(root: &Value) -> String {
    let mut lines = serialize_pretty(
        root.as_object().unwrap_or(&Map::new()),
        0,
        CompactRole::Normal,
    );
    lines.push('\n');
    lines
}

const PRETTY_LINE_MAX: usize = 100;

pub(super) fn serialize_pretty(
    object: &Map<String, Value>,
    level: usize,
    role: CompactRole,
) -> String {
    let indent = "  ".repeat(level);
    let child_indent = "  ".repeat(level + 1);

    let mut raw_entries: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in object.iter() {
        let child_role = child_role(role, key);
        let value_lines = serialize_pretty_value(value, level + 1, child_role);
        let prefix = format!("{}{}: ", child_indent, json_string(key));
        raw_entries.push((prefix, value_lines));
    }

    let mut lines = vec![format!("{}{{", indent)];
    let mut current: Vec<String> = Vec::new();

    for (pi, (prefix, value_lines)) in raw_entries.iter().enumerate() {
        let single_line = value_lines.len() == 1;
        let has_more = pi + 1 < raw_entries.len();

        if single_line {
            let field = format!("{}{}", prefix, value_lines[0].trim_start());
            if current.is_empty() {
                current.push(field);
                continue;
            }
            let candidate_len: usize =
                current.iter().map(|f| f.len()).sum::<usize>() + current.len() - 1
                    + 2
                    + field.len();
            if candidate_len <= PRETTY_LINE_MAX {
                current.push(field);
                continue;
            }
        }

        if !current.is_empty() {
            lines.push(format!("{},", joined_group(&current, &child_indent)));
            current.clear();
        }

        if single_line {
            current.push(format!("{}{}", prefix, value_lines[0].trim_start()));
        } else {
            let mut entry_lines: Vec<String> = value_lines
                .iter()
                .enumerate()
                .map(|(i, line)| {
                    if i == 0 {
                        format!("{}{}", prefix, line.trim_start())
                    } else {
                        line.clone()
                    }
                })
                .collect();
            if has_more {
                if let Some(last) = entry_lines.last_mut() {
                    last.push(',');
                }
            }
            lines.extend(entry_lines);
        }
    }
    if !current.is_empty() {
        lines.push(joined_group(&current, &child_indent));
    }
    lines.push(format!("{}}}", indent));
    lines.join("\n")
}

pub(super) fn serialize_pretty_value(
    value: &Value,
    level: usize,
    role: CompactRole,
) -> Vec<String> {
    match value {
        Value::Object(object) => {
            if level > 0 && is_leaf_object(object) {
                let single = compact_object_one_line(object);
                let prefix_len = "  ".repeat(level).len() + 2;
                if single.len() + prefix_len <= PRETTY_LINE_MAX {
                    return vec![single];
                }
            }
            serialize_pretty(object, level, role)
                .lines()
                .map(String::from)
                .collect()
        }
        Value::Array(items) if items.iter().any(|v| v.is_object()) => {
            let indent = "  ".repeat(level);
            let child_indent = "  ".repeat(level + 1);
            let mut lines = vec![format!("{}[", indent)];
            for (i, item) in items.iter().enumerate() {
                let item_lines = serialize_pretty_value(item, level + 1, role);
                let mut entry: Vec<String> = item_lines
                    .into_iter()
                    .enumerate()
                    .map(|(j, line)| {
                        if j == 0 {
                            format!("{}{}", child_indent, line.trim_start())
                        } else {
                            line
                        }
                    })
                    .collect();
                if i + 1 < items.len() {
                    if let Some(last) = entry.last_mut() {
                        last.push(',');
                    }
                }
                lines.extend(entry);
            }
            lines.push(format!("{}]", indent));
            lines
        }
        _ => vec![compact_json_value(value)],
    }
}

pub(super) fn is_leaf_object(object: &Map<String, Value>) -> bool {
    if object.is_empty() {
        return true;
    }
    object.values().all(|v| {
        matches!(
            v,
            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null | Value::Array(_)
        ) || (v.is_object() && v.as_object().is_some_and(|o| o.is_empty()))
    })
}

pub(super) fn compact_object_one_line(object: &Map<String, Value>) -> String {
    if object.is_empty() {
        return "{}".into();
    }
    let inner: Vec<String> = object
        .iter()
        .map(|(k, v)| format!("{}: {}", json_string(k), compact_json_value(v)))
        .collect();
    format!("{{ {} }}", inner.join(", "))
}

pub fn serialize_object(object: &Map<String, Value>, level: usize, role: CompactRole) -> String {
    let indent = "  ".repeat(level);
    let child_indent = "  ".repeat(level + 1);

    if level > 0
        && !matches!(
            role,
            CompactRole::AgentContainer
                | CompactRole::ProviderContainer
                | CompactRole::ModelsContainer
        )
    {
        return serialize_fields(object, level, role);
    }

    let mut lines = vec![format!("{}{{", indent)];
    for (index, (key, value)) in object.iter().enumerate() {
        let child_role = child_role(role, key);
        let value_lines = serialize_value(value, level + 1, child_role);
        let prefix = format!("{}{}: ", child_indent, json_string(key));
        let mut entry_lines: Vec<String> = value_lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    format!("{}{}", prefix, line.trim_start())
                } else {
                    line
                }
            })
            .collect();
        if index + 1 < object.len() {
            if let Some(last) = entry_lines.last_mut() {
                last.push(',');
            }
        }
        lines.extend(entry_lines);
    }
    lines.push(format!("{}}}", indent));
    lines.join("\n")
}

pub(super) fn child_role(parent: CompactRole, key: &str) -> CompactRole {
    match (parent, key) {
        // Singular is v1, plural is v2; pi / omp's top-level plural providers also hit,
        // but with the array item-role conversion (see array_item_role) their output is unchanged.
        (CompactRole::Normal, "agent" | "agents") => CompactRole::AgentContainer,
        (CompactRole::Normal, "provider" | "providers") => CompactRole::ProviderContainer,
        (CompactRole::ProviderContainer, _) => CompactRole::ProviderEntry,
        (CompactRole::ProviderEntry, "models") => CompactRole::ModelsContainer,
        (CompactRole::AgentContainer, _) | (CompactRole::ModelsContainer, _) => CompactRole::Target,
        _ => CompactRole::Normal,
    }
}

/// Object arrays' item role: arrays under a container role (like pi's models array) treat entries as
/// Target, keeping field grouping; the rest inherit the parent role.
fn array_item_role(parent: CompactRole) -> CompactRole {
    match parent {
        CompactRole::AgentContainer
        | CompactRole::ProviderContainer
        | CompactRole::ProviderEntry
        | CompactRole::ModelsContainer => CompactRole::Target,
        _ => parent,
    }
}

pub(super) fn serialize_value(value: &Value, level: usize, role: CompactRole) -> Vec<String> {
    match value {
        Value::Object(object) => serialize_object(object, level, role)
            .lines()
            .map(String::from)
            .collect(),
        Value::Array(items) if items.iter().any(|v| v.is_object()) => {
            let indent = "  ".repeat(level);
            let child_indent = "  ".repeat(level + 1);
            let item_role = array_item_role(role);
            let mut lines = vec![format!("{}[", indent)];
            for (i, item) in items.iter().enumerate() {
                let item_lines = serialize_value(item, level + 1, item_role);
                let mut entry: Vec<String> = item_lines
                    .into_iter()
                    .enumerate()
                    .map(|(j, line)| {
                        if j == 0 {
                            format!("{}{}", child_indent, line.trim_start())
                        } else {
                            line
                        }
                    })
                    .collect();
                if i + 1 < items.len() {
                    if let Some(last) = entry.last_mut() {
                        last.push(',');
                    }
                }
                lines.extend(entry);
            }
            lines.push(format!("{}]", indent));
            lines
        }
        _ => vec![compact_json_value(value)],
    }
}

pub(super) fn has_nested_obj_array(value: &Value) -> bool {
    match value {
        Value::Array(arr) => arr.iter().any(|v| v.is_object()),
        Value::Object(obj) => obj.values().any(has_nested_obj_array),
        _ => false,
    }
}

pub(super) fn serialize_fields(
    object: &Map<String, Value>,
    level: usize,
    role: CompactRole,
) -> String {
    let indent = "  ".repeat(level);
    let field_indent = "  ".repeat(level + 1);
    let mut lines = vec!["{".into()];
    let mut current = String::new();
    let fields: Vec<_> = object.iter().collect();

    for (index, (key, value)) in fields.iter().enumerate() {
        // Tier tables (v1 object / v2 `[{id}]` array) are always single-line; judged before nested object arrays
        // so v2's array shape never multi-line-expands.
        if *key == "variants" {
            let single = compact_variants(value);
            let prefix = format!("{}: ", json_string(key));
            if field_indent.chars().count() + prefix.chars().count() + single.chars().count() <= 150
            {
                let field = format!("{}{}", prefix, single);
                if current.is_empty() {
                    current = field;
                    continue;
                }
                let candidate = format!("{}, {}", current, field);
                if candidate.chars().count() + field_indent.chars().count() > 150 {
                    lines.push(format!("{},", field_indent.clone() + &current));
                    current = field;
                } else {
                    current = candidate;
                }
            } else {
                if !current.is_empty() {
                    lines.push(format!("{},", field_indent.clone() + &current));
                    current.clear();
                }
                lines.push(format!("{}{}{}", field_indent, prefix, single));
            }
            continue;
        }
        let has_obj_array = has_nested_obj_array(value);
        let prefix = format!("{}: ", json_string(key));

        if has_obj_array {
            if !current.is_empty() {
                lines.push(format!("{},", field_indent.clone() + &current));
                current.clear();
            }
            let rendered = serialize_value(value, level + 1, child_role(role, key));
            let mut nested: Vec<String> = rendered
                .into_iter()
                .enumerate()
                .map(|(i, line)| {
                    if i == 0 {
                        format!("{}{}{}", field_indent, prefix, line.trim_start())
                    } else {
                        line
                    }
                })
                .collect();
            if index + 1 < fields.len() {
                if let Some(last) = nested.last_mut() {
                    last.push(',');
                }
            }
            lines.extend(nested);
            continue;
        }

        let single = compact_json_value(value);
        if field_indent.chars().count() + prefix.chars().count() + single.chars().count() <= 150 {
            let field = format!("{}{}", prefix, single);
            if current.is_empty() {
                current = field;
                continue;
            }
            let candidate = format!("{}, {}", current, field);
            if candidate.chars().count() + field_indent.chars().count() > 150 {
                lines.push(format!("{},", field_indent.clone() + &current));
                current = field;
            } else {
                current = candidate;
            }
            continue;
        }

        if !current.is_empty() {
            lines.push(format!("{},", field_indent.clone() + &current));
            current.clear();
        }
        let rendered = serialize_value(value, level + 1, child_role(role, key));
        let mut nested: Vec<String> = rendered
            .into_iter()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    format!("{}{}{}", field_indent, prefix, line.trim_start())
                } else {
                    line
                }
            })
            .collect();
        if index + 1 < fields.len() {
            if let Some(last) = nested.last_mut() {
                last.push(',');
            }
        }
        lines.extend(nested);
    }
    if !current.is_empty() {
        lines.push(format!("{}{}", field_indent, current));
    }
    lines.push(format!("{}}}", indent));
    lines.join("\n")
}

pub(super) fn compact_json_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".into())
}

pub(super) fn compact_variants(value: &Value) -> String {
    match value {
        // v1: object tier tables single-line.
        Value::Object(object) => {
            let entries: Vec<String> = object
                .iter()
                .map(|(k, v)| format!("{}: {}", json_string(k), compact_json_value(v)))
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        // v2: `[{id}]` arrays single-line; non-object items fall back to the original compact form.
        Value::Array(items) => {
            let entries: Vec<String> = items
                .iter()
                .map(|item| match item.as_object() {
                    Some(object) => compact_object_one_line(object),
                    None => compact_json_value(item),
                })
                .collect();
            format!("[ {} ]", entries.join(", "))
        }
        _ => compact_json_value(value),
    }
}

pub(super) fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A v2-shaped root: plural top-level keys + package/settings + capabilities + a variants array.
    fn v2_root() -> Value {
        json!({
            "agents": {
                "writing": {
                    "mode": "subagent",
                    "description": "写文档",
                    "model": "sensenova/flash#high",
                    "request": { "body": { "temperature": 0.3 } },
                    "system": "负责文案。"
                }
            },
            "providers": {
                "sensenova": {
                    "package": "@opencode/ai/providers/openai-compatible",
                    "settings": { "baseURL": "https://fake.example.com/v1", "apiKey": "sk-fake", "timeout": 180000 },
                    "models": {
                        "flash": {
                            "name": "flash",
                            "capabilities": { "tools": true, "input": ["text"], "output": ["text"] },
                            "limit": { "context": 256000, "output": 64000 },
                            "variants": [ { "id": "medium" }, { "id": "high" } ]
                        }
                    }
                }
            }
        })
    }

    /// A pi-shaped root: top-level plural providers + models as an object array.
    fn pi_root() -> Value {
        json!({
            "providers": {
                "sensenova": {
                    "baseUrl": "https://fake.example.com/v1",
                    "api": "openai-completions",
                    "models": [
                        { "id": "flash", "name": "flash", "contextWindow": 256000 }
                    ]
                }
            }
        })
    }

    #[test]
    fn compact_never_exceeds_pretty_for_v2_shape() {
        let root = v2_root();
        let compact = compact_json(&root);
        let pretty = pretty_json(&root);
        assert!(
            compact.lines().count() < pretty.lines().count(),
            "压缩格式行数（{}）必须少于默认格式（{}）",
            compact.lines().count(),
            pretty.lines().count()
        );
    }

    #[test]
    fn v2_variants_array_stays_on_one_line() {
        let text = compact_json(&v2_root());
        let line = text
            .lines()
            .find(|l| l.contains("\"variants\""))
            .expect("输出里应有 variants");
        assert!(line.contains("\"id\""), "variants 数组须单行渲染：{line}");
    }

    #[test]
    fn compact_output_parses_back_to_the_same_value() {
        let root = v2_root();
        let text = compact_json(&root);
        let parsed: Value = serde_json::from_str(&text).expect("压缩输出须是合法 JSON");
        assert_eq!(parsed, root);
    }

    #[test]
    fn pi_models_array_entries_stay_field_grouped() {
        // pi's models is an object array: entries' fields still group at 150 characters (not one field per line).
        let text = compact_json(&pi_root());
        let model_line = text
            .lines()
            .find(|l| l.contains("\"id\": \"flash\""))
            .expect("应找到模型条目行");
        assert!(
            model_line.contains("\"contextWindow\""),
            "模型字段应分组在同一行：{model_line}"
        );
        // The top-level plural providers takes the container role like the singular: entries become blocks.
        assert!(text.contains("  \"providers\": {"));
        assert!(text.contains("    \"sensenova\": {"));
    }

    #[test]
    fn v1_singular_keys_still_get_container_roles() {
        // When the model table exceeds the single-line threshold, each model becomes a block; v1 tier objects single-line.
        let root = json!({
            "agent": { "a": { "mode": "subagent", "model": "x/y", "system": "s" } },
            "provider": {
                "p": {
                    "npm": "@ai-sdk/x",
                    "models": {
                        "flash-lite": {
                            "name": "flash-lite", "reasoning": true, "tool_call": true,
                            "limit": { "context": 256000, "output": 64000 },
                            "variants": { "medium": {}, "high": {} }
                        },
                        "flash-pro": {
                            "name": "flash-pro", "reasoning": true, "tool_call": true,
                            "limit": { "context": 262000, "output": 64000 },
                            "variants": { "high": {} }
                        }
                    }
                }
            }
        });
        let text = compact_json(&root);
        // v1 tier objects single-line: the first model's tier table expands in order on one line.
        let variants_line = text
            .lines()
            .find(|l| l.contains("\"variants\""))
            .expect("应找到 variants");
        assert_eq!(
            variants_line.trim(),
            "\"variants\": { \"medium\": {}, \"high\": {} }"
        );
        // The two models each become blocks (never the whole table crammed onto one line).
        assert!(text.contains("\"flash-lite\": {"));
        assert!(text.contains("\"flash-pro\": {"));
    }
}
