//! Number formatting (amounts, thousands separators).
use serde_json::Value;

/// Amount formatting: `$1,234.50`.
pub fn money(value: f64) -> String {
    let sign = if value < 0.0 { "-$" } else { "$" };
    format!("{}{}", sign, grouped(value.abs(), 2))
}

/// Raw-value formatting (unit unknown): integers without decimals, non-integers keeping two.
pub(crate) fn raw_num(value: Option<&Value>) -> String {
    let Some(v) = value else {
        return "?".to_string();
    };
    if let Some(n) = v.as_f64() {
        let decimals = if (n.fract()).abs() < f64::EPSILON {
            0
        } else {
            2
        };
        grouped(n, decimals)
    } else {
        v.to_string()
    }
}

/// Thousands grouping; `decimals` is the decimal count.
fn grouped(value: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, value);
    let (int_part, frac) = match text.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (text, None),
    };
    let mut out = String::with_capacity(int_part.len() + int_part.len() / 3);
    // The content comes from format!("{:.*}")'s float output — always ASCII.
    let total = int_part.len();
    for (idx, ch) in int_part.char_indices() {
        if idx > 0 && (total - idx) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    match frac {
        Some(f) => format!("{}.{}", out, f),
        None => out,
    }
}
