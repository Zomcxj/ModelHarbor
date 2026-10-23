use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::OpencodeSchemaVersion;

/// The model preset dropdown's "sync" target field: context (input window) or max output.
#[derive(Clone, Copy)]
pub(in crate::app) enum SyncField {
    /// Context / input window (`limit.context`).
    Context,
    /// Max output (`limit.output`).
    Output,
}

impl SyncField {
    /// The Chinese name for status-bar text.
    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::Context => "上下文",
            Self::Output => "最大输出",
        }
    }
}

/// Collects actions and drop targets outward during card rendering.
#[derive(Default)]
pub(super) struct CardActions {
    /// The provider index to delete.
    pub(super) remove: Option<usize>,
    /// The provider index to duplicate.
    pub(super) copy: Option<usize>,
    /// A provider card's drag drop target.
    pub(super) hover: Option<String>,
    /// A model card's drag drop target.
    pub(super) model_hover: Option<String>,
    /// Models checked "enable" this frame, located by `(provider index, model index)`.
    /// The same model id is globally mutually exclusive, handled centrally in [`App::ui_providers_section`].
    pub(super) model_enable: Option<(usize, usize)>,
    /// "Sync" clicks this frame: brush one model's context / output value onto every model on the page.
    /// Rendering only registers the signal; [`App::ui_providers_section`] applies it centrally (avoiding borrow conflicts).
    pub(super) sync: Option<(SyncField, String)>,
}

/// Provider / model form field visibility and dialect labels (shared by all pages).
#[derive(Clone, Copy)]
pub(in crate::app) struct ProviderFormFlags {
    pub(in crate::app) show_oc: bool,
    /// The opencode page with v2 entries in the loaded content: v1-only controls (reasoning) hide,
    /// and the add form doesn't relax either (the v2 schema has no such key).
    pub(in crate::app) oc_v2: bool,
    pub(in crate::app) show_omp: bool,
    pub(in crate::app) show_dsh: bool,
    pub(in crate::app) show_zcode: bool,
    pub(in crate::app) show_wb: bool,
    /// The QwenCode page: the key lives in the top-level `env[<envKey>]`; the entry carries the variable name (shaped like DSH).
    pub(in crate::app) show_qwen: bool,
    /// The KimiCode page: `api_key` and `api_key_env` are **mutually exclusive** — filling one clears the other.
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
    /// Each model row shows the enable/disable switch (writes `disabled`) — see
    /// [`ConfigFormat::has_model_enable`].
    pub(in crate::app) show_model_disabled: bool,
    pub(in crate::app) base_label: &'static str,
    pub(in crate::app) api_key_label: &'static str,
    pub(in crate::app) context_label: &'static str,
    pub(in crate::app) output_label: &'static str,
    pub(in crate::app) input_label: &'static str,
    /// The provider timeout label: v1 `options.timeout` / v2 `settings.timeout`.
    pub(in crate::app) timeout_label: &'static str,
    /// The output-modality label: v1 `modalities.output` / v2 `capabilities.output`.
    pub(in crate::app) modal_output_label: &'static str,
    /// The protocol-package dropdown label: v1 `npm` / v2 `package`.
    pub(in crate::app) package_label: &'static str,
    /// The agent temperature label: v1 top-level `temperature` / v2 `request.body.temperature`.
    pub(in crate::app) agent_temperature_label: &'static str,
}

impl ProviderFormFlags {
    pub(in crate::app) fn new(app: &App) -> Self {
        // The opencode family (opencode / kilocode / mimocode) shares field semantics — one switch for all.
        let show_oc = app.current_page.is_opencode_family();
        // v2 is opencode's own evolution: only the opencode page shows v2 key-name labels;
        // kilo / mimo stay v1 forever (settings.* / capabilities.* are opencode-exclusive).
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
            // opencode's options.timeout always displays (defaulting to 180000ms when the field is absent)
            show_provider_timeout: show_oc || app.page_has_provider_field("timeout"),
            show_model_name: app.page_has_model_field("name"),
            show_model_context: app.page_has_model_field("context"),
            show_model_output: app.page_has_model_field("output"),
            show_model_input: app.page_has_model_field("input"),
            show_model_variants: app.page_has_model_field("variants"),
            show_model_reasoning: app.page_has_model_field("reasoning"),
            show_model_tool_call: app.page_has_model_field("tool_call"),
            show_model_store: app.page_has_model_field("store"),
            // A switch's existence is decided by **backend semantics**, not by whether the file has the key.
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
                // The variable name auto-derives; the branch paints the label "API Key".
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
                // Keep only the last segment.
                "contextWindowSize"
            } else if show_kimi {
                "max_context_size"
            } else {
                // ZCode comes through here too (strip the `properties.` prefix, showing the field name only).
                "contextWindow"
            },
            output_label: if show_oc {
                "limit.output"
            } else if show_zcode || show_wb {
                // ZCode strips the `optionSpecs.….max` path, showing the field name only.
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
