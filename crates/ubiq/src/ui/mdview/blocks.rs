//! One root block, one row — the markdown view's renderer.
//!
//! The proposal's Part III is implemented here and nowhere else: the typographic scale (§12), the
//! measure and its leading (§13), the column's placement and its breakouts (§13.2, §14). Every
//! number below is a multiple of the body size held on [`Metrics`], so changing one point size
//! rescales the page and nothing else has to move (§12 principle 6).
//!
//! Two things are worth knowing before reading further.
//!
//! **Spacing is margin on the row, not a text property** (§18). A block's "space before" and
//! "space after" are literally `mt`/`mb` on its container — which is why per-element rhythm is a
//! table here rather than an impossibility.
//!
//! **Flex margins do not collapse**, so [`row`] collapses them by hand: a row's top margin is the
//! larger of its own space-before and the previous block's space-after. Without that, a paragraph
//! followed by an H2 would open 2.1em instead of the 1.4em §12 asks for, and every heading would
//! float in the middle of its own gap.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Div, Font, FontFeatures, FontStyle, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    TextRun, Window, WindowTextSystem, div, px, relative,
};
use gpui_component::{Icon, IconName, Size, Sizable as _};
use ubiq_md::{
    Admonition, Block, BlockKind, Cell, ColumnAlign, Document, FrontMatterStyle, ListItem,
};

use super::MdViewConfig;
use super::fences;
use super::inline::{self, Base};
use super::prose::{LinkHandler, Prose};
use super::search::{self, Find};
use crate::theme::{self, Family, MdDensity, Role};

/// The page's side margin, in em (§14.1's "min margin", 2–3em) — the same 2.5 the TextView
/// preview's `theme::md_min_margin` holds.
const MARGIN_EM: f32 = 2.5;

/// The widest measure the leading follows when the width preset caps nothing (`MdWidth::Full`):
/// past this, longer lines get no more leading.
const FULL_COLUMNS: f32 = 140.0;

/// How much tighter Compact's vertical rhythm is than Comfortable's (proposal §7).
const COMPACT_RHYTHM: f32 = 0.8;

/// How much Compact takes off the body leading — the same offset `theme::md_body_line_height`
/// applies to the TextView preview, so the two read alike.
const COMPACT_LEADING: f32 = 0.15;

/// The page's geometry and the prose colour, computed once per frame and handed to every row.
///
/// `Clone` and cheap on purpose: a quote renders its children with the same metrics and a
/// different prose colour, and that is one field's worth of change rather than a second type.
#[derive(Clone)]
pub struct Metrics {
    /// 1em, in pixels: the content family's body size times the reader's character scale.
    /// Everything else here is derived from it.
    pub body: f32,
    /// The text column's width in pixels: the width preset's measure × the body font's average
    /// advance — which is what makes "75 characters" mean 75 characters at any size or typeface
    /// (§13). Under `MdWidth::Full` there is no measure and the column is the breakout.
    pub column: f32,
    /// The page's side margin.
    pub margin: f32,
    /// The column's left edge, measured from the pane's. Centred when the pane can hold the column
    /// plus two margins, otherwise the margin itself (§14.1).
    pub inset: f32,
    /// How wide a breakout row may grow: everything from the column's left edge to the far margin
    /// (§13.2). Never narrower than the column.
    pub breakout: f32,
    /// The body's line height, as a multiple of the body size, interpolated from the measure that
    /// actually fits (§13, principle 4), and tightened under Compact.
    pub leading: f32,
    /// The multiplier on every block's space before and after: 1 for Comfortable, less for
    /// Compact.
    pub rhythm: f32,
    /// Padding under the last block, so the final section can be read at eye level (§14.2).
    pub tail: f32,
    /// The colour running prose is set in — the reader's text shade. A block quote renders its
    /// children with this changed and nothing else.
    pub prose: gpui::Rgba,
    /// The colour headings and table headers are set in — one shade stronger than `prose`
    /// ([`crate::state::editor::TextShade::heading_colour`]).
    pub heading: gpui::Rgba,
    /// The reader's row-spacing multiplier, already folded into `leading`; carried separately for
    /// headings, whose leading is their own table's.
    pub line_spacing: f32,
    /// What a click on a link in the prose does. `None` draws links without click targets.
    pub on_link: Option<LinkHandler>,
    /// The window's text system, carried so a row can shape text while it renders.
    ///
    /// A table's column widths are measured from the glyphs the cells actually draw, and [`row`]
    /// runs inside `list()`'s closure where there is no `Window` to ask — so the one thing that
    /// measures text travels with the metrics rather than through the signature.
    text: Arc<WindowTextSystem>,
}

/// Hand-written because [`WindowTextSystem`] is not `Debug`, and the geometry is the part worth
/// printing anyway.
impl fmt::Debug for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metrics")
            .field("body", &self.body)
            .field("column", &self.column)
            .field("margin", &self.margin)
            .field("inset", &self.inset)
            .field("breakout", &self.breakout)
            .field("leading", &self.leading)
            .field("rhythm", &self.rhythm)
            .field("tail", &self.tail)
            .field("prose", &self.prose)
            .field("heading", &self.heading)
            .finish_non_exhaustive()
    }
}

/// [`Metrics`]' geometry as plain numbers — everything but the text system, which is what makes it
/// testable without a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub body: f32,
    pub column: f32,
    pub margin: f32,
    pub inset: f32,
    pub breakout: f32,
    pub leading: f32,
    pub rhythm: f32,
    pub tail: f32,
}

impl Geometry {
    /// The page laid out in `available` pixels for `config`, with the body face's average advance
    /// at 1em given as a fraction of it (`advance_em`). `min_margin` floors the side margin — the
    /// annotation stack's `theme::MD_ACTION_MARGIN` while annotating, `0` otherwise.
    pub fn compute(
        config: &MdViewConfig,
        system_body: f32,
        advance_em: f32,
        available: f32,
        viewport_height: f32,
        min_margin: f32,
    ) -> Geometry {
        let body = system_body * config.reading.char_scale;
        let advance = (body * advance_em).max(1.0);
        let margin = (MARGIN_EM * body).max(min_margin);
        let measure = config.md_width.measure_ch();

        let (column, inset) = match measure {
            Some(ch) => {
                let column = ch * advance;
                let inset = if available >= column + 2.0 * margin {
                    (available - column) / 2.0
                } else {
                    margin
                };
                (column, inset)
            }
            // Full: no measure, so the column is everything between the two margins.
            None => ((available - 2.0 * margin).max(0.0), margin),
        };
        let breakout = (available - inset - margin).max(column);

        // How many characters actually fit, which is what the leading follows — not the column we
        // asked for. A narrow pane gets tighter leading rather than the same leading over a
        // shorter line.
        let cap = measure.unwrap_or(FULL_COLUMNS);
        let fitted = ((available - 2.0 * margin) / advance).clamp(30.0, cap);
        let (leading, rhythm) = match config.md_density {
            MdDensity::Comfortable => (leading_for(fitted), 1.0),
            MdDensity::Compact => (leading_for(fitted) - COMPACT_LEADING, COMPACT_RHYTHM),
        };
        // The reader's own spacing, laid over density: both multiply what density settled on, so
        // `1.0` is today's page and every height derived from `leading`/`rhythm` follows.
        let leading = leading * config.reading.line_spacing;
        let rhythm = rhythm * config.reading.paragraph_spacing;

        Geometry {
            body,
            column,
            margin,
            inset,
            breakout,
            leading,
            rhythm,
            tail: (viewport_height / 3.0).max(body * 4.0),
        }
    }
}

impl Metrics {
    /// Measure the page against the space it has, for the view's config. `min_margin` as
    /// [`Geometry::compute`]'s.
    pub fn measure(
        window: &Window,
        config: &MdViewConfig,
        available: f32,
        viewport_height: f32,
        min_margin: f32,
    ) -> Metrics {
        let text = window.text_system().clone();
        let system_body = f32::from(theme::font(Family::Content, Role::Body));
        let body = system_body * config.reading.char_scale;
        let advance_em = average_advance(&text, body) / body.max(1.0);
        let g = Geometry::compute(
            config,
            system_body,
            advance_em,
            available,
            viewport_height,
            min_margin,
        );

        Metrics {
            body: g.body,
            column: g.column,
            margin: g.margin,
            inset: g.inset,
            breakout: g.breakout,
            leading: g.leading,
            rhythm: g.rhythm,
            tail: g.tail,
            prose: config.reading.text_shade.colour(),
            heading: config.reading.text_shade.heading_colour(),
            line_spacing: config.reading.line_spacing,
            on_link: None,
            text,
        }
    }

    /// The same metrics, with links that answer a click.
    pub fn with_link_handler(mut self, on_link: LinkHandler) -> Metrics {
        self.on_link = Some(on_link);
        self
    }

    fn em(&self, n: f32) -> gpui::Pixels {
        px(self.body * n)
    }

    /// A rhythm gap — space before or after a block — in pixels, density applied.
    fn gap(&self, n: f32) -> gpui::Pixels {
        px(self.body * n * self.rhythm)
    }

    /// The width one line of body text is drawn at, shaped.
    fn shaped_width(&self, line: &str, size: f32, weight: FontWeight) -> f32 {
        shaped_width(&self.text, line, size, weight)
    }
}

/// §13's measure-to-leading table, interpolated rather than stepped so a resize never jumps the
/// leading: 65 characters reads at 1.45, 100 at 1.6, and everything between is linear.
fn leading_for(columns: f32) -> f32 {
    (1.45 + (columns - 65.0) * (0.15 / 35.0)).clamp(1.45, 1.6)
}

/// One run of body text at `weight` — the shape of every measurement this module makes.
fn body_run(len: usize, weight: FontWeight) -> TextRun {
    TextRun {
        len,
        font: Font {
            family: theme::BODY_FONT.into(),
            features: FontFeatures::default(),
            fallbacks: None,
            weight,
            style: FontStyle::Normal,
        },
        color: theme::text().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// The width the text system gives one line of body text — the real advance of the real glyphs.
///
/// `shape_line` panics on a newline, so a caller with multi-line text hands it one line at a time.
fn shaped_width(text: &WindowTextSystem, line: &str, size: f32, weight: FontWeight) -> f32 {
    if line.is_empty() {
        return 0.0;
    }
    let run = body_run(line.len(), weight);
    let shaped = text.shape_line(SharedString::from(line.to_string()), px(size), &[run], None);
    f32::from(shaped.width)
}

/// The body font's average advance at the current size, shaped for real (§18).
///
/// A sample of mixed case, digits and punctuation rather than a single `x`: a proportional face's
/// average is what sets the measure, and one glyph is not it.
fn average_advance(text: &WindowTextSystem, body: f32) -> f32 {
    const SAMPLE: &str =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 ,.;:-()'\"";

    let width = shaped_width(text, SAMPLE, body, FontWeight::NORMAL);
    if width > 0.0 {
        width / SAMPLE.chars().count() as f32
    } else {
        // The text system has not resolved the family yet. 0.5em is the usual average for a
        // proportional UI face and is only ever the first frame's estimate.
        body * 0.5
    }
}

// ── §12: the typographic scale ──────────────────────────────────────

/// A block's vertical rhythm, in em. `before` is always the larger of the two on a heading —
/// §11 principle 3, headings belong to what follows them.
#[derive(Clone, Copy, Debug)]
struct Rhythm {
    before: f32,
    after: f32,
}

fn rhythm(kind: &BlockKind) -> Rhythm {
    let (before, after) = match kind {
        BlockKind::Heading { level, .. } => match level {
            1 => (0.0, 0.6),
            2 => (1.4, 0.35),
            3 => (1.1, 0.25),
            // §12 stops at H3. Below it the taper continues on the same shape rather than
            // flattening, so an H4 still reads as a break and not as a bold paragraph.
            _ => (0.9, 0.2),
        },
        BlockKind::Paragraph | BlockKind::Html { .. } | BlockKind::Other => (0.0, 0.7),
        BlockKind::List { .. } => (0.0, 0.7),
        // §12 gives the code block 0.6em before and 0.8em after; a table is the other breakout and
        // takes the same, which §12 leaves unsaid.
        BlockKind::Code { .. } | BlockKind::Table { .. } | BlockKind::Image { .. } => (0.6, 0.8),
        BlockKind::Quote { .. } => (0.6, 0.7),
        BlockKind::ThematicBreak => (1.2, 1.2),
        BlockKind::FrontMatter { .. } => (0.0, 0.9),
        BlockKind::FootnoteDefinition { .. } => (0.0, 0.4),
    };
    Rhythm { before, after }
}

/// A heading's size, leading and weight. The size is the TextView preview's own ratio
/// (`theme::md_heading_ratio`), so the two previews agree on the scale; the leading and weight are
/// §12's table.
fn heading_scale(level: u8) -> (f32, f32, FontWeight) {
    let (lead, weight) = match level {
        1 => (1.15, FontWeight::BOLD),
        2 => (1.2, FontWeight::BOLD),
        3 => (1.25, FontWeight::SEMIBOLD),
        _ => (1.3, FontWeight::SEMIBOLD),
    };
    (theme::md_heading_ratio(level), lead, weight)
}

// ── The row ─────────────────────────────────────────────────────────

/// One `list()` row: the block at root index `ix`, with its rhythm and the page's inset.
///
/// **Only a root block's `ix` addresses a row** — `Block::ix` is sibling-local, so a block nested
/// in a list item or a quote has an `ix` that means nothing here. That is why this takes the
/// document and the index rather than a `&Block`.
///
/// `front` is the front-matter row's disclosure: whether it is open and what a click on its header does.
pub fn row(
    doc: &Document,
    ix: usize,
    m: &Metrics,
    find: Option<&Find>,
    front: Option<FrontToggle>,
) -> AnyElement {
    let Some(block) = doc.blocks.get(ix) else {
        return div().into_any_element();
    };

    let here = rhythm(&block.kind);
    let above = match ix.checked_sub(1).and_then(|p| doc.blocks.get(p)) {
        // The collapse: two margins that meet are one margin, the larger.
        Some(previous) => here.before.max(rhythm(&previous.kind).after),
        // §14.2: about one body line above the first block, and none of today's dead space.
        None => 1.0,
    };
    let last = ix + 1 == doc.blocks.len();
    // The row's search hits, published for as long as this row is being built — `inline::runs`
    // asks for them rather than taking them through a parameter on every block kind.
    let _hits = search::scope(find, ix);

    div()
        .w_full()
        .pl(px(m.inset))
        .pr(px(m.margin))
        .mt(m.gap(above))
        .when(last, |this| this.mb(px(m.tail)))
        .child(match (&block.kind, front) {
            (BlockKind::FrontMatter { style, body }, Some(front)) => {
                front_matter(*style, body, m, true, front)
            }
            _ => content(block, m, &doc.source, true),
        })
        .into_any_element()
}

/// The front matter's disclosure: open or not, and the header's click.
pub struct FrontToggle {
    pub open: bool,
    pub on_click: Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static>,
}

/// A block's own element. `capped` is set for a root block, which owns the page's width rules; a
/// nested block is already inside a container that applied them.
///
/// The two breakout kinds that scroll sideways — a code fence and a table — take `block.range.start`
/// as their element id. GPUI keys a scroll container's offset by the element id path and `list()`
/// does not scope its rows, so the id has to be stable across frames and unique across the whole
/// document: a block's source offset is both, and a per-frame counter would be neither.
fn content(block: &Block, m: &Metrics, src: &str, capped: bool) -> AnyElement {
    let at = block.range.start;
    match &block.kind {
        BlockKind::Heading { level, .. } => heading(block, *level, m, capped),
        BlockKind::Paragraph => paragraph(block, m, capped),
        BlockKind::Image { dest, alt, .. } => wide(m, capped)
            .child(fences::image(dest, alt))
            .into_any_element(),
        BlockKind::List {
            ordered,
            start,
            tight,
            items,
        } => list(*ordered, *start, *tight, items, m, src, capped),
        BlockKind::Code { language, body, .. } => {
            match fences::fence(language.as_deref(), body, src) {
                Some(picture) => wide(m, capped).child(picture).into_any_element(),
                None => code(language.as_deref(), body, m, capped, at),
            }
        }
        BlockKind::Quote { admonition, blocks } => quote(*admonition, blocks, m, src, capped),
        BlockKind::Table {
            alignments,
            head,
            rows,
        } => table(alignments, head, rows, m, capped, at),
        BlockKind::ThematicBreak => thematic_break(m, capped),
        BlockKind::FrontMatter { style, body } => front_matter_body(*style, body, m, capped),
        BlockKind::Html { body } => html(body, m, capped),
        BlockKind::FootnoteDefinition { label, blocks } => footnote(label, blocks, m, src, capped),
        // Nothing vanishes: a kind the engine does not model draws the bytes it came from.
        BlockKind::Other => other(src, &block.range, m, capped),
    }
}

/// The container for prose: capped at the measure when it is a root block.
///
/// Named `column` rather than `prose` so the module of that name — the native text element — can
/// be reached from here without qualification.
fn column(m: &Metrics, capped: bool) -> Div {
    div()
        .w_full()
        .min_w(px(0.))
        .when(capped, |this| this.max_w(px(m.column)))
}

/// The container for a breakout (§13.2): aligned to the column's left edge, free to grow right.
fn wide(m: &Metrics, capped: bool) -> Div {
    div()
        .w_full()
        .min_w(px(0.))
        .when(capped, |this| this.max_w(px(m.breakout)))
}

/// A run of nested blocks — a quote's contents, a list item's, a footnote's — with the same
/// collapsed rhythm the root list applies between rows.
fn stack(blocks: &[Block], m: &Metrics, src: &str) -> Div {
    let mut column = div().flex().flex_col().w_full().min_w(px(0.));
    let mut previous_after = 0.0_f32;
    for block in blocks {
        let here = rhythm(&block.kind);
        let top = here.before.max(previous_after);
        column = column.child(
            div()
                .w_full()
                .min_w(px(0.))
                .mt(m.gap(top))
                .child(content(block, m, src, false)),
        );
        previous_after = here.after;
    }
    column
}

// ── The block kinds ─────────────────────────────────────────────────

fn heading(block: &Block, level: u8, m: &Metrics, capped: bool) -> AnyElement {
    let (scale, lead, weight) = heading_scale(level);
    let size = m.body * scale;

    column(m, capped)
        .text_size(px(size))
        .line_height(px(size * lead * m.line_spacing))
        .child(text(
            &block.spans,
            &Base::heading(weight, m.heading),
            size,
            size * lead * m.line_spacing,
            m,
        ))
        .into_any_element()
}

fn paragraph(block: &Block, m: &Metrics, capped: bool) -> AnyElement {
    column(m, capped)
        .text_size(px(m.body))
        .line_height(px(m.body * m.leading))
        .child(text(
            &block.spans,
            &Base::coloured(m.prose),
            m.body,
            m.body * m.leading,
            m,
        ))
        .into_any_element()
}

/// One block's prose, through the native element (item 14), its links answering the metrics'
/// click handler.
///
/// **Prose only.** A heading, a paragraph, and therefore every list item's, quote's and footnote's
/// text, because those are paragraphs inside a container. A code fence, a table cell, front matter,
/// raw HTML and `BlockKind::Other` still go through `StyledText`: they are monospace or
/// grid-shaped, one size throughout, and per-run sizing buys them nothing. Converting them is a
/// separate pass, which is the point of an element that can land per block kind.
///
/// `size` is the block's 1em and `line_height` its leading, in pixels.
fn text(
    spans: &[(Range<usize>, ubiq_md::Inline)],
    base: &Base,
    size: f32,
    line_height: f32,
    m: &Metrics,
) -> AnyElement {
    let out = inline::prose_runs(spans, base);
    Prose::new(
        out.text,
        out.runs,
        out.highlights,
        px(size),
        px(line_height),
    )
    .links(out.links, m.on_link.clone())
    .into_any_element()
}

/// A code fence: monospace at 0.9em over a tinted quad, with the info string's language named in
/// the corner when the author gave one.
///
/// **The body scrolls sideways and never wraps** (item 15). A folded line of source is a lie about
/// the file, so the long line runs off the edge and the reader scrolls to it. Three things make
/// that safe inside a `list()` row:
///
/// - `whitespace_nowrap` is what turns wrapping off: with it the text element measures its natural
///   width instead of the container's, which is also what gives the scroll container something to
///   scroll.
/// - The row's measured height cannot move. GPUI's `overflow_x_scroll` draws no scrollbar and
///   reserves no gutter, so the row is as tall as its lines and stays that tall while it scrolls —
///   a `ListState` remeasure per scroll tick is exactly the thing to avoid here. A visible
///   scrollbar, if one is ever wanted, has to be an absolute overlay for the same reason.
/// - `restrict_scroll_to_axis` keeps the page scrolling. Without it GPUI folds a vertical wheel
///   delta onto x for an x-only scroller, so a tick over a code fence would slide the code
///   sideways instead of moving the document: vertical stays the list's job.
fn code(language: Option<&str>, body: &str, m: &Metrics, capped: bool, at: usize) -> AnyElement {
    let size = m.body * 0.9;

    wide(m, capped)
        .flex()
        .flex_col()
        .bg(theme::markdown().code_bg)
        .border_1()
        .border_color(theme::border())
        .when_some(language, |this, language| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .px_3()
                    .pt_1()
                    .font_family(theme::MONO_FONT)
                    .text_size(theme::font(Family::Chrome, Role::Label))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(language.to_string())),
            )
        })
        .child(
            div()
                .id(("mdview-code-scroll", at))
                .w_full()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .whitespace_nowrap()
                .px_3()
                .py_2()
                .text_size(px(size))
                .line_height(px(size * theme::MD_CODE_LINE_HEIGHT))
                .child(inline::literal(
                    body.trim_end_matches('\n').to_string(),
                    &Base::mono(theme::text()),
                )),
        )
        .into_any_element()
}

/// A list. Nesting needs no special case: a nested list is a `List` block inside an item's
/// `blocks`, and [`stack`] recurses into it.
fn list(
    ordered: bool,
    start: Option<u64>,
    tight: bool,
    items: &[ListItem],
    m: &Metrics,
    src: &str,
    capped: bool,
) -> AnyElement {
    // §12 gives a list item 0.25em after; a loose list doubles it, which is what "loose" means.
    let gap = if tight { 0.25 } else { 0.5 };

    column(m, capped)
        .flex()
        .flex_col()
        .children(items.iter().enumerate().map(|(n, item)| {
            let marker = match (item.checked, ordered) {
                (Some(true), _) => "☑".to_string(),
                (Some(false), _) => "☐".to_string(),
                (None, true) => format!("{}.", start.unwrap_or(1) + n as u64),
                (None, false) => "•".to_string(),
            };

            div()
                .flex()
                .flex_row()
                .items_start()
                .w_full()
                .min_w(px(0.))
                .when(n > 0, |this| this.mt(m.gap(gap)))
                .child(
                    div()
                        .flex_none()
                        .w(m.em(1.7))
                        .pr(m.em(0.45))
                        .text_right()
                        .text_size(px(m.body))
                        .line_height(px(m.body * 1.45))
                        .text_color(theme::markdown().marker)
                        .child(SharedString::from(marker)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(m.body))
                        .line_height(px(m.body * 1.45))
                        .child(stack(&item.blocks, m, src)),
                )
        }))
        .into_any_element()
}

/// A block quote: a left rule and an indent, with an admonition's own colour and label when the
/// engine found one.
fn quote(
    admonition: Option<Admonition>,
    blocks: &[Block],
    m: &Metrics,
    src: &str,
    capped: bool,
) -> AnyElement {
    let rule = admonition.map_or(theme::markdown().quote_rule, admonition_colour);

    // The one field a quote changes: its prose is a shade back from the page's.
    let mut inner = m.clone();
    inner.prose = theme::markdown().quote_text;

    column(m, capped)
        .flex()
        .flex_row()
        .child(div().flex_none().w(px(theme::accent_edge())).bg(rule))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .pl(m.em(0.9))
                .when_some(admonition, |this, kind| {
                    this.child(
                        div()
                            .text_size(px(m.body * 0.85))
                            .line_height(px(m.body * 1.2))
                            .text_color(admonition_colour(kind))
                            .child(SharedString::from(admonition_label(kind))),
                    )
                })
                .child(stack(blocks, &inner, src)),
        )
        .into_any_element()
}

fn admonition_colour(kind: Admonition) -> gpui::Rgba {
    match kind {
        Admonition::Note => theme::info(),
        Admonition::Tip => theme::success(),
        Admonition::Important => theme::accent(),
        Admonition::Warning => theme::warning(),
        Admonition::Caution => theme::danger(),
    }
}

fn admonition_label(kind: Admonition) -> &'static str {
    match kind {
        Admonition::Note => "NOTE",
        Admonition::Tip => "TIP",
        Admonition::Important => "IMPORTANT",
        Admonition::Warning => "WARNING",
        Admonition::Caution => "CAUTION",
    }
}

/// A table: a real grid, column widths measured from the glyphs the cells draw, and every column's
/// `ColumnAlign` honoured. A breakout (§13.2) that scrolls sideways rather than squeezing when the
/// columns want more room than the row has (item 15).
fn table(
    alignments: &[ColumnAlign],
    head: &[Cell],
    rows: &[Vec<Cell>],
    m: &Metrics,
    capped: bool,
    at: usize,
) -> AnyElement {
    let columns = alignments
        .len()
        .max(head.len())
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return div().into_any_element();
    }
    let grid = Grid::measure(m, head, rows, columns);

    wide(m, capped)
        .flex()
        .flex_col()
        .overflow_hidden()
        .border_1()
        .border_color(theme::border())
        .child(
            // The scroll container sits inside the border, so the frame stays put and the grid
            // slides within it. Horizontal only, and axis-restricted, for the reasons [`code`]
            // spells out: a wheel tick over a table still scrolls the document.
            div()
                .id(("mdview-table-scroll", at))
                .w_full()
                .flex()
                .flex_col()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .child(
                    table_row(head, alignments, &grid, m, true)
                        .bg(theme::markdown().table_head)
                        .border_b_1()
                        .border_color(theme::border()),
                )
                .children(rows.iter().enumerate().map(|(n, cells)| {
                    table_row(cells, alignments, &grid, m, false).when(n + 1 < rows.len(), |this| {
                        this.border_b_1().border_color(theme::border())
                    })
                })),
        )
        .into_any_element()
}

fn table_row(
    cells: &[Cell],
    alignments: &[ColumnAlign],
    grid: &Grid,
    m: &Metrics,
    header: bool,
) -> Div {
    let size = m.body * 0.95;

    div()
        .flex()
        .flex_row()
        // A scrolling grid is as wide as its columns; a fitting one is as wide as the row.
        .map(|this| match grid.fixed {
            Some(total) => this.flex_none().w(px(total)),
            None => this.w_full(),
        })
        .children(grid.columns.iter().enumerate().map(|(column, spec)| {
            let align = alignments
                .get(column)
                .copied()
                .unwrap_or(ColumnAlign::Default);
            let base = if header {
                Base::heading(FontWeight::SEMIBOLD, m.heading)
            } else {
                Base::coloured(m.prose)
            };

            div()
                .map(|this| match grid.fixed {
                    Some(_) => this.flex_none().w(px(spec.width)),
                    None => this.w(relative(spec.fraction)).min_w(px(0.)),
                })
                // Off only where the column is wide enough to hold its own text: a column pegged
                // at the row's width keeps wrapping, so nothing is clipped and no cell overdraws
                // its neighbour.
                .when(grid.fixed.is_some() && !spec.wraps, |this| {
                    this.whitespace_nowrap()
                })
                .px_2()
                .py_1()
                .text_size(px(size))
                .line_height(px(size * theme::MD_CODE_LINE_HEIGHT))
                .map(|this| match align {
                    ColumnAlign::Center => this.text_center(),
                    ColumnAlign::Right => this.text_right(),
                    _ => this.text_left(),
                })
                .when(column + 1 < grid.columns.len(), |this| {
                    this.border_r_1().border_color(theme::border())
                })
                .child(match cells.get(column) {
                    Some(cell) => inline::styled(&cell.spans, &base).into_any_element(),
                    None => div().into_any_element(),
                })
        }))
}

/// One column, as the row will draw it.
#[derive(Clone, Copy, Debug)]
struct Column {
    /// The column's share of the row, when the grid fits it.
    fraction: f32,
    /// The column's width in pixels, when the grid is wider than the row and scrolls.
    width: f32,
    /// Set when the column's widest cell wants more than the whole row: that column is pegged at
    /// the row's width and keeps wrapping inside it.
    wraps: bool,
}

/// A table's columns and which of the two layouts they are in.
#[derive(Clone, Debug)]
struct Grid {
    columns: Vec<Column>,
    /// The pixel width the rows are drawn at when the grid scrolls; `None` when it fits and the
    /// columns share the row as fractions, which is the common case and unchanged from before.
    fixed: Option<f32>,
}

/// How many body rows are shaped for the column measure.
///
/// The widest cell of a column is almost always the header or near the top, and shaping every cell
/// of a thousand-row table on every frame would cost far more than the widths are worth — so the
/// header plus this many rows is the sample. A freak wide cell below the sample wraps in its
/// column instead of widening it, which is the right failure.
const SAMPLED_ROWS: usize = 32;

impl Grid {
    fn measure(m: &Metrics, head: &[Cell], rows: &[Vec<Cell>], columns: usize) -> Grid {
        let wanted = wanted_widths(m, head, rows, columns);
        // A column never grows past the row: past that its text would have nowhere to go but over
        // its neighbour, so the column takes the row's width and wraps.
        let available = m.breakout;
        let capped: Vec<f32> = wanted.iter().map(|w| w.min(available)).collect();
        let total: f32 = capped.iter().sum();

        if total <= available {
            let fractions = column_weights(&capped);
            Grid {
                columns: fractions
                    .into_iter()
                    .map(|fraction| Column {
                        fraction,
                        width: 0.0,
                        wraps: false,
                    })
                    .collect(),
                fixed: None,
            }
        } else {
            Grid {
                columns: capped
                    .iter()
                    .zip(&wanted)
                    .map(|(&width, &want)| Column {
                        fraction: 0.0,
                        width,
                        wraps: want > width,
                    })
                    .collect(),
                fixed: Some(total),
            }
        }
    }
}

/// The pixel width each column's widest sampled cell wants, chrome included.
///
/// Shaped rather than counted (item 16): `chars().count()` gives a column of Chinese text half the
/// width it needs, an emoji a third, and proportional digits something between — and the error is
/// a factor, not a rounding. The text system already shapes this text to draw it.
fn wanted_widths(m: &Metrics, head: &[Cell], rows: &[Vec<Cell>], columns: usize) -> Vec<f32> {
    /// A cell's chrome: `px_2` on each side at the default rem, plus the column's divider.
    const CHROME: f32 = 2.0 * 8.0 + 1.0;

    let size = m.body * 0.95;
    let mut wanted = vec![0.0_f32; columns];
    let mut note = |cells: &[Cell], weight: FontWeight| {
        for (column, cell) in cells.iter().enumerate().take(columns) {
            // The text the cell actually draws, from the same walk that draws it — a cell's source
            // range is no guide (the plan's gap 5: it includes the spaces around the cell).
            let text = inline::runs(&cell.spans, &Base::body()).0;
            let width = text
                // A hard break makes a cell two lines; `shape_line` takes one at a time and the
                // column needs the widest of them.
                .split('\n')
                .map(|line| m.shaped_width(line, size, weight))
                .fold(0.0_f32, f32::max);
            wanted[column] = wanted[column].max(width + CHROME);
        }
    };
    note(head, FontWeight::SEMIBOLD);
    for row in rows.iter().take(SAMPLED_ROWS) {
        note(row, FontWeight::NORMAL);
    }
    wanted
}

/// Column widths as fractions of the row, from the width each column wants — so a five-column
/// table with one prose column does not give it a fifth of the width.
///
/// Plain numbers in, plain numbers out: the weighting is the part worth testing, and it does not
/// need a window to run.
fn column_weights(wanted: &[f32]) -> Vec<f32> {
    if wanted.is_empty() {
        return Vec::new();
    }

    // Clamped before normalising: an outlier cell must widen its column, not take the table.
    let mut weights: Vec<f32> = wanted.iter().map(|w| w.max(1.0)).collect();
    let mean = weights.iter().sum::<f32>() / weights.len() as f32;
    for width in &mut weights {
        *width = width.clamp(mean * 0.35, mean * 3.0);
    }
    let total: f32 = weights.iter().sum();
    weights.into_iter().map(|w| w / total).collect()
}

fn thematic_break(m: &Metrics, capped: bool) -> AnyElement {
    column(m, capped)
        .flex()
        .items_center()
        .h(m.em(0.5))
        .child(div().w_full().h(px(theme::hairline())).bg(theme::border()))
        .into_any_element()
}

/// Front matter is metadata, not prose (§ the engine's own note): collapsed by default to one
/// row — chevron, label, key count — and opened on click to a dimmed monospace panel that reads as
/// a header, never as a paragraph.
fn front_matter(
    style: FrontMatterStyle,
    body: &str,
    m: &Metrics,
    capped: bool,
    front: FrontToggle,
) -> AnyElement {
    let fields = body
        .lines()
        .filter(|l| {
            l.chars().next().is_some_and(|c| !c.is_whitespace() && !"#-]}".contains(c))
                && l.contains(match style {
                    FrontMatterStyle::Yaml => ':',
                    FrontMatterStyle::Pluses => '=',
                })
        })
        .count();
    let header = div()
        .id("md-front-matter")
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .text_size(theme::font(Family::Chrome, Role::Label))
        .text_color(theme::text_faint())
        .child(
            Icon::new(if front.open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(Size::Size(theme::icon_sm()))
            .text_color(theme::text_muted()),
        )
        .child("Front matter")
        .when(fields > 0, |this| {
            this.child(format!(
                "\u{b7} {fields} field{}",
                if fields == 1 { "" } else { "s" }
            ))
        })
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            (front.on_click)(event, window, cx);
        });

    let open = front.open;
    wide(m, capped)
        .flex()
        .flex_col()
        .child(header)
        .when(open, |this| {
            this.child(div().pt_1().child(front_matter_body(style, body, m, false)))
        })
        .into_any_element()
}

/// The expanded panel itself.
fn front_matter_body(style: FrontMatterStyle, body: &str, m: &Metrics, capped: bool) -> AnyElement {
    let size = m.body * 0.85;
    let label = match style {
        FrontMatterStyle::Yaml => "front matter — yaml",
        FrontMatterStyle::Pluses => "front matter — toml",
    };

    wide(m, capped)
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .bg(theme::markdown().gutter)
        .border_1()
        .border_color(theme::border())
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_faint())
                .child(label),
        )
        .child(
            div()
                .text_size(px(size))
                .line_height(px(size * theme::MD_CODE_LINE_HEIGHT))
                .child(inline::literal(
                    body.trim_end_matches('\n').to_string(),
                    &Base::mono(theme::text_muted()),
                )),
        )
        .into_any_element()
}

/// Raw HTML: shown as what it is, dimmed, never interpreted.
fn html(body: &str, m: &Metrics, capped: bool) -> AnyElement {
    let size = m.body * 0.85;

    wide(m, capped)
        .text_size(px(size))
        .line_height(px(size * theme::MD_CODE_LINE_HEIGHT))
        .child(inline::literal(
            body.trim_end_matches('\n').to_string(),
            &Base::mono(theme::text_faint()),
        ))
        .into_any_element()
}

fn footnote(label: &str, blocks: &[Block], m: &Metrics, src: &str, capped: bool) -> AnyElement {
    let size = m.body * 0.9;

    column(m, capped)
        .flex()
        .flex_row()
        .items_start()
        .text_size(px(size))
        .line_height(px(size * 1.4))
        .child(
            div()
                .flex_none()
                .pr(m.em(0.5))
                .text_color(theme::link_underline())
                .child(SharedString::from(format!("[^{label}]"))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(stack(blocks, m, src)),
        )
        .into_any_element()
}

/// A kind the engine does not model yet, drawn as the source it came from. A block that renders as
/// nothing is indistinguishable from a block that was lost.
fn other(src: &str, range: &Range<usize>, m: &Metrics, capped: bool) -> AnyElement {
    let text = src.get(range.clone()).unwrap_or_default().trim_end();

    column(m, capped)
        .text_size(px(m.body))
        .line_height(px(m.body * m.leading))
        .child(inline::literal(text.to_string(), &Base::coloured(m.prose)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::editor::MdReading;
    use crate::theme::MdWidth;

    /// A 13px system body, a 0.5em average advance — round numbers for the geometry below.
    fn page(config: MdViewConfig, available: f32) -> Geometry {
        Geometry::compute(&config, 13.0, 0.5, available, 900.0, 0.0)
    }

    /// Annotating floors the margin at the action stack's width; a margin already wider keeps
    /// its own.
    #[test]
    fn the_action_margin_floors_the_side_margin() {
        let mut small = config(MdWidth::Full, MdDensity::Comfortable);
        small.reading.char_scale = 0.5;
        let floored = Geometry::compute(&small, 13.0, 0.5, 1200.0, 900.0, theme::MD_ACTION_MARGIN);
        assert_eq!(floored.margin, theme::MD_ACTION_MARGIN);
        assert_eq!(floored.inset, theme::MD_ACTION_MARGIN);

        let wide = config(MdWidth::Full, MdDensity::Comfortable);
        let kept = Geometry::compute(&wide, 40.0, 0.5, 1200.0, 900.0, theme::MD_ACTION_MARGIN);
        assert_eq!(kept.margin, MARGIN_EM * 40.0 * wide.reading.char_scale);
    }

    fn config(md_width: MdWidth, md_density: MdDensity) -> MdViewConfig {
        MdViewConfig {
            md_width,
            md_density,
            ..MdViewConfig::default()
        }
    }

    /// The width preset sets the measure in characters: Readable is 75 of them, Wide 95, and the
    /// column is centred in a pane that can hold it plus two margins.
    #[test]
    fn the_width_preset_sets_the_measure() {
        let readable = page(config(MdWidth::Readable, MdDensity::Comfortable), 2000.0);
        assert!((readable.column - 75.0 * 6.5).abs() < 1e-3);
        assert!((readable.inset - (2000.0 - readable.column) / 2.0).abs() < 1e-3);

        let wide = page(config(MdWidth::Wide, MdDensity::Comfortable), 2000.0);
        assert!((wide.column - 95.0 * 6.5).abs() < 1e-3);
        assert!(wide.breakout >= wide.column);
    }

    /// Full caps nothing: the column is everything between the margins and is its own breakout.
    #[test]
    fn full_width_is_the_pane_less_its_margins() {
        let full = page(config(MdWidth::Full, MdDensity::Comfortable), 1200.0);
        assert_eq!(full.inset, full.margin);
        assert!((full.column - (1200.0 - 2.0 * full.margin)).abs() < 1e-3);
        assert!((full.breakout - full.column).abs() < 1e-3);
    }

    /// A pane too narrow for the measure gives the column the margin as its inset, not a centring
    /// that would push it off the left edge.
    #[test]
    fn a_narrow_pane_insets_by_the_margin() {
        let narrow = page(config(MdWidth::Readable, MdDensity::Comfortable), 300.0);
        assert_eq!(narrow.inset, narrow.margin);
        assert!(
            narrow.leading >= 1.45,
            "the leading never drops under the table's floor"
        );
    }

    /// Compact tightens the leading by the TextView preview's own offset and the rhythm by its
    /// factor; Comfortable leaves both alone.
    #[test]
    fn compact_tightens_leading_and_rhythm() {
        let comfortable = page(config(MdWidth::Readable, MdDensity::Comfortable), 2000.0);
        let compact = page(config(MdWidth::Readable, MdDensity::Compact), 2000.0);
        assert_eq!(comfortable.rhythm, 1.0);
        assert_eq!(compact.rhythm, COMPACT_RHYTHM);
        assert!((comfortable.leading - compact.leading - COMPACT_LEADING).abs() < 1e-5);
        assert_eq!(comfortable.column, compact.column, "density moves no width");
    }

    /// The reader's character scale is the body size, and everything derived from the body
    /// follows it.
    #[test]
    fn the_character_scale_scales_the_page() {
        let base = page(MdViewConfig::default(), 4000.0);
        let big = page(
            MdViewConfig {
                reading: MdReading {
                    char_scale: 1.5,
                    ..MdReading::default()
                },
                ..MdViewConfig::default()
            },
            4000.0,
        );
        assert!((big.body - base.body * 1.5).abs() < 1e-4);
        assert!((big.column - base.column * 1.5).abs() < 1e-3);
        assert!((big.margin - base.margin * 1.5).abs() < 1e-4);
    }

    /// The leading follows the measure that fits, interpolated, inside §13's range.
    #[test]
    fn the_leading_interpolates_between_the_table_ends() {
        assert_eq!(leading_for(30.0), 1.45);
        assert_eq!(leading_for(65.0), 1.45);
        assert!((leading_for(100.0) - 1.6).abs() < 1e-5);
        assert_eq!(leading_for(140.0), 1.6);
        assert!(leading_for(75.0) > 1.45 && leading_for(75.0) < 1.6);
    }

    /// A heading's size is the shared ratio, and the scale shrinks level by level.
    #[test]
    fn heading_sizes_follow_the_shared_ratio() {
        for level in 1..=6u8 {
            assert_eq!(heading_scale(level).0, theme::md_heading_ratio(level));
        }
        assert!(heading_scale(1).0 > heading_scale(2).0);
        assert!(heading_scale(2).0 > heading_scale(3).0);
    }

    /// A heading's space before is always the larger of its two gaps from H2 down — it belongs to
    /// what follows it — and a thematic break is symmetric.
    #[test]
    fn headings_belong_to_what_follows() {
        for level in 2..=6u8 {
            let r = rhythm(&BlockKind::Heading {
                level,
                text: String::new(),
                slug: String::new(),
            });
            assert!(r.before > r.after, "H{level}");
        }
        let rule = rhythm(&BlockKind::ThematicBreak);
        assert_eq!(rule.before, rule.after);
    }

    /// The fractions a row is divided into must divide it exactly — a shortfall leaves a gap at
    /// the right edge of every row in the table, an excess pushes the last column out of it.
    #[test]
    fn the_fractions_cover_the_row() {
        for wanted in [
            vec![100.0],
            vec![60.0, 60.0, 60.0],
            vec![40.0, 400.0, 40.0],
            vec![1.0, 1.0, 1.0, 10_000.0],
            vec![0.0, 0.0],
        ] {
            let total: f32 = column_weights(&wanted).iter().sum();
            assert!((total - 1.0).abs() < 1e-4, "{wanted:?} summed to {total}");
        }
    }

    /// Equal columns share equally, and a column that wants more gets more — the ordering the
    /// weighting exists for.
    #[test]
    fn a_wider_column_gets_a_wider_share() {
        let equal = column_weights(&[80.0, 80.0, 80.0]);
        assert!(equal.iter().all(|f| (f - 1.0 / 3.0).abs() < 1e-4));

        let mixed = column_weights(&[40.0, 400.0, 60.0]);
        assert!(mixed[1] > mixed[2], "the prose column should be widest");
        assert!(mixed[2] > mixed[0], "ordering should follow the widths");
    }

    /// An outlier widens its column and does not take the table: the clamp holds it to three times
    /// the mean, so the other columns keep a readable share.
    #[test]
    fn an_outlier_does_not_take_the_table() {
        let weights = column_weights(&[20.0, 20.0, 20.0, 10_000.0]);
        assert!(weights[3] < 0.8, "outlier took {}", weights[3]);
        for narrow in &weights[..3] {
            assert!(*narrow > 0.05, "narrow column left with {narrow}");
        }
    }

    /// No columns, no widths — `table` returns early on this, and the weighting must not divide by
    /// zero on the way there.
    #[test]
    fn no_columns_weigh_nothing() {
        assert!(column_weights(&[]).is_empty());
    }
}
