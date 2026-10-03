//! KimiCode 后端：`~/.kimi-code/config.toml`（TOML）。
//!
//! 中间表示是 `serde_json::Value`，由 `toml` crate 做 `from_str::<Value>` /
//! `to_string(&Value)`。
//!
//! ## 结构：两张分开的表，靠 `provider` 字段 join
//!
//! ```toml
//! default_model = "sensenova/sensenova-6.8-flash-lite"   # 顶层标量
//! default_permission_mode = "auto"
//!
//! [providers."managed:kimi-code"]        # 键 = provider 名（含 `:` 必须加引号）
//! base_url = "https://api.kimi.com/coding/v1"
//! type = "kimi"                          # 必填
//! api_key = ""                           # 空串 = 未设置
//!
//! [providers."managed:kimi-code".oauth]  # /login 注入，只读保留
//! storage = "file"
//! key = "oauth/kimi-code"
//!
//! [models."kimi-code/k3"]                # 顶层独立表，键 = alias
//! provider = "managed:kimi-code"         # join key → [providers.*]，必填
//! model = "k3"                           # 上游 wire id，必填
//! max_context_size = 1048576             # 必填，≥1
//! capabilities = [ "thinking", "tool_use" ]
//! display_name = "K3"
//! support_efforts = [ "low", "high", "max" ]
//! default_effort = "high"
//!
//! [thinking]
//! enabled = true
//! ```
//!
//! 模型不在 provider 下，而在顶层全局表里，靠 `provider` 字段关联；[`parse`] 做
//! join，[`serialize_root`] 拆回两张表。
//!
//! ## 约束
//!
//! 1. 表键是 alias，`model` 是 wire id，两者可以不同。界面显示 wire id
//!    （[`ModelRow::id`]，它会被发给上游），alias 另存 [`ModelRow::kimi_alias`]，
//!    写回时作表键。
//! 2. `api_key` / `api_key_env` / `oauth` 三者互斥，同时写会让 Kimi Code 启动失败。
//!    判据是**非空字符串**：`api_key = ""` 视同未设置。见
//!    [`credential_conflict`]。
//! 3. `managed:*` provider 来自 OAuth 登录，只读，只能用 `/login`、`/logout` 管理；
//!    界面不给删除、不给改字段。
//! 4. `capabilities` 只增不减。见 [`merge_capabilities`]。
//!
//! 模型 schema 里没有 `disabled`/`enabled` 字段，模型表按别名一一索引，每条别名
//! 独立生效，既不去重也没有开关，也没有全量副本；删卡片即真删。
//!
//! 界面新增（或从别的格式复制来的）模型，缺省别名按 `<provider>/<model>` 生成。
//!
//! 保存时整块原样带过的两类内容：`managed:*` provider 与其模型（界面不显示），
//! 以及孤儿模型（`provider` 指向的键在 `[providers.*]` 里不存在）。其余条目一律
//! 以界面状态为准，包括用户把卡片删掉的情况。
//!
//! TOML 全量序列化会丢注释；Kimi Code 桌面端与本文件共享。

use super::{Backend, BackendLoad};
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{home_dir_string, set_str, split_csv, wsl_home};
use serde_json::{Map, Value};

mod conflict;
mod convert;
mod merge;
mod toml_io;
pub(crate) use conflict::*;
pub(crate) use convert::*;
pub(crate) use merge::*;
pub(crate) use toml_io::*;

pub struct KimiCodeBackend;

pub static BACKEND: KimiCodeBackend = KimiCodeBackend;

/// OAuth 登录写入的 provider 前缀。
pub(crate) const MANAGED_PREFIX: &str = "managed:";

fn default_local_path() -> String {
    format!("{}\\.kimi-code\\config.toml", home_dir_string())
}

/// 某个 provider 是否由 OAuth 登录维护（只读）。
///
/// 另供 `app::strip_cross_format_containers` 使用。
pub fn is_managed_provider(name: &str) -> bool {
    name.starts_with(MANAGED_PREFIX)
}

/// 解析 TOML 文本为 root；解析失败返回 `None`。
///
/// `util::parse_config_content` 是 JSONC 解析器，不适用于 TOML。
pub fn parse_root_for_shrink(content: &str) -> Option<Value> {
    parse_toml(content).ok()
}

/// 这次保存是否会让模型表变小。
///
/// 判据是模型表条目数变少；孤儿与 `managed:` 模型都被原样保留。
pub fn shrinks_on_save(before: &Value, after: &Value) -> bool {
    let count = |root: &Value| models_map(root).map(Map::len).unwrap_or(0);
    count(before) > count(after)
}

// ---------- TOML 读写 ----------

// ---------- 结构访问 ----------

// ---------- 解析：两张表 join ----------

// ---------- 序列化：拆回两张表 ----------

impl Backend for KimiCodeBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::KimiCode
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.kimi-code/config.toml", wsl_home()?))
    }

    fn detect(&self, content: &str, path: &str) -> bool {
        // 路径强命中：Kimi Code 的用户级配置就在这个位置。
        let normalized = path.replace('\\', "/");
        if normalized.contains("/.kimi-code/") && normalized.ends_with("config.toml") {
            return true;
        }
        let Ok(root) = parse_toml(content) else {
            return false;
        };
        // `model_providers` 是 Codex 的顶层键（带下划线），Kimi 是 `providers`。
        if root.get("model_providers").is_some() {
            return false;
        }
        // 极强特征：`managed:kimi-code` + `type = "kimi"`。
        if providers_map(&root).is_some_and(|m| {
            m.iter()
                .any(|(name, v)| name.starts_with(MANAGED_PREFIX) && provider_type(v) == "kimi")
        }) {
            return true;
        }
        // 一般特征：`[providers.` 与 `[models.` 两张表同时出现，且模型带
        // `max_context_size`（Kimi 特有且必填）。
        let has_providers = providers_map(&root).is_some_and(|m| !m.is_empty());
        let has_models = models_map(&root).is_some_and(|m| {
            m.values()
                .any(|v| v.get("provider").is_some() && v.get("max_context_size").is_some())
        });
        if has_providers && has_models {
            return true;
        }
        // 顶层独有标量：`default_permission_mode` 是 Kimi 的权限模式键。
        root.get("default_permission_mode").is_some()
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_toml(content)?;
        let mut load = build_load(join(&root));
        load.root = root.clone();
        load.extras = root;
        Ok(load)
    }

    // `parse_at` 用 trait 缺省实现（直接 `parse`）。

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // 产物是 Kimi Code 的全部生效状态，`config.toml` 承担所有条目。
        let base = target_root.unwrap_or(extras);
        let mut root = base.as_object().cloned().unwrap_or_default();
        set_or_drop_table(&mut root, "providers", all_providers(providers, base));
        let mut models = model_table(providers, base);
        // 认不出来的条目原样补回（孤儿模型 / managed 名下的模型）。
        merge_unmanaged_models(&mut models, base, providers);
        set_or_drop_table(&mut root, "models", models);
        Value::Object(root)
    }

    /// 没有全量副本可写，但仍挂在保存序列里，作为写盘前的总闸：
    /// - 凭据 XOR（见模块说明第 2 点）；
    /// - `managed:` 保留前缀与别名撞名（[`first_structural_conflict`]）。
    ///
    /// 失败时返回错误、取消保存。
    fn save_sidecars(&self, _path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        if let Some(conflict) = first_credential_conflict(providers) {
            return Err(format!("凭据冲突，已取消保存: {conflict}"));
        }
        if let Some(conflict) = first_structural_conflict(providers) {
            return Err(format!("配置冲突，已取消保存: {conflict}"));
        }
        Ok(())
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        super::load_target_root_with(path, parse_toml, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../../assets/agents/kimi-code_32.bin"),
            32,
            32,
        ))
    }

    fn render(&self, root: &Value, _compact: bool) -> Result<String, String> {
        // `compact` 参数对本后端无意义（忽略）：`toml` 的输出就是规范形态。
        to_toml_string(root)
    }
}

#[cfg(test)]
mod tests;
