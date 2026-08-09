//! Line-based diff generation for write previews.
//!
//! Implements a small LCS line diff (no external dependency) with a
//! fallback for very large inputs. Used by the confirm gate to show the
//! build model what `write_file` is about to change.

/// How one line relates to the other version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Present in both versions (context).
    Same,
    /// Only in the new version.
    Add,
    /// Only in the old version.
    Remove,
}

/// One line of a computed diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// Guard against quadratic LCS memory: if `old * new` exceeds this many
/// cells we fall back to the prefix/suffix approximation.
const LCS_CELL_LIMIT: usize = 4_000_000;

/// Compute a line diff between `old` and `new`.
///
/// Lines are compared exactly (no whitespace normalization). Empty inputs
/// produce all-add / all-remove diffs as expected.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    if old_lines.is_empty() {
        return new_lines
            .into_iter()
            .map(|l| DiffLine {
                kind: DiffLineKind::Add,
                text: l.to_string(),
            })
            .collect();
    }
    if new_lines.is_empty() {
        return old_lines
            .into_iter()
            .map(|l| DiffLine {
                kind: DiffLineKind::Remove,
                text: l.to_string(),
            })
            .collect();
    }
    if old_lines.len().saturating_mul(new_lines.len()) > LCS_CELL_LIMIT {
        return approximate_diff(&old_lines, &new_lines);
    }
    lcs_diff(&old_lines, &new_lines)
}

/// Full LCS dynamic program with backtracking. `old`/`new` must be non-empty.
fn lcs_diff(old: &[&str], new: &[&str]) -> Vec<DiffLine> {
    let n = old.len();
    let m = new.len();
    // dp[i][j] = LCS length of old[i..] and new[j..].
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut out = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            out.push(DiffLine {
                kind: DiffLineKind::Same,
                text: old[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push(DiffLine {
                kind: DiffLineKind::Remove,
                text: old[i].to_string(),
            });
            i += 1;
        } else {
            out.push(DiffLine {
                kind: DiffLineKind::Add,
                text: new[j].to_string(),
            });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine {
            kind: DiffLineKind::Remove,
            text: old[i].to_string(),
        });
        i += 1;
    }
    while j < m {
        out.push(DiffLine {
            kind: DiffLineKind::Add,
            text: new[j].to_string(),
        });
        j += 1;
    }
    out
}

/// Cheap diff for very large inputs: keep the common prefix and suffix,
/// mark everything in between as removed + added.
fn approximate_diff(old: &[&str], new: &[&str]) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        out.push(DiffLine {
            kind: DiffLineKind::Same,
            text: old[prefix].to_string(),
        });
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    for line in &old[prefix..old.len() - suffix] {
        out.push(DiffLine {
            kind: DiffLineKind::Remove,
            text: (*line).to_string(),
        });
    }
    for line in &new[prefix..new.len() - suffix] {
        out.push(DiffLine {
            kind: DiffLineKind::Add,
            text: (*line).to_string(),
        });
    }
    for line in &old[old.len() - suffix..] {
        out.push(DiffLine {
            kind: DiffLineKind::Same,
            text: (*line).to_string(),
        });
    }
    out
}

/// Render a diff as +/- lines, truncating to `max_lines` (at most).
///
/// Lines are prefixed: `"  "` for context, `"- "` for removals, `"+ "` for
/// additions. When truncated, a `... (N more lines)` marker is inserted.
pub fn render_diff(diff: &[DiffLine], max_lines: usize) -> String {
    let mut out = String::new();
    let total = diff.len();
    if total <= max_lines {
        for line in diff {
            push_rendered(&mut out, line);
        }
        return out;
    }
    let head = max_lines / 2;
    let tail = max_lines - head;
    for line in &diff[..head] {
        push_rendered(&mut out, line);
    }
    out.push_str(&format!("... ({} more lines)\n", total - max_lines));
    for line in &diff[total - tail..] {
        push_rendered(&mut out, line);
    }
    out
}

fn push_rendered(out: &mut String, line: &DiffLine) {
    match line.kind {
        DiffLineKind::Same => out.push_str("  "),
        DiffLineKind::Add => out.push_str("+ "),
        DiffLineKind::Remove => out.push_str("- "),
    }
    out.push_str(&line.text);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_detects_added_lines() {
        let diff = diff_lines("a\nb\n", "a\nx\nb\n");
        let kinds: Vec<DiffLineKind> = diff.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![DiffLineKind::Same, DiffLineKind::Add, DiffLineKind::Same,]
        );
        assert_eq!(diff[1].text, "x");
    }

    #[test]
    fn diff_detects_removed_lines() {
        let diff = diff_lines("a\nb\nc\n", "a\nc\n");
        let kinds: Vec<DiffLineKind> = diff.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![DiffLineKind::Same, DiffLineKind::Remove, DiffLineKind::Same]
        );
        assert_eq!(diff[1].text, "b");
    }

    #[test]
    fn diff_detects_changed_lines() {
        let diff = diff_lines("one\ntwo\nthree\n", "one\nTWO\nthree\n");
        let kinds: Vec<DiffLineKind> = diff.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffLineKind::Same,
                DiffLineKind::Remove,
                DiffLineKind::Add,
                DiffLineKind::Same,
            ]
        );
        assert_eq!(diff[1].text, "two");
        assert_eq!(diff[2].text, "TWO");
    }

    #[test]
    fn diff_identical_inputs_is_all_context() {
        let diff = diff_lines("a\nb\n", "a\nb\n");
        assert!(diff.iter().all(|l| l.kind == DiffLineKind::Same));
    }

    #[test]
    fn render_marks_add_and_remove() {
        let diff = diff_lines("a\n", "b\n");
        let rendered = render_diff(&diff, 10);
        assert!(rendered.contains("- a"));
        assert!(rendered.contains("+ b"));
    }

    #[test]
    fn render_truncates_long_diffs() {
        let old: String = (0..100).map(|i| format!("old{i}\n")).collect();
        let new: String = (0..100).map(|i| format!("new{i}\n")).collect();
        let diff = diff_lines(&old, &new);
        assert!(diff.len() > 40);
        let rendered = render_diff(&diff, 40);
        assert!(rendered.contains("... ("));
        // 40 lines shown: 20 head + 20 tail, plus the ellipsis line.
        assert_eq!(rendered.lines().count(), 41);
    }

    #[test]
    fn diff_empty_old_is_all_add() {
        let diff = diff_lines("", "x\ny\n");
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|l| l.kind == DiffLineKind::Add));
    }

    #[test]
    fn diff_empty_new_is_all_remove() {
        let diff = diff_lines("x\ny\n", "");
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|l| l.kind == DiffLineKind::Remove));
    }
}
