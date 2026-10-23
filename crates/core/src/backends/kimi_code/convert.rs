use super::*;
use crate::model::{ModelRow, ProviderRow};
use serde_json::{Map, Value};

/// A model entry's default table key: `<provider>/<model>`.
///
/// managed providers' table keys carry the `managed:` prefix, stripped here.
pub(crate) fn alias_key(provider: &str, model: &str) -> String {
    let short = provider.strip_prefix(MANAGED_PREFIX).unwrap_or(provider);
    format!("{short}/{model}")
}

/// The real table key when this model writes out: a UI-recorded alias wins; otherwise [`alias_key`] generates.
pub(crate) fn effective_alias(provider: &str, m: &ModelRow) -> String {
    let saved = m.kimi_alias.trim();
    if saved.is_empty() {
        alias_key(provider, m.id.trim())
    } else {
        saved.to_string()
    }
}

/// This tool's internal protocol names → Kimi's `type` vocabulary (6 values).
///
/// Maps only internal names that cross-format copies might bring in; unlisted returns `None`. Google's
/// three internal protocols (generative-ai / vertex / gemini-cli) all land on `google-genai`.
pub(crate) fn kimi_type_for_api(api: &str) -> Option<&'static str> {
    match api {
        "openai-completions" => Some("openai"),
        "openai-responses" => Some("openai_responses"),
        "anthropic-messages" => Some("anthropic"),
        "google-generative-ai" | "google-vertex" | "google-gemini-cli" => Some("google-genai"),
        _ => None,
    }
}

/// Every capability label this tool recognizes.
///
/// Used only to derive UI controls when **reading** (reasoning / tool use / input modalities); write-back
/// goes through [`merge_capabilities`], which never deletes by this list.
pub(crate) const CAP_THINKING: &str = "thinking";

pub(crate) const CAP_ALWAYS_THINKING: &str = "always_thinking";

pub(crate) const CAP_TOOL_USE: &str = "tool_use";

pub(crate) const CAP_IMAGE_IN: &str = "image_in";

pub(crate) const CAP_VIDEO_IN: &str = "video_in";

pub(crate) const CAP_AUDIO_IN: &str = "audio_in";

/// [`ProviderRow`] → the provider entry shape it would write (for the conflict health check only).
///
/// Restores the presence of the three credential keys **by Kimi's semantics** only.
pub(crate) fn provider_entry_view(p: &ProviderRow) -> Value {
    let mut obj = Map::new();
    if !p.api_key.trim().is_empty() {
        obj.insert("api_key".into(), Value::String(p.api_key.clone()));
    }
    if !p.api_key_env.trim().is_empty() {
        obj.insert("api_key_env".into(), Value::String(p.api_key_env.clone()));
    }
    // `oauth` isn't modeled by the UI (kept read-only).
    if p.raw.get("oauth").is_some() {
        obj.insert("oauth".into(), p.raw["oauth"].clone());
    }
    Value::Object(obj)
}

/// A model entry's capability label set.
pub(crate) fn capabilities_of(entry: &Value) -> Vec<String> {
    entry
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Capability set → the input-modality string (the UI's accounting).
///
/// **Undeclared returns an empty string** (never `"text"`), matching WorkBuddy / QwenCode.
pub(crate) fn modalities_from_caps(caps: &[String]) -> String {
    let has = |name: &str| caps.iter().any(|c| c.eq_ignore_ascii_case(name));
    if caps.is_empty() {
        return String::new();
    }
    let image = has(CAP_IMAGE_IN) || has(CAP_VIDEO_IN) || has(CAP_AUDIO_IN);
    crate::convert::supports_to_modalities([("text", true), ("image", image)])
}

/// The UI's modality string → capability labels to **add**.
///
/// Produces only "what to add", never "what to remove".
pub(crate) fn caps_for_modalities(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if text
        .split(',')
        .any(|s| s.trim().eq_ignore_ascii_case("image"))
    {
        out.push(CAP_IMAGE_IN);
    }
    out
}

/// Existing capabilities + UI state → the new capabilities (**grow-only**).
///
/// Labels append only when the UI **checks** them on; unchecking never deletes. The sole exception is
/// `thinking`: it and `always_thinking` are two spellings of the UI's single "supports reasoning"
/// switch, removed together on uncheck.
pub(crate) fn merge_capabilities(old: &[String], m: &ModelRow) -> Vec<String> {
    let mut out: Vec<String> = old.to_vec();
    let mut push = |name: &str| {
        if !out.iter().any(|c| c.eq_ignore_ascii_case(name)) {
            out.push(name.to_string());
        }
    };
    if m.reasoning {
        push(CAP_THINKING);
    }
    if m.tool_call {
        push(CAP_TOOL_USE);
    }
    for name in caps_for_modalities(&m.modalities_input) {
        push(name);
    }
    if !m.reasoning {
        out.retain(|c| {
            !c.eq_ignore_ascii_case(CAP_THINKING) && !c.eq_ignore_ascii_case(CAP_ALWAYS_THINKING)
        });
    }
    out
}

/// Model entry → [`ModelRow`]. `alias` is the table key; `model` is the wire id.
pub(crate) fn model_from_entry(alias: &str, entry: &Value) -> ModelRow {
    let mut m = ModelRow::new();
    let wire = entry
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    // The UI displays the wire id (what's sent upstream); the table key is stored separately.
    m.id = wire.clone();
    m.kimi_alias = alias.to_string();
    m.name = entry
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&wire)
        .to_string();
    // Fields the entry doesn't declare always stay empty — never the "new model" prefills.
    m.context = entry
        .get("max_context_size")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    m.output = entry
        .get("max_output_size")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    let caps = capabilities_of(entry);
    m.modalities_input = modalities_from_caps(&caps);
    m.reasoning = caps.iter().any(|c| {
        c.eq_ignore_ascii_case(CAP_THINKING) || c.eq_ignore_ascii_case(CAP_ALWAYS_THINKING)
    });
    m.tool_call = caps.iter().any(|c| c.eq_ignore_ascii_case(CAP_TOOL_USE));
    m.variants = entry
        .get("support_efforts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    m.original_variants = m.variants.clone();
    m.disabled = entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    m.source_format = Some(ConfigFormat::KimiCode);
    m.raw = entry.clone();
    m
}

/// The UI model → the model entry's **UI-managed** fields.
///
/// The **old entry** is the base, with UI-managed keys overwritten one by one; keys the UI doesn't manage
/// (`reasoning_key` / `beta_api` / `protocol` / `overrides` / model-level `base_url` etc.) stay verbatim.
///
/// Fields the UI clears **delete** their keys rather than writing empty values.
pub(crate) fn entry_from_model(m: &ModelRow, provider: &str, old: Option<&Value>) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    // `provider` writes before `model`. The table key (alias) isn't written into the entry.
    obj.insert("provider".into(), Value::String(provider.to_string()));
    obj.insert("model".into(), Value::String(m.id.trim().to_string()));

    // `display_name` writes **verbatim**: even when equal to the wire id; an empty UI name falls back to the wire id.
    let display = if m.name.trim().is_empty() {
        m.id.trim()
    } else {
        m.name.trim()
    };
    set_str(&mut obj, "display_name", display);

    // `max_context_size` is required and ≥1: an unparseable positive integer **omits** the key.
    set_num_min1(&mut obj, "max_context_size", m.context.trim());
    set_num_min1(&mut obj, "max_output_size", m.output.trim());

    let old_caps = capabilities_of(old.unwrap_or(&Value::Null));
    let caps = merge_capabilities(&old_caps, m);
    if caps.is_empty() {
        obj.shift_remove("capabilities");
    } else {
        obj.insert(
            "capabilities".into(),
            Value::Array(caps.into_iter().map(Value::String).collect()),
        );
    }

    // Tiers: `support_efforts` is an array; `default_effort` must be among them.
    let efforts = split_csv(&m.variants);
    if efforts.is_empty() {
        obj.shift_remove("support_efforts");
        obj.shift_remove("default_effort");
    } else {
        let existing_default = obj
            .get("default_effort")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        obj.insert(
            "support_efforts".into(),
            Value::Array(efforts.iter().cloned().map(Value::String).collect()),
        );
        // An existing default_effort still in the list stays; no longer there, the key is deleted.
        if efforts.iter().any(|e| e == &existing_default) {
            obj.insert("default_effort".into(), Value::String(existing_default));
        } else {
            obj.shift_remove("default_effort");
        }
    }
    Value::Object(obj)
}

/// The provider entry's **UI-managed** fields (the old entry is the base).
pub(crate) fn provider_entry_from_row(p: &ProviderRow, old: Option<&Value>) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    set_str(&mut obj, "base_url", p.base_url.trim());
    // `type` is required, limited to Kimi's own 6-value vocabulary
    // (anthropic / openai / kimi / google-genai / openai_responses / vertexai).
    //
    // Three sources: the UI value already in the vocabulary → written **verbatim**; the UI value is this
    // tool's **internal protocol name** → translated by [`kimi_type_for_api`]; neither → keep the old value,
    // falling back to `openai` only when there is none.
    let api = p.effective_api();
    let old_type = provider_type(old.unwrap_or(&Value::Null)).to_string();
    let ty = if crate::convert::KIMI_APIS.contains(&api.as_str()) {
        api
    } else if let Some(mapped) = kimi_type_for_api(&api) {
        mapped.to_string()
    } else if !old_type.is_empty() {
        old_type
    } else {
        "openai".to_string()
    };
    obj.insert("type".into(), Value::String(ty));
    // Credentials, one of three: the UI has only `api_key` / `api_key_env` boxes; `oauth` isn't modeled.
    //
    // Filling the key **clears** `api_key_env`, and vice versa; the old entry's `oauth` is untouched.
    let key = p.api_key.trim();
    let env = p.api_key_env.trim();
    if !key.is_empty() {
        obj.insert("api_key".into(), Value::String(key.to_string()));
        obj.shift_remove("api_key_env");
    } else if !env.is_empty() {
        obj.insert("api_key_env".into(), Value::String(env.to_string()));
        obj.shift_remove("api_key");
    } else {
        // Both empty: keep the original state.
        obj.shift_remove("api_key");
        obj.shift_remove("api_key_env");
    }
    Value::Object(obj)
}
