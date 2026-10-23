use crate::backends;
use crate::prefs::ConfigPathPrefs;
use crate::util::{is_wsl_path, read_wsl_file};
use std::path::Path;

/// Config format identifier. When adding a backend, implement and register it in the `backends` module, then add a variant here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConfigFormat {
    Opencode,
    Kilocode,
    Mimocode,
    Pi,
    OhMyPi,
    DeepSeekHarness,
    ZCode,
    WorkBuddy,
    QwenCode,
    KimiCode,
}

impl ConfigFormat {
    /// Display name, also the ledger key's prefix (`client:session_id`).
    ///
    /// Returns `&'static str`: every value is a literal, so relaxing the lifetime lets callers store it in
    /// structures like `DeleteError` that don't borrow self.
    pub fn label(&self) -> &'static str {
        match self {
            ConfigFormat::Opencode => "opencode",
            ConfigFormat::Kilocode => "kilocode",
            ConfigFormat::Mimocode => "mimocode",
            ConfigFormat::Pi => "pi",
            ConfigFormat::OhMyPi => "oh-my-pi",
            ConfigFormat::DeepSeekHarness => "deepseek-harness",
            ConfigFormat::ZCode => "zcode",
            ConfigFormat::WorkBuddy => "workbuddy",
            ConfigFormat::QwenCode => "qwen-code",
            ConfigFormat::KimiCode => "kimi-code",
        }
    }

    /// Whether it belongs to the opencode family: opencode / kilocode / mimocode.
    ///
    /// The three share identical schema fields and structure — parsing, serialization, field visibility, and
    /// agents support all reuse one implementation; only the config directory, primary file name, and icon
    /// differ, so detection must go by path.
    ///
    /// mimocode additionally requires `modalities` pairs (see
    /// `backends::opencode::complete_required_model_fields`).
    pub fn is_opencode_family(&self) -> bool {
        matches!(
            self,
            ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode
        )
    }

    /// Whether the backend has a model-level enable switch (WorkBuddy only).
    ///
    /// When on, models write `disabled` and the effective list holds only enabled entries. ZCode's
    /// `config.enabled` is maintained by its own UI; this tool preserves it verbatim.
    pub fn has_model_enable(&self) -> bool {
        matches!(self, ConfigFormat::WorkBuddy)
    }
}

/// Each backend's parsed path container (including user-overridable local paths).
pub struct ConfigPaths {
    pub opencode: String,
    pub kilocode: String,
    pub mimocode: String,
    pub pi: String,
    pub oh_my_pi: String,
    pub deepseek_harness: String,
    pub zcode: String,
    pub workbuddy: String,
    pub qwen_code: String,
    pub kimi_code: String,
}

impl Default for ConfigPaths {
    fn default() -> Self {
        Self {
            opencode: backends::backend(ConfigFormat::Opencode).default_local_path(),
            kilocode: backends::backend(ConfigFormat::Kilocode).default_local_path(),
            mimocode: backends::backend(ConfigFormat::Mimocode).default_local_path(),
            pi: backends::backend(ConfigFormat::Pi).default_local_path(),
            oh_my_pi: backends::backend(ConfigFormat::OhMyPi).default_local_path(),
            deepseek_harness: backends::backend(ConfigFormat::DeepSeekHarness).default_local_path(),
            zcode: backends::backend(ConfigFormat::ZCode).default_local_path(),
            workbuddy: backends::backend(ConfigFormat::WorkBuddy).default_local_path(),
            qwen_code: backends::backend(ConfigFormat::QwenCode).default_local_path(),
            kimi_code: backends::backend(ConfigFormat::KimiCode).default_local_path(),
        }
    }
}

impl ConfigPaths {
    /// The local path this backend actually uses.
    pub fn local_path(&self, format: ConfigFormat) -> String {
        match format {
            ConfigFormat::Opencode => self.opencode.clone(),
            ConfigFormat::Kilocode => self.kilocode.clone(),
            ConfigFormat::Mimocode => self.mimocode.clone(),
            ConfigFormat::Pi => self.pi.clone(),
            ConfigFormat::OhMyPi => self.oh_my_pi.clone(),
            ConfigFormat::DeepSeekHarness => self.deepseek_harness.clone(),
            ConfigFormat::ZCode => self.zcode.clone(),
            ConfigFormat::WorkBuddy => self.workbuddy.clone(),
            ConfigFormat::QwenCode => self.qwen_code.clone(),
            ConfigFormat::KimiCode => self.kimi_code.clone(),
        }
    }

    /// Write a backend's local path (both manual overrides and restoring the default come through here).
    pub fn set_local_path(&mut self, format: ConfigFormat, path: &str) {
        match format {
            ConfigFormat::Opencode => self.opencode = path.to_string(),
            ConfigFormat::Kilocode => self.kilocode = path.to_string(),
            ConfigFormat::Mimocode => self.mimocode = path.to_string(),
            ConfigFormat::Pi => self.pi = path.to_string(),
            ConfigFormat::OhMyPi => self.oh_my_pi = path.to_string(),
            ConfigFormat::DeepSeekHarness => self.deepseek_harness = path.to_string(),
            ConfigFormat::ZCode => self.zcode = path.to_string(),
            ConfigFormat::WorkBuddy => self.workbuddy = path.to_string(),
            ConfigFormat::QwenCode => self.qwen_code = path.to_string(),
            ConfigFormat::KimiCode => self.kimi_code = path.to_string(),
        }
    }

    /// The backend's "probed default path" (for judging whether a page was overridden by the user).
    pub fn default_local_path(format: ConfigFormat) -> String {
        backends::backend(format).default_local_path()
    }

    /// Replace default paths with overrides from prefs (empty string = no override, keep the auto-detected value).
    pub fn apply_overrides(&mut self, overrides: &ConfigPathPrefs) {
        for backend in backends::BACKENDS {
            let path = overrides.get(backend.id());
            if !path.trim().is_empty() {
                self.set_local_path(backend.id(), path);
            }
        }
    }

    /// Startup probing: pages whose "user-overridden path exists" win; default-path probing comes second.
    pub fn detect_preferring_overrides(
        &self,
        overrides: &ConfigPathPrefs,
    ) -> Option<(ConfigFormat, String)> {
        for backend in backends::BACKENDS {
            let id = backend.id();
            if overrides.get(id).trim().is_empty() {
                continue;
            }
            // Overridden paths go through candidate resolution too (`.json` / `.jsonc`).
            let path = backends::resolve_local_path(id, &self.local_path(id));
            if Path::new(&path).exists() {
                return Some((id, path));
            }
        }
        Self::detect()
    }

    /// Startup probing: find the first locally-existing default config.
    ///
    /// Paths go through candidate resolution (the opencode family's `.json` / `.jsonc`).
    pub fn detect() -> Option<(ConfigFormat, String)> {
        for b in backends::BACKENDS {
            let id = b.id();
            let p = backends::resolve_local_path(id, &b.default_local_path());
            if Path::new(&p).exists() {
                return Some((id, p));
            }
        }
        None
    }

    /// Detect the config format from file content (no extension context; used by the jsonc test).
    pub fn detect_from_content(content: &str) -> ConfigFormat {
        backends::detect_format(content, "")
    }

    /// Detect the config format from path + content (WSL paths supported).
    /// When content is unavailable (creation scenarios), infer by extension: `.yml/.yaml` → oh-my-pi, else opencode.
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

    /// Whether the target is usable (local or WSL).
    pub fn validate_target(&self, format: ConfigFormat) -> bool {
        backends::target_available(format, &self.local_path(format))
    }

    /// Pages' top-bar display order: installed first, uninstalled after, name-initial within each group.
    ///
    /// `saved_order` is the user's drag order (backend identifiers, i.e. `ConfigFormat::label()` values) and
    /// applies to the installed group only.
    pub fn tab_order(
        &self,
        saved_order: &[String],
        is_installed: impl Fn(ConfigFormat) -> bool,
    ) -> Vec<ConfigFormat> {
        let mut installed: Vec<ConfigFormat> = Vec::new();
        let mut missing: Vec<ConfigFormat> = Vec::new();
        for backend in backends::BACKENDS {
            let id = backend.id();
            if is_installed(id) {
                installed.push(id);
            } else {
                missing.push(id);
            }
        }
        // Installed group: the user's drag order first, the rest appended by name initial.
        let rank = |id: ConfigFormat| {
            saved_order
                .iter()
                .position(|k| k == id.label())
                .unwrap_or(usize::MAX)
        };
        installed.sort_by(|a, b| {
            let (ra, rb) = (rank(*a), rank(*b));
            ra.cmp(&rb).then_with(|| a.label().cmp(b.label()))
        });
        missing.sort_by(|a, b| a.label().cmp(b.label()));
        installed.extend(missing);
        installed
    }

    /// Local first: write local when the local file exists, else fall back to WSL, finally the local default (creation scenarios).
    pub fn target_path(&self, format: ConfigFormat) -> String {
        backends::target_path(format, &self.local_path(format))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::ConfigPathPrefs;

    /// Top-bar order: installed first (alphabetical), uninstalled after (alphabetical).
    #[test]
    fn tab_order_puts_installed_first_then_alphabetical() {
        let paths = ConfigPaths::default();
        // Pretend only pi / zcode / workbuddy are installed
        let order = paths.tab_order(&[], |id| {
            matches!(
                id,
                ConfigFormat::Pi | ConfigFormat::ZCode | ConfigFormat::WorkBuddy
            )
        });
        let labels: Vec<&str> = order.iter().map(|id| id.label()).collect();
        assert_eq!(
            labels,
            vec![
                "pi",
                "workbuddy",
                "zcode",
                "deepseek-harness",
                "kilocode",
                "kimi-code",
                "mimocode",
                "oh-my-pi",
                "opencode",
                "qwen-code"
            ],
            "已安装的按字母在前，未安装的按字母在后"
        );
    }

    /// The saved drag order affects the installed segment only.
    #[test]
    fn tab_order_applies_saved_order_to_installed_only() {
        let paths = ConfigPaths::default();
        let saved = vec!["zcode".to_string(), "pi".to_string()];
        let order = paths.tab_order(&saved, |id| {
            matches!(id, ConfigFormat::Pi | ConfigFormat::ZCode)
        });
        let labels: Vec<&str> = order.iter().map(|id| id.label()).collect();
        assert_eq!(labels[0], "zcode", "拖动顺序生效");
        assert_eq!(labels[1], "pi");
        // Uninstalled ones still trail alphabetically, unaffected by saved
        assert_eq!(
            &labels[2..],
            &[
                "deepseek-harness",
                "kilocode",
                "kimi-code",
                "mimocode",
                "oh-my-pi",
                "opencode",
                "qwen-code",
                "workbuddy"
            ],
        );
    }

    /// Uninstalled entries in the drag order are ignored.
    #[test]
    fn tab_order_ignores_saved_entries_that_are_not_installed() {
        let paths = ConfigPaths::default();
        let saved = vec![
            // not installed
            "zcode".to_string(),
        ];
        let order = paths.tab_order(&saved, |id| {
            matches!(id, ConfigFormat::ZCode | ConfigFormat::Pi)
        });
        let labels: Vec<&str> = order.iter().map(|id| id.label()).collect();
        assert_eq!(labels[0], "zcode", "未安装的项被跳过，不影响已安装的排序");
        assert_eq!(labels[1], "pi", "剩下的按字母补在后面");
    }

    /// A newly installed agent: inserted alphabetically into the installed segment without disturbing existing order.
    #[test]
    fn tab_order_keeps_saved_prefix_when_a_new_agent_appears() {
        let paths = ConfigPaths::default();
        let saved = vec!["zcode".to_string()];
        let order = paths.tab_order(&saved, |id| {
            matches!(id, ConfigFormat::ZCode | ConfigFormat::Pi)
        });
        let labels: Vec<&str> = order.iter().map(|id| id.label()).collect();
        assert_eq!(labels[0], "zcode", "已保存的顺序保持");
        assert_eq!(labels[1], "pi", "新装的按字母补进已安装段");
        assert_eq!(labels[2], "deepseek-harness", "未安装段不受影响");
    }

    /// With none installed, everything is alphabetical.
    #[test]
    fn tab_order_all_alphabetical_when_nothing_installed() {
        let paths = ConfigPaths::default();
        let order = paths.tab_order(&[], |_| false);
        let labels: Vec<&str> = order.iter().map(|id| id.label()).collect();
        assert_eq!(
            labels,
            vec![
                "deepseek-harness",
                "kilocode",
                "kimi-code",
                "mimocode",
                "oh-my-pi",
                "opencode",
                "pi",
                "qwen-code",
                "workbuddy",
                "zcode"
            ],
        );
    }

    /// The order is a permutation of all backends, without repeats.
    #[test]
    fn tab_order_covers_every_backend_exactly_once() {
        let paths = ConfigPaths::default();
        for installed in [ConfigFormat::Opencode, ConfigFormat::WorkBuddy] {
            let order = paths.tab_order(&[], |id| id == installed);
            assert_eq!(order.len(), crate::backends::BACKENDS.len());
            let mut sorted: Vec<&str> = order.iter().map(|id| id.label()).collect();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), order.len(), "不得重复");
        }
    }

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
        // Only pi overridden, and the file exists → startup lands on pi, skipping default-order probing
        overrides.set(ConfigFormat::Pi, &file);
        paths.apply_overrides(&overrides);
        assert_eq!(
            paths.detect_preferring_overrides(&overrides),
            Some((ConfigFormat::Pi, file.clone()))
        );
        // The overridden path doesn't exist → not preferred, falls back to default probing
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
