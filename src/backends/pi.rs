//! pi 后端：`~/.pi/agent/models.json`（JSONC）。
//!
//! 结构：顶层 `providers`（map）+ 其他顶层字段（extras）原样保留。

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
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

    fn detect(&self, content: &str, _path: &str) -> bool {
        // pi 判定：含 providers 对象且无 provider 键
        parse_config_content(content)
            .map(|v| {
                v.get("providers").and_then(|x| x.as_object()).is_some()
                    && v.get("provider").is_none()
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
        // 跨格式目标：extras 取目标文件自身的顶层字段，仅重写 providers
        let base = match target_root {
            Some(target) => target,
            None => extras,
        };
        convert::to_pi_root(providers, base)
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        // 跨格式目标保存需要目标文件完整的 providers（保守合并用），
        // 不能只取顶层 extras，否则目标独有 provider 会被整体替换删掉。
        super::load_target_root_with(path, parse_config_content, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((include_bytes!("../../assets/agents/pi_32.bin"), 32, 32))
    }
}
