//! Path resolution, validation, disk writes, and per-backend load entry points for saving / previewing documents.

mod checks;
mod fields;
mod strip;

pub use strip::strip_cross_format_containers;

use super::{App, SaveFormat, StatusLevel};
use crate::backends;
use crate::credentials;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{self, is_wsl_path};
use serde_json::{Map, Value};

/// Runtime state of one save target (availability / resolved path / checked flag).
pub(super) struct SaveTarget {
    pub(super) backend: ConfigFormat,
    pub(super) available: bool,
    pub(super) path: String,
}

/// Resolution result of this page's write path.
#[derive(Clone)]
pub(super) enum PageTarget {
    /// The current file (loaded): agents / providers replaced wholesale.
    Current(String),
    /// The path was edited but not reloaded: write with "read first, then merge".
    Modified(String),
    /// The backend's default target (a Windows local path; WSL writes only after checking "WSL sync").
    Default(String),
}

/// Before cross-format conversion, evict from the target root the containers the UI manages:
/// provider (opencode's provider / pi-family and DSH providers) and opencode's agent.
///
/// Read-only content the UI neither shows nor can rebuild (Kimi's `managed:*`, Qwen's `qwen-oauth`) is
/// kept; other top-level fields are preserved verbatim. Deletion uses `shift_remove` to keep the
/// relative order of remaining keys. With `agents_owned` false, the target file's agent container stays.
///
/// Load an opencode config; returns Err on read/parse failure.
pub fn load_opencode_result(
    path: &str,
) -> Result<(Value, Vec<AgentRow>, Vec<ProviderRow>), String> {
    let load = crate::backends::load_backend(ConfigFormat::Opencode, path)?;
    Ok((load.root, load.agents, load.providers))
}

/// Compatibility wrapper: falls back to an empty state on failure.
pub fn load_or_empty(path: &str) -> (Value, Vec<AgentRow>, Vec<ProviderRow>) {
    load_opencode_result(path)
        .unwrap_or_else(|_| (Value::Object(Map::new()), Vec::new(), Vec::new()))
}

impl App {
    /// This page's write path:
    /// - the current file belongs to this page's format and is loaded → the current file (wholesale replace);
    /// - the path was edited but not loaded → still that path, but with "read first, then merge";
    /// - otherwise → the backend's default target (Windows local; WSL requires "WSL sync").
    pub(super) fn page_save_path(&self, fmt: ConfigFormat) -> PageTarget {
        if !self.config_path.is_empty() && self.config_path != self.loaded_path {
            // When the path box explicitly names a target file, that path is used even before clicking "load".
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

    /// Save the current page: write to the page format's path and sync WSL as needed.
    pub(super) fn save_page(&mut self, fmt: ConfigFormat) {
        let (level, msg) = self.save_page_report(fmt);
        self.set_status_level(level, msg);
    }

    /// Save all: write every **installed** backend once, in page order.
    ///
    /// Each page's result joins the status bar. The current page goes through [`Self::page_save_path`]
    /// (including an "edited but not loaded" path from the input box); other pages use their own targets.
    /// Uninstalled backends skip.
    pub(super) fn save_all(&mut self) {
        if let Some(dup) = self.find_duplicate_keys() {
            self.set_status_err(format!("key 重复: {}，已取消保存", dup));
            return;
        }
        let current = self.current_page;
        let mut parts: Vec<String> = Vec::new();
        // The combined level is the worst of the parts: any failure paints the whole line red, otherwise a
        // write that actually happened reads as done.
        let mut level: Option<StatusLevel> = None;
        let mut merge = |part: StatusLevel| {
            level = Some(match (level, part) {
                (Some(StatusLevel::Err), _) | (_, StatusLevel::Err) => StatusLevel::Err,
                (Some(StatusLevel::Ok), _) | (_, StatusLevel::Ok) => StatusLevel::Ok,
                _ => StatusLevel::Info,
            });
        };
        for i in 0..self.targets.len() {
            let (fmt, available, path) = {
                let t = &self.targets[i];
                (t.backend, t.available, t.path.clone())
            };
            let (part_level, text) = if fmt == current {
                self.save_page_report(fmt)
            } else if available && !path.trim().is_empty() {
                self.save_report_to(fmt, &path)
            } else {
                continue;
            };
            merge(part_level);
            parts.push(text);
        }
        if parts.is_empty() {
            self.set_status("一键保存: 没有可写的目标（本地与 WSL 均未探测到已安装的配置）");
        } else {
            let text = format!("一键保存: {}", parts.join(" | "));
            self.set_status_level(level.unwrap_or(StatusLevel::Info), text);
        }
    }

    /// Save one page and return the status text with its level (doesn't write `self.status`; shared by
    /// single-page and save-all).
    pub(super) fn save_page_report(&mut self, fmt: ConfigFormat) -> (StatusLevel, String) {
        if let Some(dup) = self.find_duplicate_keys() {
            return (StatusLevel::Err, format!("key 重复: {}，已取消保存", dup));
        }
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        match &target {
            PageTarget::Current(_) => {
                if let Some(err) = self.load_error.clone() {
                    return (
                        StatusLevel::Err,
                        format!("当前文件: 加载失败({})，已跳过", err),
                    );
                }
            }
            PageTarget::Default(_) => {
                if !self.targets.iter().any(|t| t.backend == fmt && t.available) {
                    return (
                        StatusLevel::Info,
                        format!("{}: 未安装（本地与 WSL 均未找到配置）", fmt.label()),
                    );
                }
            }
            PageTarget::Modified(_) => {}
        }
        self.save_report_to(fmt, &path)
    }

    /// Write one page via an already-resolved path and return the status text with its level (WSL sync included).
    fn save_report_to(&mut self, fmt: ConfigFormat, path: &str) -> (StatusLevel, String) {
        let res = self.save_backend_to(fmt, path);
        let ok = res.is_ok();
        let (mut level, mut status) = match res {
            Ok(backup) => {
                let mut msg = format!("{}: 已保存", fmt.label());
                if let Some(backup) = backup {
                    msg.push_str(&format!("（原文件已备份为 {}）", backup));
                }
                (StatusLevel::Ok, msg)
            }
            Err(e) => (
                StatusLevel::Err,
                format!("{}: 保存失败({})", fmt.label(), e),
            ),
        };
        if ok {
            // State in the status bar when the format doesn't support agents
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
        // WSL sync: with "WSL sync" checked and a local write path, sync to the WSL-side default path.
        // Three-state gating: only "installed" writes; "confirmed missing" and "probing" both skip, with separate wording.
        if self.sync_wsl && ok && !is_wsl_path(path) {
            match backends::wsl_target_state(fmt) {
                backends::WslTargetState::Installed(wsl_path) => {
                    match self.save_backend_to(fmt, &wsl_path) {
                        Ok(_) => status.push_str(&format!("; {}(WSL): 已同步", fmt.label())),
                        Err(e) => {
                            status.push_str(&format!("; {}(WSL): 同步失败({})", fmt.label(), e));
                            // The local write landed but the WSL mirror did not: report the failure.
                            level = StatusLevel::Err;
                        }
                    }
                }
                // Neither installed nor probing writes.
                other => status.push_str(&wsl_skip_note(fmt, &other)),
            }
        }
        (level, status)
    }

    /// Generic save: build the root per backend, render, and write.
    pub(super) fn save_backend_to(
        &mut self,
        fmt: ConfigFormat,
        path: &str,
    ) -> Result<Option<String>, String> {
        let backend = backends::backend(fmt);
        // A loaded current file replaces wholesale; other same-format paths (WSL sync etc.) read first then merge;
        // cross-format targets do a clean conversion: UI-managed provider / agent containers are dropped whole,
        // other top-level fields are kept.
        let is_current = self.source_format == fmt && path == self.loaded_path;
        // WSL sync is a complete mirror of the current page's config — no conservative upsert.
        let is_opencode_wsl_sync =
            fmt.is_opencode_family() && self.source_format == fmt && is_wsl_path(path);
        let cross_format = self.source_format != fmt;
        // When writing to another page, the agent's `model` normalizes to the target page's gateway first; writing
        // the current page lands as-is.
        let agents = if cross_format {
            self.agents_for_page(fmt)
        } else {
            self.agents.clone()
        };
        let target_root: Option<Value> = if is_current || is_opencode_wsl_sync {
            None
        } else {
            // A target that exists but can't be read / parsed → cancel the save.
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
            // With no structured modification, keep the original YAML; with changes, use the DSH renderer.
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
        // WorkBuddy / QwenCode / KimiCode saves may shrink entries — a backup is needed.
        // Criterion: WorkBuddy's entry count shrinks after bare-id dedup (root is an array); QwenCode and
        // KimiCode ask their own backends.
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
                    // An unreadable / unparseable old file counts as "may shrink", letting the backup branch below read the original.
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
        // Cross-format conversion, shrinking saves, and WSL sync: back up the original as `.bak` first;
        // a failed backup or unreadable original cancels the save.
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
        // Sidecars always write before the main config. This doubles as the pre-write master gate:
        // Kimi blocks the credential XOR and `managed:` reserved-prefix vs alias collisions; Qwen blocks envKey variable-name collisions.
        if matches!(
            fmt,
            ConfigFormat::DeepSeekHarness
                | ConfigFormat::WorkBuddy
                | ConfigFormat::KimiCode
                | ConfigFormat::QwenCode
        ) {
            backend.save_sidecars(path, &self.providers)?;
        }
        // The pre-write auto-snapshot (Auto tag): back up only when disk content differs from the last snapshot (see save::auto_snapshot_before_save).
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
        // After a successful current-file save, backfill opencode's extras carrier (self.root) to stay consistent with disk
        if is_current && fmt.is_opencode_family() {
            self.root = root;
        }
        // Disk content changed: the diff view's cache is invalid. The count only grows; wraparound miscounts a diff once.
        self.save_serial = self.save_serial.wrapping_add(1);
        // Record the just-written content's hash (for the next save's "external change" verdict) and prune the
        // backup directory by the default retention policy; either failing doesn't affect the save outcome.
        self.note_snapshot_hash(path);
        self.prune_backups_after_save();
        Ok(backup)
    }

    /// The base extras used when saving the current file (the corresponding carrier per backend).
    pub(super) fn extras_for(&self, fmt: ConfigFormat) -> &Value {
        match fmt {
            // For the opencode family (kilocode / mimocode included), extras IS the complete root.
            ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => &self.root,
            // The pi family (pi / oh-my-pi) shares an extras carrier: top-level fields beyond providers
            ConfigFormat::Pi | ConfigFormat::OhMyPi => &self.pi_extras,
            // ZCode / WorkBuddy / QwenCode / KimiCode, like opencode and DSH: extras IS the complete root.
            ConfigFormat::DeepSeekHarness
            | ConfigFormat::ZCode
            | ConfigFormat::WorkBuddy
            | ConfigFormat::QwenCode
            | ConfigFormat::KimiCode => &self.root,
        }
    }
}

/// Status-bar supplement when WSL sync is skipped (distinct per three-state).
///
/// "Probing" and "confirmed missing" both write nothing, with separate wording. Passing `Installed`
/// falls back to the generic text.
fn wsl_skip_note(fmt: ConfigFormat, state: &backends::WslTargetState) -> String {
    match state {
        backends::WslTargetState::NotInstalled => {
            format!("; {}(WSL): 未安装，跳过同步", fmt.label())
        }
        // Probe without a result: write nothing, don't report "not installed" — say plainly that detection is running.
        backends::WslTargetState::Unknown => {
            format!("; {}(WSL): 正在检测安装状态，本次跳过同步", fmt.label())
        }
        backends::WslTargetState::Installed(_) => {
            format!("; {}(WSL): 未同步（内部状态异常）", fmt.label())
        }
    }
}

// ---------- Auto-snapshot hooks (save-side methods from the former app::profiles, moved after the UI split) ----------

impl App {
    /// The auto-snapshot directory: tests may inject a tempdir; production uses backups/ under the config dir.
    pub(in crate::app) fn backups_root(&self) -> std::path::PathBuf {
        self.snapshots_root
            .clone()
            .unwrap_or_else(crate::profiles::default_backups_root)
    }

    /// The pre-save auto-snapshot (Auto tag).
    ///
    /// Goes through core's external-change detection: back up only when disk content differs from the
    /// recorded hash (or this session never snapshotted) — content we just wrote isn't re-archived; only
    /// externally-modified files get one copy before being overwritten. Snapshot failures are always silent:
    /// backups are insurance and must never block the save's main flow. WSL paths go through the wsl command
    /// while core snapshots use the local fs only — skipped outright.
    pub(in crate::app) fn auto_snapshot_before_save(&mut self, path: &str) {
        if path.trim().is_empty() || crate::util::is_wsl_path(path) {
            return;
        }
        let target = std::path::Path::new(path);
        let last_hash = self.snapshot_hashes.get(path).cloned().unwrap_or_default();
        let backups_root = self.backups_root();
        if crate::profiles::snapshot_if_changed_into(&backups_root, target, &last_hash).is_some() {
            self.note_snapshot_hash(path);
        }
    }

    /// Record the target file's current content hash (called after a successful write), for the next save's "external change" verdict.
    pub(in crate::app) fn note_snapshot_hash(&mut self, path: &str) {
        if path.trim().is_empty() || crate::util::is_wsl_path(path) {
            return;
        }
        if let Some(hash) = crate::profiles::content_hash(std::path::Path::new(path)) {
            self.snapshot_hashes.insert(path.to_string(), hash);
        }
    }

    /// Post-save backup pruning: by the default retention policy (Auto keeps the latest 20, etc.). Failures are silent.
    pub(in crate::app) fn prune_backups_after_save(&self) {
        let _ = crate::profiles::prune(
            &self.backups_root(),
            crate::profiles::PrunePolicy::default(),
        );
    }
}
#[cfg(test)]
mod wsl_skip_note_tests {
    use super::*;

    /// "Probing" and "not installed" wordings must be distinguishable.
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

    /// The wording carries the backend's name.
    #[test]
    fn note_names_the_backend() {
        let note = wsl_skip_note(ConfigFormat::KimiCode, &backends::WslTargetState::Unknown);
        assert!(note.contains(ConfigFormat::KimiCode.label()), "{note}");
    }
}
