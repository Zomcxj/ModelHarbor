use super::*;

/// The read-only pid's entries are preserved whole and stay out of the UI.
#[test]
fn readonly_pid_entries_are_preserved_but_not_listed() {
    let root: Value = serde_json::from_str(
        r#"{
                "modelProviders": {
                    "qwen-oauth": [ { "id": "qwen3.5-plus", "name": "Qwen3.5 Plus" } ],
                    "openai": [ { "id": "gpt-4o", "baseUrl": "https://api.openai.com/v1" } ]
                }
            }"#,
    )
    .unwrap();
    let listed = entries_from(&root);
    assert_eq!(listed.len(), 1, "只读 pid 不进界面");
    assert_eq!(listed[0].0, "openai");

    let placed = place(&[], &root);
    let out = QwenCodeBackend.serialize_root(&[], &[], &root, None);
    let kept = entries_of_pid(&out, READONLY_PID);
    assert_eq!(kept.len(), 1, "只读条目必须原样留在文件里");
    assert_eq!(kept[0]["id"], "qwen3.5-plus");
    let _ = placed;
}

/// Unrecognized entries (no id) aren't deleted by the save action.
#[test]
fn id_less_entries_survive_a_save() {
    let root: Value = serde_json::from_str(
        r#"{ "modelProviders": { "openai": [ { "name": "无 id 的条目" } ] } }"#,
    )
    .unwrap();
    let out = QwenCodeBackend.serialize_root(&[], &[], &root, None);
    let kept = entries_of_pid(&out, "openai");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0]["name"], "无 id 的条目");
}

/// The old wrapper shape (values not arrays) is preserved whole.
#[test]
fn legacy_wrapped_shape_is_preserved() {
    let root: Value = serde_json::from_str(
        r#"{ "modelProviders": { "legacy": { "protocol": "openai", "models": [] } } }"#,
    )
    .unwrap();
    let out = QwenCodeBackend.serialize_root(&[], &[], &root, None);
    assert_eq!(out["modelProviders"]["legacy"]["protocol"], "openai");
}

/// A brand-new file writes `$version: 4`; existing files never touch it.
#[test]
fn version_is_only_stamped_on_a_new_file() {
    let fresh = QwenCodeBackend.serialize_root(&[], &[], &Value::Object(Map::new()), None);
    assert_eq!(fresh["$version"], 4);

    let old: Value = serde_json::from_str(r#"{ "$version": 2, "general": {} }"#).unwrap();
    let out = QwenCodeBackend.serialize_root(&[], &[], &old, None);
    assert_eq!(out["$version"], 2, "已有文件的版本号不得被改写");
    assert!(out.get("general").is_some(), "其余顶层设置保留");
}

/// `env` only adds and updates, never deletes: orphan keys and UI-cleared keys both stay.
#[test]
fn env_is_never_pruned() {
    let base: Value = serde_json::from_str(
        r#"{ "env": { "BAILIAN_CODING_PLAN_API_KEY": "sk-plan", "OLD_KEY": "sk-old" } }"#,
    )
    .unwrap();
    let out = QwenCodeBackend.serialize_root(&[], &[], &base, None);
    assert_eq!(out["env"]["BAILIAN_CODING_PLAN_API_KEY"], "sk-plan");
    assert_eq!(out["env"]["OLD_KEY"], "sk-old", "无引用的键也不删");
}

/// A custom pid's mapping is reused and refreshed; deleting its card cleans up the orphan mapping.
#[test]
fn custom_pid_mapping_is_refreshed_and_orphans_pruned() {
    let base: Value = serde_json::from_str(
        r#"{
                "modelProviders": { "idealab": [ { "id": "m1" } ] },
                "providerProtocol": { "idealab": "openai", "stale": "anthropic" }
            }"#,
    )
    .unwrap();
    let env = None;
    let protocols = provider_protocols(&base).cloned();
    let load = build_load(entries_from(&base), env, protocols.as_ref());
    assert_eq!(load.providers.len(), 1);
    let out = QwenCodeBackend.serialize_root(&[], &load.providers, &base, None);
    assert_eq!(out["providerProtocol"]["idealab"], "openai");
    assert!(
        out["providerProtocol"].get("stale").is_none(),
        "孤儿映射应被清理"
    );
}

/// Built-in pids need no mapping; after a protocol switch, the entry's written `wireApi` follows.
#[test]
fn builtin_pid_needs_no_mapping_and_wire_follows_api() {
    let mut p = ProviderRow::new();
    p.key = "gpt-4o".into();
    p.base_url = "https://api.openai.com/v1".into();
    p.pi_api = "openai-responses".into();
    let out = QwenCodeBackend.serialize_root(
        &[],
        std::slice::from_ref(&p),
        &Value::Object(Map::new()),
        None,
    );
    assert_eq!(out["modelProviders"]["openai"][0]["wireApi"], "responses");
    assert!(out.get("providerProtocol").is_none(), "内置 pid 不写映射");
}

/// Non-OpenAI-family protocols must not write `wireApi` (officially documented as a config error).
#[test]
fn wire_api_is_absent_for_non_openai_protocols() {
    let mut p = ProviderRow::new();
    p.key = "claude".into();
    p.pi_api = "anthropic-messages".into();
    let out = QwenCodeBackend.serialize_root(
        &[],
        std::slice::from_ref(&p),
        &Value::Object(Map::new()),
        None,
    );
    assert_eq!(out["modelProviders"]["anthropic"][0]["id"], "claude");
    assert!(out["modelProviders"]["anthropic"][0]
        .get("wireApi")
        .is_none());
}

/// Written entries carry no `disabled` key; `ModelRow.disabled` is a field on the UI's shared structure.
#[test]
fn written_entries_carry_no_disabled_key() {
    let mut p = ProviderRow::new();
    p.key = "m1".into();
    let mut m = ModelRow::new();
    m.id = "m1".into();
    m.disabled = true; // A value on the shared structure — invisible to saving
    p.models.push(m);
    let out = QwenCodeBackend.serialize_root(
        &[],
        std::slice::from_ref(&p),
        &Value::Object(Map::new()),
        None,
    );
    let entry = &out["modelProviders"]["openai"][0];
    assert_eq!(entry["id"], "m1", "没有停用概念：全部条目都进主配置");
    assert!(entry.get("disabled").is_none());
}

/// Keys in an entry the UI doesn't manage must be inherited.
#[test]
fn unmanaged_entry_keys_are_inherited() {
    let base: Value = serde_json::from_str(
        r#"{ "modelProviders": { "openai": [ {
                "id": "gpt-4o",
                "capabilities": { "agent": true },
                "generationConfig": { "maxRetries": 3, "samplingParams": { "temperature": 0.2 } }
            } ] } }"#,
    )
    .unwrap();
    let load = build_load(entries_from(&base), None, None);
    let out = QwenCodeBackend.serialize_root(&[], &load.providers, &base, None);
    let e = &out["modelProviders"]["openai"][0];
    assert_eq!(
        e["capabilities"]["agent"], true,
        "capabilities 其余子键保留"
    );
    assert_eq!(e["generationConfig"]["maxRetries"], 3);
    assert_eq!(
        e["generationConfig"]["samplingParams"]["temperature"], 0.2,
        "采样参数的其它键保留"
    );
}

/// Detection: judges shape, not key names.
#[test]
fn detect_requires_the_array_shape() {
    assert!(QwenCodeBackend.detect(
        r#"{ "modelProviders": { "openai": [ { "id": "m" } ] } }"#,
        ""
    ));
    assert!(!QwenCodeBackend.detect(r#"{ "modelProviders": { "openai": "not-an-array" } }"#, ""));
    assert!(!QwenCodeBackend.detect(
        r#"{ "modelProviders": { "openai": [ { "name": "无 id" } ] } }"#,
        ""
    ));
    // Strong path hit: directory and file name matching suffices — even an empty file counts.
    assert!(QwenCodeBackend.detect("{}", r"C:\Users\me\.qwen\settings.json"));
}
