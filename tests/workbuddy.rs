//! WorkBuddy 后端回归：`~/.workbuddy/models.json` 的判别 / 解析 / 序列化。
//!
//! 两个结构特殊之处：
//! 1. 根是**顶层数组**（provider 信息内联在每条模型里）——通用 JSON 序列化器
//!    假定对象根，用错会把它静默变成 `{}` 毁掉配置，所以这里钉住数组形态；
//! 2. 协议由 `useCustomProtocol` + URL 后缀表达，文件里没有协议字段。

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
    let dir =
        std::env::temp_dir().join(format!("model_harbor_wb_{}_{}", std::process::id(), nonce));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// 真实形态样本：一条 Chat（不勾自定义协议）+ 一条 Messages（勾选）。
fn workbuddy_json() -> String {
    r#"[
      { "id": "chat-model", "name": "chat-model", "vendor": "Custom",
        "url": "http://127.0.0.1:3065/v1", "apiKey": "sk-chat",
        "supportsToolCall": true, "supportsImages": true, "supportsReasoning": false,
        "useCustomProtocol": false, "maxInputTokens": 500000, "maxOutputTokens": 65536 },
      { "id": "messages-model", "name": "messages-model", "vendor": "Custom",
        "url": "https://api.example.com/v1/messages", "apiKey": "sk-msg",
        "supportsToolCall": true, "supportsImages": true, "supportsReasoning": false,
        "useCustomProtocol": true, "maxInputTokens": 272000, "maxOutputTokens": 65536 }
    ]"#
    .to_string()
}

fn load_wb(content: &str) -> backends::BackendLoad {
    backends::backend(ConfigFormat::WorkBuddy)
        .parse(content)
        .expect("WorkBuddy 解析失败")
}

#[test]
fn detect_requires_non_empty_array_with_id() {
    let b = backends::backend(ConfigFormat::WorkBuddy);
    assert!(b.detect(&workbuddy_json(), ""), "非空数组应命中");
    // `[]` 不认：空数组太泛，会抢走别人的配置
    assert!(!b.detect("[]", ""));
    // 反例
    assert!(!b.detect("{}", ""));
    assert!(!b.detect(r#"{"providers":{}}"#, ""));
    assert!(!b.detect(r#"{"provider":{}}"#, ""));
}

#[test]
fn detect_does_not_steal_sibling_formats() {
    assert_eq!(
        backends::detect_format(&workbuddy_json(), ""),
        ConfigFormat::WorkBuddy
    );
    assert_eq!(
        backends::detect_format(r#"{"providers":{"a":{}}}"#, ""),
        ConfigFormat::Pi
    );
    assert_eq!(
        backends::detect_format(r#"{"provider":{"a":{}}}"#, ""),
        ConfigFormat::Opencode
    );
}

#[test]
fn parse_derives_protocol_from_custom_flag_and_url() {
    let load = load_wb(&workbuddy_json());
    assert_eq!(load.providers.len(), 2, "每条模型一个卡片");

    // 未勾自定义协议 → Chat Completions
    let chat = &load.providers[0];
    assert_eq!(chat.key, "chat-model");
    assert_eq!(chat.pi_api, "openai-completions");
    assert_eq!(chat.base_url, "http://127.0.0.1:3065/v1");
    assert_eq!(chat.api_key, "sk-chat");

    // 勾了自定义协议 → 由 URL 后缀推导
    let msg = &load.providers[1];
    assert_eq!(msg.key, "messages-model");
    assert_eq!(msg.pi_api, "anthropic-messages");
    assert_eq!(msg.base_url, "https://api.example.com/v1/messages");
}

#[test]
fn parse_maps_model_attributes() {
    let load = load_wb(&workbuddy_json());
    let m = &load.providers[0].models[0];
    assert_eq!(m.id, "chat-model");
    assert_eq!(m.name, "chat-model");
    assert_eq!(m.context, "500000");
    assert_eq!(m.output, "65536");
    assert_eq!(m.modalities_input, "text, image");
    assert!(m.tool_call);
    assert!(!m.reasoning);
    assert_eq!(m.source_format, Some(ConfigFormat::WorkBuddy));
}

#[test]
fn render_keeps_the_array_root() {
    // 这是防回归的关键：通用序列化器假定对象根，会把数组变成 `{}`。
    let load = load_wb(&workbuddy_json());
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    assert!(root.is_array(), "序列化结果必须是数组");

    for compact in [false, true] {
        let text = b.render(&root, compact).unwrap();
        assert!(
            text.trim_start().starts_with('['),
            "compact={compact} 时渲染必须是数组：{text}"
        );
        assert!(text.contains("chat-model"), "条目不得丢失");
        // 能被重新解析回来（往返自洽）
        let reparsed: serde_json::Value =
            serde_json::from_str(&text).expect("渲染结果应是合法 JSON");
        assert_eq!(reparsed.as_array().map(Vec::len), Some(2));
    }
}

#[test]
fn non_chat_protocol_gets_suffix_and_custom_flag() {
    let mut p = ProviderRow::new();
    p.key = "m1".into();
    p.base_url = "https://api.example.com".into();
    p.pi_api = "anthropic-messages".into();
    let mut m = ModelRow::new();
    m.id = "m1".into();
    m.name = "m1".into();
    p.models = vec![m];

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], std::slice::from_ref(&p), &json!([]), None);
    assert_eq!(
        out[0]["url"],
        json!("https://api.example.com/v1/messages"),
        "非 Chat 协议应补上后缀"
    );
    assert_eq!(out[0]["useCustomProtocol"], json!(true), "并置自定义协议");
}

#[test]
fn chat_protocol_keeps_base_url_and_unchecked() {
    let mut p = ProviderRow::new();
    p.key = "m1".into();
    p.base_url = "https://api.example.com".into();
    p.pi_api = "openai-completions".into();
    let mut m = ModelRow::new();
    m.id = "m1".into();
    p.models = vec![m];

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], std::slice::from_ref(&p), &json!([]), None);
    assert_eq!(out[0]["url"], json!("https://api.example.com"), "存基址");
    assert_eq!(
        out[0]["useCustomProtocol"],
        json!(false),
        "由 WorkBuddy 补全"
    );
}

#[test]
fn custom_protocol_url_is_not_normalized() {
    // 勾选自定义协议时 URL 原样使用：手写的完整路径不得被改写或重复追加。
    let mut p = ProviderRow::new();
    p.key = "m1".into();
    p.base_url = "https://api.example.com/v1/messages".into();
    p.pi_api = "anthropic-messages".into();
    let mut m = ModelRow::new();
    m.id = "m1".into();
    p.models = vec![m];

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], std::slice::from_ref(&p), &json!([]), None);
    assert_eq!(
        out[0]["url"],
        json!("https://api.example.com/v1/messages"),
        "已带后缀时不得重复追加"
    );
}

#[test]
fn messages_suffix_never_doubles_v1() {
    // 基址已带 /v1 时补后缀不得拼成 /v1/v1/messages；历史写坏的 /v1/v1/messages
    // 再次保存要收敛回 /v1/messages。
    let b = backends::backend(ConfigFormat::WorkBuddy);
    for (base, expect) in [
        (
            "https://api.example.com",
            "https://api.example.com/v1/messages",
        ),
        (
            "https://api.example.com/v1",
            "https://api.example.com/v1/messages",
        ),
        (
            "https://api.example.com/v1/v1/messages",
            "https://api.example.com/v1/messages",
        ),
    ] {
        let mut p = ProviderRow::new();
        p.key = "m1".into();
        p.base_url = base.into();
        p.pi_api = "anthropic-messages".into();
        let mut m = ModelRow::new();
        m.id = "m1".into();
        p.models = vec![m];
        let out = b.serialize_root(&[], std::slice::from_ref(&p), &json!([]), None);
        assert_eq!(out[0]["url"], json!(expect), "base={base}");
    }
}

#[test]
fn parse_leaves_variants_empty_so_zcode_export_invents_no_reasoning() {
    // WorkBuddy 无「思考档位」概念（只有布尔 supportsReasoning）。解析后 variants
    // 必须为空，否则 ModelRow::new() 的默认档位会在跨格式存到 ZCode 时被凭空写成
    // reasoningLevel.values，污染每个模型。
    let content = r#"[
      { "id": "acct", "name": "some-model", "url": "https://x.invalid/v1",
        "apiKey": "k", "useCustomProtocol": false,
        "maxInputTokens": 256000, "maxOutputTokens": 64000 }
    ]"#;
    let load = load_wb(content);
    assert_eq!(
        load.providers[0].models[0].variants, "",
        "WB 模型 variants 应为空"
    );

    // 跨格式存到 ZCode：不得出现 reasoningLevel。
    let zc = backends::backend(ConfigFormat::ZCode);
    let root = zc.serialize_root(&[], &load.providers, &json!({}), None);
    let specs =
        &root["config"]["modelConfigRules"]["providerModelRules"][0]["config"]["optionSpecs"];
    assert!(
        specs.get("reasoningLevel").is_none(),
        "不该凭空给 ZCode 模型写 reasoningLevel"
    );
}

#[test]
fn model_id_is_the_model_and_name_is_the_provider() {
    // WorkBuddy 约定：条目 `id` = 模型名（发给 API 的模型），`name` = 提供商标签。
    // 解析后模型行 id 拿模型名、provider.key 拿提供商——写回保持不变。
    let content = r#"[
      { "id": "claude-opus-5", "name": "ps.air-outer",
        "url": "https://ps.air-outer.com/v1/messages", "apiKey": "k",
        "useCustomProtocol": true, "maxInputTokens": 272000, "maxOutputTokens": 128000 }
    ]"#;
    let load = load_wb(content);
    let p = &load.providers[0];
    assert_eq!(p.key, "ps.air-outer", "provider key = 提供商（name）");
    assert_eq!(p.models[0].id, "claude-opus-5", "模型行 id = 模型名（id）");

    // 写回：id 仍是模型名，name 仍是提供商。
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], &load.providers, &load.extras, None);
    assert_eq!(out[0]["id"], json!("claude-opus-5"));
    assert_eq!(out[0]["name"], json!("ps.air-outer"));
}

#[test]
fn current_file_save_preserves_unknown_entry_keys() {
    let content = r#"[
      { "id": "m1", "name": "M1", "vendor": "Custom", "url": "https://x.invalid/v1",
        "apiKey": "k", "useCustomProtocol": false,
        "tags": ["custom"], "credits": "x0.5", "isDefault": false,
        "reasoning": { "effort": "medium", "summary": "auto" } }
    ]"#;
    let load = load_wb(content);
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entry = &root[0];
    assert_eq!(entry["tags"], json!(["custom"]), "未知键 tags 必须保留");
    assert_eq!(entry["credits"], json!("x0.5"), "credits 必须保留");
    assert_eq!(
        entry["reasoning"]["effort"],
        json!("medium"),
        "reasoning 块必须保留"
    );
}

#[test]
fn does_not_invent_capability_flags() {
    // 原条目没写 supports* 时不得凭空补，否则会把「未声明」变成「明确不支持」。
    let content = r#"[
      { "id": "m1", "name": "M1", "vendor": "Custom", "url": "https://x.invalid/v1",
        "apiKey": "k", "useCustomProtocol": false }
    ]"#;
    let load = load_wb(content);
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entry = &root[0];
    assert!(
        entry.get("supportsToolCall").is_none(),
        "不该补 supportsToolCall"
    );
    assert!(
        entry.get("supportsImages").is_none(),
        "不该补 supportsImages"
    );
    assert!(
        entry.get("supportsReasoning").is_none(),
        "不该补 supportsReasoning"
    );
}

#[test]
fn cross_format_save_does_not_leak_foreign_keys() {
    // 从 ZCode 形状的 raw 转存：group / access / api 不得泄漏进扁平条目。
    let mut p = ProviderRow::new();
    p.key = "zc1".into();
    p.base_url = "https://api.example.com".into();
    p.api_key = "sk-x".into();
    p.pi_api = "openai-completions".into();
    p.raw = json!({
        "group": "standard-personal",
        "access": { "type": "api-key", "apiKey": "sk-x" },
        "api": { "type": "openai-chat-completions", "baseUrl": "https://api.example.com" }
    });
    let mut m = ModelRow::new();
    m.id = "zc1".into();
    m.raw = json!({ "properties": { "contextWindow": 1000 } });
    p.models = vec![m];

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], std::slice::from_ref(&p), &json!([]), None);
    let entry = &out[0];
    assert!(entry.get("group").is_none(), "group 不该泄漏");
    assert!(entry.get("access").is_none(), "access 不该泄漏");
    assert!(entry.get("api").is_none(), "api 不该泄漏");
    assert_eq!(entry["url"], json!("https://api.example.com"));
}

#[test]
fn modalities_and_supports_map_both_ways() {
    // 布尔 ↔ 列表的双向映射（与 ZCode 侧共用）。
    let pairs = convert::modalities_to_supports("text, image");
    assert!(pairs.contains(&("text", true)));
    assert!(pairs.contains(&("image", true)));
    assert!(pairs.contains(&("video", false)));

    assert_eq!(
        convert::supports_to_modalities([("text", true), ("image", true), ("video", false)]),
        "text, image"
    );
    // WorkBuddy 的 supportsImages 推导
    assert_eq!(
        convert::workbuddy_modalities_from_raw(&json!({ "supportsImages": true })),
        "text, image"
    );
    assert_eq!(
        convert::workbuddy_modalities_from_raw(&json!({ "supportsImages": false })),
        "text"
    );
}

#[test]
fn round_trip_through_file_keeps_credentials() {
    let path = temp_path("models.json");
    std::fs::write(&path, workbuddy_json()).unwrap();
    let load =
        backends::load_backend(ConfigFormat::WorkBuddy, path.to_str().unwrap()).expect("应可加载");
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let text = b.render(&root, false).unwrap();
    assert!(text.contains("sk-chat"), "密钥必须保留");
    assert!(text.contains("sk-msg"), "密钥必须保留");
    assert!(text.trim_start().starts_with('['), "数组根必须保持");
    std::fs::remove_file(&path).ok();
}

/// 一家提供商的多条条目（同 `name`、不同模型），以及跨 provider 的同名模型。
fn multi_model_json() -> String {
    r#"[
      { "id": "claude-opus-5", "name": "claude_agentrouter", "vendor": "Custom",
        "url": "https://ps.air-outer.com/v1/messages", "apiKey": "sk-shared",
        "supportsImages": true, "useCustomProtocol": true,
        "maxInputTokens": 272000, "maxOutputTokens": 128000 },
      { "id": "claude-opus-4-8", "name": "claude_agentrouter", "vendor": "Custom",
        "url": "https://ps.air-outer.com/v1/messages", "apiKey": "sk-shared",
        "supportsImages": true, "useCustomProtocol": true,
        "maxInputTokens": 272000, "maxOutputTokens": 64000 },
      { "id": "claude-opus-5", "name": "claude_linxi", "vendor": "Custom",
        "url": "https://k40.example/v1/messages", "apiKey": "sk-linxi",
        "supportsImages": true, "useCustomProtocol": true,
        "maxInputTokens": 272000, "maxOutputTokens": 128000 }
    ]"#
    .to_string()
}

#[test]
fn parse_groups_one_provider_into_a_single_card() {
    // 文件按模型扁平存储，同一 `name` 的条目就是「一家提供商的多个模型」：
    // 必须合成一张卡片（各自成为一个模型行），否则一家提供商在界面里散成好几张卡。
    let load = load_wb(&multi_model_json());
    assert_eq!(load.providers.len(), 2, "两家提供商");
    let ar = &load.providers[0];
    assert_eq!(ar.key, "claude_agentrouter");
    assert_eq!(ar.description, "Custom", "vendor 落在 provider 上");
    assert_eq!(ar.api_key, "sk-shared");
    assert_eq!(
        ar.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["claude-opus-5", "claude-opus-4-8"],
        "同 provider 的模型按文件顺序合并"
    );
    // 每个模型各自带着自己那条条目的属性，不串味。
    assert_eq!(ar.models[0].output, "128000");
    assert_eq!(ar.models[1].output, "64000");
    assert_eq!(load.providers[1].key, "claude_linxi");
}

#[test]
fn save_writes_one_entry_per_model() {
    // 曾经的 bug：只写 models.first()，一家提供商第二个模型起全部丢失
    // （用户的 models.json 里 openai_zmofas 的 gpt-5.6-terra / grok-4.5 就是这么丢的）。
    let load = load_wb(&multi_model_json());
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entries = root.as_array().expect("数组根");
    assert_eq!(entries.len(), 3, "2 + 1 个模型各写一条：{entries:#?}");
    assert_eq!(
        entries
            .iter()
            .map(|e| (e["name"].as_str().unwrap(), e["id"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        vec![
            ("claude_agentrouter", "claude-opus-5"),
            ("claude_agentrouter", "claude-opus-4-8"),
            ("claude_linxi", "claude-opus-5"),
        ]
    );
    // 同 provider 的条目共用 provider 级字段（url / apiKey / 协议），
    // 模型级字段各写各的。
    assert_eq!(entries[1]["url"], entries[0]["url"]);
    assert_eq!(entries[1]["apiKey"], json!("sk-shared"));
    assert_eq!(entries[1]["maxOutputTokens"], json!(64000));
    assert_eq!(entries[2]["maxOutputTokens"], json!(128000));
}

#[test]
fn entry_id_stays_the_plain_api_model_name() {
    // `id` 是**发给 API 的模型名**。为了「避免重名」把 provider 拼进去
    // （`claude_linxi:claude-opus-5`）就是把非法模型名发给服务端，直接吃
    // "Provider rejected the model request"。同名模型靠 `name` 区分，
    // 这是 WorkBuddy 自己的设计（它的选择器也按 `provider:model` 显示）。
    let load = load_wb(&multi_model_json());
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    for entry in root.as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        let name = entry["name"].as_str().unwrap();
        assert!(!id.contains(':'), "id 不得带命名空间前缀：{id}");
        assert!(!id.contains(name), "id 里不得混入 provider：{id}");
    }
    // 同名模型分属两家时，靠 name 区分而不是改 id。
    let dupes: Vec<&str> = root
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["id"] == json!("claude-opus-5"))
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(dupes, vec!["claude_agentrouter", "claude_linxi"]);
}

#[test]
fn same_model_id_across_providers_keeps_its_own_entry_fields() {
    // 旧实现按 `id` 单独建索引：目标文件里 `gpt-5.6-sol` 有四条（各家一条），
    // 后写的 provider 会认领到别家条目的 tags / credits 等字段。
    // 索引必须是 (name, id) 二元组。
    let target = json!([
        { "id": "gpt-5.6-sol", "name": "openai_apizh", "url": "https://a.example/v1",
          "useCustomProtocol": false, "tags": ["apizh-only"] },
        { "id": "gpt-5.6-sol", "name": "openai_leyi", "url": "https://b.example/v1",
          "useCustomProtocol": false, "tags": ["leyi-only"] }
    ]);
    let provider = |key: &str, url: &str, secret: &str| {
        let mut p = ProviderRow::new();
        p.key = key.to_string();
        p.base_url = url.to_string();
        p.api_key = secret.to_string();
        p.pi_api = "openai-completions".into();
        let mut m = ModelRow::new();
        m.id = "gpt-5.6-sol".into();
        p.models = vec![m];
        p
    };
    let providers = vec![
        provider("openai_apizh", "https://a.example/v1", "sk-a"),
        provider("openai_leyi", "https://b.example/v1", "sk-b"),
    ];
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], &providers, &json!([]), Some(&target));
    let entries = out.as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0]["tags"],
        json!(["apizh-only"]),
        "甲家条目不得继承乙家字段"
    );
    assert_eq!(entries[1]["tags"], json!(["leyi-only"]));
    assert_eq!(entries[0]["apiKey"], json!("sk-a"));
    assert_eq!(entries[1]["apiKey"], json!("sk-b"));
}

#[test]
fn a_provider_without_models_still_writes_one_entry() {
    // 刚建好还没填模型的卡片保存后不能凭空消失（id 回落到 provider key）。
    let mut p = ProviderRow::new();
    p.key = "brand-new".into();
    p.base_url = "https://new.example/v1".into();
    p.pi_api = "openai-completions".into();
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], std::slice::from_ref(&p), &json!([]), None);
    assert_eq!(out.as_array().unwrap().len(), 1);
    assert_eq!(out[0]["id"], json!("brand-new"));
    assert_eq!(out[0]["name"], json!("brand-new"));
}
