//! UI 偏好持久化：`%APPDATA%\.modelharbor\prefs.json`。
//!
//! 只存**界面选择**（密钥显隐、保存格式、主题、同步 WSL）—— 配置内容永远以用户自己的
//! agent 配置文件为真源，这里一个字段都不存：不存密钥、不存模型、不存路径内容。
//!
//! 文件不存在 / 读不出 / 解析失败都用默认值（界面偏好坏了不该影响工具可用性）。
//! 空字符串表示「没设置过」，由调用方回落到自己的默认值。

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 配置目录名（App 自己的目录，与任何 agent 配置目录无关）。
pub const DIR_NAME: &str = ".modelharbor";
const FILE_NAME: &str = "prefs.json";

/// 界面偏好。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    /// 是否显示全部 API Key 明文。
    pub show_api_keys: bool,
    /// 保存格式标识（`current` / `compact`；空 = 用 App 默认）。
    pub save_format: String,
    /// 主题标识（`dark` / `light` / `ocean` / `nord` / `rose`；空 = 用 App 默认）。
    pub theme: String,
    /// 是否同步写入 WSL 侧路径。
    pub sync_wsl: bool,
}

impl Prefs {
    /// 配置文件路径（`%APPDATA%\.modelharbor\prefs.json`）。
    pub fn path() -> PathBuf {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(DIR_NAME).join(FILE_NAME);
        }
        // 非 Windows 或环境异常：退回 exe 同级目录，仍然不写进任何 agent 配置。
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(FILE_NAME)))
            .unwrap_or_else(|| PathBuf::from(FILE_NAME))
    }

    /// 从默认路径加载（失败即默认值）。
    pub fn load() -> Prefs {
        Self::parse(&std::fs::read_to_string(Self::path()).unwrap_or_default())
    }

    /// 解析已有内容（供加载与单测使用）。
    pub fn parse(text: &str) -> Prefs {
        let Ok(root) = serde_json::from_str::<Value>(text) else {
            return Prefs::default();
        };
        let get_str = |key: &str| {
            root.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        Prefs {
            show_api_keys: root
                .get("show_api_keys")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            save_format: get_str("save_format"),
            theme: get_str("theme"),
            sync_wsl: root
                .get("sync_wsl")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// 序列化（固定字段顺序，便于人工核对 / diff）。
    pub fn to_json(&self) -> String {
        let mut root = Map::new();
        root.insert("version".to_string(), Value::Number(1.into()));
        root.insert("show_api_keys".to_string(), Value::Bool(self.show_api_keys));
        root.insert(
            "save_format".to_string(),
            Value::String(self.save_format.clone()),
        );
        root.insert("theme".to_string(), Value::String(self.theme.clone()));
        root.insert("sync_wsl".to_string(), Value::Bool(self.sync_wsl));
        serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".to_string())
    }

    /// 落盘到默认路径。
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    /// 落盘到指定路径（单测用）。
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|err| format!("创建目录失败（{}）：{}", dir.display(), err))?;
        }
        std::fs::write(path, self.to_json())
            .map_err(|err| format!("写入失败（{}）：{}", path.display(), err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_known_fields_and_ignores_junk() {
        let prefs = Prefs::parse(
            r#"{"version":1,"show_api_keys":true,"save_format":"current",
                "theme":"nord","sync_wsl":true,"unknown":123}"#,
        );
        assert!(prefs.show_api_keys);
        assert_eq!(prefs.save_format, "current");
        assert_eq!(prefs.theme, "nord");
        assert!(prefs.sync_wsl);
    }

    #[test]
    fn broken_or_empty_input_falls_back_to_defaults() {
        for text in ["", "not json", "{}", "[]", r#"{"theme":42}"#] {
            let prefs = Prefs::parse(text);
            assert_eq!(prefs, Prefs::default(), "{text}");
            assert!(!prefs.show_api_keys);
            assert_eq!(prefs.save_format, "", "空字符串 = 用 App 默认");
        }
    }

    #[test]
    fn json_round_trip() {
        let prefs = Prefs {
            show_api_keys: true,
            save_format: "compact".to_string(),
            theme: "rose".to_string(),
            sync_wsl: true,
        };
        assert_eq!(Prefs::parse(&prefs.to_json()), prefs);
    }

    #[test]
    fn save_and_reload_from_disk() {
        let path = std::env::temp_dir().join(format!(
            "{}-prefs-test-{}.json",
            DIR_NAME,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let prefs = Prefs {
            theme: "ocean".to_string(),
            sync_wsl: true,
            ..Default::default()
        };
        prefs.save_to(&path).expect("写入应成功");
        assert_eq!(
            Prefs::parse(&std::fs::read_to_string(&path).unwrap()),
            prefs
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn default_path_is_app_private() {
        let path = Prefs::path();
        assert_eq!(path.file_name().unwrap_or_default(), FILE_NAME);
        let text = path.to_string_lossy().to_lowercase();
        for forbidden in [".config", ".pi", ".omp", ".dsh", "opencode"] {
            assert!(!text.contains(forbidden), "不应写进 agent 配置：{text}");
        }
        #[cfg(windows)]
        assert!(
            text.contains(DIR_NAME) || text.contains("appdata"),
            "{text}"
        );
    }
}
