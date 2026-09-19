//! ZCode 后端回归：`~/.zcode/v2/provider_config.json` 的判别 / 解析 / 序列化。
//!
//! 结构特殊之处：provider 与模型级属性分两处存放（`providerRules` 只列模型 id，
//! 元数据在 `modelConfigRules.providerModelRules`），序列化时两边必须同步。

use model_harbor::backends;
use model_harbor::convert;
use model_harbor::format::ConfigFormat;
use model_harbor::model::{ModelRow, ProviderRow};
use serde_json::json;

fn temp_path(name: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_zcode_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn zcode_json() -> String {
    r#"{
      "schemaVersion": 1,
      "config": {
        "providerOrder": ["new-provider"],
        "providerConfigRules": { "providerRules": [ {
            "providerId": "new-provider", "providerName": "workbuddy",
            "config": {
              "group": "standard-personal",
              "access": { "type": "api-key", "apiKey": "sk-test" },
              "api": { "type": "openai-chat-completions", "baseUrl": "http://127.0.0.1:3065/v1" },
              "personalModelIds": ["m1"],
              "modelOrder": ["m1"] } } ]},
        "modelConfigRules": {
          "providerModelRules": [ {
              "modelId": "m1", "providerId": "new-provider",
              "config": {
                "enabled": true,
                "properties": { "contextWindow": 1000000, "supportsJsonSchemaOutput": false },
                "optionSpecs": { "maxOutputTokens": { "max": 384000 } } } } ],
          "manualProviderModelRules": [] } } }"#
        .to_string()
}

fn load_zcode(content: &str) -> backends::BackendLoad {
    backends::backend(ConfigFormat::ZCode)
        .parse(content)
        .expect("ZCode 解析失败")
}

#[test]
fn detect_requires_provider_rules_or_order() {
    let b = backends::backend(ConfigFormat::ZCode);
    assert!(b.detect(&zcode_json(), ""), "providerRules 数组应命中");
    // 只有 providerOrder 也认
    assert!(b.detect(r#"{"config":{"providerOrder":[]}}"#, ""));
    // 反例：pi 的 providers、opencode 的 provider、空对象都不该命中
    assert!(!b.detect(r#"{"providers":{}}"#, ""));
    assert!(!b.detect(r#"{"provider":{}}"#, ""));
    assert!(!b.detect("{}", ""));
}

#[test]
fn detect_does_not_steal_sibling_formats() {
    // 判别优先级：ZCode 排在 pi 系之前，但不能把它们的配置抢走。
    assert_eq!(
        backends::detect_format(r#"{"providers":{"a":{}}}"#, ""),
        ConfigFormat::Pi
    );
    assert_eq!(
        backends::detect_format(r#"{"provider":{"a":{}}}"#, ""),
        ConfigFormat::Opencode
    );
    assert_eq!(
        backends::detect_format(&zcode_json(), ""),
        ConfigFormat::ZCode
    );
}

#[test]
fn parse_maps_provider_and_model_fields() {
    let load = load_zcode(&zcode_json());
    assert_eq!(load.providers.len(), 1);
    let p = &load.providers[0];
    assert_eq!(p.key, "new-provider");
    assert_eq!(p.description, "workbuddy");
    assert_eq!(p.base_url, "http://127.0.0.1:3065/v1");
    assert_eq!(p.api_key, "sk-test");
    // ZCode 的 openai-chat-completions 必须映射成内部的 openai-completions。
    assert_eq!(p.pi_api, "openai-completions");

    assert_eq!(p.models.len(), 1);
    let m = &p.models[0];
    assert_eq!(m.id, "m1");
    assert_eq!(m.context, "1000000");
    assert_eq!(m.output, "384000");
    assert_eq!(m.source_format, Some(ConfigFormat::ZCode));
}

#[test]
fn current_file_save_preserves_unknown_keys_and_map_expression() {
    let content = r#"{
      "schemaVersion": 1,
      "config": {
        "providerOrder": ["p1"],
        "providerConfigRules": { "providerRules": [ {
            "providerId": "p1", "providerName": "P1",
            "config": { "group": "standard-personal", "customExt": { "keep": 1 },
              "access": { "type": "api-key", "apiKey": "k" },
              "api": { "type": "openai-chat-completions", "baseUrl": "https://x.invalid" },
              "personalModelIds": ["m1"], "modelOrder": ["m1"] } } ]},
        "modelConfigRules": {
          "providerModelRules": [ {
              "modelId": "m1", "providerId": "p1",
              "config": {
                "enabled": true,
                "properties": { "contextWindow": 1000 },
                "optionSpecs": { "reasoningLevel": {
                    "values": ["low","high"], "map": "reasoningLevel == \"low\" ? {} : {}" } } } } ],
          "manualProviderModelRules": [ { "modelId": "manual", "providerId": "p1" } ] } } }"#;
    let b = backends::backend(ConfigFormat::ZCode);
    let load = b.parse(content).expect("解析失败");
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);

    // group / 自定义扩展键保留
    let cfg = &root["config"]["providerConfigRules"]["providerRules"][0]["config"];
    assert_eq!(cfg["group"], json!("standard-personal"));
    assert_eq!(cfg["customExt"]["keep"], json!(1), "未知扩展键必须保留");
    // map 表达式原样保留（我们只同步档位清单，不解析这段 JS）
    let specs =
        &root["config"]["modelConfigRules"]["providerModelRules"][0]["config"]["optionSpecs"];
    assert_eq!(
        specs["reasoningLevel"]["map"],
        json!("reasoningLevel == \"low\" ? {} : {}"),
        "map 表达式不得被改写"
    );
    assert_eq!(specs["reasoningLevel"]["values"], json!(["low", "high"]));
    // manualProviderModelRules 全量保留
    assert_eq!(
        root["config"]["modelConfigRules"]["manualProviderModelRules"][0]["modelId"],
        json!("manual")
    );
}

#[test]
fn enabled_is_a_sibling_of_properties_not_a_member() {
    // 曾经的 bug：把 enabled 写进 properties，ZCode 读不到还留下同名垃圾键。
    let load = load_zcode(&zcode_json());
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let config = &root["config"]["modelConfigRules"]["providerModelRules"][0]["config"];
    assert_eq!(config["enabled"], json!(true), "enabled 应在 config 下");
    assert!(
        config["properties"].get("enabled").is_none(),
        "properties 里不该出现 enabled"
    );
}

#[test]
fn does_not_invent_capability_flags() {
    // 原文件没写 supports* 时，保存不得凭空补 false——
    // 那会把「未声明」变成「明确不支持」，反而关掉 ZCode 的能力推断。
    let content = r#"{
      "config": {
        "providerOrder": ["p1"],
        "providerConfigRules": { "providerRules": [ {
            "providerId": "p1", "providerName": "P1",
            "config": { "access": { "type": "api-key", "apiKey": "k" },
              "api": { "type": "openai-chat-completions", "baseUrl": "https://x.invalid" },
              "personalModelIds": ["m1"], "modelOrder": ["m1"] } } ]},
        "modelConfigRules": { "providerModelRules": [ {
            "modelId": "m1", "providerId": "p1",
            "config": { "enabled": true, "properties": { "contextWindow": 1000 } } } ],
          "manualProviderModelRules": [] } } }"#;
    let load = load_zcode(content);
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let props =
        &root["config"]["modelConfigRules"]["providerModelRules"][0]["config"]["properties"];
    assert!(props.get("supportsText").is_none(), "不该补 supportsText");
    assert!(props.get("supportsImage").is_none(), "不该补 supportsImage");
    assert!(
        props.get("supportsToolCall").is_none(),
        "不该补 supportsToolCall"
    );
    assert_eq!(props["contextWindow"], json!(1000), "已有字段照常同步");
}

#[test]
fn cross_format_save_does_not_leak_foreign_keys() {
    // 从 WorkBuddy 形状的 raw 转存到 ZCode：对方的 id/vendor/url 不得进 config。
    let mut p = ProviderRow::new();
    p.key = "wb1".into();
    p.base_url = "https://api.example.com".into();
    p.api_key = "sk-x".into();
    p.pi_api = "openai-completions".into();
    p.raw = json!({
        "id": "wb1", "vendor": "Custom", "url": "https://api.example.com",
        "useCustomProtocol": false, "supportsImages": true
    });
    let mut m = ModelRow::new();
    m.id = "wb1".into();
    m.raw = json!({ "id": "wb1", "url": "https://api.example.com" });
    p.models = vec![m];

    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], std::slice::from_ref(&p), &json!({}), None);
    let cfg = &root["config"]["providerConfigRules"]["providerRules"][0]["config"];
    assert!(
        cfg.get("id").is_none(),
        "WorkBuddy 的 id 不该进 ZCode config"
    );
    assert!(cfg.get("vendor").is_none(), "vendor 不该泄漏");
    assert!(cfg.get("url").is_none(), "url 不该泄漏");
    assert!(
        cfg.get("useCustomProtocol").is_none(),
        "useCustomProtocol 不该泄漏"
    );
    assert_eq!(cfg["api"]["baseUrl"], json!("https://api.example.com"));
}

#[test]
fn protocol_vocabulary_maps_both_ways() {
    // ZCode 的 Chat Completions 叫 openai-chat-completions，pi 系叫 openai-completions。
    assert_eq!(
        convert::zcode_api_to_api("openai-chat-completions"),
        "openai-completions"
    );
    assert_eq!(
        convert::api_to_zcode_api("openai-completions"),
        "openai-chat-completions"
    );
    assert_eq!(
        convert::api_to_zcode_api("anthropic-messages"),
        "anthropic-messages"
    );
    assert_eq!(
        convert::api_to_zcode_api("openai-responses"),
        "openai-responses"
    );
    // 无对应值的协议回落到 Chat Completions（ZCode 只认三值）。
    assert_eq!(
        convert::api_to_zcode_api("google-vertex"),
        "openai-chat-completions"
    );
}

#[test]
fn round_trip_through_file_keeps_credentials() {
    let path = temp_path("provider_config.json");
    std::fs::write(&path, zcode_json()).unwrap();
    let load =
        backends::load_backend(ConfigFormat::ZCode, path.to_str().unwrap()).expect("应可加载");
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let text = b.render(&root, false).unwrap();
    assert!(text.contains("sk-test"), "密钥必须保留：\n{text}");
    assert!(text.contains("openai-chat-completions"), "协议必须保留");
    std::fs::remove_file(&path).ok();
}
