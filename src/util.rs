use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// 构造 wsl 命令：Windows 下带 CREATE_NO_WINDOW，
/// 避免 GUI 程序拉起控制台进程（wsl.exe）时闪现终端窗口。
fn wsl_command() -> Command {
    let mut cmd = Command::new("wsl");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
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

/// JSON 数字 → 文本：优先 i64/u64 精确表示，避免大整数经 f64 丢精度。
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
        m.insert(k.into(), f.into());
    } else {
        m.remove(k);
    }
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
        return Some(f.into());
    }
    None
}

pub fn ensure_parent_dir(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
    }
    Ok(())
}

/// 原子写入文本文件：先写同目录临时文件并 `sync_all`，再替换正式文件。
///
/// Windows 的 rename 不会覆盖已存在的文件，因此先把旧文件移到备份名，替换成功后删除
/// 备份；替换失败立刻把旧文件移回，绝不留下半写内容。错误文本只含路径，不含正文
/// （设置与令牌文件可能包含敏感内容，不能让诊断信息把它们带进日志）。
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

/// 用同目录临时文件替换目标。已存在时先移到备份名，失败则回滚（Windows 的 rename
/// 不覆盖现有文件）。
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

/// WSL 默认发行版的 $HOME（进程级缓存：每次 `wsl` 调用约需数百毫秒，不可重复探测）。
pub fn wsl_home() -> Option<String> {
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

/// WSL 批量探测的单项结果。
#[derive(Clone, Copy, Default, Debug)]
pub struct WslPathProbe {
    /// `test -f`：路径是常规文件。
    pub file_exists: bool,
    /// `test -e`：路径存在（任意类型，含文件）。
    pub path_exists: bool,
    /// 父目录存在（"已安装" 判定的宽松条件）。
    pub parent_dir_exists: bool,
}

/// 单次 `wsl` 调用批量探测多个路径的存在状态。
/// 每项依次输出 `F`（文件）/ `E`（存在非文件）/ `P`（父目录存在）/ `N`（均不存在）。
pub fn wsl_batch_probe(paths: &[String]) -> Vec<WslPathProbe> {
    let fallback = || vec![WslPathProbe::default(); paths.len()];
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

/// WSL 侧路径是否为常规文件（单次探测，用于按需读取前的存在性检查）。
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
    // 固定名临时文件可能被本地恶意进程预置同名符号链接指向受害文件，
    // 且内容含 API Key 明文；改用带纳秒时间的随机名降低风险。
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

/// 文件对话框支持的扩展名。
///
/// **首个滤镜就是 Windows 对话框的默认选中项**，而对话框会按选中滤镜过滤列表：
/// 只写 json 会让 `.yml` / `.yaml`（oh-my-pi 的 `models.yml`、DSH 的 `settings.yaml`）
/// 在「浏览」时直接不可见 —— 即使用户手动切到 YAML 滤镜也容易被误认为「不支持 yml」。
/// 单测 `dialog_extensions_cover_backend_defaults` 会用各后端的默认路径反向守住这份清单。
const CONFIG_FILE_EXTENSIONS: &[&str] = &["json", "jsonc", "yml", "yaml"];

pub fn show_file_dialog() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("选择配置文件")
        .add_filter("配置文件（JSON / YAML）", CONFIG_FILE_EXTENSIONS)
        .add_filter("JSON", &["json", "jsonc"])
        .add_filter("YAML", &["yml", "yaml"])
        .add_filter("所有文件", &["*"])
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

/// 剥离 JSONC 的行注释、块注释与尾逗号，产出可被 serde_json 解析的 JSON。
/// 字符串字面量内的注释符与逗号不受影响；注释中的换行保留以维持行号。
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

/// 移除 `}` / `]` 前的尾逗号（JSONC 允许，严格 JSON 不允许）。
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

/// 读取配置文件内容（支持本地与 WSL 路径）；不存在返回空串。
pub fn config_exists(path: &str) -> bool {
    if is_wsl_path(path) {
        wsl_file_exists(path)
    } else {
        Path::new(path).is_file()
    }
}

/// 读取配置文件内容（支持本地与 WSL 路径）；不存在返回空串。
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

/// 解析配置内容（支持 JSONC 注释与尾逗号）；空内容视为空对象。
pub fn parse_config_content(content: &str) -> Result<serde_json::Value, String> {
    if content.trim().is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    let stripped = strip_jsonc_comments(content);
    serde_json::from_str(&stripped).map_err(|e| format!("解析失败: {}", e))
}

/// 解析 YAML 配置内容；空内容视为空对象。
pub fn parse_yaml_content(content: &str) -> Result<serde_json::Value, String> {
    if content.trim().is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    serde_yaml_ng::from_str(content).map_err(|e| format!("解析失败: {}", e))
}

/// 将 Value 序列化为块风格 YAML 文本。
pub fn to_yaml_string(value: &serde_json::Value) -> Result<String, String> {
    serde_yaml_ng::to_string(value).map_err(|e| format!("序列化失败: {}", e))
}

/// baseUrl 体检：返回可疑点标签（**仅供界面提示，绝不自动改写配置**）。
///
/// 覆盖重复斜杠（`https://host//v1`）、末尾多余斜杠、缺少协议头与夹带空白字符；
/// 只做字符串体检，不联网、不依赖方言语义。
pub fn url_suspicions(url: &str) -> Vec<&'static str> {
    let trimmed = url.trim();
    let mut out = Vec::new();
    if trimmed.is_empty() {
        return out;
    }
    // 空白检查基于原值：首尾空格会被 trim 抹掉，但那正是要提示的情况之一。
    if url.chars().any(char::is_whitespace) {
        out.push("含空白字符");
    }
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        out.push("缺少 http(s):// 协议头");
    }
    // 只检查协议之后的路径部分，避免把 `https://` 自身的双斜杠算进来。
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
    use super::{atomic_write_text, url_suspicions, CONFIG_FILE_EXTENSIONS};
    use std::fs;
    use std::path::PathBuf;

    /// 反向守住对话框滤镜：每个后端的默认配置扩展名都必须在列表里。
    ///
    /// Windows 对话框按首项滤镜过滤列表：首项必须覆盖 json / jsonc / yml / yaml，
    /// 否则 oh-my-pi / DSH 的 `.yml` / `.yaml` 在「浏览」时看不到。
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
        // JSON 与 YAML 两侧都不能漏：首项滤镜必须同时覆盖两者。
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
        // 中间夹带空白（粘贴带空格）与首尾空格都要提示。
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
        // 用目录占住临时文件路径，让原子写无法创建临时文件。
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
