use model_harbor::app::merge_opencode_root;
use model_harbor::convert;
use model_harbor::model::{AgentRow, ProviderRow};
use serde_json::json;

#[test]
fn merge_preserves_target_agents_and_other_fields() {
    // 目标 opencode 文件已有 agents / mcp / provider
    let target = json!({
        "agent": { "writer": { "mode": "subagent", "model": "p/m" } },
        "provider": { "old": { "npm": "@ai-sdk/x", "models": {} } },
        "mcp": { "server": { "command": "node" } },
        "providers": { "unknown_field": true }
    });
    // 来源为 pi：UI agents 为空、providers 为转换后的条目
    let providers = vec![ProviderRow::from(
        "newpi",
        &json!({ "npm": "", "options": { "baseURL": "https://x", "apiKey": "k" }, "models": {} }),
    )];
    let merged = merge_opencode_root(&target, &[], &providers);
    assert_eq!(
        merged["agent"]["writer"]["mode"], "subagent",
        "target agents must be preserved when UI agents are empty"
    );
    assert_eq!(
        merged["mcp"]["server"]["command"], "node",
        "target top-level fields must survive"
    );
    assert!(
        merged["provider"].get("newpi").is_some(),
        "UI provider upserted"
    );
    assert!(
        merged["provider"].get("old").is_some(),
        "target provider entries preserved"
    );
    assert!(
        merged["providers"]["unknown_field"].is_boolean(),
        "unknown top-level fields (including pi-style 'providers') must be preserved"
    );
}

#[test]
fn current_file_save_replaces_agent_map() {
    // 当前文件语义验证：UI 删除的 agent 不得因 upsert 复活
    // （此语义在 App::save_opencode_to 的 is_current 分支，此处验证 merge 不适用于当前文件）
    let target = json!({ "agent": { "gone": { "mode": "subagent" } } });
    let merged = merge_opencode_root(&target, &[], &[]);
    assert!(
        merged["agent"].get("gone").is_some(),
        "merge is for cross-format targets only; current-file save must replace"
    );
}

#[test]
fn merge_upserts_ui_agent_over_target() {
    let target = json!({ "agent": { "a1": { "mode": "subagent", "model": "old" } } });
    let mut a = AgentRow::new();
    a.key = "a1".into();
    a.model = "new".into();
    let merged = merge_opencode_root(&target, &[a], &[]);
    assert_eq!(
        merged["agent"]["a1"]["model"], "new",
        "UI state wins on same key"
    );
}

#[test]
fn pi_merge_preserves_target_extras() {
    // 模拟 save_pi_to 的跨目标合并：目标 root 取目标文件自身（含 providers），
    // 仅重写 providers 中的条目，目标独有条目保留。
    let mut p = std::env::temp_dir();
    p.push("opencode_test_pi_merge.json");
    let target = json!({
        "providers": {
            "existing": { "baseUrl": "https://e", "api": "openai-completions", "models": [] }
        },
        "customTopLevel": 42
    });
    std::fs::write(&p, serde_json::to_string(&target).unwrap()).unwrap();

    let providers = vec![ProviderRow::from(
        "newpi",
        &json!({ "npm": "", "options": { "baseURL": "https://x", "apiKey": "k" }, "models": {} }),
    )];

    let target_root = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    let root = convert::to_pi_root(&providers, &target_root);
    assert_eq!(
        root["customTopLevel"], 42,
        "target top-level extras must survive"
    );
    assert!(root["providers"].get("newpi").is_some());
    assert!(
        root["providers"].get("existing").is_some(),
        "cross-format 保存必须保留目标文件独有 provider（非编辑内容不能改）"
    );
    std::fs::remove_file(&p).ok();
}

#[test]
fn pi_merge_preserves_target_unknown_fields_on_same_key() {
    // 同名 provider：目标未编辑字段保留（这里 cost 是 pi 文件里的扩展字段，
    // 来源转换结果不含它，不得被删）；来源字段（baseUrl）生效。
    let target = json!({
        "providers": {
            "demo": {
                "baseUrl": "https://old/v1",
                "api": "openai-completions",
                "cost": {"input": 1.0},
                "models": []
            }
        }
    });
    let mut p = ProviderRow::new();
    p.key = "demo".into();
    p.base_url = "https://new/v1".into();
    let root = convert::to_pi_root(&[p], &target);
    let demo = &root["providers"]["demo"];
    assert_eq!(demo["baseUrl"], "https://new/v1");
    assert_eq!(demo["cost"]["input"], 1.0, "目标独有字段应保留");
}

#[test]
fn opencode_merge_preserves_target_model_options() {
    // 跨格式保存到 opencode 目标：同名 provider 的模型 options 等非编辑内容保留；
    // 目标独有模型也保留。
    let target = json!({
        "provider": {
            "demo": {
                "npm": "@ai-sdk/openai",
                "options": {"baseURL": "https://old/v1"},
                "models": {
                    "m1": {"name": "M1", "options": {"store": false}, "limit": {"context": 1000}},
                    "target-only": {"name": "TO"}
                }
            }
        }
    });
    let p = ProviderRow::from(
        "demo",
        &json!({
            "npm": "", "options": {"baseURL": "https://old/v1"},
            "models": {"m1": {"name": "M1", "limit": {"context": 1000}}}
        }),
    );
    // 模拟来自其他格式：跨格式转换后 options.store 不存在于来源
    let merged = merge_opencode_root(&target, &[], &[p]);
    let demo = &merged["provider"]["demo"];
    assert_eq!(
        demo["models"]["m1"]["options"]["store"], false,
        "目标模型 options.store 不得被删除"
    );
    assert_eq!(
        demo["models"]["target-only"]["name"], "TO",
        "目标独有模型应保留"
    );
}

// ---------- 跨格式转换：目标文件的 provider/agent 容器由界面接管 ----------

use model_harbor::app::strip_cross_format_containers;
use model_harbor::backends;
use model_harbor::format::ConfigFormat;
use serde_json::{Map, Value};

fn empty_extras() -> Value {
    Value::Object(Map::new())
}

fn ui_providers(keys: &[&str]) -> Vec<ProviderRow> {
    keys.iter()
        .map(|k| {
            ProviderRow::from(
                k,
                &json!({
                    "npm": "",
                    "options": { "baseURL": format!("https://{}.example/v1", k), "apiKey": "k" },
                    "models": {}
                }),
            )
        })
        .collect()
}

#[test]
fn strip_drops_managed_containers_per_format() {
    // opencode：provider 与 agent 由界面接管，其他顶层字段保留
    let mut oc = json!({
        "provider": { "old": {} },
        "agent": { "writer": {} },
        "mcp": { "srv": {} }
    });
    strip_cross_format_containers(ConfigFormat::Opencode, &mut oc, true);
    assert!(oc.get("provider").is_none());
    assert!(oc.get("agent").is_none());
    assert!(oc["mcp"]["srv"].is_object());

    // 界面没有 agents 数据（来源为 pi / omp / DSH）时不得接管 agent 容器
    let mut oc_no_agents = json!({
        "provider": { "old": {} },
        "agent": { "writer": {} },
        "mcp": { "srv": {} }
    });
    strip_cross_format_containers(ConfigFormat::Opencode, &mut oc_no_agents, false);
    assert!(oc_no_agents.get("provider").is_none());
    assert!(
        oc_no_agents["agent"]["writer"].is_object(),
        "来源不是 opencode 时，目标文件的 agents 必须保留"
    );
    assert!(oc_no_agents["mcp"]["srv"].is_object());

    // pi / omp：providers 由界面接管，其他顶层字段保留
    for fmt in [ConfigFormat::Pi, ConfigFormat::OhMyPi] {
        let mut root = json!({ "providers": { "old": {} }, "defaultModel": "x" });
        strip_cross_format_containers(fmt, &mut root, false);
        assert!(root.get("providers").is_none(), "{:?}", fmt);
        assert_eq!(root["defaultModel"], "x", "{:?}", fmt);
    }

    // DSH：只剔除 llm-pi-ai.providers，同级其他设置保留
    let mut dsh = json!({
        "llm-pi-ai": { "providers": { "old": {} }, "timeoutMs": 10000 },
        "other": 1
    });
    strip_cross_format_containers(ConfigFormat::DeepSeekHarness, &mut dsh, false);
    assert!(dsh["llm-pi-ai"].get("providers").is_none());
    assert_eq!(dsh["llm-pi-ai"]["timeoutMs"], 10000);
    assert_eq!(dsh["other"], 1);
}

#[test]
fn cross_format_pi_doc_takes_ui_providers_and_order() {
    // 目标 pi 文件已有 old 厂商 + 其他顶层字段
    let mut target = json!({
        "providers": { "old": { "baseUrl": "https://old/v1" } },
        "defaultProvider": "old"
    });
    strip_cross_format_containers(ConfigFormat::Pi, &mut target, false);
    let providers = ui_providers(&["zeta", "alpha"]);
    let doc = backends::backend(ConfigFormat::Pi).serialize_root(
        &[],
        &providers,
        &empty_extras(),
        Some(&target),
    );
    let keys: Vec<String> = doc["providers"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(
        keys,
        vec!["zeta", "alpha"],
        "跨格式转换的 provider 顺序必须来自界面（拖拽排序在预览/保存中可见）"
    );
    assert!(
        doc["providers"].get("old").is_none(),
        "目标文件旧的 provider 不得残留（干净转换）"
    );
    assert_eq!(doc["defaultProvider"], "old", "目标文件的其他顶层字段保留");
}

#[test]
fn cross_format_opencode_doc_takes_ui_providers_and_order() {
    let mut target = json!({
        "provider": { "old": {} },
        "agent": { "writer": {} },
        "mcp": { "srv": { "command": "node" } }
    });
    strip_cross_format_containers(ConfigFormat::Opencode, &mut target, true);
    let providers = ui_providers(&["b", "a"]);
    let doc = merge_opencode_root(&target, &[], &providers);
    let keys: Vec<String> = doc["provider"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(keys, vec!["b", "a"]);
    assert!(doc["provider"].get("old").is_none());
    assert!(doc["agent"].as_object().is_none_or(|o| o.is_empty()));
    assert_eq!(doc["mcp"]["srv"]["command"], "node");
}

#[test]
fn cross_format_opencode_save_keeps_target_agents_when_ui_has_none() {
    // 回归：加载 pi / omp / DSH 后保存到 opencode，界面没有 agents 数据，
    // 目标文件里已有的 agents 不得被清空（修复前会被整体删除）
    let mut target = json!({
        "provider": { "old": {} },
        "agent": { "writer": {}, "reviewer": {} },
        "mcp": { "srv": { "command": "node" } }
    });
    strip_cross_format_containers(ConfigFormat::Opencode, &mut target, false);
    let providers = ui_providers(&["b", "a"]);
    let doc = merge_opencode_root(&target, &[], &providers);
    assert!(doc["agent"]["writer"].is_object(), "writer 必须保留");
    assert!(doc["agent"]["reviewer"].is_object(), "reviewer 必须保留");
    assert!(
        doc["provider"].get("old").is_none(),
        "provider 仍由界面接管（干净转换语义不变）"
    );
    assert_eq!(doc["mcp"]["srv"]["command"], "node");
}
