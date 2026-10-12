//! The note dialog — a textarea and Send.
//!
//! One dialog for every place a mission asks the user for words before it acts
//! ([`crate::state::mission::NoteKind`]): *Reject…* and *Fail…* on a task, whose text is recorded
//! as a comment and delivered by the one rule (a live worker gets it as its next turn, a gone one
//! shows Retry with it attached), and *Replan…*, *Redo breakdown…* and the mission-level
//! *Reject…*, whose text is the note to the planner.

use gpui::{
    AnyElement, Context, Focusable as _, IntoElement, ParentElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::input::Textarea;

use crate::app::AppState;
use crate::state::Layer;
use crate::state::mission::{MissionNote, NoteKind};
use crate::theme;
use crate::ui::kit::{field, ghost_button, modal, modal_note, primary_button};

pub fn render(
    app: &AppState,
    note: MissionNote,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let view = cx.entity();
    let focused = app
        .mission_note_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let empty = app.mission_note_input.read(cx).value().trim().is_empty();
    let needs_text = matches!(
        note.kind,
        NoteKind::Reject(_) | NoteKind::Fail(_) | NoteKind::RejectMission(_)
    );

    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(note.kind.hint()))
        .child(
            field(theme::border(), focused)
                .flex_col()
                .items_stretch()
                .child(
                    div().px_2().py_1p5().cursor_text().child(
                        Textarea::new(&app.mission_note_input)
                            .appearance(false)
                            .bordered(false)
                            .w_full()
                            .text_size(theme::font(theme::Family::Chrome, theme::Role::Body)),
                    ),
                ),
        )
        .into_any_element();

    let footer = div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .justify_end()
        .gap_2()
        .child(ghost_button(
            "mission-note-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.close_mission_note(cx)),
        ))
        .child(
            primary_button(
                "mission-note-send",
                None,
                note.kind.send_label(),
                cx.listener(|this, _, window, cx| this.send_mission_note(window, cx)),
            )
            .when_some((needs_text && empty).then_some(()), |button, _| {
                button.opacity(0.5)
            }),
        )
        .into_any_element();

    modal(
        "mission-note",
        match note.kind {
            NoteKind::Reject(_) | NoteKind::Fail(_) | NoteKind::RejectMission(_) => {
                theme::warning()
            }
            _ => theme::accent(),
        },
        note.kind.title(),
        body,
        footer,
        crate::ui::dismiss(&view, Layer::MissionNote, |this, _, cx| {
            this.close_mission_note(cx)
        }),
        window,
    )
}
