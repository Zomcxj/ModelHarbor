use crate::backends;
use crate::prefs::ConfigPathPrefs;
use crate::util::{is_wsl_path, read_wsl_file};
use std::path::Path;

/// 配置格式标识。新增后端时在 `backends` 模块实现并注册，这里加变体。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConfigFormat {
    Opencode,
    Pi,
    OhMyPi,
    DeepSeekHarness,
}

impl ConfigFormat {
    pub fn label(&self) -> &str {
        match self {
            ConfigFormat::Opencode => "opencode",
            ConfigFormat::Pi => "pi",
            ConfigFormat::OhMyPi => "oh-my-pi",
            ConfigFormat::DeepSeekHarness => "deepseek-harness",
        }
    }
}

/// 各后端的解析后路径容器（含用户可覆盖的本地路径）。
pub struct ConfigPaths {
    pub opencode: String,
    pub pi: String,
    pub oh_my_pi: String,
    pub deepseek_harness: String,
}

impl Default for ConfigPaths {
    fn default() -> Self {
        Self {
            opencode: backends::backend(ConfigFormat::Opencode).default_local_path(),
            pi: backends::backend(ConfigFormat::Pi).default_local_path(),
            oh_my_pi: backends::backend(ConfigFormat::OhMyPi).default_local_path(),
            deepseek_harness: backends::backend(ConfigFormat::DeepSeekHarness).default_local_path(),
        }
    }
}

impl ConfigPaths {
    /// 本后端实际使用的本地路径。
    pub fn local_path(&self, format: ConfigFormat) -> String {
        match format {
            ConfigFormat::Opencode => self.opencode.clone(),
            ConfigFormat::Pi => self.pi.clone(),
            ConfigFormat::OhMyPi => self.oh_my_pi.clone(),
            ConfigFormat::DeepSeekHarness => self.deepseek_harness.clone(),
        }
    }

    /// 写入某一后端的本地路径（用户手动覆盖 / 恢复默认都走这里）。
    pub fn set_local_path(&mut self, format: ConfigFormat, path: &str) {
        match format {
            ConfigFormat::Opencode => self.opencode = path.to_string(),
            ConfigFormat::Pi => self.pi = path.to_string(),
            ConfigFormat::OhMyPi => self.oh_my_pi = path.to_string(),
            ConfigFormat::DeepSeekHarness => self.deepseek_harness = path.to_string(),
        }
    }

    /// 该后端「探测到的默认路径」（用于判断某一页是否被用户覆盖过）。
    pub fn default_local_path(format: ConfigFormat) -> String {
        backends::backend(format).default_local_path()
    }

    /// 用 prefs 里的覆盖项替换默认路径（空串 = 不覆盖，保持自动探测值）。
    pub fn apply_overrides(&mut self, overrides: &ConfigPathPrefs) {
        for backend in backends::BACKENDS {
            let path = overrides.get(backend.id());
            if !path.trim().is_empty() {
                self.set_local_path(backend.id(), path);
            }
        }
    }

    /// 启动探测：优先「用户覆盖过且文件存在」的页面，其次才是默认路径探测。
    pub fn detect_preferring_overrides(
        &self,
        overrides: &ConfigPathPrefs,
    ) -> Option<(ConfigFormat, String)> {
        for backend in backends::BACKENDS {
            let id = backend.id();
            if overrides.get(id).trim().is_empty() {
                continue;
            }
            let path = self.local_path(id);
            if Path::new(&path).exists() {
                return Some((id, path));
            }
        }
        Self::detect()
    }

    /// 启动探测：找到第一个本地存在的默认配置。
    pub fn detect() -> Option<(ConfigFormat, String)> {
        for b in backends::BACKENDS {
            let p = b.default_local_path();
            if Path::new(&p).exists() {
                return Some((b.id(), p));
            }
        }
        None
    }

    /// 根据文件内容判别配置格式（无扩展名上下文）。
    pub fn detect_from_content(content: &str) -> ConfigFormat {
        backends::detect_format(content, "")
    }

    /// 根据路径 + 内容判别配置格式（支持 WSL 路径）。
    /// 内容不可得（新建场景）时按扩展名推断：`.yml/.yaml` 归 oh-my-pi，其余回落 opencode。
    pub fn detect_for_path(path: &str) -> (ConfigFormat, String) {
        let content = if is_wsl_path(path) {
            read_wsl_file(path).ok()
        } else {
            std::fs::read_to_string(path).ok()
        };
        match content {
            Some(content) => (backends::detect_format(&content, path), path.to_string()),
            None => (backends::detect_format("", path), path.to_string()),
        }
    }

    /// 目标是否可用（本地或 WSL）。
    pub fn validate_target(&self, format: ConfigFormat) -> bool {
        backends::target_available(format, &self.local_path(format))
    }

    /// 本地优先：本地文件存在时写本地，否则回落 WSL，最后回退本地默认路径（新建场景）。
    pub fn target_path(&self, format: ConfigFormat) -> String {
        backends::target_path(format, &self.local_path(format))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::ConfigPathPrefs;

    fn temp_config(name: &str) -> String {
        let path = std::env::temp_dir().join(format!(
            "model-harbor-override-{}-{}.json",
            std::process::id(),
            name
        ));
        std::fs::write(&path, "{}").expect("写临时文件应成功");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn set_local_path_targets_the_right_backend() {
        let mut paths = ConfigPaths::default();
        paths.set_local_path(ConfigFormat::Pi, r"D:\conf\pi.json");
        assert_eq!(paths.local_path(ConfigFormat::Pi), r"D:\conf\pi.json");
        assert_eq!(
            paths.local_path(ConfigFormat::Opencode),
            ConfigPaths::default_local_path(ConfigFormat::Opencode),
            "其他页面不受影响"
        );
    }

    #[test]
    fn apply_overrides_skips_empty_and_keeps_defaults() {
        let mut paths = ConfigPaths::default();
        let mut overrides = ConfigPathPrefs::default();
        overrides.set(ConfigFormat::DeepSeekHarness, "  ");
        overrides.set(ConfigFormat::OhMyPi, r"D:\conf\models.yml");
        paths.apply_overrides(&overrides);
        assert_eq!(
            paths.local_path(ConfigFormat::OhMyPi),
            r"D:\conf\models.yml"
        );
        assert_eq!(
            paths.local_path(ConfigFormat::DeepSeekHarness),
            ConfigPaths::default_local_path(ConfigFormat::DeepSeekHarness),
            "空白 = 不覆盖"
        );
    }

    #[test]
    fn startup_prefers_existing_override() {
        let file = temp_config("prefer");
        let mut paths = ConfigPaths::default();
        let mut overrides = ConfigPathPrefs::default();
        // 只覆盖 pi，且文件确实存在 → 启动应当定位到 pi 而不是按默认顺序探测
        overrides.set(ConfigFormat::Pi, &file);
        paths.apply_overrides(&overrides);
        assert_eq!(
            paths.detect_preferring_overrides(&overrides),
            Some((ConfigFormat::Pi, file.clone()))
        );
        // 覆盖的路径不存在 → 不再被优先选中（回落默认探测，与机器上装了哪些 agent 无关）
        let mut missing = ConfigPathPrefs::default();
        missing.set(ConfigFormat::Pi, r"D:\not-exist\models.json");
        let mut fresh = ConfigPaths::default();
        fresh.apply_overrides(&missing);
        let found = fresh.detect_preferring_overrides(&missing);
        assert!(
            !matches!(&found, Some((_, path)) if path == r"D:\not-exist\models.json"),
            "不存在的覆盖路径不应被选中：{found:?}"
        );
        let _ = std::fs::remove_file(&file);
    }
}
