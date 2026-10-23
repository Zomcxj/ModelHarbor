//! `/models` discovery core: URL candidate normalization, model-list probing, error classification, result caching.
//!
//! Scope: this module does pure logic and one-shot HTTP probing only — no UI (scheduling and display
//! live in the app crate). Matches netguard's policy: probe requests always connect directly, never
//! through a system proxy.

use serde_json::Value;
use std::collections::HashMap;
use std::error::Error as _;
use std::fmt;
use std::time::{Duration, Instant};

/// Cache lifetime: 24 hours (matches opencode_models' free-model cache).
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Body digest length: number of body prefix characters attached when a non-JSON error is reported.
const BODY_SNIPPET_CHARS: usize = 160;

// ---------------------------------------------------------------------------
// 1. URL normalization
// ---------------------------------------------------------------------------

/// List candidate "model list" endpoint URLs in attempt order.
///
/// Rules:
/// - An explicit `custom_path` takes priority and wins alone: once the user names an endpoint, no
///   automatic candidates are appended. Relative paths resolve against the origin (scheme + host:port)
///   and attach directly to the host — **never appended after a versioned baseURL** (avoids
///   `https://host/v1` + `v1/models` producing `/v1/v1/models`); absolute URLs (with http(s)://) are
///   used as-is.
/// - baseURL ending in `/v1` → only `{base}/models` is tried.
/// - Otherwise → `{base}/v1/models` first, falling back to `{base}/models`.
/// - Sub-paths are preserved (`https://host/api/v1` keeps `/api/v1`).
/// - Empty baseURL returns an empty list (the caller should prompt for an address first).
pub fn models_endpoint_candidates(base_url: &str, custom_path: Option<&str>) -> Vec<String> {
    if let Some(custom) = custom_path.map(str::trim).filter(|s| !s.is_empty()) {
        let lower = custom.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return vec![custom.to_string()];
        }
        // Relative path: keep only the origin and attach directly after the host.
        let path = custom.trim_start_matches('/');
        return vec![format!("{}/{}", origin_of(base_url), path)];
    }
    let base = base_url.trim();
    if base.is_empty() {
        return Vec::new();
    }
    let path = base_path(base);
    let origin = origin_of(base);
    let root = if path.is_empty() {
        origin
    } else {
        format!("{origin}/{path}")
    };
    if ends_with_v1(&path) {
        vec![format!("{root}/models")]
    } else {
        vec![format!("{root}/v1/models"), format!("{root}/models")]
    }
}

/// Take a URL's origin (scheme + host:port). Without a scheme, everything before the first `/`.
fn origin_of(url: &str) -> String {
    let trimmed = url.trim();
    let (head, rest) = match trimmed.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), trimmed),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!("{head}{authority}")
}

/// Take baseURL's path segment (after the origin): all trailing slashes removed, no leading slash.
fn base_path(url: &str) -> String {
    let trimmed = url.trim();
    let rest = match trimmed.split_once("://") {
        Some((_, rest)) => rest,
        None => trimmed,
    };
    match rest.find('/') {
        Some(idx) => rest[idx + 1..].trim_end_matches('/').to_string(),
        None => String::new(),
    }
}

/// Whether the path's last segment is `v1` (case-insensitive).
fn ends_with_v1(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|seg| seg.eq_ignore_ascii_case("v1"))
}

// ---------------------------------------------------------------------------
// 2. Probing
// ---------------------------------------------------------------------------

/// Auth mode of a probe request: decides which headers carry the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeAuth {
    /// Send only `Authorization: Bearer <key>` (the default shape of OpenAI-compatible gateways).
    Bearer,
    /// Send both Bearer and `x-api-key` (the same key in two places, compatible with both gateway kinds).
    Both,
    /// Send only `api-key: <key>` (Azure OpenAI).
    Azure,
    /// No auth headers at all (local Ollama and other unauthenticated services).
    None,
}

/// One probe's outcome: success carries the model list and elapsed time; failure carries a classified error.
#[derive(Debug)]
pub enum ProbeOutcome {
    /// Probe succeeded.
    Success {
        /// The model id list (response order preserved).
        models: Vec<String>,
        /// Elapsed from request issue to a parseable response (milliseconds).
        elapsed_ms: u64,
    },
    /// Probe failed: a classified error.
    Failure(ProbeError),
}

/// Probe `{base_url}`'s model-list endpoint: request candidates in order, return on first success.
///
/// Auth headers follow `auth`; with no key or `None` auth, requests go out without credentials and the
/// server's 401/403 classifies as "API key not configured". Requests always connect directly;
/// `timeout` bounds both the connection phase and the whole request (including body read). 404/405
/// automatically advance to the next candidate (`/v1/models` falls back to `/models`); all other
/// errors are path-independent and return immediately.
pub fn probe(
    base_url: &str,
    api_key: Option<&str>,
    auth: ProbeAuth,
    timeout: Duration,
) -> ProbeOutcome {
    let started = Instant::now();
    let key = api_key.map(str::trim).filter(|k| !k.is_empty());
    // Whether this request actually carried a key: decides whether 401/403 say "not configured" or "invalid".
    let key_sent = !matches!(auth, ProbeAuth::None) && key.is_some();
    let candidates = models_endpoint_candidates(base_url, None);
    if candidates.is_empty() {
        return ProbeOutcome::Failure(ProbeError::BadBaseUrl);
    }
    let http = agent(timeout);
    for url in candidates {
        let request = apply_auth(http.get(&url), key, auth)
            .set("Accept", "application/json")
            .set("User-Agent", "ModelHarbor");
        match request.call() {
            Ok(response) => {
                let text = match response.into_string() {
                    Ok(text) => text,
                    Err(err) => {
                        if is_io_timeout(&err) {
                            return ProbeOutcome::Failure(ProbeError::Timeout);
                        }
                        return ProbeOutcome::Failure(ProbeError::Transport(
                            crate::util::sanitize_network_error(&err.to_string()),
                        ));
                    }
                };
                return match parse_models_json(&text) {
                    Ok(models) => ProbeOutcome::Success {
                        models,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    },
                    // 2xx yet no model list parses: treat as non-JSON with a body digest attached.
                    Err(_) => ProbeOutcome::Failure(ProbeError::BadJson {
                        body: body_snippet(&text),
                    }),
                };
            }
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                let classified = classify_status(code, &body, key_sent);
                // Wrong path advances to the next candidate; all other errors are path-independent and return immediately.
                if !matches!(classified, ProbeError::NotFound) {
                    return ProbeOutcome::Failure(classified);
                }
            }
            Err(ureq::Error::Transport(transport)) => {
                if is_timeout_transport(&transport) {
                    return ProbeOutcome::Failure(ProbeError::Timeout);
                }
                return ProbeOutcome::Failure(ProbeError::Transport(
                    crate::util::sanitize_network_error(&transport.to_string()),
                ));
            }
        }
    }
    // Every candidate returned 404/405: the path is genuinely wrong.
    ProbeOutcome::Failure(ProbeError::NotFound)
}

/// Probe-specific agent: short timeout, direct connection.
///
/// ureq 2 does not read environment proxies by default (the `proxy-from-env` feature is off) nor the
/// system proxy — consistent with netguard's "probes always connect directly" policy.
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        // Whole-request timeout backstop: covers redirects and body reads so a slow response can't stall the probe.
        .timeout(timeout)
        .build()
}

/// Place the key in the matching headers per `auth`; without a usable key, headers stay as-is
/// (the server answers 401 and the error classification explains "not configured").
fn apply_auth(request: ureq::Request, key: Option<&str>, auth: ProbeAuth) -> ureq::Request {
    let Some(key) = key else {
        return request;
    };
    match auth {
        ProbeAuth::None => request,
        ProbeAuth::Bearer => request.set("Authorization", &format!("Bearer {key}")),
        ProbeAuth::Both => request
            .set("Authorization", &format!("Bearer {key}"))
            .set("x-api-key", key),
        ProbeAuth::Azure => request.set("api-key", key),
    }
}

// ---------------------------------------------------------------------------
// 3. Error classification
// ---------------------------------------------------------------------------

/// Classification of probe failures. Display renders human-readable Chinese for direct UI display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// 401/403: the request carried no key (not configured locally, or the auth mode sends none).
    MissingKey,
    /// 401/403: the body names a key problem (invalid api key / unauthorized etc.).
    InvalidKey,
    /// 401/403: a Cloudflare 1010 access-policy block — not a key problem.
    Cloudflare1010,
    /// 404/405: wrong path (every candidate including the `/models` fallback failed); the caller triggers fallback retry.
    NotFound,
    /// 429: rate-limited.
    RateLimited,
    /// Connect / read timeout.
    Timeout,
    /// The response isn't valid JSON, with the body's first 160 characters attached.
    BadJson {
        /// The body prefix (character-truncated to 160).
        body: String,
    },
    /// Other HTTP status codes (text reuses http_status's human table).
    Status(u16),
    /// Empty base URL — not even a candidate URL can be assembled.
    BadBaseUrl,
    /// Network / transport error (text sanitized: the URL's query and fragment stripped).
    Transport(String),
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey => write!(f, "未配置 API Key"),
            Self::InvalidKey => write!(f, "API Key 无效或已过期"),
            Self::Cloudflare1010 => write!(f, "Cloudflare 1010 访问策略拦截（非 key 问题）"),
            Self::NotFound => write!(f, "路径不对，已回退 /models 重试"),
            Self::RateLimited => write!(f, "{}", crate::http_status::label(429)),
            Self::Timeout => write!(f, "连接超时（本地服务未启动？）"),
            Self::BadJson { body } => write!(f, "响应不是合法 JSON：{body}"),
            Self::Status(code) => write!(f, "{}", crate::http_status::label(*code)),
            Self::BadBaseUrl => write!(f, "Base URL 未填写"),
            Self::Transport(text) => write!(f, "网络错误：{text}"),
        }
    }
}

impl std::error::Error for ProbeError {}

/// Classify an HTTP status code + response body into a probe error.
///
/// `key_sent` records whether the request actually carried a key: it decides whether 401/403 say
/// "not configured" or "invalid".
fn classify_status(code: u16, body: &str, key_sent: bool) -> ProbeError {
    match code {
        401 | 403 => {
            if is_cloudflare_1010(body) {
                // 1010 blocks before auth even happens — unrelated to the key; don't mislead users into changing it.
                ProbeError::Cloudflare1010
            } else if !key_sent {
                ProbeError::MissingKey
            } else if body_mentions_key_problem(body) {
                ProbeError::InvalidKey
            } else {
                // The body doesn't name a cause: no hasty conclusions — generic text by status code.
                ProbeError::Status(code)
            }
        }
        404 | 405 => ProbeError::NotFound,
        429 => ProbeError::RateLimited,
        _ => ProbeError::Status(code),
    }
}

/// Whether the body is a Cloudflare 1010 (browser-signature-based access block).
///
/// Block pages come in two shapes: the full interstitial (`Error 1010` … Cloudflare) and a Worker's
/// short body (`error code: 1010`).
fn is_cloudflare_1010(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("error code: 1010")
        || lower.contains("error 1010")
        || (lower.contains("1010") && lower.contains("cloudflare"))
}

/// Whether the body names a key problem (invalid api key / unauthorized and similar keywords).
fn body_mentions_key_problem(body: &str) -> bool {
    const KEYWORDS: [&str; 7] = [
        "invalid api key",
        "invalid_api_key",
        "incorrect api key",
        "api key invalid",
        "unauthorized",
        "unauthenticated",
        "authentication",
    ];
    let lower = body.to_ascii_lowercase();
    KEYWORDS.iter().any(|keyword| lower.contains(keyword))
}

/// First 160 characters of the body (character-wise, so multi-byte Chinese isn't split).
fn body_snippet(body: &str) -> String {
    body.trim().chars().take(BODY_SNIPPET_CHARS).collect()
}

/// Whether a body-read error is a timeout.
fn is_io_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// Whether a transport error is a timeout (both connect and read timeouts surface as io::Error(TimedOut) sources).
fn is_timeout_transport(transport: &ureq::Transport) -> bool {
    transport
        .source()
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .is_some_and(is_io_timeout)
}

// ---------------------------------------------------------------------------
// 4. Parsing
// ---------------------------------------------------------------------------

/// Classification of parse failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The response isn't valid JSON.
    NotJson,
    /// Valid JSON, but no model list found (neither `data[]` nor `models[]`).
    UnexpectedShape,
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson => write!(f, "响应不是合法 JSON"),
            Self::UnexpectedShape => {
                write!(f, "响应里找不到模型列表（没有 data[] 也没有 models[]）")
            }
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// Extract the model id list from a `/models` response, preserving original order.
///
/// Handles the two mainstream shapes: OpenAI's `{"data":[{"id":"..."}]}` and Ollama's
/// `{"models":[{"name":"..."}]}`; elements may be objects (taking `id` or `name`) or bare strings;
/// `models/<id>` / `model/<id>` prefixes are stripped; unknown fields are ignored; an empty list
/// returns an empty Vec.
pub fn parse_models_json(body: &str) -> Result<Vec<String>, DiscoveryError> {
    let root: Value = serde_json::from_str(body).map_err(|_| DiscoveryError::NotJson)?;
    let Some(items) = models_array(&root) else {
        return Err(DiscoveryError::UnexpectedShape);
    };
    Ok(items.iter().filter_map(item_id).collect())
}

/// Find the array holding model entries: top level / `data` / `models`, first match wins.
fn models_array(root: &Value) -> Option<&Vec<Value>> {
    if let Some(array) = root.as_array() {
        return Some(array);
    }
    ["data", "models"]
        .iter()
        .find_map(|key| root.get(*key).and_then(Value::as_array))
}

/// One entry → model id: bare strings pass through, objects take `id` / `name`, other shapes are ignored.
fn item_id(item: &Value) -> Option<String> {
    let raw = match item {
        Value::String(text) => text.as_str(),
        Value::Object(map) => ["id", "name"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))?,
        _ => return None,
    };
    let id = strip_model_prefix(raw.trim());
    (!id.is_empty()).then_some(id)
}

/// Strip a gateway's `models/` or `model/` prefix from an id (others like `openai/gpt-4o` are kept).
fn strip_model_prefix(id: &str) -> String {
    id.strip_prefix("models/")
        .or_else(|| id.strip_prefix("model/"))
        .unwrap_or(id)
        .to_string()
}

// ---------------------------------------------------------------------------
// 5. Cache
// ---------------------------------------------------------------------------

/// Probe-result cache: `(base_url, custom_path)` → `(model list, fetched at)`.
///
/// **Never stores api_key / auth headers**: the struct has no credential fields — the key exists only
/// for the instant of the probe request, and cache hits can never leak it.
#[derive(Debug, Default)]
pub struct DiscoveryCache {
    entries: HashMap<(String, String), (Vec<String>, Instant)>,
}

impl DiscoveryCache {
    /// Insert one probe result (re-inserting overwrites the old value and refreshes the timestamp).
    pub fn insert(&mut self, base_url: &str, custom_path: Option<&str>, models: Vec<String>) {
        self.entries
            .insert(cache_key(base_url, custom_path), (models, Instant::now()));
    }

    /// Cache within its lifetime (24h); `None` when expired or absent.
    ///
    /// Expired entries stay in the table awaiting the next `insert` overwrite: `fresh` only borrows, never deletes, avoiding a mutable borrow.
    pub fn fresh(
        &self,
        base_url: &str,
        custom_path: Option<&str>,
    ) -> Option<&(Vec<String>, Instant)> {
        self.entries
            .get(&cache_key(base_url, custom_path))
            .filter(|(_, fetched_at)| fetched_at.elapsed() < CACHE_TTL)
    }
}

/// Cache key: base_url + custom endpoint (empty string when unset).
fn cache_key(base_url: &str, custom_path: Option<&str>) -> (String, String) {
    (
        base_url.trim().to_string(),
        custom_path.unwrap_or_default().trim().to_string(),
    )
}

// ---------------------------------------------------------------------------
// 6. Fill-only merge
// ---------------------------------------------------------------------------

/// Fill-only merge: existing entries are kept verbatim and in order, with newly discovered items
/// appended at the tail; dedup is case-insensitive (`GPT-4o` and `gpt-4o` are the same model).
pub fn merge_missing(existing: &[String], discovered: Vec<String>) -> Vec<String> {
    let mut merged = existing.to_vec();
    for model in discovered {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            continue;
        }
        let duplicate = merged
            .iter()
            .any(|known| known.trim().eq_ignore_ascii_case(trimmed));
        if !duplicate {
            merged.push(trimmed.to_string());
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- URL candidates ----------

    #[test]
    fn candidates_without_v1_try_both_paths() {
        assert_eq!(
            models_endpoint_candidates("https://api.example.com", None),
            vec![
                "https://api.example.com/v1/models".to_string(),
                "https://api.example.com/models".to_string(),
            ]
        );
    }

    /// Trailing-slash shapes (none / one / many) and leading/trailing whitespace don't affect candidates.
    #[test]
    fn candidates_tolerate_trailing_slashes() {
        for base in [
            "https://api.example.com",
            "https://api.example.com/",
            "https://api.example.com///",
            " https://api.example.com ",
        ] {
            assert_eq!(
                models_endpoint_candidates(base, None),
                vec![
                    "https://api.example.com/v1/models".to_string(),
                    "https://api.example.com/models".to_string(),
                ],
                "{base} 的候选不应受尾斜杠影响"
            );
        }
    }

    /// A baseURL already carrying /v1 must not produce /v1/v1/models.
    #[test]
    fn candidates_for_versioned_base_only_try_base_models() {
        for base in [
            "https://api.example.com/v1",
            "https://api.example.com/v1/",
            "https://api.example.com/v1///",
        ] {
            assert_eq!(
                models_endpoint_candidates(base, None),
                vec!["https://api.example.com/v1/models".to_string()],
                "{base} 已带 /v1，只应试 {base}/models"
            );
        }
    }

    /// Sub-paths can't be lost: /api/v1 stays whole, with /models appended after it.
    #[test]
    fn candidates_keep_sub_paths() {
        assert_eq!(
            models_endpoint_candidates("https://gw.example.com/api/v1", None),
            vec!["https://gw.example.com/api/v1/models".to_string()]
        );
        // Sub-paths not ending in a version: /v1/models first, falling back to /models.
        assert_eq!(
            models_endpoint_candidates("https://gw.example.com/api/", None),
            vec![
                "https://gw.example.com/api/v1/models".to_string(),
                "https://gw.example.com/api/models".to_string(),
            ]
        );
    }

    /// An explicit endpoint wins alone: absolute URLs used as-is; relative paths attach to the host —
    /// never appended after a versioned baseURL.
    #[test]
    fn explicit_endpoint_wins_and_resolves_against_origin() {
        assert_eq!(
            models_endpoint_candidates(
                "https://api.example.com/v1",
                Some("https://other.example.com/models")
            ),
            vec!["https://other.example.com/models".to_string()]
        );
        assert_eq!(
            models_endpoint_candidates("https://api.example.com/v1", Some("v1/models")),
            vec!["https://api.example.com/v1/models".to_string()]
        );
        assert_eq!(
            models_endpoint_candidates("https://api.example.com/v1", Some("/api/models")),
            vec!["https://api.example.com/api/models".to_string()]
        );
        // Whitespace-only custom paths count as unset, going through automatic candidates.
        assert_eq!(
            models_endpoint_candidates("https://api.example.com/v1", Some("  ")),
            vec!["https://api.example.com/v1/models".to_string()]
        );
    }

    #[test]
    fn empty_base_url_has_no_candidates() {
        assert!(models_endpoint_candidates("", None).is_empty());
        assert!(models_endpoint_candidates("   ", None).is_empty());
    }

    // ---------- Error classification ----------

    #[test]
    fn unauthorized_without_key_reports_missing_key() {
        let err = classify_status(401, r#"{"error":"unauthorized"}"#, false);
        assert_eq!(err, ProbeError::MissingKey);
        assert_eq!(err.to_string(), "未配置 API Key");
    }

    /// When the body names a key problem, say clearly "invalid or expired".
    #[test]
    fn unauthorized_with_key_and_keyword_reports_invalid_key() {
        let err = classify_status(
            401,
            r#"{"error":{"message":"Incorrect API key provided: sk-xxx."}}"#,
            true,
        );
        assert_eq!(err, ProbeError::InvalidKey);
        assert_eq!(err.to_string(), "API Key 无效或已过期");
    }

    /// When the body names no cause, no hasty conclusions — generic text by status code.
    #[test]
    fn unauthorized_with_key_without_keyword_stays_generic() {
        let err = classify_status(401, "denied", true);
        assert_eq!(err, ProbeError::Status(401));
        assert_eq!(err.to_string(), "HTTP 401 认证失败");
    }

    /// 1010 is an access-policy block: with or without a key, users must not be told to change it.
    #[test]
    fn cloudflare_1010_is_not_a_key_problem() {
        for body in [
            "<html><h1>Error 1010</h1><p>Access denied | Cloudflare</p></html>",
            "error code: 1010",
        ] {
            assert_eq!(
                classify_status(403, body, false),
                ProbeError::Cloudflare1010
            );
            assert_eq!(classify_status(403, body, true), ProbeError::Cloudflare1010);
        }
        assert!(ProbeError::Cloudflare1010
            .to_string()
            .contains("非 key 问题"));
    }

    #[test]
    fn not_found_and_method_not_allowed_signal_path_fallback() {
        assert_eq!(classify_status(404, "", true), ProbeError::NotFound);
        assert_eq!(classify_status(405, "", true), ProbeError::NotFound);
        assert!(ProbeError::NotFound.to_string().contains("路径不对"));
    }

    #[test]
    fn rate_limit_has_dedicated_label() {
        assert_eq!(classify_status(429, "", true), ProbeError::RateLimited);
        assert!(ProbeError::RateLimited.to_string().contains("限流"));
    }

    #[test]
    fn timeout_message_hints_local_service() {
        assert!(ProbeError::Timeout.to_string().contains("本地服务未启动"));
    }

    /// The body digest attached to non-JSON errors is character-truncated to 160.
    #[test]
    fn bad_json_carries_truncated_body() {
        let snippet = body_snippet(&"x".repeat(300));
        assert_eq!(snippet.chars().count(), BODY_SNIPPET_CHARS);
        let err = ProbeError::BadJson { body: snippet };
        assert!(err.to_string().starts_with("响应不是合法 JSON"));
    }

    #[test]
    fn transport_errors_display_with_prefix() {
        let err = ProbeError::Transport("connection refused".to_string());
        assert_eq!(err.to_string(), "网络错误：connection refused");
    }

    // ---------- Probe flow (the offline part) ----------

    #[test]
    fn probe_without_base_url_fails_fast() {
        let outcome = probe("", None, ProbeAuth::None, Duration::from_secs(1));
        assert!(matches!(
            outcome,
            ProbeOutcome::Failure(ProbeError::BadBaseUrl)
        ));
    }

    /// Malformed URLs fail at local parsing (no network IO), classified as transport errors.
    #[test]
    fn probe_with_unparseable_url_reports_transport_error() {
        let outcome = probe("not-a-url", None, ProbeAuth::None, Duration::from_secs(1));
        assert!(matches!(
            outcome,
            ProbeOutcome::Failure(ProbeError::Transport(_))
        ));
    }

    // ---------- Parsing ----------

    #[test]
    fn parse_openai_data_ids() {
        let body =
            r#"{"object":"list","data":[{"id":"gpt-4o","object":"model"},{"id":"gpt-4o-mini"}]}"#;
        assert_eq!(
            parse_models_json(body),
            Ok(vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()])
        );
    }

    /// The Ollama shape: models[].name; other fields (size, modified_at…) ignored.
    #[test]
    fn parse_ollama_model_names() {
        let body = r#"{"models":[{"name":"llama3:latest","model":"llama3:latest","size":1}]}"#;
        assert_eq!(
            parse_models_json(body),
            Ok(vec!["llama3:latest".to_string()])
        );
    }

    #[test]
    fn parse_bare_string_items() {
        assert_eq!(
            parse_models_json(r#"{"data":["m-a","m-b"]}"#),
            Ok(vec!["m-a".to_string(), "m-b".to_string()])
        );
        assert_eq!(
            parse_models_json(r#"{"models":["m-c"]}"#),
            Ok(vec!["m-c".to_string()])
        );
    }

    /// Strip only the agreed `models/` / `model/` prefixes; `vendor/model` and case-different prefixes are kept.
    #[test]
    fn parse_strips_gateway_prefixes_only() {
        let body = r#"{"data":[
            {"id":"models/gpt-4"},
            {"id":"model/claude-3"},
            {"id":"openai/gpt-4o"},
            {"id":"MODELS/uppercase"}
        ]}"#;
        assert_eq!(
            parse_models_json(body),
            Ok(vec![
                "gpt-4".to_string(),
                "claude-3".to_string(),
                "openai/gpt-4o".to_string(),
                "MODELS/uppercase".to_string(),
            ])
        );
    }

    #[test]
    fn parse_empty_data_returns_empty_vec() {
        assert_eq!(parse_models_json(r#"{"data":[]}"#), Ok(Vec::new()));
    }

    /// Tolerates unknown fields; entries without id/name and numeric ids are skipped; bare strings collected as usual.
    #[test]
    fn parse_tolerates_unknown_fields_and_skips_unshaped_items() {
        let body = r#"{"data":[{"id":"m-1","owned_by":"x","created":1},{"object":"model"},{"id":42},"str-item"]}"#;
        assert_eq!(
            parse_models_json(body),
            Ok(vec!["m-1".to_string(), "str-item".to_string()])
        );
    }

    #[test]
    fn parse_rejects_non_json_and_unknown_shapes() {
        assert_eq!(
            parse_models_json("<html>502 Bad Gateway</html>"),
            Err(DiscoveryError::NotJson)
        );
        assert_eq!(
            parse_models_json(r#"{"foo":1}"#),
            Err(DiscoveryError::UnexpectedShape)
        );
        assert!(DiscoveryError::UnexpectedShape
            .to_string()
            .contains("模型列表"));
    }

    // ---------- Merging ----------

    /// Existing entries stay verbatim and in order; only missing items are appended.
    #[test]
    fn merge_keeps_existing_order_and_appends_only_missing() {
        assert_eq!(
            merge_missing(
                &["a".to_string(), "b".to_string()],
                vec!["c".to_string(), "b".to_string(), "d".to_string()],
            ),
            vec![
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string()
            ]
        );
    }

    #[test]
    fn merge_dedupes_case_insensitively() {
        assert_eq!(
            merge_missing(
                &["GPT-4o".to_string()],
                vec![
                    "gpt-4o".to_string(),
                    "GPT-4O".to_string(),
                    "new".to_string()
                ],
            ),
            vec!["GPT-4o".to_string(), "new".to_string()]
        );
    }

    /// Whitespace-only finds are dropped; finds differing from an existing entry only in edge whitespace count as duplicates (the existing one untouched).
    #[test]
    fn merge_ignores_blank_and_padded_duplicates() {
        assert_eq!(
            merge_missing(
                &["a".to_string()],
                vec!["  ".to_string(), String::new(), "a ".to_string()],
            ),
            vec!["a".to_string()]
        );
    }

    #[test]
    fn merge_with_empty_sides() {
        assert_eq!(merge_missing(&[], Vec::new()), Vec::<String>::new());
        assert_eq!(
            merge_missing(&[], vec!["x".to_string()]),
            vec!["x".to_string()]
        );
        assert_eq!(
            merge_missing(&["y".to_string()], Vec::new()),
            vec!["y".to_string()]
        );
    }

    // ---------- Cache ----------

    #[test]
    fn cache_returns_fresh_entries_within_ttl() {
        let mut cache = DiscoveryCache::default();
        assert!(cache.fresh("https://api.example.com", None).is_none());

        cache.insert("https://api.example.com", None, vec!["m-1".to_string()]);
        let (models, fetched_at) = cache.fresh("https://api.example.com", None).unwrap();
        assert_eq!(models, &["m-1".to_string()]);
        assert!(fetched_at.elapsed() < CACHE_TTL);

        // The key carries the custom endpoint: different endpoints never interfere.
        assert!(cache
            .fresh("https://api.example.com", Some("v1/models"))
            .is_none());
    }

    /// Construct a 25-hour-old record directly to verify the TTL verdict (skipped when the platform clock can't express it).
    #[test]
    fn cache_expires_after_ttl() {
        let mut cache = DiscoveryCache::default();
        let Some(stale_at) = Instant::now().checked_sub(Duration::from_secs(25 * 60 * 60)) else {
            return;
        };
        cache.entries.insert(
            ("https://api.example.com".to_string(), String::new()),
            (vec!["stale".to_string()], stale_at),
        );
        assert!(cache.fresh("https://api.example.com", None).is_none());
    }

    /// Structurally only (base_url, custom_path) → (model list, moment) exists — no field anywhere holds
    /// a key or auth header; the Debug output guards that in reverse.
    #[test]
    fn cache_structure_holds_no_credentials() {
        let mut cache = DiscoveryCache::default();
        cache.insert(
            "https://api.example.com",
            Some("v1"),
            vec!["m-1".to_string()],
        );
        let debug = format!("{cache:?}");
        assert!(!debug.contains("sk-"));
        assert!(!debug.contains("Bearer "));
        assert!(!debug.contains("x-api-key"));
        let (models, _) = cache.fresh("https://api.example.com", Some("v1")).unwrap();
        assert_eq!(models, &["m-1".to_string()]);
    }
}
