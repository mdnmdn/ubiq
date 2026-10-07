//! The three-way text merge an annotated document is kept whole by, when a person and an agent
//! write it at the same time.
//!
//! **Shared logic rather than a message**, on `blocks`' own footing: the host merges an agent's
//! `write_doc` against the text the agent last read, the window merges a file that moved on disk
//! into the buffer the user is typing in, and the two must agree on what a conflict is — while
//! neither half may depend on the other.
//!
//! The merge is line-level first. A region both sides changed differently is merged again at word
//! granularity, so two edits to the same paragraph that touch different words both land. What is
//! still contested after that is a [`Conflict`]: **ours wins in the text**, and the conflict carries
//! theirs so the caller can put it somewhere the losing side can still read it (an annotation
//! thread). Two insertions at the same point are not a conflict: both are kept, ours first — the
//! document grows rather than refuses.

use std::ops::Range;

use std::time::{Duration, Instant};

use similar::{Algorithm, DiffTag, capture_diff_slices_deadline};

/// What [`merge3`] made of three versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merged {
    /// The merged text — ours wherever the two sides contest a region.
    pub text: String,
    /// Every region kept as ours over a different change of theirs, in document order.
    pub conflicts: Vec<Conflict>,
}

/// One contested region: the base it started from and what each side made of it, as whole lines.
/// On the wire in `Message::DocMergeConflicts`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Conflict {
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

/// How long one diff may search before settling for a coarser answer — a document is merged on a
/// keystroke's budget, never a pathological one's.
const DIFF_BUDGET: Duration = Duration::from_millis(150);

/// The largest contested region (base, ours and theirs together, in bytes) the word-level pass is
/// tried on. Past it the region is a conflict outright: a word diff over pages is slow and its
/// answer unreadable anyway.
const WORD_PASS_LIMIT: usize = 16 * 1024;

/// Merge `ours` and `theirs`, both edited from `base`. See the module doc for the rules.
///
/// Line endings are compared as `\n` and the result is written in **ours'** endings, so a CRLF
/// file stays CRLF whoever wrote the other side. A last line with no newline is merged as if it
/// had one, which is what lets two appends after it both land; the newline is taken back off when
/// neither side's text ended with one.
pub fn merge3(base: &str, ours: &str, theirs: &str) -> Merged {
    if ours == theirs || theirs == base {
        return Merged {
            text: ours.to_string(),
            conflicts: Vec::new(),
        };
    }
    if ours == base {
        return Merged {
            text: theirs.to_string(),
            conflicts: Vec::new(),
        };
    }
    let crlf = ours.contains("\r\n");
    let open_end = !ours.is_empty() && !ours.ends_with('\n') && !theirs.ends_with('\n');
    let norm = |text: &str| {
        let mut text = text.replace("\r\n", "\n");
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text
    };
    let (b, o, t) = (norm(base), norm(ours), norm(theirs));
    let mut out = String::with_capacity(o.len().max(t.len()));
    let mut conflicts = Vec::new();
    merge_level(
        &lines(&b),
        &lines(&o),
        &lines(&t),
        true,
        &mut out,
        &mut conflicts,
    );
    if open_end && out.ends_with('\n') {
        out.pop();
    }
    if crlf {
        out = out.replace('\n', "\r\n");
    }
    Merged {
        text: out,
        conflicts,
    }
}

/// A line or word diff, bounded by [`DIFF_BUDGET`].
fn diff(old: &[&str], new: &[&str]) -> Vec<similar::DiffOp> {
    capture_diff_slices_deadline(
        Algorithm::Myers,
        old,
        new,
        Some(Instant::now() + DIFF_BUDGET),
    )
}

/// Byte ranges of `new` holding lines that differ from `old` — the regions somebody else's write
/// changed. A pure deletion is an empty range at the point it happened, so a caller can still say
/// *where*.
pub fn changed_spans(old: &str, new: &str) -> Vec<Range<usize>> {
    let (a, b) = (lines(old), lines(new));
    let starts = offsets(&b);
    diff(&a, &b)
        .iter()
        .map(|op| op.as_tag_tuple())
        .filter(|(tag, _, _)| *tag != DiffTag::Equal)
        .map(|(_, _, new)| starts[new.start]..starts[new.end])
        .collect()
}

/// Where a byte offset into `old` lands in `new`: the same column of the same line when that line
/// survived, else the start of whatever replaced it. What keeps a caret put across a write it did
/// not make.
pub fn map_offset(old: &str, new: &str, offset: usize) -> usize {
    let (a, b) = (lines(old), lines(new));
    let (from, to) = (offsets(&a), offsets(&b));
    let line = from.partition_point(|start| *start <= offset).saturating_sub(1);
    for (tag, old_range, new_range) in diff(&a, &b)
        .iter()
        .map(|op| op.as_tag_tuple())
    {
        if !old_range.contains(&line) {
            continue;
        }
        if tag == DiffTag::Equal {
            let target = new_range.start + (line - old_range.start);
            let column = offset - from[line];
            let len = b.get(target).map_or(0, |l| l.len());
            return (to[target] + column.min(len)).min(new.len());
        }
        return to[new_range.start].min(new.len());
    }
    new.len()
}

/// Lines, each with its own terminator, so joining them is the text again.
fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// Words, the whitespace runs between them, and every other character on its own — the second
/// pass's granularity.
fn words(text: &str) -> Vec<&str> {
    #[derive(PartialEq)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut last: Option<Class> = None;
    for (at, c) in text.char_indices() {
        let now = class(c);
        if let Some(prev) = &last
            && (*prev != now || now == Class::Other)
        {
            out.push(&text[start..at]);
            start = at;
        }
        last = Some(now);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// The byte offset each token starts at, plus the end as a sentinel.
fn offsets(tokens: &[&str]) -> Vec<usize> {
    let mut at = vec![0];
    for token in tokens {
        at.push(at.last().copied().unwrap_or(0) + token.len());
    }
    at
}

/// One side's change: a base range and what that side replaced it with.
struct Hunk {
    base: Range<usize>,
    side: Range<usize>,
}

/// Consecutive non-equal ops as one change each.
fn hunks(base: &[&str], side: &[&str]) -> Vec<Hunk> {
    let mut out: Vec<Hunk> = Vec::new();
    let mut open: Option<Hunk> = None;
    for (tag, b, s) in diff(base, side)
        .iter()
        .map(|op| op.as_tag_tuple())
    {
        if tag == DiffTag::Equal {
            out.extend(open.take());
            continue;
        }
        match &mut open {
            Some(hunk) => {
                hunk.base.end = b.end;
                hunk.side.end = s.end;
            }
            None => open = Some(Hunk { base: b, side: s }),
        }
    }
    out.extend(open);
    out
}

fn delta(hunk: &Hunk) -> isize {
    hunk.side.len() as isize - hunk.base.len() as isize
}

fn shift(at: usize, by: isize) -> usize {
    (at as isize + by).max(0) as usize
}

/// The diff3 walk over one granularity of tokens. `refine` says a contested region may be tried
/// again at word level; at word level a contested region is a conflict for the caller to report.
/// Answers whether anything was contested.
fn merge_level(
    base: &[&str],
    ours: &[&str],
    theirs: &[&str],
    refine: bool,
    out: &mut String,
    conflicts: &mut Vec<Conflict>,
) -> bool {
    let (a, b) = (hunks(base, ours), hunks(base, theirs));
    let (mut i, mut j) = (0usize, 0usize);
    let (mut da, mut db) = (0isize, 0isize);
    let mut pos = 0usize;
    let mut contested = false;
    loop {
        let start = match (a.get(i), b.get(j)) {
            (None, None) => break,
            (Some(x), None) => x.base.start,
            (None, Some(y)) => y.base.start,
            (Some(x), Some(y)) => x.base.start.min(y.base.start),
        };
        out.extend(base[pos..start].iter().copied());
        // Gather every hunk of either side touching the region, until it stops growing.
        let mut end = start;
        let (mut sum_a, mut sum_b) = (0isize, 0isize);
        let (mut took_a, mut took_b) = (false, false);
        loop {
            if let Some(x) = a.get(i)
                && x.base.start <= end
                && (took_a || took_b || x.base.start == start)
            {
                end = end.max(x.base.end);
                sum_a += delta(x);
                took_a = true;
                i += 1;
                continue;
            }
            if let Some(y) = b.get(j)
                && y.base.start <= end
                && (took_a || took_b || y.base.start == start)
            {
                end = end.max(y.base.end);
                sum_b += delta(y);
                took_b = true;
                j += 1;
                continue;
            }
            break;
        }
        let o = &ours[shift(start, da)..shift(end, da + sum_a)];
        let t = &theirs[shift(start, db)..shift(end, db + sum_b)];
        let bb = &base[start..end];
        da += sum_a;
        db += sum_b;
        pos = end;
        if !took_b || o == t {
            out.extend(o.iter().copied());
        } else if !took_a {
            out.extend(t.iter().copied());
        } else if bb.is_empty() && refine {
            // Both inserted here: the document grows by both, ours first — unless one insertion
            // is the other with more on it, which is the same passage written twice: the longer
            // one stands alone.
            let (o, t) = (o.concat(), t.concat());
            if t.starts_with(&o) || t.ends_with(&o) {
                out.push_str(&t);
            } else if o.starts_with(&t) || o.ends_with(&t) {
                out.push_str(&o);
            } else {
                out.push_str(&o);
                if !o.is_empty() && !o.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&t);
            }
        } else {
            let (bs, os, ts) = (bb.concat(), o.concat(), t.concat());
            let mut fine = String::new();
            let clean = refine
                && bs.len() + os.len() + ts.len() <= WORD_PASS_LIMIT
                && !merge_level(
                    &words(&bs),
                    &words(&os),
                    &words(&ts),
                    false,
                    &mut fine,
                    &mut Vec::new(),
                );
            if clean {
                out.push_str(&fine);
            } else {
                contested = true;
                out.push_str(&os);
                conflicts.push(Conflict {
                    base: bs,
                    ours: os,
                    theirs: ts,
                });
            }
        }
    }
    out.extend(base[pos..].iter().copied());
    contested
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_on_different_lines_both_land() {
        let base = "one\ntwo\nthree\n";
        let m = merge3(base, "ONE\ntwo\nthree\n", "one\ntwo\nTHREE\n");
        assert_eq!(m.text, "ONE\ntwo\nTHREE\n");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn edits_to_different_words_of_one_line_both_land() {
        let base = "The quick brown fox jumps.\n";
        let m = merge3(
            base,
            "The slow brown fox jumps.\n",
            "The quick brown fox leaps.\n",
        );
        assert_eq!(m.text, "The slow brown fox leaps.\n");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn the_same_word_changed_twice_is_ours_and_a_conflict() {
        let base = "a\nThe fox jumps.\nz\n";
        let m = merge3(base, "a\nThe fox runs.\nz\n", "a\nThe fox leaps.\nz\n");
        assert_eq!(m.text, "a\nThe fox runs.\nz\n");
        assert_eq!(
            m.conflicts,
            vec![Conflict {
                base: "The fox jumps.\n".into(),
                ours: "The fox runs.\n".into(),
                theirs: "The fox leaps.\n".into(),
            }]
        );
    }

    #[test]
    fn identical_edits_merge_to_one() {
        let base = "x\ny\n";
        let m = merge3(base, "x\nY\n", "x\nY\n");
        assert_eq!(m.text, "x\nY\n");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn an_untouched_side_takes_the_other() {
        let base = "x\ny\n";
        assert_eq!(merge3(base, base, "x\nz\n").text, "x\nz\n");
        assert_eq!(merge3(base, "w\ny\n", base).text, "w\ny\n");
    }

    #[test]
    fn two_appends_both_grow_the_document() {
        let base = "# T\n";
        let m = merge3(base, "# T\nmine\n", "# T\ntheirs\n");
        assert_eq!(m.text, "# T\nmine\ntheirs\n");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn a_deletion_and_a_distant_edit_both_land() {
        let base = "a\nb\nc\nd\n";
        let m = merge3(base, "a\nc\nd\n", "a\nb\nc\nD\n");
        assert_eq!(m.text, "a\nc\nD\n");
        assert!(m.conflicts.is_empty());
    }

    const BASE: &str = "# T\n\nAlpha one. Alpha two.\n\nBeta.\n\nGamma old.\n";
    const SPLIT: &str = "# T\n\nAlpha one.\n\nAlpha two.\n\nBeta.\n\nGamma old.\n";

    #[test]
    fn a_split_block_and_an_edit_two_blocks_down_both_land() {
        let m = merge3(BASE, SPLIT, &BASE.replace("Gamma old.", "Gamma new."));
        assert_eq!(m.text, SPLIT.replace("Gamma old.", "Gamma new."));
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn a_split_block_and_an_edit_inside_it_merge_by_word() {
        let m = merge3(BASE, SPLIT, &BASE.replace("Alpha two.", "Alpha 2."));
        assert_eq!(m.text, SPLIT.replace("Alpha two.", "Alpha 2."));
        assert!(m.conflicts.is_empty());
        // The same words rewritten by both: the split, and the user's word, stand.
        let ours = SPLIT.replace("Alpha two.", "Alpha zwei.");
        let m = merge3(BASE, &ours, &BASE.replace("Alpha two.", "Alpha deux."));
        assert_eq!(m.text, ours);
        assert_eq!(m.conflicts.len(), 1);
        assert!(m.conflicts[0].theirs.contains("deux"));
    }

    #[test]
    fn an_insertion_that_extends_the_other_is_kept_once() {
        let base = "# T\n";
        let m = merge3(base, "# T\nStep one.\n", "# T\nStep one.\nStep two.\n");
        assert_eq!(m.text, "# T\nStep one.\nStep two.\n");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn two_appends_after_an_unterminated_last_line_both_land() {
        let m = merge3("a", "a\nmine", "a\ntheirs");
        assert_eq!(m.text, "a\nmine\ntheirs");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn crlf_stays_crlf_and_merges_with_lf() {
        let base = "one\r\ntwo\r\nthree\r\n";
        let m = merge3(base, "ONE\r\ntwo\r\nthree\r\n", "one\ntwo\nTHREE\n");
        assert_eq!(m.text, "ONE\r\ntwo\r\nTHREE\r\n");
        assert!(m.conflicts.is_empty());
    }

    #[test]
    fn changed_spans_name_the_new_lines() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let spans = changed_spans(old, new);
        assert_eq!(spans.len(), 2);
        assert_eq!(&new[spans[0].clone()], "B\n");
        assert_eq!(&new[spans[1].clone()], "d\n");
    }

    #[test]
    fn a_caret_keeps_its_line_and_column() {
        let old = "a\nhello\n";
        let new = "intro\na\nhello\n";
        // The `l` in `hello`.
        let at = old.find("llo").unwrap();
        assert_eq!(map_offset(old, new, at), new.find("llo").unwrap());
        assert_eq!(map_offset(old, new, old.len()), new.len());
    }
}
