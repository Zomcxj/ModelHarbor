use crate::app::providers::ProviderFormFlags;
use crate::app::App;
use crate::format::ConfigFormat;
use std::path::PathBuf;

#[test]
fn only_the_workbuddy_page_offers_the_enable_toggle() {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let opencode = home.join(".config/opencode/opencode.json");
    if !opencode.exists() {
        return; // 非用户机器（CI）跳过
    }
    // 先加载 opencode 的文件（source ≠ 当前页）。
    let mut app = App {
        config_path: opencode.display().to_string(),
        ..App::default()
    };
    app.reload_for_page(ConfigFormat::Opencode, false);
    assert_eq!(app.source_format, ConfigFormat::Opencode);

    for page in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
        ConfigFormat::Pi,
        ConfigFormat::OhMyPi,
        ConfigFormat::DeepSeekHarness,
        ConfigFormat::ZCode,
    ] {
        app.current_page = page;
        assert!(
            !ProviderFormFlags::new(&app).show_model_disabled,
            "{} 页不该有「启用」开关",
            page.label()
        );
    }
    app.current_page = ConfigFormat::WorkBuddy;
    assert!(ProviderFormFlags::new(&app).show_model_disabled);
}
