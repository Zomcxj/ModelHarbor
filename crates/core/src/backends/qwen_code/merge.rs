use super::*;

/// Which pid a card lands in, whether to write `providerProtocol`, and the entry's `wireApi`.
///
/// A card's pre-existing **custom** pid is kept first (when that pid is already in the mapping table),
/// with only the mapping refreshed to the current protocol; new cards (or converts from other backends)
/// have no pid and fall back to the built-in one.
pub(crate) fn route(
    p: &ProviderRow,
    api: &str,
    protocols: Option<&Map<String, Value>>,
) -> (String, Option<String>, &'static str) {
    let (builtin_pid, wire) = pid_for_api(api);
    let custom = p.qwen_pid.trim();
    if !custom.is_empty() && !is_builtin_pid(custom) && !is_readonly_provider(custom) {
        let known = protocols
            .and_then(|m| m.get(custom))
            .and_then(Value::as_str)
            .is_some();
        if known {
            return (custom.to_string(), Some(builtin_pid.to_string()), wire);
        }
    }
    (builtin_pid.to_string(), None, wire)
}

/// One entry to write: its pid, that pid's needed mapping, and the entry content.
pub(crate) struct Placed {
    pid: String,
    /// `Some(protocol bucket name)` = this pid is custom and must be declared in `providerProtocol`.
    mapping: Option<String>,
    entry: Value,
}

/// UI state → every entry (`serialize_root`'s sole entry source).
pub(crate) fn place(providers: &[ProviderRow], base: &Value) -> Vec<Placed> {
    let protocols = provider_protocols(base);
    let mut out: Vec<Placed> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let api = p.effective_api();
        let (pid, mapping, wire) = route(p, &api, protocols);
        if is_readonly_provider(&pid) {
            // `route` never produces read-only pids; this is a second guard.
            continue;
        }
        // Old entries index by id; unmanaged keys (`capabilities.agent`,
        // `generationConfig.maxRetries` etc.) inherit from here.
        let index: HashMap<&str, &Value> = entries_of_pid(base, &pid)
            .iter()
            .filter_map(|e| entry_id(e).map(|id| (id, e)))
            .collect();
        // A card with no models still keeps an entry (id falls back to the key).
        let models: Vec<Option<&ModelRow>> = if p.models.is_empty() {
            vec![None]
        } else {
            p.models.iter().map(Some).collect()
        };
        for m in models {
            let id = m
                .map(|m| m.id.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| p.key.trim())
                .to_string();
            let old = index.get(id.as_str()).copied();
            let entry = entry_from_provider(p, m, &id, wire, old);
            out.push(Placed {
                pid: pid.clone(),
                mapping: mapping.clone(),
                entry,
            });
        }
    }
    out
}

/// Group by pid (no disable concept — every entry enters the main config).
pub(crate) fn group_by_pid(placed: &[Placed]) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for item in placed {
        push_entry(&mut out, &item.pid, item.entry.clone());
    }
    out
}

/// `providerProtocol`'s content: derived from UI entries, preserving unrecognized pids' existing mappings.
///
/// Custom pids the UI no longer needs get their mappings cleared; "a custom-pid entry exists ⇒ a mapping exists".
pub(crate) fn protocols_for(placed: &[Placed], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for item in placed {
        if let Some(mapping) = &item.mapping {
            out.insert(item.pid.clone(), Value::String(mapping.clone()));
        }
    }
    if let (Some(old), Some(map)) = (provider_protocols(base), providers_map(base)) {
        for (pid, value) in old {
            // Pids whose values aren't arrays are the old wrapper shape, carried over whole by `unmanaged_of`,
            // their mappings kept too.
            let unmanaged = map.get(pid).is_some_and(|v| !v.is_array());
            if unmanaged && !out.contains_key(pid) {
                out.insert(pid.clone(), value.clone());
            }
        }
    }
    out
}

/// The base's **unrecognized** entries, carried over verbatim.
///
/// Returns `pid → what to write for this pid`: an array means "append into this pid's array"; any other
/// value means "replace this pid whole", produced only when the UI has no entries for that pid this time.
pub(crate) fn unmanaged_of(base: &Value, placed: &[Placed]) -> Vec<(String, Value)> {
    let managed: HashSet<&str> = placed.iter().map(|p| p.pid.as_str()).collect();
    let mut out: Vec<(String, Value)> = Vec::new();
    let Some(m) = providers_map(base) else {
        return out;
    };
    for (pid, value) in m {
        // `qwen-oauth` is preserved whole: officially hard-coded, non-overridable, and invisible to the UI.
        if is_readonly_provider(pid) {
            out.push((pid.clone(), value.clone()));
            continue;
        }
        match value.as_array() {
            Some(items) => {
                let extra: Vec<Value> = items
                    .iter()
                    .filter(|e| entry_id(e).is_none())
                    .cloned()
                    .collect();
                if !extra.is_empty() {
                    out.push((pid.clone(), Value::Array(extra)));
                }
            }
            // A non-array value (the old wrapper shape `{protocol, models}`): preserved whole, only when no UI
            // entry lands on this pid this time.
            None => {
                if !managed.contains(pid.as_str()) {
                    out.push((pid.clone(), value.clone()));
                }
            }
        }
    }
    out
}

/// Write the UI's keys back into the top-level `env`.
///
/// **Only adds and updates, never deletes.** `env` is a flat namespace shared across providers, and Qwen
/// Code's own `/auth` writes into it too. Clearing a key in the UI never touches `env`.
pub(crate) fn sync_env(root: &mut Map<String, Value>, providers: &[ProviderRow]) {
    let mut env = root
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for p in providers {
        let key = env_key_name(p);
        if key.is_empty() || p.api_key.trim().is_empty() {
            continue;
        }
        env.insert(key, Value::String(p.api_key.clone()));
    }
    if !env.is_empty() {
        root.insert("env".into(), Value::Object(env));
    }
}

/// Entry list → [`BackendLoad`].
pub(crate) fn build_load(
    entries: Vec<(String, Value)>,
    env: Option<&Map<String, Value>>,
    protocols: Option<&Map<String, Value>>,
) -> BackendLoad {
    let mut providers: Vec<ProviderRow> = Vec::new();
    for (pid, entry) in &entries {
        let Some(row) = provider_from_entry(pid, protocols, entry, env) else {
            continue;
        };
        providers.push(row);
    }
    BackendLoad {
        root: Value::Object(Map::new()),
        agents: Vec::new(),
        providers,
        extras: Value::Object(Map::new()),
    }
}
