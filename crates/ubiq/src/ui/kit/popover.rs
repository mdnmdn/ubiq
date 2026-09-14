//! The floating panel: a list that hangs above the element that asked for it.
//!
//! **It is drawn where it is asked for, at the asker's top edge, and painted over everything.** The
//! layer goes through `deferred` and `anchored`, exactly like the modal and the dropdown, so a panel
//! asked for at the bottom of a conversation is not clipped to that surface — it covers whatever is
//! under it, upward over the transcript. `Anchor::BottomLeft` puts its bottom edge on the trigger's
//! top edge: flush, no gap, so the row of labels it hangs from doubles as its frame.
//!
//! The shape is the shape everything else is: square, filled, a coloured left edge saying what it
//! is. The caller owns the rows; the panel owns the chrome. One open panel per asker, chosen with
//! the same toggle discipline the menus follow.

use gpui::{
    AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement as _, Pixels,
    Styled as _, anchored, deferred, div, px,
};

use crate::theme;
use crate::ui::kit::menu::{MENU_ANCHOR_UP, MENU_LAYER};

/// A list in a floating panel above its trigger. `id` names the deferred layer this element sits
/// on, so outside-click and `cancel_dialog` can tell one panel from the next; `min_width` is the
/// narrowest the panel will draw; `children` are the rows, already laid out in the order they go.
/// `debug` is the `debug_selector` the panel answers to, when the caller wants its tests to find
/// it by name — the dropdowns this panel mirrors do without, the tests of a conversation screen
/// name their own.
#[allow(clippy::too_many_arguments)]
pub fn popover(
    id: ElementId,
    min_width: Pixels,
    debug: Option<&'static str>,
    children: impl IntoIterator<Item = AnyElement>,
) -> AnyElement {
    let mut panel = div()
        .id(id)
        .min_w(min_width)
        .p_1()
        .flex()
        .flex_col()
        .flex_none()
        .gap_1()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg();
    if let Some(name) = debug {
        panel = panel.debug_selector(move || name.into());
    }
    deferred(
        anchored()
            .anchor(MENU_ANCHOR_UP)
            .snap_to_window_with_margin(px(8.))
            .child(panel.children(children)),
    )
    .priority(MENU_LAYER)
    .into_any_element()
}
