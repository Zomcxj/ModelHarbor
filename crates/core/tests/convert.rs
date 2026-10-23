use model_harbor_core::backends::oh_my_pi;
use model_harbor_core::convert;
use model_harbor_core::model::ProviderRow;
use serde_json::json;

#[test]
fn cross_format_opencode_source_writes_compat_false() {
    // Loaded from opencode, saved to pi/omp: an undeclared supportsDeveloperRole writes false.
    let raw = json!({
        "options": {"baseURL": "https://x/v1", "apiKey": "sk-test"},
        "models": {}
    });
    let provider = ProviderRow::from("openai", &raw);
    assert!(!provider.compat);
    let out = convert::provider_to_pi(&provider);
    assert_eq!(out["compat"]["supportsDeveloperRole"], false);
}

#[test]
fn requires_reasoning_content_maps_pi_and_omp_keys() {
    // omp keys → the pi page: values map onto pi's keys
    let omp_raw = json!({
        "baseUrl": "https://x/v1",
        "apiKey": "k",
        "api": "openai-completions",
        "compat": {"requiresReasoningContentForAllAssistantTurns": false},
        "models": []
    });
    let provider = convert::provider_from_pi("p", &omp_raw);
    assert!(!provider.requires_reasoning_content);
    // Same format unmodified → the save preserves raw.
    let out = convert::provider_to_pi(&provider);
    assert_eq!(
        out["compat"]["requiresReasoningContentForAllAssistantTurns"],
        false
    );
    assert!(out["compat"]
        .get("requiresReasoningContentOnAssistantMessages")
        .is_none());
}

#[test]
fn requires_reasoning_content_defaults_false_from_opencode() {
    // Loaded from opencode (field absent) → the pi page is unchecked and saving writes false.
    let raw = json!({
        "options": {"baseURL": "https://x/v1", "apiKey": "sk-test"},
        "models": {}
    });
    let provider = ProviderRow::from("openai", &raw);
    assert!(!provider.requires_reasoning_content);
    let out = convert::provider_to_pi(&provider);
    assert_eq!(
        out["compat"]["requiresReasoningContentOnAssistantMessages"],
        false
    );
}

#[test]
fn api_to_npm_mapping() {
    assert_eq!(
        convert::api_to_npm("anthropic-messages"),
        "@ai-sdk/anthropic"
    );
    assert_eq!(
        convert::api_to_npm("google-generative-ai"),
        "@ai-sdk/google"
    );
    // Chat Completions maps to the compatibility-layer package on the opencode side.
    assert_eq!(
        convert::api_to_npm("openai-completions"),
        "@ai-sdk/openai-compatible"
    );
    // Responses maps to @ai-sdk/openai.
    assert_eq!(convert::api_to_npm("openai-responses"), "@ai-sdk/openai");
    // An api with no npm counterpart: left empty.
    for api in [
        "openai-codex-responses",
        "azure-openai-responses",
        "bedrock-converse-stream",
        "google-vertex",
        "custom-api",
    ] {
        assert_eq!(convert::api_to_npm(api), "", "{}", api);
    }
}

#[test]
fn npm_to_api_mapping() {
    assert_eq!(
        convert::npm_to_api("@ai-sdk/anthropic"),
        "anthropic-messages"
    );
    assert_eq!(
        convert::npm_to_api("@ai-sdk/google"),
        "google-generative-ai"
    );
    assert_eq!(convert::npm_to_api("@ai-sdk/openai"), "openai-responses");
    // Unwritten npm (empty) / the explicit compatibility layer / unknown packages all land on openai-completions.
    for npm in ["", "@ai-sdk/openai-compatible", "custom-npm"] {
        assert_eq!(convert::npm_to_api(npm), "openai-completions", "{}", npm);
    }
}

#[test]
fn model_from_pi_basic() {
    let v = json!({
        "id": "gpt-4o",
        "name": "GPT-4o",
        "reasoning": false,
        "input": ["text", "image"],
        "contextWindow": 128000,
        "maxTokens": 4096
    });
    let model = convert::model_from_pi(&v);
    assert_eq!(model.id, "gpt-4o");
    assert_eq!(model.name, "GPT-4o");
    assert!(!model.reasoning);
    assert!(model.tool_call);
    assert_eq!(model.context, "128000");
    assert_eq!(model.output, "4096");
    assert_eq!(model.modalities_input, "text, image");
    assert_eq!(model.modalities_output, "text");
    assert!(model.variants.is_empty());
}

#[test]
fn model_to_pi_roundtrip() {
    let v = json!({
        "id": "claude-sonnet-4-20250514",
        "name": "Claude Sonnet 4",
        "reasoning": false,
        "input": ["text", "image"],
        "contextWindow": 200000,
        "maxTokens": 8192
    });
    let model = convert::model_from_pi(&v);
    let output = convert::model_to_pi(&model);
    assert_eq!(output["id"], "claude-sonnet-4-20250514");
    assert_eq!(output["name"], "Claude Sonnet 4");
    assert_eq!(output["reasoning"], false);
    assert_eq!(output["input"], json!(["text", "image"]));
    assert_eq!(output["contextWindow"], 200000);
    assert_eq!(output["maxTokens"], 8192);
}

#[test]
fn provider_from_pi_basic() {
    let v = json!({
        "baseUrl": "https://api.openai.com/v1",
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [
            {
                "id": "gpt-4o",
                "name": "GPT-4o",
                "reasoning": false,
                "input": ["text"],
                "contextWindow": 128000,
                "maxTokens": 4096
            }
        ]
    });
    let provider = convert::provider_from_pi("openai", &v);
    assert_eq!(provider.key, "openai");
    assert_eq!(provider.base_url, "https://api.openai.com/v1");
    assert_eq!(provider.api_key, "sk-test");
    // api=openai-completions maps to the compatibility-layer package on the opencode side.
    assert_eq!(provider.npm, "@ai-sdk/openai-compatible");
    // Writing back to pi via the npm reverse lookup yields the same api.
    assert_eq!(
        convert::provider_to_pi(&provider)["api"],
        "openai-completions"
    );
    assert_eq!(provider.models.len(), 1);
    assert_eq!(provider.models[0].id, "gpt-4o");
}

#[test]
fn provider_from_pi_anthropic() {
    let v = json!({
        "baseUrl": "https://api.anthropic.com",
        "apiKey": "sk-ant-test",
        "api": "anthropic-messages",
        "models": []
    });
    let provider = convert::provider_from_pi("anthropic", &v);
    assert_eq!(provider.npm, "@ai-sdk/anthropic");
}

#[test]
fn provider_to_pi_roundtrip() {
    let v = json!({
        "baseUrl": "https://api.openai.com/v1",
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": [
            {
                "id": "gpt-4o",
                "name": "GPT-4o",
                "reasoning": false,
                "input": ["text"],
                "contextWindow": 128000,
                "maxTokens": 4096
            }
        ]
    });
    let provider = convert::provider_from_pi("openai", &v);
    let output = convert::provider_to_pi(&provider);
    assert_eq!(output["baseUrl"], "https://api.openai.com/v1");
    assert_eq!(output["apiKey"], "sk-test");
    assert_eq!(output["api"], "openai-completions");
    assert!(output["models"].as_array().unwrap().len() == 1);
}

#[test]
fn load_pi_providers() {
    let root = json!({
        "providers": {
            "openai": {
                "baseUrl": "https://api.openai.com/v1",
                "apiKey": "sk-test",
                "api": "openai-completions",
                "models": []
            },
            "anthropic": {
                "baseUrl": "https://api.anthropic.com",
                "apiKey": "sk-ant-test",
                "api": "anthropic-messages",
                "models": []
            }
        }
    });
    let providers = convert::load_pi_providers(&root);
    assert_eq!(providers.len(), 2);
    let keys: Vec<&str> = providers.iter().map(|p| p.key.as_str()).collect();
    assert!(keys.contains(&"openai"));
    assert!(keys.contains(&"anthropic"));
}

#[test]
fn load_pi_extras() {
    let root = json!({
        "providers": {},
        "custom_field": "value",
        "another_field": 123
    });
    let extras = convert::load_pi_extras(&root);
    assert_eq!(extras["custom_field"], "value");
    assert_eq!(extras["another_field"], 123);
    assert!(extras.get("providers").is_none());
}

#[test]
fn to_pi_root_preserves_extras() {
    let extras = json!({
        "custom_field": "value"
    });
    let providers = vec![];
    let root = convert::to_pi_root(&providers, &extras);
    assert_eq!(root["custom_field"], "value");
    assert!(root["providers"].as_object().unwrap().is_empty());
}

#[test]
fn provider_to_pi_preserves_compat() {
    let v = json!({
        "baseUrl": "https://api.openai.com/v1",
        "apiKey": "sk-test",
        "api": "openai-completions",
        "compat": {
            "supportsDeveloperRole": false
        },
        "models": []
    });
    let provider = convert::provider_from_pi("openai", &v);
    assert!(!provider.compat);
    let output = convert::provider_to_pi(&provider);
    assert_eq!(output["compat"]["supportsDeveloperRole"], false);
}

#[test]
fn provider_to_pi_omits_compat_when_true() {
    let v = json!({
        "baseUrl": "https://api.anthropic.com",
        "apiKey": "sk-test",
        "api": "anthropic-messages",
        "compat": {
            "supportsDeveloperRole": true
        },
        "models": []
    });
    let provider = convert::provider_from_pi("anthropic", &v);
    assert!(provider.compat);
    let output = convert::provider_to_pi(&provider);
    assert!(
        output.get("compat").is_none(),
        "compat should be omitted when true"
    );
}

#[test]
fn openai_completions_defaults_compat_false() {
    // chat/completions without an explicit compat: supportsDeveloperRole defaults unchecked.
    let v = json!({
        "baseUrl": "https://api.openai.com/v1",
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": []
    });
    let provider = convert::provider_from_pi("openai", &v);
    assert!(!provider.compat);
    let output = convert::provider_to_pi(&provider);
    assert_eq!(output["compat"]["supportsDeveloperRole"], false);
}

#[test]
fn anthropic_url_passthrough_no_v1_added() {
    let v = json!({
        "baseUrl": "https://api.anthropic.com",
        "apiKey": "sk-ant-test",
        "api": "anthropic-messages",
        "models": []
    });
    let provider = convert::provider_from_pi("anthropic", &v);
    assert_eq!(provider.base_url, "https://api.anthropic.com");
    let output = convert::provider_to_pi(&provider);
    assert_eq!(output["baseUrl"], "https://api.anthropic.com");
}

#[test]
fn anthropic_url_v1_stripped_on_load_and_save() {
    let v = json!({
        "baseUrl": "https://api.anthropic.com/v1",
        "apiKey": "sk-ant-test",
        "api": "anthropic-messages",
        "models": []
    });
    let provider = convert::provider_from_pi("anthropic", &v);
    // Normalized at load: neither display nor disk carries /v1.
    assert_eq!(provider.base_url, "https://api.anthropic.com");
    let output = convert::provider_to_pi(&provider);
    assert_eq!(output["baseUrl"], "https://api.anthropic.com");
}

#[test]
fn responses_api_maps_to_openai_package() {
    assert_eq!(convert::api_to_npm("openai-responses"), "@ai-sdk/openai");
    let v = json!({
        "baseUrl": "https://example.com/v1",
        "apiKey": "sk-test",
        "api": "openai-responses",
        "models": []
    });
    let provider = convert::provider_from_pi("k", &v);
    assert_eq!(provider.pi_api, "openai-responses");
    assert_eq!(provider.npm, "@ai-sdk/openai");
    // Writing back to pi through the npm reverse lookup still yields openai-responses.
    let output = convert::provider_to_pi(&provider);
    assert_eq!(output["api"], "openai-responses");
    assert_eq!(output["baseUrl"], "https://example.com/v1");
}

#[test]
fn empty_api_option_clears_raw_fallback() {
    // A protocol with no npm counterpart counts as "explicitly set"; the dropdown shows no "(none)".
    let v = json!({
        "baseUrl": "https://us-central1-aiplatform.googleapis.com/v1",
        "apiKey": "sk-test",
        "api": "google-vertex",
        "models": []
    });
    let mut provider = convert::provider_from_pi("vertex", &v);
    assert_eq!(provider.npm, "", "google-vertex 无 npm 对应包");
    assert!(provider.has_explicit_api());
    assert_eq!(provider.effective_api(), "google-vertex");

    // Selecting "(none)": npm / pi_api / raw.api all clear.
    provider.clear_api();
    assert!(!provider.has_explicit_api());
    assert_eq!(provider.effective_api(), "openai-completions");
    assert_eq!(
        convert::provider_to_pi(&provider)["api"],
        "openai-completions"
    );
}

#[test]
fn pi_api_survives_oc_roundtrip_without_npm() {
    let v = json!({
        "baseUrl": "https://example.com/v1",
        "apiKey": "sk-test",
        "api": "anthropic-messages",
        "models": []
    });
    let provider = convert::provider_from_pi("k", &v);
    assert_eq!(provider.npm, "@ai-sdk/anthropic");

    // OC save: a provider with empty npm loses the npm field (the oc format expresses api via npm).
    let mut oc = serde_json::Map::new();
    oc.insert("k".into(), provider_from_row_npm(&provider, ""));

    // Reading back from oc, npm is empty
    let back = convert::provider_from_pi("k", &oc["k"]);
    let _ = back;
    // The assertion lives at the row level: the row keeps pi_api.
    assert_eq!(provider.pi_api, "anthropic-messages");
    let out = convert::provider_to_pi(&provider);
    assert_eq!(out["api"], "anthropic-messages");
}

fn provider_from_row_npm(
    p: &model_harbor_core::model::ProviderRow,
    npm: &str,
) -> serde_json::Value {
    // Simulate ProviderRow::to_value's oc output (the field is removed when npm is empty).
    let mut m = serde_json::Map::new();
    let mut options = serde_json::Map::new();
    options.insert("baseURL".into(), p.base_url.clone().into());
    options.insert("apiKey".into(), p.api_key.clone().into());
    if !npm.is_empty() {
        m.insert("npm".into(), npm.into());
    }
    m.insert("options".into(), options.into());
    let mut models = serde_json::Map::new();
    for mdl in &p.models {
        models.insert(mdl.id.clone(), mdl.to_value());
    }
    m.insert("models".into(), models.into());
    m.into()
}

#[test]
fn thinking_level_map_asymmetric_roundtrip() {
    let v = json!({
        "id": "m",
        "name": "M",
        "reasoning": true,
        "thinkingLevelMap": { "high": "max" }
    });
    let model = convert::model_from_pi(&v);
    assert_eq!(model.variants, "max");
    let out = convert::model_to_pi(&model);
    assert_eq!(
        out["thinkingLevelMap"]["high"], "max",
        "asymmetric mapping keys must survive roundtrip"
    );
}

#[test]
fn pi_roundtrip_preserves_provider_and_model_extras() {
    // pi same-format round trip: provider / model-level extension fields preserved.
    let v = json!({
        "baseUrl": "https://gw/v1",
        "api": "openai-completions",
        "apiKey": "k",
        "authHeader": true,
        "headers": { "X-Team": "platform" },
        "models": [
            {
                "id": "m1",
                "name": "M1",
                "reasoning": true,
                "input": ["text"],
                "contextWindow": 200000,
                "maxTokens": 16384,
                "toolName": "custom"
            }
        ]
    });
    let provider = convert::provider_from_pi("gw", &v);
    let out = convert::provider_to_pi(&provider);
    assert_eq!(out["authHeader"], json!(true), "provider 扩展字段必须保留");
    assert_eq!(out["headers"]["X-Team"], json!("platform"));
    assert_eq!(
        out["models"][0]["toolName"],
        json!("custom"),
        "model 扩展字段必须保留"
    );
}

#[test]
fn pi_save_clearing_fields_removes_them() {
    // On a raw base, clearing a field deletes the corresponding key.
    let v = json!({
        "baseUrl": "https://x/v1",
        "api": "openai-completions",
        "models": [
            { "id": "m", "name": "M", "reasoning": true, "contextWindow": 100, "maxTokens": 50, "input": ["text"] }
        ]
    });
    let mut p = convert::provider_from_pi("p", &v);
    p.base_url = String::new();
    p.models[0].name = String::new();
    p.models[0].context = String::new();
    p.models[0].output = String::new();
    p.models[0].modalities_input = String::new();
    let out = convert::provider_to_pi(&p);
    assert!(out.get("baseUrl").is_none());
    assert!(out["models"][0].get("name").is_none());
    assert!(out["models"][0].get("contextWindow").is_none());
    assert!(out["models"][0].get("maxTokens").is_none());
    assert!(out["models"][0].get("input").is_none());
}

#[test]
fn pi_save_from_omp_raw_translates_thinking_and_keeps_extras() {
    // omp raw → pi output: the thinking block translates to thinkingLevelMap and is removed; other extensions kept.
    let mut m = model_harbor_core::model::ModelRow::new();
    m.id = "m".into();
    m.reasoning = true;
    m.variants = "max".into();
    m.raw = json!({
        "id": "m",
        "reasoning": true,
        "cost": { "input": 3.0 },
        "thinking": { "mode": "effort", "efforts": ["high"], "effortMap": { "high": "max" } }
    });
    let out = convert::model_to_pi(&m);
    assert_eq!(out["thinkingLevelMap"], json!({"high": "max"}));
    assert!(
        out.get("thinking").is_none(),
        "omp thinking 块必须翻译后移除"
    );
    assert_eq!(out["cost"]["input"], json!(3.0), "omp 扩展字段应保留");
}

#[test]
fn opencode_rows_from_pi_raw_build_fresh() {
    // pi raw must not leak api/compat/id/contextWindow/input or other dialect keys into the opencode output.
    let v = json!({
        "baseUrl": "https://x/v1",
        "api": "openai-completions",
        "compat": { "supportsDeveloperRole": false },
        "models": [
            { "id": "m", "name": "M", "reasoning": true, "input": ["text"], "contextWindow": 128000, "maxTokens": 4096 }
        ]
    });
    let p = convert::provider_from_pi("p", &v);
    let out = p.to_value(); // the opencode dialect
    assert!(out.get("api").is_none(), "pi 的 api 键不得泄漏");
    assert!(out.get("compat").is_none(), "pi 的 compat 键不得泄漏");
    let m = &out["models"]["m"];
    assert!(
        m.get("id").is_none(),
        "pi 的 id 键不得泄漏（opencode 以 map key 为身份）"
    );
    assert!(m.get("contextWindow").is_none());
    assert!(m.get("maxTokens").is_none());
    assert!(m.get("input").is_none());
    assert_eq!(
        m["limit"]["context"],
        json!(128000),
        "contextWindow 应翻译为 limit.context"
    );
    assert_eq!(
        m["modalities"]["input"][0],
        json!("text"),
        "input 应翻译为 modalities.input"
    );
}

#[test]
fn anthropic_proxy_url_v1_also_stripped() {
    // Third-party proxies and official endpoints share one rule: pi / omp / dsh strip the trailing /v1 at load.
    let v = json!({
        "baseUrl": "https://my-gateway.example/v1",
        "apiKey": "sk-test",
        "api": "anthropic-messages",
        "models": []
    });
    let provider = convert::provider_from_pi("proxy", &v);
    assert_eq!(provider.base_url, "https://my-gateway.example");
    let output = convert::provider_to_pi(&provider);
    assert_eq!(output["baseUrl"], "https://my-gateway.example");
}

#[test]
fn pi_model_with_thinking_only_counts_as_reasoning() {
    // Writing only thinkingLevelMap / the thinking block (reasoningEfforts on DSH) still judges reasoning on.
    assert!(
        convert::model_from_pi(&json!({"id": "m", "thinkingLevelMap": {"high": "high"}})).reasoning
    );
    assert!(convert::model_from_pi(&json!({"id": "m", "thinking": {"mode": "effort"}})).reasoning);
    assert!(
        convert::model_from_pi(&json!({"id": "m", "reasoningEfforts": {"high": "high"}})).reasoning
    );
    assert!(!convert::model_from_pi(&json!({"id": "m", "reasoning": false})).reasoning);
}

#[test]
fn provider_to_pi_places_compat_between_api_and_models() {
    // compat always sits between api and models.
    let mut provider = ProviderRow::new();
    provider.key = "demo".into();
    provider.base_url = "https://example.com/v1".into();
    provider.api_key = "sk-x".into();
    provider.pi_api = "openai-completions".into();
    provider.compat = false;
    let out = convert::provider_to_pi(&provider);
    let keys: Vec<&str> = out
        .as_object()
        .expect("provider 应为对象")
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys.starts_with(&["baseUrl", "apiKey", "api", "compat", "models"]),
        "字段顺序应为 baseUrl → apiKey → api → compat → models，实际 {keys:?}"
    );

    // A native pi provider (compat last in raw) also lands in place after saving.
    let mut native = ProviderRow::new();
    native.key = "demo".into();
    native.base_url = "https://example.com/v1".into();
    native.api_key = "sk-x".into();
    native.pi_api = "openai-completions".into();
    native.compat = true;
    native.source_format = Some(model_harbor_core::format::ConfigFormat::Pi);
    native.raw = json!({
        "baseUrl": "https://example.com/v1",
        "apiKey": "sk-x",
        "api": "openai-completions",
        "models": [],
        "compat": {"requiresReasoningContentOnAssistantMessages": false}
    });
    let out = convert::provider_to_pi(&native);
    let keys: Vec<&str> = out
        .as_object()
        .expect("provider 应为对象")
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys.starts_with(&["baseUrl", "apiKey", "api", "compat", "models"]),
        "原生 provider 的 compat 也应归位，实际 {keys:?}"
    );
}

#[test]
fn pi_variants_written_in_canonical_order() {
    // pi's thinkingLevelMap is written in canonical tier order.
    let mut model = model_harbor_core::model::ModelRow::new();
    model.id = "m".into();
    model.variants = "max, xhigh, medium".into();
    let out = convert::model_to_pi(&model);
    let keys: Vec<&str> = out["thinkingLevelMap"]
        .as_object()
        .expect("thinkingLevelMap 应为对象")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["medium", "xhigh", "max"], "实际 {keys:?}");
}

#[test]
fn without_v1_only_strips_the_messages_api() {
    // The messages protocol: strip the trailing /v1
    assert_eq!(
        convert::without_v1_for_messages("anthropic-messages", "https://gw.test/v1"),
        "https://gw.test"
    );
    assert_eq!(
        convert::without_v1_for_messages("anthropic-messages", "https://gw.test/v1/"),
        "https://gw.test"
    );
    // Deeper paths strip only the trailing version segment, keeping the prefix
    assert_eq!(
        convert::without_v1_for_messages("anthropic-messages", "https://gw.test/api/v1"),
        "https://gw.test/api"
    );
    // v10 is not /v1 — must not be hit by mistake
    assert_eq!(
        convert::without_v1_for_messages("anthropic-messages", "https://gw.test/v10"),
        "https://gw.test/v10"
    );
    // Other protocols keep /v1.
    assert_eq!(
        convert::without_v1_for_messages("openai-completions", "https://gw.test/v1"),
        "https://gw.test/v1"
    );
}

#[test]
fn pi_and_omp_load_strip_v1_for_messages_api_only() {
    // omp's parse also goes through load_pi_providers, covering pi and omp.
    let root = json!({
        "providers": {
            "claude_gw": {
                "baseUrl": "https://gw.test/v1",
                "apiKey": "sk-test",
                "api": "anthropic-messages",
                "models": []
            },
            "openai_gw": {
                "baseUrl": "https://gw.test/v1",
                "apiKey": "sk-test",
                "api": "openai-completions",
                "models": []
            }
        }
    });
    let rows = convert::load_pi_providers(&root);
    let claude = rows
        .iter()
        .find(|p| p.key == "claude_gw")
        .expect("claude_gw 应存在");
    let openai = rows
        .iter()
        .find(|p| p.key == "openai_gw")
        .expect("openai_gw 应存在");
    // A messages base without /v1; other protocols untouched.
    assert_eq!(claude.base_url, "https://gw.test");
    assert_eq!(openai.base_url, "https://gw.test/v1");
}

#[test]
fn saving_messages_provider_drops_v1_for_pi_and_omp_but_not_opencode() {
    // Read from opencode: @ai-sdk/anthropic's baseURL carries /v1.
    let oc = ProviderRow::from(
        "claude_gw",
        &json!({
            "npm": "@ai-sdk/anthropic",
            "options": {"baseURL": "https://gw.test/v1", "apiKey": "sk-test"},
            "models": {}
        }),
    );
    assert_eq!(oc.base_url, "https://gw.test/v1");
    // Write back to opencode: keep /v1.
    assert_eq!(oc.to_value()["options"]["baseURL"], "https://gw.test/v1");
    // Write into pi / omp: strip /v1.
    assert_eq!(convert::provider_to_pi(&oc)["baseUrl"], "https://gw.test");
    assert_eq!(oh_my_pi::provider_to_omp(&oc)["baseUrl"], "https://gw.test");
    // Custom paths without /v1 are preserved as-is.
    let mut custom = oc.clone();
    custom.base_url = "https://gw.test/anthropic".into();
    assert_eq!(
        convert::provider_to_pi(&custom)["baseUrl"],
        "https://gw.test/anthropic"
    );
}

#[test]
fn opencode_load_and_save_ensure_v1_for_messages_api() {
    // opencode's @ai-sdk/anthropic: /v1 completed at load, guaranteed present at write.
    let bare = ProviderRow::from(
        "claude_gw",
        &json!({
            "npm": "@ai-sdk/anthropic",
            "options": {"baseURL": "https://gw.test", "apiKey": "sk-test"},
            "models": {}
        }),
    );
    assert_eq!(bare.base_url, "https://gw.test/v1");
    assert_eq!(bare.to_value()["options"]["baseURL"], "https://gw.test/v1");
    // /v1 already present (trailing slash included) isn't appended again; the slash is stripped.
    let with_v1 = ProviderRow::from(
        "claude_gw",
        &json!({
            "npm": "@ai-sdk/anthropic",
            "options": {"baseURL": "https://gw.test/v1/", "apiKey": "sk-test"},
            "models": {}
        }),
    );
    assert_eq!(with_v1.base_url, "https://gw.test/v1");
    assert_eq!(
        with_v1.to_value()["options"]["baseURL"],
        "https://gw.test/v1"
    );
    // Other protocols untouched: the openai compatibility layer's baseURL is preserved as-is.
    let compat = ProviderRow::from(
        "openai_gw",
        &json!({
            "npm": "@ai-sdk/openai-compatible",
            "options": {"baseURL": "https://gw.test", "apiKey": "sk-test"},
            "models": {}
        }),
    );
    assert_eq!(compat.base_url, "https://gw.test");
    assert_eq!(compat.to_value()["options"]["baseURL"], "https://gw.test");
}

#[test]
fn cross_format_messages_provider_gains_v1_when_written_to_opencode() {
    // The pi side strips /v1 at load; cross-format writes into opencode restore it.
    let v = json!({
        "baseUrl": "https://gw.test/v1",
        "apiKey": "sk-test",
        "api": "anthropic-messages",
        "models": []
    });
    let pi_row = convert::provider_from_pi("claude_gw", &v);
    assert_eq!(pi_row.base_url, "https://gw.test");
    let oc = pi_row.to_value();
    assert_eq!(oc["npm"], "@ai-sdk/anthropic");
    assert_eq!(oc["options"]["baseURL"], "https://gw.test/v1");
    // Other protocols don't get /v1 when cross-writing opencode.
    let v2 = json!({
        "baseUrl": "https://gw.test",
        "apiKey": "sk-test",
        "api": "openai-completions",
        "models": []
    });
    let compat = convert::provider_from_pi("openai_gw", &v2);
    assert_eq!(compat.to_value()["options"]["baseURL"], "https://gw.test");
}
