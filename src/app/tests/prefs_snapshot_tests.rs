use crate::app::App;

#[test]
fn path_override_treats_raw_default_as_unmodified_even_when_jsonc_variant_exists() {
    // 现实场景：mimocode 默认 `.json` 不在盘上，CLI 实际用 `.jsonc` 变体。
    // 解析后的默认变成 `.jsonc`，而界面字段保持原始默认 `.json`——
    // 只对解析值比较会把没改过的默认路径误判成手动覆盖，每次退出都
    // 写回 settings.json，启动永远钉在该页。
    use crate::app::ids::override_or_empty;
    let raw = r"C:\Users\u\.config\mimocode\mimocode.json";
    let resolved = r"C:\Users\u\.config\mimocode\mimocode.jsonc";
    // 字段保持原始默认（未手动改过）→ 不算覆盖
    assert_eq!(override_or_empty(raw, raw, resolved), "");
    // 字段被探测解析带成了 .jsonc 变体 → 也不算覆盖
    assert_eq!(override_or_empty(resolved, raw, resolved), "");
    // 用户真的改了路径 → 照实记录
    assert_eq!(
        override_or_empty(r"D:\custom\mimo.json", raw, resolved),
        r"D:\custom\mimo.json"
    );
    // 空字段 → 空串，不算覆盖
    assert_eq!(override_or_empty("  ", raw, resolved), "  ");
}

#[test]
fn collapse_state_is_scoped_to_configuration_path() {
    let mut app = App {
        config_path: r"D:\configs\one.json".into(),
        loaded_path: r"D:\configs\one.json".into(),
        ..Default::default()
    };
    app.set_provider_collapsed("shared", true);
    assert!(app.provider_collapsed("shared"));

    app.config_path = r"D:\configs\two.json".into();
    app.loaded_path = app.config_path.clone();
    assert!(
        !app.provider_collapsed("shared"),
        "另一份配置不能继承第一份配置的折叠状态"
    );
    app.set_provider_collapsed("shared", true);

    app.config_path = r"D:\configs\one.json".into();
    app.loaded_path = app.config_path.clone();
    assert!(app.provider_collapsed("shared"));
}

#[test]
fn legacy_collapse_key_migrates_to_loaded_configuration() {
    let mut app = App {
        config_path: r"D:\configs\legacy.json".into(),
        loaded_path: r"D:\configs\legacy.json".into(),
        providers: vec![crate::model::ProviderRow {
            key: "legacy-provider".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let legacy = crate::prefs::legacy_collapsed_id("providers", "legacy-provider");
    app.collapsed.insert(legacy.clone());

    app.migrate_legacy_collapsed();

    assert!(!app.collapsed.contains(&legacy));
    assert!(app.provider_collapsed("legacy-provider"));
}

#[test]
fn pruning_one_configuration_keeps_other_configuration_state() {
    let mut app = App {
        config_path: r"D:\configs\one.json".into(),
        loaded_path: r"D:\configs\one.json".into(),
        providers: vec![crate::model::ProviderRow {
            key: "alive".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    app.set_provider_collapsed("alive", true);
    app.set_provider_collapsed("removed", true);
    let other = crate::prefs::collapsed_id(
        &crate::prefs::config_identity(r"D:\configs\two.json"),
        "providers",
        "other",
    );
    app.collapsed.insert(other.clone());

    app.prune_collapsed();

    assert!(app.provider_collapsed("alive"));
    assert!(!app.provider_collapsed("removed"));
    assert!(app.collapsed.contains(&other));
}
#[test]
fn current_prefs_sorts_collapsed_cards_before_comparison() {
    let mut app = App::default();
    app.collapsed.clear();
    app.collapsed.insert("providers/z".into());
    app.collapsed.insert("agents/a".into());

    assert_eq!(
        app.current_prefs().collapsed,
        vec!["agents/a".to_string(), "providers/z".to_string()]
    );
}
