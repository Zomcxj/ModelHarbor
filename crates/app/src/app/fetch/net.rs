use super::*;

/// HTTP client for latency tests (short timeout).
///
/// No `Proxy` configured: probe requests always connect directly, never through a system proxy.
pub(crate) fn latency_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_millis(LATENCY_TIMEOUT_MS))
        .build()
}

pub(crate) fn http_error(err: ureq::Error, elapsed: u64) -> String {
    match err {
        // A human reason and advice follow the status code (hover for the full text; cards show a short summary).
        ureq::Error::Status(code, _) => {
            format!("{}（{} ms）", crate::http_status::detail(code), elapsed)
        }
        ureq::Error::Transport(t) => {
            format!("网络错误：{}", sanitize_network_error(&t.to_string()))
        }
    }
}

/// The wire protocol's (api's) call shape: endpoint, auth, and minimal request body all differ.
/// Unlisted values are treated as the OpenAI Chat Completions compatibility layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApiWire {
    /// `openai-completions` / `mistral-conversations` / unknown values.
    ChatCompletions,
    /// `openai-responses` / `openai-codex-responses`.
    Responses,
    /// `azure-openai-responses` (auth header `api-key`).
    AzureResponses,
    /// `anthropic-messages`.
    AnthropicMessages,
    /// `google-generative-ai` (the key goes in the `?key=` query parameter).
    GoogleGenerativeAi,
    /// `google-vertex` (Bearer + the `publishers/google` path).
    GoogleVertex,
    /// `pi-messages`.
    PiMessages,
    /// Needs proprietary signing or a private gateway — latency can't be measured with a minimal request.
    Unsupported,
}

/// Request auth mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AuthKind {
    /// `Authorization: Bearer <key>`.
    Bearer,
    /// `x-api-key` + `anthropic-version`.
    AnthropicKey,
    /// Azure OpenAI's `api-key`.
    AzureKey,
    /// The key already sits in the URL query (Google family); no auth headers.
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
        // These two need proprietary signing / private gateways (SigV4, CloudCode) — a minimal request can't measure real availability.
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

/// Whether this protocol supports latency testing / model listing with a minimal request; gives the reason when not.
pub(crate) fn unsupported_reason(api: &str) -> Option<String> {
    match api_wire(api) {
        ApiWire::Unsupported => Some(format!(
            "协议 {} 需要专有鉴权（签名 / 私有网关），暂不支持自动测试",
            api.trim()
        )),
        _ => None,
    }
}

/// Add auth headers per the auth mode; `QueryKey`'s key is already in the URL.
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

/// Google-family protocols put the key in the query parameter; everything else returns as-is.
pub(crate) fn with_query_key(url: &str, auth: AuthKind, secret: &str) -> String {
    if auth == AuthKind::QueryKey && !secret.is_empty() {
        // The URL may already carry query parameters (Google's streaming endpoint's `?alt=sse`); join with `&`
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{}{}key={}", url, separator, secret)
    } else {
        url.to_string()
    }
}

/// Probe requests' User-Agent: masquerade as a mainstream client per protocol.
///
/// Relay stations only admit whitelisted clients and don't validate version numbers; values follow the
/// versions currently mainstream (opencode uses the actually-installed local version).
pub(crate) fn probe_user_agent(wire: ApiWire) -> &'static str {
    // Anthropic-family relays already use Claude Code's identity; the rest use opencode's.
    if matches!(wire, ApiWire::AnthropicMessages | ApiWire::PiMessages) {
        "claude-cli/2.0.28 (external, cli)"
    } else {
        "opencode/2.0.32"
    }
}

/// Dress a probe request in client identity headers: UA + extra headers.
///
/// Real Claude Code requests always carry `x-app: cli` (pure identification, no behavioral side effects),
/// so sending it looks more authentic. The `anthropic-beta` feature string is **not** imitated: stale /
/// mismatched beta declarations could actually change upstream behavior.
pub(crate) fn apply_client_headers(request: ureq::Request, wire: ApiWire) -> ureq::Request {
    let request = request.set("User-Agent", probe_user_agent(wire));
    if matches!(wire, ApiWire::AnthropicMessages | ApiWire::PiMessages) {
        request.set("X-App", "cli")
    } else {
        request
    }
}

/// Shared preamble of the three remote measurement entry points: protocol support, URL and key emptiness,
/// then wire / auth resolution.
///
/// `why` is appended after "missing baseURL / API Key" to distinguish entry points.
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

/// The measurement entry points' shared "authenticated GET" builder: UA masquerading as a whitelisted client + Accept: json.
pub(crate) fn authed_get(
    agent: &ureq::Agent,
    target: &str,
    wire: ApiWire,
    auth: AuthKind,
    secret: &str,
) -> ureq::Request {
    apply_auth(
        apply_client_headers(agent.get(target).set("Accept", "application/json"), wire),
        auth,
        secret,
    )
}

/// Measure the round-trip latency of a provider's model-list endpoint (milliseconds).
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

/// The minimal chat request's address: the path depends on the protocol (Google needs the model name in the path).
/// Google's reasoning actions: non-streaming `:generateContent`, streaming `:streamGenerateContent?alt=sse`.
///
/// Google distinguishes streaming by the endpoint, not a `stream` field in the body.
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

/// One probe's request body: a neutral short question from the question bank.
///
/// - Answers are never validated; the verdict only looks at HTTP success and round-trip time.
/// - Token cap 16: too small makes reasoning models return empty content or error.
/// - No `temperature`: some reasoning models only accept the default.
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

/// Whether an SSE payload is "the first byte" (content has genuinely started arriving).
///
/// Only content-bearing fields count; role / usage / assorted meta events don't.
pub(crate) fn chunk_has_content(wire: ApiWire, chunk: &Value) -> bool {
    match wire {
        ApiWire::ChatCompletions | ApiWire::Unsupported => {
            let delta = &chunk["choices"][0]["delta"];
            // Reasoning models emitting reasoning_content (the chain of thought) first also count as output starting
            non_empty_text(&delta["content"]) || non_empty_text(&delta["reasoning_content"])
        }
        ApiWire::Responses | ApiWire::AzureResponses => {
            // Shaped like {"type":"response.output_text.delta","delta":"Hi"}
            chunk["type"]
                .as_str()
                .is_some_and(|kind| kind.ends_with(".delta"))
                && non_empty_text(&chunk["delta"])
        }
        ApiWire::AnthropicMessages | ApiWire::PiMessages => {
            // Shaped like {"type":"content_block_delta","delta":{"text":"Hi"}}
            let delta = &chunk["delta"];
            non_empty_text(&delta["text"]) || non_empty_text(&delta["thinking"])
        }
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => chunk["candidates"][0]["content"]
            ["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| non_empty_text(&part["text"]))),
    }
}

/// Whether a field holds non-empty text (accepting the string / content-block array / nested object shapes).
pub(crate) fn non_empty_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(non_empty_text),
        Value::Object(fields) => fields.values().any(non_empty_text),
        _ => false,
    }
}

/// Stream-end markers: OpenAI-compatible `[DONE]`, Anthropic's `message_stop`, Responses' `response.completed`.
///
/// Anthropic sends an `event: message_stop` line before `data: {"type":"message_stop"}`; both shapes are
/// recognized (the bare marker line has no quotes, hence substring matching). It allows finishing early
/// instead of reading until the read timeout.
pub(crate) fn is_stream_end(line: &str) -> bool {
    line.contains("[DONE]")
        || line.contains("message_stop")
        || line.contains("response.completed")
        || line.contains("response.incomplete")
        || line.contains("response.failed")
}

/// Read a streaming response to the end, returning first-byte latency (ms, from request issue).
///
/// - Parses SSE `data:` payloads while reading, noting the elapsed time at the first content-bearing block;
/// - after noting it, keeps reading the stream to the end (`[DONE]` / `message_stop` / EOF / 64 KB cap) before closing;
/// - returns `None` when no data arrived at all;
/// - with data but no recognizable content block, falls back to "the moment the first `data:` packet arrived".
pub(crate) fn read_stream_ttft(
    reader: impl std::io::Read,
    wire: ApiWire,
    started: std::time::Instant,
) -> Option<u64> {
    use std::io::BufRead;
    /// The read cap: enough for a 16-token streaming response, while withstanding stations that never close the connection.
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
            // Read timeout / connection drop: keep the first byte already measured
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

/// Send one streaming probe request to a single model, measuring first-byte latency (ms).
///
/// The response body is parsed only to find the first content block (measuring first byte), answers never
/// validated. Endpoint, auth, and body follow the chosen protocol; failures still report elapsed time.
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
    // The same agent as the list-latency test: connect 5s / read 10s (see `latency_agent`).
    let agent = latency_agent();
    let started = std::time::Instant::now();
    // Accept matches mainstream SDKs' streaming accounting; the UA masquerades as a whitelisted client (see `probe_user_agent`).
    let result = apply_auth(
        apply_client_headers(
            agent
                .post(&url)
                .set("Accept", "text/event-stream")
                .set("Content-Type", "application/json"),
            wire,
        ),
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
