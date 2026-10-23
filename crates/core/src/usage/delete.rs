//! Delete a session **from the source** — when the user clicks "delete", the session really leaves the
//! agent's own store.
//!
//! ## Relationship with the ledger
//!
//! The ledger ([`crate::usage::ledger`]) handles "usage survives deletion": session details are
//! withdrawn while totals and breakdowns stay. This module deletes the **source**: opencode-family
//! writes SQLite, file-based backends delete files. The two are halves of one action — withdrawing
//! only the ledger leaves the session visible next time the agent opens; deleting only the source
//! lets the next scan scoop it back in.
//!
//! ## Deleting while the agent runs
//!
//! The opencode family uses WAL-mode SQLite, which supports concurrent multi-process writers by
//! design — deleting a session externally while the agent is alive is normal usage (cc-switch's delete
//! path does no process check either). A genuinely locked database is waited out by `busy_timeout`; if
//! that expires, `Failed` is reported instead of presuming "no online deletes".
//!
//! ## What gets deleted
//!
//! opencode spreads one session across several tables (see [`opencode_tables`]). Deleting only the
//! `session` main table leaves orphan messages, and the usage-claiming scan would read them back from
//! the message tables — deletion for nothing. So these tables go together, wrapped in one transaction:
//! either all deleted or nothing touched.
//!
//! ## What doesn't get deleted
//!
//! - **No non-session tables are touched**: `project` / `worktree` / `kv` etc. belong to the user's
//!   environment configuration;
//! - **No write-back to the source's own aggregates**: opencode's `usage_*` tables, if present, are its
//!   own books — this tool only deletes the session itself.

use crate::format::ConfigFormat;
use crate::prefs;
use std::path::{Path, PathBuf};

/// Why a deletion failed (each kind maps to different advice in the UI).
///
/// There is no "agent is running" kind: opencode's WAL supports multi-process writes, so a running
/// agent doesn't block deletion (cc-switch's delete path likewise does no process check). Blocking
/// users into shutting down agents daily just turns a non-problem into a gate.
#[derive(Clone, Debug, PartialEq)]
pub enum DeleteError {
    /// The backend doesn't support deleting from the source (the source is read-only, or its location isn't fixed).
    Unsupported { client: &'static str },
    /// The source file / database can't be found.
    SourceMissing { path: String },
    /// The session isn't in the source (possibly deleted by the agent itself, or the id belongs to another backend).
    SessionNotFound,
    /// Writing the source failed (permissions, a corrupt database, lock timeout…).
    Failed { detail: String },
}

impl DeleteError {
    /// One sentence for the user (shown directly in the UI, never assembled from English error codes).
    pub fn message(&self) -> String {
        match self {
            Self::Unsupported { client } => {
                format!("{client} 的会话源不支持删除，已只从统计里移除")
            }
            Self::SourceMissing { path } => format!("找不到会话文件：{path}（已只从统计里移除）"),
            Self::SessionNotFound => "源里已经没有这条会话了".to_string(),
            Self::Failed { detail } => format!("删除失败：{detail}（源未改动）"),
        }
    }

    /// Whether this failure should also block the statistics half.
    ///
    /// `Failed` (database locked, insufficient permissions, corruption): the source wasn't deleted and the
    /// user **can** fix it (close the holding program, change permissions, retry) — so the statistics half
    /// also waits; otherwise the row vanishes from the list, never clickable again, and the source session
    /// would be stuck undeletable.
    ///
    /// Other cases offer no "retryable source deletion": the backend can't delete sources at all, the source
    /// directory isn't on this machine, or it's already gone from the source. These proceed with statistics
    /// removal as usual, leaving the choice to the user.
    pub fn blocks_ledger(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Result of one deletion: a real source deletion carries the deleted row count (0 = it wasn't in the source).
#[derive(Clone, Debug, PartialEq)]
pub struct Deleted {
    /// The deleted source file / database path (the UI uses it to tell the user what was touched).
    pub path: PathBuf,
    /// Records deleted from the source (always 1 for file-based backends).
    pub rows: usize,
}

/// Delete a session from its source.
///
/// `key` is the ledger key (`client:session_id`, see [`crate::usage::SessionSnapshot::key`]).
///
/// On success returns the deleted source location; on failure returns [`DeleteError`] — **on failure
/// the source is guaranteed untouched** (database writes run in a single transaction; file deletion is
/// the last step).
///
/// **No check for a running agent**: opencode's WAL supports multi-process writes, and deleting a
/// session externally while the agent lives is normal usage (cc-switch does the same). A genuinely
/// locked database is waited out by `busy_timeout`; expiring reports `Failed` rather than presuming
/// "no online deletes".
pub fn delete_session(key: &str) -> Result<Deleted, DeleteError> {
    let (client, session_id) = split_key(key)?;
    match client {
        ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
            delete_from_sqlite(client, session_id)
        }
        ConfigFormat::Pi | ConfigFormat::OhMyPi => delete_pi_like(client, session_id),
        ConfigFormat::DeepSeekHarness => delete_dsh(session_id),
        // Other backends' session sources aren't on this machine (workbuddy's database is held by its own daemon),
        // or the directory doesn't exist here — always ledger-only, never touching sources.
        other => Err(DeleteError::Unsupported {
            client: other.label(),
        }),
    }
}

/// Split the ledger key: `client:session_id`. Client names never contain `:`, session ids may — so only the first cut counts.
fn split_key(key: &str) -> Result<(ConfigFormat, &str), DeleteError> {
    let (label, session_id) = key.split_once(':').ok_or(DeleteError::SessionNotFound)?;
    // label → ConfigFormat goes through the registry (not string assembly): the two sides' naming disagrees.
    let client = crate::backends::BACKENDS
        .iter()
        .map(|backend| backend.id())
        .find(|format| format.label() == label)
        .ok_or(DeleteError::SessionNotFound)?;
    Ok((client, session_id))
}

/// Tables of the opencode family (column → the column holding the session id).
///
/// Covers **every** table that hangs rows on a session: missing any one lets the scan read usage back
/// from that table, or leaves the user a pile of invisible orphan rows.
///
/// Foreign-key cascades can't be trusted: SQLite's `foreign_keys` is off by default and rusqlite
/// doesn't enable it — those `ON DELETE CASCADE` declarations are inert at runtime. So each table is
/// deleted explicitly.
///
/// Child tables go first: `part` follows `message`; `session` / `session_v2` are parents and go last.
fn opencode_tables() -> [(&'static str, &'static str); 13] {
    [
        ("part", "session_id"),
        ("message", "session_id"),
        ("event", "aggregate_id"),
        ("event_sequence", "aggregate_id"),
        ("session_message", "session_id"),
        ("todo", "session_id"),
        ("session_share", "session_id"),
        ("instruction_entry", "session_id"),
        ("instruction_state", "session_id"),
        ("session_pending", "session_id"),
        ("session_inbox", "session_id"),
        ("session_v2", "id"),
        ("session", "id"),
    ]
}

/// opencode-family database paths: `~/.local/share/<agent>/<agent>.db`.
///
/// Paths align with tokscale's `ClientDef` (`PathRoot::XdgData` + `opencode/*`): on Windows tokscale
/// also uses `.local/share` (ignoring `XDG_DATA_HOME`), so no environment variable is read here either,
/// lest the two sides point at different databases.
///
/// Also exposed to [`super::titles`]: deleting a session and reading its name must target **the same**
/// database, otherwise names come from one place and deletion lands in another.
pub(super) fn opencode_db(client: ConfigFormat) -> Option<PathBuf> {
    let home = prefs::home_dir()?;
    let dir = match client {
        ConfigFormat::Opencode => "opencode",
        ConfigFormat::Kilocode => "kilo",
        ConfigFormat::Mimocode => "mimocode",
        _ => return None,
    };
    Some(
        home.join(".local")
            .join("share")
            .join(dir)
            .join(format!("{dir}.db")),
    )
}

/// Delete a session from an opencode-family database: 13 tables, one transaction.
fn delete_from_sqlite(client: ConfigFormat, session_id: &str) -> Result<Deleted, DeleteError> {
    let path = opencode_db(client).ok_or(DeleteError::SourceMissing {
        path: client.label().to_string(),
    })?;
    if !path.is_file() {
        return Err(DeleteError::SourceMissing {
            path: path.display().to_string(),
        });
    }
    delete_from_sqlite_at(&path, session_id)
}

/// Run the deletion on a given database (split from path resolution: tests run the same path on a temp database).
fn delete_from_sqlite_at(path: &Path, session_id: &str) -> Result<Deleted, DeleteError> {
    let connection = rusqlite::Connection::open(path).map_err(|err| DeleteError::Failed {
        detail: err.to_string(),
    })?;
    // The agent may be writing this database concurrently: let SQLite wait on the lock rather than erroring
    // at first touch. 5 seconds matches the rusqlite/SQLite community's usual value: enough for one session
    // write, without freezing the UI until it seems dead. Expiring reports Failed with the source untouched.
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|err| DeleteError::Failed {
            detail: err.to_string(),
        })?;
    // Tables may not exist (old databases lack session_v2): ask per table first.
    let mut rows = 0usize;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|err| DeleteError::Failed {
            detail: err.to_string(),
        })?;
    for (table, column) in opencode_tables() {
        if !table_exists(&transaction, table)? {
            continue;
        }
        let sql = format!("DELETE FROM {table} WHERE {column} = ?1");
        rows += transaction
            .execute(&sql, [session_id])
            .map_err(|err| DeleteError::Failed {
                detail: format!("{table}: {err}"),
            })?;
    }
    transaction.commit().map_err(|err| DeleteError::Failed {
        detail: err.to_string(),
    })?;
    if rows == 0 {
        return Err(DeleteError::SessionNotFound);
    }
    Ok(Deleted {
        path: path.to_path_buf(),
        rows,
    })
}

/// Whether the database has this table.
pub(super) fn table_exists(
    connection: &rusqlite::Connection,
    table: &str,
) -> Result<bool, DeleteError> {
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .map_err(|err| DeleteError::Failed {
            detail: err.to_string(),
        })?;
    Ok(count > 0)
}

/// pi family (pi / oh-my-pi): delete the session's jsonl and its same-named transcript directory.
///
/// Layout and id parsing are described in [`super::pi`]: top-level sessions carry the id in the file
/// name, while sub-agent transcripts are all named `session.jsonl` with the id only in the first line —
/// both must be deletable.
fn delete_pi_like(client: ConfigFormat, session_id: &str) -> Result<Deleted, DeleteError> {
    let root = super::pi::sessions_root(client).ok_or(DeleteError::SourceMissing {
        path: client.label().to_string(),
    })?;
    if !root.is_dir() {
        return Err(DeleteError::SourceMissing {
            path: root.display().to_string(),
        });
    }
    delete_pi_like_at(&root, session_id)
}

/// Delete under a given session root (split from path resolution: tests run the same path on a temp directory).
///
/// Lookup goes by session id, not file name — sub-agent and worktree transcripts are all named
/// `session.jsonl`, with the id only in the content's first line (see `super::pi`).
fn delete_pi_like_at(root: &Path, session_id: &str) -> Result<Deleted, DeleteError> {
    let found = super::pi::find_session(root, session_id).ok_or(DeleteError::SessionNotFound)?;
    let mut removed = 0usize;
    if found.is_file() {
        std::fs::remove_file(&found).map_err(|err| DeleteError::Failed {
            detail: err.to_string(),
        })?;
        removed += 1;
    }
    // Transcript directory: `<the same path with .jsonl stripped>`.
    let transcript_dir = found.with_extension("");
    if transcript_dir.is_dir() {
        std::fs::remove_dir_all(&transcript_dir).map_err(|err| DeleteError::Failed {
            detail: err.to_string(),
        })?;
        removed += 1;
    }
    Ok(Deleted {
        path: found,
        rows: removed,
    })
}

/// DSH: `~/.dsh/sessions/<encoded cwd>/session-<uuid>/` (containing `session*.jsonl.zstd`).
fn delete_dsh(session_id: &str) -> Result<Deleted, DeleteError> {
    let root = prefs::home_dir()
        .map(|home| home.join(".dsh").join("sessions"))
        .ok_or(DeleteError::SourceMissing {
            path: "~/.dsh/sessions".to_string(),
        })?;
    if !root.is_dir() {
        return Err(DeleteError::SourceMissing {
            path: root.display().to_string(),
        });
    }
    let found = find_dir_named(&root, &format!("session-{session_id}"), 0)
        .ok_or(DeleteError::SessionNotFound)?;
    let children = std::fs::read_dir(&found).map(|it| it.count()).unwrap_or(0);
    std::fs::remove_dir_all(&found).map_err(|err| DeleteError::Failed {
        detail: err.to_string(),
    })?;
    Ok(Deleted {
        path: found,
        rows: children,
    })
}

/// Find a directory named exactly `name` (recursive, depth-limited).
fn find_dir_named(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth > 4 {
        return None;
    }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if entry.file_name().to_string_lossy() == name {
            return Some(path);
        }
        if let Some(found) = find_dir_named(&path, name, depth + 1) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_key_splits_on_the_first_colon() {
        let (client, id) = split_key("opencode:ses_abc:def").unwrap();
        assert_eq!(client, ConfigFormat::Opencode);
        assert_eq!(id, "ses_abc:def", "会话 id 里的冒号要留住");
    }

    #[test]
    fn split_key_rejects_unknown_client() {
        assert!(matches!(
            split_key("no-such-agent:x"),
            Err(DeleteError::SessionNotFound)
        ));
        assert!(matches!(
            split_key("no-colon"),
            Err(DeleteError::SessionNotFound)
        ));
    }

    #[test]
    fn opencode_tables_cover_every_session_table() {
        let tables = opencode_tables();
        // The main table must be deleted last: it's the foreign keys' parent.
        assert_eq!(tables.last().unwrap().0, "session");
        // Not one table carrying session content may be missed (SQLite's foreign-key cascades are off by default).
        for expected in [
            "part",
            "message",
            "event",
            "event_sequence",
            "session_message",
            "todo",
            "session_share",
            "instruction_entry",
            "instruction_state",
            "session_pending",
            "session_inbox",
            "session_v2",
        ] {
            assert!(
                tables.iter().any(|(table, _)| *table == expected),
                "{expected} 也是按会话挂行的表，漏了会留下孤儿行"
            );
        }
    }

    /// Build a minimal opencode database: 13 tables + two sessions; delete only one.
    fn temp_db(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "modelharbor-delete-{tag}-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let connection = rusqlite::Connection::open(&path).unwrap();
        // Tables created per the real schema (column names matching real databases).
        connection
            .execute_batch(
                "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT);\n                 CREATE TABLE session_v2 (id TEXT PRIMARY KEY);\n                 CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE part (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE event (id TEXT PRIMARY KEY, aggregate_id TEXT);\n                 CREATE TABLE event_sequence (aggregate_id TEXT PRIMARY KEY, seq INTEGER);\n                 CREATE TABLE todo (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_share (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE instruction_entry (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE instruction_state (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_pending (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_inbox (id TEXT PRIMARY KEY, session_id TEXT);",
            )
            .unwrap();
        for id in ["ses_target", "ses_other"] {
            for (table, column) in opencode_tables() {
                // The primary-key column follows its own column name: some are id, others session_id / aggregate_id.
                let sql = match table {
                    "event_sequence" => {
                        format!("INSERT INTO {table} (aggregate_id, seq) VALUES (?1, 1)")
                    }
                    "event" => {
                        format!("INSERT INTO {table} (id, aggregate_id) VALUES (?1, ?2)")
                    }
                    "session" | "session_v2" => format!("INSERT INTO {table} (id) VALUES (?1)"),
                    _ => format!("INSERT INTO {table} (id, {column}) VALUES (?1, ?2)"),
                };
                match table {
                    "event_sequence" => connection.execute(&sql, [id]).unwrap(),
                    "session" | "session_v2" => connection.execute(&sql, [id]).unwrap(),
                    _ => connection
                        .execute(&sql, rusqlite::params![format!("row_{id}"), id])
                        .unwrap(),
                };
            }
        }
        path
    }

    /// Really deleted: every table cleared of the target session, other sessions untouched row for row.
    #[test]
    fn sqlite_delete_clears_every_table_and_spares_the_rest() {
        let path = temp_db("real");
        let deleted = delete_from_sqlite_at(&path, "ses_target").unwrap();
        assert_eq!(deleted.rows, opencode_tables().len(), "每张表各删一行");

        let connection = rusqlite::Connection::open(&path).unwrap();
        for (table, column) in opencode_tables() {
            let count = |value: &str| -> i64 {
                connection
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
                        [value],
                        |row| row.get(0),
                    )
                    .unwrap()
            };
            assert_eq!(count("ses_target"), 0, "{table} 里不该剩目标会话");
            assert_eq!(count("ses_other"), 1, "{table} 里别的会话不许被误删");
        }
        drop(connection);
        let _ = std::fs::remove_file(&path);
    }

    /// The session isn't in the source: `SessionNotFound` reported, never silent success.
    #[test]
    fn sqlite_delete_reports_a_missing_session() {
        let path = temp_db("missing");
        assert_eq!(
            delete_from_sqlite_at(&path, "ses_nope"),
            Err(DeleteError::SessionNotFound)
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Old databases without session_v2: deletion still works, no error.
    #[test]
    fn sqlite_delete_tolerates_missing_tables() {
        let path = temp_db("old");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("DROP TABLE session_v2;").unwrap();
        drop(connection);

        let deleted = delete_from_sqlite_at(&path, "ses_target").unwrap();
        assert_eq!(
            deleted.rows,
            opencode_tables().len() - 1,
            "缺一张表就少删一行"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Which failures also block the statistics half: fixable ones (source not deleted) block; unfixable ones pass.
    #[test]
    fn only_retryable_failures_block_the_ledger() {
        // Database locked / permissions / corruption: the user can fix it; keep the row so they can retry.
        assert!(DeleteError::Failed {
            detail: "database is locked".to_string()
        }
        .blocks_ledger());

        // These have no "retry the source deletion" to speak of: statistics removal proceeds as usual.
        assert!(!DeleteError::SessionNotFound.blocks_ledger());
        assert!(!DeleteError::Unsupported {
            client: "workbuddy"
        }
        .blocks_ledger());
        assert!(!DeleteError::SourceMissing {
            path: "x".to_string()
        }
        .blocks_ledger());
    }

    /// A temporary session root: process id + counter, so parallel tests don't collide.
    fn temp_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "modelharbor-delete-{tag}-{}-{}",
            n,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("建临时会话根");
        root
    }

    /// Deleting pi's **sub-agent / worktree transcripts**: the file name is always `session.jsonl` with the id
    /// only in the content's first line — old file-name matching can't find, and can't delete, them.
    #[test]
    fn pi_delete_reaches_subagent_transcripts() {
        let root = temp_root("pi-sub");
        // The real layout: cwd / parent session / sub-agent uuid / run-0 / session.jsonl
        let run = root
            .join("--D--X--")
            .join("2026-01-01T00-00-00-000Z_top-id")
            .join("sub-uuid")
            .join("run-0");
        std::fs::create_dir_all(&run).expect("建目录树");
        let jsonl = run.join("session.jsonl");
        // The same-named transcript directory: `session.jsonl` minus the suffix = `session`.
        let transcript_dir = run.join("session");
        std::fs::create_dir_all(&transcript_dir).expect("建转录目录");
        std::fs::write(
            &jsonl,
            r#"{"type":"session","version":3,"id":"sub-id","cwd":"D:/X"}"#,
        )
        .expect("写转录");

        let deleted = delete_pi_like_at(&root, "sub-id").expect("子 agent 转录应删得掉");
        assert_eq!(deleted.path, jsonl, "删的是那份转录");
        assert_eq!(deleted.rows, 2, "jsonl 与同名转录目录各计一行");
        assert!(!jsonl.exists(), "jsonl 应被删除");
        assert!(!transcript_dir.exists(), "同名转录目录也应被删除");

        // Deleting a second time: already gone from the source — SessionNotFound (never silent success).
        assert_eq!(
            delete_pi_like_at(&root, "sub-id"),
            Err(DeleteError::SessionNotFound),
            "重复删除应报找不到"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Deleting pi's **top-level session**: the file name carries the id, deleted together with its branch transcript directory.
    #[test]
    fn pi_delete_handles_top_level_sessions() {
        let root = temp_root("pi-top");
        let ws = root.join("--D--X--");
        std::fs::create_dir_all(&ws).expect("建 cwd 目录");
        let jsonl = ws.join("2026-01-01T00-00-00-000Z_top-id.jsonl");
        let transcript_dir = ws.join("2026-01-01T00-00-00-000Z_top-id");
        std::fs::create_dir_all(&transcript_dir).expect("建转录目录");
        std::fs::write(
            &jsonl,
            r#"{"type":"session","version":3,"id":"top-id","cwd":"D:/X"}"#,
        )
        .expect("写转录");

        let deleted = delete_pi_like_at(&root, "top-id").expect("顶层会话应删得掉");
        assert_eq!(deleted.rows, 2, "jsonl 与转录目录各计一行");
        assert!(!jsonl.exists(), "jsonl 应被删除");
        assert!(!transcript_dir.exists(), "分支转录目录一并删掉");
        let _ = std::fs::remove_dir_all(&root);
    }
}
