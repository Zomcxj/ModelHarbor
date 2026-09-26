//! QwenCode 后端回归：`~/.qwen/settings.json` 的判别 / 解析 / 序列化 / 两份配置。
//!
//! 结构上最要紧的三点（细节见 `src/backends/qwen_code.rs` 的模块说明）：
//! 1. `modelProviders` 的键是 provider id、值是**数组**，一条元素 = 一条完整路由
//!    （自带 `baseUrl` / `envKey`），所以界面是「一条 = 一张卡片」；
//! 2. 密钥在**顶层 `env[envKey]`**，不在条目里；
//! 3. 没有 `disabled` 字段，也没有需要开关裁决的去重——`settings.json` 一份文件
//!    承担全部条目；曾经的启用开关与 `modelProviders.full.json` 副本已移除。

use model_harbor::backends;
use model_harbor::format::ConfigFormat;
use model_harbor::model::ProviderRow;
use serde_json::{json, Value};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_qwen_{}_{}_{}",
        std::process::id(),
        tag,
        nonce
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn qwen_backend() -> &'static dyn backends::Backend {
    backends::backend(ConfigFormat::QwenCode)
}

/// 真实形态样本：内置 pid `openai` 下混着三家不同端点/密钥 + 一个自定义 pid。
fn settings_json() -> String {
    r#"{
      "$version": 4,
      "env": {
        "OPENAI_API_KEY": "sk-openai",
        "OPENROUTER_API_KEY": "sk-openrouter",
        "IDEALAB_API_KEY": "sk-idealab"
      },
      "modelProviders": {
        "openai": [
          { "id": "gpt-4o", "name": "GPT-4o", "envKey": "OPENAI_API_KEY",
            "baseUrl": "https://api.openai.com/v1",
            "generationConfig": { "timeout": 60000, "contextWindowSize": 128000,
                                  "maxRetries": 3,
                                  "samplingParams": { "temperature": 0.2, "max_tokens": 4096 } } },
          { "id": "openai/gpt-4o", "name": "GPT-4o (via OpenRouter)",
            "envKey": "OPENROUTER_API_KEY",
            "baseUrl": "https://openrouter.ai/api/v1",
            "generationConfig": { "timeout": 120000 } }
        ],
        "idealab": [
          { "id": "company-model-v2", "envKey": "IDEALAB_API_KEY",
            "baseUrl": "https://gateway.example.com/v1",
            "capabilities": { "reasoning": { "profile": "openai-effort",
                                             "efforts": ["low", "medium", "high"],
                                             "defaultEffort": "medium" } } }
        ]
      },
      "providerProtocol": { "idealab": "openai" },
      "security": { "auth": { "selectedType": "openai" } },
      "model": { "name": "gpt-4o" },
      "general": { "vimMode": true }
    }"#
    .to_string()
}

fn load(content: &str) -> backends::BackendLoad {
    qwen_backend().parse(content).expect("QwenCode 解析失败")
}

fn provider<'a>(load: &'a backends::BackendLoad, key: &str) -> &'a ProviderRow {
    load.providers
        .iter()
        .find(|p| p.key == key)
        .unwrap_or_else(|| panic!("缺少 provider {key}"))
}

// ---------------------------------------------------------------- 判别

#[test]
fn detect_requires_the_array_shape() {
    let b = qwen_backend();
    assert!(b.detect(&settings_json(), ""), "真实形态应命中");
    // 只验键名不够：同名不同形的东西多得是
    assert!(!b.detect(r#"{"modelProviders": {"openai": "x"}}"#, ""));
    assert!(!b.detect(r#"{"modelProviders": {"openai": {}}}"#, ""));
    assert!(!b.detect(r#"{"modelProviders": {"openai": [{"name": "无 id"}]}}"#, ""));
    assert!(!b.detect("{}", ""));
    assert!(!b.detect("[]", ""));
}

#[test]
fn detect_matches_the_install_path_even_when_empty() {
    let b = qwen_backend();
    // 首次运行后 `~/.qwen/` 存在但 settings.json 还没生成/还是空的：
    // 路径对就该认，否则「装了却认不出来」。
    assert!(b.detect("{}", r"C:\Users\me\.qwen\settings.json"));
    assert!(b.detect("{}", "/home/me/.qwen/settings.json"));
    // 同目录的别的文件不算
    assert!(!b.detect("{}", r"C:\Users\me\.qwen\other.json"));
}

#[test]
fn detect_does_not_steal_sibling_formats() {
    // QwenCode 的配置不能被 opencode / pi 抢走
    assert_eq!(
        backends::detect_format(&settings_json(), ""),
        ConfigFormat::QwenCode
    );
    // 反过来也不能抢别人的：opencode 形与 pi 形
    assert_eq!(
        backends::detect_format(r#"{"provider": {}}"#, ""),
        ConfigFormat::Opencode
    );
    assert_eq!(
        backends::detect_format(r#"{"providers": {}}"#, ""),
        ConfigFormat::Pi
    );
    // 旧包装形状（值不是数组）不该被认成 QwenCode
    assert_ne!(
        backends::detect_format(
            r#"{"modelProviders": {"legacy": {"protocol": "openai", "models": []}}}"#,
            ""
        ),
        ConfigFormat::QwenCode
    );
}

// ---------------------------------------------------------------- 解析

#[test]
fn parse_joins_the_key_from_env() {
    let load = load(&settings_json());
    let p = provider(&load, "gpt-4o");
    assert_eq!(p.api_key, "sk-openai", "密钥来自顶层 env[envKey]");
    assert_eq!(
        p.api_key_env, "OPENAI_API_KEY",
        "变量名存下来，写回时要维护 env"
    );
    assert_eq!(p.base_url, "https://api.openai.com/v1");
    assert_eq!(p.timeout, "60000");
    assert_eq!(p.qwen_pid, "openai", "条目属于哪个 pid 要记下来");
}

#[test]
fn a_key_without_an_env_name_gets_a_derived_env_key() {
    // 跨格式复制来的 provider：密钥内联在 api_key 里，api_key_env 为空。
    // 曾因此两个地方都跳过——条目上不写 envKey、env 里不写值——密钥被静默丢掉，
    // CLI 报 "Missing credentials for modelProviders model '…'"。
    let mut p = ProviderRow::new();
    p.key = "sensenova".into();
    p.base_url = "https://token.sensenova.cn/v1".into();
    p.pi_api = "openai-completions".into();
    p.api_key = "sk-sensenova".into();
    let out = qwen_backend().serialize_root(&[], &[p], &Value::Null, None);
    let entry = &out["modelProviders"]["openai"][0];
    assert_eq!(
        entry["envKey"], "SENSENOVA_API_KEY",
        "变量名按 provider key 推导"
    );
    assert_eq!(
        out["env"]["SENSENOVA_API_KEY"], "sk-sensenova",
        "密钥必须真的写进 env"
    );
    // 显式填了变量名的仍以界面值为准
    let mut p2 = ProviderRow::new();
    p2.key = "sensenova".into();
    p2.pi_api = "openai-completions".into();
    p2.api_key_env = "MY_KEY".into();
    p2.api_key = "sk-x".into();
    let out2 = qwen_backend().serialize_root(&[], &[p2], &Value::Null, None);
    assert_eq!(out2["modelProviders"]["openai"][0]["envKey"], "MY_KEY");
    // 变量名与密钥都空：不写 envKey，也不编 env 条目
    let mut p3 = ProviderRow::new();
    p3.key = "sensenova".into();
    p3.pi_api = "openai-completions".into();
    let out3 = qwen_backend().serialize_root(&[], &[p3], &Value::Null, None);
    assert!(out3["modelProviders"]["openai"][0].get("envKey").is_none());
    assert!(out3.get("env").is_none());
}

#[test]
fn one_entry_is_one_card() {
    let load = load(&settings_json());
    // `openai` 这个 pid 下两条不同端点的条目 = 两张卡片，不能合并成一张
    assert_eq!(load.providers.len(), 3, "三条条目 → 三张卡片");
    let openrouter = provider(&load, "openai/gpt-4o");
    assert_eq!(openrouter.api_key, "sk-openrouter");
    assert_eq!(openrouter.base_url, "https://openrouter.ai/api/v1");
    assert_ne!(
        provider(&load, "gpt-4o").api_key,
        openrouter.api_key,
        "同 pid 不同条目必须各带自己的密钥"
    );
}

/// 本机实际文件的形状（Requesty 网关，三条条目共用一个 pid 与一个 `envKey`）。
///
/// 这不是从文档推的样本，是按用户 `~/.qwen/settings.json` 的结构复刻的：三条条目
/// 全在 `openai` 这一个 pid 下、共用一个 `REQUESTY_API_KEY`、都带
/// `generationConfig.customHeaders`，**都没有 `wireApi`**。Qwen Code 0.24.6 自己的
/// `/model` 实现（`ModelDialog` → `getAllConfiguredModels`）对这份形状解析出的正是
/// 下面这三条；本工具也必须一致，否则「打开没有模型」就真成了本工具的问题。
#[test]
fn the_live_requesty_shape_parses_into_three_cards() {
    let content = r#"{
      "$version": 4,
      "ui": { "autoModeAcknowledged": true },
      "env": { "REQUESTY_API_KEY": "sk-requesty" },
      "modelProviders": {
        "openai": [
          { "id": "openai/gpt-4o-mini", "name": "openai/gpt-4o-mini",
            "baseUrl": "https://router.requesty.ai/v1", "envKey": "REQUESTY_API_KEY",
            "generationConfig": { "customHeaders": { "HTTP-Referer": "https://qwen.ai", "X-Title": "Qwen Code" } } },
          { "id": "openai/gpt-4o", "name": "openai/gpt-4o",
            "baseUrl": "https://router.requesty.ai/v1", "envKey": "REQUESTY_API_KEY",
            "generationConfig": { "customHeaders": { "HTTP-Referer": "https://qwen.ai", "X-Title": "Qwen Code" } } },
          { "id": "deepseek-v4-flash", "name": "deepseek-v4-flash",
            "baseUrl": "https://router.requesty.ai/v1", "envKey": "REQUESTY_API_KEY",
            "generationConfig": { "customHeaders": { "HTTP-Referer": "https://qwen.ai", "X-Title": "Qwen Code" } } }
        ]
      },
      "security": { "auth": { "selectedType": "openai" } },
      "model": { "name": "openai/gpt-4o-mini", "baseUrl": "" },
      "providerMetadata": { "requesty": { "version": "855b9068", "ignoredVersion": "693d9264" } }
    }"#;

    assert!(qwen_backend().detect(content, ""), "本机形状必须命中判别");
    let load = load(content);
    let keys: Vec<&str> = load.providers.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(
        keys,
        ["openai/gpt-4o-mini", "openai/gpt-4o", "deepseek-v4-flash"],
        "三条条目三张卡片，顺序按文件里的先后"
    );
    for p in &load.providers {
        assert_eq!(p.models.len(), 1, "{} 该有且只有一条模型", p.key);
        assert_eq!(p.api_key, "sk-requesty", "密钥从顶层 env 按 envKey join");
        assert_eq!(p.base_url, "https://router.requesty.ai/v1");
        assert!(
            !p.models[0].disabled,
            "没有 disabled 概念的格式一律读成启用"
        );
    }
    // 顶层不认识的键原样留着（`providerMetadata` 本工具不建模）
    let out = qwen_backend().serialize_root(&[], &load.providers, &load.extras, None);
    assert_eq!(
        out["providerMetadata"]["requesty"]["ignoredVersion"],
        "693d9264"
    );
    assert_eq!(out["ui"]["autoModeAcknowledged"], true);
    assert_eq!(out["$version"], 4, "已存在的文件不该被改写版本号");
}

#[test]
fn parse_maps_model_attributes() {
    let load = load(&settings_json());
    let m = &provider(&load, "gpt-4o").models[0];
    assert_eq!(m.id, "gpt-4o");
    assert_eq!(m.name, "GPT-4o");
    assert_eq!(m.context, "128000");
    assert_eq!(m.output, "4096", "max_tokens → output");
    // 条目没声明 vision / reasoning → 保持空，不凭空补默认值
    assert_eq!(m.modalities_input, "");
    assert!(!m.reasoning);
    assert!(
        m.variants.is_empty(),
        "没声明 efforts 就不该有档位（默认档位会污染别的后端）"
    );
}

#[test]
fn parse_does_not_invent_undeclared_defaults() {
    // 条目只写了 id 与端点：上下文/输出/模态一律留空，
    // 否则 ModelRow::new() 的预填值会被当成用户配置写回文件。
    let content = r#"{"modelProviders": {"openai": [
        {"id": "bare", "baseUrl": "https://api.openai.com/v1"}
    ]}}"#;
    let load = load(content);
    let m = &provider(&load, "bare").models[0];
    assert_eq!(m.context, "");
    assert_eq!(m.output, "");
    assert_eq!(m.modalities_input, "");
    assert_eq!(m.variants, "");
}

#[test]
fn parse_reads_reasoning_efforts_and_vision() {
    let load = load(&settings_json());
    let m = &provider(&load, "company-model-v2").models[0];
    assert!(m.reasoning, "capabilities.reasoning 存在即视为支持思考");
    assert_eq!(m.variants, "low, medium, high");
}

#[test]
fn parse_does_not_list_the_readonly_oauth_pid() {
    let content = r#"{"modelProviders": {
        "qwen-oauth": [{"id": "qwen3.5-plus", "name": "Qwen3.5 Plus"}],
        "openai": [{"id": "gpt-4o"}]
    }}"#;
    let load = load(content);
    assert_eq!(load.providers.len(), 1);
    assert_eq!(load.providers[0].key, "gpt-4o");
}

#[test]
fn wire_api_decides_the_openai_protocol() {
    let content = r#"{"modelProviders": {"openai": [
        {"id": "resp", "wireApi": "responses"},
        {"id": "chat", "wireApi": "chat-completions"},
        {"id": "bare"}
    ]}}"#;
    let load = load(content);
    assert_eq!(provider(&load, "resp").effective_api(), "openai-responses");
    assert_eq!(
        provider(&load, "chat").effective_api(),
        "openai-completions"
    );
    assert_eq!(
        provider(&load, "bare").effective_api(),
        "openai-completions"
    );
}

#[test]
fn custom_pid_protocol_comes_from_provider_protocol() {
    let load = load(&settings_json());
    assert_eq!(
        provider(&load, "company-model-v2").effective_api(),
        "openai-completions",
        "自定义 pid 的协议查 providerProtocol"
    );
}

// ---------------------------------------------------------------- 序列化

#[test]
fn serialize_round_trips_without_losing_anything() {
    let first = load(&settings_json());
    let root = qwen_backend().serialize_root(&[], &first.providers, &first.extras, None);
    let second = load(&serde_json::to_string(&root).unwrap());

    assert_eq!(second.providers.len(), first.providers.len(), "条目数不变");
    for p in &first.providers {
        let q = provider(&second, &p.key);
        assert_eq!(q.api_key, p.api_key, "{} 的密钥不丢", p.key);
        assert_eq!(q.api_key_env, p.api_key_env);
        assert_eq!(q.base_url, p.base_url);
        assert_eq!(q.qwen_pid, p.qwen_pid, "{} 的 pid 归属不丢", p.key);
        assert_eq!(q.effective_api(), p.effective_api());
    }
    // env 完整保留
    assert_eq!(root["env"]["OPENROUTER_API_KEY"], "sk-openrouter");
}

#[test]
fn serialize_keeps_the_top_level_extras() {
    let first = load(&settings_json());
    let root = qwen_backend().serialize_root(&[], &first.providers, &first.extras, None);
    assert_eq!(root["security"]["auth"]["selectedType"], "openai");
    assert_eq!(root["model"]["name"], "gpt-4o");
    assert_eq!(root["general"]["vimMode"], true);
    assert_eq!(root["$version"], 4, "已有文件的版本号原样保留");
}

#[test]
fn serialize_never_stamps_version_on_an_existing_file() {
    // v2 文件里补 `$version: 4` 会让 Qwen Code 跳过自己的 v1→v4 迁移，
    // 旧结构的设置被按新结构解读。
    let old = json!({"$version": 2, "general": {"vimMode": true}});
    let root = qwen_backend().serialize_root(&[], &[], &old, None);
    assert_eq!(root["$version"], 2);
    assert_eq!(root["general"]["vimMode"], true);
}

#[test]
fn serialize_stamps_version_only_on_a_new_file() {
    let root = qwen_backend().serialize_root(&[], &[], &Value::Object(Default::default()), None);
    assert_eq!(root["$version"], 4);
}

#[test]
fn model_providers_values_are_arrays_of_bare_entries() {
    // 旧预览版的 `{protocol, models[]}` 包装在 $version:4 文件里会被静默跳过，
    // 所以写出的必须是裸数组。
    let first = load(&settings_json());
    let root = qwen_backend().serialize_root(&[], &first.providers, &first.extras, None);
    let map = root["modelProviders"]
        .as_object()
        .expect("modelProviders 是对象");
    for (pid, value) in map {
        let items = value
            .as_array()
            .unwrap_or_else(|| panic!("{pid} 的值必须是数组"));
        for entry in items {
            assert!(entry.get("id").is_some(), "{pid} 的条目必须有 id");
            assert!(entry.get("models").is_none(), "{pid} 的条目不该有包装层");
        }
    }
}

#[test]
fn serialize_keeps_unmanaged_entry_keys() {
    // 界面只接管一部分键；其余（agent 能力、重试、其它采样参数）必须原样留下。
    let first = load(&settings_json());
    let root = qwen_backend().serialize_root(&[], &first.providers, &first.extras, None);
    let gpt = &root["modelProviders"]["openai"][0];
    assert_eq!(gpt["generationConfig"]["maxRetries"], 3, "maxRetries 保留");
    assert_eq!(
        gpt["generationConfig"]["samplingParams"]["temperature"], 0.2,
        "采样参数的其它键保留"
    );
    // 自定义 pid 的 reasoning profile 是界面表达不了的能力声明，必须留着
    let idealab = &root["modelProviders"]["idealab"][0];
    assert_eq!(
        idealab["capabilities"]["reasoning"]["profile"],
        "openai-effort"
    );
}

#[test]
fn serialize_drops_wire_api_for_non_openai_protocols() {
    // 官方明说：wireApi 写在 Anthropic / Gemini / Vertex / Qwen OAuth 上是配置错误。
    let mut p = ProviderRow::new();
    p.key = "claude".into();
    p.pi_api = "anthropic-messages".into();
    p.base_url = "https://api.anthropic.com".into();
    let root = qwen_backend().serialize_root(&[], std::slice::from_ref(&p), &json!({}), None);
    let entry = &root["modelProviders"]["anthropic"][0];
    assert_eq!(entry["id"], "claude");
    assert!(entry.get("wireApi").is_none());
    assert!(
        root.get("providerProtocol").is_none(),
        "内置 pid 不需要映射"
    );
}

#[test]
fn cleared_fields_are_removed_not_written_empty() {
    // 空 `envKey` 会被当成一个真的空变量名，必须删键而不是写空串。
    let first = load(&settings_json());
    let mut providers = first.providers.clone();
    for p in &mut providers {
        p.api_key_env.clear();
        p.base_url.clear();
        p.description.clear();
    }
    let root = qwen_backend().serialize_root(&[], &providers, &first.extras, None);
    let entry = &root["modelProviders"]["openai"][0];
    assert!(entry.get("envKey").is_none());
    assert!(entry.get("baseUrl").is_none());
}

// ---------------------------------------------------------------- providerProtocol

#[test]
fn custom_pid_keeps_its_mapping() {
    let first = load(&settings_json());
    let root = qwen_backend().serialize_root(&[], &first.providers, &first.extras, None);
    assert_eq!(root["providerProtocol"]["idealab"], "openai");
}

#[test]
fn deleting_the_last_entry_of_a_custom_pid_prunes_the_mapping() {
    let first = load(&settings_json());
    let kept: Vec<ProviderRow> = first
        .providers
        .iter()
        .filter(|p| p.key != "company-model-v2")
        .cloned()
        .collect();
    let root = qwen_backend().serialize_root(&[], &kept, &first.extras, None);
    assert!(root["modelProviders"].get("idealab").is_none());
    assert!(
        root["providerProtocol"].get("idealab").is_none(),
        "孤儿映射要清掉，否则留下指向不存在 provider 的声明"
    );
}

#[test]
fn unknown_custom_pid_falls_back_to_a_builtin_one() {
    // 映射表里没有的自定义 pid，写出去只会让整条被静默跳过 —— 回落到内置 pid。
    let mut p = ProviderRow::new();
    p.key = "m1".into();
    p.pi_api = "openai-completions".into();
    p.qwen_pid = "typo-pid".into();
    let root = qwen_backend().serialize_root(&[], std::slice::from_ref(&p), &json!({}), None);
    assert!(
        root["modelProviders"]["openai"][0].get("id").is_some(),
        "认不出的 pid 应回落到 openai"
    );
    assert!(root["modelProviders"].get("typo-pid").is_none());
}

#[test]
fn known_custom_pid_is_kept_and_refreshed() {
    // 用户自己的命名要保留，映射按当前协议刷新。
    let base = json!({
        "modelProviders": {"idealab": [{"id": "m1"}]},
        "providerProtocol": {"idealab": "openai"}
    });
    let load = load(&serde_json::to_string(&base).unwrap());
    let mut providers = load.providers.clone();
    providers[0].pi_api = "anthropic-messages".into();
    let root = qwen_backend().serialize_root(&[], &providers, &base, None);
    assert_eq!(root["providerProtocol"]["idealab"], "anthropic");
    assert!(root["modelProviders"]["idealab"][0].get("id").is_some());
}

// ---------------------------------------------------------------- 启用 / 停用（没有）

#[test]
fn there_is_no_disabled_concept_on_the_qwen_page() {
    // Qwen Code 的 schema 没有 disabled/enabled 字段，`/model` 对不同厂商的重复模型
    // 照列不误（判重只针对协议+id+baseUrl 完全相同的三重重复）。曾经的启用开关与
    // `modelProviders.full.json` 副本是本工具发明的状态，已按用户指正移除。
    assert!(!ConfigFormat::QwenCode.has_model_enable());

    let root = qwen_backend().serialize_root(
        &[],
        &load(&settings_json()).providers,
        &load(&settings_json()).extras,
        None,
    );
    for entry in root["modelProviders"]["openai"].as_array().unwrap() {
        assert!(
            entry.get("disabled").is_none(),
            "{} 不该带 disabled",
            entry["id"]
        );
    }
    // 全部条目都进主配置（2 条 openai + 手加时才有的别家），没有筛选
    assert_eq!(
        root["modelProviders"]["openai"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn no_sidecar_is_written_any_more() {
    let dir = temp_dir("no_sidecar");
    let path = dir.join("settings.json");
    std::fs::write(&path, settings_json()).unwrap();

    let first = load(&settings_json());
    qwen_backend()
        .save_sidecars(&path.to_string_lossy(), &first.providers)
        .ok();
    // QwenCode 已从 sidecar 名单里摘掉：即便误调也不得产出文件
    assert!(
        !dir.join("modelProviders.full.json").exists(),
        "不该再写全量副本"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn entries_added_by_hand_to_settings_stay_visible() {
    // Qwen Code 自己也会写 settings.json（/auth、/model）：没有副本之后主配置就是
    // 全部状态，手加的条目自然能看见。
    let dir = temp_dir("hand_added");
    let path = dir.join("settings.json");
    std::fs::write(&path, settings_json()).unwrap();
    let config_path = path.to_string_lossy().to_string();

    // 手加一条
    let mut root: Value = serde_json::from_str(&settings_json()).unwrap();
    root["modelProviders"]["openai"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "hand-added", "baseUrl": "https://x.example.com/v1"}));
    std::fs::write(&path, serde_json::to_string(&root).unwrap()).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let reloaded = qwen_backend().parse_at(&content, &config_path).unwrap();
    assert!(
        reloaded.providers.iter().any(|p| p.key == "hand-added"),
        "手加的条目必须出现在界面上"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------- env

#[test]
fn an_env_name_collision_is_refused() {
    // 推导/手填的变量名撞在一起且密钥不同：sync_env 后写覆盖先写，其中一条 provider
    // 会静默拿到别人的密钥。宁可不让存。
    let dir = temp_dir("env_clash");
    let path = dir.join("settings.json");
    std::fs::write(&path, settings_json()).unwrap();

    let mut p1 = ProviderRow::new();
    p1.key = "openai-247kan".into();
    p1.pi_api = "openai-completions".into();
    p1.api_key = "sk-one".into();
    let mut p2 = ProviderRow::new();
    p2.key = "openai_247kan".into();
    p2.pi_api = "openai-completions".into();
    p2.api_key = "sk-two".into();

    let err = qwen_backend()
        .save_sidecars(&path.to_string_lossy(), &[p1, p2.clone()])
        .expect_err("envKey 撞名必须让保存失败");
    assert!(
        err.contains("OPENAI_247KAN_API_KEY"),
        "错误要给出撞名的键：{err}"
    );

    // 同名同值（同一站点复制出的两条）不算冲突
    let mut p3 = ProviderRow::new();
    p3.key = "openai_247kan".into();
    p3.pi_api = "openai-completions".into();
    p3.api_key = "sk-two".into();
    qwen_backend()
        .save_sidecars(&path.to_string_lossy(), &[p2.clone(), p3])
        .expect("同名同密钥应当放行");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn env_is_never_pruned() {
    // `env` 是共享命名空间，而且 Qwen Code 的 /auth 也往里写
    // （例如 BAILIAN_CODING_PLAN_API_KEY）。按「有没有条目引用」修剪会把用户
    // 刚配好的凭据静默删掉，且不可恢复。
    let base = json!({
        "env": {"BAILIAN_CODING_PLAN_API_KEY": "sk-plan", "ORPHAN": "sk-orphan"},
        "modelProviders": {"openai": [{"id": "m1", "envKey": "GONE"}]}
    });
    let load = load(&serde_json::to_string(&base).unwrap());
    // 界面上把这条删掉
    let root = qwen_backend().serialize_root(&[], &[], &load.extras, None);
    assert_eq!(root["env"]["BAILIAN_CODING_PLAN_API_KEY"], "sk-plan");
    assert_eq!(root["env"]["ORPHAN"], "sk-orphan", "无人引用的键也不删");
}

#[test]
fn shared_env_key_is_written_once_for_both_entries() {
    // 两条条目共用同一个 envKey：写入的值必须是最后一张卡片的（用户看到的那份），
    // 且 env 里只有一个键。
    let base = json!({
        "modelProviders": {"openai": [
            {"id": "a", "envKey": "SHARED"},
            {"id": "b", "envKey": "SHARED"}
        ]}
    });
    let mut providers = load(&serde_json::to_string(&base).unwrap()).providers;
    providers[0].api_key = "sk-a".into();
    providers[1].api_key = "sk-b".into();
    let root = qwen_backend().serialize_root(&[], &providers, &base, None);
    assert_eq!(root["env"]["SHARED"], "sk-b");
    assert_eq!(root["env"].as_object().unwrap().len(), 1, "同名键只有一个");
}

// ---------------------------------------------------------------- 认不出来的内容

#[test]
fn readonly_oauth_entries_are_preserved_verbatim() {
    let base = json!({
        "modelProviders": {
            "qwen-oauth": [{"id": "qwen3.5-plus", "name": "Qwen3.5 Plus", "vendorKey": "x"}],
            "openai": [{"id": "gpt-4o"}]
        }
    });
    let load = load(&serde_json::to_string(&base).unwrap());
    let root = qwen_backend().serialize_root(&[], &load.providers, &base, None);
    let kept = root["modelProviders"]["qwen-oauth"].as_array().unwrap();
    assert_eq!(kept.len(), 1, "只读 pid 的条目整块保留");
    assert_eq!(kept[0]["id"], "qwen3.5-plus");
    assert_eq!(kept[0]["vendorKey"], "x", "未知字段一并保留");
}

#[test]
fn id_less_entries_survive_a_save() {
    let base = json!({"modelProviders": {"openai": [{"name": "没有 id 的条目"}]}});
    let root = qwen_backend().serialize_root(&[], &[], &base, None);
    let kept = root["modelProviders"]["openai"].as_array().unwrap();
    assert_eq!(kept.len(), 1, "进不了界面的条目不能被保存动作删掉");
    assert_eq!(kept[0]["name"], "没有 id 的条目");
}

#[test]
fn legacy_wrapped_shape_is_preserved() {
    let base = json!({
        "modelProviders": {"legacy": {"protocol": "openai", "models": []}},
        "providerProtocol": {"legacy": "openai"}
    });
    let root = qwen_backend().serialize_root(&[], &[], &base, None);
    assert_eq!(root["modelProviders"]["legacy"]["protocol"], "openai");
    assert_eq!(root["providerProtocol"]["legacy"], "openai");
}

// ---------------------------------------------------------------- 跨格式

#[test]
fn cross_format_strip_removes_both_containers() {
    let mut root: Value = serde_json::from_str(&settings_json()).unwrap();
    model_harbor::app::strip_cross_format_containers(ConfigFormat::QwenCode, &mut root, true);
    assert!(
        root.get("modelProviders").is_none(),
        "没有只读 pid 时整块删掉"
    );
    assert!(
        root.get("providerProtocol").is_none(),
        "只剔 modelProviders 会留下指向已删 provider 的孤儿协议声明"
    );
    // 其余顶层设置保留
    assert_eq!(root["security"]["auth"]["selectedType"], "openai");
    assert_eq!(root["general"]["vimMode"], true);
}

#[test]
fn cross_format_strip_keeps_qwen_oauth() {
    // `qwen-oauth` 是官方硬编码的 OAuth 条目，界面不显示也无从重建。整块剔掉就是
    // 把用户已经登录好的 Qwen 模型删了。
    let mut root: Value = serde_json::from_str(&settings_json()).unwrap();
    root["modelProviders"]["qwen-oauth"] = json!([
        { "id": "qwen3-coder-plus", "name": "Qwen3 Coder Plus",
          "baseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1" }
    ]);
    model_harbor::app::strip_cross_format_containers(ConfigFormat::QwenCode, &mut root, true);
    let providers = root["modelProviders"].as_object().expect("该 pid 要留下");
    assert_eq!(providers.len(), 1, "只该留下 qwen-oauth");
    assert_eq!(
        root["modelProviders"]["qwen-oauth"][0]["id"], "qwen3-coder-plus",
        "条目逐字保留"
    );
}

#[test]
fn cross_format_export_does_not_leak_qwen_only_keys() {
    // 从 Qwen 页存到 opencode：只该出现 opencode 的字段。
    let load = load(&settings_json());
    let p = provider(&load, "gpt-4o");
    let model = p.models[0].to_value();
    assert!(
        model.get("generationConfig").is_none(),
        "Qwen 专属容器不该漏进别的后端"
    );
    assert!(model.get("envKey").is_none());
}

// ---------------------------------------------------------------- 保存流程

#[test]
fn shrinks_on_save_detects_deletions() {
    // 停用没了，但删卡片仍会让条目变少——`.bak` 备份的触发条件依然需要它。
    use model_harbor::backends::qwen_code::shrinks_on_save;
    let before: Value = serde_json::from_str(&settings_json()).unwrap();
    let after = json!({"modelProviders": {"openai": [{"id": "gpt-4o"}]}});
    assert!(shrinks_on_save(&before, &after), "条目变少要触发备份");
    assert!(!shrinks_on_save(&after, &before), "条目变多不用备份");
    assert!(!shrinks_on_save(&before, &before), "没变不用备份");
}
