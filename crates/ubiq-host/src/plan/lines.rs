//! The line diff a plan's edit provenance is built from: which lines of the new body are new or
//! rewritten, and how many were added, removed and changed on the way.
//!
//! **Written here rather than taken from `similar`**, which this crate already depends on, because
//! that dependency is behind the `git` feature and the plan module is behind `harness`. Widening
//! `harness` to pull a diff library in — or moving `similar` out of `git` — would be a bigger
//! statement than this needs: a plan is a hand-written markdown document, the comparison is
//! line-wise and whole-word, and the whole algorithm is a prefix trim and a longest-common-
//! subsequence walk. If a second caller in this crate ever wants a line diff outside `git`, that
//! is the moment to reach for the library instead.
//!
//! **The output is indexed by the new body's lines**, not by a hunk list, because that is what the
//! provenance layer stores: one stamp per line of the document as it now stands, which is the only
//! form that survives the *next* save moving every line number. See [`super::provenance`].
//!
//! **A deletion has no line of its own**, and that is the one place this makes a judgement call.
//! A hunk that removed lines and added none leaves nothing in the new body to stamp, so it stamps
//! the line that now stands where the deleted ones were — the surviving neighbour below, or the
//! last line when the deletion ran off the end. The alternative is for a human deleting a
//! paragraph to be invisible to the agent that wrote it, which is the worse answer: the count in
//! [`LineDiff::removed`] is exact either way, and the stamp is what makes the place findable.

/// How many cells of the longest-common-subsequence table this will fill before giving up and
/// calling the whole differing middle one rewrite.
///
/// A plan is a document a person reads, so the middle being over two thousand lines different on
/// both sides means the document was replaced rather than edited, and reporting it as one large
/// rewrite is both true and what a reader wants. The cap is what keeps a pathological save from
/// allocating a table the size of the document squared.
const MAX_TABLE_CELLS: usize = 2_000 * 2_000;

/// What one save changed, in lines.
///
/// Both vectors are one entry per line of the **new** body, in order, which is what lets the
/// caller walk the new document once and decide each line's stamp without holding the old one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineDiff {
    /// `true` where the line was added or rewritten by this save, or is the surviving neighbour
    /// of a pure deletion.
    pub touched: Vec<bool>,
    /// The old body's line this one carried over from, where it carried over from any — the
    /// mapping the provenance layer needs to bring an untouched line's existing stamp forward
    /// rather than restamping the whole document on every save.
    pub source: Vec<Option<usize>>,
    /// Lines in the new body with no counterpart in the old.
    pub added: u32,
    /// Lines in the old body with no counterpart in the new.
    pub removed: u32,
    /// Lines that were replaced — a deletion and an insertion in the same hunk, paired off.
    pub modified: u32,
}

impl LineDiff {
    /// One entry per new line, nothing touched and nothing yet traced back.
    fn blank(lines: usize) -> Self {
        Self {
            touched: vec![false; lines],
            source: vec![None; lines],
            ..Self::default()
        }
    }

    /// Whether this save changed anything in the body.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0 && self.modified == 0
    }
}

/// The lines of a body, as the diff and the provenance layer both count them.
///
/// `str::lines` throughout, so a trailing newline does not conjure an empty last line and the two
/// sides of a comparison are counted the same way. Line *numbers* elsewhere are 1-based; these
/// indices are not.
pub fn split(body: &str) -> Vec<&str> {
    body.lines().collect()
}

/// Compare two bodies line-wise.
pub fn diff(previous: &str, current: &str) -> LineDiff {
    let old = split(previous);
    let new = split(current);

    if old == new {
        let mut diff = LineDiff::blank(new.len());
        diff.source = (0..new.len()).map(Some).collect();
        return diff;
    }

    // The common prefix and suffix are almost the whole document on a typical edit, and trimming
    // them is what keeps the table small enough to build at all.
    let prefix = old
        .iter()
        .zip(new.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let old_middle = &old[prefix..old.len() - suffix];
    let new_middle = &new[prefix..new.len() - suffix];

    let mut diff = LineDiff::blank(new.len());
    // The trimmed ends carried over by definition, so they trace straight back.
    for line in 0..prefix {
        diff.source[line] = Some(line);
    }
    for back in 0..suffix {
        diff.source[new.len() - 1 - back] = Some(old.len() - 1 - back);
    }

    if old_middle.is_empty() {
        // A pure insertion: every line of the new middle is added, and there is nothing to pair.
        diff.added = new_middle.len() as u32;
        for line in prefix..prefix + new_middle.len() {
            diff.touched[line] = true;
        }
        return diff;
    }

    if new_middle.is_empty() {
        // A pure deletion: nothing in the new body is new, so the surviving neighbour carries the
        // mark. See the module doc.
        diff.removed = old_middle.len() as u32;
        mark_deletion(&mut diff, prefix, new.len());
        return diff;
    }

    if old_middle.len().saturating_mul(new_middle.len()) > MAX_TABLE_CELLS {
        // Too far apart to align line by line: the middle was replaced, and saying so is more
        // useful than an alignment nobody would trust.
        let paired = old_middle.len().min(new_middle.len()) as u32;
        diff.modified = paired;
        diff.added = new_middle.len() as u32 - paired;
        diff.removed = old_middle.len() as u32 - paired;
        for line in prefix..prefix + new_middle.len() {
            diff.touched[line] = true;
        }
        return diff;
    }

    let (hunks, carried) = hunks(old_middle, new_middle);
    for (old_line, new_line) in carried {
        diff.source[prefix + new_line] = Some(prefix + old_line);
    }

    for hunk in hunks {
        let paired = hunk.deleted.min(hunk.inserted.len());
        diff.modified += paired as u32;
        diff.added += (hunk.inserted.len() - paired) as u32;
        diff.removed += (hunk.deleted - paired) as u32;

        if hunk.inserted.is_empty() {
            mark_deletion(&mut diff, prefix + hunk.at, new.len());
        } else {
            for line in hunk.inserted {
                diff.touched[prefix + line] = true;
            }
        }
    }

    diff
}

/// Stamp the line that now stands where deleted lines used to be: the one below the gap, or the
/// last line when the deletion ran off the end. A body with no lines left has nowhere to put it,
/// and the counts still say what happened.
fn mark_deletion(diff: &mut LineDiff, at: usize, lines: usize) {
    if lines == 0 {
        return;
    }
    diff.touched[at.min(lines - 1)] = true;
}

/// One run of lines that did not survive unchanged.
struct Hunk {
    /// How many old lines the hunk dropped.
    deleted: usize,
    /// Which new-middle lines it introduced, by index into the new middle.
    inserted: Vec<usize>,
    /// Where the hunk sits in the new middle — the index of the line that follows it, which is
    /// also where an insertion would have started. Only a pure deletion needs it.
    at: usize,
}

/// Align two middles by longest common subsequence: the hunks that did not match, and the pairs
/// that did — `(old middle index, new middle index)`, which is the mapping an untouched line's
/// stamp travels along.
fn hunks(old: &[&str], new: &[&str]) -> (Vec<Hunk>, Vec<(usize, usize)>) {
    // `table[i][j]` is the length of the longest common subsequence of `old[i..]` and `new[j..]`.
    // Built from the back so the walk below can read it forwards and stay in document order,
    // which is what makes the hunks come out in order too.
    let mut table = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i][j] = if old[i] == new[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut out: Vec<Hunk> = Vec::new();
    let mut carried: Vec<(usize, usize)> = Vec::new();
    let mut deleted = 0usize;
    let mut inserted: Vec<usize> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);

    let mut flush = |deleted: &mut usize, inserted: &mut Vec<usize>, at: usize| {
        if *deleted > 0 || !inserted.is_empty() {
            out.push(Hunk {
                deleted: *deleted,
                inserted: std::mem::take(inserted),
                at,
            });
            *deleted = 0;
        }
    };

    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            flush(&mut deleted, &mut inserted, j);
            carried.push((i, j));
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || table[i][j + 1] >= table[i + 1][j]) {
            inserted.push(j);
            j += 1;
        } else {
            deleted += 1;
            i += 1;
        }
    }
    flush(&mut deleted, &mut inserted, j);

    (out, carried)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 1-based line numbers the diff stamped.
    fn touched(diff: &LineDiff) -> Vec<u32> {
        diff.touched
            .iter()
            .enumerate()
            .filter(|(_, hit)| **hit)
            .map(|(line, _)| line as u32 + 1)
            .collect()
    }

    #[test]
    fn an_identical_body_changes_nothing() {
        let diff = diff("a\nb\nc", "a\nb\nc");
        assert!(diff.is_empty());
        assert_eq!(touched(&diff), Vec::<u32>::new());
        assert_eq!(diff.touched.len(), 3, "still one entry per line");
    }

    #[test]
    fn an_empty_previous_body_is_all_addition() {
        let diff = diff("", "a\nb");
        assert_eq!((diff.added, diff.removed, diff.modified), (2, 0, 0));
        assert_eq!(touched(&diff), vec![1, 2]);
    }

    #[test]
    fn a_rewritten_line_is_a_modification_not_an_add_and_a_remove() {
        let diff = diff("a\nb\nc", "a\nB\nc");
        assert_eq!((diff.added, diff.removed, diff.modified), (0, 0, 1));
        assert_eq!(touched(&diff), vec![2]);
    }

    #[test]
    fn an_insertion_above_does_not_stamp_the_lines_it_pushed_down() {
        let diff = diff("a\nb", "new\na\nb");
        assert_eq!((diff.added, diff.removed, diff.modified), (1, 0, 0));
        assert_eq!(
            touched(&diff),
            vec![1],
            "only the inserted line, not everything below it",
        );
    }

    #[test]
    fn a_deletion_stamps_the_line_that_took_its_place() {
        let diff = diff("a\ngone\nb", "a\nb");
        assert_eq!((diff.added, diff.removed, diff.modified), (0, 1, 0));
        assert_eq!(
            touched(&diff),
            vec![2],
            "the surviving neighbour carries the mark",
        );
    }

    #[test]
    fn a_deletion_off_the_end_stamps_the_last_line() {
        let diff = diff("a\nb\ngone", "a\nb");
        assert_eq!(diff.removed, 1);
        assert_eq!(touched(&diff), vec![2]);
    }

    #[test]
    fn deleting_everything_stamps_nothing_and_still_counts() {
        let diff = diff("a\nb", "");
        assert_eq!((diff.added, diff.removed, diff.modified), (0, 2, 0));
        assert!(diff.touched.is_empty());
    }

    #[test]
    fn two_separate_edits_stamp_two_separate_places() {
        let previous = "one\ntwo\nthree\nfour\nfive";
        let current = "one\nTWO\nthree\nfour\nFIVE";
        let diff = diff(previous, current);
        assert_eq!((diff.added, diff.removed, diff.modified), (0, 0, 2));
        assert_eq!(touched(&diff), vec![2, 5]);
    }

    #[test]
    fn a_hunk_that_grows_pairs_what_it_can_and_adds_the_rest() {
        let diff = diff("a\nx\nb", "a\ny\nz\nb");
        assert_eq!((diff.added, diff.removed, diff.modified), (1, 0, 1));
        assert_eq!(touched(&diff), vec![2, 3]);
    }

    #[test]
    fn an_untouched_line_traces_back_to_where_it_came_from() {
        // "b" moved from index 1 to index 2, and its stamp has to move with it.
        let diff = diff("a\nb\nc", "a\nnew\nb\nc");
        assert_eq!(diff.source, vec![Some(0), None, Some(1), Some(2)]);
        assert_eq!(touched(&diff), vec![2]);
    }

    #[test]
    fn a_wholesale_replacement_beyond_the_table_cap_is_one_rewrite() {
        let previous: String = (0..2_500).map(|n| format!("old {n}\n")).collect();
        let current: String = (0..2_500).map(|n| format!("new {n}\n")).collect();
        let diff = diff(&previous, &current);
        assert_eq!(diff.modified, 2_500);
        assert_eq!((diff.added, diff.removed), (0, 0));
        assert!(diff.touched.iter().all(|hit| *hit));
    }
}
