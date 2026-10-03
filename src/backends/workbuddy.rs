//! WorkBuddy 后端：`~/.workbuddy/models.json`，顶层数组，provider 内联在每条模型里；协议由
//! `useCustomProtocol` + URL 后缀表达。配置分两份：主文件是 WorkBuddy 读的生效清单，只含
//! 勾选条目、每个 `id` 只留第一条；同目录 `models.full.json` 是全量副本，含逐条 `disabled` 标记。

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{home_dir_string, parse_config_content, read_config_content, wsl_home};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub struct WorkBuddyBackend;

pub static BACKEND: WorkBuddyBackend = WorkBuddyBackend;

/// 非 Chat 协议保存时补的 URL 后缀。
const MESSAGES_SUFFIX: &str = "/v1/messages";
const RESPONSES_SUFFIX: &str = "/v1/responses";

/// 全量副本的文件名，与 `models.json` 同目录。
/// WorkBuddy 只按精确文件名读 `models.json`。
const FULL_STORE_NAME: &str = "models.full.json";

fn default_local_path() -> String {
    format!("{}\\.workbuddy\\models.json", home_dir_string())
}

/// 全量副本路径：与主配置同目录、固定文件名，分隔符随主配置路径形态。
pub fn full_store_path(config_path: &str) -> String {
    crate::util::sibling_path(config_path, FULL_STORE_NAME)
}

/// 读全量副本的条目；不存在、非数组或解析失败返回 `None`。
fn load_full_store(config_path: &str) -> Option<Vec<Value>> {
    if config_path.trim().is_empty() {
        return None;
    }
    let text = read_config_content(&full_store_path(config_path)).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let root = parse_config_content(&text).ok()?;
    let items = entries_of(&root)?;
    if items.is_empty() {
        None
    } else {
        Some(items.clone())
    }
}

/// 合并两份配置，得到界面上要显示的全部条目。
///
/// 以全量副本为主（它带每条自己的勾选状态），再补入主配置里有、副本里没有的条目，
/// 补入的条目按已勾选处理。
fn merge_full_and_effective(full: &[Value], effective: &[Value]) -> Vec<Value> {
    let key_of = |v: &Value| {
        (
            v.get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            v.get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    };
    let known: HashSet<(String, String)> = full.iter().map(key_of).collect();
    let mut out = full.to_vec();
    for entry in effective {
        if known.contains(&key_of(entry)) {
            continue;
        }
        // 补入的条目按启用处理；`disabled` 键由 `all_entries` 落盘时决定。
        out.push(entry.clone());
    }
    out
}

/// 数组根 → 条目列表；兼容 `{ "models": [...] }` 形态。
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

/// 协议 → 保存时要补的后缀；Chat Completions 返回 `None`。
fn suffix_for_api(api: &str) -> Option<&'static str> {
    match api.trim() {
        "anthropic-messages" => Some(MESSAGES_SUFFIX),
        "openai-responses" => Some(RESPONSES_SUFFIX),
        _ => None,
    }
}

/// 把基址与后缀拼起来，幂等：先剥掉基址末尾的协议路径段
/// （`/messages`、`/responses`、`/chat/completions`、`/v1`），再补规范后缀。
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

/// 条目 → ProviderRow，一个条目一张卡片。
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

    // `id` = 模型名（发给 API 的模型），`name` = 提供商标签：
    // 模型行的 id ← 条目 `id`，provider 的 key ← 条目 `name`。
    let provider = v
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let mut model = ModelRow::new();
    model.id = id.clone();
    model.name = id.clone();
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
    // `disabled: true`：选择器里变灰、不可选，但仍在列表里。
    model.disabled = v.get("disabled").and_then(Value::as_bool).unwrap_or(false);
    // WorkBuddy 没有「思考档位」概念，清空 `ModelRow::new()` 带出的默认档位。
    model.variants.clear();
    model.original_variants.clear();
    model.source_format = Some(ConfigFormat::WorkBuddy);
    model.raw = v.clone();

    let mut row = ProviderRow::new();
    row.key = provider;
    row.description = v
        .get("vendor")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // 勾选自定义协议时协议由 URL 后缀决定，否则是 Chat Completions。
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

/// ProviderRow + 单个模型 → 条目里由界面接管的字段。
///
/// 只返回本函数管理的键；条目里其余的键（tags / credits / reasoning 等）由
/// [`WorkBuddyBackend::serialize_root`] 从同名同模型的旧条目继承。
/// `id` 是发给 API 的模型名，`name` 是提供商标签，两者不拼接。
fn entry_from_provider(p: &ProviderRow, model: Option<&ModelRow>) -> Value {
    let mut obj = Map::new();

    // `id` = 模型名（模型行的 id），`name` = 提供商（`ProviderRow.key`）。
    let model_id = model
        .map(|m| m.id.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(&p.key)
        .to_string();
    obj.insert("id".into(), Value::String(model_id));
    obj.insert("name".into(), Value::String(p.key.clone()));
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
        // 非 Chat 协议：URL 补后缀，并置 `useCustomProtocol = true`。
        Some(suffix) => {
            obj.insert(
                "url".into(),
                Value::String(with_suffix(&p.base_url, suffix)),
            );
            obj.insert("useCustomProtocol".into(), Value::Bool(true));
        }
        // Chat Completions：URL 存基址，后缀由 WorkBuddy 补。
        None => {
            obj.insert("url".into(), Value::String(p.base_url.clone()));
            obj.insert("useCustomProtocol".into(), Value::Bool(false));
        }
    }

    if let Some(m) = model {
        // 只写能解析成整数的值；解析不了则不写该键。
        if let Ok(context) = m.context.trim().parse::<i64>() {
            obj.insert("maxInputTokens".into(), Value::Number(context.into()));
        }
        if let Ok(output) = m.output.trim().parse::<i64>() {
            obj.insert("maxOutputTokens".into(), Value::Number(output.into()));
        }
        // 模态 / 能力：只在原条目已写该键时同步，不新增键。
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
        // 启用/停用不在这里写；勾选状态由全量副本逐条记录。
    }

    Value::Object(obj)
}

/// 按 (name, id) 二元组索引旧条目；不同 provider 可以有同 id 的模型。
fn existing_index(base: &Value) -> HashMap<(String, String), Value> {
    entries_of(base)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    let id = v.get("id").and_then(Value::as_str)?;
                    let name = v.get("name").and_then(Value::as_str).unwrap_or_default();
                    Some(((name.to_string(), id.to_string()), v.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 一家提供商的每个模型各写一条条目，不过滤、不去重。
///
/// 两份配置共用的构造步骤：全量副本要全部条目，生效清单再从结果里筛。
/// 勾选状态每条都显式写：勾选 `disabled: false`、未勾选 `true`。
fn all_entries(providers: &[ProviderRow], base: &Value) -> Vec<Value> {
    let existing = existing_index(base);
    let mut out: Vec<Value> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        // 没有模型的 provider 也留一条，id 回落到 provider key。
        let models: Vec<Option<&ModelRow>> = if p.models.is_empty() {
            vec![None]
        } else {
            p.models.iter().map(Some).collect()
        };
        for model in models {
            let model_id = model
                .map(|m| m.id.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(p.key.trim())
                .to_string();
            let mut entry = existing
                .get(&(p.key.trim().to_string(), model_id))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(fresh) = entry_from_provider(p, model).as_object() {
                for (k, v) in fresh {
                    entry.insert(k.clone(), v.clone());
                }
            }
            // 勾选 = `disabled: false`，未勾选 = `true`。
            entry.insert(
                "disabled".into(),
                Value::Bool(model.map(|m| m.disabled).unwrap_or(false)),
            );
            out.push(Value::Object(entry));
        }
    }
    out
}

/// 生效清单：取出勾选的条目，且每个 id 只留第一条。
///
/// WorkBuddy 的选择器按裸 id 全局去重，第二条起不生效；被筛掉的条目仍在全量副本里。
/// 留下的每条显式写 `disabled: false`。
fn effective_entries(all: &[Value]) -> Vec<Value> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for entry in all {
        if entry.get("disabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !seen.insert(id) {
            continue;
        }
        let mut entry = entry.clone();
        // 生效条目一律启用，显式写 `false`。
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("disabled".into(), Value::Bool(false));
        }
        out.push(entry);
    }
    out
}

/// 条目列表 → [`BackendLoad`]，两份配置共用。
///
/// 同 `name` 的条目合并成一张 provider 卡片，每条各成一个模型行。
///
/// `trusted_flags` 为 `true`（读全量副本）时逐条采信条目自己写的 `disabled` 键，没写该键
/// 的条目仍按位置推导（同一 id 只有第一条启用）；为 `false` 时全部按位置推导。
/// 末尾还有一道按 id 去重兜底：同一 id 至多一条启用，显式标记优先于位置推导。
fn build_load(entries: Vec<Value>, trusted_flags: bool) -> BackendLoad {
    // 第一遍：按文件顺序逐条定勾选状态，并记录该状态来自显式标记还是位置推导。
    let mut seen: HashSet<String> = HashSet::new();
    // (勾选状态, 是否来自显式标记)
    let mut flags: Vec<Option<(bool, bool)>> = Vec::with_capacity(entries.len());
    for entry in &entries {
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let Some(id) = id else {
            // 没有 id 的条目进不了界面；占位 `None` 让 flags 与 entries 同序。
            flags.push(None);
            continue;
        };
        let by_position = !seen.insert(id);
        // 是否显式写了 `disabled` 直接读条目本身，保证与 flags 同序。
        let explicit = trusted_flags && entry.get("disabled").and_then(Value::as_bool).is_some();
        let disabled = match (explicit, entry.get("disabled").and_then(Value::as_bool)) {
            (true, Some(flag)) => flag,
            _ => by_position,
        };
        flags.push(Some((disabled, explicit)));
    }

    // 第二遍：同一 id 若有多条判成启用，只留一条；显式标记优先，同源取最靠前的。
    let mut winner: HashMap<String, usize> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some((false, explicit)) = flags[i] else {
            continue;
        };
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        match winner.get(&id) {
            // 已有胜出者：仅当本条是显式标记、上一条不是时改判。
            Some(&prev) if explicit && !flags[prev].is_some_and(|(_, e)| e) => {
                winner.insert(id, i);
            }
            Some(_) => {}
            None => {
                winner.insert(id, i);
            }
        }
    }
    // 落败的启用条目一律关掉。
    for (i, entry) in entries.iter().enumerate() {
        if flags[i].map(|(disabled, _)| disabled) != Some(false) {
            continue;
        }
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        if winner.get(&id) != Some(&i) {
            flags[i] = Some((true, flags[i].is_some_and(|(_, e)| e)));
        }
    }

    let mut providers: Vec<ProviderRow> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(mut row) = provider_from_entry(entry) else {
            continue;
        };
        let Some(model) = row.models.first_mut() else {
            continue;
        };
        if let Some((disabled, _)) = flags[i] {
            model.disabled = disabled;
        }
        match providers.iter_mut().find(|p| p.key == row.key) {
            // 同 provider 以首条为准；后续条目只贡献模型行。
            Some(existing) => existing.models.push(row.models.remove(0)),
            None => providers.push(row),
        }
    }
    // extras 用原始条目数组：保存时按 (name, id) 继承未知字段取的就是它。
    let extras = Value::Array(entries);
    BackendLoad {
        root: extras.clone(),
        agents: Vec::new(),
        providers,
        extras,
    }
}

/// 数组根的 JSON 渲染：两空格缩进，`compact` 时去掉缩进。
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

    fn detect(&self, content: &str, _path: &str) -> bool {
        // 非空数组且首元素带 `id`；`[]` 不认。
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
        let entries = entries_of(&root).cloned().unwrap_or_default();
        Ok(build_load(entries, false))
    }

    /// 带路径的解析：优先读全量副本（`models.full.json`），它是界面上全部配置的来源。
    ///
    /// 副本不存在时退回 `models.json`，勾选状态按位置推导。
    /// 副本存在时仍读主配置并集进去，补入手改主配置新增的条目。
    fn parse_at(&self, content: &str, path: &str) -> Result<BackendLoad, String> {
        if let Some(full) = load_full_store(path) {
            let effective = parse_config_content(content)
                .ok()
                .and_then(|root| entries_of(&root).cloned())
                .unwrap_or_default();
            return Ok(build_load(
                merge_full_and_effective(&full, &effective),
                true,
            ));
        }
        self.parse(content)
    }

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // 产物是 WorkBuddy 的生效清单：只含勾选的条目、每个 `id` 只留第一条。
        // 全量副本由 `save_sidecars` 另写一份，两者共用 `all_entries` 构造。
        // `id` 既是选择器的去重键，也是发给上游的模型名，不能加序号。
        Value::Array(effective_entries(&all_entries(
            providers,
            target_root.unwrap_or(extras),
        )))
    }

    /// 全量副本（`models.full.json`）：所有条目 + 每条自己的勾选状态，即界面上全部配置
    /// 的落盘形态。取消勾选只是把条目从生效清单移到副本里，字段（含 API key）保留。
    fn save_sidecars(&self, path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        // 继承未知字段的基底取全量副本，读不出才退回主配置。
        // 基底里的条目不会因继承而复活：条目一律从 `providers` 生成。
        // 两者都读不出则取消保存，不静默用空基底。
        let base = match load_full_store(path).map(Value::Array) {
            Some(base) => base,
            None => self
                .load_target_root(path)
                .map_err(|e| format!("无法读取全量副本与主配置，已取消保存: {e}"))?,
        };
        let all = all_entries(providers, &base);
        super::write_config(&full_store_path(path), &render_array(&all, false))
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        // 顶层是数组（provider 内联在每条模型里），空 root 也是数组。
        super::load_target_root_with(path, parse_config_content, || Value::Array(Vec::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/workbuddy_32.bin"),
            32,
            32,
        ))
    }

    fn render(&self, root: &Value, compact: bool) -> Result<String, String> {
        // 数组根必须自己渲染；`app::pretty_json` 假定对象根。
        let items = entries_of(root).cloned().unwrap_or_default();
        Ok(render_array(&items, compact))
    }
}
