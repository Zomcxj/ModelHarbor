use crate::app::save::SaveTarget;
use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::{ModelRow, ProviderRow};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_saveall_{}_{}_{}",
        tag,
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn provider() -> ProviderRow {
    let mut p = ProviderRow::new();
    p.key = "p1".into();
    p.pi_api = "openai-completions".into();
    p.base_url = "https://x.example/v1".into();
    p.api_key = "sk-1".into();
    let mut m = ModelRow::new();
    m.id = "m1".into();
    p.models = vec![m];
    p
}

#[test]
fn every_installed_target_is_written_to_its_own_path() {
    let dir = temp_dir("paths");
    let pi_path = dir.join("pi-models.json");
    let zcode_path = dir.join("provider_config.json");
    let skipped_start = dir.join("opencode.json");
    let targets = vec![
        SaveTarget {
            backend: ConfigFormat::Pi,
            available: true,
            path: pi_path.display().to_string(),
        },
        SaveTarget {
            backend: ConfigFormat::ZCode,
            available: true,
            path: zcode_path.display().to_string(),
        },
        // 未安装的后端：一键保存不得创建
        SaveTarget {
            backend: ConfigFormat::Opencode,
            available: false,
            path: skipped_start.display().to_string(),
        },
    ];
    let mut app = App {
        providers: vec![provider()],
        targets,
        config_path: pi_path.display().to_string(),
        loaded_path: pi_path.display().to_string(),
        source_format: ConfigFormat::Pi,
        current_page: ConfigFormat::Pi,
        ..App::default()
    };
    app.save_all();

    assert!(pi_path.exists(), "当前页写自己的目标: {}", app.status);
    assert!(
        zcode_path.exists(),
        "其他页写各自的目标路径，不能都塞进当前页那个文件: {}",
        app.status
    );
    assert!(!skipped_start.exists(), "未安装的后端不得被创建");
    // ZCode 那份是跨格式转换结果：必须是 ZCode 方言，不是 pi 的 providers。
    let written = std::fs::read_to_string(&zcode_path).unwrap();
    assert!(written.contains("providerRules"), "{written}");
    assert!(!written.contains("\"providers\""), "{written}");
    // 状态栏逐页列出结果（写没写、写到哪）
    assert!(
        app.status.contains(ConfigFormat::Pi.label()),
        "{}",
        app.status
    );
    assert!(
        app.status.contains(ConfigFormat::ZCode.label()),
        "{}",
        app.status
    );
    assert!(app.status.starts_with("一键保存:"), "{}", app.status);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_typed_path_on_the_current_page_is_still_honored() {
    // 当前页输入框里改了路径但未回车加载：一键保存仍写到那个路径；
    // 其他页面改用各自的目标路径。
    let dir = temp_dir("typed");
    let pi_path = dir.join("pi-models.json");
    let typed_path = dir.join("typed-by-hand.json");
    let zcode_path = dir.join("provider_config.json");
    let targets = vec![
        SaveTarget {
            backend: ConfigFormat::Pi,
            available: true,
            path: pi_path.display().to_string(),
        },
        SaveTarget {
            backend: ConfigFormat::ZCode,
            available: true,
            path: zcode_path.display().to_string(),
        },
    ];
    let mut app = App {
        providers: vec![provider()],
        targets,
        config_path: typed_path.display().to_string(),
        loaded_path: pi_path.display().to_string(),
        source_format: ConfigFormat::Pi,
        current_page: ConfigFormat::Pi,
        ..App::default()
    };
    app.save_all();
    assert!(
        typed_path.exists(),
        "当前页仍按输入框里的路径写: {}",
        app.status
    );
    assert!(!pi_path.exists(), "默认目标不该被写（用户已改路径）");
    assert!(zcode_path.exists(), "其他页照旧写各自的目标");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn opencode_wsl_sync_mirrors_current_config_and_backups_target() {
    // 本测试依赖**真实的 wsl.exe**（写入目标、镜像、读回校验）。
    // 显式 opt-in：设置 MODELHARBOR_WSL_TEST=1 才执行，否则跳过。
    if std::env::var("MODELHARBOR_WSL_TEST").as_deref() != Ok("1") {
        eprintln!("跳过：设置 MODELHARBOR_WSL_TEST=1 才运行真实 WSL 同步测试（会启动 WSL）");
        return;
    }
    if !crate::util::wsl_usable() {
        eprintln!("跳过：本机没有可用的 WSL，无法做 WSL 同步测试");
        return;
    }
    // 预置目标文件前显式开闸。
    crate::util::wsl_set_enabled(true);
    let dir = temp_dir("opencode_wsl");
    let local_path = dir.join("opencode.json");
    let wsl_path = format!("/tmp/model_harbor_opencode_{}.json", std::process::id());
    let source = serde_json::json!({
        "agent": { "writer": { "mode": "subagent", "model": "p1/m1" } },
        "provider": {
            "p1": {
                "npm": "@ai-sdk/openai-compatible",
                "options": { "baseURL": "https://source.example/v1", "apiKey": "test-key" },
                "models": { "m1": { "name": "m1" } }
            }
        },
        "mcp": { "keep": true }
    });
    let target = serde_json::json!({
        "agent": { "old": { "mode": "subagent" } },
        "provider": { "old": { "models": {} } },
        "mcp": { "target": true }
    });
    std::fs::write(&local_path, serde_json::to_string(&source).unwrap()).unwrap();
    let target_text = serde_json::to_string(&target).unwrap();
    crate::backends::write_config(&wsl_path, &target_text).expect("写入 WSL 测试目标");

    let local_text = std::fs::read_to_string(&local_path).unwrap();
    let load = crate::backends::backend(ConfigFormat::Opencode)
        .parse_at(&local_text, &local_path.display().to_string())
        .unwrap();
    let mut app = App {
        agents: load.agents,
        providers: load.providers,
        root: load.root,
        config_path: local_path.display().to_string(),
        loaded_path: local_path.display().to_string(),
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        ..App::default()
    };
    // `App::default()` 会按 `prefs.sync_wsl` 复位总闸，构造后再开一次。
    crate::util::wsl_set_enabled(true);

    let backup = app
        .save_backend_to(ConfigFormat::Opencode, &wsl_path)
        .expect("同步到 WSL 应成功")
        .expect("覆盖 WSL 配置前必须备份旧文件");
    assert_eq!(backup, format!("{wsl_path}.bak"));
    let old_backup = crate::util::read_config_content(&backup).expect("读取 WSL 备份");
    let old_output: serde_json::Value = serde_json::from_str(&old_backup).unwrap();
    assert!(old_output["provider"].get("old").is_some());

    let written = crate::util::read_config_content(&wsl_path).expect("读取 WSL 测试目标");
    let output: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert!(
        output["agent"].get("writer").is_some(),
        "agent 未同步: {output}"
    );
    assert!(
        output["provider"].get("p1").is_some(),
        "provider 未同步: {output}"
    );
    assert!(
        output["provider"].get("old").is_none(),
        "WSL 旧 provider 不应残留: {output}"
    );
    assert_eq!(output["mcp"]["keep"], true);
    assert!(output["mcp"].get("target").is_none());

    crate::util::remove_config(&backup).ok();
    crate::util::remove_config(&wsl_path).ok();
    std::fs::remove_dir_all(&dir).ok();
    // 收尾关闸，避免影响其他并行测试。
    crate::util::wsl_set_enabled(false);
}

#[test]
fn a_workbuddy_save_that_drops_entries_backs_up_first() {
    // 生效清单比界面条目少（同 id 只留第一条、未勾选的不写），
    // 所以同格式保存也必须先备份。
    //
    // 备份与全量副本是两件事：备份是上一次的 models.json 原文，
    // 全量副本是 ModelHarbor 维护的全部条目 + 勾选状态。
    let dir = temp_dir("wb_shrink");
    let wb_path = dir.join("models.json");
    let saved = serde_json::json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1",
          "apiKey": "sk-a" },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://y.example/v1",
          "apiKey": "sk-b" }
    ]);
    std::fs::write(&wb_path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
    let text = std::fs::read_to_string(&wb_path).unwrap();
    let load = crate::backends::backend(ConfigFormat::WorkBuddy)
        .parse(&text)
        .unwrap();
    let mut app = App {
        providers: load.providers,
        config_path: wb_path.display().to_string(),
        loaded_path: wb_path.display().to_string(),
        source_format: ConfigFormat::WorkBuddy,
        current_page: ConfigFormat::WorkBuddy,
        ..App::default()
    };
    let backup = app
        .save_backend_to(ConfigFormat::WorkBuddy, &wb_path.display().to_string())
        .expect("保存应当成功");
    let backup = backup.expect("删条目时必须先备份");
    let kept: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&wb_path).unwrap()).unwrap();
    assert_eq!(kept.as_array().unwrap().len(), 1, "只留启用的那条");
    let old: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
    assert_eq!(
        old.as_array().unwrap().len(),
        2,
        "备份必须是删之前的两条（含被删厂商的 key）"
    );
    // 全量副本也必须在。
    let full_path = crate::backends::workbuddy::full_store_path(&wb_path.display().to_string());
    let full: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&full_path).unwrap()).unwrap();
    assert_eq!(
        full.as_array().unwrap().len(),
        2,
        "全量副本必须两条都在（含被筛掉那条的 key）: {full:#?}"
    );
    assert_eq!(
        full.as_array().unwrap()[1]["apiKey"],
        serde_json::json!("sk-b"),
        "被筛掉那条的 key 必须留在副本里"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn workbuddy_reload_restores_unchecked_entries_from_the_full_store() {
    // 保存 → 重新加载后，界面上的条目数回到保存前（取消勾选只是不生效）。
    let dir = temp_dir("wb_restore");
    let wb_path = dir.join("models.json");
    let saved = serde_json::json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1",
          "apiKey": "sk-a", "tags": ["a-only"] },
        { "id": "gpt-5.6-sol", "name": "b", "url": "https://y.example/v1",
          "apiKey": "sk-b", "tags": ["b-only"] },
        { "id": "unique", "name": "c", "url": "https://z.example/v1",
          "apiKey": "sk-c" }
    ]);
    std::fs::write(&wb_path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
    let path = wb_path.display().to_string();
    let text = std::fs::read_to_string(&wb_path).unwrap();
    let b = crate::backends::backend(ConfigFormat::WorkBuddy);
    let load = b.parse_at(&text, &path).unwrap();
    let before: usize = load.providers.iter().map(|p| p.models.len()).sum();
    assert_eq!(before, 3, "首次加载应看到全部三条");

    let mut app = App {
        providers: load.providers,
        config_path: path.clone(),
        loaded_path: path.clone(),
        source_format: ConfigFormat::WorkBuddy,
        current_page: ConfigFormat::WorkBuddy,
        ..App::default()
    };
    app.save_backend_to(ConfigFormat::WorkBuddy, &path)
        .expect("保存应当成功");

    // 生效清单确实变少了。
    let eff: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&wb_path).unwrap()).unwrap();
    assert_eq!(eff.as_array().unwrap().len(), 2, "生效清单 = 唯一 id 数");

    // 重新加载后条目数回到 3（从全量副本还原）。
    let reloaded = b
        .parse_at(&std::fs::read_to_string(&wb_path).unwrap(), &path)
        .unwrap();
    let after: usize = reloaded.providers.iter().map(|p| p.models.len()).sum();
    assert_eq!(after, before, "重新加载必须从全量副本还原全部条目");
    // 被筛掉那条的字段也必须完好。
    let all: Vec<(String, String)> = reloaded
        .providers
        .iter()
        .flat_map(|p| p.models.iter().map(move |m| (p.key.clone(), m.id.clone())))
        .collect();
    assert!(all.contains(&("b".to_string(), "gpt-5.6-sol".to_string())));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_workbuddy_save_that_keeps_everything_writes_no_backup() {
    // 没删东西就不产生 .bak。
    let dir = temp_dir("wb_keep");
    let wb_path = dir.join("models.json");
    let saved = serde_json::json!([
        { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1",
          "apiKey": "sk-a" },
        { "id": "claude-opus-5", "name": "b", "url": "https://y.example/v1",
          "apiKey": "sk-b" }
    ]);
    std::fs::write(&wb_path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
    let text = std::fs::read_to_string(&wb_path).unwrap();
    let load = crate::backends::backend(ConfigFormat::WorkBuddy)
        .parse(&text)
        .unwrap();
    let mut app = App {
        providers: load.providers,
        config_path: wb_path.display().to_string(),
        loaded_path: wb_path.display().to_string(),
        source_format: ConfigFormat::WorkBuddy,
        current_page: ConfigFormat::WorkBuddy,
        ..App::default()
    };
    let backup = app
        .save_backend_to(ConfigFormat::WorkBuddy, &wb_path.display().to_string())
        .expect("保存应当成功");
    assert!(backup.is_none(), "没删条目就不该产生 .bak: {backup:?}");
    assert!(!dir.join("models.json.bak").exists());
    std::fs::remove_dir_all(&dir).ok();
}
