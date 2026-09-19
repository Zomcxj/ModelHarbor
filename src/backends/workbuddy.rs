//! WorkBuddy 后端：`~/.workbuddy/models.json`。
//!
//! 结构是**顶层数组**，provider 信息内联在每条模型里（WorkBuddy 自身按模型 id
//! 扁平管理，没有 provider 分组概念）：
//! ```jsonc
//! [ { "id", "name", "vendor", "url", "apiKey",
//!     "supportsToolCall", "supportsImages", "supportsReasoning",
//!     "useCustomProtocol", "maxInputTokens", "maxOutputTokens" } ]
//! ```
//!
//! 协议由 `useCustomProtocol` + URL 后缀共同表达，文件里没有协议字段：
//! - `false`（默认）：WorkBuddy 自动补 `/chat/completions`
//! - `true`：URL 原样使用，需自己写全（如 `.../v1/messages`）
//!
//! 因此界面上的协议选择在保存时落到两处：非 Chat 协议自动置
//! `useCustomProtocol = true` 并补上对应后缀。
//!
//! 渲染**不能**复用 `app::compact_json` / `pretty_json`——那两个函数写死了
//! `root.as_object()`，数组根经过它们会静默变成 `{}`，直接毁掉用户配置。

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{parse_config_content, read_config_content, wsl_home, WslPathProbe};
use serde_json::{Map, Value};
use std::path::Path;

pub struct WorkBuddyBackend;

pub static BACKEND: WorkBuddyBackend = WorkBuddyBackend;

/// 非 Chat 协议对应的 URL 后缀（保存时补上，与 ZCode 的后缀规则同源）。
const MESSAGES_SUFFIX: &str = "/v1/messages";
const RESPONSES_SUFFIX: &str = "/v1/responses";

fn default_local_path() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    format!("{}\\.workbuddy\\models.json", home)
}

/// 数组根 → 条目列表（兼容 `{ "models": [...] }` 形态，与 WorkBuddy 的
/// `extractLocalModels` 判定一致）。
fn entries_of(root: &Value) -> Option<&Vec<Value>> {
    if let Some(arr) = root.as_array() {
        return Some(arr);
    }
    root.get("models").and_then(Value::as_array)
}

/// 从 URL 后缀推导协议：`/messages` → Anthropic Messages、`/responses` → Responses，
/// 其余按 Chat Completions。
fn api_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let trimmed = path.trim_end_matches('/');
    if trimmed.ends_with("/messages") {
        "anthropic-messages".to_string()
    } else if trimmed.ends_with("/responses") {
        "openai-responses".to_string()
    } else {
        "openai-completions".to_string()
    }
}

/// 协议 → 保存时要补的后缀（Chat Completions 由 WorkBuddy 自己补，返回 None）。
fn suffix_for_api(api: &str) -> Option<&'static str> {
    match api.trim() {
        "anthropic-messages" => Some(MESSAGES_SUFFIX),
        "openai-responses" => Some(RESPONSES_SUFFIX),
        _ => None,
    }
}

/// 把基址与后缀拼起来，且**幂等**：先把基址末尾的协议路径段
/// （`/messages`、`/responses`、`/chat/completions`、`/v1`）全部剥掉，再补规范后缀。
///
/// 这样既避免「基址本就含 `/v1` + 后缀 `/v1/messages`」拼成 `/v1/v1/messages`，
/// 也能修复历史上已写坏的 `/v1/v1/messages`（下次保存自动收敛为 `/v1/messages`）。
fn with_suffix(url: &str, suffix: &str) -> String {
    let mut base = url.trim().trim_end_matches('/');
    loop {
        let stripped = base
            .strip_suffix("/messages")
            .or_else(|| base.strip_suffix("/responses"))
            .or_else(|| base.strip_suffix("/chat/completions"))
            .or_else(|| base.strip_suffix("/v1"))
            .map(|s| s.trim_end_matches('/'));
        match stripped {
            Some(s) if s != base => base = s,
            _ => break,
        }
    }
    format!("{}{}", base, suffix)
}

/// 条目 → ProviderRow（每个模型一张卡片，key 用模型 id）。
fn provider_from_entry(v: &Value) -> Option<ProviderRow> {
    let id = v.get("id").and_then(Value::as_str)?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let url = v.get("url").and_then(Value::as_str).unwrap_or_default();
    let custom = v
        .get("useCustomProtocol")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // WorkBuddy 的 `id` 是账号/提供商级的唯一键（用户起的名，如 `claude_justwoker`），
    // `name` 才是模型名（如 `Claude Opus 4.8`）。前者落到 ProviderRow.key（卡片身份），
    // 模型行只承载模型名——否则模型 id 会显示成提供商名。
    let model_name = v
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let mut model = ModelRow::new();
    model.id = model_name.clone();
    model.name = model_name;
    model.context = v
        .get("maxInputTokens")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    model.output = v
        .get("maxOutputTokens")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    model.modalities_input = convert::workbuddy_modalities_from_raw(v);
    model.tool_call = v
        .get("supportsToolCall")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    model.reasoning = v
        .get("supportsReasoning")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // WorkBuddy 用布尔 supportsReasoning 表达推理，没有「思考档位」概念。
    // ModelRow::new() 会带一组默认档位（medium/high/xhigh/max），若不清空，
    // 跨格式存到 ZCode 时会给每个模型凭空塞 reasoningLevel.values——这些档位
    // 是发明出来的，会污染 ZCode 配置（就是「保存后 ZCode 里不对」的根因）。
    model.variants.clear();
    model.original_variants.clear();
    model.source_format = Some(ConfigFormat::WorkBuddy);
    model.raw = v.clone();

    let mut row = ProviderRow::new();
    row.key = id;
    row.description = v
        .get("vendor")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // 勾选「自定义协议」时协议由 URL 后缀决定；未勾选就是 Chat Completions。
    row.pi_api = if custom {
        api_from_url(url)
    } else {
        "openai-completions".to_string()
    };
    row.base_url = url.to_string();
    row.api_key = v
        .get("apiKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    row.models = vec![model];
    row.source_format = Some(ConfigFormat::WorkBuddy);
    row.raw = v.clone();
    Some(row)
}

/// ProviderRow → 条目。
fn entry_from_provider(p: &ProviderRow) -> Value {
    // 以 raw 为基底保留 WorkBuddy 自有字段（tags / credits / reasoning 等）；
    // 但来自其它方言的 raw 必须全新构造，否则 group / access / api 会被写进来。
    let mut obj = if convert::is_zcode_shaped(&p.raw)
        || convert::is_opencode_shaped_provider(&p.raw)
        || convert::is_dsh_shaped_provider(&p.raw)
    {
        Map::new()
    } else {
        p.raw.as_object().cloned().unwrap_or_default()
    };
    let model = p.models.first();

    // WorkBuddy `id` = 账号键（= ProviderRow.key），`name` = 模型名。
    // 模型名取模型行 id（WB 页只显示这一个模型字段），回落 name、再回落账号键。
    obj.insert("id".into(), Value::String(p.key.clone()));
    let model_name = model
        .map(|m| {
            let id = m.id.trim();
            if id.is_empty() {
                m.name.trim()
            } else {
                id
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or(&p.key)
        .to_string();
    obj.insert("name".into(), Value::String(model_name));
    if !p.description.trim().is_empty() {
        obj.insert("vendor".into(), Value::String(p.description.clone()));
    }
    if p.api_key.trim().is_empty() {
        obj.remove("apiKey");
    } else {
        obj.insert("apiKey".into(), Value::String(p.api_key.clone()));
    }

    let api = p.effective_api();
    match suffix_for_api(&api) {
        // 非 Chat 协议：URL 原样使用并补上后缀，同时置 useCustomProtocol。
        Some(suffix) => {
            obj.insert(
                "url".into(),
                Value::String(with_suffix(&p.base_url, suffix)),
            );
            obj.insert("useCustomProtocol".into(), Value::Bool(true));
        }
        // Chat Completions：URL 存基址，由 WorkBuddy 自己补 /chat/completions。
        None => {
            obj.insert("url".into(), Value::String(p.base_url.clone()));
            obj.insert("useCustomProtocol".into(), Value::Bool(false));
        }
    }

    if let Some(m) = model {
        if !m.context.trim().is_empty() {
            obj.insert(
                "maxInputTokens".into(),
                Value::Number(m.context.trim().parse::<i64>().unwrap_or(0).into()),
            );
        }
        if !m.output.trim().is_empty() {
            obj.insert(
                "maxOutputTokens".into(),
                Value::Number(m.output.trim().parse::<i64>().unwrap_or(0).into()),
            );
        }
        // 模态 / 能力：只在原条目已写该键时同步，不凭空声明——
        // 补一个 `supportsReasoning: false` 会把「未声明」变成「明确不支持」。
        let mods: Vec<String> = m
            .modalities_input
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if obj.contains_key("supportsImages") && !mods.is_empty() {
            obj.insert(
                "supportsImages".into(),
                Value::Bool(mods.iter().any(|s| s == "image")),
            );
        }
        if obj.contains_key("supportsToolCall") {
            obj.insert("supportsToolCall".into(), Value::Bool(m.tool_call));
        }
        if obj.contains_key("supportsReasoning") {
            obj.insert("supportsReasoning".into(), Value::Bool(m.reasoning));
        }
    }

    Value::Object(obj)
}

/// 数组根的 JSON 渲染（两空格缩进，对齐 WorkBuddy 自己的
/// `JSON.stringify(v, null, 2)`；compact 版去掉缩进）。
fn render_array(items: &[Value], compact: bool) -> String {
    if compact {
        let inner: Vec<String> = items
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "null".into()))
            .collect();
        return format!("[{}]", inner.join(","));
    }
    let body =
        serde_json::to_string_pretty(&Value::Array(items.to_vec())).unwrap_or_else(|_| "[]".into());
    format!("{body}\n")
}

impl Backend for WorkBuddyBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::WorkBuddy
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.workbuddy/models.json", wsl_home()?))
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
        // 非空数组且首元素带 id（`[]` 不认，避免空文件误判成本格式）。
        parse_config_content(content)
            .map(|v| {
                entries_of(&v)
                    .and_then(|items| items.first())
                    .and_then(|first| first.get("id"))
                    .is_some()
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_config_content(content)?;
        let providers = entries_of(&root)
            .map(|items| items.iter().filter_map(provider_from_entry).collect())
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
        let base = target_root.unwrap_or(extras);
        // 未接管的旧条目按 id 保留（UI 里删掉的模型不再写回）。
        let existing: Map<String, Value> = entries_of(base)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| {
                        let id = v.get("id").and_then(Value::as_str)?;
                        Some((id.to_string(), v.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut out: Vec<Value> = Vec::new();
        for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
            // 同 id 的旧条目作基底（保留 tags 等未知键）。
            let mut entry = existing
                .get(&p.key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let fresh = entry_from_provider(p);
            if let Some(fresh) = fresh.as_object() {
                for (k, v) in fresh {
                    entry.insert(k.clone(), v.clone());
                }
            }
            out.push(Value::Object(entry));
        }
        Value::Array(out)
    }

    fn load_target_root(&self, path: &str) -> Value {
        match read_config_content(path) {
            Ok(content) => parse_config_content(&content).unwrap_or(Value::Array(Vec::new())),
            Err(_) => Value::Array(Vec::new()),
        }
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/workbuddy_32.bin"),
            32,
            32,
        ))
    }

    fn render(&self, root: &Value, compact: bool) -> Result<String, String> {
        // 数组根必须自己渲染：app::pretty_json 假定对象根，会把它变成 `{}`。
        let items = entries_of(root).cloned().unwrap_or_default();
        Ok(render_array(&items, compact))
    }
}
