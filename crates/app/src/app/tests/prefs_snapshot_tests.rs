use crate::app::App;

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

/// 路径覆盖不再持久化：即使会话内切到了别的文件，current_prefs 仍写空覆盖，
/// 重启后每页回官方默认路径（用户拍板）。
#[test]
fn current_prefs_never_persists_path_overrides() {
    use crate::format::ConfigFormat;
    let mut app = App::default();
    // 会话内把 opencode 页切到桌面文件。
    app.config_paths.set_local_path(
        ConfigFormat::Opencode,
        r"C:\Users\u\Desktop\opencodev2.json",
    );
    let prefs = app.current_prefs();
    assert_eq!(
        prefs.config_paths,
        crate::prefs::ConfigPathPrefs::default(),
        "全部覆盖槽位恒为空：路径覆盖不得写进 settings.json"
    );
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

/// 用量区间被 current_prefs 捕获为稳定键，并能从 Prefs 还原（记住上次选择）。
#[test]
fn current_prefs_captures_usage_range() {
    use crate::app::usage::range::Range;
    let app = App {
        usage_range: Range::Week,
        ..Default::default()
    };
    assert_eq!(app.current_prefs().usage_range, "week");
    // 往返：写进 Prefs 再按键还原，应回到 Week。
    let prefs = app.current_prefs();
    assert_eq!(Range::from_key(&prefs.usage_range), Range::Week);
}
