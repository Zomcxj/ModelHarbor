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
//! **界面不给「自定义协议」开关**：它与协议选择表达同一件事，两个控件可以互相矛盾。
//! 协议选择非 `chat/completions` 就等价于自定义协议，保存时由这里落到
//! `useCustomProtocol = true` 并补上对应后缀。
//!
//! ## 两份配置：ModelHarbor 的全量副本 + WorkBuddy 的生效清单
//!
//! WorkBuddy 的选择器**按裸 id 全局去重**（同名模型无论挂在哪个厂商下都只列出一行、
//! 只认第一条），所以 `models.json` 里同 id 的其余条目写进去也不生效。但界面必须
//! 显示**全部**配置（用户要看得见每一条、才能决定勾哪一个），这两件事不可能由同一个
//! 文件承担，于是拆成两份：
//!
//! - `~/.workbuddy/models.json` —— WorkBuddy 真正读的生效清单：**只含勾选的条目**，
//!   且每个 id 只留第一条（即 [`serialize_root`] 的产物）。每条都显式写
//!   `disabled: false`：这份文件是用户自己的配置，勾选状态要在**它自己**里面看得见，
//!   而不是只能去副本里找。`false` 对 WorkBuddy 是无操作（`normalizeCustomModel`
//!   的基底就是 `disabled: false`），所以写它不改变 WorkBuddy 的任何行为。
//! - 同目录下的 `models.full.json` —— ModelHarbor 自己维护的**全量副本**：所有条目都在，
//!   每条**都带** `disabled` 标记记录勾选状态（见 [`save_sidecars`] / [`full_store_path`]）。
//!
//! 全量副本里 `disabled` 是**每条必写**的，勾选也要写 `false`。省掉 `false` 会让
//! 「用户把重复项全勾上了」与「这份副本从没记录过勾选」变成同一种状态（都缺这个键），
//! 加载时只能一律当启用——那正是「重复的模型名都启用了」的成因，而且会自我延续：
//! 全启用读进来、原样写回去，永远生不出勾选记录。为兼容已经写坏的旧副本，加载时
//! 逐条判断：没写 `disabled` 的条目回退到按位置推导（每个模型名只勾第一条），
//! 保存一次即落成显式标记（见 [`build_load`]）。
//!
//! 但**光靠信任显式标记还不够**：用户可能把重复项真的全勾过（那份副本于是全是
//! `false`），也可能读到的文件压根没有勾选记录。所以 [`build_load`] 末尾还有一道
//! **按 id 去重兜底**：同一 id 至多留一条启用、其余关掉。WorkBuddy 既然按裸 id
//! 全局去重，多开的条目本就不生效，界面就不该显示成全部启用。
//!
//! 去重的**优先级是「谁更有发言权」，不是「谁是文件里的第一条」**：显式标记为启用的
//! 条目胜过按位置推出来的条目，只有多条同属一种来源时才取文件里最靠前的。用户明确
//! 设置过启用哪一条、且该 id 不重复，就不该再按读取顺序重新推导——位置推导只是旧文件
//! 的兜底，与用户的意图相遇时必须让位。
//!
//! 这道去重在**加载时**做（而不是只在写 WorkBuddy 的生效清单时做），因为界面显示的
//! 状态本身就得是真实的。
//!
//! 加载时优先读全量副本，没有才退回 `models.json`。这样「取消勾选」不会让条目从界面上
//! 消失、更不会丢字段：它只是从生效清单里移出，本体仍在全量副本里，随时能勾回来。
//!
//! 放同目录而不是 `.modelharbor` 是有意的：这份副本含 API key 与完整模型配置，
//! 属于「配置内容」，按本项目一贯的边界应留在 agent 自己的配置目录里，而不是塞进
//! 只放界面偏好的工具设置目录。WorkBuddy 只按**精确文件名**读 `models.json`
//! （`join(dataFolder, "models.json")`），同目录的其他文件它一概不看——`models.json.bak`
//! 一直躺在那里也没被它读走，就是现成的证据。
//!
//! 渲染**不能**复用 `app::compact_json` / `pretty_json`——那两个函数写死了
//! `root.as_object()`，数组根经过它们会静默变成 `{}`，直接毁掉用户配置。

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{is_wsl_path, parse_config_content, read_config_content, wsl_home, WslPathProbe};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct WorkBuddyBackend;

pub static BACKEND: WorkBuddyBackend = WorkBuddyBackend;

/// 非 Chat 协议对应的 URL 后缀（保存时补上，与 ZCode 的后缀规则同源）。
const MESSAGES_SUFFIX: &str = "/v1/messages";
const RESPONSES_SUFFIX: &str = "/v1/responses";

/// 全量副本的文件名（与 `models.json` 同目录）。
///
/// 名字必须与 `models.json` 不同：WorkBuddy 按精确文件名读后者，
/// 同目录的其他文件它不读（`.bak` 一直是旁证）。
const FULL_STORE_NAME: &str = "models.full.json";

fn default_local_path() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    format!("{}\\.workbuddy\\models.json", home)
}

/// 全量副本的路径：与主配置同目录、固定文件名。
///
/// 分隔符按主配置路径的形态选（WSL 路径用 `/`），与 `credentials::sidecar_path` 同一套规则。
pub fn full_store_path(config_path: &str) -> String {
    let separator = if is_wsl_path(config_path) || config_path.contains('/') {
        '/'
    } else {
        '\\'
    };
    match config_path.rsplit_once(separator) {
        Some((parent, _)) if !parent.is_empty() => {
            format!("{}{}{}", parent, separator, FULL_STORE_NAME)
        }
        _ => FULL_STORE_NAME.to_string(),
    }
}

/// 读全量副本的条目；不存在 / 不是数组 / 解析失败都返回 `None`（调用方退回主配置）。
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
/// 以全量副本为主（它带每条自己的勾选状态），再把**主配置里有、副本里没有**的条目
/// 补进来（按已勾选处理——它出现在生效清单里，就说明它是启用的）。
///
/// 这一步是为了「用户手改了 `models.json`」这种情况：副本是 ModelHarbor 上次保存的
/// 快照，手加进去的条目不在里面；不补的话那条模型在界面上根本看不见，
/// 而用户刚亲手加过它。WorkBuddy 自己并不写这个文件（它是用户手编的配置），
/// 所以主配置里出现副本没有的条目是正常情况，不是异常。
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
        // 生效清单里的条目一律是启用的；这里不写 `disabled: false`，
        // 落盘时由 `all_entries` 决定这个键的取舍。
        out.push(entry.clone());
    }
    out
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

    // WorkBuddy 约定（用户实测确认）：`id` = 模型名（发给 API 的模型，如
    // `claude-opus-5`），`name` = 提供商标签（如 `ps.air-outer`）。因此：
    //   模型行的 id ← 条目 `id`（模型）；provider 的 key ← 条目 `name`（提供商）。
    // 每个条目自带独立 url / apiKey，所以仍是「一条目一 provider 卡片」。
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
    // WorkBuddy 的 `disabled: true`：选择器里变灰、不可选，但**仍在列表里**。
    // 这是应对「全局按裸 id 去重」的手段——同 id 只能生效一次，保留多条时停用其余。
    model.disabled = v.get("disabled").and_then(Value::as_bool).unwrap_or(false);
    // WorkBuddy 用布尔 supportsReasoning 表达推理，没有「思考档位」概念。
    // ModelRow::new() 会带一组默认档位（medium/high/xhigh/max），若不清空，
    // 跨格式存到 ZCode 时会给每个模型凭空塞 reasoningLevel.values——这些档位
    // 是发明出来的，会污染 ZCode 配置（就是「保存后 ZCode 里不对」的根因）。
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

/// ProviderRow + 单个模型 → 条目里**由界面接管**的字段。
///
/// 只返回本函数管理的键：条目里其余的键（tags / credits / reasoning 等）由
/// [`WorkBuddyBackend::serialize_root`] 从「同名同模型」的旧条目继承。
/// 这里绝不能用 provider 的 raw 当基底——那是该 provider **第一条**条目的内容，
/// 一家提供商有多个模型时会把它第一个模型的 tags / credits 串到兄弟模型上。
///
/// WorkBuddy 的 `id` 是**发给 API 的模型名**，`name` 是提供商标签，
/// 两者都不能为了「避重名」而拼在一起——拼了就是把非法模型名发给服务端。
/// 同 provider 的多条条目靠 `name` 相同、`id` 不同来区分，这正是用户自己文件里的形态。
fn entry_from_provider(p: &ProviderRow, model: Option<&ModelRow>) -> Value {
    let mut obj = Map::new();

    // WorkBuddy `id` = 模型名（模型行的 id），`name` = 提供商（= ProviderRow.key）。
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
        // 只写能解析成整数的值；解析不了就不写这个键（`obj` 是新构造的，不写即缺席）。
        // 口径与后果同 zcode 的 contextWindow：写成 `unwrap_or(0)` 会把上限归零落盘。
        if let Ok(context) = m.context.trim().parse::<i64>() {
            obj.insert("maxInputTokens".into(), Value::Number(context.into()));
        }
        if let Ok(output) = m.output.trim().parse::<i64>() {
            obj.insert("maxOutputTokens".into(), Value::Number(output.into()));
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
        // 启用/停用**不在这里写**：生效清单（`serialize_root`）里全是启用的条目，
        // 勾选状态由全量副本 `models.full.json` 逐条显式记录（见 `all_entries`）。
    }

    Value::Object(obj)
}

/// 按 **(name, id) 二元组**索引旧条目。
///
/// WorkBuddy 按模型扁平存储，不同 provider 可以有同名模型（用户文件里 `gpt-5.6-sol`
/// 就有 4 条），只按 `id` 建索引会让后写的 provider 继承到别家条目的 tags / credits。
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

/// 一家提供商的**每个模型各写一条**条目，**不过滤、不去重**。
///
/// 这是两份配置共用的构造步骤：全量副本要全部条目，生效清单再从结果里筛。
///
/// 勾选状态**每条都显式写**：勾选写 `disabled: false`、未勾选写 `true`。这个键是
/// ModelHarbor 的勾选簿记，不是留给 WorkBuddy 推断的能力位；为什么 `false` 也必须
/// 写、省掉会有什么歧义，见本文件模块说明的「两份配置」一节。
fn all_entries(providers: &[ProviderRow], base: &Value) -> Vec<Value> {
    let existing = existing_index(base);
    let mut out: Vec<Value> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        // 没有模型的 provider 也要留一条（id 回落到 provider key），
        // 否则刚建好还没填模型的卡片一保存就消失。
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
            // 勾选 = false、未勾选 = true；为什么必须显式写见模块说明。
            entry.insert(
                "disabled".into(),
                Value::Bool(model.map(|m| m.disabled).unwrap_or(false)),
            );
            out.push(Value::Object(entry));
        }
    }
    out
}

/// 生效清单：从全部条目里取出**勾选的**，且每个 id 只留第一条。
///
/// WorkBuddy 的选择器按裸 id 全局去重（`appendModel` 里 `if (ids.has(model.id)) return`），
/// 第二条起写进去也不会被采用，所以生效清单里不能有不生效的条目——留着只会让人以为配了。
/// 被筛掉的条目**不会丢**：它们仍在全量副本里，界面上随时能勾回来。
///
/// 留下的每条都显式写 `disabled: false`：勾选状态要在生效清单自己里面看得见
/// （对 WorkBuddy 是无操作，见模块说明）。
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
        // 生效条目一律启用；显式写 `false` 让文件自解释（见模块说明）。
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("disabled".into(), Value::Bool(false));
        }
        out.push(entry);
    }
    out
}

/// 条目列表 → [`BackendLoad`]（两份配置共用）。
///
/// WorkBuddy 文件按模型扁平存储：一家提供商的多个模型就是多条 `name` 相同的条目
/// （用户的文件里 `gpt-5.6-sol` 就有 4 条、`claude-opus-5` 5 条，靠 `name` 区分）。
/// 界面按 provider 分组，所以同 `name` 的条目合并成一张卡片、各自成为一个模型行，
/// 保存时再一条条目一个模型写回去，来回不丢模型。
///
/// `trusted_flags` 表示「这批条目的 `disabled` 键可以信任」（读的是全量副本时为 `true`）。
/// 但**信任是逐条的**：只有条目**确实写了** `disabled` 键时才采信，没写的仍然按位置推导。
/// 两者的差别很要紧：
/// - 写了（用户在全量副本里亲手勾过）：原样还原，不能再按位置重新推导——否则用户勾了
///   第二条、界面却把第一条显示成勾选，下一次保存就把他勾的那条从生效清单里挤掉了。
/// - 没写：按**位置**推导。WorkBuddy 的选择器按裸 id 全局去重，同一个模型名只有第一条
///   生效，后面同名的都不生效，界面就该如实显示这个事实。
///
/// 之所以要「逐条」而不是「整份」，是为了让**没有勾选记录的旧副本自愈**：拆分方案刚上线时
/// 写出的 `models.full.json` 里一条 `disabled` 都没有，若整份信任就等于把 35 条全当成启用，
/// 用户看到的正是「重复的模型名都启用了」；而这种状态还会自我延续——全启用读进来、
/// 原样写回去，永远不会有勾选记录。逐条回退到按位置推导，首次打开即得到
/// 「每个模型名只勾第一条」，下一次保存就把这份推导落成显式标记，之后不再推导。
///
/// 第二遍循环还有一道**去重兜底**：判定取「谁更有发言权」（显式标记为启用的条目
/// 胜过按位置推出来的），且在**加载时**就做——界面显示的状态必须真实。
/// 完整论证见模块说明的「两份配置」一节。
fn build_load(entries: Vec<Value>, trusted_flags: bool) -> BackendLoad {
    // 第一遍：按**文件顺序**逐条定勾选状态，并记下这条状态是「显式标记」还是「位置推导」。
    //
    // 顺序很关键，所以就地判定，而不是等分组之后再遍历——分组会把同厂商的条目聚到
    // 一起，A,a / B,b / A,c 这种顺序压平后 a、c 会先于 b，「第一条生效」就不再是
    // 文件里的第一条了。
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
            // 没有 id 的条目进不了界面（`provider_from_entry` 会跳过），
            // 占位 None 只为让 flags 与 entries 同序。
            flags.push(None);
            continue;
        };
        let by_position = !seen.insert(id);
        // 「这条条目自己写了 disabled 吗」直接问条目本身，而不是另开一个与条目同序的
        // 数组：分组会把同厂商的条目聚到一起，按下标对齐迟早错位。
        let explicit = trusted_flags && entry.get("disabled").and_then(Value::as_bool).is_some();
        let disabled = match (explicit, entry.get("disabled").and_then(Value::as_bool)) {
            (true, Some(flag)) => flag,
            _ => by_position,
        };
        flags.push(Some((disabled, explicit)));
    }

    // 第二遍：同一 id 若有多条被判成启用，只留一条。
    //
    // 优先级：显式标记为启用的 > 位置推导出来的。两者都有多条时才取文件里最靠前的
    // 那条。见函数文档末段。
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
            // 已有胜出者：只有「本条是显式、上一条不是」时才改判。
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
            // 同 provider 的 url / apiKey / vendor 本就相同，以首条为准；
            // 后续条目只贡献模型行（各模型自己的 raw 保留在该模型行里）。
            Some(existing) => existing.models.push(row.models.remove(0)),
            None => providers.push(row),
        }
    }
    // extras 用原始条目数组：保存时按 (name, id) 继承未知字段（tags / credits）取的就是它。
    let extras = Value::Array(entries);
    BackendLoad {
        root: extras.clone(),
        agents: Vec::new(),
        providers,
        extras,
    }
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
        let entries = entries_of(&root).cloned().unwrap_or_default();
        Ok(build_load(entries, false))
    }

    /// 带路径的解析：优先读 ModelHarbor 的全量副本（`models.full.json`）。
    ///
    /// 全量副本才是界面上那份「全部配置」的真源——它含所有条目（含未勾选的）以及
    /// 每条自己的勾选状态。只读 `models.json` 会看到「勾选的那几条」，取消勾选的条目
    /// 在界面上凭空消失，用户既看不到也勾不回来。
    ///
    /// 副本不存在（首次使用、用户自己删了、从别处拷来的配置）时退回 `models.json`，
    /// 此时按**位置**推导勾选状态（见 [`build_load`]），行为与拆分之前一致。
    ///
    /// 副本存在时仍要读一遍主配置并**并集**进去：用户可能手改了 `models.json`
    /// （WorkBuddy 自己不写它），手加的条目不在副本里，不补就看不见。
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
        // 这个函数的产物是 **WorkBuddy 的生效清单**：只含勾选的条目、每个 id 只留第一条。
        // 全量副本（含所有条目与勾选标记）由 `save_sidecars` 另写一份，两者共用
        // `all_entries` 构造，因此同一次保存里字段口径完全一致。
        //
        // **不要给重复 id 加序号来绕开选择器去重。** 曾经这么做过，结论是错的：
        // WorkBuddy 的 `id` 既是选择器的去重键，**也是发给上游的模型名**
        // （`configureModelConfig` 把 `ec.id` 赋给 `agent.model`，`ModelProvider.getModel`
        // 再把这个字符串原样交给请求体；唯一改动是发送前 `stripCustomLocalModelPrefix`
        // 去掉 `custom-local:` 前缀）。加序号的 `id` 会让请求体变成
        // `gpt-5.6-sol01`，上游直接 model-not-found。
        Value::Array(effective_entries(&all_entries(
            providers,
            target_root.unwrap_or(extras),
        )))
    }

    /// 全量副本（`models.full.json`）：所有条目 + 每条自己的勾选状态。
    ///
    /// 这是界面上那份「全部配置」的落盘形态。有了它，取消勾选只是把条目从生效清单
    /// 移到副本里，条目本身和它的字段（含 API key、tags、credits）都还在，
    /// 勾回来即可恢复——不会再出现「取消勾选 = 永久删除」。
    fn save_sidecars(&self, path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        // 继承未知字段的基底取**全量副本**，没有才退回主配置。
        //
        // 不能只用主配置当基底：`models.json` 里只有勾选的条目，拿它当基底会让未勾选
        // 条目的未知字段（tags / credits / 用户自己加的键）在每次保存时被抹掉——
        // 那正是这份副本要解决的问题。副本是「上次的完整状态」，字段最全。
        //
        // 用副本当基底**不会让已删除的条目复活**：条目是从 `providers`（界面状态）
        // 生成的，删掉模型行就不再生成，基底里有没有它都一样。
        // 副本读不出（缺失或损坏）才退回主配置；主配置也读不出就取消保存——
        // 静默用空基底会让未勾选条目的 tags / credits / 用户自己加的键全部丢失。
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
        // WorkBuddy 的顶层是数组（provider 内联在每条模型里），空 root 也要是数组。
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
        // 数组根必须自己渲染：app::pretty_json 假定对象根，会把它变成 `{}`。
        let items = entries_of(root).cloned().unwrap_or_default();
        Ok(render_array(&items, compact))
    }
}
