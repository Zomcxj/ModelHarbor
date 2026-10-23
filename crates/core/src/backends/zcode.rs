//! The ZCode backend: `~/.zcode/v2/provider_config.json`.
//!
//! Structure (custom-provider and model-level attributes live in two places):
//! ```jsonc
//! { "schemaVersion": 1, "config": {
//!     "providerOrder": ["<providerId>"],
//!     "providerConfigRules": { "providerRules": [ {
//!         "providerId", "providerName",
//!         "config": { "group",
//!           "access": { "type", "apiKey" },
//!           "api": { "type", "baseUrl" },
//!           "personalModelIds": [...], "modelOrder": [...] } } ]},
//!     "modelConfigRules": { "providerModelRules": [ {
//!         "modelId", "providerId",
//!         "config": { "enabled", "properties": {...}, "optionSpecs": {...} } } ],
//!       "manualProviderModelRules": [] } } }
//! ```
//!
//! Only this file is managed; built-in providers / credentials / the model library are not user-editable surface.
//!
//! Model-level fields land in `config.properties` (`contextWindow` / `supports*` booleans) and
//! `config.optionSpecs` (`maxOutputTokens.max` / `reasoningLevel.values`).
//! `optionSpecs.*.map` is preserved verbatim, never parsed.

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{home_dir_string, parse_config_content, wsl_home};
use serde_json::{Map, Value};

pub struct ZCodeBackend;

pub static BACKEND: ZCodeBackend = ZCodeBackend;

fn default_local_path() -> String {
    format!("{}\\.zcode\\v2\\provider_config.json", home_dir_string())
}

/// The `config` object (the content under the top-level `config` key).
fn config_of(root: &Value) -> Option<&Map<String, Value>> {
    root.get("config").and_then(Value::as_object)
}

/// The provider rules array.
fn provider_rules(root: &Value) -> Option<&Vec<Value>> {
    config_of(root)?
        .get("providerConfigRules")?
        .get("providerRules")?
        .as_array()
}

/// The model rules array (`modelConfigRules.providerModelRules`).
fn model_rules(root: &Value) -> Option<&Vec<Value>> {
    config_of(root)?
        .get("modelConfigRules")?
        .get("providerModelRules")?
        .as_array()
}

/// Take a model id list from provider rules (`personalModelIds`, falling back to `modelOrder` when missing).
fn model_ids_of(rule: &Value) -> Vec<String> {
    let cfg = rule.get("config");
    let pick = |key: &str| {
        cfg.and_then(|c| c.get(key))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let ids = pick("personalModelIds");
    if ids.is_empty() {
        pick("modelOrder")
    } else {
        ids
    }
}

/// Find a model's `config` in the model rules by (providerId, modelId).
fn model_config_of(root: &Value, provider_id: &str, model_id: &str) -> Option<Value> {
    model_rules(root)?
        .iter()
        .find(|rule| {
            rule.get("modelId").and_then(Value::as_str) == Some(model_id)
                && rule.get("providerId").and_then(Value::as_str) == Some(provider_id)
        })
        .and_then(|rule| rule.get("config"))
        .cloned()
}

/// Model-level raw → ModelRow.
///
/// `config` is that rule's `config` inside `providerModelRules` (may be missing). `raw` is preserved whole.
fn model_from_zcode(id: &str, config: Value) -> ModelRow {
    let mut row = ModelRow::new();
    row.id = id.to_string();
    row.name = id.to_string();
    row.source_format = Some(ConfigFormat::ZCode);

    let props = config.get("properties");
    row.context = props
        .and_then(|p| p.get("contextWindow"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    row.output = config
        .get("optionSpecs")
        .and_then(|s| s.get("maxOutputTokens"))
        .and_then(|m| m.get("max"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    row.modalities_input = convert::zcode_modalities_from_raw(&config);
    row.variants = convert::zcode_variants_from_raw(&config);
    row.original_variants = row.variants.clone();
    row.reasoning = !row.variants.is_empty();
    row.tool_call = props
        .and_then(|p| p.get("supportsToolCall"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    row.raw = config;
    row
}

/// Provider rule → ProviderRow.
fn provider_from_zcode(rule: &Value, root: &Value) -> Option<ProviderRow> {
    let id = rule.get("providerId").and_then(Value::as_str)?.to_string();
    let cfg = rule.get("config");
    let api = cfg.and_then(|c| c.get("api"));

    let models = model_ids_of(rule)
        .into_iter()
        .map(|model_id| {
            let config = model_config_of(root, &id, &model_id).unwrap_or(Value::Null);
            model_from_zcode(&model_id, config)
        })
        .collect();

    let mut row = ProviderRow::new();
    row.key = id;
    row.description = rule
        .get("providerName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // ZCode's api.type vocabulary differs from the pi family by one `chat`; convert into the internal representation.
    row.pi_api = convert::zcode_api_to_api(
        api.and_then(|a| a.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    // baseUrl is normalized before entering the UI: no `/v1`-style endpoint suffix.
    let raw_base = api
        .and_then(|a| a.get("baseUrl"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    row.base_url = convert::zcode_normalize_base_url(&row.pi_api, raw_base);
    row.api_key = cfg
        .and_then(|c| c.get("access"))
        .and_then(|a| a.get("apiKey"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    row.models = models;
    row.source_format = Some(ConfigFormat::ZCode);
    // raw uses the provider rule's `config`: at serialization it's the base preserving unknown keys like group.
    row.raw = cfg.cloned().unwrap_or(Value::Null);
    Some(row)
}

/// ProviderRow → the provider rule's `config`.
fn provider_config_to_zcode(p: &ProviderRow) -> Value {
    // raw is the base keeping ZCode's own fields (group etc.); raw from other dialects builds fresh.
    let mut obj = if convert::is_workbuddy_shaped(&p.raw)
        || convert::is_opencode_shaped_provider(&p.raw)
        || convert::is_dsh_shaped_provider(&p.raw)
    {
        Map::new()
    } else {
        p.raw.as_object().cloned().unwrap_or_default()
    };

    // `group` is required: write `standard-personal` when missing; keep an existing `group`.
    if !obj.contains_key("group") {
        obj.insert("group".into(), Value::String("standard-personal".into()));
    }

    let mut access = obj
        .get("access")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    access.insert("type".into(), Value::String("api-key".into()));
    if p.api_key.is_empty() {
        access.remove("apiKey");
    } else {
        access.insert("apiKey".into(), Value::String(p.api_key.clone()));
    }
    obj.insert("access".into(), Value::Object(access));

    let mut api = obj
        .get("api")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // The write side normalizes too: strip `/v1`-style endpoint suffixes.
    let effective = p.effective_api();
    api.insert(
        "type".into(),
        Value::String(convert::api_to_zcode_api(&effective)),
    );
    if !p.base_url.trim().is_empty() {
        let url = convert::zcode_normalize_base_url(&effective, &p.base_url);
        api.insert("baseUrl".into(), Value::String(url));
    }
    obj.insert("api".into(), Value::Object(api));

    let ids: Vec<Value> = p
        .models
        .iter()
        .filter(|m| !m.id.trim().is_empty())
        .map(|m| Value::String(m.id.clone()))
        .collect();
    obj.insert("personalModelIds".into(), Value::Array(ids.clone()));
    obj.insert("modelOrder".into(), Value::Array(ids));

    Value::Object(obj)
}

/// ModelRow → the model rule's `config`.
///
/// `enabled` is a direct child of `config` (sibling of `properties`). Capability booleans are rewritten only
/// when the original file already had the key or the UI supplied a modality list.
fn model_config_to_zcode(m: &ModelRow) -> Value {
    // raw is the base: expressions like `optionSpecs.*.map` and unknown keys are preserved verbatim;
    // raw from other dialects builds fresh.
    let mut obj = if convert::is_workbuddy_shaped(&m.raw)
        || convert::is_opencode_shaped_model(&m.raw)
        || convert::is_dsh_shaped_model(&m.raw)
    {
        Map::new()
    } else {
        m.raw.as_object().cloned().unwrap_or_default()
    };

    // enabled sits beside properties: the switch isn't managed — only backfill true when the original file lacked the key.
    if obj.get("enabled").and_then(Value::as_bool).is_none() {
        obj.insert("enabled".into(), Value::Bool(true));
    }

    let mut props = obj
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    // Write only values that parse as integers; unparseable ones stay as-is (props is a clone of raw).
    if let Ok(context) = m.context.trim().parse::<i64>() {
        props.insert("contextWindow".into(), Value::Number(context.into()));
    }
    // Modalities: when the UI supplies a list, write per it; an empty list leaves existing keys alone;
    // the landing spot is `properties.inputFormat`.
    if !m.modalities_input.trim().is_empty() {
        let mut input_format = props
            .get("inputFormat")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, on) in convert::modalities_to_supports(&m.modalities_input) {
            let field = match key {
                "text" => "supportsText",
                "image" => "supportsImage",
                "video" => "supportsVideo",
                "pdf" => "supportsPdf",
                "audio" => "supportsAudio",
                _ => continue,
            };
            // Sync only originally-declared keys or newly switched-on modalities (on); never backfill false for undeclared keys.
            if input_format.contains_key(field) || on {
                input_format.insert(field.into(), Value::Bool(on));
            }
        }
        if input_format.is_empty() {
            props.remove("inputFormat");
        } else {
            props.insert("inputFormat".into(), Value::Object(input_format));
        }
    }
    // Sync tool_call only when the original file already had the key.
    if props.contains_key("supportsToolCall") {
        props.insert("supportsToolCall".into(), Value::Bool(m.tool_call));
    }
    if !props.is_empty() {
        obj.insert("properties".into(), Value::Object(props));
    }

    let mut specs = obj
        .get("optionSpecs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Ok(output) = m.output.trim().parse::<i64>() {
        let mut max = specs
            .get("maxOutputTokens")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        max.insert("max".into(), Value::Number(output.into()));
        specs.insert("maxOutputTokens".into(), Value::Object(max));
    }
    // Tiers: when the UI has values, rewrite only `values`; the `map` expression stays verbatim.
    let variants: Vec<Value> = m
        .variants
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| Value::String(s.to_string()))
        .collect();
    if !variants.is_empty() {
        let mut level = specs
            .get("reasoningLevel")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        level.insert("values".into(), Value::Array(variants));
        specs.insert("reasoningLevel".into(), Value::Object(level));
    }
    if !specs.is_empty() {
        obj.insert("optionSpecs".into(), Value::Object(specs));
    }

    Value::Object(obj)
}

impl Backend for ZCodeBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::ZCode
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.zcode/v2/provider_config.json", wsl_home()?))
    }

    fn detect(&self, content: &str, _path: &str) -> bool {
        // Two independent markers: either providerRules or providerOrder being an array counts.
        parse_config_content(content)
            .map(|v| {
                let Some(cfg) = config_of(&v) else {
                    return false;
                };
                let rules = cfg
                    .get("providerConfigRules")
                    .and_then(|r| r.get("providerRules"))
                    .and_then(Value::as_array)
                    .is_some();
                let order = cfg.get("providerOrder").and_then(Value::as_array).is_some();
                rules || order
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_config_content(content)?;
        let providers = provider_rules(&root)
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|rule| provider_from_zcode(rule, &root))
                    .collect()
            })
            .unwrap_or_default();
        Ok(BackendLoad {
            root: root.clone(),
            agents: Vec::new(),
            providers,
            extras: root,
        })
    }

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        let mut root = target_root
            .unwrap_or(extras)
            .as_object()
            .cloned()
            .unwrap_or_default();
        if !root.contains_key("schemaVersion") {
            root.insert("schemaVersion".into(), Value::Number(1.into()));
        }

        let mut cfg = root
            .get("config")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        // providerOrder: per UI order (deletions included).
        let order: Vec<Value> = providers
            .iter()
            .filter(|p| !p.key.trim().is_empty())
            .map(|p| Value::String(p.key.clone()))
            .collect();
        cfg.insert("providerOrder".into(), Value::Array(order));

        // providerRules: rebuilt per UI order; same-key old rules serve as the base preserving unknown keys.
        let existing: Map<String, Value> = cfg
            .get("providerConfigRules")
            .and_then(|r| r.get("providerRules"))
            .and_then(Value::as_array)
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|r| {
                        let id = r.get("providerId").and_then(Value::as_str)?;
                        Some((id.to_string(), r.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rules: Vec<Value> = providers
            .iter()
            .filter(|p| !p.key.trim().is_empty())
            .map(|p| {
                let mut rule = existing
                    .get(&p.key)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                rule.insert("providerId".into(), Value::String(p.key.clone()));
                rule.insert(
                    "providerName".into(),
                    Value::String(if p.description.trim().is_empty() {
                        p.key.clone()
                    } else {
                        p.description.clone()
                    }),
                );
                rule.insert("config".into(), provider_config_to_zcode(p));
                Value::Object(rule)
            })
            .collect();
        let mut provider_cfg = Map::new();
        provider_cfg.insert("providerRules".into(), Value::Array(rules));
        let provider_cfg = convert::order_fields(provider_cfg, &["providerRules"]);
        cfg.insert("providerConfigRules".into(), Value::Object(provider_cfg));

        // modelConfigRules: rewritten per the UI's (providerId, modelId);
        // rules the UI doesn't manage and manualProviderModelRules are preserved whole.
        let managed: Vec<(String, String)> = providers
            .iter()
            .filter(|p| !p.key.trim().is_empty())
            .flat_map(|p| {
                p.models
                    .iter()
                    .filter(|m| !m.id.trim().is_empty())
                    .map(|m| (p.key.clone(), m.id.clone()))
            })
            .collect();
        let mut model_cfg = cfg
            .get("modelConfigRules")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let old_rules: Vec<Value> = model_cfg
            .get("providerModelRules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Entries of this provider in the old rules look up their base by model (keeping optionSpecs.map etc.).
        let lookup = |pid: &str, mid: &str| -> Option<&Value> {
            old_rules.iter().find(|r| {
                r.get("providerId").and_then(Value::as_str) == Some(pid)
                    && r.get("modelId").and_then(Value::as_str) == Some(mid)
            })
        };
        let mut new_rules: Vec<Value> = Vec::new();
        for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
            for m in p.models.iter().filter(|m| !m.id.trim().is_empty()) {
                let mut rule = lookup(&p.key, &m.id)
                    .and_then(|r| r.as_object().cloned())
                    .unwrap_or_default();
                rule.insert("modelId".into(), Value::String(m.id.clone()));
                rule.insert("providerId".into(), Value::String(p.key.clone()));
                rule.insert("config".into(), model_config_to_zcode(m));
                new_rules.push(Value::Object(rule));
            }
        }
        // Keep rules the UI doesn't manage: providers gone from the UI, or models deleted under a kept provider.
        for rule in &old_rules {
            let pid = rule.get("providerId").and_then(Value::as_str).unwrap_or("");
            let mid = rule.get("modelId").and_then(Value::as_str).unwrap_or("");
            let provider_gone = !providers.iter().any(|p| p.key == pid);
            let still_managed = managed.iter().any(|(a, b)| a == pid && b == mid);
            if provider_gone && !still_managed {
                new_rules.push(rule.clone());
            }
        }
        model_cfg.insert("providerModelRules".into(), Value::Array(new_rules));
        if !model_cfg.contains_key("manualProviderModelRules") {
            model_cfg.insert("manualProviderModelRules".into(), Value::Array(Vec::new()));
        }
        // Inside modelConfigRules the order is likewise fixed (providerModelRules first,
        // manualProviderModelRules after); a missing latter gets an empty array.
        let model_cfg = convert::order_fields(
            model_cfg,
            &["providerModelRules", "manualProviderModelRules"],
        );
        cfg.insert("modelConfigRules".into(), Value::Object(model_cfg));

        // Top-level and config key order is explicitly pinned: schemaVersion → config;
        // inside config: providerOrder → providerConfigRules → modelConfigRules.
        let cfg = convert::order_fields(
            cfg,
            &["providerOrder", "providerConfigRules", "modelConfigRules"],
        );
        root.insert("config".into(), Value::Object(cfg));
        let root = convert::order_fields(root, &["schemaVersion", "config"]);
        Value::Object(root)
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        super::load_target_root_with(path, parse_config_content, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((include_bytes!("../../assets/agents/zcode_32.bin"), 32, 32))
    }
}
