/// 错误摘要：HTTP 状态码 + 人话原因（如 `HTTP 503 服务临时不可用`）；
/// 其他错误截断为短文本。卡片 / 列表里只显示这一行。
pub(in crate::app) fn short_err(err: &str) -> String {
    if let Some(code) = http_status_code(err) {
        return crate::http_status::label(code);
    }
    let t = err.trim();
    if t.chars().count() > 96 {
        let mut s: String = t.chars().take(96).collect();
        s.push('…');
        s
    } else {
        t.to_string()
    }
}

/// 从 `HTTP 503 …` 形式的错误文本里取出状态码；取不到返回 `None`。
pub(in crate::app) fn http_status_code(err: &str) -> Option<u16> {
    let rest = err.strip_prefix("HTTP ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}
