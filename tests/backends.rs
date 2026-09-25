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

/// CLI 首次运行可能只生成 `.jsonc` 变体（`kilo.jsonc` / `mimocode.jsonc`），
/// 此时默认的 `.json` 并不存在——**仍须判为已安装**，且读写路径要指向真实的那个文件。
///
/// 否则页面显示「未安装」（用户实测报过），保存还会另建一个 `.json`，
/// 把用户真正的配置晾在一边。
#[test]
fn a_jsonc_only_install_still_counts_as_installed() {
    let root = std::env::temp_dir().join(format!("mh-jsonc-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let kilo_dir = root.join(".config").join("kilo");
    std::fs::create_dir_all(&kilo_dir).unwrap();
    let jsonc = kilo_dir.join("kilo.jsonc");
    std::fs::write(&jsonc, r#"{"$schema":"https://app.kilo.ai/config.json"}"#).unwrap();

    let b = backends::backend(ConfigFormat::Kilocode);
    // 默认名（.json）不存在，但等价候选 .jsonc 存在
    let default = kilo_dir.join("kilo.json").to_string_lossy().into_owned();
    assert!(
        b.local_available(&default),
        "只有 kilo.jsonc 时也应算已安装"
    );
    let resolved = backends::resolve_local_path(ConfigFormat::Kilocode, &default);
    assert_eq!(
        resolved,
        jsonc.to_string_lossy(),
        "读写路径应指向真实存在的 .jsonc"
    );

    // 两者都在时优先主名：不该悄悄改写到 .jsonc
    std::fs::write(kilo_dir.join("kilo.json"), "{}").unwrap();
    assert_eq!(
        backends::resolve_local_path(ConfigFormat::Kilocode, &default),
        default,
        "主名存在时应优先主名"
    );

    // 都不存在时回落到主名（新建场景要往主名写）
    std::fs::remove_file(&jsonc).unwrap();
    std::fs::remove_file(kilo_dir.join("kilo.json")).unwrap();
    assert_eq!(
        backends::resolve_local_path(ConfigFormat::Kilocode, &default),
        default
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 候选顺序：主名在前，`.jsonc` 变体在后；且只替换**文件名**那一段。
///
/// 父目录里恰好出现同名字串时不能被误替换（`.../kilo.json-backup/kilo.json`
/// 这种路径换错就写到别的目录去了）。
#[test]
fn jsonc_candidate_only_rewrites_the_file_name() {
    for (fmt, path, want_variant) in [
        (
            ConfigFormat::Kilocode,
            r"C:\Users\me\.config\kilo\kilo.json",
            r"C:\Users\me\.config\kilo\kilo.jsonc",
        ),
        (
            ConfigFormat::Mimocode,
            r"C:\Users\me\.config\mimocode\mimocode.json",
            r"C:\Users\me\.config\mimocode\mimocode.jsonc",
        ),
    ] {
        let b = backends::backend(fmt);
        let candidates = b.path_candidates(path);
        assert_eq!(candidates[0], path, "首个候选必须是主名");
        assert_eq!(candidates[1], want_variant, "{fmt:?} 的 .jsonc 变体不对");
    }
    // 父目录里出现同名子串：只动文件名
    let b = backends::backend(ConfigFormat::Kilocode);
    let odd = r"C:\kilo.json-backup\kilo.json";
    assert_eq!(
        b.path_candidates(odd)[1],
        r"C:\kilo.json-backup\kilo.jsonc",
        "不得替换父目录里的同名字串"
    );
    // 幂等：把解析结果再喂回去，候选不能滚出 `.jsonc.jsonc`
    for (fmt, p) in [
        (
            ConfigFormat::Kilocode,
            r"C:\Users\me\.config\kilo\kilo.jsonc",
        ),
        (
            ConfigFormat::Mimocode,
            "/home/me/.config/mimocode/mimocode.jsonc",
        ),
    ] {
        assert_eq!(
            backends::backend(fmt).path_candidates(p).len(),
            1,
            "已是 .jsonc 时不应再追加变体：{p}"
        );
    }
}

/// 非 opencode 系后端只有一条候选（默认实现），不会被这次改动波及。
#[test]
fn other_backends_have_a_single_candidate() {
    for fmt in [
        ConfigFormat::Pi,
        ConfigFormat::OhMyPi,
        ConfigFormat::DeepSeekHarness,
        ConfigFormat::ZCode,
        ConfigFormat::WorkBuddy,
    ] {
        let b = backends::backend(fmt);
        let p = b.default_local_path();
        assert_eq!(
            b.path_candidates(&p),
            vec![p.clone()],
            "{fmt:?} 不应有额外候选"
        );
    }
}

/// 新建 / 合并到不存在的目标时也要写出 `$schema`：CLI 自己生成的配置就带它，
/// 缺了编辑器与 CLI 都拿不到字段补全（用户实测报过「预览里没有 $schema」）。
#[test]
fn serialize_always_writes_the_schema_url() {
    let agents = vec![oc_agent("build")];
    let providers = vec![pi_provider("openai_x", "gpt-5.6-sol")];
    for (fmt, want) in [
        (ConfigFormat::Opencode, "https://opencode.ai/config.json"),
        (ConfigFormat::Kilocode, "https://app.kilo.ai/config.json"),
        (
            ConfigFormat::Mimocode,
            "https://mimo.xiaomi.com/mimocode/config.json",
        ),
    ] {
        let b = backends::backend(fmt);
        // 目标不存在（空对象）→ 必须补上
        let fresh = b.serialize_root(&agents, &providers, &json!({}), Some(&json!({})));
        assert_eq!(
            fresh.get("$schema").and_then(Value::as_str),
            Some(want),
            "{fmt:?} 新建时未写 $schema"
        );
        // 当前文件保存（extras 里没有 $schema）→ 同样补上
        let current = b.serialize_root(&agents, &providers, &json!({"mcp":{}}), None);
        assert_eq!(
            current.get("$schema").and_then(Value::as_str),
            Some(want),
            "{fmt:?} 当前文件保存时未写 $schema"
        );
        // 顺序：$schema 在首位（CLI 生成的文件就是这么排的）
        assert_eq!(
            current
                .as_object()
                .and_then(|o| o.keys().next())
                .map(String::as_str),
            Some("$schema"),
            "{fmt:?} 的 $schema 应排在首位"
        );
    }
}

/// 已有的 `$schema` 一律保留：用户可能改成别的地址，覆盖等于替用户改配置。
#[test]
fn an_existing_schema_url_is_preserved() {
    let agents = vec![oc_agent("build")];
    let providers = vec![pi_provider("openai_x", "gpt-5.6-sol")];
    let custom = "https://my-mirror.example/kilo.schema.json";
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let b = backends::backend(fmt);
        let existing = json!({ "$schema": custom, "theme": "x" });
        let merged = b.serialize_root(&agents, &providers, &json!({}), Some(&existing));
        assert_eq!(
            merged.get("$schema").and_then(Value::as_str),
            Some(custom),
            "{fmt:?} 覆盖了用户自定义的 $schema"
        );
        assert_eq!(
            merged.get("theme").and_then(Value::as_str),
            Some("x"),
            "{fmt:?} 丢了目标文件的其他顶层字段"
        );
        // 当前文件保存（extras 自带 $schema）同样不能改写
        let extras = json!({ "$schema": custom });
        let kept = b.serialize_root(&agents, &providers, &extras, None);
        assert_eq!(
            kept.get("$schema").and_then(Value::as_str),
            Some(custom),
            "{fmt:?} 覆盖了当前文件里的 $schema"
        );
    }
}

/// 构造一个「已配置 provider 的模型」root，模型体由调用方给定。
fn root_with_model(model: Value) -> Value {
    json!({ "provider": { "p": { "models": { "m1": model } } } })
}

/// 从序列化结果里取 `provider.p.models.m1`。
fn serialized_model(fmt: ConfigFormat, target: &Value, extras: &Value) -> Value {
    let b = backends::backend(fmt);
    let root = b.serialize_root(&[], &[], extras, Some(target));
    root["provider"]["p"]["models"]["m1"].clone()
}

/// **核心回归**：mimocode 要求 `modalities` 的 `input` 与 `output` 成对，opencode 不要求。
///
/// 用户实测：opencode 里几个只写了 `modalities.output` 的模型，切到 mimo 页保存后
/// `mimo` 直接拒绝加载（`expected array, received undefined … modalities.input`）。
/// 补齐后同一份配置在三页都能被各自的 CLI 接受。
#[test]
fn mimocode_gets_the_missing_modality_side_filled_in() {
    let target = root_with_model(json!({ "modalities": { "output": ["text"] } }));
    let got = serialized_model(ConfigFormat::Mimocode, &target, &json!({}));
    assert_eq!(
        got["modalities"]["input"],
        json!(["text"]),
        "mimocode 缺的 input 必须补成 text（源方言的语义就是纯文本）"
    );
    assert_eq!(
        got["modalities"]["output"],
        json!(["text"]),
        "用户明确写出的 output 不能被改动"
    );
    // 反向：只写 input 时补 output
    let target = root_with_model(json!({ "modalities": { "input": ["text", "image"] } }));
    let got = serialized_model(ConfigFormat::Mimocode, &target, &json!({}));
    assert_eq!(got["modalities"]["output"], json!(["text"]));
    assert_eq!(
        got["modalities"]["input"],
        json!(["text", "image"]),
        "原有的多模态输入必须原样保留"
    );
}

/// opencode / kilocode 不要求成对：**绝不能**凭空加字段，否则就是改用户的配置。
///
/// 这条测试防的是「为了修 mimo 而把三家一起改了」——那会把用户 opencode 配置里
/// 本来合法的半截 modalities 补成他没写过的内容。
#[test]
fn the_other_flavors_are_not_touched() {
    for fmt in [ConfigFormat::Opencode, ConfigFormat::Kilocode] {
        let target = root_with_model(json!({ "modalities": { "output": ["text"] } }));
        let got = serialized_model(fmt, &target, &json!({}));
        assert_eq!(
            got["modalities"],
            json!({ "output": ["text"] }),
            "{fmt:?} 不该补 modalities.input（它的 schema 不要求）"
        );
    }
}

/// 两侧都齐全时一个字节都不动（常规路径不能受影响）。
#[test]
fn complete_modalities_are_left_alone() {
    let model = json!({ "modalities": { "input": ["text"], "output": ["text", "image"] } });
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let got = serialized_model(fmt, &root_with_model(model.clone()), &json!({}));
        assert_eq!(
            got["modalities"], model["modalities"],
            "{fmt:?} 改动了完整模态"
        );
    }
}

/// 空的 `modalities` 对象整块删掉：mimo 对 `{}` 会同时报缺 input 与 output，
/// 而「没有 modalities 键」是合法的（CLI 按纯文本处理）。
#[test]
fn an_empty_modalities_object_is_removed_for_mimocode() {
    let target = root_with_model(json!({ "modalities": {} }));
    let got = serialized_model(ConfigFormat::Mimocode, &target, &json!({}));
    assert!(
        got.get("modalities").is_none(),
        "空 modalities 必须整块移除，否则 mimo 会同时报两个缺失: {got}"
    );
}

/// `limit` 的 context / output 三家 CLI 都要求成对（实测半截会被拒），
/// 而半截 limit **无法**用合法值表达（schema 要数字，「不限」没有对应值），
/// 所以整块删掉，交给 CLI 用它自己的模型库。
#[test]
fn a_half_filled_limit_is_dropped_for_every_flavor() {
    for fmt in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let target = root_with_model(json!({ "limit": { "context": 128000 } }));
        let got = serialized_model(fmt, &target, &json!({}));
        assert!(
            got.get("limit").is_none(),
            "{fmt:?} 应整块删掉半截 limit（省略 limit 三家都合法），而不是编造一个数字: {got}"
        );
        // 成对的 limit 必须原样保留
        let target = root_with_model(json!({ "limit": { "context": 128000, "output": 8192 } }));
        let got = serialized_model(fmt, &target, &json!({}));
        assert_eq!(
            got["limit"],
            json!({ "context": 128000, "output": 8192 }),
            "{fmt:?} 改动了完整的 limit"
        );
    }
}

/// 目标文件里**界面没接管的**旧条目也要补齐：它们同样要过 CLI 的校验。
#[test]
fn untouched_models_in_the_target_file_are_completed_too() {
    // 目标文件里已有一个 mimo 无法加载的模型（只写了 output）。
    let target = root_with_model(json!({ "modalities": { "output": ["text"] } }));
    let b = backends::backend(ConfigFormat::Mimocode);
    let root = b.serialize_root(&[], &[], &json!({}), Some(&target));
    assert_eq!(
        root["provider"]["p"]["models"]["m1"]["modalities"]["input"],
        json!(["text"]),
        "合并写入时目标文件里保留下来的模型也必须补齐"
    );
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
///
/// **唯一允许的差异是 `$schema` 地址**——三者各有自己的官方 schema（`app.kilo.ai`
/// / `mimo.xiaomi.com`），那正是它们被区分开的标记之一。比较前把它摘掉，
/// 剩下的部分必须逐字相同；否则就是某个成员悄悄长出了自己的字段口径。
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
        assert!(root.get("$schema").is_some(), "{fmt:?} 未写 $schema");
        // 摘掉 $schema 再比：其余字段必须逐字一致。
        let mut without_schema = root.clone();
        without_schema
            .as_object_mut()
            .unwrap()
            .shift_remove("$schema");
        roots.push(serde_json::to_string(&without_schema).unwrap());
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
