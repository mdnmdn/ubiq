//! A thin strip down the side of a scrollable document, marking where things of interest sit.
//!
//! `crate::ui::plan`'s thread rail is the first caller: one mark per thread, positioned by how
//! far down the document it anchors to and coloured by whether it is still open. **Nothing here
//! names a thread, a plan or a block** — the plan editor is meant to become a reusable markdown
//! editor later, and a minimap belongs to any document it draws, not to plans. It takes positions
//! and marks as data; the caller decides what a position means and what a colour reports.

use gpui::{
    AnyElement, Div, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, Rgba,
    Stateful, Styled, div, px, relative,
};

use crate::theme;
use crate::ui::kit::IndexedAction;

/// One mark on the strip: how far down it sits, `0.0` at the top and `1.0` at the bottom, and the
/// colour it reports in. The caller resolves both — this carries no opinion about what a fraction
/// or a colour *means*.
#[derive(Clone, Copy, Debug)]
pub struct MinimapMark {
    pub fraction: f32,
    pub colour: Rgba,
}

impl MinimapMark {
    pub fn new(fraction: f32, colour: Rgba) -> Self {
        Self {
            fraction: fraction.clamp(0.0, 1.0),
            colour,
        }
    }
}

/// The height of a tick, and how far it stops short of the strip's own edges.
const MARK_HEIGHT: f32 = 3.0;
const MARK_HEIGHT_HOVER: f32 = 5.0;
const MARK_INSET: f32 = 6.0;

/// The strip itself — a fixed-width column that fills whatever height its parent gives it. Each
/// mark is a short horizontal tick at its own fraction down; clicking one hands `on_select` its
/// index into `marks`, the caller's to resolve back to whatever it positioned by — the same
/// discipline every indexed row in the window keeps.
pub fn minimap(
    id: impl Into<ElementId>,
    width: f32,
    marks: &[MinimapMark],
    on_select: IndexedAction,
) -> AnyElement {
    let mut root: Stateful<Div> = div()
        .id(id.into())
        .relative()
        .flex_none()
        .w(px(width))
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border());

    for (index, mark) in marks.iter().enumerate() {
        let on_select = on_select.clone();
        root = root.child(
            div()
                .id(("minimap-mark", index))
                .absolute()
                .top(relative(mark.fraction))
                .left(px(MARK_INSET))
                .right(px(MARK_INSET))
                .h(px(MARK_HEIGHT))
                .bg(mark.colour)
                .cursor_pointer()
                .hover(|this| this.h(px(MARK_HEIGHT_HOVER)))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    on_select(index, window, cx)
                }),
        );
    }

    root.into_any_element()
}
