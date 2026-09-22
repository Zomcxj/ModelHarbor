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

/// opencode 系（opencode / kilocode / mimocode）三者内容形状相同，判别只能靠路径。
///
/// Kilo Code 与 MiMo Code 都是 opencode 的 fork，配置 schema 逐字相同，所以任何
/// 基于内容的判别都无法区分它们——只有配置目录名 / 主配置文件名能区分。
#[test]
fn opencode_family_members_are_told_apart_by_path() {
    let body = r#"{"provider":{"p":{"options":{"baseURL":"https://x.example/v1"}}}}"#;
    // 同一份内容：路径决定归属。
    for (path, want) in [
        (
            r"C:\Users\me\.config\opencode\opencode.json",
            ConfigFormat::Opencode,
        ),
        (
            r"C:\Users\me\.config\kilo\kilo.json",
            ConfigFormat::Kilocode,
        ),
        (
            r"C:\Users\me\.config\mimocode\mimocode.json",
            ConfigFormat::Mimocode,
        ),
        ("/home/me/.config/kilo/kilo.jsonc", ConfigFormat::Kilocode),
        (
            "/home/me/.config/mimocode/mimocode.jsonc",
            ConfigFormat::Mimocode,
        ),
    ] {
        assert_eq!(
            backends::detect_format(body, path),
            want,
            "路径 {path} 应判为 {want:?}"
        );
    }
    // 无路径线索时回落 opencode 本家（它是这一族的代表与回落项）。
    assert_eq!(backends::detect_format(body, ""), ConfigFormat::Opencode);
}

/// Kilo 会读同目录下遗留的 `~/.config/kilo/opencode.json`：文件名像 opencode，
/// 但**目录名**已经把这份文件判给了 kilocode，内容兜底不得再抢。
///
/// 这是路径优先于内容的必要结果——否则注册顺序在前的 opencode 会把它抢走，
/// 页面与保存路径都会指向 `~/.config/opencode/opencode.json`，写错门。
#[test]
fn a_siblings_directory_name_beats_a_matching_filename() {
    let body = r#"{"provider":{"p":{"options":{"baseURL":"https://x.example/v1"}}}}"#;
    for path in [
        r"C:\Users\me\.config\kilo\opencode.json",
        "/home/me/.config/kilo/opencode.jsonc",
    ] {
        assert_eq!(
            backends::detect_format(body, path),
            ConfigFormat::Kilocode,
            "kilo 目录下的 opencode.json 属于 kilocode：{path}"
        );
    }
    // 反过来同样成立：目录名永远优先于文件名。
    assert_eq!(
        backends::detect_format(body, r"C:\Users\me\.config\opencode\kilo.json"),
        ConfigFormat::Opencode,
        "opencode 目录下的 kilo.json 属于 opencode"
    );
}

/// 三者写盘口径完全一致：同一份界面状态序列化出的 provider / agent 字段相同。
#[test]
fn opencode_family_serializes_identically() {
    let providers = vec![pi_provider("openai_x", "gpt-5.6-sol")];
    let agents = vec![oc_agent("build")];
    let mut roots = Vec::new();
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let b = backends::backend(fmt);
        let extras = json!({ "mcp": { "x": { "type": "local" } } });
        let root = b.serialize_root(&agents, &providers, &extras, None);
        // 顶层未知字段必须保留，provider / agent 必须写进去。
        assert!(root.get("mcp").is_some(), "{fmt:?} 丢了顶层字段");
        assert!(root.get("provider").is_some(), "{fmt:?} 未写 provider");
        assert!(root.get("agent").is_some(), "{fmt:?} 未写 agent");
        roots.push(serde_json::to_string(&root).unwrap());
    }
    assert_eq!(roots[0], roots[1], "opencode 与 kilocode 写盘应一致");
    assert_eq!(roots[1], roots[2], "kilocode 与 mimocode 写盘应一致");
}

/// 三个成员的默认路径与图标各自独立，不能串。
///
/// 图标这里必须比**视觉**差异，不能只比字节：MiMo Code 是 opencode 的 fork，官方
/// favicon / 桌面应用图标 / console logo 全都沿用 opencode 的同一份图形，所以照抄
/// 官方资产会得到一个"字节不同、看起来一模一样"的图标——页签上根本分不出是哪一页。
/// 只断言 `assert_ne!` 恰好放过这种情形（旧图标与 opencode 有 13% 像素不同，全是抗锯齿
/// 差异）。这里改成量 RGB 差异像素占比，阈值取 25%：同图形换抗锯齿达不到，换图形能过。
#[test]
fn opencode_family_members_have_distinct_paths_and_icons() {
    let paths: Vec<String> = [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ]
    .iter()
    .map(|f| ConfigPaths::default_local_path(*f))
    .collect();
    assert!(paths[0].contains("opencode"), "{:?}", paths[0]);
    assert!(paths[1].contains("kilo"), "{:?}", paths[1]);
    assert!(paths[2].contains("mimocode"), "{:?}", paths[2]);
    assert_ne!(paths[0], paths[1]);
    assert_ne!(paths[1], paths[2]);

    let formats = [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ];
    let icons: Vec<&[u8]> = formats
        .iter()
        .map(|f| backends::backend(*f).icon_rgba().expect("有图标").0)
        .collect();
    for icon in &icons {
        assert_eq!(icon.len(), 32 * 32 * 4, "图标必须是 32×32 RGBA");
    }
    for (a, b) in [(0, 1), (0, 2), (1, 2)] {
        let d = differing_pixel_fraction(icons[a], icons[b]);
        assert!(
            d > 0.25,
            "{:?} 与 {:?} 的图标视觉上几乎相同（差异像素仅 {:.1}%），页签上分不出来",
            formats[a],
            formats[b],
            d * 100.0
        );
    }
}

/// 两个 32×32 RGBA 图标里「肉眼可辨不同」的像素占比。
///
/// 单通道差之和超过 30 才算不同，以滤掉抗锯齿与缩放的细微偏差；完全一致返回 0.0。
fn differing_pixel_fraction(a: &[u8], b: &[u8]) -> f32 {
    assert_eq!(a.len(), b.len());
    let mut differing = 0usize;
    let mut total = 0usize;
    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        total += 1;
        let delta: i32 = pa[..3]
            .iter()
            .zip(&pb[..3])
            .map(|(x, y)| (*x as i32 - *y as i32).abs())
            .sum();
        if delta > 30 {
            differing += 1;
        }
    }
    differing as f32 / total as f32
}
