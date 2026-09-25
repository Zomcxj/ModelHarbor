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
fn modalities_live_under_properties_input_format() {
    // 内置模型库把模态布尔嵌在 properties.inputFormat（inputFormat/outputFormat
    // 两个子块），不是 properties 直接子键。读入按嵌套取，写出按嵌套落。
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
            "config": { "enabled": true, "properties": {
              "contextWindow": 1000,
              "inputFormat": { "supportsText": true, "supportsImage": true, "supportsVideo": false } } } } ],
          "manualProviderModelRules": [] } } }"#;
    let load = load_zcode(content);
    // 读入：从 inputFormat 推导出模态列表（不含 video）。
    let m = &load.providers[0].models[0];
    assert_eq!(m.modalities_input, "text, image", "模态应从 inputFormat 取");

    // 写出：模态回落到 properties.inputFormat，不平铺到 properties。
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let props =
        &root["config"]["modelConfigRules"]["providerModelRules"][0]["config"]["properties"];
    assert!(
        props.get("supportsImage").is_none(),
        "supportsImage 不该平铺在 properties"
    );
    let input = &props["inputFormat"];
    assert_eq!(input["supportsImage"], json!(true));
    assert_eq!(input["supportsText"], json!(true));
    assert_eq!(input["supportsVideo"], json!(false));
}

#[test]
fn does_not_expand_declared_modalities_into_false_flags() {
    // 模型只声明了 inputFormat.supportsImage，保存不得把它展开成五个 flag
    // （supportsText:false 等）——那是「凭空发明字段」，supportsText:false 会让
    // ZCode 隐藏/拒绝该模型（保存后软件里不显示的根因）。
    let content = r#"{
      "config": {
        "providerOrder": ["p1"],
        "providerConfigRules": { "providerRules": [ {
            "providerId": "p1", "providerName": "P1",
            "config": { "access": { "type": "api-key", "apiKey": "k" },
              "api": { "type": "anthropic-messages", "baseUrl": "https://x.invalid" },
              "personalModelIds": ["m1"], "modelOrder": ["m1"] } } ]},
        "modelConfigRules": { "providerModelRules": [ {
            "modelId": "m1", "providerId": "p1",
            "config": { "enabled": true, "properties": {
              "contextWindow": 272000,
              "inputFormat": { "supportsImage": true } } } } ],
          "manualProviderModelRules": [] } } }"#;
    let load = load_zcode(content);
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let input = &root["config"]["modelConfigRules"]["providerModelRules"][0]["config"]
        ["properties"]["inputFormat"];
    assert_eq!(input["supportsImage"], json!(true));
    assert!(
        input.get("supportsText").is_none(),
        "不该补 supportsText:false"
    );
    assert!(
        input.get("supportsVideo").is_none(),
        "不该补 supportsVideo:false"
    );
    assert!(
        input.get("supportsPdf").is_none(),
        "不该补 supportsPdf:false"
    );
    assert!(
        input.get("supportsAudio").is_none(),
        "不该补 supportsAudio:false"
    );
}

#[test]
fn provider_always_gets_standard_personal_group() {
    // ZCode 要求 provider.config.group 必填，个人 provider 缺它会被整份拒绝
    // （从 WorkBuddy 转过来的 provider 没有 group，就是保存后 ZCode 里不显示的根因）。
    let mut p = ProviderRow::new();
    p.key = "wb1".into();
    p.base_url = "https://api.example.com".into();
    p.api_key = "sk-x".into();
    p.pi_api = "openai-completions".into();
    p.raw = json!({ "id": "wb1", "url": "https://api.example.com", "useCustomProtocol": false });
    let mut m = ModelRow::new();
    m.id = "some-model".into();
    p.models = vec![m];

    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], std::slice::from_ref(&p), &json!({}), None);
    let cfg = &root["config"]["providerConfigRules"]["providerRules"][0]["config"];
    assert_eq!(
        cfg["group"],
        json!("standard-personal"),
        "缺 group 时必须补 standard-personal"
    );
}

#[test]
fn existing_group_is_preserved() {
    // ZCode 源自带的 group（如 zai-family）不能被覆盖成 standard-personal。
    let content = r#"{
      "config": {
        "providerOrder": ["p1"],
        "providerConfigRules": { "providerRules": [ {
            "providerId": "p1", "providerName": "P1",
            "config": { "group": "zai-family",
              "access": { "type": "api-key", "apiKey": "k" },
              "api": { "type": "openai-chat-completions", "baseUrl": "https://x.invalid" },
              "personalModelIds": ["m1"], "modelOrder": ["m1"] } } ]},
        "modelConfigRules": { "providerModelRules": [], "manualProviderModelRules": [] } } }"#;
    let load = load_zcode(content);
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let cfg = &root["config"]["providerConfigRules"]["providerRules"][0]["config"];
    assert_eq!(cfg["group"], json!("zai-family"), "已有 group 应保留");
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

/// anthropic 协议的 provider 规则（baseUrl 由调用方给出）。
fn zcode_anthropic(base_url: &str) -> String {
    format!(
        r#"{{
      "schemaVersion": 1,
      "config": {{
        "providerOrder": ["claude_x"],
        "providerConfigRules": {{ "providerRules": [ {{
            "providerId": "claude_x", "providerName": "claude_x",
            "config": {{
              "group": "standard-personal",
              "access": {{ "type": "api-key", "apiKey": "sk-test" }},
              "api": {{ "type": "anthropic-messages", "baseUrl": "{base_url}" }},
              "personalModelIds": ["claude-opus-5"],
              "modelOrder": ["claude-opus-5"] }} }} ]}},
        "modelConfigRules": {{
          "providerModelRules": [ {{
              "modelId": "claude-opus-5", "providerId": "claude_x",
              "config": {{ "enabled": true, "properties": {{ "contextWindow": 272000 }} }} }} ],
          "manualProviderModelRules": [] }} }} }}"#
    )
}

#[test]
fn anthropic_base_url_drops_v1_like_pi() {
    // ZCode 请求时按 kind 先剥后缀再拼 `/v1/messages`，但它只剥完整端点后缀，
    // 不认光秃秃的 `/v1`：baseUrl 留 `/v1` 会被拼成 `/v1/v1/messages`，服务端直接拒
    // （ZCode 里报 "Provider rejected the model request"）。与 pi 一致：进出都不带 `/v1`。
    let load = load_zcode(&zcode_anthropic("https://api.justwoker.icu/v1"));
    assert_eq!(
        load.providers[0].base_url, "https://api.justwoker.icu",
        "读入就要去掉末尾 /v1，界面显示的必须是 ZCode 真正当基址用的值"
    );

    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let cfg = &root["config"]["providerConfigRules"]["providerRules"][0]["config"];
    assert_eq!(cfg["api"]["type"], json!("anthropic-messages"));
    assert_eq!(
        cfg["api"]["baseUrl"],
        json!("https://api.justwoker.icu"),
        "写出同样不能带 /v1"
    );
}

#[test]
fn anthropic_base_url_collapses_a_full_endpoint_path() {
    // 手填整段端点（WorkBuddy 那边就是这么存的）也要收敛回基址：
    // ZCode 自己会剥 `/v1/messages` / `/messages` 再拼回去，留着就等于两份后缀。
    let load = load_zcode(&zcode_anthropic("https://api.justwoker.icu/v1/messages"));
    assert_eq!(load.providers[0].base_url, "https://api.justwoker.icu");

    // 已经是基址时不动它（幂等）。
    let load = load_zcode(&zcode_anthropic("https://api.justwoker.icu"));
    assert_eq!(load.providers[0].base_url, "https://api.justwoker.icu");

    // 带子路径的基址只剥到路径边界，不吞掉 host 后面的前缀。
    let load = load_zcode(&zcode_anthropic("https://host.example/anthropic/v1"));
    assert_eq!(load.providers[0].base_url, "https://host.example/anthropic");
}

#[test]
fn chat_completions_base_url_keeps_its_v1() {
    // Chat Completions 相反：ZCode 拼的是 `/chat/completions`，
    // `https://host/v1` 正是它的正确基址，剥掉就会请求到 `/chat/completions` 而 404。
    let load = load_zcode(&zcode_json());
    assert_eq!(load.providers[0].base_url, "http://127.0.0.1:3065/v1");

    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let cfg = &root["config"]["providerConfigRules"]["providerRules"][0]["config"];
    assert_eq!(cfg["api"]["baseUrl"], json!("http://127.0.0.1:3065/v1"));
}

#[test]
fn zcode_base_url_normalization_is_idempotent() {
    let cases = [
        ("anthropic-messages", "https://host.example/v1"),
        ("anthropic-messages", "https://host.example/v1/messages"),
        ("anthropic-messages", "https://host.example/messages"),
        ("anthropic-messages", "https://host.example/anthropic"),
        ("openai-responses", "https://host.example/v1"),
        ("openai-completions", "https://host.example/v1"),
        (
            "openai-completions",
            "https://host.example/v1/chat/completions",
        ),
    ];
    for (api, url) in cases {
        let once = convert::zcode_normalize_base_url(api, url);
        let twice = convert::zcode_normalize_base_url(api, &once);
        assert_eq!(once, twice, "{api} {url} 归一化必须幂等");
        // 归一化结果再拼上 ZCode 自己的端点后缀，就是最终请求 URL。
        let suffix = match api {
            "anthropic-messages" => "/v1/messages",
            "openai-responses" => "/responses",
            _ => "/chat/completions",
        };
        let final_url = format!("{once}{suffix}");
        assert_eq!(
            final_url.matches("/v1").count(),
            1,
            "最终 URL 只该出现一次 /v1：{final_url}"
        );
    }
}

/// 序列化后 `config` 内键序必须恒定：providerOrder → providerConfigRules → modelConfigRules。
///
/// 背景：`serde_json` 开了 `preserve_order`（IndexMap），它的 `remove` 是 swap_remove，
/// 删键会把最后一个键搬到空位。跨格式保存先经 `strip_cross_format_containers` 删掉
/// providerOrder / providerRules / providerModelRules，键序因此被搅乱成
/// modelConfigRules → providerConfigRules → providerOrder，与 ZCode 自己的写法不一致。
fn config_keys_of(root: &serde_json::Value) -> Vec<String> {
    root.get("config")
        .and_then(|c| c.as_object())
        .map(|c| c.keys().cloned().collect())
        .unwrap_or_default()
}

#[test]
fn serialize_keeps_zcode_native_key_order() {
    let load = load_zcode(&zcode_json());
    let out = backends::backend(ConfigFormat::ZCode).serialize_root(
        &load.agents,
        &load.providers,
        &load.extras,
        Some(&load.root),
    );
    assert_eq!(
        config_keys_of(&out),
        vec!["providerOrder", "providerConfigRules", "modelConfigRules"],
        "config 键序必须与 ZCode 原生一致"
    );
    let top: Vec<String> = out
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(top, vec!["schemaVersion", "config"], "顶层键序必须固定");
}

#[test]
fn cross_format_strip_does_not_reorder_config_keys() {
    // 模拟跨格式保存的真实路径：先剔除界面接管的容器，再序列化。
    let load = load_zcode(&zcode_json());
    let mut target = load.root.clone();
    model_harbor::app::strip_cross_format_containers(ConfigFormat::ZCode, &mut target, false);
    assert!(
        !target
            .get("config")
            .and_then(|c| c.as_object())
            .map(|c| c.contains_key("providerOrder"))
            .unwrap_or(false),
        "剔除后 providerOrder 应已删除"
    );
    let out = backends::backend(ConfigFormat::ZCode).serialize_root(
        &load.agents,
        &load.providers,
        &load.extras,
        Some(&target),
    );
    assert_eq!(
        config_keys_of(&out),
        vec!["providerOrder", "providerConfigRules", "modelConfigRules"],
        "跨格式路径写出的 config 键序同样必须与 ZCode 原生一致"
    );
}

/// 内层容器的键序也要固定，且缺 `manualProviderModelRules` 时补空数组。
#[test]
fn nested_containers_keep_their_key_order() {
    let src = r#"{
      "schemaVersion": 1,
      "config": {
        "modelConfigRules": { "manualProviderModelRules": [] },
        "providerConfigRules": {},
        "providerOrder": []
      }
    }"#;
    let root: serde_json::Value = serde_json::from_str(src).unwrap();
    let out = backends::backend(ConfigFormat::ZCode).serialize_root(&[], &[], &root, Some(&root));
    let cfg = out.get("config").and_then(|c| c.as_object()).unwrap();
    let model_keys: Vec<String> = cfg
        .get("modelConfigRules")
        .and_then(|m| m.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(
        model_keys,
        vec!["providerModelRules", "manualProviderModelRules"],
        "modelConfigRules 内部键序必须固定"
    );
    let provider_keys: Vec<String> = cfg
        .get("providerConfigRules")
        .and_then(|m| m.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(provider_keys, vec!["providerRules"]);
}

/// `config.enabled` 是 ZCode 自己的模型开关，ModelHarbor **不接管**它。
///
/// 曾经的 bug：序列化时无条件写 `enabled: true`，于是用户在 ZCode 界面里关掉的模型，
/// 只要用 ModelHarbor 存一次就被重新打开。现在只补缺失的键，已有值原样保留。
#[test]
fn a_model_disabled_in_zcode_stays_disabled() {
    let src = r#"{
      "schemaVersion": 1,
      "config": {
        "providerOrder": ["p1"],
        "providerConfigRules": { "providerRules": [ {
            "providerId": "p1", "providerName": "p1",
            "config": {
              "group": "standard-personal",
              "access": { "type": "api-key", "apiKey": "sk-test" },
              "api": { "type": "openai-chat-completions", "baseUrl": "https://h/v1" },
              "personalModelIds": ["off", "on", "unset"],
              "modelOrder": ["off", "on", "unset"] } } ]},
        "modelConfigRules": {
          "providerModelRules": [
            { "modelId": "off", "providerId": "p1",
              "config": { "enabled": false, "properties": { "contextWindow": 1000 } } },
            { "modelId": "on", "providerId": "p1",
              "config": { "enabled": true, "properties": { "contextWindow": 2000 } } },
            { "modelId": "unset", "providerId": "p1",
              "config": { "properties": { "contextWindow": 3000 } } } ],
          "manualProviderModelRules": [] } } }"#;
    let load = load_zcode(src);
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let rules = root["config"]["modelConfigRules"]["providerModelRules"]
        .as_array()
        .expect("providerModelRules 应为数组");
    let enabled_of = |id: &str| -> Option<bool> {
        rules
            .iter()
            .find(|r| r["modelId"] == json!(id))
            .and_then(|r| r["config"]["enabled"].as_bool())
    };
    assert_eq!(
        enabled_of("off"),
        Some(false),
        "ZCode 里关掉的模型不能被重新打开"
    );
    assert_eq!(enabled_of("on"), Some(true), "ZCode 里开着的模型保持开着");
    assert_eq!(enabled_of("unset"), Some(true), "原文件没写该键时才补 true");
}

/// 非法数字**不能**被写成 0：界面与保存状态栏都告诉用户「该字段将被忽略」，
/// 写成 `contextWindow: 0` / `maxOutputTokens.max: 0` 是另一回事——ZCode 会照单全收，
/// 那个模型的上下文直接归零。曾经两处都用的 `unwrap_or(0)`。
#[test]
fn invalid_numbers_are_skipped_not_written_as_zero() {
    let src = r#"{
      "schemaVersion": 1,
      "config": {
        "providerOrder": ["p1"],
        "providerConfigRules": { "providerRules": [ {
            "providerId": "p1", "providerName": "p1",
            "config": {
              "group": "standard-personal",
              "access": { "type": "api-key", "apiKey": "sk-test" },
              "api": { "type": "openai-chat-completions", "baseUrl": "https://x.invalid" },
              "personalModelIds": ["bad", "good"],
              "modelOrder": ["bad", "good"] } } ]},
        "modelConfigRules": {
          "providerModelRules": [
            { "modelId": "bad", "providerId": "p1",
              "config": { "enabled": true, "properties": { "contextWindow": 1000 } } },
            { "modelId": "good", "providerId": "p1",
              "config": { "enabled": true, "properties": { "contextWindow": 2000 } } } ],
          "manualProviderModelRules": [] } } }"#;
    let mut load = load_zcode(src);
    load.providers[0].models[0].context = "十二万".into();
    load.providers[0].models[0].output = "abc".into();
    load.providers[0].models[1].context = "3000".into();
    load.providers[0].models[1].output = "4000".into();
    let b = backends::backend(ConfigFormat::ZCode);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let rules = root["config"]["modelConfigRules"]["providerModelRules"]
        .as_array()
        .expect("providerModelRules 应为数组");
    let find = |id: &str| rules.iter().find(|r| r["modelId"] == json!(id)).unwrap();
    let bad = find("bad");
    assert_eq!(
        bad["config"]["properties"]["contextWindow"],
        json!(1000),
        "解析不了应保持原值，不能写成 0"
    );
    assert!(
        bad["config"]["optionSpecs"]["maxOutputTokens"].is_null(),
        "解析不了的 maxOutputTokens 不该凭空造出 max: 0，实际为 {}",
        bad["config"]["optionSpecs"]
    );
    let good = find("good");
    assert_eq!(good["config"]["properties"]["contextWindow"], json!(3000));
    assert_eq!(
        good["config"]["optionSpecs"]["maxOutputTokens"]["max"],
        json!(4000)
    );
}
