//! opencode 系的「内置网关免费模型」列表：按后端动态获取。
//!
//! opencode 与 kilocode 有官方免费网关，mimocode 无免费层。免费判定为
//! `cost.input == 0 && cost.output == 0`（两者都须是显式的 0，字段缺失按收费处理）；
//! 网关请求失败时只信 models.dev，并排除 `deprecated` 条目。
//!
//! 结果按后端落盘到 `.modelharbor/free-models-<后端>.json`，只存模型 id，超过
//! [`CACHE_TTL_SECS`] 才在后台重新拉取。

/// 价格数据源（models.dev 全量模型库）。
mod net;
mod parse;

pub use net::{cache_path, fetch_remote, load_cache, save_cache, Cache, CACHE_TTL_SECS};
pub use parse::{parse_kilo_free, parse_live_models, parse_models_dev_free, FreeModel};

// 扁平门面（仅测试）：把子模块项平铺进本模块命名空间，供 `use super::*` 使用。
#[cfg(test)]
use net::*;
#[cfg(test)]
use parse::*;

use crate::format::ConfigFormat;
#[cfg(test)]
use serde_json::Value;

pub const SOURCE_URL: &str = "https://models.dev/api.json";

/// opencode Zen 网关的模型列表（可用性数据源）。
pub const OPENCODE_LIVE_URL: &str = "https://opencode.ai/zen/v1/models";

/// Kilo 网关的模型列表（含 `isFree` 标记）。
pub const KILO_LIVE_URL: &str = "https://api.kilo.ai/api/gateway/models";

/// 某个后端的免费模型配置。`None` 表示该后端没有免费层。
pub struct Source {
    /// 该后端的网关 provider id（agent 里 `provider/model` 的前半段）。
    pub provider_id: &'static str,
    /// 数据源说明，用于悬停提示。
    pub origin: &'static str,
}

/// 取某个后端的免费模型来源；没有免费层时返回 `None`。
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
        // MiMo Code 没有免费层，不提供候选。
        _ => None,
    }
}

/// 某个 opencode 系页面**自家网关**认的 provider id（agent `model` 的前半段）。
///
/// 与 [`source_for`] 无关：这里回答「哪些前缀在本页算自家」。
/// MiMo Code 有两个：`mimo/`（自动路由）与 `xiaomi/`（小米模型族）。
pub fn gateway_provider_ids(format: ConfigFormat) -> &'static [&'static str] {
    match format {
        ConfigFormat::Opencode => &["opencode"],
        ConfigFormat::Kilocode => &["kilo"],
        ConfigFormat::Mimocode => &["mimo", "xiaomi"],
        _ => &[],
    }
}

/// 某页网关的首选模型：替换 agent 的 model 的目标，也是下拉第一项。
///
/// kilo 用 `kilo-auto/free`、mimo 用 `mimo/mimo-auto`（自动路由），
/// opencode 用 `big-pickle`。
fn preferred_gateway_model(format: ConfigFormat) -> Option<&'static str> {
    match format {
        ConfigFormat::Opencode => Some("opencode/big-pickle"),
        ConfigFormat::Kilocode => Some("kilo/kilo-auto/free"),
        ConfigFormat::Mimocode => Some("mimo/mimo-auto"),
        _ => None,
    }
}

/// 动态列表为空时用的内置兜底清单（已带 `provider/` 前缀）。
fn builtin_gateway_models(format: ConfigFormat) -> &'static [&'static str] {
    match format {
        ConfigFormat::Opencode => &["opencode/big-pickle"],
        ConfigFormat::Kilocode => &["kilo/kilo-auto/free"],
        // MiMo Code 的网关模型，顺序即官方顺序。
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

/// 某页自家网关的模型候选（已带 `provider/` 前缀，首选在最前）。
///
/// `free` 是该页动态拉到的免费模型裸 id（见 [`Source::provider_id`]）；非空时以它
/// 为准，为空时退回内置兜底。排序：首选第一，其余按字典序。
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

/// `model` 引用在指定页面上是否有效：前缀要么是本页网关，要么是用户自配的
/// provider。
///
/// 前缀解析不出来（没有斜杠 / 空）一律算无效。
pub fn model_is_valid_on(page: ConfigFormat, configured_keys: &[String], model: &str) -> bool {
    let Some(prefix) = model_provider_prefix(model) else {
        return false;
    };
    gateway_provider_ids(page).contains(&prefix) || configured_keys.iter().any(|key| key == prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一份最小 api.json：只有 opencode 厂商与几个模型。
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

    /// Kilo 风格的网关响应：`isFree` 决定免费，`id` 不一定带 `:free` 后缀。
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

    /// 标记为 deprecated、但网关仍在供的模型要保留。
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

    /// Kilo 按 `isFree` 判定：不带 `:free` 后缀的免费项也要收。
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

    /// 各后端的 provider id 与免费层支持情况。
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
        // MiMo Code 没有免费层
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

    /// 每页认的自家网关前缀。
    #[test]
    fn gateway_provider_ids_match_the_real_clis() {
        assert_eq!(gateway_provider_ids(ConfigFormat::Opencode), ["opencode"]);
        assert_eq!(gateway_provider_ids(ConfigFormat::Kilocode), ["kilo"]);
        // MiMo Code 有两个：自动路由 + 小米模型族
        assert_eq!(
            gateway_provider_ids(ConfigFormat::Mimocode),
            ["mimo", "xiaomi"]
        );
        // 非 opencode 系没有自家网关
        assert!(gateway_provider_ids(ConfigFormat::WorkBuddy).is_empty());
        assert!(gateway_provider_ids(ConfigFormat::ZCode).is_empty());
    }

    /// 动态列表为空时必须有兜底模型。
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

    /// 兜底值取各页的自动路由 id。
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

    /// 有动态列表时以它为准，不掺入兜底项。
    #[test]
    fn live_free_models_win_over_the_builtin_fallback() {
        let live = vec!["zzz-free".to_string(), "aaa-free".to_string()];
        let models = gateway_models(ConfigFormat::Kilocode, &live);
        // 首选不在动态列表里时不加入
        assert_eq!(models, vec!["kilo/aaa-free", "kilo/zzz-free"]);
        assert!(!models.iter().any(|m| m == "kilo/kilo-auto/free"));
    }

    /// 首选在动态列表里时被提到第一位。
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

    /// mimo 页的兜底清单与顺序。
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

    /// 别家网关的前缀在本页无效，自家网关与自配 provider 有效。
    #[test]
    fn validity_is_about_the_pages_own_gateway() {
        let configured = vec!["sensenova".to_string()];
        // 本页网关
        assert!(model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "kilo/kilo-auto/free"
        ));
        // 自配 provider 在任何一页都有效
        assert!(model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "sensenova/deepseek-v4-flash"
        ));
        // 别家网关前缀无效
        assert!(!model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "opencode/ling-3.0-flash-fin-free"
        ));
        // 同一份引用在 opencode 页有效
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

    /// 解析不出来的引用一律算无效。
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

    /// 过期缓存仍返回列表，但标记为不新鲜。
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
