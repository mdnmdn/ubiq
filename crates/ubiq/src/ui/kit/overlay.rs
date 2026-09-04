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
    AnyElement, App, ElementId, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    Rgba, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px, relative,
};
use gpui_component::IconName;
use gpui_component::input::{Input, InputState};

use crate::theme;
use crate::ui::kit::controls::{field, ghost_button, icon_button, primary_button, section_label};
use crate::ui::kit::settings::label_block;

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

/// A question with two answers, on top of [`modal`].
///
/// `danger` picks the edge colour: something irreversible reads in [`theme::danger`], an
/// ordinary question in [`theme::accent`] — the same rule a modal's edge always follows. This
/// is the shape three hand-rolled confirms in the window used to repeat each with its own
/// footer; they answer to this one instead.
#[allow(clippy::too_many_arguments)]
pub fn confirm_modal(
    id: &'static str,
    title: &str,
    message: &str,
    confirm_label: &str,
    danger: bool,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    window: &Window,
) -> AnyElement {
    let edge = if danger {
        theme::danger()
    } else {
        theme::accent()
    };
    let dismiss = Rc::new(on_dismiss);
    let cancel_dismiss = dismiss.clone();
    let modal_dismiss = dismiss;

    let body = div().pt_3().child(modal_note(message)).into_any_element();

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            ElementId::Name(format!("{id}-cancel").into()),
            None,
            "Cancel",
            move |_, window, cx| cancel_dismiss(window, cx),
        ))
        .child(primary_button(
            ElementId::Name(format!("{id}-confirm").into()),
            None,
            confirm_label.to_string(),
            move |_, window, cx| on_confirm(window, cx),
        ))
        .into_any_element();

    modal(
        id,
        edge,
        title,
        body,
        footer,
        move |window, cx| modal_dismiss(window, cx),
        window,
    )
}

/// One labelled field and a confirm, on top of [`modal`].
///
/// The caller owns the [`InputState`], so what was typed survives a redraw and the caller reads
/// it back when `on_confirm` fires — this primitive never copies the text into state of its
/// own. `note`, when given, is the paragraph above the field explaining what it is for, in
/// [`modal_note`]'s voice; `confirm_enabled: false` dims the confirm button the same way a
/// disabled action dims anywhere else in this window, rather than inventing a second disabled
/// style.
#[allow(clippy::too_many_arguments)]
pub fn prompt_modal(
    id: &'static str,
    title: &str,
    note: Option<&str>,
    label: &str,
    input: &Entity<InputState>,
    confirm_label: &str,
    confirm_enabled: bool,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);

    let mut body = div().flex().flex_col().gap_3().pt_3();
    if let Some(note) = note {
        body = body.child(modal_note(note));
    }
    let body = body
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(label, ""))
                .child(
                    field(theme::border(), focused)
                        .h(px(30.))
                        .px_2()
                        .child(Input::new(input).appearance(false)),
                ),
        )
        .into_any_element();

    let dismiss = Rc::new(on_dismiss);
    let cancel_dismiss = dismiss.clone();
    let modal_dismiss = dismiss;

    let confirm = primary_button(
        ElementId::Name(format!("{id}-confirm").into()),
        None,
        confirm_label.to_string(),
        move |_, window, cx| on_confirm(window, cx),
    );

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            ElementId::Name(format!("{id}-cancel").into()),
            None,
            "Cancel",
            move |_, window, cx| cancel_dismiss(window, cx),
        ))
        .child(if confirm_enabled {
            confirm
        } else {
            confirm.opacity(0.5)
        })
        .into_any_element();

    modal(
        id,
        theme::accent(),
        title,
        body,
        footer,
        move |window, cx| modal_dismiss(window, cx),
        window,
    )
}
