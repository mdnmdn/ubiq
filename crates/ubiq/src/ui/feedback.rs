//! The feedback modal: a title, what kind of thing it is, what happened, and the picture.
//!
//! Drawn on `ui::clone`'s shape — the fields down the body, the send in the footer, dimmed rather
//! than unwired while it would do nothing. Three things dim it: a build with no destination, a
//! blank title, and a report already in flight.
//!
//! The screenshot is shown, not merely reported: what leaves the machine is a picture of the
//! user's own screen, so they see exactly what they are sending and can drop it with one press.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Focusable, Image, ImageFormat, ImageSource, InteractiveElement,
    IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div, img, px, relative,
};
use gpui_component::input::{Input, Textarea};
use ubiq_proto::feedback::FeedbackKind;

use crate::app::AppState;
use crate::state::feedback::FeedbackForm;
use crate::state::{Layer, MenuId};
use crate::theme;
use crate::ui::kit::{
    Picker, PickerStyle, ghost_button, label_block, modal_note, modal_sized, primary_button,
};
use crate::ui::{handler, indexed};

/// The help page this modal claims — rung 1, for the reason `ui::settings::help_page` is: a modal
/// has no context key of its own to be bound by.
pub fn help_page() -> Option<&'static str> {
    Some("feedback")
}

const FEEDBACK_WIDTH: f32 = 520.0;
/// How tall the picture is drawn. Wide enough to recognise the screen, short enough that the
/// fields stay on the same page as it.
const SHOT_HEIGHT: f32 = 180.0;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(feedback) = app.workbench.feedback.as_ref() else {
        return div().into_any_element();
    };
    let view = cx.entity();

    let body = div()
        .id("feedback-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .gap_3()
        .pt_3()
        .children(feedback.error.as_ref().map(|error| {
            div()
                .px_2()
                .py_2()
                .flex_none()
                .bg(theme::danger_soft())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::danger())
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Label))
                .text_color(theme::text())
                .child(error.clone())
        }))
        .child(title_field(app, window, cx))
        .child(kind_picker(app, feedback, cx))
        .child(description_field(app, window, cx))
        .child(shot(feedback, cx))
        .into_any_element();

    modal_sized(
        "feedback-modal",
        theme::accent(),
        FEEDBACK_WIDTH,
        None,
        "Send feedback",
        body,
        footer(app, feedback, cx),
        crate::ui::dismiss(&view, Layer::Feedback, |this, window, cx| {
            this.close_feedback(window, cx)
        }),
        window,
    )
}

fn title_field(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let focused = app
        .feedback_title_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(label_block("Title", "One line. This is what it is called."))
        .child(
            crate::ui::kit::field(theme::border(), focused)
                .h(px(28.))
                .px_2()
                .child(Input::new(&app.feedback_title_input).appearance(false)),
        )
        .into_any_element()
}

fn kind_picker(app: &AppState, feedback: &FeedbackForm, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let labels: Vec<&'static str> = FeedbackKind::ALL.iter().map(|kind| kind.label()).collect();
    let at = FeedbackKind::ALL
        .iter()
        .position(|kind| *kind == feedback.kind)
        .unwrap_or(0);

    let picker = Picker::new("feedback-kind", FeedbackKind::ALL[at].label())
        .items(labels)
        .selected(at)
        .style(PickerStyle::Field)
        .above_modal()
        .open(app.workbench.open_menu == Some(MenuId::FeedbackKind))
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::FeedbackKind, cx)
        }))
        .on_pick(indexed(&view, |this, index, _, cx| {
            if let Some(kind) = FeedbackKind::ALL.get(index).copied() {
                this.pick_feedback_kind(kind, cx);
            }
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)));

    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(label_block("Type", "What kind of thing this is."))
        .child(picker)
        .into_any_element()
}

fn description_field(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let focused = app
        .feedback_description
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(label_block(
            "Description",
            "What you did, what you expected, and what happened instead.",
        ))
        .child(
            crate::ui::kit::field(theme::border(), focused)
                .flex_col()
                .items_stretch()
                .child(
                    div().px_2().py_1p5().cursor_text().child(
                        Textarea::new(&app.feedback_description)
                            .appearance(false)
                            .bordered(false)
                            .w_full()
                            .text_size(theme::font(theme::Family::Chrome, theme::Role::Body)),
                    ),
                ),
        )
        .into_any_element()
}

/// The picture, or the sentence that says why there is none.
fn shot(feedback: &FeedbackForm, cx: &mut Context<AppState>) -> AnyElement {
    let Some(png) = feedback.shot.as_ref() else {
        return div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_2()
            .child(label_block(
                "Screenshot",
                "None taken. A report is worth sending without one.",
            ))
            .into_any_element();
    };

    let image = Arc::new(Image::from_bytes(ImageFormat::Png, png.clone()));
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(label_block(
                    "Screenshot",
                    "The window as it was when you pressed the balloon.",
                ))
                .child(ghost_button(
                    "feedback-drop-shot",
                    None,
                    "Remove",
                    cx.listener(|this, _, _, cx| this.drop_feedback_shot(cx)),
                )),
        )
        .child(
            div()
                .h(px(SHOT_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .p_1()
                .bg(theme::app_bg())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::border())
                .child(
                    img(ImageSource::Image(image))
                        .max_w(relative(1.))
                        .max_h(relative(1.)),
                ),
        )
        .into_any_element()
}

fn footer(app: &AppState, feedback: &FeedbackForm, cx: &mut Context<AppState>) -> AnyElement {
    let offer = &app.workbench.feedback_offer;

    // A report that landed is the whole footer: there is nothing left to send, and the link is
    // the only thing the user still wants from the modal.
    if let Some(sent) = feedback.sent.as_ref() {
        let note = match (&sent.reference, &sent.url) {
            (Some(reference), _) => format!("Sent \u{b7} {reference}"),
            (None, _) => "Sent. Thank you.".to_string(),
        };
        let url = sent.url.clone();
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_between()
            .gap_2()
            .child(modal_note(&note))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(url.map(|url| {
                        ghost_button("feedback-open", None, "Open", move |_, _, cx| {
                            cx.open_url(&url)
                        })
                    }))
                    .child(primary_button(
                        "feedback-done",
                        None,
                        "Done",
                        cx.listener(|this, _, window, cx| this.close_feedback(window, cx)),
                    )),
            )
            .into_any_element();
    }

    let ready = feedback.ready(offer);
    let send = primary_button(
        "feedback-send",
        None,
        match feedback.sending.is_some() {
            true => "Sending\u{2026}",
            false => "Send",
        },
        cx.listener(|this, _, _, cx| this.send_feedback(cx)),
    );

    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_between()
        .gap_2()
        // Where it goes, said before it is sent rather than after. A build with no destination
        // says so here, which is the same sentence that explains the dim button.
        .child(modal_note(offer.channel.label()))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "feedback-cancel",
                    None,
                    "Cancel",
                    cx.listener(|this, _, window, cx| this.close_feedback(window, cx)),
                ))
                .child(match ready {
                    true => send,
                    false => send.opacity(0.5),
                }),
        )
        .into_any_element()
}
