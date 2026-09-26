//! 行级差异：把「磁盘上的原文件」与「待保存文档」对齐，供预览面板的对比视图使用。
//!
//! ## 为什么需要
//!
//! 保存会把目标文件的 `provider` / `agent` 容器**整体接管**（条目与顺序都来自界面），
//! 跨格式写入还会把目标里由界面接管的容器整段丢弃后重建。用户在下手前最想知道的是
//! 「这一下到底会改掉什么」，而在此之前只能对着两份几百行 JSON 肉眼比对。
//!
//! ## 算法
//!
//! 取最长公共子序列（LCS）做对齐，用 **Hirschberg** 分治：时间仍是 O(n·m)，
//! 但空间降到 O(min(n,m))，不需要为了防内存爆掉而设「文件太大就不比了」的上限——
//! 那种上限会让恰恰最需要看对比的大文件反而看不到。
//!
//! 结果按 **hunk** 输出（改动块 + 前后各 [`CONTEXT_LINES`] 行上下文），
//! 而不是整份文件铺开：配置改动通常只落在几处，铺开全文会把真正的改动淹掉。

/// 对比视图里改动块上下各保留的上下文行数。
pub(super) const CONTEXT_LINES: usize = 3;

/// 差异行的种类，决定显示颜色与行首标记。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LineKind {
    /// 两侧都有，未改动（仅作上下文）。
    Context,
    /// 只在新内容里：本次保存会新增。
    Added,
    /// 只在旧内容里：本次保存会删除。
    Removed,
    /// `@@ -a,b +c,d @@` 改动块头。
    Hunk,
}

#[derive(Clone, Debug)]
pub(super) struct DiffLine {
    pub kind: LineKind,
    pub text: String,
}

/// 改动统计：新增 / 删除的行数（不含上下文与块头）。
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

/// 对齐的一步：`Keep` 是两侧同序的同一行，`Del` / `Ins` 是单侧独有的行。
enum Op {
    Keep(usize, usize),
    Del(usize),
    Ins(usize),
}

/// 按行切分，忽略行尾的 `\r`：磁盘文件可能是 CRLF，而界面生成的草稿是 LF，
/// 不归一的话每一行都会被判成「改了」，对比视图直接失去意义。
///
/// 行数按 `diff` 的口径：空文件是 0 行，以 `\n` 结尾的文件不把结尾那个空串算成一行
/// （`"a\n"` 是 1 行而不是 2 行）。否则「新建文件」会凭空多出一处删除。
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

/// 正向 LCS 长度表的最后一行：`res[j] = LCS(a, b[..j])`。
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

/// 反向 LCS 长度表的首行：`res[j] = LCS(a, b[j..])`。
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

/// Hirschberg 分治求 LCS 匹配对，结果按 `(旧下标, 新下标)` 升序追加到 `out`。
///
/// `a_off` / `b_off` 是当前子问题在各自全文里的起始下标，用于把递归结果还原成
/// 全文坐标（分治只处理切片，但对外必须是绝对位置）。
fn lcs_pairs(a: &[&str], b: &[&str], a_off: usize, b_off: usize, out: &mut Vec<(usize, usize)>) {
    if a.is_empty() || b.is_empty() {
        return;
    }
    if a.len() == 1 {
        // 单行：取 b 里第一处相同行即可（取最后一处也能构成一组 LCS，
        // 但取第一处让「插入到前面」的改动显示得更自然）。
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

/// 把两侧文本对齐成 `Keep` / `Del` / `Ins` 序列。
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

/// 生成对比视图的显示行（改动块 + 上下文 + `@@` 块头）与改动统计。
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

    // 标记每个 op 是否需要显示：改动本身，以及距改动不超过 context 行的相邻行。
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
        // 块头：旧侧起止行 / 新侧起止行（1 基，与 `diff -u` 同形）。
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

    /// 取出非块头的显示行，便于断言。
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
        // 磁盘上是 CRLF、草稿是 LF：不能把每一行都判成改动。
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
        // 第 2 行改动：旧侧从第 1 行起 3 行、新侧同样。
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
        // 旧侧这一块没有内容行 → 按 unified diff 惯例写作 0,0。
        assert_eq!(header.text, "@@ -0,0 +1,1 @@");
    }

    #[test]
    fn reordering_a_block_keeps_the_diff_small() {
        // 交换两行：LCS 只保留一条，另一条算「删+增」，不能变成整段重写。
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
        // 足够长、且改动在尾部：会走多轮 Hirschberg 递归。
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
        // 新建文件（磁盘侧为空）时全部算新增。
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
        // 行内的 \r 是内容的一部分，不能动。
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
        // 磁盘文件常以换行结尾，草稿渲染也以换行结尾：不能因此多报一处改动。
        let (lines, summary) = diff_hunks("a\nb\n", "a\nb", 3);
        assert!(lines.is_empty(), "仅结尾换行差异不该算改动: {lines:?}");
        assert!(summary.is_empty());
    }
}
