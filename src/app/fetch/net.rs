use super::*;

/// 延迟测试用的 HTTP 客户端（较短超时，避免卡住 UI 线程池）。
///
/// **不要给它设置 `Proxy`**：ureq 默认不使用系统代理，探测请求始终直连，开着 Clash /
/// VPN 时也不会从代理出口发出（中转站的「多 IP 检测」看的正是出口 IP）。
/// 一旦在这里引入 `Proxy::try_from_env()`，探测就会改走代理口，务必保持直连。
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
/// 未列出的值按 OpenAI Chat Completions 兼容层处理，与 [`crate::convert::npm_to_api`] 的口径一致。
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
    /// 密钥已放进 URL 查询参数（Google 系），不再加鉴权头。
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
        // URL 里可能已经带了查询参数（Google 流式端点的 `?alt=sse`），要用 `&` 接着拼
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{}{}key={}", url, separator, secret)
    } else {
        url.to_string()
    }
}

/// 探测请求的 User-Agent：按协议伪装成主流客户端。
///
/// 中转站普遍只放行白名单客户端：实测同一站点同一 key，`ureq/2.12.1`、不传 UA、
/// `pi/0.1.0` 一律返回 `401 unauthorized client detected`（有的站点直接卡住到超时），
/// 而 `claude-cli/*` 与 `opencode/*` 正常返回 200。版本号不被校验（`claude-cli/9.9.9`
/// 同样放行），但写成真实存在的版本更自然（版本号取自本机安装的客户端）。
pub(crate) fn probe_user_agent(wire: ApiWire) -> &'static str {
    match wire {
        // Anthropic 系中转站本来就是给 Claude Code 用的
        ApiWire::AnthropicMessages | ApiWire::PiMessages => "claude-cli/1.18.30 (external, cli)",
        // OpenAI 兼容 / Responses / Google 系用 opencode 的身份
        _ => "opencode/1.18.30",
    }
}

/// 三个远程测量入口共用的前置：协议支持性、URL 与密钥判空，然后解析 wire / auth。
///
/// `why` 拼在「缺少 baseURL / API Key」后面区分入口（模型列表接口加「，无法获取模型」）。
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
/// Google 的流式靠**换端点**区分，而不是请求体里的 `stream` 字段。
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
/// - **不校验答案**：目的只是让请求看起来像正常对话（规避中转站测活特征），
///   判定只看 HTTP 是否成功与往返耗时。
/// - token 上限给到 16：太小会让推理模型返回空内容甚至直接报错。
/// - 不设 `temperature`：部分推理模型只接受默认值，设了反而报错。
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
/// 只看内容类字段，不看 role / usage / 各种元事件：中转站常常在模型真正开始生成前
/// 先推一个 role 块或心跳块，把它当首字，测出来的就不是用户体感的「首字延迟」。
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
/// Anthropic 会先发一行 `event: message_stop` 再发 `data: {"type":"message_stop"}`，
/// 两种形态都要认（裸标记行没有引号，所以按子串匹配）。
/// 站点发完标记后未必立刻关连接，靠它提前收尾，避免一直读到读超时才结束。
pub(crate) fn is_stream_end(line: &str) -> bool {
    line.contains("[DONE]")
        || line.contains("message_stop")
        || line.contains("response.completed")
        || line.contains("response.incomplete")
        || line.contains("response.failed")
}

/// 读完流式响应，返回**首字延迟**（毫秒，从请求发出算起）。
///
/// - 边读边解析 SSE 的 `data:` 载荷，碰到第一个带内容的块立刻记下耗时；
/// - 记下之后**继续把流读完**（`[DONE]` / `message_stop` / EOF / 64 KB 上限）再关闭连接：
///   真实客户端不会拿到流就断，匆匆断开在中转站日志里反而像探测流量；
/// - 全程没有数据返回时返回 `None`（调用方按超时 / 协议不支持流式处理）；
/// - 有数据但认不出内容块（形态罕见）时退回「第一个 `data:` 包到达的时刻」。
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

/// 对单个模型发一个**流式**探测请求，测量**首字延迟**（毫秒）。
///
/// 走流式是为了不像脚本测活：主流客户端（Claude Code / opencode / pi 等）默认全部流式，
/// 同步请求在中转站日志里会显示成「类型：同步」，反而是少数派特征。
/// 解响应体只是为了找第一个内容块（测首字），**不校验答案**。
/// 端点、鉴权与请求体都按所选协议构造；失败仍会报出耗时，便于判断服务是否可达。
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
    // 与列表测延迟同一个 agent：连接 5s / 读 10s（见 latency_agent）。
    let agent = latency_agent();
    let started = std::time::Instant::now();
    // Accept 与主流 SDK 的流式口径一致；UA 伪装成白名单客户端（见 probe_user_agent）。
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

/// 调用 OpenAI 兼容 /models 接口获取模型 id 列表（后台线程内执行）。
pub(crate) fn fetch_models_remote(
    url: &str,
    secret: &str,
    api: &str,
) -> Result<Vec<String>, String> {
    let (wire, auth) = request_prelude(api, url.is_empty(), secret.is_empty(), "，无法获取模型")?;
    let target = with_query_key(url, auth, secret);
    // 模型列表可能来自很慢的中转站：连接 10s / 读 30s，比测延迟宽松得多。
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let request = authed_get(&agent, &target, wire, auth, secret);
    let response = request.call().map_err(|err| match err {
        ureq::Error::Status(code, resp) => {
            // 「HTTP 404 接口或模型不存在：Not Found」+ 换行给出处理建议。
            let mut msg = crate::http_status::label(code);
            let text = resp.status_text().trim();
            if !text.is_empty() {
                msg.push('：');
                msg.push_str(text);
            }
            if let Some(hint) = crate::http_status::hint(code) {
                msg.push('\n');
                msg.push_str(hint);
            }
            msg
        }
        ureq::Error::Transport(transport) => format!(
            "网络错误：{}",
            sanitize_network_error(&transport.to_string())
        ),
    })?;
    let text = response.into_string().map_err(|err| err.to_string())?;
    parse_models_response(&text)
}

/// 解析 /models 响应中的模型 id（兼容 OpenAI/Anthropic/Gemini 等格式）。
pub(crate) fn parse_models_response(text: &str) -> Result<Vec<String>, String> {
    let root: Value = serde_json::from_str(text).map_err(|err| {
        let snippet = text.chars().take(160).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })?;
    if let Some(error) = root.get("error") {
        let msg = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("未知错误");
        return Err(msg.to_string());
    }
    pub(crate) fn push_ids(item: &Value, seen: &mut HashSet<String>, ids: &mut Vec<String>) {
        let raw = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or("");
        let id = raw.trim().trim_start_matches("models/").to_string();
        if !id.is_empty() && seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    let mut ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(arr) = root.as_array() {
        for item in arr {
            push_ids(item, &mut seen, &mut ids);
        }
    }
    for key in ["data", "models"] {
        if let Some(arr) = root.get(key).and_then(Value::as_array) {
            for item in arr {
                push_ids(item, &mut seen, &mut ids);
            }
        }
    }
    Ok(ids)
}
