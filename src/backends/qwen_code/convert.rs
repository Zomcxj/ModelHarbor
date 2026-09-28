use super::*;

pub(crate) fn is_builtin_pid(pid: &str) -> bool {
    BUILTIN_PIDS.contains(&pid)
}

/// 内部协议 → (内置 pid, 该条目要写的 `wireApi`)。
///
/// 一律回落到**内置 pid**：官方最稳的写法，不需要额外维护映射。OpenAI 的两种协议共用
/// `openai` 这一个 pid，靠条目自己的 `wireApi` 区分。
pub(crate) fn pid_for_api(api: &str) -> (&'static str, &'static str) {
    match api.trim() {
        "openai-responses" => ("openai", WIRE_RESPONSES),
        "anthropic-messages" => ("anthropic", ""),
        "google-generative-ai" | "google-vertex" | "google-gemini-cli" => ("gemini", ""),
        _ => ("openai", WIRE_CHAT),
    }
}

/// 协议桶名（内置 pid 名，或 `providerProtocol` 的值）→ 内部协议。
///
/// `wireApi` **优先于桶名**（官方："explicit `wireApi` takes precedence over either
/// OpenAI protocol"）；非 OpenAI 系协议没有 `wireApi` 概念，写了反而是配置错误。
pub(crate) fn bucket_to_api(bucket: &str, wire: &str) -> String {
    match bucket {
        "openai" => {
            if wire == WIRE_RESPONSES {
                "openai-responses".to_string()
            } else {
                "openai-completions".to_string()
            }
        }
        // 历史写法：桶名直接叫 `openai-responses`（v0.23.3 起可读，写只用新格式）。
        "openai-responses" => "openai-responses".to_string(),
        "anthropic" => "anthropic-messages".to_string(),
        "gemini" | "vertex-ai" => "google-generative-ai".to_string(),
        "qwen-oauth" => "openai-completions".to_string(),
        // 未映射的自定义 pid：协议不可知。条目本身会被 Qwen Code 跳过，但界面仍要显示它，
        // 让用户看得见并去修好，所以这里给一个兜底协议而不是把条目藏起来。
        _ => "openai-completions".to_string(),
    }
}

/// 某个 pid 的协议桶名：内置 pid 就是它自己，自定义 pid 查 `providerProtocol`。
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

/// 条目 → ProviderRow（一条条目 = 一张卡片）。
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
    // 写回时要知道这条属于哪个 pid，见模块说明第 1 点。
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
    // 密钥在顶层 `env[<envKey>]`；`envKey` 本身也存下来（写回时要同步维护 `env`）。
    row.api_key_env = env_key.clone();
    row.original_api_key_env = env_key.clone();
    row.api_key = env
        .and_then(|m| m.get(&env_key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 条目没写 timeout 时留空，避免「没配过的字段」在下次保存时被填上一个默认值。
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

/// 条目 → ModelRow。一条条目只有一个模型，字段全在同一条目上。
pub(crate) fn model_from_entry(entry: &Value) -> ModelRow {
    let mut m = ModelRow::new();
    m.id = entry_id(entry).unwrap_or_default().to_string();
    m.name = entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&m.id)
        .to_string();
    // 逐个覆盖 `ModelRow::new()` 的默认值：条目没声明的字段一律留空，不能沿用
    // 「新建模型」的预填值——那会把默认的上下文/输出/档位当成用户配置写进文件。
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
    // QwenCode 没有「工具调用」与 `store` 字段：不建模，也就不会凭空写出这两个键。
    m.tool_call = false;
    m.store = false;
    m.variants = efforts_from_entry(entry).join(", ");
    m.original_variants = m.variants.clone();
    m.source_format = Some(ConfigFormat::QwenCode);
    m.raw = entry.clone();
    m
}

/// 条目的输入模态：`capabilities.vision` → `text[, image]`。
///
/// **未声明时返回空串**（不返回 `"text"`）：那会把「未声明」变成「只支持文本」，
/// 跨格式写出时给别的后端凭空补字段。与 WorkBuddy 同一口径。
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

/// 条目的思考档位：`capabilities.reasoning.efforts`。
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

/// 卡片 + 模型 → 条目里**由界面接管**的字段。
///
/// 以**旧条目**为基底（`old`），逐个覆盖界面接管的键；界面没接管的键原样留着——
/// 官方把 `generationConfig` 说成「完全替换」的原子层，但界面只接管其中三个子键，
/// 其余（`maxRetries` / `customHeaders` / `extra_body` / 其它采样参数）是用户自己的
/// 设置，必须保留，所以这里做**子键级**合并。
///
/// 界面清空的字段要**删掉**对应的键，而不是写空值：`envKey: ""` 会被当成一个真的
/// 空变量名，`wireApi` 写在非 OpenAI 系协议上是官方明确的配置错误。
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
    // `wireApi` 只对 OpenAI 系协议有意义，其它协议写了是配置错误。
    set_str(&mut obj, "wireApi", wire);

    if let Some(m) = m {
        // capabilities：只在界面能表达的两个子键上动手，其余（`agent` /
        // `supportsImageGeneration` / `reasoning.profile` …）从旧条目继承。
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
        // 勾选框就是「声明 / 不声明」这个键：勾上写、取消删。条目里本就没有时
        // 取消勾选是无操作，来回保存稳定。
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

        // generationConfig：同上，只动 `contextWindowSize` / `timeout` /
        // `samplingParams.max_tokens` 三个子键。
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

/// 条目要用的 `envKey` 变量名：界面填了就用界面值；只填了密钥没填变量名时，按
/// provider key 推一个（与 DSH 的 [`crate::credentials::default_env_name`] 同一约定）。
///
/// 后一种情况就是**跨格式复制**：opencode 系的密钥内联在条目里，复制到 Qwen 页时
/// `api_key` 有值而 `api_key_env` 为空——不推一个名字，`sync_env` 会跳过、条目上也不写
/// `envKey`，密钥被静默丢掉，CLI 报 "Missing credentials for modelProviders model …"。
///
/// 「界面明确清空过」要跟「从来没填过」区分开（与 `sync_provider_secrets` 对 DSH
/// 密钥的判据同形）：原来有变量名、现在空 = 用户清掉的，尊重清空，不推新名；
/// 本来就空（跨格式来的行、新建的行）才推导。变量名与密钥都空也返回空串。
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
