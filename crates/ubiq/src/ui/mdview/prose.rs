//! The native element: one block's prose, shaped and painted here rather than handed to
//! [`gpui::StyledText`].
//!
//! **Why this file exists.** A `gpui::TextRun` carries a family, a weight, a slant, a colour, a
//! background, an underline and a strikethrough — and no size. `StyledText` takes one font size
//! for the whole element, from the ambient text style. So three roadmap items were all stuck
//! behind the same wall: inline code at 0.9× body (item 14), a chip with padding and a corner
//! radius instead of a background hugging the glyph box (§15.1), and letter-spacing or a justified
//! measure (§7, §12 — `gpui::TextAlign` has no `Justify` and `TextStyle` has no letter-spacing
//! field at all). All three need the same thing: an element that owns its own shaping and its own
//! paint. This is that element; it does the first two and opens the door to the third.
//!
//! # How it lays out
//!
//! The text arrives as one string plus a [`Run`] per style change — [`inline::prose_runs`] builds
//! both from the engine's inline spans, in the same walk the old path uses.
//!
//! 1. **One shaped line per run**, at that run's own size (`body × run.scale`). Runs are split at
//!    `\n` first, because a hard break is a line the author asked for and
//!    [`gpui::WindowTextSystem::shape_line`] refuses a newline. Shaping per *run* rather than per
//!    word keeps kerning inside a run; it is lost across a run boundary, which is the price of
//!    per-run sizes and is invisible at the boundary of a style change.
//! 2. **Greedy wrapping at word boundaries**, measured through
//!    [`gpui::LineLayout::x_for_index`] and cut with [`gpui::ShapedLine::split_at`] — so a wrapped
//!    fragment keeps the glyph positions it was shaped with instead of being reshaped.
//! 3. **One shared baseline per line**, not a top alignment. A line's ascent is the largest ascent
//!    any of its pieces wants, each piece is drawn so that *its* baseline lands on the line's, and
//!    the line box is the larger of the block's requested leading and what the glyphs need. That
//!    is the whole of what makes a 0.9× code span sit in a 1.0× sentence instead of floating.
//! 4. **Quads under the glyphs**: the inline-code chip ([`chip_box`]) and the search highlight
//!    ([`mark_box`]), both padded, which a `TextRun::background_color` cannot be. Square, like
//!    every surface in Ubiq: the house has no radii.
//! 5. **A click target per link piece** ([`Link`]): a hitbox over each laid-out fragment of a link,
//!    a pointing hand over it, and the destination handed to the view's [`LinkHandler`] on click.
//!
//! # What still uses the old path
//!
//! Prose only — paragraphs, headings, and therefore list-item, quote and footnote text, since
//! those are paragraphs inside a container. A code fence, a table, front matter, raw HTML and
//! `BlockKind::Other` keep [`gpui::StyledText`]: they are monospace or grid-shaped, one size
//! throughout, and gain nothing from per-run sizing. Landing this per block kind rather than as
//! one cutover is deliberate.
//!
//! The spike's `MDVIEW_PROSE=0` revert switch is gone: prose always paints through this element.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, AvailableSpace, BorderStyle, Bounds, CursorStyle, DispatchPhase, Element, ElementId,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseUpEvent, Pixels, ShapedLine, SharedString, Size, Style, TextAlign, TextRun,
    Window, WindowTextSystem, fill, point, px, quad, size,
};

use crate::theme;

// ── The numbers ─────────────────────────────────────────────────────

/// Inline code's size, as a multiple of the block's body size — §15.1's 0.9×, the thing a
/// `TextRun` could not express and this element exists for.
pub const CODE_SCALE: f32 = 0.9;

/// How far the chip's fill reaches past the glyphs, horizontally. §15.1's 2–3px.
const CHIP_PAD_X: f32 = 2.5;

/// And vertically, past the run's own ascent and descent. Smaller than the horizontal padding on
/// purpose: a chip taller than the line box pushes the leading out.
const CHIP_PAD_Y: f32 = 1.0;

/// Inline code's optical baseline offset, downward, as a multiple of the code run's own size.
///
/// Not a correction for the size difference — the shared baseline already handles that. A
/// monospace face at 0.9× has a taller x-height relative to its em than the prose around it, so it
/// reads as sitting slightly high on the same baseline; a hair under half a percent of the size
/// puts it back. Deliberately tiny: this is the one number here that has never been looked at on a
/// screen.
const CODE_NUDGE: f32 = 0.02;

/// A search hit's quad, past the glyphs it covers.
const MARK_PAD_X: f32 = 1.0;
const MARK_PAD_Y: f32 = 1.0;

// ── What the element is given ───────────────────────────────────────

/// One run of the block's text: everything a [`TextRun`] carries, plus the two things it cannot.
#[derive(Clone, Debug)]
pub struct Run {
    /// Family, weight, slant, colour, underline, strikethrough — and `len`, the run's length in
    /// bytes of the block's string. The lengths must sum to that string's length exactly, the same
    /// invariant `inline::runs` keeps.
    pub run: TextRun,
    /// This run's size, as a multiple of the block's body size. 1.0 for prose, [`CODE_SCALE`] for
    /// inline code.
    pub scale: f32,
    /// Draw the chip behind it (§15.1). Set for an inline code span and nothing else.
    pub chip: bool,
}

/// A search hit, as a byte range of the block's *rendered* string — `inline::prose_runs` has
/// already mapped it out of source bytes through the same piece table the old path uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub current: bool,
}

/// A link, as a byte range of the block's rendered string and where it goes —
/// `inline::prose_runs` records one per closed `Mark::Link`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub range: Range<usize>,
    pub dest: SharedString,
}

/// What a click on a link does. The view that draws the rows supplies it; the element only knows
/// the destination it was given.
pub type LinkHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;

// ── Geometry, as plain numbers ──────────────────────────────────────

/// A rectangle in the element's own coordinates, before it becomes a [`Bounds`].
///
/// Plain `f32` because every geometry decision in this file is arithmetic that wants testing and
/// `Pixels` only gets in the way of reading the test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn bounds(self) -> Bounds<Pixels> {
        Bounds::new(point(px(self.x), px(self.y)), size(px(self.w), px(self.h)))
    }
}

/// A line's ascent and descent, from its pieces' own — each piece given as
/// `(ascent, descent, shift)`, where `shift` moves that piece's baseline **down** off the line's.
///
/// A piece nudged down by `s` needs `s` less room above the baseline and `s` more below it, which
/// is the whole reason the shift cannot be applied at paint time and forgotten: a chip nudged into
/// the descender of the line below is the bug this prevents.
pub fn line_extents(pieces: &[(f32, f32, f32)]) -> (f32, f32) {
    let mut ascent = 0.0_f32;
    let mut descent = 0.0_f32;
    for (piece_ascent, piece_descent, shift) in pieces {
        ascent = ascent.max(piece_ascent - shift);
        descent = descent.max(piece_descent + shift);
    }
    (ascent, descent)
}

/// A line's box: its height, and how far the shared baseline sits below its top.
///
/// `requested` is the block's leading. The glyphs win when they need more than it — an H1 with a
/// 1.15 leading still has to fit its own ascenders — and the extra is split evenly above and
/// below, which is what `ShapedLine::paint` does for a single line and what keeps a mixed-size
/// line centred in its leading rather than hanging from the top of it.
pub fn line_box(ascent: f32, descent: f32, requested: f32) -> (f32, f32) {
    let content = ascent + descent;
    let height = requested.max(content);
    (height, (height - content) / 2.0 + ascent)
}

/// The chip behind one piece of an inline code span (§15.1).
///
/// `x` and `width` are the piece's pen position and shaped width, `baseline` the line's shared
/// baseline, `ascent`/`descent` the piece's own, and `shift` its optical offset. `open` and
/// `close` say whether this piece holds the span's first and last bytes: a span that wrapped is
/// padded at its real ends and cut square where it broke, so the two halves do not read as two
/// chips.
#[allow(clippy::too_many_arguments)]
pub fn chip_box(
    x: f32,
    width: f32,
    baseline: f32,
    ascent: f32,
    descent: f32,
    shift: f32,
    open: bool,
    close: bool,
) -> Rect {
    let left = if open { CHIP_PAD_X } else { 0.0 };
    let right = if close { CHIP_PAD_X } else { 0.0 };
    Rect {
        x: x - left,
        y: baseline + shift - ascent - CHIP_PAD_Y,
        w: width + left + right,
        h: ascent + descent + 2.0 * CHIP_PAD_Y,
    }
}

/// A search hit's quad, over the glyphs from `x0` to `x1` of one piece.
pub fn mark_box(x0: f32, x1: f32, baseline: f32, ascent: f32, descent: f32, shift: f32) -> Rect {
    Rect {
        x: x0 - MARK_PAD_X,
        y: baseline + shift - ascent - MARK_PAD_Y,
        w: (x1 - x0) + 2.0 * MARK_PAD_X,
        h: ascent + descent + 2.0 * MARK_PAD_Y,
    }
}

/// The byte offsets inside `text` where a line may break: the first byte of every word after the
/// first.
///
/// Word boundaries only, and never offset zero — a cut at zero would make no progress and the
/// wrapping loop relies on that. A single word longer than the measure has no break point here and
/// overflows rather than being split mid-word, which is the right failure for prose (it is the
/// wrong one for a URL, and that is a roadmap line, not a hedge).
pub fn break_points(text: &str) -> Vec<usize> {
    let mut points = Vec::new();
    let mut after_space = false;
    for (at, ch) in text.char_indices() {
        if ch.is_whitespace() {
            after_space = true;
        } else {
            if after_space && at > 0 {
                points.push(at);
            }
            after_space = false;
        }
    }
    points
}

/// The hits that fall inside one piece, as offsets **local to that piece**.
///
/// `start` and `len` place the piece in the block's string. A hit is clipped to the piece, so a hit
/// that spans a wrap or a style change comes back once per piece it touches and the quads join up.
pub fn local_marks(
    start: usize,
    len: usize,
    highlights: &[Highlight],
) -> Vec<(Range<usize>, bool)> {
    highlights
        .iter()
        .filter_map(|hit| clip(start, len, &hit.range).map(|range| (range, hit.current)))
        .collect()
}

/// `range` clipped to the piece at `start..start + len`, as offsets local to the piece — `None`
/// when they do not overlap. What a search hit and a link both go through.
pub fn clip(start: usize, len: usize, range: &Range<usize>) -> Option<Range<usize>> {
    let from = range.start.max(start);
    let to = range.end.min(start + len);
    (from < to).then(|| from - start..to - start)
}

// ── The laid-out block ──────────────────────────────────────────────

/// One shaped fragment, placed.
struct Piece {
    line: ShapedLine,
    /// Pen x within the line.
    x: f32,
    /// This piece's baseline offset off the line's, downward.
    shift: f32,
    /// Where this piece's first byte sits in the block's string.
    start: usize,
    chip: bool,
    /// Whether this piece holds its run's first / last byte — see [`chip_box`].
    chip_open: bool,
    chip_close: bool,
}

/// One drawn line.
struct Line {
    pieces: Vec<Piece>,
    /// The shared baseline, below the line's top.
    baseline: f32,
    height: f32,
    width: f32,
}

/// The whole block, shaped for one wrap width.
struct Laid {
    lines: Vec<Line>,
    /// The width it was shaped for. A measure pass at a different width reshapes; the same width
    /// answers from here, which matters because GPUI calls the measure closure more than once.
    wrap: Option<Pixels>,
    size: Size<Pixels>,
    bounds: Option<Bounds<Pixels>>,
}

/// One run, split at its hard breaks and shaped.
struct Segment {
    line: ShapedLine,
    start: usize,
    scale: f32,
    chip: bool,
    /// The author's own line break follows this segment.
    hard_break: bool,
    /// Whether this segment still holds its run's first / last byte — the chip's end caps.
    opens_run: bool,
    closes_run: bool,
}

/// Shape every run, split at the hard breaks, at the run's own size.
///
/// A run's background is dropped here on purpose: this element paints the chip and the highlight
/// as quads, so leaving `background_color` on would draw the old flat fill under the new one.
fn segments(text: &str, runs: &[Run], body: f32, text_system: &WindowTextSystem) -> Vec<Segment> {
    let mut out = Vec::with_capacity(runs.len());
    let mut at = 0usize;

    for run in runs {
        let end = (at + run.run.len).min(text.len());
        let Some(slice) = text.get(at..end) else {
            at = end;
            continue;
        };
        let font_size = px(body * run.scale);
        let mut cursor = 0usize;
        loop {
            let (chunk, next) = match slice[cursor..].find('\n') {
                Some(n) => (&slice[cursor..cursor + n], Some(cursor + n + 1)),
                None => (&slice[cursor..], None),
            };
            if !chunk.is_empty() {
                let mut styled = run.run.clone();
                styled.len = chunk.len();
                styled.background_color = None;
                out.push(Segment {
                    line: text_system.shape_line(
                        SharedString::new(chunk),
                        font_size,
                        &[styled],
                        None,
                    ),
                    start: at + cursor,
                    scale: run.scale,
                    chip: run.chip,
                    hard_break: next.is_some(),
                    opens_run: cursor == 0,
                    closes_run: next.is_none(),
                });
            } else if let Some(last) = out.last_mut()
                && next.is_some()
            {
                // An empty chunk carries no glyphs, but the break it sits before is still the
                // author's — hand it to whatever came last rather than losing the line.
                last.hard_break = true;
            }
            match next {
                Some(n) => cursor = n,
                None => break,
            }
        }
        at = end;
    }

    out
}

/// Pack the shaped segments into lines no wider than `wrap`, and give each line its baseline.
fn wrap_lines(mut queue: Vec<Segment>, body: f32, requested: f32, wrap: Option<f32>) -> Vec<Line> {
    // Reversed so the queue pops from the front cheaply; a cut segment's tail goes back on top.
    queue.reverse();

    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Piece> = Vec::new();
    let mut pen = 0.0_f32;
    // A break *before* a segment is only allowed at a word boundary, so `foo**bar**baz` does not
    // wrap between its runs.
    let mut at_word_start = true;

    while let Some(segment) = queue.pop() {
        let width = f32::from(segment.line.width());
        // Half a pixel of slack: a measure that rounds up by a hair must not wrap the word that
        // fit when it was shaped.
        let fits = wrap.is_none_or(|limit| pen + width <= limit + 0.5);

        if !fits {
            let limit = wrap.unwrap_or(f32::MAX);
            let available = (limit - pen).max(0.0);
            let cut = break_points(&segment.line.text)
                .into_iter()
                .rev()
                .find(|&at| f32::from(segment.line.x_for_index(at)) <= available);

            match cut {
                Some(at) => {
                    let (head, tail) = segment.line.split_at(at);
                    queue.push(Segment {
                        line: tail,
                        start: segment.start + at,
                        scale: segment.scale,
                        chip: segment.chip,
                        hard_break: segment.hard_break,
                        opens_run: false,
                        closes_run: segment.closes_run,
                    });
                    // The head keeps the span's opening cap and loses its closing one: a chip that
                    // wrapped is one chip in two halves, not two chips.
                    current.push(place(&segment, head, pen, body, segment.opens_run, false));
                    lines.push(finish(std::mem::take(&mut current), requested));
                    pen = 0.0;
                    at_word_start = true;
                    continue;
                }
                None if at_word_start && !current.is_empty() => {
                    // Nothing of this segment fits and the line ends at a word boundary: the whole
                    // segment moves down.
                    queue.push(segment);
                    lines.push(finish(std::mem::take(&mut current), requested));
                    pen = 0.0;
                    at_word_start = true;
                    continue;
                }
                // Mid-word, or a word longer than the measure: overflow rather than cut a word.
                None => {}
            }
        }

        at_word_start = segment
            .line
            .text
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        let hard_break = segment.hard_break;
        current.push(place(
            &segment,
            segment.line.clone(),
            pen,
            body,
            segment.opens_run,
            segment.closes_run,
        ));
        pen += width;

        if hard_break {
            lines.push(finish(std::mem::take(&mut current), requested));
            pen = 0.0;
            at_word_start = true;
        }
    }

    if !current.is_empty() {
        lines.push(finish(current, requested));
    }
    lines
}

/// One segment's shaped fragment, placed at the pen.
fn place(
    segment: &Segment,
    line: ShapedLine,
    x: f32,
    body: f32,
    opens: bool,
    closes: bool,
) -> Piece {
    Piece {
        shift: if segment.chip {
            body * segment.scale * CODE_NUDGE
        } else {
            0.0
        },
        x,
        start: segment.start,
        chip: segment.chip,
        chip_open: segment.chip && opens,
        chip_close: segment.chip && closes,
        line,
    }
}

/// Close a line: its extents, its box, and its width.
fn finish(pieces: Vec<Piece>, requested: f32) -> Line {
    let extents: Vec<(f32, f32, f32)> = pieces
        .iter()
        .map(|piece| {
            (
                f32::from(piece.line.ascent),
                f32::from(piece.line.descent),
                piece.shift,
            )
        })
        .collect();
    let (ascent, descent) = line_extents(&extents);
    let (height, baseline) = line_box(ascent, descent, requested);
    let width = pieces
        .iter()
        .map(|piece| piece.x + f32::from(piece.line.width()))
        .fold(0.0_f32, f32::max);

    Line {
        pieces,
        baseline,
        height,
        width,
    }
}

/// Shape and place the whole block.
fn lay_out(
    text: &str,
    runs: &[Run],
    body: f32,
    requested: f32,
    wrap: Option<f32>,
    text_system: &WindowTextSystem,
) -> (Vec<Line>, f32, f32) {
    let lines = wrap_lines(
        segments(text, runs, body, text_system),
        body,
        requested,
        wrap,
    );
    let width = lines.iter().map(|line| line.width).fold(0.0_f32, f32::max);
    let height = lines.iter().map(|line| line.height).sum::<f32>();
    // An empty block still occupies its leading, or the row's margins collapse onto each other.
    (lines, width, if height > 0.0 { height } else { requested })
}

// ── The element ─────────────────────────────────────────────────────

/// One block's prose, shaped and painted by this crate.
pub struct Prose {
    text: SharedString,
    runs: Rc<Vec<Run>>,
    highlights: Rc<Vec<Highlight>>,
    /// The block's body size in pixels — 1em, the thing every run's `scale` multiplies.
    body: Pixels,
    /// The leading the block asks for. The glyphs override it upward, never downward.
    line_height: Pixels,
    links: Rc<Vec<Link>>,
    /// What a click on one of [`Self::links`] does. `None` draws the links without click targets.
    on_link: Option<LinkHandler>,
    state: Rc<RefCell<Option<Laid>>>,
}

impl Prose {
    pub fn new(
        text: SharedString,
        runs: Vec<Run>,
        highlights: Vec<Highlight>,
        body: Pixels,
        line_height: Pixels,
    ) -> Self {
        Prose {
            text,
            runs: Rc::new(runs),
            highlights: Rc::new(highlights),
            body,
            line_height,
            links: Rc::new(Vec::new()),
            on_link: None,
            state: Rc::new(RefCell::new(None)),
        }
    }

    /// The block's links, and what clicking one does. Without a handler the links draw (they are
    /// runs like any other) but take no click.
    pub fn links(mut self, links: Vec<Link>, on_link: Option<LinkHandler>) -> Self {
        self.links = Rc::new(links);
        self.on_link = on_link;
        self
    }
}

impl Element for Prose {
    type RequestLayoutState = ();
    /// One hitbox per laid-out fragment of a link, with the destination it opens.
    type PrepaintState = Vec<(Hitbox, SharedString)>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text = self.text.clone();
        let runs = self.runs.clone();
        let state = self.state.clone();
        let body = f32::from(self.body);
        let requested = f32::from(self.line_height);

        // `Fn`, and called more than once per frame: everything it needs is cloned in, and the
        // result is cached against the width it was shaped for.
        let layout_id = window.request_measured_layout(
            Style::default(),
            move |known, available, window, _cx| {
                let wrap = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let cached = state
                    .borrow()
                    .as_ref()
                    .and_then(|laid| (laid.wrap == wrap).then_some(laid.size));
                if let Some(cached) = cached {
                    return cached;
                }

                let (lines, width, height) = lay_out(
                    &text,
                    &runs,
                    body,
                    requested,
                    wrap.map(f32::from),
                    window.text_system(),
                );
                let measured = size(px(width.ceil()), px(height));
                *state.borrow_mut() = Some(Laid {
                    lines,
                    wrap,
                    size: measured,
                    bounds: None,
                });
                measured
            },
        );

        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let mut state = self.state.borrow_mut();
        let Some(laid) = state.as_mut() else {
            return Vec::new();
        };
        laid.bounds = Some(bounds);
        if self.on_link.is_none() || self.links.is_empty() {
            return Vec::new();
        }

        let left = f32::from(bounds.origin.x);
        let mut top = f32::from(bounds.origin.y);
        let mut targets = Vec::new();
        for line in &laid.lines {
            for piece in &line.pieces {
                for link in self.links.iter() {
                    let Some(range) = clip(piece.start, piece.line.len(), &link.range) else {
                        continue;
                    };
                    let x0 = f32::from(piece.line.x_for_index(range.start));
                    let x1 = f32::from(piece.line.x_for_index(range.end));
                    let rect = Rect {
                        x: left + piece.x + x0,
                        y: top,
                        w: (x1 - x0).max(1.0),
                        h: line.height,
                    };
                    let hitbox = window.insert_hitbox(rect.bounds(), HitboxBehavior::Normal);
                    targets.push((hitbox, link.dest.clone()));
                }
            }
            top += line.height;
        }
        targets
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        targets: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(on_link) = &self.on_link {
            for (hitbox, dest) in targets.drain(..) {
                window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
                let on_link = on_link.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble
                        && event.button == MouseButton::Left
                        && hitbox.is_hovered(window)
                    {
                        on_link(&dest, window, cx);
                    }
                });
            }
        }

        let state = self.state.clone();
        let laid = state.borrow();
        let Some(laid) = laid.as_ref() else { return };
        let Some(bounds) = laid.bounds else { return };

        let left = f32::from(bounds.origin.x);
        let mut top = f32::from(bounds.origin.y);

        for line in &laid.lines {
            let baseline = top + line.baseline;

            // Every quad first, so the glyphs land on top of both the chip and the highlight.
            for piece in &line.pieces {
                let x = left + piece.x;
                let ascent = f32::from(piece.line.ascent);
                let descent = f32::from(piece.line.descent);
                let width = f32::from(piece.line.width());

                if piece.chip {
                    let rect = chip_box(
                        x,
                        width,
                        baseline,
                        ascent,
                        descent,
                        piece.shift,
                        piece.chip_open,
                        piece.chip_close,
                    );
                    window.paint_quad(quad(
                        rect.bounds(),
                        px(0.),
                        theme::markdown().code_chip_strong,
                        px(theme::hairline()),
                        theme::markdown().code_chip_border,
                        BorderStyle::Solid,
                    ));
                }

                for (range, current) in local_marks(piece.start, piece.line.len(), &self.highlights)
                {
                    let x0 = x + f32::from(piece.line.x_for_index(range.start));
                    let x1 = if range.end >= piece.line.len() {
                        x + width
                    } else {
                        x + f32::from(piece.line.x_for_index(range.end))
                    };
                    let rect = mark_box(x0, x1, baseline, ascent, descent, piece.shift);
                    let colour = if current {
                        theme::markdown().search_match_current
                    } else {
                        theme::markdown().search_match
                    };
                    window.paint_quad(fill(rect.bounds(), colour));
                }
            }

            for piece in &line.pieces {
                let ascent = f32::from(piece.line.ascent);
                let descent = f32::from(piece.line.descent);
                // A line height of exactly the piece's own extents makes `ShapedLine::paint`'s
                // internal padding zero, so its baseline is `origin.y + ascent` — which is how a
                // piece of any size is put on the line's shared baseline.
                let origin = point(px(left + piece.x), px(baseline + piece.shift - ascent));
                let _ = piece.line.paint(
                    origin,
                    px(ascent + descent),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            top += line.height;
        }
    }
}

impl IntoElement for Prose {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Word boundaries ─────────────────────────────────────────────

    #[test]
    fn break_points_are_word_starts_and_never_zero() {
        assert_eq!(break_points("one two three"), vec![4, 8]);
        assert_eq!(break_points("one"), Vec::<usize>::new());
        // Leading and doubled spaces do not produce a break at zero or an empty word.
        assert_eq!(break_points("  one  two"), vec![2, 7]);
        assert_eq!(break_points(""), Vec::<usize>::new());
        assert_eq!(break_points("   "), Vec::<usize>::new());
    }

    /// Every break point is a char boundary, or `split_at` would cut a code point in half.
    #[test]
    fn break_points_land_on_char_boundaries() {
        let text = "héllo wörld — ünicode";
        for at in break_points(text) {
            assert!(text.is_char_boundary(at), "{at} is not a boundary");
            assert!(at > 0);
        }
        assert!(!break_points(text).is_empty());
    }

    // ── Baseline alignment for mixed sizes ──────────────────────────

    /// One size: the line is exactly that piece's extents.
    #[test]
    fn a_single_size_line_takes_its_own_extents() {
        let (ascent, descent) = line_extents(&[(12.0, 4.0, 0.0)]);
        assert_eq!((ascent, descent), (12.0, 4.0));
    }

    /// The point of the whole file: a 0.9× run beside a 1.0× run shares the larger's baseline, so
    /// the line's ascent is the *body's* and not the code's, and nothing is top-aligned.
    #[test]
    fn a_smaller_run_does_not_shrink_the_line() {
        let body = (12.0, 4.0, 0.0);
        let code = (10.8, 3.6, 0.0);
        assert_eq!(line_extents(&[body, code]), (12.0, 4.0));
        assert_eq!(line_extents(&[code, body]), (12.0, 4.0));
    }

    /// A larger run takes the line, in both directions.
    #[test]
    fn a_larger_run_grows_the_line() {
        assert_eq!(
            line_extents(&[(12.0, 4.0, 0.0), (20.0, 6.0, 0.0)]),
            (20.0, 6.0)
        );
    }

    /// A downward shift buys room below the baseline and gives it back above — the invariant that
    /// stops a nudged chip clipping into the next line.
    #[test]
    fn a_shift_moves_room_from_above_the_baseline_to_below() {
        let (ascent, descent) = line_extents(&[(10.0, 3.0, 2.0)]);
        assert_eq!((ascent, descent), (8.0, 5.0));
        // The piece still needs the same total.
        assert_eq!(ascent + descent, 13.0);
    }

    /// Two pieces, one nudged: the line takes the worst of each, so both fit. The nudge is what
    /// makes the code run the deeper of the two even though it is the smaller.
    #[test]
    fn a_shifted_piece_beside_an_unshifted_one_takes_the_worst_of_each() {
        let body = (12.0, 4.0, 0.0);
        let code = (10.8, 3.9, 0.5);
        let (ascent, descent) = line_extents(&[body, code]);
        assert_eq!(ascent, 12.0, "the body still sets the ascent");
        assert!((descent - 4.4).abs() < 1e-5, "descent was {descent}");
        // Neither piece is clipped: each fits inside the line it helped size.
        assert!(ascent >= body.0 - body.2 && descent >= body.1 + body.2);
        assert!(ascent >= code.0 - code.2 && descent >= code.1 + code.2);
    }

    /// A nudge small enough to disappear into the body's own descent costs the line nothing —
    /// which is the case [`CODE_NUDGE`] is sized for.
    #[test]
    fn a_nudge_inside_the_bodys_descent_does_not_grow_the_line() {
        let nudge = 15.0 * CODE_SCALE * CODE_NUDGE;
        assert!(nudge < 0.4, "the nudge is {nudge} at a 15px body");
        let (ascent, descent) = line_extents(&[(12.0, 4.0, 0.0), (10.8, 3.6, nudge)]);
        assert_eq!((ascent, descent), (12.0, 4.0));
    }

    #[test]
    fn no_pieces_need_no_room() {
        assert_eq!(line_extents(&[]), (0.0, 0.0));
    }

    /// The requested leading wins when it is the larger, and the surplus is split evenly — so the
    /// glyphs sit in the middle of the line box, not at its top.
    #[test]
    fn the_leading_centres_the_glyphs() {
        let (height, baseline) = line_box(12.0, 4.0, 24.0);
        assert_eq!(height, 24.0);
        assert_eq!(baseline, 16.0);
        // Equal air above the ascent and below the descent.
        assert_eq!(baseline - 12.0, height - baseline - 4.0);
    }

    /// The glyphs win when the leading is too tight: an H1 at 1.15 leading still fits its own
    /// ascenders rather than being clipped.
    #[test]
    fn the_glyphs_win_a_leading_that_is_too_tight() {
        let (height, baseline) = line_box(20.0, 6.0, 18.0);
        assert_eq!(height, 26.0);
        assert_eq!(baseline, 20.0);
    }

    /// An empty line still has a box, so a blank block keeps its margins.
    #[test]
    fn an_empty_line_still_has_its_leading() {
        let (height, baseline) = line_box(0.0, 0.0, 21.0);
        assert_eq!(height, 21.0);
        assert_eq!(baseline, 10.5);
    }

    // ── Chip geometry ───────────────────────────────────────────────

    /// A whole chip is padded on both sides and sits over the run's measured box.
    #[test]
    fn a_chip_pads_and_covers_its_glyphs() {
        let rect = chip_box(40.0, 30.0, 100.0, 10.8, 3.6, 0.0, true, true);
        assert_eq!(rect.x, 40.0 - CHIP_PAD_X);
        assert_eq!(rect.w, 30.0 + 2.0 * CHIP_PAD_X);
        assert_eq!(rect.y, 100.0 - 10.8 - CHIP_PAD_Y);
        assert!((rect.h - (10.8 + 3.6 + 2.0 * CHIP_PAD_Y)).abs() < 1e-5);
        // The glyphs are inside it, with room to spare.
        assert!(rect.x < 40.0 && rect.x + rect.w > 70.0);
        assert!(rect.y < 100.0 - 10.8 && rect.y + rect.h > 100.0 + 3.6);
    }

    /// A chip that wrapped is padded at its real ends and square where it broke.
    #[test]
    fn a_wrapped_chip_is_capped_only_at_its_ends() {
        let head = chip_box(10.0, 20.0, 50.0, 9.0, 3.0, 0.0, true, false);
        assert_eq!(head.x, 10.0 - CHIP_PAD_X);
        assert_eq!(head.w, 20.0 + CHIP_PAD_X);

        let tail = chip_box(0.0, 20.0, 80.0, 9.0, 3.0, 0.0, false, true);
        assert_eq!(tail.x, 0.0);
        assert_eq!(tail.w, 20.0 + CHIP_PAD_X);
    }

    /// The nudge moves the chip with its glyphs, not against them.
    #[test]
    fn the_nudge_moves_the_chip_with_the_text() {
        let flat = chip_box(0.0, 10.0, 50.0, 9.0, 3.0, 0.0, true, true);
        let nudged = chip_box(0.0, 10.0, 50.0, 9.0, 3.0, 0.3, true, true);
        assert!((nudged.y - flat.y - 0.3).abs() < 1e-5);
        assert_eq!(nudged.h, flat.h);
    }

    /// §15.1's horizontal padding, which is the difference between a chip and a highlight.
    #[test]
    fn the_chip_padding_is_the_one_the_proposal_asks_for() {
        assert!((2.0..=3.0).contains(&CHIP_PAD_X), "§15.1 asks for 2–3px");
        const {
            assert!(
                CHIP_PAD_Y < CHIP_PAD_X,
                "a tall chip pushes the leading out"
            )
        };
    }

    /// Inline code is 0.9× body, and the nudge is optical rather than structural.
    #[test]
    fn inline_code_is_nine_tenths_of_body() {
        assert_eq!(CODE_SCALE, 0.9);
        const { assert!(CODE_NUDGE > 0.0 && CODE_NUDGE < 0.05) };
    }

    // ── The search highlight ────────────────────────────────────────

    fn hit(start: usize, end: usize, current: bool) -> Highlight {
        Highlight {
            range: start..end,
            current,
        }
    }

    /// A hit inside one piece maps to that piece's own offsets.
    #[test]
    fn a_hit_inside_a_piece_is_local_to_it() {
        let marks = local_marks(10, 20, &[hit(12, 16, true)]);
        assert_eq!(marks, vec![(2..6, true)]);
    }

    /// A hit that runs past the piece is clipped at both ends, so two pieces of one hit produce
    /// two quads that meet rather than one quad that overhangs.
    #[test]
    fn a_hit_across_two_pieces_is_clipped_to_each() {
        let hits = vec![hit(5, 25, false)];
        assert_eq!(local_marks(0, 10, &hits), vec![(5..10, false)]);
        assert_eq!(local_marks(10, 10, &hits), vec![(0..10, false)]);
        assert_eq!(local_marks(20, 10, &hits), vec![(0..5, false)]);
        // And nothing outside it.
        assert!(local_marks(30, 10, &hits).is_empty());
    }

    /// A hit in another piece, an empty hit, and an inverted one all draw nothing here.
    #[test]
    fn a_hit_that_touches_nothing_draws_nothing() {
        assert!(local_marks(0, 4, &[hit(10, 12, false)]).is_empty());
        assert!(local_marks(0, 10, &[hit(4, 4, false)]).is_empty());
        assert!(local_marks(0, 10, &[hit(9, 3, false)]).is_empty());
    }

    /// The current hit keeps its flag through the mapping — the only thing that says which hit
    /// "3 of 17" means.
    #[test]
    fn the_current_hit_stays_the_current_one() {
        let marks = local_marks(
            0,
            12,
            &[hit(0, 2, false), hit(4, 6, true), hit(8, 10, false)],
        );
        assert_eq!(marks, vec![(0..2, false), (4..6, true), (8..10, false)]);
    }

    /// A hit's quad covers the glyphs it marks and no more than a pixel past them — the old
    /// `TextRun::background_color` hugged the glyph box exactly, and this is that box plus the
    /// padding that keeps the quad off the glyphs' edges.
    #[test]
    fn a_hit_quad_covers_the_glyph_box_it_replaces() {
        let rect = mark_box(20.0, 60.0, 100.0, 12.0, 4.0, 0.0);
        assert!(rect.x <= 20.0 && rect.x + rect.w >= 60.0);
        assert!((rect.x - (20.0 - MARK_PAD_X)).abs() < 1e-5);
        assert!((rect.w - (40.0 + 2.0 * MARK_PAD_X)).abs() < 1e-5);
        assert!(rect.y <= 100.0 - 12.0 && rect.y + rect.h >= 100.0 + 4.0);
        const { assert!(MARK_PAD_X <= CHIP_PAD_X, "a hit is not a chip") };
    }

    /// A zero-width hit — a hit on a byte that shaped to no advance — still yields a quad of only
    /// its padding rather than a negative rectangle GPUI would refuse.
    #[test]
    fn a_zero_width_hit_is_not_a_negative_quad() {
        let rect = mark_box(30.0, 30.0, 50.0, 9.0, 3.0, 0.0);
        assert!(rect.w > 0.0);
        assert!(rect.h > 0.0);
    }

    // ── Links ───────────────────────────────────────────────────────

    /// A link's range is clipped to each piece it touches, in that piece's own offsets — so a link
    /// that wrapped gets one click target per line, and a piece outside it gets none.
    #[test]
    fn a_link_is_clipped_to_each_piece_it_touches() {
        let link = 5..25;
        assert_eq!(clip(0, 10, &link), Some(5..10));
        assert_eq!(clip(10, 10, &link), Some(0..10));
        assert_eq!(clip(20, 10, &link), Some(0..5));
        assert_eq!(clip(30, 10, &link), None);
        assert_eq!(clip(0, 5, &link), None, "touching is not overlapping");
    }
}
