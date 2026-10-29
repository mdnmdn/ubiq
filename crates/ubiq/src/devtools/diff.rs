//! A line diff of two texts, in the contract's [`FileDiff`] shape so `ui::viewer::diff` draws it
//! exactly as it draws a file's change against Git.
//!
//! The one place a diff library reaches the interface: these are two texts the user pasted or
//! dropped into the Tools dialog, which the host never held and so has nothing to compare. Same
//! library, same context width and the same row rules as the host's `files::diff`.

use similar::{ChangeTag, TextDiff};
use ubiq_proto::files::{DiffBase, DiffHunk, DiffRow, DiffRowKind, FileDiff};

/// Lines of context around a change — the host's own number.
const CONTEXT: usize = 3;
/// Rows past which the diff stops and says it was cut.
const MAX_ROWS: usize = 20_000;
/// How long the diff algorithm may search before settling for a coarser answer.
const DIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

pub fn line_diff(old: &str, new: &str) -> FileDiff {
    // Myers is quadratic on pathological input; the deadline makes it fall back to a coarser (still
    // correct) diff rather than freeze the window on two large, very different texts.
    let comparison = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(old, new);
    let mut hunks = Vec::new();
    let mut total = 0usize;
    let mut truncated = false;

    'groups: for group in comparison.grouped_ops(CONTEXT) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let mut rows = Vec::new();
        for op in &group {
            for change in comparison.iter_changes(op) {
                if total + rows.len() >= MAX_ROWS {
                    truncated = true;
                }
                if truncated {
                    break;
                }
                rows.push(DiffRow {
                    kind: match change.tag() {
                        ChangeTag::Equal => DiffRowKind::Context,
                        ChangeTag::Delete => DiffRowKind::Removed,
                        ChangeTag::Insert => DiffRowKind::Added,
                    },
                    old_line: change.old_index().map(one_based),
                    new_line: change.new_index().map(one_based),
                    text: line(change.value()),
                });
            }
        }
        total += rows.len();
        let old_range = first.old_range().start..last.old_range().end;
        let new_range = first.new_range().start..last.new_range().end;
        hunks.push(DiffHunk {
            old_start: start(old_range.start, old_range.len()),
            old_lines: old_range.len() as u32,
            new_start: start(new_range.start, new_range.len()),
            new_lines: new_range.len() as u32,
            rows,
        });
        if truncated {
            break 'groups;
        }
    }

    FileDiff {
        // Not a version-control comparison at all; the renderer never reads it.
        base: DiffBase::Head,
        hunks,
        binary: false,
        truncated,
        ignore_eol: false,
    }
}

/// Lines added and removed, for the section's summary.
pub fn counts(diff: &FileDiff) -> (usize, usize) {
    diff.hunks
        .iter()
        .flat_map(|hunk| &hunk.rows)
        .fold((0, 0), |(added, removed), row| match row.kind {
            DiffRowKind::Added => (added + 1, removed),
            DiffRowKind::Removed => (added, removed + 1),
            DiffRowKind::Context => (added, removed),
        })
}

fn start(index: usize, lines: usize) -> u32 {
    (if lines == 0 { index } else { index + 1 }) as u32
}

fn one_based(index: usize) -> u32 {
    (index as u32).saturating_add(1)
}

fn line(value: &str) -> String {
    let value = value.strip_suffix('\n').unwrap_or(value);
    value.strip_suffix('\r').unwrap_or(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_have_no_hunks() {
        assert!(line_diff("a\nb\n", "a\nb\n").hunks.is_empty());
    }

    #[test]
    fn one_changed_line() {
        let diff = line_diff("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(counts(&diff), (1, 1));
        let hunk = &diff.hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines), (1, 3));
        let removed = hunk
            .rows
            .iter()
            .find(|r| r.kind == DiffRowKind::Removed)
            .unwrap();
        assert_eq!((removed.text.as_str(), removed.old_line), ("b", Some(2)));
        let added = hunk
            .rows
            .iter()
            .find(|r| r.kind == DiffRowKind::Added)
            .unwrap();
        assert_eq!((added.text.as_str(), added.new_line), ("B", Some(2)));
    }

    #[test]
    fn from_empty() {
        let diff = line_diff("", "x\ny");
        assert_eq!(counts(&diff), (2, 0));
        assert_eq!(diff.hunks[0].old_start, 0);
    }
}
