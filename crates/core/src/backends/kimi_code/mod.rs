//! The KimiCode backend: `~/.kimi-code/config.toml` (TOML).
//!
//! The intermediate representation is `serde_json::Value`, with the `toml` crate doing
//! `from_str::<Value>` / `to_string(&Value)`.
//!
//! ## Structure: two separate tables joined by the `provider` field
//!
//! ```toml
//! default_model = "sensenova/sensenova-6.8-flash-lite"   # top-level scalar
//! default_permission_mode = "auto"
//!
//! [providers."managed:kimi-code"]        # key = provider name (with `:`, must be quoted)
//! base_url = "https://api.kimi.com/coding/v1"
//! type = "kimi"                          # required
//! api_key = ""                           # empty string = unset
//!
//! [providers."managed:kimi-code".oauth]  # injected by /login, kept read-only
//! storage = "file"
//! key = "oauth/kimi-code"
//!
//! [models."kimi-code/k3"]                # standalone top-level table, key = alias
//! provider = "managed:kimi-code"         # join key → [providers.*], required
//! model = "k3"                           # upstream wire id, required
//! max_context_size = 1048576             # required, ≥1
//! capabilities = [ "thinking", "tool_use" ]
//! display_name = "K3"
//! support_efforts = [ "low", "high", "max" ]
//! default_effort = "high"
//!
//! [thinking]
//! enabled = true
//! ```
//!
//! Models don't nest under providers — they sit in a top-level global table joined by the `provider`
//! field; [`parse`] performs the join and [`serialize_root`] splits back into the two tables.
//!
//! ## Constraints
//!
//! 1. The table key is the alias; `model` is the wire id; the two may differ. The UI displays the wire id
//!    ([`ModelRow::id`] — it's what gets sent upstream); the alias is stored separately in
//!    [`ModelRow::kimi_alias`] and serves as the table key at write-back.
//! 2. `api_key` / `api_key_env` / `oauth` are mutually exclusive; writing two makes Kimi Code fail to start.
//!    The criterion is a **non-empty string**: `api_key = ""` counts as unset. See [`credential_conflict`].
//! 3. `managed:*` providers come from OAuth login and are read-only — manageable only via `/login` and
//!    `/logout`; the UI offers neither deletion nor field edits.
//! 4. `capabilities` only grows, never shrinks. See [`merge_capabilities`].
//!
//! The model schema has no `disabled`/`enabled` field; the model table indexes one-to-one by alias, every
//! alias independently effective — no dedup, no toggles, no full copy; deleting a card really deletes.
//!
//! Models added by the UI (or copied in from other formats) get a default alias of `<provider>/<model>`.
//!
//! Two kinds of content pass through untouched at save: `managed:*` providers and their models (not shown
//! in the UI), and orphan models (whose `provider` key doesn't exist in `[providers.*]`). Everything else
//! follows the UI state, including cards the user deleted.
//!
//! Full TOML serialization drops comments; the Kimi Code desktop app shares this file.

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

/// The provider prefix written by OAuth login.
pub(crate) const MANAGED_PREFIX: &str = "managed:";

fn default_local_path() -> String {
    format!("{}\\.kimi-code\\config.toml", home_dir_string())
}

/// Whether a provider is OAuth-login-managed (read-only).
///
/// Also used by `app::strip_cross_format_containers`.
pub fn is_managed_provider(name: &str) -> bool {
    name.starts_with(MANAGED_PREFIX)
}

/// Parse TOML text into a root; `None` on parse failure.
///
/// `util::parse_config_content` is a JSONC parser — unsuitable for TOML.
pub fn parse_root_for_shrink(content: &str) -> Option<Value> {
    parse_toml(content).ok()
}

/// Whether this save would shrink the model table.
///
/// Criterion: the model table's entry count decreasing; orphans and `managed:` models are preserved as-is.
pub fn shrinks_on_save(before: &Value, after: &Value) -> bool {
    let count = |root: &Value| models_map(root).map(Map::len).unwrap_or(0);
    count(before) > count(after)
}

// ---------- TOML IO ----------

// ---------- Structure access ----------

// ---------- Parsing: joining the two tables ----------

// ---------- Serialization: splitting back into two tables ----------

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
        // Strong path hit: Kimi Code's user-level config lives exactly here.
        let normalized = path.replace('\\', "/");
        if normalized.contains("/.kimi-code/") && normalized.ends_with("config.toml") {
            return true;
        }
        let Ok(root) = parse_toml(content) else {
            return false;
        };
        // `model_providers` is Codex's top-level key (underscored); Kimi uses `providers`.
        if root.get("model_providers").is_some() {
            return false;
        }
        // Very strong fingerprint: `managed:kimi-code` + `type = "kimi"`.
        if providers_map(&root).is_some_and(|m| {
            m.iter()
                .any(|(name, v)| name.starts_with(MANAGED_PREFIX) && provider_type(v) == "kimi")
        }) {
            return true;
        }
        // General fingerprint: the `[providers.` and `[models.` tables together, with models carrying
        // `max_context_size` (Kimi-specific and required).
        let has_providers = providers_map(&root).is_some_and(|m| !m.is_empty());
        let has_models = models_map(&root).is_some_and(|m| {
            m.values()
                .any(|v| v.get("provider").is_some() && v.get("max_context_size").is_some())
        });
        if has_providers && has_models {
            return true;
        }
        // Top-level unique scalar: `default_permission_mode` is Kimi's permission-mode key.
        root.get("default_permission_mode").is_some()
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_toml(content)?;
        let mut load = build_load(join(&root));
        load.root = root.clone();
        load.extras = root;
        Ok(load)
    }

    // `parse_at` uses the trait's default implementation (straight `parse`).

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // The product is Kimi Code's entire effective state; `config.toml` carries all entries.
        let base = target_root.unwrap_or(extras);
        let mut root = base.as_object().cloned().unwrap_or_default();
        set_or_drop_table(&mut root, "providers", all_providers(providers, base));
        let mut models = model_table(providers, base);
        // Unrecognized entries are added back as-is (orphan models / models under managed names).
        merge_unmanaged_models(&mut models, base, providers);
        set_or_drop_table(&mut root, "models", models);
        Value::Object(root)
    }

    /// No full copy to write, yet it stays in the save sequence as the pre-write master gate:
    /// - the credential XOR (see module note 2);
    /// - `managed:` reserved-prefix vs alias collisions ([`first_structural_conflict`]).
    ///
    /// Returns an error and cancels the save on failure.
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
        // The `compact` parameter is meaningless for this backend (ignored): the `toml` output is already canonical.
        to_toml_string(root)
    }
}

#[cfg(test)]
mod tests;
