use crate::app::App;
use crate::format::ConfigFormat;
use serde_json::Value;

impl App {
    /// Whether the current agent file uses a provider-level field.
    /// The judgment spans the whole file, not one provider; a target page not yet loaded uses the full schema.
    pub(in crate::app) fn page_has_provider_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers
            .iter()
            .any(|provider| match self.current_page {
                // The opencode family's three share field semantics; v1 uses options.*, v2 settings.*.
                ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
                    let has = |key: &str| {
                        provider
                            .raw
                            .get("options")
                            .and_then(|v| v.get(key))
                            .is_some()
                            || provider
                                .raw
                                .get("settings")
                                .and_then(|v| v.get(key))
                                .is_some()
                    };
                    match field {
                        "base_url" => has("baseURL"),
                        "timeout" => has("timeout"),
                        _ => false,
                    }
                }
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "base_url" => provider.raw.get("baseUrl").is_some(),
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "base_url" => provider.raw.get("baseURL").is_some(),
                    _ => false,
                },
                // ZCode's endpoint/key land in `api.baseUrl` / `access.apiKey`.
                ConfigFormat::ZCode => match field {
                    "base_url" => provider
                        .raw
                        .get("api")
                        .and_then(|v| v.get("baseUrl"))
                        .is_some(),
                    _ => false,
                },
                // WorkBuddy carries url / apiKey on every model.
                ConfigFormat::WorkBuddy => match field {
                    "base_url" => provider.raw.get("url").is_some(),
                    _ => false,
                },
                // QwenCode carries baseUrl / envKey on every entry; the timeout lands in generationConfig.
                ConfigFormat::QwenCode => match field {
                    "base_url" => provider.raw.get("baseUrl").is_some(),
                    "timeout" => provider
                        .raw
                        .get("generationConfig")
                        .and_then(|g| g.get("timeout"))
                        .is_some(),
                    _ => false,
                },
                // KimiCode's provider table holds only base_url / type / credentials — no provider-level timeout.
                ConfigFormat::KimiCode => match field {
                    "base_url" => provider.raw.get("base_url").is_some(),
                    _ => false,
                },
            })
    }

    /// Whether the current agent file uses a model-level field. Existence is judged across the whole agent file.
    pub(in crate::app) fn page_has_model_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers.iter().any(|provider| {
            provider.models.iter().any(|model| match self.current_page {
                // The opencode family's three share model-field semantics; v1 uses reasoning / tool_call /
                // modalities / options.store, v2 uses capabilities.tools / capabilities.input / body.store (v2 has no reasoning).
                ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
                    let cap = |key: &str| {
                        model
                            .raw
                            .get("capabilities")
                            .and_then(|v| v.get(key))
                            .is_some()
                    };
                    match field {
                        "name" => model.raw.get("name").is_some(),
                        "reasoning" => model.raw.get("reasoning").is_some(),
                        "tool_call" => model.raw.get("tool_call").is_some() || cap("tools"),
                        "store" => {
                            model
                                .raw
                                .get("options")
                                .and_then(|v| v.get("store"))
                                .is_some()
                                || model.raw.get("body").and_then(|v| v.get("store")).is_some()
                        }
                        "context" => model
                            .raw
                            .get("limit")
                            .and_then(|v| v.get("context"))
                            .is_some(),
                        "output" => model
                            .raw
                            .get("limit")
                            .and_then(|v| v.get("output"))
                            .is_some(),
                        "input" => {
                            model
                                .raw
                                .get("modalities")
                                .and_then(|v| v.get("input"))
                                .is_some()
                                || cap("input")
                        }
                        "variants" => model.raw.get("variants").is_some(),
                        _ => false,
                    }
                }
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "name" => model.raw.get("name").is_some(),
                    // reasoning can also be expressed by pi/omp's thinking block / thinkingLevelMap.
                    "reasoning" => {
                        model.raw.get("reasoning").is_some()
                            || model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                            || model.raw.get("reasoningEfforts").is_some()
                    }
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => {
                        model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                    }
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "name" => model.raw.get("name").is_some(),
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => model.raw.get("reasoningEfforts").is_some(),
                    _ => false,
                },
                // ZCode's model attributes land in config.properties / config.optionSpecs.
                ConfigFormat::ZCode => match field {
                    "context" => model
                        .raw
                        .get("properties")
                        .and_then(|v| v.get("contextWindow"))
                        .is_some(),
                    "output" => model
                        .raw
                        .get("optionSpecs")
                        .and_then(|v| v.get("maxOutputTokens"))
                        .is_some(),
                    "input" => model
                        .raw
                        .get("properties")
                        .and_then(|v| v.get("inputFormat"))
                        .is_some(),
                    "tool_call" => model
                        .raw
                        .get("properties")
                        .and_then(|v| v.get("supportsToolCall"))
                        .is_some(),
                    "variants" => model
                        .raw
                        .get("optionSpecs")
                        .and_then(|v| v.get("reasoningLevel"))
                        .is_some(),
                    _ => false,
                },
                // WorkBuddy carries every attribute on each model.
                ConfigFormat::WorkBuddy => match field {
                    // The model name already lives in the model row's id field (WB has no separate model id); name isn't listed separately.
                    "name" => false,
                    // The "enable switch" is decided by `ConfigFormat::has_model_enable()` — see providers.rs.
                    "context" => model.raw.get("maxInputTokens").is_some(),
                    "output" => model.raw.get("maxOutputTokens").is_some(),
                    "input" => model.raw.get("supportsImages").is_some(),
                    "tool_call" => model.raw.get("supportsToolCall").is_some(),
                    "reasoning" => model.raw.get("supportsReasoning").is_some(),
                    _ => false,
                },
                // QwenCode's model attributes land in generationConfig / capabilities.
                ConfigFormat::QwenCode => match field {
                    "name" => model.raw.get("name").is_some(),
                    "context" => model
                        .raw
                        .get("generationConfig")
                        .and_then(|g| g.get("contextWindowSize"))
                        .is_some(),
                    "output" => model
                        .raw
                        .get("generationConfig")
                        .and_then(|g| g.get("samplingParams"))
                        .and_then(|s| s.get("max_tokens"))
                        .is_some(),
                    "input" => model
                        .raw
                        .get("capabilities")
                        .and_then(|c| c.get("vision"))
                        .is_some(),
                    // QwenCode has no "tool use" or `store` fields.
                    "tool_call" | "store" => false,
                    "reasoning" => model
                        .raw
                        .get("capabilities")
                        .and_then(|c| c.get("reasoning"))
                        .is_some(),
                    "variants" => model
                        .raw
                        .get("capabilities")
                        .and_then(|c| c.get("reasoning"))
                        .and_then(|r| r.get("efforts"))
                        .is_some(),
                    _ => false,
                },
                // KimiCode's model attributes are a `capabilities` label set + three numeric/string keys.
                ConfigFormat::KimiCode => match field {
                    "name" => model.raw.get("display_name").is_some(),
                    "context" => model.raw.get("max_context_size").is_some(),
                    "output" => model.raw.get("max_output_size").is_some(),
                    // "Supports reasoning" maps to the thinking / always_thinking labels.
                    "reasoning" => caps_has(model, &["thinking", "always_thinking"]),
                    // "Tool use" maps to the tool_use label.
                    "tool_call" => caps_has(model, &["tool_use"]),
                    "variants" => model.raw.get("support_efforts").is_some(),
                    // Input modalities don't display: Kimi expresses them via the image_in / video_in / audio_in labels,
                    // a mapping that doesn't invert into a single "input modality" box. [`merge_capabilities`] preserves
                    // these labels verbatim.
                    "input" => false,
                    // Kimi has no `store` field.
                    "store" => false,
                    _ => false,
                },
            })
        })
    }
}

/// Whether a model's `capabilities` label set contains any of these (KimiCode).
///
/// Case-insensitive.
fn caps_has(model: &crate::model::ModelRow, names: &[&str]) -> bool {
    model
        .raw
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|caps| {
            caps.iter().filter_map(Value::as_str).any(|c| {
                let c = c.trim();
                names.iter().any(|n| c.eq_ignore_ascii_case(n))
            })
        })
}
