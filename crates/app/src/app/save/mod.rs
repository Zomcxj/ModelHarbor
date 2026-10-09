//! 保存 / 预览文档的路径解析、校验、写盘与各后端加载入口。

mod checks;
mod fields;
mod strip;

pub use strip::strip_cross_format_containers;

use super::{App, SaveFormat};
use crate::backends;
use crate::credentials;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{self, is_wsl_path};
use serde_json::{Map, Value};

/// 一个保存目标的运行时状态（可用性 / 解析路径 / 勾选）。
pub(super) struct SaveTarget {
    pub(super) backend: ConfigFormat,
    pub(super) available: bool,
    pub(super) path: String,
}

/// 本页写入路径的解析结果。
#[derive(Clone)]
pub(super) enum PageTarget {
    /// 当前文件（已加载）：整体替换 agent / provider。
    Current(String),
    /// 路径已修改但未重新加载：按“先读后合并”写入。
    Modified(String),
    /// 该后端默认目标（Windows 本地路径；WSL 仅在勾选「WSL同步」后写入）。
    Default(String),
}

/// 跨格式转换前，从目标 root 中剔除由界面接管的容器：
/// provider（opencode 的 provider / pi 系与 DSH 的 providers）与 opencode 的 agent。
///
/// 界面不显示、也无从重建的只读内容（Kimi 的 `managed:*`、Qwen 的 `qwen-oauth`）保留，
/// 其余顶层字段原样保留。删除用 `shift_remove` 保持其余键的相对顺序。
/// `agents_owned` 为假时保留目标文件里的 agent 容器。
///
/// 加载 opencode 配置；读取/解析失败返回 Err。
pub fn load_opencode_result(
    path: &str,
) -> Result<(Value, Vec<AgentRow>, Vec<ProviderRow>), String> {
    let load = crate::backends::load_backend(ConfigFormat::Opencode, path)?;
    Ok((load.root, load.agents, load.providers))
}

/// 兼容包装：失败时回退空状态。
pub fn load_or_empty(path: &str) -> (Value, Vec<AgentRow>, Vec<ProviderRow>) {
    load_opencode_result(path)
        .unwrap_or_else(|_| (Value::Object(Map::new()), Vec::new(), Vec::new()))
}

impl App {
    /// 本页写入路径：
    /// - 当前文件属于本页格式且已加载 → 当前文件（整体替换）；
    /// - 路径已修改但未加载 → 仍写该路径，但按“先读后合并”；
    /// - 其余 → 该后端默认目标（Windows 本地；WSL 需勾选「WSL同步」）。
    pub(super) fn page_save_path(&self, fmt: ConfigFormat) -> PageTarget {
        if !self.config_path.is_empty() && self.config_path != self.loaded_path {
            // 路径框中已明确指定目标文件时使用该路径，即使尚未点击“加载”。
            return PageTarget::Modified(self.config_path.clone());
        }
        if self.source_format == fmt && !self.config_path.is_empty() {
            return PageTarget::Current(self.config_path.clone());
        }
        let path = self
            .targets
            .iter()
            .find(|t| t.backend == fmt)
            .map(|t| t.path.clone())
            .unwrap_or_default();
        PageTarget::Default(path)
    }

    /// 保存当前页面：写入该页格式对应的路径，并按需同步 WSL。
    pub(super) fn save_page(&mut self, fmt: ConfigFormat) {
        self.status = self.save_page_report(fmt);
    }

    /// 一键保存：按页面顺序把所有**已安装**后端各写一遍。
    ///
    /// 每页结果拼进状态栏。当前页走 [`Self::page_save_path`]（含输入框里「已改未加载」
    /// 的路径），其他页面用各自的目标路径。未安装的后端跳过。
    pub(super) fn save_all(&mut self) {
        if let Some(dup) = self.find_duplicate_keys() {
            self.status = format!("key 重复: {}，已取消保存", dup);
            return;
        }
        let current = self.current_page;
        let mut parts: Vec<String> = Vec::new();
        for i in 0..self.targets.len() {
            let (fmt, available, path) = {
                let t = &self.targets[i];
                (t.backend, t.available, t.path.clone())
            };
            if fmt == current {
                parts.push(self.save_page_report(fmt));
            } else if available && !path.trim().is_empty() {
                parts.push(self.save_report_to(fmt, &path));
            }
        }
        self.status = if parts.is_empty() {
            "一键保存: 没有可写的目标（本地与 WSL 均未探测到已安装的配置）".to_string()
        } else {
            format!("一键保存: {}", parts.join(" | "))
        };
    }

    /// 保存一页并返回状态文本（不写 `self.status`，供单页保存与一键保存共用）。
    pub(super) fn save_page_report(&mut self, fmt: ConfigFormat) -> String {
        if let Some(dup) = self.find_duplicate_keys() {
            return format!("key 重复: {}，已取消保存", dup);
        }
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        match &target {
            PageTarget::Current(_) => {
                if let Some(err) = self.load_error.clone() {
                    return format!("当前文件: 加载失败({})，已跳过", err);
                }
            }
            PageTarget::Default(_) => {
                if !self.targets.iter().any(|t| t.backend == fmt && t.available) {
                    return format!("{}: 未安装（本地与 WSL 均未找到配置）", fmt.label());
                }
            }
            PageTarget::Modified(_) => {}
        }
        self.save_report_to(fmt, &path)
    }

    /// 按已解析好的路径写入一页并返回状态文本（含 WSL 同步）。
    fn save_report_to(&mut self, fmt: ConfigFormat, path: &str) -> String {
        let res = self.save_backend_to(fmt, path);
        let ok = res.is_ok();
        let mut status = match res {
            Ok(backup) => {
                let mut msg = format!("{}: 已保存", fmt.label());
                if let Some(backup) = backup {
                    msg.push_str(&format!("（原文件已备份为 {}）", backup));
                }
                msg
            }
            Err(e) => format!("{}: 保存失败({})", fmt.label(), e),
        };
        if ok {
            // 该格式不支持 agents 时在状态栏说明
            if !fmt.is_opencode_family() && !self.agents.is_empty() {
                status.push_str(&format!(
                    "（{} 个 agents 未写入：该格式不支持）",
                    self.agents.len()
                ));
            }
            let bad = self.count_invalid_numeric_fields();
            if bad > 0 {
                status.push_str(&format!("（已忽略 {} 个无效数字字段）", bad));
            }
        }
        // WSL 同步：勾选“WSL同步”且写入路径为本地时，同步到 WSL 侧默认路径。
        // 三态判定：「已安装」才写，「确认未安装」与「探测中」都跳过，文案分开。
        if self.sync_wsl && ok && !is_wsl_path(path) {
            match backends::wsl_target_state(fmt) {
                backends::WslTargetState::Installed(wsl_path) => {
                    match self.save_backend_to(fmt, &wsl_path) {
                        Ok(_) => status.push_str(&format!("; {}(WSL): 已同步", fmt.label())),
                        Err(e) => {
                            status.push_str(&format!("; {}(WSL): 同步失败({})", fmt.label(), e))
                        }
                    }
                }
                // 未安装 / 探测中都不写。
                other => status.push_str(&wsl_skip_note(fmt, &other)),
            }
        }
        status
    }

    /// 通用保存：按后端构造 root、渲染内容并写入。
    pub(super) fn save_backend_to(
        &mut self,
        fmt: ConfigFormat,
        path: &str,
    ) -> Result<Option<String>, String> {
        let backend = backends::backend(fmt);
        // 已加载的当前文件整体替换；同格式的其他路径（WSL 同步等）先读后合并；
        // 跨格式目标做干净转换：界面接管的 provider / agent 容器整体丢弃，其余顶层字段保留。
        let is_current = self.source_format == fmt && path == self.loaded_path;
        // WSL 同步是当前页面配置的完整镜像，不做保守 upsert。
        let is_opencode_wsl_sync =
            fmt.is_opencode_family() && self.source_format == fmt && is_wsl_path(path);
        let cross_format = self.source_format != fmt;
        // 写往别的页面时，agent 的 `model` 先按目标页网关归一；写当前页则原样落盘。
        let agents = if cross_format {
            self.agents_for_page(fmt)
        } else {
            self.agents.clone()
        };
        let target_root: Option<Value> = if is_current || is_opencode_wsl_sync {
            None
        } else {
            // 目标文件存在但读不出 / 解析不了 → 取消保存。
            let mut target = backend
                .load_target_root(path)
                .map_err(|e| format!("目标文件无法读取，已取消保存（未动原文件）: {e}"))?;
            if cross_format {
                strip_cross_format_containers(fmt, &mut target, !agents.is_empty());
            }
            Some(target)
        };
        let root = backend.serialize_root(
            &agents,
            &self.providers,
            self.extras_for(fmt),
            target_root.as_ref(),
        );
        let content = if fmt == ConfigFormat::DeepSeekHarness && is_current {
            // 未发生结构化修改时保留原始 YAML；有修改则用 DSH 渲染器。
            match util::read_config_content(path) {
                Ok(original)
                    if util::parse_yaml_content(&original).ok().as_ref() == Some(&root) =>
                {
                    original
                }
                _ => backend.render(&root, self.save_format == SaveFormat::Compact)?,
            }
        } else {
            backend.render(&root, self.save_format == SaveFormat::Compact)?
        };
        let dsh_sidecar_backup = if fmt == ConfigFormat::DeepSeekHarness {
            let sidecar = credentials::sidecar_path(path);
            if util::config_exists(&sidecar) {
                Some((sidecar.clone(), util::read_config_content(&sidecar)?))
            } else {
                Some((sidecar, String::new()))
            }
        } else {
            None
        };
        // WorkBuddy / QwenCode / KimiCode 保存后条目可能变少，需要备份。
        // 判据：WorkBuddy 按裸 id 去重后条目变少（根是数组）；QwenCode 与 KimiCode 问各自后端。
        let shrinks = is_current
            && match fmt {
                ConfigFormat::WorkBuddy => util::read_config_content(path)
                    .ok()
                    .and_then(|old| util::parse_config_content(&old).ok())
                    .map(|old_root| {
                        let before = old_root.as_array().map(Vec::len).unwrap_or(0);
                        let after = root.as_array().map(Vec::len).unwrap_or(0);
                        before > after
                    })
                    // 读不出 / 解析不了旧文件时按「会收缩」处理，交由下面的备份分支读原文件。
                    .unwrap_or(true),
                ConfigFormat::QwenCode => util::read_config_content(path)
                    .ok()
                    .and_then(|old| util::parse_config_content(&old).ok())
                    .map(|old_root| crate::backends::qwen_code::shrinks_on_save(&old_root, &root))
                    .unwrap_or(true),
                ConfigFormat::KimiCode => util::read_config_content(path)
                    .ok()
                    .and_then(|old| crate::backends::kimi_code::parse_root_for_shrink(&old))
                    .map(|old_root| crate::backends::kimi_code::shrinks_on_save(&old_root, &root))
                    .unwrap_or(true),
                _ => false,
            };
        // 跨格式转换、收缩式保存与 WSL 同步：先把原文件备份为 `.bak`；
        // 备份失败或原文件读不出则取消保存。
        let backup = if (cross_format && !is_current) || shrinks || is_opencode_wsl_sync {
            match util::read_config_content(path) {
                Ok(old) if !old.is_empty() && (old != content || is_opencode_wsl_sync) => {
                    let backup_path = format!("{}.bak", path);
                    backends::write_config(&backup_path, &old).map_err(|e| {
                        format!("保存前备份失败（{}），已取消保存: {}", backup_path, e)
                    })?;
                    Some(backup_path)
                }
                Ok(_) => None,
                Err(e) => return Err(format!("读取原文件失败（{path}），已取消保存: {e}")),
            }
        } else {
            None
        };
        // sidecar 一律在主配置之前写。这里同时是写盘前的总闸：
        // Kimi 挡凭据 XOR、`managed:` 保留前缀与别名撞名，Qwen 挡 envKey 变量名撞名。
        if matches!(
            fmt,
            ConfigFormat::DeepSeekHarness
                | ConfigFormat::WorkBuddy
                | ConfigFormat::KimiCode
                | ConfigFormat::QwenCode
        ) {
            backend.save_sidecars(path, &self.providers)?;
        }
        // 写盘前的自动快照（Auto tag）：磁盘内容与上次快照不同才备份（见 app::profiles）。
        self.auto_snapshot_before_save(path);
        if let Err(error) = backends::write_config(path, &content) {
            if let Some((sidecar, old_content)) = dsh_sidecar_backup {
                let restore = if old_content.is_empty() {
                    if util::config_exists(&sidecar) {
                        util::remove_config(&sidecar)
                    } else {
                        Ok(())
                    }
                } else {
                    backends::write_config(&sidecar, &old_content)
                };
                if let Err(restore_error) = restore {
                    return Err(format!("{}；凭据回滚失败: {}", error, restore_error));
                }
            }
            return Err(error);
        }
        // 当前文件保存成功后，回填 opencode 的 extras 载体（self.root）保持与磁盘一致
        if is_current && fmt.is_opencode_family() {
            self.root = root;
        }
        // 磁盘内容变了：对比视图的缓存作废。计数只增不减，回绕只多算一次差异。
        self.save_serial = self.save_serial.wrapping_add(1);
        // 记录刚写入内容的 hash（下次保存判定「外部改动」用），并按默认保留
        // 策略清理备份目录；两者失败都不影响保存结果。
        self.note_snapshot_hash(path);
        self.prune_backups_after_save();
        Ok(backup)
    }

    /// 当前文件保存时使用的基底 extras（按后端取对应载体）。
    pub(super) fn extras_for(&self, fmt: ConfigFormat) -> &Value {
        match fmt {
            // opencode 系（含 kilocode / mimocode）extras 就是完整 root。
            ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => &self.root,
            // pi 系（pi / oh-my-pi）共用 extras 载体：providers 之外的顶层字段
            ConfigFormat::Pi | ConfigFormat::OhMyPi => &self.pi_extras,
            // ZCode / WorkBuddy / QwenCode / KimiCode 与 opencode、DSH 一样，extras 就是完整 root。
            ConfigFormat::DeepSeekHarness
            | ConfigFormat::ZCode
            | ConfigFormat::WorkBuddy
            | ConfigFormat::QwenCode
            | ConfigFormat::KimiCode => &self.root,
        }
    }
}

/// WSL 同步被跳过时的状态栏补充文案（按三态区分）。
///
/// 「探测中」与「确认未安装」都不写入，措辞分开。传入 `Installed` 时退回通用文案。
fn wsl_skip_note(fmt: ConfigFormat, state: &backends::WslTargetState) -> String {
    match state {
        backends::WslTargetState::NotInstalled => {
            format!("; {}(WSL): 未安装，跳过同步", fmt.label())
        }
        // 探测未出结果：不写、不报「未安装」，明说在检测。
        backends::WslTargetState::Unknown => {
            format!("; {}(WSL): 正在检测安装状态，本次跳过同步", fmt.label())
        }
        backends::WslTargetState::Installed(_) => {
            format!("; {}(WSL): 未同步（内部状态异常）", fmt.label())
        }
    }
}

#[cfg(test)]
mod wsl_skip_note_tests {
    use super::*;

    /// 「探测中」与「未安装」的文案必须可区分。
    #[test]
    fn unknown_does_not_claim_not_installed() {
        let fmt = ConfigFormat::Opencode;
        let unknown = wsl_skip_note(fmt, &backends::WslTargetState::Unknown);
        let missing = wsl_skip_note(fmt, &backends::WslTargetState::NotInstalled);

        assert!(
            unknown.contains("正在检测"),
            "探测中应明说在检测：{unknown}"
        );
        assert!(
            !unknown.contains("未安装"),
            "探测中不得说成未安装（用户会以为真没装）：{unknown}"
        );
        assert!(missing.contains("未安装"), "确认未安装应直说：{missing}");
        assert_ne!(unknown, missing, "两种跳过的文案必须不同");
    }

    /// 文案带上后端名。
    #[test]
    fn note_names_the_backend() {
        let note = wsl_skip_note(ConfigFormat::KimiCode, &backends::WslTargetState::Unknown);
        assert!(note.contains(ConfigFormat::KimiCode.label()), "{note}");
    }
}
