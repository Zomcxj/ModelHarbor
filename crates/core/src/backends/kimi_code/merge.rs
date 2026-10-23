use super::*;
use crate::model::ProviderRow;
use serde_json::{Map, Value};

/// One provider entry + the model entries hanging under it (aliases included).
pub(crate) struct Joined {
    pub(crate) name: String,
    pub(crate) provider: Value,
    pub(crate) models: Vec<(String, Value)>,
}

/// Hang the top-level `[models.*]` onto `[providers.*]` by the `provider` field.
///
/// Models with a missing `provider`, one pointing at a nonexistent key, or one pointing at `managed:*`
/// hang under no provider and so stay out of the UI; [`unmanaged_models`] writes them back verbatim.
pub(crate) fn join(root: &Value) -> Vec<Joined> {
    let mut joined: Vec<Joined> = Vec::new();
    let empty = Map::new();
    let providers = providers_map(root).unwrap_or(&empty);
    for (name, provider) in providers {
        joined.push(Joined {
            name: name.clone(),
            provider: provider.clone(),
            models: Vec::new(),
        });
    }
    for (alias, model) in models_map(root).unwrap_or(&empty) {
        let owner = model.get("provider").and_then(Value::as_str).unwrap_or("");
        if let Some(entry) = joined.iter_mut().find(|j| j.name == owner) {
            entry.models.push((alias.clone(), model.clone()));
        }
    }
    joined
}

/// Provider entry → [`ProviderRow`] (one provider per card, models hanging beneath).
pub(crate) fn provider_from_entry(
    name: &str,
    provider: &Value,
    models: &[(String, Value)],
) -> ProviderRow {
    let mut row = ProviderRow::new();
    row.key = name.to_string();
    row.base_url = provider
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.api_key = provider
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.api_key_env = provider
        .get("api_key_env")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.original_api_key_env = row.api_key_env.clone();
    // `type` goes into `pi_api` verbatim: it's both the dropdown's current value and the original at write-back.
    // An empty type is left for `effective_api()` to fall back on.
    row.pi_api = provider_type(provider).to_string();
    row.models = models
        .iter()
        .map(|(alias, m)| model_from_entry(alias, m))
        .collect();
    row.source_format = Some(ConfigFormat::KimiCode);
    row.raw = provider.clone();
    row
}

/// UI state → every model entry.
pub(crate) fn all_models(providers: &[ProviderRow], base: &Value) -> Vec<(String, String, Value)> {
    // Old entries index by alias: keys the UI doesn't manage inherit from here.
    let old: Map<String, Value> = models_map(base).cloned().unwrap_or_default();
    let mut out: Vec<(String, String, Value)> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let provider = p.key.trim().to_string();
        // A provider with no models writes no entry: Kimi's models must hang under a provider, but a provider
        // legitimately having none is legal — writing an empty model list would produce entries missing `model`.
        for m in &p.models {
            let wire = m.id.trim();
            if wire.is_empty() {
                // A model row without a wire id writes out as an invalid entry (`model` is required) — skipped at save.
                continue;
            }
            let alias = effective_alias(&provider, m);
            let entry = entry_from_model(m, &provider, old.get(&alias));
            out.push((provider.clone(), alias, entry));
        }
    }
    out
}

/// `[models.*]`: every entry.
///
/// Kimi has no "disable" concept — just this one table, with no "effective list / full copy" split.
pub(crate) fn model_table(providers: &[ProviderRow], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for (_, alias, entry) in all_models(providers, base) {
        out.insert(alias, entry);
    }
    out
}

/// Merge **unrecognized** model entries into one model table (verbatim, no `disabled` added).
pub(crate) fn merge_unmanaged_models(
    models: &mut Map<String, Value>,
    base: &Value,
    ui_providers: &[ProviderRow],
) {
    for (alias, model) in unmanaged_models(base, ui_providers) {
        models.entry(alias).or_insert(model);
    }
}

/// `[providers.*]`: UI providers + read-only providers (verbatim) + unrecognized shapes.
pub(crate) fn all_providers(providers: &[ProviderRow], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    let old = providers_map(base).cloned().unwrap_or_default();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let name = p.key.trim();
        // Read-only providers aren't the UI's to manage — preserved verbatim (see [`unmanaged_providers`]).
        if is_managed_provider(name) {
            continue;
        }
        out.insert(name.to_string(), provider_entry_from_row(p, old.get(name)));
    }
    for (name, value) in unmanaged_providers(base) {
        out.insert(name, value);
    }
    out
}

/// Base providers outside the UI's management: only `managed:*` (the OAuth login state), passed through verbatim.
///
/// Nothing else from the base is kept: cards the user deleted or renamed are never added back.
pub(crate) fn unmanaged_providers(base: &Value) -> Vec<(String, Value)> {
    providers_map(base)
        .map(|m| {
            m.iter()
                .filter(|(name, _)| is_managed_provider(name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The base's unrecognized model entries: orphans (`provider` pointing at a nonexistent key) and those under
/// `managed:*`.
/// Returns `alias → entry`; they stay out of the UI but write back verbatim.
pub(crate) fn unmanaged_models(base: &Value, ui_providers: &[ProviderRow]) -> Map<String, Value> {
    let providers = providers_map(base).cloned().unwrap_or_default();
    let mut out: Map<String, Value> = Map::new();
    for (alias, model) in models_map(base).unwrap_or(&Map::new()) {
        let owner = model.get("provider").and_then(Value::as_str).unwrap_or("");
        // UI-recognized: the provider exists, isn't managed, and that provider has a card.
        let known = !owner.is_empty()
            && !is_managed_provider(owner)
            && providers.contains_key(owner)
            && ui_providers.iter().any(|p| p.key.trim() == owner);
        if !known {
            out.insert(alias.clone(), model.clone());
        }
    }
    out
}

/// Judge each entry's check state by alias.
///
/// Kimi has no "disable" concept — a model's `disabled` is always `false`.
pub(crate) fn build_load(joined: Vec<Joined>) -> BackendLoad {
    let mut providers: Vec<ProviderRow> = Vec::new();
    for j in &joined {
        // `managed:*` stays out of the UI: OAuth maintains it, and its models were written automatically at login.
        if is_managed_provider(&j.name) {
            continue;
        }
        providers.push(provider_from_entry(&j.name, &j.provider, &j.models));
    }
    BackendLoad {
        root: Value::Object(Map::new()),
        agents: Vec::new(),
        providers,
        extras: Value::Object(Map::new()),
    }
}
