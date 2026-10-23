//! The opencode family's "built-in gateway free models" list: fetched dynamically per backend.
//!
//! opencode and kilocode have official free gateways; mimocode has no free tier. Free means
//! `cost.input == 0 && cost.output == 0` (both must be explicit zeros; missing fields count as paid);
//! when the gateway request fails, only models.dev is trusted, and `deprecated` entries are excluded.
//!
//! Results are written per backend to `.modelharbor/free-models-<backend>.json`, storing model ids only;
//! past [`CACHE_TTL_SECS`] they are refreshed in the background.

/// Pricing data source (models.dev's full model library).
mod net;
mod parse;

pub use net::{cache_path, fetch_remote, load_cache, save_cache, Cache, CACHE_TTL_SECS};
pub use parse::{parse_kilo_free, parse_live_models, parse_models_dev_free, FreeModel};

// Flat facade (tests only): flatten submodule items into this module's namespace for `use super::*`.
#[cfg(test)]
use net::*;
#[cfg(test)]
use parse::*;

use crate::format::ConfigFormat;
#[cfg(test)]
use serde_json::Value;

pub const SOURCE_URL: &str = "https://models.dev/api.json";

/// The opencode Zen gateway's model list (availability data source).
pub const OPENCODE_LIVE_URL: &str = "https://opencode.ai/zen/v1/models";

/// The Kilo gateway's model list (carries an `isFree` flag).
pub const KILO_LIVE_URL: &str = "https://api.kilo.ai/api/gateway/models";

/// One backend's free-model source. `None` = that backend has no free tier.
pub struct Source {
    /// The backend's gateway provider id (the first half of `provider/model` in agents).
    pub provider_id: &'static str,
    /// The data source's description, for hover tips.
    pub origin: &'static str,
}

/// Take a backend's free-model source; `None` when there is no free tier.
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
        // MiMo Code has no free tier and offers no candidates.
        _ => None,
    }
}

/// The provider ids **this page's own gateway** accepts for an opencode-family page (the first half of an agent's `model`).
///
/// Unrelated to [`source_for`]: this answers "which prefixes count as native on this page".
/// MiMo Code has two: `mimo/` (auto routing) and `xiaomi/` (the Xiaomi model family).
pub fn gateway_provider_ids(format: ConfigFormat) -> &'static [&'static str] {
    match format {
        ConfigFormat::Opencode => &["opencode"],
        ConfigFormat::Kilocode => &["kilo"],
        ConfigFormat::Mimocode => &["mimo", "xiaomi"],
        _ => &[],
    }
}

/// A page gateway's preferred model: the replacement target for an agent's model, and the dropdown's first item.
///
/// kilo uses `kilo-auto/free`, mimo uses `mimo/mimo-auto` (auto routing), opencode uses `big-pickle`.
fn preferred_gateway_model(format: ConfigFormat) -> Option<&'static str> {
    match format {
        ConfigFormat::Opencode => Some("opencode/big-pickle"),
        ConfigFormat::Kilocode => Some("kilo/kilo-auto/free"),
        ConfigFormat::Mimocode => Some("mimo/mimo-auto"),
        _ => None,
    }
}

/// Built-in fallback list used when the dynamic list is empty (already carrying `provider/` prefixes).
fn builtin_gateway_models(format: ConfigFormat) -> &'static [&'static str] {
    match format {
        ConfigFormat::Opencode => &["opencode/big-pickle"],
        ConfigFormat::Kilocode => &["kilo/kilo-auto/free"],
        // MiMo Code's gateway models, in official order.
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

/// Model candidates of a page's own gateway (with `provider/` prefixes, preferred first).
///
/// `free` is the bare id of the page's dynamically fetched free model (see [`Source::provider_id`]);
/// non-empty takes precedence, empty falls back to the built-in list. Ordering: preferred first, the
/// rest lexicographic.
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

/// The target when swapping an agent's model to its own gateway model: the first candidate.
pub fn default_gateway_model(format: ConfigFormat, free: &[String]) -> Option<String> {
    gateway_models(format, free).into_iter().next()
}

/// The provider prefix of a `model` reference (`provider/model`); `None` without a slash.
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

/// Whether a `model` reference is valid on a given page: the prefix is either this page's gateway or a user-configured provider.
///
/// Unparseable prefixes (no slash / empty) are always invalid.
pub fn model_is_valid_on(page: ConfigFormat, configured_keys: &[String], model: &str) -> bool {
    let Some(prefix) = model_provider_prefix(model) else {
        return false;
    };
    gateway_provider_ids(page).contains(&prefix) || configured_keys.iter().any(|key| key == prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal api.json: only the opencode vendor and a few models.
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

    /// A Zen-style `/models` response.
    fn live(ids: &[&str]) -> String {
        let data: Vec<Value> = ids
            .iter()
            .map(|id| serde_json::json!({ "id": id, "object": "model" }))
            .collect();
        serde_json::json!({ "object": "list", "data": data }).to_string()
    }

    /// A Kilo-style gateway response: `isFree` decides free; `id` doesn't necessarily carry a `:free` suffix.
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
        // Only the input price written, no output: treated as unknown, not recommended
        assert!(!ids.contains(&"only-input".to_string()));
        // One-sided zero doesn't count as free
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
        // Valid JSON but no target vendor
        let err = parse_models_dev_free(r#"{"anthropic":{"models":{}}}"#, "opencode").unwrap_err();
        assert!(err.contains("opencode"), "错误应点明缺失的厂商：{}", err);
        // A vendor present but no models container
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

    /// With both sources present, take the intersection: free AND offered by the gateway.
    #[test]
    fn intersects_free_with_live() {
        let free = parse_models_dev_free(&sample(), "opencode").unwrap();
        let ids = combine(
            free,
            Some(&["big-pickle".to_string(), "paid-model".to_string()]),
        );
        assert_eq!(ids, vec!["big-pickle"]);
    }

    /// Models flagged deprecated but still gateway-offered are kept.
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

    /// Gateway unavailable → fallback: trust only models.dev, excluding deprecated.
    #[test]
    fn falls_back_to_models_dev_without_gateway() {
        let free = parse_models_dev_free(&sample(), "opencode").unwrap();
        assert_eq!(combine(free, None), vec!["big-pickle", "some-free"]);
    }

    /// Kilo judges by `isFree`: free entries without a `:free` suffix are collected too.
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

    /// Kilo's paid entries and those without an `isFree` flag don't count as free.
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
        // An empty list is a valid result, not an error
        assert_eq!(
            parse_kilo_free(r#"{"data":[]}"#).unwrap(),
            Vec::<String>::new()
        );
    }

    /// Each backend's provider id and free-tier support.
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
        // MiMo Code has no free tier
        assert!(source_for(ConfigFormat::Mimocode).is_none());
        // Non-opencode-family backends have none either
        assert!(source_for(ConfigFormat::WorkBuddy).is_none());
        assert!(source_for(ConfigFormat::ZCode).is_none());
    }

    /// Backends' cache files never overwrite each other.
    #[test]
    fn cache_files_are_per_backend() {
        let a = cache_file(ConfigFormat::Opencode);
        let b = cache_file(ConfigFormat::Kilocode);
        assert_ne!(a, b);
        assert!(a.contains("opencode") && b.contains("kilocode"));
    }

    /// Each page's own-gateway prefixes.
    #[test]
    fn gateway_provider_ids_match_the_real_clis() {
        assert_eq!(gateway_provider_ids(ConfigFormat::Opencode), ["opencode"]);
        assert_eq!(gateway_provider_ids(ConfigFormat::Kilocode), ["kilo"]);
        // MiMo Code has two: auto routing + the Xiaomi model family
        assert_eq!(
            gateway_provider_ids(ConfigFormat::Mimocode),
            ["mimo", "xiaomi"]
        );
        // Non-opencode-family pages have no own gateway
        assert!(gateway_provider_ids(ConfigFormat::WorkBuddy).is_empty());
        assert!(gateway_provider_ids(ConfigFormat::ZCode).is_empty());
    }

    /// An empty dynamic list must still have a fallback model.
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

    /// Fallbacks take each page's auto-routing id.
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

    /// With a dynamic list, it wins — no fallback items mixed in.
    #[test]
    fn live_free_models_win_over_the_builtin_fallback() {
        let live = vec!["zzz-free".to_string(), "aaa-free".to_string()];
        let models = gateway_models(ConfigFormat::Kilocode, &live);
        // A preferred model missing from the dynamic list isn't added
        assert_eq!(models, vec!["kilo/aaa-free", "kilo/zzz-free"]);
        assert!(!models.iter().any(|m| m == "kilo/kilo-auto/free"));
    }

    /// A preferred model present in the dynamic list is promoted to first.
    #[test]
    fn the_preferred_model_is_promoted_to_the_front() {
        let live = vec![
            "aaa-free".to_string(),
            "big-pickle".to_string(),
            "zzz-free".to_string(),
        ];
        let models = gateway_models(ConfigFormat::Opencode, &live);
        assert_eq!(models.first().unwrap(), "opencode/big-pickle");
        // The rest stay lexicographic, without duplicates
        assert_eq!(
            models,
            vec![
                "opencode/big-pickle",
                "opencode/aaa-free",
                "opencode/zzz-free"
            ]
        );
    }

    /// The mimo page's fallback list and order.
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

    /// Prefix parsing: no slash, empty segment, slash-only all count as "unparseable".
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

    /// Another page's gateway prefix is invalid here; the own gateway and user-configured providers are valid.
    #[test]
    fn validity_is_about_the_pages_own_gateway() {
        let configured = vec!["sensenova".to_string()];
        // This page's gateway
        assert!(model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "kilo/kilo-auto/free"
        ));
        // User-configured providers are valid on every page
        assert!(model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "sensenova/deepseek-v4-flash"
        ));
        // Another gateway's prefix is invalid
        assert!(!model_is_valid_on(
            ConfigFormat::Kilocode,
            &configured,
            "opencode/ling-3.0-flash-fin-free"
        ));
        // The same reference is valid on the opencode page
        assert!(model_is_valid_on(
            ConfigFormat::Opencode,
            &configured,
            "opencode/ling-3.0-flash-fin-free"
        ));
        // The mimo page accepts mimo/ and xiaomi/, not kilo/
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

    /// Unparseable references are always invalid.
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

    /// Cache round trip: readable after writing, and judged within its lifetime.
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

    /// An expired cache still returns the list, flagged as not fresh.
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

    /// A corrupt / empty cache counts as no cache — no panic.
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
