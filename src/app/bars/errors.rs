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

/// 网络错误文本脱敏：剥离 URL 的 query/fragment 后返回。
///
/// ureq 的 Transport Display 会带上目标 URL。
pub(crate) fn sanitize_network_error(text: &str) -> String {
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
