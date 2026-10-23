//! Session title collection — tokscale's parsing channel returns only tokens and models; titles must be
//! read from the sources separately.
//!
//! ## Where each client's title comes from
//!
//! - **opencode family** (opencode / kilocode / mimocode): `session.title` and `session_v2.title` in the
//!   database — the authoritative names written by the agent itself;
//! - **pi family** (pi / oh-my-pi): a jsonl's first line is `{"type":"session", ...}`, **carrying no name**;
//!   the opening of the first user message becomes the session title (pi's own session list uses it as the
//!   identifier too);
//! - Other clients: no name in the sources — left `None`, and the UI falls back to project name or a
//!   truncated id.
//!
//! ## Why not merge into the usage scan
//!
//! [`super::scan`] runs through tokscale's flat message stream; `ParsedMessage` has no name field, so it
//! can't help. This is a separate pass reading metadata only: one SQLite query + the **file head** of each
//! pi session (no re-parsing of whole transcripts — even a 59MB session reads only its first few hundred KB).
//!
//! ## Fail means give up
//!
//! Titles are a bonus; unreadable means `None`: a locked database, missing directory, corrupted jsonl, or
//! a first user message not within the read window — all silently skipped, so titles never drag the scan
//! into an error.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::format::ConfigFormat;

use super::delete;
use super::ledger::SessionSnapshot;

/// Max bytes read from a pi session file's head: the first user message usually sits in the first few dozen
/// lines; if the cap is exhausted without finding it, give up — never read a whole transcript for a title.
const PI_HEAD_BYTES: u64 = 512 * 1024;

/// Max characters stored for a title in the ledger: the UI truncates display anyway, and pasted long
/// prompts shouldn't bloat the ledger.
const TITLE_CHARS: usize = 120;

/// Backfill session titles into scan results: writes `SessionSnapshot::title` in place.
///
/// Only processes entries that lack a title (the ledger merge prefers new values; see `absorb_update`).
pub fn collect(sessions: &mut HashMap<String, SessionSnapshot>) {
    collect_sqlite(sessions);
    collect_pi(sessions);
}

/// opencode family: read `title` from the database.
fn collect_sqlite(sessions: &mut HashMap<String, SessionSnapshot>) {
    for client in [
        ConfigFormat::Opencode,
        ConfigFormat::Kilocode,
        ConfigFormat::Mimocode,
    ] {
        let wanted = keys_needing_title(sessions, client);
        if wanted.is_empty() {
            continue;
        }
        let Some(titles) = sqlite_titles(client) else {
            continue;
        };
        for key in wanted {
            let Some(snapshot) = sessions.get_mut(&key) else {
                continue;
            };
            if let Some(title) = titles.get(&snapshot.session_id) {
                snapshot.title = Some(title.clone());
            }
        }
    }
}

/// pi family: take the first user message from the jsonl file head.
fn collect_pi(sessions: &mut HashMap<String, SessionSnapshot>) {
    for client in [ConfigFormat::Pi, ConfigFormat::OhMyPi] {
        let wanted = keys_needing_title(sessions, client);
        if wanted.is_empty() {
            continue;
        }
        let Some(root) = super::pi::sessions_root(client) else {
            continue;
        };
        if !root.is_dir() {
            continue;
        }
        // The whole tree is walked once: searching directories per session would re-read the same levels repeatedly.
        let mut files: Vec<PathBuf> = Vec::new();
        super::pi::walk_jsonl(&root, &mut files);
        // id → path. Ids come from each transcript's first line (sub-agent transcripts are all named
        // `session.jsonl`, carrying no id in the file name — see `super::pi`), so each file reads one line here;
        // only title-less sessions go on to read the file head for the first user message.
        let mut index: HashMap<String, PathBuf> = HashMap::new();
        for path in files {
            if let Some(id) = super::pi::session_id_of(&path) {
                index.entry(id).or_insert(path);
            }
        }
        for key in wanted {
            let Some(snapshot) = sessions.get_mut(&key) else {
                continue;
            };
            let Some(path) = index.get(&snapshot.session_id) else {
                continue;
            };
            snapshot.title = first_user_message(path);
        }
    }
}

/// Ledger keys under a client that still lack a title.
fn keys_needing_title(
    sessions: &HashMap<String, SessionSnapshot>,
    client: ConfigFormat,
) -> Vec<String> {
    sessions
        .iter()
        .filter(|(_, snapshot)| snapshot.client == client && snapshot.title.is_none())
        .map(|(key, _)| key.clone())
        .collect()
}

/// Read session titles from an opencode-family database: both `session` and `session_v2` are queried.
///
/// Both tables may exist (newer databases migrated sessions into `session_v2`); the same id takes the first read.
fn sqlite_titles(client: ConfigFormat) -> Option<HashMap<String, String>> {
    let path = delete::opencode_db(client)?;
    if !path.is_file() {
        return None;
    }
    let connection = open_read_only(&path)?;
    let mut titles: HashMap<String, String> = HashMap::new();
    for table in ["session", "session_v2"] {
        // Old databases lack session_v2: ask per table first (the same check as the deletion path).
        if !delete::table_exists(&connection, table).unwrap_or(false) {
            continue;
        }
        let Ok(mut statement) = connection.prepare(&format!("SELECT id, title FROM {table}"))
        else {
            continue;
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            let (id, title) = row;
            let Some(title) = clean_title(title.as_deref()) else {
                continue;
            };
            titles.entry(id).or_insert(title);
        }
    }
    Some(titles)
}

/// Open read-only: the agent may be writing this database, and a read-only connection never touches
/// `-wal` / `-shm`.
/// If read-only open fails, fall back to a normal connection — SQLite refuses read-only opens of WAL
/// databases missing `-shm`, and this module issues only SELECT queries, never writes.
fn open_read_only(path: &Path) -> Option<rusqlite::Connection> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .or_else(|_| rusqlite::Connection::open(path))
            .ok()?;
    // A locked database gets waited out (the same value as the deletion path); expiring counts as no title.
    let _ = connection.busy_timeout(std::time::Duration::from_secs(5));
    Some(connection)
}

/// Read a jsonl file's head and take the first user message's text as the session title.
///
/// Only the first [`PI_HEAD_BYTES`] bytes: `None` if no user message appears within the cap; a half-line
/// of JSON that fails to parse also counts as nothing.
fn first_user_message(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(std::io::Read::take(file, PI_HEAD_BYTES));
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader.read_line(&mut line).ok()?;
        if read == 0 {
            return None;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim_end()) else {
            continue;
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("role").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        // Plain-text attachment notes (images / tool results) can't serve as titles — keep looking.
        if let Some(title) = clean_title(Some(&message_text(message.get("content")))) {
            return Some(title);
        }
    }
}

/// Extract text from a message's `content`: strings used directly, arrays concatenated from `text` blocks.
fn message_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                // Take text blocks only: tool calls and image blocks must not become session titles.
                let kind = part.get("type").and_then(|value| value.as_str());
                if kind.is_some_and(|kind| kind != "text") {
                    return None;
                }
                part.get("text").and_then(|value| value.as_str())
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Normalize a title: line breaks and runs of whitespace collapse to single spaces, over-long truncates, empty counts as none.
fn clean_title(raw: Option<&str>) -> Option<String> {
    let collapsed = raw?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_chars(&collapsed, TITLE_CHARS))
}

/// Truncate by **character** (Chinese counts per glyph, never splitting one), appending an ellipsis.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a temporary pi transcript.
    fn temp_jsonl(tag: &str, body: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mh-titles-{}-{}-{}.jsonl",
            tag,
            n,
            std::process::id()
        ));
        std::fs::write(&path, body).expect("写临时转录");
        path
    }

    #[test]
    fn clean_title_collapses_whitespace_and_drops_empty() {
        assert_eq!(
            clean_title(Some("  项目分析,\n查找\t缺陷  ")).as_deref(),
            Some("项目分析, 查找 缺陷"),
            "折行与连续空白压成单个空格"
        );
        assert_eq!(clean_title(Some("   ")), None, "全空白等于没有名字");
        assert_eq!(clean_title(None), None);
    }

    #[test]
    fn truncate_chars_counts_characters_not_bytes() {
        let long = "汉".repeat(TITLE_CHARS + 5);
        let cut = truncate_chars(&long, TITLE_CHARS);
        assert_eq!(cut.chars().count(), TITLE_CHARS + 1, "正文 120 字 + 省略号");
        assert!(cut.ends_with('…'));
        assert_eq!(truncate_chars("短", TITLE_CHARS), "短", "没超就不动");
    }

    #[test]
    fn message_text_takes_string_and_text_blocks_only() {
        let text = message_text(Some(&serde_json::json!("直接一段话")));
        assert_eq!(text, "直接一段话");

        let mixed = message_text(Some(&serde_json::json!([
            { "type": "text", "text": "第一段" },
            { "type": "toolCall", "name": "bash" },
            { "type": "image", "data": "..." },
            { "type": "text", "text": "第二段" },
        ])));
        assert_eq!(mixed, "第一段 第二段", "工具调用与图片块不当名字");

        assert_eq!(message_text(None), "");
    }

    /// A pile of model_change / assistant lines precedes the first user message — they must be skipped.
    #[test]
    fn first_user_message_skips_non_user_lines() {
        let path = temp_jsonl(
            "skip",
            &[
                r#"{"type":"session","version":3,"id":"abc","cwd":"D:\\x"}"#,
                r#"{"type":"model_change","id":"1","provider":"p","modelId":"m"}"#,
                r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"我先看看"}]}}"#,
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"项目分析，查找缺陷"}]}}"#,
                r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"第二条不该被取"}]}}"#,
            ]
            .join("\n"),
        );
        assert_eq!(
            first_user_message(&path).as_deref(),
            Some("项目分析，查找缺陷"),
            "取首条用户消息，assistant 行跳过"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A user message holding only image blocks doesn't count as a title; keep looking for text.
    #[test]
    fn first_user_message_ignores_non_text_content() {
        let path = temp_jsonl(
            "notext",
            &[
                r#"{"type":"message","message":{"role":"user","content":[{"type":"image","data":"..."}]}}"#,
                r#"{"type":"message","message":{"role":"user","content":"真正的名字"}}"#,
            ]
            .join("\n"),
        );
        assert_eq!(first_user_message(&path).as_deref(), Some("真正的名字"));
        let _ = std::fs::remove_file(&path);
    }

    /// Unreadable / corrupt lines / no user message: always `None`, never an error.
    #[test]
    fn first_user_message_gives_up_quietly() {
        let path = temp_jsonl("none", "{\"type\":\"session\"}\n{ 坏行\n");
        assert_eq!(first_user_message(&path), None, "没有用户消息就是没有名字");
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            first_user_message(Path::new("Z:/不存在的目录/不存在.jsonl")),
            None,
            "文件打不开也不该炸"
        );
    }

    /// Give up past the read cap: never read a whole large transcript for one title.
    #[test]
    fn first_user_message_stops_at_the_byte_cap() {
        // A 600KB filler line (past the 512KB cap), with the user message behind it.
        let filler = format!(
            r#"{{"type":"message","message":{{"role":"assistant","content":"{}"}}}}"#,
            "x".repeat(600 * 1024)
        );
        let path = temp_jsonl(
            "cap",
            &format!(
                "{filler}\n{}\n",
                r#"{"type":"message","message":{"role":"user","content":"读不到的名字"}}"#
            ),
        );
        assert_eq!(
            first_user_message(&path),
            None,
            "上限之外的内容不读，宁可没有名字"
        );
        let _ = std::fs::remove_file(&path);
    }
}
