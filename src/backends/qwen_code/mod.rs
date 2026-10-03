//! QwenCode 后端：`~/.qwen/settings.json`（JSONC）。
//!
//! `modelProviders[<pid>]` 的每个数组元素对应一张卡片，卡片名取元素的 `id`，
//! 所属 pid 记在 [`ProviderRow::qwen_pid`]；密钥在顶层 `env[<envKey>]`。
//! 自定义 pid 需要 `providerProtocol` 映射，内置 pid 不需要；`qwen-oauth` 下的条目只读。
//! `$version` 只在全新文件里写。界面认不出的条目原样保留，见 [`unmanaged_of`]。

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

/// 内置 pid：协议由 pid 自身决定，不需要 `providerProtocol` 映射。
pub(crate) const BUILTIN_PIDS: [&str; 5] =
    ["openai", "anthropic", "gemini", "vertex-ai", "qwen-oauth"];

/// 只读 pid：Qwen Code 自己的 OAuth 托管条目，写入被忽略。
pub(crate) const READONLY_PID: &str = "qwen-oauth";

/// `wireApi` 的两个合法值（只对 OpenAI 系协议有效）。
pub(crate) const WIRE_CHAT: &str = "chat-completions";
pub(crate) const WIRE_RESPONSES: &str = "responses";

fn default_local_path() -> String {
    format!("{}\\.qwen\\settings.json", home_dir_string())
}

/// 本次保存是否会让条目数变少（供保存流程决定是否先备份原文件）。
///
/// 原文件读不出时按「会收缩」处理。
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
/// 无 `id` 的条目留在基底里，由 [`unmanaged_of`] 原样带过去。
pub fn is_ui_managed_entry(entry: &Value) -> bool {
    entry_id(entry).is_some()
}

/// 是否是那个只读的内置 pid（`qwen-oauth`）。
///
/// 跨格式保存时 `modelProviders` 归界面接管，`qwen-oauth` 原样留着。
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
        // 路径强命中：用户级设置固定在这个路径与文件名。
        let normalized = path.replace('\\', "/");
        if normalized.contains("/.qwen/") && normalized.ends_with("settings.json") {
            return true;
        }
        // 内容判定：`modelProviders` 是对象，且至少一个值是非空数组、元素是含 `id` 的对象。
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

    // `parse_at` 用 trait 缺省实现（直接 `parse`）：主配置就是全部状态。

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // 产物就是 Qwen Code 的全部生效状态：`settings.json` 一份文件承担所有条目。
        let base = target_root.unwrap_or(extras);
        let placed = place(providers, base);
        let mut root = base.as_object().cloned().unwrap_or_default();
        // `$version` 只给全新文件打标（此时 root 是空的），已有文件不动这个键。
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

    /// 没有全量副本要写，但挂在保存序列里：[`first_env_name_conflict`] 在主配置落盘前挡住
    /// envKey 撞名。
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
