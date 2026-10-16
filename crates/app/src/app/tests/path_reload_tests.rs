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
    // 先清掉本机可能给 pi 页留的覆盖路径，使断言独立成立。
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

/// 路径记到用户操作的当前页（owner），不跟文件内容走；内容检测只决定解析格式。
#[test]
fn path_is_remembered_by_owner_page_not_by_detected_content() {
    let path =
        std::env::temp_dir().join(format!("model-harbor-pi-owner-{}.json", std::process::id()));
    std::fs::write(&path, r#"{"providers": {}}"#).expect("写 pi 临时配置");
    let path = path.to_string_lossy().into_owned();
    let mut app = App::default();
    let opencode_path = r"D:\existing-opencode\opencode.json";
    app.config_paths
        .set_local_path(ConfigFormat::Opencode, opencode_path);
    // 在 opencode 页打开一个 pi 内容的文件。
    app.current_page = ConfigFormat::Opencode;
    app.config_path = path.clone();
    app.loaded_path.clear();

    app.reload();

    // 解析格式跟随内容（pi）；但页面留在 opencode、路径记到 opencode 页。
    assert_eq!(app.source_format, ConfigFormat::Pi, "解析格式跟随文件内容");
    assert_eq!(
        app.current_page,
        ConfigFormat::Opencode,
        "页面留在用户操作的 owner 页，不跟内容切页"
    );
    assert_eq!(
        app.config_paths.local_path(ConfigFormat::Opencode),
        path,
        "路径记到 owner 页（opencode），下次打开该页加载同一文件"
    );
    assert_eq!(
        app.config_paths.local_path(ConfigFormat::Pi),
        ConfigPaths::default_local_path(ConfigFormat::Pi),
        "pi 页的路径不被别人的切换污染"
    );
    let _ = std::fs::remove_file(app.config_path);
}

/// 启动页 = 顶栏第一个已安装后端 + 它自己的解析路径，不按文件内容切页。
#[test]
fn startup_page_is_first_installed_tab_regardless_of_content() {
    use crate::app::startup_page_with;
    // pi 页指向一个 opencode 内容的文件（误配）：启动仍应停在 pi 页、
    // 加载 pi 自己的解析路径，不再因内容是 opencode 而切到 opencode 页。
    let oc_content = std::env::temp_dir().join(format!(
        "model-harbor-startup-first-{}.json",
        std::process::id()
    ));
    std::fs::write(&oc_content, r#"{"provider": {"a": {"models": []}}}"#).expect("写临时配置");
    let oc_content = oc_content.to_string_lossy().into_owned();

    let mut paths = ConfigPaths::default();
    paths.set_local_path(ConfigFormat::Pi, &oc_content);
    let tab_order = vec!["pi".to_string(), "opencode".to_string()];
    let installed = |id: ConfigFormat| id == ConfigFormat::Pi || id == ConfigFormat::Opencode;

    let (format, path) = startup_page_with(&paths, &tab_order, installed).expect("有已安装页");
    assert_eq!(
        format,
        ConfigFormat::Pi,
        "第一个已安装后端是 pi，启动就停在 pi 页，不因文件内容是 opencode 而切页"
    );
    assert!(
        path.contains("model-harbor-startup-first"),
        "应加载 pi 页自己的解析路径，实际得到 {path}"
    );

    let _ = std::fs::remove_file(&oc_content);
}
