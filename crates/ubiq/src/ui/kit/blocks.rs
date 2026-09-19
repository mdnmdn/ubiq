//! The blocks a graph is drawn as: the board they sit on, the fences round them, and the surface
//! one block is.
//!
//! [`canvas`](super::canvas) paints the ground, the curves and the trail; this is the layer above
//! it — the stack those paintings go into, in the one order a graph reads in: ground, the fences
//! on it, the links between what they hold, the blocks themselves, and whatever goes over the lot.
//! A caller adds things in whatever order it measures them and gets that order anyway, which is
//! what stops two screens drawing the same graph with the z-order one notch apart.
//!
//! **Everything here is view-agnostic and takes plain geometry at 100% zoom.** Nothing in it knows
//! what a card, an agent or a task is: it is handed a rectangle, an element id, and the body to put
//! inside — and the one zoom it scales all of them by, so no call site multiplies a coordinate
//! itself. The drag stays the caller's: [`block`] and [`handle`] answer the positioned, styled
//! element and the caller hangs its own `on_drag` on it, which keeps the carried type the screen's
//! own rather than something this module has to name.

use gpui::{
    AnyElement, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement, Pixels, Rgba,
    ScrollHandle, SharedString, Size, Stateful, StatefulInteractiveElement as _, Styled, div,
    point, prelude::FluentBuilder as _, px,
};

use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::canvas::{self, Link};
use crate::ui::kit::{card, mono};

/// A rectangle at 100% zoom: `(x, y, w, h)`, in the board's own frame.
pub type Rect = (f32, f32, f32, f32);

/// One word on a fence's label: what it says, in what colour, at what size in the chrome scale.
///
/// A label is a row of these rather than one string, because a container says more than one thing
/// about itself and the parts are not the same weight — what it is reads quieter than what it is
/// called.
pub struct Word {
    pub text: SharedString,
    pub colour: Rgba,
    pub role: Role,
}

impl Word {
    pub fn new(text: impl Into<SharedString>, colour: Rgba, role: Role) -> Self {
        Word {
            text: text.into(),
            colour,
            role,
        }
    }
}

/// A grouping: a dashed outline, the label chip on its top edge, and — where the caller gave one —
/// the element that takes a drag on its empty ground.
///
/// `active` is a fence lit as a drop target, which is the one thing about it that changes while
/// something is being carried.
pub struct Fence {
    pub rect: Rect,
    pub colour: Rgba,
    pub active: bool,
    /// The label's words, left to right. Empty draws no chip at all rather than an empty one.
    pub label: Vec<Word>,
    /// Where the chip sits, from the rect's top-left, and how tall it is. The caller's, because
    /// the inset is a property of the padding the grouping was measured with and this module
    /// does not know it.
    pub label_at: (f32, f32),
    pub label_height: f32,
    /// What takes a drag on the fence's ground, from [`handle`]. `None` is a fence nothing moves.
    pub handle: Option<AnyElement>,
}

impl Fence {
    pub fn new(rect: Rect, colour: Rgba, active: bool) -> Self {
        Fence {
            rect,
            colour,
            active,
            label: Vec::new(),
            label_at: (0.0, 0.0),
            label_height: 0.0,
            handle: None,
        }
    }

    pub fn labelled(mut self, label: Vec<Word>, at: (f32, f32), height: f32) -> Self {
        self.label = label;
        self.label_at = at;
        self.label_height = height;
        self
    }

    pub fn handle(mut self, handle: AnyElement) -> Self {
        self.handle = Some(handle);
        self
    }
}

/// How one block reads: the colour of its left edge, whether it is the one being looked at, and
/// whether it is the one under the pointer.
///
/// A carried block is lifted off the ground — opaque against whatever it is dropping, with the
/// accent on a doubled edge — so it is the one thing in focus while it moves.
#[derive(Clone, Copy)]
pub struct Look {
    pub edge: Rgba,
    pub selected: bool,
    pub carried: bool,
    /// Whether the block needs a fill of its own when it is not the selected one. A block on a
    /// dotted ground does; one over an opaque surface does not.
    pub opaque: bool,
}

impl Look {
    pub fn new(edge: Rgba) -> Self {
        Look {
            edge,
            selected: false,
            carried: false,
            opaque: false,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn carried(mut self, carried: bool) -> Self {
        self.carried = carried;
        self
    }

    pub fn opaque(mut self, opaque: bool) -> Self {
        self.opaque = opaque;
        self
    }
}

/// One block: a pickable surface at a fixed place on the board, scaled by its zoom.
///
/// The caller fills it and hangs its own click and drag on it — this is the shell, which is the
/// part two screens would otherwise each spell out and each get slightly differently.
pub fn block(id: impl Into<ElementId>, rect: Rect, look: Look, zoom: f32) -> Stateful<Div> {
    card(id, look.edge, look.selected)
        .absolute()
        .left(px(rect.0 * zoom))
        .top(px(rect.1 * zoom))
        .w(px(rect.2 * zoom))
        .h(px(rect.3 * zoom))
        .p(px(10.0 * zoom))
        .gap(px(6.0 * zoom))
        // A block is a fixed box on a canvas, so a long line is clipped by it rather than spilling
        // over the blocks below.
        .overflow_hidden()
        .when(look.opaque && !look.selected, |this| {
            this.bg(theme::pane_bg())
        })
        .when(look.carried, |this| {
            this.bg(theme::surface_raised())
                .border_l(px(theme::accent_edge() * 2.0))
                .border_color(theme::accent())
        })
        .cursor_grab()
}

/// The empty ground inside a fence, as the thing that takes a drag on it.
///
/// It draws nothing: it is a rectangle over the fence's whole box, added under the blocks, so
/// grabbing a block moves one block and grabbing anywhere else in the fence moves the grouping.
pub fn handle(id: impl Into<ElementId>, rect: Rect, zoom: f32) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .left(px(rect.0 * zoom))
        .top(px(rect.1 * zoom))
        .w(px(rect.2 * zoom))
        .h(px(rect.3 * zoom))
        .cursor_grab()
}

/// The board: what is on the graph, in the order it reads.
///
/// Things are added as they are measured and stacked as they should be drawn — the five layers are
/// this type's business and not the caller's. It also accumulates the extent as it goes, so the
/// canvas ends up as big as what is on it and every caller's scroll reaches the same edge.
pub struct Board {
    zoom: f32,
    /// The room left past the outermost thing, so something at the edge can still be picked up and
    /// put down without fighting the scroll.
    margin: f32,
    extent: (f32, f32),
    fences: Vec<AnyElement>,
    links: Vec<Link>,
    inner: Vec<AnyElement>,
    blocks: Vec<AnyElement>,
    over: Vec<AnyElement>,
}

impl Board {
    pub fn new(zoom: f32, margin: f32) -> Self {
        Board {
            zoom,
            margin,
            extent: (0.0, 0.0),
            fences: Vec::new(),
            links: Vec::new(),
            inner: Vec::new(),
            blocks: Vec::new(),
            over: Vec::new(),
        }
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// What is on the board takes up, margin included — the size the canvas ends up.
    pub fn extent(&self) -> (f32, f32) {
        self.extent
    }

    /// Grow the extent to hold a rectangle, drawing nothing. For the geometry that has to be
    /// reachable without being a layer of its own.
    pub fn covers(&mut self, rect: Rect) {
        self.extent = (
            self.extent.0.max(rect.0 + rect.2 + self.margin),
            self.extent.1.max(rect.1 + rect.3 + self.margin),
        );
    }

    /// A grouping, under the links: its outline, the ground that takes its drag, and its label.
    pub fn fence(&mut self, fence: Fence) {
        self.covers(fence.rect);
        let parts = self.fence_parts(fence);
        self.fences.extend(parts);
    }

    /// A grouping drawn *over* the links rather than under them — a fence inside a fence, which
    /// belongs to the block it surrounds rather than to the ground.
    pub fn inner_fence(&mut self, fence: Fence) {
        self.covers(fence.rect);
        let parts = self.fence_parts(fence);
        self.inner.extend(parts);
    }

    fn fence_parts(&self, fence: Fence) -> Vec<AnyElement> {
        let zoom = self.zoom;
        let (x, y, w, h) = fence.rect;
        let mut parts: Vec<AnyElement> = vec![
            canvas::dashed_box(
                (x * zoom, y * zoom, w * zoom, h * zoom),
                fence.colour,
                fence.active,
            )
            .into_any_element(),
        ];
        if let Some(handle) = fence.handle {
            parts.push(handle);
        }
        if !fence.label.is_empty() {
            parts.push(
                div()
                    .absolute()
                    .left(px((x + fence.label_at.0) * zoom))
                    .top(px((y + fence.label_at.1) * zoom))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .h(px(fence.label_height * zoom))
                    .px(px(8.0 * zoom))
                    .bg(theme::pane_bg())
                    .children(fence.label.into_iter().map(|word| {
                        mono(word.text, word.colour)
                            .text_size(theme::font(Family::Chrome, word.role) * zoom)
                    }))
                    .into_any_element(),
            );
        }
        parts
    }

    pub fn link(&mut self, link: Link) {
        self.links.push(link);
    }

    /// One block, over the fences and the links. `rect` is what it takes up, which is how the
    /// board knows to leave room for it whatever the block itself drew.
    pub fn block(&mut self, rect: Rect, block: impl IntoElement) {
        self.covers(rect);
        self.blocks.push(block.into_any_element());
    }

    /// A layer over everything, including the blocks — the trail a carried block sheds.
    pub fn over(&mut self, element: impl IntoElement) {
        self.over.push(element.into_any_element());
    }

    /// The stack, sized to what is on it and never smaller than the viewport, so a block dragged
    /// near the right or bottom edge of an otherwise-small graph still has somewhere to scroll to.
    ///
    /// The caller hangs the drop handlers on what comes back: the whole canvas is the drop target,
    /// and which fence something landed in is worked out from where it is rather than from what it
    /// was dropped on.
    pub fn content(self, viewport: Size<Pixels>) -> Div {
        let zoom = self.zoom;
        let mut content = div()
            .relative()
            .w(px((self.extent.0 * zoom).max(f32::from(viewport.width))))
            .h(px((self.extent.1 * zoom).max(f32::from(viewport.height))))
            .child(canvas::dot_grid(
                theme::GRAPH_DOT_PITCH * zoom,
                point(0.0, 0.0),
            ));
        for fence in self.fences {
            content = content.child(fence);
        }
        content = content.child(canvas::links(self.links));
        for inner in self.inner {
            content = content.child(inner);
        }
        for block in self.blocks {
            content = content.child(block);
        }
        for over in self.over {
            content = content.child(over);
        }
        content
    }
}

/// The frame a board scrolls in. Separate from [`Board::content`] because the content is what takes
/// the drop and the frame is what takes the scroll, and one element cannot be both without the
/// drop reading the scroller's bounds instead of the canvas's.
pub fn scroller(id: impl Into<ElementId>, scroll: &ScrollHandle) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_scroll()
        .track_scroll(scroll)
        .bg(theme::app_bg())
}
