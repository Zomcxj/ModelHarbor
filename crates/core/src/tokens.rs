//! Station panel tokens (PAT) persistence: `.modelharbor/tokens.json` under the home directory.
//!
//! Stored separately from `settings.json`, inside the [[`crate::prefs::Prefs::config_dir`]] directory;
//! deleting this file clears every token.
//!
//! Keys are normalized origins (`scheme://host`, lowercase, no trailing slash), reusing [`crate::billing::endpoints`]'
//! origin derivation directly.
//!
//! The file holds credentials: never into `settings.json`, never into agent config files, never into logs /
//! status bars / error text; error messages carry paths only, never content.

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "tokens.json";
/// Schema version for the written file (ignored on read).
const SCHEMA_VERSION: u64 = 3;

/// Station kind marker value: sub2api (queries go straight to `/v1/usage`, no panel token needed).
const KIND_SUB2API: &str = "sub2api";

/// Station token table (key = normalized origin, value = PAT).
///
/// A `BTreeMap` keeps the written order stable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StationTokens {
    tokens: BTreeMap<String, String>,
    /// The user ID required by older new-api (the `New-Api-User` header), keyed like `tokens`.
    /// Newer versions don't need it; empty at most stations.
    user_ids: BTreeMap<String, String>,
    /// Station-kind markers (only non-default values persist; default = the NewAPI panel system).
    /// The only non-default so far: `sub2api` — balances query `/v1/usage` directly with the `sk-` key,
    /// needing neither the panel token nor a user ID.
    site_kinds: BTreeMap<String, String>,
}

/// Station identity: normalize a baseUrl into its "station root", the token table's key.
///
/// Derivation reuses [`crate::billing::endpoints`] (`https://host/v1` → `https://host`), then lowercases.
pub fn station_key(base_url: &str) -> String {
    crate::billing::endpoints(base_url).origin.to_lowercase()
}

impl StationTokens {
    /// The token file's path (same directory as the settings file).
    pub fn path() -> PathBuf {
        crate::prefs::Prefs::config_dir().join(FILE_NAME)
    }

    /// Load from the default path; missing / unreadable / unparseable files all yield an empty table.
    pub fn load() -> StationTokens {
        let path = Self::path();
        std::fs::read_to_string(&path)
            .ok()
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// Parse existing content (for loading and unit tests); any structural mismatch falls back to an empty table.
    pub fn parse(text: &str) -> StationTokens {
        let Ok(root) = serde_json::from_str::<Value>(text) else {
            return StationTokens::default();
        };
        let tokens = root
            .get("tokens")
            .and_then(Value::as_object)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(key, value)| {
                        // Only non-empty strings are collected.
                        let token = value.as_str()?;
                        let key = key.trim();
                        if key.is_empty() || token.trim().is_empty() {
                            return None;
                        }
                        Some((key.to_string(), token.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // User IDs: an absent key (v1 old files) means an empty table.
        let user_ids = root
            .get("user_ids")
            .and_then(Value::as_object)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(key, value)| {
                        let id = value.as_str()?.trim();
                        let key = key.trim();
                        if key.is_empty() || id.is_empty() {
                            return None;
                        }
                        Some((key.to_string(), id.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Station kinds: only known marker values are collected (currently sub2api); unknown values ignored.
        let site_kinds = root
            .get("site_kinds")
            .and_then(Value::as_object)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|(key, value)| {
                        let kind = value.as_str()?.trim();
                        let key = key.trim();
                        if key.is_empty() || kind != KIND_SUB2API {
                            return None;
                        }
                        Some((key.to_string(), kind.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        StationTokens {
            tokens,
            user_ids,
            site_kinds,
        }
    }

    /// Serialize (fixed key order, diff-friendly).
    pub fn to_json(&self) -> String {
        let mut root = Map::new();
        root.insert("version".to_string(), Value::Number(SCHEMA_VERSION.into()));
        let mut tokens = Map::new();
        for (key, token) in &self.tokens {
            tokens.insert(key.clone(), Value::String(token.clone()));
        }
        root.insert("tokens".to_string(), Value::Object(tokens));
        // User IDs in their own section, parallel to tokens.
        let mut user_ids = Map::new();
        for (key, id) in &self.user_ids {
            user_ids.insert(key.clone(), Value::String(id.clone()));
        }
        root.insert("user_ids".to_string(), Value::Object(user_ids));
        // Station kinds: only non-defaults are written, keeping the file compact.
        let mut site_kinds = Map::new();
        for (key, kind) in &self.site_kinds {
            site_kinds.insert(key.clone(), Value::String(kind.clone()));
        }
        root.insert("site_kinds".to_string(), Value::Object(site_kinds));
        serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".to_string())
    }

    /// Flush to the default path.
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    /// Flush to a given path (unit tests). The atomic-write strategy is shared with the settings file.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        crate::util::atomic_write_text(path, &self.to_json())
    }

    /// Take a station's token (empty string when absent).
    pub fn get(&self, origin: &str) -> &str {
        self.tokens.get(origin).map(String::as_str).unwrap_or("")
    }

    /// Whether a token is set for the station.
    pub fn has(&self, origin: &str) -> bool {
        !self.get(origin).is_empty()
    }

    /// Take a station's user ID (older new-api's `New-Api-User` header; empty string when absent).
    ///
    /// Needed only when the station replies "missing New-Api-User".
    pub fn user_id(&self, origin: &str) -> &str {
        self.user_ids
            .get(origin.trim().to_lowercase().as_str())
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Write a user ID; an empty (or whitespace-only) string deletes the station's entry.
    pub fn set_user_id(&mut self, origin: &str, id: &str) {
        let origin = origin.trim().to_lowercase();
        if origin.is_empty() {
            return;
        }
        let id = id.trim();
        if id.is_empty() {
            self.user_ids.remove(&origin);
        } else {
            self.user_ids.insert(origin, id.to_string());
        }
    }

    /// Write a token; an empty (or whitespace-only) string deletes the station's entry.
    pub fn set(&mut self, origin: &str, token: &str) {
        let origin = origin.trim().to_lowercase();
        if origin.is_empty() {
            return;
        }
        let token = token.trim();
        if token.is_empty() {
            self.tokens.remove(&origin);
        } else {
            self.tokens.insert(origin, token.to_string());
        }
    }

    /// Whether the station is marked sub2api (balances query `/v1/usage` directly with the `sk-` key).
    pub fn is_sub2api(&self, origin: &str) -> bool {
        self.site_kinds
            .get(origin.trim().to_lowercase().as_str())
            .map(String::as_str)
            == Some(KIND_SUB2API)
    }

    /// Write a station kind; `sub2api = false` restores the default (the NewAPI panel system) and clears the marker.
    pub fn set_site_kind(&mut self, origin: &str, sub2api: bool) {
        let origin = origin.trim().to_lowercase();
        if origin.is_empty() {
            return;
        }
        if sub2api {
            self.site_kinds.insert(origin, KIND_SUB2API.to_string());
        } else {
            self.site_kinds.remove(&origin);
        }
    }

    /// Delete a station's token (together with its user ID and station kind).
    pub fn remove(&mut self, origin: &str) {
        let origin = origin.trim().to_lowercase();
        self.tokens.remove(&origin);
        self.user_ids.remove(&origin);
        self.site_kinds.remove(&origin);
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("modelharbor-tokens-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn station_key_normalizes_origin_to_site_root() {
        // With a path, a trailing slash, or different casing, all must land on the same key.
        assert_eq!(
            station_key("https://gemai.huchan.cn/v1"),
            "https://gemai.huchan.cn"
        );
        assert_eq!(
            station_key("https://Gemai.Huchan.CN/v1/"),
            station_key("https://gemai.huchan.cn")
        );
        // Different stations must stay apart.
        assert_ne!(
            station_key("https://a.example.com/v1"),
            station_key("https://b.example.com/v1")
        );
    }

    #[test]
    fn user_id_is_per_station_and_normalized() {
        let mut tokens = StationTokens::default();
        assert_eq!(
            tokens.user_id("https://a.example.com"),
            "",
            "没设置就是空串"
        );
        tokens.set_user_id("  https://A.Example.com  ", " 12345 ");
        assert_eq!(tokens.user_id("https://a.example.com"), "12345");
        // Empty / whitespace-only = delete.
        tokens.set_user_id("https://a.example.com", "   ");
        assert_eq!(tokens.user_id("https://a.example.com"), "");
        assert!(tokens.is_empty(), "只剩空用户 ID 不该算“有配置”");
    }

    #[test]
    fn user_id_round_trips_and_v1_files_still_load() {
        let mut tokens = StationTokens::default();
        tokens.set("https://a.example.com", "pat-value");
        tokens.set_user_id("https://a.example.com", "777");
        let text = tokens.to_json();
        assert!(text.contains("user_ids"), "用户 ID 要落盘：{text}");
        let back = StationTokens::parse(&text);
        assert_eq!(back.get("https://a.example.com"), "pat-value");
        assert_eq!(back.user_id("https://a.example.com"), "777");

        // v1 old files (no user_ids key) still read tokens; user IDs empty.
        let legacy = r#"{"version":1,"tokens":{"https://a.example.com":"old-pat"}}"#;
        let old = StationTokens::parse(legacy);
        assert_eq!(old.get("https://a.example.com"), "old-pat");
        assert_eq!(old.user_id("https://a.example.com"), "");
        // Non-string / empty user_id values are never collected.
        let junk =
            r#"{"tokens":{},"user_ids":{"https://a.example.com":123,"https://b.example.com":""}}"#;
        assert_eq!(
            StationTokens::parse(junk).user_id("https://a.example.com"),
            ""
        );
        assert_eq!(
            StationTokens::parse(junk).user_id("https://b.example.com"),
            ""
        );
    }

    #[test]
    fn removing_a_station_also_clears_its_user_id() {
        let mut tokens = StationTokens::default();
        tokens.set("https://a.example.com", "pat-value");
        tokens.set_user_id("https://a.example.com", "777");
        tokens.remove("https://a.example.com");
        assert_eq!(tokens.user_id("https://a.example.com"), "");
        assert_eq!(tokens.get("https://a.example.com"), "");
    }

    #[test]
    fn set_get_and_remove_round_trip() {
        let mut tokens = StationTokens::default();
        assert!(!tokens.has("https://a.example.com"));
        assert_eq!(tokens.get("https://a.example.com"), "", "缺省是空串");

        tokens.set("https://a.example.com", "pat-one");
        assert!(tokens.has("https://a.example.com"));
        assert_eq!(tokens.get("https://a.example.com"), "pat-one");
        assert_eq!(tokens.len(), 1);

        // Case / whitespace insensitive: the same station overwrites.
        tokens.set("  https://A.Example.com  ", " pat-two ");
        assert_eq!(tokens.get("https://a.example.com"), "pat-two");
        assert_eq!(tokens.len(), 1, "同一站点不应产生第二条");

        // Empty string = delete
        tokens.set("https://a.example.com", "   ");
        assert!(!tokens.has("https://a.example.com"));
        assert!(tokens.is_empty());
    }

    #[test]
    fn remove_drops_only_the_named_station() {
        let mut tokens = StationTokens::default();
        tokens.set("https://a.example.com", "one");
        tokens.set("https://b.example.com", "two");
        tokens.remove("HTTPS://A.EXAMPLE.COM");
        assert!(!tokens.has("https://a.example.com"));
        assert!(tokens.has("https://b.example.com"), "别的站点不受影响");
        assert_eq!(tokens.len(), 1);
    }

    #[test]
    fn broken_input_falls_back_to_empty_table() {
        for text in [
            "",
            "not json",
            "{}",
            "[]",
            r#"{"tokens":[]}"#,
            r#"{"tokens":"x"}"#,
        ] {
            let tokens = StationTokens::parse(text);
            assert!(tokens.is_empty(), "{text}");
        }
        // Non-string / empty values: skip
        let tokens = StationTokens::parse(
            r#"{"version":1,"tokens":{"https://a.example.com":42,
                "https://b.example.com":"","https://c.example.com":"ok"}}"#,
        );
        assert_eq!(tokens.len(), 1, "只保留有效条目");
        assert_eq!(tokens.get("https://c.example.com"), "ok");
    }

    #[test]
    fn json_round_trip_keeps_entries_and_is_stable() {
        let mut tokens = StationTokens::default();
        tokens.set("https://b.example.com", "two");
        tokens.set("https://a.example.com", "one");
        let text = tokens.to_json();
        let back = StationTokens::parse(&text);
        assert_eq!(back, tokens);
        // Stable key order: re-serializing must be byte-identical.
        assert_eq!(back.to_json(), text);
        assert!(text.contains(r#""version": 3"#), "{text}");
        // Empty sections are written even without user IDs / station kinds.
        assert!(text.contains(r#""user_ids": {}"#), "{text}");
        assert!(text.contains(r#""site_kinds": {}"#), "{text}");
    }

    #[test]
    fn site_kind_sub2api_round_trips_and_defaults_to_newapi() {
        let mut tokens = StationTokens::default();
        assert!(!tokens.is_sub2api("https://a.example.com"), "缺省是 NewAPI");

        tokens.set_site_kind("  https://A.Example.com  ", true);
        assert!(tokens.is_sub2api("https://a.example.com"), "大小写不敏感");
        // Preserved after flush-and-reread.
        let back = StationTokens::parse(&tokens.to_json());
        assert!(back.is_sub2api("https://a.example.com"));

        // Restoring the default = clearing the marker; the station leaves the file.
        tokens.set_site_kind("https://a.example.com", false);
        assert!(!tokens.is_sub2api("https://a.example.com"));
        assert!(
            !tokens.to_json().contains("a.example.com"),
            "不应残留空条目"
        );

        // Unknown marker values and empty keys are never collected.
        let junk = r#"{"site_kinds":{"https://a.example.com":"openai","https://b.example.com":"sub2api","":"sub2api"}}"#;
        let parsed = StationTokens::parse(junk);
        assert!(!parsed.is_sub2api("https://a.example.com"));
        assert!(parsed.is_sub2api("https://b.example.com"));
    }

    #[test]
    fn removing_a_station_also_clears_its_site_kind() {
        let mut tokens = StationTokens::default();
        tokens.set("https://a.example.com", "sk-x");
        tokens.set_site_kind("https://a.example.com", true);
        tokens.remove("https://a.example.com");
        assert!(!tokens.is_sub2api("https://a.example.com"));
        assert!(!tokens.has("https://a.example.com"));
    }

    #[test]
    fn save_and_reload_from_disk_leaves_no_temp() {
        let dir = scratch_dir("save");
        let path = dir.join(FILE_NAME);
        let mut tokens = StationTokens::default();
        tokens.set("https://a.example.com", "pat-value");
        tokens.save_to(&path).expect("写入应成功");

        let reloaded = StationTokens::parse(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(reloaded, tokens);
        assert!(
            !path.with_file_name(format!("{FILE_NAME}.tmp")).exists(),
            "成功替换后不得残留临时文件"
        );
        assert!(
            !path
                .with_file_name(format!("{FILE_NAME}.replace-old"))
                .exists(),
            "成功替换后不得残留备份文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_failure_keeps_previous_file_and_hides_content() {
        let dir = scratch_dir("fail");
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "previous").expect("写旧内容");
        // Hold the temp-file path with a directory.
        std::fs::create_dir(path.with_file_name(format!("{FILE_NAME}.tmp"))).expect("占住临时路径");

        let mut tokens = StationTokens::default();
        tokens.set("https://a.example.com", "super-secret-pat");
        let err = tokens.save_to(&path).expect_err("无法创建临时文件时应失败");

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous");
        assert!(
            err.contains(&path.display().to_string()),
            "错误要带路径：{err}"
        );
        assert!(
            !err.contains("super-secret-pat"),
            "错误信息不得泄露令牌：{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_path_sits_next_to_settings_and_outside_agent_dirs() {
        let path = StationTokens::path();
        assert_eq!(path.file_name().unwrap_or_default(), FILE_NAME);
        assert_eq!(
            path.parent(),
            Some(crate::prefs::Prefs::path().parent().unwrap())
        );
        let text = path.to_string_lossy().to_lowercase();
        for forbidden in [".config", ".pi", ".omp", ".dsh", "opencode"] {
            assert!(!text.contains(forbidden), "不应写进 agent 配置：{text}");
        }
    }
}
