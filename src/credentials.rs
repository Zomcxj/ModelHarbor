//! DeepSeek Harness（DSH）凭据 sidecar 支持。
//!
//! DSH 将 provider 配置放在 `settings.yaml`，实际密钥放在同目录的
//! `.credentials.yaml`：`refs.{apiKeyEnv}`。本模块只更新指定 ref，保留其余
//! 版本、记录和未知字段。

use crate::model::ProviderRow;
use crate::util::{is_wsl_path, parse_yaml_content, read_config_content, to_yaml_string};
use serde_json::{Map, Value};

/// 根据模型配置路径得到同级凭据路径。
pub fn sidecar_path(config_path: &str) -> String {
    let separator = if is_wsl_path(config_path) || config_path.contains('/') {
        '/'
    } else {
        '\\'
    };
    match config_path.rsplit_once(separator) {
        Some((parent, _)) if !parent.is_empty() => {
            format!("{}{}{}", parent, separator, ".credentials.yaml")
        }
        _ => ".credentials.yaml".to_string(),
    }
}

/// 读取凭据文件的完整 root。
///
/// 文件不存在（新建场景）返回骨架；**文件存在但读不出 / 解析不了 / 根不是对象
/// 返回 Err**——静默回落骨架会让下一次保存把原文件整个换掉：refs、records、
/// 未知字段全部丢失且无备份。宁可让保存报错，让用户先修好或备份这个文件。
pub fn load_root(config_path: &str) -> Result<Value, String> {
    let path = sidecar_path(config_path);
    let content = read_config_content(&path).map_err(|e| format!("读取失败（{path}）: {e}"))?;
    if content.trim().is_empty() {
        return Ok(skeleton_root());
    }
    let root = parse_yaml_content(&content)
        .map_err(|e| format!("凭据文件不是合法 YAML（{path}）: {e}"))?;
    if !root.is_object() {
        return Err(format!("凭据文件根节点必须是对象（{path}）"));
    }
    Ok(root)
}

/// 全新凭据文件的骨架。
fn skeleton_root() -> Value {
    let mut root = Map::new();
    root.insert("version".into(), Value::Number(1.into()));
    root.insert("refs".into(), Value::Object(Map::new()));
    root.insert("records".into(), Value::Object(Map::new()));
    Value::Object(root)
}

/// 从凭据 root 解析 refs 中的字符串密钥。
pub fn secret_for(root: &Value, env_name: &str) -> String {
    root.get("refs")
        .and_then(Value::as_object)
        .and_then(|refs| refs.get(env_name))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// 将 provider 的实际密钥更新到同级凭据文件。
/// 空密钥删除对应 ref；其余 refs/records/未知字段完全保留。
pub fn save(config_path: &str, providers: &[ProviderRow]) -> Result<(), String> {
    if !providers.iter().any(|provider| {
        !effective_env_name(provider).is_empty() || !provider.original_api_key_env.trim().is_empty()
    }) {
        return Ok(());
    }
    let path = sidecar_path(config_path);
    let mut root = load_root(config_path)?;
    let Some(object) = root.as_object_mut() else {
        return Err("凭据文件根节点必须是对象".into());
    };
    let refs = object
        .entry("refs")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(refs) = refs.as_object_mut() else {
        return Err("凭据文件 refs 必须是对象".into());
    };

    let current_names: std::collections::HashSet<String> = providers
        .iter()
        .map(effective_env_name)
        .filter(|name| !name.is_empty())
        .collect();
    for provider in providers {
        let env_name = effective_env_name(provider);
        let old_name = provider.original_api_key_env.trim();
        // 重命名/清空引用时，只清理本次加载且已不再使用的旧 ref；
        // 不触碰凭据文件中与本配置无关的 refs。
        if !old_name.is_empty() && old_name != env_name && !current_names.contains(old_name) {
            refs.remove(old_name);
        }
        if env_name.is_empty() {
            continue;
        }
        let secret = effective_secret(provider);
        if !secret.is_empty() {
            refs.insert(env_name, Value::String(secret));
        } else if provider.original_api_key_secret != provider.api_key_secret {
            refs.remove(&env_name);
        }
    }

    let content = to_yaml_string(&Value::Object(object.clone()))?;
    crate::backends::write_config(&path, &content)
}

/// 根据 provider key 生成 DSH 默认凭据引用名。
/// 非 ASCII 字母数字统一转为下划线，避免生成不可移植的环境变量名。
pub fn default_env_name(provider_key: &str) -> String {
    let stem: String = provider_key
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let stem = stem.trim_matches('_');
    if stem.is_empty() {
        String::new()
    } else {
        format!("{}_API_KEY", stem)
    }
}

/// DSH 需要写入 settings.yaml 的引用名。加载其他 agent 或旧 DSH
/// 配置缺少 apiKeyEnv 时，按 provider key 自动生成稳定的默认值。
pub fn effective_env_name(provider: &ProviderRow) -> String {
    let explicit = provider.api_key_env.trim();
    if explicit.is_empty() {
        default_env_name(&provider.key)
    } else {
        explicit.to_string()
    }
}

/// DSH sidecar 中的实际密钥。优先使用 DSH 编辑框；从其他 agent
/// 转换且该框尚未编辑时，复用来源 provider 的 apiKey。
pub fn effective_secret(provider: &ProviderRow) -> String {
    if !provider.api_key_secret.is_empty() {
        provider.api_key_secret.clone()
    } else if provider.source_format != Some(crate::format::ConfigFormat::DeepSeekHarness) {
        provider.api_key.clone()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderRow;
    use serde_json::json;

    #[test]
    fn sidecar_is_same_directory() {
        assert_eq!(
            sidecar_path(r"C:\Users\cxj\.dsh\settings.yaml"),
            r"C:\Users\cxj\.dsh\.credentials.yaml"
        );
        assert_eq!(
            sidecar_path("/home/cxj/.dsh/settings.yaml"),
            "/home/cxj/.dsh/.credentials.yaml"
        );
    }

    #[test]
    fn default_env_name_is_derived_from_provider_key() {
        assert_eq!(default_env_name("openai_apizh"), "OPENAI_APIZH_API_KEY");
        assert_eq!(default_env_name("claude-kktoken"), "CLAUDE_KKTOKEN_API_KEY");
    }

    #[test]
    fn cross_format_credentials_are_projected_to_dsh() {
        let mut provider = ProviderRow::new();
        provider.key = "openai_apizh".into();
        provider.api_key = "test-only-secret".into();
        provider.source_format = Some(crate::format::ConfigFormat::Opencode);
        assert_eq!(effective_env_name(&provider), "OPENAI_APIZH_API_KEY");
        assert_eq!(effective_secret(&provider), "test-only-secret");
    }

    #[test]
    fn save_preserves_unrelated_credentials() {
        let root = json!({
            "version": 1,
            "refs": {"OLD": "old-secret", "DSH_KEY": "old"},
            "records": {"keep": true},
            "extra": {"keep": true}
        });
        let mut p = ProviderRow::new();
        p.key = "dsh".into();
        p.api_key_env = "DSH_KEY".into();
        p.api_key_secret = "new-secret".into();
        let mut refs = root["refs"].as_object().unwrap().clone();
        refs.insert("DSH_KEY".into(), Value::String(effective_secret(&p)));
        let mut out = root.as_object().unwrap().clone();
        out.insert("refs".into(), Value::Object(refs));
        assert_eq!(out["refs"]["OLD"], "old-secret");
        assert_eq!(out["refs"]["DSH_KEY"], "new-secret");
        assert_eq!(out["records"]["keep"], true);
        assert_eq!(out["extra"]["keep"], true);
    }
}
