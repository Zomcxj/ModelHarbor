//! oh-my-pi（omp）后端：`~/.omp/agent/models.yml`（YAML）。
//!
//! schema 与 pi 同族（顶层 `providers` + extras），差异：
//! - 思考档位用 `thinking: {mode, efforts, effortMap}`（pi 用 `thinkingLevelMap`）；
//! - provider/model 支持大量扩展字段（headers/auth/discovery/modelOverrides/cost/...），
//!   保存时以 raw 为基底原样保留，仅重写 UI 管理的字段；
//! - 序列化为 YAML。
//!
//! 加载为双方言宽容：`thinking` 与 `thinkingLevelMap` 都能读入 IR variants。

use super::{Backend, BackendLoad};
use crate::convert::{
    self, is_dsh_shaped_model, is_dsh_shaped_provider, is_opencode_shaped_model,
    is_opencode_shaped_provider,
};
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::WslPathProbe;
use crate::util::{
    parse_config_content, parse_yaml_content, read_config_content, to_yaml_string, wsl_home,
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::Path;

pub struct OhMyPiBackend;

pub static BACKEND: OhMyPiBackend = OhMyPiBackend;

fn default_local_path() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    format!("{}\\.omp\\agent\\models.yml", home)
}

/// 判断 raw 是否为 opencode 方言的助手已迁至 convert 模块（pi 后端同样需要）。
fn string_set(items: impl Iterator<Item = String>) -> HashSet<String> {
    items.collect()
}

/// 模型 → omp 方言对象（保留 raw 中的未知字段，思考档位输出 thinking 块）。
pub fn model_to_omp(m: &ModelRow) -> Value {
    // opencode 来源全新构造；pi/omp 来源以 raw 为基底保留扩展字段
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_model(&m.raw) || is_dsh_shaped_model(&m.raw) {
            Map::new()
        } else {
            m.raw.as_object().cloned().unwrap_or_default()
        };
    // pi 方言思考键统一转为 thinking 块
    obj.remove("thinkingLevelMap");

    obj.insert("id".into(), Value::String(m.id.clone()));
    if !m.name.trim().is_empty() {
        obj.insert("name".into(), Value::String(m.name.clone()));
    } else {
        obj.remove("name");
    }
    obj.insert("reasoning".into(), Value::Bool(m.reasoning));
    let input: Vec<Value> = m
        .modalities_input
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| Value::String(s.to_string()))
        .collect();
    if input.is_empty() {
        obj.remove("input");
    } else {
        obj.insert("input".into(), Value::Array(input));
    }
    if let Ok(ctx) = m.context.parse::<i64>() {
        obj.insert("contextWindow".into(), Value::Number(ctx.into()));
    } else {
        obj.remove("contextWindow");
    }
    if let Ok(out) = m.output.parse::<i64>() {
        obj.insert("maxTokens".into(), Value::Number(out.into()));
    } else {
        obj.remove("maxTokens");
    }

    if m.variants.trim().is_empty() {
        obj.remove("thinking");
    } else if let Some(thinking) = omp_thinking(m) {
        obj.insert("thinking".into(), thinking);
    }
    Value::Object(obj)
}

/// 构造 omp thinking 块（variants 非空时调用）：
/// 1. omp 原生往返：raw.thinking 的档位值集合与当前一致 → 整块保留（含 defaultLevel 等）；
/// 2. pi 方言翻译：raw.thinkingLevelMap 值集合一致 → efforts=键集合，非对称时附 effortMap；
/// 3. 新建对称块。
fn omp_thinking(m: &ModelRow) -> Option<Value> {
    let names: Vec<String> = crate::model::ordered_variants_text(&m.variants);
    let cur: HashSet<String> = names.iter().cloned().collect();
    let vals_of = |o: &Map<String, Value>| -> HashSet<String> {
        o.values()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect()
    };

    // 1) omp 原生
    if let Some(t) = m.raw.get("thinking").and_then(|v| v.as_object()) {
        let raw_vals = t
            .get("effortMap")
            .and_then(|v| v.as_object())
            .map(&vals_of)
            .unwrap_or_else(|| {
                t.get("efforts")
                    .and_then(|v| v.as_array())
                    .map(|ef| string_set(ef.iter().filter_map(|v| v.as_str().map(String::from))))
                    .unwrap_or_default()
            });
        if raw_vals == cur {
            return Some(Value::Object(crate::model::order_variant_map(t)));
        }
    }

    // 2) pi 方言翻译
    if let Some(tlm) = m.raw.get("thinkingLevelMap").and_then(|v| v.as_object()) {
        let tlm_vals = vals_of(tlm);
        if tlm_vals == cur {
            let keys: HashSet<String> = tlm.keys().cloned().collect();
            let symmetric = keys == tlm_vals;
            let ordered = crate::model::order_variant_map(tlm);
            let mut t = Map::new();
            t.insert("mode".into(), json!("effort"));
            t.insert(
                "efforts".into(),
                Value::Array(ordered.keys().map(|k| Value::String(k.clone())).collect()),
            );
            if !symmetric {
                t.insert("effortMap".into(), Value::Object(ordered));
            }
            return Some(Value::Object(t));
        }
    }

    // 3) 新建对称
    Some(json!({
        "mode": "effort",
        "efforts": names,
    }))
}

/// Provider → omp 方言对象（raw 基底保留 headers/auth/discovery/modelOverrides 等）。
pub fn provider_to_omp(p: &ProviderRow) -> Value {
    let mut obj: Map<String, Value> =
        if is_opencode_shaped_provider(&p.raw) || is_dsh_shaped_provider(&p.raw) {
            Map::new()
        } else {
            p.raw.as_object().cloned().unwrap_or_default()
        };

    let api = p.effective_api();

    if !p.base_url.is_empty() {
        // 与 pi / dsh 同一套归一化：messages 协议的 base 不带 /v1（客户端自行补 /v1/messages）。
        let save_url = convert::without_v1_for_messages(&api, &p.base_url);
        obj.insert("baseUrl".into(), Value::String(save_url));
    } else {
        obj.remove("baseUrl");
    }
    if !p.api_key.is_empty() {
        obj.insert("apiKey".into(), Value::String(p.api_key.clone()));
    } else {
        obj.remove("apiKey");
    }
    obj.insert("api".into(), Value::String(api));

    // compat 仅管理 supportsDeveloperRole，其余键（maxTokensField/extraBody/...）保留
    if !p.compat {
        let mut c = obj
            .get("compat")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        c.insert("supportsDeveloperRole".into(), Value::Bool(false));
        obj.insert("compat".into(), Value::Object(c));
    } else if let Some(c) = obj.get_mut("compat").and_then(|v| v.as_object_mut()) {
        c.remove("supportsDeveloperRole");
        if c.is_empty() {
            obj.remove("compat");
        }
    }
    // requiresReasoningContentForAllAssistantTurns（omp 键，与 pi 键相互映射）：
    // 同格式未修改时保留 raw 原样；跨格式或用户改动时写出当前值（缺省打勾）。
    let native_omp = matches!(
        p.source_format,
        Some(crate::format::ConfigFormat::Pi) | Some(crate::format::ConfigFormat::OhMyPi)
    );
    if p.requires_reasoning_content != p.original_requires_reasoning_content || !native_omp {
        let mut c = obj
            .get("compat")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        c.insert(
            "requiresReasoningContentForAllAssistantTurns".into(),
            Value::Bool(p.requires_reasoning_content),
        );
        obj.insert("compat".into(), Value::Object(c));
    }

    let models: Vec<Value> = p.models.iter().map(model_to_omp).collect();
    obj.insert("models".into(), Value::Array(models));
    Value::Object(convert::order_fields(
        obj,
        &["baseUrl", "apiKey", "api", "compat", "models"],
    ))
}

impl Backend for OhMyPiBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::OhMyPi
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.omp/agent/models.yml", wsl_home()?))
    }

    fn local_available(&self, local_path: &str) -> bool {
        // 宽松判定：文件或父目录存在即可（父目录存在 = 可新建）
        Path::new(local_path).exists()
            || Path::new(local_path)
                .parent()
                .map(|p| p.exists())
                .unwrap_or(false)
    }

    fn wsl_available(&self, probe: WslPathProbe) -> bool {
        // 已安装判定：配置文件或其目录存在
        probe.path_exists || probe.parent_dir_exists
    }

    fn detect(&self, content: &str, path: &str) -> bool {
        let lower = path.to_lowercase();
        if lower.ends_with(".json") || lower.ends_with(".jsonc") {
            return false; // JSON 文件归 pi 处理
        }
        let ext_yml = lower.ends_with(".yml") || lower.ends_with(".yaml");
        // 空内容（新建场景）：仅凭 .yml 扩展名归 omp
        if content.trim().is_empty() {
            return ext_yml;
        }
        // 无扩展名上下文时，JSON 语法内容让位给 pi（JSON 是 YAML 子集）
        if !ext_yml
            && parse_config_content(content)
                .map(|v| v.get("providers").and_then(|x| x.as_object()).is_some())
                .unwrap_or(false)
        {
            return false;
        }
        parse_yaml_content(content)
            .map(|v| {
                v.get("providers").and_then(|x| x.as_object()).is_some()
                    && v.get("provider").is_none()
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let v = parse_yaml_content(content)?;
        let providers = convert::load_pi_providers(&v);
        let extras = convert::load_pi_extras(&v);
        Ok(BackendLoad {
            root: v,
            agents: Vec::new(),
            providers,
            extras,
        })
    }

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // 跨格式目标：extras 取目标文件自身的顶层字段，仅重写 providers。
        // 目标已有同名 provider 时做保守合并（非编辑内容保留）；
        // 目标独有 provider 一律保留。当前文件保存时 extras 不含 providers。
        let base = match target_root {
            Some(target) => target,
            None => extras,
        };
        let mut root = base.as_object().cloned().unwrap_or_default();
        let mut providers_map = root
            .get("providers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for p in providers.iter().filter(|p| !p.key.is_empty()) {
            let value = provider_to_omp(p);
            let entry = match providers_map.get(&p.key) {
                Some(target) => convert::merge_conservative(target, &value),
                None => value,
            };
            providers_map.insert(p.key.clone(), entry);
        }
        root.insert("providers".into(), Value::Object(providers_map));
        Value::Object(root)
    }

    fn load_target_root(&self, path: &str) -> Value {
        // 跨格式目标保存需要目标文件完整的 providers（保守合并用）。
        match read_config_content(path) {
            Ok(content) => parse_yaml_content(&content).unwrap_or(Value::Object(Map::new())),
            Err(_) => Value::Object(Map::new()),
        }
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/oh-my-pi_32.bin"),
            32,
            32,
        ))
    }

    fn render(&self, root: &Value, _compact: bool) -> Result<String, String> {
        to_yaml_string(root)
    }
}
