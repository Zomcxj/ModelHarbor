//! Profile storage plus snapshot / rollback core.
//!
//! A profile is a named container for "one configuration state": on disk it's a subdirectory under the
//! config root; metadata is centralized in `profiles.json` at the root, and the active profile is
//! recorded in `.current` (one line of text). A snapshot is a backup of any config file, grouped by
//! tag and pruned by retention policy; before a restore, the current file is first backed up as
//! BeforeRestore so the rollback chain stays reversible.
//!
//! Layout (root follows the config-dir convention of [`crate::prefs::Prefs::config_dir`]):
//! ```text
//! <root>/profiles/<name>/        one content directory per profile
//! <root>/profiles.json           metadata index (name, description, created/modified, default flag)
//! <root>/.current                current active profile name (one line of text)
//! <root>/backups/                default snapshot directory ([`default_backups_root`], configurable)
//! ```

use crate::prefs::Prefs;
use crate::util::atomic_write_text;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// State-file name for the active profile (under root).
const CURRENT_FILE: &str = ".current";
/// Metadata index file name (under root).
const METADATA_FILE: &str = "profiles.json";
/// Parent directory name for profile directories.
const PROFILES_DIR: &str = "profiles";
/// Metadata schema version (ignored on read).
const METADATA_VERSION: u64 = 1;
/// Seconds per day (retention policy counts in days).
const DAY_SECS: u64 = 86_400;

/// Current Unix seconds (0 when the system clock is unavailable; only affects timestamp records).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Validate and normalize a profile name: after trim it must be non-empty, must not start with a dot
/// (blocking `.`, `..`, and state names like `.current`), must not contain path separators or Windows
/// reserved characters, max length 64. Returns the normalized name.
fn validate_profile_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("profile 名不能为空".to_string());
    }
    if trimmed.starts_with('.') {
        return Err(format!("profile 名不能以点开头：「{trimmed}」"));
    }
    if trimmed.chars().count() > 64 {
        return Err("profile 名过长（上限 64 字符）".to_string());
    }
    if let Some(bad) = trimmed.chars().find(|ch| {
        matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || ch.is_control()
    }) {
        return Err(format!("profile 名含非法字符 {bad:?}：「{trimmed}」"));
    }
    Ok(trimmed.to_string())
}

/// One profile's metadata (an entry in the `profiles.json` index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileMeta {
    pub name: String,
    pub description: String,
    /// Unix seconds; 0 means metadata is missing (the directory exists, the index lacks it).
    pub created_at: u64,
    pub modified_at: u64,
    pub is_default: bool,
}

impl ProfileMeta {
    fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), Value::String(self.name.clone()));
        m.insert(
            "description".into(),
            Value::String(self.description.clone()),
        );
        m.insert("created_at".into(), Value::Number(self.created_at.into()));
        m.insert("modified_at".into(), Value::Number(self.modified_at.into()));
        m.insert("is_default".into(), Value::Bool(self.is_default));
        Value::Object(m)
    }

    /// Parse from an index entry; missing fields fall back to defaults (the name follows the index key).
    fn from_value(name: &str, value: &Value) -> ProfileMeta {
        ProfileMeta {
            name: name.to_string(),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            created_at: value.get("created_at").and_then(Value::as_u64).unwrap_or(0),
            modified_at: value
                .get("modified_at")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            is_default: value
                .get("is_default")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

/// Profile store: directory-based, all state lives under root.
#[derive(Clone, Debug)]
pub struct ProfileStore {
    root: PathBuf,
}

impl ProfileStore {
    /// Construct with a given root (tests use a tempdir; the real user directory is never touched).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Construct per the app's convention: root = the config directory (`.modelharbor` under home).
    pub fn open() -> Self {
        Self::new(Prefs::config_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The parent of profile directories: `<root>/profiles`.
    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join(PROFILES_DIR)
    }

    /// One profile's content directory: `<root>/profiles/<name>`.
    pub fn profile_dir(&self, name: &str) -> PathBuf {
        self.profiles_dir().join(name)
    }

    fn metadata_path(&self) -> PathBuf {
        self.root.join(METADATA_FILE)
    }

    fn current_path(&self) -> PathBuf {
        self.root.join(CURRENT_FILE)
    }

    /// Read the metadata index; a missing / corrupt file counts as an empty index (directories are the source of truth).
    fn load_metadata(&self) -> BTreeMap<String, ProfileMeta> {
        let Ok(text) = fs::read_to_string(self.metadata_path()) else {
            return BTreeMap::new();
        };
        let Ok(root) = serde_json::from_str::<Value>(&text) else {
            return BTreeMap::new();
        };
        root.get("profiles")
            .and_then(Value::as_object)
            .map(|items| {
                items
                    .iter()
                    .map(|(name, value)| (name.clone(), ProfileMeta::from_value(name, value)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Atomically write the metadata index back.
    fn save_metadata(&self, metas: &BTreeMap<String, ProfileMeta>) -> Result<(), String> {
        let profiles: Map<String, Value> = metas
            .values()
            .map(|meta| (meta.name.clone(), meta.to_value()))
            .collect();
        let mut root = Map::new();
        root.insert(
            "version".to_string(),
            Value::Number(METADATA_VERSION.into()),
        );
        root.insert("profiles".to_string(), Value::Object(profiles));
        let text = serde_json::to_string_pretty(&Value::Object(root))
            .map_err(|err| format!("序列化 profiles.json 失败：{err}"))?;
        atomic_write_text(&self.metadata_path(), &text)
    }

    /// List every profile (name-sorted).
    ///
    /// Directories under `profiles/` are the authority: directories missing from the index get synthesized
    /// default metadata (timestamps 0); index entries whose directories are gone are stale and not listed.
    pub fn list(&self) -> Vec<ProfileMeta> {
        let metas = self.load_metadata();
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(self.profiles_dir()) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            out.push(metas.get(name).cloned().unwrap_or(ProfileMeta {
                name: name.to_string(),
                description: String::new(),
                created_at: 0,
                modified_at: 0,
                is_default: false,
            }));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Create a profile: build the directory + write the index. Duplicate names (including trim-collisions) error.
    pub fn create(&self, name: &str) -> Result<ProfileMeta, String> {
        let name = validate_profile_name(name)?;
        let mut metas = self.load_metadata();
        let dir = self.profile_dir(&name);
        if dir.exists() || metas.contains_key(&name) {
            return Err(format!("profile「{name}」已存在"));
        }
        fs::create_dir_all(&dir)
            .map_err(|err| format!("创建 profile 目录失败（{}）：{}", dir.display(), err))?;
        let now = now_secs();
        let meta = ProfileMeta {
            name: name.clone(),
            description: String::new(),
            created_at: now,
            modified_at: now,
            is_default: false,
        };
        metas.insert(name.clone(), meta.clone());
        self.save_metadata(&metas)?;
        Ok(meta)
    }

    /// Delete a profile: remove the directory + clear the index. `confirm` must be true, else refused.
    /// Deleting the currently active one clears `.current`.
    pub fn delete(&self, name: &str, confirm: bool) -> Result<(), String> {
        if !confirm {
            return Err("删除 profile 需要显式确认（confirm = true）".to_string());
        }
        let name = validate_profile_name(name)?;
        let mut metas = self.load_metadata();
        let dir = self.profile_dir(&name);
        if !dir.exists() && !metas.contains_key(&name) {
            return Err(format!("profile「{name}」不存在"));
        }
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .map_err(|err| format!("删除 profile 目录失败（{}）：{}", dir.display(), err))?;
        }
        metas.remove(&name);
        self.save_metadata(&metas)?;
        if self.current().as_deref() == Some(name.as_str()) {
            fs::remove_file(self.current_path())
                .map_err(|err| format!("清理 .current 失败（{}）：{}", name, err))?;
        }
        Ok(())
    }

    /// Rename: move the directory + update the index; the active profile follows to the new name.
    pub fn rename(&self, old: &str, new: &str) -> Result<(), String> {
        let old = validate_profile_name(old)?;
        let new = validate_profile_name(new)?;
        if old == new {
            return Err(format!("新名字与旧名字相同：「{new}」"));
        }
        let mut metas = self.load_metadata();
        let old_dir = self.profile_dir(&old);
        if !old_dir.is_dir() {
            return Err(format!("profile「{old}」不存在"));
        }
        if self.profile_dir(&new).exists() || metas.contains_key(&new) {
            return Err(format!("profile「{new}」已存在"));
        }
        fs::rename(&old_dir, self.profile_dir(&new)).map_err(|err| {
            format!(
                "重命名 profile 目录失败（{} → {}）：{}",
                old_dir.display(),
                self.profile_dir(&new).display(),
                err
            )
        })?;
        let now = now_secs();
        match metas.remove(&old) {
            Some(mut meta) => {
                meta.name = new.clone();
                meta.modified_at = now;
                metas.insert(new.clone(), meta);
            }
            // Directory present but metadata missing: synthesize it, so rename doesn't orphan it.
            None => {
                metas.insert(
                    new.clone(),
                    ProfileMeta {
                        name: new.clone(),
                        description: String::new(),
                        created_at: now,
                        modified_at: now,
                        is_default: false,
                    },
                );
            }
        }
        self.save_metadata(&metas)?;
        if self.current().as_deref() == Some(old.as_str()) {
            self.switch(&new)?;
        }
        Ok(())
    }

    /// The currently active profile's name; `None` when `.current` is missing / empty.
    pub fn current(&self) -> Option<String> {
        let text = fs::read_to_string(self.current_path()).ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_string())
    }

    /// Switch the active profile: atomically write the name into `.current` (temp+rename).
    /// The profile must exist (its directory present).
    pub fn switch(&self, name: &str) -> Result<(), String> {
        let name = validate_profile_name(name)?;
        if !self.profile_dir(&name).is_dir() {
            return Err(format!("profile「{name}」不存在"));
        }
        atomic_write_text(&self.current_path(), &format!("{name}\n"))
    }
}

/// Snapshot tag: marks where this backup came from; also the retention policy's grouping key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapshotTag {
    /// The automatic snapshot triggered by schedules / external modification.
    Auto,
    /// The user's manual snapshot.
    Manual,
    /// The safety snapshot before switching profiles.
    PreSwitch,
    /// The safety snapshot of the current file before a restore (keeping further rollback reversible).
    BeforeRestore,
}

impl SnapshotTag {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
            Self::PreSwitch => "preswitch",
            Self::BeforeRestore => "beforerestore",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(Self::Auto),
            "manual" => Some(Self::Manual),
            "preswitch" => Some(Self::PreSwitch),
            "beforerestore" => Some(Self::BeforeRestore),
            _ => None,
        }
    }
}

/// Default snapshot directory: `backups/` under the config dir.
pub fn default_backups_root() -> PathBuf {
    Prefs::config_dir().join("backups")
}

/// Snapshot to the default directory (`backups/` under the config dir).
pub fn snapshot(target_path: &Path, tag: SnapshotTag) -> Result<PathBuf, String> {
    snapshot_into(&default_backups_root(), target_path, tag)
}

/// Copy target_path into backups_root and return the backup file path.
///
/// Naming: `{target filename}.{YYYYMMDD_HHMMSS}.{tag}.bak` (UTC timestamp; the target filename is kept
/// whole including its extension so same-named json / yml configs can't overwrite each other in the
/// backup dir). Multiple snapshots of the same target within one second bump the seconds to avoid name
/// collisions. The target must exist and be UTF-8 text (config files are text; binaries are not
/// snapshotted). Disk writes go through [`atomic_write_text`].
pub fn snapshot_into(
    backups_root: &Path,
    target_path: &Path,
    tag: SnapshotTag,
) -> Result<PathBuf, String> {
    let target_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("目标路径没有可用文件名：{}", target_path.display()))?
        .to_string();
    let content = fs::read_to_string(target_path)
        .map_err(|err| format!("读取目标文件失败（{}）：{}", target_path.display(), err))?;
    let start = now_secs();
    let mut ts = start;
    let dest = loop {
        let candidate = backups_root.join(backup_name(&target_name, ts, tag, false));
        if !candidate.exists() {
            break candidate;
        }
        ts += 1;
        if ts - start > DAY_SECS {
            return Err(format!(
                "备份重名过多，无法生成快照名：{}",
                target_path.display()
            ));
        }
    };
    atomic_write_text(&dest, &content)?;
    Ok(dest)
}

/// Restore a backup onto target_path.
///
/// Before restoring, the current target is first backed up as BeforeRestore (into the backup's own
/// directory), keeping a further rollback reversible; the final write is atomic — on failure the
/// current file stays untouched. When the target doesn't exist, the safety snapshot is skipped and
/// the restore proceeds directly.
pub fn restore(backup_path: &Path, target_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(backup_path)
        .map_err(|err| format!("读取备份失败（{}）：{}", backup_path.display(), err))?;
    if target_path.exists() {
        // The safety snapshot joins the restored backup's own directory, keeping the whole rollback chain in one place.
        let backups_root = backup_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(default_backups_root);
        snapshot_into(&backups_root, target_path, SnapshotTag::BeforeRestore)?;
    }
    atomic_write_text(target_path, &content)
}

/// FNV-1a 64-bit digest of file content (16 hex characters).
///
/// The only purpose is "detect external modification": different hashes before and after = the file
/// was changed. Not a secure digest and used for nothing beyond content comparison.
pub fn content_hash(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(fnv1a_hex(&bytes))
}

/// Hex text of the FNV-1a 64-bit digest (same algorithm family as prefs::config_identity).
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// External-modification detection: auto-snapshot (Auto tag) only when the current hash differs from `last_hash`.
///
/// An empty `last_hash` means "never snapshotted" — any existing file gets snapshotted. Missing file
/// or failed snapshot both return None — detection must never interrupt the main flow.
pub fn snapshot_if_changed(target_path: &Path, last_hash: &str) -> Option<PathBuf> {
    snapshot_if_changed_into(&default_backups_root(), target_path, last_hash)
}

/// Same as [`snapshot_if_changed`], but the snapshot goes to a given directory (unit tests / custom dirs).
pub fn snapshot_if_changed_into(
    backups_root: &Path,
    target_path: &Path,
    last_hash: &str,
) -> Option<PathBuf> {
    let current = content_hash(target_path)?;
    if !last_hash.is_empty() && current == last_hash {
        return None;
    }
    snapshot_into(backups_root, target_path, SnapshotTag::Auto).ok()
}

/// Retention policy: prune per tag ([`prune`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrunePolicy {
    /// Manual snapshots' retention days: older ones prune; anything with `.pin.` never prunes.
    pub manual_keep_days: u64,
    /// Auto snapshots' retention count (keeping the most recent).
    pub auto_keep: usize,
    /// BeforeRestore snapshots' retention count (keeping the most recent).
    pub before_restore_keep: usize,
}

impl Default for PrunePolicy {
    fn default() -> Self {
        Self {
            manual_keep_days: 30,
            auto_keep: 20,
            before_restore_keep: 2,
        }
    }
}

/// Prune a backup directory by retention policy; returns the number of deleted files.
///
/// Only recognizes `*.bak` files named by this module; files whose tag / timestamp can't be parsed
/// are left alone; a missing directory means nothing to prune. PreSwitch has no retention policy yet
/// and is untouched.
pub fn prune(backups_root: &Path, policy: PrunePolicy) -> Result<usize, String> {
    let Ok(entries) = fs::read_dir(backups_root) else {
        return Ok(0);
    };
    let mut by_tag: BTreeMap<SnapshotTag, Vec<(u64, PathBuf, bool)>> = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(parsed) = parse_backup_name(name) else {
            continue;
        };
        by_tag
            .entry(parsed.tag)
            .or_default()
            .push((parsed.ts, path, parsed.pinned));
    }
    let now = now_secs();
    let mut removed = 0usize;
    for (tag, mut items) in by_tag {
        // Newest timestamps first; "keep the latest N" means skipping everything past the first N.
        items.sort_by_key(|item| std::cmp::Reverse(item.0));
        let doomed: Vec<PathBuf> = match tag {
            SnapshotTag::Auto => items
                .into_iter()
                .skip(policy.auto_keep)
                .map(|(_, path, _)| path)
                .collect(),
            SnapshotTag::BeforeRestore => items
                .into_iter()
                .skip(policy.before_restore_keep)
                .map(|(_, path, _)| path)
                .collect(),
            SnapshotTag::Manual => {
                let cutoff = now.saturating_sub(policy.manual_keep_days * DAY_SECS);
                items
                    .into_iter()
                    .filter(|(ts, _, pinned)| !*pinned && *ts < cutoff)
                    .map(|(_, path, _)| path)
                    .collect()
            }
            SnapshotTag::PreSwitch => Vec::new(),
        };
        for path in doomed {
            fs::remove_file(&path)
                .map_err(|err| format!("删除备份失败（{}）：{}", path.display(), err))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Backup file name: `{target filename}.{YYYYMMDD_HHMMSS}.{tag}.bak`;
/// pinned backups insert `.pin` before `.bak` (the retention policy keys on that name).
fn backup_name(target_name: &str, ts: u64, tag: SnapshotTag, pinned: bool) -> String {
    let pin = if pinned { ".pin" } else { "" };
    format!(
        "{}.{}.{}{}.bak",
        target_name,
        format_timestamp(ts),
        tag.as_str(),
        pin
    )
}

/// Parsed backup file name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedBackupName {
    ts: u64,
    tag: SnapshotTag,
    pinned: bool,
}

/// Parse `{stem}.{YYYYMMDD_HHMMSS}.{tag}[.pin].bak`; returns None for anything else.
/// The stem may contain dots (the target filename is kept whole), so only the tag and timestamp are
/// stripped from the right.
fn parse_backup_name(name: &str) -> Option<ParsedBackupName> {
    let without_bak = name.strip_suffix(".bak")?;
    let (without_pin, pinned) = match without_bak.strip_suffix(".pin") {
        Some(rest) => (rest, true),
        None => (without_bak, false),
    };
    let tag_dot = without_pin.rfind('.')?;
    let tag = SnapshotTag::parse(&without_pin[tag_dot + 1..])?;
    let before_tag = &without_pin[..tag_dot];
    let ts_dot = before_tag.rfind('.')?;
    if ts_dot == 0 {
        return None; // stem can't be empty
    }
    let ts = parse_timestamp(&before_tag[ts_dot + 1..])?;
    Some(ParsedBackupName { ts, tag, pinned })
}

/// Unix seconds → `YYYYMMDD_HHMMSS` (UTC). Used for snapshot naming; no local timezone involved.
fn format_timestamp(unix: u64) -> String {
    let seconds_of_day = unix % DAY_SECS;
    let (year, month, day) = civil_from_days((unix / DAY_SECS) as i64);
    let (hh, mm, ss) = (
        seconds_of_day / 3_600,
        seconds_of_day % 3_600 / 60,
        seconds_of_day % 60,
    );
    format!("{year:04}{month:02}{day:02}_{hh:02}{mm:02}{ss:02}")
}

/// `YYYYMMDD_HHMMSS` (UTC) → Unix seconds. Only parses this module's own format.
fn parse_timestamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 15 || bytes[8] != b'_' {
        return None;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(i, b)| i == 8 || b.is_ascii_digit())
    {
        return None;
    }
    let num = |range: std::ops::Range<usize>| text[range].parse::<u64>().ok();
    let (year, month, day) = (num(0..4)? as i64, num(4..6)? as u32, num(6..8)? as u32);
    let (hh, mm, ss) = (num(9..11)?, num(11..13)?, num(13..15)?);
    // Validation is lenient: refuse only clear violations (this module's own names never violate).
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    Some((days_from_civil(year, month, day) as u64) * DAY_SECS + hh * 3_600 + mm * 60 + ss)
}

/// Days since epoch (1970-01-01 = 0) → (year, month, day). Howard Hinnant's civil_from_days.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (year + i64::from(month <= 2), month, day)
}

/// (year, month, day) → days since epoch (1970-01-01 = 0). Inverse of civil_from_days.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 } as u64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + u64::from(day) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// An isolated tempdir: never touches the real user directory; cleared then created.
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("modelharbor-profiles-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).expect("建父目录");
        }
        fs::write(path, content).expect("写文件");
    }

    /// The count of files in the backup directory ending with the given suffix (`.auto.bak` / `.beforerestore.bak` etc.).
    fn count_backups(backups_root: &Path, suffix: &str) -> usize {
        fs::read_dir(backups_root)
            .expect("读备份目录")
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(suffix))
            })
            .count()
    }

    #[test]
    fn backup_name_format_matches_spec() {
        // 2024-01-01 12:00:00 UTC (1_704_110_400).
        assert_eq!(
            backup_name("settings.json", 1_704_110_400, SnapshotTag::Manual, false),
            "settings.json.20240101_120000.manual.bak"
        );
        // A pinned file name carries .pin before .bak.
        assert_eq!(
            backup_name("settings.json", 1_704_110_400, SnapshotTag::Manual, true),
            "settings.json.20240101_120000.manual.pin.bak"
        );
        for (tag, text) in [
            (SnapshotTag::Auto, "auto"),
            (SnapshotTag::Manual, "manual"),
            (SnapshotTag::PreSwitch, "preswitch"),
            (SnapshotTag::BeforeRestore, "beforerestore"),
        ] {
            assert_eq!(tag.as_str(), text);
            let name = backup_name("x.json", 0, tag, false);
            assert_eq!(parse_backup_name(&name).map(|p| p.tag), Some(tag), "{name}");
        }
        // Files not named by this module are always refused.
        assert!(parse_backup_name("notes.txt").is_none());
        assert!(parse_backup_name("x.auto.bak").is_none(), "缺时间戳");
        assert!(parse_backup_name("x.9999.bak").is_none(), "时间戳格式不对");
        assert!(
            parse_backup_name("x.20240101_120000.bogus.bak").is_none(),
            "tag 不认识"
        );
        assert!(
            parse_backup_name(".20240101_120000.auto.bak").is_none(),
            "stem 不能为空"
        );
        // Dots in the target file name don't confuse parsing (only the right side is stripped).
        assert!(parse_backup_name("settings.json.20240101_120000.auto.bak").is_some());
        assert!(parse_backup_name("a.b.c.20240101_120000.auto.pin.bak").is_some());
    }

    #[test]
    fn timestamp_format_is_utc_and_round_trips() {
        assert_eq!(format_timestamp(0), "19700101_000000");
        assert_eq!(format_timestamp(1_704_067_200), "20240101_000000");
        // Leap day.
        assert_eq!(format_timestamp(1_709_164_800), "20240229_000000");
        for ts in [
            0u64,
            951_782_400,
            1_704_067_200,
            1_709_164_800,
            2_233_478_400,
            4_102_444_800,
        ] {
            assert_eq!(
                parse_timestamp(&format_timestamp(ts)),
                Some(ts),
                "{ts} 应可往返"
            );
        }
        assert_eq!(parse_timestamp("20240101 000000"), None, "分隔符不对");
        assert_eq!(parse_timestamp("2024-101_000000"), None, "夹了非数字");
        assert_eq!(parse_timestamp("2024010_120000"), None, "长度不对");
        assert_eq!(parse_timestamp("20241301_000000"), None, "月份越界");
        assert_eq!(parse_timestamp("20240101_240000"), None, "小时越界");
    }

    #[test]
    fn profile_store_round_trips_list_current_switch() {
        let root = scratch_dir("round-trip");
        let store = ProfileStore::new(root.clone());
        assert!(store.list().is_empty(), "空 store 没有任何 profile");
        assert_eq!(store.current(), None);

        let work = store.create("work").expect("创建 work");
        assert_eq!(work.name, "work");
        assert!(work.created_at > 0, "创建时间要记录");
        assert!(!work.is_default, "默认标记初始为 false");
        store.create("home").expect("创建 home");

        let names: Vec<String> = store.list().iter().map(|m| m.name.clone()).collect();
        assert_eq!(
            names,
            vec!["home".to_string(), "work".to_string()],
            "按名字排序"
        );

        store.switch("home").expect("切换");
        assert_eq!(store.current().as_deref(), Some("home"));
        assert_eq!(
            fs::read_to_string(root.join(".current")).unwrap(),
            "home\n",
            ".current 是一行文本"
        );

        // Re-read through a fresh store instance: metadata and the current item must both come back from disk.
        let reopened = ProfileStore::new(root.clone());
        assert_eq!(reopened.list().len(), 2);
        assert_eq!(reopened.current().as_deref(), Some("home"));
        assert!(root.join("profiles").join("work").is_dir());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn create_rejects_invalid_and_duplicate_names() {
        let root = scratch_dir("invalid-names");
        let store = ProfileStore::new(root.clone());
        for bad in [
            "", "   ", "a/b", "a\\b", ".", "..", ".hidden", "a:b", "a*b", "a<b", "a>b", "a|b",
            "a\"b", "a\tb",
        ] {
            assert!(store.create(bad).is_err(), "「{bad}」不应被接受");
        }
        store.create("ok").expect("正常名字");
        assert!(store.create(" ok ").is_err(), "trim 后撞名也要拒绝");
        assert_eq!(store.list().len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn delete_requires_confirmation_and_clears_current() {
        let root = scratch_dir("delete-confirm");
        let store = ProfileStore::new(root.clone());
        store.create("a").unwrap();
        store.create("b").unwrap();
        store.switch("a").unwrap();

        assert!(store.delete("a", false).is_err(), "未确认不得删除");
        assert_eq!(store.list().len(), 2, "未确认时什么都还在");
        assert!(store.delete("ghost", true).is_err(), "删不存在的要报错");

        store.delete("a", true).expect("确认后删除");
        assert_eq!(store.list().len(), 1);
        assert!(!root.join("profiles").join("a").exists());
        assert_eq!(store.current(), None, "删掉的是当前项，.current 应清空");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_moves_directory_updates_metadata_and_follows_current() {
        let root = scratch_dir("rename");
        let store = ProfileStore::new(root.clone());
        store.create("old").unwrap();
        store.create("taken").unwrap();
        store.switch("old").unwrap();

        assert!(store.rename("old", "taken").is_err(), "撞名要拒绝");
        assert!(store.rename("ghost", "new").is_err(), "源不存在要报错");

        store.rename("old", "renamed").expect("重命名");
        assert!(!root.join("profiles").join("old").exists());
        assert!(root.join("profiles").join("renamed").is_dir());
        let names: Vec<String> = store.list().iter().map(|m| m.name.clone()).collect();
        assert_eq!(names, vec!["renamed".to_string(), "taken".to_string()]);
        assert_eq!(
            store.current().as_deref(),
            Some("renamed"),
            "当前激活项跟着改名"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_metadata_falls_back_to_directory_scan() {
        let root = scratch_dir("corrupt-meta");
        let store = ProfileStore::new(root.clone());
        store.create("kept").unwrap();
        // Stray files under profiles/ don't count as profiles.
        fs::write(root.join("profiles").join("stray.txt"), "x").unwrap();
        fs::write(root.join("profiles.json"), "not json {").unwrap();

        let list = store.list();
        assert_eq!(list.len(), 1, "目录还在的 profile 不能丢");
        assert_eq!(list[0].name, "kept");
        assert_eq!(list[0].created_at, 0, "缺元数据时时间戳记 0");
        assert_eq!(list[0].description, "");

        // Hand-write metadata with is_default; reading must recognize it.
        fs::write(
            root.join("profiles.json"),
            r#"{"version":1,"profiles":{"kept":{"description":"d","created_at":10,"modified_at":20,"is_default":true}}}"#,
        )
        .unwrap();
        let list = store.list();
        assert_eq!(list[0].description, "d");
        assert_eq!(list[0].created_at, 10);
        assert_eq!(list[0].modified_at, 20);
        assert!(list[0].is_default);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn snapshot_restore_chain_is_reversible() {
        let root = scratch_dir("reversible");
        let backups = root.join("backups");
        let target = root.join("conf").join("settings.json");
        write_file(&target, "v1");

        let snap1 = snapshot_into(&backups, &target, SnapshotTag::Manual).expect("首次快照");
        assert!(
            snap1.to_string_lossy().ends_with(".manual.bak"),
            "{snap1:?}"
        );

        write_file(&target, "v2");
        restore(&snap1, &target).expect("恢复到 v1");
        assert_eq!(fs::read_to_string(&target).unwrap(), "v1");

        // Before restoring, the then-current v2 is backed up as BeforeRestore: rolling back again returns to v2.
        let before_restores: Vec<PathBuf> = fs::read_dir(&backups)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".beforerestore.bak"))
            .collect();
        assert_eq!(before_restores.len(), 1, "恢复前要生成一份 BeforeRestore");
        assert_eq!(fs::read_to_string(&before_restores[0]).unwrap(), "v2");

        restore(&before_restores[0], &target).expect("再回滚到 v2");
        assert_eq!(fs::read_to_string(&target).unwrap(), "v2");
        assert_eq!(
            count_backups(&backups, ".beforerestore.bak"),
            2,
            "第二次恢复又生成一份 BeforeRestore"
        );

        restore(&snap1, &target).expect("回到最初的 v1");
        assert_eq!(fs::read_to_string(&target).unwrap(), "v1");
        assert!(
            !fs::read_dir(&backups)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".tmp"))),
            "快照 / 恢复不得残留临时文件"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_restore_keeps_current_content() {
        let root = scratch_dir("restore-fail");
        let backups = root.join("backups");
        let target = root.join("conf.json");
        write_file(&target, "current");

        let snap = snapshot_into(&backups, &target, SnapshotTag::Manual).unwrap();
        write_file(&target, "changed");
        // Hold the temp-file path with a directory so the atomic write necessarily fails (simulating an interruption).
        fs::create_dir(root.join("conf.json.tmp")).unwrap();

        assert!(restore(&snap, &target).is_err());
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "changed",
            "失败的恢复不能动当前文件"
        );
        // The pre-restore safety snapshot is already on disk: `changed` can still be restored manually afterwards.
        assert_eq!(count_backups(&backups, ".beforerestore.bak"), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn snapshot_requires_target_and_avoids_name_collision() {
        let root = scratch_dir("snapshot-edge");
        let backups = root.join("backups");
        let target = root.join("settings.json");

        assert!(
            snapshot_into(&backups, &target, SnapshotTag::Auto).is_err(),
            "目标不存在要报错"
        );

        write_file(&target, "content");
        let first = snapshot_into(&backups, &target, SnapshotTag::Auto).expect("第一次");
        let second = snapshot_into(&backups, &target, SnapshotTag::Auto).expect("第二次");
        assert_ne!(first, second, "同一秒内的两次快照不能顶掉前一份");
        assert!(first.is_file() && second.is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prune_applies_retention_policy_per_tag() {
        let root = scratch_dir("prune");
        let backups = root.join("backups");
        fs::create_dir_all(&backups).unwrap();
        let now = now_secs();
        let day = DAY_SECS;

        // Auto: 25 hourly snapshots → only the latest 20 stay.
        for i in 0..25u64 {
            let name = backup_name(
                &format!("auto{i:02}.json"),
                now - i * 3_600,
                SnapshotTag::Auto,
                false,
            );
            fs::write(backups.join(name), "x").unwrap();
        }
        // BeforeRestore: 4 snapshots → only the latest 2 stay.
        for i in 0..4u64 {
            let name = backup_name(
                &format!("br{i}.json"),
                now - i * day,
                SnapshotTag::BeforeRestore,
                false,
            );
            fs::write(backups.join(name), "x").unwrap();
        }
        // Manual: 40-day-old ones prune; the same day's pin stays; yesterday's stays.
        let old = now - 40 * day;
        fs::write(
            backups.join(backup_name("old.json", old, SnapshotTag::Manual, false)),
            "x",
        )
        .unwrap();
        fs::write(
            backups.join(backup_name("old.json", old, SnapshotTag::Manual, true)),
            "x",
        )
        .unwrap();
        fs::write(
            backups.join(backup_name(
                "recent.json",
                now - day,
                SnapshotTag::Manual,
                false,
            )),
            "x",
        )
        .unwrap();
        // Unparsable files are left alone.
        fs::write(backups.join("readme.txt"), "x").unwrap();
        fs::write(backups.join("broken.auto.bak"), "x").unwrap();

        let removed = prune(&backups, PrunePolicy::default()).expect("清理");
        assert_eq!(
            removed,
            5 + 2 + 1,
            "Auto 超 5 份 + BeforeRestore 超 2 份 + 40 天前的未 pin Manual"
        );

        let left: Vec<String> = fs::read_dir(&backups)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        assert_eq!(
            left.iter()
                .filter(|n| parse_backup_name(n).is_some_and(|p| p.tag == SnapshotTag::Auto))
                .count(),
            20,
            "合法 Auto 备份剩 20 份（broken.auto.bak 不算）"
        );
        assert_eq!(
            left.iter()
                .filter(|n| n.ends_with(".beforerestore.bak"))
                .count(),
            2
        );
        let manual: Vec<&String> = left.iter().filter(|n| n.contains(".manual")).collect();
        assert_eq!(
            manual.len(),
            2,
            "pin 的与 30 天内的 Manual 都要留：{manual:?}"
        );
        assert!(left.contains(&"readme.txt".to_string()));
        assert!(left.contains(&"broken.auto.bak".to_string()));

        // Running again converges to 0: nothing left to prune.
        assert_eq!(prune(&backups, PrunePolicy::default()).unwrap(), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn content_hash_change_triggers_auto_snapshot() {
        let root = scratch_dir("hash");
        let backups = root.join("backups");
        let target = root.join("settings.json");
        write_file(&target, "aaa");

        let h1 = content_hash(&target).expect("有内容就有 hash");
        assert_eq!(h1.len(), 16, "FNV-1a 64 位输出 16 个 hex 字符");
        write_file(&target, "bbb");
        assert_ne!(
            content_hash(&target).as_deref(),
            Some(h1.as_str()),
            "内容变了 hash 要变"
        );
        write_file(&target, "aaa");
        assert_eq!(
            content_hash(&target).as_deref(),
            Some(h1.as_str()),
            "内容还原 hash 也要还原"
        );
        assert_eq!(
            content_hash(&root.join("missing.json")),
            None,
            "缺文件返回 None"
        );

        // Never snapshotted (empty hash) → snapshot directly.
        let first = snapshot_if_changed_into(&backups, &target, "").expect("首次触发");
        assert!(first.to_string_lossy().ends_with(".auto.bak"));
        assert_eq!(count_backups(&backups, ".auto.bak"), 1);

        // Hash unchanged → no snapshot.
        let current = content_hash(&target).unwrap();
        assert_eq!(snapshot_if_changed_into(&backups, &target, &current), None);
        assert_eq!(count_backups(&backups, ".auto.bak"), 1);

        // The file was externally changed; comparing the old hash → snapshot triggers.
        write_file(&target, "external edit");
        assert!(snapshot_if_changed_into(&backups, &target, &current).is_some());
        assert_eq!(count_backups(&backups, ".auto.bak"), 2);

        // Missing target → None, no error.
        assert_eq!(
            snapshot_if_changed_into(&backups, &root.join("missing.json"), ""),
            None
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn default_backups_root_sits_in_config_dir() {
        assert_eq!(
            default_backups_root(),
            crate::prefs::Prefs::config_dir().join("backups")
        );
    }
}
