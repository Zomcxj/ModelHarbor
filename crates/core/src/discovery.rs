//! `/models` 探测核心：URL 候选规范化、模型列表探测、错误分级与结果缓存。
//!
//! 职责边界：本模块只做纯逻辑与单次 HTTP 探测，不碰界面（调度与展示在 app crate）。
//! 与 netguard 的口径一致：探测请求始终直连，不使用系统代理。

use serde_json::Value;
use std::collections::HashMap;
use std::error::Error as _;
use std::fmt;
use std::time::{Duration, Instant};

/// 缓存有效期：24 小时（与 opencode_models 的免费模型缓存一致）。
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 响应体摘要长度：非 JSON 报错时附带的正文前缀字符数。
const BODY_SNIPPET_CHARS: usize = 160;

// ---------------------------------------------------------------------------
// 1. URL 规范化
// ---------------------------------------------------------------------------

/// 按尝试顺序列出「模型列表」端点的候选 URL。
///
/// 规则：
/// - 显式 `custom_path` 优先且唯一生效：用户明确指定了端点就不再追加自动候选。
///   相对路径按 origin（协议 + 域名端口）解析、直接拼到 host 上，**绝不追加到
///   带版本号的 baseURL 后面**（避免 `https://host/v1` + `v1/models` 拼出
///   `/v1/v1/models`）；绝对 URL（带 http(s)://）直接使用。
/// - baseURL 以 `/v1` 结尾 → 只试 `{base}/models`。
/// - 其余 → 先试 `{base}/v1/models`，失败再退 `{base}/models`。
/// - 子路径原样保留（`https://host/api/v1` 不会丢 `/api/v1`）。
/// - baseURL 为空时返回空列表（上层应提示先填地址）。
pub fn models_endpoint_candidates(base_url: &str, custom_path: Option<&str>) -> Vec<String> {
    if let Some(custom) = custom_path.map(str::trim).filter(|s| !s.is_empty()) {
        let lower = custom.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return vec![custom.to_string()];
        }
        // 相对路径：只保留 origin，直接拼在 host 后面。
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

/// 取 URL 的 origin（协议 + 域名端口）。没有协议头时取首个 `/` 前的部分。
fn origin_of(url: &str) -> String {
    let trimmed = url.trim();
    let (head, rest) = match trimmed.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), trimmed),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!("{head}{authority}")
}

/// 取 baseURL 的路径段（origin 之后的部分）：去掉结尾的全部斜杠，不含开头斜杠。
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

/// 路径末段是否为 `v1`（大小写不敏感）。
fn ends_with_v1(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|seg| seg.eq_ignore_ascii_case("v1"))
}

// ---------------------------------------------------------------------------
// 2. 探测
// ---------------------------------------------------------------------------

/// 探测请求的鉴权方式：决定把 key 放进哪些请求头。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeAuth {
    /// 只发 `Authorization: Bearer <key>`（OpenAI 兼容网关的默认形态）。
    Bearer,
    /// Bearer 与 `x-api-key` 双发（同一个 key 放两处，兼容两种网关）。
    Both,
    /// 只发 `api-key: <key>`（Azure OpenAI）。
    Azure,
    /// 不带任何鉴权头（本地 Ollama 等无鉴权服务）。
    None,
}

/// 一次探测的结果：成功带模型列表与耗时，失败带分类错误。
#[derive(Debug)]
pub enum ProbeOutcome {
    /// 探测成功。
    Success {
        /// 模型 id 列表（保持响应顺序）。
        models: Vec<String>,
        /// 从发起请求到拿到可解析响应的耗时（毫秒）。
        elapsed_ms: u64,
    },
    /// 探测失败：已分类的错误。
    Failure(ProbeError),
}

/// 探测 `{base_url}` 的模型列表端点：按候选顺序请求，成功即返回。
///
/// 鉴权头按 `auth` 决定；key 缺省或鉴权方式为 None 时不带凭据，服务端的
/// 401/403 会归类为「未配置 API Key」。请求始终直连，`timeout` 同时约束
/// 连接阶段与整个请求（含响应体读取）。404/405 会自动换下一个候选
/// （`/v1/models` 失败退 `/models`），其余错误与路径无关、立即返回。
pub fn probe(
    base_url: &str,
    api_key: Option<&str>,
    auth: ProbeAuth,
    timeout: Duration,
) -> ProbeOutcome {
    let started = Instant::now();
    let key = api_key.map(str::trim).filter(|k| !k.is_empty());
    // 本次请求是否实际携带了 key：决定 401/403 时说「未配置」还是「无效」。
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
                    // 2xx 却解析不出模型列表：按非 JSON 处理并附正文摘要。
                    Err(_) => ProbeOutcome::Failure(ProbeError::BadJson {
                        body: body_snippet(&text),
                    }),
                };
            }
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                let classified = classify_status(code, &body, key_sent);
                // 路径不对时换下一个候选；其余错误与路径无关，直接返回。
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
    // 所有候选都返回 404/405：路径确实不对。
    ProbeOutcome::Failure(ProbeError::NotFound)
}

/// 探测专用 agent：短超时、直连。
///
/// ureq 2 默认不读环境代理（`proxy-from-env` 特性未开启），也不读系统代理，
/// 与 netguard「探测始终直连」的口径一致。
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        // 整体超时兜底：覆盖重定向与响应体读取，防止慢速响应拖住探测。
        .timeout(timeout)
        .build()
}

/// 按 `auth` 把 key 放进对应的请求头；没有可用 key 时保持原样
/// （服务端会以 401 回应，由错误分级说清「未配置」）。
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
// 3. 错误分级
// ---------------------------------------------------------------------------

/// 探测失败的分类。Display 输出中文人话，供界面直接展示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeError {
    /// 401/403：请求里没带 key（本地未配置，或鉴权方式选择了不发）。
    MissingKey,
    /// 401/403：响应体点明了 key 问题（invalid api key / unauthorized 等）。
    InvalidKey,
    /// 401/403：Cloudflare 1010 访问策略拦截，非 key 问题。
    Cloudflare1010,
    /// 404/405：路径不对（含 `/models` 回退候选在内全部失败），供上层触发回退重试。
    NotFound,
    /// 429：限流。
    RateLimited,
    /// 连接 / 读取超时。
    Timeout,
    /// 响应不是合法 JSON，附响应体前 160 字符。
    BadJson {
        /// 响应体前缀（按字符截断到 160）。
        body: String,
    },
    /// 其他 HTTP 状态码（文案复用 http_status 的人话表）。
    Status(u16),
    /// Base URL 为空，连候选 URL 都拼不出来。
    BadBaseUrl,
    /// 网络 / 传输错误（文本已脱敏：剥离 URL 的 query 与 fragment）。
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

/// 把 HTTP 状态码 + 响应体分类成探测错误。
///
/// `key_sent` 表示本次请求是否实际携带了 key：决定 401/403 说「未配置」
/// 还是「无效」。
fn classify_status(code: u16, body: &str, key_sent: bool) -> ProbeError {
    match code {
        401 | 403 => {
            if is_cloudflare_1010(body) {
                // 1010 在鉴权之前就拦截了，跟 key 无关，别误导用户去改 key。
                ProbeError::Cloudflare1010
            } else if !key_sent {
                ProbeError::MissingKey
            } else if body_mentions_key_problem(body) {
                ProbeError::InvalidKey
            } else {
                // 响应体没点明原因：不武断下结论，按状态码给通用文案。
                ProbeError::Status(code)
            }
        }
        404 | 405 => ProbeError::NotFound,
        429 => ProbeError::RateLimited,
        _ => ProbeError::Status(code),
    }
}

/// 响应体是否为 Cloudflare 1010（基于浏览器签名的访问拦截）。
///
/// 拦截响应有两种形态：完整拦截页（`Error 1010` … Cloudflare）与 Worker
/// 短正文（`error code: 1010`）。
fn is_cloudflare_1010(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("error code: 1010")
        || lower.contains("error 1010")
        || (lower.contains("1010") && lower.contains("cloudflare"))
}

/// 响应体是否点明了 key 问题（invalid api key / unauthorized 等关键词）。
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

/// 响应体前 160 字符（按字符截断，避免劈开多字节中文）。
fn body_snippet(body: &str) -> String {
    body.trim().chars().take(BODY_SNIPPET_CHARS).collect()
}

/// 响应体读取错误是否为超时。
fn is_io_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// 传输错误是否为超时（连接超时与读取超时都以 io::Error(TimedOut) 为 source）。
fn is_timeout_transport(transport: &ureq::Transport) -> bool {
    transport
        .source()
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .is_some_and(is_io_timeout)
}

// ---------------------------------------------------------------------------
// 4. 解析
// ---------------------------------------------------------------------------

/// 解析失败的分类。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    /// 响应不是合法 JSON。
    NotJson,
    /// JSON 合法，但找不到模型列表（既没有 `data[]` 也没有 `models[]`）。
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

/// 从 `/models` 响应中提取模型 id 列表，保持原有顺序。
///
/// 兼容两种主流形态：OpenAI 的 `{"data":[{"id":"..."}]}` 与 Ollama 的
/// `{"models":[{"name":"..."}]}`；元素可以是对象（取 `id` 或 `name` 字段）
/// 也可以是裸字符串；`models/<id>`、`model/<id>` 前缀会被剥掉；未知字段
/// 一律忽略；空列表返回空 Vec。
pub fn parse_models_json(body: &str) -> Result<Vec<String>, DiscoveryError> {
    let root: Value = serde_json::from_str(body).map_err(|_| DiscoveryError::NotJson)?;
    let Some(items) = models_array(&root) else {
        return Err(DiscoveryError::UnexpectedShape);
    };
    Ok(items.iter().filter_map(item_id).collect())
}

/// 找到承载模型条目的数组：顶层 / `data` / `models`，先到先得。
fn models_array(root: &Value) -> Option<&Vec<Value>> {
    if let Some(array) = root.as_array() {
        return Some(array);
    }
    ["data", "models"]
        .iter()
        .find_map(|key| root.get(*key).and_then(Value::as_array))
}

/// 单个条目 → 模型 id：裸字符串直接用，对象取 `id` / `name`，其余形态忽略。
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

/// 剥掉网关加在 id 前的 `models/` 或 `model/` 前缀（其余如 `openai/gpt-4o` 保留）。
fn strip_model_prefix(id: &str) -> String {
    id.strip_prefix("models/")
        .or_else(|| id.strip_prefix("model/"))
        .unwrap_or(id)
        .to_string()
}

// ---------------------------------------------------------------------------
// 5. 缓存
// ---------------------------------------------------------------------------

/// 探测结果缓存：`(base_url, custom_path)` → `(模型列表, 抓取时刻)`。
///
/// **绝不存 api_key / 鉴权头**：结构里没有任何存放凭据的字段，key 只在
/// 探测请求的瞬间使用，缓存命中也绝不会把 key 带出去。
#[derive(Debug, Default)]
pub struct DiscoveryCache {
    entries: HashMap<(String, String), (Vec<String>, Instant)>,
}

impl DiscoveryCache {
    /// 写入一条探测结果（重复写入覆盖旧值并刷新时间戳）。
    pub fn insert(&mut self, base_url: &str, custom_path: Option<&str>, models: Vec<String>) {
        self.entries
            .insert(cache_key(base_url, custom_path), (models, Instant::now()));
    }

    /// 有效期内（24h）的缓存；过期或不存在返回 `None`。
    ///
    /// 过期条目留在表里等下次 `insert` 覆盖：`fresh` 只借不删，避免可变借用。
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

/// 缓存键：base_url + 自定义端点（未填用空串）。
fn cache_key(base_url: &str, custom_path: Option<&str>) -> (String, String) {
    (
        base_url.trim().to_string(),
        custom_path.unwrap_or_default().trim().to_string(),
    )
}

// ---------------------------------------------------------------------------
// 6. 只填空合并
// ---------------------------------------------------------------------------

/// 只填空合并：已有条目原样保留、顺序不变，只把缺失的发现项追加到尾部；
/// 大小写不敏感去重（`GPT-4o` 与 `gpt-4o` 视为同一个模型）。
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

    // ---------- URL 候选 ----------

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

    /// 尾斜杠三态（无 / 一个 / 多个）与首尾空白都不影响候选。
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

    /// 已带 /v1 的 baseURL 不能再拼出 /v1/v1/models。
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

    /// 子路径不能丢：/api/v1 整体保留，只在其后补 /models。
    #[test]
    fn candidates_keep_sub_paths() {
        assert_eq!(
            models_endpoint_candidates("https://gw.example.com/api/v1", None),
            vec!["https://gw.example.com/api/v1/models".to_string()]
        );
        // 非版本结尾的子路径：先补 /v1/models，再退 /models。
        assert_eq!(
            models_endpoint_candidates("https://gw.example.com/api/", None),
            vec![
                "https://gw.example.com/api/v1/models".to_string(),
                "https://gw.example.com/api/models".to_string(),
            ]
        );
    }

    /// 显式端点唯一生效：绝对 URL 直接用；相对路径拼到 host 上，
    /// 绝不追加到带版本号的 baseURL。
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
        // 空白自定义路径视为未填，走自动候选。
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

    // ---------- 错误分级 ----------

    #[test]
    fn unauthorized_without_key_reports_missing_key() {
        let err = classify_status(401, r#"{"error":"unauthorized"}"#, false);
        assert_eq!(err, ProbeError::MissingKey);
        assert_eq!(err.to_string(), "未配置 API Key");
    }

    /// 响应体点明 key 问题时，明确说「无效或已过期」。
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

    /// 响应体没点明原因时不武断下结论，按状态码给通用文案。
    #[test]
    fn unauthorized_with_key_without_keyword_stays_generic() {
        let err = classify_status(401, "denied", true);
        assert_eq!(err, ProbeError::Status(401));
        assert_eq!(err.to_string(), "HTTP 401 认证失败");
    }

    /// 1010 是访问策略拦截，有没有 key 都不该误导用户去改 key。
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

    /// 非 JSON 报错附带的正文摘要按字符截断到 160。
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

    // ---------- 探测流程（不联网的部分） ----------

    #[test]
    fn probe_without_base_url_fails_fast() {
        let outcome = probe("", None, ProbeAuth::None, Duration::from_secs(1));
        assert!(matches!(
            outcome,
            ProbeOutcome::Failure(ProbeError::BadBaseUrl)
        ));
    }

    /// 非法 URL 在本地解析阶段就失败（无网络 IO），归入传输错误。
    #[test]
    fn probe_with_unparseable_url_reports_transport_error() {
        let outcome = probe("not-a-url", None, ProbeAuth::None, Duration::from_secs(1));
        assert!(matches!(
            outcome,
            ProbeOutcome::Failure(ProbeError::Transport(_))
        ));
    }

    // ---------- 解析 ----------

    #[test]
    fn parse_openai_data_ids() {
        let body =
            r#"{"object":"list","data":[{"id":"gpt-4o","object":"model"},{"id":"gpt-4o-mini"}]}"#;
        assert_eq!(
            parse_models_json(body),
            Ok(vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()])
        );
    }

    /// Ollama 形态：models[].name，其余字段（size、modified_at…）忽略。
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

    /// 只剥约定的 `models/`、`model/` 前缀；`厂商/模型` 与大小写不同的前缀保留。
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

    /// 未知字段容忍；不含 id/name 的条目与数字 id 跳过；裸字符串照常收。
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

    // ---------- 合并 ----------

    /// 已有条目原样保留、顺序不变，只追加缺失项。
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

    /// 空白发现项丢弃；与已有条目仅差首尾空白的视为重复（已有条目不动）。
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

    // ---------- 缓存 ----------

    #[test]
    fn cache_returns_fresh_entries_within_ttl() {
        let mut cache = DiscoveryCache::default();
        assert!(cache.fresh("https://api.example.com", None).is_none());

        cache.insert("https://api.example.com", None, vec!["m-1".to_string()]);
        let (models, fetched_at) = cache.fresh("https://api.example.com", None).unwrap();
        assert_eq!(models, &["m-1".to_string()]);
        assert!(fetched_at.elapsed() < CACHE_TTL);

        // 键里带上自定义端点：不同端点互不干扰。
        assert!(cache
            .fresh("https://api.example.com", Some("v1/models"))
            .is_none());
    }

    /// 直接构造一条 25 小时前的旧记录验证 TTL 判定（平台时钟表示不了时跳过）。
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

    /// 结构上只有 (base_url, custom_path) → (模型列表, 时刻)，没有任何存
    /// key / 鉴权头的字段；用 Debug 输出反向守住这一点。
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
