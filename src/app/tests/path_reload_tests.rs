use crate::app::App;
use crate::format::{ConfigFormat, ConfigPaths};

fn missing_json_path() -> String {
    let path = std::env::temp_dir().join(format!(
        "model-harbor-missing-pi-{}-models.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

#[test]
fn missing_json_on_pi_page_does_not_overwrite_opencode_path() {
    let mut app = App::default();
    let opencode_path = r"D:\existing-opencode\opencode.json";
    app.config_paths
        .set_local_path(ConfigFormat::Opencode, opencode_path);
    // 本机 settings.json 可能给 pi 页留过覆盖路径（例如误填的 opencode 文件）；
    // 这条测试断言的是「不存在的路径不该记进 pi 覆盖」，与既有覆盖无关，
    // 先清掉才能独立成立。
    app.config_paths.set_local_path(
        ConfigFormat::Pi,
        &ConfigPaths::default_local_path(ConfigFormat::Pi),
    );
    app.current_page = ConfigFormat::Pi;
    app.config_path = missing_json_path();
    app.loaded_path.clear();

    app.reload();

    assert_eq!(
        app.config_paths.local_path(ConfigFormat::Opencode),
        opencode_path,
        "不存在的 .json 不能因探测回落污染 opencode 覆盖"
    );
    assert_eq!(app.current_page, ConfigFormat::Pi, "加载失败时应留在原页面");
    assert_eq!(
        app.config_paths.local_path(ConfigFormat::Pi),
        ConfigPaths::default_local_path(ConfigFormat::Pi),
        "加载失败也不能把不存在路径记到 pi 覆盖"
    );
}

#[test]
fn existing_pi_file_is_remembered_only_by_detected_page() {
    let path =
        std::env::temp_dir().join(format!("model-harbor-pi-owner-{}.json", std::process::id()));
    std::fs::write(&path, r#"{"providers": {}}"#).expect("写 pi 临时配置");
    let path = path.to_string_lossy().into_owned();
    let mut app = App::default();
    let opencode_path = r"D:\existing-opencode\opencode.json";
    app.config_paths
        .set_local_path(ConfigFormat::Opencode, opencode_path);
    app.current_page = ConfigFormat::Opencode;
    app.config_path = path.clone();
    app.loaded_path.clear();

    app.reload();

    assert_eq!(app.source_format, ConfigFormat::Pi);
    assert_eq!(app.current_page, ConfigFormat::Pi);
    assert_eq!(app.config_paths.local_path(ConfigFormat::Pi), path);
    assert_eq!(
        app.config_paths.local_path(ConfigFormat::Opencode),
        opencode_path,
        "真实 pi 文件应记到 pi 页面"
    );
    let _ = std::fs::remove_file(app.config_path);
}

/// 启动方言判定必须以文件内容为准：把 opencode.json 填到 pi 页时，
/// 按页面推断会解析出 0 条 provider（「写了路径却不自动加载」）。
#[test]
fn startup_format_follows_file_content_not_the_page() {
    use crate::app::startup_format;
    // opencode 方言（有 agent / provider 顶层键）
    let oc = std::env::temp_dir().join(format!(
        "model-harbor-startup-oc-{}.json",
        std::process::id()
    ));
    std::fs::write(&oc, r#"{"provider": {"a": {"models": []}}}"#).expect("写 opencode 临时配置");
    let oc = oc.to_string_lossy().into_owned();

    // pi 方言（providers 复数）
    let pi = std::env::temp_dir().join(format!(
        "model-harbor-startup-pi-{}.json",
        std::process::id()
    ));
    std::fs::write(&pi, r#"{"providers": {"a": {"models": []}}}"#).expect("写 pi 临时配置");
    let pi = pi.to_string_lossy().into_owned();

    // 关键场景：pi 页填了 opencode 文件 → 必须纠正成 opencode
    assert_eq!(
        startup_format(ConfigFormat::Pi, &oc),
        ConfigFormat::Opencode,
        "opencode 文件被填到 pi 页时应按内容纠正为 opencode"
    );
    // 反向同理
    assert_eq!(
        startup_format(ConfigFormat::Opencode, &pi),
        ConfigFormat::Pi
    );
    // 路径为空 / 不存在：保持页面推断（不猜）
    assert_eq!(startup_format(ConfigFormat::Pi, "   "), ConfigFormat::Pi);
    let missing = std::env::temp_dir()
        .join("model-harbor-definitely-missing.json")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        startup_format(ConfigFormat::Pi, &missing),
        ConfigFormat::Pi,
        "文件不存在时没有格式证据，不能据此改判"
    );

    let _ = std::fs::remove_file(&oc);
    let _ = std::fs::remove_file(&pi);
}
