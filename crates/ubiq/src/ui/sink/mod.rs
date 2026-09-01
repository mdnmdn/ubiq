//! The kitchen sink screen: the application's own test bench.
//!
//! **It is the one centre screen with no project behind it.** Every page in it is drawn from a
//! fixture in [`crate::state::sink`] or from the theme itself, so the sink opens on a first run
//! with an empty catalogue and looks the same in every window. That is what it is for: a control is
//! looked at here before a screen is built out of it, and a palette change is checked against
//! everything at once.
//!
//! Two kinds of page, and the strip along the top selects between them: four documents, each drawn
//! by the viewer its name implies — which is how a special viewer is exercised without opening a
//! file — and the drawn pages, which are the style reference, the file picker, and the two
//! settings layouts composed from the kit.
//!
//! The modal the style reference raises is drawn here rather than in [`style`], because a modal is
//! the screen's and not a section's: exactly one may be up, and where it is asked for is not where
//! it is painted.

pub mod docs;
pub mod files;
pub mod project;
pub mod settings;
pub mod style;

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::input::Input;

use crate::app::AppState;
use crate::state::sink::{SinkModal, SinkSection};
use crate::theme;
use crate::ui::indexed;
use crate::ui::kit::panel::tab_strip;
use crate::ui::kit::{Tab, ghost_button, modal, modal_note, mono, primary_button};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let section = app.sink.section;
    let active = SinkSection::all()
        .iter()
        .position(|candidate| *candidate == section)
        .unwrap_or(0);

    let tabs: Vec<Tab> = SinkSection::all()
        .iter()
        .map(|section| Tab::new(section.label()))
        .collect();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(tab_strip(
            "sink-sections",
            tabs,
            active,
            indexed(&view, |this, index, _, cx| {
                let section = SinkSection::all()
                    .get(index)
                    .copied()
                    .unwrap_or(SinkSection::Editor);
                this.set_sink_section(section, cx);
            }),
            None,
            Some(
                mono(section.note(), theme::text_faint())
                    .text_size(px(11.))
                    .into_any_element(),
            ),
        ))
        .child(match section {
            SinkSection::Style => style::render(app, window, cx),
            SinkSection::Files => files::render(app, cx),
            SinkSection::Settings => settings::render(app, window, cx),
            SinkSection::Project => project::render(app, window, cx),
            // Every other page is one document, drawn by the viewer its name implies.
            other => match other.doc() {
                Some(doc) => docs::render(app, doc, cx),
                None => div().flex_1().into_any_element(),
            },
        })
        .children(app.sink.modal.map(|which| raised(app, which, window, cx)))
        // The picker belongs to the window rather than to this page — one may be up at a time —
        // but it is painted from here, like the modal above it and for the same reason: where a
        // dialog is asked for is not where it is drawn.
        .children(
            app.file_picker
                .as_ref()
                .map(|picker| crate::ui::file_picker::render(app, picker, window, cx)),
        )
        .into_any_element()
}

/// The modal that is up, in the shape the kit draws one in.
///
/// The three differ in exactly what a real modal differs in: what its edge says, what its body
/// holds, and whether its confirming button is the accent or the danger colour. Nothing behind any
/// of them happens — this is the sink, and a fixture that pretended to close a pane would be a lie,
/// so both buttons dismiss and neither claims anything.
fn raised(
    app: &AppState,
    which: SinkModal,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let edge = match which {
        SinkModal::Danger => theme::danger(),
        _ => theme::accent(),
    };

    let body = match which {
        // The one that asks for something rather than about something, so it carries a field.
        SinkModal::Form => div()
            .flex()
            .flex_col()
            .gap_3()
            .pt_3()
            .child(modal_note(which.note()))
            .child(style::labelled(
                "Name",
                style::framed_active(
                    theme::border(),
                    style::input_on(&app.sink_modal_input, window, cx),
                )
                .h(px(30.))
                .items_center()
                .child(Input::new(&app.sink_modal_input).appearance(false))
                .into_any_element(),
            ))
            .into_any_element(),
        _ => div()
            .pt_3()
            .child(modal_note(which.note()))
            .into_any_element(),
    };

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "sink-modal-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_sink_modal(cx)),
        ))
        .child(
            primary_button(
                "sink-modal-confirm",
                None,
                which.confirm(),
                cx.listener(|this, _, _, cx| this.close_sink_modal(cx)),
            )
            .when(which == SinkModal::Danger, |button| {
                button.bg(theme::danger())
            }),
        )
        .into_any_element();

    modal(
        "sink-modal",
        edge,
        which.title(),
        body,
        footer,
        crate::ui::handler(&view, |this, _, cx| this.close_sink_modal(cx)),
        window,
    )
}
