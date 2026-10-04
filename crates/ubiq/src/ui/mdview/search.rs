//! Find in document: the navigation state and the per-row hit plumbing. The find bar itself — the
//! field, the facets, the bindings — is not ported yet.
//!
//! The engine owns the scan — [`ubiq_md::Document::find`] returns every hit as a root block index
//! plus a source byte range, non-overlapping and in source order — and owns **no navigation
//! state**. [`Find`] is that state: which hit is current, which options are on, and the query the
//! list was computed for. One per view (the `MdView` entity, T-301), not a thread-local as in the
//! spike: Ubiq has one view per markdown tab, and each keeps its own query.
//!
//! **The matches are recomputed from the document the render path is handed**, keyed on the block
//! count plus the first block's hash. Nothing hooks the reparse: a document that reparsed behind
//! the debounce has a different key by the next frame, and the match list is rebuilt before any row
//! draws with it.
//!
//! **A row's highlight travels out of band.** `blocks::row` opens a [`scope`] for the root block it
//! is about to build and `inline::runs` asks [`row_hits`] for it, so no signature between the two
//! had to grow a search parameter — the alternative was threading `Option<&Hits>` through every
//! block kind, the nested stack, and the table cells. The hand-off is a thread-local that lives
//! exactly as long as one row is being built, which is safe because rows build on the one thread
//! that draws and never nest.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use ubiq_md::{Document, FindOptions, Match};

// ── The state ───────────────────────────────────────────────────────

/// What the cached match list was computed from. Any change here invalidates it, and the document
/// half of it — the block count and the first block's hash — is what makes a reparse behind the
/// debounce invalidate it without this module hearing about the reparse at all.
#[derive(PartialEq, Eq)]
struct Key {
    blocks: usize,
    hash: u64,
    query: String,
    opts: FindOptions,
}

/// One view's find state.
#[derive(Default)]
pub struct Find {
    pub open: bool,
    pub options: FindOptions,
    key: Option<Key>,
    matches: Vec<Match>,
    /// Index into [`Self::matches`]. Meaningless when the list is empty, and clamped into it
    /// whenever the list is rebuilt.
    current: usize,
    /// Set while nothing has been navigated to yet — a fresh query, a flipped option, a reopened
    /// bar. The first step then *reveals* the current match instead of stepping past it, which is
    /// what makes the first Enter land on the first hit rather than the second.
    fresh: bool,
}

impl Find {
    /// Drop the list — the bar closed, or there is no document.
    pub fn forget(&mut self) {
        self.key = None;
        self.matches.clear();
        self.restart();
    }

    /// A fresh query or a flipped option: the next step lands on the first hit.
    pub fn restart(&mut self) {
        self.current = 0;
        self.fresh = true;
    }

    /// Bring the match list up to date with the document the frame is about to draw.
    ///
    /// Called on every frame, open or closed, because the closed case is the one that has to
    /// *drop* a stale list: rows would otherwise keep highlighting the hits of a query nobody can
    /// see.
    pub fn refresh(&mut self, doc: &Document, query: &str) {
        if !self.open {
            self.forget();
            return;
        }
        let key = Key {
            blocks: doc.blocks.len(),
            hash: doc.blocks.first().map_or(0, |block| block.hash),
            query: query.to_string(),
            opts: self.options,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        self.matches = search(doc, &key.query, key.opts);
        self.current = clamp(self.current, self.matches.len());
        self.key = Some(key);
    }

    /// Move to the next or previous match, wrapping at both ends. Answers the root block the
    /// current match is in — the `list()` row to reveal — or `None` with no matches.
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        let count = self.matches.len();
        if count == 0 {
            return None;
        }
        self.current = match (std::mem::take(&mut self.fresh), forward) {
            // Nothing navigated to yet: the first step lands on the first hit, and a backwards
            // first step on the last — the same reading of "wrap" from a standing start.
            (true, true) => 0,
            (true, false) => count - 1,
            (false, true) => next_match(self.current, count, true),
            (false, false) => prev_match(self.current, count, true),
        };
        Some(self.matches[self.current].root_block)
    }

    pub fn count(&self) -> usize {
        self.matches.len()
    }

    /// The hits inside root block `ix`, or `None` when the bar is shut or the row has none.
    pub fn hits(&self, ix: usize) -> Option<Hits> {
        if !self.open || self.matches.is_empty() {
            return None;
        }
        let mut ranges = Vec::new();
        let mut current = None;
        for (n, m) in self.matches.iter().enumerate() {
            if m.root_block != ix {
                continue;
            }
            if n == self.current {
                current = Some(ranges.len());
            }
            ranges.push(m.range.clone());
        }
        (!ranges.is_empty()).then_some(Hits { ranges, current })
    }

    /// "3 of 17", or what there is to say instead — what the find bar will show.
    pub fn position(&self) -> String {
        let empty = self
            .key
            .as_ref()
            .is_none_or(|key| key.query.trim().is_empty());
        match (empty, self.matches.len()) {
            (true, _) => String::new(),
            (false, 0) => "no matches".to_string(),
            (false, count) => format!("{} of {count}", self.current + 1),
        }
    }
}

/// The hits inside one row, in the terms the row draws in: source byte ranges, and which of them is
/// the current one.
pub struct Hits {
    pub ranges: Vec<Range<usize>>,
    /// Index into [`Self::ranges`], or `None` when the current match is in another row.
    pub current: Option<usize>,
}

thread_local! {
    /// The hits of the row being built, for as long as it is being built. See [`scope`].
    static ROW: RefCell<Option<Rc<Hits>>> = const { RefCell::new(None) };
}

// ── Navigation arithmetic ───────────────────────────────────────────

/// The next match after `current`, wrapping at the end when `wrap` is set and stopping at it
/// otherwise. `0` for an empty list, which is the only answer an empty list has.
pub fn next_match(current: usize, count: usize, wrap: bool) -> usize {
    if count == 0 {
        return 0;
    }
    let current = current.min(count - 1);
    if current + 1 < count {
        current + 1
    } else if wrap {
        0
    } else {
        current
    }
}

/// The match before `current`, wrapping at the start when `wrap` is set.
pub fn prev_match(current: usize, count: usize, wrap: bool) -> usize {
    if count == 0 {
        return 0;
    }
    let current = current.min(count - 1);
    match current.checked_sub(1) {
        Some(previous) => previous,
        None if wrap => count - 1,
        None => 0,
    }
}

/// The engine's scan. Here only so the query's emptiness is checked in one place.
fn search(doc: &Document, query: &str, opts: FindOptions) -> Vec<Match> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    doc.find(query, opts)
}

fn clamp(current: usize, count: usize) -> usize {
    match count {
        0 => 0,
        count => current.min(count - 1),
    }
}

// ── The row's highlight ─────────────────────────────────────────────

/// Publish the hits of root block `ix` for as long as the returned guard lives.
///
/// `blocks::row` holds one of these across the row it builds; `inline::runs` reads it through
/// [`row_hits`]. The guard is what keeps a row's hits from leaking into the next row, including on
/// the early-return paths a block kind can take. `find` is `None` for a view with no find state —
/// a read-only surface — which publishes nothing.
#[must_use = "the hits are published for as long as the guard lives"]
pub fn scope(find: Option<&Find>, ix: usize) -> RowScope {
    let hits = find.and_then(|find| find.hits(ix)).map(Rc::new);
    let published = hits.is_some();
    if published {
        ROW.with_borrow_mut(|row| *row = hits);
    }
    RowScope(published)
}

/// See [`scope`].
pub struct RowScope(bool);

impl Drop for RowScope {
    fn drop(&mut self) {
        if self.0 {
            ROW.with_borrow_mut(|row| *row = None);
        }
    }
}

/// The hits of the row currently being built, if it has any.
pub fn row_hits() -> Option<Rc<Hits>> {
    ROW.with_borrow(Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forwards, wrapping: the last match steps to the first.
    #[test]
    fn next_wraps_at_the_end() {
        assert_eq!(next_match(0, 3, true), 1);
        assert_eq!(next_match(1, 3, true), 2);
        assert_eq!(next_match(2, 3, true), 0);
    }

    /// Backwards, wrapping: the first match steps to the last.
    #[test]
    fn prev_wraps_at_the_start() {
        assert_eq!(prev_match(2, 3, true), 1);
        assert_eq!(prev_match(1, 3, true), 0);
        assert_eq!(prev_match(0, 3, true), 2);
    }

    /// Without the wrap both ends hold, which is what a "stop at the end" mode would want.
    #[test]
    fn without_the_wrap_the_ends_hold() {
        assert_eq!(next_match(2, 3, false), 2);
        assert_eq!(prev_match(0, 3, false), 0);
    }

    /// One match: every step is a no-op, wrapping or not, and never an out-of-range index.
    #[test]
    fn a_single_match_steps_to_itself() {
        for wrap in [true, false] {
            assert_eq!(next_match(0, 1, wrap), 0);
            assert_eq!(prev_match(0, 1, wrap), 0);
        }
    }

    /// No matches: `0`, and no arithmetic that could underflow or index anything.
    #[test]
    fn no_matches_step_nowhere() {
        for wrap in [true, false] {
            assert_eq!(next_match(0, 0, wrap), 0);
            assert_eq!(prev_match(0, 0, wrap), 0);
            assert_eq!(next_match(7, 0, wrap), 0);
            assert_eq!(prev_match(7, 0, wrap), 0);
        }
    }

    /// A `current` left over from a longer list — which is what a reparse hands us — lands back
    /// inside the new one rather than panicking or answering out of range.
    #[test]
    fn a_stale_current_is_clamped() {
        assert_eq!(clamp(9, 3), 2);
        assert_eq!(clamp(9, 0), 0);
        assert_eq!(next_match(9, 3, true), 0);
        assert_eq!(prev_match(9, 3, true), 1);
    }

    /// The whole state path: refresh against a document, step through it, publish one row's hits
    /// through a scope and see them gone once the scope drops.
    #[test]
    fn a_query_steps_through_its_rows_and_scopes_one_row_at_a_time() {
        let doc = ubiq_md::parse("alpha beta\n\nbeta gamma\n\nnothing\n");
        let mut find = Find {
            open: true,
            ..Find::default()
        };
        find.refresh(&doc, "beta");
        find.restart();
        assert_eq!(find.count(), 2);
        assert_eq!(
            find.step(true),
            Some(0),
            "the first step lands on the first hit"
        );
        assert_eq!(find.position(), "1 of 2");
        assert_eq!(find.step(true), Some(1));
        assert_eq!(find.step(true), Some(0), "and wraps");

        {
            let _row = scope(Some(&find), 0);
            let hits = row_hits().expect("row 0 has a hit");
            assert_eq!(hits.ranges.len(), 1);
            assert_eq!(hits.current, Some(0));
        }
        assert!(row_hits().is_none(), "the scope took its hits with it");
        {
            let _row = scope(Some(&find), 2);
            assert!(row_hits().is_none(), "row 2 has no hit");
        }

        find.open = false;
        find.refresh(&doc, "beta");
        assert_eq!(find.count(), 0, "a shut bar drops its list");
        assert!(find.hits(0).is_none());
    }
}
