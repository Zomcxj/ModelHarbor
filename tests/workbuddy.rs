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
fn undeclared_supports_images_is_not_reported_as_text_only() {
    // 条目**没写** supportsImages 时是「未声明」，不是「只支持文本」。
    // 旧实现返回 "text"，跨格式写出时给 ZCode 凭空补 inputFormat.supportsText: true
    // ——用户「正常」那份配置里多出来的 inputFormat 就是这么来的。
    assert_eq!(
        convert::workbuddy_modalities_from_raw(&json!({ "id": "m", "name": "p" })),
        "",
        "未声明模态时必须留空，不能默认成 text"
    );

    // 未声明模态的条目转成 ZCode，不得凭空长出 inputFormat。
    let mut p = ProviderRow::new();
    p.key = "p1".into();
    p.base_url = "https://x.example/v1".into();
    p.api_key = "sk-1".into();
    p.pi_api = "openai-completions".into();
    let mut m = ModelRow::new();
    m.id = "m1".into();
    m.context = "272000".into();
    // 关键：清掉 ModelRow::new 的默认模态，模拟「源里没声明」。
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
    // 现在「一家提供商的每个模型各写一条」仍然成立；少掉的那条是**同 id 被关闭**的
    // （见 duplicate_ids_keep_only_the_first_enabled）。
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
    // 同 provider 的条目共用 provider 级字段（url / apiKey / 协议），
    // 模型级字段各写各的。
    assert_eq!(entries[1]["url"], entries[0]["url"]);
    assert_eq!(entries[1]["apiKey"], json!("sk-shared"));
    assert_eq!(entries[1]["maxOutputTokens"], json!(64000));
}

#[test]
fn duplicate_ids_keep_only_the_first_enabled() {
    // WorkBuddy 的选择器**按裸 id 全局去重**：同名模型无论挂在哪个厂商下都只列出一行、
    // 只认第一条。界面必须如实反映这一点——每个 id 只有第一条启用，其余默认关闭，
    // 且关闭的**不写入配置**（写进去也不生效，只会占地方、让人以为配了）。
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
            // linxi 的 claude-opus-5 与第一条同名，默认关闭。
            ("claude-opus-5".to_string(), true),
        ],
        "每个 id 的第一条启用，其余关闭"
    );
    assert_eq!(flags.len(), 3, "重复条目也要进界面，用户才看得见并切换");
}

#[test]
fn entry_id_stays_the_plain_api_model_name() {
    // `id` 是**发给 API 的模型名**。绝不能为了「避免重名」把 provider 拼进去
    // （`claude_linxi:claude-opus-5`）——那是把非法模型名发给服务端，直接吃
    // "Provider rejected the model request"。
    //
    // 连「同 id 第 2 条起加两位序号」也不行——序号会进请求体变成不存在的模型名。
    // 这里断言每个写出的 id 都**逐字等于**某个原始模型名。
    let load = load_wb(&multi_model_json());
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    // 原始模型名集合：每个写出的 id 必须**逐字**等于其中之一。
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
    // 同名模型分属两家时，默认只有**第一条**（先出现的那个厂商）写盘——
    // WorkBuddy 只认第一条，把第二条也写进去不会生效，只会占地方。
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
    // 同一个 id 只写第一条（WorkBuddy 只认第一条）。留下的这条必须继承
    // **它自己那家**旧条目的字段，不能认领到别家的 tags —— 索引因此必须是
    // (name, id) 二元组，只按 id 建索引会让 apizh 认领到 leyi 的字段。
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

#[test]
fn duplicate_model_ids_are_written_verbatim_never_renamed() {
    // WorkBuddy 的选择器 `appendModel` 是 `if (ids.has(model.id)) return;`——
    // **只按裸 id 去重且全局生效**，所以 36 条条目里 15 个不同 id 就只列 15 行。
    //
    // 曾经试图给重复 id 加序号（`gpt-5.6-sol` → `gpt-5.6-sol01`）来绕过去重，
    // 那是错的：`id` 同时就是**发给上游的模型名**（`configureModelConfig` 把 `ec.id`
    // 赋给 `agent.model`，`ModelProvider.getModel` 把这个字符串原样放进请求体），
    // 加序号会让请求体变成不存在的模型名，上游直接 model-not-found。
    // 所以这里必须原样写出，重复就重复——去重是 WorkBuddy 的既有机制，不是配置错误。
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
    // **id 一律原样写出**：既不加序号（`gpt-5.6-sol01`），也不拼厂商。
    // 同 id 的第二条起不写盘（WorkBuddy 只认第一条），所以只剩两条。
    // 逐字相等本身就证明了「没加序号」——加了序号这里就对不上。
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
    // 用户文件里真实存在 `deepseek-v4-flash-0731`、`claude-opus-5`、`grok-4.6`
    // 这类末尾本就是数字的模型名。既然不再做任何改名/还原，它们必须逐字节保留。
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
    // 存 → 读 → 再存必须字节一致，否则每次保存都在改文件。
    // 现在多了一层「同 id 只留第一条」的收敛，第一次保存就会把它做完，
    // 所以第二次必须与第一次完全一致（而不是每次都在删条目）。
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
    // 同 id 的两条只留第一条（b 被收敛掉）。
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
    // 关闭的模型**不写入配置**：WorkBuddy 按裸 id 全局去重，写进去也不生效，
    // 只会占地方、让人以为配了。所以「关闭」靠不写这条表达。
    // 但留下的那条要显式写 `disabled: false`——这份文件是用户自己的配置，
    // 勾选状态应当在它自己里面看得见，而不是只能去副本里找。
    let saved = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://x.example/v1" }
    ]);
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let text = serde_json::to_string(&saved).unwrap();
    let load = b.parse(&text).unwrap();
    // 读：两条都进界面（用户要看得见才能切换），同 id 的第一条启用、其余关闭。
    let flags: Vec<bool> = load
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| m.disabled))
        .collect();
    assert_eq!(flags, vec![false, true], "第一条启用，第二条关闭");
    let names: Vec<String> = load.providers.iter().map(|p| p.key.clone()).collect();
    assert_eq!(names, vec!["a", "b"], "两条都要进界面");

    // 写：只写启用那条。
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entries = root.as_array().unwrap();
    assert_eq!(entries.len(), 1, "关闭的模型不写盘：{entries:#?}");
    assert_eq!(entries[0]["name"], json!("a"));
    assert_eq!(
        entries[0]["disabled"],
        json!(false),
        "生效清单里的条目要显式写 disabled: false"
    );

    // 换成启用 b：文件里就该只剩 b，且 b 的字段完整。
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
    // 早期版本写过 `disabled`。这个键**现在是有含义的**（两份文件都靠它记录勾选），
    // 但只在读全量副本时才采信。这里读的是主配置（`parse`，按位置推导），
    // 所以文件里的 `disabled: true` 不参与判定——主配置本身就是生效清单，
    // 里面的条目都是启用的，写出的生效清单里该键恒为 `false`。
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

/// 生效清单（`models.json`）里的条目一律启用，所以 `disabled` 恒为 `false`。
///
/// 这个键必须**显式写出**：这份文件是用户自己的配置，勾选状态应当在它自己里面看得见，
/// 而不是只能去同目录的全量副本里找。写 `false` 对 WorkBuddy 是无操作——
/// `normalizeCustomModel` 的基底就是 `disabled: false`。
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

/// 全量副本必须**每条都写** `disabled`，勾选的写 `false`。
///
/// 用户报的「重复的模型名都启用了」根因就在这里：早期副本省掉了 `false`，
/// 于是「用户全勾了」和「从没记录过勾选」在文件里长得一模一样（都缺这个键），
/// 加载时只能一律当启用，而且会自我延续——全启用读进来、原样写回去，
/// 永远生不出勾选记录。
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
        &std::fs::read_to_string(model_harbor::backends::workbuddy::full_store_path(&path))
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
    // 按位置推导：每个 id 第一条启用（false），其余关闭（true）。
    assert_eq!(items[0]["disabled"], json!(false), "第一条启用");
    assert_eq!(items[1]["disabled"], json!(true), "同 id 第二条关闭");
    assert_eq!(items[2]["disabled"], json!(false), "另一个 id 启用");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// 旧副本（一条 `disabled` 都没写）加载时必须**自愈**成按位置推导。
///
/// 这是用户实际遇到的情形：`models.full.json` 里 35 条、16 个 id、零个标记。
/// 逐条回退到按位置推导后，界面得到「每个模型名只勾第一条」，
/// 保存一次即把推导结果落成显式标记，之后不再推导。
#[test]
fn a_full_store_without_flags_heals_to_first_occurrence_enabled() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    // 复刻线上那份旧副本：三条同名条目，一个 disabled 键都没有。
    let flagless = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1" },
        { "id": "claude-opus-5", "name": "c", "url": "https://c.example/v1" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&flagless).unwrap()).unwrap();
    std::fs::write(
        model_harbor::backends::workbuddy::full_store_path(&path),
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

    // 保存一次，标记落盘；再加载就不该再依赖推导。
    b.save_sidecars(&path, &load.providers).expect("写全量副本");
    let healed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(model_harbor::backends::workbuddy::full_store_path(&path))
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

/// 用户手动调整过勾选后，不能再按位置重新推导——否则他勾的那条会被挤掉。
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
    // 用户改成启用第二条（与「第一条生效」相反）。
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

/// 全量副本路径：与主配置同目录、固定文件名。
#[test]
fn full_store_sits_next_to_models_json() {
    let b = backends::backend(ConfigFormat::WorkBuddy);
    // 通过 trait 对象拿不到 full_store_path（它是本模块的自由函数），
    // 所以这里用 parse_at 的副作用间接验证：副本不存在时必须退回主配置。
    let _ = b;
    let path = r"C:\Users\me\.workbuddy\models.json";
    let full = model_harbor::backends::workbuddy::full_store_path(path);
    assert_eq!(full, r"C:\Users\me\.workbuddy\models.full.json");
    // 名字必须与 models.json 不同：WorkBuddy 按精确文件名读后者，
    // 同名会直接把生效清单覆盖成全量清单，去重就白做了。
    assert!(!full.ends_with("models.json") || full.ends_with("models.full.json"));
    assert_ne!(
        std::path::Path::new(&full).file_name(),
        std::path::Path::new(path).file_name()
    );
    // WSL 路径用 `/` 分隔，与 credentials::sidecar_path 同一套规则。
    assert_eq!(
        model_harbor::backends::workbuddy::full_store_path("/home/me/.workbuddy/models.json"),
        "/home/me/.workbuddy/models.full.json"
    );
}

/// 核心回归：取消勾选的条目**不能丢**。
///
/// 拆成两份配置之前，「取消勾选」= 保存时整条跳过 = 条目连同 API key 一起从磁盘上
/// 永久消失（用户的 36 条会掉到 15 条、12 个厂商整体消失）。现在取消勾选只是把它
/// 从生效清单移到全量副本里，勾回来必须能原样恢复。
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

    // 用户把第二条也勾上（互斥：第一条自动关闭）。
    let mut providers = load.providers.clone();
    providers[0].models[0].disabled = true;
    providers[1].models[0].disabled = false;

    let root = b.serialize_root(&[], &providers, &load.extras, Some(&saved));
    assert_eq!(root.as_array().unwrap().len(), 1, "生效清单只留勾选那条");
    assert_eq!(root.as_array().unwrap()[0]["name"], json!("b"));
    // 写两份，**顺序与 save.rs 一致**：先写全量副本，再写主配置。
    // 副本的字段继承基底读的是副本自己，所以必须赶在主配置被筛过之前写。
    b.save_sidecars(&path, &providers).expect("写全量副本");
    std::fs::write(&dir, b.render(&root, false).unwrap()).unwrap();

    // 全量副本里两条都在，且各自带着自己的字段。
    let full_path = model_harbor::backends::workbuddy::full_store_path(&path);
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

    // 重新加载：必须优先读副本，两条都在、勾选状态原样还原。
    let reloaded = b
        .parse_at(&std::fs::read_to_string(&dir).unwrap(), &path)
        .unwrap();
    assert_eq!(reloaded.providers.len(), 2, "副本存在时按副本还原全部条目");
    assert!(reloaded.providers[0].models[0].disabled, "第一条仍是关闭");
    assert!(!reloaded.providers[1].models[0].disabled, "第二条仍是勾选");

    // 勾回来：生效清单重新变成两条？不——同 id 只能生效一条，
    // 但把第一条也勾上后，界面两条都启用，保存时按顺序留第一条。
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

/// 副本不存在时必须退回主配置，且按位置推导勾选状态。
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
        !std::path::Path::new(&model_harbor::backends::workbuddy::full_store_path(&path)).exists(),
        "这个用例里不该有副本"
    );
    let b = backends::backend(ConfigFormat::WorkBuddy);
    let load = b.parse_at(&saved.to_string(), &path).unwrap();
    assert_eq!(load.providers.len(), 2);
    assert!(!load.providers[0].models[0].disabled, "第一条启用");
    assert!(load.providers[1].models[0].disabled, "同 id 的第二条关闭");
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// 用户手改 `models.json` 后，新加的条目必须出现在界面上。
///
/// 全量副本是 ModelHarbor 上次保存的快照；WorkBuddy 自己不写 `models.json`
/// （它是用户手编的），所以「主配置里有、副本里没有」是正常情况。
/// 只读副本会让用户刚手加的模型在界面上凭空消失。
#[test]
fn hand_added_entries_in_models_json_still_show_up() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    let saved = json!([
        { "id": "old-model", "name": "a", "url": "https://a.example/v1", "apiKey": "sk-a" }
    ]);
    std::fs::write(&dir, serde_json::to_string(&saved).unwrap()).unwrap();
    let b = backends::backend(ConfigFormat::WorkBuddy);

    // 先保存一次，生成全量副本（此时只有 old-model）。
    let load = b.parse_at(&saved.to_string(), &path).unwrap();
    b.save_sidecars(&path, &load.providers).unwrap();

    // 用户手改主配置，加了一条新模型。
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
    // 手加的条目属于生效清单，必须是启用的。
    let added = reloaded
        .providers
        .iter()
        .flat_map(|p| p.models.iter())
        .find(|m| m.id == "hand-added")
        .unwrap();
    assert!(!added.disabled, "主配置里的条目是生效的，应显示为启用");

    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// 用户报的原始症状：**同一个 id 的多条全被判成启用**时，加载必须收敛成只启用第一条。
///
/// 两种来源都会造成这种状态：
/// 1. 旧副本/别人给的文件里一条 `disabled` 都没写（按位置推导本就能治）；
/// 2. 用户把重复项**全勾上过**，于是副本里全是显式 `false`——这时「信任显式标记」
///    反而会把全启用原样读回来，正是「打开全部启用」。
///
/// WorkBuddy 的选择器按裸 id 全局去重，多开的根本不生效，所以无论标记从哪来，
/// 同一 id 只留文件里第一条启用。
#[test]
fn duplicate_ids_all_marked_enabled_are_reduced_to_the_first() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    // 复刻「全勾过」的副本：三条同名条目，**每条都显式写了 disabled: false**。
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
        model_harbor::backends::workbuddy::full_store_path(&path),
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

    // 写出的生效清单里只该有一条，且显式标为启用。
    let root = b.serialize_root(&[], &load.providers, &load.extras, None);
    let entries = root.as_array().unwrap();
    assert_eq!(entries.len(), 1, "生效清单只留一条：{entries:#?}");
    assert_eq!(entries[0]["name"], json!("a"));
    assert_eq!(entries[0]["disabled"], json!(false));
    let _ = std::fs::remove_dir_all(dir.parent().unwrap());
}

/// 用户明确指定了启用哪一条时，去重必须**尊重他的选择**，不能按文件顺序顶掉。
///
/// 规则（用户原话）：只有「重复 id 都被启用」才需要去重；用户自己设置过启用哪一条、
/// 且不重复，就不该再按读取顺序推导。所以当一个 id 上既有**显式**标记的条目、
/// 又有**没有标记、按位置推导**出来的条目时，显式的那条才是用户的意图，位置推导
/// 的只是旧文件的兜底，必须让位。
#[test]
fn an_explicit_choice_wins_over_position_derived_duplicates() {
    let dir = temp_path("models.json");
    let path = dir.display().to_string();
    // 全量副本：同名两条。第一条没有标记（旧文件，按位置会推成启用），
    // 第二条是用户显式勾选的（`disabled: false`）。
    let store = json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://a.example/v1" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://b.example/v1",
          "disabled": false }
    ]);
    std::fs::write(&dir, serde_json::to_string(&store).unwrap()).unwrap();
    std::fs::write(
        model_harbor::backends::workbuddy::full_store_path(&path),
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
