use crate::backends;
use crate::prefs::ConfigPathPrefs;
use crate::util::{is_wsl_path, read_wsl_file};
use std::path::Path;

/// 配置格式标识。新增后端时在 `backends` 模块实现并注册，这里加变体。
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
}

impl ConfigFormat {
    pub fn label(&self) -> &str {
        match self {
            ConfigFormat::Opencode => "opencode",
            ConfigFormat::Kilocode => "kilocode",
            ConfigFormat::Mimocode => "mimocode",
            ConfigFormat::Pi => "pi",
            ConfigFormat::OhMyPi => "oh-my-pi",
            ConfigFormat::DeepSeekHarness => "deepseek-harness",
            ConfigFormat::ZCode => "zcode",
            ConfigFormat::WorkBuddy => "workbuddy",
        }
    }

    /// 是否属于 **opencode 系**：opencode / kilocode / mimocode。
    ///
    /// 三者是同一份代码的后代（Kilo Code 与 MiMo Code 都是 opencode 的 fork），配置
    /// schema 的**字段与结构一致**：顶层 `provider` map + `agent` map，provider 用
    /// `options.baseURL` / `options.apiKey`，模型用 `models.<id>.limit.context|output`
    /// / `tool_call` / `reasoning`。差别只有**配置目录名、主配置文件名、图标**。
    ///
    /// 注意「字段相同」不等于「required 相同」：mimocode 额外要求 `modalities` 成对
    /// （见 `backends::opencode::complete_required_model_fields`），写盘时要按目标方言补齐。
    ///
    /// 所以解析、序列化、字段可见性、agents 支持全部共用一套实现；也正因为内容形状
    /// 一致，**判别只能靠路径**（目录名或文件名），不能靠内容特征。
    pub fn is_opencode_family(&self) -> bool {
        matches!(
            self,
            ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode
        )
    }

    /// 该后端是否有**模型级启用开关**——目前只有 WorkBuddy 一家。
    ///
    /// WorkBuddy 的模型写 `disabled`，且它的选择器按裸 id **全局去重**：同一个模型名
    /// 只能有一条生效，所以「关掉其余同名条目」是它独有的语义，界面必须能表达。
    /// 其余七家的模型 schema 里没有这个字段（ZCode 的 `config.enabled` 由它自己的
    /// 界面维护，ModelHarbor 只负责原样保留，不接管），凭空加一个只会被当成未知键。
    ///
    /// 不能改用 `page_has_model_field("disabled")` 判定：那个函数在「已加载的文件格式
    /// 与当前页不同」时一律返回 true（为了让新页面能填所有字段），会把开关漏到每一页。
    pub fn has_model_enable(&self) -> bool {
        matches!(self, ConfigFormat::WorkBuddy)
    }
}

/// 各后端的解析后路径容器（含用户可覆盖的本地路径）。
pub struct ConfigPaths {
    pub opencode: String,
    pub kilocode: String,
    pub mimocode: String,
    pub pi: String,
    pub oh_my_pi: String,
    pub deepseek_harness: String,
    pub zcode: String,
    pub workbuddy: String,
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
        }
    }
}

impl ConfigPaths {
    /// 本后端实际使用的本地路径。
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
        }
    }

    /// 写入某一后端的本地路径（用户手动覆盖 / 恢复默认都走这里）。
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
            // 覆盖路径也要走候选解析：用户填 `.json`、盘上是 `.jsonc` 时，
            // 这里若按原样判存在，覆盖就永远命中不了，表现为「填了路径却不打开」。
            let path = backends::resolve_local_path(id, &self.local_path(id));
            if Path::new(&path).exists() {
                return Some((id, path));
            }
        }
        Self::detect()
    }

    /// 启动探测：找到第一个本地存在的默认配置。
    ///
    /// 路径走候选解析（opencode 系的 `.json` / `.jsonc`）：只认默认名的话，
    /// CLI 首次运行生成 `.jsonc` 的机器会一个都探测不到，启动就落在空配置上。
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

    /// 页面在顶栏的显示顺序：**已安装在前、未安装在后**，各组内按名字首字母。
    ///
    /// `saved_order` 是用户拖动过的顺序（后端标识，`ConfigFormat::label()` 的值）：
    /// 只对已安装的那一组生效——未安装的排在哪里是推导出来的，不该被手动顺序干扰。
    /// 拖动后新装了一个 agent 也不会打乱：它按字母序插进已安装组。
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
        // 已安装组：先按用户拖动顺序，其余按名字首字母补在其后。
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

    /// 本地优先：本地文件存在时写本地，否则回落 WSL，最后回退本地默认路径（新建场景）。
    pub fn target_path(&self, format: ConfigFormat) -> String {
        backends::target_path(format, &self.local_path(format))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::ConfigPathPrefs;

    /// 顶栏顺序：已安装在前（按字母）、未安装在后（按字母）。
    #[test]
    fn tab_order_puts_installed_first_then_alphabetical() {
        let paths = ConfigPaths::default();
        // 假装只有 pi / zcode / workbuddy 装了
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
                "mimocode",
                "oh-my-pi",
                "opencode"
            ],
            "已安装的按字母在前，未安装的按字母在后"
        );
    }

    /// 保存的拖动顺序只作用于已安装的那一段。
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
        // 未安装的仍按字母序跟在后面，不受 saved 影响
        assert_eq!(
            &labels[2..],
            &[
                "deepseek-harness",
                "kilocode",
                "mimocode",
                "oh-my-pi",
                "opencode",
                "workbuddy"
            ],
        );
    }

    /// 拖动顺序里出现了当前未安装的项时不该出错：它被忽略，等装了再排进去。
    #[test]
    fn tab_order_ignores_saved_entries_that_are_not_installed() {
        let paths = ConfigPaths::default();
        let saved = vec![
            "workbuddy".to_string(), // 没装
            "zcode".to_string(),
        ];
        let order = paths.tab_order(&saved, |id| {
            matches!(id, ConfigFormat::ZCode | ConfigFormat::Pi)
        });
        let labels: Vec<&str> = order.iter().map(|id| id.label()).collect();
        assert_eq!(labels[0], "zcode", "未安装的项被跳过，不影响已安装的排序");
        assert_eq!(labels[1], "pi", "剩下的按字母补在后面");
    }

    /// 新装一个 agent：按字母插进已安装段，不打断已有顺序。
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

    /// 一个都没装时全按字母序。
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
                "mimocode",
                "oh-my-pi",
                "opencode",
                "pi",
                "workbuddy",
                "zcode"
            ],
        );
    }

    /// 顺序必须是全部后端的一个排列：漏一个就等于顶栏少一个页面。
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
