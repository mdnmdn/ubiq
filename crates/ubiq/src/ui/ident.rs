//! `.ui_id(...)` — how an element tells the frame which place it is, and where it was drawn.
//!
//! [`crate::state::ui_id`] holds the names and the hit-test; this is the GPUI half that fills the
//! registry. An identified element gains one absolutely-positioned child that reports its own
//! laid-out rectangle during prepaint. That is the only way the bounds can be known: an element
//! does not learn where it ended up until layout has run, and a `canvas`'s prepaint closure is the
//! hook GPUI offers for reading it. `ui::web_view` tracks its live webviews through the same trick.
//!
//! **Why not a hover handler per element.** The in-place help overlay covers the window, so every
//! mouse move lands on the overlay and nothing underneath ever sees one. The overlay therefore has
//! to hit-test for itself, which means it needs a list of rectangles rather than a callback tree.
//!
//! **Nothing here is about the brand mark.** `ui::mark` is the logo; the word does not appear in
//! this module's API for that reason, and `UiMark` in `state::ui_id` is one recorded rectangle.
//!
//! The registry is per window and lives here in a thread-local, the same shape `ui::web_view`'s
//! own frame bookkeeping uses: it is not application state, and routing it through `AppState` would
//! mean an `Entity::update` inside prepaint.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{App, Bounds, Pixels, Position, Window, WindowId, canvas, prelude::*, px};

use crate::state::ui_id::{MarkRect, MarkRegistry, UiId, UiMark};

thread_local! {
    /// One registry per window. Windows render on this thread one at a time, but they interleave
    /// across frames, so a single shared list would have window A's frame wipe window B's.
    static REGISTRIES: RefCell<HashMap<WindowId, MarkRegistry>> = RefCell::new(HashMap::new());
}

/// Retire the frame that just finished and start a fresh one.
///
/// Called once per frame from the root of the element tree, before any child is built — element
/// construction runs to completion before layout does, so every record for this frame arrives
/// after this call.
pub fn begin_frame(window: &Window) {
    let id = window.window_handle().window_id();
    REGISTRIES.with_borrow_mut(|map| map.entry(id).or_default().begin_frame());
}

/// Drop a closed window's records.
pub fn forget(window_id: WindowId) {
    REGISTRIES.with_borrow_mut(|map| {
        map.remove(&window_id);
    });
}

/// The most specific identified place under a point, from the last complete frame.
///
/// This is what the in-place help overlay calls on a mouse move: one lookup for the whole window,
/// against a list, with no per-element handlers involved.
pub fn hit_at(window: &Window, at: gpui::Point<Pixels>) -> Option<UiId> {
    let id = window.window_handle().window_id();
    REGISTRIES.with_borrow(|map| {
        map.get(&id)
            .and_then(|reg| reg.hit(at.x.into(), at.y.into()))
            .map(|mark| mark.id.clone())
    })
}

/// The same lookup, with the rectangle the name was drawn in.
///
/// What the in-place help overlay actually calls: it has to draw a highlight around the target as
/// well as name it, and asking twice — once for the id and once for its bounds — would be two
/// answers from one list that a resize between them could disagree about.
pub fn hit_mark_at(window: &Window, at: gpui::Point<Pixels>) -> Option<UiMark> {
    let id = window.window_handle().window_id();
    REGISTRIES.with_borrow(|map| {
        map.get(&id)
            .and_then(|reg| reg.hit(at.x.into(), at.y.into()))
            .cloned()
    })
}

/// Every record from the last complete frame, in layout order.
pub fn recorded(window: &Window) -> Vec<UiMark> {
    let id = window.window_handle().window_id();
    REGISTRIES.with_borrow(|map| {
        map.get(&id)
            .map(|reg| reg.marks().to_vec())
            .unwrap_or_default()
    })
}

fn record(window: &Window, id: UiId, bounds: Bounds<Pixels>) {
    let rect = MarkRect::new(
        bounds.origin.x.into(),
        bounds.origin.y.into(),
        bounds.size.width.into(),
        bounds.size.height.into(),
    );
    let window_id = window.window_handle().window_id();
    REGISTRIES.with_borrow_mut(|map| map.entry(window_id).or_default().record(id, rect));
}

/// Name a place on the screen.
///
/// ```ignore
/// div().child(…).ui_id(ui_id::TITLEBAR_HELP)
/// ```
///
/// The name must be in [`crate::state::ui_id::CATALOGUE`] — that is what makes it addressable by
/// help content and by anything later keyed to the same place. This is unrelated to the
/// kebab-case `ElementId` a stateful element already passes to GPUI, which is hover and scroll
/// bookkeeping and means nothing outside its frame.
///
/// **It makes its receiver `relative`, unless it is already `absolute`.** The probe is an
/// absolutely-positioned child, and an absolute child measures against its nearest positioned
/// ancestor, so the receiver has to be one. `relative` with no offsets moves nothing on an
/// otherwise unpositioned element — but `position` is one field, and forcing it unconditionally
/// would silently turn an already-`.absolute()` receiver back into a flow item, moving it in its
/// *own* parent's layout as a side effect of naming it. An `.absolute()` element is already a
/// positioned ancestor for its children, so `.ui_id()` leaves that alone and only sets `relative`
/// when the receiver has no position of its own yet.
pub trait Identified: ParentElement + Styled + Sized {
    fn ui_id(mut self, id: UiId) -> Self {
        if self.style().position != Some(Position::Absolute) {
            self = self.relative();
        }
        self.child(
            canvas(
                move |bounds, window: &mut Window, _: &mut App| record(window, id.clone(), bounds),
                |_, _, _: &mut Window, _: &mut App| {},
            )
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .size_full(),
        )
    }
}

impl<T: ParentElement + Styled + Sized> Identified for T {}
