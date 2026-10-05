//! The structural minimap: a strip down the preview's edge that draws the document's *shape*.
//!
//! Three things decide everything else in this file.
//!
//! **It draws structure, not shrunken text.** Miniature glyphs at a tenth of their size are noise
//! at every scale; a heading bar, a stack of prose lines, a tinted code rectangle and a grid read
//! at any of them. So [`Shape`] is a closed set of six drawings and there is no text in the strip
//! at all: only the layout of the blocks.
//!
//! **Positions are measured, never derived from byte offsets.** The geometry comes from
//! [`ListState::bounds_for_item`], the same sum-tree of measured row heights the preview scrolls
//! on. `bounds_for_item` answers `None` **both** above the current scroll top and below the
//! measured range, so [`Map::build`] treats an unmeasured row as an estimate and lets the
//! measurement replace it the moment the row is laid out (the [`Minimap`] remembers what it has
//! seen).
//!
//! **A short document is not stretched.** [`Projection::fit`] scales to fit *up to a maximum*, and
//! that maximum is the strip's width over the text column's, so a one-page file looks like one
//! page. Past the other end, where a body line would be under a pixel tall, the strip stops
//! shrinking and scrolls itself in proportion to the main view instead.
//!
//! The whole strip is one [`gpui::canvas`] painting quads. The state ([`Minimap`]) is the
//! caller's — the `MdView` entity owns one — and so is the drag: [`strip`] reports a press through
//! `on_down`, the caller sets [`Minimap::dragging`] and calls [`Minimap::scrub`], then keeps calling
//! it from its own mouse-move while `dragging` and clears the flag on mouse-up (a drag's first
//! pixel is usually off the strip, so `on_mouse_move` on the strip alone would miss it).

use gpui::{
    AnyElement, App, Bounds, ContentMask, InteractiveElement as _, IntoElement as _, ListOffset,
    ListState, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Point, Styled as _, Window,
    canvas, div, fill, point, px, size,
};
use ubiq_md::{Block, BlockKind, Document};

use super::blocks::Metrics;
use crate::theme;

// ── The strip's own measure ─────────────────────────────────────────

/// How wide the strip draws. The brief's 80–120px band: wide enough that an H1 bar and a short
/// label are two distinguishable things, narrow enough to cost the prose one indent.
pub const WIDTH: f32 = 104.0;

/// The gap between the text column's far margin and the strip, in em.
const GAP_EM: f32 = 2.0;

/// The strip's own inner padding, so a full-width mark does not touch the edge.
const PAD: f32 = 7.0;

/// The narrowest measure worth reserving the strip against, as a fraction of the target column.
/// Below it the text column has been squeezed past readable and orientation loses — the one rule
/// that decides whether the strip appears at all.
const MIN_MEASURE: f32 = 0.6;

/// The measure the prose estimate assumes, in characters.
///
/// The same 75 the block renderer sets the column to (§13.1). Duplicated rather than shared
/// because `blocks::TARGET_COLUMNS` is private, and it is only ever used to guess the height of a
/// row the list has not measured yet — a measured row never consults it.
const PROSE_COLUMNS: f32 = 75.0;

/// The scale below which the strip stops shrinking and starts scrolling: a body line is about
/// 1.5em, so 0.04 is where one line stops being able to claim a whole pixel.
const MIN_SCALE: f32 = 0.04;

/// What the strip costs the preview, or `None` when it must not be shown.
///
/// **This runs before the centring rule, not after it.** `Metrics::measure` places the text column
/// in whatever width it is handed, so the only way the strip cannot overlap prose is for the width
/// it takes to be gone before the column is placed. The rule itself is one line: readable text
/// wins, so if reserving the strip would leave the column under [`MIN_MEASURE`] of its target with
/// both margins intact, there is no strip.
pub fn reserve(wanted: bool, available: f32, column: f32, margin: f32, body: f32) -> Option<f32> {
    if !wanted {
        return None;
    }
    let cost = WIDTH + GAP_EM * body;
    let floor = column * MIN_MEASURE + 2.0 * margin;
    (available - cost >= floor).then_some(cost)
}

// ── Document pixels onto strip pixels ───────────────────────────────

/// How the document's measured height maps onto the strip: a scale, and how far the strip has
/// scrolled itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    /// Strip pixels per document pixel.
    pub scale: f32,
    /// How far down the scaled document the strip's top edge sits, in strip pixels. Zero whenever
    /// the whole document fits.
    pub offset: f32,
}

impl Projection {
    /// Fit `doc` pixels of document into `strip` pixels of strip.
    ///
    /// `max_scale` is the no-stretch rule: a document shorter than the strip is drawn at that
    /// scale from the top and left alone. `view_top`/`view_height` only matter once the scale has
    /// bottomed out at [`MIN_SCALE`] and the strip has to scroll.
    pub fn fit(
        doc: f32,
        strip: f32,
        max_scale: f32,
        view_top: f32,
        view_height: f32,
    ) -> Projection {
        let max_scale = max_scale.max(MIN_SCALE);
        if doc <= 0.0 || strip <= 0.0 {
            return Projection {
                scale: max_scale,
                offset: 0.0,
            };
        }

        let scale = (strip / doc).clamp(MIN_SCALE, max_scale);
        let scaled = doc * scale;
        // Fits: no scroll, and — the whole point — no stretch either.
        if scaled <= strip {
            return Projection { scale, offset: 0.0 };
        }

        // Too long to fit even at the floor scale. The strip travels with the main view: at the
        // top of the document the strip is at its top, at the bottom at its bottom.
        let travel = (doc - view_height).max(1.0);
        let fraction = (view_top / travel).clamp(0.0, 1.0);
        Projection {
            scale,
            offset: (scaled - strip) * fraction,
        }
    }

    /// A document y in strip coordinates.
    pub fn y(self, doc_y: f32) -> f32 {
        doc_y * self.scale - self.offset
    }

    /// A document height in strip pixels.
    pub fn h(self, doc_h: f32) -> f32 {
        doc_h * self.scale
    }

    /// A strip y back in document coordinates — the click-to-jump direction.
    pub fn doc_y(self, strip_y: f32) -> f32 {
        (strip_y + self.offset) / self.scale.max(f32::MIN_POSITIVE)
    }
}

/// The block a document y lands in, and how far into that block's own span it is, in pixels.
///
/// `tops` is one entry per root block, ascending. A y above the first block lands on block 0 at
/// zero; a y past the last lands on the last block.
pub fn locate(tops: &[f32], doc_y: f32) -> (usize, f32) {
    if tops.is_empty() {
        return (0, 0.0);
    }
    let ix = tops
        .partition_point(|&top| top <= doc_y)
        .saturating_sub(1)
        .min(tops.len() - 1);
    (ix, (doc_y - tops[ix]).max(0.0))
}

/// How many lines a run of `chars` characters wraps to at `per_line`, and how full the last one
/// is, as a fraction.
///
/// The last line's length is the only thing about a paragraph's shape that is not "a full line" —
/// which is exactly what makes a stack of bars read as prose rather than as a filled rectangle.
pub fn wrapped_lines(chars: f32, per_line: f32) -> (usize, f32) {
    let per_line = per_line.max(1.0);
    if chars <= 0.0 {
        return (1, 0.35);
    }
    let count = (chars / per_line).ceil().max(1.0);
    let last = (chars - (count - 1.0) * per_line) / per_line;
    (count as usize, last.clamp(0.2, 1.0))
}

// ── What a block looks like at a tenth of its size ──────────────────

/// The six drawings the strip has. Everything the engine can produce maps onto one of them.
#[derive(Clone, Debug)]
enum Shape {
    /// A short bar whose length and weight follow the level.
    Heading { level: u8 },
    /// A stack of grey lines — a paragraph, a list, a footnote, raw HTML. `indent` is a fraction
    /// of the content width, which is what tells a list from a paragraph at this size.
    Lines {
        last: f32,
        indent: f32,
        /// A quote also draws its left rule, the one piece of block chrome that survives the scale.
        rule: bool,
    },
    /// A code fence or front matter: a tinted rectangle, because code is a shape and not prose.
    Code,
    /// A grid.
    Table { rows: usize, columns: usize },
    /// An image or a diagram: a neutral fill that is plainly neither text nor code.
    Figure,
    /// A thematic break: a hairline, and nothing else.
    Rule,
}

/// One root block as the strip draws it, in document pixels.
#[derive(Clone, Debug)]
struct Mark {
    /// Distance from the document's first pixel to this block's first.
    top: f32,
    /// What the block itself occupies, excluding the gap to the next one.
    height: f32,
    shape: Shape,
}

/// The document as the strip sees it: every block placed, plus where the preview's viewport
/// currently sits in the same coordinates.
#[derive(Clone, Debug)]
struct Map {
    marks: Vec<Mark>,
    /// One entry per block, ascending — [`locate`]'s input.
    tops: Vec<f32>,
    /// The document's full height in document pixels, measured where the list has measured and
    /// estimated everywhere else.
    height: f32,
    /// The viewport's top, in the same coordinates. Exact: it is anchored on the one row the list
    /// always has measured, the one it is scrolled to.
    view_top: f32,
    view_height: f32,
}

impl Map {
    fn project(&self, strip: f32, max_scale: f32) -> Projection {
        Projection::fit(
            self.height,
            strip,
            max_scale,
            self.view_top,
            self.view_height,
        )
    }
}

/// The advances the list has measured, kept between frames.
///
/// Without this the strip's shape would breathe as it scrolled: a row measured a moment ago falls
/// back to its estimate the instant it leaves `bounds_for_item`'s range. The cache only ever
/// *replaces an estimate with a measurement*. Its key is what invalidates it: an edited document or
/// a reflow at a new width.
#[derive(Default)]
struct Cache {
    key: Option<(usize, u64, i32)>,
    /// Top-to-top distance to the next block, where it has been measured.
    advances: Vec<Option<f32>>,
    /// The block's own drawn height, where it has been measured.
    heights: Vec<Option<f32>>,
}

/// The strip's state, one per markdown view: the measurement cache and whether a drag is in
/// flight.
#[derive(Default)]
pub struct Minimap {
    cache: Cache,
    /// A press landed on the strip and the button is still down. Set and cleared by the caller.
    pub dragging: bool,
}

impl Minimap {
    /// Forget every remembered measurement (the document was replaced wholesale).
    pub fn invalidate(&mut self) {
        self.cache.key = None;
    }

    /// A click or a drag on the strip: put the preview where the pointer is, and answer the root
    /// block it landed in (the caller's `on_jump(block_ix)`).
    ///
    /// The rectangle is centred on the pointer rather than pinned by its top edge — dragging a
    /// viewport rectangle means "show me here", and a top-edge grab makes the first pixel of every
    /// drag jump by half a screen.
    pub fn scrub(
        &mut self,
        doc: &Document,
        list: &ListState,
        m: &Metrics,
        position: Point<Pixels>,
    ) -> Option<usize> {
        let map = Map::build(doc, list, m, &mut self.cache);
        let projection = map.project(map.view_height, max_scale(m));
        // The strip's top edge is the list's: they are the two children of one flex row.
        let top = f32::from(list.viewport_bounds().origin.y);
        let strip_y = f32::from(position.y) - top;
        let doc_y = projection.doc_y(strip_y) - map.view_height / 2.0;
        let (ix, into) = locate(&map.tops, doc_y.max(0.0));
        let height = map.marks.get(ix)?.height;
        list.scroll_to(ListOffset {
            item_ix: ix,
            offset_in_item: px(into.clamp(0.0, height)),
        });
        Some(ix)
    }
}

/// The no-stretch maximum: the strip's drawn width over the text column's, so the miniature is the
/// page at the same aspect rather than a page stretched to the window.
fn max_scale(m: &Metrics) -> f32 {
    (WIDTH - 2.0 * PAD) / m.column.max(1.0)
}

impl Map {
    /// Place every block, measured where the list can say and estimated where it cannot.
    ///
    /// `bounds_for_item` gives *window* coordinates and only for the rows around the viewport. Two
    /// consecutive measured rows therefore give an exact top-to-top advance — which is what this
    /// accumulates, rather than absolute positions. The viewport is then anchored on the scroll
    /// top's row, the one row that is measured by definition.
    fn build(doc: &Document, list: &ListState, m: &Metrics, cache: &mut Cache) -> Map {
        let count = doc.blocks.len();
        let scroll = list.logical_scroll_top();
        let viewport = list.viewport_bounds();

        let key = (
            count,
            doc.blocks.first().map_or(0, |b| b.hash),
            m.column.round() as i32,
        );
        if cache.key != Some(key) {
            cache.key = Some(key);
            cache.advances = vec![None; count];
            cache.heights = vec![None; count];
        }

        // The measured window only: start at the scroll top and stop at the first `None` after it.
        let mut previous: Option<f32> = None;
        for ix in scroll.item_ix..count {
            let Some(bounds) = list.bounds_for_item(ix) else {
                if previous.is_some() {
                    break;
                }
                continue;
            };
            let top = f32::from(bounds.origin.y);
            cache.heights[ix] = Some(f32::from(bounds.size.height));
            if let Some(prev_top) = previous {
                cache.advances[ix - 1] = Some(top - prev_top);
            }
            previous = Some(top);
        }

        let mut marks = Vec::with_capacity(count);
        let mut tops = Vec::with_capacity(count);
        let mut cursor = 0.0_f32;
        for (ix, block) in doc.blocks.iter().enumerate() {
            let height = cache.heights[ix].unwrap_or_else(|| estimate(block, m));
            let advance = cache.advances[ix].unwrap_or(height + m.body * 0.7);
            tops.push(cursor);
            marks.push(Mark {
                top: cursor,
                height: height.max(1.0),
                shape: shape_of(block),
            });
            cursor += advance.max(height).max(1.0);
        }

        let view_height = f32::from(viewport.size.height).max(1.0);
        let view_top =
            tops.get(scroll.item_ix).copied().unwrap_or(0.0) + f32::from(scroll.offset_in_item);

        Map {
            height: cursor.max(1.0),
            marks,
            tops,
            view_top,
            view_height,
        }
    }
}

/// A row's height when the list has not measured it yet.
///
/// Every number here is a multiple of the body size, matching what `blocks::row` will actually
/// lay out — near enough that the strip's scale barely moves when the real measurement arrives.
fn estimate(block: &Block, m: &Metrics) -> f32 {
    let line = m.body * m.leading;
    let prose = |chars: usize| wrapped_lines(chars as f32, PROSE_COLUMNS).0 as f32 * line;

    match &block.kind {
        BlockKind::Heading { level, text, .. } => {
            let scale = match level {
                1 => 1.7,
                2 => 1.3,
                3 => 1.1,
                _ => 1.0,
            };
            let per_line = PROSE_COLUMNS / scale;
            wrapped_lines(text.chars().count() as f32, per_line).0 as f32 * m.body * scale * 1.2 * m.line_spacing
        }
        BlockKind::Code { body, .. } | BlockKind::FrontMatter { body, .. } => {
            (body.lines().count().max(1) as f32) * m.body * 0.9 * 1.35 + m.body
        }
        BlockKind::Table { rows, .. } => (rows.len() + 1) as f32 * m.body * 0.95 * 1.5,
        BlockKind::Image { .. } => m.body * 6.0,
        BlockKind::ThematicBreak => m.body * 0.5,
        // Everything else is prose or reads like it, and its source length is the only guide there
        // is before a measurement exists.
        _ => prose(block.range.len()),
    }
}

/// Which drawing stands for a block.
///
/// Only the last line's length comes from the text; how *many* lines are drawn is decided at paint
/// time from the height the row was measured at, so a measured paragraph draws the lines it really
/// has and an estimated one draws the lines it probably has.
fn shape_of(block: &Block) -> Shape {
    let chars = block.range.len() as f32;
    match &block.kind {
        BlockKind::Heading { level, .. } => Shape::Heading { level: *level },
        BlockKind::Code { .. } | BlockKind::FrontMatter { .. } => Shape::Code,
        BlockKind::Image { .. } => Shape::Figure,
        BlockKind::ThematicBreak => Shape::Rule,
        BlockKind::Table { head, rows, .. } => Shape::Table {
            rows: rows.len() + 1,
            columns: head.len().max(rows.first().map_or(1, Vec::len)).max(1),
        },
        BlockKind::Quote { .. } => Shape::Lines {
            last: wrapped_lines(chars, PROSE_COLUMNS).1,
            indent: 0.16,
            rule: true,
        },
        BlockKind::List { .. } => Shape::Lines {
            last: wrapped_lines(chars, PROSE_COLUMNS).1,
            indent: 0.12,
            rule: false,
        },
        _ => Shape::Lines {
            last: wrapped_lines(chars, PROSE_COLUMNS).1,
            indent: 0.0,
            rule: false,
        },
    }
}

// ── The element ─────────────────────────────────────────────────────

/// The strip, as wide as [`reserve`] said it costs — the gap is this element's left padding, so
/// the preview's own width arithmetic stays one subtraction.
///
/// `on_down` is handed the press; the caller marks the drag as started and calls
/// [`Minimap::scrub`] (see the module note).
pub fn strip(
    state: &mut Minimap,
    doc: &Document,
    list: &ListState,
    m: &Metrics,
    reserved: f32,
    on_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let painted = Map::build(doc, list, m, &mut state.cache);
    let max_scale = max_scale(m);

    div()
        .id("md-minimap")
        .flex()
        .flex_none()
        .w(px(reserved))
        .h_full()
        .pl(px((reserved - WIDTH).max(0.0)))
        .relative()
        .child(
            canvas(
                |_bounds, _window, _cx| (),
                move |bounds, (), window, _cx| {
                    // The projection is recomputed here: the strip's real height is a prepaint fact.
                    let projection = painted.project(f32::from(bounds.size.height), max_scale);
                    paint(&painted, projection, bounds, window);
                },
            )
            .size_full(),
        )
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            on_down(event, window, cx);
        })
        .into_any_element()
}

// ── The paint ───────────────────────────────────────────────────────

fn paint(map: &Map, p: Projection, bounds: Bounds<Pixels>, window: &mut Window) {
    let origin_x = f32::from(bounds.origin.x);
    let origin_y = f32::from(bounds.origin.y);
    let height = f32::from(bounds.size.height);
    let left = origin_x + PAD;
    let content = f32::from(bounds.size.width) - 2.0 * PAD;

    window.paint_quad(fill(bounds, theme::pane_bg()));

    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for mark in &map.marks {
            let y = p.y(mark.top);
            let h = p.h(mark.height);
            if y + h < -2.0 || y > height + 2.0 {
                continue;
            }
            draw(mark, origin_y + y, h, left, content, window);
        }

        // The viewport rectangle, over everything it covers.
        let view_y = origin_y + p.y(map.view_top);
        let view_h = p.h(map.view_height).max(3.0);
        window.paint_quad(
            fill(
                Bounds {
                    origin: point(px(origin_x), px(view_y)),
                    size: size(px(f32::from(bounds.size.width)), px(view_h)),
                },
                theme::accent_soft(),
            )
            .border_widths(px(1.0))
            .border_color(theme::accent()),
        );
    });
}

/// One mark. `y` and `h` are already in window coordinates.
fn draw(mark: &Mark, y: f32, h: f32, left: f32, content: f32, window: &mut Window) {
    let quad = |window: &mut Window, x: f32, y: f32, w: f32, h: f32, colour: gpui::Rgba| {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        window.paint_quad(fill(
            Bounds {
                origin: point(px(x), px(y)),
                size: size(px(w), px(h)),
            },
            colour,
        ));
    };

    match &mark.shape {
        Shape::Heading { level } => {
            // A bar whose length and weight follow the level: the document's skeleton, read
            // without reading a word of it.
            let (share, thickness, colour) = match level {
                1 => (0.34, 3.0, theme::markdown().minimap_heading),
                2 => (0.26, 2.5, theme::markdown().minimap_heading),
                3 => (0.18, 1.5, theme::markdown().minimap_heading_faint),
                _ => (0.14, 1.0, theme::markdown().minimap_heading_faint),
            };
            let bar = content * share;
            let bar_y = y + (h - thickness).max(0.0) / 2.0;
            quad(window, left, bar_y, bar, thickness, colour);
        }

        Shape::Lines { last, indent, rule } => {
            let x = left + content * indent;
            let w = content - content * indent;
            if *rule {
                quad(window, left, y, 1.0, h, theme::markdown().minimap_grid);
            }

            // How many lines the block's own height is worth. Derived from the height rather than
            // from the text, so a measured row draws the lines it really has.
            let pitch = LINE_PITCH;
            let count = (h / pitch).round().max(1.0) as usize;
            let pitch = h / count as f32;
            if pitch < 1.4 {
                // Too dense to resolve into lines: one block of tone is honest, a grey mush of
                // overlapping quads is not.
                quad(window, x, y, w * 0.98, h, theme::markdown().minimap_line);
                return;
            }
            let thickness = (pitch * 0.55).clamp(0.8, 2.0);
            for n in 0..count {
                let width = if n + 1 == count { w * last } else { w };
                quad(
                    window,
                    x,
                    y + n as f32 * pitch,
                    width,
                    thickness,
                    theme::markdown().minimap_line,
                );
            }
        }

        Shape::Code => {
            quad(
                window,
                left,
                y,
                content,
                h.max(2.0),
                theme::markdown().minimap_code,
            );
        }

        Shape::Table { rows, columns } => {
            quad(
                window,
                left,
                y,
                content,
                h.max(2.0),
                theme::markdown().minimap_code,
            );
            let row_pitch = h / (*rows).max(1) as f32;
            if row_pitch >= 2.0 {
                for n in 1..*rows {
                    quad(
                        window,
                        left,
                        y + n as f32 * row_pitch,
                        content,
                        1.0,
                        theme::markdown().minimap_grid,
                    );
                }
            }
            let column_pitch = content / (*columns).max(1) as f32;
            if column_pitch >= 4.0 {
                for n in 1..*columns {
                    quad(
                        window,
                        left + n as f32 * column_pitch,
                        y,
                        1.0,
                        h.max(2.0),
                        theme::markdown().minimap_grid,
                    );
                }
            }
        }

        Shape::Figure => {
            quad(
                window,
                left,
                y,
                content,
                h.max(3.0),
                theme::markdown().minimap_figure,
            );
        }

        Shape::Rule => {
            quad(
                window,
                left,
                y + h / 2.0,
                content,
                1.0,
                theme::markdown().minimap_grid,
            );
        }
    }
}

/// How far apart the strip draws two lines of prose. Fixed rather than scaled: below about a pixel
/// and a half a stack of lines stops being a stack, so the strip draws the number of lines that
/// *fit* at a legible pitch rather than the number the paragraph has.
const LINE_PITCH: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// A document shorter than the strip is drawn at the maximum scale and left alone — the
    /// no-stretch rule, which is the whole difference between "one page looks like one page" and
    /// "one page fills the window".
    #[test]
    fn a_short_document_is_not_stretched() {
        let p = Projection::fit(400.0, 800.0, 0.2, 0.0, 800.0);
        assert_eq!(p.scale, 0.2, "a short document should sit at the maximum");
        assert_eq!(p.offset, 0.0);
        // And it occupies its own fraction of the strip, not all of it.
        assert!((p.y(400.0) - 80.0).abs() < 1e-4);
    }

    /// A long document scales to fit, and the whole of it lands inside the strip.
    #[test]
    fn a_long_document_scales_to_fit() {
        let p = Projection::fit(8_000.0, 800.0, 0.2, 0.0, 800.0);
        assert!((p.scale - 0.1).abs() < 1e-4, "scale was {}", p.scale);
        assert_eq!(p.offset, 0.0);
        assert!((p.y(8_000.0) - 800.0).abs() < 1e-3);
    }

    /// Past the floor scale the strip stops shrinking and travels with the main view instead:
    /// at the top of the document its top, at the bottom its bottom.
    #[test]
    fn a_very_long_document_scrolls_the_strip() {
        let doc = 400_000.0;
        let strip = 800.0;
        let view = 800.0;

        let top = Projection::fit(doc, strip, 0.2, 0.0, view);
        assert_eq!(top.scale, MIN_SCALE, "the floor should hold");
        assert_eq!(top.offset, 0.0);

        let bottom = Projection::fit(doc, strip, 0.2, doc - view, view);
        assert_eq!(bottom.scale, MIN_SCALE);
        assert!((bottom.offset - (doc * MIN_SCALE - strip)).abs() < 1e-2);
        // The document's last pixel is at the strip's bottom edge, not past it.
        assert!((bottom.y(doc) - strip).abs() < 1e-2);
    }

    /// A zero-height document or a zero-height strip must not divide by zero on the way to a
    /// scale — the first frame, before anything has been measured, is exactly this case.
    #[test]
    fn an_empty_document_projects_without_dividing_by_zero() {
        for p in [
            Projection::fit(0.0, 800.0, 0.2, 0.0, 0.0),
            Projection::fit(800.0, 0.0, 0.2, 0.0, 0.0),
        ] {
            assert!(p.scale.is_finite() && p.scale > 0.0);
            assert!(p.y(0.0).is_finite());
            assert!(p.doc_y(0.0).is_finite());
        }
    }

    /// Strip y and document y are inverses, scrolled or not — the click-to-jump round trip.
    #[test]
    fn the_projection_inverts() {
        for p in [
            Projection::fit(400.0, 800.0, 0.2, 0.0, 800.0),
            Projection::fit(8_000.0, 800.0, 0.2, 2_000.0, 800.0),
            Projection::fit(400_000.0, 800.0, 0.2, 90_000.0, 800.0),
        ] {
            for doc_y in [0.0, 137.0, 399.0] {
                assert!((p.doc_y(p.y(doc_y)) - doc_y).abs() < 1e-2);
            }
        }
    }

    /// A strip y lands in the block that contains it, and the offset is measured from that block's
    /// own top — which is what `ListOffset` wants.
    #[test]
    fn a_strip_y_finds_its_block() {
        let tops = [0.0, 40.0, 100.0, 260.0];

        assert_eq!(locate(&tops, 0.0), (0, 0.0));
        assert_eq!(locate(&tops, 39.0), (0, 39.0));
        assert_eq!(locate(&tops, 40.0), (1, 0.0));
        assert_eq!(locate(&tops, 150.0), (2, 50.0));
        // Past the end clamps to the last block rather than wrapping or panicking.
        assert_eq!(locate(&tops, 9_999.0).0, 3);
        // Above the first, likewise.
        assert_eq!(locate(&tops, -50.0), (0, 0.0));
        assert_eq!(locate(&[], 10.0), (0, 0.0));
    }

    /// The reserve rule: the strip costs its width plus the gap, and gives up when what is left
    /// would take the text column under its minimum.
    #[test]
    fn the_strip_gives_up_before_the_prose_does() {
        let column = 525.0;
        let margin = 37.5;
        let body = 15.0;

        // A wide pane: reserved, and the cost is the width plus two em of gap.
        let wide = reserve(true, 1_200.0, column, margin, body).expect("wide pane keeps the strip");
        assert!((wide - (WIDTH + 2.0 * body)).abs() < 1e-4);
        // What is left still holds the column and both margins.
        assert!(1_200.0 - wide >= column + 2.0 * margin);

        // A narrow one: no strip, because the prose would pay for it.
        assert_eq!(reserve(true, 460.0, column, margin, body), None);
        // And asking for no strip is answered before any arithmetic.
        assert_eq!(reserve(false, 1_200.0, column, margin, body), None);
    }

    /// The reserve rule has no gap in it: there is exactly one width at which the strip appears.
    #[test]
    fn the_reserve_rule_is_monotonic() {
        let (column, margin, body) = (525.0, 37.5, 15.0);
        let mut seen_some = false;
        let mut width = 200.0;
        while width < 1_600.0 {
            match reserve(true, width, column, margin, body) {
                Some(_) => seen_some = true,
                None => assert!(
                    !seen_some,
                    "the strip came back after going away at {width}"
                ),
            }
            width += 5.0;
        }
        assert!(seen_some, "the strip never appeared");
    }

    /// Prose wraps to whole lines with a short last one — the only thing that makes a stack of
    /// bars read as a paragraph.
    #[test]
    fn prose_wraps_to_a_short_last_line() {
        let (count, last) = wrapped_lines(150.0, 75.0);
        assert_eq!(count, 2);
        assert!((last - 1.0).abs() < 1e-4);

        let (count, last) = wrapped_lines(160.0, 75.0);
        assert_eq!(count, 3);
        assert!(last < 0.25 || (last - 0.2).abs() < 1e-4, "last was {last}");

        // A short paragraph is one line, and an empty block still draws something.
        assert_eq!(wrapped_lines(10.0, 75.0).0, 1);
        assert_eq!(wrapped_lines(0.0, 75.0).0, 1);
        // Never a zero or negative pitch, whatever the caller passes.
        assert!(wrapped_lines(100.0, 0.0).0 >= 1);
    }
}
