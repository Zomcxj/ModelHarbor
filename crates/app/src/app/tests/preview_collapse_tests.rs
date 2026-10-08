use crate::app::App;
use crate::format::ConfigFormat;

#[test]
fn preview_edits_preserve_surviving_collapsed_cards_and_prune_removed_ones() {
    let mut app = App {
        current_page: ConfigFormat::Opencode,
        source_format: ConfigFormat::Opencode,
        ..Default::default()
    };
    app.set_provider_collapsed("a", true);
    app.set_provider_collapsed("removed", true);
    app.preview_draft = r#"{
            "provider": {
                "a": {"description": "edited"},
                "c": {}
            }
        }"#
    .into();

    assert!(app.apply_preview_draft());
    assert!(app.provider_collapsed("a"), "存活卡片应保留折叠状态");
    assert!(!app.provider_collapsed("c"), "新增卡片天然展开");
    assert!(
        !app.provider_collapsed("removed"),
        "草稿删除的卡片应清理失效折叠键"
    );
}

#[test]
fn invalid_preview_does_not_change_cards_or_collapse_state() {
    let mut app = App {
        current_page: ConfigFormat::Opencode,
        source_format: ConfigFormat::Opencode,
        preview_draft: r#"{"provider":{"a":{}}}"#.into(),
        ..Default::default()
    };
    assert!(app.apply_preview_draft());
    app.set_provider_collapsed("a", true);
    let before_root = app.root.clone();
    let before_keys: Vec<String> = app.providers.iter().map(|p| p.key.clone()).collect();
    let before_collapsed = app.collapsed.clone();

    app.preview_draft = "{ invalid".into();
    assert!(!app.apply_preview_draft());

    assert_eq!(app.root, before_root);
    assert_eq!(
        app.providers
            .iter()
            .map(|p| p.key.clone())
            .collect::<Vec<_>>(),
        before_keys
    );
    assert_eq!(app.collapsed, before_collapsed);
}
