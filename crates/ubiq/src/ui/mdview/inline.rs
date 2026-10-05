//! Inline spans to GPUI text runs.
//!
//! **This is the half of the spike that proves the block table is enough.** The engine hands every
//! block a flat `Vec<(Range<usize>, Inline)>` where nesting is balanced [`Inline::Enter`] /
//! [`Inline::Exit`] pairs; this module walks it once with a mark stack and emits a [`StyledText`]
//! with one [`TextRun`] per style change. No markdown is parsed here and no `TextView` is
//! constructed — which is exactly the limitation the proposal (§3, §10) exists to remove.
//!
//! What a run can carry, and what it cannot: family, weight, slant, colour, background,
//! underline and strikethrough are per-run. **Size is not** — a `StyledText` has one font size,
//! taken from the ambient text style.
//!
//! # Two consumers, one walk
//!
//! That size limit is what `ui/mdview/prose.rs` — the native element, item 14 — exists to lift, and this
//! module feeds both paths from the same pass over the spans:
//!
//! - [`runs`] and [`styled`] build the `TextRun`s a [`StyledText`] takes. Inline code gets the
//!   monospace family, the code colour and a flat background standing in for its chip, but not
//!   §15.1's 0.9× size, because a `TextRun` has nowhere to put it.
//! - [`prose_runs`] builds [`prose::Run`]s, which are the same `TextRun`s **plus** the run's own
//!   size and a flag for the chip — and hands the search hits back as ranges rather than as
//!   backgrounds, so the element can pad and round them.
//!
//! The walk itself is [`walk`] and there is only one of it: a difference between what the two
//! paths draw would be a difference between a measured row height and a painted one.
//!
//! # The search highlight
//!
//! A run's background is how a search hit is drawn on the old path (item 10). The engine reports a
//! hit as a **source** byte range and this walk is the only place that knows which source bytes
//! ended up where in the string it built — so the mapping lives here: [`Builder`] records a piece
//! table as it pushes, [`text_ranges`] maps the hits through it, and then either
//! [`highlight_runs`] splits the runs at those boundaries and paints them, or [`prose_runs`]
//! returns the same boundaries for the element to lay quads under.

use std::ops::Range;

use gpui::{
    Font, FontFeatures, FontStyle, FontWeight, Hsla, SharedString, StrikethroughStyle, StyledText,
    TextRun, UnderlineStyle, px,
};
use ubiq_md::{Inline, Mark};

use super::prose;
use super::search::{self, Hits};
use crate::theme;

/// The style a block's text starts from, before any mark applies. A heading passes its own weight
/// and colour; a quote passes the receded prose colour; everything else takes [`Base::body`].
#[derive(Clone, Debug)]
pub struct Base {
    pub family: SharedString,
    pub weight: FontWeight,
    pub color: Hsla,
}

impl Base {
    pub fn body() -> Self {
        Base {
            family: theme::BODY_FONT.into(),
            weight: FontWeight::NORMAL,
            color: theme::text().into(),
        }
    }

    pub fn coloured(color: gpui::Rgba) -> Self {
        Base {
            color: color.into(),
            ..Base::body()
        }
    }

    pub fn heading(weight: FontWeight, color: gpui::Rgba) -> Self {
        Base {
            weight,
            color: color.into(),
            ..Base::body()
        }
    }

    pub fn mono(color: gpui::Rgba) -> Self {
        Base {
            family: theme::MONO_FONT.into(),
            weight: FontWeight::NORMAL,
            color: color.into(),
        }
    }
}

/// One run's style, as a value that compares — so adjacent identical runs merge instead of
/// shipping one `TextRun` per span entry.
#[derive(Clone, PartialEq)]
struct Style {
    family: SharedString,
    weight: FontWeight,
    italic: bool,
    color: Hsla,
    background: Option<Hsla>,
    underline: bool,
    strikethrough: bool,
    /// The run's size, as a multiple of the block's body size. Carried on every style even though
    /// only the native element can honour it — one walk, two consumers, and the walk does not know
    /// which one it is feeding.
    scale: f32,
    /// Inline code, which is the one span the native element draws a chip behind (§15.1).
    chip: bool,
}

impl Style {
    fn run(&self, len: usize) -> TextRun {
        TextRun {
            len,
            font: Font {
                family: self.family.clone(),
                features: FontFeatures::default(),
                fallbacks: None,
                weight: self.weight,
                style: if self.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
            },
            color: self.color,
            background_color: self.background,
            underline: self.underline.then(|| UnderlineStyle {
                thickness: px(1.),
                color: Some(self.color),
                wavy: false,
            }),
            strikethrough: self.strikethrough.then(|| StrikethroughStyle {
                thickness: px(1.),
                color: Some(self.color),
            }),
        }
    }

    /// The same run, plus the two things a `TextRun` cannot hold: the run's own size and whether
    /// the native element draws a chip behind it.
    fn piece(&self, len: usize) -> prose::Run {
        prose::Run {
            run: self.run(len),
            scale: self.scale,
            chip: self.chip,
        }
    }
}

/// One contribution to the built string, and the source bytes it came from.
///
/// The same piece table `ubiq_md::find` builds to map the other way, and with the same notion of
/// exactness: a piece is exact when its rendered bytes correspond one-to-one with its source bytes,
/// which is the test `text.len() == src.len()`. An inexact piece — an unescaped `&amp;`, a
/// normalised code span, a `\r\n` that rendered as one space — is highlighted **whole** rather than
/// at a computed offset, for the reason the engine gives: a range that is too wide still marks the
/// right words, while a shifted offset marks the wrong ones.
struct Piece {
    text: Range<usize>,
    src: Range<usize>,
    exact: bool,
}

/// The accumulator: the text being built, and the runs that describe it.
///
/// The runs are kept in the native element's shape — a `TextRun` plus a size and a chip flag —
/// because it is the wider of the two, and [`Builder::text_runs`] drops the extra for the old path.
struct Builder {
    text: String,
    runs: Vec<prose::Run>,
    last: Option<Style>,
    /// Recorded only when something is going to ask for it — a row with no search hits in it pays
    /// nothing for the mapping.
    pieces: Option<Vec<Piece>>,
    /// Every closed link, as a range of [`Self::text`] and its destination — what the native
    /// element turns into click targets.
    links: Vec<prose::Link>,
    /// The links still open, innermost last: where each one's text started, and where it goes.
    open_links: Vec<(usize, SharedString)>,
}

impl Builder {
    fn new(map: bool) -> Self {
        Builder {
            text: String::new(),
            runs: Vec::new(),
            last: None,
            pieces: map.then(Vec::new),
            links: Vec::new(),
            open_links: Vec::new(),
        }
    }

    fn open_link(&mut self, dest: &str) {
        self.open_links
            .push((self.text.len(), SharedString::from(dest.to_string())));
    }

    fn close_link(&mut self) {
        if let Some((start, dest)) = self.open_links.pop()
            && start < self.text.len()
        {
            self.links.push(prose::Link {
                range: start..self.text.len(),
                dest,
            });
        }
    }

    fn push(&mut self, s: &str, style: &Style, src: &Range<usize>) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        if let Some(pieces) = &mut self.pieces {
            pieces.push(Piece {
                text: start..self.text.len(),
                src: src.clone(),
                exact: s.len() == src.len(),
            });
        }
        if self.last.as_ref() == Some(style)
            && let Some(piece) = self.runs.last_mut()
        {
            piece.run.len += s.len();
            return;
        }
        self.runs.push(style.piece(s.len()));
        self.last = Some(style.clone());
    }

    /// The runs as the old path wants them: plain `TextRun`s, chip background and all.
    fn text_runs(&self) -> Vec<TextRun> {
        self.runs.iter().map(|piece| piece.run.clone()).collect()
    }

    /// The runs as the native element wants them: the size and the chip flag kept, and the flat
    /// chip background dropped — the element lays a rounded quad there instead, and the old fill
    /// would only show through underneath it.
    fn pieces(&self) -> Vec<prose::Run> {
        self.runs
            .iter()
            .map(|piece| {
                let mut piece = piece.clone();
                piece.run.background_color = None;
                piece
            })
            .collect()
    }
}

/// One block's prose in the shape `ui/mdview/prose.rs` takes: the string, its runs (each carrying
/// its own size), the search hits as ranges, and the links as click targets.
pub struct ProseText {
    pub text: SharedString,
    pub runs: Vec<prose::Run>,
    pub highlights: Vec<prose::Highlight>,
    pub links: Vec<prose::Link>,
}

/// The current style, derived from the base plus whatever marks are open.
///
/// Derived rather than pushed/popped because an `Enter`/`Exit` pair carries the whole element's
/// range and the stack is always balanced — so the style at any point is a fold over the stack,
/// and that is cheaper to get right than a parallel stack of styles.
fn style_for(base: &Base, stack: &[Mark]) -> Style {
    let mut style = Style {
        family: base.family.clone(),
        weight: base.weight,
        italic: false,
        color: base.color,
        background: None,
        underline: false,
        strikethrough: false,
        scale: 1.0,
        chip: false,
    };
    for mark in stack {
        match mark {
            Mark::Emphasis => style.italic = true,
            Mark::Strong => style.weight = FontWeight::BOLD,
            Mark::Strikethrough => style.strikethrough = true,
            Mark::Link { .. } => {
                style.color = theme::link_underline().into();
                style.underline = true;
            }
            // An inline image's alt text reads as a caption, not as prose.
            Mark::Image { .. } => style.color = theme::text_faint().into(),
        }
    }
    style
}

/// Walk a block's inline stream and build the text it draws.
pub fn styled(spans: &[(Range<usize>, Inline)], base: &Base) -> StyledText {
    let (text, runs) = runs(spans, base);
    StyledText::new(text).with_runs(runs)
}

/// [`styled`]'s working half: the string and the runs that describe it.
///
/// Split out because the one invariant a text renderer can get catastrophically wrong is silent —
/// the run lengths must sum to the string's byte length exactly, or GPUI shapes the wrong glyphs
/// for the wrong style and, in a debug build, trips an assertion deep in the text system. Tested
/// directly, below.
pub fn runs(spans: &[(Range<usize>, Inline)], base: &Base) -> (SharedString, Vec<TextRun>) {
    // The hits of the row being built, published by `blocks::row` for the length of the row.
    let hits = search::row_hits();
    runs_with(spans, base, hits.as_deref())
}

/// The same string and the same style boundaries, in the shape `ui/mdview/prose.rs` takes: every run
/// carrying its own size, and the search hits as ranges rather than as backgrounds.
///
/// This is where §15.1's 0.9× inline code actually happens — the one thing the `StyledText` path
/// cannot express.
pub fn prose_runs(spans: &[(Range<usize>, Inline)], base: &Base) -> ProseText {
    let hits = search::row_hits();
    prose_with(spans, base, hits.as_deref())
}

/// [`runs`] with the row's hits passed in rather than fetched — the seam the highlight is tested
/// through, since a test has no row to open a [`search::scope`] for.
fn runs_with(
    spans: &[(Range<usize>, Inline)],
    base: &Base,
    hits: Option<&Hits>,
) -> (SharedString, Vec<TextRun>) {
    let b = walk(spans, base, hits.is_some());

    let mut out = b.text_runs();
    if let (Some(hits), Some(pieces)) = (hits, b.pieces.as_ref()) {
        let (ranges, current) = text_ranges(pieces, hits);
        out = highlight_runs(out, &ranges, current);
    }

    (SharedString::from(b.text), out)
}

/// [`prose_runs`] with the row's hits passed in rather than fetched — the same seam, for the same
/// reason.
fn prose_with(spans: &[(Range<usize>, Inline)], base: &Base, hits: Option<&Hits>) -> ProseText {
    let b = walk(spans, base, hits.is_some());

    let highlights = match (hits, b.pieces.as_ref()) {
        (Some(hits), Some(pieces)) => {
            let (ranges, current) = text_ranges(pieces, hits);
            marks(&ranges, current)
                .into_iter()
                .map(|(range, current)| prose::Highlight { range, current })
                .collect()
        }
        _ => Vec::new(),
    };

    ProseText {
        runs: b.pieces(),
        text: SharedString::from(b.text),
        highlights,
        links: b.links,
    }
}

/// The one walk: a block's inline stream, folded into a string and the runs that describe it.
///
/// `map` records the piece table the search highlight is mapped through; a row with no hits in it
/// pays nothing for it.
fn walk(spans: &[(Range<usize>, Inline)], base: &Base, map: bool) -> Builder {
    let mut b = Builder::new(map);
    let mut stack: Vec<Mark> = Vec::new();

    for (range, inline) in spans {
        let style = style_for(base, &stack);
        match inline {
            Inline::Text(s) => b.push(s, &style, range),
            Inline::Code(s) => {
                let mut code = style.clone();
                code.family = theme::MONO_FONT.into();
                code.color = theme::markdown().code_text.into();
                code.background = Some(theme::markdown().code_chip.into());
                code.weight = FontWeight::NORMAL;
                // The size a `TextRun` has no room for. Ignored by the `StyledText` path and
                // honoured by the native element, which is the whole of item 14.
                code.scale = prose::CODE_SCALE;
                code.chip = true;
                b.push(s, &code, range);
            }
            Inline::Html(s) => {
                let mut raw = style.clone();
                raw.family = theme::MONO_FONT.into();
                raw.color = theme::text_faint().into();
                b.push(s, &raw, range);
            }
            Inline::FootnoteReference(label) => {
                let mut note = style.clone();
                note.color = theme::link_underline().into();
                b.push(&format!("[^{label}]"), &note, range);
            }
            // Never prose. A tight list item carries its `[x]` inside the paragraph, but the
            // engine has already lifted it onto `ListItem::checked` either way, and the list
            // renderer draws it as a box in the marker column. Emitting it here too would print
            // the checkbox twice.
            Inline::TaskMarker(_) => {}
            // A soft break reflows: it is a space, not a line.
            Inline::SoftBreak => b.push(" ", &style, range),
            Inline::HardBreak => b.push("\n", &style, range),
            Inline::Enter(mark) => {
                if let Mark::Link { dest, .. } = mark {
                    b.open_link(dest);
                }
                stack.push(mark.clone());
            }
            Inline::Exit(_) => {
                if let Some(Mark::Link { .. }) = stack.pop() {
                    b.close_link();
                }
            }
        }
    }

    // An empty row would measure at zero height and swallow the block's spacing with it.
    if b.text.is_empty() {
        b.push(" ", &style_for(base, &[]), &(0..1));
    }

    b
}

/// A row's hits, mapped out of source bytes and into offsets in the string this walk built.
///
/// A hit that covers no piece at all — every block carries hits from its nested blocks, and a table
/// cell's walk sees only its own spans — is dropped, which is why the current hit's index has to be
/// tracked through the mapping rather than carried across it.
fn text_ranges(pieces: &[Piece], hits: &Hits) -> (Vec<Range<usize>>, Option<usize>) {
    let mut ranges = Vec::with_capacity(hits.ranges.len());
    let mut current = None;

    for (n, hit) in hits.ranges.iter().enumerate() {
        let mut start = usize::MAX;
        let mut end = 0usize;
        for piece in pieces {
            if piece.src.end <= hit.start || hit.end <= piece.src.start {
                continue;
            }
            // An exact piece maps offset for offset; an inexact one is taken whole.
            let (from, to) = if piece.exact {
                (
                    piece.text.start + hit.start.max(piece.src.start) - piece.src.start,
                    piece.text.start + hit.end.min(piece.src.end) - piece.src.start,
                )
            } else {
                (piece.text.start, piece.text.end)
            };
            start = start.min(from);
            end = end.max(to);
        }
        // Contiguous by construction even where the hit crossed dropped syntax: the `*` of
        // `foo *bar*` contributed no bytes to the string, so there is no gap to leave out.
        if start < end {
            if hits.current == Some(n) {
                current = Some(ranges.len());
            }
            ranges.push(start..end);
        }
    }

    (ranges, current)
}

/// Split `runs` at every highlight boundary and give the covered pieces a background — the current
/// hit its own, the rest theirs.
///
/// The one invariant, the same one [`runs`] has: the lengths still sum to the string's byte length.
/// Every path here either copies a run whole or splits it into pieces that sum to its length, and
/// nothing is dropped.
///
/// `ranges` are offsets into the string the runs describe, and `current` indexes `ranges`. The
/// engine guarantees non-overlapping hits and this does not rely on it: overlaps are clipped to the
/// first range that claimed the byte, so a mistake upstream draws a slightly short highlight rather
/// than corrupting the run lengths.
pub fn highlight_runs(
    runs: Vec<TextRun>,
    ranges: &[Range<usize>],
    current: Option<usize>,
) -> Vec<TextRun> {
    let marks = marks(ranges, current);
    if marks.is_empty() {
        return runs;
    }

    let mut out: Vec<TextRun> = Vec::with_capacity(runs.len() + marks.len() * 2);
    let mut at = 0usize;
    for run in runs {
        let end = at + run.len;
        let mut cursor = at;
        while cursor < end {
            // The mark covering `cursor`, if any; otherwise the next one, which is where this
            // unmarked piece has to stop.
            let covering = marks.iter().find(|(range, _)| range.contains(&cursor));
            let (until, background) = match covering {
                Some((range, current)) => (
                    range.end.min(end),
                    Some(if *current {
                        theme::markdown().search_match_current
                    } else {
                        theme::markdown().search_match
                    }),
                ),
                None => (
                    marks
                        .iter()
                        .map(|(range, _)| range.start)
                        .find(|start| *start > cursor)
                        .unwrap_or(end)
                        .min(end),
                    None,
                ),
            };
            let mut piece = run.clone();
            piece.len = until - cursor;
            if let Some(background) = background {
                piece.background_color = Some(background.into());
            }
            out.push(piece);
            cursor = until;
        }
        at = end;
    }
    out
}

/// The ranges to paint, sorted, clipped against each other, and each flagged with whether it is the
/// current hit. Empty ranges are dropped: a zero-length highlight is nothing to draw.
fn marks(ranges: &[Range<usize>], current: Option<usize>) -> Vec<(Range<usize>, bool)> {
    let mut marks: Vec<(Range<usize>, bool)> = ranges
        .iter()
        .enumerate()
        .filter(|(_, range)| range.start < range.end)
        .map(|(n, range)| (range.clone(), current == Some(n)))
        .collect();
    marks.sort_by_key(|(range, _)| (range.start, range.end));

    let mut end = 0usize;
    marks.retain_mut(|(range, _)| {
        range.start = range.start.max(end);
        end = end.max(range.end);
        range.start < range.end
    });
    marks
}

/// A run of literal text in one style — the fallback every block kind without inline spans uses
/// (a code fence's body, raw HTML, front matter, `BlockKind::Other`'s raw source).
pub fn literal(text: impl Into<SharedString>, base: &Base) -> StyledText {
    let text: SharedString = text.into();
    let style = style_for(base, &[]);
    let runs = if text.is_empty() {
        Vec::new()
    } else {
        vec![style.run(text.len())]
    };
    StyledText::new(text).with_runs(runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_md::{BlockKind, parse};

    /// Every block of a document, and every table cell in it, walked in one pass.
    fn walk_blocks(blocks: &[ubiq_md::Block], visit: &mut impl FnMut(&[(Range<usize>, Inline)])) {
        for block in blocks {
            visit(&block.spans);
            match &block.kind {
                BlockKind::Quote { blocks, .. } | BlockKind::FootnoteDefinition { blocks, .. } => {
                    walk_blocks(blocks, visit)
                }
                BlockKind::List { items, .. } => {
                    for item in items {
                        walk_blocks(&item.blocks, visit);
                    }
                }
                BlockKind::Table { head, rows, .. } => {
                    for cell in head.iter().chain(rows.iter().flatten()) {
                        visit(&cell.spans);
                    }
                }
                _ => {}
            }
        }
    }

    /// The invariant: run lengths sum to the string's byte length, for every block in a document
    /// that uses every inline the engine emits.
    #[test]
    fn runs_cover_the_text_exactly() {
        const SOURCE: &str = "\
---
title: sample
---

# A *heading* with `code`

A paragraph with **strong**, _emphasis_, ~~struck~~, `inline code`, a [link](https://x.test),
a soft break, a hard break\\
and an ![image](pic.png) plus a footnote[^n].

> [!NOTE]
> A quote with **marks**.

- [x] a done task with `code`
- [ ] an open one
  1. nested
  2. ordered

| Left | Centre | Right |
|:--|:-:|--:|
| `a` | **b** | _c_ |

```rust
fn main() {}
```

<div>raw html</div>

***

[^n]: The note itself, with *emphasis*.
";

        let doc = parse(SOURCE);
        assert!(doc.blocks.len() > 8, "sample should exercise many kinds");

        let mut seen = 0usize;
        walk_blocks(&doc.blocks, &mut |spans| {
            let (text, runs) = runs(spans, &Base::body());
            let total: usize = runs.iter().map(|run| run.len).sum();
            assert_eq!(total, text.len(), "runs do not cover {text:?}");
            seen += 1;
        });
        assert!(seen > 10, "walked only {seen} span lists");
    }

    /// A task item's `[x]` is drawn in the marker column, never twice.
    #[test]
    fn the_task_marker_never_reaches_the_prose() {
        let doc = parse("- [x] done\n- [ ] not\n");
        let mut all = String::new();
        walk_blocks(&doc.blocks, &mut |spans| {
            all.push_str(&runs(spans, &Base::body()).0);
        });
        assert!(!all.contains('['), "task marker leaked into prose: {all:?}");
    }

    /// Adjacent spans in one style become one run rather than one run per span entry.
    #[test]
    fn identical_neighbours_merge() {
        let doc = parse("plain text with a soft\nbreak in it\n");
        let (_, runs) = runs(&doc.blocks[0].spans, &Base::body());
        assert_eq!(runs.len(), 1);
    }

    // ── The search highlight ────────────────────────────────────────

    /// `n` runs of the given lengths, in the body style and with no background.
    fn plain(lengths: &[usize]) -> Vec<TextRun> {
        let style = style_for(&Base::body(), &[]);
        lengths.iter().map(|len| style.run(*len)).collect()
    }

    /// Every run's length and background, which is the whole of what a highlight changes.
    fn shape(runs: &[TextRun]) -> Vec<(usize, Option<Hsla>)> {
        runs.iter()
            .map(|run| (run.len, run.background_color))
            .collect()
    }

    fn total(runs: &[TextRun]) -> usize {
        runs.iter().map(|run| run.len).sum()
    }

    /// Hit ranges, built rather than written out: `&[3..6]` trips clippy's
    /// `single_range_in_vec_init`, which reads a one-range array as a range someone meant to
    /// collect, and a deliberately inverted one is a hard error there.
    fn at(bounds: &[(usize, usize)]) -> Vec<Range<usize>> {
        bounds.iter().map(|(start, end)| *start..*end).collect()
    }

    fn other() -> Option<Hsla> {
        Some(theme::markdown().search_match.into())
    }

    fn current() -> Option<Hsla> {
        Some(theme::markdown().search_match_current.into())
    }

    /// No hits, no change — and the same allocation, not a rebuilt one.
    #[test]
    fn no_matches_leave_the_runs_alone() {
        let before = plain(&[4, 6]);
        let after = highlight_runs(before.clone(), &[], None);
        assert_eq!(shape(&after), shape(&before));
    }

    /// One hit inside one run splits it in three, and the middle piece is the one painted.
    #[test]
    fn a_match_splits_the_run_it_sits_in() {
        let out = highlight_runs(plain(&[10]), &at(&[(3, 6)]), Some(0));
        assert_eq!(
            shape(&out),
            vec![(3, None), (3, current()), (4, None)],
            "3..6 of a 10-byte run"
        );
        assert_eq!(total(&out), 10);
    }

    /// A hit that spans two runs paints both halves and keeps each run's own style boundary — the
    /// case that fails silently if the splitting is done per hit instead of per run.
    #[test]
    fn a_match_spanning_two_runs_paints_both() {
        let out = highlight_runs(plain(&[5, 5]), &at(&[(3, 8)]), Some(0));
        assert_eq!(
            shape(&out),
            vec![(3, None), (2, current()), (3, current()), (2, None)]
        );
        assert_eq!(total(&out), 10);
    }

    /// A hit that starts exactly where a run does, and ends exactly where it ends, adds no
    /// zero-length pieces.
    #[test]
    fn a_match_on_a_run_boundary_adds_no_empty_runs() {
        let out = highlight_runs(plain(&[4, 6]), &at(&[(4, 10)]), Some(0));
        assert_eq!(shape(&out), vec![(4, None), (6, current())]);
        assert_eq!(total(&out), 10);

        let out = highlight_runs(plain(&[4, 6]), &at(&[(0, 4)]), Some(0));
        assert_eq!(shape(&out), vec![(4, current()), (6, None)]);
        assert!(out.iter().all(|run| run.len > 0));
    }

    /// The current hit is painted in its own colour and the others in theirs — the only thing that
    /// tells the reader which hit "3 of 17" means.
    #[test]
    fn the_current_match_is_distinguished() {
        let out = highlight_runs(plain(&[12]), &at(&[(0, 2), (4, 6), (8, 10)]), Some(1));
        assert_eq!(
            shape(&out),
            vec![
                (2, other()),
                (2, None),
                (2, current()),
                (2, None),
                (2, other()),
                (2, None),
            ]
        );
        assert_ne!(other(), current(), "the two backgrounds must differ");
    }

    /// The current hit in another row: every hit here is painted, none of them as current.
    #[test]
    fn a_row_without_the_current_match_paints_none_of_it() {
        let out = highlight_runs(plain(&[8]), &at(&[(2, 4)]), None);
        assert_eq!(shape(&out), vec![(2, None), (2, other()), (4, None)]);
    }

    /// The engine guarantees non-overlapping hits. This asserts the splitting does not *rely* on
    /// it: overlapping and empty input clip instead of producing runs that no longer sum to the
    /// string's length, which is the failure that corrupts the shaped text.
    #[test]
    fn overlapping_or_empty_input_still_covers_the_text_exactly() {
        for bounds in [
            // Overlapping, nested, empty, inverted, identical, and past the end.
            vec![(2, 6), (4, 8)],
            vec![(0, 10), (3, 4)],
            vec![(5, 5)],
            vec![(9, 3)],
            vec![(0, 10), (0, 10)],
            vec![(8, 20)],
        ] {
            let ranges = at(&bounds);
            let out = highlight_runs(plain(&[4, 6]), &ranges, Some(0));
            assert_eq!(total(&out), 10, "{ranges:?} did not cover the text");
            assert!(
                out.iter().all(|run| run.len > 0),
                "{ranges:?} produced an empty run"
            );
        }
    }

    /// A hit past the end of the text cannot reach past the last run either.
    #[test]
    fn a_match_past_the_end_is_clipped() {
        let out = highlight_runs(plain(&[4]), &at(&[(2, 99)]), Some(0));
        assert_eq!(shape(&out), vec![(2, None), (2, current())]);
    }

    // ── End to end: the engine's hits through the real walk ─────────

    /// Every run painted in `background`, concatenated — what a reader would see marked.
    fn marked(text: &SharedString, runs: &[TextRun], background: Option<Hsla>) -> String {
        let mut out = String::new();
        let mut at = 0usize;
        for run in runs {
            if run.background_color == background {
                out.push_str(&text[at..at + run.len]);
            }
            at += run.len;
        }
        out
    }

    /// The whole path for one block: parse, ask the engine, walk the spans, read back what came out
    /// marked — the current hit first, the others second. `runs_with` is the seam, because a test
    /// has no `list()` row to open a scope for.
    fn find_and_mark(source: &str, query: &str, at: Option<usize>) -> (String, String) {
        let doc = parse(source);
        let hits = Hits {
            ranges: doc
                .find(query, ubiq_md::FindOptions::default())
                .into_iter()
                .map(|m| m.range)
                .collect(),
            current: at,
        };
        let (text, runs) = runs_with(&doc.blocks[0].spans, &Base::body(), Some(&hits));
        assert_eq!(
            runs.iter().map(|run| run.len).sum::<usize>(),
            text.len(),
            "runs no longer cover {text:?}"
        );
        (
            marked(&text, &runs, current()),
            marked(&text, &runs, other()),
        )
    }

    /// A hit given in source bytes is painted on the rendered bytes of the same words, across the
    /// inline syntax the rendered text drops: the engine reports `foo bar` in `foo *bar* baz` as
    /// the source range *through* the `*`, and the `*` is not in the string being painted.
    #[test]
    fn a_hit_is_painted_on_the_words_it_matched() {
        let (current, others) = find_and_mark("foo *bar* baz\n", "foo bar", Some(0));
        assert_eq!(current, "foo bar");
        assert_eq!(others, "");
    }

    /// Three hits in one row, the second current: each is painted, and exactly one in the current
    /// colour.
    #[test]
    fn one_of_several_hits_in_a_row_is_the_current_one() {
        let (current, others) = find_and_mark("hello hello hello\n", "hello", Some(1));
        assert_eq!(current, "hello");
        assert_eq!(others, "hellohello");
    }

    /// The current hit is in some other row: this one paints all of its hits, none of them as
    /// current.
    #[test]
    fn a_row_without_the_current_hit_paints_the_rest() {
        let (current, others) = find_and_mark("a word and a word\n", "word", None);
        assert_eq!(current, "");
        assert_eq!(others, "wordword");
    }

    // ── The native element's half of the same walk ──────────────────

    /// Every run's length, scale and chip flag — the whole of what the native element gets that a
    /// `TextRun` could not carry.
    fn sizes(pieces: &[prose::Run]) -> Vec<(usize, f32, bool)> {
        pieces
            .iter()
            .map(|piece| (piece.run.len, piece.scale, piece.chip))
            .collect()
    }

    /// The same invariant as [`runs`], on the other consumer: the run lengths sum to the string's
    /// byte length, for every block in a document that uses every inline the engine emits.
    #[test]
    fn prose_runs_cover_the_text_exactly() {
        let doc = parse(
            "# A *heading* with `code`\n\n\
             Prose with **strong**, `a span`, a [link](https://x.test) and a break\\\nafter it.\n\n\
             - [x] an item with `code`\n",
        );
        let mut seen = 0usize;
        walk_blocks(&doc.blocks, &mut |spans| {
            let ProseText {
                text, runs: pieces, ..
            } = prose_runs(spans, &Base::body());
            let total: usize = pieces.iter().map(|piece| piece.run.len).sum();
            assert_eq!(total, text.len(), "runs do not cover {text:?}");
            seen += 1;
        });
        assert!(seen >= 4, "walked only {seen} span lists");
    }

    /// The point of the element: an inline code span comes out at 0.9× with the chip asked for,
    /// and the prose around it at 1.0× without one — which the `StyledText` path cannot say.
    #[test]
    fn inline_code_carries_its_own_size_and_a_chip() {
        let doc = parse("say `it` now\n");
        let ProseText {
            text, runs: pieces, ..
        } = prose_runs(&doc.blocks[0].spans, &Base::body());
        assert_eq!(&*text, "say it now");
        assert_eq!(
            sizes(&pieces),
            vec![
                (4, 1.0, false),
                (2, prose::CODE_SCALE, true),
                (4, 1.0, false),
            ]
        );
        const { assert!(prose::CODE_SCALE < 1.0) };
    }

    /// The flat chip background is the old path's stand-in for the chip; the element paints a
    /// quad instead, so the run must arrive without it or both would be drawn.
    #[test]
    fn the_element_gets_no_flat_chip_background() {
        let doc = parse("a `span` here\n");
        let pieces = prose_runs(&doc.blocks[0].spans, &Base::body()).runs;
        assert!(pieces.iter().any(|piece| piece.chip), "no chip run");
        assert!(
            pieces
                .iter()
                .all(|piece| piece.run.background_color.is_none()),
            "a background survived into the element's runs"
        );

        // And the old path still has it, because a `StyledText` has no other way to draw it.
        let (_, runs) = runs(&doc.blocks[0].spans, &Base::body());
        assert!(
            runs.iter()
                .any(|run| run.background_color == Some(theme::markdown().code_chip.into())),
            "the fallback lost the chip"
        );
    }

    /// **The two highlight paths mark the same bytes.** `highlight_runs` splits runs and sets a
    /// background; the element gets ranges and lays quads. Both come out of one [`text_ranges`]
    /// mapping, and this is what says they have not drifted — a quad over the wrong words is the
    /// failure that would otherwise only show on a screen nobody here can open.
    #[test]
    fn the_quads_cover_what_the_backgrounds_covered() {
        for (source, query, at) in [
            ("foo *bar* baz\n", "foo bar", Some(0)),
            ("hello hello hello\n", "hello", Some(1)),
            ("a word and a word\n", "word", None),
            ("code `in a span` and out\n", "in a span", Some(0)),
            ("a **strong** word\n", "strong word", Some(0)),
            ("nothing matches here\n", "zzz", None),
        ] {
            let doc = parse(source);
            let hits = Hits {
                ranges: doc
                    .find(query, ubiq_md::FindOptions::default())
                    .into_iter()
                    .map(|m| m.range)
                    .collect(),
                current: at,
            };
            let base = Base::body();
            let (text, old) = runs_with(&doc.blocks[0].spans, &base, Some(&hits));
            let ProseText {
                text: prose_text,
                runs: pieces,
                highlights: quads,
                ..
            } = prose_with(&doc.blocks[0].spans, &base, Some(&hits));

            assert_eq!(text, prose_text, "{source:?} built two different strings");
            assert_eq!(
                pieces.iter().map(|piece| piece.run.len).sum::<usize>(),
                text.len(),
                "{source:?} runs no longer cover the text"
            );

            let painted = |want: bool| -> String {
                quads
                    .iter()
                    .filter(|hit| hit.current == want)
                    .map(|hit| &text[hit.range.clone()])
                    .collect()
            };
            assert_eq!(
                marked(&text, &old, current()),
                painted(true),
                "{source:?}: the current hit moved"
            );
            assert_eq!(
                marked(&text, &old, other()),
                painted(false),
                "{source:?}: the other hits moved"
            );
        }
    }

    /// A row with no hits asks the element to paint nothing, and costs it no ranges.
    #[test]
    fn a_row_with_no_hits_has_no_quads() {
        let doc = parse("plain prose\n");
        let quads = prose_with(&doc.blocks[0].spans, &Base::body(), None).highlights;
        assert!(quads.is_empty());
    }

    /// A link's click target covers exactly the words it draws, across the marks inside it, and
    /// carries its destination; prose outside any link has none.
    #[test]
    fn a_link_is_a_range_of_the_rendered_text() {
        let doc = parse("see [the **docs**](https://x.test/a) and [b](b.md) or `x`\n");
        let out = prose_runs(&doc.blocks[0].spans, &Base::body());
        let found: Vec<(&str, &str)> = out
            .links
            .iter()
            .map(|link| (&out.text[link.range.clone()], link.dest.as_ref()))
            .collect();
        assert_eq!(found, [("the docs", "https://x.test/a"), ("b", "b.md")]);

        let plain = prose_runs(&parse("no links here\n").blocks[0].spans, &Base::body());
        assert!(plain.links.is_empty());
    }

    /// A hit attributed to the row but belonging to a nested block this walk never saw — a table
    /// cell's, a list item's — maps to nothing and is dropped rather than painted at offset zero.
    #[test]
    fn a_hit_outside_this_walk_is_dropped() {
        let doc = parse("hello\n");
        let hits = Hits {
            ranges: at(&[(400, 404)]),
            current: Some(0),
        };
        let (text, runs) = runs_with(&doc.blocks[0].spans, &Base::body(), Some(&hits));
        assert_eq!(marked(&text, &runs, current()), "");
        assert_eq!(marked(&text, &runs, other()), "");
        assert_eq!(runs.len(), 1, "an unmapped hit split the runs anyway");
    }
}
