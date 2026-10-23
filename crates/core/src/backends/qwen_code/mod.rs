//! The QwenCode backend: `~/.qwen/settings.json` (JSONC).
//!
//! Each array element of `modelProviders[<pid>]` is one card, named by the element's `id`, with its owning
//! pid recorded in [`ProviderRow::qwen_pid`]; keys live in the top-level `env[<envKey>]`.
//! Custom pids need a `providerProtocol` mapping; built-in pids don't; entries under `qwen-oauth` are
//! read-only. `$version` is written only into brand-new files. Entries the UI doesn't recognize are
//! preserved verbatim — see [`unmanaged_of`].

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

/// Built-in pids: the protocol follows the pid itself — no `providerProtocol` mapping needed.
pub(crate) const BUILTIN_PIDS: [&str; 5] =
    ["openai", "anthropic", "gemini", "vertex-ai", "qwen-oauth"];

/// The read-only pid: Qwen Code's own OAuth-managed entries; writes are ignored.
pub(crate) const READONLY_PID: &str = "qwen-oauth";

/// `wireApi`'s two legal values (valid only for OpenAI-family protocols).
pub(crate) const WIRE_CHAT: &str = "chat-completions";
pub(crate) const WIRE_RESPONSES: &str = "responses";

fn default_local_path() -> String {
    format!("{}\\.qwen\\settings.json", home_dir_string())
}

/// Whether this save would shrink the entry count (the save flow uses it to decide on backing up first).
///
/// An unreadable original counts as "may shrink".
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

/// Is this entry UI-managed (has a usable `id`)?
///
/// Entries without an `id` stay in the base, carried over verbatim by [`unmanaged_of`].
pub fn is_ui_managed_entry(entry: &Value) -> bool {
    entry_id(entry).is_some()
}

/// Whether it's that read-only built-in pid (`qwen-oauth`).
///
/// On cross-format saves the UI takes over `modelProviders`, leaving `qwen-oauth` untouched.
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
        // Strong path hit: user-level settings live at exactly this path and file name.
        let normalized = path.replace('\\', "/");
        if normalized.contains("/.qwen/") && normalized.ends_with("settings.json") {
            return true;
        }
        // Content verdict: `modelProviders` is an object with at least one non-empty array value whose elements are objects carrying `id`.
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

    // `parse_at` uses the trait's default implementation (straight `parse`): the main config is the whole state.

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // The product is Qwen Code's entire effective state: one `settings.json` carries every entry.
        let base = target_root.unwrap_or(extras);
        let placed = place(providers, base);
        let mut root = base.as_object().cloned().unwrap_or_default();
        // `$version` only tags brand-new files (root empty then); existing files never touch this key.
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

    /// No full copy to write, yet it hangs in the save sequence: [`first_env_name_conflict`] blocks envKey
    /// collisions before the main config lands.
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
