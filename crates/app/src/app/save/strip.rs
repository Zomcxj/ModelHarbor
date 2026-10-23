use crate::format::ConfigFormat;
use serde_json::{Map, Value};

/// Before cross-format conversion, evict from the target root the containers the current component state manages:
/// provider (opencode's provider / the pi family's and DSH's providers) and opencode's agent.
///
/// Only what the UI can manage is evicted: read-only content the UI neither shows nor can rebuild (Kimi's
/// `managed:*` OAuth providers and their models, Qwen's `qwen-oauth`) is kept, as are other top-level fields.
///
/// Deletion always uses `shift_remove`, preserving the relative order of remaining keys.
///
/// `agents_owned` says the UI genuinely holds agents data; when false, the target file's agent container stays.
pub fn strip_cross_format_containers(fmt: ConfigFormat, root: &mut Value, agents_owned: bool) {
    // workbuddy's root is an array: cleared whole.
    if fmt == ConfigFormat::WorkBuddy {
        *root = Value::Array(Vec::new());
        return;
    }
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    match fmt {
        // The opencode family (opencode / kilocode / mimocode) shares container names — one eviction set.
        ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
            obj.shift_remove("provider");
            if agents_owned {
                obj.shift_remove("agent");
            }
        }
        ConfigFormat::Pi | ConfigFormat::OhMyPi => {
            obj.shift_remove("providers");
        }
        ConfigFormat::DeepSeekHarness => {
            if let Some(llm) = obj.get_mut("llm-pi-ai").and_then(Value::as_object_mut) {
                llm.shift_remove("providers");
            }
        }
        ConfigFormat::ZCode => {
            if let Some(cfg) = obj.get_mut("config").and_then(Value::as_object_mut) {
                cfg.shift_remove("providerOrder");
                if let Some(rules) = cfg
                    .get_mut("providerConfigRules")
                    .and_then(Value::as_object_mut)
                {
                    rules.shift_remove("providerRules");
                }
                if let Some(rules) = cfg
                    .get_mut("modelConfigRules")
                    .and_then(Value::as_object_mut)
                {
                    rules.shift_remove("providerModelRules");
                }
            }
        }
        // Already returned above — unreachable; listed to keep the match exhaustive.
        ConfigFormat::WorkBuddy => {}
        // QwenCode: evict only the UI-managed part; `qwen-oauth` and unrecognized content stay.
        ConfigFormat::QwenCode => {
            if let Some(providers) = obj.get_mut("modelProviders").and_then(Value::as_object_mut) {
                let pids: Vec<String> = providers.keys().cloned().collect();
                for pid in pids {
                    if crate::backends::qwen_code::is_readonly_provider(&pid) {
                        continue;
                    }
                    // A non-array value (the old wrapper shape): left as-is.
                    let Some(items) = providers.get_mut(&pid).and_then(Value::as_array_mut) else {
                        continue;
                    };
                    items.retain(|e| !crate::backends::qwen_code::is_ui_managed_entry(e));
                    if items.is_empty() {
                        providers.shift_remove(&pid);
                    }
                }
            }
            if obj
                .get("modelProviders")
                .and_then(Value::as_object)
                .is_some_and(Map::is_empty)
            {
                obj.shift_remove("modelProviders");
            }
            // `providerProtocol` keeps only mappings of still-existing pids.
            let kept: Vec<String> = obj
                .get("modelProviders")
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            if let Some(protocols) = obj
                .get_mut("providerProtocol")
                .and_then(Value::as_object_mut)
            {
                let drop: Vec<String> = protocols
                    .keys()
                    .filter(|pid| !kept.iter().any(|k| k == *pid))
                    .cloned()
                    .collect();
                for pid in drop {
                    protocols.shift_remove(&pid);
                }
                if protocols.is_empty() {
                    obj.shift_remove("providerProtocol");
                }
            }
        }
        // KimiCode: likewise evicts only the UI-managed part; `managed:*` and its models stay.
        ConfigFormat::KimiCode => {
            // Model ownership is judged against the pre-eviction provider table.
            let owners: Vec<String> = obj
                .get("providers")
                .and_then(Value::as_object)
                .map(|provs| provs.keys().cloned().collect())
                .unwrap_or_default();
            if let Some(providers) = obj.get_mut("providers").and_then(Value::as_object_mut) {
                let drop: Vec<String> = providers
                    .keys()
                    .filter(|name| !crate::backends::kimi_code::is_managed_provider(name))
                    .cloned()
                    .collect();
                for name in drop {
                    providers.shift_remove(&name);
                }
            }
            if let Some(models) = obj.get_mut("models").and_then(Value::as_object_mut) {
                let drop: Vec<String> = models
                    .iter()
                    .filter(|(_, entry)| {
                        let owner = entry.get("provider").and_then(Value::as_str).unwrap_or("");
                        // Entries under `managed:*` stay.
                        if crate::backends::kimi_code::is_managed_provider(owner) {
                            return false;
                        }
                        // Orphans (no `provider` written, or pointing at a key absent from the table) stay too; only entries
                        // pointing at the provider being evicted this time are evicted.
                        !owner.is_empty() && owners.iter().any(|o| o == owner)
                    })
                    .map(|(alias, _)| alias.clone())
                    .collect();
                for alias in drop {
                    models.shift_remove(&alias);
                }
            }
            // An emptied table is removed whole.
            for key in ["providers", "models"] {
                if obj
                    .get(key)
                    .and_then(Value::as_object)
                    .is_some_and(Map::is_empty)
                {
                    obj.shift_remove(key);
                }
            }
        }
    }
}
