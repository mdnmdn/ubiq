//! The modal: one question, over the window, with the keyboard's attention.
//!
//! **It is drawn where it is asked for and painted over everything.** Like the kit's dropdown, the
//! layer goes through `deferred` and `anchored`, so a modal asked for inside a dock panel is not
//! clipped to that panel — it covers the window, scrim and all. That is what lets a screen own its
//! own modal instead of the shell keeping a layer nobody but one screen uses.
//!
//! The shape is the same shape as everything else: square, filled, a coloured left edge saying what
//! it is — accent for a question, `danger` for something that will not come back. The scrim is a
//! token, so both palettes dim by the amount their own ground needs.
//!
//! **Dismissal is outside-click plus the header's close**, and never the scrim as a click target:
//! the panel answers `on_mouse_down_out`, exactly as the dropdown does, so the two dismiss the same
//! way. The scrim occludes the mouse, so nothing behind a modal can be clicked while it is up.

use std::rc::Rc;

use gpui::{
    AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px, relative,
};
use gpui_component::IconName;

use crate::theme;
use crate::ui::kit::controls::{icon_button, section_label};

/// One modal: a title, a body, and whatever actions the caller offers under it.
///
/// `edge` is what the modal is about — `theme::accent()` for a question, `theme::danger()` for
/// something irreversible — because the edge is what identifies a surface in this interface.
pub fn modal(
    id: &'static str,
    edge: Rgba,
    title: &str,
    body: AnyElement,
    footer: AnyElement,
    on_dismiss: impl Fn(&mut Window, &mut gpui::App) + 'static,
    window: &Window,
) -> AnyElement {
    let dismiss = Rc::new(on_dismiss);
    let close = dismiss.clone();
    let viewport = window.viewport_size();

    let panel = div()
        .id(ElementId::Name(format!("{id}-panel").into()))
        .w(px(theme::MODAL_WIDTH))
        .max_h(viewport.height * theme::MODAL_MAX_HEIGHT)
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(edge)
        .shadow_lg()
        .child(
            div()
                .h(px(38.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .gap_2()
                .child(section_label(title))
                .child(icon_button(
                    ElementId::Name(format!("{id}-close").into()),
                    IconName::Close,
                    false,
                    move |_, window, cx| close(window, cx),
                )),
        )
        .child(
            div()
                .id(ElementId::Name(format!("{id}-body").into()))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .gap_3()
                .px_3()
                .pb_3()
                .overflow_y_scroll()
                .child(body),
        )
        .child(
            div()
                .px_3()
                .py_2()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .gap_2()
                .bg(theme::pane_bg())
                .border_t_1()
                .border_color(theme::border())
                .child(footer),
        )
        .on_mouse_down_out(move |_, window, cx| dismiss(window, cx));

    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .id(id)
                .w(viewport.width)
                .h(viewport.height)
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::scrim())
                .occlude()
                .child(panel),
        ),
    )
    // Above the kit's dropdowns, which sit at 1: a modal that a menu could cover is not modal.
    .priority(2)
    .into_any_element()
}

/// A paragraph inside a modal's body, at the size the rest of the window reads at.
pub fn modal_note(text: &str) -> impl IntoElement {
    div()
        .w(relative(1.))
        .text_size(px(12.5))
        .text_color(theme::text_muted())
        .child(text.to_string())
}
