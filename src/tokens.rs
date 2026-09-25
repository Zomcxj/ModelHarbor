//! 站点面板令牌（PAT）持久化：家目录 `.modelharbor/tokens.json`。
//!
//! 与 `settings.json` **分开存放**：那个文件对外承诺「一个密钥都不存」，
//! 这里只放面板访问令牌；两者都放在 [[`crate::prefs::Prefs::config_dir`]] 目录下，
//! 删掉本文件即清空全部令牌。
//!
//! 令牌是**站点 / 账号级**的，不是 provider 级：同一个中转站的多个 provider
//! 共用同一份 PAT。因此键取**规范化 origin**（`scheme://host`，小写、无末尾斜杠），
//! 直接复用 [`crate::billing::endpoints`] 的 origin 推导，避免出现第二套 URL 口径。
//!
//! 文件含凭证，因此：不进 `settings.json`、不进 agent 配置文件、
//! 不进日志 / 状态栏 / 错误文本；错误信息只带路径不带正文。

use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "tokens.json";
/// 写盘用的 schema 版本（仅供人工核对 / 将来迁移，读取时忽略）。
const SCHEMA_VERSION: u64 = 2;

/// 站点令牌表（键 = 规范化 origin，值 = PAT）。
///
/// 用 `BTreeMap` 保证写出顺序稳定：内容没变就不产生 diff。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StationTokens {
    tokens: BTreeMap<String, String>,
    /// 旧版 new-api 要求的用户 ID（`New-Api-User` 头），与 `tokens` 同键。
    /// 新版不需要，所以大多数站点这里是空的。
    user_ids: BTreeMap<String, String>,
}

/// 站点身份：把 baseUrl 归一到「站点根」，作为令牌表的键。
///
/// 推导复用 [`crate::billing::endpoints`]（`https://host/v1` → `https://host`），
/// 再统一小写（主机名大小写不敏感，避免同一个站点存成两条）。
pub fn station_key(base_url: &str) -> String {
    crate::billing::endpoints(base_url).origin.to_lowercase()
}

impl StationTokens {
    /// 令牌文件路径（与设置文件同目录）。
    pub fn path() -> PathBuf {
        crate::prefs::Prefs::config_dir().join(FILE_NAME)
    }

    /// 从默认路径加载；文件不存在 / 读不出 / 解析失败都返回空表。
    pub fn load() -> StationTokens {
        let path = Self::path();
        std::fs::read_to_string(&path)
            .ok()
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// 解析已有内容（供加载与单测使用）；结构不符一律回落空表。
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
                        // 只收非空字符串：空值等于没设置过，留着只会误导。
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
        // 用户 ID：键不存在（v1 老文件）就是空表，不影响令牌读取。
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
        StationTokens { tokens, user_ids }
    }

    /// 序列化（键序固定，便于人工核对 / diff）。
    pub fn to_json(&self) -> String {
        let mut root = Map::new();
        root.insert("version".to_string(), Value::Number(SCHEMA_VERSION.into()));
        let mut tokens = Map::new();
        for (key, token) in &self.tokens {
            tokens.insert(key.clone(), Value::String(token.clone()));
        }
        root.insert("tokens".to_string(), Value::Object(tokens));
        // 用户 ID 单独一段，与令牌平行：老版本读这个文件时忽略未知键，
        // 所以不需要改已有令牌的存储形态（也不需要迁移）。
        let mut user_ids = Map::new();
        for (key, id) in &self.user_ids {
            user_ids.insert(key.clone(), Value::String(id.clone()));
        }
        root.insert("user_ids".to_string(), Value::Object(user_ids));
        serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".to_string())
    }

    /// 落盘到默认路径。
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    /// 落盘到指定路径（单测用）。原子写策略与设置文件共用。
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        crate::util::atomic_write_text(path, &self.to_json())
    }

    /// 取某站点的令牌（没有则空串）。
    pub fn get(&self, origin: &str) -> &str {
        self.tokens.get(origin).map(String::as_str).unwrap_or("")
    }

    /// 是否已为该站点设置令牌。
    pub fn has(&self, origin: &str) -> bool {
        !self.get(origin).is_empty()
    }

    /// 取某站点的用户 ID（旧版 new-api 的 `New-Api-User` 头，没有则空串）。
    ///
    /// 只在站点回了「缺 `New-Api-User`」时才需要填；新版不需要它。
    pub fn user_id(&self, origin: &str) -> &str {
        self.user_ids
            .get(origin.trim().to_lowercase().as_str())
            .map(String::as_str)
            .unwrap_or("")
    }

    /// 写入用户 ID；空串（或纯空白）等于删除该站点这一项。
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

    /// 写入令牌；空串（或纯空白）等于删除该站点条目。
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

    /// 删除某站点的令牌（连同用户 ID：两者同属一个站点的凭证）。
    pub fn remove(&mut self, origin: &str) {
        let origin = origin.trim().to_lowercase();
        self.tokens.remove(&origin);
        self.user_ids.remove(&origin);
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
        // 同一个站点：带路径、末尾斜杠、大小写不同，都必须归到同一个键。
        assert_eq!(
            station_key("https://gemai.huchan.cn/v1"),
            "https://gemai.huchan.cn"
        );
        assert_eq!(
            station_key("https://Gemai.Huchan.CN/v1/"),
            station_key("https://gemai.huchan.cn")
        );
        // 不同站点必须分开，不能被合并成一个键。
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
        // 空串 / 纯空白 = 删除（与 set 对令牌的语义一致）。
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

        // v1 老文件（没有 user_ids 键）必须照常读出令牌，用户 ID 为空。
        let legacy = r#"{"version":1,"tokens":{"https://a.example.com":"old-pat"}}"#;
        let old = StationTokens::parse(legacy);
        assert_eq!(old.get("https://a.example.com"), "old-pat");
        assert_eq!(old.user_id("https://a.example.com"), "");
        // 非字符串 / 空串的 user_id 一律不收。
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

        // 大小写 / 空白不敏感：同一站点覆盖而不是新增一条。
        tokens.set("  https://A.Example.com  ", " pat-two ");
        assert_eq!(tokens.get("https://a.example.com"), "pat-two");
        assert_eq!(tokens.len(), 1, "同一站点不应产生第二条");

        // 空串 = 删除
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
        // 值不是字符串 / 空值：跳过而不是崩掉
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
        // 键序稳定：再次序列化必须完全一致（内容没变就不产生 diff）
        assert_eq!(back.to_json(), text);
        assert!(text.contains(r#""version": 2"#), "{text}");
        // 没有用户 ID 时也写出空段：结构固定才好人工核对 / diff。
        assert!(text.contains(r#""user_ids": {}"#), "{text}");
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
        // 用目录占住临时文件路径，让原子写无法创建临时文件。
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
