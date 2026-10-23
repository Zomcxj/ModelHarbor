//! Backend registry architecture regression: the full detect / parse / serialize_root pipeline.

use model_harbor_core::backends;
use model_harbor_core::format::{ConfigFormat, ConfigPaths};
use model_harbor_core::model::{AgentRow, ProviderRow};
use serde_json::{json, Value};

fn oc_agent(key: &str) -> AgentRow {
    let mut a = AgentRow::new();
    a.key = key.to_string();
    a.model = "p/m".into();
    a
}

fn pi_provider(key: &str, model_id: &str) -> ProviderRow {
    use model_harbor_core::model::ModelRow;
    let mut p = ProviderRow::new();
    p.key = key.to_string();
    p.base_url = "https://api.example.com/v1".into();
    p.api_key = "sk-test".into();
    p.npm = "@ai-sdk/openai-compatible".into();
    let mut m = ModelRow::new();
    m.id = model_id.to_string();
    m.name = model_id.to_string();
    m.reasoning = true;
    m.context = "128000".into();
    m.output = "8192".into();
    m.modalities_input = "text".into();
    p.models.push(m);
    p
}

#[test]
fn backend_registry_covers_all_formats() {
    assert_eq!(
        backends::backend(ConfigFormat::Opencode).id(),
        ConfigFormat::Opencode
    );
    assert_eq!(backends::backend(ConfigFormat::Pi).id(), ConfigFormat::Pi);
    // Registry order: entry 0 is the detection fallback
    assert_eq!(backends::BACKENDS[0].id(), ConfigFormat::Opencode);
}

/// Every backend must have a 32×32 icon: the tab bar indexes into them, and one gap means one blank button.
///
/// Icons are raw RGBA bytes embedded in the binary (non-premultiplied); length must be `w * h * 4`.
#[test]
fn every_backend_ships_a_32x32_icon() {
    for backend in backends::BACKENDS {
        let (rgba, w, h) = backend
            .icon_rgba()
            .unwrap_or_else(|| panic!("后端 {:?} 缺少图标", backend.id()));
        assert_eq!((w, h), (32, 32), "后端 {:?} 的图标尺寸不对", backend.id());
        assert_eq!(
            rgba.len(),
            (w * h * 4) as usize,
            "后端 {:?} 的图标字节数不对（应为未预乘 RGBA）",
            backend.id()
        );
        // Confirm the icon isn't fully transparent
        let (pixels, _) = rgba.as_chunks::<4>();
        assert!(
            pixels.iter().any(|px| px[3] > 0),
            "后端 {:?} 的图标全透明",
            backend.id()
        );
    }
}

#[test]
fn detect_format_distinguishes_backends() {
    assert_eq!(
        backends::detect_format("{\"providers\": {}}", ""),
        ConfigFormat::Pi
    );
    assert_eq!(
        backends::detect_format("{\"provider\": {}}", ""),
        ConfigFormat::Opencode
    );
    assert_eq!(backends::detect_format("{}", ""), ConfigFormat::Opencode);
    // pi detection yields to opencode: with both key sets present, opencode wins
    assert_eq!(
        backends::detect_format("{\"providers\": {}, \"provider\": {}}", ""),
        ConfigFormat::Opencode
    );
    // opencode v2: top-level plural providers (overlapping pi), but entries carry package/settings and the
    // top level carries agents/plugins — never claimed by pi at any path.
    assert_eq!(
        backends::detect_format(
            "{\"providers\": {\"p\": {\"package\": \"@opencode/ai/providers/openai-compatible\"}}}",
            ""
        ),
        ConfigFormat::Opencode
    );
    assert_eq!(
        backends::detect_format(
            "{\"agents\": {}, \"providers\": {\"p\": {\"settings\": {}}}}",
            ""
        ),
        ConfigFormat::Opencode
    );
    // pi's own content (entries with baseUrl/apiKey/api) is unaffected.
    assert_eq!(
        backends::detect_format(
            "{\"providers\": {\"p\": {\"baseUrl\": \"https://x\", \"api\": \"openai-completions\"}}}",
            ""
        ),
        ConfigFormat::Pi
    );
    // A desktop v2 sample with an arbitrary name (no /opencode/ path segment) still reads as opencode.
    assert_eq!(
        backends::detect_format(
            "{\"plugins\": [\"a\"], \"providers\": {\"p\": {\"package\": \"x\"}}}",
            "C:/Users/x/Desktop/opencodev2.json"
        ),
        ConfigFormat::Opencode
    );
}

#[test]
fn opencode_parse_and_current_file_save() {
    let content = r#"{
        "mcp": { "server": { "command": "node" } },
        "agent": { "old": { "mode": "subagent", "model": "x/y" } },
        "provider": { "oldp": { "npm": "@ai-sdk/x", "models": {} } }
    }"#;
    let b = backends::backend(ConfigFormat::Opencode);
    let load = b.parse(content).expect("解析失败");
    assert_eq!(load.agents.len(), 1);
    assert_eq!(load.providers.len(), 1);
    assert!(load.root.get("mcp").is_some());
    // opencode extras = the whole root
    assert!(load.extras.get("mcp").is_some());

    // Current-file save: agents/providers replaced wholesale, mcp kept
    let agents = vec![oc_agent("writer")];
    let providers = vec![pi_provider("newp", "m1")];
    let root = b.serialize_root(&agents, &providers, &load.extras, None);
    assert!(
        root["agent"].get("old").is_none(),
        "UI 中已删除的 agent 不得复活"
    );
    assert!(root["agent"]["writer"].is_object());
    assert!(root["provider"].get("oldp").is_none());
    assert!(root["provider"]["newp"].is_object());
    assert!(root["mcp"]["server"].is_object(), "未知顶层字段必须保留");
}

#[test]
fn pi_parse_and_current_file_save() {
    let content = r#"{
        "providers": { "p1": { "baseUrl": "https://x/v1", "api": "openai-completions", "apiKey": "k", "models": [] } },
        "extraTop": { "kept": true }
    }"#;
    let b = backends::backend(ConfigFormat::Pi);
    let load = b.parse(content).expect("解析失败");
    assert_eq!(load.providers.len(), 1);
    assert!(load.agents.is_empty());
    // pi extras = top-level fields beyond providers
    assert!(load.extras.get("extraTop").is_some());
    assert!(load.extras.get("providers").is_none());

    let providers = {
        let mut v = load.providers.clone();
        v.push(pi_provider("p2", "m2"));
        v
    };
    let root = b.serialize_root(&[], &providers, &load.extras, None);
    assert!(root["providers"]["p1"].is_object());
    assert!(root["providers"]["p2"].is_object());
    assert!(root["extraTop"].is_object(), "extras 必须保留");
}

#[test]
fn opencode_cross_target_merge_upserts() {
    let b = backends::backend(ConfigFormat::Opencode);
    let target = json!({
        "agent": { "writer": { "mode": "subagent", "model": "p/m" } },
        "provider": { "old": { "npm": "@ai-sdk/x", "models": {} } },
        "mcp": { "server": { "command": "node" } }
    });
    let providers = vec![pi_provider("newp", "m1")];
    let root = b.serialize_root(&[], &providers, &Value::Null, Some(&target));
    // The target's existing entries stay; UI entries upsert
    assert!(root["agent"]["writer"].is_object());
    assert!(root["provider"]["old"].is_object());
    assert!(root["provider"]["newp"].is_object());
    assert!(root["mcp"]["server"].is_object());
}

#[test]
fn pi_cross_target_uses_target_extras() {
    let b = backends::backend(ConfigFormat::Pi);
    // The target file's own top-level fields are adopted, not the source extras'
    let target_extras = json!({ "targetExtra": 1 });
    let providers = vec![pi_provider("p2", "m2")];
    let root = b.serialize_root(&[], &providers, &Value::Null, Some(&target_extras));
    assert_eq!(root["targetExtra"], json!(1));
    assert!(root["providers"]["p2"].is_object());
}

#[test]
fn opencode_current_save_omits_empty_sections() {
    // Empty agents / providers lists write neither "agent": {} nor "provider": {}
    let b = backends::backend(ConfigFormat::Opencode);
    let load = b.parse(r#"{ "mcp": { "s": {} } }"#).expect("解析失败");
    let root = b.serialize_root(&[], &[], &load.extras, None);
    assert!(
        root.get("agent").is_none(),
        "empty agent map must be omitted"
    );
    assert!(
        root.get("provider").is_none(),
        "empty provider map must be omitted"
    );
    assert!(root["mcp"].is_object(), "未知顶层字段必须保留");
}

#[test]
fn detect_empty_yml_as_oh_my_pi() {
    // Empty-content creation: the .yml extension → oh-my-pi, the rest → opencode
    assert_eq!(
        backends::detect_format("", "models.yml"),
        ConfigFormat::OhMyPi
    );
    assert_eq!(
        backends::detect_format("", "models.yaml"),
        ConfigFormat::OhMyPi
    );
    assert_eq!(
        backends::detect_format("", "models.json"),
        ConfigFormat::Opencode
    );
    assert_eq!(backends::detect_format("", ""), ConfigFormat::Opencode);
}

#[test]
fn detect_for_path_uses_extension_for_new_files() {
    // When the path names a nonexistent file (creation), the format infers from the extension
    let (fmt, _) = ConfigPaths::detect_for_path(r"C:\nonexistent_dir_zz\models.yml");
    assert_eq!(fmt, ConfigFormat::OhMyPi);
    let (fmt, _) = ConfigPaths::detect_for_path(r"C:\nonexistent_dir_zz\opencode.json");
    assert_eq!(fmt, ConfigFormat::Opencode);
}

#[test]
fn load_backend_via_generic_pipeline() {
    // The generic load pipeline: a temp file + the opencode backend
    let mut p = std::env::temp_dir();
    p.push("backends_pipeline_test.json");
    std::fs::write(&p, "{\"provider\": {\"x\": {}}}").unwrap();
    let load =
        backends::load_backend(ConfigFormat::Opencode, p.to_str().unwrap()).expect("通用加载失败");
    assert_eq!(load.providers.len(), 1);
    std::fs::remove_file(&p).ok();
}

#[test]
fn write_config_creates_parent_dirs() {
    let mut p = std::env::temp_dir();
    p.push(format!("backends_write_{}", std::process::id()));
    p.push("nested");
    p.push("cfg.json");
    backends::write_config(p.to_str().unwrap(), "{}").expect("写入失败");
    assert!(p.exists());
    std::fs::remove_file(&p).ok();
    p.parent().map(|d| std::fs::remove_dir(d).ok());
}

/// A CLI's first run may only produce the `.jsonc` variant (`kilo.jsonc` / `mimocode.jsonc`): the
/// default `.json` doesn't exist then, yet the backend must still read as installed, with IO pointed
/// at the file that actually exists.
#[test]
fn a_jsonc_only_install_still_counts_as_installed() {
    let root = std::env::temp_dir().join(format!("mh-jsonc-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let kilo_dir = root.join(".config").join("kilo");
    std::fs::create_dir_all(&kilo_dir).unwrap();
    let jsonc = kilo_dir.join("kilo.jsonc");
    std::fs::write(&jsonc, r#"{"$schema":"https://app.kilo.ai/config.json"}"#).unwrap();

    let b = backends::backend(ConfigFormat::Kilocode);
    // The default name (.json) is missing, but the equivalent .jsonc candidate exists
    let default = kilo_dir.join("kilo.json").to_string_lossy().into_owned();
    assert!(
        b.local_available(&default),
        "只有 kilo.jsonc 时也应算已安装"
    );
    let resolved = backends::resolve_local_path(ConfigFormat::Kilocode, &default);
    assert_eq!(
        resolved,
        jsonc.to_string_lossy(),
        "读写路径应指向真实存在的 .jsonc"
    );

    // With both present, the primary name wins
    std::fs::write(kilo_dir.join("kilo.json"), "{}").unwrap();
    assert_eq!(
        backends::resolve_local_path(ConfigFormat::Kilocode, &default),
        default,
        "主名存在时应优先主名"
    );

    // With neither, fall back to the primary name (creation writes there)
    std::fs::remove_file(&jsonc).unwrap();
    std::fs::remove_file(kilo_dir.join("kilo.json")).unwrap();
    assert_eq!(
        backends::resolve_local_path(ConfigFormat::Kilocode, &default),
        default
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Candidate order: primary name first, `.jsonc` variant after; only the file-name segment is replaced.
///
/// A same-named string appearing in the parent path doesn't trigger replacement (e.g. `.../kilo.json-backup/kilo.json`).
#[test]
fn jsonc_candidate_only_rewrites_the_file_name() {
    for (fmt, path, want_variant) in [
        (
            ConfigFormat::Kilocode,
            r"C:\Users\me\.config\kilo\kilo.json",
            r"C:\Users\me\.config\kilo\kilo.jsonc",
        ),
        (
            ConfigFormat::Mimocode,
            r"C:\Users\me\.config\mimocode\mimocode.json",
            r"C:\Users\me\.config\mimocode\mimocode.jsonc",
        ),
    ] {
        let b = backends::backend(fmt);
        let candidates = b.path_candidates(path);
        assert_eq!(candidates[0], path, "首个候选必须是主名");
        assert_eq!(candidates[1], want_variant, "{fmt:?} 的 .jsonc 变体不对");
    }
    // A same-named substring in the parent path: only the file name is touched
    let b = backends::backend(ConfigFormat::Kilocode);
    let odd = r"C:\kilo.json-backup\kilo.json";
    assert_eq!(
        b.path_candidates(odd)[1],
        r"C:\kilo.json-backup\kilo.jsonc",
        "不得替换父目录里的同名字串"
    );
    // Idempotent: feeding the parse result back never appends `.jsonc.jsonc`
    for (fmt, p) in [
        (
            ConfigFormat::Kilocode,
            r"C:\Users\me\.config\kilo\kilo.jsonc",
        ),
        (
            ConfigFormat::Mimocode,
            "/home/me/.config/mimocode/mimocode.jsonc",
        ),
    ] {
        assert_eq!(
            backends::backend(fmt).path_candidates(p).len(),
            1,
            "已是 .jsonc 时不应再追加变体：{p}"
        );
    }
}

/// Non-opencode-family backends have a single candidate (default implementation).
#[test]
fn other_backends_have_a_single_candidate() {
    for fmt in [
        ConfigFormat::Pi,
        ConfigFormat::OhMyPi,
        ConfigFormat::DeepSeekHarness,
        ConfigFormat::ZCode,
        ConfigFormat::WorkBuddy,
    ] {
        let b = backends::backend(fmt);
        let p = b.default_local_path();
        assert_eq!(
            b.path_candidates(&p),
            vec![p.clone()],
            "{fmt:?} 不应有额外候选"
        );
    }
}

/// Creating / merging into a missing target must still write `$schema`, and place it first.
#[test]
fn serialize_always_writes_the_schema_url() {
    let agents = vec![oc_agent("build")];
    let providers = vec![pi_provider("openai_x", "gpt-5.6-sol")];
    for (fmt, want) in [
        (ConfigFormat::Opencode, "https://opencode.ai/config.json"),
        (ConfigFormat::Kilocode, "https://app.kilo.ai/config.json"),
        (
            ConfigFormat::Mimocode,
            "https://mimo.xiaomi.com/mimocode/config.json",
        ),
    ] {
        let b = backends::backend(fmt);
        // The target doesn't exist (empty object) → added
        let fresh = b.serialize_root(&agents, &providers, &json!({}), Some(&json!({})));
        assert_eq!(
            fresh.get("$schema").and_then(Value::as_str),
            Some(want),
            "{fmt:?} 新建时未写 $schema"
        );
        // Current-file save (extras without $schema) → added all the same
        let current = b.serialize_root(&agents, &providers, &json!({"mcp":{}}), None);
        assert_eq!(
            current.get("$schema").and_then(Value::as_str),
            Some(want),
            "{fmt:?} 当前文件保存时未写 $schema"
        );
        // Order: $schema comes first
        assert_eq!(
            current
                .as_object()
                .and_then(|o| o.keys().next())
                .map(String::as_str),
            Some("$schema"),
            "{fmt:?} 的 $schema 应排在首位"
        );
    }
}

/// An existing `$schema` is always kept, never overwritten.
#[test]
fn an_existing_schema_url_is_preserved() {
    let agents = vec![oc_agent("build")];
    let providers = vec![pi_provider("openai_x", "gpt-5.6-sol")];
    let custom = "https://my-mirror.example/kilo.schema.json";
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let b = backends::backend(fmt);
        let existing = json!({ "$schema": custom, "theme": "x" });
        let merged = b.serialize_root(&agents, &providers, &json!({}), Some(&existing));
        assert_eq!(
            merged.get("$schema").and_then(Value::as_str),
            Some(custom),
            "{fmt:?} 覆盖了用户自定义的 $schema"
        );
        assert_eq!(
            merged.get("theme").and_then(Value::as_str),
            Some("x"),
            "{fmt:?} 丢了目标文件的其他顶层字段"
        );
        // Current-file save (extras carrying its own $schema) likewise doesn't overwrite
        let extras = json!({ "$schema": custom });
        let kept = b.serialize_root(&agents, &providers, &extras, None);
        assert_eq!(
            kept.get("$schema").and_then(Value::as_str),
            Some(custom),
            "{fmt:?} 覆盖了当前文件里的 $schema"
        );
    }
}

/// Build a root of "a provider-configured model", with the model body given by the caller.
fn root_with_model(model: Value) -> Value {
    json!({ "provider": { "p": { "models": { "m1": model } } } })
}

/// Read `provider.p.models.m1` from a serialized result.
fn serialized_model(fmt: ConfigFormat, target: &Value, extras: &Value) -> Value {
    let b = backends::backend(fmt);
    let root = b.serialize_root(&[], &[], extras, Some(target));
    root["provider"]["p"]["models"]["m1"].clone()
}

/// **Core regression**: mimocode requires `modalities`' `input` and `output` as a pair; opencode doesn't.
///
/// When only one side is written, the other is filled as plain text; once complete, all three CLIs
/// accept the file.
#[test]
fn mimocode_gets_the_missing_modality_side_filled_in() {
    let target = root_with_model(json!({ "modalities": { "output": ["text"] } }));
    let got = serialized_model(ConfigFormat::Mimocode, &target, &json!({}));
    assert_eq!(
        got["modalities"]["input"],
        json!(["text"]),
        "mimocode 缺的 input 必须补成 text（源方言的语义就是纯文本）"
    );
    assert_eq!(
        got["modalities"]["output"],
        json!(["text"]),
        "用户明确写出的 output 不能被改动"
    );
    // The reverse: only input written → output completed
    let target = root_with_model(json!({ "modalities": { "input": ["text", "image"] } }));
    let got = serialized_model(ConfigFormat::Mimocode, &target, &json!({}));
    assert_eq!(got["modalities"]["output"], json!(["text"]));
    assert_eq!(
        got["modalities"]["input"],
        json!(["text", "image"]),
        "原有的多模态输入必须原样保留"
    );
}

/// opencode / kilocode don't require pairs: no fields invented.
#[test]
fn the_other_flavors_are_not_touched() {
    for fmt in [ConfigFormat::Opencode, ConfigFormat::Kilocode] {
        let target = root_with_model(json!({ "modalities": { "output": ["text"] } }));
        let got = serialized_model(fmt, &target, &json!({}));
        assert_eq!(
            got["modalities"],
            json!({ "output": ["text"] }),
            "{fmt:?} 不该补 modalities.input（它的 schema 不要求）"
        );
    }
}

/// With both sides present, not a single byte changes.
#[test]
fn complete_modalities_are_left_alone() {
    let model = json!({ "modalities": { "input": ["text"], "output": ["text", "image"] } });
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let got = serialized_model(fmt, &root_with_model(model.clone()), &json!({}));
        assert_eq!(
            got["modalities"], model["modalities"],
            "{fmt:?} 改动了完整模态"
        );
    }
}

/// An empty `modalities` object is removed whole: mimo reports both missing input and output for `{}`,
/// while "no modalities key" is legal (the CLI treats it as plain text).
#[test]
fn an_empty_modalities_object_is_removed_for_mimocode() {
    let target = root_with_model(json!({ "modalities": {} }));
    let got = serialized_model(ConfigFormat::Mimocode, &target, &json!({}));
    assert!(
        got.get("modalities").is_none(),
        "空 modalities 必须整块移除，否则 mimo 会同时报两个缺失: {got}"
    );
}

/// `limit`'s context / output must be paired by all three CLIs; a half `limit` is removed whole,
/// leaving the CLI to use its own model library.
#[test]
fn a_half_filled_limit_is_dropped_for_every_flavor() {
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let target = root_with_model(json!({ "limit": { "context": 128000 } }));
        let got = serialized_model(fmt, &target, &json!({}));
        assert!(
            got.get("limit").is_none(),
            "{fmt:?} 应整块删掉半截 limit（省略 limit 三家都合法），而不是编造一个数字: {got}"
        );
        // A paired limit must be preserved verbatim
        let target = root_with_model(json!({ "limit": { "context": 128000, "output": 8192 } }));
        let got = serialized_model(fmt, &target, &json!({}));
        assert_eq!(
            got["limit"],
            json!({ "context": 128000, "output": 8192 }),
            "{fmt:?} 改动了完整的 limit"
        );
    }
}

/// Legacy entries in the target file that the UI doesn't manage also get completed.
#[test]
fn untouched_models_in_the_target_file_are_completed_too() {
    // The target file already has a model with only output written.
    let target = root_with_model(json!({ "modalities": { "output": ["text"] } }));
    let b = backends::backend(ConfigFormat::Mimocode);
    let root = b.serialize_root(&[], &[], &json!({}), Some(&target));
    assert_eq!(
        root["provider"]["p"]["models"]["m1"]["modalities"]["input"],
        json!(["text"]),
        "合并写入时目标文件里保留下来的模型也必须补齐"
    );
}

/// The opencode family (opencode / kilocode / mimocode) shares identical content shapes; only paths
/// can tell them apart.
///
/// Kilo Code and MiMo Code are both opencode forks with word-identical config schemas; only the
/// config directory name / primary config file name differ.
#[test]
fn opencode_family_members_are_told_apart_by_path() {
    let body = r#"{"provider":{"p":{"options":{"baseURL":"https://x.example/v1"}}}}"#;
    // The same content: the path decides ownership.
    for (path, want) in [
        (
            r"C:\Users\me\.config\opencode\opencode.json",
            ConfigFormat::Opencode,
        ),
        (
            r"C:\Users\me\.config\kilo\kilo.json",
            ConfigFormat::Kilocode,
        ),
        (
            r"C:\Users\me\.config\mimocode\mimocode.json",
            ConfigFormat::Mimocode,
        ),
        ("/home/me/.config/kilo/kilo.jsonc", ConfigFormat::Kilocode),
        (
            "/home/me/.config/mimocode/mimocode.jsonc",
            ConfigFormat::Mimocode,
        ),
    ] {
        assert_eq!(
            backends::detect_format(body, path),
            want,
            "路径 {path} 应判为 {want:?}"
        );
    }
    // Without path clues, fall back to opencode itself (the family's representative and fallback).
    assert_eq!(backends::detect_format(body, ""), ConfigFormat::Opencode);
}

/// Kilo also reads the leftover `~/.config/kilo/opencode.json` in the same directory: the directory
/// name already assigns that file to kilocode, so content sniffing no longer competes.
#[test]
fn a_siblings_directory_name_beats_a_matching_filename() {
    let body = r#"{"provider":{"p":{"options":{"baseURL":"https://x.example/v1"}}}}"#;
    for path in [
        r"C:\Users\me\.config\kilo\opencode.json",
        "/home/me/.config/kilo/opencode.jsonc",
    ] {
        assert_eq!(
            backends::detect_format(body, path),
            ConfigFormat::Kilocode,
            "kilo 目录下的 opencode.json 属于 kilocode：{path}"
        );
    }
    // The reverse holds equally: the directory name always beats the file name.
    assert_eq!(
        backends::detect_format(body, r"C:\Users\me\.config\opencode\kilo.json"),
        ConfigFormat::Opencode,
        "opencode 目录下的 kilo.json 属于 opencode"
    );
}

/// All three serialize identically: the same UI state yields the same provider / agent fields.
///
/// The only allowed difference is the `$schema` address (each has its own official schema); it is
/// removed before comparison, and the remainder must match word for word.
#[test]
fn opencode_family_serializes_identically() {
    let providers = vec![pi_provider("openai_x", "gpt-5.6-sol")];
    let agents = vec![oc_agent("build")];
    let mut roots = Vec::new();
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let b = backends::backend(fmt);
        let extras = json!({ "mcp": { "x": { "type": "local" } } });
        let root = b.serialize_root(&agents, &providers, &extras, None);
        // Unknown top-level fields must survive; provider / agent must be written in.
        assert!(root.get("mcp").is_some(), "{fmt:?} 丢了顶层字段");
        assert!(root.get("provider").is_some(), "{fmt:?} 未写 provider");
        assert!(root.get("agent").is_some(), "{fmt:?} 未写 agent");
        assert!(root.get("$schema").is_some(), "{fmt:?} 未写 $schema");
        // Compare with $schema removed: every other field matches word for word.
        let mut without_schema = root.clone();
        without_schema
            .as_object_mut()
            .unwrap()
            .shift_remove("$schema");
        roots.push(serde_json::to_string(&without_schema).unwrap());
    }
    assert_eq!(roots[0], roots[1], "opencode 与 kilocode 写盘应一致");
    assert_eq!(roots[1], roots[2], "kilocode 与 mimocode 写盘应一致");
}

/// The three members' default paths and icons are independent and never cross.
///
/// Icons are compared by visually-distinct pixel share, threshold 25%.
#[test]
fn opencode_family_members_have_distinct_paths_and_icons() {
    let paths: Vec<String> = [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ]
    .iter()
    .map(|f| ConfigPaths::default_local_path(*f))
    .collect();
    assert!(paths[0].contains("opencode"), "{:?}", paths[0]);
    assert!(paths[1].contains("kilo"), "{:?}", paths[1]);
    assert!(paths[2].contains("mimocode"), "{:?}", paths[2]);
    assert_ne!(paths[0], paths[1]);
    assert_ne!(paths[1], paths[2]);

    let formats = [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ];
    let icons: Vec<&[u8]> = formats
        .iter()
        .map(|f| backends::backend(*f).icon_rgba().expect("有图标").0)
        .collect();
    for icon in &icons {
        assert_eq!(icon.len(), 32 * 32 * 4, "图标必须是 32×32 RGBA");
    }
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        let d = differing_pixel_fraction(icons[a], icons[b]);
        assert!(
            d > 0.25,
            "{:?} 与 {:?} 的图标视觉上几乎相同（差异像素仅 {:.1}%），页签上分不出来",
            formats[a],
            formats[b],
            d * 100.0
        );
    }
}

/// Read semantics for the merge base: missing file = create (empty root); present but unparseable =
/// error, and the save is cancelled.
#[test]
fn load_target_root_reports_corrupt_target_instead_of_empty_root() {
    let dir = std::env::temp_dir().join(format!("mh_load_target_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let broken = dir.join("broken.json");
    std::fs::write(&broken, "{\"provider\": ").unwrap();
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::ZCode,
        ConfigFormat::OhMyPi,
        ConfigFormat::DeepSeekHarness,
    ] {
        let err = backends::backend(fmt)
            .load_target_root(broken.to_str().unwrap())
            .expect_err(&format!("{fmt:?}: 坏文件必须报错"));
        assert!(
            err.contains("broken.json"),
            "{fmt:?}: 错误信息必须带路径: {err}"
        );
    }

    // WorkBuddy's top level is an array; even an empty root is an array.
    let broken_wb = dir.join("broken_models.json");
    std::fs::write(&broken_wb, "[{").unwrap();
    assert!(backends::backend(ConfigFormat::WorkBuddy)
        .load_target_root(broken_wb.to_str().unwrap())
        .is_err());

    let missing = dir.join("missing.json");
    for fmt in [ConfigFormat::Opencode, ConfigFormat::ZCode] {
        let root = backends::backend(fmt)
            .load_target_root(missing.to_str().unwrap())
            .unwrap();
        assert_eq!(root, json!({}), "{fmt:?}: 缺文件回落空对象（新建场景）");
    }
    let root = backends::backend(ConfigFormat::WorkBuddy)
        .load_target_root(dir.join("missing_models.json").to_str().unwrap())
        .unwrap();
    assert_eq!(root, json!([]), "WorkBuddy 缺文件回落空数组");

    // What's read back from an intact file must be the complete root.
    let good = dir.join("good.json");
    std::fs::write(&good, r#"{"theme":"dark","provider":{}}"#).unwrap();
    let root = backends::backend(ConfigFormat::Opencode)
        .load_target_root(good.to_str().unwrap())
        .unwrap();
    assert_eq!(root["theme"], json!("dark"));

    std::fs::remove_dir_all(&dir).ok();
}

/// The share of "visibly different" pixels between two 32×32 RGBA icons.
///
/// A channel differs when its per-pixel delta sums past 30; identical icons return 0.0.
fn differing_pixel_fraction(a: &[u8], b: &[u8]) -> f32 {
    assert_eq!(a.len(), b.len());
    let mut differing = 0usize;
    let mut total = 0usize;
    // `as_chunks` rather than `chunks_exact`; both sides were already asserted equal-length, so no tail sliver.
    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0.iter()) {
        total += 1;
        let delta: i32 = pa[..3]
            .iter()
            .zip(&pb[..3])
            .map(|(x, y)| (*x as i32 - *y as i32).abs())
            .sum();
        if delta > 30 {
            differing += 1;
        }
    }
    differing as f32 / total as f32
}

/// Newer opencode writes plural `agents` / `providers` at the top level (no singular keys):
/// parsing must read the entries, saving must write plural keys back, and path-based detection must
/// not be stolen by pi's content fingerprints.
#[test]
fn opencode_plural_keys_round_trip_and_detect() {
    let content = r#"{
        "agents": { "writer": { "mode": "subagent", "model": "p/m" } },
        "providers": { "newp": { "npm": "@ai-sdk/x", "models": {} } }
    }"#;
    let path = "/home/u/.config/opencode/opencode.json";
    // Path detection: opencode-family paths must not be stolen by pi's content fingerprints (providers without provider).
    assert_eq!(
        backends::detect_format(content, path),
        ConfigFormat::Opencode,
        "新版 opencode 配置不得误判为 pi"
    );
    // Without path context: top-level plural `agents` is opencode v2's content evidence (pi never has the key)
    // and must not be claimed by pi; bare plural `providers` (no v2 entry fingerprints) is pi's shape.
    assert_eq!(
        backends::detect_format(content, ""),
        ConfigFormat::Opencode,
        "带复数 agents 的内容应按 v2 证据归 opencode"
    );
    assert_eq!(
        backends::detect_format(r#"{ "providers": {}, "provider": {} }"#, ""),
        ConfigFormat::Opencode
    );

    let b = backends::backend(ConfigFormat::Opencode);
    let load = b.parse(content).expect("复数键应解析出条目");
    assert_eq!(load.agents.len(), 1, "复数 agents 表应读出条目");
    assert_eq!(load.providers.len(), 1, "复数 providers 表应读出条目");

    // Current-file save: key names follow the source file, writing plural.
    let root = b.serialize_root(&load.agents, &load.providers, &load.extras, None);
    assert!(root["agents"]["writer"].is_object(), "应写回复数 agents 键");
    assert!(
        root["providers"]["newp"].is_object(),
        "应写回复数 providers 键"
    );
    assert!(root.get("agent").is_none(), "不得另写单数键");
    assert!(root.get("provider").is_none(), "不得另写单数键");

    // Cross-target merges follow the target file's key names too.
    let merged = backends::opencode::merge_opencode_root(&load.root, &load.agents, &load.providers);
    assert!(merged["agents"]["writer"].is_object());
    assert!(merged.get("agent").is_none());

    // Old singular files: parsing and saving both stay singular, behavior unchanged.
    let legacy = r#"{ "agent": { "w": { "mode": "subagent" } } }"#;
    let load2 = b.parse(legacy).expect("单数键仍应解析");
    assert_eq!(load2.agents.len(), 1);
    let root2 = b.serialize_root(&load2.agents, &[], &load2.extras, None);
    assert!(root2["agent"]["w"].is_object(), "单数源文件应写单数键");
    assert!(root2.get("agents").is_none(), "不得另写复数键");
}
