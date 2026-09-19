//! ZCode 后端：`~/.zcode/v2/provider_config.json`。
//!
//! 结构（自定 provider 与模型级属性分两处存放）：
//! ```jsonc
//! { "schemaVersion": 1, "config": {
//!     "providerOrder": ["<providerId>"],
//!     "providerConfigRules": { "providerRules": [ {
//!         "providerId", "providerName",
//!         "config": { "group",
//!           "access": { "type", "apiKey" },
//!           "api": { "type", "baseUrl" },
//!           "personalModelIds": [...], "modelOrder": [...] } } ]},
//!     "modelConfigRules": { "providerModelRules": [ {
//!         "modelId", "providerId",
//!         "config": { "enabled", "properties": {...}, "optionSpecs": {...} } } ],
//!       "manualProviderModelRules": [] } } }
//! ```
//!
//! 只接管本文件：ZCode 的内置 provider 在 `config.json`、加密凭据在
//! `credentials.json`、只读模型库在 `runtime/provider/.../zcode-builtin.json`，
//! 三者都不属于用户可编辑面。
//!
//! 模型级字段落在 `config.properties`（`contextWindow` / `supports*` 布尔）与
//! `config.optionSpecs`（`maxOutputTokens.max` / `reasoningLevel.values`）。
//! `optionSpecs.*.map` 是 ZCode 自己生成的 JS 表达式，**原样保留、不解析**。

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{parse_config_content, read_config_content, wsl_home, WslPathProbe};
use serde_json::{Map, Value};
use std::path::Path;

pub struct ZCodeBackend;

pub static BACKEND: ZCodeBackend = ZCodeBackend;

fn default_local_path() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    format!("{}\\.zcode\\v2\\provider_config.json", home)
}

/// `config` 对象（顶层 `config` 键下的内容）。
fn config_of(root: &Value) -> Option<&Map<String, Value>> {
    root.get("config").and_then(Value::as_object)
}

/// provider 规则数组。
fn provider_rules(root: &Value) -> Option<&Vec<Value>> {
    config_of(root)?
        .get("providerConfigRules")?
        .get("providerRules")?
        .as_array()
}

/// 模型规则数组（`modelConfigRules.providerModelRules`）。
fn model_rules(root: &Value) -> Option<&Vec<Value>> {
    config_of(root)?
        .get("modelConfigRules")?
        .get("providerModelRules")?
        .as_array()
}

/// 从 provider 规则里取模型 id 列表（`personalModelIds`，缺失时回落 `modelOrder`）。
fn model_ids_of(rule: &Value) -> Vec<String> {
    let cfg = rule.get("config");
    let pick = |key: &str| {
        cfg.and_then(|c| c.get(key))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let ids = pick("personalModelIds");
    if ids.is_empty() {
        pick("modelOrder")
    } else {
        ids
    }
}

/// 在模型规则里按 (providerId, modelId) 找该模型的 `config`。
fn model_config_of(root: &Value, provider_id: &str, model_id: &str) -> Option<Value> {
    model_rules(root)?
        .iter()
        .find(|rule| {
            rule.get("modelId").and_then(Value::as_str) == Some(model_id)
                && rule.get("providerId").and_then(Value::as_str) == Some(provider_id)
        })
        .and_then(|rule| rule.get("config"))
        .cloned()
}

/// 模型级 raw → ModelRow。
///
/// `config` 为 `providerModelRules` 里那条规则的 `config`（可能缺失）。
/// `raw` 整体保留，`optionSpecs.*.map` 等不认识的键因此不会丢。
fn model_from_zcode(id: &str, config: Value) -> ModelRow {
    let mut row = ModelRow::new();
    row.id = id.to_string();
    row.name = id.to_string();
    row.source_format = Some(ConfigFormat::ZCode);

    let props = config.get("properties");
    row.context = props
        .and_then(|p| p.get("contextWindow"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    row.output = config
        .get("optionSpecs")
        .and_then(|s| s.get("maxOutputTokens"))
        .and_then(|m| m.get("max"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    row.modalities_input = convert::zcode_modalities_from_raw(&config);
    row.variants = convert::zcode_variants_from_raw(&config);
    row.original_variants = row.variants.clone();
    row.reasoning = !row.variants.is_empty();
    row.tool_call = props
        .and_then(|p| p.get("supportsToolCall"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    row.raw = config;
    row
}

/// provider 规则 → ProviderRow。
fn provider_from_zcode(rule: &Value, root: &Value) -> Option<ProviderRow> {
    let id = rule.get("providerId").and_then(Value::as_str)?.to_string();
    let cfg = rule.get("config");
    let api = cfg.and_then(|c| c.get("api"));

    let models = model_ids_of(rule)
        .into_iter()
        .map(|model_id| {
            let config = model_config_of(root, &id, &model_id).unwrap_or(Value::Null);
            model_from_zcode(&model_id, config)
        })
        .collect();

    let mut row = ProviderRow::new();
    row.key = id;
    row.description = rule
        .get("providerName")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // ZCode 的 api.type 词表与 pi 系差一个 `chat`，转换后再进内部表示。
    row.pi_api = convert::zcode_api_to_api(
        api.and_then(|a| a.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    row.base_url = api
        .and_then(|a| a.get("baseUrl"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    row.api_key = cfg
        .and_then(|c| c.get("access"))
        .and_then(|a| a.get("apiKey"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    row.models = models;
    row.source_format = Some(ConfigFormat::ZCode);
    // raw 用 provider 规则里的 `config`：序列化时以它为基底保留 group 等未知键。
    row.raw = cfg.cloned().unwrap_or(Value::Null);
    Some(row)
}

/// ProviderRow → provider 规则的 `config`。
fn provider_config_to_zcode(p: &ProviderRow) -> Value {
    // 以 raw 为基底保留 ZCode 自有字段（group 等）；但来自其它方言的 raw
    // 必须全新构造，否则会把对方的 id / vendor / url 当成扩展字段写进来。
    let mut obj = if convert::is_workbuddy_shaped(&p.raw)
        || convert::is_opencode_shaped_provider(&p.raw)
        || convert::is_dsh_shaped_provider(&p.raw)
    {
        Map::new()
    } else {
        p.raw.as_object().cloned().unwrap_or_default()
    };

    // `group` 是必填项：ZCode schema 要求，且个人 provider 缺它或用错值会直接抛
    // 「Personal-only Provider 必须使用 standard-personal group」并拒绝整份配置
    // （从 WorkBuddy 等转过来的 provider 没有 group，就是保存后 ZCode 里不显示的根因）。
    // 已有 group（来自 ZCode 源，如 zai-family）则保留。
    if !obj.contains_key("group") {
        obj.insert("group".into(), Value::String("standard-personal".into()));
    }

    let mut access = obj
        .get("access")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    access.insert("type".into(), Value::String("api-key".into()));
    if p.api_key.is_empty() {
        access.remove("apiKey");
    } else {
        access.insert("apiKey".into(), Value::String(p.api_key.clone()));
    }
    obj.insert("access".into(), Value::Object(access));

    let mut api = obj
        .get("api")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    api.insert(
        "type".into(),
        Value::String(convert::api_to_zcode_api(&p.effective_api())),
    );
    if !p.base_url.trim().is_empty() {
        api.insert("baseUrl".into(), Value::String(p.base_url.clone()));
    }
    obj.insert("api".into(), Value::Object(api));

    let ids: Vec<Value> = p
        .models
        .iter()
        .filter(|m| !m.id.trim().is_empty())
        .map(|m| Value::String(m.id.clone()))
        .collect();
    obj.insert("personalModelIds".into(), Value::Array(ids.clone()));
    obj.insert("modelOrder".into(), Value::Array(ids));

    Value::Object(obj)
}

/// ModelRow → 模型规则的 `config`。
///
/// `enabled` 是 `config` 的直接子键（与 `properties` 平级），不是 `properties`
/// 里的项——写错位置 ZCode 会读不到，还会留下一个同名垃圾键。
///
/// 能力布尔只在**原文件已写该键**或**界面明确给出模态列表**时改写：
/// 凭空补 `supports*: false` 会把「未声明」变成「明确不支持」，
/// 反而关掉 ZCode 本来会按模型名推断的能力。
fn model_config_to_zcode(m: &ModelRow) -> Value {
    // raw 为基底：`optionSpecs.*.map` 这类表达式与未知键原样保留。
    // 来自其它方言的 raw 全新构造，避免对方字段（id / url / supportsImages…）泄漏。
    let mut obj = if convert::is_workbuddy_shaped(&m.raw)
        || convert::is_opencode_shaped_model(&m.raw)
        || convert::is_dsh_shaped_model(&m.raw)
    {
        Map::new()
    } else {
        m.raw.as_object().cloned().unwrap_or_default()
    };

    // enabled 与 properties 平级。
    obj.insert("enabled".into(), Value::Bool(true));

    let mut props = obj
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !m.context.trim().is_empty() {
        props.insert(
            "contextWindow".into(),
            Value::Number(m.context.trim().parse::<i64>().unwrap_or(0).into()),
        );
    }
    // 模态：界面给出列表时按它写；列表为空时不动原有键。
    // 落点是 `properties.inputFormat`（内置库如此嵌套），不是 properties 直接子键——
    // 写平了 ZCode 读不到，还会和它自己写的 inputFormat 并存两套。
    if !m.modalities_input.trim().is_empty() {
        let mut input_format = props
            .get("inputFormat")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, on) in convert::modalities_to_supports(&m.modalities_input) {
            let field = match key {
                "text" => "supportsText",
                "image" => "supportsImage",
                "video" => "supportsVideo",
                "pdf" => "supportsPdf",
                "audio" => "supportsAudio",
                _ => continue,
            };
            // 只同步原本已声明的键，或新打开的模态（on）。绝不给未声明的键补
            // false——那会把「未声明」变成「明确不支持」，凭空展开成五个 flag；
            // 尤其 supportsText:false 会让 ZCode 隐藏/拒绝该模型（就是保存后
            // 软件里不显示的根因）。
            if input_format.contains_key(field) || on {
                input_format.insert(field.into(), Value::Bool(on));
            }
        }
        if input_format.is_empty() {
            props.remove("inputFormat");
        } else {
            props.insert("inputFormat".into(), Value::Object(input_format));
        }
    }
    // 同理：只在原文件已有该键时同步 tool_call，否则不凭空声明。
    if props.contains_key("supportsToolCall") {
        props.insert("supportsToolCall".into(), Value::Bool(m.tool_call));
    }
    if !props.is_empty() {
        obj.insert("properties".into(), Value::Object(props));
    }

    let mut specs = obj
        .get("optionSpecs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !m.output.trim().is_empty() {
        let mut max = specs
            .get("maxOutputTokens")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        max.insert(
            "max".into(),
            Value::Number(m.output.trim().parse::<i64>().unwrap_or(0).into()),
        );
        specs.insert("maxOutputTokens".into(), Value::Object(max));
    }
    // 档位：只在 UI 里有值且原文件已有该块时改写 values，其余情况不动——
    // `map` 表达式由 ZCode 自己生成，我们只同步档位清单。
    let variants: Vec<Value> = m
        .variants
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| Value::String(s.to_string()))
        .collect();
    if !variants.is_empty() {
        let mut level = specs
            .get("reasoningLevel")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        level.insert("values".into(), Value::Array(variants));
        specs.insert("reasoningLevel".into(), Value::Object(level));
    }
    if !specs.is_empty() {
        obj.insert("optionSpecs".into(), Value::Object(specs));
    }

    Value::Object(obj)
}

impl Backend for ZCodeBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::ZCode
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.zcode/v2/provider_config.json", wsl_home()?))
    }

    fn local_available(&self, local_path: &str) -> bool {
        Path::new(local_path).exists()
            || Path::new(local_path)
                .parent()
                .map(|p| p.exists())
                .unwrap_or(false)
    }

    fn wsl_available(&self, probe: WslPathProbe) -> bool {
        probe.path_exists || probe.parent_dir_exists
    }

    fn detect(&self, content: &str, _path: &str) -> bool {
        // 两个独立标记：providerRules 或 providerOrder 任一为数组即认。
        parse_config_content(content)
            .map(|v| {
                let Some(cfg) = config_of(&v) else {
                    return false;
                };
                let rules = cfg
                    .get("providerConfigRules")
                    .and_then(|r| r.get("providerRules"))
                    .and_then(Value::as_array)
                    .is_some();
                let order = cfg.get("providerOrder").and_then(Value::as_array).is_some();
                rules || order
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_config_content(content)?;
        let providers = provider_rules(&root)
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|rule| provider_from_zcode(rule, &root))
                    .collect()
            })
            .unwrap_or_default();
        Ok(BackendLoad {
            root: root.clone(),
            agents: Vec::new(),
            providers,
            extras: root,
        })
    }

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        let mut root = target_root
            .unwrap_or(extras)
            .as_object()
            .cloned()
            .unwrap_or_default();
        if !root.contains_key("schemaVersion") {
            root.insert("schemaVersion".into(), Value::Number(1.into()));
        }

        let mut cfg = root
            .get("config")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        // providerOrder：完全按 UI 顺序（含删除生效）。
        let order: Vec<Value> = providers
            .iter()
            .filter(|p| !p.key.trim().is_empty())
            .map(|p| Value::String(p.key.clone()))
            .collect();
        cfg.insert("providerOrder".into(), Value::Array(order));

        // providerRules：按 UI 顺序重建；同 key 的旧规则作基底保留未知键。
        let existing: Map<String, Value> = cfg
            .get("providerConfigRules")
            .and_then(|r| r.get("providerRules"))
            .and_then(Value::as_array)
            .map(|rules| {
                rules
                    .iter()
                    .filter_map(|r| {
                        let id = r.get("providerId").and_then(Value::as_str)?;
                        Some((id.to_string(), r.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rules: Vec<Value> = providers
            .iter()
            .filter(|p| !p.key.trim().is_empty())
            .map(|p| {
                let mut rule = existing
                    .get(&p.key)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                rule.insert("providerId".into(), Value::String(p.key.clone()));
                rule.insert(
                    "providerName".into(),
                    Value::String(if p.description.trim().is_empty() {
                        p.key.clone()
                    } else {
                        p.description.clone()
                    }),
                );
                rule.insert("config".into(), provider_config_to_zcode(p));
                Value::Object(rule)
            })
            .collect();
        let mut provider_cfg = Map::new();
        provider_cfg.insert("providerRules".into(), Value::Array(rules));
        cfg.insert("providerConfigRules".into(), Value::Object(provider_cfg));

        // modelConfigRules：按 UI 的 (providerId, modelId) 重写；
        // 未被接管的旧规则与 manualProviderModelRules 全量保留。
        let managed: Vec<(String, String)> = providers
            .iter()
            .filter(|p| !p.key.trim().is_empty())
            .flat_map(|p| {
                p.models
                    .iter()
                    .filter(|m| !m.id.trim().is_empty())
                    .map(|m| (p.key.clone(), m.id.clone()))
            })
            .collect();
        let mut model_cfg = cfg
            .get("modelConfigRules")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let old_rules: Vec<Value> = model_cfg
            .get("providerModelRules")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // 旧规则里属于本 provider 的条目按模型查基底（保留 optionSpecs.map 等）。
        let lookup = |pid: &str, mid: &str| -> Option<&Value> {
            old_rules.iter().find(|r| {
                r.get("providerId").and_then(Value::as_str) == Some(pid)
                    && r.get("modelId").and_then(Value::as_str) == Some(mid)
            })
        };
        let mut new_rules: Vec<Value> = Vec::new();
        for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
            for m in p.models.iter().filter(|m| !m.id.trim().is_empty()) {
                let mut rule = lookup(&p.key, &m.id)
                    .and_then(|r| r.as_object().cloned())
                    .unwrap_or_default();
                rule.insert("modelId".into(), Value::String(m.id.clone()));
                rule.insert("providerId".into(), Value::String(p.key.clone()));
                rule.insert("config".into(), model_config_to_zcode(m));
                new_rules.push(Value::Object(rule));
            }
        }
        // 保留未被 UI 接管的规则：provider 已不在 UI 的、或该 provider 下模型已删的。
        for rule in &old_rules {
            let pid = rule.get("providerId").and_then(Value::as_str).unwrap_or("");
            let mid = rule.get("modelId").and_then(Value::as_str).unwrap_or("");
            let provider_gone = !providers.iter().any(|p| p.key == pid);
            let still_managed = managed.iter().any(|(a, b)| a == pid && b == mid);
            if provider_gone && !still_managed {
                new_rules.push(rule.clone());
            }
        }
        model_cfg.insert("providerModelRules".into(), Value::Array(new_rules));
        if !model_cfg.contains_key("manualProviderModelRules") {
            model_cfg.insert("manualProviderModelRules".into(), Value::Array(Vec::new()));
        }
        cfg.insert("modelConfigRules".into(), Value::Object(model_cfg));

        root.insert("config".into(), Value::Object(cfg));
        Value::Object(root)
    }

    fn load_target_root(&self, path: &str) -> Value {
        match read_config_content(path) {
            Ok(content) => parse_config_content(&content).unwrap_or(Value::Object(Map::new())),
            Err(_) => Value::Object(Map::new()),
        }
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((include_bytes!("../../assets/agents/zcode_32.bin"), 32, 32))
    }

    fn render(&self, root: &Value, compact: bool) -> Result<String, String> {
        Ok(if compact {
            crate::app::compact_json(root)
        } else {
            crate::app::pretty_json(root)
        })
    }
}
