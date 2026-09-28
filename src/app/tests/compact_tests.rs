use crate::app::serialize::{compact_json, pretty_json, serialize_object, CompactRole};

#[test]
fn pretty_agent_fields_combine_on_same_line() {
    let root = serde_json::json!({
        "agent": {
            "writing": {
                "mode": "subagent",
                "description": "写技术文档",
                "model": "sensenova/sensenova-6.8-flash-lite",
                "variant": "high",
                "temperature": 0.3,
                "color": "info",
                "system": "负责文档"
            }
        }
    });
    let output = pretty_json(&root);
    println!("=== PRETTY OUTPUT ===\n{}", output);
    // model, variant, temperature, color should be on the same line
    assert!(
        output
            .contains("\"model\": \"sensenova/sensenova-6.8-flash-lite\", \"variant\": \"high\","),
        "agent model+variant should combine:\n{}",
        output
    );
    assert!(
        output.contains("\"temperature\": 0.3, \"color\": \"info\", \"system\": \"负责文档\""),
        "agent temperature+color+system should combine:\n{}",
        output
    );
}

#[test]
fn compact_variants_preserve_values() {
    let value = serde_json::json!({
        "variants": {
            "medium": { "reasoningEffort": "medium" },
            "high": { "reasoningEffort": "high" }
        }
    });
    let output = serialize_object(value.as_object().unwrap(), 1, CompactRole::Target);
    assert!(output.contains(
            "\"variants\": { \"medium\": {\"reasoningEffort\":\"medium\"}, \"high\": {\"reasoningEffort\":\"high\"} }"
        ));
}

#[test]
fn pi_models_array_expands_to_multiple_lines() {
    let root = serde_json::json!({
        "providers": {
            "openai": {
                "api": "openai-completions",
                "models": [
                    { "id": "gpt-4o", "name": "GPT-4o" },
                    { "id": "gpt-4o-mini", "name": "GPT-4o mini" }
                ]
            }
        }
    });
    let pretty = pretty_json(&root);
    assert!(pretty.contains("\"models\": [\n"));
    assert!(pretty.contains("\"id\": \"gpt-4o\", \"name\": \"GPT-4o\""));
    assert!(pretty.contains("\"id\": \"gpt-4o-mini\", \"name\": \"GPT-4o mini\""));

    let compact = compact_json(&root);
    assert!(compact.contains("\"models\": [\n"));
    assert!(compact.contains("\"id\": \"gpt-4o\", \"name\": \"GPT-4o\""));
    assert!(compact.contains("\"id\": \"gpt-4o-mini\", \"name\": \"GPT-4o mini\""));
}

#[test]
fn compact_targets_agent_fields_only() {
    let root = serde_json::json!({
        "agent": { "writer": { "mode": "subagent", "description": "text", "model": "p/m" } },
        "other": { "a": 1, "b": 2 }
    });
    let output = compact_json(&root);
    assert!(output.contains("\"writer\": {"));
    assert!(output.contains("\"mode\": \"subagent\", \"description\": \"text\""));
    assert!(output.contains("\"other\": {\n    \"a\": 1,"));
}

#[test]
fn compact_targets_provider_model_fields() {
    let root = serde_json::json!({
        "provider": {
            "p": {
                "models": {
                    "m": {
                        "name": "Model",
                        "reasoning": true,
                        "variants": { "medium": {}, "high": {} }
                    }
                }
            }
        }
    });
    let output = compact_json(&root);
    assert!(output.contains("\"models\": {\"m\":{"));
    assert!(output.contains("\"name\":\"Model\",\"reasoning\":true"));
    assert!(output.contains("\"variants\":{\"medium\":{},\"high\":{}}"));
}

#[test]
fn compact_wraps_mcp_and_options_from_second_level() {
    let root = serde_json::json!({
        "mcp": {
            "server": {
                "command": "node",
                "args": ["server.js"],
                "enabled": true
            }
        },
        "provider": {
            "p": {
                "options": {
                    "baseURL": "https://example.com/v1",
                    "apiKey": "sk-test",
                    "timeout": 30000
                }
            }
        }
    });
    let output = compact_json(&root);

    assert!(output
        .contains("\"server\": {\"command\":\"node\",\"args\":[\"server.js\"],\"enabled\":true"));
    assert!(output.contains("\"options\": {\"baseURL\":\"https://example.com/v1\",\"apiKey\":\"sk-test\",\"timeout\":30000"));
}

#[test]
fn default_format_keeps_nested_leaf_objects_on_one_line() {
    let root = serde_json::json!({
        "provider": {
            "p": {
                "options": { "baseURL": "https://example.com/v1", "timeout": 30000 }
            }
        }
    });

    let output = compact_json(&root);

    assert!(
        output.contains("\"options\": {\"baseURL\":\"https://example.com/v1\",\"timeout\":30000}")
    );
    assert!(!output.contains("\"options\": {\n"));
}

#[test]
fn save_serializers_preserve_variant_settings() {
    let root = serde_json::json!({
        "provider": {
            "p": {
                "models": {
                    "m": {
                        "variants": { "high": { "reasoningEffort": "high" } }
                    }
                }
            }
        }
    });

    assert!(compact_json(&root).contains("reasoningEffort"));
    assert!(compact_json(&root).contains("\"high\":{\"reasoningEffort\":\"high\"}"));
}
