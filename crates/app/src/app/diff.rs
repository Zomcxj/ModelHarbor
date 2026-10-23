//! Line-level diff: aligns "the original file on disk" with "the pending document" for the preview panel's diff view.
//!
//! Uses **Hirschberg's** divide-and-conquer LCS alignment: O(n·m) time, O(min(n,m)) space. Results emit as
//! **hunks** (changed blocks + [`CONTEXT_LINES`] context lines on each side).

/// Context lines kept above and below each hunk in the diff view.
pub(super) const CONTEXT_LINES: usize = 3;

/// A diff line's kind; decides display color and the line's leading marker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LineKind {
    /// Present on both sides, unchanged (context only).
    Context,
    /// Only in the new content: this save adds it.
    Added,
    /// Only in the old content: this save removes it.
    Removed,
    /// The `@@ -a,b +c,d @@` hunk header.
    Hunk,
}

#[derive(Clone, Debug)]
pub(super) struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// Change stats: lines added / removed (context and headers excluded).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(super) struct DiffSummary {
    pub added: usize,
    pub removed: usize,
}

impl DiffSummary {
    pub(super) fn is_empty(self) -> bool {
        self.added == 0 && self.removed == 0
    }
}

/// One alignment step: `Keep` is the same line in both; `Del` / `Ins` are one-sided lines.
enum Op {
    Keep(usize, usize),
    Del(usize),
    Ins(usize),
}

/// Split by lines, ignoring trailing `\r`.
///
/// Line counts follow `diff`'s accounting: an empty file is 0 lines, and a file ending in `\n` doesn't
/// count the trailing empty string as a line (`"a\n"` is 1 line, not 2).
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// The forward LCS length table's last row: `res[j] = LCS(a, b[..j])`.
fn lcs_prefix_row(a: &[&str], b: &[&str]) -> Vec<usize> {
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for a_line in a.iter() {
        for (j, b_line) in b.iter().enumerate() {
            cur[j + 1] = if a_line == b_line {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev
}

/// The reverse LCS length table's first row: `res[j] = LCS(a, b[j..])`.
fn lcs_suffix_row(a: &[&str], b: &[&str]) -> Vec<usize> {
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for a_line in a.iter().rev() {
        for (j, b_line) in b.iter().enumerate().rev() {
            cur[j] = if a_line == b_line {
                prev[j + 1] + 1
            } else {
                prev[j].max(cur[j + 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev
}

/// Hirschberg's divide-and-conquer LCS match pairs, appended to `out` in ascending `(old index, new index)` order.
///
/// `a_off` / `b_off` are the current subproblem's offsets within each full text, used to map recursion
/// results back to whole-text coordinates.
fn lcs_pairs(a: &[&str], b: &[&str], a_off: usize, b_off: usize, out: &mut Vec<(usize, usize)>) {
    if a.is_empty() || b.is_empty() {
        return;
    }
    if a.len() == 1 {
        // Single line: take b's first matching line.
        if let Some(j) = b.iter().position(|line| *line == a[0]) {
            out.push((a_off, b_off + j));
        }
        return;
    }
    let mid = a.len() / 2;
    let left = lcs_prefix_row(&a[..mid], b);
    let right = lcs_suffix_row(&a[mid..], b);
    // LCS(a, b) = max_k [ LCS(a[..mid], b[..k]) + LCS(a[mid..], b[k..]) ]
    let mut best_k = 0;
    let mut best = 0;
    for k in 0..=b.len() {
        let total = left[k] + right[k];
        if total > best {
            best = total;
            best_k = k;
        }
    }
    lcs_pairs(&a[..mid], &b[..best_k], a_off, b_off, out);
    lcs_pairs(&a[mid..], &b[best_k..], a_off + mid, b_off + best_k, out);
}

/// Align both sides' texts into a `Keep` / `Del` / `Ins` sequence.
fn align(old: &[&str], new: &[&str]) -> Vec<Op> {
    let mut matches = Vec::new();
    lcs_pairs(old, new, 0, 0, &mut matches);
    let mut ops = Vec::with_capacity(old.len().max(new.len()));
    let (mut oi, mut ni) = (0, 0);
    for (mo, mn) in matches {
        while oi < mo {
            ops.push(Op::Del(oi));
            oi += 1;
        }
        while ni < mn {
            ops.push(Op::Ins(ni));
            ni += 1;
        }
        ops.push(Op::Keep(oi, ni));
        oi += 1;
        ni += 1;
    }
    while oi < old.len() {
        ops.push(Op::Del(oi));
        oi += 1;
    }
    while ni < new.len() {
        ops.push(Op::Ins(ni));
        ni += 1;
    }
    ops
}

/// Produce the diff view's display lines (hunks + context + `@@` headers) and change stats.
pub(super) fn diff_hunks(old: &str, new: &str, context: usize) -> (Vec<DiffLine>, DiffSummary) {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let ops = align(&old_lines, &new_lines);

    let mut summary = DiffSummary::default();
    for op in &ops {
        match op {
            Op::Del(_) => summary.removed += 1,
            Op::Ins(_) => summary.added += 1,
            Op::Keep(..) => {}
        }
    }
    if summary.is_empty() {
        return (Vec::new(), summary);
    }

    // Mark which ops need display: the changes themselves, plus neighbors within `context` lines of a change.
    let changed: Vec<bool> = ops.iter().map(|op| !matches!(op, Op::Keep(..))).collect();
    let mut show = vec![false; ops.len()];
    for (i, is_changed) in changed.iter().enumerate() {
        if !is_changed {
            continue;
        }
        let lo = i.saturating_sub(context);
        let hi = (i + context + 1).min(ops.len());
        for flag in show.iter_mut().take(hi).skip(lo) {
            *flag = true;
        }
    }

    let mut out: Vec<DiffLine> = Vec::new();
    let mut i = 0;
    while i < ops.len() {
        if !show[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < ops.len() && show[i] {
            i += 1;
        }
        // Hunk header: old-side start/end / new-side start/end (1-based, shaped like `diff -u`).
        let (mut old_start, mut new_start) = (usize::MAX, usize::MAX);
        let (mut old_count, mut new_count) = (0usize, 0usize);
        for op in &ops[start..i] {
            match op {
                Op::Keep(o, n) => {
                    old_start = old_start.min(*o);
                    new_start = new_start.min(*n);
                    old_count += 1;
                    new_count += 1;
                }
                Op::Del(o) => {
                    old_start = old_start.min(*o);
                    old_count += 1;
                }
                Op::Ins(n) => {
                    new_start = new_start.min(*n);
                    new_count += 1;
                }
            }
        }
        let fmt_range = |start: usize, count: usize| {
            if start == usize::MAX {
                "0,0".to_string()
            } else {
                format!("{},{}", start + 1, count)
            }
        };
        out.push(DiffLine {
            kind: LineKind::Hunk,
            text: format!(
                "@@ -{} +{} @@",
                fmt_range(old_start, old_count),
                fmt_range(new_start, new_count)
            ),
        });
        for op in &ops[start..i] {
            let (kind, text) = match op {
                Op::Keep(o, _) => (LineKind::Context, old_lines[*o].to_string()),
                Op::Del(o) => (LineKind::Removed, old_lines[*o].to_string()),
                Op::Ins(n) => (LineKind::Added, new_lines[*n].to_string()),
            };
            out.push(DiffLine { kind, text });
        }
    }
    (out, summary)
}

#[cfg(test)]
mod tests {
    use super::{diff_hunks, split_lines, DiffSummary, LineKind};

    /// Take the display lines excluding hunk headers, for assertions.
    fn body(old: &str, new: &str) -> Vec<(LineKind, String)> {
        diff_hunks(old, new, 3)
            .0
            .into_iter()
            .filter(|line| line.kind != LineKind::Hunk)
            .map(|line| (line.kind, line.text))
            .collect()
    }

    #[test]
    fn identical_text_has_no_hunks() {
        let (lines, summary) = diff_hunks("a\nb\nc", "a\nb\nc", 3);
        assert!(lines.is_empty(), "无改动时不该产生显示行");
        assert_eq!(summary, DiffSummary::default());
        assert!(summary.is_empty());
    }

    #[test]
    fn crlf_and_lf_are_the_same_lines() {
        let (lines, summary) = diff_hunks("a\r\nb\r\n", "a\nb\n", 3);
        assert!(lines.is_empty(), "行尾差异不该算改动: {lines:?}");
        assert!(summary.is_empty());
    }

    #[test]
    fn a_pure_addition_is_only_added() {
        let (_, summary) = diff_hunks("a\nc", "a\nb\nc", 3);
        assert_eq!(
            summary,
            DiffSummary {
                added: 1,
                removed: 0
            }
        );
        assert_eq!(
            body("a\nc", "a\nb\nc"),
            vec![
                (LineKind::Context, "a".into()),
                (LineKind::Added, "b".into()),
                (LineKind::Context, "c".into()),
            ]
        );
    }

    #[test]
    fn a_pure_removal_is_only_removed() {
        let (_, summary) = diff_hunks("a\nb\nc", "a\nc", 3);
        assert_eq!(
            summary,
            DiffSummary {
                added: 0,
                removed: 1
            }
        );
        assert_eq!(
            body("a\nb\nc", "a\nc"),
            vec![
                (LineKind::Context, "a".into()),
                (LineKind::Removed, "b".into()),
                (LineKind::Context, "c".into()),
            ]
        );
    }

    #[test]
    fn a_changed_line_shows_both_sides() {
        assert_eq!(
            body("a\nold\nc", "a\nnew\nc"),
            vec![
                (LineKind::Context, "a".into()),
                (LineKind::Removed, "old".into()),
                (LineKind::Added, "new".into()),
                (LineKind::Context, "c".into()),
            ]
        );
    }

    #[test]
    fn distant_changes_are_split_into_separate_hunks() {
        let old = (1..=30)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let new = old
            .replace("\n5\n", "\nfive\n")
            .replace("\n25\n", "\ntwentyfive\n");
        let (lines, summary) = diff_hunks(&old, &new, 1);
        let hunks = lines
            .iter()
            .filter(|line| line.kind == LineKind::Hunk)
            .count();
        assert_eq!(hunks, 2, "两处相隔很远的改动应分成两个块: {lines:?}");
        assert_eq!(
            summary,
            DiffSummary {
                added: 2,
                removed: 2
            }
        );
    }

    #[test]
    fn hunk_header_carries_one_based_line_numbers() {
        // Line 2 changed: old side starts at line 1 for 3 lines, new side the same.
        let (lines, _) = diff_hunks("a\nb\nc\nd\ne", "a\nB\nc\nd\ne", 1);
        let header = lines
            .iter()
            .find(|line| line.kind == LineKind::Hunk)
            .expect("应有块头");
        assert_eq!(header.text, "@@ -1,3 +1,3 @@");
    }

    #[test]
    fn insertion_at_the_start_anchors_the_old_side_at_zero() {
        let (lines, _) = diff_hunks("a\nb", "x\na\nb", 0);
        let header = lines
            .iter()
            .find(|line| line.kind == LineKind::Hunk)
            .expect("应有块头");
        // The old side's block has no content lines → written 0,0 per unified-diff convention.
        assert_eq!(header.text, "@@ -0,0 +1,1 @@");
    }

    #[test]
    fn reordering_a_block_keeps_the_diff_small() {
        // Swap two lines: the LCS keeps one; the other counts as "delete + insert".
        let (_, summary) = diff_hunks("a\nb\nc", "b\na\nc", 3);
        assert_eq!(
            summary,
            DiffSummary {
                added: 1,
                removed: 1
            }
        );
    }

    #[test]
    fn deep_recursion_still_finds_a_minimal_diff() {
        // Long enough with the change at the tail, exercising multiple Hirschberg recursion rounds.
        let old = (0..400)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = format!("{old}\nline-400");
        let (_, summary) = diff_hunks(&old, &new, 3);
        assert_eq!(
            summary,
            DiffSummary {
                added: 1,
                removed: 0
            },
            "尾部追加一行只应算一处新增"
        );
    }

    #[test]
    fn a_single_line_file_compares_against_empty() {
        // A new file (empty disk side) counts everything as inserted.
        let (_, summary) = diff_hunks("", "a\nb", 3);
        assert_eq!(
            summary,
            DiffSummary {
                added: 2,
                removed: 0
            }
        );
    }

    #[test]
    fn split_lines_strips_only_the_trailing_carriage_return() {
        assert_eq!(split_lines("a\r\nb"), vec!["a", "b"]);
        // A line's internal \r is content — never touched.
        assert_eq!(split_lines("a\rb"), vec!["a\rb"]);
    }

    #[test]
    fn split_lines_counts_lines_the_way_diff_does() {
        assert!(split_lines("").is_empty(), "空文件是 0 行");
        assert_eq!(split_lines("a\n"), vec!["a"], "结尾换行不算额外一行");
        assert_eq!(split_lines("a"), vec!["a"]);
        assert_eq!(split_lines("a\n\n"), vec!["a", ""], "空行本身是内容");
    }

    #[test]
    fn a_trailing_newline_alone_is_not_a_change() {
        let (lines, summary) = diff_hunks("a\nb\n", "a\nb", 3);
        assert!(lines.is_empty(), "仅结尾换行差异不该算改动: {lines:?}");
        assert!(summary.is_empty());
    }
}
