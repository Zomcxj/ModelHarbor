//! The tool's own settings persistence: `.modelharbor/settings.json` under the home directory
//! (Windows: `C:\Users\<user>\.modelharbor\settings.json`).
//!
//! Stores only UI choices (key visibility, save format, theme, WSL sync, card collapse, per-page config
//! path overrides) — never keys, models, or config contents.
//!
//! Missing / unreadable / unparseable files all yield defaults; empty strings mean "never set".
//! Reads fall back through "new name → old name" and "home → %APPDATA%"; writes only produce the new
//! name, and the old file in the same directory is cleaned up after the first write.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// Read a nested string of the form `root.a.b` (type mismatches count as empty).
fn nested_str(root: &Value, outer: &str, inner: &str) -> String {
    root.get(outer)
        .and_then(|value| value.get(inner))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Config directory name (a dot-directory under home).
pub const DIR_NAME: &str = ".modelharbor";
const FILE_NAME: &str = "settings.json";
/// Old file name: only for read fallback and post-write cleanup.
const LEGACY_FILE_NAME: &str = "prefs.json";
/// Crash log file name.
const CRASH_FILE_NAME: &str = "crash.log";

/// Home directory: `USERPROFILE` on Windows, `HOME` elsewhere.
pub fn home_dir() -> Option<PathBuf> {
    ["USERPROFILE", "HOME"]
        .into_iter()
        .find_map(|key| std::env::var_os(key).map(PathBuf::from))
        .filter(|dir| !dir.as_os_str().is_empty())
}

/// Config directory (`.modelharbor` under home): settings, credentials, and the crash log live here.
pub fn config_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(DIR_NAME))
}

/// Crash log path (`<config dir>/crash.log`).
pub fn crash_log_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(CRASH_FILE_NAME))
}

/// Append one record to the given file: creates the parent directory when missing; returns `false` on write failure.
fn append_line(path: &Path, entry: &str) -> bool {
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return false;
        }
    }
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return false;
    };
    file.write_all(entry.as_bytes()).is_ok() && file.flush().is_ok()
}

/// Append one crash record to `crash.log`: returns whether the write succeeded; never panics itself.
pub fn append_crash_log(entry: &str) -> bool {
    crash_log_path().is_some_and(|path| append_line(&path, entry))
}

/// Old location (the same-named directory under `%APPDATA%`).
fn legacy_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join(DIR_NAME))
}

/// Read candidate paths in priority order:
/// home settings.json → home prefs.json → both under %APPDATA%.
fn read_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = home_dir() {
        let dir = home.join(DIR_NAME);
        out.push(dir.join(FILE_NAME));
        out.push(dir.join(LEGACY_FILE_NAME));
    }
    if let Some(dir) = legacy_dir() {
        out.push(dir.join(FILE_NAME));
        out.push(dir.join(LEGACY_FILE_NAME));
    }
    out
}

/// Remove old file names in the same directory; failures don't affect the current save.
fn remove_legacy_next_to(path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    let legacy = dir.join(LEGACY_FILE_NAME);
    if legacy != path && legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
    }
}

/// Schema version for the written file (ignored on read).
const SCHEMA_VERSION: u64 = 12;

/// Config identity: a stable FNV-1a hash of the normalized path.
pub fn config_identity(path: &str) -> String {
    let normalized = path.trim().replace('\\', "/").to_lowercase();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in normalized.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Persistence key for collapse state: `config identity/category/name`.
pub fn collapsed_id(config_id: &str, kind: &str, key: &str) -> String {
    format!("{config_id}/{kind}/{key}")
}

/// v2 collapse key: `category/name` (only for migration on first successful load).
pub fn legacy_collapsed_id(kind: &str, key: &str) -> String {
    format!("{kind}/{key}")
}

/// Per-backend config path overrides (empty string = no override, use the default path auto-detected at startup).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigPathPrefs {
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

impl ConfigPathPrefs {
    /// Per-backend lookup.
    pub fn get(&self, format: crate::format::ConfigFormat) -> &str {
        match format {
            crate::format::ConfigFormat::Opencode => &self.opencode,
            crate::format::ConfigFormat::Kilocode => &self.kilocode,
            crate::format::ConfigFormat::Mimocode => &self.mimocode,
            crate::format::ConfigFormat::Pi => &self.pi,
            crate::format::ConfigFormat::OhMyPi => &self.oh_my_pi,
            crate::format::ConfigFormat::DeepSeekHarness => &self.deepseek_harness,
            crate::format::ConfigFormat::ZCode => &self.zcode,
            crate::format::ConfigFormat::WorkBuddy => &self.workbuddy,
            crate::format::ConfigFormat::QwenCode => &self.qwen_code,
            crate::format::ConfigFormat::KimiCode => &self.kimi_code,
        }
    }

    /// Per-backend write (empty string = clear the override).
    pub fn set(&mut self, format: crate::format::ConfigFormat, path: &str) {
        let slot = match format {
            crate::format::ConfigFormat::Opencode => &mut self.opencode,
            crate::format::ConfigFormat::Kilocode => &mut self.kilocode,
            crate::format::ConfigFormat::Mimocode => &mut self.mimocode,
            crate::format::ConfigFormat::Pi => &mut self.pi,
            crate::format::ConfigFormat::OhMyPi => &mut self.oh_my_pi,
            crate::format::ConfigFormat::DeepSeekHarness => &mut self.deepseek_harness,
            crate::format::ConfigFormat::ZCode => &mut self.zcode,
            crate::format::ConfigFormat::WorkBuddy => &mut self.workbuddy,
            crate::format::ConfigFormat::QwenCode => &mut self.qwen_code,
            crate::format::ConfigFormat::KimiCode => &mut self.kimi_code,
        };
        *slot = path.to_string();
    }
}

/// UI preferences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// Whether all API keys show plaintext.
    pub show_api_keys: bool,
    /// Save-format identifier (`current` / `compact`; empty = the App default).
    pub save_format: String,
    /// Theme identifier (`dark` / `light` / `ocean` / `nord` / `rose`; empty = the App default).
    pub theme: String,
    /// Whether to sync writes to the WSL-side paths.
    pub sync_wsl: bool,
    /// Each backend's own config path override (non-empty only when the user specified one).
    pub config_paths: ConfigPathPrefs,
    /// Collapsed cards (`config identity/category/name`; anything absent from the map is expanded).
    /// v2's old `category/name` keys migrate into the current config identity after the first successful load.
    pub collapsed: Vec<String>,
    /// Whether "model latency tests" stay allowed when a system proxy / VPN is detected.
    ///
    /// Defaults to `false`; affects latency tests only — not connectivity tests or usage queries.
    pub allow_model_test_with_proxy: bool,
    /// Whether the first-use guidance bar has been dismissed.
    pub guide_dismissed: bool,
    /// UI shape identifier (`soft` / `compact` / `slab` / `sharp` / `panel` / `pill`; empty = the default tier).
    pub ui_style: String,
    /// Acrylic: translucent panels / cards revealing DWM acrylic blur (orthogonal to theme and shape).
    pub glass: bool,
    /// The top bar's installed-page drag order (backend identifiers; unlisted ones append alphabetically).
    /// Applies to the installed group only; uninstalled pages trail in alphabetical order.
    pub tab_order: Vec<String>,
    /// Resident taskbar: clicking X only tucks the main window into the tray; the tray icon restores / quits.
    ///
    /// Old settings without the field default to on (user asked for resident-by-default).
    pub tray_resident: bool,
    /// The usage page's remembered range key (`today` / `week` / `month` / `all`; empty = the App default).
    pub usage_range: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            show_api_keys: false,
            save_format: String::new(),
            theme: String::new(),
            sync_wsl: false,
            config_paths: ConfigPathPrefs::default(),
            collapsed: Vec::new(),
            allow_model_test_with_proxy: false,
            guide_dismissed: false,
            ui_style: String::new(),
            glass: false,
            tab_order: Vec::new(),
            // Resident-by-default, lit (user request).
            tray_resident: true,
            usage_range: String::new(),
        }
    }
}

impl Prefs {
    /// The tool's settings directory (`.modelharbor` under home): all self-owned files concentrate here
    /// (`settings.json` and the station tokens `tokens.json`), never inside any agent's config directory.
    ///
    /// Without a home directory, falls back to `%APPDATA%\.modelharbor`, then the executable's own directory.
    pub fn config_dir() -> PathBuf {
        if let Some(home) = home_dir() {
            return home.join(DIR_NAME);
        }
        legacy_dir().unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("."))
        })
    }

    /// Settings file path (`.modelharbor/settings.json` under home).
    pub fn path() -> PathBuf {
        Self::config_dir().join(FILE_NAME)
    }

    /// Load from the default path.
    ///
    /// Falls back through [`read_candidates`] in order; nothing readable yields defaults.
    pub fn load() -> Prefs {
        for candidate in read_candidates() {
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                return Self::parse(&text);
            }
        }
        Prefs::default()
    }

    /// Parse existing content (used by loading and unit tests).
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
        // Read a string-array field (the collapse table / check-in grants).
        let get_list = |key: &str| {
            root.get(key)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
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
            config_paths: ConfigPathPrefs {
                opencode: nested_str(&root, "config_paths", "opencode"),
                kilocode: nested_str(&root, "config_paths", "kilocode"),
                mimocode: nested_str(&root, "config_paths", "mimocode"),
                pi: nested_str(&root, "config_paths", "pi"),
                oh_my_pi: nested_str(&root, "config_paths", "oh_my_pi"),
                deepseek_harness: nested_str(&root, "config_paths", "deepseek_harness"),
                zcode: nested_str(&root, "config_paths", "zcode"),
                workbuddy: nested_str(&root, "config_paths", "workbuddy"),
                qwen_code: nested_str(&root, "config_paths", "qwen_code"),
                kimi_code: nested_str(&root, "config_paths", "kimi_code"),
            },
            collapsed: get_list("collapsed"),
            // Only a genuine boolean true passes: missing fields and the string "true" both count as blocked.
            allow_model_test_with_proxy: root
                .get("allow_model_test_with_proxy")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            // Missing field = never dismissed = show the guidance.
            guide_dismissed: root
                .get("guide_dismissed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ui_style: get_str("ui_style"),
            // Only a genuine boolean true enables acrylic: missing fields and the string "true" both count as off.
            glass: root.get("glass").and_then(Value::as_bool).unwrap_or(false),
            tab_order: get_list("tab_order"),
            // Missing field = old settings file: default on (user asked for resident-by-default).
            tray_resident: root
                .get("tray_resident")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            usage_range: get_str("usage_range"),
        }
    }

    /// Serialize (fixed field order).
    pub fn to_json(&self) -> String {
        let mut root = Map::new();
        root.insert("version".to_string(), Value::Number(SCHEMA_VERSION.into()));
        root.insert("show_api_keys".to_string(), Value::Bool(self.show_api_keys));
        root.insert(
            "save_format".to_string(),
            Value::String(self.save_format.clone()),
        );
        root.insert("theme".to_string(), Value::String(self.theme.clone()));
        root.insert("sync_wsl".to_string(), Value::Bool(self.sync_wsl));
        let mut paths = Map::new();
        for (key, value) in [
            ("opencode", &self.config_paths.opencode),
            ("kilocode", &self.config_paths.kilocode),
            ("mimocode", &self.config_paths.mimocode),
            ("pi", &self.config_paths.pi),
            ("oh_my_pi", &self.config_paths.oh_my_pi),
            ("deepseek_harness", &self.config_paths.deepseek_harness),
            ("zcode", &self.config_paths.zcode),
            ("workbuddy", &self.config_paths.workbuddy),
            ("qwen_code", &self.config_paths.qwen_code),
            ("kimi_code", &self.config_paths.kimi_code),
        ] {
            paths.insert(key.to_string(), Value::String(value.clone()));
        }
        root.insert("config_paths".to_string(), Value::Object(paths));
        // Both string arrays are sorted and deduped before writing.
        let sorted_array = |items: &[String]| {
            let mut list: Vec<&String> = items.iter().collect();
            list.sort();
            list.dedup();
            Value::Array(
                list.into_iter()
                    .map(|key| Value::String(key.clone()))
                    .collect(),
            )
        };
        root.insert("collapsed".to_string(), sorted_array(&self.collapsed));
        root.insert(
            "allow_model_test_with_proxy".to_string(),
            Value::Bool(self.allow_model_test_with_proxy),
        );
        root.insert(
            "guide_dismissed".to_string(),
            Value::Bool(self.guide_dismissed),
        );
        root.insert("ui_style".to_string(), Value::String(self.ui_style.clone()));
        root.insert("glass".to_string(), Value::Bool(self.glass));
        // The top-bar order is written as-is: the order itself IS this key's content — no sorting or dedup.
        root.insert(
            "tab_order".to_string(),
            Value::Array(
                self.tab_order
                    .iter()
                    .map(|k| Value::String(k.clone()))
                    .collect(),
            ),
        );
        root.insert("tray_resident".to_string(), Value::Bool(self.tray_resident));
        root.insert(
            "usage_range".to_string(),
            Value::String(self.usage_range.clone()),
        );
        serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".to_string())
    }

    /// Flush to the default path and clean up the old file name in the same directory.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        let result = self.save_to(&path);
        if result.is_ok() {
            remove_legacy_next_to(&path);
        }
        result
    }

    /// Flush to a given path (unit tests).
    ///
    /// Atomic write: a same-directory temp file first with sync, then replace; a failed replace keeps the original settings.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        crate::util::atomic_write_text(path, &self.to_json())
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
            config_paths: ConfigPathPrefs {
                opencode: "D:\\conf\\opencode.json".to_string(),
                kilocode: String::new(),
                mimocode: String::new(),
                pi: String::new(),
                oh_my_pi: String::new(),
                deepseek_harness: "D:\\conf\\dsh.yaml".to_string(),
                zcode: String::new(),
                workbuddy: String::new(),
                qwen_code: String::new(),
                kimi_code: String::new(),
            },
            // Given alphabetically; to_json sorts on write.
            collapsed: vec![
                collapsed_id("cfg", "agents", "build"),
                collapsed_id("cfg", "providers", "openai"),
            ],
            allow_model_test_with_proxy: true,
            guide_dismissed: true,
            ui_style: "slab".to_string(),
            // Non-default value, overriding the acrylic tier's read/write.
            glass: true,
            tab_order: vec!["pi".to_string(), "zcode".to_string()],
            tray_resident: false,
            // Non-default value, overriding the usage range's read/write.
            usage_range: "week".to_string(),
        };
        assert_eq!(Prefs::parse(&prefs.to_json()), prefs);
    }

    /// The top-bar order is written as-is, unsorted.
    #[test]
    fn tab_order_is_written_verbatim_not_sorted() {
        let prefs = Prefs {
            tab_order: vec!["zcode".into(), "pi".into(), "opencode".into()],
            ..Default::default()
        };
        let back = Prefs::parse(&prefs.to_json());
        assert_eq!(
            back.tab_order,
            vec![
                "zcode".to_string(),
                "pi".to_string(),
                "opencode".to_string()
            ],
            "顺序本身是内容，不得排序"
        );
    }

    #[test]
    fn collapsed_is_written_sorted_and_deduped() {
        let prefs = Prefs {
            collapsed: vec!["b/2".into(), "a/1".into(), "b/2".into()],
            ..Default::default()
        };
        let text = prefs.to_json();
        assert!(text.contains("a/1"), "{text}");
        assert_eq!(text.matches("b/2").count(), 1, "不应重复：{text}");
        let back = Prefs::parse(&text);
        assert_eq!(back.collapsed, vec!["a/1".to_string(), "b/2".to_string()]);
        // Writing again must be byte-identical.
        assert_eq!(back.to_json(), text);
    }

    #[test]
    fn wrong_types_fall_back_for_new_fields() {
        for text in [
            r#"{"collapsed":"nope"}"#,
            r#"{"collapsed":[1,2,{"a":1}]}"#,
            r#"{"config_paths":"nope"}"#,
            r#"{"config_paths":{"opencode":42,"pi":null}}"#,
        ] {
            let prefs = Prefs::parse(text);
            assert_eq!(prefs.collapsed, Vec::<String>::new(), "{text}");
            assert_eq!(prefs.config_paths, ConfigPathPrefs::default(), "{text}");
        }
    }

    #[test]
    fn config_identity_normalizes_case_and_path_separators() {
        assert_eq!(
            config_identity(r"C:\Users\Alice\.pi\agent\models.json"),
            config_identity("c:/users/alice/.pi/agent/models.json")
        );
        assert_ne!(
            config_identity(r"C:\configs\one.json"),
            config_identity(r"C:\configs\two.json")
        );
    }

    #[test]
    fn legacy_v2_collapsed_keys_remain_available_for_app_migration() {
        let prefs = Prefs::parse(r#"{"version":2,"collapsed":["providers/p","agents/a"]}"#);
        assert_eq!(
            prefs.collapsed,
            vec!["providers/p".to_string(), "agents/a".to_string()]
        );
        // The written version follows the current schema.
        assert!(
            prefs
                .to_json()
                .contains(&format!(r#""version": {SCHEMA_VERSION}"#)),
            "{}",
            prefs.to_json()
        );
    }

    #[test]
    fn glass_defaults_off_and_round_trips() {
        // Off by default.
        assert!(!Prefs::default().glass);
        // Only a genuine boolean true enables: missing fields and the string "true" both count as off.
        assert!(!Prefs::parse(r#"{"version":3,"show_api_keys":true}"#).glass);
        assert!(!Prefs::parse(r#"{"glass":"true"}"#).glass);
        assert!(Prefs::parse(r#"{"glass":true}"#).glass);
        let prefs = Prefs {
            glass: true,
            ..Default::default()
        };
        let text = prefs.to_json();
        assert!(text.contains(r#""glass": true"#), "{text}");
        assert!(Prefs::parse(&text).glass);
        assert_eq!(Prefs::parse(&text), prefs);
    }

    #[test]
    fn proxy_model_test_override_defaults_off_and_round_trips() {
        // Missing field defaults to blocking.
        assert!(
            !Prefs::default().allow_model_test_with_proxy,
            "默认要保持拦截"
        );
        assert!(!Prefs::parse(r#"{"version":3,"show_api_keys":true}"#).allow_model_test_with_proxy);
        assert!(
            !Prefs::parse(r#"{"allow_model_test_with_proxy":"true"}"#).allow_model_test_with_proxy,
            "非布尔值不能当开放行"
        );

        let text = Prefs {
            allow_model_test_with_proxy: true,
            ..Default::default()
        }
        .to_json();
        assert!(
            text.contains(r#""allow_model_test_with_proxy": true"#),
            "开关要落盘：{text}"
        );
        assert!(Prefs::parse(&text).allow_model_test_with_proxy);
    }
    #[test]
    fn config_path_overrides_are_keyed_by_backend() {
        use crate::format::ConfigFormat;
        let mut paths = ConfigPathPrefs::default();
        paths.set(ConfigFormat::OhMyPi, "D:\\conf\\models.yml");
        assert_eq!(paths.get(ConfigFormat::OhMyPi), "D:\\conf\\models.yml");
        assert_eq!(paths.get(ConfigFormat::Pi), "", "其他页不受影响");
        paths.set(ConfigFormat::OhMyPi, "");
        assert_eq!(paths.get(ConfigFormat::OhMyPi), "", "空串 = 清除覆盖");
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
        assert!(
            !path.with_file_name(format!("{FILE_NAME}.tmp")).exists(),
            "成功替换后不得残留临时文件"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn failed_atomic_replace_preserves_existing_settings() {
        let dir = std::env::temp_dir().join(format!(
            "{}-atomic-failure-{}",
            DIR_NAME,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "old settings").expect("写旧设置");
        let temp_path = path.with_file_name(format!("{FILE_NAME}.tmp"));
        std::fs::create_dir(&temp_path).expect("用目录占住临时文件路径");

        let err = Prefs::default()
            .save_to(&path)
            .expect_err("无法创建临时文件时保存应失败");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old settings");
        assert!(err.contains(&path.display().to_string()));
        assert!(!err.contains("show_api_keys"), "错误不得包含设置正文");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_candidates_try_new_name_and_home_first() {
        let candidates: Vec<String> = read_candidates()
            .iter()
            .map(|path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        // The first two candidates: home's new name → home's old name.
        assert_eq!(candidates.first().map(String::as_str), Some(FILE_NAME));
        assert_eq!(
            candidates.get(1).map(String::as_str),
            Some(LEGACY_FILE_NAME)
        );
        assert!(
            candidates
                .iter()
                .all(|name| name == FILE_NAME || name == LEGACY_FILE_NAME),
            "只回退这两个文件名：{candidates:?}"
        );
        if let Some(home) = home_dir() {
            assert!(
                read_candidates()[0].starts_with(&home),
                "新位置必须在家目录"
            );
        }
    }

    #[test]
    fn legacy_file_is_removed_after_first_save() {
        let dir = std::env::temp_dir().join(format!("{}-legacy-{}", DIR_NAME, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let new = dir.join(FILE_NAME);
        let old = dir.join(LEGACY_FILE_NAME);
        std::fs::write(&old, "{\"theme\":\"nord\"}").expect("写旧文件");
        Prefs::default().save_to(&new).expect("写新位置");
        remove_legacy_next_to(&new);
        assert!(new.exists(), "新文件应保留");
        assert!(!old.exists(), "旧文件应被清理");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_path_is_app_private() {
        let path = Prefs::path();
        assert_eq!(path.file_name().unwrap_or_default(), FILE_NAME);
        assert_eq!(
            path.parent().and_then(|dir| dir.file_name()),
            Some(std::ffi::OsStr::new(DIR_NAME))
        );
        let text = path.to_string_lossy().to_lowercase();
        for forbidden in [".config", ".pi", ".omp", ".dsh", "opencode"] {
            assert!(!text.contains(forbidden), "不应写进 agent 配置：{text}");
        }
        // With a home directory, it must live at home.
        let has_home =
            std::env::var_os("USERPROFILE").is_some() || std::env::var_os("HOME").is_some();
        if has_home {
            assert!(
                !text.contains("roaming"),
                "应放家目录而不是 %APPDATA%：{text}"
            );
        }
    }

    #[test]
    fn guide_shows_until_it_is_dismissed() {
        // Missing field = never dismissed = show the guidance.
        assert!(!Prefs::default().guide_dismissed);
        assert!(!Prefs::parse(r#"{"version":6}"#).guide_dismissed);
        // The string "true" doesn't count as dismissed — only genuine booleans do.
        assert!(!Prefs::parse(r#"{"guide_dismissed":"true"}"#).guide_dismissed);
        assert!(Prefs::parse(r#"{"guide_dismissed":true}"#).guide_dismissed);
    }

    #[test]
    fn guide_state_round_trips_through_json() {
        let prefs = Prefs {
            guide_dismissed: true,
            ..Default::default()
        };
        let text = prefs.to_json();
        assert!(text.contains(r#""guide_dismissed": true"#), "{text}");
        assert!(Prefs::parse(&text).guide_dismissed);
        assert_eq!(Prefs::parse(&text), prefs);
    }

    #[test]
    fn ui_style_defaults_to_empty_and_round_trips() {
        // Empty string = use the default tier.
        assert_eq!(Prefs::default().ui_style, "");
        assert_eq!(Prefs::parse(r#"{"ui_style":"slab"}"#).ui_style, "slab");
        // A type mismatch counts as never-set.
        assert_eq!(Prefs::parse(r#"{"ui_style":3}"#).ui_style, "");
        let prefs = Prefs {
            ui_style: "band".to_string(),
            ..Default::default()
        };
        let text = prefs.to_json();
        assert!(text.contains(r#""ui_style": "band""#), "{text}");
        assert_eq!(Prefs::parse(&text), prefs);
    }

    #[test]
    fn crash_log_sits_next_to_the_settings_file() {
        let dir = config_dir().expect("本机有家目录");
        assert_eq!(dir.file_name().unwrap(), DIR_NAME);
        assert_eq!(crash_log_path().unwrap(), dir.join("crash.log"));
    }

    #[test]
    fn crash_log_creates_its_directory_and_appends() {
        let dir = std::env::temp_dir().join(format!("{}-crash-{}", DIR_NAME, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("crash.log");
        assert!(append_line(&path, "first\n"), "应建出父目录并写入");
        assert!(append_line(&path, "second\n"), "第二次应追加成功");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
