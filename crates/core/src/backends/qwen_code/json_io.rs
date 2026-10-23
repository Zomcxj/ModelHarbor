use super::*;

/// The `modelProviders` object; `None` when it isn't one.
pub(crate) fn providers_map(root: &Value) -> Option<&Map<String, Value>> {
    root.get("modelProviders").and_then(Value::as_object)
}

/// The top-level `providerProtocol` mapping; `None` when missing or not an object.
pub(crate) fn provider_protocols(root: &Value) -> Option<&Map<String, Value>> {
    root.get("providerProtocol").and_then(Value::as_object)
}

/// An entry's usable `id` (whitespace doesn't count).
pub(crate) fn entry_id(entry: &Value) -> Option<&str> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// A pid's entry array; an empty vec when the value isn't an array.
pub(crate) fn entries_of_pid<'a>(root: &'a Value, pid: &str) -> &'a [Value] {
    providers_map(root)
        .and_then(|m| m.get(pid))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Every entry in the file, in **file order**: `(pid, entry)`.
///
/// Skips `qwen-oauth` (read-only, never in the UI) and entries without an `id`
/// (the latter preserved verbatim by [`unmanaged_of`]).
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

/// Written only when it parses as an integer; unparseable (empty strings included) deletes the key.
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
