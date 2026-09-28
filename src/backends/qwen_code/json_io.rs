use super::*;

/// `modelProviders` 对象；不是对象时返回 None。
pub(crate) fn providers_map(root: &Value) -> Option<&Map<String, Value>> {
    root.get("modelProviders").and_then(Value::as_object)
}

/// 顶层 `providerProtocol` 映射；缺失 / 不是对象时返回 None。
pub(crate) fn provider_protocols(root: &Value) -> Option<&Map<String, Value>> {
    root.get("providerProtocol").and_then(Value::as_object)
}

/// 条目的可用 `id`（空白不算）。
pub(crate) fn entry_id(entry: &Value) -> Option<&str> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// 某个 pid 的条目数组；值不是数组时返回空。
pub(crate) fn entries_of_pid<'a>(root: &'a Value, pid: &str) -> &'a [Value] {
    providers_map(root)
        .and_then(|m| m.get(pid))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// 文件里的全部条目，按**文件顺序**：`(pid, 条目)`。
///
/// 跳过 `qwen-oauth`（只读，不进界面）与没有 `id` 的条目（认不出来，由
/// [`unmanaged_of`] 原样保留）。
pub(crate) fn entries_from(root: &Value) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    let Some(m) = providers_map(root) else {
        return out;
    };
    for (pid, value) in m {
        if is_readonly_provider(pid) {
            continue;
        }
        for entry in value.as_array().map(Vec::as_slice).unwrap_or_default() {
            if entry_id(entry).is_some() {
                out.push((pid.clone(), entry.clone()));
            }
        }
    }
    out
}

/// 能解析成整数才写入；解析不了（含空串）就删除该键。
///
/// 写成 `unwrap_or(0)` 会把上限归零落盘，与 zcode 的 `contextWindow` 同一条教训。
pub(crate) fn set_num(obj: &mut Map<String, Value>, key: &str, text: &str) {
    match text.parse::<i64>() {
        Ok(n) => {
            obj.insert(key.to_string(), Value::Number(n.into()));
        }
        Err(_) => {
            obj.shift_remove(key);
        }
    }
}

pub(crate) fn push_entry(map: &mut Map<String, Value>, pid: &str, entry: Value) {
    match map.get_mut(pid).and_then(Value::as_array_mut) {
        Some(arr) => arr.push(entry),
        None => {
            map.insert(pid.to_string(), Value::Array(vec![entry]));
        }
    }
}
