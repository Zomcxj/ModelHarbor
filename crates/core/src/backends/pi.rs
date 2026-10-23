//! The pi backend: `~/.pi/agent/models.json` (JSONC).
//!
//! Structure: the top-level `providers` (map) + other top-level fields (extras) preserved verbatim.

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, OpencodeSchemaVersion, ProviderRow};
use crate::util::{home_dir_string, parse_config_content, wsl_home};
use serde_json::{Map, Value};

pub struct PiBackend;

pub static BACKEND: PiBackend = PiBackend;

fn default_local_path() -> String {
    format!("{}\\.pi\\agent\\models.json", home_dir_string())
}

impl Backend for PiBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::Pi
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.pi/agent/models.json", wsl_home()?))
    }

    fn detect(&self, content: &str, path: &str) -> bool {
        // opencode-family paths are never claimed by pi: newer opencode also writes top-level `providers`
        // without `provider`, overlapping pi's content fingerprints.
        if super::opencode::family_path_matches(path) {
            return false;
        }
        // A `providers` object present with no `provider` key; opencode v2's content fingerprints (top-level
        // plural agents/plugins, providers entries carrying package/settings) also hit the previous rule —
        // v2 fingerprints yield to opencode.
        parse_config_content(content)
            .map(|v| {
                v.get("providers").and_then(|x| x.as_object()).is_some()
                    && v.get("provider").is_none()
                    && super::opencode::detect_schema_version(&v) != OpencodeSchemaVersion::V2
            })
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let v = parse_config_content(content)?;
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
        // Cross-format target: extras take the target file's own top-level fields, rewriting only providers
        let base = match target_root {
            Some(target) => target,
            None => extras,
        };
        convert::to_pi_root(providers, base)
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        // Take the target file's complete providers for conservative merging, not just the top-level extras.
        super::load_target_root_with(path, parse_config_content, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((include_bytes!("../../assets/agents/pi_32.bin"), 32, 32))
    }
}
