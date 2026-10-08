use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_pageagents_{}_{}_{}",
        tag,
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn agent(key: &str, model: &str) -> AgentRow {
    let mut row = AgentRow::new();
    row.key = key.to_string();
    row.model = model.to_string();
    row
}

fn configured_provider(key: &str, model: &str) -> ProviderRow {
    let mut p = ProviderRow::new();
    p.key = key.to_string();
    p.models = vec![{
        let mut m = crate::model::ModelRow::new();
        m.id = model.to_string();
        m
    }];
    p
}

/// 端到端：opencode 页的数据写进 kilo 文件，落盘的是 kilo 认的引用。
#[test]
fn saving_to_another_page_writes_that_pages_gateway_model() {
    let dir = temp_dir("kilo");
    let kilo_path = dir.join("kilo.json");
    let path = kilo_path.display().to_string();
    // 来源是 opencode 页，agents 里混着自家网关与自配 provider 两类引用。
    let mut app = App {
        agents: vec![
            agent("writing", "sensenova/sensenova-6.8-flash-lite"),
            agent("fallback", "opencode/ling-3.0-flash-fin-free"),
        ],
        providers: vec![configured_provider("sensenova", "sensenova-6.8-flash-lite")],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        config_path: path.clone(),
        loaded_path: path.clone(),
        ..App::default()
    };
    app.save_backend_to(ConfigFormat::Kilocode, &path)
        .expect("保存应当成功");

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&kilo_path).unwrap()).unwrap();
    let agents = written.get("agent").and_then(|v| v.as_object()).unwrap();
    assert_eq!(
        agents["writing"]["model"], "sensenova/sensenova-6.8-flash-lite",
        "自配 provider 的引用在 kilo 页同样有效（provider 容器一并写入），不该被改"
    );
    assert_eq!(
        agents["fallback"]["model"], "kilo/kilo-auto/free",
        "opencode 网关的引用必须换成 kilo 自家网关模型"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 端到端：从别的页存到 Kimi 文件，`managed:*` 登录态与它名下的官方模型都保留。
#[test]
fn saving_to_the_kimi_page_keeps_the_managed_provider_and_its_models() {
    let dir = temp_dir("kimi_managed");
    let kimi_path = dir.join("config.toml");
    let path = kimi_path.display().to_string();
    std::fs::write(
        &kimi_path,
        "default_model = \"kimi-code/k3\"\n\n\
             [providers.\"managed:kimi-code\"]\n\
             base_url = \"https://api.kimi.com/coding/v1\"\n\
             type = \"kimi\"\n\
             api_key = \"\"\n\n\
             [providers.\"managed:kimi-code\".oauth]\n\
             storage = \"file\"\n\
             key = \"oauth/kimi-code\"\n\n\
             [providers.sensenova]\n\
             type = \"openai\"\n\
             base_url = \"https://token.sensenova.cn/v1\"\n\n\
             [models.\"kimi-code/k3\"]\n\
             provider = \"managed:kimi-code\"\n\
             model = \"k3\"\n\
             max_context_size = 1048576\n\
             display_name = \"K3\"\n\n\
             [models.\"sensenova/sensenova-6.8-flash-lite\"]\n\
             provider = \"sensenova\"\n\
             model = \"sensenova-6.8-flash-lite\"\n\
             max_context_size = 65536\n",
    )
    .unwrap();

    let mut app = App {
        providers: vec![configured_provider("sensenova", "sensenova-6.8-flash-lite")],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        config_path: path.clone(),
        loaded_path: path.clone(),
        ..App::default()
    };
    app.save_backend_to(ConfigFormat::KimiCode, &path)
        .expect("保存应当成功");

    let written: toml::Value =
        toml::from_str(&std::fs::read_to_string(&kimi_path).unwrap()).unwrap();
    let providers = written["providers"].as_table().unwrap();
    assert!(providers.contains_key("managed:kimi-code"), "登录态被删了");
    assert_eq!(
        providers["managed:kimi-code"]["oauth"]["key"].as_str(),
        Some("oauth/kimi-code"),
        "oauth 子表要逐字保留"
    );
    let models = written["models"].as_table().unwrap();
    assert_eq!(
        models["kimi-code/k3"]["model"].as_str(),
        Some("k3"),
        "官方模型被删了"
    );
    assert_eq!(models["kimi-code/k3"]["display_name"].as_str(), Some("K3"));
    assert!(
        models.contains_key("sensenova/sensenova-6.8-flash-lite"),
        "界面接管的模型仍要写进去（缺省别名 = provider/model，Kimi 自己的约定）"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 端到端：opencode 页的 provider（密钥内联）跨格式存到 Qwen 文件，
/// 落盘的凭据含 `envKey` 与 `env` 两项。
#[test]
fn cross_format_save_writes_the_credential_for_an_env_less_provider() {
    let dir = temp_dir("qwen_env");
    let qwen_path = dir.join("settings.json");
    let path = qwen_path.display().to_string();

    let mut app = App {
        providers: Vec::new(),
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        config_path: path.clone(),
        loaded_path: path.clone(),
        ..App::default()
    };
    let mut p = ProviderRow::new();
    p.key = "sensenova".into();
    p.base_url = "https://token.sensenova.cn/v1".into();
    p.pi_api = "openai-completions".into();
    p.api_key = "sk-sensenova".into(); // opencode 的密钥是内联的
    let mut m = crate::model::ModelRow::new();
    m.id = "sensenova-6.8-flash-lite".into();
    m.name = "sensenova-6.8-flash-lite".into();
    p.models.push(m);
    app.providers = vec![p];

    app.save_backend_to(ConfigFormat::QwenCode, &path)
        .expect("保存应当成功");

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&qwen_path).unwrap()).unwrap();
    let entry = &written["modelProviders"]["openai"][0];
    assert_eq!(entry["id"], "sensenova-6.8-flash-lite");
    assert_eq!(entry["envKey"], "SENSENOVA_API_KEY");
    assert_eq!(written["env"]["SENSENOVA_API_KEY"], "sk-sensenova");
    std::fs::remove_dir_all(&dir).ok();
}

/// 端到端：从别的页存到 Qwen 文件，硬编码的 `qwen-oauth` 条目保留。
#[test]
fn saving_to_the_qwen_page_keeps_the_oauth_entries() {
    let dir = temp_dir("qwen_oauth");
    let qwen_path = dir.join("settings.json");
    let path = qwen_path.display().to_string();
    std::fs::write(
        &qwen_path,
        r#"{
              "$version": 4,
              "modelProviders": {
                "openai": [
                  { "id": "gpt-4o", "name": "GPT-4o", "envKey": "OPENAI_API_KEY",
                    "baseUrl": "https://api.openai.com/v1" }
                ],
                "qwen-oauth": [
                  { "id": "qwen3-coder-plus", "name": "Qwen3 Coder Plus",
                    "baseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1" }
                ]
              },
              "security": { "auth": { "selectedType": "openai" } }
            }"#,
    )
    .unwrap();

    let mut app = App {
        providers: vec![configured_provider("sensenova", "sensenova-6.8-flash-lite")],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        config_path: path.clone(),
        loaded_path: path.clone(),
        ..App::default()
    };
    app.save_backend_to(ConfigFormat::QwenCode, &path)
        .expect("保存应当成功");

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&qwen_path).unwrap()).unwrap();
    assert_eq!(
        written["modelProviders"]["qwen-oauth"][0]["id"], "qwen3-coder-plus",
        "硬编码的 OAuth 条目被删了"
    );
    assert!(
        written["modelProviders"]["openai"][0]["id"] == "sensenova-6.8-flash-lite",
        "界面接管的 pid 要整体换成界面的条目"
    );
    assert_eq!(written["security"]["auth"]["selectedType"], "openai");
    std::fs::remove_dir_all(&dir).ok();
}

/// 同一份 agents 分别写三页，各自拿到自己网关认的值。
#[test]
fn each_page_gets_its_own_gateway_reference() {
    let dir = temp_dir("three");
    let mut app = App {
        agents: vec![agent("fallback", "opencode/ling-3.0-flash-fin-free")],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        ..App::default()
    };
    let mut written: Vec<(ConfigFormat, String)> = Vec::new();
    for (fmt, file) in [
        (ConfigFormat::Opencode, "opencode.json"),
        (ConfigFormat::Kilocode, "kilo.json"),
        (ConfigFormat::Mimocode, "mimocode.json"),
    ] {
        let path = dir.join(file).display().to_string();
        app.save_backend_to(fmt, &path).expect("保存应当成功");
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let model = root["agent"]["fallback"]["model"]
            .as_str()
            .unwrap()
            .to_string();
        written.push((fmt, model));
    }
    assert_eq!(written[0].1, "opencode/ling-3.0-flash-fin-free");
    assert_eq!(written[1].1, "kilo/kilo-auto/free");
    assert_eq!(written[2].1, "mimo/mimo-auto");
    std::fs::remove_dir_all(&dir).ok();
}

/// 保存到别的页不改动界面状态：写出去时归一，`agents` 保持原值。
#[test]
fn saving_to_another_page_does_not_mutate_the_ui_state() {
    let dir = temp_dir("nomutate");
    let path = dir.join("kilo.json").display().to_string();
    let mut app = App {
        agents: vec![agent("fallback", "opencode/ling-3.0-flash-fin-free")],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        ..App::default()
    };
    app.save_backend_to(ConfigFormat::Kilocode, &path)
        .expect("保存应当成功");
    assert_eq!(
        app.agents[0].model, "opencode/ling-3.0-flash-fin-free",
        "写别的页不能顺手改掉当前页显示的配置"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 真实配置回归：把用户本机的 opencode.json 分别写向三页，逐条检查落盘的
/// `model` 前缀是不是目标页认的。非用户机器（CI）跳过。
#[test]
fn the_real_opencode_agents_stay_valid_on_every_page() {
    let home = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let oc = home.join(".config/opencode/opencode.json");
    if !oc.exists() {
        return; // 非用户机器（CI）跳过
    }
    let dir = temp_dir("real");
    let mut app = App {
        config_path: oc.display().to_string(),
        ..App::default()
    };
    app.reload_for_page(ConfigFormat::Opencode, false);
    assert!(
        !app.agents.is_empty(),
        "真实 opencode.json 里应当有 agents 才能验证"
    );
    let configured: Vec<String> = app.providers.iter().map(|p| p.key.clone()).collect();

    for (fmt, file) in [
        (ConfigFormat::Opencode, "opencode.json"),
        (ConfigFormat::Kilocode, "kilo.json"),
        (ConfigFormat::Mimocode, "mimocode.json"),
    ] {
        let path = dir.join(file).display().to_string();
        app.save_backend_to(fmt, &path).expect("保存应当成功");
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let agents = root
            .get("agent")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("{} 应当写出 agent 容器", fmt.label()));
        for (key, value) in agents {
            let model = value.get("model").and_then(|v| v.as_str()).unwrap_or("");
            if model.trim().is_empty() {
                continue;
            }
            assert!(
                crate::opencode_models::model_is_valid_on(fmt, &configured, model),
                "{} 页写出的 {}.model = {:?} 在该页无效（网关不认这个前缀）",
                fmt.label(),
                key,
                model
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---- 切页往返：用户没改任何东西，配置不被改动 ----

/// 造一个只含 agents 的 App。
fn app_with(agents: Vec<AgentRow>, page: ConfigFormat) -> App {
    App {
        agents,
        source_format: page,
        current_page: page,
        ..App::default()
    }
}

/// 同文件重载（手动加载 / 预览应用）后，各页的 model 记忆保留。
#[test]
fn reloading_the_same_file_keeps_the_per_page_model_memory() {
    let dir = temp_dir("reload-keeps-memory");
    let path = dir.join("cfg.json");
    std::fs::write(
        &path,
        r#"{"provider": {}, "agent": {"fallback": {"model": "opencode/mimo-v2.6-flash-free"}}}"#,
    )
    .unwrap();
    let mut app = app_with(
        vec![agent("fallback", "opencode/mimo-v2.6-flash-free")],
        ConfigFormat::Opencode,
    );
    app.config_path = path.to_string_lossy().into_owned();
    app.loaded_path = app.config_path.clone();

    // 切到 mimocode：指向别家网关的引用被换成本页首选
    app.normalize_agent_models_for_page(ConfigFormat::Mimocode, Some(ConfigFormat::Opencode));
    assert_eq!(app.agents[0].model, "mimo/mimo-auto");

    // 同文件重载
    app.reload_for_page(ConfigFormat::Opencode, false);
    assert_eq!(app.agents[0].model, "opencode/mimo-v2.6-flash-free");

    // 切回 opencode：记忆仍在，还原用户原值
    app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Mimocode));
    assert_eq!(app.agents[0].model, "opencode/mimo-v2.6-flash-free");
}

/// opencode → kilo → opencode 走一圈，原来配好的 `opencode/…` 仍在。
#[test]
fn switching_pages_and_back_restores_the_original_model() {
    let mut app = app_with(
        vec![agent("fallback", "opencode/mimo-v2.6-flash-free")],
        ConfigFormat::Opencode,
    );
    // 切到 kilo 页：换成 kilo 自家网关模型
    app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
    app.current_page = ConfigFormat::Kilocode;
    assert_eq!(app.agents[0].model, "kilo/kilo-auto/free");
    // 切回 opencode 页：还原成用户原来配的那个
    app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
    app.current_page = ConfigFormat::Opencode;
    assert_eq!(
        app.agents[0].model, "opencode/mimo-v2.6-flash-free",
        "切回来必须还原用户原来配的值，不能变成默认模型"
    );
}

/// 三页各配各的，来回切各自保留自己的选择。
#[test]
fn each_page_remembers_its_own_model_choice() {
    let mut app = app_with(
        vec![agent("a", "opencode/big-pickle")],
        ConfigFormat::Opencode,
    );
    // opencode → kilo，用户在 kilo 页手动挑了另一个
    app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
    app.current_page = ConfigFormat::Kilocode;
    app.agents[0].model = "kilo/kilo-auto/balanced".into();
    // kilo → mimo（kilo 的选择被记住），用户在 mimo 页也挑了一个
    app.normalize_agent_models_for_page(ConfigFormat::Mimocode, Some(ConfigFormat::Kilocode));
    app.current_page = ConfigFormat::Mimocode;
    assert_eq!(app.agents[0].model, "mimo/mimo-auto");
    app.agents[0].model = "xiaomi/mimo-v2.6-pro".into();
    // mimo → kilo：回到用户在 kilo 页挑的那个
    app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Mimocode));
    app.current_page = ConfigFormat::Kilocode;
    assert_eq!(app.agents[0].model, "kilo/kilo-auto/balanced");
    // kilo → opencode：回到最初的
    app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
    app.current_page = ConfigFormat::Opencode;
    assert_eq!(app.agents[0].model, "opencode/big-pickle");
}

/// 记忆只在切页时建立：第一次进某页没有记忆，按无效引用替换。
#[test]
fn the_first_visit_to_a_page_has_no_memory_to_restore() {
    let mut app = app_with(
        vec![agent("a", "opencode/big-pickle")],
        ConfigFormat::Opencode,
    );
    // 直接进 mimo 页
    let replaced =
        app.normalize_agent_models_for_page(ConfigFormat::Mimocode, Some(ConfigFormat::Opencode));
    assert_eq!(replaced, 1);
    assert_eq!(app.agents[0].model, "mimo/mimo-auto");
}

/// 保存到某页时用的是该页记忆里的值，而不是当前页那份。
#[test]
fn saving_to_a_page_uses_that_pages_remembered_choice() {
    let dir = temp_dir("remember");
    let mut app = app_with(
        vec![agent("a", "opencode/big-pickle")],
        ConfigFormat::Opencode,
    );
    // 去 kilo 页挑一个非默认的模型，再回到 opencode 页
    app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
    app.current_page = ConfigFormat::Kilocode;
    app.agents[0].model = "kilo/kilo-auto/balanced".into();
    app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
    app.current_page = ConfigFormat::Opencode;

    let path = dir.join("kilo.json").display().to_string();
    app.save_backend_to(ConfigFormat::Kilocode, &path)
        .expect("保存应当成功");
    let root: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        root["agent"]["a"]["model"], "kilo/kilo-auto/balanced",
        "写 kilo 页必须用用户在 kilo 页挑的那个，而不是当前页的引用"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// 重新加载后：加载页的记忆重置为文件值，其他页的记忆保留。
#[test]
fn reloading_reseeds_loaded_page_and_keeps_other_pages_memory() {
    let dir = temp_dir("reload-reseed");
    let path = dir.join("cfg.json");
    std::fs::write(
        &path,
        r#"{"provider": {}, "agent": {"a": {"model": "opencode/big-pickle"}}}"#,
    )
    .unwrap();
    let mut app = app_with(
        vec![agent("a", "opencode/big-pickle")],
        ConfigFormat::Opencode,
    );
    app.config_path = path.to_string_lossy().into_owned();
    app.loaded_path = app.config_path.clone();

    // 去 kilo 页再回来，留下两页的记忆
    app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
    app.current_page = ConfigFormat::Kilocode;
    app.agents[0].model = "kilo/kilo-auto/balanced".into();
    app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
    app.current_page = ConfigFormat::Opencode;
    assert_eq!(app.agents[0].model, "opencode/big-pickle");

    // 用户在 opencode 页改了模型（未保存），然后同文件重载
    app.agents[0].model = "opencode/longcat-2.5-preview-free".into();
    app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
    app.current_page = ConfigFormat::Kilocode;
    app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
    app.current_page = ConfigFormat::Opencode;
    assert_eq!(app.agents[0].model, "opencode/longcat-2.5-preview-free");

    app.reload_for_page(ConfigFormat::Opencode, false);

    // 加载页记忆 = 文件值
    let op = (app.config_id(), ConfigFormat::Opencode);
    assert_eq!(
        app.agent_models_by_page[&op]["a"], "opencode/big-pickle",
        "重载后加载页的记忆应回到文件值"
    );
    // 其他页记忆保留
    let kilo = (app.config_id(), ConfigFormat::Kilocode);
    assert_eq!(
        app.agent_models_by_page[&kilo]["a"], "kilo/kilo-auto/balanced",
        "其他页的记忆不应被重载抹掉"
    );
    // 当前 agents 也回到文件值
    assert_eq!(app.agents[0].model, "opencode/big-pickle");
    std::fs::remove_dir_all(&dir).ok();
}
