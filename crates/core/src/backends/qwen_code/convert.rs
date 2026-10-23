use super::*;

pub(crate) fn is_builtin_pid(pid: &str) -> bool {
    BUILTIN_PIDS.contains(&pid)
}

/// Internal protocol → (built-in pid, the entry's `wireApi`).
///
/// Always falls back to a built-in pid. OpenAI's two protocols share the single `openai` pid,
/// distinguished by the entry's own `wireApi`.
pub(crate) fn pid_for_api(api: &str) -> (&'static str, &'static str) {
    match api.trim() {
        "openai-responses" => ("openai", WIRE_RESPONSES),
        "anthropic-messages" => ("anthropic", ""),
        "google-generative-ai" | "google-vertex" | "google-gemini-cli" => ("gemini", ""),
        _ => ("openai", WIRE_CHAT),
    }
}

/// Protocol bucket name (a built-in pid, or a `providerProtocol` value) → internal protocol.
///
/// `wireApi` beats the bucket name; non-OpenAI protocols have no `wireApi` concept.
pub(crate) fn bucket_to_api(bucket: &str, wire: &str) -> String {
    match bucket {
        "openai" => {
            if wire == WIRE_RESPONSES {
                "openai-responses".to_string()
            } else {
                "openai-completions".to_string()
            }
        }
        // A bucket named `openai-responses` directly is recognized too.
        "openai-responses" => "openai-responses".to_string(),
        "anthropic" => "anthropic-messages".to_string(),
        "gemini" | "vertex-ai" => "google-generative-ai".to_string(),
        "qwen-oauth" => "openai-completions".to_string(),
        // An unmapped custom pid: gets the fallback protocol; the entry still displays in the UI.
        _ => "openai-completions".to_string(),
    }
}

/// A pid's protocol bucket name: built-in pids are themselves; custom pids look up `providerProtocol`.
pub(crate) fn bucket_of(protocols: Option<&Map<String, Value>>, pid: &str) -> String {
    if is_builtin_pid(pid) {
        return pid.to_string();
    }
    protocols
        .and_then(|m| m.get(pid))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Entry → ProviderRow (one entry = one card).
pub(crate) fn provider_from_entry(
    pid: &str,
    protocols: Option<&Map<String, Value>>,
    entry: &Value,
    env: Option<&Map<String, Value>>,
) -> Option<ProviderRow> {
    let id = entry_id(entry)?.to_string();
    let wire = entry.get("wireApi").and_then(Value::as_str).unwrap_or("");
    let bucket = bucket_of(protocols, pid);
    let env_key = entry
        .get("envKey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let mut row = ProviderRow::new();
    row.key = id;
    // Write-back needs to know which pid this entry belongs to.
    row.qwen_pid = pid.to_string();
    row.pi_api = if bucket.is_empty() {
        "openai-completions".to_string()
    } else {
        bucket_to_api(&bucket, wire)
    };
    row.description = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.base_url = entry
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // The key lives in the top-level `env[<envKey>]`; `envKey` itself is stored too.
    row.api_key_env = env_key.clone();
    row.original_api_key_env = env_key.clone();
    row.api_key = env
        .and_then(|m| m.get(&env_key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // Entries without a timeout stay empty.
    row.timeout = entry
        .get("generationConfig")
        .and_then(|g| g.get("timeout"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    row.models = vec![model_from_entry(entry)];
    row.source_format = Some(ConfigFormat::QwenCode);
    row.raw = entry.clone();
    Some(row)
}

/// Entry → ModelRow. One entry is one model, all fields on the same entry.
pub(crate) fn model_from_entry(entry: &Value) -> ModelRow {
    let mut m = ModelRow::new();
    m.id = entry_id(entry).unwrap_or_default().to_string();
    m.name = entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&m.id)
        .to_string();
    // Override `ModelRow::new()`'s defaults one by one: fields the entry doesn't declare stay empty.
    m.context = entry
        .get("generationConfig")
        .and_then(|g| g.get("contextWindowSize"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    m.output = entry
        .get("generationConfig")
        .and_then(|g| g.get("samplingParams"))
        .and_then(|s| s.get("max_tokens"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    m.modalities_input = modalities_from_entry(entry);
    m.reasoning = entry
        .get("capabilities")
        .and_then(|c| c.get("reasoning"))
        .is_some_and(|r| r != &Value::Bool(false));
    // QwenCode has no "tool use" or `store` fields: not modeled.
    m.tool_call = false;
    m.store = false;
    m.variants = efforts_from_entry(entry).join(", ");
    m.original_variants = m.variants.clone();
    m.source_format = Some(ConfigFormat::QwenCode);
    m.raw = entry.clone();
    m
}

/// The entry's input modalities: `capabilities.vision` → `text[, image]`.
///
/// Undeclared returns an empty string (never `"text"`). Same accounting as WorkBuddy.
pub(crate) fn modalities_from_entry(entry: &Value) -> String {
    match entry
        .get("capabilities")
        .and_then(|c| c.get("vision"))
        .and_then(Value::as_bool)
    {
        Some(vision) => crate::convert::supports_to_modalities([("text", true), ("image", vision)]),
        None => String::new(),
    }
}

/// The entry's reasoning tiers: `capabilities.reasoning.efforts`.
pub(crate) fn efforts_from_entry(entry: &Value) -> Vec<String> {
    entry
        .get("capabilities")
        .and_then(|c| c.get("reasoning"))
        .and_then(|r| r.get("efforts"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Card + model → the entry's UI-managed fields.
///
/// The old entry (`old`) is the base; UI-managed keys are overwritten one by one while keys the UI doesn't
/// manage stay as-is — a sub-key-level merge.
///
/// Fields the UI clears delete their keys rather than writing empty values.
pub(crate) fn entry_from_provider(
    p: &ProviderRow,
    m: Option<&ModelRow>,
    id: &str,
    wire: &str,
    old: Option<&Value>,
) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    obj.insert("id".into(), Value::String(id.to_string()));
    set_str(
        &mut obj,
        "name",
        m.map(|m| m.name.trim()).unwrap_or_default(),
    );
    set_str(&mut obj, "description", p.description.trim());
    set_str(&mut obj, "baseUrl", p.base_url.trim());
    set_str(&mut obj, "envKey", &env_key_name(p));
    // `wireApi` is meaningful only for OpenAI-family protocols.
    set_str(&mut obj, "wireApi", wire);

    if let Some(m) = m {
        // capabilities: only the two sub-keys the UI can express are touched; the rest inherit from the old entry.
        let mut caps = obj
            .get("capabilities")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mods = split_csv(&m.modalities_input);
        if mods.is_empty() {
            caps.shift_remove("vision");
        } else {
            caps.insert(
                "vision".into(),
                Value::Bool(mods.iter().any(|s| s.eq_ignore_ascii_case("image"))),
            );
        }
        // The checkbox IS the "declare / don't declare" key: checked writes, unchecked deletes.
        if m.reasoning {
            let mut r = caps
                .get("reasoning")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let efforts = split_csv(&m.variants);
            if efforts.is_empty() {
                r.shift_remove("efforts");
            } else {
                r.insert(
                    "efforts".into(),
                    Value::Array(efforts.into_iter().map(Value::String).collect()),
                );
            }
            caps.insert("reasoning".into(), Value::Object(r));
        } else {
            caps.shift_remove("reasoning");
        }
        if caps.is_empty() {
            obj.shift_remove("capabilities");
        } else {
            obj.insert("capabilities".into(), Value::Object(caps));
        }

        // generationConfig: only `contextWindowSize` / `timeout` /
        // `samplingParams.max_tokens` are touched.
        let mut gen = obj
            .get("generationConfig")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        set_num(&mut gen, "contextWindowSize", m.context.trim());
        set_num(&mut gen, "timeout", p.timeout.trim());
        let mut sp = gen
            .get("samplingParams")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        set_num(&mut sp, "max_tokens", m.output.trim());
        if sp.is_empty() {
            gen.shift_remove("samplingParams");
        } else {
            gen.insert("samplingParams".into(), Value::Object(sp));
        }
        if gen.is_empty() {
            obj.shift_remove("generationConfig");
        } else {
            obj.insert("generationConfig".into(), Value::Object(gen));
        }
    }

    Value::Object(obj)
}

/// The `envKey` variable name for the entry: the UI's value wins; with only a key filled and no variable
/// name, one derives from the provider key (the same convention as DSH's
/// [`crate::credentials::default_env_name`]).
///
/// "Explicitly cleared in the UI" and "never filled" are judged separately: a previously-set name now
/// empty = the user cleared it — the clearing is respected; only originally-empty names derive. Both
/// name and key empty also return an empty string.
pub(crate) fn env_key_name(p: &ProviderRow) -> String {
    let named = p.api_key_env.trim();
    if !named.is_empty() {
        return named.to_string();
    }
    if !p.original_api_key_env.trim().is_empty() || p.api_key.trim().is_empty() {
        return String::new();
    }
    crate::credentials::default_env_name(&p.key)
}
