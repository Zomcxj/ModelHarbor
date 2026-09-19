//! 后端注册表架构回归：detect / parse / serialize_root 全管线。

use model_harbor::backends;
use model_harbor::format::{ConfigFormat, ConfigPaths};
use model_harbor::model::{AgentRow, ProviderRow};
use serde_json::{json, Value};

fn oc_agent(key: &str) -> AgentRow {
    let mut a = AgentRow::new();
    a.key = key.to_string();
    a.model = "p/m".into();
    a
}

fn pi_provider(key: &str, model_id: &str) -> ProviderRow {
    use model_harbor::model::ModelRow;
    let mut p = ProviderRow::new();
    p.key = key.to_string();
    p.base_url = "https://api.example.com/v1".into();
    p.api_key = "sk-test".into();
    p.npm = "@ai-sdk/openai-compatible".into();
    let mut m = ModelRow::new();
    m.id = model_id.to_string();
    m.name = model_id.to_string();
    m.reasoning = true;
    m.context = "128000".into();
    m.output = "8192".into();
    m.modalities_input = "text".into();
    p.models.push(m);
    p
}

#[test]
fn backend_registry_covers_all_formats() {
    assert_eq!(
        backends::backend(ConfigFormat::Opencode).id(),
        ConfigFormat::Opencode
    );
    assert_eq!(backends::backend(ConfigFormat::Pi).id(), ConfigFormat::Pi);
    // 注册表顺序：第 0 个为判别回落项
    assert_eq!(backends::BACKENDS[0].id(), ConfigFormat::Opencode);
}

/// 每个后端都要有 32×32 图标：标签栏按索引取用，缺一个就是一个空按钮。
///
/// 图标是嵌进二进制的原始 RGBA 字节（未预乘），尺寸对不上会在
/// `ColorImage::from_rgba_unmultiplied` 处 panic 或画成花屏，所以钉住长度。
#[test]
fn every_backend_ships_a_32x32_icon() {
    for backend in backends::BACKENDS {
        let (rgba, w, h) = backend
            .icon_rgba()
            .unwrap_or_else(|| panic!("后端 {:?} 缺少图标", backend.id()));
        assert_eq!((w, h), (32, 32), "后端 {:?} 的图标尺寸不对", backend.id());
        assert_eq!(
            rgba.len(),
            (w * h * 4) as usize,
            "后端 {:?} 的图标字节数不对（应为未预乘 RGBA）",
            backend.id()
        );
        // 全透明等于没画：抽几个点确认确实有内容。
        let (pixels, _) = rgba.as_chunks::<4>();
        assert!(
            pixels.iter().any(|px| px[3] > 0),
            "后端 {:?} 的图标全透明",
            backend.id()
        );
    }
}

#[test]
fn detect_format_distinguishes_backends() {
    assert_eq!(
        backends::detect_format("{\"providers\": {}}", ""),
        ConfigFormat::Pi
    );
    assert_eq!(
        backends::detect_format("{\"provider\": {}}", ""),
        ConfigFormat::Opencode
    );
    assert_eq!(backends::detect_format("{}", ""), ConfigFormat::Opencode);
    // pi 判定优先于 opencode：双键并存时 opencode 胜出（pi.detect 要求无 provider 键）
    assert_eq!(
        backends::detect_format("{\"providers\": {}, \"provider\": {}}", ""),
        ConfigFormat::Opencode
    );
}

#[test]
fn opencode_parse_and_current_file_save() {
    let content = r#"{
        "mcp": { "server": { "command": "node" } },
        "agent": { "old": { "mode": "subagent", "model": "x/y" } },
        "provider": { "oldp": { "npm": "@ai-sdk/x", "models": {} } }
    }"#;
    let b = backends::backend(ConfigFormat::Opencode);
    let load = b.parse(content).expect("解析失败");
    assert_eq!(load.agents.len(), 1);
    assert_eq!(load.providers.len(), 1);
    assert!(load.root.get("mcp").is_some());
    // opencode extras = 整个 root
    assert!(load.extras.get("mcp").is_some());

    // 当前文件保存：整体替换 agent/provider（删除即生效），保留 mcp
    let agents = vec![oc_agent("writer")];
    let providers = vec![pi_provider("newp", "m1")];
    let root = b.serialize_root(&agents, &providers, &load.extras, None);
    assert!(
        root["agent"].get("old").is_none(),
        "UI 中已删除的 agent 不得复活"
    );
    assert!(root["agent"]["writer"].is_object());
    assert!(root["provider"].get("oldp").is_none());
    assert!(root["provider"]["newp"].is_object());
    assert!(root["mcp"]["server"].is_object(), "未知顶层字段必须保留");
}

#[test]
fn pi_parse_and_current_file_save() {
    let content = r#"{
        "providers": { "p1": { "baseUrl": "https://x/v1", "api": "openai-completions", "apiKey": "k", "models": [] } },
        "extraTop": { "kept": true }
    }"#;
    let b = backends::backend(ConfigFormat::Pi);
    let load = b.parse(content).expect("解析失败");
    assert_eq!(load.providers.len(), 1);
    assert!(load.agents.is_empty());
    // pi extras = providers 之外的顶层字段
    assert!(load.extras.get("extraTop").is_some());
    assert!(load.extras.get("providers").is_none());

    let providers = {
        let mut v = load.providers.clone();
        v.push(pi_provider("p2", "m2"));
        v
    };
    let root = b.serialize_root(&[], &providers, &load.extras, None);
    assert!(root["providers"]["p1"].is_object());
    assert!(root["providers"]["p2"].is_object());
    assert!(root["extraTop"].is_object(), "extras 必须保留");
}

#[test]
fn opencode_cross_target_merge_upserts() {
    let b = backends::backend(ConfigFormat::Opencode);
    let target = json!({
        "agent": { "writer": { "mode": "subagent", "model": "p/m" } },
        "provider": { "old": { "npm": "@ai-sdk/x", "models": {} } },
        "mcp": { "server": { "command": "node" } }
    });
    let providers = vec![pi_provider("newp", "m1")];
    let root = b.serialize_root(&[], &providers, &Value::Null, Some(&target));
    // 目标已有条目保留，UI 条目 upsert
    assert!(root["agent"]["writer"].is_object());
    assert!(root["provider"]["old"].is_object());
    assert!(root["provider"]["newp"].is_object());
    assert!(root["mcp"]["server"].is_object());
}

#[test]
fn pi_cross_target_uses_target_extras() {
    let b = backends::backend(ConfigFormat::Pi);
    // 目标文件自身的顶层字段应被采用，而非来源 extras
    let target_extras = json!({ "targetExtra": 1 });
    let providers = vec![pi_provider("p2", "m2")];
    let root = b.serialize_root(&[], &providers, &Value::Null, Some(&target_extras));
    assert_eq!(root["targetExtra"], json!(1));
    assert!(root["providers"]["p2"].is_object());
}

#[test]
fn opencode_current_save_omits_empty_sections() {
    // 空 agents / providers 列表不得写入 "agent": {} / "provider": {}
    let b = backends::backend(ConfigFormat::Opencode);
    let load = b.parse(r#"{ "mcp": { "s": {} } }"#).expect("解析失败");
    let root = b.serialize_root(&[], &[], &load.extras, None);
    assert!(
        root.get("agent").is_none(),
        "empty agent map must be omitted"
    );
    assert!(
        root.get("provider").is_none(),
        "empty provider map must be omitted"
    );
    assert!(root["mcp"].is_object(), "未知顶层字段必须保留");
}

#[test]
fn detect_empty_yml_as_oh_my_pi() {
    // 空内容新建场景：.yml 扩展名归 oh-my-pi，其余回落 opencode
    assert_eq!(
        backends::detect_format("", "models.yml"),
        ConfigFormat::OhMyPi
    );
    assert_eq!(
        backends::detect_format("", "models.yaml"),
        ConfigFormat::OhMyPi
    );
    assert_eq!(
        backends::detect_format("", "models.json"),
        ConfigFormat::Opencode
    );
    assert_eq!(backends::detect_format("", ""), ConfigFormat::Opencode);
}

#[test]
fn detect_for_path_uses_extension_for_new_files() {
    // 路径指向不存在的文件（新建场景）时按扩展名推断格式
    let (fmt, _) = ConfigPaths::detect_for_path(r"C:\nonexistent_dir_zz\models.yml");
    assert_eq!(fmt, ConfigFormat::OhMyPi);
    let (fmt, _) = ConfigPaths::detect_for_path(r"C:\nonexistent_dir_zz\opencode.json");
    assert_eq!(fmt, ConfigFormat::Opencode);
}

#[test]
fn load_backend_via_generic_pipeline() {
    // 通用加载管线：临时文件 + opencode 后端
    let mut p = std::env::temp_dir();
    p.push("backends_pipeline_test.json");
    std::fs::write(&p, "{\"provider\": {\"x\": {}}}").unwrap();
    let load =
        backends::load_backend(ConfigFormat::Opencode, p.to_str().unwrap()).expect("通用加载失败");
    assert_eq!(load.providers.len(), 1);
    std::fs::remove_file(&p).ok();
}

#[test]
fn write_config_creates_parent_dirs() {
    let mut p = std::env::temp_dir();
    p.push(format!("backends_write_{}", std::process::id()));
    p.push("nested");
    p.push("cfg.json");
    backends::write_config(p.to_str().unwrap(), "{}").expect("写入失败");
    assert!(p.exists());
    std::fs::remove_file(&p).ok();
    p.parent().map(|d| std::fs::remove_dir(d).ok());
}
