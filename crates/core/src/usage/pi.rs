//! The pi family's (pi / oh-my-pi) session storage layout — the path knowledge shared by deletion and title reading.
//!
//! ## Two layouts, the id in different places
//!
//! - **Top-level sessions**: `<session root>/<encoded cwd>/<timestamp>_<uuid>.jsonl`;
//!   the id is both the file name's last segment and written in the content's first line;
//! - **Sub-agent / worktree transcripts**:
//!   `<session root>/<encoded cwd>/<parent session>/<uuid>/run-N/session.jsonl`,
//!   the file name is always `session.jsonl`, and the id is **only in the content's first line**
//!   (`{"type":"session","id":"..."}`).
//!
//! Recognizing ids by file name alone misses every sub-agent session: they'd be neither titled nor
//! deletable. In this machine's real data, 16 of 21 pi sessions are that kind, hidden four levels deep.
//! So [`session_id_of`] always trusts the first line, with the file name only as a fallback.

use std::path::{Path, PathBuf};

use crate::format::ConfigFormat;
use crate::prefs;

/// Recursion depth cap (relative to the session root).
///
/// The real layout reaches level 4 (`<cwd>/<parent session>/<sub-agent>/run-N/session.jsonl`); sub-agents
/// can spawn further sub-agents, so leave headroom. A cap prevents infinite recursion on directory cycles
/// or abnormal depth — an earlier cap of 3 missed exactly the level-4 transcripts.
const MAX_DEPTH: usize = 8;

/// Max bytes read for a session's head (first line): the first line is a few hundred bytes of JSON;
/// the cap only guards against one gigantic line in a malformed file exhausting memory.
const HEADER_BYTES: u64 = 64 * 1024;

/// Session roots: pi is `~/.pi/agent/sessions`, oh-my-pi is `~/.omp/agent/sessions`.
pub(super) fn sessions_root(client: ConfigFormat) -> Option<PathBuf> {
    match client {
        ConfigFormat::Pi => {
            prefs::home_dir().map(|home| home.join(".pi").join("agent").join("sessions"))
        }
        ConfigFormat::OhMyPi => {
            prefs::home_dir().map(|home| home.join(".omp").join("agent").join("sessions"))
        }
        _ => None,
    }
}

/// Take a transcript's session id: **first line wins**, falling back to the file name.
pub(super) fn session_id_of(path: &Path) -> Option<String> {
    header_session_id(path).or_else(|| file_name_session_id(path))
}

/// Read the first line's `{"type":"session","id":...}`.
fn header_session_id(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(std::io::Read::take(file, HEADER_BYTES));
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(line.trim_end()).ok()?;
    if value.get("type").and_then(|value| value.as_str()) != Some("session") {
        return None;
    }
    value
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

/// Take the id from a file name: the last segment of `<timestamp>_<uuid>.jsonl`.
///
/// Sub-agent transcripts are named `session.jsonl` with no `_`; returns `None` here.
fn file_name_session_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    stem.rsplit_once('_').map(|(_, id)| id.to_string())
}

/// Whether it's a session transcript file (`.jsonl`).
///
/// The same-named transcript directory (suffix stripped) isn't a file and never reaches here; sibling
/// non-`.jsonl` files (sub-agent artifacts) don't count either.
fn is_jsonl(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "jsonl")
}

/// Collect every `*.jsonl` under the session root (recursing to [`MAX_DEPTH`]).
pub(super) fn walk_jsonl(root: &Path, out: &mut Vec<PathBuf>) {
    walk(root, 0, out);
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, depth + 1, out);
            continue;
        }
        if is_jsonl(&path) {
            out.push(path);
        }
    }
}

/// Find a given session's transcript under the session root.
///
/// Comparison goes by id (see [`session_id_of`]), so a sub-agent's `session.jsonl` is found all the same.
/// Returns on first hit — no need to walk the whole tree.
pub(super) fn find_session(root: &Path, session_id: &str) -> Option<PathBuf> {
    search(root, session_id, 0)
}

fn search(dir: &Path, session_id: &str, depth: usize) -> Option<PathBuf> {
    if depth > MAX_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    // Scan this level's files first: a hit means no descent needed.
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
            continue;
        }
        if !is_jsonl(&path) {
            continue;
        }
        if session_id_of(&path).as_deref() == Some(session_id) {
            return Some(path);
        }
    }
    for dir in subdirs {
        if let Some(found) = search(&dir, session_id, depth + 1) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary session root.
    fn temp_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("mh-pi-{}-{}-{}", tag, n, std::process::id()));
        std::fs::create_dir_all(&root).expect("建临时会话根");
        root
    }

    /// Assemble the real two-level layout: a top-level session + a sub-agent transcript hidden at level 4.
    /// Returns (top-level file, sub-agent file).
    fn layout(root: &Path) -> (PathBuf, PathBuf) {
        let ws = root.join("--D--VsPro-X--");
        let top = ws.join("2026-01-01T00-00-00-000Z_top-id.jsonl");
        std::fs::create_dir_all(&ws).expect("建 cwd 目录");
        std::fs::write(
            &top,
            r#"{"type":"session","version":3,"id":"top-id","cwd":"D:\\X"}"#,
        )
        .expect("写顶层会话");
        // Sub-agent: cwd / parent session dir / sub-agent uuid / run-0 / session.jsonl
        let run = ws
            .join("2026-01-01T00-00-00-000Z_top-id")
            .join("sub-uuid")
            .join("run-0");
        std::fs::create_dir_all(&run).expect("建 run 目录");
        let sub = run.join("session.jsonl");
        std::fs::write(
            &sub,
            r#"{"type":"session","version":3,"id":"sub-id","cwd":"D:\\X"}"#,
        )
        .expect("写子 agent 转录");
        (top, sub)
    }

    /// The level-4 sub-agent transcript must be findable — a depth cap of 3 fails here.
    #[test]
    fn find_session_reaches_nested_subagent_transcripts() {
        let root = temp_root("find");
        let (top, sub) = layout(&root);
        assert_eq!(
            find_session(&root, "top-id"),
            Some(top),
            "顶层会话按 id 找得到"
        );
        assert_eq!(
            find_session(&root, "sub-id"),
            Some(sub),
            "第 4 层的子 agent 转录也要找得到（文件名是 session.jsonl）"
        );
        assert_eq!(find_session(&root, "不存在"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `session.jsonl`'s file name yields no id; only the first line can.
    #[test]
    fn session_id_of_prefers_the_header_over_the_file_name() {
        let root = temp_root("id");
        let (top, sub) = layout(&root);
        assert_eq!(session_id_of(&top).as_deref(), Some("top-id"));
        assert_eq!(
            session_id_of(&sub).as_deref(),
            Some("sub-id"),
            "文件名是 session.jsonl，id 只在首行"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// When the first line isn't a session header (or the file is empty), fall back to the id in the file name.
    #[test]
    fn session_id_of_falls_back_to_the_file_name() {
        let root = temp_root("fallback");
        let path = root.join("2026-01-01T00-00-00-000Z_sess-b.jsonl");
        std::fs::write(&path, "{\"type\":\"message\"}\n").expect("写转录");
        assert_eq!(
            session_id_of(&path).as_deref(),
            Some("sess-b"),
            "首行不是会话头就退回文件名"
        );
        std::fs::write(&path, "").expect("清空转录");
        assert_eq!(
            session_id_of(&path).as_deref(),
            Some("sess-b"),
            "空文件同样退回文件名"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Collect `.jsonl` only: same-named transcript directories and other suffixes don't count.
    #[test]
    fn walk_jsonl_collects_only_jsonl_files() {
        let root = temp_root("walk");
        let (top, sub) = layout(&root);
        std::fs::write(root.join("notes.txt"), "不是会话").expect("写干扰文件");
        std::fs::create_dir_all(root.join("2026-01-01T00-00-00-000Z_top-id")).expect("建转录目录");

        let mut found = Vec::new();
        walk_jsonl(&root, &mut found);
        found.sort();
        let mut expected = vec![top, sub];
        expected.sort();
        assert_eq!(found, expected, "两份转录都要收到，且只收 .jsonl");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Session roots are per client: pi and oh-my-pi each have their own directory; other clients have none.
    #[test]
    fn sessions_root_is_per_client() {
        let pi = sessions_root(ConfigFormat::Pi).expect("pi 应有会话根");
        let omp = sessions_root(ConfigFormat::OhMyPi).expect("oh-my-pi 应有会话根");
        assert!(pi.ends_with(".pi/agent/sessions") || pi.ends_with(".pi\\agent\\sessions"));
        assert!(omp.ends_with(".omp/agent/sessions") || omp.ends_with(".omp\\agent\\sessions"));
        assert_eq!(
            sessions_root(ConfigFormat::Opencode),
            None,
            "opencode 不是 pi 系，没有这个会话根"
        );
    }
}
