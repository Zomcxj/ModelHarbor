//! Preview syntax highlighting: a lightweight tokenizer for JSON(C) / YAML / TOML (VSCode Dark+ palette).

use eframe::egui;

// ---------- Preview syntax highlighting (VSCode Dark+ palette) ----------

/// The preview text's syntax: opencode family / pi / ZCode / WorkBuddy / QwenCode are JSON(C);
/// omp / DSH are YAML; kimi-code is TOML.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PreviewSyntax {
    Json,
    Yaml,
    Toml,
}

/// One syntax palette (light and dark sets, picked by the current theme's background lightness).
#[derive(Clone, Copy)]
pub(super) struct SyntaxPalette {
    pub(super) key: egui::Color32,
    pub(super) string: egui::Color32,
    pub(super) number: egui::Color32,
    pub(super) literal: egui::Color32,
    pub(super) comment: egui::Color32,
    pub(super) punct: egui::Color32,
    /// The current find hit's background and its text color.
    pub(super) find_current_bg: egui::Color32,
    pub(super) find_current_fg: egui::Color32,
    /// A find hit's background and its text color.
    pub(super) find_other_bg: egui::Color32,
    pub(super) find_other_fg: egui::Color32,
    /// The hint color when preview parsing fails.
    pub(super) error: egui::Color32,
}

/// The set for dark themes (VSCode Dark+).
pub(super) const SYN_DARK: SyntaxPalette = SyntaxPalette {
    key: egui::Color32::from_rgb(0x9C, 0xDC, 0xFE),
    string: egui::Color32::from_rgb(0xCE, 0x91, 0x78),
    number: egui::Color32::from_rgb(0xB5, 0xCE, 0xA8),
    literal: egui::Color32::from_rgb(0x56, 0x9C, 0xD6),
    comment: egui::Color32::from_rgb(0x6A, 0x99, 0x55),
    punct: egui::Color32::from_rgb(0xD4, 0xD4, 0xD4),
    find_current_bg: egui::Color32::from_rgb(0xB0, 0x81, 0x1E),
    find_current_fg: egui::Color32::from_rgb(0x00, 0x00, 0x00),
    find_other_bg: egui::Color32::from_rgb(0x7A, 0x68, 0x20),
    find_other_fg: egui::Color32::from_rgb(0xFF, 0xFF, 0xFF),
    error: egui::Color32::from_rgb(0xFF, 0x8A, 0x80),
};

/// The set for light themes (VSCode Light+'s hues, deepened to read on pale backgrounds).
pub(super) const SYN_LIGHT: SyntaxPalette = SyntaxPalette {
    key: egui::Color32::from_rgb(0x04, 0x51, 0xA5),
    string: egui::Color32::from_rgb(0xA3, 0x15, 0x15),
    number: egui::Color32::from_rgb(0x0A, 0x70, 0x48),
    literal: egui::Color32::from_rgb(0x00, 0x60, 0xB0),
    comment: egui::Color32::from_rgb(0x00, 0x80, 0x00),
    punct: egui::Color32::from_rgb(0x3B, 0x3B, 0x3B),
    find_current_bg: egui::Color32::from_rgb(0xFF, 0xB8, 0x4D),
    find_current_fg: egui::Color32::from_rgb(0x1E, 0x1E, 0x1E),
    find_other_bg: egui::Color32::from_rgb(0xFF, 0xE0, 0x7A),
    find_other_fg: egui::Color32::from_rgb(0x1E, 0x1E, 0x1E),
    error: egui::Color32::from_rgb(0xB0, 0x00, 0x20),
};

impl SyntaxPalette {
    /// Pick a set by whether the theme is dark.
    pub(super) fn for_dark(dark: bool) -> Self {
        if dark {
            SYN_DARK
        } else {
            SYN_LIGHT
        }
    }
}

/// Scan text per syntax; returns `(byte start, byte end, color)` segments (boundaries all on character edges).
pub(super) fn syntax_tokens_with(
    text: &str,
    syntax: PreviewSyntax,
    pal: SyntaxPalette,
) -> Vec<(usize, usize, egui::Color32)> {
    match syntax {
        PreviewSyntax::Json => json_tokens_with(text, pal),
        PreviewSyntax::Yaml => yaml_tokens_with(text, pal),
        PreviewSyntax::Toml => toml_tokens_with(text, pal),
    }
}

/// JSON / JSONC: strings (keys and values colored separately), comments, numbers, literals, punctuation.
pub(super) fn json_tokens_with(
    text: &str,
    pal: SyntaxPalette,
) -> Vec<(usize, usize, egui::Color32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        i = match b[i] {
            b'"' => json_string_at(b, i, pal, &mut out),
            b'/' if b.get(i + 1) == Some(&b'/') => json_line_comment_at(b, i, pal, &mut out),
            b'/' if b.get(i + 1) == Some(&b'*') => json_block_comment_at(b, i, pal, &mut out),
            c if c == b'-' || c.is_ascii_digit() => json_number_at(b, i, pal, &mut out),
            c if c.is_ascii_alphabetic() => json_literal_at(text, i, pal, &mut out),
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                out.push((i, i + 1, pal.punct));
                i + 1
            }
            _ => i + 1,
        };
    }
    out
}

/// Scan and color one JSON string (both quotes included); returns the end position.
fn json_string_at(
    b: &[u8],
    start: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
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
    // A following colon makes it an object key.
    let color = if json_followed_by_colon(b, end) {
        pal.key
    } else {
        pal.string
    };
    out.push((start, end, color));
    end
}

/// After skipping whitespace, is the next char `:`: decides whether a string is an object key or a plain value.
fn json_followed_by_colon(b: &[u8], from: usize) -> bool {
    let mut j = from;
    while j < b.len() && (b[j] as char).is_ascii_whitespace() {
        j += 1;
    }
    b.get(j) == Some(&b':')
}

/// `//` line comment: colored to end of line (newline excluded).
fn json_line_comment_at(
    b: &[u8],
    start: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    out.push((start, i, pal.comment));
    i
}

/// `/* ... */` block comment: unclosed ones color to the text's end.
fn json_block_comment_at(
    b: &[u8],
    start: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start + 2;
    while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
        i += 1;
    }
    let end = (i + 2).min(b.len());
    out.push((start, end, pal.comment));
    end
}

/// Numbers (leading `-` and exponent included).
fn json_number_at(
    b: &[u8],
    start: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        i += 1;
    }
    out.push((start, i, pal.number));
    i
}

/// Literals `true` / `false` / `null`; on no match, advance one byte.
fn json_literal_at(
    text: &str,
    i: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let rest = &text[i..];
    for lit in ["true", "false", "null"] {
        if rest.starts_with(lit) {
            out.push((i, i + lit.len(), pal.literal));
            return i + lit.len();
        }
    }
    i + 1
}

/// YAML: comments, `---` document markers, list items, keys, quoted/bare scalars, numbers, booleans.
pub(super) fn yaml_tokens_with(
    text: &str,
    pal: SyntaxPalette,
) -> Vec<(usize, usize, egui::Color32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut line_start = 0;
    while line_start <= b.len() {
        let line_end = yaml_line_end(b, line_start);
        let indent_end = yaml_indent_end(b, line_start, line_end);
        let value_start = yaml_head_token(b, indent_end, line_end, pal, &mut out);
        yaml_value_tokens(text, b, value_start, pal, line_start, line_end, &mut out);
        if line_end >= b.len() {
            break;
        }
        line_start = line_end + 1;
    }
    out
}

/// The current line's newline position (text end when there is none).
fn yaml_line_end(b: &[u8], line_start: usize) -> usize {
    b[line_start..]
        .iter()
        .position(|&c| c == b'\n')
        .map(|p| line_start + p)
        .unwrap_or(b.len())
}

/// The position after skipping leading spaces / tabs.
fn yaml_indent_end(b: &[u8], line_start: usize, line_end: usize) -> usize {
    let mut i = line_start;
    while i < line_end && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

/// Line-start markers: `---` / `...` document markers, `- ` list items, `key:` keys; returns the scan start.
fn yaml_head_token(
    b: &[u8],
    i: usize,
    line_end: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    if line_end.saturating_sub(i) >= 3 && (b[i..i + 3] == *b"---" || b[i..i + 3] == *b"...") {
        out.push((i, i + 3, pal.punct));
        return i + 3;
    }
    if i >= line_end || b[i] == b'#' {
        return i;
    }
    let mut i = i;
    // List item "- "
    if b[i] == b'-' && (i + 1 >= line_end || b[i + 1] == b' ' || b[i + 1] == b'\t') {
        out.push((i, i + 1, pal.punct));
        i += 1;
    }
    // Key: up to ':' (an inline '#' starts a comment)
    if let Some(colon) = yaml_key_colon(b, i, line_end) {
        if colon > i {
            out.push((i, colon, pal.key));
        }
        out.push((colon, colon + 1, pal.punct));
        i = colon + 1;
    }
    i
}

/// The line's first key-separator colon; a comment start before it means no key.
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

/// The line's value part: whitespace, comments, quoted scalars, numbers, bare scalars.
fn yaml_value_tokens(
    text: &str,
    b: &[u8],
    mut i: usize,
    pal: SyntaxPalette,
    line_start: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) {
    while i < line_end {
        i = match b[i] {
            b' ' | b'\t' => i + 1,
            b'#' => {
                out.push((i, line_end, pal.comment));
                line_end
            }
            q @ (b'"' | b'\'') => yaml_quoted_at(b, i, line_end, q, pal, out),
            c if c == b'-' || c.is_ascii_digit() => yaml_number_at(b, i, line_end, pal, out),
            _ => yaml_plain_scalar(text, b, i, line_start, line_end, pal, out),
        };
    }
}

/// Quoted scalars: `"` supports backslash escapes, `'` doesn't; unclosed colors to end of line.
fn yaml_quoted_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    quote: u8,
    pal: SyntaxPalette,
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
    out.push((start, i.min(line_end), pal.string));
    i
}

/// Numbers (leading `-` and exponent included).
fn yaml_number_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    pal: SyntaxPalette,
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
    out.push((start, i, pal.number));
    i
}

/// Bare scalars: booleans / null / `~` etc. color as literals, the rest as strings.
fn yaml_plain_scalar(
    text: &str,
    b: &[u8],
    start: usize,
    line_start: usize,
    line_end: usize,
    pal: SyntaxPalette,
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
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off" => pal.literal,
        _ => pal.string,
    };
    out.push((start, i, color));
    i
}

/// TOML: `[table headers]`, `[[array headers]]`, `key = value`, comments, quoted strings, arrays, numbers, booleans.
///
/// The comment marker is `#` (anywhere inline), the key-value separator is `=`, and headers use brackets.
pub(super) fn toml_tokens_with(
    text: &str,
    pal: SyntaxPalette,
) -> Vec<(usize, usize, egui::Color32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut line_start = 0;
    while line_start <= b.len() {
        let line_end = toml_line_end(b, line_start);
        let i = toml_indent_end(b, line_start, line_end);
        // Whole-line comment / empty line
        if i >= line_end || b[i] == b'#' {
            if i < line_end {
                out.push((i, line_end, pal.comment));
            }
        } else if b[i] == b'[' {
            toml_table_header(b, i, line_end, pal, &mut out);
            toml_value_tokens(
                text,
                b,
                toml_after_header(b, i, line_end),
                pal,
                line_end,
                &mut out,
            );
        } else {
            let value_start = toml_key_and_eq(b, i, line_end, pal, &mut out);
            toml_value_tokens(text, b, value_start, pal, line_end, &mut out);
        }
        if line_end >= b.len() {
            break;
        }
        line_start = line_end + 1;
    }
    out
}

/// The current line's newline position (text end when there is none).
fn toml_line_end(b: &[u8], line_start: usize) -> usize {
    b[line_start..]
        .iter()
        .position(|&c| c == b'\n')
        .map(|p| line_start + p)
        .unwrap_or(b.len())
}

/// The position after skipping leading spaces / tabs.
fn toml_indent_end(b: &[u8], line_start: usize, line_end: usize) -> usize {
    let mut i = line_start;
    while i < line_end && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

/// Headers `[…]` / `[[…]]`: brackets color as punctuation, table names as keys.
///
/// Table names may be bare keys (`providers.sensenova`) or quoted; a `]` inside quotes doesn't end the
/// header, and a `#` inside quotes isn't a comment.
fn toml_table_header(
    b: &[u8],
    start: usize,
    line_end: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) {
    let mut i = start;
    let double = b.get(i + 1) == Some(&b'[');
    let brackets = if double { 2 } else { 1 };
    out.push((i, i + brackets, pal.punct));
    i += brackets;
    let name_start = i;
    let mut in_quote: Option<u8> = None;
    while i < line_end {
        let c = b[i];
        match in_quote {
            Some(q) => {
                if c == b'\\' && q == b'"' {
                    i += 2;
                    continue;
                }
                if c == q {
                    in_quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    in_quote = Some(c);
                } else if c == b']' {
                    break;
                }
            }
        }
        i += 1;
    }
    if i > name_start {
        out.push((name_start, i, pal.key));
    }
    // The closing `]` / `]]`
    let close = i.min(line_end);
    if close < line_end {
        let n = if double && b.get(close + 1) == Some(&b']') {
            2
        } else {
            1
        };
        out.push((close, (close + n).min(line_end), pal.punct));
    }
}

/// The position after `]` in a header line (theoretically only whitespace and comments remain).
fn toml_after_header(b: &[u8], start: usize, line_end: usize) -> usize {
    let mut i = start;
    while i < line_end && b[i] == b'[' {
        i += 1;
    }
    // Find the first `]` outside quotes
    let mut in_quote: Option<u8> = None;
    while i < line_end {
        let c = b[i];
        match in_quote {
            Some(q) => {
                if c == b'\\' && q == b'"' {
                    i += 2;
                    continue;
                }
                if c == q {
                    in_quote = None;
                }
            }
            None => {
                if c == b'"' || c == b'\'' {
                    in_quote = Some(c);
                } else if c == b']' {
                    break;
                }
            }
        }
        i += 1;
    }
    while i < line_end && b[i] == b']' {
        i += 1;
    }
    i
}

/// A line-leading key and `=`: the key colors as key (quoted keys and dot-separated multi-segment keys included),
/// `=` as punctuation. Returns the scan start (after the `=`).
fn toml_key_and_eq(
    b: &[u8],
    start: usize,
    line_end: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    while i < line_end && b[i] != b'=' {
        i += 1;
    }
    if i >= line_end {
        // Lines without `=` (malformed input): the whole line colors as a string.
        out.push((start, line_end, pal.string));
        return line_end;
    }
    // The key part: trim right-side whitespace before coloring.
    let mut key_end = i;
    while key_end > start && (b[key_end - 1] == b' ' || b[key_end - 1] == b'\t') {
        key_end -= 1;
    }
    if key_end > start {
        out.push((start, key_end, pal.key));
    }
    out.push((i, i + 1, pal.punct));
    i + 1
}

/// The line's value part: whitespace, comments, quoted strings, arrays / inline tables, numbers, booleans, and dates.
fn toml_value_tokens(
    text: &str,
    b: &[u8],
    mut i: usize,
    pal: SyntaxPalette,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) {
    while i < line_end {
        i = match b[i] {
            b' ' | b'\t' => i + 1,
            b'#' => {
                out.push((i, line_end, pal.comment));
                line_end
            }
            q @ (b'"' | b'\'') => toml_quoted_at(b, i, line_end, q, pal, out),
            b'[' | b']' | b'{' | b'}' | b',' => {
                out.push((i, i + 1, pal.punct));
                i + 1
            }
            c if c == b'-' || c == b'+' || c.is_ascii_digit() => {
                toml_number_at(b, i, line_end, pal, out)
            }
            _ => toml_bare_at(text, b, i, line_end, pal, out),
        };
    }
}

/// Quoted strings: `"` supports backslash escapes, `'` doesn't; unclosed colors to end of line.
fn toml_quoted_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    quote: u8,
    pal: SyntaxPalette,
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
    out.push((start, i.min(line_end), pal.string));
    i
}

/// Numbers (leading sign, decimals, exponents, underscore separators, and `-` / `:` inside dates).
fn toml_number_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    if matches!(b[i], b'-' | b'+') {
        i += 1;
    }
    while i < line_end
        && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'.' | b'_' | b'-' | b'+' | b':'))
    {
        i += 1;
    }
    out.push((start, i, pal.number));
    i
}

/// Bare values: `true` / `false` color as literals, the rest as strings.
fn toml_bare_at(
    text: &str,
    b: &[u8],
    start: usize,
    line_end: usize,
    pal: SyntaxPalette,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    while i < line_end {
        if b[i] == b'#' {
            break;
        }
        i += 1;
    }
    let color = match text[start..i].trim_end() {
        "true" | "false" => pal.literal,
        _ => pal.string,
    };
    out.push((start, i, color));
    i
}

/// Overlay find-hit backgrounds onto an already syntax-colored LayoutJob (splitting by section).
///
/// The hit region also switches to the matching foreground color.
pub(super) fn apply_find_background(
    job: &mut egui::text::LayoutJob,
    matches: &[(usize, usize)],
    current: usize,
    pal: SyntaxPalette,
) {
    for (i, &(start, end)) in matches.iter().enumerate() {
        if start >= end {
            continue;
        }
        let (bg, fg) = if i == current {
            (pal.find_current_bg, pal.find_current_fg)
        } else {
            (pal.find_other_bg, pal.find_other_fg)
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
            mid.format.color = fg;
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
