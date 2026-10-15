//! pi 系（pi / oh-my-pi）的会话存储布局 —— 删除与读名字共用的那套路径知识。
//!
//! ## 两种布局，id 的位置不同
//!
//! - **顶层会话**：`<会话根>/<编码 cwd>/<时间戳>_<uuid>.jsonl`，
//!   id 既是文件名最后一段，也写在内容首行；
//! - **子 agent / worktree 转录**：
//!   `<会话根>/<编码 cwd>/<父会话>/<uuid>/run-N/session.jsonl`，
//!   文件名一律是 `session.jsonl`，id **只在内容首行**
//!   （`{"type":"session","id":"..."}`）。
//!
//! 只按文件名认 id 会漏掉全部子 agent 会话：它们既取不到名字，也删不掉。
//! 本机真实数据里 21 个 pi 会话有 16 个是这种，藏在第 4 层目录下。
//! 因此 [`session_id_of`] 一律以首行为准，文件名只作兜底。

use std::path::{Path, PathBuf};

use crate::format::ConfigFormat;
use crate::prefs;

/// 递归深度上限（相对会话根）。
///
/// 实际布局到第 4 层（`<cwd>/<父会话>/<子 agent>/run-N/session.jsonl`）；
/// 子 agent 还能再派生子 agent，因此留出余量。设上限是防目录成环或
/// 异常深时无限递归 —— 早先取 3 层，正好把第 4 层的转录全漏掉。
const MAX_DEPTH: usize = 8;

/// 读会话头（首行）最多读这么多字节：首行是几百字节的 JSON，
/// 上限只为防住畸形文件里一行超长时把内存吃满。
const HEADER_BYTES: u64 = 64 * 1024;

/// 会话根：pi 是 `~/.pi/agent/sessions`，oh-my-pi 是 `~/.omp/agent/sessions`。
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

/// 取一份转录的会话 id：**首行优先**，读不到再退回文件名。
pub(super) fn session_id_of(path: &Path) -> Option<String> {
    header_session_id(path).or_else(|| file_name_session_id(path))
}

/// 读首行的 `{"type":"session","id":...}`。
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

/// 从文件名取 id：`<时间戳>_<uuid>.jsonl` 的最后一段。
///
/// 子 agent 转录叫 `session.jsonl`，没有 `_`，这里返回 `None`。
fn file_name_session_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    stem.rsplit_once('_').map(|(_, id)| id.to_string())
}

/// 是不是会话转录文件（`.jsonl`）。
///
/// 同名转录目录（去掉后缀）不是文件，走不到这里；非 `.jsonl` 的兄弟文件
/// （子 agent 产物）也不算。
fn is_jsonl(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "jsonl")
}

/// 收集会话根下的全部 `*.jsonl`（递归到 [`MAX_DEPTH`]）。
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

/// 在会话根下找指定会话的转录文件。
///
/// 按 id 比对（见 [`session_id_of`]），因此子 agent 的 `session.jsonl`
/// 同样找得到。命中即返回，不必走完整棵树。
pub(super) fn find_session(root: &Path, session_id: &str) -> Option<PathBuf> {
    search(root, session_id, 0)
}

fn search(dir: &Path, session_id: &str, depth: usize) -> Option<PathBuf> {
    if depth > MAX_DEPTH {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    // 先扫本层文件：命中就不必下探。
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

    /// 临时会话根。
    fn temp_root(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("mh-pi-{}-{}-{}", tag, n, std::process::id()));
        std::fs::create_dir_all(&root).expect("建临时会话根");
        root
    }

    /// 搭出真实的两层布局：顶层会话 + 藏在第 4 层的子 agent 转录。
    ///
    /// 返回（顶层文件, 子 agent 文件）。
    fn layout(root: &Path) -> (PathBuf, PathBuf) {
        let ws = root.join("--D--VsPro-X--");
        let top = ws.join("2026-01-01T00-00-00-000Z_top-id.jsonl");
        std::fs::create_dir_all(&ws).expect("建 cwd 目录");
        std::fs::write(
            &top,
            r#"{"type":"session","version":3,"id":"top-id","cwd":"D:\\X"}"#,
        )
        .expect("写顶层会话");
        // 子 agent：cwd / 父会话目录 / 子 agent uuid / run-0 / session.jsonl
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

    /// 第 4 层的子 agent 转录也要能找到 —— 深度上限取 3 时这里会失败。
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

    /// `session.jsonl` 的文件名推不出 id，只能读首行。
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

    /// 首行不是会话头（或文件空）时退回文件名里的 id。
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

    /// 只收集 `.jsonl`：同名转录目录与其它后缀都不算。
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

    /// 会话根按客户端分：pi 与 oh-my-pi 各在自己的目录下，其余客户端没有。
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
