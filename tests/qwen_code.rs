//! QwenCode 后端回归：`~/.qwen/settings.json` 的判别 / 解析 / 序列化 / 两份配置。
//!
//! 结构上最要紧的三点（细节见 `src/backends/qwen_code.rs` 的模块说明）：
//! 1. `modelProviders` 的键是 provider id、值是**数组**，一条元素 = 一条完整路由
//!    （自带 `baseUrl` / `envKey`），所以界面是「一条 = 一张卡片」；
//! 2. 密钥在**顶层 `env[envKey]`**，不在条目里；
//! 3. 没有 `disabled` 字段，停用 = 整条不写 → 拆成生效清单 + 全量副本两份文件。

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

// ---------------------------------------------------------------- 启用 / 停用

#[test]
fn disabled_entries_leave_the_effective_file() {
    let first = load(&settings_json());
    let mut providers = first.providers.clone();
    providers[0].models[0].disabled = true;
    let root = qwen_backend().serialize_root(&[], &providers, &first.extras, None);
    let ids: Vec<&str> = root["modelProviders"]["openai"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    assert!(!ids.contains(&"gpt-4o"), "停用条目不得进生效清单");
    assert!(ids.contains(&"openai/gpt-4o"), "其余条目照常");
    // schema 里没有 disabled 这个键，生效清单里不能出现它
    assert!(root["modelProviders"]["openai"][0]
        .get("disabled")
        .is_none());
}

#[test]
fn full_store_records_every_flag_including_false() {
    let dir = temp_dir("full_store");
    let path = dir.join("settings.json");
    std::fs::write(&path, settings_json()).unwrap();

    let first = load(&settings_json());
    let mut providers = first.providers.clone();
    providers[0].models[0].disabled = true;
    qwen_backend()
        .save_sidecars(&path.to_string_lossy(), &providers)
        .expect("写全量副本");

    let sidecar = dir.join("modelProviders.full.json");
    let text = std::fs::read_to_string(&sidecar).expect("副本应存在");
    let full: Value = serde_json::from_str(&text).unwrap();
    let entries = full["modelProviders"]["openai"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "副本里两条都在");
    // 每条**都**写 disabled（含 false）：省掉 false 会让「全勾上」与「从没记录过勾选」
    // 变成同一状态，而且会自我延续。
    for entry in entries {
        assert!(
            entry.get("disabled").and_then(Value::as_bool).is_some(),
            "副本里每条都必须写 disabled：{entry}"
        );
    }
    let disabled: Vec<bool> = entries
        .iter()
        .map(|e| e["disabled"].as_bool().unwrap())
        .collect();
    assert_eq!(disabled, vec![true, false]);
    // 副本带协议映射，否则一个 pid 只剩停用条目时协议就无从得知了
    assert_eq!(full["providerProtocol"]["idealab"], "openai");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn loading_prefers_the_full_store_so_disabled_entries_stay_visible() {
    let dir = temp_dir("prefer_full");
    let path = dir.join("settings.json");
    std::fs::write(&path, settings_json()).unwrap();
    let config_path = path.to_string_lossy().to_string();

    let first = load(&settings_json());
    let mut providers = first.providers.clone();
    providers[0].models[0].disabled = true;
    // 生效清单只留启用的
    let effective = qwen_backend().serialize_root(&[], &providers, &first.extras, None);
    std::fs::write(&path, serde_json::to_string(&effective).unwrap()).unwrap();
    qwen_backend()
        .save_sidecars(&config_path, &providers)
        .unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let reloaded = qwen_backend().parse_at(&content, &config_path).unwrap();
    let gpt = reloaded
        .providers
        .iter()
        .find(|p| p.key == "gpt-4o")
        .expect("停用条目必须仍在界面上，否则用户勾不回来");
    assert!(gpt.models[0].disabled, "勾选状态要还原");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn entries_added_by_hand_to_settings_survive_the_full_store() {
    // Qwen Code 自己也会写 settings.json（/auth、/model）：副本里没有的条目要并进来，
    // 否则用户在 Qwen Code 里新配的模型在界面上看不见。
    let dir = temp_dir("hand_added");
    let path = dir.join("settings.json");
    std::fs::write(&path, settings_json()).unwrap();
    let config_path = path.to_string_lossy().to_string();

    let first = load(&settings_json());
    qwen_backend()
        .save_sidecars(&config_path, &first.providers)
        .unwrap();

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
fn full_store_aborts_when_the_config_is_unreadable() {
    // 副本与主配置都读不出时必须**报错**（由调用方取消保存），不能静默用空基底：
    // 空基底会让停用条目的未知字段（`capabilities.agent`、`generationConfig.*`）
    // 在这一次保存里被抹掉。宁可不让存，也不能蒙眼覆写。
    let dir = temp_dir("sidecar_corrupt");
    let path = dir.join("settings.json");
    std::fs::write(&path, r#"{"modelProviders": {"openai": ["#).unwrap();

    let err = qwen_backend()
        .save_sidecars(&path.to_string_lossy(), &[])
        .expect_err("损坏的配置必须让保存失败");
    assert!(err.contains("无法读取"), "应给出可读的错误：{err}");
    // 副本不该被写出来
    assert!(!dir.join("modelProviders.full.json").exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn shrinks_on_save_detects_disabling() {
    use model_harbor::backends::qwen_code::shrinks_on_save;
    let before: Value = serde_json::from_str(&settings_json()).unwrap();
    let after = json!({"modelProviders": {"openai": [{"id": "gpt-4o"}]}});
    assert!(shrinks_on_save(&before, &after), "条目变少要触发备份");
    assert!(!shrinks_on_save(&after, &before), "条目变多不用备份");
    assert!(!shrinks_on_save(&before, &before), "没变不用备份");
}

#[test]
fn full_store_path_sits_beside_the_config() {
    use model_harbor::backends::qwen_code::full_store_path;
    assert_eq!(
        full_store_path(r"C:\Users\me\.qwen\settings.json"),
        r"C:\Users\me\.qwen\modelProviders.full.json"
    );
    assert_eq!(
        full_store_path("/home/me/.qwen/settings.json"),
        "/home/me/.qwen/modelProviders.full.json"
    );
    // 名字必须与主配置不同，否则会把生效清单覆盖成副本
    assert!(!full_store_path("settings.json").ends_with("settings.json"));
}
