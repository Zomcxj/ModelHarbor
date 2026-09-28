use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::{ModelRow, ProviderRow};

fn provider(key: &str, ids: &[&str]) -> ProviderRow {
    let mut p = ProviderRow::new();
    p.key = key.to_string();
    p.models = ids
        .iter()
        .map(|id| {
            let mut m = ModelRow::new();
            m.id = (*id).to_string();
            m
        })
        .collect();
    p
}

#[test]
fn duplicates_all_enabled_are_reduced_to_the_first() {
    let mut app = App {
        providers: vec![
            provider("a", &["gpt-5.6-sol", "other"]),
            provider("b", &["gpt-5.6-sol"]),
            provider("c", &["gpt-5.6-sol"]),
        ],
        source_format: ConfigFormat::WorkBuddy,
        current_page: ConfigFormat::WorkBuddy,
        ..App::default()
    };
    app.normalize_workbuddy_enable_flags();
    let flags: Vec<(&str, &str, bool)> = app
        .providers
        .iter()
        .flat_map(|p| {
            p.models
                .iter()
                .map(|m| (p.key.as_str(), m.id.as_str(), m.disabled))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        flags,
        vec![
            ("a", "gpt-5.6-sol", false),
            ("a", "other", false),
            ("b", "gpt-5.6-sol", true),
            ("c", "gpt-5.6-sol", true),
        ],
        "同一 id 只留第一条启用，不同 id 不受影响"
    );
}

#[test]
fn normalization_is_idempotent_and_keeps_a_users_choice() {
    let mut app = App {
        providers: vec![provider("a", &["m1"]), provider("b", &["m1"])],
        source_format: ConfigFormat::WorkBuddy,
        current_page: ConfigFormat::WorkBuddy,
        ..App::default()
    };
    // 用户明确关掉第一条、启用第二条（与「第一条生效」相反）。
    app.providers[0].models[0].disabled = true;
    app.providers[1].models[0].disabled = false;
    app.normalize_workbuddy_enable_flags();
    assert!(
        app.providers[0].models[0].disabled,
        "用户关掉的那条不能被重新打开"
    );
    assert!(
        !app.providers[1].models[0].disabled,
        "用户勾的那条必须保持启用"
    );
    // 幂等：再跑一次不变。
    app.normalize_workbuddy_enable_flags();
    assert!(app.providers[0].models[0].disabled);
    assert!(!app.providers[1].models[0].disabled);
}
