//! 会话名采集 —— tokscale 的解析通道只回 token 与模型，名字要单独从源里读。
//!
//! ## 各客户端的名字从哪来
//!
//! - **opencode 系**（opencode / kilocode / mimocode）：库里的 `session.title`
//!   与 `session_v2.title`，是 agent 自己写下的权威名字；
//! - **pi 系**（pi / oh-my-pi）：jsonl 的首行是 `{"type":"session", ...}`，
//!   **不带名字**；取首条用户消息的开头当会话名（pi 自己列会话时同样拿它当标识）；
//! - 其余客户端：源里没有名字，留 `None`，界面退回项目名或截断 id。
//!
//! ## 为什么不并进用量扫描
//!
//! [`super::scan`] 走的是 tokscale 的扁平消息流，`ParsedMessage` 里没有名字字段，
//! 指望不上。这里单独一趟，只读元数据：一个 SQLite 查询 + 每个 pi 会话的**文件头**
//! （不重新解析整份转录，59MB 的会话也只读前几百 KB）。
//!
//! ## 失败即放弃
//!
//! 名字是锦上添花，读不到就留 `None`：库被占、目录不存在、jsonl 损坏、
//! 首条用户消息在读到的范围内没出现 —— 一律静默跳过，不让会话名把扫描拖成错误。

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::format::ConfigFormat;

use super::delete;
use super::ledger::SessionSnapshot;

/// pi 会话文件头最多读这么多字节：首条用户消息通常在前几十行，
/// 读满上限还没找到就放弃 —— 不为一个名字把整份转录读完。
const PI_HEAD_BYTES: u64 = 512 * 1024;

/// 存进账本的名字最长这么多字符：界面本来就截断显示，
/// 用户粘进来的长 prompt 不该把账本撑大。
const TITLE_CHARS: usize = 120;

/// 给扫描结果补会话名：就地写回 `SessionSnapshot::title`。
///
/// 只处理还没有名字的条目（账本合并时新值优先，见 `absorb_update`）。
pub fn collect(sessions: &mut HashMap<String, SessionSnapshot>) {
    collect_sqlite(sessions);
    collect_pi(sessions);
}

/// opencode 系：从库里读 `title`。
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

/// pi 系：从 jsonl 文件头取首条用户消息。
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
        // 整棵树只走一遍：逐个会话去搜目录会把同几层反复读。
        let mut files: Vec<PathBuf> = Vec::new();
        super::pi::walk_jsonl(&root, &mut files);
        // id → 路径。id 取自每份转录的首行（子 agent 转录一律叫
        // `session.jsonl`，文件名里没有 id，见 `super::pi`），因此这里每个
        // 文件只读一行；缺名字的会话才继续读文件头取首条用户消息。
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

/// 该客户端下还没有名字的账本键。
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

/// 读 opencode 系库里的会话名：`session` 与 `session_v2` 两张表都问。
///
/// 两张表都存在（新库把会话迁到了 `session_v2`），同一 id 以先读到的为准。
fn sqlite_titles(client: ConfigFormat) -> Option<HashMap<String, String>> {
    let path = delete::opencode_db(client)?;
    if !path.is_file() {
        return None;
    }
    let connection = open_read_only(&path)?;
    let mut titles: HashMap<String, String> = HashMap::new();
    for table in ["session", "session_v2"] {
        // 老库没有 session_v2：逐表先问有没有（与删除路径同一套判断）。
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

/// 只读打开：agent 可能正在写这个库，只读连接不去动 `-wal` / `-shm`。
///
/// 只读打不开就退回普通连接 —— WAL 库缺 `-shm` 时 SQLite 会拒绝只读打开，
/// 而本模块只跑 SELECT，不写库。
fn open_read_only(path: &Path) -> Option<rusqlite::Connection> {
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .or_else(|_| rusqlite::Connection::open(path))
            .ok()?;
    // 库被占着就等一等（与删除路径同取值），等不到当作读不到名字。
    let _ = connection.busy_timeout(std::time::Duration::from_secs(5));
    Some(connection)
}

/// 读 jsonl 文件头，取首条用户消息的文本当会话名。
///
/// 只读前 [`PI_HEAD_BYTES`] 字节：读到上限还没出现用户消息就返回 `None`，
/// 半行 JSON 解析不出来同样当作没有。
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
        // 纯文本的附件说明（图片 / 工具结果）当不了名字，接着往下找。
        if let Some(title) = clean_title(Some(&message_text(message.get("content")))) {
            return Some(title);
        }
    }
}

/// 从消息的 `content` 里取出文本：字符串直接用，数组只拼 `text` 块。
fn message_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                // 只取文本块：工具调用与图片块不该当会话名。
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

/// 规整名字：折行与连续空白压成单个空格，超长截断，空的当没有。
fn clean_title(raw: Option<&str>) -> Option<String> {
    let collapsed = raw?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_chars(&collapsed, TITLE_CHARS))
}

/// 按**字符**截断（中文按字算，不会切出半个字），末尾补省略号。
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

    /// 造一份临时 pi 转录。
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

    /// 首条用户消息之前有一堆 model_change / assistant 行，要跳过它们。
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

    /// 用户消息只有图片块时不算名字，继续往下找文本。
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

    /// 读不到 / 坏行 / 没有用户消息：一律 `None`，不报错。
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

    /// 超过读取上限就放弃：不为一个名字读完整份大转录。
    #[test]
    fn first_user_message_stops_at_the_byte_cap() {
        // 一行 600KB 的填充（超过 512KB 上限），用户消息排在它后面。
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
