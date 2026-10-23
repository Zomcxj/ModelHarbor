//! The opencode v2 page's field-visibility and label regression.
//!
//! v1 and v2 name keys differently (options↔settings, modalities↔capabilities, options.store↔body.store);
//! visibility judgments and labels must recognize both versions, or the v2 page would lose the baseURL /
//! input-modality / tool-call / store controls.

use crate::app::providers::ProviderFormFlags;
use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::ProviderRow;
use serde_json::json;

/// Build an App of "current page = opencode, file from opencode" from a v2 provider JSON.
fn app_with_v2_provider() -> App {
    let v2 = json!({
        "package": "@opencode/ai/providers/openai-compatible",
        "settings": { "baseURL": "https://x/v1", "apiKey": "sk", "timeout": 180000 },
        "models": {
            "m1": {
                "name": "m1",
                "capabilities": { "tools": true, "input": ["text"], "output": ["text"] },
                "limit": { "context": 100, "output": 50 },
                "body": { "store": false },
                "variants": [ { "id": "high" } ]
            }
        }
    });
    App {
        providers: vec![ProviderRow::from("p1", &v2)],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        ..App::default()
    }
}

/// Build the same-shaped App from a v1 provider JSON.
fn app_with_v1_provider() -> App {
    let v1 = json!({
        "npm": "@ai-sdk/openai-compatible",
        "options": { "baseURL": "https://x/v1", "apiKey": "sk", "timeout": 180000 },
        "models": {
            "m1": {
                "name": "m1",
                "reasoning": true,
                "tool_call": true,
                "modalities": { "input": ["text"], "output": ["text"] },
                "limit": { "context": 100, "output": 50 },
                "options": { "store": false },
                "variants": { "high": {} }
            }
        }
    });
    App {
        providers: vec![ProviderRow::from("p1", &v1)],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        ..App::default()
    }
}

#[test]
fn v2_page_shows_all_mapped_fields_with_v2_labels() {
    let flags = ProviderFormFlags::new(&app_with_v2_provider());
    assert!(
        flags.show_provider_base_url,
        "v2 页应显示 baseURL（settings.baseURL）"
    );
    assert!(
        flags.show_model_input,
        "v2 页应显示输入模态（capabilities.input）"
    );
    assert!(
        flags.show_model_tool_call,
        "v2 页应显示工具调用（capabilities.tools）"
    );
    assert!(flags.show_model_store, "v2 页应显示 store（body.store）");
    // v2 has no reasoning key: the reasoning switch doesn't display (matching the "v2 never writes reasoning" decision).
    assert!(!flags.show_model_reasoning, "v2 页不该显示推理开关");
    // The v2 gate: the add form (relaxed) must not relax reasoning either.
    assert!(flags.oc_v2, "v2 页应置 oc_v2 门控");
    // Labels follow the v2 key names.
    assert_eq!(flags.base_label, "settings.baseURL");
    assert_eq!(flags.api_key_label, "settings.apiKey");
    assert_eq!(flags.input_label, "capabilities.input");
    assert_eq!(flags.timeout_label, "settings.timeout");
    assert_eq!(flags.modal_output_label, "capabilities.output");
    assert_eq!(flags.package_label, "package");
}

#[test]
fn v1_page_keeps_v1_field_visibility_and_labels() {
    let flags = ProviderFormFlags::new(&app_with_v1_provider());
    assert!(flags.show_provider_base_url, "v1 页应显示 baseURL");
    assert!(flags.show_model_input, "v1 页应显示输入模态");
    assert!(flags.show_model_tool_call, "v1 页应显示工具调用");
    assert!(flags.show_model_store, "v1 页应显示 store");
    assert!(
        flags.show_model_reasoning,
        "v1 有 reasoning 键应显示推理开关"
    );
    assert!(!flags.oc_v2, "v1 页不得置 v2 门控");
    assert_eq!(flags.base_label, "options.baseURL");
    assert_eq!(flags.api_key_label, "options.apiKey");
    assert_eq!(flags.input_label, "modalities.input");
    assert_eq!(flags.timeout_label, "options.timeout");
    assert_eq!(flags.modal_output_label, "modalities.output");
    assert_eq!(flags.package_label, "npm");
}

/// Each model preset dropdown's first-item "sync": brush the source model's context / output values onto every model on the page.
#[test]
fn limit_sync_applies_source_value_to_every_model() {
    use crate::app::providers::SyncField;
    use crate::model::ModelRow;
    // Two providers, 3 models total, with differing context/output initials.
    let mk = |id: &str| {
        let mut m = ModelRow::new();
        m.id = id.into();
        m.context = "1".into();
        m.output = "2".into();
        m
    };
    let mut p1 = ProviderRow::from("p1", &json!({ "models": {} }));
    p1.models = vec![mk("a"), mk("b")];
    let mut p2 = ProviderRow::from("p2", &json!({ "models": {} }));
    p2.models = vec![mk("c")];
    let mut app = App {
        providers: vec![p1, p2],
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        ..App::default()
    };

    // Sync context: output untouched.
    app.apply_limit_sync(SyncField::Context, "262000".into());
    for p in &app.providers {
        for m in &p.models {
            assert_eq!(m.context, "262000", "context 应被刷到全部模型");
            assert_eq!(m.output, "2", "未同步 output 时保持原值");
        }
    }

    // Sync output: context keeps the previous step's value.
    app.apply_limit_sync(SyncField::Output, "64000".into());
    let count: usize = app.providers.iter().map(|p| p.models.len()).sum();
    assert_eq!(count, 3);
    for p in &app.providers {
        for m in &p.models {
            assert_eq!(m.context, "262000");
            assert_eq!(m.output, "64000");
        }
    }

    // Empty values don't brush (the source model's field is empty).
    app.apply_limit_sync(SyncField::Context, "  ".into());
    assert_eq!(app.providers[0].models[0].context, "262000", "空值不该覆盖");
}

/// v2 is opencode's own evolution: on the kilo / mimo pages, even v2-shaped loaded data (say, loaded from
/// an opencode v2 file then switching pages) always shows v1 key-name labels.
#[test]
fn kilocode_and_mimocode_pages_always_show_v1_labels() {
    for page in [ConfigFormat::Kilocode, ConfigFormat::Mimocode] {
        let mut app = app_with_v2_provider();
        app.current_page = page;
        let flags = ProviderFormFlags::new(&app);
        assert_eq!(
            flags.base_label, "options.baseURL",
            "{:?} 页 baseURL 标签应为 v1 键名",
            page
        );
        assert_eq!(flags.api_key_label, "options.apiKey");
        assert_eq!(flags.timeout_label, "options.timeout");
        assert_eq!(flags.input_label, "modalities.input");
        assert_eq!(flags.modal_output_label, "modalities.output");
        assert_eq!(flags.package_label, "npm", "{:?} 页包名字段应为 npm", page);
    }
}
