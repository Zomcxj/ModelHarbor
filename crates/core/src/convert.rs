use crate::model::{ModelRow, ProviderRow};
use crate::util::{bool_at, nested_list_str, num_at, str_at};
use serde_json::{Map, Value};
use std::collections::HashSet;

/// 判断 raw 是否为 opencode 方言（含 opencode 特征键）。
pub(crate) fn is_opencode_shaped_model(raw: &Value) -> bool {
    ["limit", "modalities", "options", "variants"]
        .iter()
        .any(|k| raw.get(k).is_some())
}

pub(crate) fn is_opencode_shaped_provider(raw: &Value) -> bool {
    raw.get("options").is_some() || raw.get("models").map(|m| m.is_object()).unwrap_or(false)
}

/// 判断 raw 是否为 pi / omp 方言（含 pi 特征键）。
pub(crate) fn is_dsh_shaped_model(raw: &Value) -> bool {
    raw.get("reasoningEfforts").is_some()
}

pub(crate) fn is_dsh_shaped_provider(raw: &Value) -> bool {
    raw.get("apiKeyEnv").is_some() || raw.get("baseURL").is_some()
}

/// 判断 raw 是否为 WorkBuddy 方言（扁平条目，含 `useCustomProtocol` 或 `url`）。
pub(crate) fn is_workbuddy_shaped(raw: &Value) -> bool {
    raw.get("useCustomProtocol").is_some() || raw.get("url").is_some()
}

/// `anthropic-messages` 协议的 base URL 归一化：保证末尾带 `/v1`（opencode 侧读入与写出共用）。
///
/// 非 `anthropic-messages` 或空串原样返回。
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

/// `anthropic-messages` 协议的 base URL 归一化：去掉末尾 `/v1`（读入与写出共用）。
///
/// 非 `anthropic-messages`、或末尾不带 `/v1` 时原样返回。
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

/// ZCode 端的 `baseUrl` 归一化：剥掉 ZCode 自己会补的端点路径后缀（读入与写出共用）。
///
/// 后缀表按 api 取值：`anthropic-messages` 剥 `/v1/messages`、`/messages`、`/v1`；
/// `openai-responses` 剥 `/responses`；其余剥 `/chat/completions`。循环剥到不再匹配为止。
pub fn zcode_normalize_base_url(api: &str, url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // 按 api 选择要剥的后缀，先长后短。
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

/// ZCode 的 `api.type` → 内部统一的 api（pi / omp / DSH 词表）。
///
/// `openai-chat-completions` 映射为 `openai-completions`，其余原样返回。
pub fn zcode_api_to_api(zcode_api: &str) -> String {
    match zcode_api.trim() {
        "openai-chat-completions" => "openai-completions".to_string(),
        other => other.to_string(),
    }
}

/// 内部统一的 api（pi / omp / DSH 词表）→ ZCode 的 `api.type`。
///
/// 只认三值；未知值与 `openai-completions` 都映射为 `openai-chat-completions`。
pub fn api_to_zcode_api(api: &str) -> String {
    match api.trim() {
        "openai-completions" | "openai-chat-completions" => "openai-chat-completions".to_string(),
        "anthropic-messages" => "anthropic-messages".to_string(),
        "openai-responses" => "openai-responses".to_string(),
        _ => "openai-chat-completions".to_string(),
    }
}

/// 输入模态列表（`text, image` 形式）→ 一组能力布尔。
///
/// 返回固定顺序的 5 项：`text` / `image` / `video` / `pdf` / `audio`，匹配时大小写不敏感。
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

/// 一组能力布尔 → 输入模态列表（`text, image` 形式，按固定顺序）。
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

/// ZCode 的模型 raw → 输入模态列表。
///
/// 模态布尔取自 `properties.inputFormat.supportsText/Image/Video/Pdf/Audio`；
/// 一个 `supports*` 键都没写时返回空串。
pub fn zcode_modalities_from_raw(raw: &Value) -> String {
    let props = raw.get("properties").and_then(|p| p.get("inputFormat"));
    let flag = |key: &str| {
        props
            .and_then(|p| p.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    // 未写任何 supports* 键时返回空串。
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

/// WorkBuddy 的模型 raw → 输入模态列表（由 `supportsImages` 推导）。
///
/// 没写 `supportsImages` 时返回空串（未声明）；写了则返回 `text` 加可选的 `image`。
pub fn workbuddy_modalities_from_raw(raw: &Value) -> String {
    match raw.get("supportsImages").and_then(Value::as_bool) {
        // 声明了 supportsImages：文本恒为 true。
        Some(images) => supports_to_modalities([("text", true), ("image", images)]),
        None => String::new(),
    }
}

/// ZCode 的模型 raw → 思考档位文本（逗号分隔）。
///
/// 档位存在 `optionSpecs.reasoningLevel.values`；未写该键时留空。
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

/// opencode 的 npm 包 → pi / omp 的 api（线上协议）。
///
/// `@ai-sdk/anthropic` → `anthropic-messages`、`@ai-sdk/google` → `google-generative-ai`、
/// `@ai-sdk/mistral` → `mistral-conversations`、`@ai-sdk/openai` → `openai-responses`；
/// 其余（含空值）→ `openai-completions`。
pub fn npm_to_api(npm: &str) -> String {
    match npm {
        "@ai-sdk/anthropic" => "anthropic-messages".to_string(),
        "@ai-sdk/google" => "google-generative-ai".to_string(),
        "@ai-sdk/mistral" => "mistral-conversations".to_string(),
        "@ai-sdk/openai" => "openai-responses".to_string(),
        _ => "openai-completions".to_string(),
    }
}

/// omp 官方 9 值（`google-gemini-cli` 为 omp 专属）。
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

/// pi KnownApi 10 值（`mistral-conversations` / `pi-messages` 为 pi 专属）。
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

/// ZCode 官方 3 值（`api.type`；Chat Completions 多一个 `chat`）。
pub const ZCODE_APIS: [&str; 3] = [
    "openai-chat-completions",
    "anthropic-messages",
    "openai-responses",
];

/// WorkBuddy 页面可选协议：用于界面选择，保存时落到 URL 后缀。
pub const WORKBUDDY_APIS: [&str; 3] = [
    "openai-completions",
    "anthropic-messages",
    "openai-responses",
];

/// QwenCode 页面可选协议：保存时落到 pid + `wireApi`。
pub const QWEN_APIS: [&str; 3] = [
    "openai-completions",
    "anthropic-messages",
    "openai-responses",
];

/// KimiCode 页面可选协议：`[providers.<name>].type` 的 6 个合法值。
///
/// 逐字存进 `ProviderRow::pi_api`，不做映射翻译。
pub const KIMI_APIS: [&str; 6] = [
    "openai",
    "kimi",
    "anthropic",
    "openai_responses",
    "google-genai",
    "vertexai",
];

/// pi / omp 的 api → opencode 的 npm 包。
///
/// `anthropic-messages` → `@ai-sdk/anthropic`、`google-generative-ai` → `@ai-sdk/google`、
/// `mistral-conversations` → `@ai-sdk/mistral`、
/// `openai-completions` → `@ai-sdk/openai-compatible`、`openai-responses` → `@ai-sdk/openai`；
/// 其余 api 返回空串。
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

/// 提取思考档位的“发送值”集合（逗号分隔）：
/// - omp 方言：thinking.effortMap 的值（无 effortMap 时用 efforts）
/// - pi 方言：thinkingLevelMap 的值
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
        // thinking 块无 efforts/effortMap 时回落到 thinkingLevelMap。
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
    // 除 `reasoning` 外，`thinking` / `thinkingLevelMap` / `reasoningEfforts` 也算支持思考。
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
        // `disabled` 是 WorkBuddy 专属字段，其他方言的模型恒为启用。
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
        raw: v.clone(),
        kimi_alias: String::new(),
    }
}

pub fn model_to_pi(m: &ModelRow) -> Value {
    // opencode / DSH 来源全新构造，其余以 raw 为基底保留扩展字段。
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_model(&m.raw) || is_dsh_shaped_model(&m.raw) {
            Map::new()
        } else {
            m.raw.as_object().cloned().unwrap_or_default()
        };
    // thinking 块由 thinkingLevelMap 表达，写出前移除。
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
        // 1) raw.thinkingLevelMap 的值集合与当前档位一致时原样保留（含非对称映射）。
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
        // 2) raw.thinking.effortMap 的值集合一致时作为 thinkingLevelMap。
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
    // 都不匹配 → 对称映射 {档位: 档位}
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
    // 读入时归一化：`anthropic-messages` 的 baseUrl 去掉末尾 `/v1`。
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
        // `compat` 缺省：`api` 非空且不是 `openai-completions`。
        .unwrap_or_else(|| !api.is_empty() && api != "openai-completions");
    // 两个键任一为 true 即为 true，缺省 false。
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
        // DSH 侧默认值。
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
        raw: v.clone(),
        pi_api: api.to_string(),
        qwen_pid: String::new(),
    };
    r
}

/// 把指定键按给定顺序排到对象最前，其余键保持原有相对顺序。
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

/// pi / omp provider 的字段顺序：baseUrl → apiKey → api → compat → models，其余保留。
const PROVIDER_FIELD_ORDER: &[&str] = &["baseUrl", "apiKey", "api", "compat", "models"];

pub fn provider_to_pi(p: &ProviderRow) -> Value {
    // opencode / DSH 来源全新构造，其余以 raw 为基底保留扩展字段。
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_provider(&p.raw) || is_dsh_shaped_provider(&p.raw) {
            Map::new()
        } else {
            p.raw.as_object().cloned().unwrap_or_default()
        };
    // compat 只增删 supportsDeveloperRole，其余键保留。
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
    // 同格式且未改动时不写该键；否则写出当前值。
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
    // 与目标现有 providers 合并：同名覆盖，目标独有项保留。
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

/// 保守合并：以 `target` 为基底，`source` 中存在的键覆盖对应值，`target` 独有的键保留。
/// 对象递归合并；数组与标量以 `source` 为准。
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
