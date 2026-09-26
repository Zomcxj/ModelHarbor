use model_harbor::app::{load_opencode_result, load_or_empty};
use model_harbor::model::{AgentRow, ModelRow, ProviderRow};
use model_harbor::util::{is_wsl_path, win_to_wsl};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

fn tmp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("opencode_test_{}.json", name));
    p
}

#[test]
fn agent_row_roundtrip() {
    let v = json!({
        "mode": "subagent",
        "description": "test agent",
        "model": "gpt-4",
        "variant": "turbo",
        "color": "#ff0000",
        "system": "system prompt",
        "temperature": 0.7,
        "extra_field": "preserved"
    });
    let agent = AgentRow::from("my_agent", &v);
    assert_eq!(agent.key, "my_agent");
    assert_eq!(agent.mode, "subagent");
    assert_eq!(agent.description, "test agent");
    assert_eq!(agent.model, "gpt-4");
    assert_eq!(agent.variant, "turbo");
    assert_eq!(agent.color, "#ff0000");
    assert_eq!(agent.system, "system prompt");

    let out = agent.to_value();
    assert_eq!(out["mode"], "subagent");
    assert_eq!(out["description"], "test agent");
    assert_eq!(out["model"], "gpt-4");
    assert_eq!(out["variant"], "turbo");
    assert_eq!(out["color"], "#ff0000");
    assert_eq!(out["system"], "system prompt");
    assert_eq!(out["temperature"], 0.7);
    assert_eq!(out["extra_field"], "preserved");
}

#[test]
fn agent_temperature_integer() {
    let v = json!({ "temperature": 1 });
    let agent = AgentRow::from("a", &v);
    let out = agent.to_value();
    assert_eq!(out["temperature"], 1);
}

#[test]
fn agent_temperature_float() {
    let v = json!({ "temperature": 0.7 });
    let agent = AgentRow::from("a", &v);
    let out = agent.to_value();
    assert_eq!(out["temperature"], 0.7);
}

#[test]
fn agent_temperature_empty_removes_field() {
    let v = json!({ "temperature": 0.5 });
    let mut agent = AgentRow::from("a", &v);
    agent.temperature = String::new();
    let out = agent.to_value();
    assert!(
        out.get("temperature").is_none(),
        "temperature should be removed when empty"
    );
}

#[test]
fn agent_temperature_invalid_removes_field() {
    let v = json!({ "temperature": 0.5 });
    let mut agent = AgentRow::from("a", &v);
    agent.temperature = "abc".to_string();
    let out = agent.to_value();
    assert!(
        out.get("temperature").is_none(),
        "temperature should be removed for invalid string"
    );
}

#[test]
fn model_row_roundtrip() {
    let v = json!({
        "name": "GPT-4o",
        "reasoning": true,
        "tool_call": false,
        "limit": {
            "context": 128000,
            "output": 4096
        },
        "modalities": {
            "input": ["text", "image"],
            "output": ["text"]
        }
    });
    let model = ModelRow::from("gpt-4o", &v);
    assert_eq!(model.id, "gpt-4o");
    assert_eq!(model.name, "GPT-4o");
    assert!(model.reasoning);
    assert!(!model.tool_call);
    assert_eq!(model.context, "128000");
    assert_eq!(model.output, "4096");
    assert_eq!(model.modalities_input, "text, image");
    assert_eq!(model.modalities_output, "text");

    let out = model.to_value();
    assert_eq!(out["name"], "GPT-4o");
    assert_eq!(out["reasoning"], true);
    assert_eq!(out["tool_call"], false);
    assert_eq!(out["limit"]["context"], 128000);
    assert_eq!(out["limit"]["output"], 4096);
    assert_eq!(out["modalities"]["input"][0], "text");
    assert_eq!(out["modalities"]["input"][1], "image");
    assert_eq!(out["modalities"]["output"][0], "text");
}

#[test]
fn model_modalities_empty_removes_field() {
    let v = json!({
        "name": "test",
        "reasoning": false,
        "tool_call": false,
        "limit": { "context": 100, "output": 50 }
    });
    let mut model = ModelRow::from("m", &v);
    model.modalities_input = String::new();
    model.modalities_output = String::new();
    let out = model.to_value();
    assert!(
        out.get("modalities").is_none(),
        "modalities should be removed when both empty"
    );
}

#[test]
fn provider_row_roundtrip() {
    let v = json!({
        "description": "OpenAI provider",
        "npm": "@opencode/openai",
        "options": {
            "baseURL": "https://api.openai.com",
            "apiKey": "sk-xxx",
            "timeout": 30
        },
        "models": {
            "gpt-4o": {
                "name": "GPT-4o",
                "reasoning": false,
                "tool_call": true,
                "limit": { "context": 128000, "output": 4096 }
            }
        }
    });
    let provider = ProviderRow::from("openai", &v);
    assert_eq!(provider.key, "openai");
    assert_eq!(provider.description, "OpenAI provider");
    assert_eq!(provider.npm, "@opencode/openai");
    assert_eq!(provider.base_url, "https://api.openai.com");
    assert_eq!(provider.api_key, "sk-xxx");
    assert_eq!(provider.timeout, "30");
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].id, "gpt-4o");

    let out = provider.to_value();
    assert_eq!(out["description"], "OpenAI provider");
    assert_eq!(out["npm"], "@opencode/openai");
    assert_eq!(out["options"]["baseURL"], "https://api.openai.com");
    assert_eq!(out["options"]["apiKey"], "sk-xxx");
    assert_eq!(out["options"]["timeout"], 30);
    assert_eq!(out["models"]["gpt-4o"]["name"], "GPT-4o");
}

#[test]
fn opencode_chat_completions_defaults_compat_false() {
    // opencode 缺省 npm（等价 @ai-sdk/openai）或显式 @ai-sdk/openai(-compatible)
    // 都是 chat/completions 风格，未声明 compat 时 supportsDeveloperRole 默认不勾选。
    for npm in ["", "@ai-sdk/openai", "@ai-sdk/openai-compatible"] {
        let v = json!({
            "npm": npm,
            "options": {"baseURL": "https://api.openai.com", "apiKey": "sk-xxx"},
            "models": {}
        });
        let provider = ProviderRow::from("openai", &v);
        assert!(!provider.compat, "npm={:?} 应默认不打勾", npm);
    }
    // 非 chat/completions（如 anthropic）保持默认勾选
    let v = json!({
        "npm": "@ai-sdk/anthropic",
        "options": {"baseURL": "https://api.anthropic.com", "apiKey": "sk-xxx"},
        "models": {}
    });
    let provider = ProviderRow::from("anthropic", &v);
    assert!(provider.compat);
}

#[test]
fn cross_format_source_writes_default_timeout_to_opencode() {
    // pi 来源 → opencode：文件没有 options.timeout 时写出默认 180000。
    let raw = json!({
        "baseUrl": "https://x/v1",
        "apiKey": "k",
        "api": "openai-completions",
        "models": []
    });
    let provider = model_harbor::convert::provider_from_pi("p", &raw);
    let out = provider.to_value();
    assert_eq!(out["options"]["timeout"], 180000);
}

#[test]
fn load_or_empty_nonexistent_path() {
    let (root, agents, providers) = load_or_empty("C:\\nonexistent_path_12345.json");
    assert!(root.is_object());
    assert!(root.as_object().unwrap().is_empty());
    assert!(agents.is_empty());
    assert!(providers.is_empty());
}

#[test]
fn load_or_empty_real_file() {
    let path = tmp_path("load_real");
    let content = serde_json::to_string_pretty(&json!({
        "agent": {
            "agent1": { "mode": "subagent", "model": "m1" },
            "agent2": { "mode": "main", "model": "m2" }
        },
        "provider": {
            "prov1": { "description": "p1", "npm": "n1" }
        }
    }))
    .unwrap();
    fs::write(&path, &content).unwrap();

    let (_, agents, providers) = load_or_empty(path.to_str().unwrap());
    assert_eq!(agents.len(), 2, "should have 2 agents");
    assert_eq!(providers.len(), 1, "should have 1 provider");
    fs::remove_file(&path).ok();
}

#[test]
fn nested_roundtrip_modify_and_reload() {
    let path = tmp_path("nested_rt");

    let root = json!({
        "agent": {
            "a1": { "mode": "subagent", "description": "old desc", "model": "m1" }
        },
        "provider": {}
    });
    fs::write(&path, serde_json::to_string_pretty(&root).unwrap()).unwrap();

    let (mut root_val, mut agents, _) = load_or_empty(path.to_str().unwrap());
    assert_eq!(agents[0].description, "old desc");

    agents[0].description = "new desc".to_string();

    if let Value::Object(ref mut o) = root_val {
        let mut am = Map::new();
        for a in &agents {
            am.insert(a.key.clone(), a.to_value());
        }
        o.insert("agent".into(), Value::Object(am));
    }
    fs::write(&path, serde_json::to_string_pretty(&root_val).unwrap()).unwrap();

    let (_, agents2, _) = load_or_empty(path.to_str().unwrap());
    assert_eq!(agents2[0].description, "new desc");
    fs::remove_file(&path).ok();
}

#[test]
fn is_wsl_path_test() {
    assert!(is_wsl_path("/home/user/file.json"));
    assert!(is_wsl_path("/mnt/c/config.json"));
    assert!(!is_wsl_path("C:/Users/test/file.json"));
    assert!(!is_wsl_path("D:/VsPro/project/file.json"));
    assert!(!is_wsl_path(""));
}

#[test]
fn win_to_wsl_test() {
    assert_eq!(win_to_wsl("C:/Users/test"), "/mnt/c/Users/test");
    assert_eq!(win_to_wsl("D:/VsPro/project"), "/mnt/d/VsPro/project");
    assert_eq!(win_to_wsl("/home/user"), "/home/user");
}

#[test]
fn model_modalities_single_side_kept() {
    let v = json!({
        "name": "test",
        "reasoning": false,
        "tool_call": false,
        "limit": { "context": 100, "output": 50 },
        "modalities": { "input": ["text", "image"], "output": ["text"] }
    });
    let mut model = ModelRow::from("m", &v);
    model.modalities_input = String::new();
    let out = model.to_value();
    assert!(
        out.get("modalities").is_some(),
        "modalities should survive when one side is empty"
    );
    assert!(
        out["modalities"].get("input").is_none(),
        "input subkey removed"
    );
    assert_eq!(
        out["modalities"]["output"][0], "text",
        "output side preserved"
    );
}

#[test]
fn model_store_omitted_when_false() {
    let v = json!({ "name": "m", "reasoning": false, "tool_call": false });
    let model = ModelRow::from("m", &v);
    let out = model.to_value();
    assert!(
        out.get("options").is_none(),
        "options should be omitted when store=false and raw had none"
    );
}

#[test]
fn model_store_written_when_true() {
    let mut model = ModelRow::new();
    model.id = "m".into();
    model.store = true;
    let out = model.to_value();
    assert_eq!(out["options"]["store"], true);
}

#[test]
fn model_variants_raw_values_preserved() {
    let v = json!({
        "name": "m",
        "reasoning": false,
        "tool_call": false,
        "variants": { "high": { "reasoningEffort": "high" } }
    });
    let model = ModelRow::from("m", &v);
    let out = model.to_value();
    assert_eq!(
        out["variants"]["high"]["reasoningEffort"], "high",
        "variant detail values must survive save"
    );
}

#[test]
fn model_limit_omitted_when_empty() {
    // 空上下文/输出限制不得写入 "limit": {} 污染配置
    let v = json!({ "name": "m", "reasoning": false, "tool_call": false });
    let model = ModelRow::from("m", &v);
    let out = model.to_value();
    assert!(out.get("limit").is_none(), "empty limit must be omitted");
}

#[test]
fn provider_options_omitted_when_empty() {
    // 无 baseURL/apiKey/timeout 时不得写入 "options": {} 污染配置
    let v = json!({ "npm": "@ai-sdk/openai", "models": {} });
    let provider = ProviderRow::from("p", &v);
    // 文件未写 options.timeout 时默认显示 180000（未修改不写回）
    assert_eq!(provider.timeout, "180000");
    assert_eq!(provider.original_timeout, "180000");
    let out = provider.to_value();
    assert!(
        out.get("options").is_none(),
        "empty options must be omitted"
    );
}

#[test]
fn load_opencode_result_invalid_json_is_err() {
    let path = tmp_path("invalid_json");
    fs::write(&path, "{ not valid json !!!").unwrap();
    let res = load_opencode_result(path.to_str().unwrap());
    assert!(res.is_err(), "corrupted JSON must return Err");
    fs::remove_file(&path).ok();
}

#[test]
fn load_opencode_result_nonexistent_is_ok_empty() {
    let res = load_opencode_result("C:\\nonexistent_opencode_test_12345.json");
    assert!(res.is_ok(), "nonexistent path is a valid new-file scenario");
    let (v, agents, providers) = res.unwrap();
    assert!(v.as_object().unwrap().is_empty());
    assert!(agents.is_empty());
    assert!(providers.is_empty());
}

#[test]
fn new_model_writes_defaults_for_reasoning_tool_call_and_limit() {
    // 新增模型默认勾选 reasoning/tool_call，并预填上下文/输出上限，
    // 保存后这些字段自动写出（与 gpt-5.6-sol 的语义一致）。
    let mut model = ModelRow::new();
    model.id = "glm-5.3".into();
    model.name = "glm-5.3".into();
    let out = model.to_value();
    assert_eq!(out["reasoning"], true);
    assert_eq!(out["tool_call"], true);
    assert_eq!(out["limit"]["context"], 262000);
    assert_eq!(out["limit"]["output"], 131000);
    assert_eq!(
        out["options"]["store"], false,
        "新增模型应写出 options.store=false（与既有配置一致）"
    );
}

#[test]
fn new_model_fields_in_canonical_opencode_order() {
    // 新建模型按 opencode 惯例键顺序输出：name → modalities → reasoning →
    // tool_call → limit → options → variants。
    let mut model = ModelRow::new();
    model.id = "glm-5.3".into();
    model.name = "glm-5.3".into();
    model.modalities_input = "text".into();
    let out = model.to_value();
    let keys: Vec<&str> = out
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec![
            "name",
            "modalities",
            "reasoning",
            "tool_call",
            "limit",
            "options",
            "variants"
        ],
        "新模型键顺序应为 opencode 惯例顺序"
    );
}

#[test]
fn existing_model_key_order_preserved_when_unmodified() {
    // 同格式已有 raw 的模型不重排键顺序（最小 diff），只更新实际改动字段。
    let v = json!({"variants": {"high": {}}, "name": "m", "reasoning": true});
    let model = ModelRow::from("m", &v);
    let out = model.to_value();
    let keys: Vec<&str> = out
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["variants", "name", "reasoning"]);
}

#[test]
fn model_variants_written_in_canonical_order() {
    // 删除后再勾选不应把档位追加到末尾：写出时按规范顺序排序，
    // 同时保留 raw 中各档位的原始映射内容。
    let raw = json!({
        "name": "m",
        "variants": {"medium": {"reasoningEffort": "medium"}, "high": {}, "max": {}, "xhigh": {}}
    });
    let mut model = ModelRow::from("m", &raw);
    model.variants = "max, xhigh, medium, high".into();
    let out = model.to_value();
    let variants = out["variants"].as_object().expect("variants 应为对象");
    let keys: Vec<&str> = variants.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec!["medium", "high", "xhigh", "max"],
        "档位应按规范顺序写出，实际 {keys:?}"
    );
    assert_eq!(
        variants["medium"]["reasoningEffort"], "medium",
        "原始档位内容应保留"
    );
}
