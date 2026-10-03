use super::*;
use crate::model::{ModelRow, ProviderRow};
use serde_json::{Map, Value};

/// 模型条目的缺省表键：`<provider>/<model>`。
///
/// managed provider 的表键带 `managed:` 前缀，这里要去掉。
pub(crate) fn alias_key(provider: &str, model: &str) -> String {
    let short = provider.strip_prefix(MANAGED_PREFIX).unwrap_or(provider);
    format!("{short}/{model}")
}

/// 这条模型写出去时真正的表键：界面记过的别名优先，没有才按 [`alias_key`] 生成。
pub(crate) fn effective_alias(provider: &str, m: &ModelRow) -> String {
    let saved = m.kimi_alias.trim();
    if saved.is_empty() {
        alias_key(provider, m.id.trim())
    } else {
        saved.to_string()
    }
}

/// 本工具的内部协议名 → Kimi `type` 词表（6 值）。
///
/// 只映射跨格式复制可能带进来的内部名；不在表里的返回 `None`。Google 的三个内部协议
/// （generative-ai / vertex / gemini-cli）都落 `google-genai`。
pub(crate) fn kimi_type_for_api(api: &str) -> Option<&'static str> {
    match api {
        "openai-completions" => Some("openai"),
        "openai-responses" => Some("openai_responses"),
        "anthropic-messages" => Some("anthropic"),
        "google-generative-ai" | "google-vertex" | "google-gemini-cli" => Some("google-genai"),
        _ => None,
    }
}

/// 本工具认得的全部 capability 标签。
///
/// 只用于**读**时推导界面控件（思考 / 工具调用 / 输入模态）；写回时走
/// [`merge_capabilities`]，不按这份清单去删。
pub(crate) const CAP_THINKING: &str = "thinking";

pub(crate) const CAP_ALWAYS_THINKING: &str = "always_thinking";

pub(crate) const CAP_TOOL_USE: &str = "tool_use";

pub(crate) const CAP_IMAGE_IN: &str = "image_in";

pub(crate) const CAP_VIDEO_IN: &str = "video_in";

pub(crate) const CAP_AUDIO_IN: &str = "audio_in";

/// [`ProviderRow`] → 它将要写出的 provider 条目形状（仅用于冲突体检）。
///
/// 只按 **Kimi 的语义**还原出三个凭据键的存在性。
pub(crate) fn provider_entry_view(p: &ProviderRow) -> Value {
    let mut obj = Map::new();
    if !p.api_key.trim().is_empty() {
        obj.insert("api_key".into(), Value::String(p.api_key.clone()));
    }
    if !p.api_key_env.trim().is_empty() {
        obj.insert("api_key_env".into(), Value::String(p.api_key_env.clone()));
    }
    // `oauth` 不由界面建模（只读保留）。
    if p.raw.get("oauth").is_some() {
        obj.insert("oauth".into(), p.raw["oauth"].clone());
    }
    Value::Object(obj)
}

/// 模型条目的 capability 标签集。
pub(crate) fn capabilities_of(entry: &Value) -> Vec<String> {
    entry
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// capability 集合 → 输入模态串（界面口径）。
///
/// **未声明时返回空串**（不返回 `"text"`），与 WorkBuddy / QwenCode 同一口径。
pub(crate) fn modalities_from_caps(caps: &[String]) -> String {
    let has = |name: &str| caps.iter().any(|c| c.eq_ignore_ascii_case(name));
    if caps.is_empty() {
        return String::new();
    }
    let image = has(CAP_IMAGE_IN) || has(CAP_VIDEO_IN) || has(CAP_AUDIO_IN);
    crate::convert::supports_to_modalities([("text", true), ("image", image)])
}

/// 界面模态串 → 要**增加**的 capability 标签。
///
/// 只产出「该加什么」，不产出「该删什么」。
pub(crate) fn caps_for_modalities(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if text
        .split(',')
        .any(|s| s.trim().eq_ignore_ascii_case("image"))
    {
        out.push(CAP_IMAGE_IN);
    }
    out
}

/// 已有 capabilities + 界面状态 → 新的 capabilities（**只增不减**）。
///
/// 只在界面**勾上**时追加标签，不因为没勾就删。唯一的例外是 `thinking`：
/// 它与 `always_thinking` 是界面同一个「支持思考」开关的两种表达，取消勾选时一并移除。
pub(crate) fn merge_capabilities(old: &[String], m: &ModelRow) -> Vec<String> {
    let mut out: Vec<String> = old.to_vec();
    let mut push = |name: &str| {
        if !out.iter().any(|c| c.eq_ignore_ascii_case(name)) {
            out.push(name.to_string());
        }
    };
    if m.reasoning {
        push(CAP_THINKING);
    }
    if m.tool_call {
        push(CAP_TOOL_USE);
    }
    for name in caps_for_modalities(&m.modalities_input) {
        push(name);
    }
    if !m.reasoning {
        out.retain(|c| {
            !c.eq_ignore_ascii_case(CAP_THINKING) && !c.eq_ignore_ascii_case(CAP_ALWAYS_THINKING)
        });
    }
    out
}

/// 模型条目 → [`ModelRow`]。`alias` 是表键，`model` 是 wire id。
pub(crate) fn model_from_entry(alias: &str, entry: &Value) -> ModelRow {
    let mut m = ModelRow::new();
    let wire = entry
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    // 界面显示 wire id（发给上游的那个）；表键单独存。
    m.id = wire.clone();
    m.kimi_alias = alias.to_string();
    m.name = entry
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&wire)
        .to_string();
    // 条目没声明的字段一律留空，不沿用「新建模型」的预填值。
    m.context = entry
        .get("max_context_size")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    m.output = entry
        .get("max_output_size")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    let caps = capabilities_of(entry);
    m.modalities_input = modalities_from_caps(&caps);
    m.reasoning = caps.iter().any(|c| {
        c.eq_ignore_ascii_case(CAP_THINKING) || c.eq_ignore_ascii_case(CAP_ALWAYS_THINKING)
    });
    m.tool_call = caps.iter().any(|c| c.eq_ignore_ascii_case(CAP_TOOL_USE));
    m.variants = entry
        .get("support_efforts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    m.original_variants = m.variants.clone();
    m.disabled = entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    m.source_format = Some(ConfigFormat::KimiCode);
    m.raw = entry.clone();
    m
}

/// 界面模型 → 模型条目里**由界面接管**的字段。
///
/// 以**旧条目**为基底，逐个覆盖界面接管的键；界面没接管的键（`reasoning_key` /
/// `beta_api` / `protocol` / `overrides` / 模型级 `base_url` 等）原样留着。
///
/// 界面清空的字段**删掉**对应键而不是写空值。
pub(crate) fn entry_from_model(m: &ModelRow, provider: &str, old: Option<&Value>) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    // `provider` 写在 `model` 之前。表键（别名）不写进条目里。
    obj.insert("provider".into(), Value::String(provider.to_string()));
    obj.insert("model".into(), Value::String(m.id.trim().to_string()));

    // `display_name` **逐字写**：等于 wire id 也写，界面名称为空时回落 wire id。
    let display = if m.name.trim().is_empty() {
        m.id.trim()
    } else {
        m.name.trim()
    };
    set_str(&mut obj, "display_name", display);

    // `max_context_size` 必填且 ≥1：解析不出正整数就**不写**这个键。
    set_num_min1(&mut obj, "max_context_size", m.context.trim());
    set_num_min1(&mut obj, "max_output_size", m.output.trim());

    let old_caps = capabilities_of(old.unwrap_or(&Value::Null));
    let caps = merge_capabilities(&old_caps, m);
    if caps.is_empty() {
        obj.shift_remove("capabilities");
    } else {
        obj.insert(
            "capabilities".into(),
            Value::Array(caps.into_iter().map(Value::String).collect()),
        );
    }

    // 档位：`support_efforts` 是数组；`default_effort` 必须落在其中。
    let efforts = split_csv(&m.variants);
    if efforts.is_empty() {
        obj.shift_remove("support_efforts");
        obj.shift_remove("default_effort");
    } else {
        let existing_default = obj
            .get("default_effort")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        obj.insert(
            "support_efforts".into(),
            Value::Array(efforts.iter().cloned().map(Value::String).collect()),
        );
        // 原有 default_effort 仍在清单里就留着，不在了就删掉这个键。
        if efforts.iter().any(|e| e == &existing_default) {
            obj.insert("default_effort".into(), Value::String(existing_default));
        } else {
            obj.shift_remove("default_effort");
        }
    }
    Value::Object(obj)
}

/// provider 条目里**由界面接管**的字段（以旧条目为基底）。
pub(crate) fn provider_entry_from_row(p: &ProviderRow, old: Option<&Value>) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    set_str(&mut obj, "base_url", p.base_url.trim());
    // `type` 必填，取值限于 Kimi 自己的 6 值词表
    // （anthropic / openai / kimi / google-genai / openai_responses / vertexai）。
    //
    // 三种来源：界面值本来就在词表里 → **逐字写**；界面值是本工具的**内部协议名**
    // → 按 [`kimi_type_for_api`] 翻译；都不是 → 保留旧值，再没有才回落 `openai`。
    let api = p.effective_api();
    let old_type = provider_type(old.unwrap_or(&Value::Null)).to_string();
    let ty = if crate::convert::KIMI_APIS.contains(&api.as_str()) {
        api
    } else if let Some(mapped) = kimi_type_for_api(&api) {
        mapped.to_string()
    } else if !old_type.is_empty() {
        old_type
    } else {
        "openai".to_string()
    };
    obj.insert("type".into(), Value::String(ty));
    // 凭据三选一：界面只有 `api_key` / `api_key_env` 两个框，`oauth` 不建模。
    //
    // 填了密钥就**清掉** `api_key_env`，反之亦然；旧条目带的 `oauth` 不动。
    let key = p.api_key.trim();
    let env = p.api_key_env.trim();
    if !key.is_empty() {
        obj.insert("api_key".into(), Value::String(key.to_string()));
        obj.shift_remove("api_key_env");
    } else if !env.is_empty() {
        obj.insert("api_key_env".into(), Value::String(env.to_string()));
        obj.shift_remove("api_key");
    } else {
        // 两个都空：保持原状。
        obj.shift_remove("api_key");
        obj.shift_remove("api_key_env");
    }
    Value::Object(obj)
}
