use crate::app::App;
use crate::format::ConfigFormat;
use serde_json::Value;

impl App {
    /// 当前 agent 文件是否使用某个 provider 级字段。
    /// 判断范围是整个文件，不是单个 provider；切换到尚未加载的目标页时
    /// 使用完整 schema，保证新增 provider 可以输入所有专属字段。
    pub(in crate::app) fn page_has_provider_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers
            .iter()
            .any(|provider| match self.current_page {
                // opencode 系三者字段口径相同，共用一套判定。
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
                // KimiCode 的 provider 表里只有 base_url / type / 凭据三类字段，
                // **没有** provider 级超时。
                ConfigFormat::KimiCode => match field {
                    "base_url" => provider.raw.get("base_url").is_some(),
                    _ => false,
                },
            })
    }

    /// 当前 agent 文件是否使用某个 model 级字段。字段存在性按整个
    /// agent 文件判断，避免单个模型缺字段时导致同一页面布局跳变。
    pub(in crate::app) fn page_has_model_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers.iter().any(|provider| {
            provider.models.iter().any(|model| match self.current_page {
                // opencode 系三者模型字段口径相同，共用一套判定。
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
                    // reasoning 也可由 pi/omp 的 thinking 块 / thinkingLevelMap 表达，
                    // 只写了这些键时同样应显示（并勾选）reasoning。
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
                    // 模型名已落到模型行的 id 字段（WB 无独立模型 id），不再单列 name 字段，
                    // 否则 id / name 两个框都显示同一个模型名，反而误导。
                    "name" => false,
                    // 「启用开关」不走这里：它是**语义**字段（不是「文件里有没有」），
                    // 由 `ConfigFormat::has_model_enable()` 判定，见 providers.rs。
                    // 在这里返回 true 会把开关漏到所有页面（曾经就是这样）。
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
                    // QwenCode 没有「工具调用」与 `store` 字段，不给控件。
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
                    // 输入模态**不显示**：Kimi 用 image_in / video_in / audio_in 三个独立
                    // 标签表达，而界面那一个「输入模态」框表达不了三者的差别——取消勾选
                    // 时该删哪个标签无从判断，勾选时又会把「只有 video_in」的模型写成
                    // 同时有 image_in。映射不可逆，给一个勾了也删不掉的框是骗人。
                    // 这些标签仍由 [`merge_capabilities`] 原样保留，不会因为不显示而丢失。
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
/// 不区分大小写：Kimi 自己的读取逻辑会把标签 `trim().toLowerCase()` 再比对
/// （见 `withAnthropicProfile`），手写的 `Tool_Use` 同样生效，界面就不该显示成没勾。
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
