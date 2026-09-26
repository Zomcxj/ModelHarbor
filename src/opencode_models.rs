//! opencode 系的「内置网关免费模型」列表：按后端动态获取，避免下拉里留下过期的模型 id。
//!
//! ## 为什么需要它
//!
//! Agents 的 `model` 字段取值形如 `provider/model`，其中 `provider` 是**网关自带的
//! provider id**（不是用户自己配的 provider key）。opencode 与 Kilo Code 各自有一个
//! 官方网关，都提供免费模型；免费层会随上游上下架，写死在界面里的 id 迟早变成
//! 「选了却跑不起来」的过期项，所以改为按需拉取。
//!
//! ## 各后端的来源
//!
//! | 后端 | provider id | 免费模型来源 | 判定 |
//! |---|---|---|---|
//! | opencode | `opencode` | models.dev 的 `opencode` 厂商 + Zen 网关可用性 | 价格为零 **且** 网关在供 |
//! | kilocode | `kilo` | Kilo 网关 `api.kilo.ai/api/gateway/models` | 响应里的 `isFree` 字段 |
//! | mimocode | `mimo` / `xiaomi` | **无免费层**，列表恒为空 | — |
//!
//! **MiMo Code 为什么没有**：models.dev 上它对应的 `xiaomi` 厂商 9 个模型全部收费；
//! 免费的那些挂在 `xiaomi-token-plan-{cn,sgp,ams}` 下，那是**订阅套餐**（Token Plan）
//! 而不是免费层，与「不花钱就能用」不是一回事。所以 mimocode 页不提供免费模型候选，
//! 只列用户自己配的 provider 模型。
//!
//! ## opencode 为什么要两个源求交
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
//! 网关请求失败时回退为「只信 models.dev，并排除 deprecated 标记的条目」——
//! 拿不到实测依据时，宁可少列几个。
//!
//! 免费判定：`cost.input == 0 && cost.output == 0`，两者都必须是**显式的 0**；
//! 字段缺失说明数据源还没收录价格，按收费处理（宁可少列，也不要把收费模型推荐出去）。
//!
//! ## 缓存
//!
//! `api.json` 未压缩约 4.8 MB（gzip 后约 470 KB），每次启动都下载太浪费，因此
//! 结果按后端分别落盘到 `.modelharbor/free-models-<后端>.json`，超过
//! [`CACHE_TTL_SECS`] 才在后台重新拉取。缓存只存模型 id 列表，不含任何凭证。

use crate::format::ConfigFormat;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 价格数据源（models.dev 全量模型库）。
pub const SOURCE_URL: &str = "https://models.dev/api.json";

/// opencode Zen 网关的模型列表（可用性数据源）。
pub const OPENCODE_LIVE_URL: &str = "https://opencode.ai/zen/v1/models";

/// Kilo 网关的模型列表（含 `isFree` 标记）。
pub const KILO_LIVE_URL: &str = "https://api.kilo.ai/api/gateway/models";

/// 缓存有效期（秒）：超过则在后台重新拉取。24 小时。
///
/// 免费模型上下架不是分钟级事件，一天一次足够；界面上的「刷新」按钮可随时强制重取。
pub const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// 某个后端的免费模型配置。`None` 表示该后端没有免费层。
pub struct Source {
    /// 该后端的网关 provider id（agent 里 `provider/model` 的前半段）。
    pub provider_id: &'static str,
    /// 数据源说明，用于悬停提示。
    pub origin: &'static str,
}

/// 取某个后端的免费模型来源；没有免费层时返回 `None`。
///
/// opencode 系三页共用 Agents 实现，但只有 opencode 与 kilocode 有官方免费网关。
pub fn source_for(format: ConfigFormat) -> Option<Source> {
    match format {
        ConfigFormat::Opencode => Some(Source {
            provider_id: "opencode",
            origin: SOURCE_URL,
        }),
        ConfigFormat::Kilocode => Some(Source {
            provider_id: "kilo",
            origin: KILO_LIVE_URL,
        }),
        // MiMo Code：没有免费层（见模块说明），不提供候选。
        _ => None,
    }
}

/// 某个 opencode 系页面**自家网关**认的 provider id（agent `model` 的前半段）。
///
/// 与 [`source_for`] 的区别：`source_for` 只回答「免费层从哪拉」，这里回答
/// 「哪些前缀在本页算自家」。两者不是一回事——mimocode 没有免费层，却同样有自己的
/// 网关；而 agent 的 `model` 若指向别家网关（如把 `opencode/ling-3.0-flash-fin-free`
/// 写进 kilo.json），kilo 网关根本不认这个前缀，agent 跑不起来。
///
/// MiMo Code 有两个：`mimo/`（网关自身的自动路由）与 `xiaomi/`（小米模型族）。
pub fn gateway_provider_ids(format: ConfigFormat) -> &'static [&'static str] {
    match format {
        ConfigFormat::Opencode => &["opencode"],
        ConfigFormat::Kilocode => &["kilo"],
        ConfigFormat::Mimocode => &["mimo", "xiaomi"],
        _ => &[],
    }
}

/// 某页网关的首选模型：替换 agent 的 model 时的目标，也是下拉里的第一项。
///
/// 一律选**自动路由**类 id（kilo 的 `kilo-auto/free`、mimo 的 `mimo/mimo-auto`）：
/// 这类 id 由网关自己挑后端，不会因为某个具体模型上下架而失效，正适合当兜底默认值。
/// opencode 没有自动路由，用它最经典的免费模型 `big-pickle`。
fn preferred_gateway_model(format: ConfigFormat) -> Option<&'static str> {
    match format {
        ConfigFormat::Opencode => Some("opencode/big-pickle"),
        ConfigFormat::Kilocode => Some("kilo/kilo-auto/free"),
        ConfigFormat::Mimocode => Some("mimo/mimo-auto"),
        _ => None,
    }
}

/// 动态列表拿不到时用的内置兜底清单（**已带 `provider/` 前缀**）。
///
/// 只在动态列表为空时生效。写死 id 会过期，所以这里只放最稳的那几个；
/// 有动态来源的页面一旦拉到列表就以动态为准（见 [`gateway_models`]）。
fn builtin_gateway_models(format: ConfigFormat) -> &'static [&'static str] {
    match format {
        ConfigFormat::Opencode => &["opencode/big-pickle"],
        ConfigFormat::Kilocode => &["kilo/kilo-auto/free"],
        // MiMo Code 的网关模型（`mimo models` 的全量输出），顺序即官方顺序。
        ConfigFormat::Mimocode => &[
            "mimo/mimo-auto",
            "xiaomi/mimo-v2.5",
            "xiaomi/mimo-v2.5-pro",
            "xiaomi/mimo-v2.5-pro-ultraspeed",
            "xiaomi/mimo-v2.6-flash",
            "xiaomi/mimo-v2.6-pro",
            "xiaomi/mimo-v2.6-pro-ultraspeed",
        ],
        _ => &[],
    }
}

/// 某页自家网关的模型候选（已带 `provider/` 前缀，**首选在最前**）。
///
/// `free` 是该页动态拉到的免费模型裸 id（见 [`Source::provider_id`]）。有动态列表就
/// 以它为准，为空才退回内置兜底——否则「切页即替换」会因为列表还没拉回来而静默失效。
///
/// 排序：首选（自动路由）第一，其余按字典序。下拉的「前几家」因此就是最稳的那几个，
/// 替换时取第一个也才是对的。
pub fn gateway_models(format: ConfigFormat, free: &[String]) -> Vec<String> {
    let mut models: Vec<String> = match source_for(format) {
        Some(source) if !free.is_empty() => free
            .iter()
            .map(|id| format!("{}/{}", source.provider_id, id))
            .collect(),
        _ => builtin_gateway_models(format)
            .iter()
            .map(|id| id.to_string())
            .collect(),
    };
    models.sort();
    models.dedup();
    if let Some(preferred) = preferred_gateway_model(format) {
        if let Some(at) = models.iter().position(|m| m == preferred) {
            let head = models.remove(at);
            models.insert(0, head);
        }
    }
    models
}

/// 把 agent 的 model 换成自家网关模型时的目标：候选列表的第一个。
pub fn default_gateway_model(format: ConfigFormat, free: &[String]) -> Option<String> {
    gateway_models(format, free).into_iter().next()
}

/// `model` 引用（`provider/model`）的 provider 前缀；没有斜杠时返回 `None`。
pub fn model_provider_prefix(model: &str) -> Option<&str> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let (prefix, rest) = model.split_once('/')?;
    let prefix = prefix.trim();
    if prefix.is_empty() || rest.trim().is_empty() {
        None
    } else {
        Some(prefix)
    }
}

/// `model` 引用在指定页面上是否有效：前缀要么是本页网关，要么是用户自己配的 provider。
///
/// 判据与保存时的实际结果一致——opencode 系三页共用同一份 provider 列表，保存时
/// provider 容器会一并写进目标文件，所以「已配置的 provider key」在任何一页都有效；
/// 只有**别家网关**的前缀（本页既没这个网关、也没这个 provider）才是无效的。
///
/// 前缀解析不出来（没有斜杠 / 空）一律算无效：这种引用在任何网关上都无法解析，
/// 换成自家网关模型是修正而不是破坏。
pub fn model_is_valid_on(page: ConfigFormat, configured_keys: &[String], model: &str) -> bool {
    let Some(prefix) = model_provider_prefix(model) else {
        return false;
    };
    gateway_provider_ids(page).contains(&prefix) || configured_keys.iter().any(|key| key == prefix)
}

/// 落盘缓存的内容（`fetched_at` + 模型 id 列表）。
pub struct Cache {
    pub models: Vec<String>,
    /// 是否仍在有效期内（过期也照样返回 `models`，由调用方决定要不要后台刷新）。
    pub fresh: bool,
}

/// 当前时间（秒级 Unix 时间戳）。
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 缓存文件名：按后端区分，避免两个页面的列表互相覆盖。
///
/// 用 `label()` 作标识（它同时是页签顺序等持久化状态用的稳定 key）。
fn cache_file(format: ConfigFormat) -> String {
    format!("free-models-{}.json", format.label())
}

/// 某个后端的缓存文件路径。
pub fn cache_path(format: ConfigFormat) -> PathBuf {
    crate::prefs::Prefs::config_dir().join(cache_file(format))
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

/// 解析响应 JSON，失败时给出**带开头片段**的错误。
///
/// 片段只取 120 个字符：models.dev 的正文约 4.8 MB，整个塞进提示里既没人看也拖慢界面。
/// 三个解析器（models.dev / 网关 `/models` / Kilo 网关）共用这一段——错误文案必须一致，
/// 否则同一个网络故障在不同后端下会显示成不同的话。
pub(crate) fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|err| {
        let snippet = text.chars().take(120).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })
}

/// 取响应里的 `data` 数组（OpenAI 风格的 `/models` 响应）。
fn data_items(root: &Value) -> Result<&Vec<Value>, String> {
    root.get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "响应里没有 data 数组".to_string())
}

/// 从 `data[].id` 收集模型 id，排序去重。
fn ids_from_data(items: &[Value], keep: impl Fn(&Value) -> bool) -> Vec<String> {
    let mut ids: Vec<String> = items
        .iter()
        .filter(|item| keep(item))
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// 从 models.dev 的 `api.json` 全文里筛出指定厂商的免费模型。
pub fn parse_models_dev_free(text: &str, provider: &str) -> Result<Vec<FreeModel>, String> {
    let root = parse_json(text)?;
    let provider = root
        .get(provider)
        .ok_or_else(|| format!("数据里没有 {} 厂商", provider))?;
    let models = provider
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} 厂商下没有 models", provider))?;
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

/// 解析 OpenAI 风格 `/models` 响应里的 id（`data[].id`）。
pub fn parse_live_models(text: &str) -> Result<Vec<String>, String> {
    let root = parse_json(text)?;
    Ok(ids_from_data(data_items(&root)?, |_| true))
}

/// 解析 Kilo 网关响应里 `isFree == true` 的模型 id。
///
/// 用响应自带的 `isFree` 字段而不是「id 以 `:free` 结尾」或价格推断：
/// `kilo-auto/free` 与 `openrouter/free` 两个免费项并不带 `:free` 后缀，
/// 按后缀筛会漏掉它们。
pub fn parse_kilo_free(text: &str) -> Result<Vec<String>, String> {
    let root = parse_json(text)?;
    Ok(ids_from_data(data_items(&root)?, |item| {
        item.get("isFree").and_then(Value::as_bool) == Some(true)
    }))
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
pub(crate) fn get_text(url: &str, read_secs: u64) -> Result<String, String> {
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

/// 后台线程内执行：按后端拉取免费模型列表。
///
/// 没有免费层的后端直接返回空列表（不是错误）。
pub fn fetch_remote(format: ConfigFormat) -> Result<Vec<String>, String> {
    match format {
        // opencode：models.dev 的价格 + Zen 网关的可用性，两者求交。
        ConfigFormat::Opencode => {
            let free = parse_models_dev_free(&get_text(SOURCE_URL, 60)?, "opencode")?;
            // 网关列表很小（几 KB），拉不到就回退，不让整个流程失败。
            let live = get_text(OPENCODE_LIVE_URL, 15)
                .ok()
                .and_then(|text| parse_live_models(&text).ok());
            Ok(combine(free, live.as_deref()))
        }
        // Kilo：网关响应自带 isFree，一个请求即可。
        ConfigFormat::Kilocode => parse_kilo_free(&get_text(KILO_LIVE_URL, 60)?),
        // 其余后端没有免费层。
        _ => Ok(Vec::new()),
    }
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

/// 读取某个后端的缓存。
pub fn load_cache(format: ConfigFormat) -> Option<Cache> {
    load_cache_at(&cache_path(format))
}

/// 把结果写入某个后端的缓存。
pub fn save_cache(format: ConfigFormat, models: &[String]) -> Result<(), String> {
    save_cache_at(&cache_path(format), models)
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

    /// Kilo 风格的网关响应：`isFree` 决定免费，`id` 自带斜杠与 `:free` 后缀不一定有。
    fn kilo_live() -> String {
        serde_json::json!({
            "data": [
                { "id": "cohere/north-mini-code:free", "isFree": true },
                { "id": "kilo-auto/free", "isFree": true },
                { "id": "openrouter/free", "isFree": true },
                { "id": "anthropic/claude-opus-5", "isFree": false },
                { "id": "qwen/qwen3.7-max" },
                { "id": "kilo-auto/efficient", "isFree": false }
            ]
        })
        .to_string()
    }

    fn free_ids(text: &str) -> Vec<String> {
        parse_models_dev_free(text, "opencode")
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
        let found = parse_models_dev_free(&sample(), "opencode").unwrap();
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
        assert!(parse_models_dev_free("not json", "opencode").is_err());
        // 合法 JSON 但没有目标厂商
        let err = parse_models_dev_free(r#"{"anthropic":{"models":{}}}"#, "opencode").unwrap_err();
        assert!(err.contains("opencode"), "错误应点明缺失的厂商：{}", err);
        // 有厂商但没有 models 容器
        assert!(parse_models_dev_free(r#"{"opencode":{}}"#, "opencode").is_err());
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
        let free = parse_models_dev_free(&sample(), "opencode").unwrap();
        let ids = combine(
            free,
            Some(&["big-pickle".to_string(), "paid-model".to_string()]),
        );
        assert_eq!(ids, vec!["big-pickle"]);
    }

    /// 关键行为：被保守标记为 deprecated、但网关仍在供的模型要保留。
    #[test]
    fn keeps_deprecated_model_that_is_still_served() {
        let free = parse_models_dev_free(&sample(), "opencode").unwrap();
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
        let free = parse_models_dev_free(&sample(), "opencode").unwrap();
        assert_eq!(combine(free, None), vec!["big-pickle", "some-free"]);
    }

    /// Kilo 按 `isFree` 判定：`kilo-auto/free`、`openrouter/free` 不带 `:free`
    /// 后缀但确实是免费的，不能漏。
    #[test]
    fn kilo_free_uses_the_is_free_flag() {
        let ids = parse_kilo_free(&kilo_live()).unwrap();
        assert_eq!(
            ids,
            vec![
                "cohere/north-mini-code:free",
                "kilo-auto/free",
                "openrouter/free"
            ]
        );
    }

    /// Kilo 的付费项与未标注 `isFree` 的项都不算免费。
    #[test]
    fn kilo_excludes_paid_and_unflagged_models() {
        let ids = parse_kilo_free(&kilo_live()).unwrap();
        assert!(!ids.contains(&"anthropic/claude-opus-5".to_string()));
        assert!(!ids.contains(&"qwen/qwen3.7-max".to_string()));
        assert!(!ids.contains(&"kilo-auto/efficient".to_string()));
    }

    #[test]
    fn kilo_reports_broken_payloads() {
        assert!(parse_kilo_free("not json").is_err());
        assert!(parse_kilo_free(r#"{"object":"list"}"#).is_err());
        // 空列表是合法结果，不是错误
        assert_eq!(
            parse_kilo_free(r#"{"data":[]}"#).unwrap(),
            Vec::<String>::new()
        );
    }

    /// 各后端的 provider id 与「有没有免费层」必须与实测一致。
    #[test]
    fn only_opencode_and_kilocode_have_free_tiers() {
        assert_eq!(
            source_for(ConfigFormat::Opencode).unwrap().provider_id,
            "opencode"
        );
        assert_eq!(
            source_for(ConfigFormat::Kilocode).unwrap().provider_id,
            "kilo"
        );
        // MiMo Code 没有免费层：9 个模型全收费，免费的那些属于订阅套餐
        assert!(source_for(ConfigFormat::Mimocode).is_none());
        // 非 opencode 系后端同样没有
        assert!(source_for(ConfigFormat::WorkBuddy).is_none());
        assert!(source_for(ConfigFormat::ZCode).is_none());
    }

    /// 各后端的缓存文件互不覆盖。
    #[test]
    fn cache_files_are_per_backend() {
        let a = cache_file(ConfigFormat::Opencode);
        let b = cache_file(ConfigFormat::Kilocode);
        assert_ne!(a, b);
        assert!(a.contains("opencode") && b.contains("kilocode"));
    }

    /// 每页认的自家网关前缀必须与各 CLI 实测一致（`kilo models` / `mimo models`）。
    #[test]
    fn gateway_provider_ids_match_the_real_clis() {
        assert_eq!(gateway_provider_ids(ConfigFormat::Opencode), ["opencode"]);
        assert_eq!(gateway_provider_ids(ConfigFormat::Kilocode), ["kilo"]);
        // MiMo Code 有两个：网关自身的自动路由 + 小米模型族
        assert_eq!(
            gateway_provider_ids(ConfigFormat::Mimocode),
            ["mimo", "xiaomi"]
        );
        // 非 opencode 系没有 agent 概念，也就没有自家网关
        assert!(gateway_provider_ids(ConfigFormat::WorkBuddy).is_empty());
        assert!(gateway_provider_ids(ConfigFormat::ZCode).is_empty());
    }

    /// 动态列表为空时必须有兜底，否则「切页即替换」会因为列表没拉回来而静默失效。
    #[test]
    fn every_opencode_family_page_has_a_gateway_default() {
        for page in [
            ConfigFormat::Opencode,
            ConfigFormat::Kilocode,
            ConfigFormat::Mimocode,
        ] {
            let default = default_gateway_model(page, &[]);
            assert!(default.is_some(), "{} 页缺兜底模型", page.label());
            let default = default.unwrap();
            let prefix = model_provider_prefix(&default).unwrap();
            assert!(
                gateway_provider_ids(page).contains(&prefix),
                "{} 页的兜底 {} 不属于自家网关",
                page.label(),
                default
            );
        }
    }

    /// 兜底值必须是各页**最稳**的那个 id：自动路由优先。
    #[test]
    fn gateway_defaults_prefer_auto_routing_models() {
        assert_eq!(
            default_gateway_model(ConfigFormat::Kilocode, &[]).unwrap(),
            "kilo/kilo-auto/free"
        );
        assert_eq!(
            default_gateway_model(ConfigFormat::Mimocode, &[]).unwrap(),
            "mimo/mimo-auto"
        );
        assert_eq!(
            default_gateway_model(ConfigFormat::Opencode, &[]).unwrap(),
            "opencode/big-pickle"
        );
    }

    /// 动态列表拿到后以它为准（写死的兜底不能盖过实测列表）。
    #[test]
    fn live_free_models_win_over_the_builtin_fallback() {
        let live = vec!["zzz-free".to_string(), "aaa-free".to_string()];
        let models = gateway_models(ConfigFormat::Kilocode, &live);
        // 首选（kilo-auto/free）不在动态列表里，就不该被硬塞进来
        assert_eq!(models, vec!["kilo/aaa-free", "kilo/zzz-free"]);
        assert!(!models.iter().any(|m| m == "kilo/kilo-auto/free"));
    }

    /// 首选若在动态列表里，必须被提到第一位——下拉的「前几家」才有意义。
    #[test]
    fn the_preferred_model_is_promoted_to_the_front() {
        let live = vec![
            "aaa-free".to_string(),
            "big-pickle".to_string(),
            "zzz-free".to_string(),
        ];
        let models = gateway_models(ConfigFormat::Opencode, &live);
        assert_eq!(models.first().unwrap(), "opencode/big-pickle");
        // 其余仍按字典序，且不重复
        assert_eq!(
            models,
            vec![
                "opencode/big-pickle",
                "opencode/aaa-free",
                "opencode/zzz-free"
            ]
        );
    }

    /// mimo 页的兜底清单必须与 `mimo models` 的实测输出逐条一致。
    #[test]
    fn mimocode_fallback_matches_the_cli_output() {
        let models = gateway_models(ConfigFormat::Mimocode, &[]);
        assert_eq!(
            models,
            vec![
                "mimo/mimo-auto",
                "xiaomi/mimo-v2.5",
                "xiaomi/mimo-v2.5-pro",
                "xiaomi/mimo-v2.5-pro-ultraspeed",
                "xiaomi/mimo-v2.6-flash",
                "xiaomi/mimo-v2.6-pro",
                "xiaomi/mimo-v2.6-pro-ultraspeed",
            ]
        );
    }

    /// 前缀解析：没有斜杠、空段、只有斜杠都要判成「解析不出来」。
    #[test]
    fn model_prefix_parsing_rejects_malformed_references() {
        assert_eq!(model_provider_prefix("kilo/kilo-auto/free"), Some("kilo"));
        assert_eq!(
            model_provider_prefix("  xiaomi/mimo-v2.5  "),
            Some("xiaomi")
        );
        assert_eq!(model_provider_prefix(""), None);
        assert_eq!(model_provider_prefix("   "), None);
        assert_eq!(model_provider_prefix("no-slash"), None);
        assert_eq!(model_provider_prefix("/model"), None);
        assert_eq!(model_provider_prefix("provider/"), None);
    }

    /// 核心判据：别家网关的前缀在本页无效，自家网关与自配 provider 都有效。
    #[test]
    fn validity_is_about_the_pages_own_gateway() {
        let configured = vec!["sensenova".to_string()];
        // 本页网关
        assert!(model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "kilo/kilo-auto/free"
        ));
        // 自配 provider：保存时 provider 容器一并写入，任何一页都有效
        assert!(model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "sensenova/deepseek-v4-flash"
        ));
        // 别家网关：kilo 网关不认 opencode/ 前缀 —— 这正是要替换掉的情形
        assert!(!model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "opencode/ling-3.0-flash-fin-free"
        ));
        // 同一份引用在 opencode 页是有效的（对照）
        assert!(model_is_valid_on(
            ConfigFormat::Opencode,
            &configured,
            "opencode/ling-3.0-flash-fin-free"
        ));
        // mimo 页认 mimo/ 与 xiaomi/，不认 kilo/
        assert!(model_is_valid_on(
            ConfigFormat::Mimocode,
            &[],
            "xiaomi/mimo-v2.5-pro"
        ));
        assert!(!model_is_valid_on(
            ConfigFormat::Mimocode,
            &[],
            "kilo/kilo-auto/free"
        ));
    }

    /// 解析不出来的引用一律算无效：换成自家网关模型是修正，不是破坏。
    #[test]
    fn malformed_references_are_never_valid() {
        for page in [
            ConfigFormat::Opencode,
            ConfigFormat::Kilocode,
            ConfigFormat::Mimocode,
        ] {
            assert!(!model_is_valid_on(page, &[], ""));
            assert!(!model_is_valid_on(page, &[], "no-slash"));
            assert!(!model_is_valid_on(
                page,
                &["no-slash".to_string()],
                "no-slash"
            ));
        }
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
        let path = dir.join("free-models-opencode.json");
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
        let path = dir.join("free-models-opencode.json");
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
        let path = dir.join("free-models-opencode.json");
        std::fs::write(&path, "{ not json").expect("写坏缓存");
        assert!(load_cache_at(&path).is_none());
        std::fs::write(&path, r#"{"fetched_at":1,"models":[]}"#).expect("写空列表");
        assert!(load_cache_at(&path).is_none(), "空列表等于没有缓存");
        assert!(load_cache_at(&dir.join("does-not-exist.json")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
