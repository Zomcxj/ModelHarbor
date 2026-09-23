//! opencode Zen 免费模型列表：动态获取，避免下拉里留下已过期的模型 id。
//!
//! ## 为什么需要它
//!
//! Agents 的 `model` 字段取值形如 `provider/model`，其中 `provider` 为 `opencode`
//! 时指的是 opencode **内置的 Zen 网关**（不是用户自己配的 provider key）。Zen 的
//! 免费模型会随上游上下架，早先写死在界面里的两个 id（`mimo-v2.5-free`、`big-pickle`）
//! 迟早会变成「选了却跑不起来」的过期项，所以改为按需拉取。
//!
//! ## 两个数据源，各答一半
//!
//! | 源 | 地址 | 提供 |
//! |---|---|---|
//! | models.dev | `https://models.dev/api.json` | 价格（`cost`）与上下架标记（`status`） |
//! | Zen 网关 | `https://opencode.ai/zen/v1/models` | 当前**真实可用**的模型 id |
//!
//! 单靠任何一个都不够：Zen 的 `/models` 只有 id、没有价格，分不出免费与收费；
//! models.dev 的价格准确，但它的 `deprecated` 标记**偏保守**——实测
//! `mimo-v2.5-free` 被标为 deprecated，Zen 网关却仍在正常提供（返回 403
//! FreeTierError，即「模型存在，只是限定在 opencode 内使用」）。
//!
//! 所以判定取**两者交集**：models.dev 说免费 **且** Zen 网关确实提供。
//! 这样既不会推荐已下架的 id（`glm-5-free` / `kimi-k2.5-free` / `grok-code`
//! 在网关上返回 401「Model is not supported」），也不会漏掉仍可用但被保守标记的模型。
//!
//! 网关请求失败时回退为「只信 models.dev」，并额外排除 deprecated 标记的条目——
//! 拿不到实测依据时，宁可少列几个。
//!
//! 免费判定：`cost.input == 0 && cost.output == 0`，两者都必须是**显式的 0**；
//! 字段缺失说明数据源还没收录价格，按收费处理（宁可少列，也不要把收费模型推荐出去）。
//!
//! ## 缓存
//!
//! `api.json` 未压缩约 4.8 MB（gzip 后约 470 KB），每次启动都下载太浪费，因此
//! 结果落盘到 `.modelharbor/opencode-free-models.json`，超过 [`CACHE_TTL_SECS`]
//! 才在后台重新拉取。缓存只存模型 id 列表，不含任何凭证。

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 价格数据源（models.dev 全量模型库）。
pub const SOURCE_URL: &str = "https://models.dev/api.json";

/// 可用性数据源（Zen 网关的模型列表）。
pub const LIVE_URL: &str = "https://opencode.ai/zen/v1/models";

/// 数据源里 opencode 官方网关的厂商 id。
pub const PROVIDER_ID: &str = "opencode";

/// 缓存文件名（与设置、令牌同目录）。
const CACHE_FILE: &str = "opencode-free-models.json";

/// 缓存有效期（秒）：超过则在后台重新拉取。24 小时。
///
/// 免费模型上下架不是分钟级事件，一天一次足够；界面上的「刷新」按钮可随时强制重取。
pub const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// 落盘缓存的内容（`fetched_at` + 模型 id 列表）。
pub struct Cache {
    pub models: Vec<String>,
    /// 是否仍在有效期内（过期也照样返回 `models`，由调用方决定要不要后台刷新）。
    pub fresh: bool,
}

/// 当前时间（秒级 Unix 时间戳）。
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 缓存文件路径。
pub fn cache_path() -> PathBuf {
    crate::prefs::Prefs::config_dir().join(CACHE_FILE)
}

/// models.dev 里的一个免费模型。
#[derive(Debug)]
pub struct FreeModel {
    pub id: String,
    /// 数据源是否已标记其下架（保守标记，不代表网关已停供）。
    pub deprecated: bool,
}

/// 该模型是否「价格为零」。
///
/// 价格必须是**显式的 0**：字段缺失说明数据源还没收录价格，按收费处理。
fn is_zero_cost(model: &Value) -> bool {
    let zero = |key: &str| {
        model
            .get("cost")
            .and_then(|cost| cost.get(key))
            .and_then(Value::as_f64)
            == Some(0.0)
    };
    zero("input") && zero("output")
}

/// 从 `api.json` 全文里筛出 opencode 的免费模型（含 deprecated 标记，交由调用方取舍）。
pub fn parse_free_models(text: &str) -> Result<Vec<FreeModel>, String> {
    let root: Value = serde_json::from_str(text).map_err(|err| {
        // 只带错误与开头片段：正文 4.8 MB，不能整个塞进提示里。
        let snippet = text.chars().take(120).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })?;
    let provider = root
        .get(PROVIDER_ID)
        .ok_or_else(|| format!("数据里没有 {} 厂商", PROVIDER_ID))?;
    let models = provider
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} 厂商下没有 models", PROVIDER_ID))?;
    let mut found: Vec<FreeModel> = models
        .iter()
        .filter(|(_, model)| is_zero_cost(model))
        .map(|(id, model)| FreeModel {
            id: id.clone(),
            deprecated: model.get("status").and_then(Value::as_str) == Some("deprecated"),
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found.dedup_by(|a, b| a.id == b.id);
    Ok(found)
}

/// 解析 Zen 网关 `/models` 响应里的 id（OpenAI 风格 `data[].id`）。
pub fn parse_live_models(text: &str) -> Result<Vec<String>, String> {
    let root: Value = serde_json::from_str(text).map_err(|err| {
        let snippet = text.chars().take(120).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })?;
    let items = root
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "响应里没有 data 数组".to_string())?;
    let mut ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// 合并两个数据源：`live` 为 `Some` 时取交集（免费且网关在供）；
/// 为 `None`（网关请求失败）时退回「只信 models.dev，并排除 deprecated」。
fn combine(free: Vec<FreeModel>, live: Option<&[String]>) -> Vec<String> {
    let mut ids: Vec<String> = match live {
        Some(live) => free
            .into_iter()
            .filter(|model| live.iter().any(|id| id == &model.id))
            .map(|model| model.id)
            .collect(),
        None => free
            .into_iter()
            .filter(|model| !model.deprecated)
            .map(|model| model.id)
            .collect(),
    };
    ids.sort();
    ids.dedup();
    ids
}

/// 统一的 HTTP 客户端：`api.json` 有 4.8 MB，读超时按大文件放宽。
fn agent(read_secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(read_secs))
        .build()
}

/// 发起一次 GET 并取回正文，错误文本统一走人话映射。
fn get_text(url: &str, read_secs: u64) -> Result<String, String> {
    let response = agent(read_secs)
        .get(url)
        .set("Accept", "application/json")
        .set("User-Agent", "ModelHarbor")
        .call()
        .map_err(|err| match err {
            ureq::Error::Status(code, _) => crate::http_status::label(code),
            ureq::Error::Transport(transport) => format!(
                "网络错误：{}",
                crate::app::bars::sanitize_network_error(&transport.to_string())
            ),
        })?;
    response.into_string().map_err(|err| err.to_string())
}

/// 后台线程内执行：拉取价格与可用性两个数据源并求交集。
pub fn fetch_remote() -> Result<Vec<String>, String> {
    let free = parse_free_models(&get_text(SOURCE_URL, 60)?)?;
    // 网关列表很小（几 KB），拉不到就回退，不让整个流程失败。
    let live = get_text(LIVE_URL, 15)
        .ok()
        .and_then(|text| parse_live_models(&text).ok());
    Ok(combine(free, live.as_deref()))
}

/// 从指定路径读取缓存；文件缺失 / 读不出 / 结构不符都返回 `None`。
fn load_cache_at(path: &Path) -> Option<Cache> {
    let text = std::fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let models: Vec<String> = root
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|item| item.as_str())
        .map(str::to_string)
        .filter(|id| !id.trim().is_empty())
        .collect();
    if models.is_empty() {
        return None;
    }
    let fetched_at = root.get("fetched_at").and_then(Value::as_i64).unwrap_or(0);
    let age = unix_now().saturating_sub(fetched_at);
    Some(Cache {
        models,
        // 时间戳落在未来（改过系统时间）时按「刚取过」处理，不必重取。
        fresh: age < CACHE_TTL_SECS,
    })
}

/// 从指定路径写入缓存（原子写，失败只返回错误文本，不影响界面）。
fn save_cache_at(path: &Path, models: &[String]) -> Result<(), String> {
    let payload = serde_json::json!({
        "fetched_at": unix_now(),
        "models": models,
    });
    crate::util::atomic_write_text(path, &crate::app::pretty_json(&payload))
}

/// 读取默认路径的缓存。
pub fn load_cache() -> Option<Cache> {
    load_cache_at(&cache_path())
}

/// 把结果写入默认路径的缓存。
pub fn save_cache(models: &[String]) -> Result<(), String> {
    save_cache_at(&cache_path(), models)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一份最小 api.json：只有 opencode 厂商与几个各具代表性的模型。
    fn sample() -> String {
        serde_json::json!({
            "anthropic": { "models": { "claude-x": { "cost": { "input": 0, "output": 0 } } } },
            "opencode": {
                "models": {
                    "big-pickle":       { "cost": { "input": 0, "output": 0 } },
                    "some-free":        { "cost": { "input": 0, "output": 0, "cache_read": 0 } },
                    "retired-free":     { "cost": { "input": 0, "output": 0 }, "status": "deprecated" },
                    "paid-model":       { "cost": { "input": 5, "output": 25 } },
                    "free-in-paid-out": { "cost": { "input": 0, "output": 3 } },
                    "no-cost":          { "name": "Unknown" },
                    "only-input":       { "cost": { "input": 0 } }
                }
            }
        })
        .to_string()
    }

    /// Zen 风格的 `/models` 响应。
    fn live(ids: &[&str]) -> String {
        let data: Vec<Value> = ids
            .iter()
            .map(|id| serde_json::json!({ "id": id, "object": "model" }))
            .collect();
        serde_json::json!({ "object": "list", "data": data }).to_string()
    }

    fn free_ids(text: &str) -> Vec<String> {
        parse_free_models(text)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect()
    }

    #[test]
    fn keeps_only_zero_cost_models() {
        assert_eq!(
            free_ids(&sample()),
            vec!["big-pickle", "retired-free", "some-free"]
        );
    }

    #[test]
    fn missing_cost_is_not_treated_as_free() {
        let ids = free_ids(&sample());
        assert!(!ids.contains(&"no-cost".to_string()));
        // 只写了 input 价格、没有 output：按未知处理，不推荐
        assert!(!ids.contains(&"only-input".to_string()));
        // 单边为零不算免费
        assert!(!ids.contains(&"free-in-paid-out".to_string()));
        assert!(!ids.contains(&"paid-model".to_string()));
    }

    #[test]
    fn marks_deprecated_models() {
        let found = parse_free_models(&sample()).unwrap();
        let retired = found.iter().find(|m| m.id == "retired-free").unwrap();
        assert!(retired.deprecated);
        let live_one = found.iter().find(|m| m.id == "big-pickle").unwrap();
        assert!(!live_one.deprecated);
    }

    #[test]
    fn ignores_other_providers() {
        assert!(!free_ids(&sample()).contains(&"claude-x".to_string()));
    }

    #[test]
    fn reports_broken_payloads_without_panicking() {
        assert!(parse_free_models("not json").is_err());
        // 合法 JSON 但没有 opencode 厂商
        let err = parse_free_models(r#"{"anthropic":{"models":{}}}"#).unwrap_err();
        assert!(err.contains("opencode"), "错误应点明缺失的厂商：{}", err);
        // 有厂商但没有 models 容器
        assert!(parse_free_models(r#"{"opencode":{}}"#).is_err());
    }

    #[test]
    fn empty_result_is_ok_not_error() {
        let text = r#"{"opencode":{"models":{"x":{"cost":{"input":1,"output":2}}}}}"#;
        assert_eq!(free_ids(text), Vec::<String>::new());
    }

    #[test]
    fn parses_live_model_ids() {
        assert_eq!(
            parse_live_models(&live(&["b", "a", "a"])).unwrap(),
            vec!["a", "b"]
        );
        assert!(parse_live_models("not json").is_err());
        assert!(parse_live_models(r#"{"object":"list"}"#).is_err());
    }

    /// 两个源都在时取交集：免费且网关在供。
    #[test]
    fn intersects_free_with_live() {
        let free = parse_free_models(&sample()).unwrap();
        let ids = combine(
            free,
            Some(&["big-pickle".to_string(), "paid-model".to_string()]),
        );
        assert_eq!(ids, vec!["big-pickle"]);
    }

    /// 关键行为：被保守标记为 deprecated、但网关仍在供的模型要保留。
    #[test]
    fn keeps_deprecated_model_that_is_still_served() {
        let free = parse_free_models(&sample()).unwrap();
        let ids = combine(
            free,
            Some(&["retired-free".to_string(), "big-pickle".to_string()]),
        );
        assert!(
            ids.contains(&"retired-free".to_string()),
            "网关仍在提供就不该丢：{:?}",
            ids
        );
    }

    /// 网关拿不到时回退：只信 models.dev，并排除 deprecated。
    #[test]
    fn falls_back_to_models_dev_without_gateway() {
        let free = parse_free_models(&sample()).unwrap();
        assert_eq!(combine(free, None), vec!["big-pickle", "some-free"]);
    }

    /// 缓存往返：写入后能读回，且被判定为有效期内。
    #[test]
    fn cache_round_trip_marks_fresh() {
        let dir = std::env::temp_dir().join(format!(
            "modelharbor-free-models-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(CACHE_FILE);
        let models = vec!["big-pickle".to_string(), "some-free".to_string()];
        save_cache_at(&path, &models).expect("写缓存");
        let cache = load_cache_at(&path).expect("读回缓存");
        assert_eq!(cache.models, models);
        assert!(cache.fresh, "刚写入的缓存应在有效期内");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 过期缓存仍然返回列表，但标记为不新鲜（调用方据此后台刷新）。
    #[test]
    fn stale_cache_still_yields_models() {
        let dir = std::env::temp_dir().join(format!(
            "modelharbor-free-stale-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(CACHE_FILE);
        let old = unix_now() - CACHE_TTL_SECS - 60;
        std::fs::write(
            &path,
            serde_json::json!({ "fetched_at": old, "models": ["big-pickle"] }).to_string(),
        )
        .expect("写旧缓存");
        let cache = load_cache_at(&path).expect("读回过期缓存");
        assert_eq!(cache.models, vec!["big-pickle".to_string()]);
        assert!(!cache.fresh, "超过 TTL 的缓存不应标记为新鲜");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 损坏 / 空缓存当作没有缓存，不 panic。
    #[test]
    fn broken_cache_is_ignored() {
        let dir = std::env::temp_dir().join(format!(
            "modelharbor-free-broken-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(CACHE_FILE);
        std::fs::write(&path, "{ not json").expect("写坏缓存");
        assert!(load_cache_at(&path).is_none());
        std::fs::write(&path, r#"{"fetched_at":1,"models":[]}"#).expect("写空列表");
        assert!(load_cache_at(&path).is_none(), "空列表等于没有缓存");
        assert!(load_cache_at(&dir.join("does-not-exist.json")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
