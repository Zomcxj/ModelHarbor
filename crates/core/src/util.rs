use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Build a wsl command: with CREATE_NO_WINDOW on Windows.
pub(crate) fn wsl_command() -> Command {
    let mut cmd = Command::new("wsl");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Whether WSL genuinely works: only `wsl -e true` exiting 0 counts. Cached per process.
/// Serves test guards chiefly (skip WSL sync tests without WSL), plus save-flow queries.
/// Needed in normal (non-test) builds too: the app crate's test builds depend on it.
pub fn wsl_usable() -> bool {
    static USABLE: OnceLock<bool> = OnceLock::new();
    *USABLE.get_or_init(|| {
        wsl_command()
            .args(["-e", "true"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    })
}

pub fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or_default()
}

pub fn bool_at(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or_default()
}

pub fn num_at(v: &Value, k: &str) -> String {
    v.get(k).map(number_text).unwrap_or_default()
}

/// JSON number → text: prefers exact i64/u64 representation.
fn number_text(v: &Value) -> String {
    match v.as_number() {
        Some(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(u) = n.as_u64() {
                u.to_string()
            } else if let Some(f) = n.as_f64() {
                f.to_string()
            } else {
                String::new()
            }
        }
        None => String::new(),
    }
}

pub fn number_text_public(v: &Value) -> String {
    number_text(v)
}

pub fn nested_str<'a>(v: &'a Value, path: &[&str]) -> &'a str {
    let mut cur = v;
    for p in path {
        match cur.get(*p) {
            Some(x) => cur = x,
            None => return "",
        }
    }
    cur.as_str().unwrap_or_default()
}

pub fn nested_num(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        match cur.get(*p) {
            Some(x) => cur = x,
            None => return String::new(),
        }
    }
    number_text(cur)
}

pub fn nested_list_str(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for p in path {
        match cur.get(*p) {
            Some(x) => cur = x,
            None => return String::new(),
        }
    }
    cur.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

pub fn set_str(m: &mut Map<String, Value>, k: &str, v: &str) {
    if v.is_empty() {
        m.remove(k);
    } else {
        m.insert(k.into(), v.into());
    }
}

/// Comma string → list (whitespace trimmed, empty items dropped).
///
/// Multi-value fields in the UI (tiers, input modalities, etc.) are comma strings; each backend
/// splits them into arrays for the config file.
pub fn split_csv(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn set_num_opt(m: &mut Map<String, Value>, k: &str, v: &str) {
    let t = v.trim();
    if t.is_empty() {
        m.remove(k);
        return;
    }
    if let Ok(i) = t.parse::<i64>() {
        if i.to_string() == t {
            m.insert(k.into(), i.into());
            return;
        }
    }
    if let Ok(f) = t.parse::<f64>() {
        // Non-finite numbers (inf / nan) count as invalid — never written.
        if f.is_finite() {
            m.insert(k.into(), f.into());
            return;
        }
    }
    m.remove(k);
}

pub fn parse_number_text(v: &str) -> Option<Value> {
    let t = v.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(i) = t.parse::<i64>() {
        if i.to_string() == t {
            return Some(i.into());
        }
    }
    if let Ok(f) = t.parse::<f64>() {
        // `inf` / `nan` / `1e400` count as invalid (not counted as valid values; the save status bar notes the ignores).
        if f.is_finite() {
            return Some(f.into());
        }
    }
    None
}

/// Atomic text-file write: write a temp file in the same directory with `sync_all`, then replace the real file.
///
/// Windows rename doesn't overwrite existing files: when the target exists, the old file first moves
/// to a backup name; after a successful replace the backup is deleted, on failure it moves back. Error
/// text carries paths only, never content.
pub fn atomic_write_text(path: &Path, content: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir)
                .map_err(|err| format!("创建目录失败（{}）：{}", dir.display(), err))?;
        }
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temp_path = path.with_file_name(format!("{file_name}.tmp"));
    let result = (|| -> Result<(), String> {
        let mut temp = fs::File::create(&temp_path)
            .map_err(|err| format!("写入临时文件失败（{}）：{}", path.display(), err))?;
        temp.write_all(content.as_bytes())
            .map_err(|err| format!("写入临时文件失败（{}）：{}", path.display(), err))?;
        temp.sync_all()
            .map_err(|err| format!("同步临时文件失败（{}）：{}", path.display(), err))?;
        replace_file(&temp_path, path, file_name)
            .map_err(|err| format!("替换文件失败（{}）：{}", path.display(), err))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

/// Replace the target using a same-directory temp file. Existing targets move to a backup name first; failures roll back.
fn replace_file(temp_path: &Path, path: &Path, file_name: &str) -> std::io::Result<()> {
    if !path.exists() {
        return fs::rename(temp_path, path);
    }
    let backup_path = path.with_file_name(format!("{file_name}.replace-old"));
    if backup_path.exists() {
        fs::remove_file(&backup_path)?;
    }
    fs::rename(path, &backup_path)?;
    match fs::rename(temp_path, path) {
        Ok(()) => {
            let _ = fs::remove_file(backup_path);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup_path, path);
            Err(error)
        }
    }
}

pub fn is_wsl_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains(':')
}

/// The WSL master switch: off by default. While closed, all probing / IO short-circuits and no WSL
/// VM is ever started. Single-point interception: per-caller checks would leak.
static WSL_ENABLED: AtomicBool = AtomicBool::new(false);

/// Allow / forbid WSL subprocess calls (App syncs this when the checkbox changes).
pub fn wsl_set_enabled(on: bool) {
    WSL_ENABLED.store(on, Ordering::Relaxed);
}

/// Whether WSL calls are currently allowed.
pub fn wsl_enabled() -> bool {
    WSL_ENABLED.load(Ordering::Relaxed)
}

/// The default WSL distro's $HOME (cached per process).
pub fn wsl_home() -> Option<String> {
    // With the master switch off, short-circuit without touching the cache: otherwise a probed None would cache in the OnceLock forever.
    if !wsl_enabled() {
        return None;
    }
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let out = wsl_command()
                .args(["-e", "sh", "-c", "printf %s \"$HOME\""])
                .output()
                .ok()?;
            if !out.status.success() {
                return None;
            }
            let home = String::from_utf8(out.stdout)
                .unwrap_or_default()
                .trim()
                .to_string();
            if home.is_empty() {
                None
            } else {
                Some(home)
            }
        })
        .clone()
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\\\''"))
}

/// One item's result from a WSL batch probe.
#[derive(Clone, Copy, Default, Debug)]
pub struct WslPathProbe {
    /// `test -f`: the path is a regular file.
    pub file_exists: bool,
    /// `test -e`: the path exists (any type, files included).
    pub path_exists: bool,
    /// The parent directory exists (the "installed" verdict's loose condition).
    pub parent_dir_exists: bool,
}

/// Probe several paths' existence in a single `wsl` call.
/// Each item prints in order: `F` (regular file) / `E` (exists, not a file) / `P` (parent exists) / `N` (neither).
pub fn wsl_batch_probe(paths: &[String]) -> Vec<WslPathProbe> {
    let fallback = || vec![WslPathProbe::default(); paths.len()];
    // Master switch off: no wsl process spawns — everything reads as "absent".
    if !wsl_enabled() {
        return fallback();
    }
    if paths.is_empty() {
        return Vec::new();
    }
    let mut script = String::from("for p in");
    for p in paths {
        script.push(' ');
        script.push_str(&shell_quote(p));
    }
    script.push_str(
        "; do if [ -f \"$p\" ]; then echo F; \
         elif [ -e \"$p\" ]; then echo E; \
         elif [ -d \"${p%/*}\" ]; then echo P; \
         else echo N; fi; done",
    );
    let out = match wsl_command().args(["-e", "sh", "-c", &script]).output() {
        Ok(o) if o.status.success() => o,
        _ => return fallback(),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut result = Vec::with_capacity(paths.len());
    for line in stdout.lines().take(paths.len()) {
        result.push(match line.trim() {
            "F" => WslPathProbe {
                file_exists: true,
                path_exists: true,
                parent_dir_exists: true,
            },
            "E" => WslPathProbe {
                file_exists: false,
                path_exists: true,
                parent_dir_exists: true,
            },
            "P" => WslPathProbe {
                file_exists: false,
                path_exists: false,
                parent_dir_exists: true,
            },
            _ => WslPathProbe::default(),
        });
    }
    if result.len() != paths.len() {
        return fallback();
    }
    result
}

pub fn win_to_wsl(path: &str) -> String {
    if let Some(ch) = path.chars().next() {
        if ch.is_ascii_alphabetic() && path.len() > 1 && path.as_bytes()[1] == b':' {
            let rest = &path[2..];
            let rest = rest.trim_start_matches('/').trim_start_matches('\\');
            return format!("/mnt/{}/{}", ch.to_ascii_lowercase(), rest);
        }
    }
    path.to_string()
}

pub fn read_wsl_file(path: &str) -> Result<String, String> {
    if !wsl_enabled() {
        return Err("WSL 同步未开启".to_string());
    }
    let out = wsl_command()
        .args(["cat", path])
        .output()
        .map_err(|e| format!("wsl 命令失败: {}", e))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("wsl 读取失败: {}", err));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("读取失败: {}", e))
}

/// Whether a WSL-side path is a regular file (single probe).
pub fn wsl_file_exists(path: &str) -> bool {
    let out = wsl_command()
        .args([
            "-e",
            "sh",
            "-c",
            &format!("test -f {} && echo y", shell_quote(path)),
        ])
        .output();
    matches!(out, Ok(o) if o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "y")
}

pub fn remove_config(path: &str) -> Result<(), String> {
    if is_wsl_path(path) {
        let out = wsl_command()
            .args(["-e", "sh", "-c", &format!("rm -f -- {}", shell_quote(path))])
            .output()
            .map_err(|e| format!("wsl 命令失败: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "wsl 删除失败: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}
pub fn write_wsl_file(path: &str, content: &str) -> Result<(), String> {
    if !wsl_enabled() {
        return Err("WSL 同步未开启".to_string());
    }
    // Temp files get random names with nanosecond timestamps.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp: PathBuf = std::env::temp_dir().join(format!(
        "model_harbor_tmp_{}_{}.json",
        std::process::id(),
        nonce
    ));
    fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
    let tmp_str = tmp.to_string_lossy().replace('\\', "/");
    let tmp_wsl = win_to_wsl(&tmp_str);
    let out = wsl_command()
        .args(["cp", &tmp_wsl, path])
        .output()
        .map_err(|e| format!("wsl 命令失败: {}", e))?;
    fs::remove_file(&tmp).ok();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("wsl 写入失败: {}", err));
    }
    Ok(())
}

/// File extensions supported by the file dialog.
///
/// **The first filter is the Windows dialog's default selection**, and the dialog filters the list by
/// the selected filter: it must cover json / jsonc / yml / yaml / toml. The unit test
/// `dialog_extensions_cover_backend_defaults` guards this list against every backend's default path.
/// The dialog itself (rfd-dependent) lives in the app crate.
pub const CONFIG_FILE_EXTENSIONS: &[&str] = &["json", "jsonc", "yml", "yaml", "toml"];

/// Sanitize network error text: strip the URL's query/fragment and return.
///
/// ureq's Transport Display includes the target URL. Shared by core and app.
pub fn sanitize_network_error(text: &str) -> String {
    const MARK: &str = "for URL \"";
    if let Some(idx) = text.find(MARK) {
        let head = &text[..idx + MARK.len()];
        let rest = &text[idx + MARK.len()..];
        let url = rest.split('"').next().unwrap_or(rest);
        let cut = url.find(['?', '#']).unwrap_or(url.len());
        format!("{}{}\"", head, &url[..cut])
    } else {
        text.to_string()
    }
}

/// Strip JSONC line comments, block comments, and trailing commas into JSON serde_json can parse.
/// Comment markers and commas inside string literals are unaffected; newlines in comments are kept to preserve line numbers.
pub fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    chars.next();
                    for c2 in chars.by_ref() {
                        if c2 == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    while let Some(c2) = chars.next() {
                        if c2 == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            break;
                        }
                        if c2 == '\n' {
                            out.push('\n');
                        }
                    }
                }
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    remove_trailing_commas(&out)
}

/// Remove trailing commas before `}` / `]`.
fn remove_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Read a config file's content (local and WSL paths supported); missing files return an empty string.
pub fn config_exists(path: &str) -> bool {
    if is_wsl_path(path) {
        wsl_file_exists(path)
    } else {
        Path::new(path).is_file()
    }
}

/// Windows-style home directory string (`USERPROFILE` preferred; empty string when unavailable).
///
/// Every backend assembles its default config path from this; path separators stay as literal backslashes.
pub fn home_dir_string() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default()
}

/// Path of a companion file next to the config file (separator follows the main config path's shape: WSL paths use `/`).
pub fn sibling_path(config_path: &str, file_name: &str) -> String {
    let separator = if is_wsl_path(config_path) || config_path.contains('/') {
        '/'
    } else {
        '\\'
    };
    match config_path.rsplit_once(separator) {
        Some((parent, _)) if !parent.is_empty() => {
            format!("{}{}{}", parent, separator, file_name)
        }
        _ => file_name.to_string(),
    }
}

/// Read a config file's content (local and WSL paths supported); missing files return an empty string.
pub fn read_config_content(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Ok(String::new());
    }
    if is_wsl_path(path) {
        if !wsl_file_exists(path) {
            return Ok(String::new());
        }
        read_wsl_file(path)
    } else if std::path::Path::new(path).exists() {
        std::fs::read_to_string(path).map_err(|e| format!("读取失败: {}", e))
    } else {
        Ok(String::new())
    }
}

/// Parse config content (JSONC comments and trailing commas supported); empty content parses as an empty object.
pub fn parse_config_content(content: &str) -> Result<serde_json::Value, String> {
    if content.trim().is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    let stripped = strip_jsonc_comments(content);
    serde_json::from_str(&stripped).map_err(|e| format!("解析失败: {}", e))
}

/// Parse YAML config content; empty content parses as an empty object.
pub fn parse_yaml_content(content: &str) -> Result<serde_json::Value, String> {
    if content.trim().is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    serde_yaml_ng::from_str(content).map_err(|e| format!("解析失败: {}", e))
}

/// Serialize a Value into block-style YAML text.
pub fn to_yaml_string(value: &serde_json::Value) -> Result<String, String> {
    serde_yaml_ng::to_string(value).map_err(|e| format!("序列化失败: {}", e))
}

/// baseUrl health check: returns suspicious-point labels (**UI hints only; configs are never auto-rewritten**).
///
/// Covers doubled slashes (`https://host//v1`), stray trailing slashes, missing scheme, and embedded whitespace.
pub fn url_suspicions(url: &str) -> Vec<&'static str> {
    let trimmed = url.trim();
    let mut out = Vec::new();
    if trimmed.is_empty() {
        return out;
    }
    // The whitespace check reads the original value: edge spaces would be trimmed away.
    if url.chars().any(char::is_whitespace) {
        out.push("含空白字符");
    }
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        out.push("缺少 http(s):// 协议头");
    }
    // Only the path after the scheme is checked.
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    if after_scheme.contains("//") {
        out.push("路径中出现重复斜杠");
    }
    if after_scheme.ends_with('/') {
        out.push("末尾有多余斜杠");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        atomic_write_text, parse_number_text, set_num_opt, url_suspicions, CONFIG_FILE_EXTENSIONS,
    };
    use serde_json::{json, Map};
    use std::fs;
    use std::path::PathBuf;

    /// Non-finite floats (inf / nan) must count as invalid numbers.
    #[test]
    fn parse_number_text_rejects_non_finite_floats() {
        for bad in ["inf", "-inf", "infinity", "NaN", "nan", "1e400", "-1e400"] {
            assert_eq!(parse_number_text(bad), None, "{bad} 不该被判成有效数字");
        }
        assert_eq!(parse_number_text("12"), Some(json!(12)));
        assert_eq!(parse_number_text("1.5"), Some(json!(1.5)));
        assert_eq!(parse_number_text("1e10"), Some(json!(1e10f64)));
    }

    #[test]
    fn set_num_opt_drops_non_finite_instead_of_writing_null() {
        let mut m = Map::new();
        set_num_opt(&mut m, "context", "inf");
        assert!(
            m.get("context").is_none(),
            "inf 不能落盘（会序列化成 null）"
        );
        set_num_opt(&mut m, "context", "1.25");
        assert_eq!(m["context"], json!(1.25));
        set_num_opt(&mut m, "context", "abc");
        assert!(m.get("context").is_none(), "无效数字按既有口径删除该键");
    }

    /// Guard the dialog filter in reverse: every backend's default config extension must be in the list.
    ///
    /// The Windows dialog filters by the first filter: it must cover json / jsonc / yml / yaml.
    #[test]
    fn dialog_extensions_cover_backend_defaults() {
        for backend in crate::backends::BACKENDS {
            let path = backend.default_local_path();
            let ext = std::path::Path::new(&path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            assert!(
                !ext.is_empty(),
                "后端 {:?} 的默认配置没有扩展名：{path}",
                backend.id()
            );
            assert!(
                CONFIG_FILE_EXTENSIONS.contains(&ext.as_str()),
                "后端 {:?} 的默认配置 {path} 扩展名 .{ext} 不在对话框滤镜里，\
                 会导致浏览时选不到该文件",
                backend.id()
            );
        }
        // Neither the JSON nor the YAML side may be missed.
        for ext in ["json", "jsonc", "yml", "yaml"] {
            assert!(
                CONFIG_FILE_EXTENSIONS.contains(&ext),
                "对话框滤镜缺少 {ext}"
            );
        }
    }

    #[test]
    fn url_suspicions_flags_double_slash() {
        assert_eq!(
            url_suspicions("https://7891vip.cc.cd//v1"),
            vec!["路径中出现重复斜杠"]
        );
    }

    #[test]
    fn url_suspicions_flags_trailing_slash() {
        assert_eq!(
            url_suspicions("https://api.openai.com/v1/"),
            vec!["末尾有多余斜杠"]
        );
    }

    #[test]
    fn url_suspicions_flags_missing_scheme() {
        assert_eq!(
            url_suspicions("api.example.com/v1"),
            vec!["缺少 http(s):// 协议头"]
        );
    }

    #[test]
    fn url_suspicions_flags_whitespace() {
        // Embedded whitespace (pasted with spaces) and edge spaces both get flagged.
        assert_eq!(
            url_suspicions("https://api.example.com /v1"),
            vec!["含空白字符"]
        );
        assert_eq!(
            url_suspicions(" https://api.example.com/v1 "),
            vec!["含空白字符"]
        );
    }

    #[test]
    fn url_suspicions_accepts_clean_urls() {
        for url in [
            "https://api.openai.com/v1",
            "http://127.0.0.1:8080/v1",
            "https://generativelanguage.googleapis.com",
        ] {
            assert!(url_suspicions(url).is_empty(), "{url} 不应被判为可疑");
        }
    }

    #[test]
    fn url_suspicions_ignores_empty() {
        assert!(url_suspicions("").is_empty());
        assert!(url_suspicions("   ").is_empty());
    }

    #[test]
    fn url_suspicions_reports_multiple_reasons() {
        assert_eq!(
            url_suspicions("https://host//v1/"),
            vec!["路径中出现重复斜杠", "末尾有多余斜杠"]
        );
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("modelharbor-util-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn atomic_write_creates_parents_and_leaves_no_temp() {
        let dir = scratch_dir("atomic-ok");
        let path = dir.join("nested").join("settings.json");
        atomic_write_text(&path, "{\"a\":1}").expect("写入应成功");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"a\":1}");
        assert!(
            !path.with_file_name("settings.json.tmp").exists(),
            "成功后不得残留临时文件"
        );
        assert!(
            !path.with_file_name("settings.json.replace-old").exists(),
            "成功后不得残留备份文件"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_replaces_existing_content() {
        let dir = scratch_dir("atomic-replace");
        let path = dir.join("tokens.json");
        atomic_write_text(&path, "first").expect("首次写入");
        atomic_write_text(&path, "second").expect("覆盖写入");
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_failure_keeps_previous_content() {
        // Hold the temp-file path with a directory so the atomic write can't create its temp file.
        let dir = scratch_dir("atomic-fail");
        fs::create_dir_all(&dir).expect("建目录");
        let path = dir.join("tokens.json");
        fs::write(&path, "previous").expect("写旧内容");
        fs::create_dir(path.with_file_name("tokens.json.tmp")).expect("占住临时路径");

        let err = atomic_write_text(&path, "next").expect_err("无法创建临时文件时应失败");

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "previous",
            "旧内容必须保留"
        );
        assert!(
            err.contains(&path.display().to_string()),
            "错误要带路径：{err}"
        );
        assert!(!err.contains("next"), "错误不得泄露正文：{err}");
        let _ = fs::remove_dir_all(&dir);
    }
}
