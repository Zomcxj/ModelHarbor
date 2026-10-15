//! opencode v2 schema 读写回归：给 v1 更新 v1、给 v2 更新 v2，跨版本按目标转换。
//!
//! 夹具为合成数据（结构与真实样本一致，密钥为假值）。

use model_harbor_core::backends;
use model_harbor_core::backends::opencode::detect_schema_version;
use model_harbor_core::convert::package_or_npm_to_api;
use model_harbor_core::format::ConfigFormat;
use model_harbor_core::model::{AgentRow, ModelRow, OpencodeSchemaVersion, ProviderRow};
use serde_json::{json, Value};

/// 合成 v1 内容：顶层单数键、npm / options、modalities / reasoning、variants 对象。
fn v1_content() -> String {
    r#"{
        "$schema": "https://opencode.ai/config.json",
        "mcp": { "server": { "command": "node" } },
        "plugin": ["x"],
        "agent": {
            "writing": {
                "mode": "subagent",
                "description": "写文档",
                "model": "sensenova/flash",
                "variant": "high",
                "temperature": 0.3,
                "color": "info",
                "system": "负责文案。"
            }
        },
        "provider": {
            "sensenova": {
                "npm": "@ai-sdk/openai-compatible",
                "options": {
                    "baseURL": "https://fake.example.com/v1",
                    "apiKey": "sk-fake",
                    "timeout": 180000
                },
                "models": {
                    "flash": {
                        "name": "flash",
                        "modalities": { "output": ["text"] },
                        "reasoning": true,
                        "tool_call": true,
                        "limit": { "context": 256000, "output": 64000 },
                        "options": { "store": false },
                        "variants": { "medium": {}, "high": {} }
                    }
                }
            }
        }
    }"#
    .to_string()
}

/// 合成 v2 内容：顶层复数键、package / settings、capabilities、variants 数组、
/// agent 的 `model#variant` 与 `request.body.temperature`。
fn v2_content() -> String {
    r#"{
        "$schema": "https://opencode.ai/config.json",
        "mcp": { "servers": { "server": { "command": "node" } } },
        "plugins": ["x"],
        "agents": {
            "writing": {
                "mode": "subagent",
                "description": "写文档",
                "model": "sensenova/flash#high",
                "color": "info",
                "request": { "body": { "temperature": 0.3 } },
                "system": "负责文案。"
            }
        },
        "providers": {
            "sensenova": {
                "package": "@opencode/ai/providers/openai-compatible",
                "settings": {
                    "baseURL": "https://fake.example.com/v1",
                    "apiKey": "sk-fake",
                    "timeout": 180000
                },
                "models": {
                    "flash": {
                        "name": "flash",
                        "capabilities": { "tools": true, "input": ["text"], "output": ["text"] },
                        "limit": { "context": 256000, "output": 64000 },
                        "body": { "store": false },
                        "variants": [ { "id": "medium" }, { "id": "high" } ]
                    }
                }
            }
        }
    }"#
    .to_string()
}

fn backend() -> &'static dyn backends::Backend {
    backends::backend(ConfigFormat::Opencode)
}

#[test]
fn detect_version_of_both_shapes() {
    let v1: Value = serde_json::from_str(&v1_content()).unwrap();
    let v2: Value = serde_json::from_str(&v2_content()).unwrap();
    assert_eq!(detect_schema_version(&v1), OpencodeSchemaVersion::V1);
    assert_eq!(detect_schema_version(&v2), OpencodeSchemaVersion::V2);
    // 空文件按 v1 处理（无任何 v2 证据）。
    assert_eq!(detect_schema_version(&json!({})), OpencodeSchemaVersion::V1);
}

#[test]
fn v2_parse_maps_fields_into_rows() {
    let load = backend().parse(&v2_content()).expect("解析失败");
    assert_eq!(load.agents.len(), 1);
    assert_eq!(load.providers.len(), 1);

    let a = &load.agents[0];
    assert_eq!(a.schema, OpencodeSchemaVersion::V2);
    assert_eq!(a.model, "sensenova/flash");
    assert_eq!(a.variant, "high");
    assert_eq!(a.temperature, "0.3");

    let p = &load.providers[0];
    assert_eq!(p.schema, OpencodeSchemaVersion::V2);
    assert_eq!(p.npm, "@opencode/ai/providers/openai-compatible");
    assert_eq!(p.base_url, "https://fake.example.com/v1");
    assert_eq!(p.api_key, "sk-fake");
    assert_eq!(p.timeout, "180000");

    let m = &p.models[0];
    assert_eq!(m.schema, OpencodeSchemaVersion::V2);
    assert!(m.tool_call);
    // v2 无 reasoning：读入恒为关。
    assert!(!m.reasoning);
    assert_eq!(m.modalities_input, "text");
    assert_eq!(m.modalities_output, "text");
    assert_eq!(m.variants, "medium, high");
    assert!(!m.store);
    // v2 的 package 名也要能推导协议。
    assert_eq!(p.effective_api(), "openai-completions");
}

#[test]
fn v2_round_trip_is_lossless() {
    let original: Value = serde_json::from_str(&v2_content()).unwrap();
    let load = backend().parse(&v2_content()).expect("解析失败");
    let root = backend().serialize_root(&load.agents, &load.providers, &load.extras, None);
    // 未做任何修改：agent / provider 表整体保真（含键序与嵌套结构）。
    assert_eq!(root["agents"], original["agents"]);
    assert_eq!(root["providers"], original["providers"]);
    // 其余顶层字段原样保留。
    assert_eq!(root["mcp"], original["mcp"]);
    assert_eq!(root["plugins"], original["plugins"]);
}

#[test]
fn v2_edits_write_back_in_v2_shape() {
    let load = backend().parse(&v2_content()).expect("解析失败");
    let mut agents = load.agents.clone();
    let mut providers = load.providers.clone();

    // 改 agent：variant high → medium，温度 0.3 → 0.7。
    agents[0].variant = "medium".into();
    agents[0].temperature = "0.7".into();
    // 改模型：关闭 tool_call，清空档位。
    providers[0].models[0].tool_call = false;
    providers[0].models[0].variants = String::new();
    // 改 provider 密钥。
    providers[0].api_key = "sk-rotated".into();

    let root = backend().serialize_root(&agents, &providers, &load.extras, None);

    let a = &root["agents"]["writing"];
    assert_eq!(a["model"], json!("sensenova/flash#medium"));
    assert_eq!(a["request"]["body"]["temperature"], json!(0.7));
    // v2 无独立 variant / temperature 顶层键。
    assert!(a.get("variant").is_none());
    assert!(a.get("temperature").is_none());

    let p = &root["providers"]["sensenova"];
    assert_eq!(p["settings"]["apiKey"], json!("sk-rotated"));
    assert!(p.get("npm").is_none());
    assert!(p.get("options").is_none());

    let m = &p["models"]["flash"];
    assert_eq!(m["capabilities"]["tools"], json!(false));
    // 清空档位：整块删除。
    assert!(m.get("variants").is_none());
    // v2 永不写 reasoning。
    assert!(m.get("reasoning").is_none());
}

#[test]
fn v1_round_trip_stays_v1() {
    let original: Value = serde_json::from_str(&v1_content()).unwrap();
    let load = backend().parse(&v1_content()).expect("解析失败");
    let root = backend().serialize_root(&load.agents, &load.providers, &load.extras, None);
    assert_eq!(root["agent"], original["agent"]);
    assert_eq!(root["provider"], original["provider"]);
    let m = &root["provider"]["sensenova"]["models"]["flash"];
    // v1 形状原样保留：npm / options / modalities / reasoning / variants 对象。
    assert_eq!(
        root["provider"]["sensenova"]["npm"],
        json!("@ai-sdk/openai-compatible")
    );
    assert!(m.get("capabilities").is_none());
    assert_eq!(m["reasoning"], json!(true));
    assert_eq!(m["variants"], json!({ "medium": {}, "high": {} }));
}

#[test]
fn v1_rows_saved_into_v2_target_become_v2() {
    let load = backend().parse(&v1_content()).expect("解析失败");
    let target: Value = serde_json::from_str(&v2_content()).unwrap();
    let root = backend().serialize_root(&load.agents, &load.providers, &json!({}), Some(&target));

    let a = &root["agents"]["writing"];
    assert_eq!(a["model"], json!("sensenova/flash#high"));
    assert_eq!(a["request"]["body"]["temperature"], json!(0.3));

    let p = &root["providers"]["sensenova"];
    assert_eq!(
        p["package"],
        json!("@opencode/ai/providers/openai-compatible")
    );
    assert_eq!(
        p["settings"]["baseURL"],
        json!("https://fake.example.com/v1")
    );
    assert!(p.get("npm").is_none());
    assert!(p.get("options").is_none());

    let m = &p["models"]["flash"];
    assert_eq!(m["capabilities"]["tools"], json!(true));
    assert_eq!(m["capabilities"]["input"], json!(["text"]));
    assert_eq!(m["variants"], json!([{ "id": "medium" }, { "id": "high" }]));
    // v2 不落 reasoning。
    assert!(m.get("reasoning").is_none());
}

#[test]
fn v2_rows_saved_into_v1_target_become_v1() {
    let load = backend().parse(&v2_content()).expect("解析失败");
    let target: Value = serde_json::from_str(&v1_content()).unwrap();
    let root = backend().serialize_root(&load.agents, &load.providers, &json!({}), Some(&target));

    let a = &root["agent"]["writing"];
    assert_eq!(a["model"], json!("sensenova/flash"));
    assert_eq!(a["variant"], json!("high"));
    assert_eq!(a["temperature"], json!(0.3));
    assert!(a.get("request").is_none());

    let p = &root["provider"]["sensenova"];
    assert_eq!(p["npm"], json!("@ai-sdk/openai-compatible"));
    assert_eq!(p["options"]["apiKey"], json!("sk-fake"));
    assert!(p.get("package").is_none());
    assert!(p.get("settings").is_none());

    let m = &p["models"]["flash"];
    assert_eq!(m["tool_call"], json!(true));
    assert_eq!(m["modalities"]["input"], json!(["text"]));
    assert_eq!(m["variants"], json!({ "medium": {}, "high": {} }));
    // v2 来源没有 reasoning 信息，不覆盖；目标文件自己的 reasoning 保留。
    assert_eq!(m["reasoning"], json!(true));
}

#[test]
fn new_rows_saved_into_v2_file_get_v2_shape() {
    // 目标文件已是 v2（复数键 + package provider），UI 新建的行默认 v1 形状，
    // 落盘必须跟随目标文件转成 v2。
    let mut extras: Value = serde_json::from_str(&v2_content()).unwrap();
    // 清空表，模拟「文件里还没有条目」的 v2 空表。
    extras["agents"] = json!({});
    extras["providers"] = json!({});

    let mut a = AgentRow::new();
    a.key = "writer".into();
    a.model = "sensenova/flash".into();
    a.variant = "high".into();
    a.temperature = "0.3".into();

    let mut p = ProviderRow::new();
    p.key = "sensenova".into();
    p.npm = "@ai-sdk/anthropic".into();
    p.base_url = "https://fake.example.com".into();
    p.api_key = "sk-fake".into();
    let mut m = ModelRow::new();
    m.id = "flash".into();
    m.name = "flash".into();
    m.reasoning = true;
    m.tool_call = true;
    m.variants = "medium, high".into();
    p.models.push(m);

    let root = backend().serialize_root(&[a], &[p], &extras, None);

    let agent = &root["agents"]["writer"];
    assert_eq!(agent["model"], json!("sensenova/flash#high"));
    assert_eq!(agent["request"]["body"]["temperature"], json!(0.3));

    let prov = &root["providers"]["sensenova"];
    // UI 下拉写的是 v1 包名，落盘前翻译成 v2 package 名。
    assert_eq!(prov["package"], json!("@opencode/ai/providers/anthropic"));
    assert_eq!(
        prov["settings"]["baseURL"],
        json!("https://fake.example.com/v1")
    );
    assert!(prov.get("npm").is_none());

    let model = &prov["models"]["flash"];
    assert_eq!(model["capabilities"]["tools"], json!(true));
    assert_eq!(model["capabilities"]["input"], json!(["text", "image"]));
    assert_eq!(
        model["variants"],
        json!([{ "id": "medium" }, { "id": "high" }])
    );
    // 新建行 reasoning=true 也不得写进 v2。
    assert!(model.get("reasoning").is_none());
}

#[test]
fn package_suffix_maps_to_api() {
    assert_eq!(
        package_or_npm_to_api("@opencode/ai/providers/openai-compatible"),
        "openai-completions"
    );
    assert_eq!(
        package_or_npm_to_api("@opencode/ai/providers/anthropic"),
        "anthropic-messages"
    );
    assert_eq!(
        package_or_npm_to_api("@opencode/ai/providers/google"),
        "google-generative-ai"
    );
    assert_eq!(
        package_or_npm_to_api("@opencode/ai/providers/openai"),
        "openai-responses"
    );
    assert_eq!(
        package_or_npm_to_api("@opencode/ai/providers/mistral"),
        "mistral-conversations"
    );
    // 未知后缀退回兼容层。
    assert_eq!(
        package_or_npm_to_api("@opencode/ai/providers/custom-thing"),
        "openai-completions"
    );
    // v1 包名照旧走 npm_to_api。
    assert_eq!(
        package_or_npm_to_api("@ai-sdk/anthropic"),
        "anthropic-messages"
    );
    assert_eq!(package_or_npm_to_api(""), "openai-completions");
}
