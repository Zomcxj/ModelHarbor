use crate::app::App;
use crate::format::ConfigFormat;

/// 卡片渲染时向外收集的动作与落点。
#[derive(Default)]
pub(super) struct CardActions {
    /// 要删除的 provider 下标。
    pub(super) remove: Option<usize>,
    /// 要复制的 provider 下标。
    pub(super) copy: Option<usize>,
    /// provider 卡片的拖拽落点。
    pub(super) hover: Option<String>,
    /// model 卡片的拖拽落点。
    pub(super) model_hover: Option<String>,
    /// 本帧被勾上「启用」的模型，用 `(provider 下标, 模型下标)` 定位。
    /// 同一模型 id 全局互斥，统一在 [`App::ui_providers_section`] 里处理。
    pub(super) model_enable: Option<(usize, usize)>,
}

/// provider / model 表单的字段可见性与方言标签（各页面共用）。
#[derive(Clone, Copy)]
pub(in crate::app) struct ProviderFormFlags {
    pub(in crate::app) show_oc: bool,
    pub(in crate::app) show_omp: bool,
    pub(in crate::app) show_dsh: bool,
    pub(in crate::app) show_zcode: bool,
    pub(in crate::app) show_wb: bool,
    /// QwenCode 页：密钥在顶层 `env[<envKey>]`，条目上写的是变量名（与 DSH 同形）。
    pub(in crate::app) show_qwen: bool,
    /// KimiCode 页：`api_key` 与 `api_key_env` **互斥**，两个框填一个就清掉另一个。
    pub(in crate::app) show_kimi: bool,
    pub(in crate::app) show_provider_base_url: bool,
    pub(in crate::app) show_provider_timeout: bool,
    pub(in crate::app) show_model_name: bool,
    pub(in crate::app) show_model_context: bool,
    pub(in crate::app) show_model_output: bool,
    pub(in crate::app) show_model_input: bool,
    pub(in crate::app) show_model_variants: bool,
    pub(in crate::app) show_model_reasoning: bool,
    pub(in crate::app) show_model_tool_call: bool,
    pub(in crate::app) show_model_store: bool,
    /// 每个模型行显示「启用/停用」开关（写 `disabled`），见
    /// [`ConfigFormat::has_model_enable`]。
    pub(in crate::app) show_model_disabled: bool,
    pub(in crate::app) base_label: &'static str,
    pub(in crate::app) api_key_label: &'static str,
    pub(in crate::app) context_label: &'static str,
    pub(in crate::app) output_label: &'static str,
    pub(in crate::app) input_label: &'static str,
}

impl ProviderFormFlags {
    pub(in crate::app) fn new(app: &App) -> Self {
        // opencode 系（opencode / kilocode / mimocode）字段口径相同，共用一个开关。
        let show_oc = app.current_page.is_opencode_family();
        let show_dsh = app.current_page == ConfigFormat::DeepSeekHarness;
        let show_zcode = app.current_page == ConfigFormat::ZCode;
        let show_wb = app.current_page == ConfigFormat::WorkBuddy;
        let show_qwen = app.current_page == ConfigFormat::QwenCode;
        let show_kimi = app.current_page == ConfigFormat::KimiCode;
        Self {
            show_oc,
            show_omp: app.current_page == ConfigFormat::OhMyPi,
            show_dsh,
            show_zcode,
            show_wb,
            show_qwen,
            show_kimi,
            show_provider_base_url: app.page_has_provider_field("base_url"),
            // opencode 的 options.timeout 始终显示（未写该字段时默认 180000ms）
            show_provider_timeout: show_oc || app.page_has_provider_field("timeout"),
            show_model_name: app.page_has_model_field("name"),
            show_model_context: app.page_has_model_field("context"),
            show_model_output: app.page_has_model_field("output"),
            show_model_input: app.page_has_model_field("input"),
            show_model_variants: app.page_has_model_field("variants"),
            show_model_reasoning: app.page_has_model_field("reasoning"),
            show_model_tool_call: app.page_has_model_field("tool_call"),
            show_model_store: app.page_has_model_field("store"),
            // 开关的存在性由**后端语义**决定，不看文件里有没有这个键。
            show_model_disabled: app.current_page.has_model_enable(),
            base_label: if show_oc {
                "options.baseURL"
            } else if show_dsh {
                "baseURL"
            } else if show_zcode {
                "api.baseUrl"
            } else if show_wb {
                "url"
            } else if show_kimi {
                "base_url"
            } else {
                "baseUrl"
            },
            api_key_label: if show_oc {
                "options.apiKey"
            } else if show_dsh {
                "apiKeyEnv"
            } else if show_zcode {
                "access.apiKey"
            } else if show_qwen {
                // 变量名自动推导，标签在分支里画「API Key」。
                "envKey"
            } else if show_kimi {
                "api_key"
            } else {
                "apiKey"
            },
            context_label: if show_oc {
                "limit.context"
            } else if show_wb {
                "maxInputTokens"
            } else if show_qwen {
                // 只留最后一段。
                "contextWindowSize"
            } else if show_kimi {
                "max_context_size"
            } else {
                // ZCode 也走这里（去掉 `properties.` 前缀，只显示字段名）。
                "contextWindow"
            },
            output_label: if show_oc {
                "limit.output"
            } else if show_zcode || show_wb {
                // ZCode 去掉 `optionSpecs.….max` 路径，只显示字段名。
                "maxOutputTokens"
            } else if show_qwen {
                "max_tokens"
            } else if show_kimi {
                "max_output_size"
            } else {
                "maxTokens"
            },
            input_label: if show_oc {
                "modalities.input"
            } else if show_zcode {
                "properties.supports*"
            } else if show_wb {
                "supportsImages"
            } else if show_qwen {
                "vision"
            } else if show_kimi {
                "capabilities"
            } else {
                "input"
            },
        }
    }
}
