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
