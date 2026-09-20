//! In-place help: point at anything in the window and read what it is.
//!
//! Chrome's element inspector is the model. The mode is switched on (⇧F1, or **Point at
//! something…** in the titlebar's overflow menu), a layer takes the whole window, and whatever the
//! cursor is over is drawn with a translucent bordered rectangle and a balloon saying what it is.
//! Moving the mouse moves the highlight; the mode persists until Escape or the overlay's own
//! **Done**.
//!
//! **Nothing here knows where a sentence comes from.** The overlay asks
//! [`crate::state::ui_id::describe`] for a label and a blurb and that is the whole of its
//! contract with the content: the table behind that function is a `const` list today and may be a
//! `ui-targets.toml` with a packer behind it tomorrow (`_docs/wip/in-place-help.md` §2.5), and
//! this module must not notice the difference.
//!
//! **The hit-test is the overlay's own.** The layer covers the window, so every mouse move lands
//! here and nothing underneath ever receives one — per-element hover handlers cannot work. Marked
//! elements report their rectangles as they are laid out and `ui::ident::hit_mark_at` looks the
//! cursor up in that list. The list is one frame behind, by construction: the whole element tree,
//! this overlay included, is built before layout runs. On a highlight that follows the mouse that
//! is not visible.
//!
//! **No click does anything.** The catcher occludes the mouse so the window underneath cannot be
//! operated, and it binds no press of its own, so a click on a target is inert rather than
//! dismissing the mode or falling through to the control. That is the brief: the mode persists
//! until it is explicitly closed. Opening the target's full help page on a click is the obvious
//! next thing and is `G319` — it needs a page to open, and `ui.*` keys are phase 3's.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, Window, anchored,
    deferred, div, point, px,
};

use crate::app::AppState;
use crate::state::ui_id::{MarkRect, TargetInfo, UiId, describe};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::{ident, kit};

/// Draw the targeting overlay, or nothing when the mode is down. Called from the window root.
pub fn overlay(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(mode) = app.workbench.help_target else {
        return div().into_any_element();
    };

    let viewport = window.viewport_size();
    let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));

    // One lookup against the frame that finished, for the name *and* its rectangle.
    let hit = mode
        .cursor
        .and_then(|(x, y)| ident::hit_mark_at(window, point(px(x), px(y))));

    let entity = cx.entity();
    let moved = entity.clone();
    let done = entity;

    let mut catcher = div()
        .id("help-target-catcher")
        .w(px(vw))
        .h(px(vh))
        .relative()
        // Everything under this is unreachable while the mode is up — which is the point. No
        // press is bound here, so a click on a target does nothing at all.
        .occlude()
        .on_mouse_move(move |event, _, cx| {
            let at = (f32::from(event.position.x), f32::from(event.position.y));
            moved.update(cx, |this, cx| this.move_help_target(at.0, at.1, cx));
        });

    if let Some(mark) = &hit {
        catcher = catcher
            .child(highlight(mark.rect))
            .child(balloon(&mark.id, mark.rect, vw, vh));
    }

    deferred(
        anchored()
            .position(point(px(0.), px(0.)))
            .child(catcher.child(chrome(hit.is_some(), done))),
    )
    // Above the kit's modals, which defer at 2. The mode is deliberately able to cover a dialog
    // and point at its controls — see `state::overlay::Layer::HelpTarget`.
    .priority(3)
    .into_any_element()
}

/// The rectangle around the target: a translucent accent fill under an accent border.
///
/// Square, like every other surface, and drawn *over* the target rather than around it — the fill
/// is faint enough to read the control through, which is why the mode does not dim the window the
/// way a modal does. A reader pointing at a button wants to see the button.
fn highlight(rect: MarkRect) -> AnyElement {
    div()
        .absolute()
        .left(px(rect.x))
        .top(px(rect.y))
        .w(px(rect.width))
        .h(px(rect.height))
        .bg(theme::fade(theme::accent(), 0.18))
        .border_1()
        .border_color(theme::accent())
        .into_any_element()
}

/// The sentence, beside the thing it is about, and always on screen.
fn balloon(id: &UiId, rect: MarkRect, vw: f32, vh: f32) -> AnyElement {
    let width = theme::help_balloon_width().min(vw - 2. * theme::help_balloon_gap());
    let (left, top) = place(rect, width, vw, vh);

    let info = describe(id);
    let label = info.map(|i| i.label).unwrap_or_else(|| id.leaf());
    // A name with no sentence is not a failure: marking is incremental, and `describe` returning
    // `None` means "named, not yet written about". The balloon says so rather than showing an
    // empty panel.
    let blurb = info
        .map(|i: TargetInfo| i.blurb.to_string())
        .unwrap_or_else(|| "This place has a name but no description yet.".to_string());

    div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(width))
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text())
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_muted())
                .child(blurb),
        )
        .into_any_element()
}

/// Where the balloon goes: under the target, flipped above it near the bottom, clamped to the
/// window on every side.
///
/// Its own function because it is the only arithmetic in this module a test can drive — an
/// off-screen balloon is the failure a reader actually hits, and it hides in the corner cases
/// rather than in the common one. See `crates/ubiq/tests/ui_id.rs`.
pub fn place(rect: MarkRect, width: f32, vw: f32, vh: f32) -> (f32, f32) {
    let gap = theme::help_balloon_gap();
    let room = theme::help_balloon_room();

    let below = rect.y + rect.height + gap;
    let above = rect.y - room - gap;
    // Below unless that runs off the bottom and there is room above — a target near the foot of
    // the window gets its sentence over it rather than half off the screen.
    let top = if below + room <= vh - gap || above < gap {
        below
    } else {
        above
    };
    let top = top.clamp(gap, (vh - room - gap).max(gap));

    // Aligned with the target's left edge, pulled back in when that would hang off the right.
    let left = rect.x.clamp(gap, (vw - width - gap).max(gap));
    (left, top)
}

/// The mode's own furniture: what this is, and the visible way out.
///
/// Bottom-right rather than centred at the top, because the titlebar and the rail are the two
/// areas a reader most wants to point at and a bar across either of them would be furniture
/// standing on its own subject.
fn chrome(hit: bool, entity: gpui::Entity<AppState>) -> AnyElement {
    let note = if hit {
        "Point at anything to read what it is."
    } else {
        "Move the pointer over the window. Unnamed areas have nothing to say yet."
    };

    div()
        .absolute()
        .right(px(theme::help_balloon_gap()))
        .bottom(px(theme::help_balloon_gap()))
        .flex()
        .items_center()
        .gap_3()
        .py_1()
        .px_2()
        .bg(theme::surface_raised())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::accent())
        .shadow_lg()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(note),
        )
        .child(kit::ghost_button(
            "help-target-done",
            None,
            "Done",
            move |_, _, cx| {
                entity.update(cx, |this, cx| this.close_help_target(cx));
            },
        ))
        .into_any_element()
}
