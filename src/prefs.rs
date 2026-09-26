//! 工具自身设置持久化：家目录下的 `.modelharbor/settings.json`
//! （Windows：`C:\Users\<用户名>\.modelharbor\settings.json`）。
//!
//! 只存**界面选择**（密钥显隐、保存格式、主题、同步 WSL、卡片折叠、各页配置路径覆盖）
//! —— 配置内容永远以用户自己的 agent 配置文件为真源，这里一个字段都不存：
//! 不存密钥、不存模型、不存配置内容。叫 settings 而不是 config，正是为了不和
//! 那些「配置文件」混淆。
//!
//! 文件不存在 / 读不出 / 解析失败都用默认值（界面偏好坏了不该影响工具可用性）。
//! 空字符串表示「没设置过」，由调用方回落到自己的默认值。
//!
//! 兼容：早期版本叫 `prefs.json`（先在家目录、更早还在 `%APPDATA%`）。
//! 读取时按「新名字 → 旧名字」「家目录 → %APPDATA%」逐个回退，写盘只写新位置的新名字，
//! 并在首次写入后清掉同目录的旧文件。

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// 读取 `root.a.b` 形式的嵌套字符串（类型不符视为空）。
fn nested_str(root: &Value, outer: &str, inner: &str) -> String {
    root.get(outer)
        .and_then(|value| value.get(inner))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// 配置目录名（家目录下的点目录，与任何 agent 配置目录无关）。
pub const DIR_NAME: &str = ".modelharbor";
const FILE_NAME: &str = "settings.json";
/// 旧文件名（早期版本）：只用于读取时回退与写入后清理。
const LEGACY_FILE_NAME: &str = "prefs.json";
/// 崩溃日志文件名。
const CRASH_FILE_NAME: &str = "crash.log";

/// 家目录：Windows 用 `USERPROFILE`，其他平台用 `HOME`。
pub fn home_dir() -> Option<PathBuf> {
    ["USERPROFILE", "HOME"]
        .into_iter()
        .find_map(|key| std::env::var_os(key).map(PathBuf::from))
        .filter(|dir| !dir.as_os_str().is_empty())
}

/// 配置目录（家目录下的 `.modelharbor`）：设置、凭证与崩溃日志都放这里。
pub fn config_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(DIR_NAME))
}

/// 崩溃日志路径（`<配置目录>/crash.log`）。
pub fn crash_log_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(CRASH_FILE_NAME))
}

/// 把一段记录追加到指定文件：父目录不存在则创建，写失败返回 `false`。
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

/// 把一段崩溃记录追加到 `crash.log`。
///
/// 返回是否写入成功，供调用方与测试判断；调用它本身绝不会 panic。
pub fn append_crash_log(entry: &str) -> bool {
    crash_log_path().is_some_and(|path| append_line(&path, entry))
}

/// 旧位置（`%APPDATA%` 下同名目录）：早期版本放在这里。
fn legacy_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join(DIR_NAME))
}

/// 读取候选路径，按优先级排列：
/// 家目录 settings.json → 家目录 prefs.json → %APPDATA% 下两者。
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

/// 写入后清掉同目录下的旧文件名（一次性迁移收尾；失败不影响本次保存）。
fn remove_legacy_next_to(path: &Path) {
    let Some(dir) = path.parent() else {
        return;
    };
    let legacy = dir.join(LEGACY_FILE_NAME);
    if legacy != path && legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
    }
}

/// 写盘用的 schema 版本（仅供人工核对 / 将来迁移，读取时忽略）。
const SCHEMA_VERSION: u64 = 12;

/// 配置身份：规范化路径后做稳定 FNV-1a 哈希，避免把用户目录明文写进设置键。
pub fn config_identity(path: &str) -> String {
    let normalized = path.trim().replace('\\', "/").to_lowercase();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in normalized.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 折叠状态的持久化键：`配置身份/类别/名字`。
pub fn collapsed_id(config_id: &str, kind: &str, key: &str) -> String {
    format!("{config_id}/{kind}/{key}")
}

/// v2 折叠键：`类别/名字`（仅供首次成功加载时迁移）。
pub fn legacy_collapsed_id(kind: &str, key: &str) -> String {
    format!("{kind}/{key}")
}

/// 各后端的配置路径覆盖（空字符串 = 不覆盖，用启动时自动探测到的默认路径）。
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
}

impl ConfigPathPrefs {
    /// 按后端取值。
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
        }
    }

    /// 按后端写入（空串 = 清除覆盖）。
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
        };
        *slot = path.to_string();
    }
}

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
    /// 各后端自己的配置路径覆盖（用户手动指定过才非空）。
    pub config_paths: ConfigPathPrefs,
    /// 已折叠的卡片（`配置身份/类别/名字`；不在表里的即展开）。
    /// v2 的 `类别/名字` 旧键会在首次成功加载配置后迁入当前配置身份。
    pub collapsed: Vec<String>,
    /// 检测到系统代理 / VPN 时是否仍允许「模型延迟测试」。
    ///
    /// 中转站普遍有多 IP 检测 / 测活风控，默认 `false` 保持拦截；
    /// 用户明确知道风险时可以打开（会影响延迟测试，不影响连通性测试与用量查询）。
    pub allow_model_test_with_proxy: bool,
    /// 是否已经关掉首次使用引导条。
    pub guide_dismissed: bool,
    /// 界面形状标识（`soft` / `compact` / `slab` / `sharp` / `panel` / `pill`；空 = 用默认档）。
    pub ui_style: String,
    /// 顶栏已安装页面的拖动顺序（后端标识；未列出的按名字首字母补在其后）。
    /// 只对已安装的那一组生效——未安装的页面始终排在后面并按字母序。
    pub tab_order: Vec<String>,
}

impl Prefs {
    /// 工具设置目录（家目录下的 `.modelharbor`）：自有文件都集中在这里
    /// （`settings.json` 与站点令牌 `tokens.json`），不写进任何 agent 配置目录。
    ///
    /// 取不到家目录时回退 `%APPDATA%\.modelharbor`，再回退程序同级目录。
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

    /// 设置文件路径（家目录下的 `.modelharbor/settings.json`）。
    pub fn path() -> PathBuf {
        Self::config_dir().join(FILE_NAME)
    }

    /// 从默认路径加载（全读不到即默认值）。
    ///
    /// 按 [`read_candidates`] 顺序回退：新位置没有就读旧文件名、再读旧的
    /// `%APPDATA%` 位置（一次性迁移：老设置不丢，下次写盘自动落到新位置）。
    pub fn load() -> Prefs {
        for candidate in read_candidates() {
            if let Ok(text) = std::fs::read_to_string(&candidate) {
                return Self::parse(&text);
            }
        }
        Prefs::default()
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
        // 字符串数组字段（折叠表 / 签到授权）读取方式一致，提一个闭包。
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
            },
            collapsed: get_list("collapsed"),
            // 只有真正的布尔 true 才放行：缺字段、字符串 "true" 都按拦截处理。
            allow_model_test_with_proxy: root
                .get("allow_model_test_with_proxy")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            // 缺字段 = 没关过 = 显示引导。
            guide_dismissed: root
                .get("guide_dismissed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            ui_style: get_str("ui_style"),
            tab_order: get_list("tab_order"),
        }
    }

    /// 序列化（固定字段顺序，便于人工核对 / diff）。
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
        ] {
            paths.insert(key.to_string(), Value::String(value.clone()));
        }
        root.insert("config_paths".to_string(), Value::Object(paths));
        // 两个字符串数组都排序去重后写出：内容一样就不产生 diff（避免写盘噪声）。
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
        // 顶栏顺序按用户拖动结果原样写出：这里**不能**排序，
        // 顺序本身就是这个键的内容（与 collapsed 的排序去重不同）。
        root.insert(
            "tab_order".to_string(),
            Value::Array(
                self.tab_order
                    .iter()
                    .map(|k| Value::String(k.clone()))
                    .collect(),
            ),
        );
        serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".to_string())
    }

    /// 落盘到默认路径，并清掉同目录下的旧文件名（一次性迁移收尾）。
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        let result = self.save_to(&path);
        if result.is_ok() {
            remove_legacy_next_to(&path);
        }
        result
    }

    /// 落盘到指定路径（单测用）。
    ///
    /// 原子写：先写同目录临时文件并同步，再替换正式文件；替换失败保留原设置
    ///（实现见 [`crate::util::atomic_write_text`]，与站点令牌文件共用同一套策略）。
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
            },
            // 按字母序给出：to_json 会排序写出，因此往返应完全相等
            collapsed: vec![
                collapsed_id("cfg", "agents", "build"),
                collapsed_id("cfg", "providers", "openai"),
            ],
            allow_model_test_with_proxy: true,
            guide_dismissed: true,
            ui_style: "slab".to_string(),
            tab_order: vec!["pi".to_string(), "zcode".to_string()],
        };
        assert_eq!(Prefs::parse(&prefs.to_json()), prefs);
    }

    /// 顶栏顺序按拖动结果原样写出：排序会毁掉这个键的内容。
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
        // 再次写出应完全相同（内容一致 → 不产生 diff）
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
        // 写出的版本号跟当前 schema 走（这里不写死数字，避免每次升版都要改测试）。
        assert!(
            prefs
                .to_json()
                .contains(&format!(r#""version": {SCHEMA_VERSION}"#)),
            "{}",
            prefs.to_json()
        );
    }

    #[test]
    fn proxy_model_test_override_defaults_off_and_round_trips() {
        // 默认必须是拦截：老设置文件里没有这个键时，绝不能变成“默认放行”。
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
        // 前两个候选必须是「家目录的新名字 → 家目录的旧名字」
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
        // 有家目录时必须在家里，而不是 %APPDATA%
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
        // 缺字段 = 没关过 = 显示引导。
        assert!(!Prefs::default().guide_dismissed);
        assert!(!Prefs::parse(r#"{"version":6}"#).guide_dismissed);
        // 字符串 "true" 不算关闭（与代理开关同一口径：只认真布尔）。
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
        // 空字符串 = 用默认档（与 `theme` / `save_format` 同一口径）。
        assert_eq!(Prefs::default().ui_style, "");
        assert_eq!(Prefs::parse(r#"{"ui_style":"slab"}"#).ui_style, "slab");
        // 类型不对按没设置过处理。
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
