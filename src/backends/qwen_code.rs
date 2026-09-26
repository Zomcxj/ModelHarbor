//! QwenCode 后端：`~/.qwen/settings.json`（JSONC）。
//!
//! 结构（官方 `model-providers.md`；本机 `@qwen-code/qwen-code@0.24.6` 包内文档核实）：
//!
//! ```jsonc
//! {
//!   "$version": 4,
//!   "env": { "DASHSCOPE_API_KEY": "sk-…" },        // 密钥的真正存放处
//!   "modelProviders": {
//!     "openai": [                                   // 键 = provider id
//!       { "id", "name", "description", "baseUrl", "envKey", "wireApi",
//!         "capabilities": { "vision", "reasoning": { "efforts", … } },
//!         "generationConfig": { "timeout", "contextWindowSize",
//!                               "samplingParams": { "max_tokens", … } } }
//!     ]
//!   },
//!   "providerProtocol": { "idealab": "openai" },    // 自定义 pid → 协议
//!   "security": { "auth": { "selectedType": "openai" } },
//!   "model": { "name": "…" }
//! }
//! ```
//!
//! ## 四个决定建模方式的要点
//!
//! 1. **一条条目 = 一张卡片**（与 WorkBuddy 同构）。`modelProviders[<pid>]` 的值是数组，
//!    每条元素**自带 `baseUrl` + `envKey` + `generationConfig`**——官方示例里 `openai`
//!    这一个 pid 下就混着 `api.openai.com`、`openrouter.ai`、`requesty.ai` 三家不同端点
//!    与密钥。若按「pid = 一张卡片、模型挂其下」建模，同 pid 不同 baseUrl 的条目会被合并，
//!    端点与密钥必然串味。所以卡片名用条目的 `id`，条目属于哪个 pid 记在
//!    [`ProviderRow::qwen_pid`] 里（写回时要用）。
//!
//! 2. **密钥不在条目里**，在顶层 `env[<envKey>]`。读时 join，写时同步写回 `env`。
//!    见 [`sync_env`]——那里有一条硬规则：**只增改、绝不删**。
//!
//! 3. **自定义 pid 必须配 `providerProtocol`**，否则整条被**静默跳过**（官方 warning）。
//!    内置 pid（`openai` / `anthropic` / `gemini` / `vertex-ai` / `qwen-oauth`）不需要映射。
//!    `qwen-oauth` 是硬编码的（"cannot be overridden"），那条 pid 下的条目一律只读。
//!
//! 4. **`$version` 是「已迁移」标志**，只在**全新文件**里写。已有文件一律不动这个键：
//!    往一个 v1/v2 文件里补 `$version: 4` 会让 Qwen Code 跳过它自己的 v1→v4 迁移，
//!    旧结构的设置被按新结构解读——那是在帮用户改坏配置。
//!
//! ## 没有「停用」这回事
//!
//! QwenCode 的 schema 里没有 `disabled` 字段，`/model` 选择器对不同厂商的重复模型也
//! 照列不误（判重只针对「协议 + id + baseUrl」完全相同的三重重复，跨厂商撞不上）——
//! 既没有需要开关去裁决的去重，也没有可写的开关字段。曾经照 WorkBuddy 的样子给这里
//! 加过停用开关与全量副本 `modelProviders.full.json`，按用户指正移除：那是本工具发明
//! 的状态。删卡片就是真删（`.bak` 备份仍然兜底）。
//!
//! ## 两条「认不出来就原样保留」的规则
//!
//! 界面只重建自己认得的条目，所以有两类内容必须显式带过去，否则一次保存就没了：
//!
//! - **`qwen-oauth` 的条目**：官方硬编码、不可覆盖，界面也不显示，只能整块原样保留。
//! - **没有可用 `id` 的条目 / 值不是数组的 pid**（旧版包装形状 `{protocol, models}`）：
//!   它们进不了界面，`settings.json` 里也就没有对应卡片，删掉就等于静默丢配置。
//!   见 [`unmanaged_of`]。
//!
//! 反过来，**认得出来**的条目一律以界面状态为准——包括用户把卡片删掉的情况。所以
//! 「保留」只针对认不出来的内容，不会让删掉的条目复活。

use super::{Backend, BackendLoad};
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{home_dir_string, parse_config_content, wsl_home};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

pub struct QwenCodeBackend;

pub static BACKEND: QwenCodeBackend = QwenCodeBackend;

/// 内置 pid：协议由 pid 自身决定，**不需要** `providerProtocol` 映射。
const BUILTIN_PIDS: [&str; 5] = ["openai", "anthropic", "gemini", "vertex-ai", "qwen-oauth"];

/// 只读 pid：Qwen Code 自己的 OAuth 托管条目，写进去也会被忽略，改坏还会破坏登录态。
const READONLY_PID: &str = "qwen-oauth";

/// `wireApi` 的两个合法值（只对 OpenAI 系协议有效）。
const WIRE_CHAT: &str = "chat-completions";
const WIRE_RESPONSES: &str = "responses";

fn default_local_path() -> String {
    format!("{}\\.qwen\\settings.json", home_dir_string())
}

/// 本次保存是否会让条目数变少（供保存流程决定是否先备份原文件）。
///
/// 删卡片就会收缩——没有停用概念之后，`.bak` 兜底的对象是「删除」。
/// 原文件读不出时按「会收缩」处理，让调用方去读原文件并备份。
pub fn shrinks_on_save(before: &Value, after: &Value) -> bool {
    fn count(root: &Value) -> usize {
        providers_map(root)
            .map(|m| {
                m.values()
                    .filter_map(Value::as_array)
                    .map(|a| a.iter().filter(|e| entry_id(e).is_some()).count())
                    .sum()
            })
            .unwrap_or(0)
    }
    count(before) > count(after)
}

/// `modelProviders` 对象；不是对象时返回 None。
fn providers_map(root: &Value) -> Option<&Map<String, Value>> {
    root.get("modelProviders").and_then(Value::as_object)
}

/// 顶层 `providerProtocol` 映射；缺失 / 不是对象时返回 None。
fn provider_protocols(root: &Value) -> Option<&Map<String, Value>> {
    root.get("providerProtocol").and_then(Value::as_object)
}

/// 条目的可用 `id`（空白不算）。
fn entry_id(entry: &Value) -> Option<&str> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// 某个 pid 的条目数组；值不是数组时返回空。
fn entries_of_pid<'a>(root: &'a Value, pid: &str) -> &'a [Value] {
    providers_map(root)
        .and_then(|m| m.get(pid))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn is_builtin_pid(pid: &str) -> bool {
    BUILTIN_PIDS.contains(&pid)
}

/// 是否是那个只读的内置 pid（`qwen-oauth`）。
///
/// 公开是因为 `app::strip_cross_format_containers` 也要用：跨格式保存时界面接管
/// `modelProviders`，但 `qwen-oauth` 不归界面管，必须原样留着。
pub fn is_readonly_provider(pid: &str) -> bool {
    pid == READONLY_PID
}

/// 内部协议 → (内置 pid, 该条目要写的 `wireApi`)。
///
/// 一律回落到**内置 pid**：官方最稳的写法，不需要额外维护映射。OpenAI 的两种协议共用
/// `openai` 这一个 pid，靠条目自己的 `wireApi` 区分。
fn pid_for_api(api: &str) -> (&'static str, &'static str) {
    match api.trim() {
        "openai-responses" => ("openai", WIRE_RESPONSES),
        "anthropic-messages" => ("anthropic", ""),
        "google-generative-ai" | "google-vertex" | "google-gemini-cli" => ("gemini", ""),
        _ => ("openai", WIRE_CHAT),
    }
}

/// 协议桶名（内置 pid 名，或 `providerProtocol` 的值）→ 内部协议。
///
/// `wireApi` **优先于桶名**（官方："explicit `wireApi` takes precedence over either
/// OpenAI protocol"）；非 OpenAI 系协议没有 `wireApi` 概念，写了反而是配置错误。
fn bucket_to_api(bucket: &str, wire: &str) -> String {
    match bucket {
        "openai" => {
            if wire == WIRE_RESPONSES {
                "openai-responses".to_string()
            } else {
                "openai-completions".to_string()
            }
        }
        // 历史写法：桶名直接叫 `openai-responses`（v0.23.3 起可读，写只用新格式）。
        "openai-responses" => "openai-responses".to_string(),
        "anthropic" => "anthropic-messages".to_string(),
        "gemini" | "vertex-ai" => "google-generative-ai".to_string(),
        "qwen-oauth" => "openai-completions".to_string(),
        // 未映射的自定义 pid：协议不可知。条目本身会被 Qwen Code 跳过，但界面仍要显示它，
        // 让用户看得见并去修好，所以这里给一个兜底协议而不是把条目藏起来。
        _ => "openai-completions".to_string(),
    }
}

/// 某个 pid 的协议桶名：内置 pid 就是它自己，自定义 pid 查 `providerProtocol`。
fn bucket_of(protocols: Option<&Map<String, Value>>, pid: &str) -> String {
    if is_builtin_pid(pid) {
        return pid.to_string();
    }
    protocols
        .and_then(|m| m.get(pid))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// 文件里的全部条目，按**文件顺序**：`(pid, 条目)`。
///
/// 跳过 `qwen-oauth`（只读，不进界面）与没有 `id` 的条目（认不出来，由
/// [`unmanaged_of`] 原样保留）。
fn entries_from(root: &Value) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    let Some(m) = providers_map(root) else {
        return out;
    };
    for (pid, value) in m {
        if is_readonly_provider(pid) {
            continue;
        }
        for entry in value.as_array().map(Vec::as_slice).unwrap_or_default() {
            if entry_id(entry).is_some() {
                out.push((pid.clone(), entry.clone()));
            }
        }
    }
    out
}

/// 条目 → ProviderRow（一条条目 = 一张卡片）。
fn provider_from_entry(
    pid: &str,
    protocols: Option<&Map<String, Value>>,
    entry: &Value,
    env: Option<&Map<String, Value>>,
) -> Option<ProviderRow> {
    let id = entry_id(entry)?.to_string();
    let wire = entry.get("wireApi").and_then(Value::as_str).unwrap_or("");
    let bucket = bucket_of(protocols, pid);
    let env_key = entry
        .get("envKey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let mut row = ProviderRow::new();
    row.key = id;
    // 写回时要知道这条属于哪个 pid，见模块说明第 1 点。
    row.qwen_pid = pid.to_string();
    row.pi_api = if bucket.is_empty() {
        "openai-completions".to_string()
    } else {
        bucket_to_api(&bucket, wire)
    };
    row.description = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.base_url = entry
        .get("baseUrl")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 密钥在顶层 `env[<envKey>]`；`envKey` 本身也存下来（写回时要同步维护 `env`）。
    row.api_key_env = env_key.clone();
    row.original_api_key_env = env_key.clone();
    row.api_key = env
        .and_then(|m| m.get(&env_key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 条目没写 timeout 时留空，避免「没配过的字段」在下次保存时被填上一个默认值。
    row.timeout = entry
        .get("generationConfig")
        .and_then(|g| g.get("timeout"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    row.models = vec![model_from_entry(entry)];
    row.source_format = Some(ConfigFormat::QwenCode);
    row.raw = entry.clone();
    Some(row)
}

/// 条目 → ModelRow。一条条目只有一个模型，字段全在同一条目上。
fn model_from_entry(entry: &Value) -> ModelRow {
    let mut m = ModelRow::new();
    m.id = entry_id(entry).unwrap_or_default().to_string();
    m.name = entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&m.id)
        .to_string();
    // 逐个覆盖 `ModelRow::new()` 的默认值：条目没声明的字段一律留空，不能沿用
    // 「新建模型」的预填值——那会把默认的上下文/输出/档位当成用户配置写进文件。
    m.context = entry
        .get("generationConfig")
        .and_then(|g| g.get("contextWindowSize"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    m.output = entry
        .get("generationConfig")
        .and_then(|g| g.get("samplingParams"))
        .and_then(|s| s.get("max_tokens"))
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    m.modalities_input = modalities_from_entry(entry);
    m.reasoning = entry
        .get("capabilities")
        .and_then(|c| c.get("reasoning"))
        .is_some_and(|r| r != &Value::Bool(false));
    // QwenCode 没有「工具调用」与 `store` 字段：不建模，也就不会凭空写出这两个键。
    m.tool_call = false;
    m.store = false;
    m.variants = efforts_from_entry(entry).join(", ");
    m.original_variants = m.variants.clone();
    m.source_format = Some(ConfigFormat::QwenCode);
    m.raw = entry.clone();
    m
}

/// 条目的输入模态：`capabilities.vision` → `text[, image]`。
///
/// **未声明时返回空串**（不返回 `"text"`）：那会把「未声明」变成「只支持文本」，
/// 跨格式写出时给别的后端凭空补字段。与 WorkBuddy 同一口径。
fn modalities_from_entry(entry: &Value) -> String {
    match entry
        .get("capabilities")
        .and_then(|c| c.get("vision"))
        .and_then(Value::as_bool)
    {
        Some(vision) => crate::convert::supports_to_modalities([("text", true), ("image", vision)]),
        None => String::new(),
    }
}

/// 条目的思考档位：`capabilities.reasoning.efforts`。
fn efforts_from_entry(entry: &Value) -> Vec<String> {
    entry
        .get("capabilities")
        .and_then(|c| c.get("reasoning"))
        .and_then(|r| r.get("efforts"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 逗号串 → 列表（去空白、去空项）。
fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 一张卡片该落到哪个 pid、要不要写 `providerProtocol`、条目要写什么 `wireApi`。
///
/// 优先沿用卡片上原有的**自定义** pid（保留用户自己的命名），只把映射刷新成当前协议；
/// 新建卡片（或从别的后端转来）没有 pid，就回落到内置 pid。
///
/// 沿用自定义 pid 有个前提：它**本来就在映射表里**。映射表里没有的自定义 pid 是个没人
/// 认得的键，写出去只会让 Qwen Code 把整条静默跳过——那种情况回落到内置 pid 才是对的。
fn route(
    p: &ProviderRow,
    api: &str,
    protocols: Option<&Map<String, Value>>,
) -> (String, Option<String>, &'static str) {
    let (builtin_pid, wire) = pid_for_api(api);
    let custom = p.qwen_pid.trim();
    if !custom.is_empty() && !is_builtin_pid(custom) && !is_readonly_provider(custom) {
        let known = protocols
            .and_then(|m| m.get(custom))
            .and_then(Value::as_str)
            .is_some();
        if known {
            return (custom.to_string(), Some(builtin_pid.to_string()), wire);
        }
    }
    (builtin_pid.to_string(), None, wire)
}

/// 一条待写入的条目：落在哪个 pid、该 pid 需要的映射、条目内容。
struct Placed {
    pid: String,
    /// `Some(协议桶名)` = 这个 pid 是自定义的，要在 `providerProtocol` 里声明。
    mapping: Option<String>,
    entry: Value,
}

/// 界面状态 → 全部条目（`serialize_root` 唯一的条目来源）。
fn place(providers: &[ProviderRow], base: &Value) -> Vec<Placed> {
    let protocols = provider_protocols(base);
    let mut out: Vec<Placed> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let api = p.effective_api();
        let (pid, mapping, wire) = route(p, &api, protocols);
        if is_readonly_provider(&pid) {
            // 理论到不了：`route` 不会产出只读 pid。留一道闸门，别让只读条目被重建。
            continue;
        }
        // 旧条目按 id 建索引：界面没接管的键（`capabilities.agent`、
        // `generationConfig.maxRetries` 等）要从这里继承。
        let index: HashMap<&str, &Value> = entries_of_pid(base, &pid)
            .iter()
            .filter_map(|e| entry_id(e).map(|id| (id, e)))
            .collect();
        // 没有模型的卡片也要留一条（id 回落到 key），否则刚建好还没填模型的卡片
        // 一保存就消失。
        let models: Vec<Option<&ModelRow>> = if p.models.is_empty() {
            vec![None]
        } else {
            p.models.iter().map(Some).collect()
        };
        for m in models {
            let id = m
                .map(|m| m.id.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| p.key.trim())
                .to_string();
            let old = index.get(id.as_str()).copied();
            let entry = entry_from_provider(p, m, &id, wire, old);
            out.push(Placed {
                pid: pid.clone(),
                mapping: mapping.clone(),
                entry,
            });
        }
    }
    out
}

/// 卡片 + 模型 → 条目里**由界面接管**的字段。
///
/// 以**旧条目**为基底（`old`），逐个覆盖界面接管的键；界面没接管的键原样留着——
/// 官方把 `generationConfig` 说成「完全替换」的原子层，但界面只接管其中三个子键，
/// 其余（`maxRetries` / `customHeaders` / `extra_body` / 其它采样参数）是用户自己的
/// 设置，必须保留，所以这里做**子键级**合并。
///
/// 界面清空的字段要**删掉**对应的键，而不是写空值：`envKey: ""` 会被当成一个真的
/// 空变量名，`wireApi` 写在非 OpenAI 系协议上是官方明确的配置错误。
fn entry_from_provider(
    p: &ProviderRow,
    m: Option<&ModelRow>,
    id: &str,
    wire: &str,
    old: Option<&Value>,
) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    obj.insert("id".into(), Value::String(id.to_string()));
    set_or_remove(
        &mut obj,
        "name",
        m.map(|m| m.name.trim()).unwrap_or_default(),
    );
    set_or_remove(&mut obj, "description", p.description.trim());
    set_or_remove(&mut obj, "baseUrl", p.base_url.trim());
    set_or_remove(&mut obj, "envKey", &env_key_name(p));
    // `wireApi` 只对 OpenAI 系协议有意义，其它协议写了是配置错误。
    set_or_remove(&mut obj, "wireApi", wire);

    if let Some(m) = m {
        // capabilities：只在界面能表达的两个子键上动手，其余（`agent` /
        // `supportsImageGeneration` / `reasoning.profile` …）从旧条目继承。
        let mut caps = obj
            .get("capabilities")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mods = split_list(&m.modalities_input);
        if mods.is_empty() {
            caps.shift_remove("vision");
        } else {
            caps.insert(
                "vision".into(),
                Value::Bool(mods.iter().any(|s| s.eq_ignore_ascii_case("image"))),
            );
        }
        // 勾选框就是「声明 / 不声明」这个键：勾上写、取消删。条目里本就没有时
        // 取消勾选是无操作，来回保存稳定。
        if m.reasoning {
            let mut r = caps
                .get("reasoning")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let efforts = split_list(&m.variants);
            if efforts.is_empty() {
                r.shift_remove("efforts");
            } else {
                r.insert(
                    "efforts".into(),
                    Value::Array(efforts.into_iter().map(Value::String).collect()),
                );
            }
            caps.insert("reasoning".into(), Value::Object(r));
        } else {
            caps.shift_remove("reasoning");
        }
        if caps.is_empty() {
            obj.shift_remove("capabilities");
        } else {
            obj.insert("capabilities".into(), Value::Object(caps));
        }

        // generationConfig：同上，只动 `contextWindowSize` / `timeout` /
        // `samplingParams.max_tokens` 三个子键。
        let mut gen = obj
            .get("generationConfig")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        set_num(&mut gen, "contextWindowSize", m.context.trim());
        set_num(&mut gen, "timeout", p.timeout.trim());
        let mut sp = gen
            .get("samplingParams")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        set_num(&mut sp, "max_tokens", m.output.trim());
        if sp.is_empty() {
            gen.shift_remove("samplingParams");
        } else {
            gen.insert("samplingParams".into(), Value::Object(sp));
        }
        if gen.is_empty() {
            obj.shift_remove("generationConfig");
        } else {
            obj.insert("generationConfig".into(), Value::Object(gen));
        }
    }

    Value::Object(obj)
}

/// 非空则写入字符串，空白则删除该键。
fn set_or_remove(obj: &mut Map<String, Value>, key: &str, value: &str) {
    if value.is_empty() {
        obj.shift_remove(key);
    } else {
        obj.insert(key.to_string(), Value::String(value.to_string()));
    }
}

/// 能解析成整数才写入；解析不了（含空串）就删除该键。
///
/// 写成 `unwrap_or(0)` 会把上限归零落盘，与 zcode 的 `contextWindow` 同一条教训。
fn set_num(obj: &mut Map<String, Value>, key: &str, text: &str) {
    match text.parse::<i64>() {
        Ok(n) => {
            obj.insert(key.to_string(), Value::Number(n.into()));
        }
        Err(_) => {
            obj.shift_remove(key);
        }
    }
}

/// 按 pid 分组（没有停用概念，全部条目都进主配置）。
fn group_by_pid(placed: &[Placed]) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for item in placed {
        push_entry(&mut out, &item.pid, item.entry.clone());
    }
    out
}

fn push_entry(map: &mut Map<String, Value>, pid: &str, entry: Value) {
    match map.get_mut(pid).and_then(Value::as_array_mut) {
        Some(arr) => arr.push(entry),
        None => {
            map.insert(pid.to_string(), Value::Array(vec![entry]));
        }
    }
}

/// `providerProtocol` 的内容：由界面条目推导，并保留「认不出来的 pid」原有的映射。
///
/// 界面不再需要的自定义 pid（卡片被删掉了）其映射会被清掉——留着就是一个指向不存在
/// provider 的孤儿声明。官方要求自定义 pid 必须有映射，所以这里也顺带保证了
/// 「有自定义 pid 条目 ⇒ 有映射」。
fn protocols_for(placed: &[Placed], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for item in placed {
        if let Some(mapping) = &item.mapping {
            out.insert(item.pid.clone(), Value::String(mapping.clone()));
        }
    }
    if let (Some(old), Some(map)) = (provider_protocols(base), providers_map(base)) {
        for (pid, value) in old {
            // 值不是数组的 pid 是旧包装形状，整块由 `unmanaged_of` 原样带过去，
            // 它的映射自然也该留着。
            let unmanaged = map.get(pid).is_some_and(|v| !v.is_array());
            if unmanaged && !out.contains_key(pid) {
                out.insert(pid.clone(), value.clone());
            }
        }
    }
    out
}

/// 基座里**认不出来**的条目，必须原样带过去（见模块说明末节）。
///
/// 返回 `pid → 该 pid 要补写的内容`：数组表示「补进这个 pid 的数组里」，其它值表示
/// 「整块替换这个 pid」。后者只在该 pid 本次没有界面条目时才产出。
fn unmanaged_of(base: &Value, placed: &[Placed]) -> Vec<(String, Value)> {
    let managed: HashSet<&str> = placed.iter().map(|p| p.pid.as_str()).collect();
    let mut out: Vec<(String, Value)> = Vec::new();
    let Some(m) = providers_map(base) else {
        return out;
    };
    for (pid, value) in m {
        // `qwen-oauth` 整块原样保留：官方硬编码、不可覆盖，界面也不显示它。
        if is_readonly_provider(pid) {
            out.push((pid.clone(), value.clone()));
            continue;
        }
        match value.as_array() {
            Some(items) => {
                let extra: Vec<Value> = items
                    .iter()
                    .filter(|e| entry_id(e).is_none())
                    .cloned()
                    .collect();
                if !extra.is_empty() {
                    out.push((pid.clone(), Value::Array(extra)));
                }
            }
            // 值不是数组（旧包装形状 `{protocol, models}`）：整块保留。只有本次没有
            // 界面条目落到这个 pid 时才这么做——有的话我们的数组会把它顶掉，而那种
            // 情况本就不该出现（那个形状解析不出卡片）。
            None => {
                if !managed.contains(pid.as_str()) {
                    out.push((pid.clone(), value.clone()));
                }
            }
        }
    }
    out
}

/// 把界面上的密钥写回顶层 `env`。
///
/// **只增改，绝不删。** `env` 是跨 provider 共享的扁平命名空间，而且**不归本工具独有**：
/// Qwen Code 自己的 `/auth` 也往里写（例如 Coding Plan 的
/// `BAILIAN_CODING_PLAN_API_KEY`）。按「有没有条目引用」去修剪，就会把用户刚用 `/auth`
/// 配好的凭据静默删掉——那是不可恢复的。孤立键对 Qwen Code 无害（它只按 `envKey` 查），
/// 所以宁可留一个没人引用的键，也不删。
///
/// 界面清空密钥时同样不动 `env`：那个值可能是 `/auth` 写的、也可能是用户手填的，
/// 本工具无从区分「清空」与「不接管」。清掉字段保存后密钥还在，是可恢复的；
/// 反过来误删一个凭据则不可恢复。
fn sync_env(root: &mut Map<String, Value>, providers: &[ProviderRow]) {
    let mut env = root
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for p in providers {
        let key = env_key_name(p);
        if key.is_empty() || p.api_key.trim().is_empty() {
            continue;
        }
        env.insert(key, Value::String(p.api_key.clone()));
    }
    if !env.is_empty() {
        root.insert("env".into(), Value::Object(env));
    }
}

/// 写盘前的总闸：两个 provider 的凭据若落到**同一个** `envKey` 变量名上，
/// [`sync_env`] 后写的会覆盖先写的——两个条目都指向它，其中一个必然拿错密钥，
/// 且无任何报错。这种冲突宁可不让存，请给卡片手填不同的变量名。
fn first_env_name_conflict(providers: &[ProviderRow]) -> Option<String> {
    // name -> (先到的 provider key, 它的密钥值)
    let mut seen: std::collections::HashMap<String, (&str, &str)> =
        std::collections::HashMap::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let name = env_key_name(p);
        if name.is_empty() {
            continue;
        }
        if let Some((prev_key, prev_value)) = seen.get(name.as_str()) {
            // 同一个变量名 + 同一个密钥值（比如同一站点复制出两条）：共享无妨。
            if *prev_value == p.api_key.trim() {
                continue;
            }
            return Some(format!(
                "envKey \"{}\" 被 provider \"{}\" 与 \"{}\" 同时占用，密钥却不同",
                name,
                prev_key,
                p.key.trim()
            ));
        }
        seen.insert(name, (p.key.trim(), p.api_key.trim()));
    }
    None
}

/// 条目要用的 `envKey` 变量名：界面填了就用界面值；只填了密钥没填变量名时，按
/// provider key 推一个（与 DSH 的 [`crate::credentials::default_env_name`] 同一约定）。
///
/// 后一种情况就是**跨格式复制**：opencode 系的密钥内联在条目里，复制到 Qwen 页时
/// `api_key` 有值而 `api_key_env` 为空——不推一个名字，`sync_env` 会跳过、条目上也不写
/// `envKey`，密钥被静默丢掉，CLI 报 "Missing credentials for modelProviders model …"。
///
/// 「界面明确清空过」要跟「从来没填过」区分开（与 `sync_provider_secrets` 对 DSH
/// 密钥的判据同形）：原来有变量名、现在空 = 用户清掉的，尊重清空，不推新名；
/// 本来就空（跨格式来的行、新建的行）才推导。变量名与密钥都空也返回空串。
fn env_key_name(p: &ProviderRow) -> String {
    let named = p.api_key_env.trim();
    if !named.is_empty() {
        return named.to_string();
    }
    if !p.original_api_key_env.trim().is_empty() || p.api_key.trim().is_empty() {
        return String::new();
    }
    crate::credentials::default_env_name(&p.key)
}

/// 条目列表 → [`BackendLoad`]。
fn build_load(
    entries: Vec<(String, Value)>,
    env: Option<&Map<String, Value>>,
    protocols: Option<&Map<String, Value>>,
) -> BackendLoad {
    let mut providers: Vec<ProviderRow> = Vec::new();
    for (pid, entry) in &entries {
        let Some(row) = provider_from_entry(pid, protocols, entry, env) else {
            continue;
        };
        providers.push(row);
    }
    BackendLoad {
        root: Value::Object(Map::new()),
        agents: Vec::new(),
        providers,
        extras: Value::Object(Map::new()),
    }
}

impl Backend for QwenCodeBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::QwenCode
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.qwen/settings.json", wsl_home()?))
    }

    fn detect(&self, content: &str, path: &str) -> bool {
        // 路径强命中：Qwen Code 的用户级设置就在这个位置，文件名也是它自己写死的。
        let normalized = path.replace('\\', "/");
        if normalized.contains("/.qwen/") && normalized.ends_with("settings.json") {
            return true;
        }
        // 内容判定：`modelProviders` 是对象，且至少一个值是非空数组、元素是含 `id`
        // 的对象。只验键名不够——同名不同形的东西多得是。
        parse_config_content(content)
            .map(|v| {
                providers_map(&v).is_some_and(|m| {
                    m.values().any(|value| {
                        value
                            .as_array()
                            .is_some_and(|items| items.iter().any(|it| entry_id(it).is_some()))
                    })
                })
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_config_content(content)?;
        let env = root.get("env").and_then(Value::as_object).cloned();
        let protocols = provider_protocols(&root).cloned();
        let mut load = build_load(entries_from(&root), env.as_ref(), protocols.as_ref());
        load.root = root.clone();
        load.extras = root;
        Ok(load)
    }

    // `parse_at` 用 trait 缺省实现（直接 `parse`）：没有全量副本可读，
    // 主配置就是全部状态。

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // 这个函数的产物就是 **Qwen Code 的全部生效状态**：没有停用概念，
        // `settings.json` 一份文件承担所有条目。
        let base = target_root.unwrap_or(extras);
        let placed = place(providers, base);
        let mut root = base.as_object().cloned().unwrap_or_default();
        // `$version` 只给**全新文件**打标（此时 root 是空的，没有任何旧结构要迁移）。
        // 已有文件一律不动这个键：往 v1/v2 文件里补 4 会让 Qwen Code 跳过自己的迁移，
        // 把旧结构的设置按新结构解读。
        if root.is_empty() {
            root.insert("$version".into(), Value::Number(4.into()));
        }
        sync_env(&mut root, providers);

        let mut map = group_by_pid(&placed);
        for (pid, extra) in unmanaged_of(base, &placed) {
            match extra {
                Value::Array(items) => {
                    for entry in items {
                        push_entry(&mut map, &pid, entry);
                    }
                }
                other => {
                    if !map.contains_key(&pid) {
                        map.insert(pid, other);
                    }
                }
            }
        }
        root.insert("modelProviders".into(), Value::Object(map));

        let protocols = protocols_for(&placed, base);
        if protocols.is_empty() {
            root.shift_remove("providerProtocol");
        } else {
            root.insert("providerProtocol".into(), Value::Object(protocols));
        }
        Value::Object(root)
    }

    /// 没有全量副本要写（见模块说明「没有『停用』这回事」），但仍挂在保存序列里：
    /// [`first_env_name_conflict`] 要在主配置落盘**之前**挡住 envKey 撞名——那种文件
    /// 写得出去，Qwen Code 也读得进去，但其中一条 provider 会静默拿到别人的密钥。
    fn save_sidecars(&self, _path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        if let Some(conflict) = first_env_name_conflict(providers) {
            return Err(format!("凭据变量名冲突，已取消保存: {conflict}"));
        }
        Ok(())
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        super::load_target_root_with(path, parse_config_content, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/qwen-code_32.bin"),
            32,
            32,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只读 pid 的条目整块保留，且不进界面。
    #[test]
    fn readonly_pid_entries_are_preserved_but_not_listed() {
        let root: Value = serde_json::from_str(
            r#"{
                "modelProviders": {
                    "qwen-oauth": [ { "id": "qwen3.5-plus", "name": "Qwen3.5 Plus" } ],
                    "openai": [ { "id": "gpt-4o", "baseUrl": "https://api.openai.com/v1" } ]
                }
            }"#,
        )
        .unwrap();
        let listed = entries_from(&root);
        assert_eq!(listed.len(), 1, "只读 pid 不进界面");
        assert_eq!(listed[0].0, "openai");

        let placed = place(&[], &root);
        let out = QwenCodeBackend.serialize_root(&[], &[], &root, None);
        let kept = entries_of_pid(&out, READONLY_PID);
        assert_eq!(kept.len(), 1, "只读条目必须原样留在文件里");
        assert_eq!(kept[0]["id"], "qwen3.5-plus");
        let _ = placed;
    }

    /// 认不出来的条目（没有 id）不能被保存动作删掉。
    #[test]
    fn id_less_entries_survive_a_save() {
        let root: Value = serde_json::from_str(
            r#"{ "modelProviders": { "openai": [ { "name": "无 id 的条目" } ] } }"#,
        )
        .unwrap();
        let out = QwenCodeBackend.serialize_root(&[], &[], &root, None);
        let kept = entries_of_pid(&out, "openai");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["name"], "无 id 的条目");
    }

    /// 旧包装形状（值不是数组）整块保留。
    #[test]
    fn legacy_wrapped_shape_is_preserved() {
        let root: Value = serde_json::from_str(
            r#"{ "modelProviders": { "legacy": { "protocol": "openai", "models": [] } } }"#,
        )
        .unwrap();
        let out = QwenCodeBackend.serialize_root(&[], &[], &root, None);
        assert_eq!(out["modelProviders"]["legacy"]["protocol"], "openai");
    }

    /// 全新文件写 `$version: 4`；已有文件不动它。
    #[test]
    fn version_is_only_stamped_on_a_new_file() {
        let fresh = QwenCodeBackend.serialize_root(&[], &[], &Value::Object(Map::new()), None);
        assert_eq!(fresh["$version"], 4);

        let old: Value = serde_json::from_str(r#"{ "$version": 2, "general": {} }"#).unwrap();
        let out = QwenCodeBackend.serialize_root(&[], &[], &old, None);
        assert_eq!(out["$version"], 2, "已有文件的版本号不得被改写");
        assert!(out.get("general").is_some(), "其余顶层设置保留");
    }

    /// `env` 只增改不删：孤儿键与界面清空的键都留着。
    #[test]
    fn env_is_never_pruned() {
        let base: Value = serde_json::from_str(
            r#"{ "env": { "BAILIAN_CODING_PLAN_API_KEY": "sk-plan", "OLD_KEY": "sk-old" } }"#,
        )
        .unwrap();
        let out = QwenCodeBackend.serialize_root(&[], &[], &base, None);
        assert_eq!(out["env"]["BAILIAN_CODING_PLAN_API_KEY"], "sk-plan");
        assert_eq!(out["env"]["OLD_KEY"], "sk-old", "无引用的键也不删");
    }

    /// 自定义 pid 沿用并刷新映射；卡片删掉后孤儿映射被清理。
    #[test]
    fn custom_pid_mapping_is_refreshed_and_orphans_pruned() {
        let base: Value = serde_json::from_str(
            r#"{
                "modelProviders": { "idealab": [ { "id": "m1" } ] },
                "providerProtocol": { "idealab": "openai", "stale": "anthropic" }
            }"#,
        )
        .unwrap();
        let env = None;
        let protocols = provider_protocols(&base).cloned();
        let load = build_load(entries_from(&base), env, protocols.as_ref());
        assert_eq!(load.providers.len(), 1);
        let out = QwenCodeBackend.serialize_root(&[], &load.providers, &base, None);
        assert_eq!(out["providerProtocol"]["idealab"], "openai");
        assert!(
            out["providerProtocol"].get("stale").is_none(),
            "孤儿映射应被清理"
        );
    }

    /// 内置 pid 不需要映射，切换协议后写进条目的 `wireApi` 跟着变。
    #[test]
    fn builtin_pid_needs_no_mapping_and_wire_follows_api() {
        let mut p = ProviderRow::new();
        p.key = "gpt-4o".into();
        p.base_url = "https://api.openai.com/v1".into();
        p.pi_api = "openai-responses".into();
        let out = QwenCodeBackend.serialize_root(
            &[],
            std::slice::from_ref(&p),
            &Value::Object(Map::new()),
            None,
        );
        assert_eq!(out["modelProviders"]["openai"][0]["wireApi"], "responses");
        assert!(out.get("providerProtocol").is_none(), "内置 pid 不写映射");
    }

    /// 非 OpenAI 系协议不得写 `wireApi`（官方明说是配置错误）。
    #[test]
    fn wire_api_is_absent_for_non_openai_protocols() {
        let mut p = ProviderRow::new();
        p.key = "claude".into();
        p.pi_api = "anthropic-messages".into();
        let out = QwenCodeBackend.serialize_root(
            &[],
            std::slice::from_ref(&p),
            &Value::Object(Map::new()),
            None,
        );
        assert_eq!(out["modelProviders"]["anthropic"][0]["id"], "claude");
        assert!(out["modelProviders"]["anthropic"][0]
            .get("wireApi")
            .is_none());
    }

    /// 写出的条目永远不带 `disabled`（schema 没这个键，也没有停用概念；
    /// `ModelRow.disabled` 是界面共享结构上的字段，与本后端无关）。
    #[test]
    fn written_entries_carry_no_disabled_key() {
        let mut p = ProviderRow::new();
        p.key = "m1".into();
        let mut m = ModelRow::new();
        m.id = "m1".into();
        m.disabled = true; // 共享结构上的残留值，保存时必须视而不见
        p.models.push(m);
        let out = QwenCodeBackend.serialize_root(
            &[],
            std::slice::from_ref(&p),
            &Value::Object(Map::new()),
            None,
        );
        let entry = &out["modelProviders"]["openai"][0];
        assert_eq!(entry["id"], "m1", "没有停用概念：全部条目都进主配置");
        assert!(entry.get("disabled").is_none());
    }

    /// 条目里界面没接管的键要继承下来。
    #[test]
    fn unmanaged_entry_keys_are_inherited() {
        let base: Value = serde_json::from_str(
            r#"{ "modelProviders": { "openai": [ {
                "id": "gpt-4o",
                "capabilities": { "agent": true },
                "generationConfig": { "maxRetries": 3, "samplingParams": { "temperature": 0.2 } }
            } ] } }"#,
        )
        .unwrap();
        let load = build_load(entries_from(&base), None, None);
        let out = QwenCodeBackend.serialize_root(&[], &load.providers, &base, None);
        let e = &out["modelProviders"]["openai"][0];
        assert_eq!(
            e["capabilities"]["agent"], true,
            "capabilities 其余子键保留"
        );
        assert_eq!(e["generationConfig"]["maxRetries"], 3);
        assert_eq!(
            e["generationConfig"]["samplingParams"]["temperature"], 0.2,
            "采样参数的其它键保留"
        );
    }

    /// 检测：认形状不认键名。
    #[test]
    fn detect_requires_the_array_shape() {
        assert!(QwenCodeBackend.detect(
            r#"{ "modelProviders": { "openai": [ { "id": "m" } ] } }"#,
            ""
        ));
        assert!(
            !QwenCodeBackend.detect(r#"{ "modelProviders": { "openai": "not-an-array" } }"#, "")
        );
        assert!(!QwenCodeBackend.detect(
            r#"{ "modelProviders": { "openai": [ { "name": "无 id" } ] } }"#,
            ""
        ));
        // 路径强命中：目录对、文件名对就算，空文件也认。
        assert!(QwenCodeBackend.detect("{}", r"C:\Users\me\.qwen\settings.json"));
    }
}
