use crate::app::bars::{sanitize_network_error, short_err};
use crate::app::fetch::chat_url;
use crate::app::providers_form::{fetch_grid_columns, FETCH_GRID_GAP_X};
use crate::app::App;

/// The one-line summary in cards / lists: keeps the human reason after the status code, drops elapsed time and advice.
#[test]
fn short_err_keeps_status_code_with_reason() {
    assert_eq!(
        short_err(
            "HTTP 503 服务临时不可用：上游繁忙、维护或过载；稍后重试，或切到备用线路（1234 ms）"
        ),
        "HTTP 503 服务临时不可用"
    );
    // The multi-line error of a failed model fetch: take only the first line's status code + reason
    assert_eq!(
            short_err("HTTP 404 接口或模型不存在：Not Found\nBase URL 路径或模型名写错；检查接口地址与模型 id"),
            "HTTP 404 接口或模型不存在"
        );
    // Unlisted status codes: number only.
    assert_eq!(short_err("HTTP 599（200 ms）"), "HTTP 599");
    // Non-HTTP errors: short text as-is (truncated only when over-long)
    assert_eq!(
        short_err("网络错误：Connection refused"),
        "网络错误：Connection refused"
    );
    assert_eq!(short_err("  "), "");
}

#[test]
fn fetch_grid_columns_never_exceed_available_width() {
    // Total width = columns * column width + gaps * (columns - 1), never exceeding the available width.
    let total =
        |(cols, col_w): (usize, f32)| cols as f32 * col_w + FETCH_GRID_GAP_X * (cols - 1) as f32;
    // Wide window: the full 5 columns with widths split evenly (5 * 217.6 + 4 * 28 = 1200).
    let wide = fetch_grid_columns(50, 1200.0);
    assert_eq!(wide.0, 5);
    assert!((wide.1 - 217.6).abs() < 0.01);
    assert!(total(wide) <= 1200.5);
    // Medium width: column count drops with available width, still exactly filling.
    let mid = fetch_grid_columns(50, 400.0);
    assert_eq!(mid.0, 2);
    assert!(total(mid) <= 400.5);
    // Extremely narrow window: degrades to one column.
    let narrow = fetch_grid_columns(50, 120.0);
    assert_eq!(narrow.0, 1);
    assert!(narrow.1 <= 120.0);
    // Few models leave no surplus columns.
    assert_eq!(fetch_grid_columns(3, 1200.0).0, 3);
    // An empty list neither panics nor returns 0 columns.
    assert_eq!(fetch_grid_columns(0, 300.0).0, 1);
}

#[test]
fn models_url_openai_variants() {
    assert_eq!(
        App::models_url("https://api.openai.com/v1", "openai-completions"),
        "https://api.openai.com/v1/models"
    );
    assert_eq!(
        App::models_url("https://api.openai.com/v1/", "openai-completions"),
        "https://api.openai.com/v1/models"
    );
    assert_eq!(
        App::models_url("https://gw.example.com", "openai-completions"),
        "https://gw.example.com/models"
    );
}

#[test]
fn models_url_anthropic_uses_v1() {
    assert_eq!(
        App::models_url("https://api.anthropic.com", "anthropic-messages"),
        "https://api.anthropic.com/v1/models"
    );
    assert_eq!(
        App::models_url("https://api.anthropic.com/v1", "anthropic-messages"),
        "https://api.anthropic.com/v1/models"
    );
}

#[test]
fn chat_url_openai_and_anthropic() {
    assert_eq!(
        chat_url(
            "https://api.openai.com/v1",
            "openai-completions",
            "gpt-4o",
            true
        ),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        chat_url(
            "https://api.anthropic.com",
            "anthropic-messages",
            "claude-sonnet-4",
            true
        ),
        "https://api.anthropic.com/v1/messages"
    );
}

#[test]
fn chat_url_streaming_only_changes_google() {
    // Except Google's family, streaming and non-streaming share one endpoint (distinguished by the body's stream field)
    assert_eq!(
        chat_url(
            "https://gw.example.com/v1",
            "openai-completions",
            "gpt-4o",
            false
        ),
        chat_url(
            "https://gw.example.com/v1",
            "openai-completions",
            "gpt-4o",
            true
        )
    );
    // Google's family swaps the verb (streamGenerateContent) and adds the SSE query parameter
    assert_eq!(
            chat_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai",
                "gemini-2.5-pro",
                true
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
    assert_eq!(
            chat_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex",
                "gemini-2.5-pro",
                true
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
}

#[test]
fn query_key_appends_after_existing_query() {
    use crate::app::fetch::{with_query_key, AuthKind};
    assert_eq!(
        with_query_key("https://gw.example.com/models", AuthKind::QueryKey, "sk-1"),
        "https://gw.example.com/models?key=sk-1"
    );
    // Google's streaming endpoint already carries ?alt=sse; the key joins with &
    assert_eq!(
        with_query_key(
            "https://gw.example.com/models/m:streamGenerateContent?alt=sse",
            AuthKind::QueryKey,
            "sk-1"
        ),
        "https://gw.example.com/models/m:streamGenerateContent?alt=sse&key=sk-1"
    );
}

#[test]
fn chat_url_follows_selected_protocol() {
    let base = "https://gw.example.com/v1";
    // The Responses family goes through /responses (Azure included)
    assert_eq!(
        chat_url(base, "openai-responses", "gpt-4o", false),
        "https://gw.example.com/v1/responses"
    );
    assert_eq!(
        chat_url(base, "azure-openai-responses", "gpt-4o", false),
        "https://gw.example.com/v1/responses"
    );
    // Mistral's conversation protocol is still chat/completions
    assert_eq!(
        chat_url(base, "mistral-conversations", "mistral-large", false),
        "https://gw.example.com/v1/chat/completions"
    );
    // pi's own protocol is /messages (used directly on the base even without a v1 prefix)
    assert_eq!(
        chat_url(base, "pi-messages", "some-model", false),
        "https://gw.example.com/v1/messages"
    );
    // Google's family needs the model name in the path
    assert_eq!(
        chat_url(
            "https://generativelanguage.googleapis.com/v1beta",
            "google-generative-ai",
            "gemini-2.5-pro",
            false
        ),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent"
    );
    assert_eq!(
            chat_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex",
                "gemini-2.5-pro",
                false
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:generateContent"
        );
}

#[test]
fn api_wire_classifies_and_flags_unsupported() {
    use crate::app::fetch::{api_wire, auth_kind, unsupported_reason, ApiWire, AuthKind};
    // Unknown and empty values both land on the compatibility layer.
    assert_eq!(api_wire("openai-completions"), ApiWire::ChatCompletions);
    assert_eq!(api_wire("unknown-api"), ApiWire::ChatCompletions);
    assert_eq!(api_wire(""), ApiWire::ChatCompletions);
    assert_eq!(api_wire("openai-codex-responses"), ApiWire::Responses);
    assert_eq!(api_wire("anthropic-messages"), ApiWire::AnthropicMessages);
    assert_eq!(api_wire("pi-messages"), ApiWire::PiMessages);
    // Auth mode follows the protocol
    assert_eq!(
        auth_kind(api_wire("anthropic-messages")),
        AuthKind::AnthropicKey
    );
    assert_eq!(
        auth_kind(api_wire("azure-openai-responses")),
        AuthKind::AzureKey
    );
    assert_eq!(
        auth_kind(api_wire("google-generative-ai")),
        AuthKind::QueryKey
    );
    assert_eq!(auth_kind(api_wire("openai-responses")), AuthKind::Bearer);
    // Protocols needing proprietary auth error early.
    assert!(unsupported_reason("google-gemini-cli").is_some());
    assert!(unsupported_reason("bedrock-converse-stream").is_some());
    assert!(unsupported_reason("openai-completions").is_none());
}

#[test]
fn minimal_body_matches_protocol() {
    use crate::app::fetch::{api_wire, minimal_body};
    let question = "世界上最长的河流是哪条？只回答河名";
    let chat = minimal_body(api_wire("openai-completions"), "m", question);
    assert_eq!(chat["messages"][0]["content"], question);
    assert_eq!(chat["max_tokens"], 16);
    let resp = minimal_body(api_wire("openai-responses"), "m", question);
    assert_eq!(resp["input"], question);
    assert_eq!(resp["max_output_tokens"], 16);
    let anthropic = minimal_body(api_wire("anthropic-messages"), "m", question);
    assert_eq!(anthropic["messages"][0]["content"], question);
    let google = minimal_body(api_wire("google-generative-ai"), "m", question);
    assert_eq!(google["contents"][0]["parts"][0]["text"], question);
    assert_eq!(google["generationConfig"]["maxOutputTokens"], 16);
    // Everything streams (Google excepted: its streaming swaps endpoints, no stream field in the body)
    assert_eq!(chat["stream"], true);
    assert_eq!(resp["stream"], true);
    assert_eq!(anthropic["stream"], true);
    assert!(google.get("stream").is_none());
    // Field names must not cross over (Responses has no messages, Google has no model)
    assert!(resp.get("messages").is_none());
    assert!(google.get("model").is_none());
}

#[test]
fn probe_user_agent_matches_whitelisted_clients() {
    use crate::app::fetch::{api_wire, probe_user_agent};
    // Anthropic-family uses Claude Code's identity; the rest use opencode's.
    assert!(probe_user_agent(api_wire("anthropic-messages")).starts_with("claude-cli/"));
    assert!(probe_user_agent(api_wire("pi-messages")).starts_with("claude-cli/"));
    assert!(probe_user_agent(api_wire("openai-completions")).starts_with("opencode/"));
    assert!(probe_user_agent(api_wire("openai-responses")).starts_with("opencode/"));
    assert!(probe_user_agent(api_wire("google-generative-ai")).starts_with("opencode/"));
    // Versions follow the mainstream current release: must not stay at the old 1.18.
    assert!(!probe_user_agent(api_wire("openai-completions")).contains("1.18"));
    assert!(!probe_user_agent(api_wire("anthropic-messages")).contains("1.18"));
}

#[test]
fn chunk_content_detection_per_protocol() {
    use crate::app::fetch::{api_wire, chunk_has_content};
    let parse = |text: &str| match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => value,
        Err(err) => panic!("测试用例不是合法 JSON：{err}"),
    };
    // Chat Completions: role blocks don't count as output; content / reasoning_content do
    let chat = api_wire("openai-completions");
    assert!(!chunk_has_content(
        chat,
        &parse(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#)
    ));
    assert!(chunk_has_content(
        chat,
        &parse(r#"{"choices":[{"delta":{"content":"尼罗河"}}]}"#)
    ));
    assert!(chunk_has_content(
        chat,
        &parse(r#"{"choices":[{"delta":{"reasoning_content":"嗯"}}]}"#)
    ));
    // The content-block-array shape counts too (some relays reply with parts)
    assert!(chunk_has_content(
        chat,
        &parse(r#"{"choices":[{"delta":{"content":[{"text":"你"}]}}]}"#)
    ));
    // Responses: only *.delta events carrying delta text count
    let responses = api_wire("openai-responses");
    assert!(!chunk_has_content(
        responses,
        &parse(r#"{"type":"response.created"}"#)
    ));
    assert!(chunk_has_content(
        responses,
        &parse(r#"{"type":"response.output_text.delta","delta":"你"}"#)
    ));
    // Anthropic: text / thinking inside content_block_delta
    let anthropic = api_wire("anthropic-messages");
    assert!(!chunk_has_content(
        anthropic,
        &parse(r#"{"type":"message_start","message":{"id":"x"}}"#)
    ));
    assert!(chunk_has_content(
        anthropic,
        &parse(r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"你"}}"#)
    ));
    // Google: candidates[0].content.parts[*].text
    let google = api_wire("google-generative-ai");
    assert!(!chunk_has_content(
        google,
        &parse(r#"{"candidates":[{"content":{"parts":[{"text":""}]}}]}"#)
    ));
    assert!(chunk_has_content(
        google,
        &parse(r#"{"candidates":[{"content":{"parts":[{"text":"尼罗河"}]}}]}"#)
    ));
}

#[test]
fn stream_ttft_reads_first_content_and_drains() {
    use crate::app::fetch::{api_wire, read_stream_ttft};
    let started = std::time::Instant::now();
    // A role block first: the first byte must land on a content block, and reading the stream mustn't error
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"尼罗河\"}}]}\n\n",
        "data: [DONE]\n\n"
    );
    let ttft = read_stream_ttft(
        std::io::Cursor::new(sse.as_bytes()),
        api_wire("openai-completions"),
        started,
    );
    assert!(matches!(ttft, Some(ms) if ms < 1_000));
    // Meta events only (no content at all): fall back to the first packet's moment, never None
    let meta = "data: {\"type\":\"response.created\"}\n\n";
    let fallback = read_stream_ttft(
        std::io::Cursor::new(meta.as_bytes()),
        api_wire("openai-responses"),
        started,
    );
    assert!(fallback.is_some());
    // An empty stream (not even packets) counts as failure
    let empty: &[u8] = b"";
    assert!(read_stream_ttft(
        std::io::Cursor::new(empty),
        api_wire("openai-completions"),
        started
    )
    .is_none());
}

#[test]
fn stream_end_markers_are_recognized() {
    use crate::app::fetch::is_stream_end;
    assert!(is_stream_end("data: [DONE]"));
    assert!(is_stream_end("event: message_stop"));
    assert!(is_stream_end("data: {\"type\":\"response.completed\"}"));
    // Ordinary content blocks must not be misjudged as the end
    assert!(!is_stream_end(
        "data: {\"choices\":[{\"delta\":{\"content\":\"尼罗河\"}}]}"
    ));
    // content_block_stop contains stop but isn't the end event
    assert!(!is_stream_end(
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\"}"
    ));
}

#[test]
fn probe_gate_throttles_per_provider() {
    use crate::app::fetch::{ProbeGate, ProbeGateState, PROBE_PROVIDER_GAP_S};
    let mut gate = ProbeGate::default();
    assert_eq!(gate.state("a", 100.0, None), ProbeGateState::Ready);
    gate.start("a", 100.0);
    // Same provider in flight → Busy (a probe may run up to 10s, exceeding the 5s interval)
    assert_eq!(gate.state("a", 101.0, None), ProbeGateState::Busy);
    // Different providers don't interfere: another vendor is testable immediately (parallelizable)
    assert_eq!(gate.state("b", 100.0, None), ProbeGateState::Ready);
    gate.start("b", 100.0);
    gate.finish("a");
    // Any two probes of one provider (same or different models) share the 5-second spacing
    match gate.state("a", 102.0, None) {
        ProbeGateState::Cooling(left) => {
            assert!(left > 0.0 && left <= PROBE_PROVIDER_GAP_S)
        }
        other => panic!("应为 Cooling，实际 {other:?}"),
    }
    assert_eq!(gate.state("a", 105.0, None), ProbeGateState::Ready);
    // b still in flight doesn't affect a
    assert_eq!(gate.state("b", 106.0, None), ProbeGateState::Busy);
    gate.finish("b");
    // The network guard has top priority: probing stays blocked even after cooldown ends
    assert!(matches!(
        gate.state("a", 200.0, Some("检测到系统代理")),
        ProbeGateState::NetBlocked(_)
    ));
    // Release: reloading / closing the form releases a provider's hold
    gate.start("c", 300.0);
    gate.release(Some("a"));
    assert_eq!(gate.state("c", 400.0, None), ProbeGateState::Busy);
    gate.release(None);
    assert_eq!(gate.state("c", 400.0, None), ProbeGateState::Ready);
}

#[test]
fn probe_questions_rotate_per_provider() {
    use crate::app::fetch::{probe_question, ProbeGate, PROBE_QUESTIONS};
    // Consecutive picks for one provider never repeat
    let mut gate = ProbeGate::default();
    let first = gate.next_question("relay-a");
    let second = gate.next_question("relay-a");
    assert_ne!(first, second);
    // Different providers start at different offsets (the same cursor yields different questions)
    let other = probe_question("relay-b", 0);
    assert_ne!(probe_question("relay-a", 0), other);
    assert!(PROBE_QUESTIONS.contains(&other));
}

#[test]
fn models_url_google_vertex_uses_publishers_path() {
    assert_eq!(
        App::models_url(
            "https://us-central1-aiplatform.googleapis.com/v1",
            "google-vertex"
        ),
        "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models"
    );
    assert_eq!(
        App::models_url(
            "https://generativelanguage.googleapis.com/v1beta",
            "google-generative-ai"
        ),
        "https://generativelanguage.googleapis.com/v1beta/models"
    );
}

#[test]
fn sanitize_network_error_strips_url_query() {
    let msg = r#"connection failed: Connection refused (os error 10061) for URL "https://example.com/v1?key=sk-super-secret#frag""#;
    let out = sanitize_network_error(msg);
    assert!(!out.contains("sk-super-secret"), "泄露了 query: {}", out);
    assert!(!out.contains('#'), "泄露了 fragment: {}", out);
    assert!(out.contains("https://example.com/v1"));
}

#[test]
fn sanitize_network_error_passthrough_without_url() {
    let msg = "dns error: failed to lookup address";
    assert_eq!(sanitize_network_error(msg), msg);
}

#[test]
fn sanitize_network_error_url_no_query_untouched() {
    let msg = r#"connection failed for URL "https://api.example.com/v1/models""#;
    assert_eq!(sanitize_network_error(msg), msg);
}
