use crate::model::{ModelRow, ProviderRow};
use crate::util::{bool_at, nested_list_str, num_at, str_at};
use serde_json::{Map, Value};
use std::collections::HashSet;

/// Whether raw is the opencode dialect (carries opencode fingerprint keys).
pub(crate) fn is_opencode_shaped_model(raw: &Value) -> bool {
    ["limit", "modalities", "options", "variants"]
        .iter()
        .any(|k| raw.get(k).is_some())
}

pub(crate) fn is_opencode_shaped_provider(raw: &Value) -> bool {
    raw.get("options").is_some() || raw.get("models").map(|m| m.is_object()).unwrap_or(false)
}

/// Whether raw is the pi / omp dialect (carries pi fingerprint keys).
pub(crate) fn is_dsh_shaped_model(raw: &Value) -> bool {
    raw.get("reasoningEfforts").is_some()
}

pub(crate) fn is_dsh_shaped_provider(raw: &Value) -> bool {
    raw.get("apiKeyEnv").is_some() || raw.get("baseURL").is_some()
}

/// Whether raw is the WorkBuddy dialect (flat entries with `useCustomProtocol` or `url`).
pub(crate) fn is_workbuddy_shaped(raw: &Value) -> bool {
    raw.get("useCustomProtocol").is_some() || raw.get("url").is_some()
}

/// Normalize an `anthropic-messages` protocol base URL: ensure a trailing `/v1` (shared by opencode read and write).
///
/// Non-`anthropic-messages` or empty strings pass through.
pub fn with_v1_for_messages(api: &str, url: &str) -> String {
    if api != "anthropic-messages" || url.trim().is_empty() {
        return url.to_string();
    }
    let trimmed = url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{}/v1", trimmed)
    }
}

/// Normalize an `anthropic-messages` protocol base URL: strip the trailing `/v1` (shared by read and write).
///
/// Non-`anthropic-messages`, or URLs without a trailing `/v1`, pass through.
pub fn without_v1_for_messages(api: &str, url: &str) -> String {
    if api != "anthropic-messages" {
        return url.to_string();
    }
    let trimmed = url.trim_end_matches('/');
    match trimmed.strip_suffix("/v1") {
        Some(rest) => rest.trim_end_matches('/').to_string(),
        None => url.to_string(),
    }
}

/// Normalize ZCode's `baseUrl`: strip the endpoint path suffixes ZCode itself would append (shared by read and write).
///
/// The suffix table keys on api: `anthropic-messages` strips `/v1/messages`, `/messages`, `/v1`;
/// `openai-responses` strips `/responses`; the rest strip `/chat/completions`. Strips in a loop until no match.
pub fn zcode_normalize_base_url(api: &str, url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Pick the suffixes to strip by api, longest first.
    let suffixes: &[&str] = match api {
        "anthropic-messages" => &["/v1/messages", "/messages", "/v1"],
        "openai-responses" => &["/responses"],
        _ => &["/chat/completions"],
    };
    let mut base = trimmed.trim_end_matches('/');
    loop {
        let mut stripped = false;
        for suffix in suffixes {
            if let Some(rest) = base.strip_suffix(suffix) {
                base = rest.trim_end_matches('/');
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    base.to_string()
}

/// ZCode's `api.type` → the internally unified api (pi / omp / DSH vocabulary).
///
/// `openai-chat-completions` maps to `openai-completions`; everything else passes through.
pub fn zcode_api_to_api(zcode_api: &str) -> String {
    match zcode_api.trim() {
        "openai-chat-completions" => "openai-completions".to_string(),
        other => other.to_string(),
    }
}

/// The internally unified api (pi / omp / DSH vocabulary) → ZCode's `api.type`.
///
/// Only the three values are recognized; unknown values and `openai-completions` both map to `openai-chat-completions`.
pub fn api_to_zcode_api(api: &str) -> String {
    match api.trim() {
        "openai-completions" | "openai-chat-completions" => "openai-chat-completions".to_string(),
        "anthropic-messages" => "anthropic-messages".to_string(),
        "openai-responses" => "openai-responses".to_string(),
        _ => "openai-chat-completions".to_string(),
    }
}

/// Input modality list (`text, image` form) → a set of capability booleans.
///
/// Returns 5 items in fixed order: `text` / `image` / `video` / `pdf` / `audio`; matching is case-insensitive.
pub fn modalities_to_supports(list: &str) -> Vec<(&'static str, bool)> {
    let items: HashSet<String> = list
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    vec![
        ("text", items.contains("text")),
        ("image", items.contains("image")),
        ("video", items.contains("video")),
        ("pdf", items.contains("pdf")),
        ("audio", items.contains("audio")),
    ]
}

/// A set of capability booleans → input modality list (`text, image` form, fixed order).
pub fn supports_to_modalities<'a>(pairs: impl IntoIterator<Item = (&'a str, bool)>) -> String {
    const ORDER: [&str; 5] = ["text", "image", "video", "pdf", "audio"];
    let on: HashSet<String> = pairs
        .into_iter()
        .filter(|(_, v)| *v)
        .map(|(k, _)| k.to_ascii_lowercase())
        .collect();
    ORDER
        .iter()
        .filter(|k| on.contains(**k))
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// ZCode's model raw → input modality list.
///
/// Modality booleans come from `properties.inputFormat.supportsText/Image/Video/Pdf/Audio`; when none of
/// the `supports*` keys are written, an empty string returns.
pub fn zcode_modalities_from_raw(raw: &Value) -> String {
    let props = raw.get("properties").and_then(|p| p.get("inputFormat"));
    let flag = |key: &str| {
        props
            .and_then(|p| p.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    // No supports* keys written at all returns an empty string.
    let any = [
        "supportsText",
        "supportsImage",
        "supportsVideo",
        "supportsPdf",
        "supportsAudio",
    ]
    .iter()
    .any(|k| props.and_then(|p| p.get(k)).is_some());
    if !any {
        return String::new();
    }
    supports_to_modalities([
        ("text", flag("supportsText")),
        ("image", flag("supportsImage")),
        ("video", flag("supportsVideo")),
        ("pdf", flag("supportsPdf")),
        ("audio", flag("supportsAudio")),
    ])
}

/// WorkBuddy's model raw → input modality list (derived from `supportsImages`).
///
/// No `supportsImages` returns an empty string (undeclared); written, it returns `text` plus an optional `image`.
pub fn workbuddy_modalities_from_raw(raw: &Value) -> String {
    match raw.get("supportsImages").and_then(Value::as_bool) {
        // supportsImages declared: text is always true.
        Some(images) => supports_to_modalities([("text", true), ("image", images)]),
        None => String::new(),
    }
}

/// ZCode's model raw → reasoning tier text (comma-separated).
///
/// Tiers live in `optionSpecs.reasoningLevel.values`; left empty when the key is absent.
pub fn zcode_variants_from_raw(raw: &Value) -> String {
    raw.get("optionSpecs")
        .and_then(|s| s.get("reasoningLevel"))
        .and_then(|r| r.get("values"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// opencode's npm package → pi / omp api (the wire protocol).
///
/// `@ai-sdk/anthropic` → `anthropic-messages`, `@ai-sdk/google` → `google-generative-ai`,
/// `@ai-sdk/mistral` → `mistral-conversations`, `@ai-sdk/openai` → `openai-responses`;
/// the rest (including empty) → `openai-completions`.
pub fn npm_to_api(npm: &str) -> String {
    match npm {
        "@ai-sdk/anthropic" => "anthropic-messages".to_string(),
        "@ai-sdk/google" => "google-generative-ai".to_string(),
        "@ai-sdk/mistral" => "mistral-conversations".to_string(),
        "@ai-sdk/openai" => "openai-responses".to_string(),
        _ => "openai-completions".to_string(),
    }
}

/// v2's `@opencode/ai/providers/<suffix>` package → api (the wire protocol).
///
/// Suffix mapping: anthropic → anthropic-messages, google → google-generative-ai,
/// openai → openai-responses, mistral → mistral-conversations;
/// openai-compatible and unknown suffixes → openai-completions.
/// Package names without the v2 prefix go through [`npm_to_api`].
pub fn package_or_npm_to_api(pkg: &str) -> String {
    if let Some(suffix) = pkg.strip_prefix("@opencode/ai/providers/") {
        let last = suffix.rsplit('/').next().unwrap_or(suffix);
        return match last {
            "anthropic" => "anthropic-messages".to_string(),
            "google" => "google-generative-ai".to_string(),
            "openai" => "openai-responses".to_string(),
            "mistral" => "mistral-conversations".to_string(),
            _ => "openai-completions".to_string(),
        };
    }
    npm_to_api(pkg)
}

/// omp's official 9 values (`google-gemini-cli` is omp-only).
pub const OMP_APIS: [&str; 9] = [
    "openai-completions",
    "openai-responses",
    "openai-codex-responses",
    "azure-openai-responses",
    "anthropic-messages",
    "bedrock-converse-stream",
    "google-generative-ai",
    "google-gemini-cli",
    "google-vertex",
];

/// pi KnownApi's 10 values (`mistral-conversations` / `pi-messages` are pi-only).
pub const PI_APIS: [&str; 10] = [
    "openai-completions",
    "mistral-conversations",
    "openai-responses",
    "azure-openai-responses",
    "openai-codex-responses",
    "anthropic-messages",
    "bedrock-converse-stream",
    "google-generative-ai",
    "google-vertex",
    "pi-messages",
];

/// ZCode's official 3 values (`api.type`; Chat Completions also accepts `chat`).
pub const ZCODE_APIS: [&str; 3] = [
    "openai-chat-completions",
    "anthropic-messages",
    "openai-responses",
];

/// WorkBuddy page's selectable protocols: chosen in the UI, persisted as the URL suffix at save.
pub const WORKBUDDY_APIS: [&str; 3] = [
    "openai-completions",
    "anthropic-messages",
    "openai-responses",
];

/// QwenCode page's selectable protocols: saved as pid + `wireApi`.
pub const QWEN_APIS: [&str; 3] = [
    "openai-completions",
    "anthropic-messages",
    "openai-responses",
];

/// KimiCode page's selectable protocols: the 6 legal values of `[providers.<name>].type`.
///
/// Stored verbatim into `ProviderRow::pi_api` — no mapping translation.
pub const KIMI_APIS: [&str; 6] = [
    "openai",
    "kimi",
    "anthropic",
    "openai_responses",
    "google-genai",
    "vertexai",
];

/// pi / omp api → opencode's npm package.
///
/// `anthropic-messages` → `@ai-sdk/anthropic`, `google-generative-ai` → `@ai-sdk/google`,
/// `mistral-conversations` → `@ai-sdk/mistral`,
/// `openai-completions` → `@ai-sdk/openai-compatible`, `openai-responses` → `@ai-sdk/openai`;
/// other apis return an empty string.
pub fn api_to_npm(api: &str) -> String {
    match api {
        "anthropic-messages" => "@ai-sdk/anthropic".to_string(),
        "google-generative-ai" => "@ai-sdk/google".to_string(),
        "mistral-conversations" => "@ai-sdk/mistral".to_string(),
        "openai-completions" => "@ai-sdk/openai-compatible".to_string(),
        "openai-responses" => "@ai-sdk/openai".to_string(),
        _ => String::new(),
    }
}

/// Extract a reasoning tier's set of "sent values" (comma-separated):
/// - omp dialect: the values of thinking.effortMap (efforts when there is no effortMap)
/// - pi dialect: the values of thinkingLevelMap
fn thinking_values(v: &Value) -> String {
    if let Some(t) = v.get("thinking") {
        if let Some(em) = t.get("effortMap").and_then(|m| m.as_object()) {
            return em
                .values()
                .filter_map(|val| val.as_str())
                .collect::<Vec<_>>()
                .join(", ");
        }
        if let Some(ef) = t.get("efforts").and_then(|a| a.as_array()) {
            return ef
                .iter()
                .filter_map(|val| val.as_str())
                .collect::<Vec<_>>()
                .join(", ");
        }
        // A thinking block without efforts/effortMap falls back to thinkingLevelMap.
    }
    v.get("thinkingLevelMap")
        .and_then(|m| m.as_object())
        .map(|obj| {
            obj.values()
                .map(|val| val.as_str().unwrap_or(""))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

pub fn model_from_pi(v: &Value) -> ModelRow {
    let id = str_at(v, "id").to_string();
    let modalities_input = nested_list_str(v, &["input"]);
    // Besides `reasoning`, `thinking` / `thinkingLevelMap` / `reasoningEfforts` also express reasoning support.
    let reasoning = bool_at(v, "reasoning")
        || v.get("thinkingLevelMap").is_some()
        || v.get("thinking").is_some()
        || v.get("reasoningEfforts").is_some();
    ModelRow {
        id: id.clone(),
        name: str_at(v, "name").to_string(),
        reasoning,
        tool_call: true,
        store: false,
        // `disabled` is WorkBuddy-exclusive; other dialects' models are always enabled.
        disabled: false,
        context: num_at(v, "contextWindow"),
        output: num_at(v, "maxTokens"),
        modalities_input,
        modalities_output: "text".to_string(),
        variants: thinking_values(v),
        original_variants: thinking_values(v),
        source_format: Some(if is_dsh_shaped_model(v) {
            crate::format::ConfigFormat::DeepSeekHarness
        } else {
            crate::format::ConfigFormat::Pi
        }),
        schema: crate::model::OpencodeSchemaVersion::V1,
        raw: v.clone(),
        kimi_alias: String::new(),
    }
}

pub fn model_to_pi(m: &ModelRow) -> Value {
    // opencode / DSH sources build fresh; the rest keep extension fields over raw as the base.
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_model(&m.raw) || is_dsh_shaped_model(&m.raw) {
            Map::new()
        } else {
            m.raw.as_object().cloned().unwrap_or_default()
        };
    // The thinking block is expressed by thinkingLevelMap — removed before writing out.
    obj.remove("thinking");
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
        obj.remove("thinkingLevelMap");
        return Value::Object(obj);
    }
    let names: Vec<String> = crate::model::ordered_variants_text(&m.variants);
    let cur: HashSet<String> = names.iter().cloned().collect();
    let thinking_map: Map<String, Value> =
        // 1) When raw.thinkingLevelMap's value set matches the current tiers, keep verbatim (asymmetric mappings included).
        if let Some(rm) = m.raw.get("thinkingLevelMap").and_then(|v| v.as_object()) {
            let rm_vals: HashSet<String> = rm
                .values()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
            if rm_vals == cur {
                crate::model::order_variant_map(rm)
            } else {
                Map::new()
            }
        }
        // 2) When raw.thinking.effortMap's value set matches, serve as thinkingLevelMap.
        else if let Some(em) = m
            .raw
            .get("thinking")
            .and_then(|t| t.get("effortMap"))
            .and_then(|v| v.as_object())
        {
            let em_vals: HashSet<String> = em
                .values()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect();
            if em_vals == cur {
                crate::model::order_variant_map(em)
            } else {
                Map::new()
            }
        } else {
            Map::new()
        };
    // No match → the symmetric mapping {tier: tier}
    let thinking_map = if thinking_map.is_empty() {
        names
            .iter()
            .map(|n| (n.clone(), Value::String(n.clone())))
            .collect()
    } else {
        thinking_map
    };
    if !thinking_map.is_empty() {
        obj.insert("thinkingLevelMap".into(), Value::Object(thinking_map));
    }
    Value::Object(obj)
}

pub fn provider_from_pi(key: &str, v: &Value) -> ProviderRow {
    let api = str_at(v, "api");
    let npm = api_to_npm(api);
    // Normalized at load: `anthropic-messages`' baseUrl loses its trailing /v1.
    let base_url = without_v1_for_messages(api, str_at(v, "baseUrl"));
    let models = v
        .get("models")
        .and_then(|x| x.as_array())
        .map(|arr| arr.iter().map(model_from_pi).collect())
        .unwrap_or_default();
    let compat = v
        .get("compat")
        .and_then(|c| c.get("supportsDeveloperRole"))
        .and_then(|v| v.as_bool())
        // `compat` defaults: `api` non-empty and not `openai-completions`.
        .unwrap_or_else(|| !api.is_empty() && api != "openai-completions");
    // Either key true means true; default false.
    let requires_reasoning_content = v
        .get("compat")
        .and_then(Value::as_object)
        .and_then(|c| {
            c.get("requiresReasoningContentOnAssistantMessages")
                .or_else(|| c.get("requiresReasoningContentForAllAssistantTurns"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let r = ProviderRow {
        key: key.to_string(),
        description: String::new(),
        npm,
        base_url,
        api_key: str_at(v, "apiKey").to_string(),
        api_key_env: String::new(),
        original_api_key_env: String::new(),
        api_key_secret: String::new(),
        original_api_key_secret: String::new(),
        // DSH-side defaults.
        dsh_timeout_ms: "180000".into(),
        dsh_retry_mode: "normal".into(),
        dsh_max_retries: String::new(),
        original_dsh_timeout_ms: "180000".into(),
        original_dsh_retry_mode: "normal".into(),
        original_dsh_max_retries: String::new(),
        timeout: "180000".into(),
        original_timeout: "180000".into(),
        compat,
        requires_reasoning_content,
        original_requires_reasoning_content: requires_reasoning_content,
        models,
        new_model: ModelRow::new(),
        source_format: Some(crate::format::ConfigFormat::Pi),
        schema: crate::model::OpencodeSchemaVersion::V1,
        raw: v.clone(),
        pi_api: api.to_string(),
        qwen_pid: String::new(),
    };
    r
}

/// Move the given keys to the front of an object in the given order; remaining keys keep their relative order.
pub fn order_fields(object: Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    let mut ordered = Map::new();
    for key in keys {
        if let Some(value) = object.get(*key) {
            ordered.insert((*key).to_string(), value.clone());
        }
    }
    for (key, value) in &object {
        if !keys.contains(&key.as_str()) {
            ordered.insert(key.clone(), value.clone());
        }
    }
    ordered
}

/// pi / omp provider field order: baseUrl → apiKey → api → compat → models, the rest preserved.
const PROVIDER_FIELD_ORDER: &[&str] = &["baseUrl", "apiKey", "api", "compat", "models"];

pub fn provider_to_pi(p: &ProviderRow) -> Value {
    // opencode / DSH sources build fresh; the rest keep extension fields over raw as the base.
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_provider(&p.raw) || is_dsh_shaped_provider(&p.raw) {
            Map::new()
        } else {
            p.raw.as_object().cloned().unwrap_or_default()
        };
    // compat only adds/removes supportsDeveloperRole; other keys are kept.
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
    // Same format and unmodified → the key isn't written; otherwise the current value writes.
    let native_pi = matches!(
        p.source_format,
        Some(crate::format::ConfigFormat::Pi) | Some(crate::format::ConfigFormat::OhMyPi)
    );
    if p.requires_reasoning_content != p.original_requires_reasoning_content || !native_pi {
        let mut c = obj
            .get("compat")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        c.insert(
            "requiresReasoningContentOnAssistantMessages".into(),
            Value::Bool(p.requires_reasoning_content),
        );
        obj.insert("compat".into(), Value::Object(c));
    }
    let api = p.effective_api();
    if !p.base_url.is_empty() {
        let save_url = without_v1_for_messages(&api, &p.base_url);
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
    let models: Vec<Value> = p.models.iter().map(model_to_pi).collect();
    obj.insert("models".into(), Value::Array(models));
    Value::Object(order_fields(obj, PROVIDER_FIELD_ORDER))
}

pub fn load_pi_providers(root: &Value) -> Vec<ProviderRow> {
    root.get("providers")
        .and_then(|x| x.as_object())
        .map(|o| o.iter().map(|(k, pv)| provider_from_pi(k, pv)).collect())
        .unwrap_or_default()
}

pub fn load_pi_extras(root: &Value) -> Value {
    if let Some(obj) = root.as_object() {
        let extras: Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| k.as_str() != "providers")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Value::Object(extras)
    } else {
        Value::Object(Map::new())
    }
}

pub fn to_pi_root(providers: &[ProviderRow], extras: &Value) -> Value {
    let mut root = extras.as_object().cloned().unwrap_or_default();
    // Merge with the target's existing providers: same names overwrite, target-only entries stay.
    let existing = root
        .get("providers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut providers_map = existing;
    for p in providers.iter().filter(|p| !p.key.is_empty()) {
        let value = provider_to_pi(p);
        let entry = match providers_map.get(&p.key) {
            Some(target) => merge_conservative(target, &value),
            None => value,
        };
        providers_map.insert(p.key.clone(), entry);
    }
    root.insert("providers".into(), Value::Object(providers_map));
    Value::Object(root)
}

/// Conservative merge: `target` is the base; keys present in `source` overwrite, target-only keys are kept.
/// Objects merge recursively; arrays and scalars take `source`'s side.
pub fn merge_conservative(target: &Value, source: &Value) -> Value {
    match (target, source) {
        (Value::Object(target_obj), Value::Object(source_obj)) => {
            let mut out = target_obj.clone();
            for (key, source_value) in source_obj {
                match target_obj.get(key) {
                    Some(target_value) => {
                        out.insert(key.clone(), merge_conservative(target_value, source_value));
                    }
                    None => {
                        out.insert(key.clone(), source_value.clone());
                    }
                };
            }
            Value::Object(out)
        }
        _ => source.clone(),
    }
}
