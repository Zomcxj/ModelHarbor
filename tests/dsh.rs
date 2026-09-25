use model_harbor::backends;
use model_harbor::format::ConfigFormat;
use model_harbor::model::ProviderRow;
use model_harbor::util::parse_yaml_content;
use serde_json::json;

fn temp_path(name: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("model_harbor_dsh_{}_{}", std::process::id(), nonce));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn dsh_parse_reads_sidecar_without_putting_secret_in_settings() {
    let settings = temp_path("settings.yaml");
    let credentials = model_harbor::credentials::sidecar_path(settings.to_str().unwrap());
    std::fs::write(
        &settings,
        r#"
ui-theme:
  name: dark
llm-pi-ai:
  providers:
    demo:
      apiKeyEnv: DSH_TEST_KEY
      api: openai-completions
      baseURL: https://example.invalid/v1
      timeoutMs: 180000
      retryPolicy:
        mode: normal
        maxRetries: 3
      customProviderField: keep-me
      models:
        - id: demo-model
          name: Demo
          contextWindow: 1234
          maxTokens: 321
          input: [text]
          reasoningEfforts:
            medium: medium
agent-default-model:
  provider: demo
  model: demo-model
"#,
    )
    .unwrap();
    std::fs::write(
        &credentials,
        "version: 1\nrefs:\n  DSH_TEST_KEY: test-only-placeholder\nrecords:\n  keep: true\n",
    )
    .unwrap();

    let load = backends::load_backend(ConfigFormat::DeepSeekHarness, settings.to_str().unwrap())
        .expect("DSH 配置应可加载");
    assert_eq!(load.providers.len(), 1);
    assert_eq!(load.providers[0].api_key_env, "DSH_TEST_KEY");
    assert_eq!(load.providers[0].api_key_secret, "test-only-placeholder");
    assert_eq!(load.providers[0].dsh_timeout_ms, "180000");
    assert_eq!(load.providers[0].dsh_retry_mode, "normal");
    assert_eq!(load.providers[0].dsh_max_retries, "3");
    assert_eq!(load.providers[0].models[0].variants, "medium");
    assert_eq!(load.root["ui-theme"]["name"], "dark");
    assert_eq!(load.providers[0].raw["customProviderField"], "keep-me");

    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let output = backend
        .render(
            &backend.serialize_root(&[], &load.providers, &load.extras, None),
            false,
        )
        .unwrap();
    assert!(!output.contains("test-only-placeholder"));
    assert!(output.contains("apiKeyEnv"));

    std::fs::remove_file(&settings).ok();
    std::fs::remove_file(&credentials).ok();
}

#[test]
fn dsh_save_updates_one_ref_and_preserves_other_credentials() {
    let settings = temp_path("save-settings.yaml");
    let credentials = model_harbor::credentials::sidecar_path(settings.to_str().unwrap());
    std::fs::write(&settings, "llm-pi-ai:\n  providers: {}\n").unwrap();
    std::fs::write(
        &credentials,
        "version: 1\nrefs:\n  OTHER_KEY: untouched-placeholder\n  DSH_TEST_KEY: old-placeholder\nrecords:\n  keep: true\nunknown: value\n",
    )
    .unwrap();

    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.api_key_env = "DSH_TEST_KEY".into();
    provider.api_key_secret = "new-placeholder".into();
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    backend
        .save_sidecars(settings.to_str().unwrap(), &[provider])
        .unwrap();

    let saved = std::fs::read_to_string(&credentials).unwrap();
    let root = parse_yaml_content(&saved).unwrap();
    assert_eq!(root["refs"]["OTHER_KEY"], "untouched-placeholder");
    assert_eq!(root["refs"]["DSH_TEST_KEY"], "new-placeholder");
    assert_eq!(root["records"]["keep"], true);
    assert_eq!(root["unknown"], "value");
    assert!(!std::fs::read_to_string(&settings)
        .unwrap()
        .contains("new-placeholder"));

    std::fs::remove_file(&settings).ok();
    std::fs::remove_file(&credentials).ok();
}

#[test]
fn dsh_cross_format_save_uses_dsh_schema_without_foreign_keys() {
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let mut provider = ProviderRow::new();
    provider.key = "gateway".into();
    provider.npm = "@ai-sdk/anthropic".into();
    provider.pi_api = "anthropic-messages".into();
    provider.base_url = "https://gateway.example/v1".into();
    provider.dsh_timeout_ms = "180000".into();
    provider.dsh_retry_mode = "normal".into();
    provider.dsh_max_retries = "3".into();
    provider.api_key = "sk-must-not-enter-dsh-settings".into();
    let mut model = model_harbor::model::ModelRow::new();
    model.id = "m1".into();
    model.context = "128000".into();
    model.output = "8192".into();
    model.modalities_input = "text, image".into();
    model.variants = "high".into();
    provider.models.push(model);

    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let saved = &root["llm-pi-ai"]["providers"]["gateway"];
    assert_eq!(saved["api"], "anthropic-messages");
    assert_eq!(saved["baseURL"], "https://gateway.example");
    assert_eq!(saved["timeoutMs"], 180000);
    assert_eq!(saved["retryPolicy"]["mode"], "normal");
    assert_eq!(saved["retryPolicy"]["maxRetries"], 3);
    assert!(saved.get("apiKey").is_none());
    assert!(saved.get("baseUrl").is_none());
    assert!(saved["models"][0].get("limit").is_none());
    assert!(saved["models"][0].get("modalities").is_none());
    assert!(saved["models"][0].get("variants").is_none());
    assert_eq!(
        saved["models"][0]["reasoningEfforts"],
        json!({"high": "high"})
    );
}

#[test]
fn dsh_renderer_matches_native_scalar_and_flow_style() {
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let load = backend
        .parse(
            r#"ui-theme:
  preference: system
llm-pi-ai:
  providers:
    demo:
      apiKeyEnv: DSH_TEST_KEY
      api: openai-completions
      baseURL: https://example.invalid/v1
      models:
        - id: demo-model
          name: Demo
          contextWindow: 1234
          maxTokens: 321
          input: [text]
          reasoningEfforts: {medium: medium}
"#,
        )
        .unwrap();
    let root = backend.serialize_root(&[], &load.providers, &load.extras, None);
    let output = backend.render(&root, false).unwrap();
    assert!(output.contains("apiKeyEnv: \"DSH_TEST_KEY\""));
    assert!(output.contains("api: \"openai-completions\""));
    assert!(output.contains("baseURL: \"https://example.invalid/v1\""));
    assert!(output.contains("- id: \"demo-model\""));
    assert!(output.contains("  name: \"Demo\""));
    assert!(output.contains("input: [ \"text\" ]"));
    assert!(output.contains("reasoningEfforts: { \"medium\": \"medium\" }"));
    assert!(!output.contains("ui-theme: {"));
}

#[test]
fn dsh_cross_format_save_writes_default_timeout() {
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    // opencode 来源（缺省 npm，无 timeout 字段）保存到 DSH 时，
    // timeoutMs 默认值也要写入目标文件。
    let raw = json!({
        "options": {"baseURL": "https://x/v1", "apiKey": "sk-test"},
        "models": {}
    });
    let provider = ProviderRow::from("openai_apizh", &raw);
    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let saved = &root["llm-pi-ai"]["providers"]["openai_apizh"];
    assert_eq!(saved["timeoutMs"], 180000);
}

#[test]
fn dsh_native_model_without_optional_fields_stays_without_them() {
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.source_format = Some(ConfigFormat::DeepSeekHarness);
    // 走真实加载路径构造模型（raw 无 contextWindow/maxTokens/reasoningEfforts 等可选字段）。
    // 不用 ModelRow::new()：那是 UI 新增模型的缺省态（自带默认值）。
    let raw = json!({"id": "m1", "custom": {"keep": true}});
    let mut model = model_harbor::convert::model_from_pi(&raw);
    model.source_format = Some(ConfigFormat::DeepSeekHarness);
    provider.models.push(model);
    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let saved = &root["llm-pi-ai"]["providers"]["demo"]["models"][0];
    assert_eq!(saved["id"], "m1");
    assert!(saved.get("name").is_none());
    assert!(saved.get("input").is_none());
    assert!(saved.get("contextWindow").is_none());
    assert!(saved.get("maxTokens").is_none());
    assert!(saved.get("reasoningEfforts").is_none());
    assert_eq!(saved["custom"]["keep"], true);
}

#[test]
fn dsh_agent_default_model_preserved_as_unknown_field() {
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let extras = json!({
        "ui-theme": {"name": "dark"},
        "agent-default-model": {"provider": "old", "model": "old-model"},
        "llm-pi-ai": {"providers": {}}
    });
    let root = backend.serialize_root(&[], &[], &extras, None);
    assert_eq!(root["agent-default-model"]["provider"], "old");
    assert_eq!(root["agent-default-model"]["model"], "old-model");
    assert_eq!(root["ui-theme"]["name"], "dark");
}

#[test]
fn dsh_missing_timeout_defaults_to_180000_without_writeback() {
    let settings = temp_path("no_timeout.yaml");
    std::fs::write(
        &settings,
        "ui-theme:\n  name: dark\nllm-pi-ai:\n  providers:\n    demo:\n      api: openai-completions\n",
    )
    .unwrap();
    let load = backends::load_backend(ConfigFormat::DeepSeekHarness, settings.to_str().unwrap())
        .expect("DSH 配置应可加载");
    // 配置没有 timeoutMs 时默认显示 180000ms（DSH 页与 opencode 页一致）
    assert_eq!(load.providers[0].dsh_timeout_ms, "180000");
    assert_eq!(load.providers[0].timeout, "180000");
    // 未修改时保存不应写回 timeoutMs
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let root = backend.serialize_root(&[], &load.providers, &load.root, None);
    let demo = &root["llm-pi-ai"]["providers"]["demo"];
    assert!(demo.get("timeoutMs").is_none());
    assert!(demo.get("retryPolicy").is_none());
}

#[test]
fn dsh_max_retries_written_even_when_mode_empty() {
    // 只填 maxRetries、mode 留空时：mode 按 DSH 默认 normal 写出，
    // 不能因为 mode 为空把整个 retryPolicy（含 maxRetries）删掉。
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.source_format = Some(ConfigFormat::DeepSeekHarness);
    provider.dsh_retry_mode = String::new();
    provider.dsh_max_retries = "5".into();
    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let demo = &root["llm-pi-ai"]["providers"]["demo"];
    assert_eq!(demo["retryPolicy"]["mode"], "normal");
    assert_eq!(demo["retryPolicy"]["maxRetries"], 5);
}

#[test]
fn dsh_retry_policy_removed_when_mode_and_retries_empty() {
    // mode 与 maxRetries 都为空时仍不写 retryPolicy（跨格式也不凭空添加）。
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.dsh_retry_mode = String::new();
    provider.dsh_max_retries = String::new();
    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let demo = &root["llm-pi-ai"]["providers"]["demo"];
    assert!(demo.get("retryPolicy").is_none());
}

#[test]
fn dsh_writes_retry_policy_before_timeout_ms() {
    // 字段顺序与 DSH 文件惯例一致：models → retryPolicy → timeoutMs（最小 diff）。
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.dsh_retry_mode = "normal".into();
    provider.dsh_max_retries = "3".into();
    provider.dsh_timeout_ms = "180000".into();
    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let demo = &root["llm-pi-ai"]["providers"]["demo"];
    let keys: Vec<&str> = demo
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let rp = keys.iter().position(|k| *k == "retryPolicy").unwrap();
    let tm = keys.iter().position(|k| *k == "timeoutMs").unwrap();
    assert!(
        rp < tm,
        "retryPolicy 应排在 timeoutMs 之前，实际顺序 {keys:?}"
    );
}

#[test]
fn dsh_cross_format_preserves_target_only_provider() {
    // 跨格式保存到 DSH 目标：目标文件独有的 provider 必须保留（非编辑内容不能改）。
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let target = json!({
        "llm-pi-ai": {
            "providers": {
                "target-only": {"api": "openai-completions", "models": []}
            }
        }
    });
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.dsh_retry_mode = "normal".into();
    provider.dsh_timeout_ms = "180000".into();
    let root = backend.serialize_root(&[], &[provider], &json!({}), Some(&target));
    let provs = &root["llm-pi-ai"]["providers"];
    assert_eq!(
        provs["target-only"]["api"], "openai-completions",
        "目标独有 provider 应保留"
    );
    assert!(provs.get("demo").is_some(), "来源 provider 应写入");
}

#[test]
fn dsh_retry_max_retries_round_trip_from_file() {
    // 贴近实际流程：加载文件 → 改 maxRetries → 当前文件保存 → 渲染 YAML 文本
    let settings = temp_path("retry-roundtrip.yaml");
    std::fs::write(
        &settings,
        "ui-theme:\n  name: dark\nllm-pi-ai:\n  providers:\n    demo:\n      apiKeyEnv: DSH_TEST_KEY\n      api: openai-completions\n      baseURL: https://example.invalid/v1\n      timeoutMs: 180000\n      retryPolicy:\n        mode: normal\n      models:\n        - id: m1\n          name: M1\n",
    )
    .unwrap();
    let load = backends::load_backend(ConfigFormat::DeepSeekHarness, settings.to_str().unwrap())
        .expect("DSH 配置应可加载");
    let mut provider = load.providers[0].clone();
    assert_eq!(
        provider.dsh_max_retries, "",
        "文件未写 maxRetries 时 UI 应为空"
    );
    provider.dsh_max_retries = "7".into();

    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let root = backend.serialize_root(&[], &[provider], &load.root, None);
    let text = backend.render(&root, false).unwrap();
    assert!(
        text.contains("maxRetries: 7"),
        "保存后应写入 maxRetries，实际输出：\n{text}"
    );
    std::fs::remove_file(&settings).ok();
}

#[test]
fn dsh_max_retries_accepts_padded_and_float_input() {
    // UI 允许带空白/小数的数字输入（会 trim），保存时必须写入而不是静默丢弃。
    let backend = backends::backend(ConfigFormat::DeepSeekHarness);
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.dsh_retry_mode = "normal".into();
    provider.dsh_max_retries = " 3 ".into();
    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let demo = &root["llm-pi-ai"]["providers"]["demo"];
    assert_eq!(demo["retryPolicy"]["maxRetries"], 3, "带空白输入应写入");

    let mut provider = ProviderRow::new();
    provider.key = "demo2".into();
    provider.dsh_retry_mode = "normal".into();
    provider.dsh_max_retries = " 2 ".into();
    provider.dsh_timeout_ms = " 180000 ".into();
    let root = backend.serialize_root(&[], &[provider], &json!({}), None);
    let demo = &root["llm-pi-ai"]["providers"]["demo2"];
    assert_eq!(demo["retryPolicy"]["maxRetries"], 2);
    assert_eq!(demo["timeoutMs"], 180000, "timeoutMs 带空白输入应写入");
}

/// sidecar 损坏必须报错并保持原文件不变，而不是静默拿骨架当基底覆写——
/// 那会丢掉全部 refs / records / 未知字段（load_root 旧版的行为）。
#[test]
fn corrupt_sidecar_is_an_error_and_never_gets_overwritten() {
    let settings = temp_path("settings.yaml");
    let sidecar = model_harbor::credentials::sidecar_path(settings.to_str().unwrap());
    let broken = "refs: [unclosed\n\trecords: {";
    std::fs::write(&sidecar, broken).unwrap();

    let err = model_harbor::credentials::load_root(settings.to_str().unwrap())
        .expect_err("坏 YAML 必须报错而不是回落骨架");
    assert!(
        err.contains(sidecar.as_str()),
        "错误信息必须带路径，用户才知道哪个文件坏了: {err}"
    );

    let mut p = ProviderRow::new();
    p.key = "dsh".into();
    p.api_key_secret = "new-secret".into();
    let save = model_harbor::credentials::save(settings.to_str().unwrap(), &[p]);
    assert!(save.is_err(), "sidecar 损坏时保存必须被拒绝: {:?}", save);
    assert_eq!(
        std::fs::read_to_string(&sidecar).unwrap(),
        broken,
        "保存被拒绝后原文件必须原样保留"
    );
    std::fs::remove_file(&sidecar).ok();
}

/// 根不是对象（外来工具 / 手改写成数组）同样报错，不能骨架化后覆写。
#[test]
fn non_object_sidecar_root_is_an_error() {
    let settings = temp_path("settings.yaml");
    let sidecar = model_harbor::credentials::sidecar_path(settings.to_str().unwrap());
    std::fs::write(&sidecar, "- just\n- a list\n").unwrap();
    let err = model_harbor::credentials::load_root(settings.to_str().unwrap())
        .expect_err("数组根必须报错");
    assert!(err.contains("对象"), "错误要说明根必须是对象: {err}");
    std::fs::remove_file(&sidecar).ok();
}

/// 文件不存在 = 新建场景：骨架照常返回，保存正常建立 sidecar。
#[test]
fn missing_sidecar_still_loads_as_skeleton() {
    let settings = temp_path("settings.yaml");
    let sidecar = model_harbor::credentials::sidecar_path(settings.to_str().unwrap());
    std::fs::remove_file(&sidecar).ok();
    let root = model_harbor::credentials::load_root(settings.to_str().unwrap())
        .expect("缺文件是新建场景，不该报错");
    assert_eq!(root["version"], 1);
    assert!(root["refs"].is_object());
}
