use super::*;
use crate::model::ModelRow;

fn root_of(text: &str) -> Value {
    parse_toml(text).unwrap()
}

const SAMPLE: &str = r#"
default_model = "kimi-code/k3"
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
api_key = "sk-test"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking", "tool_use" ]
display_name = "K3"

[models."sensenova/deepseek-v4-flash"]
provider = "sensenova"
model = "deepseek-v4-flash"
max_context_size = 262000
capabilities = [ "tool_use" ]

[thinking]
enabled = true
"#;

/// 双表 join：模型按 `provider` 挂到对应 provider 下。
#[test]
fn models_join_to_their_provider() {
    let root = root_of(SAMPLE);
    let joined = join(&root);
    let sensenova = joined.iter().find(|j| j.name == "sensenova").unwrap();
    assert_eq!(sensenova.models.len(), 1);
    assert_eq!(sensenova.models[0].0, "sensenova/deepseek-v4-flash");
    // 两条模型都挂到了 provider 上（没有孤儿）
    assert_eq!(joined.iter().map(|j| j.models.len()).sum::<usize>(), 2);
}

/// 孤儿模型（provider 不存在）不 panic、不被丢弃。
#[test]
fn orphan_models_are_kept() {
    let root = root_of(
        r#"
[providers.p]
type = "openai"

[models."gone/x"]
provider = "nonexistent"
model = "x"
max_context_size = 1
"#,
    );
    let joined = join(&root);
    assert_eq!(joined.len(), 1);
    assert_eq!(joined[0].models.len(), 0, "孤儿模型不挂到任何 provider 上");
    // 保存后仍在
    let out = KimiCodeBackend.serialize_root(&[], &[], &root, None);
    assert_eq!(out["models"]["gone/x"]["model"], "x");
}

/// `managed:*` 的 provider 与模型都不进界面。
#[test]
fn managed_providers_are_not_listed() {
    let root = root_of(SAMPLE);
    let joined = join(&root);
    let load = build_load(joined);
    assert_eq!(load.providers.len(), 1, "只列出 sensenova");
    assert_eq!(load.providers[0].key, "sensenova");
}

/// `managed:*` 的 provider 与模型保存后原样还在（含 oauth 子表）。
#[test]
fn managed_entries_survive_a_save() {
    let root = root_of(SAMPLE);
    let joined = join(&root);
    let load = build_load(joined);
    let out = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
    let managed = &out["providers"]["managed:kimi-code"];
    assert_eq!(managed["type"], "kimi");
    assert_eq!(managed["oauth"]["key"], "oauth/kimi-code", "oauth 子表保留");
    assert_eq!(
        out["models"]["kimi-code/k3"]["model"], "k3",
        "managed 模型保留"
    );
}

/// 界面没有 provider 时，基座里的 provider 不被删。
#[test]
fn providers_are_not_dropped_when_ui_is_empty() {
    let root = root_of(SAMPLE);
    let out = KimiCodeBackend.serialize_root(&[], &[], &root, None);
    assert!(out["providers"].get("managed:kimi-code").is_some());
    assert!(out["models"].get("kimi-code/k3").is_some());
}

/// alias ≠ model 时往返保持 alias 作表键、model 作 wire id。
#[test]
fn alias_is_preserved_when_it_differs_from_model() {
    let root = root_of(
        r#"
[providers.p]
type = "openai"
api_key = "k"

[models."my-alias"]
provider = "p"
model = "real-wire-id"
max_context_size = 1000
"#,
    );
    let joined = join(&root);
    let load = build_load(joined);
    assert_eq!(
        load.providers[0].models[0].id, "real-wire-id",
        "id 是 wire id"
    );
    assert_eq!(load.providers[0].models[0].kimi_alias, "my-alias");
    let out = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
    assert!(out["models"].get("my-alias").is_some(), "alias 仍是表键");
    assert_eq!(out["models"]["my-alias"]["model"], "real-wire-id");
}

/// 含 `.` / `:` 的键序列化后仍是合法 TOML 且键名不变。
#[test]
fn dotted_and_colon_keys_round_trip() {
    let root = root_of(SAMPLE);
    let text = to_toml_string(&root).unwrap();
    let back = parse_toml(&text).unwrap();
    assert_eq!(root, back, "往返语义不变");
    assert!(
        text.contains("[providers.\"managed:kimi-code\"]"),
        "含冒号的键必须加引号"
    );
}

/// capabilities 只增不减：界面没勾的已有标签保存后仍在。
#[test]
fn capabilities_are_only_added() {
    let entry: Value = toml::from_str(
            "model = \"m\"\nmax_context_size = 1\ncapabilities = [\"thinking\", \"dynamically_loaded_tools\"]\n",
        )
        .unwrap();
    let mut m = model_from_entry("a", &entry);
    m.tool_call = true; // 界面勾上工具调用
    m.reasoning = true;
    let out = entry_from_model(&m, "p", Some(&entry));
    let caps: Vec<&str> = out["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        caps.contains(&"dynamically_loaded_tools"),
        "原有标签不得删除"
    );
    assert!(caps.contains(&"tool_use"), "新勾的标签要加上");
    assert!(caps.contains(&"thinking"));
}

/// 取消「支持思考」要能真的关掉。
#[test]
fn unchecking_reasoning_removes_thinking_caps() {
    let entry: Value = toml::from_str(
            "model = \"m\"\nmax_context_size = 1\ncapabilities = [\"thinking\", \"always_thinking\", \"tool_use\"]\n",
        )
        .unwrap();
    let mut m = model_from_entry("a", &entry);
    m.reasoning = false;
    let out = entry_from_model(&m, "p", Some(&entry));
    let caps: Vec<&str> = out["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(!caps.contains(&"thinking"));
    assert!(!caps.contains(&"always_thinking"));
    assert!(caps.contains(&"tool_use"), "只动思考相关标签");
}

/// 写出的模型条目永远不带 `disabled`。
#[test]
fn written_entries_carry_no_disabled_key() {
    let root = root_of(SAMPLE);
    let joined = join(&root);
    let load = build_load(joined);
    let out = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
    for (alias, entry) in out["models"].as_object().unwrap() {
        assert!(entry.get("disabled").is_none(), "{alias} 不该带 disabled");
    }
}

/// XOR 体检：同时填 api_key 与 api_key_env 判成冲突。
#[test]
fn credential_conflict_is_detected() {
    let mut p = ProviderRow::new();
    p.key = "x".into();
    p.api_key = "sk-1".into();
    p.api_key_env = "MY_KEY".into();
    let err = first_credential_conflict(&[p]).unwrap();
    assert!(err.contains("api_key_env"), "{err}");
    assert!(err.contains("mutually exclusive"), "{err}");
}

/// `api_key = ""` 与 `oauth` 并存不是冲突。
#[test]
fn empty_api_key_does_not_conflict_with_oauth() {
    let root = root_of(SAMPLE);
    let managed = &root["providers"]["managed:kimi-code"];
    assert_eq!(credential_conflict("managed:kimi-code", managed), None);
}

/// 非空 api_key 与 oauth 并存才是冲突。
#[test]
fn non_empty_api_key_conflicts_with_oauth() {
    let entry: Value = toml::from_str(
        "type = \"kimi\"\napi_key = \"sk-x\"\n[oauth]\nstorage = \"file\"\nkey = \"k\"\n",
    )
    .unwrap();
    let msg = credential_conflict("p", &entry).unwrap();
    assert!(msg.contains("api_key") && msg.contains("oauth"), "{msg}");
}

/// 界面填了密钥就清掉 env 变量名（XOR 的落地点）。
#[test]
fn writing_a_key_clears_the_env_name() {
    let old: Value = toml::from_str("type = \"openai\"\napi_key_env = \"OLD_ENV\"\n").unwrap();
    let mut p = ProviderRow::new();
    p.key = "p".into();
    p.pi_api = "openai".into();
    p.api_key = "sk-new".into();
    p.api_key_env = "OLD_ENV".into();
    let out = provider_entry_from_row(&p, Some(&old));
    assert_eq!(out["api_key"], "sk-new");
    assert!(out.get("api_key_env").is_none(), "必须清掉 env 名");
}

/// 界面填了 env 名就清掉密钥。
#[test]
fn writing_an_env_name_clears_the_key() {
    let old: Value = toml::from_str("type = \"openai\"\napi_key = \"sk-old\"\n").unwrap();
    let mut p = ProviderRow::new();
    p.key = "p".into();
    p.pi_api = "openai".into();
    p.api_key_env = "MY_KEY".into();
    let out = provider_entry_from_row(&p, Some(&old));
    assert_eq!(out["api_key_env"], "MY_KEY");
    assert!(out.get("api_key").is_none(), "必须清掉密钥");
}

/// `max_context_size` 必填且 ≥1：填不出正数就不写这个键。
#[test]
fn invalid_context_size_is_omitted_not_zeroed() {
    let mut obj = Map::new();
    set_num_min1(&mut obj, "max_context_size", "0");
    assert!(obj.get("max_context_size").is_none());
    set_num_min1(&mut obj, "max_context_size", "");
    assert!(obj.get("max_context_size").is_none());
    set_num_min1(&mut obj, "max_context_size", "abc");
    assert!(obj.get("max_context_size").is_none());
    set_num_min1(&mut obj, "max_context_size", "1024");
    assert_eq!(obj["max_context_size"], 1024);
}

/// `default_effort` 不在 `support_efforts` 里时删掉。
#[test]
fn default_effort_must_stay_within_support_efforts() {
    let entry: Value = toml::from_str(
            "model = \"m\"\nmax_context_size = 1\nsupport_efforts = [\"low\", \"high\"]\ndefault_effort = \"high\"\n",
        )
        .unwrap();
    let mut m = model_from_entry("a", &entry);
    m.variants = "low, max".into(); // 用户改了档位，high 不在了
    let out = entry_from_model(&m, "p", Some(&entry));
    assert!(
        out.get("default_effort").is_none(),
        "不在清单里的 default_effort 必须删掉"
    );
    // 仍在清单里时保留用户选的默认档
    let mut m2 = model_from_entry("a", &entry);
    m2.variants = "low, high, max".into();
    let out2 = entry_from_model(&m2, "p", Some(&entry));
    assert_eq!(out2["default_effort"], "high");
}

/// 顶层 extras（`[thinking]`、`default_*`）往返不丢。
#[test]
fn top_level_extras_survive() {
    let root = root_of(SAMPLE);
    let out = KimiCodeBackend.serialize_root(&[], &[], &root, None);
    assert_eq!(out["default_model"], "kimi-code/k3");
    assert_eq!(out["default_permission_mode"], "auto");
    assert_eq!(out["thinking"]["enabled"], true);
}

/// `null` 不会让 TOML 序列化失败。
#[test]
fn nulls_are_stripped_before_serializing() {
    let v: Value = serde_json::json!({"a": null, "b": {"c": null, "d": 1}, "e": [1, null]});
    let text = to_toml_string(&v).expect("null 不得让序列化失败");
    assert!(!text.contains("null"), "{text}");
    let back = parse_toml(&text).unwrap();
    assert_eq!(back["b"]["d"], 1);
}

/// 判别：Codex 的 `model_providers` 不归本后端。
#[test]
fn detect_rejects_codex_shape() {
    let codex = r#"
model = "gpt-5"
model_provider = "openai"

[model_providers.openai]
name = "OpenAI"
base_url = "https://api.openai.com/v1"
"#;
    assert!(!KimiCodeBackend.detect(codex, ""));
}

/// 判别：路径命中与内容特征都认。
#[test]
fn detect_accepts_kimi_shapes() {
    assert!(KimiCodeBackend.detect("", r"C:\Users\x\.kimi-code\config.toml"));
    assert!(KimiCodeBackend.detect(SAMPLE, ""));
    assert!(KimiCodeBackend.detect("default_permission_mode = \"auto\"\n", ""));
    // 空内容 / 无关 TOML 不认
    assert!(!KimiCodeBackend.detect("", ""));
    assert!(!KimiCodeBackend.detect("[package]\nname = \"x\"\n", ""));
}

/// 解析失败要有可读报错（非法 TOML）。
#[test]
fn invalid_toml_reports_an_error() {
    assert!(KimiCodeBackend.parse("this is not = = toml").is_err());
}

/// 空文件解析成空 load，不 panic。
#[test]
fn empty_file_parses_to_nothing() {
    let load = KimiCodeBackend.parse("").unwrap();
    assert!(load.providers.is_empty());
}

/// 别名缺省值：`<provider>/<model>`，managed 用去掉前缀的平台名。
#[test]
fn alias_key_follows_kimis_own_convention() {
    assert_eq!(
        alias_key("sensenova", "deepseek-v4-flash"),
        "sensenova/deepseek-v4-flash"
    );
    assert_eq!(alias_key("managed:kimi-code", "k3"), "kimi-code/k3");
    assert_eq!(
        alias_key("workbuddy", "deepseek-v4.1-flash"),
        "workbuddy/deepseek-v4.1-flash"
    );
}

/// 新增（或跨格式复制来的）模型没有 `kimi_alias`：写出时生成 `<provider>/<model>`，
/// `display_name` 回落到 wire id。
#[test]
fn a_new_model_gets_a_vendor_prefixed_alias_and_display_name() {
    let root = root_of("[providers.p]\ntype = \"openai\"\n");
    let mut p = ProviderRow::new();
    p.key = "sensenova".into();
    p.pi_api = "openai-completions".into();
    let mut m = ModelRow::new();
    m.id = "deepseek-v4-flash".into(); // name 留空（界面中间态）
    p.models.push(m);
    let out = KimiCodeBackend.serialize_root(&[], &[p], &root, None);
    let entry = &out["models"]["sensenova/deepseek-v4-flash"];
    assert_eq!(
        entry["model"], "deepseek-v4-flash",
        "alias = provider/model"
    );
    assert_eq!(entry["provider"], "sensenova");
    assert_eq!(
        entry["display_name"], "deepseek-v4-flash",
        "名称为空时回落 wire id，不省略这个键"
    );
}

/// 没有 wire id 的模型行（界面中间态）不写进文件。
#[test]
fn models_without_a_wire_id_are_skipped() {
    let root = root_of(SAMPLE);
    let joined = join(&root);
    let mut load = build_load(joined);
    load.providers[0].models.push(ModelRow::new()); // id 为空
    let out = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
    for (_, entry) in out["models"].as_object().unwrap() {
        assert!(!entry["model"].as_str().unwrap_or("").is_empty());
    }
}

/// 新条目的键序：`provider` 在 `model` 之前。
#[test]
fn a_new_entry_puts_provider_before_model() {
    let entry: Value = toml::from_str(
        "model = \"m\"
max_context_size = 1
",
    )
    .unwrap();
    let m = model_from_entry("a", &entry);
    let out = entry_from_model(&m, "openai_247kan", None);
    let keys: Vec<&String> = out.as_object().unwrap().keys().collect();
    assert_eq!(keys[0], "provider");
    assert_eq!(keys[1], "model");
}

/// 旧条目的键序不受影响。
#[test]
fn an_existing_entry_keeps_its_file_order() {
    let entry: Value = toml::from_str(
        "provider = \"p\"
model = \"m\"
max_context_size = 1
display_name = \"M\"
",
    )
    .unwrap();
    let m = model_from_entry("a", &entry);
    let out = entry_from_model(&m, "p", Some(&entry));
    let keys: Vec<&String> = out.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["provider", "model", "max_context_size", "display_name"]
    );
}

/// 认不出来的键（`overrides` / `reasoning_key` / 模型级 `base_url`）原样保留。
#[test]
fn unmodelled_keys_are_inherited() {
    let entry: Value = toml::from_str(
        r#"
model = "m"
max_context_size = 1000
reasoning_key = "reasoning_content"
beta_api = true
base_url = "https://override/v1"

[overrides]
max_output_size = 500
"#,
    )
    .unwrap();
    let m = model_from_entry("a", &entry);
    let out = entry_from_model(&m, "p", Some(&entry));
    assert_eq!(out["reasoning_key"], "reasoning_content");
    assert_eq!(out["beta_api"], true);
    assert_eq!(out["base_url"], "https://override/v1");
    assert_eq!(out["overrides"]["max_output_size"], 500);
}
