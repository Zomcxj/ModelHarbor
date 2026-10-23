//! KimiCode backend regression: discrimination / parsing / serialization of `~/.kimi-code/config.toml`.
//!
//! Structure: models live in the top-level global table `[models."<alias>"]`, joined via the
//! `provider` field to `[providers.<name>]`; the table key is the alias and `model` is the wire id
//! sent upstream — the two may differ. There is no `disabled` field and no dedup. `api_key` /
//! `api_key_env` / `oauth` are mutually exclusive; `managed:*` providers come from OAuth login and
//! are read-only.
//!
//! This file never reads `~/.kimi-code/config.toml`; samples are frozen copies.

use model_harbor_core::backends;
use model_harbor_core::format::ConfigFormat;
use model_harbor_core::model::ProviderRow;
use serde_json::Value;

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
        "model_harbor_kimi_{}_{}_{}_{}",
        std::process::id(),
        tag,
        nonce,
        seq
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn kimi_backend() -> &'static dyn backends::Backend {
    backends::backend(ConfigFormat::KimiCode)
}

/// Realistic sample: a frozen copy of this machine's `~/.kimi-code/config.toml` (keys replaced with placeholders).
///
/// Contains a `managed:` + oauth subtable, alias ≠ model, both provider types, the `[thinking]`
/// top-level table, and an entry without `display_name`.
fn config_toml() -> String {
    r#"default_model = "sensenova/sensenova-6.8-flash-lite"
default_permission_mode = "auto"

[providers."managed:kimi-code"]
base_url = "https://api.kimi.com/coding/v1"
type = "kimi"
api_key = ""

[providers."managed:kimi-code".oauth]
storage = "file"
key = "oauth/kimi-code"

[providers.sensenova]
base_url = "https://token.sensenova.cn/v1"
type = "openai"
api_key = "sk-REPLACED"

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 1048576
capabilities = [ "thinking", "always_thinking", "image_in", "video_in", "tool_use", "dynamically_loaded_tools" ]
display_name = "K2.8 Preview"
support_efforts = [ "low", "high", "max" ]
default_effort = "max"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking", "always_thinking", "image_in", "video_in", "tool_use", "dynamically_loaded_tools" ]
display_name = "K3"
support_efforts = [ "low", "high", "max" ]
default_effort = "high"

[models."sensenova/sensenova-6.8-flash-lite"]
provider = "sensenova"
model = "sensenova-6.8-flash-lite"
max_context_size = 65536
capabilities = [ "tool_use", "thinking", "dynamically_loaded_tools"  ]
adaptive_thinking = true
display_name = "sensenova-6.8-flash-lite"
support_efforts = [ "high", "max" ]
default_effort = "high"

[models."sensenova/deepseek-v4-flash"]
provider = "sensenova"
model = "deepseek-v4-flash"
max_context_size = 262000
capabilities = [ "tool_use", "thinking", "dynamically_loaded_tools"  ]
adaptive_thinking = true
display_name = "deepseek-v4-flash"
support_efforts = [ "high", "max" ]
default_effort = "high"

[thinking]
enabled = true
"#
    .to_string()
}

fn load(content: &str) -> backends::BackendLoad {
    kimi_backend().parse(content).expect("KimiCode 解析失败")
}

fn provider<'a>(load: &'a backends::BackendLoad, key: &str) -> &'a ProviderRow {
    load.providers
        .iter()
        .find(|p| p.key == key)
        .unwrap_or_else(|| panic!("缺少 provider {key}"))
}

/// Parse TOML (test-internal; the backend exposes no parser).
fn toml_value(text: &str) -> Value {
    toml::from_str(text).expect("测试样本必须是合法 TOML")
}

// ---------------------------------------------------------------- Discrimination

#[test]
fn detect_matches_the_install_path() {
    let b = kimi_backend();
    assert!(b.detect("", r"C:\Users\me\.kimi-code\config.toml"));
    assert!(b.detect("", "/home/me/.kimi-code/config.toml"));
    // The right path counts, even for an empty file
    assert!(b.detect("", r"C:\Users\me\.kimi-code\config.toml"));
    // Other files in the same directory don't
    assert!(!b.detect("{}", r"C:\Users\me\.kimi-code\mcp.json"));
    assert!(!b.detect("", r"C:\Users\me\.kimi-code\tui.toml"));
}

#[test]
fn detect_matches_the_content_shape() {
    let b = kimi_backend();
    assert!(b.detect(&config_toml(), ""), "真实形态应命中");
    // `managed:kimi-code` + `type = "kimi"`
    assert!(b.detect("[providers.\"managed:kimi-code\"]\ntype = \"kimi\"\n", ""));
    // `default_permission_mode` is a Kimi top-level key
    assert!(b.detect("default_permission_mode = \"auto\"\n", ""));
    // Both tables present + models carrying max_context_size
    assert!(b.detect(
        "[providers.p]\ntype = \"openai\"\n\n[models.a]\nprovider = \"p\"\nmax_context_size = 1\n",
        ""
    ));
}

#[test]
fn detect_rejects_codex_shape() {
    // Codex's top-level key is `model_providers` (underscored); told apart by path and key name.
    let codex = r#"
model = "gpt-5"
model_provider = "openai"

[model_providers.openai]
name = "OpenAI"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
"#;
    assert!(!kimi_backend().detect(codex, ""));
    // With explicit Codex fingerprints, even a Kimi-like path isn't claimed
    assert!(!kimi_backend().detect(codex, r"C:\Users\me\.codex\config.toml"));
}

#[test]
fn detect_does_not_steal_sibling_formats() {
    // KimiCode's TOML isn't stolen by the pi family
    assert_eq!(
        backends::detect_format(&config_toml(), ""),
        ConfigFormat::KimiCode
    );
    // And it doesn't steal others' either
    assert_eq!(
        backends::detect_format(r#"{"provider": {}}"#, ""),
        ConfigFormat::Opencode
    );
    assert_eq!(
        backends::detect_format(r#"{"providers": {}}"#, ""),
        ConfigFormat::Pi
    );
    // Unrelated TOML isn't recognized
    assert!(!kimi_backend().detect("[package]\nname = \"x\"\n", ""));
    assert!(!kimi_backend().detect("", ""));
}

// ---------------------------------------------------------------- Parsing

#[test]
fn models_join_to_their_provider_by_the_provider_field() {
    let load = load(&config_toml());
    let sensenova = provider(&load, "sensenova");
    let ids: Vec<&str> = sensenova.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["sensenova-6.8-flash-lite", "deepseek-v4-flash"],
        "两条模型都该挂到 sensenova 下（按文件顺序）"
    );
    assert_eq!(sensenova.base_url, "https://token.sensenova.cn/v1");
    assert_eq!(sensenova.api_key, "sk-REPLACED");
    assert_eq!(sensenova.pi_api, "openai", "type 逐字进 pi_api");
}

#[test]
fn managed_providers_stay_out_of_the_ui() {
    // `managed:*` is OAuth-maintained; the UI doesn't show it.
    let load = load(&config_toml());
    assert_eq!(load.providers.len(), 1, "只列出 sensenova");
    assert_eq!(load.providers[0].key, "sensenova");
    assert!(
        !load.providers.iter().any(|p| p.key.starts_with("managed:")),
        "managed 不该出现在界面上"
    );
}

#[test]
fn alias_differs_from_the_wire_model_id() {
    // Inside `[models."kimi-code/k3"]`, `model = "k3"`: the table key is the alias, model is the wire id.
    let load = load(&config_toml());
    let sensenova = provider(&load, "sensenova");
    // sensenova's two entries have identical alias and wire id
    assert_eq!(
        sensenova.models[0].kimi_alias,
        "sensenova/sensenova-6.8-flash-lite"
    );
    assert_eq!(sensenova.models[0].id, "sensenova-6.8-flash-lite");
}

#[test]
fn parse_maps_model_attributes() {
    let load = load(&config_toml());
    let m = &provider(&load, "sensenova").models[0];
    assert_eq!(m.id, "sensenova-6.8-flash-lite", "id 是 wire id");
    assert_eq!(
        m.name, "sensenova-6.8-flash-lite",
        "display_name 与 wire id 同名时照样读进来（本机文件就是如此）"
    );
    assert_eq!(m.context, "65536");
    assert_eq!(m.output, "", "没写 max_output_size 就留空");
    assert!(m.reasoning, "capabilities 含 thinking");
    assert!(m.tool_call, "capabilities 含 tool_use");
    assert_eq!(
        m.modalities_input, "text",
        "只有 tool_use/thinking，没有 image_in"
    );
    assert_eq!(m.variants, "high, max");
}

#[test]
fn parse_falls_back_when_optional_keys_are_absent() {
    // Defaults when optional fields are missing: the name falls back to the wire id; tiers and output cap stay empty.
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1000
"#;
    let load2 = load(content);
    let m = &provider(&load2, "p").models[0];
    assert_eq!(m.name, "m", "没有 display_name 就回落 wire id");
    assert_eq!(m.variants, "", "没写 support_efforts 就留空");
    assert_eq!(m.output, "", "没写 max_output_size 就留空");
}

#[test]
fn parse_reads_display_name_and_efforts() {
    // k3 is managed; the constructed sample verifies the display_name and efforts read paths
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1000
display_name = "My Model"
capabilities = [ "image_in", "video_in" ]
support_efforts = [ "low", "high" ]
"#;
    let load2 = load(content);
    let m = &provider(&load2, "p").models[0];
    assert_eq!(m.name, "My Model");
    assert_eq!(m.context, "1000");
    assert_eq!(m.variants, "low, high");
    assert!(
        m.modalities_input.contains("image"),
        "image_in / video_in 都算图像输入：{}",
        m.modalities_input
    );
}

#[test]
fn parse_does_not_invent_undeclared_defaults() {
    // The entry lists only required fields: everything beyond context — numbers / modalities / tiers — stays empty.
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1
"#;
    let load = load(content);
    let m = &provider(&load, "p").models[0];
    assert_eq!(m.output, "");
    assert_eq!(m.modalities_input, "");
    assert_eq!(m.variants, "");
    assert!(!m.reasoning);
    assert!(!m.tool_call);
}

#[test]
fn orphan_models_do_not_panic_and_are_kept() {
    // `provider` points at a nonexistent key: unplaceable, hidden from the UI, but preserved.
    let content = r#"
[providers.p]
type = "openai"

[models."p/ok"]
provider = "p"
model = "ok"
max_context_size = 1

[models."orphan"]
provider = "does-not-exist"
model = "orphan"
max_context_size = 2
"#;
    let load = load(content);
    assert_eq!(load.providers.len(), 1);
    assert_eq!(provider(&load, "p").models.len(), 1, "孤儿不进界面");
    // Still there after saving
    let root = toml_value(content);
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    assert_eq!(out["models"]["orphan"]["model"], "orphan");
}

#[test]
fn a_model_without_a_provider_key_is_treated_as_orphan() {
    let content = r#"
[providers.p]
type = "openai"

[models."p/ok"]
provider = "p"
model = "ok"
max_context_size = 1

[models."no-provider"]
model = "no-provider"
max_context_size = 2
"#;
    let load = load(content);
    assert_eq!(provider(&load, "p").models.len(), 1);
    let root = toml_value(content);
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    assert_eq!(out["models"]["no-provider"]["model"], "no-provider");
}

// ---------------------------------------------------------------- Serialization

#[test]
fn serialize_round_trips_without_losing_anything() {
    let root = toml_value(&config_toml());
    let load = load(&config_toml());
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    let text = kimi_backend().render(&out, false).unwrap();
    let back = toml_value(&text);
    assert_eq!(out, back, "render → parse 必须语义不变");
    // Both tables still present
    assert!(back["providers"].get("managed:kimi-code").is_some());
    assert!(back["models"].get("kimi-code/k3").is_some());
    assert!(back["models"].get("sensenova/deepseek-v4-flash").is_some());
}

#[test]
fn serialize_keeps_the_top_level_extras() {
    let root = toml_value(&config_toml());
    let out = kimi_backend().serialize_root(&[], &[], &root, None);
    assert_eq!(out["default_model"], "sensenova/sensenova-6.8-flash-lite");
    assert_eq!(out["default_permission_mode"], "auto");
    assert_eq!(out["thinking"]["enabled"], true);
}

#[test]
fn serialize_preserves_managed_entries_verbatim() {
    // The managed entry pairs with credentials in credentials/, preserved as-is.
    let root = toml_value(&config_toml());
    let out = kimi_backend().serialize_root(&[], &[], &root, None);
    let managed = &out["providers"]["managed:kimi-code"];
    assert_eq!(managed["type"], "kimi");
    assert_eq!(managed["base_url"], "https://api.kimi.com/coding/v1");
    assert_eq!(managed["oauth"]["storage"], "file");
    assert_eq!(managed["oauth"]["key"], "oauth/kimi-code");
    // Its models are preserved as-is too
    assert_eq!(out["models"]["kimi-code/k3"]["model"], "k3");
    assert_eq!(
        out["models"]["kimi-code/kimi-for-coding"]["display_name"],
        "K2.8 Preview"
    );
}

#[test]
fn serialize_writes_display_name_even_when_it_equals_the_wire_id() {
    // The official format gives every model a `display_name` (even when equal to `model`).
    let content = "[providers.p]\ntype = \"openai\"\n\n\
         [models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1000\n\
         display_name = \"m\"\n";
    let root = toml_value(content);
    let load = load(content);
    assert_eq!(provider(&load, "p").models[0].name, "m");
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    assert_eq!(
        out["models"]["p/m"]["display_name"], "m",
        "名称与 wire id 相同也要逐字写回去"
    );
}

#[test]
fn serialize_keeps_unmodelled_keys() {
    // Keys the UI doesn't model (reasoning_key / adaptive_thinking / overrides / beta_api) must be inherited.
    let root = toml_value(&config_toml());
    let load = load(&config_toml());
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    let entry = &out["models"]["sensenova/sensenova-6.8-flash-lite"];
    assert_eq!(
        entry["adaptive_thinking"], true,
        "adaptive_thinking 不在界面里，但必须原样保留"
    );
}

#[test]
fn serialize_keeps_extra_model_keys() {
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1000
reasoning_key = "reasoning_content"
beta_api = true

[models."p/m".overrides]
max_output_size = 500
"#;
    let root = toml_value(content);
    let load = load(content);
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    let entry = &out["models"]["p/m"];
    assert_eq!(entry["reasoning_key"], "reasoning_content");
    assert_eq!(entry["beta_api"], true);
    assert_eq!(entry["overrides"]["max_output_size"], 500);
}

#[test]
fn quoted_keys_round_trip() {
    // Keys of `[providers.<name>]` may contain `.` and `:` — unquoted, TOML would read them as nested tables.
    let content = r#"
[providers."my.provider"]
type = "openai"
api_key = "k"

[providers."managed:other"]
type = "kimi"

[models."a.b/c"]
provider = "my.provider"
model = "c"
max_context_size = 1
"#;
    let root = toml_value(content);
    let text = kimi_backend().render(&root, false).unwrap();
    let back = toml_value(&text);
    assert_eq!(root, back, "含点号/冒号的键往返不变");
    assert!(
        text.contains(r#"[providers."my.provider"]"#),
        "必须加引号：{text}"
    );
}

#[test]
fn type_is_written_verbatim_and_never_rewritten() {
    // An existing type is kept verbatim, no mapping.
    for ty in [
        "kimi",
        "openai",
        "anthropic",
        "openai_responses",
        "google-genai",
        "vertexai",
    ] {
        let content = format!(
            "[providers.p]\ntype = \"{ty}\"\napi_key = \"k\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\n"
        );
        let root = toml_value(&content);
        let load = load(&content);
        assert_eq!(load.providers[0].pi_api, ty);
        let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
        assert_eq!(out["providers"]["p"]["type"], ty, "type 必须逐字保留");
    }
}

#[test]
fn cross_format_providers_get_a_kimi_vocabulary_type() {
    // Providers copied in cross-format carry internal protocol names; writing `type` translates per Kimi's vocabulary.
    for (api, expected) in [
        ("openai-completions", "openai"),
        ("openai-responses", "openai_responses"),
        ("anthropic-messages", "anthropic"),
        ("google-generative-ai", "google-genai"),
        ("google-vertex", "google-genai"),
        ("google-gemini-cli", "google-genai"),
    ] {
        let mut p = ProviderRow::new();
        p.key = "openai_apizh".into();
        p.base_url = "https://gw.example.com/v1".into();
        p.pi_api = api.into();
        let out = kimi_backend().serialize_root(&[], &[p], &Value::Null, None);
        assert_eq!(
            out["providers"]["openai_apizh"]["type"], expected,
            "{api} 必须翻译成 Kimi 的词表"
        );
    }
}

#[test]
fn an_invalid_type_in_the_file_is_healed_on_the_next_save() {
    // The file's internal protocol name goes into pi_api verbatim; translated per the vocabulary at save.
    let content = "[providers.openai_apizh]
type = \"openai-completions\"
base_url = \"https://gw.example.com/v1\"
api_key = \"k\"

[models.\"openai_apizh/gpt-6-luna\"]
provider = \"openai_apizh\"
model = \"gpt-6-luna\"
max_context_size = 262000
";
    let root = toml_value(content);
    let load = load(content);
    assert_eq!(load.providers[0].pi_api, "openai-completions");
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    assert_eq!(
        out["providers"]["openai_apizh"]["type"], "openai",
        "文件里的非法 type 要在下次保存时翻译成词表内的值"
    );
    // Aliases still follow Kimi's convention
    assert!(out["models"].get("openai_apizh/gpt-6-luna").is_some());
}

// ---------------------------------------------------------------- Credential XOR

#[test]
fn api_key_and_api_key_env_are_mutually_exclusive_on_save() {
    // Writing both api_key and api_key_env gets rejected by Kimi Code — blocked before the disk write.
    let dir = temp_dir("xor");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();

    let load = load(&config_toml());
    let mut providers = load.providers.clone();
    providers[0].api_key = "sk-inline".into();
    providers[0].api_key_env = "MY_ENV".into();

    let err = kimi_backend()
        .save_sidecars(&path.to_string_lossy(), &providers)
        .expect_err("凭据冲突必须让保存失败");
    assert!(err.contains("mutually exclusive"), "错误要说清原因：{err}");
    assert!(
        !dir.join("models.full.toml").exists(),
        "冲突时不该写出任何文件"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_reserved_managed_name_is_refused() {
    // `managed:` is /login's reserved prefix — blocked before the disk write.
    let dir = temp_dir("managed_reserved");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();

    let load = load(&config_toml());
    let mut providers = load.providers.clone();
    providers[0].key = "managed:fake".into();

    let err = kimi_backend()
        .save_sidecars(&path.to_string_lossy(), &providers)
        .expect_err("保留前缀必须让保存失败");
    assert!(err.contains("managed:"), "错误要说清原因：{err}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_alias_collision_across_providers_is_refused() {
    // The alias IS the TOML table key: provider "a" + model "b/c" and provider "a/b" + model "c"
    // produce the same key, so cross-provider combination collisions are blocked here (same-provider
    // duplicates are checked by the save entry point).
    let dir = temp_dir("alias_clash");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();

    let mut p1 = ProviderRow::new();
    p1.key = "a".into();
    p1.pi_api = "openai".into();
    let mut m1 = model_harbor_core::model::ModelRow::new();
    m1.id = "b/c".into();
    p1.models.push(m1);
    let mut p2 = ProviderRow::new();
    p2.key = "a/b".into();
    p2.pi_api = "openai".into();
    let mut m2 = model_harbor_core::model::ModelRow::new();
    m2.id = "c".into();
    p2.models.push(m2);

    let err = kimi_backend()
        .save_sidecars(&path.to_string_lossy(), &[p1, p2])
        .expect_err("别名撞名必须让保存失败");
    assert!(err.contains("a/b/c"), "错误要给出撞名的键：{err}");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_api_key_does_not_conflict_with_oauth() {
    // The criterion is a "non-empty string"; empty counts as unset.
    let load = load(&config_toml());
    // Construct a provider carrying oauth directly
    let mut p = ProviderRow::new();
    p.key = "oauth-provider".into();
    p.api_key = String::new();
    p.raw = toml_value("[oauth]\nstorage = \"file\"\nkey = \"k\"\n");
    // Empty key + oauth don't conflict
    assert!(
        kimi_backend()
            .save_sidecars(
                &temp_dir("empty_key").join("config.toml").to_string_lossy(),
                &[p]
            )
            .is_ok(),
        "空 api_key 与 oauth 并存必须允许"
    );
    let _ = load;
}

#[test]
fn writing_a_key_clears_the_env_name_and_vice_versa() {
    // Two boxes in the UI: filling one clears the other.
    let content = "[providers.p]\ntype = \"openai\"\napi_key_env = \"OLD\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    providers[0].api_key = "sk-new".into();
    providers[0].api_key_env = String::new();
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    assert_eq!(out["providers"]["p"]["api_key"], "sk-new");
    assert!(out["providers"]["p"].get("api_key_env").is_none());
}

// ---------------------------------------------------------------- capabilities

#[test]
fn capabilities_are_only_added_never_removed() {
    // Existing labels only grow, never shrink.
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\ncapabilities = [ \"dynamically_loaded_tools\", \"max_context_tokens\" ]\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    // The UI checks "tool use" on
    providers[0].models[0].tool_call = true;
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    let caps: Vec<&str> = out["models"]["p/m"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        caps.contains(&"dynamically_loaded_tools"),
        "界面不认得的标签不得删除：{caps:?}"
    );
    assert!(caps.contains(&"max_context_tokens"), "同上");
    assert!(caps.contains(&"tool_use"), "新勾的要加上：{caps:?}");
}

#[test]
fn unchecking_reasoning_can_actually_disable_thinking() {
    // Exception: thinking / always_thinking are two spellings of the "supports reasoning" switch — unchecking removes both.
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\ncapabilities = [ \"thinking\", \"always_thinking\", \"tool_use\" ]\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    providers[0].models[0].reasoning = false;
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    let caps: Vec<&str> = out["models"]["p/m"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(!caps.contains(&"thinking"), "{caps:?}");
    assert!(!caps.contains(&"always_thinking"), "{caps:?}");
    assert!(caps.contains(&"tool_use"), "只动思考相关标签：{caps:?}");
}

// ---------------------------------------------------------------- Required-field validation

#[test]
fn max_context_size_is_required_and_never_zeroed() {
    // The schema requires ≥1: a value of 0 omits the key.
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1000\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    providers[0].models[0].context = "0".into();
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    assert!(
        out["models"]["p/m"].get("max_context_size").is_none(),
        "0 不是合法值，必须省略而不是写 0"
    );
}

#[test]
fn default_effort_must_stay_inside_support_efforts() {
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\nsupport_efforts = [ \"low\", \"high\" ]\ndefault_effort = \"high\"\n";
    let root = toml_value(content);
    let load = load(content);
    // high is no longer in the tier list → default_effort is removed
    let mut providers = load.providers.clone();
    providers[0].models[0].variants = "low, max".into();
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    assert!(out["models"]["p/m"].get("default_effort").is_none());
    // Still in the list → the user's chosen default is kept
    let mut providers2 = load.providers.clone();
    providers2[0].models[0].variants = "low, high, max".into();
    let out2 = kimi_backend().serialize_root(&[], &providers2, &root, None);
    assert_eq!(out2["models"]["p/m"]["default_effort"], "high");
}

// ---------------------------------------------------------------- Enable/disable (none)

#[test]
fn there_is_no_disabled_concept_on_the_kimi_page() {
    // Kimi's model schema (`ModelAliasBaseSchema`) has no disabled/enabled field, and the model table indexes
    // one-to-one by alias — no dedup, no toggles. Written entries carry no `disabled`; every entry goes into
    // `config.toml`.
    assert!(!ConfigFormat::KimiCode.has_model_enable());

    let root = toml_value(&config_toml());
    let load = load(&config_toml());
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    for (alias, entry) in out["models"].as_object().unwrap() {
        assert!(entry.get("disabled").is_none(), "{alias} 不该带 disabled");
    }
    // All 4 present (2 UI-managed + 2 under managed), no filtering.
    assert_eq!(out["models"].as_object().unwrap().len(), 4);
}

#[test]
fn no_sidecar_is_written_any_more() {
    // After saving, no full-copy models.full.toml appears in the directory.
    let dir = temp_dir("no_sidecar");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();

    let load = load(&config_toml());
    kimi_backend()
        .save_sidecars(&path.to_string_lossy(), &load.providers)
        .expect("凭据不冲突时必须成功");
    assert!(!dir.join("models.full.toml").exists(), "不该再写全量副本");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hand_added_entries_are_visible_without_a_sidecar() {
    // The main config is the whole state; hand-added entries are visible.
    let dir = temp_dir("hand_added");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();

    let mut text = config_toml();
    text.push_str(
        "\n[providers.hand]\ntype = \"openai\"\napi_key = \"k\"\n\n[models.\"hand/added\"]\nprovider = \"hand\"\nmodel = \"added\"\nmax_context_size = 1\n",
    );
    std::fs::write(&path, text).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let reloaded = kimi_backend()
        .parse_at(&content, &path.to_string_lossy())
        .unwrap();
    assert!(
        reloaded.providers.iter().any(|p| p.key == "hand"),
        "手加的 provider 要能看见"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn shrinks_on_save_detects_deletions() {
    // Deleting a card still shrinks entries — the `.bak` backup's trigger condition needs it.
    use model_harbor_core::backends::kimi_code::shrinks_on_save;
    let before = toml_value(&config_toml());
    let after = toml_value(
        "[providers.p]\ntype = \"openai\"\n\n[models.\"p/a\"]\nprovider = \"p\"\nmodel = \"a\"\nmax_context_size = 1\n",
    );
    assert!(shrinks_on_save(&before, &after), "条目变少要触发备份");
    assert!(!shrinks_on_save(&after, &before), "条目变多不用备份");
    assert!(!shrinks_on_save(&before, &before), "没变不用备份");
}

// ---------------------------------------------------------------- Cross-format

#[test]
fn cross_format_strip_keeps_managed_and_drops_ui_owned() {
    let mut root = toml_value(&config_toml());
    model_harbor_app::app::strip_cross_format_containers(ConfigFormat::KimiCode, &mut root, false);
    let obj = root.as_object().unwrap();

    // `managed:*` is /login's login state (paired with credentials/), invisible to the UI and unrebuildable.
    let providers = obj["providers"].as_object().expect("providers 表要留下");
    assert_eq!(providers.len(), 1, "只该留下 managed provider");
    let managed = &providers["managed:kimi-code"];
    assert_eq!(
        managed["oauth"]["key"], "oauth/kimi-code",
        "oauth 子表逐字保留"
    );
    assert_eq!(managed["type"], "kimi");
    assert_eq!(managed["api_key"], "");

    // Only UI-managed providers and their models are evicted (clean conversion).
    let models = obj["models"].as_object().expect("models 表要留下");
    let aliases: Vec<&str> = models.keys().map(String::as_str).collect();
    assert_eq!(
        aliases,
        ["kimi-code/kimi-for-coding", "kimi-code/k3"],
        "只该留下 managed 名下的模型，且顺序不变"
    );

    // Top-level settings preserved
    assert_eq!(obj["default_model"], "sensenova/sensenova-6.8-flash-lite");
    assert_eq!(obj["thinking"]["enabled"], true);
}

#[test]
fn cross_format_strip_drops_the_tables_when_nothing_is_managed() {
    // With no read-only content it's still "wholly managed": the table is removed whole, no empty shell left.
    let mut root = toml_value(
        "[providers.sensenova]\ntype = \"openai\"\n\n\
         [models.\"sensenova/x\"]\nprovider = \"sensenova\"\nmodel = \"x\"\nmax_context_size = 1000\n",
    );
    model_harbor_app::app::strip_cross_format_containers(ConfigFormat::KimiCode, &mut root, false);
    let obj = root.as_object().unwrap();
    assert!(obj.get("providers").is_none());
    assert!(obj.get("models").is_none());
}

#[test]
fn cross_format_strip_keeps_orphan_models() {
    // `provider` points at a key absent from the table: content this tool can't interpret — preserved as-is.
    let mut root = toml_value(
        "[providers.sensenova]\ntype = \"openai\"\n\n\
         [models.\"ghost/x\"]\nprovider = \"ghost\"\nmodel = \"x\"\nmax_context_size = 1000\n",
    );
    model_harbor_app::app::strip_cross_format_containers(ConfigFormat::KimiCode, &mut root, false);
    let obj = root.as_object().unwrap();
    assert!(obj.get("providers").is_none(), "界面接管的 provider 仍要剔");
    assert_eq!(obj["models"]["ghost/x"]["provider"], "ghost");
}

#[test]
fn cross_format_strip_leaves_other_formats_alone() {
    // Evicting Kimi's tables doesn't touch others' containers
    let mut root = toml_value("[providers.p]\ntype = \"openai\"\n");
    model_harbor_app::app::strip_cross_format_containers(ConfigFormat::Opencode, &mut root, false);
    assert!(root.as_object().unwrap().get("providers").is_some());
}

#[test]
fn models_can_be_copied_to_another_format() {
    // When copying to other formats, the model id is the wire id (not the alias).
    let load = load(&config_toml());
    let m = &provider(&load, "sensenova").models[0];
    let value = m.to_value();
    assert_eq!(value["name"], "sensenova-6.8-flash-lite");
    assert_eq!(m.id, "sensenova-6.8-flash-lite");
}

// ---------------------------------------------------------------- Type round-trip

#[test]
fn all_toml_types_round_trip() {
    // All five types — string / integer / boolean / array / nested table — must survive.
    let content = r#"
a_string = "hello"
a_int = 42
a_bool = true
an_array = [ "x", "y" ]

[providers.p]
type = "openai"
api_key = "k"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1
"#;
    let root = toml_value(content);
    let text = kimi_backend().render(&root, false).unwrap();
    let back = toml_value(&text);
    assert_eq!(back["a_string"], "hello");
    assert_eq!(back["a_int"], 42);
    assert_eq!(back["a_bool"], true);
    assert_eq!(back["an_array"][0], "x");
    assert_eq!(back["providers"]["p"]["type"], "openai");
    assert_eq!(root, back);
}

#[test]
fn empty_content_parses_to_an_empty_load() {
    let load = kimi_backend().parse("").unwrap();
    assert!(load.providers.is_empty());
    // Empty content + empty provider list → saving produces an empty file, no panic
    let out = kimi_backend().serialize_root(&[], &[], &Value::Object(Default::default()), None);
    assert!(out.as_object().unwrap().is_empty());
}

#[test]
fn invalid_toml_reports_a_readable_error() {
    let err = match kimi_backend().parse("this is not = = toml") {
        Err(e) => e,
        Ok(_) => panic!("非法 TOML 必须解析失败"),
    };
    assert!(!err.is_empty(), "要有可读报错");
}
