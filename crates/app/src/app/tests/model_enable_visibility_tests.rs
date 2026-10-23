use crate::app::providers::ProviderFormFlags;
use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::{ModelRow, ProviderRow};

/// Build an App of "file from opencode, current page = `page`".
fn app_loaded_from_opencode(page: ConfigFormat) -> App {
    let mut model = ModelRow::new();
    model.id = "m1".into();
    model.source_format = Some(ConfigFormat::Opencode);
    model.raw = serde_json::json!({ "name": "m1" });
    let mut provider = ProviderRow::new();
    provider.key = "p1".into();
    provider.models = vec![model];
    App {
        providers: vec![provider],
        source_format: ConfigFormat::Opencode,
        current_page: page,
        ..App::default()
    }
}

#[test]
fn only_workbuddy_shows_the_enable_toggle() {
    for page in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
        ConfigFormat::Pi,
        ConfigFormat::OhMyPi,
        ConfigFormat::DeepSeekHarness,
        ConfigFormat::ZCode,
        ConfigFormat::QwenCode,
        ConfigFormat::KimiCode,
    ] {
        let app = app_loaded_from_opencode(page);
        assert!(
            !ProviderFormFlags::new(&app).show_model_disabled,
            "{} 页不该有「启用」开关",
            page.label()
        );
    }
    let app = app_loaded_from_opencode(ConfigFormat::WorkBuddy);
    assert!(
        ProviderFormFlags::new(&app).show_model_disabled,
        "WorkBuddy 页必须有「启用」开关"
    );
}

/// With the file from WorkBuddy, other pages don't show the switch either.
#[test]
fn a_workbuddy_file_does_not_leak_the_toggle_onto_other_pages() {
    let mut app = app_loaded_from_opencode(ConfigFormat::ZCode);
    app.source_format = ConfigFormat::WorkBuddy;
    assert!(
        !ProviderFormFlags::new(&app).show_model_disabled,
        "文件来自 WorkBuddy，但当前页是 ZCode，不该显示开关"
    );
}

/// The predicate itself: only WorkBuddy is true.
#[test]
fn has_model_enable_is_workbuddy_only() {
    for format in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
        ConfigFormat::Pi,
        ConfigFormat::OhMyPi,
        ConfigFormat::DeepSeekHarness,
        ConfigFormat::ZCode,
        ConfigFormat::QwenCode,
        ConfigFormat::KimiCode,
    ] {
        assert!(
            !format.has_model_enable(),
            "{} 不该有启用语义",
            format.label()
        );
    }
    assert!(ConfigFormat::WorkBuddy.has_model_enable());
}
