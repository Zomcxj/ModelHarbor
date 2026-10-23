/// Error summary: HTTP status code + human reason (like `HTTP 503 service temporarily unavailable`);
/// other errors truncate to short text. Cards / lists show only this line.
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

/// Extract the status code from error text shaped `HTTP 503 …`; `None` when absent.
pub(in crate::app) fn http_status_code(err: &str) -> Option<u16> {
    let rest = err.strip_prefix("HTTP ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}
