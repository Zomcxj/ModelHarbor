use crate::model::{ModelRow, ProviderRow};
use crate::util::{bool_at, nested_list_str, num_at, str_at};
use serde_json::{Map, Value};
use std::collections::HashSet;

/// 判断 raw 是否为 opencode 方言（含 opencode 特征键）。
/// pi / omp 输出时以此为界：opencode 形状全新构造，其余以 raw 为基底保留扩展字段。
pub(crate) fn is_opencode_shaped_model(raw: &Value) -> bool {
    ["limit", "modalities", "options", "variants"]
        .iter()
        .any(|k| raw.get(k).is_some())
}

pub(crate) fn is_opencode_shaped_provider(raw: &Value) -> bool {
    raw.get("options").is_some() || raw.get("models").map(|m| m.is_object()).unwrap_or(false)
}

/// 判断 raw 是否为 pi / omp 方言（含 pi 特征键）。
/// opencode 输出时以此为界：pi 形状全新构造，防止方言键泄漏。
pub(crate) fn is_dsh_shaped_model(raw: &Value) -> bool {
    raw.get("reasoningEfforts").is_some()
}

pub(crate) fn is_dsh_shaped_provider(raw: &Value) -> bool {
    raw.get("apiKeyEnv").is_some() || raw.get("baseURL").is_some()
}

/// 判断 raw 是否为 WorkBuddy 方言（扁平条目，含 `useCustomProtocol` 或 `url`）。
/// ZCode 输出时以此为界：WorkBuddy 形状全新构造，防止它的 id / vendor / url
/// 被当成 ZCode 的扩展字段写进 `config`。
pub(crate) fn is_workbuddy_shaped(raw: &Value) -> bool {
    raw.get("useCustomProtocol").is_some() || raw.get("url").is_some()
}

/// `anthropic-messages` 协议的 base URL 归一化：**保证末尾带 `/v1`**（opencode 侧读入与写出共用）。
///
/// opencode 的 `@ai-sdk/anthropic` 客户端只往 baseURL 追加 `/messages`，所以 baseURL 必须
/// 包含 `/v1`（官方默认值就是 `https://api.anthropic.com/v1`）。pi / oh-my-pi / DSH 相反，
/// 见 [`without_v1_for_messages`]。
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
/// pi / omp / dsh 都把这个值原样交给 Anthropic SDK 风格的客户端，由客户端自行拼接
/// `/v1/messages`（字符串拼接），所以 base 里再带 `/v1` 会请求成 `/v1/v1/messages`。
/// opencode（`@ai-sdk/anthropic`）则相反：它的 baseURL 必须包含 `/v1`（客户端只追加
/// `/messages`），因此**不做**归一化。
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

/// ZCode 端的 `baseUrl` 归一化：剥掉 ZCode 自己会补的那段端点路径（读入与写出共用）。
///
/// ZCode 请求时按 kind 先剥后缀、再拼回同一后缀
/// （`normalizeModelProviderBaseUrlForKind` + `joinBaseUrlAndPath`）：
/// `anthropic` 拼 `/v1/messages`、`openai` 拼 `/responses`、
/// `openai-compatible` 拼 `/chat/completions`。它剥的只有**完整端点后缀**，
/// 光秃秃的 `/v1` 不在其列——所以 `baseUrl` 以 `/v1` 结尾时会被拼成
/// `/v1/v1/messages`，服务端直接拒（ZCode 里报 `Provider rejected the model request`）。
/// 这与 pi / omp / dsh 的约定一致（见 [`without_v1_for_messages`]）；opencode 相反，
/// 它的 `@ai-sdk/anthropic` 只追加 `/messages`，baseURL 必须自带 `/v1`。
///
/// 读入与写出都走这里，界面显示的就是 ZCode 真正当基址用的值，
/// 顺带自愈历史上已写坏的 `/v1/…` 与整段端点路径。
pub fn zcode_normalize_base_url(api: &str, url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // 先长后短，与 ZCode 自己的后缀表同源。`openai-compatible` 反而不剥 `/v1`：
    // 它拼的是 `/chat/completions`，`https://host/v1` 才是正确基址。
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
/// ZCode 只有三值，且 Chat Completions 叫 `openai-chat-completions`（多一个 `chat`），
/// 与 pi 系的 `openai-completions` 是同一个线上协议——不转换就会把对方不认的字符串
/// 写进配置。其余两值与 pi 系同名，原样透传。
pub fn zcode_api_to_api(zcode_api: &str) -> String {
    match zcode_api.trim() {
        "openai-chat-completions" => "openai-completions".to_string(),
        other => other.to_string(),
    }
}

/// 内部统一的 api（pi / omp / DSH 词表）→ ZCode 的 `api.type`。
///
/// ZCode 只认三值；其余协议没有对应值，回落到 Chat Completions
/// （ZCode 的默认协议，也是它 `openai-compatible` kind 的含义）。
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
/// WorkBuddy 用 `supportsImages` 这类布尔表达模态，ZCode 用
/// `properties.supportsImage/Video/Pdf/Audio/Text`；两边都由本函数从同一个
/// 逗号分隔列表推导，保证跨格式转换时语义一致。
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
/// 模态布尔嵌在 `properties.inputFormat`（与内置模型库一致：`inputFormat` /
/// `outputFormat` 两个子块），**不是** `properties` 的直接子键。
pub fn zcode_modalities_from_raw(raw: &Value) -> String {
    let props = raw.get("properties").and_then(|p| p.get("inputFormat"));
    let flag = |key: &str| {
        props
            .and_then(|p| p.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    // 未写任何 supports* 键时留空，避免把「没写」误判成「只支持 text」。
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
/// 条目**没写** `supportsImages` 时返回空串，表示「未声明」——不能返回 `"text"`：
/// 那会把「未声明」当成「只支持文本」，跨格式写出时给 ZCode 凭空补一个
/// `inputFormat.supportsText: true`（用户的正常配置里就是这么多出来的）。
pub fn workbuddy_modalities_from_raw(raw: &Value) -> String {
    match raw.get("supportsImages").and_then(Value::as_bool) {
        // 声明了 supportsImages：文本必然支持（WorkBuddy 的模型都是文本模型）。
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

/// opencode 的 npm 包 → pi / omp 的 api（线上协议）：
/// - `@ai-sdk/anthropic` / `@ai-sdk/google` / `@ai-sdk/mistral` 各自对应同名协议；
/// - `@ai-sdk/openai` 走 Responses（`/v1/responses`）；
/// - `@ai-sdk/openai-compatible` 与未写 npm（空值）一样，走 OpenAI 兼容层（Chat Completions）；
/// - 其他未知包按兼容层处理（不再把包名当 api 写出去）。
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
pub(crate) const OMP_APIS: [&str; 9] = [
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
pub(crate) const PI_APIS: [&str; 10] = [
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
pub(crate) const ZCODE_APIS: [&str; 3] = [
    "openai-chat-completions",
    "anthropic-messages",
    "openai-responses",
];

/// WorkBuddy 页面可选协议：文件里没有协议字段，协议由 URL 后缀 + 勾选框表达。
/// 这里列出的值用于界面选择，保存时落到 URL 后缀（见 `backends::workbuddy`）。
pub(crate) const WORKBUDDY_APIS: [&str; 3] = [
    "openai-completions",
    "anthropic-messages",
    "openai-responses",
];

/// QwenCode 页面可选协议：官方只有三个协议桶（`openai` / `anthropic` / `gemini`），
/// OpenAI 的两种 API 共用 `openai` 桶、靠条目自己的 `wireApi` 区分。
/// 保存时落到 pid + `wireApi`（见 `backends::qwen_code`）。
pub(crate) const QWEN_APIS: [&str; 3] = [
    "openai-completions",
    "anthropic-messages",
    "openai-responses",
];

/// KimiCode 页面可选协议：`[providers.<name>].type` 的 6 个合法值
/// （源码 `ProviderTypeSchema`，也是 `KNOWN_WIRE_TYPES`）。
///
/// 这 6 个值是**逐字**存进 `ProviderRow::pi_api` 的，不做任何翻译。Kimi 的 type 命名与
/// 本项目的内部协议名**并不重合**（`openai` vs `openai-completions`、
/// `openai_responses` vs `openai-responses`），硬套一张映射表会在保存时把用户的 `type`
/// 悄悄改写——`kimi` 更是 Kimi 自己的 wire 类型，映射到 `openai` 会把走 OAuth 的
/// `managed:kimi-code` 改成另一种协议。逐字存取则同格式往返**恒等无损**。
pub(crate) const KIMI_APIS: [&str; 6] = [
    "openai",
    "kimi",
    "anthropic",
    "openai_responses",
    "google-genai",
    "vertexai",
];

/// pi / omp 的 api → opencode 的 npm 包：
/// - `openai-completions` → `@ai-sdk/openai-compatible`（规范化写法；未写 npm 也是这个语义）；
/// - `openai-responses` → `@ai-sdk/openai`；
/// - 无对应包的 api（`openai-codex-responses` / `azure-openai-responses` /
///   `bedrock-converse-stream` / `google-gemini-cli` / `google-vertex` / `pi-messages` 等）
///   返回空串：写入 pi 时仍保留 pi 自己的 api 字段，不会被改写成兼容层。
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
        // thinking 块无 efforts/effortMap（如 budget 模式）→ 回落 thinkingLevelMap
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
    // pi/omp 用 reasoning 布尔表达是否支持思考；部分配置只写了 thinking 块 /
    // thinkingLevelMap（DSH 则为 reasoningEfforts），这些同样意味着支持思考，
    // 否则勾选状态在其他页面显示不出来。
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
    // opencode 来源全新构造；pi/omp 来源以 raw 为基底保留扩展字段（cost/toolName 等）
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_model(&m.raw) || is_dsh_shaped_model(&m.raw) {
            Map::new()
        } else {
            m.raw.as_object().cloned().unwrap_or_default()
        };
    // omp 方言的 thinking 块由 thinkingLevelMap 表达，翻译后移除
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
        // 1) pi 方言原样保留：raw.thinkingLevelMap 值集合与当前选择一致
        //    （保护 {"high":"max"} 这类非对称映射的键）
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
        // 2) omp 方言翻译：raw.thinking.effortMap 值集合一致 → 直接作为 thinkingLevelMap
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
    // pi / omp 的 messages 协议由客户端补 /v1/messages，配置里不带 /v1：
    // 读入时就归一化，界面显示的也是不带 /v1 的值，再写回目标文件保持一致。
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
        // api 缺省时 pi 默认 openai-completions（chat/completions）。
        .unwrap_or_else(|| !api.is_empty() && api != "openai-completions");
    // pi 与 omp 的对应字段相互映射；缺省（opencode/dsh 转换或文件未声明）时不勾选。
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
        // 跨格式保存到 DSH 时写出默认 timeoutMs。
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
        kimi_env_mode: false,
    };
    r
}

/// 把指定键按给定顺序排到对象最前，其余键保持原有相对顺序（稳定输出，避免
/// 新增键被追加到文件末尾造成字段位置不一致）。
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
    // opencode 来源全新构造；pi/omp 来源以 raw 为基底保留扩展字段（headers/auth 等）
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_provider(&p.raw) || is_dsh_shaped_provider(&p.raw) {
            Map::new()
        } else {
            p.raw.as_object().cloned().unwrap_or_default()
        };
    // compat 仅管理 supportsDeveloperRole，其余键（maxTokensField/extraBody/...）保留
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
    // requiresReasoningContentOnAssistantMessages（pi 键）：同格式未修改时
    // 保留 raw 原样；跨格式或用户改动时写出当前值（缺省打勾）。
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
    // 跨格式目标保存时以“目标现有内容为基底”做保守合并：同名 provider 覆盖、
    // 目标独有 provider 保留（非编辑内容不能被整文件替换删掉）。
    // 当前文件保存时 extras 不含 providers，等价于整体替换。
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

/// 保守合并：以 `target`（目标文件现有内容）为基底，`source`（UI 转换结果）
/// 中存在的键覆盖对应值；target 独有的键一律保留。对象递归合并，
/// 数组与标量在 source 有该键时以 source 为准。用于跨格式保存，
/// 保证「非编辑内容不能改」。
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
