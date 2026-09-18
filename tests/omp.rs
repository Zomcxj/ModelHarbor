//! oh-my-pi 后端：双方言加载 / thinking 翻译 / raw 保留 / YAML 往返。

use model_harbor::backends;
use model_harbor::backends::oh_my_pi::{model_to_omp, provider_to_omp};
use model_harbor::convert;
use model_harbor::format::ConfigFormat;
use model_harbor::model::{ModelRow, ProviderRow};
use model_harbor::util::parse_yaml_content;
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

fn load_omp(content: &str) -> model_harbor::backends::BackendLoad {
    backends::backend(ConfigFormat::OhMyPi)
        .parse(content)
        .expect("omp 解析失败")
}

#[test]
fn detect_yml_files_as_oh_my_pi() {
    let b = backends::backend(ConfigFormat::OhMyPi);
    // .yml + YAML 语法
    assert!(b.detect("providers:\n  a:\n", "models.yml"));
    // .yml + JSON 语法（JSON 是 YAML 子集，扩展名优先归 omp）
    assert!(b.detect("{\"providers\": {}}", "models.yml"));
    // .json 归 pi
    assert!(!b.detect("{\"providers\": {}}", "models.json"));
    // 无扩展名：JSON 语法让位 pi，纯 YAML 归 omp
    assert!(!b.detect("{\"providers\": {}}", ""));
    assert!(b.detect("providers:\n  a:\n", ""));
    // 无 providers 键
    assert!(!b.detect("foo: bar\n", "models.yml"));
}

#[test]
fn detect_format_registry_order() {
    // 注册表顺序：omp 在 pi 前，.yml 优先命中 omp
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
    // efforts（无 effortMap）→ variants
    assert_eq!(m.variants, "medium, high, xhigh, max");
    // omp 扩展字段保留在 raw
    assert!(m.raw.get("cost").is_some());
    assert!(m.raw["thinking"]["defaultLevel"].is_string());
}

#[test]
fn parse_pi_dialect_thinking_level_map_in_yaml() {
    // 加载器双方言：YAML 中的 pi 风格 thinkingLevelMap 也能读
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
    // 整块保留（含 defaultLevel）
    let t = &root["providers"]["gw"]["models"][0]["thinking"];
    assert_eq!(t["mode"], json!("effort"));
    assert_eq!(t["defaultLevel"], json!("high"));
    assert_eq!(t["efforts"], json!(["medium", "high", "xhigh", "max"]));
}

#[test]
fn pi_asymmetric_map_translates_to_effort_map() {
    // pi 方言 {high: max} → omp {mode: effort, efforts: [high], effortMap: {high: max}}
    let content = "providers:\n  p:\n    baseUrl: https://x/v1\n    api: openai-completions\n    apiKey: k\n    models:\n    - id: m\n      reasoning: true\n      thinkingLevelMap:\n        high: max\n";
    let load = load_omp(content);
    let m = &load.providers[0].models[0];
    let out = model_to_omp(m);
    assert_eq!(out["thinking"]["mode"], json!("effort"));
    assert_eq!(out["thinking"]["efforts"], json!(["high"]));
    assert_eq!(out["thinking"]["effortMap"], json!({"high": "max"}));
    // pi 方言键不得残留
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
    // 对称 thinkingLevelMap {medium: medium} → efforts: [medium]，无需 effortMap
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
    // omp 扩展字段原样保留
    assert_eq!(out["authHeader"], json!(true));
    assert_eq!(out["headers"]["X-Team"], json!("platform"));
    assert_eq!(out["discovery"]["type"], json!("openai-models-list"));
    // compat：false 时写入，且保留其他 compat 键
    assert_eq!(out["compat"]["supportsDeveloperRole"], json!(false));
    assert_eq!(out["compat"]["maxTokensField"], json!("max_tokens"));
    // 模型扩展字段（cost）保留
    assert!(out["models"][0]["cost"].is_object());
}

#[test]
fn omp_requires_reasoning_content_written_from_opencode_source() {
    // 加载 opencode（无该字段）→ omp 页面默认不勾选，保存时写入 false
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
    // UI compat=true → 移除 supportsDeveloperRole，保留 extraBody
    assert!(out["compat"].get("supportsDeveloperRole").is_none());
    assert_eq!(out["compat"]["extraBody"]["gw"], json!("m1"));
}

#[test]
fn opencode_source_builds_fresh_omp_objects() {
    // opencode 方言 raw（options/limit/variants）不得泄漏进 omp 输出
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
    // npm @ai-sdk/anthropic → api anthropic-messages；无 opencode 特征键泄漏
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
    // 注入需要引号的 id（裸 [ 开头会被 YAML 解析为流序列）
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
    // 开发机冒烟：真实用户配置往返（路径按环境变量取，文件不存在则跳过）
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
    // 所有模型的 thinking 块必须能读入 variants 且往返保留
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
    // 渲染为合法 YAML
    let yaml_text = b.render(&root, false).expect("渲染失败");
    parse_yaml_content(&yaml_text).expect("渲染结果必须可回读");
}

#[test]
fn provider_to_omp_places_compat_between_api_and_models() {
    // 与 pi 一致：compat 固定跟在 api 之后、models 之前。
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
    // omp 的 thinking.efforts 同样按规范档位顺序写出。
    let mut model = model_harbor::model::ModelRow::new();
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
