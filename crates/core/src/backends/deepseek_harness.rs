//! The DeepSeek Harness (DSH) backend: `~/.dsh/profiles/<profile>/cordis.patch.yml`.
//!
//! Only the `llm-pi-ai` entry's `config.providers` is managed; everything else in the document is preserved.
//!
//! DSH ≥ 0.2 keeps provider config inside its **profile patch** documents (`profiles/desktop/`,
//! `profiles/sdk/`, `profiles/web/`), as the `llm-pi-ai` entry of a top-level array of loader patches. The older
//! Provider secrets live in the harness-wide `~/.dsh/.credentials.yaml` (never inside a profile directory).
//!
//! `~/.dsh/settings.yaml` is no longer read as live config: DSH imports it once, re-serializes it to
//! `settings.yaml.imported` and moves the original aside. That form is still parsed here so a path pointed at
//! one keeps working, but new saves target (and create) the patch document.

use super::{Backend, BackendLoad};
use crate::convert;
use crate::convert::order_fields;
use crate::credentials;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{home_dir_string, parse_yaml_content, wsl_home};
use serde_json::{Map, Value};

pub struct DeepSeekHarnessBackend;
pub static BACKEND: DeepSeekHarnessBackend = DeepSeekHarnessBackend;

/// The profile names a DSH install can carry, in priority order.
///
/// DSH keeps one profile per entry point — the desktop app (`desktop`), the CLI / SDK (`sdk`) and the web app
/// (`web`) — and which of them exist depends on what the machine installed: an SDK-only box has no `desktop`
/// document, its providers live in `profiles/sdk/`. The page therefore targets **the first candidate present
/// on disk** instead of a hard-coded name, and mirrors into the other present ones.
const PROFILE_CANDIDATES: [&str; 3] = ["desktop", "sdk", "web"];

/// The profile used when none of the candidates exists yet, so a fresh machine creates the desktop document —
/// the one DSH's own desktop app would create.
const PROFILE_FALLBACK: &str = "desktop";

/// The patch entry carrying provider config: `id: llm-pi-ai` → `config.providers`.
const PATCH_ENTRY_ID: &str = "llm-pi-ai";
const PATCH_ENTRY_NAME: &str = "@deepseek-ai/dsh-llm-pi-ai";

/// The providers block's indentation in the patch document (`- id` / `  name` / `  config` / `    providers`).
const PATCH_PROVIDERS_INDENT: usize = 6;

/// A fresh patch document's header, copied from DSH's own template so a file created here looks like DSH's.
const PATCH_HEADER: &str = "# Your patch layer for this dsh profile, applied after every bundle layer:\n# a top-level YAML array of loader patch entries (id-targeted config\n# overrides, disables, and insert lists; `!!js` expressions allowed).\n";

fn default_local_path() -> String {
    format!("{}\\cordis.patch.yml", profile_dir(default_profile()))
}

/// One DSH profile directory (`~/.dsh/profiles/<profile>`).
fn profile_dir(profile: &str) -> String {
    format!("{}\\.dsh\\profiles\\{}", home_dir_string(), profile)
}

/// The credentials store DSH actually reads: `<dsh home>/.credentials.yaml`.
///
/// The store is harness-wide, not per profile: `dsh-credentials-local` resolves it as
/// `$DSH_HOME/.credentials.yaml` and never reads `<profile>/.credentials.yaml`. For a profile patch document
/// (`~/.dsh/profiles/<name>/cordis.patch.yml`) that means walking up past `profiles/<name>`; a legacy
/// `~/.dsh/settings.yaml` already sits in the dsh home, so its sibling is the same file.
fn credentials_path(config_path: &str) -> String {
    let normalized = config_path.replace('\\', "/");
    if let Some(idx) = normalized.find("/profiles/") {
        let sep = if config_path[idx..].starts_with('\\') {
            '\\'
        } else {
            '/'
        };
        let mut out = String::with_capacity(idx + 20);
        out.push_str(&config_path[..idx]);
        out.push_str(&format!("{sep}.credentials.yaml"));
        return out;
    }
    crate::util::sibling_path(config_path, ".credentials.yaml")
}

/// The candidate profiles present on this machine, in priority order.
fn existing_profiles() -> Vec<&'static str> {
    PROFILE_CANDIDATES
        .into_iter()
        .filter(|profile| std::path::Path::new(&profile_dir(profile)).exists())
        .collect()
}

/// The profile the DSH page reads and writes by default: the first candidate present on disk.
///
/// Nothing present means no DSH profile exists at all (or the home directory can't be read): fall back to the
/// desktop name so the page still shows a plausible target rather than an empty path.
fn default_profile() -> &'static str {
    existing_profiles()
        .first()
        .copied()
        .unwrap_or(PROFILE_FALLBACK)
}

/// Whether a path names a profile patch document (the DSH ≥ 0.2 config form).
fn is_patch_path(path: &str) -> bool {
    path.replace('\\', "/").ends_with("/cordis.patch.yml")
}

/// The other profiles' patch documents for `path`: every candidate except the profile the path names.
///
/// Empty for a path that isn't a profile patch path — an arbitrary file has no profile set to keep in step.
///
/// Matching runs on a slash-normalized copy — both separators are single ASCII bytes, so indices stay valid for
/// the original — and the inserted segment reuses the original's separator style. A path that mixes styles (typed
/// by hand, or joined from parts) therefore still finds its siblings, exactly like `is_patch_path` accepts it as a
/// patch document: the profiles must not drift apart because of a separator.
fn mirror_paths(path: &str) -> Vec<String> {
    let normalized = path.replace('\\', "/");
    let Some((from, idx)) = PROFILE_CANDIDATES.into_iter().find_map(|profile| {
        normalized
            .find(&format!("/profiles/{profile}/"))
            .map(|idx| (profile, idx))
    }) else {
        return Vec::new();
    };
    let needle = format!("/profiles/{from}/");
    let sep = if path[idx..].starts_with('\\') {
        '\\'
    } else {
        '/'
    };
    let tail = &path[idx + needle.len()..];
    PROFILE_CANDIDATES
        .into_iter()
        .filter(|profile| *profile != from)
        .map(|profile| {
            let mut out = String::with_capacity(path.len() + 8);
            out.push_str(&path[..idx]);
            out.push_str(&format!("{sep}profiles{sep}{profile}{sep}"));
            out.push_str(tail);
            out
        })
        .collect()
}

/// The `llm-pi-ai` entry of a patch document.
fn patch_entry(root: &Value) -> Option<&Value> {
    root.as_array()?
        .iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(PATCH_ENTRY_ID))
}

/// The provider map, from either document form: the profile patch array, or a legacy `settings.yaml` root.
fn providers_of(root: &Value) -> Option<&Map<String, Value>> {
    if root.is_array() {
        patch_entry(root)?
            .get("config")?
            .get("providers")?
            .as_object()
    } else {
        root.get("llm-pi-ai")?.get("providers")?.as_object()
    }
}

fn is_dsh_root(v: &Value) -> bool {
    providers_of(v).is_some()
}

fn model_from_dsh(v: &Value) -> ModelRow {
    let mut m = convert::model_from_pi(v);
    m.variants = v
        .get("reasoningEfforts")
        .and_then(Value::as_object)
        .map(|o| {
            o.values()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    m.original_variants = m.variants.clone();
    m.source_format = Some(ConfigFormat::DeepSeekHarness);
    m.raw = v.clone();
    m
}

fn model_to_dsh(m: &ModelRow, preserve_raw: bool) -> Value {
    // DSH's native round trip keeps unknown fields; cross-format writes use DSH's fixed field order.
    let mut obj = dsh_model_base(m, preserve_raw);

    dsh_set_id(m, &mut obj, preserve_raw);
    dsh_set_name(m, &mut obj, preserve_raw);
    dsh_set_input(m, &mut obj, preserve_raw);
    dsh_set_number(
        &mut obj,
        "contextWindow",
        &m.context,
        !preserve_raw || m.context != crate::util::num_at(&m.raw, "contextWindow"),
    );
    dsh_set_number(
        &mut obj,
        "maxTokens",
        &m.output,
        !preserve_raw || m.output != crate::util::num_at(&m.raw, "maxTokens"),
    );
    if !preserve_raw || m.variants != m.original_variants {
        dsh_set_reasoning_efforts(m, &mut obj);
    }

    Value::Object(order_fields(obj, DSH_MODEL_FIELDS))
}

/// A DSH model's fixed field order.
const DSH_MODEL_FIELDS: &[&str] = &[
    "id",
    "name",
    "contextWindow",
    "maxTokens",
    "input",
    "reasoningEfforts",
];

/// The base object: native round trips reuse raw; cross-format writes rebuild from id / name / numeric fields.
fn dsh_model_base(m: &ModelRow, preserve_raw: bool) -> Map<String, Value> {
    if preserve_raw {
        return m.raw.as_object().cloned().unwrap_or_default();
    }
    let mut fields = Map::new();
    fields.insert("id".into(), Value::String(m.id.clone()));
    if !m.name.trim().is_empty() {
        fields.insert("name".into(), Value::String(m.name.clone()));
    }
    if let Some(v) = crate::util::parse_number_text(&m.context) {
        fields.insert("contextWindow".into(), v);
    }
    if let Some(v) = crate::util::parse_number_text(&m.output) {
        fields.insert("maxTokens".into(), v);
    }
    fields
}

fn dsh_set_id(m: &ModelRow, obj: &mut Map<String, Value>, preserve_raw: bool) {
    if !preserve_raw || m.id != m.raw.get("id").and_then(Value::as_str).unwrap_or("") {
        obj.insert("id".into(), Value::String(m.id.clone()));
    }
}

fn dsh_set_name(m: &ModelRow, obj: &mut Map<String, Value>, preserve_raw: bool) {
    if preserve_raw && m.name == m.raw.get("name").and_then(Value::as_str).unwrap_or("") {
        return;
    }
    if m.name.trim().is_empty() {
        obj.remove("name");
    } else {
        obj.insert("name".into(), Value::String(m.name.clone()));
    }
}

fn dsh_set_input(m: &ModelRow, obj: &mut Map<String, Value>, preserve_raw: bool) {
    let input: Vec<Value> = m
        .modalities_input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| Value::String(s.to_string()))
        .collect();
    let raw_input = m.raw.get("input").and_then(Value::as_array);
    let input_changed = raw_input
        .map(|raw| raw != input.as_slice())
        .unwrap_or(!input.is_empty());
    if !preserve_raw || input_changed {
        if input.is_empty() {
            obj.remove("input");
        } else {
            obj.insert("input".into(), Value::Array(input));
        }
    }
}

/// Numeric fields: empty or invalid input deletes the key.
fn dsh_set_number(obj: &mut Map<String, Value>, key: &str, text: &str, changed: bool) {
    if !changed {
        return;
    }
    match crate::util::parse_number_text(text) {
        Some(v) => {
            obj.insert(key.into(), v);
        }
        None => {
            obj.remove(key);
        }
    }
}

/// `reasoningEfforts`: written in canonical tier order; mappings already in raw keep their keys verbatim.
fn dsh_set_reasoning_efforts(m: &ModelRow, obj: &mut Map<String, Value>) {
    if m.variants.trim().is_empty() {
        obj.remove("reasoningEfforts");
        return;
    }
    let raw_efforts = m.raw.get("reasoningEfforts").and_then(Value::as_object);
    let mut efforts = Map::new();
    for name in crate::model::ordered_variants_text(&m.variants) {
        let existing =
            raw_efforts.and_then(|raw| raw.iter().find(|(_, v)| v.as_str() == Some(name.as_str())));
        match existing {
            Some((key, value)) => {
                efforts.insert(key.clone(), value.clone());
            }
            None => {
                efforts.insert(name.clone(), Value::String(name));
            }
        }
    }
    obj.insert("reasoningEfforts".into(), Value::Object(efforts));
}

fn provider_from_dsh(key: &str, v: &Value, credentials_root: &Value) -> ProviderRow {
    let api = v.get("api").and_then(Value::as_str).unwrap_or_default();
    let models = v
        .get("models")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(model_from_dsh).collect())
        .unwrap_or_default();
    let env = dsh_api_key_env(key, v);
    let timeout_text = dsh_timeout_text(v);
    let retry_mode = dsh_retry_mode(v);
    let max_retries = dsh_max_retries(v);
    let secret = credentials::secret_for(credentials_root, &env);
    ProviderRow {
        key: key.to_string(),
        description: String::new(),
        npm: convert::api_to_npm(api),
        base_url: convert::without_v1_for_messages(
            api,
            v.get("baseURL").and_then(Value::as_str).unwrap_or_default(),
        ),
        api_key: secret.clone(),
        api_key_env: env.clone(),
        original_api_key_env: env.clone(),
        api_key_secret: secret.clone(),
        original_api_key_secret: secret,
        dsh_timeout_ms: timeout_text.clone(),
        dsh_retry_mode: retry_mode.clone(),
        dsh_max_retries: max_retries.clone(),
        original_dsh_timeout_ms: timeout_text.clone(),
        original_dsh_retry_mode: retry_mode,
        original_dsh_max_retries: max_retries,
        // Switching to the opencode page carries DSH's timeoutMs over (default 180000).
        timeout: timeout_text.clone(),
        original_timeout: timeout_text,
        // DSH has no compat key: unchecked by default when the api is absent or openai-completions.
        compat: !api.is_empty() && api != "openai-completions",
        // DSH has no such field: unchecked by default when moving to pi/omp pages.
        requires_reasoning_content: false,
        original_requires_reasoning_content: false,
        models,
        new_model: ModelRow::new(),
        source_format: Some(ConfigFormat::DeepSeekHarness),
        schema: crate::model::OpencodeSchemaVersion::V1,
        raw: v.clone(),
        pi_api: api.to_string(),
        qwen_pid: String::new(),
    }
}

/// `apiKeyEnv`: a missing or blank one generates the default env name from the provider key.
fn dsh_api_key_env(key: &str, v: &Value) -> String {
    v.get("apiKeyEnv")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| credentials::default_env_name(key))
}

/// Without a timeoutMs, 180000ms is the default.
fn dsh_timeout_text(v: &Value) -> String {
    let t = crate::util::num_at(v, "timeoutMs");
    if t.is_empty() {
        "180000".to_string()
    } else {
        t
    }
}

/// `retryPolicy.mode`: normal when missing.
fn dsh_retry_mode(v: &Value) -> String {
    v.get("retryPolicy")
        .and_then(|value| value.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("normal")
        .to_string()
}

/// `retryPolicy.maxRetries`: read as raw text, empty when unset.
fn dsh_max_retries(v: &Value) -> String {
    v.get("retryPolicy")
        .and_then(|value| value.get("maxRetries"))
        .map(crate::util::number_text_public)
        .unwrap_or_default()
}

fn dsh_api_for(p: &ProviderRow) -> String {
    // Shares one priority with pi / omp / the latency tests.
    p.effective_api()
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => {
            if v.is_empty()
                || v.chars().any(|c| c.is_whitespace() && c != ' ')
                || v.contains(':')
                || v.starts_with([
                    '[', ']', '{', '}', '#', '!', '&', '*', '?', '-', '|', '>', '@', '`',
                ])
                || matches!(v.as_str(), "null" | "true" | "false" | "~")
            {
                serde_yaml_ng::to_string(v)
                    .unwrap_or_else(|_| format!("\"{}\"", v.replace('"', "\\\"")))
                    .trim()
                    .to_string()
            } else {
                v.clone()
            }
        }
        _ => serde_yaml_ng::to_string(value)
            .unwrap_or_else(|_| "null\n".into())
            .trim()
            .to_string(),
    }
}

fn yaml_quoted(value: &str) -> String {
    // A JSON string is a valid YAML double-quoted string, escaping quotes, backslashes, and newlines.
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value.replace('"', "\\\"")))
}

fn dsh_scalar(key: &str, value: &Value) -> String {
    if matches!(key, "apiKeyEnv" | "api" | "baseURL" | "id" | "name") {
        if let Value::String(text) = value {
            return yaml_quoted(text);
        }
    }
    yaml_scalar(value)
}

fn yaml_flow(value: &Value, quote_strings: bool) -> Option<String> {
    let scalar = |value: &Value| match value {
        Value::String(text) if quote_strings => yaml_quoted(text),
        _ => yaml_scalar(value),
    };
    match value {
        Value::Array(items) if items.iter().all(Value::is_string) => Some(format!(
            "[ {} ]",
            items.iter().map(scalar).collect::<Vec<_>>().join(", ")
        )),
        Value::Object(object)
            if object
                .values()
                .all(|v| v.is_string() || v.is_number() || v.is_boolean()) =>
        {
            Some(format!(
                "{{ {} }}",
                object
                    .iter()
                    .map(|(k, v)| {
                        let key = if quote_strings {
                            yaml_quoted(k)
                        } else {
                            yaml_scalar(&Value::String(k.clone()))
                        };
                        format!("{}: {}", key, scalar(v))
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
        _ => None,
    }
}

fn render_dsh_value(value: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let key_text = yaml_scalar(&Value::String(key.clone()));
                let flow = if matches!(key.as_str(), "input" | "reasoningEfforts") {
                    yaml_flow(child, true)
                } else {
                    None
                };
                if let Some(flow) = flow {
                    out.push_str(&format!("{}{}: {}\n", pad, key_text, flow));
                } else if child.is_object() {
                    if child.as_object().is_some_and(Map::is_empty) {
                        out.push_str(&format!("{}{}: {{}}\n", pad, key_text));
                    } else {
                        out.push_str(&format!("{}{}:\n", pad, key_text));
                        render_dsh_value(child, indent + 2, out);
                    }
                } else if child.is_array() {
                    if child.as_array().is_some_and(Vec::is_empty) {
                        out.push_str(&format!("{}{}: []\n", pad, key_text));
                    } else {
                        out.push_str(&format!("{}{}:\n", pad, key_text));
                        render_dsh_value(child, indent + 2, out);
                    }
                } else {
                    out.push_str(&format!(
                        "{}{}: {}\n",
                        pad,
                        key_text,
                        dsh_scalar(key, child)
                    ));
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(object) = item.as_object() {
                    if object.is_empty() {
                        out.push_str(&format!("{}- {{}}\n", pad));
                        continue;
                    }
                    let mut first = true;
                    for (key, child) in object {
                        let key_text = yaml_scalar(&Value::String(key.clone()));
                        if first {
                            first = false;
                            let flow = if matches!(key.as_str(), "input" | "reasoningEfforts") {
                                yaml_flow(child, true)
                            } else {
                                None
                            };
                            if let Some(flow) = flow {
                                out.push_str(&format!("{}- {}: {}\n", pad, key_text, flow));
                            } else if child.is_object() || child.is_array() {
                                out.push_str(&format!("{}- {}:\n", pad, key_text));
                                render_dsh_value(child, indent + 4, out);
                            } else {
                                out.push_str(&format!(
                                    "{}- {}: {}\n",
                                    pad,
                                    key_text,
                                    dsh_scalar(key, child)
                                ));
                            }
                        } else {
                            let flow = if matches!(key.as_str(), "input" | "reasoningEfforts") {
                                yaml_flow(child, true)
                            } else {
                                None
                            };
                            if let Some(flow) = flow {
                                out.push_str(&format!("{}  {}: {}\n", pad, key_text, flow));
                            } else if child.is_object() || child.is_array() {
                                out.push_str(&format!("{}  {}:\n", pad, key_text));
                                render_dsh_value(child, indent + 4, out);
                            } else {
                                out.push_str(&format!(
                                    "{}  {}: {}\n",
                                    pad,
                                    key_text,
                                    dsh_scalar(key, child)
                                ));
                            }
                        }
                    }
                } else {
                    out.push_str(&format!("{}- {}\n", pad, yaml_scalar(item)));
                }
            }
        }
        _ => out.push_str(&format!("{}{}\n", pad, yaml_scalar(value))),
    }
}

fn render_dsh_yaml(root: &Value) -> Result<String, String> {
    let mut output = String::new();
    render_dsh_value(root, 0, &mut output);
    Ok(output)
}

fn provider_to_dsh(p: &ProviderRow) -> Value {
    // The save verdict keys on the source agent's format.
    let preserve_raw = p.source_format == Some(ConfigFormat::DeepSeekHarness);
    let env = credentials::effective_env_name(p);
    let mut obj = if preserve_raw {
        p.raw.as_object().cloned().unwrap_or_default()
    } else {
        let mut fields = Map::new();
        if !env.is_empty() {
            fields.insert("apiKeyEnv".into(), Value::String(env.clone()));
        }
        fields.insert("api".into(), Value::String(dsh_api_for(p)));
        if !p.base_url.is_empty() {
            fields.insert("baseURL".into(), Value::String(p.base_url.clone()));
        }
        fields
    };
    obj.insert("api".into(), Value::String(dsh_api_for(p)));
    if p.base_url.is_empty() {
        obj.remove("baseURL");
    } else {
        // Shares normalization with pi / omp: a messages protocol's base carries no /v1.
        let base_url = convert::without_v1_for_messages(&dsh_api_for(p), &p.base_url);
        obj.insert("baseURL".into(), Value::String(base_url));
    }
    if env.is_empty() {
        // A native DSH provider with an unmodified apiKeyEnv keeps its original reference name.
        if !preserve_raw {
            obj.remove("apiKeyEnv");
        }
    } else {
        obj.insert("apiKeyEnv".into(), Value::String(env));
    }
    obj.insert(
        "models".into(),
        Value::Array(
            p.models
                .iter()
                .map(|model| model_to_dsh(model, preserve_raw))
                .collect(),
        ),
    );
    if p.dsh_retry_mode != p.original_dsh_retry_mode
        || p.dsh_max_retries != p.original_dsh_max_retries
        || !preserve_raw
    {
        let retries_set = crate::util::parse_number_text(&p.dsh_max_retries).is_some();
        if p.dsh_retry_mode.trim().is_empty() && !retries_set {
            obj.remove("retryPolicy");
        } else {
            let mut policy = obj
                .get("retryPolicy")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            // With mode empty but maxRetries filled, DSH's default normal is written.
            let mode = if p.dsh_retry_mode.trim().is_empty() {
                "normal"
            } else {
                p.dsh_retry_mode.as_str()
            };
            policy.insert("mode".into(), Value::String(mode.to_string()));
            match crate::util::parse_number_text(&p.dsh_max_retries) {
                Some(value) => {
                    policy.insert("maxRetries".into(), value);
                }
                None => {
                    policy.remove("maxRetries");
                }
            }
            obj.insert("retryPolicy".into(), Value::Object(policy));
        }
    }
    // retryPolicy writes before timeoutMs.
    if p.dsh_timeout_ms != p.original_dsh_timeout_ms || !preserve_raw {
        match crate::util::parse_number_text(&p.dsh_timeout_ms) {
            Some(value) => {
                obj.insert("timeoutMs".into(), value);
            }
            None => {
                obj.remove("timeoutMs");
            }
        }
    }
    Value::Object(order_fields(
        obj,
        &["apiKeyEnv", "api", "baseURL", "models"],
    ))
}

/// The provider map for a document: UI rows → DSH provider values.
///
/// `existing` is the merge base for a cross-format (upsert) save; a plain save starts empty so deletions made
/// in the UI take effect.
/// The provider map to write: the UI's rows, in the UI's order (drag-reorder is visible across pages).
///
/// A cross-format target donates only **fields** the UI does not model: a same-named entry merges
/// conservatively, so a provider keeps the target's `retryPolicy` / `timeoutMs` (written only when the UI
/// sets them, so their absence means "keep the target's"). Target-only providers are dropped: providers
/// are the UI-managed container, and a deletion has to take effect.
fn provider_values(
    providers: &[ProviderRow],
    cross_format: bool,
    existing: &Map<String, Value>,
) -> Map<String, Value> {
    let mut values = Map::new();
    for p in providers.iter().filter(|p| !p.key.is_empty()) {
        let value = provider_to_dsh(p);
        let entry = match (cross_format, existing.get(&p.key)) {
            (true, Some(target)) => convert::merge_conservative(target, &value),
            _ => value,
        };
        values.insert(p.key.clone(), entry);
    }
    values
}

/// The patch document with the UI's providers written into the `llm-pi-ai` entry.
///
/// Every other entry — and the entry order — stays as it was: DSH's profile patch also carries ui / theme /
/// permission entries, and rebuilding the document wholesale would drop or move them. A document without the
/// entry gets one appended.
fn patch_root(base: &Value, providers: &[ProviderRow], cross_format: bool) -> Value {
    let mut items = base.as_array().cloned().unwrap_or_default();
    let idx = items
        .iter()
        .position(|item| item.get("id").and_then(Value::as_str) == Some(PATCH_ENTRY_ID));
    let existing = idx
        .and_then(|i| items[i].get("config"))
        .and_then(|config| config.get("providers"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let values = provider_values(providers, cross_format, &existing);
    let mut entry = match idx {
        Some(i) => items[i].clone(),
        None => {
            let mut entry = Map::new();
            entry.insert("id".into(), Value::String(PATCH_ENTRY_ID.into()));
            entry.insert("name".into(), Value::String(PATCH_ENTRY_NAME.into()));
            Value::Object(entry)
        }
    };
    // Only `config.providers` is ours; any other key under `config` (and in the entry) stays.
    let mut config = entry
        .get("config")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    config.insert("providers".into(), Value::Object(values));
    if let Some(obj) = entry.as_object_mut() {
        obj.insert("config".into(), Value::Object(config));
    }
    match idx {
        Some(i) => items[i] = entry,
        None => items.push(entry),
    }
    Value::Array(items)
}

/// The providers map rendered as YAML at the patch document's indentation (`render_dsh_value` nests identically).
fn render_provider_block(providers: &Map<String, Value>) -> String {
    let mut out = String::new();
    render_dsh_value(
        &Value::Object(providers.clone()),
        PATCH_PROVIDERS_INDENT,
        &mut out,
    );
    out
}

/// A line's leading-space width (these documents use spaces, never tabs).
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Whether a line opens a new top-level patch entry (entries start at column 0).
fn starts_entry(line: &str) -> bool {
    line.starts_with("- ")
}

/// Whether a line is the `- id: <id>` header of a patch entry.
///
/// Tolerates the quoted form (`- id: "llm-pi-ai"`): this document's writer (and DSH's own editor) quote the id,
/// and missing that match would send the save down the "create a fresh document" path, silently dropping the
/// header comment and every other entry.
fn is_entry_id_line(line: &str, id: &str) -> bool {
    line.strip_prefix("- id:").is_some_and(|rest| {
        let value = rest.trim();
        value == id || value == format!("\"{id}\"") || value == format!("'{id}'")
    })
}

/// The target file's line ending, read from its **raw bytes**.
///
/// `read_config_content` hands back normalized text, so the document's own ending can't be inferred from it;
/// writing LF into a CRLF file leaves it with mixed endings, which the next splice then reads wrongly.
fn file_line_ending(path: &str) -> &'static str {
    match std::fs::read(path) {
        Ok(bytes) if bytes.windows(2).any(|pair| pair == b"\r\n") => "\r\n",
        _ => "\n",
    }
}

/// Replace the children of `providers:` inside the `llm-pi-ai` entry, in place.
///
/// Text surgery rather than a re-render: the document belongs to DSH and carries a header comment plus (in the
/// desktop profile) nine other patch entries the user may have hand-edited. Parsing it to a value and emitting
/// it again would drop the comments and reformat all of them. Returns None when the entry or its `providers:`
/// key is missing — the caller then falls back to a fresh document.
fn splice_providers(text: &str, providers_block: &str, eol: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let entry_start = lines
        .iter()
        .position(|line| is_entry_id_line(line, PATCH_ENTRY_ID))?;
    // The entry runs to the next entry that opens (or to the end of the document).
    let entry_end = lines[entry_start + 1..]
        .iter()
        .position(|line| starts_entry(line))
        .map_or(lines.len(), |i| entry_start + 1 + i);
    let key_line = (entry_start..entry_end).find(|&i| {
        // `split_inclusive` keeps each line's terminator: compare against the trimmed content, not the raw line.
        let trimmed = lines[i].trim();
        trimmed == "providers:" || trimmed.starts_with("providers: ")
    })?;
    let key_indent = indent_of(lines[key_line]);
    // The block's children end at the next line at or above the key's own indentation.
    let children_end = (key_line + 1..entry_end)
        .find(|&i| !lines[i].trim().is_empty() && indent_of(lines[i]) <= key_indent)
        .unwrap_or(entry_end);
    let pad = " ".repeat(key_indent);
    let mut out = String::with_capacity(text.len() + providers_block.len());
    for line in &lines[..key_line] {
        out.push_str(line);
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(eol);
    }
    if providers_block.is_empty() {
        // An empty map needs the flow form: a bare `providers:` with no children parses as null.
        out.push_str(&format!("{pad}providers: {{}}{eol}"));
    } else {
        // The key line is rewritten rather than copied: the original may carry an inline `{}`.
        out.push_str(&format!("{pad}providers:{eol}"));
        if eol == "\n" {
            out.push_str(providers_block);
        } else {
            out.push_str(&providers_block.replace('\n', eol));
        }
    }
    for line in &lines[children_end..] {
        out.push_str(line);
    }
    Some(out)
}

/// A legacy `settings.yaml` root converted to the patch-document form, keeping `llm-pi-ai`'s other fields.
///
/// Only `providers` is UI-managed; any sibling key under `llm-pi-ai` (retry defaults and so on) travels with
/// the entry so a conversion doesn't silently drop configuration.
fn legacy_to_patch(root: &Value) -> Result<Value, String> {
    let llm = root
        .get("llm-pi-ai")
        .ok_or_else(|| "旧式 settings.yaml 缺少 llm-pi-ai 节点".to_string())?;
    let mut config = llm.as_object().cloned().unwrap_or_default();
    let providers = config.remove("providers").unwrap_or_else(json_object);
    let mut entry = Map::new();
    entry.insert("id".into(), Value::String(PATCH_ENTRY_ID.into()));
    entry.insert("name".into(), Value::String(PATCH_ENTRY_NAME.into()));
    config.insert("providers".into(), providers);
    entry.insert("config".into(), Value::Object(config));
    Ok(Value::Array(vec![Value::Object(entry)]))
}

fn json_object() -> Value {
    Value::Object(Map::new())
}

impl Backend for DeepSeekHarnessBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::DeepSeekHarness
    }
    fn default_local_path(&self) -> String {
        default_local_path()
    }
    /// Read from the named document first, then from the other profiles': a machine whose providers sit in
    /// another profile's document still loads them instead of showing an empty list, whichever profile the page
    /// happens to point at.
    fn path_candidates(&self, path: &str) -> Vec<String> {
        let mut candidates = vec![path.to_string()];
        candidates.extend(mirror_paths(path));
        candidates
    }
    fn default_wsl_path(&self) -> Option<String> {
        Some(format!(
            "{}/.dsh/profiles/{}/cordis.patch.yml",
            wsl_home()?,
            default_profile()
        ))
    }

    fn detect(&self, content: &str, path: &str) -> bool {
        let lower = path.to_lowercase();
        if lower.contains("/.dsh/") || lower.contains("\\.dsh\\") {
            return true;
        }
        parse_yaml_content(content)
            .map(|v| is_dsh_root(&v))
            .unwrap_or(false)
    }
    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        self.parse_at(content, "")
    }
    fn parse_at(&self, content: &str, path: &str) -> Result<BackendLoad, String> {
        let root = parse_yaml_content(content)?;
        let creds = if path.is_empty() {
            Value::Object(Map::new())
        } else {
            // Display only: when the store is unreadable, the key shows empty.
            credentials::load_root_at(&credentials_path(path)).unwrap_or(Value::Null)
        };
        let providers = providers_of(&root)
            .map(|o| {
                o.iter()
                    .map(|(k, v)| provider_from_dsh(k, v, &creds))
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
        let base = target_root.cloned().unwrap_or_else(|| extras.clone());
        // The UI's provider list decides both the set and the order in either case — a deletion has to take
        // effect. A cross-format target only donates fields the UI does not model, for the providers it
        // already has (see `provider_values`).
        let cross_format = target_root.is_some();
        if base.is_array() {
            return patch_root(&base, providers, cross_format);
        }
        let mut root = base.as_object().cloned().unwrap_or_default();
        let existing = root
            .get("llm-pi-ai")
            .and_then(|v| v.get("providers"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let provider_values = provider_values(providers, cross_format, &existing);
        // No remove + re-insert of llm-pi-ai: a re-inserted key would move to the root's tail.
        if let Some(llm) = root.get_mut("llm-pi-ai").and_then(Value::as_object_mut) {
            llm.insert("providers".into(), Value::Object(provider_values));
        } else {
            let mut llm = Map::new();
            llm.insert("providers".into(), Value::Object(provider_values));
            root.insert("llm-pi-ai".into(), Value::Object(llm));
        }
        Value::Object(root)
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        // A patch document is a top-level array; a missing one starts empty in that shape, so a cross-format
        // save appends a fresh `llm-pi-ai` entry instead of writing a legacy settings.yaml object.
        let empty = if is_patch_path(path) {
            Value::Array(Vec::new())
        } else {
            Value::Object(Map::new())
        };
        super::load_target_root_with(path, parse_yaml_content, || empty)
    }
    /// The credentials store DSH reads (`<dsh home>/.credentials.yaml`). The sibling profile's document is
    /// **not** written here: it needs the same providers the primary write produced, which this hook doesn't
    /// receive (see `mirror_targets`).
    fn save_sidecars(&self, path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        credentials::save_at(&credentials_path(path), providers)
    }

    /// The other profiles' documents, each of which exists **and can be spliced into**.
    ///
    /// Only existing profiles are mirrored: creating another profile's document could show DSH a profile it
    /// never set up. A document must already carry the `llm-pi-ai` entry, because a mirror is rendered into
    /// the file's own text — a document without the entry would take `render_at`'s "write a fresh document"
    /// path and lose every other entry it had.
    fn mirror_targets(&self, path: &str) -> Vec<String> {
        mirror_paths(path)
            .into_iter()
            .filter(|sibling| {
                crate::util::read_config_content(sibling)
                    .ok()
                    .and_then(|text| parse_yaml_content(&text).ok())
                    .is_some_and(|doc| providers_of(&doc).is_some())
            })
            .collect()
    }
    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/deepseek-harness_32.bin"),
            32,
            32,
        ))
    }
    fn render(&self, root: &Value, _compact: bool) -> Result<String, String> {
        render_dsh_yaml(root)
    }

    /// Render into the target document, editing an existing patch document in place.
    fn render_at(&self, root: &Value, compact: bool, path: &str) -> Result<String, String> {
        // A legacy settings.yaml root (the UI loaded that form, or nothing was loaded yet) still saves into a
        // patch document when the target is one: the **path** decides the document form, not the in-memory
        // shape. Without this a machine that first opened an old settings.yaml would keep rewriting that form
        // at a patch path, which DSH never reads.
        let converted;
        let root = if !root.is_array() && is_patch_path(path) {
            converted = legacy_to_patch(root)?;
            &converted
        } else {
            root
        };
        if !root.is_array() {
            return self.render(root, compact);
        }
        let providers = providers_of(root)
            .ok_or_else(|| format!("缺少 {PATCH_ENTRY_ID} 的 config.providers 节点"))?;
        let block = render_provider_block(providers);
        // The patch document belongs to DSH: splice the providers block into it so the header comment and the
        // other patch entries (ui / theme / permission …) come through exactly as they are.
        if let Some(content) = crate::util::read_config_content(path)
            .ok()
            .filter(|existing| !existing.trim().is_empty())
            .and_then(|existing| splice_providers(&existing, &block, file_line_ending(path)))
        {
            return Ok(content);
        }
        // No document yet, or the entry is missing from it: write a fresh one.
        Ok(format!("{PATCH_HEADER}{}", render_dsh_yaml(root)?))
    }
}
