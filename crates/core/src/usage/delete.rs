//! 从**源**里删除一个会话 —— 用户点「删除」时真把会话从 agent 自己的库里抹掉。
//!
//! ## 与账本的关系
//!
//! 账本（[`crate::usage::ledger`]）负责「删了之后用量还在」：会话明细摘掉，
//! 合计与各分解保留。本模块负责删**源**：opencode 系写 SQLite，文件式的删文件。
//! 两者是同一次动作的两半 —— 只摘账本不删源，用户下次打开 agent 仍能看到那条
//! 会话；只删源不摘账本，它会被扫描重新捞回来。
//!
//! ## agent 在跑也能删
//!
//! opencode 系用 WAL 模式的 SQLite，而 WAL 本就支持多进程并发写 —— agent 活着时
//! 从外部删会话是正常用法（cc-switch 的删除路径也不做进程检查）。库真被占着时
//! 由 `busy_timeout` 等一等，等不到就报 `Failed`，而不是提前假定「不许在线删」。
//!
//! ## 删什么
//!
//! opencode 把一次会话摊在多张表里（见 [`opencode_tables`]）。只删 `session`
//! 主表会留下孤儿消息，反占用量的扫描还会从 message 表把它们读回来，删除就
//! 白做了。所以这些表一起删，装在一次事务里：要么全删掉，要么一行都不动。
//!
//! ## 不删什么
//!
//! - **不改任何非会话表**：`project` / `worktree` / `kv` 等属于用户的环境配置；
//! - **不回写源方的统计聚合**：opencode 的 `usage_*` 之类如果存在，那是它自己
//!   的账，本工具只删会话本体。

use crate::format::ConfigFormat;
use crate::prefs;
use std::path::{Path, PathBuf};

/// 删除失败的原因（每一类都对应界面上不同的处置建议）。
///
/// 没有「agent 正在运行」这一类：opencode 的 WAL 支持多进程写，agent 在跑不影响
/// 删除（cc-switch 的删除路径同样不做进程检查）。成天拦着用户去关 agent，只是把
/// 一个不存在的问题当成门槛。
#[derive(Clone, Debug, PartialEq)]
pub enum DeleteError {
    /// 该后端不支持从源删除（源是只读的、或位置不固定）。
    Unsupported { client: &'static str },
    /// 找不到源文件 / 源库。
    SourceMissing { path: String },
    /// 源里没有这条会话（可能已被 agent 自己删掉，或 id 不属于该后端）。
    SessionNotFound,
    /// 写源失败（权限、库损坏、锁超时……）。
    Failed { detail: String },
}

impl DeleteError {
    /// 给用户看的一句话（界面直接显示，不拼英文错误码）。
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

    /// 这个失败是否要连统计那半也一起拦下。
    ///
    /// `Failed`（库被占着、权限不足、库损坏）源没删成、而用户**能**修
    /// （关掉占用的程序、换权限、重试），所以统计那半也不做 —— 否则行从列表
    /// 里消失、用户再也点不到它，源里的会话就水远删不掉了。
    ///
    /// 其余情形没有「可重试的源删除」可言：后端本就不支持删源、源目录不在
    /// 本机、源里已经没有它。这些照旧从统计里移除，把选择权留给用户。
    pub fn blocks_ledger(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// 一次删除的结果：真删了源就带上删掉的行数（0 表示源里本来就没有）。
#[derive(Clone, Debug, PartialEq)]
pub struct Deleted {
    /// 被删的源文件 / 库路径（界面用来告知用户动过什么）。
    pub path: PathBuf,
    /// 源里删掉的记录数（文件式恒为 1）。
    pub rows: usize,
}

/// 从源里删掉一个会话。
///
/// `key` 是账本键（`client:session_id`，见
/// [`crate::usage::SessionSnapshot::key`]）。
///
/// 成功返回删掉的源位置；失败返回 [`DeleteError`] —— **失败时源一定没被动过**
///（写库走单事务，文件删除是最后一步）。
///
/// **不检查 agent 是否在跑**：opencode 的 WAL 支持多进程写，agent 活着时
/// 从外部删会话是正常用法（cc-switch 也这么做）。库真被占着时由 `busy_timeout`
/// 等一等，等不到就报 `Failed`，而不是提前假定「不许在线删」。
pub fn delete_session(key: &str) -> Result<Deleted, DeleteError> {
    let (client, session_id) = split_key(key)?;
    match client {
        ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
            delete_from_sqlite(client, session_id)
        }
        ConfigFormat::Pi | ConfigFormat::OhMyPi => delete_pi_like(client, session_id),
        ConfigFormat::DeepSeekHarness => delete_dsh(session_id),
        // 其余后端的会话源不在本机（workbuddy 的库由它自己的守护进程持有），
        // 或者本机没有该目录 —— 一律只摘账本，不动源。
        other => Err(DeleteError::Unsupported {
            client: other.label(),
        }),
    }
}

/// 拆账本键：`client:session_id`。客户端名不含 `:`，会话 id 可能含，故只切第一刀。
fn split_key(key: &str) -> Result<(ConfigFormat, &str), DeleteError> {
    let (label, session_id) = key.split_once(':').ok_or(DeleteError::SessionNotFound)?;
    // label → ConfigFormat 走注册表（不是拼字符串）：两边的命名并不一致。
    let client = crate::backends::BACKENDS
        .iter()
        .map(|backend| backend.id())
        .find(|format| format.label() == label)
        .ok_or(DeleteError::SessionNotFound)?;
    Ok((client, session_id))
}

/// 该后端是否有进程正在使用它的会话源；有就返回进程名。
///
/// Windows 上用 `tasklist` 查：`windows-sys` 的进程枚举要跑一圈 Toolhelp 快照，
/// 代码量比一次 `tasklist` 大得多，而删除是低频动作，不值得为它加依赖。
/// opencode 系要删的表（列 → 会话 id 所在列名）。
///
/// 覆盖**全部**按会话挂行的表：漏掉任何一张，扫描都会从那张表里把用量读回来，
/// 或者给用户留下一堆看不见的孤儿行。
///
/// 不能指望外键级联：SQLite 的 `foreign_keys` 默认关闭，rusqlite 也不开，
/// 那些 `ON DELETE CASCADE` 声明在运行时是不生效的。所以逐表显式删。
///
/// 顺序上子表在前：`part` 跟着 `message` 走，`session` / `session_v2` 是父，
/// 放最后。
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

/// opencode 系的库路径：`~/.local/share/<agent>/<agent>.db`。
///
/// 路径与 tokscale 的 `ClientDef` 对齐（`PathRoot::XdgData` + `opencode/*`）：
/// Windows 上 t okscale 也用 `.local/share`（不吃 `XDG_DATA_HOME`），
/// 所以这里同样不读环境变量，免得两边指向不同的库。
///
/// 对 [`super::titles`] 也开放：删会话与读会话名要指向**同一个**库，
/// 否则名字来自一处、删除落在另一处。
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

/// 从 opencode 系的库里删会话：13 张表一次事务。
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

/// 在指定的库上执行删除（与路径解析分开：测试要拿临时库跑同一条路径）。
fn delete_from_sqlite_at(path: &Path, session_id: &str) -> Result<Deleted, DeleteError> {
    let connection = rusqlite::Connection::open(path).map_err(|err| DeleteError::Failed {
        detail: err.to_string(),
    })?;
    // agent 可能在同时写这个库：让 SQLite 自己等锁，而不是一碰就报错。
    // 5 秒与 rusqlite/SQLite 社区的常用取值一致：够写完一次会话，
    // 又不会让界面卡到用户以为死掉。等不到就报 Failed，源未被改动。
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|err| DeleteError::Failed {
            detail: err.to_string(),
        })?;
    // 表可能不存在（老版本库没有 session_v2）：逐表先问有没有。
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

/// 库里有没有这张表。
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

/// pi 系（pi / oh-my-pi）：删会话的 jsonl 与它的同名转录目录。
///
/// 布局与 id 的解析见 [`super::pi`]：顶层会话的文件名带 id，子 agent 转录
/// 一律叫 `session.jsonl`、id 只在首行，两者都要能删。
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

/// 在指定的会话根下删除（与路径解析分开：测试要拿临时目录跑同一条路径）。
///
/// 查找按会话 id 而非文件名 —— 子 agent 与 worktree 的转录一律叫
/// `session.jsonl`，id 只在内容首行（见 `super::pi`）。
fn delete_pi_like_at(root: &Path, session_id: &str) -> Result<Deleted, DeleteError> {
    let found = super::pi::find_session(root, session_id).ok_or(DeleteError::SessionNotFound)?;
    let mut removed = 0usize;
    if found.is_file() {
        std::fs::remove_file(&found).map_err(|err| DeleteError::Failed {
            detail: err.to_string(),
        })?;
        removed += 1;
    }
    // 转录目录：`<同样的路径去掉 .jsonl 后缀>`。
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

/// DSH：`~/.dsh/sessions/<编码 cwd>/session-<uuid>/`（含 `session*.jsonl.zstd`）。
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

/// 找名字正好是 `name` 的目录（递归，限深）。
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
        // 主表必须最后删：它是外键的父。
        assert_eq!(tables.last().unwrap().0, "session");
        // 会话内容的表一张都不能漏（SQLite 的外键级联默认不生效）。
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

    /// 造一个最小 opencode 库：13 张表 + 两条会话，只删其中一条。
    fn temp_db(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "modelharbor-delete-{tag}-{}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let connection = rusqlite::Connection::open(&path).unwrap();
        // 按真实 schema 建表（列名与真实库一致）。
        connection
            .execute_batch(
                "CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT);\n                 CREATE TABLE session_v2 (id TEXT PRIMARY KEY);\n                 CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE part (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE event (id TEXT PRIMARY KEY, aggregate_id TEXT);\n                 CREATE TABLE event_sequence (aggregate_id TEXT PRIMARY KEY, seq INTEGER);\n                 CREATE TABLE todo (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_share (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE instruction_entry (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE instruction_state (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_pending (id TEXT PRIMARY KEY, session_id TEXT);\n                 CREATE TABLE session_inbox (id TEXT PRIMARY KEY, session_id TEXT);",
            )
            .unwrap();
        for id in ["ses_target", "ses_other"] {
            for (table, column) in opencode_tables() {
                // 主键列跟着所在列名走：有的是 id，有的是 session_id / aggregate_id。
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

    /// 真删：每张表都清掉目标会话，其他会话一行不动。
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

    /// 源里没有这条会话：报 `SessionNotFound`，不是静默成功。
    #[test]
    fn sqlite_delete_reports_a_missing_session() {
        let path = temp_db("missing");
        assert_eq!(
            delete_from_sqlite_at(&path, "ses_nope"),
            Err(DeleteError::SessionNotFound)
        );
        let _ = std::fs::remove_file(&path);
    }

    /// 老库没有 session_v2：照样能删，不报错。
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

    /// 哪些失败要连统计那半一起拦下：可修的（源没删成）拦，不可修的放行。
    #[test]
    fn only_retryable_failures_block_the_ledger() {
        // 库被占着 / 权限不足 / 库坏了：用户能修，行要留着让他重试。
        assert!(DeleteError::Failed {
            detail: "database is locked".to_string()
        }
        .blocks_ledger());

        // 这几种没有「重试源删除」可言：照旧从统计里移除。
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

    /// 临时会话根：进程 id + 计数器，避免并行测试撞目录。
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

    /// 删 pi 的**子 agent / worktree 转录**：文件名一律是 `session.jsonl`，
    /// id 只在内容首行 —— 按文件名匹配的老逻辑找不到它，删不掉。
    #[test]
    fn pi_delete_reaches_subagent_transcripts() {
        let root = temp_root("pi-sub");
        // 真实布局：cwd / 父会话 / 子 agent uuid / run-0 / session.jsonl
        let run = root
            .join("--D--X--")
            .join("2026-01-01T00-00-00-000Z_top-id")
            .join("sub-uuid")
            .join("run-0");
        std::fs::create_dir_all(&run).expect("建目录树");
        let jsonl = run.join("session.jsonl");
        // 同名转录目录：`session.jsonl` 去掉后缀 = `session`。
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

        // 删第二次：源里已没有，报 SessionNotFound（不静默成功）。
        assert_eq!(
            delete_pi_like_at(&root, "sub-id"),
            Err(DeleteError::SessionNotFound),
            "重复删除应报找不到"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 删 pi 的**顶层会话**：文件名带 id，连带它的分支转录目录一起删。
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
