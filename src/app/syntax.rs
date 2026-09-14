//! 预览语法高亮：JSON(C) / YAML 的轻量 tokenizer（VSCode Dark+ 配色）。

use eframe::egui;

// ---------- 预览语法高亮（VSCode Dark+ 配色） ----------

/// 预览文本语法：opencode 页面为 JSON(C)，pi / omp / DSH 为 YAML。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewSyntax {
    Json,
    Yaml,
}

pub(super) const SYN_KEY: egui::Color32 = egui::Color32::from_rgb(0x9C, 0xDC, 0xFE);
pub(super) const SYN_STRING: egui::Color32 = egui::Color32::from_rgb(0xCE, 0x91, 0x78);
pub(super) const SYN_NUMBER: egui::Color32 = egui::Color32::from_rgb(0xB5, 0xCE, 0xA8);
pub(super) const SYN_LITERAL: egui::Color32 = egui::Color32::from_rgb(0x56, 0x9C, 0xD6);
pub(super) const SYN_COMMENT: egui::Color32 = egui::Color32::from_rgb(0x6A, 0x99, 0x55);
pub(super) const SYN_PUNCT: egui::Color32 = egui::Color32::from_rgb(0xD4, 0xD4, 0xD4);

/// 按语法扫描文本，返回 `(字节起, 字节止, 颜色)` 段落（边界均在字符边界上）。
pub(super) fn syntax_tokens(
    text: &str,
    syntax: PreviewSyntax,
) -> Vec<(usize, usize, egui::Color32)> {
    match syntax {
        PreviewSyntax::Json => json_tokens(text),
        PreviewSyntax::Yaml => yaml_tokens(text),
    }
}

/// JSON / JSONC：字符串（键与值分开着色）、注释、数字、字面量、标点。
pub(super) fn json_tokens(text: &str) -> Vec<(usize, usize, egui::Color32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        i = match b[i] {
            b'"' => json_string_at(b, i, &mut out),
            b'/' if b.get(i + 1) == Some(&b'/') => json_line_comment_at(b, i, &mut out),
            b'/' if b.get(i + 1) == Some(&b'*') => json_block_comment_at(b, i, &mut out),
            c if c == b'-' || c.is_ascii_digit() => json_number_at(b, i, &mut out),
            c if c.is_ascii_alphabetic() => json_literal_at(text, i, &mut out),
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                out.push((i, i + 1, SYN_PUNCT));
                i + 1
            }
            _ => i + 1,
        };
    }
    out
}

/// 扫描一个 JSON 字符串（含两侧引号）并着色，返回结束位置。
fn json_string_at(b: &[u8], start: usize, out: &mut Vec<(usize, usize, egui::Color32)>) -> usize {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => {
                i += 1;
                break;
            }
            _ => i += 1,
        }
    }
    let end = i.min(b.len());
    // 后面紧跟冒号则为对象键（VSCode Dark+ 用不同颜色）。
    let color = if json_followed_by_colon(b, end) {
        SYN_KEY
    } else {
        SYN_STRING
    };
    out.push((start, end, color));
    end
}

/// 跳过空白后是否为 `:`：用于判断字符串是对象键还是普通值。
fn json_followed_by_colon(b: &[u8], from: usize) -> bool {
    let mut j = from;
    while j < b.len() && (b[j] as char).is_ascii_whitespace() {
        j += 1;
    }
    b.get(j) == Some(&b':')
}

/// `//` 行注释：着色到行尾（不含换行）。
fn json_line_comment_at(
    b: &[u8],
    start: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    out.push((start, i, SYN_COMMENT));
    i
}

/// `/* ... */` 块注释：未闭合时着色到文本末尾。
fn json_block_comment_at(
    b: &[u8],
    start: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start + 2;
    while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
        i += 1;
    }
    let end = (i + 2).min(b.len());
    out.push((start, end, SYN_COMMENT));
    end
}

/// 数字（含前导 `-` 与指数部分）。
fn json_number_at(b: &[u8], start: usize, out: &mut Vec<(usize, usize, egui::Color32)>) -> usize {
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        i += 1;
    }
    out.push((start, i, SYN_NUMBER));
    i
}

/// 字面量 `true` / `false` / `null`；未命中则前进一个字节。
fn json_literal_at(text: &str, i: usize, out: &mut Vec<(usize, usize, egui::Color32)>) -> usize {
    let rest = &text[i..];
    for lit in ["true", "false", "null"] {
        if rest.starts_with(lit) {
            out.push((i, i + lit.len(), SYN_LITERAL));
            return i + lit.len();
        }
    }
    i + 1
}

/// YAML：注释、`---` 文档标记、列表项、键、引号/裸标量、数字、布尔。
pub(super) fn yaml_tokens(text: &str) -> Vec<(usize, usize, egui::Color32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut line_start = 0;
    while line_start <= b.len() {
        let line_end = yaml_line_end(b, line_start);
        let indent_end = yaml_indent_end(b, line_start, line_end);
        let value_start = yaml_head_token(b, indent_end, line_end, &mut out);
        yaml_value_tokens(text, b, value_start, line_start, line_end, &mut out);
        if line_end >= b.len() {
            break;
        }
        line_start = line_end + 1;
    }
    out
}

/// 当前行换行符位置（无换行则为文本末尾）。
fn yaml_line_end(b: &[u8], line_start: usize) -> usize {
    b[line_start..]
        .iter()
        .position(|&c| c == b'\n')
        .map(|p| line_start + p)
        .unwrap_or(b.len())
}

/// 跳过行首空格 / 制表符后的位置。
fn yaml_indent_end(b: &[u8], line_start: usize, line_end: usize) -> usize {
    let mut i = line_start;
    while i < line_end && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

/// 行首标记：`---` / `...` 文档标记、`- ` 列表项、`key:` 键；返回值扫描起点。
fn yaml_head_token(
    b: &[u8],
    i: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    if line_end.saturating_sub(i) >= 3 && (b[i..i + 3] == *b"---" || b[i..i + 3] == *b"...") {
        out.push((i, i + 3, SYN_PUNCT));
        return i + 3;
    }
    if i >= line_end || b[i] == b'#' {
        return i;
    }
    let mut i = i;
    // 列表项 "- "
    if b[i] == b'-' && (i + 1 >= line_end || b[i + 1] == b' ' || b[i + 1] == b'\t') {
        out.push((i, i + 1, SYN_PUNCT));
        i += 1;
    }
    // 键：到 ':' 为止（行内 '#' 起为注释）
    if let Some(colon) = yaml_key_colon(b, i, line_end) {
        if colon > i {
            out.push((i, colon, SYN_KEY));
        }
        out.push((colon, colon + 1, SYN_PUNCT));
        i = colon + 1;
    }
    i
}

/// 行内第一个键分隔冒号；遇到注释起点则视为无键。
fn yaml_key_colon(b: &[u8], from: usize, line_end: usize) -> Option<usize> {
    let mut j = from;
    while j < line_end {
        if b[j] == b'#' && (j == from || b[j - 1] == b' ' || b[j - 1] == b'\t') {
            return None;
        }
        if b[j] == b':' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// 行内值部分：空白、注释、引号标量、数字、裸标量。
fn yaml_value_tokens(
    text: &str,
    b: &[u8],
    mut i: usize,
    line_start: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) {
    while i < line_end {
        i = match b[i] {
            b' ' | b'\t' => i + 1,
            b'#' => {
                out.push((i, line_end, SYN_COMMENT));
                line_end
            }
            q @ (b'"' | b'\'') => yaml_quoted_at(b, i, line_end, q, out),
            c if c == b'-' || c.is_ascii_digit() => yaml_number_at(b, i, line_end, out),
            _ => yaml_plain_scalar(text, b, i, line_start, line_end, out),
        };
    }
}

/// 引号标量：`"` 支持反斜杠转义，`'` 不支持；未闭合时着色到行尾。
fn yaml_quoted_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    quote: u8,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start + 1;
    while i < line_end {
        if quote == b'"' && b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == quote {
            i += 1;
            break;
        }
        i += 1;
    }
    out.push((start, i.min(line_end), SYN_STRING));
    i
}

/// 数字（含前导 `-` 与指数部分）。
fn yaml_number_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    while i < line_end
        && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        i += 1;
    }
    out.push((start, i, SYN_NUMBER));
    i
}

/// 裸标量：布尔 / null / `~` 等着色为字面量，其余为字符串。
fn yaml_plain_scalar(
    text: &str,
    b: &[u8],
    start: usize,
    line_start: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    while i < line_end {
        if b[i] == b'#' && i > line_start && b[i - 1] == b' ' {
            break;
        }
        i += 1;
    }
    let color = match text[start..i].trim_end() {
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off" => SYN_LITERAL,
        _ => SYN_STRING,
    };
    out.push((start, i, color));
    i
}

/// 把查找命中的底色叠加到已按语法着色的 LayoutJob 上（按 section 拆分，保留前景色）。
pub(super) fn apply_find_background(
    job: &mut egui::text::LayoutJob,
    matches: &[(usize, usize)],
    current: usize,
) {
    for (i, &(start, end)) in matches.iter().enumerate() {
        if start >= end {
            continue;
        }
        let bg = if i == current {
            egui::Color32::from_rgb(150, 105, 25)
        } else {
            egui::Color32::from_rgb(92, 80, 28)
        };
        let mut out = Vec::with_capacity(job.sections.len() + 2);
        for section in job.sections.drain(..) {
            let (s, e) = (section.byte_range.start, section.byte_range.end);
            let (is, ie) = (start.max(s), end.min(e));
            if is >= ie {
                out.push(section);
                continue;
            }
            if s < is {
                let mut pre = section.clone();
                pre.byte_range = s..is;
                out.push(pre);
            }
            let mut mid = section.clone();
            mid.byte_range = is..ie;
            mid.leading_space = 0.0;
            mid.format.background = bg;
            out.push(mid);
            if ie < e {
                let mut post = section;
                post.byte_range = ie..e;
                post.leading_space = 0.0;
                out.push(post);
            }
        }
        job.sections = out;
    }
}
