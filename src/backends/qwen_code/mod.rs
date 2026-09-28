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
use crate::util::{home_dir_string, parse_config_content, set_str, split_csv, wsl_home};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

mod conflict;
mod convert;
mod json_io;
mod merge;
pub(crate) use conflict::*;
pub(crate) use convert::*;
pub(crate) use json_io::*;
pub(crate) use merge::*;

pub struct QwenCodeBackend;

pub static BACKEND: QwenCodeBackend = QwenCodeBackend;

/// 内置 pid：协议由 pid 自身决定，**不需要** `providerProtocol` 映射。
pub(crate) const BUILTIN_PIDS: [&str; 5] =
    ["openai", "anthropic", "gemini", "vertex-ai", "qwen-oauth"];

/// 只读 pid：Qwen Code 自己的 OAuth 托管条目，写进去也会被忽略，改坏还会破坏登录态。
pub(crate) const READONLY_PID: &str = "qwen-oauth";

/// `wireApi` 的两个合法值（只对 OpenAI 系协议有效）。
pub(crate) const WIRE_CHAT: &str = "chat-completions";
pub(crate) const WIRE_RESPONSES: &str = "responses";

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

/// 这条条目归界面管吗（有可用的 `id`）？
///
/// 公开是因为 `app::strip_cross_format_containers` 也要用：跨格式保存时它只剔界面接管的
/// 条目，没有 `id` 的那种（界面认不出来）必须留在基底里，[`unmanaged_of`] 才能原样带过去。
pub fn is_ui_managed_entry(entry: &Value) -> bool {
    entry_id(entry).is_some()
}

/// 是否是那个只读的内置 pid（`qwen-oauth`）。
///
/// 公开是因为 `app::strip_cross_format_containers` 也要用：跨格式保存时界面接管
/// `modelProviders`，但 `qwen-oauth` 不归界面管，必须原样留着。
pub fn is_readonly_provider(pid: &str) -> bool {
    pid == READONLY_PID
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
            include_bytes!("../../../assets/agents/qwen-code_32.bin"),
            32,
            32,
        ))
    }
}

#[cfg(test)]
mod tests;
