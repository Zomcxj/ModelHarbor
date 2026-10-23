//! The oh-my-pi backend: dual-dialect loading / thinking translation / raw preservation / YAML round trips.

use model_harbor_core::backends;
use model_harbor_core::backends::oh_my_pi::{model_to_omp, provider_to_omp};
use model_harbor_core::convert;
use model_harbor_core::format::ConfigFormat;
use model_harbor_core::model::{ModelRow, ProviderRow};
use model_harbor_core::util::parse_yaml_content;
use serde_json::{json, Value};

fn omp_yaml() -> String {
    r#"providers:
  gw:
    baseUrl: https://gw.example.com/v1
    api: openai-completions
    apiKey: sk-x
    authHeader: true
    headers:
      X-Team: platform
    discovery:
      type: openai-models-list
    compat:
      supportsDeveloperRole: false
      maxTokensField: max_tokens
    models:
    - id: m1
      name: Model One
      reasoning: true
      input:
      - text
      - image
      contextWindow: 200000
      maxTokens: 16384
      cost:
        input: 3.0
        output: 15.0
        cacheRead: 0.3
        cacheWrite: 3.75
      thinking:
        mode: effort
        efforts:
        - medium
        - high
        - xhigh
        - max
        defaultLevel: high
"#
    .to_string()
}

fn load_omp(content: &str) -> model_harbor_core::backends::BackendLoad {
    backends::backend(ConfigFormat::OhMyPi)
        .parse(content)
        .expect("omp 解析失败")
}

#[test]
fn detect_yml_files_as_oh_my_pi() {
    let b = backends::backend(ConfigFormat::OhMyPi);
    // .yml + YAML syntax
    assert!(b.detect("providers:\n  a:\n", "models.yml"));
    // .yml + JSON syntax
    assert!(b.detect("{\"providers\": {}}", "models.yml"));
    // .json goes to pi
    assert!(!b.detect("{\"providers\": {}}", "models.json"));
    // No extension: JSON syntax → pi, pure YAML → omp
    assert!(!b.detect("{\"providers\": {}}", ""));
    assert!(b.detect("providers:\n  a:\n", ""));
    // No providers key
    assert!(!b.detect("foo: bar\n", "models.yml"));
}

#[test]
fn detect_format_registry_order() {
    // Registry order: omp before pi, so .yml hits omp
    assert_eq!(
        backends::detect_format("{\"providers\": {}}", "models.yml"),
        ConfigFormat::OhMyPi
    );
    assert_eq!(
        backends::detect_format("{\"providers\": {}}", "models.json"),
        ConfigFormat::Pi
    );
    assert_eq!(
        backends::detect_format("providers: {}", "models.yaml"),
        ConfigFormat::OhMyPi
    );
}

#[test]
fn parse_omp_thinking_block_into_variants() {
    let load = load_omp(&omp_yaml());
    assert_eq!(load.providers.len(), 1);
    let m = &load.providers[0].models[0];
    // efforts (no effortMap) → variants
    assert_eq!(m.variants, "medium, high, xhigh, max");
    // omp extension fields stay in raw
    assert!(m.raw.get("cost").is_some());
    assert!(m.raw["thinking"]["defaultLevel"].is_string());
}

#[test]
fn parse_pi_dialect_thinking_level_map_in_yaml() {
    // A pi-style thinkingLevelMap in YAML reads too
    let content = "providers:\n  p:\n    baseUrl: https://x/v1\n    api: openai-completions\n    apiKey: k\n    models:\n    - id: m\n      reasoning: true\n      thinkingLevelMap:\n        high: max\n";
    let load = load_omp(content);
    let m = &load.providers[0].models[0];
    assert_eq!(m.variants, "max");
}

#[test]
fn omp_round_trip_preserves_thinking_block() {
    let load = load_omp(&omp_yaml());
    let b = backends::backend(ConfigFormat::OhMyPi);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    // Preserved whole (defaultLevel included)
    let t = &root["providers"]["gw"]["models"][0]["thinking"];
    assert_eq!(t["mode"], json!("effort"));
    assert_eq!(t["defaultLevel"], json!("high"));
    assert_eq!(t["efforts"], json!(["medium", "high", "xhigh", "max"]));
}

#[test]
fn pi_asymmetric_map_translates_to_effort_map() {
    // The pi dialect {high: max} → omp {mode: effort, efforts: [high], effortMap: {high: max}}
    let content = "providers:\n  p:\n    baseUrl: https://x/v1\n    api: openai-completions\n    apiKey: k\n    models:\n    - id: m\n      reasoning: true\n      thinkingLevelMap:\n        high: max\n";
    let load = load_omp(content);
    let m = &load.providers[0].models[0];
    let out = model_to_omp(m);
    assert_eq!(out["thinking"]["mode"], json!("effort"));
    assert_eq!(out["thinking"]["efforts"], json!(["high"]));
    assert_eq!(out["thinking"]["effortMap"], json!({"high": "max"}));
    // No pi-dialect keys may remain
    assert!(out.get("thinkingLevelMap").is_none());
}

#[test]
fn omp_asymmetric_effort_map_translates_back_to_pi() {
    // omp {efforts: [high], effortMap: {high: max}} → pi thinkingLevelMap {high: max}
    let mut m = ModelRow::new();
    m.id = "m".into();
    m.reasoning = true;
    m.variants = "max".into();
    m.raw = json!({
        "id": "m",
        "reasoning": true,
        "thinking": {"mode": "effort", "efforts": ["high"], "effortMap": {"high": "max"}}
    });
    let out = convert::model_to_pi(&m);
    assert_eq!(out["thinkingLevelMap"], json!({"high": "max"}));
}

#[test]
fn omp_symmetric_map_omits_effort_map() {
    // A symmetric thinkingLevelMap {medium: medium} → efforts: [medium]
    let mut m = ModelRow::new();
    m.id = "m".into();
    m.variants = "medium".into();
    m.raw = json!({
        "id": "m",
        "thinkingLevelMap": {"medium": "medium"}
    });
    let out = model_to_omp(&m);
    assert_eq!(out["thinking"]["efforts"], json!(["medium"]));
    assert!(out["thinking"].get("effortMap").is_none());
}

#[test]
fn provider_raw_fields_preserved_on_save() {
    let load = load_omp(&omp_yaml());
    let p = &load.providers[0];
    let out = provider_to_omp(p);
    // omp extension fields preserved verbatim
    assert_eq!(out["authHeader"], json!(true));
    assert_eq!(out["headers"]["X-Team"], json!("platform"));
    assert_eq!(out["discovery"]["type"], json!("openai-models-list"));
    // compat: written when false, other compat keys kept
    assert_eq!(out["compat"]["supportsDeveloperRole"], json!(false));
    assert_eq!(out["compat"]["maxTokensField"], json!("max_tokens"));
    // Model extension fields (cost) preserved
    assert!(out["models"][0]["cost"].is_object());
}

#[test]
fn omp_requires_reasoning_content_written_from_opencode_source() {
    // Loaded from opencode (field absent) → the omp page defaults unchecked, saving writes false
    let raw = json!({
        "options": {"baseURL": "https://x/v1", "apiKey": "sk-test"},
        "models": {}
    });
    let provider = ProviderRow::from("openai", &raw);
    assert!(!provider.requires_reasoning_content);
    let out = provider_to_omp(&provider);
    assert_eq!(
        out["compat"]["requiresReasoningContentForAllAssistantTurns"],
        false
    );
}

#[test]
fn provider_compat_true_removes_flag_keeps_other_keys() {
    let mut p = ProviderRow::new();
    p.key = "p".into();
    p.base_url = "https://x/v1".into();
    p.api_key = "k".into();
    p.npm = "@ai-sdk/openai-compatible".into();
    p.compat = true;
    p.raw = json!({
        "baseUrl": "https://x/v1",
        "api": "openai-completions",
        "compat": {"supportsDeveloperRole": false, "extraBody": {"gw": "m1"}}
    });
    let out = provider_to_omp(&p);
    // UI compat=true → removes supportsDeveloperRole, keeps extraBody
    assert!(out["compat"].get("supportsDeveloperRole").is_none());
    assert_eq!(out["compat"]["extraBody"]["gw"], json!("m1"));
}

#[test]
fn opencode_source_builds_fresh_omp_objects() {
    // opencode-dialect raw (options/limit/variants) must not leak into omp output
    let mut p = ProviderRow::new();
    p.key = "p".into();
    p.base_url = "https://x/v1".into();
    p.api_key = "k".into();
    p.npm = "@ai-sdk/anthropic".into();
    p.compat = true;
    let mut m = ModelRow::new();
    m.id = "m".into();
    m.name = "M".into();
    m.reasoning = true;
    m.context = "100000".into();
    m.output = "4096".into();
    m.modalities_input = "text, image".into();
    m.variants = "high".into();
    m.raw = json!({
        "id": "m",
        "name": "M",
        "reasoning": true,
        "limit": {"context": 100000, "output": 4096},
        "modalities": {"input": ["text", "image"]},
        "variants": {"high": {}}
    });
    p.models.push(m);
    p.raw = json!({
        "options": {"baseURL": "https://x/v1", "apiKey": "k"},
        "models": {"m": {}}
    });
    let out = provider_to_omp(&p);
    // npm @ai-sdk/anthropic → api anthropic-messages
    assert_eq!(out["api"], json!("anthropic-messages"));
    assert!(out.get("options").is_none());
    let mo = &out["models"][0];
    assert!(mo.get("limit").is_none());
    assert!(mo.get("modalities").is_none());
    assert!(mo.get("variants").is_none());
    assert_eq!(mo["contextWindow"], json!(100000));
    assert_eq!(mo["maxTokens"], json!(4096));
    assert_eq!(mo["input"], json!(["text", "image"]));
    assert_eq!(mo["thinking"]["efforts"], json!(["high"]));
}

#[test]
fn yaml_render_round_trip_with_special_ids() {
    let load = load_omp(&omp_yaml());
    let b = backends::backend(ConfigFormat::OhMyPi);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    // Inject an id needing quotes (a bare [ opening is a YAML flow sequence)
    let mut root2 = root;
    root2["providers"]["gw"]["models"][0]["id"] = json!("[次]m1");
    let yaml_text = b.render(&root2, false).expect("YAML 渲染失败");
    assert!(
        yaml_text.contains("'[次]m1'"),
        "特殊 id 必须被引号包裹: {}",
        yaml_text
    );
    let parsed = parse_yaml_content(&yaml_text).expect("YAML 回读失败");
    assert_eq!(
        parsed["providers"]["gw"]["models"][0]["id"],
        json!("[次]m1")
    );
}

#[test]
fn cross_target_uses_target_extras() {
    let load = load_omp(&omp_yaml());
    let b = backends::backend(ConfigFormat::OhMyPi);
    let target_extras = json!({"someTopLevel": 42});
    let root = b.serialize_root(&[], &load.providers, &Value::Null, Some(&target_extras));
    assert_eq!(root["someTopLevel"], json!(42));
    assert!(root["providers"]["gw"].is_object());
}

#[test]
fn load_backend_reads_yaml_file() {
    let mut p = std::env::temp_dir();
    p.push(format!("omp_load_{}.yml", std::process::id()));
    std::fs::write(&p, omp_yaml()).unwrap();
    let load =
        backends::load_backend(ConfigFormat::OhMyPi, p.to_str().unwrap()).expect("通用加载失败");
    assert_eq!(load.providers.len(), 1);
    assert_eq!(
        load.providers[0].models[0].variants,
        "medium, high, xhigh, max"
    );
    std::fs::remove_file(&p).ok();
}

#[test]
fn real_user_models_yml_round_trip() {
    // Dev-machine smoke test: a real user config round trip (path from env; skipped when the file is missing)
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return;
    };
    let path = std::path::PathBuf::from(home).join(".omp/agent/models.yml");
    if !path.exists() {
        return;
    }
    let content = std::fs::read_to_string(&path).unwrap();
    let b = backends::backend(ConfigFormat::OhMyPi);
    let load = b.parse(&content).expect("真实 models.yml 解析失败");
    assert!(!load.providers.is_empty());
    // Every model's thinking block must parse into variants and survive the round trip
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    for (key, pv) in root["providers"].as_object().unwrap() {
        for mv in pv["models"].as_array().unwrap() {
            let src = load
                .providers
                .iter()
                .find(|p| &p.key == key)
                .unwrap()
                .models
                .iter()
                .find(|m| m.id == mv["id"].as_str().unwrap())
                .unwrap();
            if src.variants.trim().is_empty() {
                assert!(
                    mv.get("thinking").is_none(),
                    "{} 无档位不得输出 thinking",
                    mv["id"]
                );
            } else {
                assert!(
                    mv["thinking"]["mode"] == json!("effort"),
                    "{} thinking 块丢失",
                    mv["id"]
                );
            }
        }
    }
    // Renders as valid YAML
    let yaml_text = b.render(&root, false).expect("渲染失败");
    parse_yaml_content(&yaml_text).expect("渲染结果必须可回读");
}

#[test]
fn provider_to_omp_places_compat_between_api_and_models() {
    // Same as pi: compat after api, before models.
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.base_url = "https://example.com/v1".into();
    provider.api_key = "sk-x".into();
    provider.pi_api = "openai-completions".into();
    provider.compat = false;
    let out = provider_to_omp(&provider);
    let keys: Vec<&str> = out
        .as_object()
        .expect("provider 应为对象")
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys.starts_with(&["baseUrl", "apiKey", "api", "compat", "models"]),
        "字段顺序应为 baseUrl → apiKey → api → compat → models，实际 {keys:?}"
    );
}

#[test]
fn omp_variants_written_in_canonical_order() {
    // omp's thinking.efforts also writes in canonical tier order.
    let mut model = model_harbor_core::model::ModelRow::new();
    model.id = "m".into();
    model.variants = "max, minimal, high".into();
    let out = model_to_omp(&model);
    let efforts: Vec<&str> = out["thinking"]["efforts"]
        .as_array()
        .expect("efforts 应为数组")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(efforts, vec!["minimal", "high", "max"], "实际 {efforts:?}");
}
