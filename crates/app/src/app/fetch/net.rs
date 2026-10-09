use super::*;

/// 延迟测试用的 HTTP 客户端（较短超时）。
///
/// 不设置 `Proxy`：探测请求始终直连，不走系统代理。
pub(crate) fn latency_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_millis(LATENCY_TIMEOUT_MS))
        .build()
}

pub(crate) fn http_error(err: ureq::Error, elapsed: u64) -> String {
    match err {
        // 状态码后面补人话原因与处理建议（悬停提示看全文，卡片上只显示短摘要）。
        ureq::Error::Status(code, _) => {
            format!("{}（{} ms）", crate::http_status::detail(code), elapsed)
        }
        ureq::Error::Transport(t) => {
            format!("网络错误：{}", sanitize_network_error(&t.to_string()))
        }
    }
}

/// 线上协议（api）的调用形状：端点、鉴权与最小请求体各不相同。
/// 未列出的值按 OpenAI Chat Completions 兼容层处理。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApiWire {
    /// `openai-completions` / `mistral-conversations` / 未知值。
    ChatCompletions,
    /// `openai-responses` / `openai-codex-responses`。
    Responses,
    /// `azure-openai-responses`（鉴权头为 `api-key`）。
    AzureResponses,
    /// `anthropic-messages`。
    AnthropicMessages,
    /// `google-generative-ai`（密钥走 `?key=` 查询参数）。
    GoogleGenerativeAi,
    /// `google-vertex`（Bearer + `publishers/google` 路径）。
    GoogleVertex,
    /// `pi-messages`。
    PiMessages,
    /// 需要专有签名或私有网关，无法用最小请求测延迟。
    Unsupported,
}

/// 请求鉴权方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AuthKind {
    /// `Authorization: Bearer <key>`。
    Bearer,
    /// `x-api-key` + `anthropic-version`。
    AnthropicKey,
    /// Azure OpenAI 的 `api-key`。
    AzureKey,
    /// 密钥已放进 URL 查询参数（Google 系），不加鉴权头。
    QueryKey,
}

pub(crate) fn api_wire(api: &str) -> ApiWire {
    match api.trim() {
        "anthropic-messages" => ApiWire::AnthropicMessages,
        "openai-responses" | "openai-codex-responses" => ApiWire::Responses,
        "azure-openai-responses" => ApiWire::AzureResponses,
        "google-generative-ai" => ApiWire::GoogleGenerativeAi,
        "google-vertex" => ApiWire::GoogleVertex,
        "pi-messages" => ApiWire::PiMessages,
        // 这两个需要专有签名 / 私有网关（SigV4、CloudCode），最小请求测不出真实可用性。
        "bedrock-converse-stream" | "google-gemini-cli" => ApiWire::Unsupported,
        _ => ApiWire::ChatCompletions,
    }
}

pub(crate) fn auth_kind(wire: ApiWire) -> AuthKind {
    match wire {
        ApiWire::AnthropicMessages => AuthKind::AnthropicKey,
        ApiWire::AzureResponses => AuthKind::AzureKey,
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => AuthKind::QueryKey,
        _ => AuthKind::Bearer,
    }
}

/// 该协议是否支持用最小请求测延迟 / 拉模型列表；不支持时给出原因。
pub(crate) fn unsupported_reason(api: &str) -> Option<String> {
    match api_wire(api) {
        ApiWire::Unsupported => Some(format!(
            "协议 {} 需要专有鉴权（签名 / 私有网关），暂不支持自动测试",
            api.trim()
        )),
        _ => None,
    }
}

/// 按鉴权方式给请求加鉴权头；`QueryKey` 的密钥已在 URL 里。
pub(crate) fn apply_auth(request: ureq::Request, auth: AuthKind, secret: &str) -> ureq::Request {
    match auth {
        AuthKind::Bearer => request.set("Authorization", &format!("Bearer {}", secret)),
        AuthKind::AnthropicKey => request
            .set("x-api-key", secret)
            .set("anthropic-version", "2023-06-01"),
        AuthKind::AzureKey => request.set("api-key", secret),
        AuthKind::QueryKey => request,
    }
}

/// Google 系把密钥放查询参数；其余协议原样返回。
pub(crate) fn with_query_key(url: &str, auth: AuthKind, secret: &str) -> String {
    if auth == AuthKind::QueryKey && !secret.is_empty() {
        // URL 里可能已经带了查询参数（Google 流式端点的 `?alt=sse`），用 `&` 接着拼
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{}{}key={}", url, separator, secret)
    } else {
        url.to_string()
    }
}

/// 探测请求的 User-Agent：按协议伪装成主流客户端。
///
/// 中转站只放行白名单客户端，版本号不被校验。
pub(crate) fn probe_user_agent(wire: ApiWire) -> &'static str {
    match wire {
        // Anthropic 系中转站本来就用 Claude Code 的身份
        ApiWire::AnthropicMessages | ApiWire::PiMessages => "claude-cli/1.18.30 (external, cli)",
        // 其余用 opencode 的身份
        _ => "opencode/1.18.30",
    }
}

/// 三个远程测量入口共用的前置：协议支持性、URL 与密钥判空，然后解析 wire / auth。
///
/// `why` 拼在「缺少 baseURL / API Key」后面区分入口。
pub(crate) fn request_prelude(
    api: &str,
    url_empty: bool,
    secret_empty: bool,
    why: &str,
) -> Result<(ApiWire, AuthKind), String> {
    if let Some(reason) = unsupported_reason(api) {
        return Err(reason);
    }
    if url_empty {
        return Err(format!("缺少 baseURL{why}"));
    }
    if secret_empty {
        return Err(format!("缺少 API Key{why}"));
    }
    let wire = api_wire(api);
    Ok((wire, auth_kind(wire)))
}

/// 测量入口共用的「已鉴权 GET」构造：UA 伪装成白名单客户端 + Accept: json。
pub(crate) fn authed_get(
    agent: &ureq::Agent,
    target: &str,
    wire: ApiWire,
    auth: AuthKind,
    secret: &str,
) -> ureq::Request {
    apply_auth(
        agent
            .get(target)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "application/json"),
        auth,
        secret,
    )
}

/// 测量 provider 模型列表接口的往返延迟（毫秒）。
pub(crate) fn measure_provider_latency(url: &str, secret: &str, api: &str) -> Result<u64, String> {
    let (wire, auth) = request_prelude(api, url.is_empty(), secret.is_empty(), "")?;
    let target = with_query_key(url, auth, secret);
    let request = authed_get(&latency_agent(), &target, wire, auth, secret);
    let started = std::time::Instant::now();
    let result = request.call();
    let elapsed = started.elapsed().as_millis() as u64;
    if elapsed >= LATENCY_TIMEOUT_MS {
        return Err(format!("超时（{} ms）", elapsed));
    }
    match result {
        Ok(_) => Ok(elapsed),
        Err(err) => Err(http_error(err, elapsed)),
    }
}

/// 最小对话请求的地址：按协议决定路径（Google 系需要模型名参与路径）。
/// Google 系的推理动作：非流式 `:generateContent`，流式 `:streamGenerateContent?alt=sse`。
///
/// Google 的流式靠换端点区分，而非请求体里的 `stream` 字段。
pub(crate) fn google_action(stream: bool) -> &'static str {
    if stream {
        ":streamGenerateContent?alt=sse"
    } else {
        ":generateContent"
    }
}

pub(crate) fn chat_url(base_url: &str, api: &str, model: &str, stream: bool) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let model = model.trim();
    match api_wire(api) {
        ApiWire::AnthropicMessages => {
            if base.ends_with("/v1") {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            }
        }
        ApiWire::Responses | ApiWire::AzureResponses => format!("{}/responses", base),
        ApiWire::GoogleGenerativeAi => {
            format!("{}/models/{}{}", base, model, google_action(stream))
        }
        ApiWire::GoogleVertex => format!(
            "{}/publishers/google/models/{}{}",
            base,
            model,
            google_action(stream)
        ),
        ApiWire::PiMessages => format!("{}/messages", base),
        ApiWire::ChatCompletions | ApiWire::Unsupported => format!("{}/chat/completions", base),
    }
}

/// 单次探测的请求体：内容是题库里的中性短问句。
///
/// - 不校验答案，判定只看 HTTP 是否成功与往返耗时。
/// - token 上限 16，过小会让推理模型返回空内容或报错。
/// - 不设 `temperature`，部分推理模型只接受默认值。
pub(crate) fn minimal_body(wire: ApiWire, model: &str, question: &str) -> Value {
    match wire {
        ApiWire::Responses | ApiWire::AzureResponses => serde_json::json!({
            "model": model,
            "max_output_tokens": 16,
            "input": question,
            "stream": true
        }),
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => serde_json::json!({
            "contents": [{ "role": "user", "parts": [{ "text": question }] }],
            "generationConfig": { "maxOutputTokens": 16 }
        }),
        ApiWire::AnthropicMessages | ApiWire::PiMessages => serde_json::json!({
            "model": model,
            "max_tokens": 16,
            "stream": true,
            "messages": [{ "role": "user", "content": question }]
        }),
        ApiWire::ChatCompletions | ApiWire::Unsupported => serde_json::json!({
            "model": model,
            "max_tokens": 16,
            "stream": true,
            "messages": [{ "role": "user", "content": question }]
        }),
    }
}

/// 判断一个 SSE 载荷是不是「第一个字」（即真的开始出内容了）。
///
/// 只看内容类字段，不看 role / usage / 各种元事件。
pub(crate) fn chunk_has_content(wire: ApiWire, chunk: &Value) -> bool {
    match wire {
        ApiWire::ChatCompletions | ApiWire::Unsupported => {
            let delta = &chunk["choices"][0]["delta"];
            // 推理模型先出 reasoning_content（思维链）也算已经出字
            non_empty_text(&delta["content"]) || non_empty_text(&delta["reasoning_content"])
        }
        ApiWire::Responses | ApiWire::AzureResponses => {
            // 形如 {"type":"response.output_text.delta","delta":"你"}
            chunk["type"]
                .as_str()
                .is_some_and(|kind| kind.ends_with(".delta"))
                && non_empty_text(&chunk["delta"])
        }
        ApiWire::AnthropicMessages | ApiWire::PiMessages => {
            // 形如 {"type":"content_block_delta","delta":{"text":"你"}}
            let delta = &chunk["delta"];
            non_empty_text(&delta["text"]) || non_empty_text(&delta["thinking"])
        }
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => chunk["candidates"][0]["content"]
            ["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| non_empty_text(&part["text"]))),
    }
}

/// 字段是否含有非空文本（兼容「字符串 / 内容块数组 / 嵌套对象」三种形态）。
pub(crate) fn non_empty_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(non_empty_text),
        Value::Object(fields) => fields.values().any(non_empty_text),
        _ => false,
    }
}

/// 流结束标记：OpenAI 兼容的 `[DONE]`、Anthropic 的 `message_stop`、Responses 的 `response.completed`。
///
/// Anthropic 会先发一行 `event: message_stop` 再发 `data: {"type":"message_stop"}`，两种形态都认
/// （裸标记行没有引号，所以按子串匹配）。靠它提前收尾，不必读到读超时。
pub(crate) fn is_stream_end(line: &str) -> bool {
    line.contains("[DONE]")
        || line.contains("message_stop")
        || line.contains("response.completed")
        || line.contains("response.incomplete")
        || line.contains("response.failed")
}

/// 读完流式响应，返回首字延迟（毫秒，从请求发出算起）。
///
/// - 边读边解析 SSE 的 `data:` 载荷，碰到第一个带内容的块记下耗时；
/// - 记下后继续把流读完（`[DONE]` / `message_stop` / EOF / 64 KB 上限）再关闭连接；
/// - 全程没有数据时返回 `None`；
/// - 有数据但认不出内容块时退回「第一个 `data:` 包到达的时刻」。
pub(crate) fn read_stream_ttft(
    reader: impl std::io::Read,
    wire: ApiWire,
    started: std::time::Instant,
) -> Option<u64> {
    use std::io::BufRead;
    /// 读取上限：足够装下 16 token 的流式响应，又能顶住发完不关连接的站点。
    pub(crate) const MAX_STREAM_BYTES: usize = 64 * 1024;
    let mut buffer = std::io::BufReader::new(reader);
    let mut line = String::new();
    let mut read_bytes = 0usize;
    let mut first_data: Option<u64> = None;
    let mut ttft: Option<u64> = None;
    loop {
        line.clear();
        match buffer.read_line(&mut line) {
            Ok(0) => break,
            Ok(read) => read_bytes += read,
            // 读超时 / 连接中断：保留已经测到的首字
            Err(_) => break,
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if is_stream_end(trimmed) {
            break;
        }
        if let Some(payload) = trimmed.strip_prefix("data:") {
            if let Ok(chunk) = serde_json::from_str::<Value>(payload.trim()) {
                if first_data.is_none() {
                    first_data = Some(started.elapsed().as_millis() as u64);
                }
                if ttft.is_none() && chunk_has_content(wire, &chunk) {
                    ttft = Some(started.elapsed().as_millis() as u64);
                }
            }
        }
        if read_bytes >= MAX_STREAM_BYTES {
            break;
        }
    }
    ttft.or(first_data)
}

/// 对单个模型发一个流式探测请求，测量首字延迟（毫秒）。
///
/// 解响应体只为找第一个内容块（测首字），不校验答案。端点、鉴权与请求体按所选协议构造；
/// 失败仍会报出耗时。
pub(crate) fn measure_model_latency(
    base_url: &str,
    secret: &str,
    api: &str,
    model: &str,
    question: &str,
) -> Result<u64, String> {
    let (wire, auth) = request_prelude(api, base_url.trim().is_empty(), secret.is_empty(), "")?;
    let url = with_query_key(&chat_url(base_url, api, model, true), auth, secret);
    let body = minimal_body(wire, model, question).to_string();
    // 与列表测延迟同一个 agent：连接 5s / 读 10s（见 `latency_agent`）。
    let agent = latency_agent();
    let started = std::time::Instant::now();
    // Accept 与主流 SDK 的流式口径一致；UA 伪装成白名单客户端（见 `probe_user_agent`）。
    let result = apply_auth(
        agent
            .post(&url)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "text/event-stream")
            .set("Content-Type", "application/json"),
        auth,
        secret,
    )
    .send_string(&body);
    let response = match result {
        Ok(response) => response,
        Err(err) => return Err(http_error(err, started.elapsed().as_millis() as u64)),
    };
    match read_stream_ttft(response.into_reader(), wire, started) {
        Some(ms) if ms < LATENCY_TIMEOUT_MS => Ok(ms),
        Some(ms) => Err(format!("超时（{} ms）", ms)),
        None => {
            let waited = started.elapsed().as_millis() as u64;
            Err(if waited >= LATENCY_TIMEOUT_MS {
                format!("超时（{} ms）", waited)
            } else {
                "流式响应没有数据".to_string()
            })
        }
    }
}
