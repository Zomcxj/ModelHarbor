//! WorkBuddy backend regression: discrimination / parsing / serialization of `~/.workbuddy/models.json`.
//!
//! Structural notes: the root is a top-level array with provider info inlined into each model entry;
//! the protocol is expressed via `useCustomProtocol` plus the URL suffix — the file has no protocol field.

use model_harbor_core::backends;
use model_harbor_core::convert;
use model_harbor_core::format::ConfigFormat;
use model_harbor_core::model::{ModelRow, ProviderRow};
use serde_json::json;

fn temp_path(name: &str) -> std::path::PathBuf {
    // Directory names must be globally unique: Windows clock granularity is coarse (millisecond), and parallel
    // tests relying on pid + nanoseconds can land in the same tick, sharing a directory and overwriting each
    // other's files (this once made duplicate_ids fail flakily under full parallelism). An atomic counter
    // eliminates that collision class.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_wb_{}_{}_{}",
        std::process::id(),
        nonce,
        seq
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Realistic sample: one Chat entry (custom protocol unchecked) + one Messages entry (checked).
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
    // `[]` is not recognized
    assert!(!b.detect("[]", ""));
    // Counterexample
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

    // Custom protocol unchecked → Chat Completions
    let chat = &load.providers[0];
    assert_eq!(chat.key, "chat-model");
    assert_eq!(chat.pi_api, "openai-completions");
    assert_eq!(chat.base_url, "http://127.0.0.1:3065/v1");
    assert_eq!(chat.api_key, "sk-chat");

    // Custom protocol checked → derived from the URL suffix
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
        // Must re-parse back (self-consistent round trip)
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
    // With the custom protocol checked, the URL is used verbatim.
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
    // After the suffix is appended, /v1/v1/messages must not appear.
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
    // WorkBuddy has only a boolean `supportsReasoning`, no reasoning tiers; variants are empty after parsing.
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

    // Saved cross-format to ZCode: reasoningLevel must not appear.
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
    // WorkBuddy convention: entry `id` = model name (sent to the API), `name` = provider label;
    // write-back keeps that.
    let content = r#"[
      { "id": "claude-opus-5", "name": "ps.air-outer",
        "url": "https://ps.air-outer.com/v1/messages", "apiKey": "k",
        "useCustomProtocol": true, "maxInputTokens": 272000, "maxOutputTokens": 128000 }
    ]"#;
    let load = load_wb(content);
    let p = &load.providers[0];
    assert_eq!(p.key, "ps.air-outer", "provider key = 提供商（name）");
    assert_eq!(p.models[0].id, "claude-opus-5", "模型行 id = 模型名（id）");

    // Write-back: id is still the model name, name still the provider.
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
    // Entries without supports* in the source get nothing added.
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
    // Converting from a ZCode-shaped raw: group / access / api must not leak into flat entries.
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
    // The boolean ↔ list two-way mapping (shared with the ZCode side).
    let pairs = convert::modalities_to_supports("text, image");
    assert!(pairs.contains(&("text", true)));
    assert!(pairs.contains(&("image", true)));
    assert!(pairs.contains(&("video", false)));

    assert_eq!(
        convert::supports_to_modalities([("text", true), ("image", true), ("video", false)]),
        "text, image"
    );
    // WorkBuddy's supportsImages derivation
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
fn undeclared_supports_images_is_not_reported_as_text_only() {
    // An entry without supportsImages is "undeclared", not "text-only".
    assert_eq!(
        convert::workbuddy_modalities_from_raw(&json!({ "id": "m", "name": "p" })),
        "",
        "未声明模态时必须留空，不能默认成 text"
    );

    // Entries with undeclared modalities convert to ZCode without writing inputFormat.
    let mut p = ProviderRow::new();
    p.key = "p1".into();
    p.base_url = "https://x.example/v1".into();
    p.api_key = "sk-1".into();
    p.pi_api = "openai-completions".into();
    let mut m = ModelRow::new();
    m.id = "m1".into();
    m.context = "272000".into();
    // Clear ModelRow::new's default modalities to simulate an undeclared source.
    m.modalities_input.clear();
    m.raw = json!({ "id": "m1", "name": "p1", "url": "https://x.example/v1" });
    p.models = vec![m];

    let z = backends::backend(ConfigFormat::ZCode);
    let root = z.serialize_root(&[], std::slice::from_ref(&p), &json!({}), None);
    let props =
        &root["config"]["modelConfigRules"]["providerModelRules"][0]["config"]["properties"];
    assert!(
        props.get("inputFormat").is_none(),
        "未声明模态不该给 ZCode 补 inputFormat: {props}"
    );
    assert_eq!(props["contextWindow"], json!(272000), "上下文照常带过去");
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

/// Multiple entries of one provider (same `name`, different models), plus same-named models across providers.
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
    // The file stores models flat; entries with the same `name` merge into one card, each becoming a model row.
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
    // Each model carries its own entry's attributes.
    assert_eq!(ar.models[0].output, "128000");
    assert_eq!(ar.models[1].output, "64000");
    assert_eq!(load.providers[1].key, "claude_linxi");
}

#[test]
fn save_writes_one_entry_per_model() {
    // One entry per model of a provider; the second same-id entry isn't written.
    let load = load_wb(&multi_model_json());
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entries = root.as_array().expect("数组根");
    assert_eq!(
        entries.len(),
        2,
        "同 id 的第二条被关闭，不写盘：{entries:#?}"
    );
    assert_eq!(
        entries
            .iter()
            .map(|e| (e["name"].as_str().unwrap(), e["id"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        vec![
            ("claude_agentrouter", "claude-opus-5"),
            ("claude_agentrouter", "claude-opus-4-8"),
        ],
        "同一家提供商的多个模型都要写出来"
    );
    // A provider's entries share provider-level fields (url / apiKey / protocol);
    // model-level fields are written per entry.
    assert_eq!(entries[1]["url"], entries[0]["url"]);
    assert_eq!(entries[1]["apiKey"], json!("sk-shared"));
    assert_eq!(entries[1]["maxOutputTokens"], json!(64000));
}

#[test]
fn duplicate_ids_keep_only_the_first_enabled() {
    // WorkBuddy's selector dedupes globally by bare id, honoring only the first; the first entry per id enables,
    // the rest disable and stay out of the config.
    let load = load_wb(&multi_model_json());
    let flags: Vec<(String, bool)> = load
        .providers
        .iter()
        .flat_map(|p| {
            p.models
                .iter()
                .map(|m| (m.id.clone(), m.disabled))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        flags,
        vec![
            ("claude-opus-5".to_string(), false),
            ("claude-opus-4-8".to_string(), false),
            // linxi's claude-opus-5 shares the first's name and defaults to disabled.
            ("claude-opus-5".to_string(), true),
        ],
        "每个 id 的第一条启用，其余关闭"
    );
    assert_eq!(flags.len(), 3, "重复条目也要进界面，用户才看得见并切换");
}

#[test]
fn entry_id_stays_the_plain_api_model_name() {
    // `id` is the model name sent to the API; every written id equals some original model name verbatim.
    let load = load_wb(&multi_model_json());
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    // The original model-name set: every written id must equal exactly one of them.
    let originals: Vec<String> = load
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| m.id.clone()))
        .collect();
    for entry in root.as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        let name = entry["name"].as_str().unwrap();
        assert!(!id.contains(':'), "id 不得带命名空间前缀：{id}");
        assert!(!id.contains(name), "id 里不得混入 provider：{id}");
        assert!(
            originals.iter().any(|o| o == id),
            "id 必须是原始模型名逐字，不得改名或加序号：{id}"
        );
    }
    // Same-named models across two providers: only the first is written by default.
    let opus: Vec<(&str, &str)> = root
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["id"].as_str().unwrap() == "claude-opus-5")
        .map(|e| (e["name"].as_str().unwrap(), e["id"].as_str().unwrap()))
        .collect();
    assert_eq!(
        opus,
        vec![("claude_agentrouter", "claude-opus-5")],
        "同名模型默认只留第一条，且 id 保持原模型名"
    );
}

#[test]
fn same_model_id_across_providers_keeps_its_own_entry_fields() {
    // Indexing by the (name, id) pair: the target file holds several entries per id (one per provider),
    // each inheriting only its own provider's old entry's fields.
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
    // Only the first entry per id is written; the kept one inherits its own provider's old fields.
    assert_eq!(entries.len(), 1, "同 id 只留第一条：{entries:#?}");
    assert_eq!(entries[0]["name"], json!("openai_apizh"));
    assert_eq!(
        entries[0]["tags"],
        json!(["apizh-only"]),
        "甲家条目不得继承乙家字段"
    );
    assert_eq!(entries[0]["apiKey"], json!("sk-a"));
}

#[test]
fn a_provider_without_models_still_writes_one_entry() {
    // A card with no models still writes one entry, id falling back to the provider key.
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

#[test]
fn duplicate_model_ids_are_written_verbatim_never_renamed() {
    // ids are written verbatim — no numbering, no vendor concatenation; from the second same-id on, nothing is written.
    let provider = |key: &str, model: &str| {
        let mut p = ProviderRow::new();
        p.key = key.to_string();
        p.base_url = "https://x.example/v1".into();
        p.pi_api = "openai-completions".into();
        let mut m = ModelRow::new();
        m.id = model.to_string();
        p.models = vec![m];
        p
    };
    let providers = vec![
        provider("a", "gpt-5.6-sol"),
        provider("b", "gpt-5.6-sol"),
        provider("c", "gpt-5.6-sol"),
        provider("d", "unique-model"),
    ];
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let out = b.serialize_root(&[], &providers, &json!([]), None);
    let ids: Vec<&str> = out
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    // From the second same-id on nothing is written — two entries remain.
    assert_eq!(
        ids,
        vec!["gpt-5.6-sol", "unique-model"],
        "id 是上游模型名，必须原样写出，重复也不许改名"
    );
    let names: Vec<&str> = out
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["a", "d"], "同 id 只留第一条，其余不写盘");
}

#[test]
fn genuine_model_names_ending_in_digits_are_left_alone() {
    // Model names already ending in digits must survive byte for byte.
    let saved = json!([
        { "id": "deepseek-v4-flash-0731", "name": "gm_huige0", "url": "https://x.example/v1" },
        { "id": "claude-opus-5", "name": "claude_a", "url": "https://x.example/v1" },
        { "id": "MiniMax-M3", "name": "openai_hyper", "url": "https://x.example/v1" },
        { "id": "grok-4.6", "name": "grok_247kan", "url": "https://x.example/v1" }
    ]);
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let text = serde_json::to_string(&saved).unwrap();
    let load = b.parse(&text).unwrap();
    let mut ids: Vec<&str> = load
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| m.id.as_str()))
        .collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        vec![
            "MiniMax-M3",
            "claude-opus-5",
            "deepseek-v4-flash-0731",
            "grok-4.6"
        ],
        "末尾是数字的合法模型名必须原样保留"
    );
}

#[test]
fn saving_twice_is_byte_stable() {
    // Save → read → save again must be byte-identical.
    let provider = |key: &str, model: &str| {
        let mut p = ProviderRow::new();
        p.key = key.to_string();
        p.base_url = "https://x.example/v1".into();
        p.pi_api = "openai-completions".into();
        p.api_key = "sk-x".into();
        let mut m = ModelRow::new();
        m.id = model.to_string();
        p.models = vec![m];
        p
    };
    let providers = vec![
        provider("a", "gpt-5.6-sol"),
        provider("b", "gpt-5.6-sol"),
        provider("c", "claude-opus-5"),
    ];
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let first = b.serialize_root(&[], &providers, &json!([]), None);
    let text1 = b.render(&first, false).unwrap();
    // Two same-id entries keep only the first.
    let names: Vec<&str> = first
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["a", "c"], "同 id 只留第一条");

    let reloaded = b.parse(&text1).unwrap();
    let second = b.serialize_root(
        &[],
        &reloaded.providers,
        &reloaded.extras,
        Some(&reloaded.extras),
    );
    let text2 = b.render(&second, false).unwrap();
    assert_eq!(text1, text2, "存→读→再存 必须字节一致");
}

#[test]
fn disabled_models_are_omitted_from_the_file() {
    // Disabled models stay out of the config; the kept one writes `disabled: false` explicitly.
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://x.example/v1" }
    ]);
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let text = serde_json::to_string(&saved).unwrap();
    let load = b.parse(&text).unwrap();
    // Read: both enter the UI; the first per id enables, the rest disable.
    let flags: Vec<bool> = load
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| m.disabled))
        .collect();
    assert_eq!(flags, vec![false, true], "第一条启用，第二条关闭");
    let names: Vec<String> = load.providers.iter().map(|p| p.key.clone()).collect();
    assert_eq!(names, vec!["a", "b"], "两条都要进界面");

    // Write: only the enabled one is written.
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entries = root.as_array().unwrap();
    assert_eq!(entries.len(), 1, "关闭的模型不写盘：{entries:#?}");
    assert_eq!(entries[0]["name"], json!("a"));
    assert_eq!(
        entries[0]["disabled"],
        json!(false),
        "生效清单里的条目要显式写 disabled: false"
    );

    // Switch to enabling b: the file should hold only b, with b's fields complete.
    let mut providers = load.providers.clone();
    providers[0].models[0].disabled = true;
    providers[1].models[0].disabled = false;
    let root2 = b.serialize_root(&[], &providers, &load.extras, Some(&root));
    let entries2 = root2.as_array().unwrap();
    assert_eq!(entries2.len(), 1, "切换后只留新启用的那条");
    assert_eq!(entries2[0]["name"], json!("b"));
    assert_eq!(entries2[0]["id"], json!("gpt-5.6-sol"), "id 始终是原模型名");
    assert_eq!(entries2[0]["url"], json!("https://x.example/v1"));
}

#[test]
fn legacy_disabled_keys_are_honored_not_ignored() {
    // An earlier version wrote `disabled`. The main config (`parse`, positional derivation) ignores that key,
    // and the written effective list always sets it `false`.
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1",
          "disabled": true },
        { "id": "claude-opus-5", "name": "b", "url": "https://x.example/v1",
          "disabled": false }
    ]);
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let text = serde_json::to_string(&saved).unwrap();
    let load = b.parse(&text).unwrap();
    let root = b.serialize_root(&[], &load.providers, &load.extras, Some(&saved));
    for e in root.as_array().unwrap() {
        assert_eq!(
            e.get("disabled"),
            Some(&json!(false)),
            "生效清单里的每条都是启用的，该键恒为 false：{e:#?}"
        );
    }
}

/// Entries in the effective list (`models.json`) are always enabled: `disabled` is always `false` and always written.
#[test]
fn the_effective_list_marks_every_entry_enabled() {
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1" },
        { "id": "claude-opus-5", "name": "b", "url": "https://x.example/v1" }
    ]);
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let text = serde_json::to_string(&saved).unwrap();
    let load = b.parse(&text).unwrap();
    let root = b.serialize_root(&[], &load.providers, &load.extras, Some(&saved));
    for e in root.as_array().unwrap() {
        assert_eq!(
            e.get("disabled"),
            Some(&json!(false)),
            "生效清单里每条都要显式写 disabled: false：{e:#?}"
        );
    }
}

/// The full copy must write `disabled` on every entry; checked entries write `false`.
#[test]
fn the_full_store_records_every_entrys_flag_explicitly() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1" },
        { "id": "claude-opus-5", "name": "c", "url": "https://c.example/v1" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&saved).unwrap()).unwrap();
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b.parse_at(&saved.to_string(), &path).unwrap();
    b.save_sidecars(&path, &load.providers).expect("写全量副本");

    let full: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(model_harbor_core::backends::workbuddy::full_store_path(
            &path,
        ))
        .unwrap(),
    )
    .unwrap();
    let items = full.as_array().unwrap();
    assert_eq!(items.len(), 3, "全量副本含全部条目");
    for (i, e) in items.iter().enumerate() {
        assert!(
            e.get("disabled").is_some(),
            "第 {i} 条必须显式写 disabled（勾选也要写 false）：{e:#?}"
        );
    }
    // Positional derivation: the first per id enables (false), the rest disable (true).
    assert_eq!(items[0]["disabled"], json!(false), "第一条启用");
    assert_eq!(items[1]["disabled"], json!(true), "同 id 第二条关闭");
    assert_eq!(items[2]["disabled"], json!(false), "另一个 id 启用");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// A legacy copy with no `disabled` keys at all derives check states by position on load.
#[test]
fn a_full_store_without_flags_heals_to_first_occurrence_enabled() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    // Old copy: three same-named entries, not one disabled key among them.
    let flagless = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1" },
        { "id": "claude-opus-5", "name": "c", "url": "https://c.example/v1" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&flagless).unwrap()).unwrap();
    std::fs::write(
        model_harbor_core::backends::workbuddy::full_store_path(&path),
        serde_json::to_string(&flagless).unwrap(),
    )
    .unwrap();

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b
        .parse_at(&std::fs::read_to_string(&dir).unwrap(), &path)
        .unwrap();
    let flags: Vec<(String, bool)> = load
        .providers
        .iter()
        .flat_map(|p| {
            p.models
                .iter()
                .map(|m| (m.id.clone(), m.disabled))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        flags,
        vec![
            ("gpt-5.6-sol".to_string(), false),
            ("gpt-5.6-sol".to_string(), true),
            ("claude-opus-5".to_string(), false),
        ],
        "没有标记的旧副本必须回退到按位置推导，而不是把每条都当成启用"
    );

    // Save once so the flags persist; reloading must no longer depend on derivation.
    b.save_sidecars(&path, &load.providers).expect("写全量副本");
    let healed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(model_harbor_core::backends::workbuddy::full_store_path(
            &path,
        ))
        .unwrap(),
    )
    .unwrap();
    for e in healed.as_array().unwrap() {
        assert!(e.get("disabled").is_some(), "自愈后每条都有标记：{e:#?}");
    }
    let again = b
        .parse_at(&std::fs::read_to_string(&dir).unwrap(), &path)
        .unwrap();
    let flags2: Vec<bool> = again
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| m.disabled))
        .collect();
    assert_eq!(flags2, vec![false, true, false], "二次加载保持一致");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// After the user manually adjusts checks, no more position-based derivation.
#[test]
fn explicit_flags_survive_the_round_trip_even_when_they_break_position_order() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1",
          "apiKey": "sk-a" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1",
          "apiKey": "sk-b" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&saved).unwrap()).unwrap();
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b.parse_at(&saved.to_string(), &path).unwrap();
    // The user switches to enabling the second entry (opposite of "first wins").
    let mut providers = load.providers.clone();
    providers[0].models[0].disabled = true;
    providers[1].models[0].disabled = false;
    let root = b.serialize_root(&[], &providers, &load.extras, Some(&saved));
    b.save_sidecars(&path, &providers).expect("写全量副本");
    std::fs::write(&dir, b.render(&root, false).unwrap()).unwrap();

    let reloaded = b
        .parse_at(&std::fs::read_to_string(&dir).unwrap(), &path)
        .unwrap();
    assert!(
        reloaded.providers[0].models[0].disabled,
        "用户关掉的第一条必须还是关闭"
    );
    assert!(
        !reloaded.providers[1].models[0].disabled,
        "用户勾上的第二条必须还是勾选——不能被按位置推导覆盖回第一条"
    );
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// Full-copy path: same directory as the main config, fixed file name.
#[test]
fn full_store_sits_next_to_models_json() {
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let _ = b;
    let path = r"C:\Users\me\.workbuddy\models.json";
    let full = model_harbor_core::backends::workbuddy::full_store_path(path);
    assert_eq!(full, r"C:\Users\me\.workbuddy\models.full.json");
    // The copy's name differs from the main config's file name.
    assert!(!full.ends_with("models.json") || full.ends_with("models.full.json"));
    assert_ne!(
        std::path::Path::new(&full).file_name(),
        std::path::Path::new(path).file_name()
    );
    // WSL paths separate with `/`, same rules as `credentials::sidecar_path`.
    assert_eq!(
        model_harbor_core::backends::workbuddy::full_store_path("/home/me/.workbuddy/models.json"),
        "/home/me/.workbuddy/models.full.json"
    );
}

/// Unchecked entries still remain in the full copy.
#[test]
fn unchecking_a_model_keeps_it_in_the_full_store() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1",
          "apiKey": "sk-a", "tags": ["a-only"] },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1",
          "apiKey": "sk-b", "tags": ["b-only"] }
    ]);
    let text = serde_json::to_string(&saved).unwrap();
    std::fs::write(&dir, &text).unwrap();

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b.parse_at(&text, &path).unwrap();
    assert_eq!(load.providers.len(), 2, "两条都要进界面");

    // The user checks the second too (mutually exclusive: the first auto-disables).
    let mut providers = load.providers.clone();
    providers[0].models[0].disabled = true;
    providers[1].models[0].disabled = false;

    let root = b.serialize_root(&[], &providers, &load.extras, Some(&saved));
    assert_eq!(root.as_array().unwrap().len(), 1, "生效清单只留勾选那条");
    assert_eq!(root.as_array().unwrap()[0]["name"], json!("b"));
    // Write both, in save.rs's order: the full copy first, then the main config.
    b.save_sidecars(&path, &providers).expect("写全量副本");
    std::fs::write(&dir, b.render(&root, false).unwrap()).unwrap();

    // Both entries are in the full copy, each carrying its own fields.
    let full_path = model_harbor_core::backends::workbuddy::full_store_path(&path);
    let full: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&full_path).unwrap()).unwrap();
    let items = full.as_array().unwrap();
    assert_eq!(items.len(), 2, "全量副本必须保留被取消勾选的那条");
    assert_eq!(items[0]["tags"], json!(["a-only"]));
    assert_eq!(items[1]["tags"], json!(["b-only"]));
    assert_eq!(
        items[0]["apiKey"],
        json!("sk-a"),
        "被取消勾选那条的 key 不能丢"
    );
    assert_eq!(items[0]["disabled"], json!(true), "勾选状态记录在副本里");

    // Reload: the copy is read first; both entries present with check states restored as-is.
    let reloaded = b
        .parse_at(&std::fs::read_to_string(&dir).unwrap(), &path)
        .unwrap();
    assert_eq!(reloaded.providers.len(), 2, "副本存在时按副本还原全部条目");
    assert!(reloaded.providers[0].models[0].disabled, "第一条仍是关闭");
    assert!(!reloaded.providers[1].models[0].disabled, "第二条仍是勾选");

    // Checking back: only one per id can be effective; with both checked, the first in order stays.
    let mut back = reloaded.providers.clone();
    back[0].models[0].disabled = false;
    let root2 = b.serialize_root(&[], &back, &reloaded.extras, None);
    assert_eq!(root2.as_array().unwrap().len(), 1);
    assert_eq!(
        root2.as_array().unwrap()[0]["name"],
        json!("a"),
        "按顺序取第一条"
    );

    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// Falls back to the main config when the copy is missing, deriving check states by position.
#[test]
fn without_the_full_store_the_effective_list_is_read_by_position() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&saved).unwrap()).unwrap();
    assert!(
        !std::path::Path::new(&model_harbor_core::backends::workbuddy::full_store_path(
            &path
        ))
        .exists(),
        "这个用例里不该有副本"
    );
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b.parse_at(&saved.to_string(), &path).unwrap();
    assert_eq!(load.providers.len(), 2);
    assert!(!load.providers[0].models[0].disabled, "第一条启用");
    assert!(load.providers[1].models[0].disabled, "同 id 的第二条关闭");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// After the user hand-edits `models.json`, newly added entries show up in the UI.
#[test]
fn hand_added_entries_in_models_json_still_show_up() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    let saved = json!([
        { "id": "old-model", "name": "a", "url": "https://a.example/v1", "apiKey": "sk-a" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&saved).unwrap()).unwrap();
    let b = backends::backend(ConfigFormat::WorkBuddy);

    // Save once to create the full copy (only old-model exists at this point).
    let load = b.parse_at(&saved.to_string(), &path).unwrap();
    b.save_sidecars(&path, &load.providers).unwrap();

    // The user hand-edits the main config, adding a new model.
    let hand = json!([
        { "id": "old-model", "name": "a", "url": "https://a.example/v1", "apiKey": "sk-a" },
        { "id": "hand-added", "name": "z", "url": "https://z.example/v1", "apiKey": "sk-z" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&hand).unwrap()).unwrap();

    let reloaded = b.parse_at(&hand.to_string(), &path).unwrap();
    let ids: Vec<String> = reloaded
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| m.id.clone()))
        .collect();
    assert!(
        ids.contains(&"hand-added".to_string()),
        "手加的条目必须出现在界面上：{ids:?}"
    );
    assert!(ids.contains(&"old-model".to_string()));
    // The hand-added entry belongs to the effective list and must be enabled.
    let added = reloaded
        .providers
        .iter()
        .flat_map(|p| p.models.iter())
        .find(|m| m.id == "hand-added")
        .unwrap();
    assert!(!added.disabled, "主配置里的条目是生效的，应显示为启用");

    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// When multiple entries share one id and all read as enabled, loading converges to enabling only the first.
#[test]
fn duplicate_ids_all_marked_enabled_are_reduced_to_the_first() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    // Three same-named entries, each explicitly writing disabled: false.
    let all_on = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1",
          "disabled": false },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1",
          "disabled": false },
        { "id": "gpt-5.6-sol", "name": "c", "url": "https://c.example/v1",
          "disabled": false }
    ]);
    std::fs::write(&dir, serde_json::to_string(&all_on).unwrap()).unwrap();
    std::fs::write(
        model_harbor_core::backends::workbuddy::full_store_path(&path),
        serde_json::to_string(&all_on).unwrap(),
    )
    .unwrap();

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b
        .parse_at(&std::fs::read_to_string(&dir).unwrap(), &path)
        .unwrap();
    let flags: Vec<(String, bool)> = load
        .providers
        .iter()
        .flat_map(|p| {
            p.models
                .iter()
                .map(|m| (p.key.clone(), m.disabled))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        flags,
        vec![
            ("a".to_string(), false),
            ("b".to_string(), true),
            ("c".to_string(), true),
        ],
        "同一 id 全标记为启用时必须收敛：只留第一条，其余关闭"
    );

    // The written effective list should hold exactly one entry, explicitly enabled.
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entries = root.as_array().unwrap();
    assert_eq!(entries.len(), 1, "生效清单只留一条：{entries:#?}");
    assert_eq!(entries[0]["name"], json!("a"));
    assert_eq!(entries[0]["disabled"], json!(false));
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// With explicit flags present, dedup respects them instead of deriving from file order.
#[test]
fn an_explicit_choice_wins_over_position_derived_duplicates() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    // The full copy: two same-named entries. The first has no flag, the second explicitly enabled (`disabled: false`).
    let store = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1",
          "disabled": false }
    ]);
    std::fs::write(&dir, serde_json::to_string(&store).unwrap()).unwrap();
    std::fs::write(
        model_harbor_core::backends::workbuddy::full_store_path(&path),
        serde_json::to_string(&store).unwrap(),
    )
    .unwrap();

    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b
        .parse_at(&std::fs::read_to_string(&dir).unwrap(), &path)
        .unwrap();
    let flags: Vec<(String, bool)> = load
        .providers
        .iter()
        .flat_map(|p| {
            p.models
                .iter()
                .map(|m| (p.key.clone(), m.disabled))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        flags,
        vec![("a".to_string(), true), ("b".to_string(), false)],
        "用户显式勾的 b 必须保持启用；没有标记、按位置推出来的 a 让位"
    );
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// Invalid numbers are not written as 0 — the write is skipped instead.
#[test]
fn invalid_numbers_are_skipped_not_written_as_zero() {
    let mut load = load_wb(&workbuddy_json());
    // The first entry's context is set to a non-number; the second keeps a normal value.
    load.providers[0].models[0].context = "五十万".into();
    load.providers[0].models[0].output = "".into();
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entries = root.as_array().expect("WorkBuddy 根应是数组");
    let first = entries
        .iter()
        .find(|e| e["id"] == json!("chat-model"))
        .expect("chat-model 条目应存在");
    // Old entries inherit by (name, id), so "skip the write" shows up as the original value preserved.
    assert_eq!(
        first["maxInputTokens"],
        json!(500000),
        "解析不了应保持原值，不能写成 0"
    );
    assert_eq!(
        first["maxOutputTokens"],
        json!(65536),
        "空值同样应保持原值，不能写成 0"
    );
    let second = entries
        .iter()
        .find(|e| e["id"] == json!("messages-model"))
        .expect("messages-model 条目应存在");
    assert_eq!(second["maxInputTokens"], json!(272000), "正常值照旧写入");
}
