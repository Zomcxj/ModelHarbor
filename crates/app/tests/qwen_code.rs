//! QwenCode backend regression: discrimination / parsing / serialization / dual configs of `~/.qwen/settings.json`.
//!
//! `modelProviders` keys are provider ids with array values; one element = one complete route (carrying
//! its own `baseUrl` / `envKey`) = one card in the UI; keys live at the top level in `env[envKey]`; there
//! is no `disabled` field, and dedup needs no toggle arbitration.

use model_harbor_app::app::strip_cross_format_containers;
use model_harbor_core::backends;
use model_harbor_core::format::ConfigFormat;
use model_harbor_core::model::ProviderRow;
use serde_json::{json, Value};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    // Directory names globally unique: Windows clock granularity is coarse, and parallel tests relying on pid+nanoseconds
    // can collide within a tick, sharing a directory and overwriting each other. An atomic counter removes the collisions.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_qwen_{}_{}_{}_{}",
        std::process::id(),
        tag,
        nonce,
        seq
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn qwen_backend() -> &'static dyn backends::Backend {
    backends::backend(ConfigFormat::QwenCode)
}

/// Realistic sample: the built-in pid `openai` mixing three different endpoints/keys + one custom pid.
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

// ---------------------------------------------------------------- Discrimination

#[test]
fn detect_requires_the_array_shape() {
    let b = qwen_backend();
    assert!(b.detect(&settings_json(), ""), "真实形态应命中");
    assert!(!b.detect(r#"{"modelProviders": {"openai": "x"}}"#, ""));
    assert!(!b.detect(r#"{"modelProviders": {"openai": {}}}"#, ""));
    assert!(!b.detect(r#"{"modelProviders": {"openai": [{"name": "无 id"}]}}"#, ""));
    assert!(!b.detect("{}", ""));
    assert!(!b.detect("[]", ""));
}

#[test]
fn detect_matches_the_install_path_even_when_empty() {
    let b = qwen_backend();
    // With settings.json not yet created or empty, the install path still hits detection.
    assert!(b.detect("{}", r"C:\Users\me\.qwen\settings.json"));
    assert!(b.detect("{}", "/home/me/.qwen/settings.json"));
    // Other files in the same directory don't count
    assert!(!b.detect("{}", r"C:\Users\me\.qwen\other.json"));
}

#[test]
fn detect_does_not_steal_sibling_formats() {
    // QwenCode's config must not be stolen by opencode / pi
    assert_eq!(
        backends::detect_format(&settings_json(), ""),
        ConfigFormat::QwenCode
    );
    // Nor steal others': the opencode shape and the pi shape
    assert_eq!(
        backends::detect_format(r#"{"provider": {}}"#, ""),
        ConfigFormat::Opencode
    );
    assert_eq!(
        backends::detect_format(r#"{"providers": {}}"#, ""),
        ConfigFormat::Pi
    );
    // The old wrapper shape (values not arrays) must not read as QwenCode
    assert_ne!(
        backends::detect_format(
            r#"{"modelProviders": {"legacy": {"protocol": "openai", "models": []}}}"#,
            ""
        ),
        ConfigFormat::QwenCode
    );
}

// ---------------------------------------------------------------- Parsing

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
    // A provider copied in cross-format: the key is inlined in `api_key`, with `api_key_env` empty.
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
    // An explicitly filled variable name keeps the UI's value
    let mut p2 = ProviderRow::new();
    p2.key = "sensenova".into();
    p2.pi_api = "openai-completions".into();
    p2.api_key_env = "MY_KEY".into();
    p2.api_key = "sk-x".into();
    let out2 = qwen_backend().serialize_root(&[], &[p2], &Value::Null, None);
    assert_eq!(out2["modelProviders"]["openai"][0]["envKey"], "MY_KEY");
    // Both variable name and key empty: no envKey written, no env entry invented
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
    // Two different-endpoint entries under the `openai` pid = two cards, never merged into one
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

/// The shape of a real local file: a Requesty gateway, three entries sharing one pid and one `envKey`.
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
    // Unrecognized top-level keys stay as-is (`providerMetadata` isn't modeled by this tool)
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
    // Entries without vision / reasoning declared → stay empty; no invented defaults
    assert_eq!(m.modalities_input, "");
    assert!(!m.reasoning);
    assert!(
        m.variants.is_empty(),
        "没声明 efforts 就不该有档位（默认档位会污染别的后端）"
    );
}

#[test]
fn parse_does_not_invent_undeclared_defaults() {
    // The entry lists only id and endpoint: context / output / modalities all stay empty.
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

// ---------------------------------------------------------------- Serialization

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
    // env preserved whole
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
    // What's written must be a bare entry array, without the `{protocol, models[]}` wrapper.
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
    // The UI manages only some keys; the rest (agent abilities, retries, other sampling params) must survive verbatim.
    let first = load(&settings_json());
    let root = qwen_backend().serialize_root(&[], &first.providers, &first.extras, None);
    let gpt = &root["modelProviders"]["openai"][0];
    assert_eq!(gpt["generationConfig"]["maxRetries"], 3, "maxRetries 保留");
    assert_eq!(
        gpt["generationConfig"]["samplingParams"]["temperature"], 0.2,
        "采样参数的其它键保留"
    );
    // The custom pid's reasoning profile is preserved as-is.
    let idealab = &root["modelProviders"]["idealab"][0];
    assert_eq!(
        idealab["capabilities"]["reasoning"]["profile"],
        "openai-effort"
    );
}

#[test]
fn serialize_drops_wire_api_for_non_openai_protocols() {
    // Non-openai protocols write no wireApi.
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
    // Cleared fields delete keys rather than writing empty strings.
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
    // Custom pids missing from the mapping table fall back to the built-in pid.
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
    // The user's own naming is preserved; the mapping refreshes per the current protocol.
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

// ---------------------------------------------------------------- Enable / disable (none)

#[test]
fn there_is_no_disabled_concept_on_the_qwen_page() {
    // Qwen Code's schema has no disabled/enabled field, and `/model` lists duplicate models across vendors
    // as-is (dedup targets only the triple-exact repetition of protocol+id+baseUrl).
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
    // Every entry enters the main config (2 openai + others only when hand-added); no filtering
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
    // Even a mistaken call must not produce a sidecar file.
    assert!(
        !dir.join("modelProviders.full.json").exists(),
        "不该再写全量副本"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn entries_added_by_hand_to_settings_stay_visible() {
    // The main config is the whole state; hand-added entries should be visible.
    let dir = temp_dir("hand_added");
    let path = dir.join("settings.json");
    std::fs::write(&path, settings_json()).unwrap();
    let config_path = path.to_string_lossy().to_string();

    // Hand-add one entry
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
    // A derived / hand-filled variable name collides with a different key: the save fails.
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

    // Same name and value (two entries copied from one station) don't conflict
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
    // `env` is a shared namespace; unreferenced keys aren't pruned.
    let base = json!({
        "env": {"BAILIAN_CODING_PLAN_API_KEY": "sk-plan", "ORPHAN": "sk-orphan"},
        "modelProviders": {"openai": [{"id": "m1", "envKey": "GONE"}]}
    });
    let load = load(&serde_json::to_string(&base).unwrap());
    // Delete this entry in the UI
    let root = qwen_backend().serialize_root(&[], &[], &load.extras, None);
    assert_eq!(root["env"]["BAILIAN_CODING_PLAN_API_KEY"], "sk-plan");
    assert_eq!(root["env"]["ORPHAN"], "sk-orphan", "无人引用的键也不删");
}

#[test]
fn shared_env_key_is_written_once_for_both_entries() {
    // Two entries share one envKey: the last card's value wins; env holds a single key.
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

// ---------------------------------------------------------------- Unrecognizable content

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

// ---------------------------------------------------------------- Cross-format

#[test]
fn cross_format_strip_removes_both_containers() {
    let mut root: Value = serde_json::from_str(&settings_json()).unwrap();
    model_harbor_app::app::strip_cross_format_containers(ConfigFormat::QwenCode, &mut root, true);
    assert!(
        root.get("modelProviders").is_none(),
        "没有只读 pid 时整块删掉"
    );
    assert!(
        root.get("providerProtocol").is_none(),
        "只剔 modelProviders 会留下指向已删 provider 的孤儿协议声明"
    );
    // Other top-level settings preserved
    assert_eq!(root["security"]["auth"]["selectedType"], "openai");
    assert_eq!(root["general"]["vimMode"], true);
}

#[test]
fn cross_format_strip_keeps_qwen_oauth() {
    // `qwen-oauth` is the officially hard-coded OAuth entry — invisible to the UI, preserved whole.
    let mut root: Value = serde_json::from_str(&settings_json()).unwrap();
    root["modelProviders"]["qwen-oauth"] = json!([
        { "id": "qwen3-coder-plus", "name": "Qwen3 Coder Plus",
          "baseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1" }
    ]);
    model_harbor_app::app::strip_cross_format_containers(ConfigFormat::QwenCode, &mut root, true);
    let providers = root["modelProviders"].as_object().expect("该 pid 要留下");
    assert_eq!(providers.len(), 1, "只该留下 qwen-oauth");
    assert_eq!(
        root["modelProviders"]["qwen-oauth"][0]["id"], "qwen3-coder-plus",
        "条目逐字保留"
    );
}

#[test]
fn cross_format_export_does_not_leak_qwen_only_keys() {
    // Saving from the Qwen page to opencode: only opencode's fields should appear.
    let load = load(&settings_json());
    let p = provider(&load, "gpt-4o");
    let model = p.models[0].to_value();
    assert!(
        model.get("generationConfig").is_none(),
        "Qwen 专属容器不该漏进别的后端"
    );
    assert!(model.get("envKey").is_none());
}

// ---------------------------------------------------------------- Save flow

#[test]
fn shrinks_on_save_detects_deletions() {
    // Deleting a card shrinks entries — the `.bak` backup's trigger depends on it.
    use model_harbor_core::backends::qwen_code::shrinks_on_save;
    let before: Value = serde_json::from_str(&settings_json()).unwrap();
    let after = json!({"modelProviders": {"openai": [{"id": "gpt-4o"}]}});
    assert!(shrinks_on_save(&before, &after), "条目变少要触发备份");
    assert!(!shrinks_on_save(&after, &before), "条目变多不用备份");
    assert!(!shrinks_on_save(&before, &before), "没变不用备份");
}

// --------------------------------------- Cross-format save's eviction rules

#[test]
fn cross_format_strip_keeps_what_the_ui_cannot_rebuild() {
    // Cross-page saves evict only entries the UI recognizes (with ids); id-less entries, the old wrapper shape,
    // and read-only pids are preserved whole.
    let mut root: Value = serde_json::from_str(
        r#"{
            "modelProviders": {
                "openai": [ { "id": "gpt-4o" }, { "name": "无 id 的条目" } ],
                "legacy": { "protocol": "openai", "models": [] },
                "qwen-oauth": [ { "id": "qwen3-coder-plus" } ]
            },
            "providerProtocol": { "legacy": "openai", "stale": "anthropic" }
        }"#,
    )
    .unwrap();
    strip_cross_format_containers(ConfigFormat::QwenCode, &mut root, false);

    let pids = root["modelProviders"].as_object().unwrap();
    assert!(
        pids["openai"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e.get("id").is_none()),
        "有 id 的条目归界面管，要剔掉"
    );
    assert_eq!(
        pids["openai"][0]["name"], "无 id 的条目",
        "没有 id 的条目认不出来，要留下"
    );
    assert_eq!(pids["legacy"]["protocol"], "openai", "旧包装形状整块留下");
    assert_eq!(pids["qwen-oauth"][0]["id"], "qwen3-coder-plus");
    assert_eq!(
        root["providerProtocol"]["legacy"], "openai",
        "留下的 pid 的映射也要跟着留"
    );
    assert!(
        root["providerProtocol"].get("stale").is_none(),
        "孤儿映射（pid 已不存在）要剔掉"
    );
}
