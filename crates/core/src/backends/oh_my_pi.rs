//! The oh-my-pi (omp) backend: `~/.omp/agent/models.yml` (YAML).
//!
//! Schema is pi's sibling (top-level `providers` + extras): reasoning tiers use the `thinking` block;
//! provider/model extension fields keep raw as their base at save, rewriting only UI-managed fields.
//! On load, both `thinking` and `thinkingLevelMap` parse into IR variants.

use super::{Backend, BackendLoad};
use crate::convert::{
    self, is_dsh_shaped_model, is_dsh_shaped_provider, is_opencode_shaped_model,
    is_opencode_shaped_provider,
};
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{
    home_dir_string, parse_config_content, parse_yaml_content, to_yaml_string, wsl_home,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub struct OhMyPiBackend;

pub static BACKEND: OhMyPiBackend = OhMyPiBackend;

fn default_local_path() -> String {
    format!("{}\\.omp\\agent\\models.yml", home_dir_string())
}

/// Model → the omp dialect object (keeping raw's unknown fields; tiers output as the thinking block).
pub fn model_to_omp(m: &ModelRow) -> Value {
    // opencode / DSH sources build fresh; pi/omp sources keep extension fields over raw as the base
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_model(&m.raw) || is_dsh_shaped_model(&m.raw) {
            Map::new()
        } else {
            m.raw.as_object().cloned().unwrap_or_default()
        };
    // The pi dialect's thinking keys unify into the thinking block
    obj.remove("thinkingLevelMap");

    obj.insert("id".into(), Value::String(m.id.clone()));
    if !m.name.trim().is_empty() {
        obj.insert("name".into(), Value::String(m.name.clone()));
    } else {
        obj.remove("name");
    }
    obj.insert("reasoning".into(), Value::Bool(m.reasoning));
    let input: Vec<Value> = m
        .modalities_input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| Value::String(s.to_string()))
        .collect();
    if input.is_empty() {
        obj.remove("input");
    } else {
        obj.insert("input".into(), Value::Array(input));
    }
    if let Ok(ctx) = m.context.parse::<i64>() {
        obj.insert("contextWindow".into(), Value::Number(ctx.into()));
    } else {
        obj.remove("contextWindow");
    }
    if let Ok(out) = m.output.parse::<i64>() {
        obj.insert("maxTokens".into(), Value::Number(out.into()));
    } else {
        obj.remove("maxTokens");
    }

    if m.variants.trim().is_empty() {
        obj.remove("thinking");
    } else if let Some(thinking) = omp_thinking(m) {
        obj.insert("thinking".into(), thinking);
    }
    Value::Object(obj)
}

/// Assemble omp's thinking block (called when variants are non-empty):
/// 1. omp native round trip: raw.thinking's tier value set matches the current one → the block is kept whole;
/// 2. pi-dialect translation: raw.thinkingLevelMap's value set matches → efforts = the key set, with effortMap attached when asymmetric;
/// 3. build a new symmetric block.
fn omp_thinking(m: &ModelRow) -> Option<Value> {
    let names: Vec<String> = crate::model::ordered_variants_text(&m.variants);
    let cur: HashSet<String> = names.iter().cloned().collect();
    let vals_of = |o: &Map<String, Value>| -> HashSet<String> {
        o.values()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect()
    };

    // 1) omp native
    if let Some(t) = m.raw.get("thinking").and_then(|v| v.as_object()) {
        let raw_vals = t
            .get("effortMap")
            .and_then(|v| v.as_object())
            .map(vals_of)
            .unwrap_or_else(|| {
                t.get("efforts")
                    .and_then(|v| v.as_array())
                    .map(|ef| {
                        ef.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect::<HashSet<String>>()
                    })
                    .unwrap_or_default()
            });
        if raw_vals == cur {
            return Some(Value::Object(crate::model::order_variant_map(t)));
        }
    }

    // 2) pi-dialect translation
    if let Some(tlm) = m.raw.get("thinkingLevelMap").and_then(|v| v.as_object()) {
        let tlm_vals = vals_of(tlm);
        if tlm_vals == cur {
            let keys: HashSet<String> = tlm.keys().cloned().collect();
            let symmetric = keys == tlm_vals;
            let ordered = crate::model::order_variant_map(tlm);
            let mut t = Map::new();
            t.insert("mode".into(), json!("effort"));
            t.insert(
                "efforts".into(),
                Value::Array(ordered.keys().map(|k| Value::String(k.clone())).collect()),
            );
            if !symmetric {
                t.insert("effortMap".into(), Value::Object(ordered));
            }
            return Some(Value::Object(t));
        }
    }

    // 3) new symmetric
    Some(json!({
        "mode": "effort",
        "efforts": names,
    }))
}

/// Provider → the omp dialect object (raw as base keeping headers/auth/discovery/modelOverrides etc.).
pub fn provider_to_omp(p: &ProviderRow) -> Value {
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_provider(&p.raw) || is_dsh_shaped_provider(&p.raw) {
            Map::new()
        } else {
            p.raw.as_object().cloned().unwrap_or_default()
        };

    let api = p.effective_api();

    if !p.base_url.is_empty() {
        // The same normalization as pi / dsh: a messages protocol's base carries no /v1.
        let save_url = convert::without_v1_for_messages(&api, &p.base_url);
        obj.insert("baseUrl".into(), Value::String(save_url));
    } else {
        obj.remove("baseUrl");
    }
    if !p.api_key.is_empty() {
        obj.insert("apiKey".into(), Value::String(p.api_key.clone()));
    } else {
        obj.remove("apiKey");
    }
    obj.insert("api".into(), Value::String(api));

    // compat manages only supportsDeveloperRole; other keys are kept
    if !p.compat {
        let mut c = obj
            .get("compat")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        c.insert("supportsDeveloperRole".into(), Value::Bool(false));
        obj.insert("compat".into(), Value::Object(c));
    } else if let Some(c) = obj.get_mut("compat").and_then(|v| v.as_object_mut()) {
        c.remove("supportsDeveloperRole");
        if c.is_empty() {
            obj.remove("compat");
        }
    }
    // requiresReasoningContentForAllAssistantTurns: an unmodified same-format save keeps raw as-is;
    // cross-format or user-modified writes the current value.
    let native_omp = matches!(
        p.source_format,
        Some(crate::format::ConfigFormat::Pi) | Some(crate::format::ConfigFormat::OhMyPi)
    );
    if p.requires_reasoning_content != p.original_requires_reasoning_content || !native_omp {
        let mut c = obj
            .get("compat")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        c.insert(
            "requiresReasoningContentForAllAssistantTurns".into(),
            Value::Bool(p.requires_reasoning_content),
        );
        obj.insert("compat".into(), Value::Object(c));
    }

    let models: Vec<Value> = p.models.iter().map(model_to_omp).collect();
    obj.insert("models".into(), Value::Array(models));
    Value::Object(convert::order_fields(
        obj,
        &["baseUrl", "apiKey", "api", "compat", "models"],
    ))
}

impl Backend for OhMyPiBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::OhMyPi
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.omp/agent/models.yml", wsl_home()?))
    }

    fn detect(&self, content: &str, path: &str) -> bool {
        let lower = path.to_lowercase();
        if lower.ends_with(".json") || lower.ends_with(".jsonc") {
            return false; // JSON files go to pi
        }
        let ext_yml = lower.ends_with(".yml") || lower.ends_with(".yaml");
        // Empty content (creation scenario): the .yml extension alone assigns omp
        if content.trim().is_empty() {
            return ext_yml;
        }
        // Without extension context, JSON-syntax content yields to pi (JSON is a YAML subset)
        if !ext_yml
            && parse_config_content(content)
                .map(|v| v.get("providers").and_then(|x| x.as_object()).is_some())
                .unwrap_or(false)
        {
            return false;
        }
        parse_yaml_content(content)
            .map(|v| {
                v.get("providers").and_then(|x| x.as_object()).is_some()
                    && v.get("provider").is_none()
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let v = parse_yaml_content(content)?;
        let providers = convert::load_pi_providers(&v);
        let extras = convert::load_pi_extras(&v);
        Ok(BackendLoad {
            root: v,
            agents: Vec::new(),
            providers,
            extras,
        })
    }

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // Cross-format target: extras take the target file's own top-level fields, rewriting only providers.
        // A same-named target provider merges conservatively; target-only providers are always kept.
        let base = match target_root {
            Some(target) => target,
            None => extras,
        };
        let mut root = base.as_object().cloned().unwrap_or_default();
        let mut providers_map = root
            .get("providers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for p in providers.iter().filter(|p| !p.key.is_empty()) {
            let value = provider_to_omp(p);
            let entry = match providers_map.get(&p.key) {
                Some(target) => convert::merge_conservative(target, &value),
                None => value,
            };
            providers_map.insert(p.key.clone(), entry);
        }
        root.insert("providers".into(), Value::Object(providers_map));
        Value::Object(root)
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        // Cross-format target saves need the target file's complete providers.
        super::load_target_root_with(path, parse_yaml_content, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/oh-my-pi_32.bin"),
            32,
            32,
        ))
    }

    fn render(&self, root: &Value, _compact: bool) -> Result<String, String> {
        to_yaml_string(root)
    }
}
