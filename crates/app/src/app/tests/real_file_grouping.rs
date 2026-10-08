use crate::app::App;
use crate::format::ConfigFormat;
use std::path::PathBuf;

/// 走 App 的真实加载路径（`reload_for_page`）确认 WorkBuddy 的分组结果。
#[test]
fn workbuddy_loads_all_models_per_provider() {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let path = home.join(".workbuddy/models.json");
    if !path.exists() {
        return; // 非用户机器（CI）跳过
    }
    let mut app = App {
        config_path: path.display().to_string(),
        ..App::default()
    };
    app.reload_for_page(ConfigFormat::WorkBuddy, false);

    let multi: Vec<(String, usize)> = app
        .providers
        .iter()
        .filter(|p| p.models.len() > 1)
        .map(|p| (p.key.clone(), p.models.len()))
        .collect();
    println!(
        "App 加载: page={:?} source={:?} providers={} models={} 多模型={:?}",
        app.current_page,
        app.source_format,
        app.providers.len(),
        app.providers.iter().map(|p| p.models.len()).sum::<usize>(),
        multi
    );
    assert!(
        app.providers.iter().any(|p| p.models.len() > 1),
        "一家提供商的多个模型必须都进同一个 provider 行"
    );
}
