//! 数字格式化（金额、千分位）。
use serde_json::Value;

/// 金额格式化：`$1,234.50`。
pub fn money(value: f64) -> String {
    let sign = if value < 0.0 { "-$" } else { "$" };
    format!("{}{}", sign, grouped(value.abs(), 2))
}

/// 原值格式化（单位未知）：整数不带小数，非整数保留两位。
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

/// 千分位分组；`decimals` 为小数位数。
fn grouped(value: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, value);
    let (int_part, frac) = match text.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (text, None),
    };
    let mut out = String::with_capacity(int_part.len() + int_part.len() / 3);
    // 内容来自 format!("{:.*}") 的浮点输出，必定是 ASCII，故字节下标与字符数一致。
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
