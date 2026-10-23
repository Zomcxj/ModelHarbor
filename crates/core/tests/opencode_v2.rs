//! opencode v2 schema read/write regression: given v1 update v1, given v2 update v2, cross-version converts to the target.
//!
//! Fixtures are synthetic (structure matching real samples, keys fakeged).

use model_harbor_core::backends;
use model_harbor_core::backends::opencode::detect_schema_version;
use model_harbor_core::convert::package_or_npm_to_api;
use model_harbor_core::format::ConfigFormat;
use model_harbor_core::model::{
    package_full_name, package_suffix, AgentRow, ModelRow, OpencodeSchemaVersion, ProviderRow,
};
use serde_json::{json, Value};

/// Synthetic v1 content: top-level singular keys, npm / options, modalities / reasoning, a variants object.
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

/// Synthetic v2 content: top-level plural keys, package / settings, capabilities, a variants array,
/// and the agent's `model#variant` with `request.body.temperature`.
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
    // An empty file counts as v1 (no v2 evidence at all).
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
    // v2 has no reasoning key: treated as checked by default (user decision), landing true when crossing to a v1 target.
    assert!(m.reasoning);
    assert_eq!(m.modalities_input, "text");
    assert_eq!(m.modalities_output, "text");
    assert_eq!(m.variants, "medium, high");
    assert!(!m.store);
    // v2's package names must derive protocols too.
    assert_eq!(p.effective_api(), "openai-completions");
}

#[test]
fn v2_round_trip_is_lossless() {
    let original: Value = serde_json::from_str(&v2_content()).unwrap();
    let load = backend().parse(&v2_content()).expect("解析失败");
    let root = backend().serialize_root(&load.agents, &load.providers, &load.extras, None);
    // With nothing modified: the agent / provider tables stay fully faithful (key order and nesting included).
    assert_eq!(root["agents"], original["agents"]);
    assert_eq!(root["providers"], original["providers"]);
    // Other top-level fields preserved verbatim.
    assert_eq!(root["mcp"], original["mcp"]);
    assert_eq!(root["plugins"], original["plugins"]);
}

#[test]
fn v2_edits_write_back_in_v2_shape() {
    let load = backend().parse(&v2_content()).expect("解析失败");
    let mut agents = load.agents.clone();
    let mut providers = load.providers.clone();

    // Edit the agent: variant high → medium, temperature 0.3 → 0.7.
    agents[0].variant = "medium".into();
    agents[0].temperature = "0.7".into();
    // Edit the model: tool_call off, tiers cleared.
    providers[0].models[0].tool_call = false;
    providers[0].models[0].variants = String::new();
    // Edit the provider key.
    providers[0].api_key = "sk-rotated".into();

    let root = backend().serialize_root(&agents, &providers, &load.extras, None);

    let a = &root["agents"]["writing"];
    assert_eq!(a["model"], json!("sensenova/flash#medium"));
    assert_eq!(a["request"]["body"]["temperature"], json!(0.7));
    // v2 has no standalone variant / temperature top-level keys.
    assert!(a.get("variant").is_none());
    assert!(a.get("temperature").is_none());

    let p = &root["providers"]["sensenova"];
    assert_eq!(p["settings"]["apiKey"], json!("sk-rotated"));
    assert!(p.get("npm").is_none());
    assert!(p.get("options").is_none());

    let m = &p["models"]["flash"];
    assert_eq!(m["capabilities"]["tools"], json!(false));
    // Tiers cleared: the block is removed whole.
    assert!(m.get("variants").is_none());
    // v2 never writes reasoning.
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
    // The v1 shape preserved verbatim: npm / options / modalities / reasoning / the variants object.
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
    // v2 writes no reasoning.
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
    // A v2 source's reasoning defaults checked: crossing to v1 lands true (matching the target file's own value).
    assert_eq!(m["reasoning"], json!(true));
}

#[test]
fn new_rows_saved_into_v2_file_get_v2_shape() {
    // The target file is already v2 (plural keys + a package provider); UI-created rows default to the v1 shape,
    // and the disk write must convert to v2 following the target file.
    let mut extras: Value = serde_json::from_str(&v2_content()).unwrap();
    // Clear the tables, simulating a v2 file with no entries yet.
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
    // The UI dropdown writes v1 package names; they translate to v2 package names before hitting disk.
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
    // A new row's reasoning=true must not enter v2 either.
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
    // Unknown suffixes fall back to the compatibility layer.
    assert_eq!(
        package_or_npm_to_api("@opencode/ai/providers/custom-thing"),
        "openai-completions"
    );
    // v1 package names still go through npm_to_api.
    assert_eq!(
        package_or_npm_to_api("@ai-sdk/anthropic"),
        "anthropic-messages"
    );
    assert_eq!(package_or_npm_to_api(""), "openai-completions");
}

/// A package name's last segment: v1 / v2 full names agree on it; custom packages pass through.
#[test]
fn package_suffix_strips_known_prefixes() {
    assert_eq!(
        package_suffix("@ai-sdk/openai-compatible"),
        "openai-compatible"
    );
    assert_eq!(
        package_suffix("@opencode/ai/providers/openai-compatible"),
        "openai-compatible"
    );
    assert_eq!(package_suffix("@ai-sdk/anthropic"), "anthropic");
    assert_eq!(package_suffix("@opencode/ai/providers/google"), "google");
    // No slash / custom package: returned as-is.
    assert_eq!(package_suffix("custom-pkg"), "custom-pkg");
    assert_eq!(package_suffix(""), "");
}

/// Last segment + version → full name: v1 uses @ai-sdk/, v2 uses @opencode/ai/providers/.
#[test]
fn package_full_name_follows_schema() {
    assert_eq!(
        package_full_name("anthropic", OpencodeSchemaVersion::V1),
        "@ai-sdk/anthropic"
    );
    assert_eq!(
        package_full_name("anthropic", OpencodeSchemaVersion::V2),
        "@opencode/ai/providers/anthropic"
    );
    assert_eq!(
        package_full_name("openai-compatible", OpencodeSchemaVersion::V2),
        "@opencode/ai/providers/openai-compatible"
    );
    // An empty segment returns an empty string (= no package specified).
    assert_eq!(package_full_name("", OpencodeSchemaVersion::V2), "");
}

/// Round trip: assembling the full name from the segment then taking the segment matches the original (both versions).
#[test]
fn package_suffix_full_name_round_trip() {
    for suffix in [
        "openai",
        "anthropic",
        "google",
        "openai-compatible",
        "mistral",
    ] {
        for schema in [OpencodeSchemaVersion::V1, OpencodeSchemaVersion::V2] {
            let full = package_full_name(suffix, schema);
            assert_eq!(package_suffix(&full), suffix, "{suffix} @ {schema:?}");
        }
    }
}

/// v2 is opencode's own evolution: kilo / mimo always write v1 to disk, even when the target content
/// mixed in v2 fingerprints (like a hand-copied opencode v2 section).
/// The row's schema records raw's real shape (possibly v2); the write path's target_version clamps the
/// conversion.
#[test]
fn kilocode_and_mimocode_always_write_v1_even_with_v2_content() {
    let v2_content = r#"{
        "provider": {
            "p1": {
                "package": "@opencode/ai/providers/anthropic",
                "settings": { "baseURL": "https://x/v1", "apiKey": "sk" },
                "models": {
                    "m1": {
                        "name": "m1",
                        "capabilities": { "tools": true, "input": ["text"], "output": ["text"] },
                        "limit": { "context": 100 }
                    }
                }
            }
        }
    }"#;
    let extras: Value = serde_json::from_str(v2_content).unwrap();
    for id in [ConfigFormat::Kilocode, ConfigFormat::Mimocode] {
        let backend = backends::backend(id);
        let load = backend.parse(v2_content).expect("解析成功");
        // The write-back is forced into the v1 shape: npm (translated back to @ai-sdk/*), options, tool_call —
        // never package / settings / capabilities.
        let root = backend.serialize_root(&[], &load.providers, &extras, None);
        let p = &root["provider"]["p1"];
        assert_eq!(p["npm"], json!("@ai-sdk/anthropic"), "{id:?} 写回 v1 包名");
        assert!(p.get("package").is_none(), "{id:?} 不写 package");
        assert!(p.get("settings").is_none(), "{id:?} 不写 settings");
        assert_eq!(
            p["options"]["baseURL"],
            json!("https://x/v1"),
            "{id:?} baseURL 落在 options"
        );
        let m = &p["models"]["m1"];
        assert!(m.get("capabilities").is_none(), "{id:?} 不写 capabilities");
        assert_eq!(m["tool_call"], json!(true), "{id:?} 工具调用落 v1 键");
        assert_eq!(
            m["modalities"]["input"],
            json!(["text"]),
            "{id:?} 模态落 v1 键"
        );
    }
}

/// opencode itself is unaffected by the clamp: the same v2 content still parses and writes back as v2.
#[test]
fn opencode_itself_still_follows_v2_content() {
    let v2_content = r#"{
        "provider": {
            "p1": {
                "package": "@opencode/ai/providers/anthropic",
                "settings": { "baseURL": "https://x/v1" },
                "models": { "m1": { "capabilities": { "tools": true } } }
            }
        }
    }"#;
    let extras: Value = serde_json::from_str(v2_content).unwrap();
    let backend = backends::backend(ConfigFormat::Opencode);
    let load = backend.parse(v2_content).expect("解析成功");
    assert_eq!(load.providers[0].schema, OpencodeSchemaVersion::V2);
    // extras holds v2 content → the write-back follows v2.
    let root = backend.serialize_root(&[], &load.providers, &extras, None);
    let p = &root["provider"]["p1"];
    assert_eq!(
        p["package"],
        json!("@opencode/ai/providers/anthropic"),
        "opencode 本家 v2 写回不受影响"
    );
    assert!(p.get("npm").is_none(), "v2 写回不该出现 npm");
}
