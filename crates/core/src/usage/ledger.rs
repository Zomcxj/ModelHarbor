//! Usage ledger — makes usage of **deleted sessions** persist forever.
//!
//! Problem: once an opencode / workbuddy session is deleted, its usage vanishes from the source files;
//! reading sources alone would silently shrink historical totals.
//!
//! Approach (mirroring token-monitor's `sessionUsageArchive`): after every scan, merge session
//! snapshots into the ledger; sessions gone from the sources are flagged [`SessionSnapshot::archived`] —
//! they **don't appear in the list** (the session table only shows sessions still present in the agent's
//! own store), but their usage stays in the aggregates, so totals never shrink.
//!
//! The ledger has two layers with different lifecycles:
//! - **Aggregates** (daily totals and agent / model / cross breakdowns): append-only, kept forever;
//! - **Details** (session snapshots and per-day `by_session`): only feed aggregates once gone from the
//!   sources; user-initiated deletion really deletes ([`Ledger::ignore_session`]).
//!
//! Three hard constraints:
//! - **Atomic writes**: write a temp file then rename — a crash mid-way never leaves half a file;
//! - **Delta writes**: only changed entries hit disk, not a full rewrite each time;
//! - **Silent recovery**: corrupted / unreadable file → rebuild as an empty ledger; ledger problems must
//!   never take the feature down.
//!
//! User-initiated session deletion takes a separate path ([`Ledger::ignore_session`]):
//! **details withdrawn, aggregates kept** — isomorphic to cc-switch / token-monitor.

use crate::format::ConfigFormat;
use crate::usage::daily::{DailyBucket, DailyMap};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// A session's usage snapshot at one point in time.
///
/// Field semantics match tokscale: `input` is **fresh input** (cache reads already excluded),
/// `cache_write_1h` is a subset of `cache_write` and **excluded from totals**.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionSnapshot {
    pub client: ConfigFormat,
    pub session_id: String,
    /// The last model name seen (a session may switch models mid-way; take the most recent).
    pub model_id: String,
    /// The owning project (workspace): tokscale's parsed `workspace_label`, falling back to `workspace_key`
    /// when absent. Old ledger entries lack this field — `None`.
    pub project: Option<String>,
    /// Session title: clients that store names (opencode family's `session.title`) take the original text;
    /// those that don't (pi) take the opening of the first user message. See [`crate::usage::titles`].
    ///
    /// Persisted like `project`: names come from a separate collection channel, take no part in message
    /// aggregation, and ride along in the ledger between rescans. Old ledger entries lack this field — `None`.
    pub title: Option<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    /// Message count within the session.
    pub message_count: i64,
    /// Converted cost (USD): as reported upstream, or computed from the local pricing cache. Old entries have none — 0.
    pub cost: f64,
    /// When the session was first seen (Unix millis).
    pub first_seen_ms: i64,
    /// When the session was last seen (Unix millis).
    pub last_seen_ms: i64,
    /// Whether the session has vanished from the source files (deleted, or rolled off source-side), its usage kept by the ledger.
    ///
    /// This flag **persists** (see `to_json` / `from_json`): without it, every restart would list the ledger's
    /// long-gone sessions as if they were live results.
    pub archived: bool,
}

impl SessionSnapshot {
    /// Ledger key: `client:session_id` (isomorphic to token-monitor's `sessionKey`).
    pub fn key(client: ConfigFormat, session_id: &str) -> String {
        format!("{}:{}", client.label(), session_id)
    }

    /// Build a snapshot from one message of a scan result.
    pub fn new(client: ConfigFormat, session_id: &str, model_id: &str) -> Self {
        Self {
            client,
            session_id: session_id.to_string(),
            model_id: model_id.to_string(),
            project: None,
            title: None,
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 0,
            cost: 0.0,
            first_seen_ms: i64::MAX,
            last_seen_ms: i64::MIN,
            archived: false,
        }
    }

    /// Absorb one message's usage.
    ///
    /// Times take min/max (a session's messages may arrive out of order); the model takes the newest message's.
    pub fn absorb(&mut self, message: &tokscale_core::ParsedMessage) {
        self.input = self.input.saturating_add(message.input);
        self.output = self.output.saturating_add(message.output);
        self.cache_read = self.cache_read.saturating_add(message.cache_read);
        self.cache_write = self.cache_write.saturating_add(message.cache_write);
        self.reasoning = self.reasoning.saturating_add(message.reasoning);
        self.message_count = self
            .message_count
            .saturating_add(i64::from(message.message_count.max(1)));
        self.cost += message.cost;
        if message.timestamp < self.first_seen_ms {
            self.first_seen_ms = message.timestamp;
        }
        if message.timestamp >= self.last_seen_ms {
            self.last_seen_ms = message.timestamp;
            if !message.model_id.is_empty() {
                self.model_id = message.model_id.clone();
            }
        }
        // Project follows the message: a session never changes workspace, so fill it on first sight (old entries lack it).
        if message.workspace_label.is_some() || message.workspace_key.is_some() {
            self.project = message
                .workspace_label
                .clone()
                .or_else(|| message.workspace_key.clone());
        }
    }

    /// Total tokens (same accounting as tokscale's `TokenBreakdown::total`).
    ///
    /// `cache_write_1h` is **excluded**: it is a subset of `cache_write`.
    pub fn total(&self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }

    /// Whether this snapshot carries real usage (all-zero entries aren't worth the ledger).
    pub fn has_usage(&self) -> bool {
        self.total() > 0
    }

    fn to_json(&self) -> Value {
        json!({
            "client": self.client.label(),
            "session_id": self.session_id,
            "model_id": self.model_id,
            "project": self.project,
            "title": self.title,
            "input": self.input,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
            "reasoning": self.reasoning,
            "message_count": self.message_count,
            "cost": self.cost,
            "first_seen_ms": self.first_seen_ms,
            "last_seen_ms": self.last_seen_ms,
            "archived": self.archived,
        })
    }

    fn from_json(key: &str, value: &Value) -> Option<Self> {
        let client_label = value.get("client")?.as_str()?;
        let client = client_format_from_label(client_label)?;
        let session_id = value.get("session_id")?.as_str()?.to_string();
        // Entries whose key and content disagree count as corrupt and are dropped (dirty data must not pollute stats).
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
            project: value
                .get("project")
                .and_then(Value::as_str)
                .map(str::to_string),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string),
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
            cost: value.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
            first_seen_ms: value
                .get("first_seen_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX),
            last_seen_ms: value
                .get("last_seen_ms")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN),
            // Old ledgers lack this key: treat as "gone from the sources". Ledgers missing the key come from the
            // totals-only era, where which sessions still existed in the sources was unknowable — and the session
            // table only lists live sessions. Better to list none (the next scan immediately backfills live ones)
            // than to list vanished ones as live.
            archived: value
                .get("archived")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

/// `ConfigFormat::label()` → enum. The ledger persists labels (readable, hand-editable).
fn client_format_from_label(label: &str) -> Option<ConfigFormat> {
    crate::backends::BACKENDS
        .iter()
        .map(|backend| backend.id())
        .find(|format| format.label() == label)
}

/// Current ledger schema version. Bump on format changes; older versions are dropped and rebuilt.
///
/// Adding an optional `daily` field does **not** bump the version: when an old ledger lacks the key it
/// is treated as empty and the next scan backfills it; bumping the version would throw away accumulated
/// session history (usage of deleted sessions can never be rescanned — that's the ledger's whole point).
const LEDGER_VERSION: u32 = 1;

/// Deserialized ledger file: session snapshots, daily buckets, ignore list, data timestamp.
type LoadedLedger = (
    HashMap<String, SessionSnapshot>,
    DailyMap,
    BTreeSet<String>,
    Option<i64>,
);

/// The usage ledger.
///
/// Holds all session snapshots in memory; [`Ledger::merge_scan`] merges new scan results,
/// [`Ledger::view`] produces the "live ∪ archived" display view.
#[derive(Debug, Default)]
pub struct Ledger {
    /// `client:session_id` → snapshot (archived included; user-deleted ones are absent).
    entries: HashMap<String, SessionSnapshot>,
    /// Buckets per local calendar day (aggregates and breakdowns keep deleted sessions' usage; session details don't).
    daily: DailyMap,
    /// Keys of sessions the user deleted and **permanently ignored**: later scans never merge them back.
    ignored: BTreeSet<String>,
    /// Session keys changed since the last flush.
    dirty: Vec<String>,
    /// Date keys changed since the last flush.
    dirty_days: Vec<String>,
    /// Ledger file path; `None` = purely in-memory (tests / disk unavailable).
    path: Option<PathBuf>,
    /// Data flush timestamp (unix millis): the instant-display cache uses it for the "updated how long ago" note.
    /// Old-format files lacking the key fall back to file mtime; `None` = unknown.
    saved_at: Option<i64>,
}

impl Ledger {
    /// Ledger file name (under [`crate::prefs::Prefs::config_dir`], same directory as settings.json /
    /// tokens.json).
    pub const FILE_NAME: &'static str = "usage-ledger.json";

    /// Default ledger path: `~/.modelharbor/usage-ledger.json`.
    pub fn default_path() -> PathBuf {
        crate::prefs::Prefs::config_dir().join(Self::FILE_NAME)
    }

    /// Open the ledger at its default location.
    pub fn open_default() -> Self {
        Self::open(Self::default_path())
    }

    /// Open the ledger: read the persisted file; corrupt / wrong-version rebuilds as empty.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (entries, daily, ignored, saved_at) = Self::load(&path).unwrap_or_default();
        Self {
            entries,
            daily,
            ignored,
            dirty: Vec::new(),
            dirty_days: Vec::new(),
            path: Some(path),
            saved_at,
        }
    }

    /// Purely in-memory ledger (never flushed).
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Ledger entry count (archived included).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Data flush timestamp (unix millis); `None` = never flushed / unknown.
    pub fn saved_at(&self) -> Option<i64> {
        self.saved_at
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Count of sessions gone from the sources, existing only in the ledger.
    ///
    /// The UI uses it to explain "why totals include sessions older than anything in the session table": those
    /// sessions take no list rows, but their usage still counts toward totals.
    pub fn archived_len(&self) -> usize {
        self.entries.values().filter(|entry| entry.archived).count()
    }

    /// Whether there are unflushed changes.
    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty() || !self.dirty_days.is_empty()
    }

    /// Keys of deleted, permanently ignored sessions (the scan thread skips these).
    pub fn ignored(&self) -> &BTreeSet<String> {
        &self.ignored
    }

    /// Delete a session: withdraw **all of its details** from the ledger and add the key to the ignore list —
    /// later scans never merge it back.
    ///
    /// What's withdrawn is the details (the session snapshot + each day's `by_session` entries); the daily
    /// bucket's totals and agent / model / cross breakdowns **stay untouched**, so totals, the heatmap, and
    /// the time table don't shrink from deletion. This matches cc-switch (`usage_daily_rollups` carries no
    /// session dimension; deleting a session only removes the `session_log_sync` bookmark row) and
    /// token-monitor (pruning drops only `periods`' today / month windows while `allTime` persists):
    /// **aggregates stay, details go**.
    ///
    /// Details must really be deleted: keeping them would grow the ledger without bound as sessions
    /// accumulate, and they no longer serve the UI (the session table takes rows from `by_session` but
    /// templates from ledger snapshots — snapshots are removed here).
    ///
    /// Returns whether anything changed (`false` when deleting the same key twice).
    pub fn ignore_session(&mut self, key: &str) -> bool {
        if !self.ignored.insert(key.to_string()) {
            return false;
        }
        self.entries.remove(key);
        for (date, bucket) in &mut self.daily {
            if bucket.remove_session_detail(key) {
                self.dirty_days.push(date.clone());
            }
        }
        self.dirty.push(key.to_string());
        true
    }

    /// Merge one scan result.
    ///
    /// - New sessions → added;
    /// - Existing sessions → usage takes the **larger** value (tokens only grow; an occasional source-side
    ///   rescan dip must not shrink totals);
    /// - In the ledger but not this scan → flagged [`SessionSnapshot::archived`]: neither listed, nor removed
    ///   from the aggregates.
    pub fn merge_scan(&mut self, scanned: HashMap<String, SessionSnapshot>) {
        // Keys seen this round: needed at the end to tell "not seen" from "just seen".
        let seen: BTreeSet<String> = scanned.keys().cloned().collect();
        for (key, fresh) in scanned {
            if self.ignored.contains(&key) {
                // User-deleted sessions: seen in the scan, never merged back.
                continue;
            }
            match self.entries.get_mut(&key) {
                Some(existing) => {
                    let changed = existing.absorb_update(&fresh);
                    // Returning from "gone" to "in the sources" must also flush: the state flipped but wouldn't be marked dirty,
                    // and the next restart would treat live sessions as gone again.
                    let reappeared = existing.archived;
                    existing.archived = false;
                    if changed || reappeared {
                        self.dirty.push(key);
                    }
                }
                None => {
                    let mut fresh = fresh;
                    fresh.archived = false;
                    self.entries.insert(key.clone(), fresh);
                    self.dirty.push(key);
                }
            }
        }
        // Not seen this round: flip to "gone" and persist (already-gone ones aren't re-marked dirty).
        for (key, entry) in &mut self.entries {
            if seen.contains(key) || entry.archived {
                continue;
            }
            entry.archived = true;
            self.dirty.push(key.clone());
        }
    }

    /// Merge one scan's daily buckets.
    ///
    /// Merged separately from session snapshots: the two serve different views (session details vs the
    /// heatmap / time-range filters), and a deleted session's totals must survive. Repeated scans of the
    /// same day take the larger value per field — tokens only grow.
    pub fn merge_daily(&mut self, mut scanned: DailyMap) {
        // In-flight scan detail pruning: the ignore list this scan took off with didn't yet contain keys deleted
        // mid-flight (the user clicked delete while the scan ran), so results carry deleted sessions' details.
        // Totals and breakdowns merge as usual (token counts don't shrink); details are withdrawn per the
        // current list — otherwise one stale scan would quietly resurrect a deletion.
        for bucket in scanned.values_mut() {
            for key in &self.ignored {
                bucket.remove_session_detail(key);
            }
        }
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

    /// Daily buckets (the heatmap's and "today / week / month / total" data source).
    pub fn daily(&self) -> &DailyMap {
        &self.daily
    }

    /// Display view: **sessions still in the sources**, most recent activity first.
    ///
    /// Sessions gone from the sources ([`SessionSnapshot::archived`]) are not listed: the session table only
    /// presents sessions still in the agent's own store, otherwise most rows would be ghosts long gone from
    /// disk. Their usage remains in [`Self::daily`]'s totals and breakdowns — rows are lost, never numbers.
    pub fn view(&self) -> Vec<SessionSnapshot> {
        let mut out: Vec<SessionSnapshot> = self
            .entries
            .values()
            .filter(|entry| !entry.archived)
            .cloned()
            .collect();
        // Stable sort: most recent activity first, then by session id, so the UI doesn't jump.
        out.sort_by(|a, b| {
            b.last_seen_ms
                .cmp(&a.last_seen_ms)
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        out
    }

    /// Flush (delta write + atomic write).
    ///
    /// Returns immediately with no changes — no IO.
    pub fn save(&mut self) -> std::io::Result<()> {
        if self.dirty.is_empty() && self.dirty_days.is_empty() {
            return Ok(());
        }
        let Some(path) = self.path.clone() else {
            self.dirty.clear();
            self.dirty_days.clear();
            return Ok(());
        };
        let saved_at = unix_now_ms();
        let payload = json!({
            "version": LEDGER_VERSION,
            "saved_at": saved_at,
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
            "ignored": self.ignored.iter().cloned().collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&payload)?;
        write_atomic(&path, &text)?;
        self.saved_at = Some(saved_at);
        self.dirty.clear();
        self.dirty_days.clear();
        Ok(())
    }

    /// Read the persisted content. `None` = missing / unreadable / corrupt file.
    ///
    /// A missing `daily` is treated as empty rather than discarding the whole ledger: `daily` was added later,
    /// and upgrading must not throw away accumulated session history — the next scan backfills it.
    fn load(path: &Path) -> Option<LoadedLedger> {
        let text = std::fs::read_to_string(path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        if value.get("version").and_then(Value::as_u64)? != u64::from(LEDGER_VERSION) {
            // Version mismatch: discard and rebuild wholesale (better to lose history than keep dirty data).
            return Some((HashMap::new(), DailyMap::new(), BTreeSet::new(), None));
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
        let mut daily: DailyMap = value
            .get("daily")
            .and_then(Value::as_object)
            .map(|obj| {
                obj.iter()
                    .filter(|(_, v)| {
                        // All-zero day buckets aren't worth the space.
                        !DailyBucket::from_json("", v).totals.is_empty()
                    })
                    .map(|(date, v)| (date.clone(), DailyBucket::from_json(date, v)))
                    .collect()
            })
            .unwrap_or_default();
        let ignored: BTreeSet<String> = value
            .get("ignored")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        // On read, withdraw details per the ignore list: early versions only hid rows without withdrawing details,
        // so old ledgers carry deleted sessions' `by_session` entries. Self-healing here, without a version
        // bump (bumping would throw history away). Totals and breakdowns untouched — same semantics as deletion.
        for key in &ignored {
            for bucket in daily.values_mut() {
                bucket.remove_session_detail(key);
            }
        }
        // saved_at is a new key added for the instant-display cache: old formats fall back to file mtime, no version bump.
        let saved_at = value
            .get("saved_at")
            .and_then(Value::as_i64)
            .or_else(|| file_mtime_ms(path));
        Some((out, daily, ignored, saved_at))
    }
}

/// Current unix millis; treated as 0 when the clock misbehaves (only affects the "updated at" display).
fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// File modification time (unix millis): fallback for old ledgers without a `saved_at` key.
fn file_mtime_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as i64,
    )
}

impl SessionSnapshot {
    /// Update self with a new snapshot. Returns whether anything changed.
    ///
    /// Usage takes the larger value: tokens only grow, and an occasional source-side rescan dip must not shrink totals.
    fn absorb_update(&mut self, fresh: &SessionSnapshot) -> bool {
        let before = self.clone();
        self.input = self.input.max(fresh.input);
        self.output = self.output.max(fresh.output);
        self.cache_read = self.cache_read.max(fresh.cache_read);
        self.cache_write = self.cache_write.max(fresh.cache_write);
        self.reasoning = self.reasoning.max(fresh.reasoning);
        self.message_count = self.message_count.max(fresh.message_count);
        self.cost = self.cost.max(fresh.cost);
        self.first_seen_ms = self.first_seen_ms.min(fresh.first_seen_ms);
        self.last_seen_ms = self.last_seen_ms.max(fresh.last_seen_ms);
        if fresh.last_seen_ms >= before.last_seen_ms && !fresh.model_id.is_empty() {
            self.model_id = fresh.model_id.clone();
        }
        // Project takes the new snapshot's: it comes from the scan, always the newest parse.
        if fresh.project.is_some() {
            self.project = fresh.project.clone();
        }
        // Same for the title: the collection channel re-reads sources each round and new values win (the user may have renamed it in the agent).
        if fresh.title.is_some() {
            self.title = fresh.title.clone();
        }
        *self != before
    }
}

/// Atomic write: write a temp file in the same directory, then rename over the target.
///
/// Rename is atomic within one filesystem: readers see either the old content or the complete new
/// content, never half a file. The temp name carries the process id so concurrent writers don't stomp.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    // On Windows rename doesn't overwrite an existing target — delete first (failure is fine; rename will error again).
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
    use tokscale_core::ParsedMessage;

    /// Temp ledger path: process id + atomic counter, so parallel tests don't collide.
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
            project: None,
            title: None,
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 1,
            cost: 0.0,
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

    /// Build a daily bucket: `(date, input, output)`.
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

    /// Key format: `client_label:session_id`.
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

    /// Totals exclude cache_write_1h-style subsets (the field verifies the accounting here).
    #[test]
    fn total_sums_every_bucket_except_subsets() {
        let mut s = snapshot("a", 100, 20);
        s.cache_read = 1000;
        s.cache_write = 50;
        s.reasoning = 5;
        assert_eq!(s.total(), 1175);
    }

    /// A new session enters the ledger.
    #[test]
    fn new_session_is_recorded() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        assert_eq!(ledger.len(), 1);
        let view = ledger.view();
        assert_eq!(view[0].session_id, "s1");
        assert!(!view[0].archived);
    }

    /// Usage growth is merged in.
    #[test]
    fn growing_session_updates_in_place() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        ledger.merge_scan(scan_of(vec![snapshot("s1", 300, 40)]));
        assert_eq!(ledger.len(), 1, "同一个会话不应变成两条");
        assert_eq!(ledger.view()[0].input, 300);
        assert_eq!(ledger.view()[0].output, 40);
    }

    /// **The core requirement**: once a session vanishes from the sources, the ledger keeps its usage but **keeps it out of the display view**.
    #[test]
    fn deleted_session_survives_in_ledger() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![
            snapshot("keep", 100, 10),
            snapshot("gone", 500, 50),
        ]));
        // Next scan: gone vanished
        ledger.merge_scan(scan_of(vec![snapshot("keep", 150, 15)]));

        assert_eq!(ledger.len(), 2, "两条都还在账本里");
        assert_eq!(ledger.archived_len(), 1, "消失的那条标记为已消失");
        let view = ledger.view();
        assert_eq!(view.len(), 1, "源里已消失的不进列表");
        assert_eq!(view[0].session_id, "keep");
        assert!(!view[0].archived, "仍在源里的不应标记");
    }

    /// An archived session reappearing → back in the list.
    #[test]
    fn reappearing_session_clears_archived() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        ledger.merge_scan(scan_of(vec![]));
        assert!(ledger.view().is_empty(), "源里没了就不列");
        assert_eq!(ledger.archived_len(), 1);
        ledger.merge_scan(scan_of(vec![snapshot("s1", 100, 10)]));
        assert_eq!(ledger.view().len(), 1, "会话回来后要回到列表");
        assert!(!ledger.view()[0].archived, "会话回来后不应再标记");
        assert_eq!(ledger.archived_len(), 0);
    }

    /// Source-side dips (a rescan reading smaller values) don't shrink totals.
    #[test]
    fn shrinking_scan_does_not_reduce_totals() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("s1", 1000, 100)]));
        ledger.merge_scan(scan_of(vec![snapshot("s1", 10, 1)]));
        let view = ledger.view();
        assert_eq!(view[0].input, 1000, "用量只增不减");
        assert_eq!(view[0].output, 100);
    }

    /// Same session_id across agents never interfere.
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

    /// Flush → reopen: deleted sessions' usage survives.
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
            // The archived flag written before reopening must read back: the source-vanished one stays off the list.
            assert_eq!(ledger.view().len(), 2, "两条都还在源里");
            assert_eq!(ledger.archived_len(), 0, "重开时两条都是活的");
            // A new round: gone has vanished from the sources
            ledger.merge_scan(scan_of(vec![snapshot("keep", 100, 10)]));
            let view = ledger.view();
            assert_eq!(view.len(), 1, "源里已消失的不进列表");
            assert_eq!(view[0].session_id, "keep");
            assert_eq!(ledger.len(), 2, "重开后已删会话仍留在账本里");
            assert_eq!(ledger.archived_len(), 1);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Flushing records the data timestamp: the instant-display cache's "updated" note depends on it.
    #[test]
    fn save_records_saved_at() {
        let path = temp_path("saved-at");
        let mut ledger = Ledger::open(&path);
        ledger.merge_scan(scan_of(vec![snapshot("a", 100, 10)]));
        assert!(ledger.saved_at().is_none(), "落盘前没有可靠的数据时刻");
        ledger.save().expect("落盘应成功");
        let saved = ledger.saved_at().expect("落盘后应记下数据时刻");
        let now = unix_now_ms();
        assert!(
            saved > now - 60_000 && saved <= now,
            "数据时刻应是刚刚: {saved}"
        );
        // Reopen reads it back.
        let reopened = Ledger::open(&path);
        assert_eq!(reopened.saved_at(), Some(saved));
        let _ = std::fs::remove_file(&path);
    }

    /// Old format (no saved_at key) falls back to file mtime: upgrading keeps the "updated" note, no version bump.
    #[test]
    fn legacy_ledger_without_saved_at_falls_back_to_mtime() {
        let path = temp_path("legacy-saved-at");
        std::fs::write(&path, r#"{"version":1,"sessions":{},"ignored":[]}"#).expect("应能写入");
        let ledger = Ledger::open(&path);
        assert_eq!(
            ledger.saved_at(),
            file_mtime_ms(&path),
            "缺 saved_at 键时回落文件 mtime"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Daily buckets survive reopen (heatmap data must not vanish on restart).
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

    /// Old ledgers (without the `daily` key) must not lose session history.
    ///
    /// Deleted sessions' usage can never be rescanned — that's the ledger's whole point; the newly added
    /// optional `daily` field may only be appended, never washed away by a version bump.
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

    /// Rescanning the same day takes the larger value: occasional source-side dips don't shrink history.
    #[test]
    fn daily_merge_never_shrinks() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_daily(daily_of(&[("2026-01-01", 1000, 100)]));
        ledger.merge_daily(daily_of(&[("2026-01-01", 10, 1)]));
        assert_eq!(ledger.daily()["2026-01-01"].totals.input, 1000);
    }

    /// With daily present and no new changes, `save` performs no IO (delta write).
    #[test]
    fn daily_unchanged_scan_does_not_dirty() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_daily(daily_of(&[("2026-01-01", 100, 10)]));
        ledger.save().expect("内存账本落盘应成功");
        assert!(!ledger.is_dirty(), "落盘后应变干净");

        // The same day's identical data again: not a change.
        ledger.merge_daily(daily_of(&[("2026-01-01", 100, 10)]));
        assert!(!ledger.is_dirty(), "无变化不应标脏");
    }

    /// Corrupt ledger file → silent rebuild, no panic.
    #[test]
    fn corrupt_ledger_recovers_silently() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ this is not json").unwrap();
        let ledger = Ledger::open(&path);
        assert!(ledger.is_empty(), "损坏文件应被当成空账本");

        // Truncated JSON gets the same treatment
        std::fs::write(&path, "{\"version\":1,\"sessions\":{\"a\":").unwrap();
        assert!(Ledger::open(&path).is_empty());

        // Binary garbage
        std::fs::write(&path, [0u8, 159, 146, 150]).unwrap();
        assert!(Ledger::open(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// Version mismatch → discard and rebuild (dirty data is never migrated).
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

    /// Entries whose key and content disagree are dropped.
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

    /// No changes → no flush (no IO).
    #[test]
    fn save_is_noop_when_clean() {
        let path = temp_path("clean");
        let mut ledger = Ledger::open(&path);
        ledger.save().expect("无改动落盘应成功");
        assert!(!path.exists(), "无改动不应创建文件");
    }

    /// Atomic write: no temp files left in the directory after flushing.
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

    /// Missing directories are created automatically.
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

    /// The display view sorts by most recent activity.
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

    /// Zero-usage sessions stay out of the ledger (no piling up empty entries).
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

    /// Readable after flushing: client / session_id visible in the JSON.
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

    /// Delete a session: removed from the ledger, into the ignore list, never merged back by later scans.
    #[test]
    fn ignore_session_removes_and_blocks_resurrection() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![
            snapshot("keep", 100, 10),
            snapshot("gone", 500, 50),
        ]));
        let key = SessionSnapshot::key(ConfigFormat::Pi, "gone");
        assert!(ledger.ignore_session(&key));
        assert_eq!(ledger.len(), 1, "删掉的会话应从账本移除");
        assert!(ledger.ignored().contains(&key));
        assert!(!ledger.ignore_session(&key), "重复删同一键返回 false");

        // The next scan still sees it: no revival allowed.
        ledger.merge_scan(scan_of(vec![
            snapshot("keep", 100, 10),
            snapshot("gone", 500, 50),
        ]));
        assert_eq!(ledger.len(), 1, "被忽略的会话不许复活");
        assert!(!ledger.view().iter().any(|s| s.session_id == "gone"));
    }

    /// Delete a session: details (snapshot + per-day `by_session`) really deleted,
    /// totals and breakdowns kept — totals don't shrink from deletion, and the ledger doesn't bloat with session count.
    #[test]
    fn ignore_session_keeps_daily_contribution() {
        let mut ledger = Ledger::in_memory();
        ledger.merge_scan(scan_of(vec![snapshot("gone", 500, 50)]));
        let key = SessionSnapshot::key(ConfigFormat::Pi, "gone");
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.totals.input = 500;
        bucket.totals.output = 50;
        bucket.totals.messages = 1;
        bucket.by_session.insert(key.clone(), bucket.totals);
        bucket
            .by_model
            .insert("test-model".to_string(), bucket.totals);
        bucket.by_client.insert("pi".to_string(), bucket.totals);
        bucket
            .by_client_model
            .entry("pi".to_string())
            .or_default()
            .insert("test-model".to_string(), bucket.totals);
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".to_string(), bucket);
        ledger.merge_daily(daily);

        ledger.ignore_session(&key);
        let day = ledger.daily().get("2026-01-01").unwrap();
        assert_eq!(day.totals.input, 500, "总量不因删除缩水");
        assert_eq!(day.by_model["test-model"].input, 500, "按模型保留");
        assert_eq!(day.by_client["pi"].input, 500, "按 agent 保留");
        assert_eq!(day.by_client_model["pi"]["test-model"].input, 500);
        assert!(
            !day.by_session.contains_key(&key),
            "会话明细真删（留着会随会话数无上限增长）"
        );
    }

    /// In-flight scan detail pruning: the scan's ignore list didn't yet contain the key deleted mid-flight,
    /// and the result's details must not wash the deletion away; totals merge as usual.
    #[test]
    fn merge_daily_drops_ignored_session_details() {
        let mut ledger = Ledger::in_memory();
        let key = SessionSnapshot::key(ConfigFormat::Pi, "gone");
        ledger.merge_scan(scan_of(vec![snapshot("gone", 500, 50)]));
        ledger.ignore_session(&key);

        // Simulate an in-flight scan's result: details still present, totals still the session's usage.
        let mut bucket = DailyBucket::new("2026-01-02");
        bucket.totals.input = 500;
        bucket.totals.messages = 1;
        bucket.by_session.insert(key.clone(), bucket.totals);
        bucket.by_client.insert("pi".to_string(), bucket.totals);
        let mut daily = DailyMap::new();
        daily.insert("2026-01-02".to_string(), bucket);
        ledger.merge_daily(daily);

        let day = ledger.daily().get("2026-01-02").unwrap();
        assert_eq!(day.totals.input, 500, "合计照并");
        assert_eq!(day.by_client["pi"].input, 500, "分解照并");
        assert!(
            !day.by_session.contains_key(&key),
            "在途扫描的明细也要按名单摘掉"
        );
    }

    /// The ignore list persists: still effective after restart.
    #[test]
    fn ignored_list_survives_a_roundtrip() {
        let path = temp_path("ignored");
        let mut ledger = Ledger::open(&path);
        ledger.merge_scan(scan_of(vec![snapshot("gone", 1, 1)]));
        ledger.ignore_session(&SessionSnapshot::key(ConfigFormat::Pi, "gone"));
        ledger.save().unwrap();
        drop(ledger);

        let mut reopened = Ledger::open(&path);
        assert_eq!(reopened.len(), 0);
        assert!(reopened
            .ignored()
            .contains(&SessionSnapshot::key(ConfigFormat::Pi, "gone")));
        reopened.merge_scan(scan_of(vec![snapshot("gone", 1, 1)]));
        assert_eq!(reopened.len(), 0, "重启后仍不复活");
        let _ = std::fs::remove_file(&path);
    }

    /// The project field: absorb backfills from the message's workspace, and the JSON round trip keeps it.
    #[test]
    fn project_flows_from_message_through_json() {
        let message_with = |label: Option<&str>, key: Option<&str>| ParsedMessage {
            client: "pi".to_string(),
            model_id: "m".to_string(),
            provider_id: String::new(),
            session_id: "s1".to_string(),
            workspace_key: key.map(str::to_string),
            workspace_label: label.map(str::to_string),
            timestamp: 0,
            date: String::new(),
            input: 1,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
            duration_ms: None,
            message_count: 1,
            agent: None,
            cost: 0.0,
            cost_source: Default::default(),
            service_tier: None,
        };
        let mut snap = snapshot("s1", 1, 0);
        snap.absorb(&message_with(Some("ModelHarbor"), Some("/f/ModelHarbor")));
        assert_eq!(snap.project.as_deref(), Some("ModelHarbor"), "label 优先");

        snap.absorb(&message_with(None, Some("/f/Other")));
        assert_eq!(
            snap.project.as_deref(),
            Some("/f/Other"),
            "缺 label 退回 key"
        );

        snap.absorb(&message_with(None, None));
        assert_eq!(
            snap.project.as_deref(),
            Some("/f/Other"),
            "空消息不清掉已知的"
        );

        let json = snap.to_json();
        let back = SessionSnapshot::from_json(&SessionSnapshot::key(ConfigFormat::Pi, "s1"), &json)
            .expect("往返应可读");
        assert_eq!(back.project, snap.project);
    }

    /// Cost: absorbed additively, preserved through the JSON round trip; old entries (no cost key) read back as 0.
    #[test]
    fn cost_flows_through_absorb_and_json() {
        let mut snap = snapshot("costly", 1, 1);
        let priced = ParsedMessage {
            client: "pi".to_string(),
            model_id: "m".to_string(),
            provider_id: String::new(),
            session_id: "costly".to_string(),
            workspace_key: None,
            workspace_label: None,
            timestamp: 0,
            date: String::new(),
            input: 1,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
            duration_ms: None,
            message_count: 1,
            agent: None,
            cost: 0.75,
            cost_source: Default::default(),
            service_tier: None,
        };
        snap.absorb(&priced);
        snap.absorb(&priced);
        assert!((snap.cost - 1.5).abs() < 1e-9, "两条各 0.75 应累加");

        let back = SessionSnapshot::from_json(
            &SessionSnapshot::key(ConfigFormat::Pi, "costly"),
            &snap.to_json(),
        )
        .expect("往返应可读");
        assert!((back.cost - 1.5).abs() < 1e-9);

        // Old ledger entries lack the cost key: read back as 0, not flagged corrupt.
        let mut legacy = snap.to_json();
        legacy.as_object_mut().unwrap().remove("cost");
        let old =
            SessionSnapshot::from_json(&SessionSnapshot::key(ConfigFormat::Pi, "costly"), &legacy)
                .expect("旧条目应可读");
        assert_eq!(old.cost, 0.0);
    }
}
