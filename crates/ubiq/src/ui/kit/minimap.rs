//! A thin strip down the side of a scrollable document, drawn as a miniature of its layout rather
//! than a row of identical ticks — `_docs/inbox/markdown-improvement-proposal.md` §8.2: a heading
//! draws a bar, a paragraph line draws at its own real length, a table draws a dotted row, a code
//! block or an image draws a filled rectangle. `crate::state::document::minimap_rows` is the first
//! caller for the shapes and `thread_marks` for the ticks; **nothing here names a block kind, a
//! plan or a thread** — the caller resolves geometry and colour, this only draws them.
//!
//! A [`MinimapMark`] is one drawn shape: a filled bar for everything except a table row, which
//! draws as a dotted line instead (`dotted: true`) — the shape the proposal's table row and the
//! user's own "for tables, dotted line representing chars" both ask for. A [`MinimapTick`] is the
//! small coloured mark on the strip's outer edge §8.4 describes for change markers — this module's
//! second caller is `thread_marks`' open/resolved dot, carried over unchanged from the first
//! minimap. The [`MinimapViewport`] is the translucent, draggable rectangle §8.3 adds: dragging or
//! clicking anywhere on the strip calls `on_scrub` with where the pointer landed, `0.0` at the top
//! and `1.0` at the bottom — the caller's job to turn that into a scroll offset, the same division
//! of labour `on_select` already keeps for a tick.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, Bounds, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Pixels, Rgba, Styled, canvas, div, px, relative,
};

use crate::theme;
use crate::ui::kit::{IndexedAction, ScrubAction};

/// One shape the strip draws for a block or a real line of one — position and extent are all
/// fractions of the strip's own box, `0.0..=1.0`, so the caller never hands this pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapMark {
    /// How far down the strip this mark's top sits.
    pub top: f32,
    /// How tall the mark is, down the strip.
    pub height: f32,
    /// How far across the strip's own width the mark runs — a real line's length against a full
    /// column, for the shapes the proposal draws as lines; a fixed span for the ones it draws as
    /// a shape instead (a heading's bar, a code block's or an image's rectangle).
    pub length: f32,
    /// A row of small ticks along `length` instead of a solid fill — the table row shape.
    pub dotted: bool,
    pub colour: Rgba,
}

impl MinimapMark {
    pub fn new(top: f32, height: f32, length: f32, dotted: bool, colour: Rgba) -> Self {
        Self {
            top: top.clamp(0.0, 1.0),
            height: height.max(0.0),
            length: length.clamp(0.0, 1.0),
            dotted,
            colour,
        }
    }
}

/// A small coloured mark on the strip's outer edge — a thread's open/resolved state today, a git
/// change tomorrow (proposal §8.4). Kept apart from [`MinimapMark`] because it answers a click of
/// its own (`on_select`) rather than the strip's general scrub.
#[derive(Clone, Copy, Debug)]
pub struct MinimapTick {
    pub fraction: f32,
    pub colour: Rgba,
}

impl MinimapTick {
    pub fn new(fraction: f32, colour: Rgba) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            colour,
        }
    }
}

/// The translucent rectangle standing for the document's visible region — draggable, per T-110's
/// feedback on the first minimap, which had none.
#[derive(Clone, Copy, Debug)]
pub struct MinimapViewport {
    pub top: f32,
    pub height: f32,
}

/// The height of a tick, and how far it stops short of the strip's own edges.
const TICK_HEIGHT: f32 = 3.0;
const TICK_HEIGHT_HOVER: f32 = 5.0;
/// The strip's left inset, and the outer-edge band the ticks draw in — narrower than the marks'
/// own span, so a tick reads as sitting past them rather than among them.
const MARK_INSET: f32 = 4.0;
const TICK_BAND: f32 = 6.0;
/// A dotted mark's own dot size and pitch.
const DOT_SIZE: f32 = 1.5;
const DOT_PITCH: f32 = 4.0;
/// Nothing draws shorter than this, in pixels, whatever a mark's own fraction rounds to — a block
/// measured at a sliver of the strip's height still has to read as a mark rather than vanish.
const MIN_MARK_HEIGHT_PX: f32 = 1.5;

/// The strip itself — a fixed-width column that fills whatever height its parent gives it.
///
/// Structure marks and the viewport rectangle are drawn from a `canvas` capturing the strip's own
/// painted bounds into `bounds` — the one thing the strip needs to turn an absolute pointer
/// position into the fraction `on_scrub` and the mark fractions both already speak.
#[allow(clippy::too_many_arguments)]
pub fn minimap(
    id: impl Into<ElementId>,
    width: f32,
    marks: &[MinimapMark],
    ticks: &[MinimapTick],
    viewport: Option<MinimapViewport>,
    on_select: IndexedAction,
    on_scrub: ScrubAction,
) -> AnyElement {
    let bounds: Rc<Cell<Bounds<Pixels>>> = Rc::new(Cell::new(Bounds::default()));

    let capture = bounds.clone();
    let bounds_capture = canvas(move |bounds, _, _| capture.set(bounds), |_, _, _, _| {})
        .absolute()
        .inset_0()
        .size_full();

    let mut root = div()
        .id(id.into())
        .relative()
        .flex_none()
        .w(px(width))
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border())
        .child(bounds_capture);

    let inner_width = (width - 2.0 * MARK_INSET).max(0.0);
    for mark in marks {
        root = root.child(draw_mark(mark, inner_width));
    }

    for (index, tick) in ticks.iter().enumerate() {
        let on_select = on_select.clone();
        root = root.child(
            div()
                .id(("minimap-tick", index))
                .absolute()
                .top(relative(tick.fraction))
                .right(px(0.))
                .w(px(TICK_BAND))
                .h(px(TICK_HEIGHT))
                .bg(tick.colour)
                .cursor_pointer()
                .hover(|this| this.h(px(TICK_HEIGHT_HOVER)))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    on_select(index, window, cx)
                }),
        );
    }

    if let Some(viewport) = viewport {
        root = root.child(
            div()
                .absolute()
                .top(relative(viewport.top))
                .left(px(1.))
                .right(px(1.))
                .h(relative(viewport.height.max(0.01)))
                .bg(theme::accent_soft())
                .border_1()
                .border_color(theme::accent()),
        );
    }

    let scrub_click = on_scrub.clone();
    let bounds_for_click = bounds.clone();
    root = root.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
        if let Some(fraction) = fraction_at(bounds_for_click.get(), event.position.y) {
            scrub_click(fraction, window, cx);
        }
    });
    let bounds_for_drag = bounds;
    root = root.on_mouse_move(move |event: &MouseMoveEvent, window, cx| {
        if !event.dragging() {
            return;
        }
        if let Some(fraction) = fraction_at(bounds_for_drag.get(), event.position.y) {
            on_scrub(fraction, window, cx);
        }
    });

    root.into_any_element()
}

fn fraction_at(bounds: Bounds<Pixels>, y: Pixels) -> Option<f32> {
    if bounds.size.height <= px(0.) {
        return None;
    }
    Some(((y - bounds.top()) / bounds.size.height).clamp(0.0, 1.0))
}

fn draw_mark(mark: &MinimapMark, inner_width: f32) -> AnyElement {
    let length_px = px((inner_width * mark.length).max(0.0));
    let base = div()
        .absolute()
        .top(relative(mark.top))
        .left(px(MARK_INSET))
        .h(relative(mark.height))
        .min_h(px(MIN_MARK_HEIGHT_PX));

    if mark.dotted {
        let dots = ((f32::from(length_px) / DOT_PITCH).round() as usize).clamp(1, 128);
        // Its own dots are absolutely positioned too, so this needs to be a containing block in
        // its own right rather than only positioned against the strip.
        let mut row = base.relative();
        for i in 0..dots {
            row = row.child(
                div()
                    .absolute()
                    .left(px(i as f32 * DOT_PITCH))
                    .w(px(DOT_SIZE))
                    .h(px(DOT_SIZE))
                    .bg(mark.colour),
            );
        }
        row.into_any_element()
    } else {
        base.w(length_px).bg(mark.colour).into_any_element()
    }
}
