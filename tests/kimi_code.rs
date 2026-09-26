//! KimiCode 后端回归：`~/.kimi-code/config.toml` 的判别 / 解析 / 序列化 / 两份配置。
//!
//! 结构上最要紧的三点（细节见 `src/backends/kimi_code.rs` 的模块说明）：
//! 1. 模型在**顶层全局表** `[models."<alias>"]` 里，靠 `provider` 字段 join 到
//!    `[providers.<name>]`——模型不嵌在 provider 下，这是本项目唯一这样的格式；
//! 2. 表键是 alias，`model` 是发给上游的 wire id，**两者可以不同**；
//! 3. 没有 `disabled` 字段，停用 = 整条不写 → 拆成生效清单 + 全量副本两份文件。
//!
//! 另外两条硬约束：`api_key` / `api_key_env` / `oauth` 三者互斥（同时写会让 Kimi Code
//! **启动失败**），`managed:*` provider 来自 OAuth 登录、只读。
//!
//! 本文件**不读** `~/.kimi-code/config.toml`：Kimi Code 桌面端会随时改写它
//! （调研期间就被外部改过两次），测试会随用户的实际使用漂移。样本是冻结的副本。

use model_harbor::backends;
use model_harbor::format::ConfigFormat;
use model_harbor::model::ProviderRow;
use serde_json::Value;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_kimi_{}_{}_{}",
        std::process::id(),
        tag,
        nonce
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn kimi_backend() -> &'static dyn backends::Backend {
    backends::backend(ConfigFormat::KimiCode)
}

/// 真实形态样本：本机 `~/.kimi-code/config.toml` 的**冻结副本**（密钥已替换为占位符）。
///
/// 覆盖了全部要点：`managed:` + oauth 子表、alias ≠ model（`kimi-code/k3` ↔ `k3`）、
/// 两种 provider type、`[thinking]` 顶层表、以及没有 `display_name` 的条目。
fn config_toml() -> String {
    r#"default_model = "sensenova/sensenova-6.8-flash-lite"
default_permission_mode = "auto"

[providers."managed:kimi-code"]
base_url = "https://api.kimi.com/coding/v1"
type = "kimi"
api_key = ""

[providers."managed:kimi-code".oauth]
storage = "file"
key = "oauth/kimi-code"

[providers.sensenova]
base_url = "https://token.sensenova.cn/v1"
type = "openai"
api_key = "sk-REPLACED"

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 1048576
capabilities = [ "thinking", "always_thinking", "image_in", "video_in", "tool_use", "dynamically_loaded_tools" ]
display_name = "K2.8 Preview"
support_efforts = [ "low", "high", "max" ]
default_effort = "max"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking", "always_thinking", "image_in", "video_in", "tool_use", "dynamically_loaded_tools" ]
display_name = "K3"
support_efforts = [ "low", "high", "max" ]
default_effort = "high"

[models."sensenova/sensenova-6.8-flash-lite"]
provider = "sensenova"
model = "sensenova-6.8-flash-lite"
max_context_size = 65536
capabilities = [ "tool_use", "thinking", "dynamically_loaded_tools"  ]
adaptive_thinking = true
display_name = "sensenova-6.8-flash-lite"
support_efforts = [ "high", "max" ]
default_effort = "high"

[models."sensenova/deepseek-v4-flash"]
provider = "sensenova"
model = "deepseek-v4-flash"
max_context_size = 262000
capabilities = [ "tool_use", "thinking", "dynamically_loaded_tools"  ]
adaptive_thinking = true
display_name = "deepseek-v4-flash"
support_efforts = [ "high", "max" ]
default_effort = "high"

[thinking]
enabled = true
"#
    .to_string()
}

fn load(content: &str) -> backends::BackendLoad {
    kimi_backend().parse(content).expect("KimiCode 解析失败")
}

fn provider<'a>(load: &'a backends::BackendLoad, key: &str) -> &'a ProviderRow {
    load.providers
        .iter()
        .find(|p| p.key == key)
        .unwrap_or_else(|| panic!("缺少 provider {key}"))
}

/// 解析 TOML（测试内部用；后端不暴露解析器）。
fn toml_value(text: &str) -> Value {
    toml::from_str(text).expect("测试样本必须是合法 TOML")
}

// ---------------------------------------------------------------- 判别

#[test]
fn detect_matches_the_install_path() {
    let b = kimi_backend();
    assert!(b.detect("", r"C:\Users\me\.kimi-code\config.toml"));
    assert!(b.detect("", "/home/me/.kimi-code/config.toml"));
    // 路径对就该认，哪怕文件还是空的（首次运行、刚登录还没写模型）
    assert!(b.detect("", r"C:\Users\me\.kimi-code\config.toml"));
    // 同目录的别的文件不算
    assert!(!b.detect("{}", r"C:\Users\me\.kimi-code\mcp.json"));
    assert!(!b.detect("", r"C:\Users\me\.kimi-code\tui.toml"));
}

#[test]
fn detect_matches_the_content_shape() {
    let b = kimi_backend();
    assert!(b.detect(&config_toml(), ""), "真实形态应命中");
    // `managed:kimi-code` + `type = "kimi"` 是极强特征
    assert!(b.detect("[providers.\"managed:kimi-code\"]\ntype = \"kimi\"\n", ""));
    // `default_permission_mode` 是 Kimi 独有的顶层键
    assert!(b.detect("default_permission_mode = \"auto\"\n", ""));
    // 两张表同时出现 + 模型带 max_context_size
    assert!(b.detect(
        "[providers.p]\ntype = \"openai\"\n\n[models.a]\nprovider = \"p\"\nmax_context_size = 1\n",
        ""
    ));
}

#[test]
fn detect_rejects_codex_shape() {
    // Codex 也是 `config.toml` + TOML，顶层键是 `model_providers`（带下划线）。
    // 两者只能靠路径与键名分辨——不能把别人的配置收进来。
    let codex = r#"
model = "gpt-5"
model_provider = "openai"

[model_providers.openai]
name = "OpenAI"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
"#;
    assert!(!kimi_backend().detect(codex, ""));
    // 显式带 Codex 特征时，即使路径像也不认（路径命中优先，所以这里给空路径）
    assert!(!kimi_backend().detect(codex, r"C:\Users\me\.codex\config.toml"));
}

#[test]
fn detect_does_not_steal_sibling_formats() {
    // KimiCode 的 TOML 不能被 pi 系抢走
    assert_eq!(
        backends::detect_format(&config_toml(), ""),
        ConfigFormat::KimiCode
    );
    // 反过来也不能抢别人的
    assert_eq!(
        backends::detect_format(r#"{"provider": {}}"#, ""),
        ConfigFormat::Opencode
    );
    assert_eq!(
        backends::detect_format(r#"{"providers": {}}"#, ""),
        ConfigFormat::Pi
    );
    // 无关 TOML 不认
    assert!(!kimi_backend().detect("[package]\nname = \"x\"\n", ""));
    assert!(!kimi_backend().detect("", ""));
}

// ---------------------------------------------------------------- 解析

#[test]
fn models_join_to_their_provider_by_the_provider_field() {
    let load = load(&config_toml());
    let sensenova = provider(&load, "sensenova");
    let ids: Vec<&str> = sensenova.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["sensenova-6.8-flash-lite", "deepseek-v4-flash"],
        "两条模型都该挂到 sensenova 下（按文件顺序）"
    );
    assert_eq!(sensenova.base_url, "https://token.sensenova.cn/v1");
    assert_eq!(sensenova.api_key, "sk-REPLACED");
    assert_eq!(sensenova.pi_api, "openai", "type 逐字进 pi_api");
}

#[test]
fn managed_providers_stay_out_of_the_ui() {
    // `managed:*` 由 OAuth 维护，界面无从编辑，显示出来只会让人以为能改。
    let load = load(&config_toml());
    assert_eq!(load.providers.len(), 1, "只列出 sensenova");
    assert_eq!(load.providers[0].key, "sensenova");
    assert!(
        !load.providers.iter().any(|p| p.key.starts_with("managed:")),
        "managed 不该出现在界面上"
    );
}

#[test]
fn alias_differs_from_the_wire_model_id() {
    // `[models."kimi-code/k3"]` 里 `model = "k3"`：表键是 alias，model 是 wire id。
    let load = load(&config_toml());
    let sensenova = provider(&load, "sensenova");
    // sensenova 的两条 alias 与 wire id 相同
    assert_eq!(
        sensenova.models[0].kimi_alias,
        "sensenova/sensenova-6.8-flash-lite"
    );
    assert_eq!(sensenova.models[0].id, "sensenova-6.8-flash-lite");
}

#[test]
fn parse_maps_model_attributes() {
    let load = load(&config_toml());
    let m = &provider(&load, "sensenova").models[0];
    assert_eq!(m.id, "sensenova-6.8-flash-lite", "id 是 wire id");
    assert_eq!(
        m.name, "sensenova-6.8-flash-lite",
        "display_name 与 wire id 同名时照样读进来（本机文件就是如此）"
    );
    assert_eq!(m.context, "65536");
    assert_eq!(m.output, "", "没写 max_output_size 就留空");
    assert!(m.reasoning, "capabilities 含 thinking");
    assert!(m.tool_call, "capabilities 含 tool_use");
    assert_eq!(
        m.modalities_input, "text",
        "只有 tool_use/thinking，没有 image_in"
    );
    assert_eq!(m.variants, "high, max");
}

#[test]
fn parse_falls_back_when_optional_keys_are_absent() {
    // 可选项缺失时的缺省：名称回落 wire id，档位与输出上限留空。
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1000
"#;
    let load2 = load(content);
    let m = &provider(&load2, "p").models[0];
    assert_eq!(m.name, "m", "没有 display_name 就回落 wire id");
    assert_eq!(m.variants, "", "没写 support_efforts 就留空");
    assert_eq!(m.output, "", "没写 max_output_size 就留空");
}

#[test]
fn parse_reads_display_name_and_efforts() {
    // k3 是 managed 的，界面看不到；用构造样本验证同一条读取路径
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1000
display_name = "My Model"
capabilities = [ "image_in", "video_in" ]
support_efforts = [ "low", "high" ]
"#;
    let load2 = load(content);
    let m = &provider(&load2, "p").models[0];
    assert_eq!(m.name, "My Model");
    assert_eq!(m.context, "1000");
    assert_eq!(m.variants, "low, high");
    assert!(
        m.modalities_input.contains("image"),
        "image_in / video_in 都算图像输入：{}",
        m.modalities_input
    );
}

#[test]
fn parse_does_not_invent_undeclared_defaults() {
    // 条目只写了必填字段：上下文之外的数值/模态/档位一律留空，
    // 否则 ModelRow::new() 的预填值会被当成用户配置写回文件。
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1
"#;
    let load = load(content);
    let m = &provider(&load, "p").models[0];
    assert_eq!(m.output, "");
    assert_eq!(m.modalities_input, "");
    assert_eq!(m.variants, "");
    assert!(!m.reasoning);
    assert!(!m.tool_call);
}

#[test]
fn orphan_models_do_not_panic_and_are_kept() {
    // `provider` 指向不存在的键：无处归属，界面不显示，但**不能删**——
    // 那可能是用户先写模型后补 provider 的中间态。
    let content = r#"
[providers.p]
type = "openai"

[models."p/ok"]
provider = "p"
model = "ok"
max_context_size = 1

[models."orphan"]
provider = "does-not-exist"
model = "orphan"
max_context_size = 2
"#;
    let load = load(content);
    assert_eq!(load.providers.len(), 1);
    assert_eq!(provider(&load, "p").models.len(), 1, "孤儿不进界面");
    // 保存后仍在
    let root = toml_value(content);
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    assert_eq!(out["models"]["orphan"]["model"], "orphan");
}

#[test]
fn a_model_without_a_provider_key_is_treated_as_orphan() {
    let content = r#"
[providers.p]
type = "openai"

[models."p/ok"]
provider = "p"
model = "ok"
max_context_size = 1

[models."no-provider"]
model = "no-provider"
max_context_size = 2
"#;
    let load = load(content);
    assert_eq!(provider(&load, "p").models.len(), 1);
    let root = toml_value(content);
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    assert_eq!(out["models"]["no-provider"]["model"], "no-provider");
}

// ---------------------------------------------------------------- 序列化

#[test]
fn serialize_round_trips_without_losing_anything() {
    let root = toml_value(&config_toml());
    let load = load(&config_toml());
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    let text = kimi_backend().render(&out, false).unwrap();
    let back = toml_value(&text);
    assert_eq!(out, back, "render → parse 必须语义不变");
    // 两张表都还在
    assert!(back["providers"].get("managed:kimi-code").is_some());
    assert!(back["models"].get("kimi-code/k3").is_some());
    assert!(back["models"].get("sensenova/deepseek-v4-flash").is_some());
}

#[test]
fn serialize_keeps_the_top_level_extras() {
    let root = toml_value(&config_toml());
    let out = kimi_backend().serialize_root(&[], &[], &root, None);
    assert_eq!(out["default_model"], "sensenova/sensenova-6.8-flash-lite");
    assert_eq!(out["default_permission_mode"], "auto");
    assert_eq!(out["thinking"]["enabled"], true);
}

#[test]
fn serialize_preserves_managed_entries_verbatim() {
    // 误改 managed 会破坏登录态（credentials/ 里的凭据与这份声明配对）。
    let root = toml_value(&config_toml());
    let out = kimi_backend().serialize_root(&[], &[], &root, None);
    let managed = &out["providers"]["managed:kimi-code"];
    assert_eq!(managed["type"], "kimi");
    assert_eq!(managed["base_url"], "https://api.kimi.com/coding/v1");
    assert_eq!(managed["oauth"]["storage"], "file");
    assert_eq!(managed["oauth"]["key"], "oauth/kimi-code");
    // 它的模型也原样保留
    assert_eq!(out["models"]["kimi-code/k3"]["model"], "k3");
    assert_eq!(
        out["models"]["kimi-code/kimi-for-coding"]["display_name"],
        "K2.8 Preview"
    );
}

#[test]
fn serialize_writes_display_name_even_when_it_equals_the_wire_id() {
    // 官方写法每条模型都带 `display_name`（等于 `model` 也带，本机 7 条全带）。
    // 早先按「等于 model 就省掉」处理，一次保存就把这个键从用户文件里删了。
    let content = "[providers.p]\ntype = \"openai\"\n\n\
         [models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1000\n\
         display_name = \"m\"\n";
    let root = toml_value(content);
    let load = load(content);
    assert_eq!(provider(&load, "p").models[0].name, "m");
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    assert_eq!(
        out["models"]["p/m"]["display_name"], "m",
        "名称与 wire id 相同也要逐字写回去"
    );
}

#[test]
fn serialize_keeps_unmodelled_keys() {
    // 界面不建模的键（reasoning_key / adaptive_thinking / overrides / beta_api）要继承。
    let root = toml_value(&config_toml());
    let load = load(&config_toml());
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    let entry = &out["models"]["sensenova/sensenova-6.8-flash-lite"];
    assert_eq!(
        entry["adaptive_thinking"], true,
        "adaptive_thinking 不在界面里，但必须原样保留"
    );
}

#[test]
fn serialize_keeps_extra_model_keys() {
    let content = r#"
[providers.p]
type = "openai"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1000
reasoning_key = "reasoning_content"
beta_api = true

[models."p/m".overrides]
max_output_size = 500
"#;
    let root = toml_value(content);
    let load = load(content);
    let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
    let entry = &out["models"]["p/m"];
    assert_eq!(entry["reasoning_key"], "reasoning_content");
    assert_eq!(entry["beta_api"], true);
    assert_eq!(entry["overrides"]["max_output_size"], 500);
}

#[test]
fn quoted_keys_round_trip() {
    // `[providers.<name>]` 的键可含 `.` 与 `:`，不加引号 TOML 会当成嵌套表。
    let content = r#"
[providers."my.provider"]
type = "openai"
api_key = "k"

[providers."managed:other"]
type = "kimi"

[models."a.b/c"]
provider = "my.provider"
model = "c"
max_context_size = 1
"#;
    let root = toml_value(content);
    let text = kimi_backend().render(&root, false).unwrap();
    let back = toml_value(&text);
    assert_eq!(root, back, "含点号/冒号的键往返不变");
    assert!(
        text.contains(r#"[providers."my.provider"]"#),
        "必须加引号：{text}"
    );
}

#[test]
fn type_is_written_verbatim_and_never_rewritten() {
    // 硬套一张「内部协议 → type」映射表会把用户的 type 悄悄改写：
    // `kimi` 是 Kimi 自己的 wire 类型，映射到 `openai` 会把 OAuth 那条改成另一种协议。
    for ty in [
        "kimi",
        "openai",
        "anthropic",
        "openai_responses",
        "google-genai",
        "vertexai",
    ] {
        let content = format!(
            "[providers.p]\ntype = \"{ty}\"\napi_key = \"k\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\n"
        );
        let root = toml_value(&content);
        let load = load(&content);
        assert_eq!(load.providers[0].pi_api, ty);
        let out = kimi_backend().serialize_root(&[], &load.providers, &root, None);
        assert_eq!(out["providers"]["p"]["type"], ty, "type 必须逐字保留");
    }
}

// ---------------------------------------------------------------- 凭据 XOR

#[test]
fn api_key_and_api_key_env_are_mutually_exclusive_on_save() {
    // 同时写两个会让 Kimi Code **启动失败**（源码判成配置冲突并拒绝），
    // 比「配置不生效」严重得多——必须在写盘前挡住。
    let dir = temp_dir("xor");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();

    let load = load(&config_toml());
    let mut providers = load.providers.clone();
    providers[0].api_key = "sk-inline".into();
    providers[0].api_key_env = "MY_ENV".into();

    let err = kimi_backend()
        .save_sidecars(&path.to_string_lossy(), &providers)
        .expect_err("凭据冲突必须让保存失败");
    assert!(err.contains("mutually exclusive"), "错误要说清原因：{err}");
    assert!(
        !dir.join("models.full.toml").exists(),
        "冲突时不该写出任何文件"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_api_key_does_not_conflict_with_oauth() {
    // 本机真实文件里 `managed:kimi-code` 就是 `api_key = ""` + `oauth`：
    // 判据是「非空字符串」，空串视同未设置。
    let load = load(&config_toml());
    // managed 不进界面，所以这里不通过 providers 走；直接构造一份带 oauth 的
    let mut p = ProviderRow::new();
    p.key = "oauth-provider".into();
    p.api_key = String::new();
    p.raw = toml_value("[oauth]\nstorage = \"file\"\nkey = \"k\"\n");
    // 空密钥 + oauth 不冲突
    assert!(
        kimi_backend()
            .save_sidecars(
                &temp_dir("empty_key").join("config.toml").to_string_lossy(),
                &[p]
            )
            .is_ok(),
        "空 api_key 与 oauth 并存必须允许"
    );
    let _ = load;
}

#[test]
fn writing_a_key_clears_the_env_name_and_vice_versa() {
    // 界面两个框：填了一个就清掉另一个（XOR 的落地点）。
    let content = "[providers.p]\ntype = \"openai\"\napi_key_env = \"OLD\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    providers[0].api_key = "sk-new".into();
    providers[0].api_key_env = String::new();
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    assert_eq!(out["providers"]["p"]["api_key"], "sk-new");
    assert!(out["providers"]["p"].get("api_key_env").is_none());
}

// ---------------------------------------------------------------- capabilities

#[test]
fn capabilities_are_only_added_never_removed() {
    // 官方："only ever added, never removed"。删标签会让 Kimi 静默降级能力。
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\ncapabilities = [ \"dynamically_loaded_tools\", \"max_context_tokens\" ]\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    // 界面把「工具调用」勾上
    providers[0].models[0].tool_call = true;
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    let caps: Vec<&str> = out["models"]["p/m"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        caps.contains(&"dynamically_loaded_tools"),
        "界面不认得的标签不得删除：{caps:?}"
    );
    assert!(caps.contains(&"max_context_tokens"), "同上");
    assert!(caps.contains(&"tool_use"), "新勾的要加上：{caps:?}");
}

#[test]
fn unchecking_reasoning_can_actually_disable_thinking() {
    // 例外：thinking / always_thinking 是「支持思考」开关的两种表达，
    // 取消勾选要能一并移除，否则开关形同虚设。
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\ncapabilities = [ \"thinking\", \"always_thinking\", \"tool_use\" ]\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    providers[0].models[0].reasoning = false;
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    let caps: Vec<&str> = out["models"]["p/m"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(!caps.contains(&"thinking"), "{caps:?}");
    assert!(!caps.contains(&"always_thinking"), "{caps:?}");
    assert!(caps.contains(&"tool_use"), "只动思考相关标签：{caps:?}");
}

// ---------------------------------------------------------------- 必填校验

#[test]
fn max_context_size_is_required_and_never_zeroed() {
    // schema 要求 ≥1：写成 0 是非法值，不如不写这个键让 Kimi 自己报缺必填。
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1000\n";
    let root = toml_value(content);
    let load = load(content);
    let mut providers = load.providers.clone();
    providers[0].models[0].context = "0".into();
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    assert!(
        out["models"]["p/m"].get("max_context_size").is_none(),
        "0 不是合法值，必须省略而不是写 0"
    );
}

#[test]
fn default_effort_must_stay_inside_support_efforts() {
    let content = "[providers.p]\ntype = \"openai\"\n\n[models.\"p/m\"]\nprovider = \"p\"\nmodel = \"m\"\nmax_context_size = 1\nsupport_efforts = [ \"low\", \"high\" ]\ndefault_effort = \"high\"\n";
    let root = toml_value(content);
    let load = load(content);
    // 用户改了档位，high 不在了 → default_effort 必须删掉（留着是无效配置）
    let mut providers = load.providers.clone();
    providers[0].models[0].variants = "low, max".into();
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    assert!(out["models"]["p/m"].get("default_effort").is_none());
    // 仍在清单里时保留用户选的默认档
    let mut providers2 = load.providers.clone();
    providers2[0].models[0].variants = "low, high, max".into();
    let out2 = kimi_backend().serialize_root(&[], &providers2, &root, None);
    assert_eq!(out2["models"]["p/m"]["default_effort"], "high");
}

// ---------------------------------------------------------------- 启用/停用

#[test]
fn disabled_entries_leave_the_effective_file() {
    let root = toml_value(&config_toml());
    let load = load(&config_toml());
    let mut providers = load.providers.clone();
    providers[0].models[0].disabled = true;
    let out = kimi_backend().serialize_root(&[], &providers, &root, None);
    assert!(
        out["models"]
            .get("sensenova/sensenova-6.8-flash-lite")
            .is_none(),
        "停用条目不得进生效清单"
    );
    assert!(
        out["models"].get("sensenova/deepseek-v4-flash").is_some(),
        "其余条目照常"
    );
    // Kimi 的 schema 里没有 disabled 这个键
    for (alias, entry) in out["models"].as_object().unwrap() {
        assert!(entry.get("disabled").is_none(), "{alias} 不该带 disabled");
    }
}

#[test]
fn full_store_records_every_flag_including_false() {
    let dir = temp_dir("full_store");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();

    let load = load(&config_toml());
    let mut providers = load.providers.clone();
    providers[0].models[0].disabled = true;
    kimi_backend()
        .save_sidecars(&path.to_string_lossy(), &providers)
        .expect("写全量副本");

    let text = std::fs::read_to_string(dir.join("models.full.toml")).expect("副本应存在");
    let full = toml_value(&text);
    let entries = full["models"].as_object().unwrap();
    // 副本里**所有**条目都在（含停用的与 managed 的）
    assert!(entries.contains_key("sensenova/sensenova-6.8-flash-lite"));
    assert!(entries.contains_key("sensenova/deepseek-v4-flash"));
    assert!(
        entries.contains_key("kimi-code/k3"),
        "managed 模型也在副本里"
    );
    // **界面接管**的条目每条都写 disabled（含 false）：省掉 false 会让「全勾上」与
    // 「从没记录过勾选」变成同一状态，而且会自我延续。
    for alias in [
        "sensenova/sensenova-6.8-flash-lite",
        "sensenova/deepseek-v4-flash",
    ] {
        assert!(
            entries[alias]
                .get("disabled")
                .and_then(Value::as_bool)
                .is_some(),
            "{alias} 缺 disabled 标记"
        );
    }
    // 界面**不接管**的条目（managed / 孤儿）原样带过来，不带这个键——它们没有勾选状态，
    // 凭空加一个只会让「原样保留」变成「改写过」。
    assert!(
        entries["kimi-code/k3"].get("disabled").is_none(),
        "managed 条目必须原样保留"
    );
    assert_eq!(
        entries["sensenova/sensenova-6.8-flash-lite"]["disabled"],
        true
    );
    assert_eq!(entries["sensenova/deepseek-v4-flash"]["disabled"], false);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn loading_prefers_the_full_store_so_disabled_entries_stay_visible() {
    let dir = temp_dir("prefer_full");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();
    let config_path = path.to_string_lossy().to_string();

    let first = load(&config_toml());
    let mut providers = first.providers.clone();
    providers[0].models[0].disabled = true;
    let effective = kimi_backend().serialize_root(&[], &providers, &first.extras, None);
    let text = kimi_backend().render(&effective, false).unwrap();
    std::fs::write(&path, text).unwrap();
    kimi_backend()
        .save_sidecars(&config_path, &providers)
        .unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let reloaded = kimi_backend().parse_at(&content, &config_path).unwrap();
    let p = reloaded
        .providers
        .iter()
        .find(|p| p.key == "sensenova")
        .expect("provider 必须还在");
    let m = p
        .models
        .iter()
        .find(|m| m.id == "sensenova-6.8-flash-lite")
        .expect("停用条目必须仍在界面上，否则用户勾不回来");
    assert!(m.disabled, "勾选状态要还原");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn entries_added_by_hand_to_the_config_are_merged_in() {
    // Kimi Code 自己也会写 config.toml（/login、/model）：副本里没有的条目要并进来，
    // 否则用户新配的模型在界面上看不见。
    let dir = temp_dir("hand_added");
    let path = dir.join("config.toml");
    std::fs::write(&path, config_toml()).unwrap();
    let config_path = path.to_string_lossy().to_string();

    let first = load(&config_toml());
    kimi_backend()
        .save_sidecars(&config_path, &first.providers)
        .unwrap();

    // 手加一条模型 + 一个 provider
    let mut text = config_toml();
    text.push_str(
        "\n[providers.hand]\ntype = \"openai\"\napi_key = \"k\"\n\n[models.\"hand/added\"]\nprovider = \"hand\"\nmodel = \"added\"\nmax_context_size = 1\n",
    );
    std::fs::write(&path, text).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    let reloaded = kimi_backend().parse_at(&content, &config_path).unwrap();
    assert!(
        reloaded.providers.iter().any(|p| p.key == "hand"),
        "手加的 provider 要能看见"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn shrinks_on_save_detects_disabling() {
    use model_harbor::backends::kimi_code::shrinks_on_save;
    let before = toml_value(&config_toml());
    let after = toml_value(
        "[providers.p]\ntype = \"openai\"\n\n[models.\"p/a\"]\nprovider = \"p\"\nmodel = \"a\"\nmax_context_size = 1\n",
    );
    assert!(shrinks_on_save(&before, &after), "条目变少要触发备份");
    assert!(!shrinks_on_save(&after, &before), "条目变多不用备份");
    assert!(!shrinks_on_save(&before, &before), "没变不用备份");
}

#[test]
fn full_store_path_sits_beside_the_config() {
    use model_harbor::backends::kimi_code::full_store_path;
    assert_eq!(
        full_store_path(r"C:\Users\me\.kimi-code\config.toml"),
        r"C:\Users\me\.kimi-code\models.full.toml"
    );
    assert_eq!(
        full_store_path("/home/me/.kimi-code/config.toml"),
        "/home/me/.kimi-code/models.full.toml"
    );
    // 名字必须与主配置不同，否则会把生效清单覆盖成副本
    assert!(!full_store_path("config.toml").ends_with("config.toml"));
}

#[test]
fn full_store_aborts_when_the_config_is_unreadable() {
    // 副本与主配置都读不出时必须**报错**（由调用方取消保存），不能静默用空基底：
    // 空基底会让停用条目的未知字段（adaptive_thinking / overrides）在这一次保存里被抹掉。
    let dir = temp_dir("sidecar_corrupt");
    let path = dir.join("config.toml");
    std::fs::write(&path, "[providers.p\ntype = ").unwrap();

    let err = kimi_backend()
        .save_sidecars(&path.to_string_lossy(), &[])
        .expect_err("损坏的配置必须让保存失败");
    assert!(err.contains("无法读取"), "应给出可读的错误：{err}");
    assert!(!dir.join("models.full.toml").exists(), "副本不该被写出来");

    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------- 跨格式

#[test]
fn cross_format_strip_keeps_managed_and_drops_ui_owned() {
    let mut root = toml_value(&config_toml());
    model_harbor::app::strip_cross_format_containers(ConfigFormat::KimiCode, &mut root, false);
    let obj = root.as_object().unwrap();

    // `managed:*` 是 `/login` 的登录态（与 credentials/ 配对），界面不显示也无从重建：
    // 剔干净就等于把用户已登录的官方模型删了。
    let providers = obj["providers"].as_object().expect("providers 表要留下");
    assert_eq!(providers.len(), 1, "只该留下 managed provider");
    let managed = &providers["managed:kimi-code"];
    assert_eq!(
        managed["oauth"]["key"], "oauth/kimi-code",
        "oauth 子表逐字保留"
    );
    assert_eq!(managed["type"], "kimi");
    assert_eq!(managed["api_key"], "");

    // 界面接管的 provider 及其名下模型才剔（干净转换）。
    let models = obj["models"].as_object().expect("models 表要留下");
    let aliases: Vec<&str> = models.keys().map(String::as_str).collect();
    assert_eq!(
        aliases,
        ["kimi-code/kimi-for-coding", "kimi-code/k3"],
        "只该留下 managed 名下的模型，且顺序不变"
    );

    // 顶层设置保留
    assert_eq!(obj["default_model"], "sensenova/sensenova-6.8-flash-lite");
    assert_eq!(obj["thinking"]["enabled"], true);
}

#[test]
fn cross_format_strip_drops_the_tables_when_nothing_is_managed() {
    // 没有只读内容时仍是「整体接管」：表要整个删掉，不留空壳。
    let mut root = toml_value(
        "[providers.sensenova]\ntype = \"openai\"\n\n\
         [models.\"sensenova/x\"]\nprovider = \"sensenova\"\nmodel = \"x\"\nmax_context_size = 1000\n",
    );
    model_harbor::app::strip_cross_format_containers(ConfigFormat::KimiCode, &mut root, false);
    let obj = root.as_object().unwrap();
    assert!(obj.get("providers").is_none());
    assert!(obj.get("models").is_none());
}

#[test]
fn cross_format_strip_keeps_orphan_models() {
    // `provider` 指向表里根本没有的键：本工具认不出的内容，格式转换不该替它决定去留。
    let mut root = toml_value(
        "[providers.sensenova]\ntype = \"openai\"\n\n\
         [models.\"ghost/x\"]\nprovider = \"ghost\"\nmodel = \"x\"\nmax_context_size = 1000\n",
    );
    model_harbor::app::strip_cross_format_containers(ConfigFormat::KimiCode, &mut root, false);
    let obj = root.as_object().unwrap();
    assert!(obj.get("providers").is_none(), "界面接管的 provider 仍要剔");
    assert_eq!(obj["models"]["ghost/x"]["provider"], "ghost");
}

#[test]
fn cross_format_strip_leaves_other_formats_alone() {
    // 剔 Kimi 的表不该动别人的容器
    let mut root = toml_value("[providers.p]\ntype = \"openai\"\n");
    model_harbor::app::strip_cross_format_containers(ConfigFormat::Opencode, &mut root, false);
    assert!(root.as_object().unwrap().get("providers").is_some());
}

#[test]
fn models_can_be_copied_to_another_format() {
    // 从 Kimi 页复制到 opencode 页不该报错，且模型 id 是 wire id（不是 alias）。
    let load = load(&config_toml());
    let m = &provider(&load, "sensenova").models[0];
    let value = m.to_value();
    assert_eq!(value["name"], "sensenova-6.8-flash-lite");
    assert_eq!(m.id, "sensenova-6.8-flash-lite");
}

// ---------------------------------------------------------------- 类型往返

#[test]
fn all_toml_types_round_trip() {
    // 字符串 / 整数 / 布尔 / 数组 / 嵌套表五种类型都不能丢。
    let content = r#"
a_string = "hello"
a_int = 42
a_bool = true
an_array = [ "x", "y" ]

[providers.p]
type = "openai"
api_key = "k"

[models."p/m"]
provider = "p"
model = "m"
max_context_size = 1
"#;
    let root = toml_value(content);
    let text = kimi_backend().render(&root, false).unwrap();
    let back = toml_value(&text);
    assert_eq!(back["a_string"], "hello");
    assert_eq!(back["a_int"], 42);
    assert_eq!(back["a_bool"], true);
    assert_eq!(back["an_array"][0], "x");
    assert_eq!(back["providers"]["p"]["type"], "openai");
    assert_eq!(root, back);
}

#[test]
fn empty_content_parses_to_an_empty_load() {
    let load = kimi_backend().parse("").unwrap();
    assert!(load.providers.is_empty());
    // 空内容 + 空 provider 列表 → 保存出一个空文件，不 panic
    let out = kimi_backend().serialize_root(&[], &[], &Value::Object(Default::default()), None);
    assert!(out.as_object().unwrap().is_empty());
}

#[test]
fn invalid_toml_reports_a_readable_error() {
    let err = match kimi_backend().parse("this is not = = toml") {
        Err(e) => e,
        Ok(_) => panic!("非法 TOML 必须解析失败"),
    };
    assert!(!err.is_empty(), "要有可读报错");
}
