//! Search-to-position: a text query resolved to a root block index and exact source bytes.
//!
//! The interface never scans markdown, so the resolution lives here. A hit has to be two things at
//! once: a **row** the list can scroll to (the root block index — the only `ix` that addresses a
//! row) and a **byte range** the row can highlight. Both come out of one linear walk; there is no
//! index and no cache, because this runs per query, not per frame.
//!
//! # Rendered text, not raw markdown
//!
//! By default the scan runs over each block's *rendered* text: the string a reader sees, built from
//! the inline span stream with the syntax dropped. `foo *bar*` renders as `foo bar`, so the query
//! `foo bar` finds it, and a query of `*` or `#` finds nothing because those bytes were never
//! content. [`FindOptions::raw_source`] turns that off and scans `source[block.range]` instead, for
//! a user hunting syntax.
//!
//! # The mapping back, and where it is inexact
//!
//! Building the rendered string also builds a piece table: each contribution records the rendered
//! range it occupies and the source range it came from. A match's rendered range is mapped back
//! through it by binary search.
//!
//! A piece is **exact** when its rendered bytes correspond one-to-one, in order, with its source
//! bytes — then a rendered offset maps to `src.start + (offset - text.start)` and
//! `&source[m.range]` is the matched text itself. A piece is inexact when it is not:
//!
//! - [`Inline::Text`] arrives already unescaped, so `&amp;` (5 source bytes) or `\_` (2) is one
//!   rendered byte and every offset after it in that span is shifted. The test is
//!   `text.len() == range.len()`, the same one [`crate::autolink`] uses.
//! - [`Inline::Code`] has its backticks removed, so its payload is found inside the source span
//!   when it sits there verbatim (the common case, which stays exact) and is inexact when
//!   `pulldown-cmark` normalised it.
//! - A soft or hard break renders as one space and stands for whatever the source wrote — a bare
//!   `\n` is exact, `\r\n` or two trailing spaces before the newline is not.
//!
//! **Where a rendered offset falls inside an inexact piece the reported byte is the piece's
//! boundary, not a computed offset**: the match's start snaps back to the piece's `src.start` and
//! its end snaps forward to the piece's `src.end`. So an inexact piece is always included whole,
//! the range never lands mid-character or inside the wrong word, and it is never inverted — but it
//! can be wider than the matched text. Widening is the right failure: a range that is too wide
//! still scrolls to the right place, while a computed-but-shifted offset does not.
//!
//! A match that crosses inline syntax is wider than the query for the same reason and by design:
//! `foo bar` against `foo *bar*` yields `0..8`, i.e. `foo *bar` — the tightest source span that
//! covers both ends of the match.
//!
//! # Blocks with no inline stream
//!
//! Code, HTML, front matter and [`BlockKind::Other`] carry no spans, so there is no rendered string
//! to walk; they are scanned over their own source bytes even in rendered mode, and the hit says so
//! with [`Match::in_rendered_text`] set to `false`.
//!
//! # Matching
//!
//! Literal only — no regex, no fuzzy, no dependency. Matching is a hand-rolled forward scan with a
//! first-character prefilter, case folded per `char` so offsets stay valid (lowercasing the whole
//! haystack would move them).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::{Block, BlockKind, Document, Inline, slugify};

/// How to search. `Default` is case-insensitive, not whole-word, over the rendered text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindOptions {
    pub case_sensitive: bool,
    /// Both edges of the match must sit against a non-word character — alphanumeric and `_` are
    /// word characters, as everywhere else.
    pub whole_word: bool,
    /// Scan `source[block.range]` instead of the rendered text, syntax included.
    pub raw_source: bool,
}

impl FindOptions {
    /// The raw-source scan, for a query that is looking for markdown syntax.
    pub fn raw() -> Self {
        Self {
            raw_source: true,
            ..Self::default()
        }
    }
}

/// One hit: the row to reveal and the bytes to highlight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Match {
    /// Index into [`Document::blocks`] — the **root** block, walked up from a nested hit, because
    /// a nested block's `ix` is sibling-local and addresses no row.
    pub root_block: usize,
    /// The match's source byte range. See the module docs for where it is wider than the query.
    pub range: Range<usize>,
    /// `false` when the hit came from a raw-source scan — either [`FindOptions::raw_source`] or a
    /// block with no inline stream.
    pub in_rendered_text: bool,
}

impl Document {
    /// Every match of `query`, in source order.
    ///
    /// Literal, case-insensitive by default, over the rendered text of every block including the
    /// contents of quotes, footnote definitions, list items and table cells — each hit attributed
    /// to its enclosing root block. An empty query matches nothing.
    pub fn find(&self, query: &str, opts: FindOptions) -> Vec<Match> {
        let mut out = Vec::new();
        if query.is_empty() {
            return out;
        }
        for (root, block) in self.blocks.iter().enumerate() {
            if opts.raw_source {
                // The root range covers every nested block, so raw mode needs no recursion.
                self.push_raw(block.range.clone(), root, query, opts, &mut out);
            } else {
                self.find_in_block(block, root, query, opts, &mut out);
            }
        }
        out.sort_by_key(|m| (m.range.start, m.range.end));
        out
    }

    /// The root block index of the heading `query` names, for a navigator or an anchor jump.
    ///
    /// Tried in order: the slug as written (`#intro` and `intro` both work), the slug `query`
    /// slugifies to, the heading text in full, then the first heading whose text contains it. All
    /// of it case-insensitive, and only root-level headings — the same set [`Document::headings`]
    /// returns, because a heading inside a quote or a list item is not a section.
    pub fn find_heading(&self, query: &str) -> Option<usize> {
        let query = query.trim().trim_start_matches('#').trim();
        if query.is_empty() {
            return None;
        }
        let ci = FindOptions::default();
        let headings: Vec<(usize, &str, &str)> = self
            .blocks
            .iter()
            .filter_map(|b| match &b.kind {
                BlockKind::Heading { text, slug, .. } => Some((b.ix, text.as_str(), slug.as_str())),
                _ => None,
            })
            .collect();

        let slugified = slugify(query);
        headings
            .iter()
            .find(|(_, _, slug)| equals(slug, query, ci) || *slug == slugified)
            .or_else(|| headings.iter().find(|(_, text, _)| equals(text, query, ci)))
            .or_else(|| {
                headings
                    .iter()
                    .find(|(_, text, _)| !matches_in(text, query, ci).is_empty())
            })
            .map(|(ix, _, _)| *ix)
    }

    /// One block and everything nested inside it, all attributed to `root`.
    fn find_in_block(
        &self,
        block: &Block,
        root: usize,
        query: &str,
        opts: FindOptions,
        out: &mut Vec<Match>,
    ) {
        match &block.kind {
            BlockKind::Quote { blocks, .. } | BlockKind::FootnoteDefinition { blocks, .. } => {
                for nested in blocks {
                    self.find_in_block(nested, root, query, opts, out);
                }
            }
            BlockKind::List { items, .. } => {
                for nested in items.iter().flat_map(|i| &i.blocks) {
                    self.find_in_block(nested, root, query, opts, out);
                }
            }
            BlockKind::Table { head, rows, .. } => {
                for cell in head.iter().chain(rows.iter().flatten()) {
                    self.push_rendered(&cell.spans, root, query, opts, out);
                }
            }
            // Code, HTML, front matter, `Other`, a thematic break: no inline stream to render, so
            // the honest scan is the one over the bytes themselves.
            _ if block.spans.is_empty() => {
                self.push_raw(block.range.clone(), root, query, opts, out)
            }
            _ => self.push_rendered(&block.spans, root, query, opts, out),
        }
    }

    fn push_rendered(
        &self,
        spans: &[(Range<usize>, Inline)],
        root: usize,
        query: &str,
        opts: FindOptions,
        out: &mut Vec<Match>,
    ) {
        if spans.is_empty() {
            return;
        }
        let rendered = Rendered::of(spans, &self.source);
        for m in matches_in(&rendered.text, query, opts) {
            if let Some(range) = rendered.source_range(m) {
                out.push(Match {
                    root_block: root,
                    range,
                    in_rendered_text: true,
                });
            }
        }
    }

    fn push_raw(
        &self,
        range: Range<usize>,
        root: usize,
        query: &str,
        opts: FindOptions,
        out: &mut Vec<Match>,
    ) {
        let Some(src) = self.source.get(range.clone()) else {
            return;
        };
        for m in matches_in(src, query, opts) {
            out.push(Match {
                root_block: root,
                range: range.start + m.start..range.start + m.end,
                in_rendered_text: false,
            });
        }
    }
}

/// A block's rendered text plus the piece table that maps it back to source bytes.
struct Rendered {
    text: String,
    /// Contiguous in `text` by construction: `pieces` tiles `0..text.len()`, which is what makes
    /// the lookup a `partition_point`.
    pieces: Vec<Piece>,
}

struct Piece {
    text: Range<usize>,
    src: Range<usize>,
    /// Rendered bytes correspond one-to-one with source bytes. See the module docs.
    exact: bool,
}

impl Rendered {
    fn of(spans: &[(Range<usize>, Inline)], source: &str) -> Self {
        let mut out = Rendered {
            text: String::with_capacity(spans.len() * 8),
            pieces: Vec::with_capacity(spans.len()),
        };
        for (range, inline) in spans {
            match inline {
                Inline::Text(s) => out.push(s, range.clone(), s.len() == range.len()),
                Inline::Code(s) => match source
                    .get(range.clone())
                    .filter(|_| !s.is_empty())
                    .and_then(|src| src.find(s.as_str()))
                {
                    Some(at) => out.push(s, range.start + at..range.start + at + s.len(), true),
                    None => out.push(s, range.clone(), false),
                },
                // A break is one space in the rendered text; exact only when the source spent one
                // byte on it.
                Inline::SoftBreak | Inline::HardBreak => {
                    out.push(" ", range.clone(), range.len() == 1)
                }
                // `Enter`/`Exit` are the syntax the rendered text drops — that is the whole point.
                // Raw HTML, a footnote reference and a task marker render as markup or a glyph, not
                // as searchable prose.
                Inline::Enter(_)
                | Inline::Exit(_)
                | Inline::Html(_)
                | Inline::FootnoteReference(_)
                | Inline::TaskMarker(_) => {}
            }
        }
        out
    }

    fn push(&mut self, s: &str, src: Range<usize>, exact: bool) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        self.pieces.push(Piece {
            text: start..self.text.len(),
            src,
            exact,
        });
    }

    /// The source range of a rendered range, or `None` if the piece table cannot cover it.
    fn source_range(&self, m: Range<usize>) -> Option<Range<usize>> {
        let start = self.piece_at(m.start)?.map_start(m.start);
        let end = self.piece_at(m.end.checked_sub(1)?)?.map_end(m.end);
        Some(start..end.max(start))
    }

    fn piece_at(&self, offset: usize) -> Option<&Piece> {
        let ix = self
            .pieces
            .partition_point(|p| p.text.start <= offset)
            .checked_sub(1)?;
        self.pieces.get(ix).filter(|p| offset < p.text.end)
    }
}

impl Piece {
    fn map_start(&self, offset: usize) -> usize {
        if self.exact {
            self.src.start + (offset - self.text.start)
        } else {
            self.src.start
        }
    }

    fn map_end(&self, offset: usize) -> usize {
        if self.exact {
            self.src.start + (offset - self.text.start)
        } else {
            self.src.end
        }
    }
}

/// Every non-overlapping literal match of `needle` in `haystack`, in order.
///
/// A forward scan: a first-character prefilter, then a char-by-char compare from that offset. `O(n
/// ·m)` in the worst case and linear on real text, which is the right trade for a per-query scan
/// with no index to maintain.
fn matches_in(haystack: &str, needle: &str, opts: FindOptions) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let Some(first) = needle.chars().next() else {
        return out;
    };
    let mut at = 0usize;
    while let Some(c) = haystack[at..].chars().next() {
        if !chars_eq(c, first, opts.case_sensitive) {
            at += c.len_utf8();
            continue;
        }
        match match_at(haystack, at, needle, opts.case_sensitive) {
            Some(end) if !opts.whole_word || is_word_bounded(haystack, at, end) => {
                out.push(at..end);
                at = end;
            }
            _ => at += c.len_utf8(),
        }
    }
    out
}

/// The end offset of `needle` matched at `at`, or `None`.
fn match_at(haystack: &str, at: usize, needle: &str, case_sensitive: bool) -> Option<usize> {
    let mut rest = haystack[at..].chars();
    for want in needle.chars() {
        if !chars_eq(rest.next()?, want, case_sensitive) {
            return None;
        }
    }
    Some(haystack.len() - rest.as_str().len())
}

/// Case folding per `char`, so a fold can never shift a byte offset.
fn chars_eq(a: char, b: char, case_sensitive: bool) -> bool {
    if case_sensitive || a == b {
        return a == b;
    }
    if a.is_ascii() || b.is_ascii() {
        return a.eq_ignore_ascii_case(&b);
    }
    a.to_lowercase().eq(b.to_lowercase())
}

fn is_word_bounded(haystack: &str, start: usize, end: usize) -> bool {
    let before = haystack[..start].chars().next_back();
    let after = haystack[end..].chars().next();
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The whole string equals the query, under `opts`.
fn equals(text: &str, query: &str, opts: FindOptions) -> bool {
    match_at(text, 0, query, opts.case_sensitive) == Some(text.len())
}
