//! Profile 存储与快照 / 回滚核心。
//!
//! Profile 是「一套配置状态」的命名容器：磁盘上就是配置目录下的一个子目录，
//! 元数据集中在根下的 `profiles.json`，当前激活项记在根下的 `.current`
//! （一行文本）。快照是对任意配置文件的一次备份，按 tag 分类并由保留策略
//! 清理；恢复前会先把当前文件再备份成 BeforeRestore，保证回滚链可逆。
//!
//! 布局（root 沿用 [`crate::prefs::Prefs::config_dir`] 的配置目录约定）：
//! ```text
//! <root>/profiles/<name>/        每个 profile 一个内容目录
//! <root>/profiles.json           元数据索引（名称、描述、创建/修改时间、是否默认）
//! <root>/.current                当前激活 profile 名（一行文本）
//! <root>/backups/                默认快照目录（[`default_backups_root`]，可配置）
//! ```

use crate::prefs::Prefs;
use crate::util::atomic_write_text;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 当前激活 profile 的状态文件名（root 下）。
const CURRENT_FILE: &str = ".current";
/// 元数据索引文件名（root 下）。
const METADATA_FILE: &str = "profiles.json";
/// profile 目录的父目录名。
const PROFILES_DIR: &str = "profiles";
/// 元数据 schema 版本（读取时忽略）。
const METADATA_VERSION: u64 = 1;
/// 一天的秒数（保留策略按天算）。
const DAY_SECS: u64 = 86_400;

/// 现在的 Unix 秒（取不到系统时间按 0 处理，只影响时间戳记录）。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 校验并规范 profile 名：trim 后不能为空、不能以点开头（挡住 `.` `..`
/// 与 `.current` 之类的状态名）、不能含路径分隔符与 Windows 保留字符，
/// 长度上限 64。返回规范后的名字。
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

/// 单个 profile 的元数据（`profiles.json` 索引里的一条）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileMeta {
    pub name: String,
    pub description: String,
    /// Unix 秒；0 表示元数据缺失（目录在、索引里没有）。
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

    /// 从索引条目解析；缺字段按默认值兜底（名字以索引键为准）。
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

/// Profile 存储：目录式，全部状态都在 root 之下。
#[derive(Clone, Debug)]
pub struct ProfileStore {
    root: PathBuf,
}

impl ProfileStore {
    /// 用指定根目录构造（单测用 tempdir，不碰真实用户目录）。
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// 按应用约定构造：root = 配置目录（家目录下的 `.modelharbor`）。
    pub fn open() -> Self {
        Self::new(Prefs::config_dir())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// profile 目录的父目录：`<root>/profiles`。
    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join(PROFILES_DIR)
    }

    /// 单个 profile 的内容目录：`<root>/profiles/<name>`。
    pub fn profile_dir(&self, name: &str) -> PathBuf {
        self.profiles_dir().join(name)
    }

    fn metadata_path(&self) -> PathBuf {
        self.root.join(METADATA_FILE)
    }

    fn current_path(&self) -> PathBuf {
        self.root.join(CURRENT_FILE)
    }

    /// 读取元数据索引；文件缺失 / 损坏按空索引处理（目录才是事实来源）。
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

    /// 原子写回元数据索引。
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

    /// 列出全部 profile（按名字排序）。
    ///
    /// 以 `profiles/` 下的目录为准；索引里缺条目的目录会合成一份默认元数据
    /// （时间戳记 0），索引里有而目录没了的条目视为过期、不列出。
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

    /// 新建 profile：建目录 + 写索引。重名（含 trim 后撞名）报错。
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

    /// 删除 profile：删目录 + 清索引。`confirm` 必须为 true，否则拒绝。
    /// 删掉的是当前激活项时，清空 `.current`。
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

    /// 重命名：挪目录 + 改索引；当前激活项跟着指向新名字。
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
            // 目录在而元数据缺失：补一份，别让 rename 把它变孤儿。
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

    /// 当前激活的 profile 名；`.current` 缺失 / 为空时返回 None。
    pub fn current(&self) -> Option<String> {
        let text = fs::read_to_string(self.current_path()).ok()?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_string())
    }

    /// 切换激活 profile：把名字原子写进 `.current`（temp+rename）。
    /// profile 必须存在（目录在）。
    pub fn switch(&self, name: &str) -> Result<(), String> {
        let name = validate_profile_name(name)?;
        if !self.profile_dir(&name).is_dir() {
            return Err(format!("profile「{name}」不存在"));
        }
        atomic_write_text(&self.current_path(), &format!("{name}\n"))
    }
}

/// 快照 tag：标记这次备份的来源，也是保留策略的分组依据。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapshotTag {
    /// 定时 / 外部修改触发的自动快照。
    Auto,
    /// 用户手动快照。
    Manual,
    /// 切换 profile 前的保险快照。
    PreSwitch,
    /// 恢复前对当前文件的保险快照（保证再回滚可逆）。
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

/// 默认快照目录：配置目录下的 `backups/`。
pub fn default_backups_root() -> PathBuf {
    Prefs::config_dir().join("backups")
}

/// 快照到默认目录（配置目录下的 `backups/`）。
pub fn snapshot(target_path: &Path, tag: SnapshotTag) -> Result<PathBuf, String> {
    snapshot_into(&default_backups_root(), target_path, tag)
}

/// 把 target_path 复制一份到 backups_root，返回备份文件路径。
///
/// 命名 `{目标文件名}.{YYYYMMDD_HHMMSS}.{tag}.bak`（UTC 时间戳；目标文件名
/// 整体保留含扩展名，避免同名的 json / yml 配置在备份目录里互相顶替）。
/// 同一秒内对同一目标快照多次时秒数 +1 避开重名。目标必须存在且为 UTF-8
/// 文本（配置文件都是文本；二进制不做快照）。落盘走 [`atomic_write_text`]。
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

/// 把备份恢复到 target_path。
///
/// 恢复前先把当前目标文件备份成 BeforeRestore（放进备份所在目录），
/// 保证再回滚可逆；最终落盘走原子写，失败时当前文件保持原样。
/// 目标不存在时跳过保险快照，直接恢复。
pub fn restore(backup_path: &Path, target_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(backup_path)
        .map_err(|err| format!("读取备份失败（{}）：{}", backup_path.display(), err))?;
    if target_path.exists() {
        // 保险快照与被恢复的备份放同一个目录，回滚链都在一处。
        let backups_root = backup_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(default_backups_root);
        snapshot_into(&backups_root, target_path, SnapshotTag::BeforeRestore)?;
    }
    atomic_write_text(target_path, &content)
}

/// 文件内容的 FNV-1a 64 位摘要（16 个 hex 字符）。
///
/// 用途只是「检测外部修改」：前后两次 hash 不同 = 文件被改过。
/// 不是安全摘要，也不用于内容比对之外的任何事。
pub fn content_hash(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(fnv1a_hex(&bytes))
}

/// FNV-1a 64 位摘要的 hex 文本（与 prefs::config_identity 同族算法）。
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// 外部修改检测：当前内容 hash 与 `last_hash` 不同才自动快照（Auto tag）。
///
/// `last_hash` 传空串表示「从没快照过」，只要文件存在就会快照。
/// 文件缺失或快照失败都返回 None——检测流程不应打断主流程。
pub fn snapshot_if_changed(target_path: &Path, last_hash: &str) -> Option<PathBuf> {
    snapshot_if_changed_into(&default_backups_root(), target_path, last_hash)
}

/// 同 [`snapshot_if_changed`]，但快照写到指定目录（单测 / 自定义目录用）。
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

/// 保留策略：按 tag 分别清理（[`prune`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrunePolicy {
    /// Manual 快照保留天数：超过的清理，带 `.pin.` 的永不清理。
    pub manual_keep_days: u64,
    /// Auto 快照保留条数（保留最近的）。
    pub auto_keep: usize,
    /// BeforeRestore 快照保留条数（保留最近的）。
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

/// 按保留策略清理备份目录，返回删除的文件数。
///
/// 只认本模块命名的 `*.bak`；解析不出 tag / 时间戳的文件一律不动，
/// 目录不存在视为无可清理。PreSwitch 暂无保留策略，先不动。
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
        // 时间戳新的在前；「保留最近 N 条」即跳过前 N 条之外的部分。
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

/// 备份文件名：`{目标文件名}.{YYYYMMDD_HHMMSS}.{tag}.bak`；
/// pin 的备份在 `.bak` 前插 `.pin`（保留策略认这个名字）。
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

/// 备份文件名的解析结果。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedBackupName {
    ts: u64,
    tag: SnapshotTag,
    pinned: bool,
}

/// 解析 `{stem}.{YYYYMMDD_HHMMSS}.{tag}[.pin].bak`；不是这个形状的返回 None。
/// stem 允许带点（目标文件名整体保留），所以只从右侧剥 tag 与时间戳。
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
        return None; // stem 不能为空
    }
    let ts = parse_timestamp(&before_tag[ts_dot + 1..])?;
    Some(ParsedBackupName { ts, tag, pinned })
}

/// Unix 秒 → `YYYYMMDD_HHMMSS`（UTC）。快照命名用，不涉及本地时区。
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

/// `YYYYMMDD_HHMMSS`（UTC）→ Unix 秒。只解析本模块生成的格式。
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
    // 校验从宽：只在明显越界时拒绝（本模块生成的名字不会越界）。
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    Some((days_from_civil(year, month, day) as u64) * DAY_SECS + hh * 3_600 + mm * 60 + ss)
}

/// 天数（1970-01-01 = 0）→ (年, 月, 日)。Howard Hinnant 的 civil_from_days。
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

/// (年, 月, 日) → 天数（1970-01-01 = 0）。civil_from_days 的逆运算。
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

    /// 独立 tempdir：不碰真实用户目录，先清后建。
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

    /// 备份目录里以指定后缀结尾的文件数（`.auto.bak` / `.beforerestore.bak` 等）。
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
        // 2024-01-01 12:00:00 UTC（1_704_110_400）。
        assert_eq!(
            backup_name("settings.json", 1_704_110_400, SnapshotTag::Manual, false),
            "settings.json.20240101_120000.manual.bak"
        );
        // pin 的文件名在 .bak 前带 .pin。
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
        // 不是本模块命名的文件一律拒收。
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
        // 目标文件名里的点不影响解析（只从右侧剥）。
        assert!(parse_backup_name("settings.json.20240101_120000.auto.bak").is_some());
        assert!(parse_backup_name("a.b.c.20240101_120000.auto.pin.bak").is_some());
    }

    #[test]
    fn timestamp_format_is_utc_and_round_trips() {
        assert_eq!(format_timestamp(0), "19700101_000000");
        assert_eq!(format_timestamp(1_704_067_200), "20240101_000000");
        // 闰日。
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

        // 换一个 store 实例再读：元数据与当前项都要从盘上回来。
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
        // profiles/ 下的杂散文件不算 profile。
        fs::write(root.join("profiles").join("stray.txt"), "x").unwrap();
        fs::write(root.join("profiles.json"), "not json {").unwrap();

        let list = store.list();
        assert_eq!(list.len(), 1, "目录还在的 profile 不能丢");
        assert_eq!(list[0].name, "kept");
        assert_eq!(list[0].created_at, 0, "缺元数据时时间戳记 0");
        assert_eq!(list[0].description, "");

        // 手写一份带 is_default 的元数据，读取要能认出来。
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

        // 恢复前会把当时的 v2 备份成 BeforeRestore：再回滚就能回到 v2。
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
        // 用目录占住临时文件路径，让原子写必然失败（模拟中断）。
        fs::create_dir(root.join("conf.json.tmp")).unwrap();

        assert!(restore(&snap, &target).is_err());
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "changed",
            "失败的恢复不能动当前文件"
        );
        // 恢复前的保险快照已经落盘：事后还能手工回到 changed。
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

        // Auto：25 份每小时一份 → 只留最近 20 份。
        for i in 0..25u64 {
            let name = backup_name(
                &format!("auto{i:02}.json"),
                now - i * 3_600,
                SnapshotTag::Auto,
                false,
            );
            fs::write(backups.join(name), "x").unwrap();
        }
        // BeforeRestore：4 份 → 只留最近 2 份。
        for i in 0..4u64 {
            let name = backup_name(
                &format!("br{i}.json"),
                now - i * day,
                SnapshotTag::BeforeRestore,
                false,
            );
            fs::write(backups.join(name), "x").unwrap();
        }
        // Manual：40 天前的清理；同一天的 pin 保留；昨天的保留。
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
        // 解析不了的文件不动。
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

        // 再跑一遍应收敛到 0：没有可清的了。
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

        // 从没快照过（空 hash）→ 直接快照。
        let first = snapshot_if_changed_into(&backups, &target, "").expect("首次触发");
        assert!(first.to_string_lossy().ends_with(".auto.bak"));
        assert_eq!(count_backups(&backups, ".auto.bak"), 1);

        // hash 没变 → 不快照。
        let current = content_hash(&target).unwrap();
        assert_eq!(snapshot_if_changed_into(&backups, &target, &current), None);
        assert_eq!(count_backups(&backups, ".auto.bak"), 1);

        // 外部改了文件，拿旧 hash 对比 → 触发快照。
        write_file(&target, "external edit");
        assert!(snapshot_if_changed_into(&backups, &target, &current).is_some());
        assert_eq!(count_backups(&backups, ".auto.bak"), 2);

        // 目标缺失 → None，不报错。
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
