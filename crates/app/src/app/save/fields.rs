use crate::app::App;
use crate::format::ConfigFormat;
use serde_json::Value;

impl App {
    /// 当前 agent 文件是否使用某个 provider 级字段。
    /// 判断范围是整个文件，不是单个 provider；切换到尚未加载的目标页时
    /// 使用完整 schema。
    pub(in crate::app) fn page_has_provider_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers
            .iter()
            .any(|provider| match self.current_page {
                // opencode 系三者字段口径相同。
                ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
                    match field {
                        "base_url" => provider
                            .raw
                            .get("options")
                            .and_then(|v| v.get("baseURL"))
                            .is_some(),
                        "timeout" => provider
                            .raw
                            .get("options")
                            .and_then(|v| v.get("timeout"))
                            .is_some(),
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
                // ZCode 的端点/密钥落在 `api.baseUrl` / `access.apiKey`。
                ConfigFormat::ZCode => match field {
                    "base_url" => provider
                        .raw
                        .get("api")
                        .and_then(|v| v.get("baseUrl"))
                        .is_some(),
                    _ => false,
                },
                // WorkBuddy 每条模型自带 url / apiKey。
                ConfigFormat::WorkBuddy => match field {
                    "base_url" => provider.raw.get("url").is_some(),
                    _ => false,
                },
                // QwenCode 每条条目自带 baseUrl / envKey；timeout 落在 generationConfig。
                ConfigFormat::QwenCode => match field {
                    "base_url" => provider.raw.get("baseUrl").is_some(),
                    "timeout" => provider
                        .raw
                        .get("generationConfig")
                        .and_then(|g| g.get("timeout"))
                        .is_some(),
                    _ => false,
                },
                // KimiCode 的 provider 表只有 base_url / type / 凭据，没有 provider 级超时。
                ConfigFormat::KimiCode => match field {
                    "base_url" => provider.raw.get("base_url").is_some(),
                    _ => false,
                },
            })
    }

    /// 当前 agent 文件是否使用某个 model 级字段。字段存在性按整个
    /// agent 文件判断。
    pub(in crate::app) fn page_has_model_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers.iter().any(|provider| {
            provider.models.iter().any(|model| match self.current_page {
                // opencode 系三者模型字段口径相同。
                ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
                    match field {
                        "name" => model.raw.get("name").is_some(),
                        "reasoning" => model.raw.get("reasoning").is_some(),
                        "tool_call" => model.raw.get("tool_call").is_some(),
                        "store" => model
                            .raw
                            .get("options")
                            .and_then(|v| v.get("store"))
                            .is_some(),
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
                        "input" => model
                            .raw
                            .get("modalities")
                            .and_then(|v| v.get("input"))
                            .is_some(),
                        "variants" => model.raw.get("variants").is_some(),
                        _ => false,
                    }
                }
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "name" => model.raw.get("name").is_some(),
                    // reasoning 也可由 pi/omp 的 thinking 块 / thinkingLevelMap 表达。
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
                // ZCode 的模型属性落在 config.properties / config.optionSpecs。
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
                // WorkBuddy 每条模型自带全部属性。
                ConfigFormat::WorkBuddy => match field {
                    // 模型名已落到模型行的 id 字段（WB 无独立模型 id），name 不单列。
                    "name" => false,
                    // 「启用开关」由 `ConfigFormat::has_model_enable()` 判定，见 providers.rs。
                    "context" => model.raw.get("maxInputTokens").is_some(),
                    "output" => model.raw.get("maxOutputTokens").is_some(),
                    "input" => model.raw.get("supportsImages").is_some(),
                    "tool_call" => model.raw.get("supportsToolCall").is_some(),
                    "reasoning" => model.raw.get("supportsReasoning").is_some(),
                    _ => false,
                },
                // QwenCode 的模型属性落在 generationConfig / capabilities。
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
                    // QwenCode 没有「工具调用」与 `store` 字段。
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
                // KimiCode 的模型属性是 `capabilities` 标签集 + 三个数值/字符串键。
                ConfigFormat::KimiCode => match field {
                    "name" => model.raw.get("display_name").is_some(),
                    "context" => model.raw.get("max_context_size").is_some(),
                    "output" => model.raw.get("max_output_size").is_some(),
                    // 「支持思考」对应 thinking / always_thinking 两个标签。
                    "reasoning" => caps_has(model, &["thinking", "always_thinking"]),
                    // 「工具调用」对应 tool_use 标签。
                    "tool_call" => caps_has(model, &["tool_use"]),
                    "variants" => model.raw.get("support_efforts").is_some(),
                    // 输入模态不显示：Kimi 用 image_in / video_in / audio_in 三个标签表达，
                    // 与单个「输入模态」框的映射不可逆。这些标签由 [`merge_capabilities`]
                    // 原样保留。
                    "input" => false,
                    // Kimi 没有 `store` 字段。
                    "store" => false,
                    _ => false,
                },
            })
        })
    }
}

/// 模型的 `capabilities` 标签集里是否有其中任意一个（KimiCode 用）。
///
/// 不区分大小写。
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
