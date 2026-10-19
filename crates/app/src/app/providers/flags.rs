use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::OpencodeSchemaVersion;

/// 模型预设下拉里「同步」的目标字段：上下文（输入窗口）或最大输出。
#[derive(Clone, Copy)]
pub(in crate::app) enum SyncField {
    /// 上下文 / 输入窗口（`limit.context`）。
    Context,
    /// 最大输出（`limit.output`）。
    Output,
}

impl SyncField {
    /// 状态栏文案用的中文名。
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Context => "上下文",
            Self::Output => "最大输出",
        }
    }
}

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
    /// 本帧被点下的「同步」：把某个模型的 context / output 值刷到本页全部模型。
    /// 渲染时只登记信号，统一在 [`App::ui_providers_section`] 里应用（避开借用冲突）。
    pub(super) sync: Option<(SyncField, String)>,
}

/// provider / model 表单的字段可见性与方言标签（各页面共用）。
#[derive(Clone, Copy)]
pub(in crate::app) struct ProviderFormFlags {
    pub(in crate::app) show_oc: bool,
    /// opencode 页且加载内容含 v2 条目：隐藏 v1 独有控件（reasoning），
    /// 新增表单也不放宽（v2 schema 无此键）。
    pub(in crate::app) oc_v2: bool,
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
    /// provider 超时标签：v1 `options.timeout` / v2 `settings.timeout`。
    pub(in crate::app) timeout_label: &'static str,
    /// 输出模态标签：v1 `modalities.output` / v2 `capabilities.output`。
    pub(in crate::app) modal_output_label: &'static str,
    /// 协议包下拉标签：v1 `npm` / v2 `package`。
    pub(in crate::app) package_label: &'static str,
    /// agent 温度标签：v1 顶层 `temperature` / v2 `request.body.temperature`。
    pub(in crate::app) agent_temperature_label: &'static str,
}

impl ProviderFormFlags {
    pub(in crate::app) fn new(app: &App) -> Self {
        // opencode 系（opencode / kilocode / mimocode）字段口径相同，共用一个开关。
        let show_oc = app.current_page.is_opencode_family();
        // v2 只是 opencode 本家的演进：只有 opencode 页显示 v2 键名标签，
        // kilo / mimo 两页永远 v1（settings.* / capabilities.* 是 opencode 独有）。
        let oc_v2 = app.current_opencode_schema() == OpencodeSchemaVersion::V2;
        let show_dsh = app.current_page == ConfigFormat::DeepSeekHarness;
        let show_zcode = app.current_page == ConfigFormat::ZCode;
        let show_wb = app.current_page == ConfigFormat::WorkBuddy;
        let show_qwen = app.current_page == ConfigFormat::QwenCode;
        let show_kimi = app.current_page == ConfigFormat::KimiCode;
        Self {
            show_oc,
            oc_v2,
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
                if oc_v2 {
                    "settings.baseURL"
                } else {
                    "options.baseURL"
                }
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
                if oc_v2 {
                    "settings.apiKey"
                } else {
                    "options.apiKey"
                }
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
                if oc_v2 {
                    "capabilities.input"
                } else {
                    "modalities.input"
                }
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
            timeout_label: if oc_v2 {
                "settings.timeout"
            } else {
                "options.timeout"
            },
            modal_output_label: if oc_v2 {
                "capabilities.output"
            } else {
                "modalities.output"
            },
            package_label: if oc_v2 { "package" } else { "npm" },
            agent_temperature_label: if oc_v2 {
                "request.body.temperature"
            } else {
                "temperature"
            },
        }
    }
}
