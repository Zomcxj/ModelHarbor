//! WorkBuddy backend: `~/.workbuddy/models.json`, a top-level array with provider info inlined into each
//! model; the protocol is expressed via `useCustomProtocol` + URL suffix. Config comes in two files: the
//! main file is the effective list WorkBuddy reads — only checked entries, first of each `id` kept; the
//! same-directory `models.full.json` is a full copy carrying per-entry `disabled` flags.

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{home_dir_string, parse_config_content, read_config_content, wsl_home};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub struct WorkBuddyBackend;

pub static BACKEND: WorkBuddyBackend = WorkBuddyBackend;

/// URL suffix appended when saving a non-Chat protocol.
const MESSAGES_SUFFIX: &str = "/v1/messages";
const RESPONSES_SUFFIX: &str = "/v1/responses";

/// The full copy's file name, same directory as `models.json`.
/// WorkBuddy reads `models.json` only by exact file name.
const FULL_STORE_NAME: &str = "models.full.json";

fn default_local_path() -> String {
    format!("{}\\.workbuddy\\models.json", home_dir_string())
}

/// Full-copy path: same directory as the main config, fixed file name, separator following the main config path's shape.
pub fn full_store_path(config_path: &str) -> String {
    crate::util::sibling_path(config_path, FULL_STORE_NAME)
}

/// Read the full copy's entries; `None` when missing, non-array, or unparseable.
fn load_full_store(config_path: &str) -> Option<Vec<Value>> {
    if config_path.trim().is_empty() {
        return None;
    }
    let text = read_config_content(&full_store_path(config_path)).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let root = parse_config_content(&text).ok()?;
    let items = entries_of(&root)?;
    if items.is_empty() {
        None
    } else {
        Some(items.clone())
    }
}

/// Merge the two configs into everything the UI should show.
///
/// The full copy leads (it carries each entry's own check state); entries present in the main config but
/// missing from the copy are appended, treated as checked.
fn merge_full_and_effective(full: &[Value], effective: &[Value]) -> Vec<Value> {
    let key_of = |v: &Value| {
        (
            v.get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            v.get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    };
    let known: HashSet<(String, String)> = full.iter().map(key_of).collect();
    let mut out = full.to_vec();
    for entry in effective {
        if known.contains(&key_of(entry)) {
            continue;
        }
        // Appended entries count as enabled; the `disabled` key is decided when `all_entries` writes.
        out.push(entry.clone());
    }
    out
}

/// Array root → entry list; also accepts the `{ "models": [...] }` shape.
fn entries_of(root: &Value) -> Option<&Vec<Value>> {
    if let Some(arr) = root.as_array() {
        return Some(arr);
    }
    root.get("models").and_then(Value::as_array)
}

/// Derive the protocol from the URL suffix: `/messages` → Anthropic Messages, `/responses` → Responses,
/// everything else Chat Completions.
fn api_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let trimmed = path.trim_end_matches('/');
    if trimmed.ends_with("/messages") {
        "anthropic-messages".to_string()
    } else if trimmed.ends_with("/responses") {
        "openai-responses".to_string()
    } else {
        "openai-completions".to_string()
    }
}

/// Protocol → suffix to append at save time; Chat Completions returns `None`.
fn suffix_for_api(api: &str) -> Option<&'static str> {
    match api.trim() {
        "anthropic-messages" => Some(MESSAGES_SUFFIX),
        "openai-responses" => Some(RESPONSES_SUFFIX),
        _ => None,
    }
}

/// Join base and suffix, idempotently: first strip protocol path segments from the base's tail
/// (`/messages`, `/responses`, `/chat/completions`, `/v1`), then append the canonical suffix.
fn with_suffix(url: &str, suffix: &str) -> String {
    let mut base = url.trim().trim_end_matches('/');
    loop {
        let stripped = base
            .strip_suffix("/messages")
            .or_else(|| base.strip_suffix("/responses"))
            .or_else(|| base.strip_suffix("/chat/completions"))
            .or_else(|| base.strip_suffix("/v1"))
            .map(|s| s.trim_end_matches('/'));
        match stripped {
            Some(s) if s != base => base = s,
            _ => break,
        }
    }
    format!("{}{}", base, suffix)
}

/// Entry → ProviderRow; one entry per card.
fn provider_from_entry(v: &Value) -> Option<ProviderRow> {
    let id = v.get("id").and_then(Value::as_str)?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let url = v.get("url").and_then(Value::as_str).unwrap_or_default();
    let custom = v
        .get("useCustomProtocol")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // `id` = model name (sent to the API), `name` = provider label:
    // the model row's id ← entry `id`; the provider's key ← entry `name`.
    let provider = v
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let mut model = ModelRow::new();
    model.id = id.clone();
    model.name = id.clone();
    model.context = v
        .get("maxInputTokens")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    model.output = v
        .get("maxOutputTokens")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    model.modalities_input = convert::workbuddy_modalities_from_raw(v);
    model.tool_call = v
        .get("supportsToolCall")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    model.reasoning = v
        .get("supportsReasoning")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // `disabled: true`: grayed out and unselectable in the selector, but still in the list.
    model.disabled = v.get("disabled").and_then(Value::as_bool).unwrap_or(false);
    // WorkBuddy has no "reasoning tiers" concept; clear the default tiers ModelRow::new() brings.
    model.variants.clear();
    model.original_variants.clear();
    model.source_format = Some(ConfigFormat::WorkBuddy);
    model.raw = v.clone();

    let mut row = ProviderRow::new();
    row.key = provider;
    row.description = v
        .get("vendor")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // With the custom protocol checked, the protocol follows the URL suffix; otherwise it's Chat Completions.
    row.pi_api = if custom {
        api_from_url(url)
    } else {
        "openai-completions".to_string()
    };
    row.base_url = url.to_string();
    row.api_key = v
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    row.models = vec![model];
    row.source_format = Some(ConfigFormat::WorkBuddy);
    row.raw = v.clone();
    Some(row)
}

/// ProviderRow + single model → the entry's UI-managed fields.
///
/// Returns only the keys this function manages; the entry's other keys (tags / credits / reasoning etc.)
/// are inherited by [`WorkBuddyBackend::serialize_root`] from the old entry with the same name and model.
/// `id` is the model name sent to the API, `name` is the provider label; the two are never concatenated.
fn entry_from_provider(p: &ProviderRow, model: Option<&ModelRow>) -> Value {
    let mut obj = Map::new();

    // `id` = model name (the model row's id), `name` = provider (`ProviderRow.key`).
    let model_id = model
        .map(|m| m.id.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(&p.key)
        .to_string();
    obj.insert("id".into(), Value::String(model_id));
    obj.insert("name".into(), Value::String(p.key.clone()));
    if !p.description.trim().is_empty() {
        obj.insert("vendor".into(), Value::String(p.description.clone()));
    }
    if p.api_key.trim().is_empty() {
        obj.remove("apiKey");
    } else {
        obj.insert("apiKey".into(), Value::String(p.api_key.clone()));
    }

    let api = p.effective_api();
    match suffix_for_api(&api) {
        // Non-Chat protocol: append the URL suffix and set `useCustomProtocol = true`.
        Some(suffix) => {
            obj.insert(
                "url".into(),
                Value::String(with_suffix(&p.base_url, suffix)),
            );
            obj.insert("useCustomProtocol".into(), Value::Bool(true));
        }
        // Chat Completions: the URL stores the base; WorkBuddy appends the suffix.
        None => {
            obj.insert("url".into(), Value::String(p.base_url.clone()));
            obj.insert("useCustomProtocol".into(), Value::Bool(false));
        }
    }

    if let Some(m) = model {
        // Write only values that parse as integers; unparseable ones skip the key.
        if let Ok(context) = m.context.trim().parse::<i64>() {
            obj.insert("maxInputTokens".into(), Value::Number(context.into()));
        }
        if let Ok(output) = m.output.trim().parse::<i64>() {
            obj.insert("maxOutputTokens".into(), Value::Number(output.into()));
        }
        // Modalities / capabilities: synced only when the original entry already had the key; no new keys.
        let mods: Vec<String> = m
            .modalities_input
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if obj.contains_key("supportsImages") && !mods.is_empty() {
            obj.insert(
                "supportsImages".into(),
                Value::Bool(mods.iter().any(|s| s == "image")),
            );
        }
        if obj.contains_key("supportsToolCall") {
            obj.insert("supportsToolCall".into(), Value::Bool(m.tool_call));
        }
        if obj.contains_key("supportsReasoning") {
            obj.insert("supportsReasoning".into(), Value::Bool(m.reasoning));
        }
        // Enable/disable isn't written here; check states are recorded per entry by the full copy.
    }

    Value::Object(obj)
}

/// Index old entries by the (name, id) pair; different providers may have same-id models.
fn existing_index(base: &Value) -> HashMap<(String, String), Value> {
    entries_of(base)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    let id = v.get("id").and_then(Value::as_str)?;
                    let name = v.get("name").and_then(Value::as_str).unwrap_or_default();
                    Some(((name.to_string(), id.to_string()), v.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One entry per model of a provider — no filtering, no dedup.
///
/// Construction shared by both configs: the full copy takes all entries; the effective list filters from
/// the result. Check state is written explicitly on every entry: checked `disabled: false`, unchecked `true`.
fn all_entries(providers: &[ProviderRow], base: &Value) -> Vec<Value> {
    let existing = existing_index(base);
    let mut out: Vec<Value> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        // A provider with no models still keeps one entry, id falling back to the provider key.
        let models: Vec<Option<&ModelRow>> = if p.models.is_empty() {
            vec![None]
        } else {
            p.models.iter().map(Some).collect()
        };
        for model in models {
            let model_id = model
                .map(|m| m.id.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(p.key.trim())
                .to_string();
            let mut entry = existing
                .get(&(p.key.trim().to_string(), model_id))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(fresh) = entry_from_provider(p, model).as_object() {
                for (k, v) in fresh {
                    entry.insert(k.clone(), v.clone());
                }
            }
            // Checked = `disabled: false`, unchecked = `true`.
            entry.insert(
                "disabled".into(),
                Value::Bool(model.map(|m| m.disabled).unwrap_or(false)),
            );
            out.push(Value::Object(entry));
        }
    }
    out
}

/// Effective list: take checked entries, keeping only the first per id.
///
/// WorkBuddy's selector dedupes globally by bare id — from the second on it has no effect; filtered-out
/// entries remain in the full copy. Each kept entry writes `disabled: false` explicitly.
fn effective_entries(all: &[Value]) -> Vec<Value> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for entry in all {
        if entry.get("disabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !seen.insert(id) {
            continue;
        }
        let mut entry = entry.clone();
        // Effective entries are always enabled, written `false` explicitly.
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("disabled".into(), Value::Bool(false));
        }
        out.push(entry);
    }
    out
}

/// Entry list → [`BackendLoad`], shared by both configs.
///
/// Entries with the same `name` merge into one provider card; each entry becomes a model row.
///
/// With `trusted_flags` true (reading the full copy), each entry's own `disabled` key is taken at face
/// value, entries lacking the key still derive by position (only the first of a same id enabled); with
/// false, everything derives by position. A final id-based dedup pass backstops: at most one enabled per
/// id, explicit flags winning over positional derivation.
fn build_load(entries: Vec<Value>, trusted_flags: bool) -> BackendLoad {
    // Pass one: settle each entry's check state in file order, recording whether it came from an explicit flag or positional derivation.
    let mut seen: HashSet<String> = HashSet::new();
    // (check state, whether from an explicit flag)
    let mut flags: Vec<Option<(bool, bool)>> = Vec::with_capacity(entries.len());
    for entry in &entries {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let Some(id) = id else {
            // Entries without an id can't reach the UI; a placeholder `None` keeps flags aligned with entries.
            flags.push(None);
            continue;
        };
        let by_position = !seen.insert(id);
        // Whether `disabled` was explicitly written reads from the entry itself, keeping flags aligned.
        let explicit = trusted_flags && entry.get("disabled").and_then(Value::as_bool).is_some();
        let disabled = match (explicit, entry.get("disabled").and_then(Value::as_bool)) {
            (true, Some(flag)) => flag,
            _ => by_position,
        };
        flags.push(Some((disabled, explicit)));
    }

    // Pass two: when several entries of one id read as enabled, keep one; explicit flags win, same source takes the earliest.
    let mut winner: HashMap<String, usize> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some((false, explicit)) = flags[i] else {
            continue;
        };
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        match winner.get(&id) {
            // An existing winner: re-judged only when this entry is explicit and the previous wasn't.
            Some(&prev) if explicit && !flags[prev].is_some_and(|(_, e)| e) => {
                winner.insert(id, i);
            }
            Some(_) => {}
            None => {
                winner.insert(id, i);
            }
        }
    }
    // Losing enabled entries are all switched off.
    for (i, entry) in entries.iter().enumerate() {
        if flags[i].map(|(disabled, _)| disabled) != Some(false) {
            continue;
        }
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if winner.get(&id) != Some(&i) {
            flags[i] = Some((true, flags[i].is_some_and(|(_, e)| e)));
        }
    }

    let mut providers: Vec<ProviderRow> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(mut row) = provider_from_entry(entry) else {
            continue;
        };
        let Some(model) = row.models.first_mut() else {
            continue;
        };
        if let Some((disabled, _)) = flags[i] {
            model.disabled = disabled;
        }
        match providers.iter_mut().find(|p| p.key == row.key) {
            // Same provider: the first entry wins; later ones contribute only model rows.
            Some(existing) => existing.models.push(row.models.remove(0)),
            None => providers.push(row),
        }
    }
    // extras uses the original entry array: at save time, inheriting unknown fields by (name, id) looks it up here.
    let extras = Value::Array(entries);
    BackendLoad {
        root: extras.clone(),
        agents: Vec::new(),
        providers,
        extras,
    }
}

/// JSON rendering of an array root: two-space indent; `compact` drops the indent.
fn render_array(items: &[Value], compact: bool) -> String {
    if compact {
        let inner: Vec<String> = items
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".into()))
            .collect();
        return format!("[{}]", inner.join(","));
    }
    let body =
        serde_json::to_string_pretty(&Value::Array(items.to_vec())).unwrap_or_else(|_| "[]".into());
    format!("{body}\n")
}

impl Backend for WorkBuddyBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::WorkBuddy
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.workbuddy/models.json", wsl_home()?))
    }

    fn detect(&self, content: &str, _path: &str) -> bool {
        // A non-empty array whose first element carries `id`; `[]` isn't recognized.
        parse_config_content(content)
            .map(|v| {
                entries_of(&v)
                    .and_then(|items| items.first())
                    .and_then(|first| first.get("id"))
                    .is_some()
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_config_content(content)?;
        let entries = entries_of(&root).cloned().unwrap_or_default();
        Ok(build_load(entries, false))
    }

    /// Parsing with a path: the full copy (`models.full.json`) is read first — it's the source of everything the
    /// UI shows.
    ///
    /// Without the copy, fall back to `models.json` with check states derived by position. With the copy
    /// present, the main config is still read and unioned in, picking up entries hand-added to it.
    fn parse_at(&self, content: &str, path: &str) -> Result<BackendLoad, String> {
        if let Some(full) = load_full_store(path) {
            let effective = parse_config_content(content)
                .ok()
                .and_then(|root| entries_of(&root).cloned())
                .unwrap_or_default();
            return Ok(build_load(
                merge_full_and_effective(&full, &effective),
                true,
            ));
        }
        self.parse(content)
    }

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // The product is WorkBuddy's effective list: only checked entries, the first per `id` kept.
        // The full copy is written separately by `save_sidecars`; both share the `all_entries` construction.
        // `id` is both the selector's dedup key and the model name sent upstream — no numbering.
        Value::Array(effective_entries(&all_entries(
            providers,
            target_root.unwrap_or(extras),
        )))
    }

    /// The full copy (`models.full.json`): every entry + its own check state — the on-disk form of everything
    /// the UI configures. Unchecking merely moves an entry out of the effective list into the copy, with fields
    /// (API key included) preserved.
    fn save_sidecars(&self, path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        // The base for inheriting unknown fields takes the full copy, falling back to the main config when unreadable.
        // Entries in the base don't revive through inheritance: entries are always generated from `providers`.
        // With neither readable, the save is cancelled — never a silent empty base.
        let base = match load_full_store(path).map(Value::Array) {
            Some(base) => base,
            None => self
                .load_target_root(path)
                .map_err(|e| format!("无法读取全量副本与主配置，已取消保存: {e}"))?,
        };
        let all = all_entries(providers, &base);
        super::write_config(&full_store_path(path), &render_array(&all, false))
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        // The top level is an array (provider inlined into each model); even an empty root is an array.
        super::load_target_root_with(path, parse_config_content, || Value::Array(Vec::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/workbuddy_32.bin"),
            32,
            32,
        ))
    }

    fn render(&self, root: &Value, compact: bool) -> Result<String, String> {
        // Array roots must render themselves; `app::pretty_json` assumes object roots.
        let items = entries_of(root).cloned().unwrap_or_default();
        Ok(render_array(&items, compact))
    }
}
