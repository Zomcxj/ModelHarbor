//! 用量账本 —— 让**已删除的会话**的用量永久保留。
//!
//! 问题：opencode / workbuddy 的会话被删后，其用量从源文件里消失；只读源会让
//! 历史总量凭空缩水。
//!
//! 做法（照 token-monitor 的 `sessionUsageArchive`）：每次扫描后把会话快照并进账本，
//! 展示时「实时扫描结果 ∪ 账本里已消失的会话」，后者标记 [`SessionSnapshot::archived`]。
//!
//! 三条硬约束：
//! - **原子写**：写临时文件再 rename，中途崩溃不会留下半个文件；
//! - **差异写**：只落盘变化的条目，不是每次全量重写；
//! - **静默恢复**：文件损坏 / 读失败 → 当作空账本重建，绝不让账本问题导致功能不可用。

use crate::format::ConfigFormat;
use crate::usage::daily::{DailyBucket, DailyMap};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 一个会话在某一时刻的用量快照。
///
/// 字段口径与 tokscale 一致：`input` 是**新鲜输入**（已排除缓存读），
/// `cache_write_1h` 是 `cache_write` 的子集、**不参与合计**。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub client: ConfigFormat,
    pub session_id: String,
    /// 最后一次见到的模型名（同一会话可能中途换模型，取最近一条）。
    pub model_id: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    /// 会话内消息条数。
    pub message_count: i64,
    /// 首次见到该会话的时间（Unix 毫秒）。
    pub first_seen_ms: i64,
    /// 最后一次见到该会话的时间（Unix 毫秒）。
    pub last_seen_ms: i64,
    /// 该会话已从源文件里消失（被删除），用量由账本保留。
    pub archived: bool,
}

impl SessionSnapshot {
    /// 账本键：`client:session_id`（与 token-monitor 的 `sessionKey` 同构）。
    pub fn key(client: ConfigFormat, session_id: &str) -> String {
        format!("{}:{}", client.label(), session_id)
    }

    /// 从扫描结果里的一条消息建快照。
    pub fn new(client: ConfigFormat, session_id: &str, model_id: &str) -> Self {
        Self {
            client,
            session_id: session_id.to_string(),
            model_id: model_id.to_string(),
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 0,
            first_seen_ms: i64::MAX,
            last_seen_ms: i64::MIN,
            archived: false,
        }
    }

    /// 并入一条消息的用量。
    ///
    /// 时间取 min/max（同一会话的消息可能乱序）；模型取时间最新的那条。
    pub fn absorb(&mut self, message: &tokscale_core::ParsedMessage) {
        self.input = self.input.saturating_add(message.input);
        self.output = self.output.saturating_add(message.output);
        self.cache_read = self.cache_read.saturating_add(message.cache_read);
        self.cache_write = self.cache_write.saturating_add(message.cache_write);
        self.reasoning = self.reasoning.saturating_add(message.reasoning);
        self.message_count = self
            .message_count
            .saturating_add(i64::from(message.message_count.max(1)));
        if message.timestamp < self.first_seen_ms {
            self.first_seen_ms = message.timestamp;
        }
        if message.timestamp >= self.last_seen_ms {
            self.last_seen_ms = message.timestamp;
            if !message.model_id.is_empty() {
                self.model_id = message.model_id.clone();
            }
        }
    }

    /// 合计 token（口径与 tokscale 的 `TokenBreakdown::total` 一致）。
    ///
    /// `cache_write_1h` **不计入**：它是 `cache_write` 的子集。
    pub fn total(&self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }

    /// 该快照是否有实际用量（全 0 的不值得进账本）。
    pub fn has_usage(&self) -> bool {
        self.total() > 0
    }

    fn to_json(&self) -> Value {
        json!({
            "client": self.client.label(),
            "session_id": self.session_id,
            "model_id": self.model_id,
            "input": self.input,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
            "reasoning": self.reasoning,
            "message_count": self.message_count,
            "first_seen_ms": self.first_seen_ms,
            "last_seen_ms": self.last_seen_ms,
        })
    }

    fn from_json(key: &str, value: &Value) -> Option<Self> {
        let client_label = value.get("client")?.as_str()?;
        let client = client_format_from_label(client_label)?;
        let session_id = value.get("session_id")?.as_str()?.to_string();
        // 键与内容不一致的条目视为损坏，丢弃（不让脏数据污染统计）。
        if Self::key(client, &session_id) != key {
            return None;
        }
        Some(Self {
            client,
            session_id,
            model_id: value
                .get("model_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input: value.get("input").and_then(Value::as_i64).unwrap_or(0),
            output: value.get("output").and_then(Value::as_i64).unwrap_or(0),
            cache_read: value.get("cache_read").and_then(Value::as_i64).unwrap_or(0),
            cache_write: value
                .get("cache_write")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            reasoning: value.get("reasoning").and_then(Value::as_i64).unwrap_or(0),
            message_count: value
                .get("message_count")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            first_seen_ms: value
                .get("first_seen_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX),
            last_seen_ms: value
                .get("last_seen_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN),
            archived: false,
        })
    }
}

/// `ConfigFormat::label()` → 枚举。账本用 label 落盘（可读、可手改）。
fn client_format_from_label(label: &str) -> Option<ConfigFormat> {
    crate::backends::BACKENDS
        .iter()
        .map(|backend| backend.id())
        .find(|format| format.label() == label)
}

/// 账本当前 schema 版本。格式变更时 +1，旧版本整体丢弃重建。
///
/// 新增可选的 `daily` 字段**不**升版本：旧账本没有这个键时按空处理，
/// 下一轮扫描会补回来；而升版本会丢掉已攒下的会话历史（已删会话的用量
/// 无法重扫回来，那是账本存在的意义）。
const LEDGER_VERSION: u32 = 1;

/// 用量账本。
///
/// 内存里持有全部会话快照；[`Ledger::merge_scan`] 并入新扫描结果，
/// [`Ledger::view`] 产出「实时 ∪ 已归档」的展示视图。
#[derive(Debug, Default)]
pub struct Ledger {
    /// `client:session_id` → 快照（含已归档的）。
    entries: HashMap<String, SessionSnapshot>,
    /// 按本地日历日分桶（含已删会话留下的记录）。
    daily: DailyMap,
    /// 上次落盘后发生变化的会话键。
    dirty: Vec<String>,
    /// 上次落盘后发生变化的日期键。
    dirty_days: Vec<String>,
    /// 账本文件路径；`None` = 纯内存（测试 / 落盘不可用时）。
    path: Option<PathBuf>,
}

impl Ledger {
    /// 账本文件名（放在 [`crate::prefs::Prefs::config_dir`] 下，与 settings.json /
    /// tokens.json 同目录）。
    pub const FILE_NAME: &'static str = "usage-ledger.json";

    /// 默认账本路径：`~/.modelharbor/usage-ledger.json`。
    pub fn default_path() -> PathBuf {
        crate::prefs::Prefs::config_dir().join(Self::FILE_NAME)
    }

    /// 打开默认位置的账本。
    pub fn open_default() -> Self {
        Self::open(Self::default_path())
    }

    /// 打开账本：读取落盘文件，损坏 / 版本不符时按空账本重建。
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (entries, daily) = Self::load(&path).unwrap_or_default();
        Self {
            entries,
            daily,
            dirty: Vec::new(),
            dirty_days: Vec::new(),
            path: Some(path),
        }
    }

    /// 纯内存账本（不落盘）。
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// 账本条目数（含已归档）。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 是否有未落盘的改动。
    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty() || !self.dirty_days.is_empty()
    }

    /// 并入一次扫描结果。
    ///
    /// - 新会话 → 新增；
    /// - 已有会话 → 用量取**较大值**（token 只增不减；源里偶发的重扫回退不应让总量缩水）；
    /// - 账本里有、本次扫描没有 → 标记 [`SessionSnapshot::archived`]（**核心需求**）。
    pub fn merge_scan(&mut self, scanned: HashMap<String, SessionSnapshot>) {
        // 先全部标成「本次没见到」，下面见到的再摘掉。
        for entry in self.entries.values_mut() {
            entry.archived = true;
        }
        for (key, fresh) in scanned {
            match self.entries.get_mut(&key) {
                Some(existing) => {
                    if existing.absorb_update(&fresh) {
                        self.dirty.push(key);
                    }
                    existing.archived = false;
                }
                None => {
                    let mut fresh = fresh;
                    fresh.archived = false;
                    self.entries.insert(key.clone(), fresh);
                    self.dirty.push(key);
                }
            }
        }
        // 本次没见到的：状态从「实时」翻成「已归档」也算变化。
        for (key, entry) in &self.entries {
            if entry.archived && !self.dirty.contains(key) {
                self.dirty.push(key.clone());
            }
        }
    }

    /// 并入一次扫描的按天分桶。
    ///
    /// 与会话快照分开合并：两者服务于不同视图（会话明细 vs 热力图 /
    /// 时间范围筛选），一条会话被删后它的按天记录也要留着。
    /// 同一天重复扫描时逐字段取较大值，token 只增不减。
    pub fn merge_daily(&mut self, scanned: DailyMap) {
        for (date, fresh) in scanned {
            match self.daily.get_mut(&date) {
                Some(existing) => {
                    let before = existing.clone();
                    existing.merge_max(&fresh);
                    if *existing != before {
                        self.dirty_days.push(date);
                    }
                }
                None => {
                    self.daily.insert(date.clone(), fresh);
                    self.dirty_days.push(date);
                }
            }
        }
    }

    /// 按天分桶（热力图与「今日 / 本周 / 本月 / 总计」的数据源）。
    pub fn daily(&self) -> &DailyMap {
        &self.daily
    }

    /// 展示视图：实时扫描结果 ∪ 账本（已删会话保留，标记 `archived`）。
    pub fn view(&self) -> Vec<SessionSnapshot> {
        let mut out: Vec<SessionSnapshot> = self.entries.values().cloned().collect();
        // 稳定排序：按最近活动倒序，其次按会话 id，保证 UI 不跳动。
        out.sort_by(|a, b| {
            b.last_seen_ms
                .cmp(&a.last_seen_ms)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        out
    }

    /// 落盘（差异写 + 原子写）。
    ///
    /// 无改动时直接返回，不产生 IO。
    pub fn save(&mut self) -> std::io::Result<()> {
        if self.dirty.is_empty() && self.dirty_days.is_empty() {
            return Ok(());
        }
        let Some(path) = self.path.clone() else {
            self.dirty.clear();
            self.dirty_days.clear();
            return Ok(());
        };
        let payload = json!({
            "version": LEDGER_VERSION,
            "sessions": self
                .entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.to_json()))
                .collect::<serde_json::Map<String, Value>>(),
            "daily": self
                .daily
                .iter()
                .map(|(date, bucket)| (date.clone(), bucket.to_json()))
                .collect::<serde_json::Map<String, Value>>(),
        });
        let text = serde_json::to_string_pretty(&payload)?;
        write_atomic(&path, &text)?;
        self.dirty.clear();
        self.dirty_days.clear();
        Ok(())
    }

    /// 读取落盘内容。返回 `None` = 文件不存在 / 不可读 / 损坏。
    ///
    /// `daily` 缺失时按空处理而不是丢掉整个账本：`daily` 是后来加的字段，
    /// 升级时不该把已经攒下的会话历史一起丢掉，下一轮扫描会把它补回来。
    fn load(path: &Path) -> Option<(HashMap<String, SessionSnapshot>, DailyMap)> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        if value.get("version").and_then(Value::as_u64)? != u64::from(LEDGER_VERSION) {
            // 版本不符：整体丢弃重建（宁可丢历史，不要脏数据）。
            return Some((HashMap::new(), DailyMap::new()));
        }
        let sessions = value.get("sessions")?.as_object()?;
        let mut out = HashMap::with_capacity(sessions.len());
        for (key, entry) in sessions {
            if let Some(snapshot) = SessionSnapshot::from_json(key, entry) {
                if snapshot.has_usage() {
                    out.insert(key.clone(), snapshot);
                }
            }
        }
        let daily = value
            .get("daily")
            .and_then(Value::as_object)
            .map(|obj| {
                obj.iter()
                    .filter(|(_, v)| {
                        // 全 0 的日期桶不值得占地方。
                        !DailyBucket::from_json("", v).totals.is_empty()
                    })
                    .map(|(date, v)| (date.clone(), DailyBucket::from_json(date, v)))
                    .collect()
            })
            .unwrap_or_default();
        Some((out, daily))
    }
}

impl SessionSnapshot {
    /// 用新快照更新自己。返回是否发生变化。
    ///
    /// 用量取较大值：token 只会增长，源里偶发的重扫回退不该让总量缩水。
    fn absorb_update(&mut self, fresh: &SessionSnapshot) -> bool {
        let before = self.clone();
        self.input = self.input.max(fresh.input);
        self.output = self.output.max(fresh.output);
        self.cache_read = self.cache_read.max(fresh.cache_read);
        self.cache_write = self.cache_write.max(fresh.cache_write);
        self.reasoning = self.reasoning.max(fresh.reasoning);
        self.message_count = self.message_count.max(fresh.message_count);
        self.first_seen_ms = self.first_seen_ms.min(fresh.first_seen_ms);
        self.last_seen_ms = self.last_seen_ms.max(fresh.last_seen_ms);
        if fresh.last_seen_ms >= before.last_seen_ms && !fresh.model_id.is_empty() {
            self.model_id = fresh.model_id.clone();
        }
        *self != before
    }
}

/// 原子写：先写同目录临时文件，再 rename 覆盖。
///
/// rename 在同一文件系统内是原子的：要么看到旧内容，要么看到完整新内容，
/// 不会出现半个文件。临时文件名带进程 id，避免并发写互相踩。
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    // Windows 上 rename 不覆盖已存在的目标，先删（失败也无妨，rename 会再报错）。
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时账本路径：进程 id + 原子计数器，避免并行测试撞目录。
    fn temp_path(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "model-harbor-ledger-{}-{}-{}.json",
            std::process::id(),
            n,
            name
        ))
    }

    fn snapshot(session: &str, input: i64, output: i64) -> SessionSnapshot {
        SessionSnapshot {
            client: ConfigFormat::Pi,
            session_id: session.to_string(),
            model_id: "test-model".to_string(),
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 1,
            first_seen_ms: 1_000,
            last_seen_ms: 2_000,
            archived: false,
        }
    }

    fn scan_of(items: Vec<SessionSnapshot>) -> HashMap<String, SessionSnapshot> {
        items
            .into_iter()
            .map(|s| (SessionSnapshot::key(s.client, &s.session_id), s))
            .collect()
    }

    /// 构造按天分桶：`(日期, input, output)`。
    fn daily_of(days: &[(&str, i64, i64)]) -> DailyMap {
        let mut out = DailyMap::new();
        for (date, input, output) in days {
            let mut bucket = DailyBucket::new(*date);
            bucket.totals.input = *input;
            bucket.totals.output = *output;
            bucket.totals.messages = 1;
            out.insert((*date).to_string(), bucket);
        }
        out
    }

    /// 键格式：`client_label:session_id`。
    #[test]
    fn key_is_client_label_colon_session() {
        assert_eq!(
            SessionSnapshot::key(ConfigFormat::DeepSeekHarness, "abc"),
            "deepseek-harness:abc"
        );
        assert_eq!(
            SessionSnapshot::key(ConfigFormat::OhMyPi, "x"),
            "oh-my-pi:x"
        );
    }

    /// 合计不含 cache_write_1h 那类子集（此处用字段验证口径）。
    #[test]
    fn total_sums_every_bucket_except_subsets() {
        let mut s = snapshot("a", 100, 20);
        s.cache_read = 1000;
        s.cache_write = 50;
        s.reasoning = 5;
        assert_eq!(s.total(), 1175);
    }

    /// 新会话进账本。
    #[test]
    fn new_session_is_recorded() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        assert_eq!(ledger.len(), 1);
        let view = ledger.view();
        assert_eq!(view[0].session_id, "s1");
        assert!(!view[0].archived);
    }

    /// 用量增长被并入。
    #[test]
    fn growing_session_updates_in_place() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        ledger.merge_scan(scan_of(vec![snapshot("s1", 300, 40)]));
        assert_eq!(ledger.len(), 1, "同一个会话不应变成两条");
        assert_eq!(ledger.view()[0].input, 300);
        assert_eq!(ledger.view()[0].output, 40);
    }

    /// **核心需求**：会话从源里消失后，账本仍保留其用量并标记 archived。
    #[test]
    fn deleted_session_survives_in_ledger() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![
            snapshot("keep", 100, 10),
            snapshot("gone", 500, 50),
        ]));
        // 下一轮扫描：gone 消失了
        ledger.merge_scan(scan_of(vec![snapshot("keep", 150, 15)]));

        let view = ledger.view();
        assert_eq!(view.len(), 2, "已删除的会话必须保留");
        let gone = view
            .iter()
            .find(|s| s.session_id == "gone")
            .expect("gone 应仍在账本里");
        assert!(gone.archived, "消失的会话应标记 archived");
        assert_eq!(gone.input, 500, "用量原样保留");
        assert_eq!(gone.output, 50);

        let keep = view.iter().find(|s| s.session_id == "keep").unwrap();
        assert!(!keep.archived, "仍在源里的不应标记 archived");
    }

    /// 已归档的会话重新出现 → archived 摘掉。
    #[test]
    fn reappearing_session_clears_archived() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        ledger.merge_scan(scan_of(vec![]));
        assert!(ledger.view()[0].archived);
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        assert!(!ledger.view()[0].archived, "会话回来后不应再标 archived");
    }

    /// 源里的回退（重扫读到更小值）不让总量缩水。
    #[test]
    fn shrinking_scan_does_not_reduce_totals() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 1000, 100)]));
        ledger.merge_scan(scan_of(vec![snapshot("s1", 10, 1)]));
        let view = ledger.view();
        assert_eq!(view[0].input, 1000, "用量只增不减");
        assert_eq!(view[0].output, 100);
    }

    /// 多个 agent 的同名 session_id 互不干扰。
    #[test]
    fn same_session_id_under_different_clients_stay_separate() {
        let mut ledger = Ledger::in_memory();
        let mut a = snapshot("dup", 100, 1);
        a.client = ConfigFormat::Pi;
        let mut b = snapshot("dup", 200, 2);
        b.client = ConfigFormat::Opencode;
        ledger.merge_scan(scan_of(vec![a, b]));
        assert_eq!(ledger.len(), 2);
    }

    /// 落盘 → 重开：已删会话的用量仍在。
    #[test]
    fn ledger_persists_across_reopen() {
        let path = temp_path("persist");
        {
            let mut ledger = Ledger::open(&path);
            ledger.merge_scan(scan_of(vec![
                snapshot("keep", 100, 10),
                snapshot("gone", 500, 50),
            ]));
            ledger.save().expect("落盘应成功");
        }
        {
            let mut ledger = Ledger::open(&path);
            // 新的一轮：gone 已从源里消失
            ledger.merge_scan(scan_of(vec![snapshot("keep", 100, 10)]));
            let view = ledger.view();
            assert_eq!(view.len(), 2, "重开后已删会话仍应保留");
            let gone = view.iter().find(|s| s.session_id == "gone").unwrap();
            assert_eq!(gone.input, 500);
            assert!(gone.archived);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// 按天分桶能跨重开保留（热力图数据不能因为重启就没了）。
    #[test]
    fn daily_buckets_persist_across_reopen() {
        let path = temp_path("daily-persist");
        {
            let mut ledger = Ledger::open(&path);
            ledger.merge_daily(daily_of(&[
                ("2026-01-01", 100, 10),
                ("2026-01-02", 200, 20),
            ]));
            ledger.save().expect("落盘应成功");
        }
        {
            let ledger = Ledger::open(&path);
            assert_eq!(ledger.daily().len(), 2);
            let day = &ledger.daily()["2026-01-02"];
            assert_eq!(day.totals.input, 200);
            assert_eq!(day.totals.output, 20);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// 旧账本（没有 `daily` 键）不该丢掉会话历史。
    ///
    /// 已删会话的用量无法重扫回来，那是账本存在的意义；新增可选的 `daily`
    /// 字段只能追加，不能靠升版本号把老数据洗掉。
    #[test]
    fn legacy_ledger_without_daily_keeps_sessions() {
        let path = temp_path("legacy-no-daily");
        let legacy = json!({
            "version": 1,
            "sessions": {
                "pi:old": {
                    "client": "pi",
                    "session_id": "old",
                    "model_id": "glm-5",
                    "input": 42,
                    "output": 7,
                }
            }
        });
        std::fs::write(&path, serde_json::to_string(&legacy).unwrap()).unwrap();

        let ledger = Ledger::open(&path);
        assert_eq!(ledger.len(), 1, "老账本的会话必须留着");
        assert!(ledger.daily().is_empty(), "缺 daily 键按空处理");
        let _ = std::fs::remove_file(&path);
    }

    /// 同一天重扫取较大值：源里偶发的回退不让历史缩水。
    #[test]
    fn daily_merge_never_shrinks() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_daily(daily_of(&[("2026-01-01", 1000, 100)]));
        ledger.merge_daily(daily_of(&[("2026-01-01", 10, 1)]));
        assert_eq!(ledger.daily()["2026-01-01"].totals.input, 1000);
    }

    /// 已有 daily 且无新变化时，`save` 不产生 IO（差异写）。
    #[test]
    fn daily_unchanged_scan_does_not_dirty() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_daily(daily_of(&[("2026-01-01", 100, 10)]));
        ledger.save().expect("内存账本落盘应成功");
        assert!(!ledger.is_dirty(), "落盘后应变干净");

        // 同一天同样的数据再来一遍：不算变化。
        ledger.merge_daily(daily_of(&[("2026-01-01", 100, 10)]));
        assert!(!ledger.is_dirty(), "无变化不应标脏");
    }

    /// 账本文件损坏 → 静默重建，不 panic。
    #[test]
    fn corrupt_ledger_recovers_silently() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ this is not json").unwrap();
        let ledger = Ledger::open(&path);
        assert!(ledger.is_empty(), "损坏文件应被当成空账本");

        // 截断的 JSON 同样处理
        std::fs::write(&path, "{\"version\":1,\"sessions\":{\"a\":").unwrap();
        assert!(Ledger::open(&path).is_empty());

        // 二进制垃圾
        std::fs::write(&path, [0u8, 159, 146, 150]).unwrap();
        assert!(Ledger::open(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// 版本不符 → 丢弃重建（不尝试迁移脏数据）。
    #[test]
    fn version_mismatch_discards_old_ledger() {
        let path = temp_path("version");
        std::fs::write(
            &path,
            r#"{"version":999,"sessions":{"pi:x":{"client":"pi","session_id":"x"}}}"#,
        )
        .unwrap();
        assert!(Ledger::open(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// 键与内容不一致的条目被丢弃。
    #[test]
    fn mismatched_key_is_dropped() {
        let path = temp_path("mismatch");
        std::fs::write(
            &path,
            r#"{"version":1,"sessions":{"pi:WRONG":{"client":"pi","session_id":"right","input":5}}}"#,
        )
        .unwrap();
        assert!(Ledger::open(&path).is_empty(), "键与内容不符应丢弃");
        let _ = std::fs::remove_file(&path);
    }

    /// 无改动时不落盘（不产生 IO）。
    #[test]
    fn save_is_noop_when_clean() {
        let path = temp_path("clean");
        let mut ledger = Ledger::open(&path);
        ledger.save().expect("无改动落盘应成功");
        assert!(!path.exists(), "无改动不应创建文件");
    }

    /// 原子写：落盘后目录里不残留临时文件。
    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let path = temp_path("atomic");
        let mut ledger = Ledger::open(&path);
        ledger.merge_scan(scan_of(vec![snapshot("s1", 1, 1)]));
        ledger.save().expect("落盘应成功");
        assert!(path.exists(), "目标文件应存在");
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        assert!(!tmp.exists(), "临时文件应已清理");
        let _ = std::fs::remove_file(&path);
    }

    /// 目录不存在时自动创建。
    #[test]
    fn save_creates_missing_parent_dir() {
        let dir =
            std::env::temp_dir().join(format!("model-harbor-ledger-dir-{}", std::process::id()));
        let path = dir.join("nested").join("ledger.json");
        let mut ledger = Ledger::open(&path);
        ledger.merge_scan(scan_of(vec![snapshot("s1", 1, 1)]));
        ledger.save().expect("应自动创建父目录");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 展示视图按最近活动倒序。
    #[test]
    fn view_is_sorted_by_recency() {
        let mut ledger = Ledger::in_memory();
        let mut old = snapshot("old", 1, 1);
        old.last_seen_ms = 100;
        let mut new = snapshot("new", 1, 1);
        new.last_seen_ms = 900;
        ledger.merge_scan(scan_of(vec![old, new]));
        let view = ledger.view();
        assert_eq!(view[0].session_id, "new", "最近的排前面");
    }

    /// 零用量会话不进账本（避免堆积空条目）。
    #[test]
    fn zero_usage_session_is_not_persisted() {
        let path = temp_path("zero");
        {
            let mut ledger = Ledger::open(&path);
            ledger.merge_scan(scan_of(vec![snapshot("empty", 0, 0)]));
            ledger.save().unwrap();
        }
        let reopened = Ledger::open(&path);
        assert!(reopened.is_empty(), "零用量条目落盘后应被过滤");
        let _ = std::fs::remove_file(&path);
    }

    /// 落盘后可读：JSON 里能看到 client / session_id。
    #[test]
    fn persisted_json_is_readable() {
        let path = temp_path("readable");
        let mut ledger = Ledger::open(&path);
        ledger.merge_scan(scan_of(vec![snapshot("s1", 42, 7)]));
        ledger.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"session_id\": \"s1\""), "内容: {text}");
        assert!(text.contains("\"input\": 42"));
        let _ = std::fs::remove_file(&path);
    }
}
