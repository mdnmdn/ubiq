//! The ask dialog: an agent's questions, one tab each, and what goes back.
//!
//! **A tab strip inside a modal**, which nothing else in the window does — and it is the right
//! shape here for the reason the header exists on the wire: an ask carries up to four questions,
//! each with its own options and its own free text, and a modal that stacked them would be a form.
//! One at a time, labelled by the header the agent wrote.
//!
//! **Every option is a card, and the card is the whole answer.** Picking is the card's own click,
//! so the whole row is the target and there is no marker beside the label to keep in step with
//! it — picked reads as the card's own fill and edge instead. "Other" is the last card of every
//! question and is picked the same way; what it opens under itself is the only field on the page
//! that decides whether Confirm is enabled.
//!
//! **An ask that has ended is the same dialog with no controls in it.** Confirmed, chatted away or
//! timed out, the questions and what was chosen are still worth reading, so the body draws the
//! answers where the options were and the footer says how it ended.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::{Input, Textarea};
use ubiq_proto::ask::{AskAnswer, AskClosed, AskQuestion};

use crate::app::{
    AppState, AskFieldNext, AskMoveDown, AskMoveUp, AskToggle, DialogConfirm, SubmitSearch,
};
use crate::state::Layer;
use crate::state::ask::{AskRecord, AskStage};
use crate::theme;
use crate::ui::kit::panel::{Tab, tab_strip};
use crate::ui::kit::{card, ghost_button, label_block, modal_note, modal_sized, mono};
use crate::ui::{eid, eid2, indexed};

/// Wider than a plain modal: an option carries a label, a sentence under it and sometimes a
/// snippet, and three of those stacked in `MODAL_WIDTH` is a column of wrapped fragments.
const ASK_WIDTH: f32 = 560.0;

/// The key context the modal answers to, and the one the component library gives "Other" and
/// "Notes" — both fields the whole modal wraps, so every action registered here is one the
/// dialog sees while either field holds the keyboard too.
const CONTEXT: &str = "Ask";
const FIELD_CONTEXT: &str = "Ask > Input";

/// The keys the dialog answers to that are not already global.
///
/// `enter` (`DialogConfirm`) and `⌘⏎` (`SubmitSearch`) need no binding here — both are already
/// bound at `Workbench` (and, for `⌘⏎`, `Input` too), on `new_agent.rs`'s device, and this
/// module's own `render` intercepts them before they reach anything else. Up and down are this
/// dialog's own and need no field override: the component library's field already claims those
/// inside itself, at the deepest node, so typing in "Other" or "Notes" keeps doing what typing
/// does with no help from here (`ubiq-ui`'s "a key binding against a field must be registered
/// late"). `tab` is bound twice, on `navigator.rs`'s device, because the field claims that one
/// too, for indentation, and the dialog wants it moving focus instead — registered again at
/// `FIELD_CONTEXT`, after `gpui_component::init`, so this one wins the tie.
///
/// **`space` needs the opposite treatment.** Nothing in the field claims it — a plain character
/// has no binding of its own to win the depth tie — so with only `AskToggle` bound at `CONTEXT`,
/// that one keeps firing even while a field holds the keyboard, and a space meant for "Other"'s
/// free text never reaches it (T-97). `NoAction` at `FIELD_CONTEXT` is the fix: it outranks
/// `AskToggle`'s shallower match without binding an action of its own, so the keystroke falls
/// through to the field instead of being consumed.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", AskMoveUp, Some(CONTEXT)),
        KeyBinding::new("down", AskMoveDown, Some(CONTEXT)),
        KeyBinding::new("space", AskToggle, Some(CONTEXT)),
        KeyBinding::new("space", gpui::NoAction, Some(FIELD_CONTEXT)),
        KeyBinding::new("tab", AskFieldNext, Some(CONTEXT)),
        KeyBinding::new("tab", AskFieldNext, Some(FIELD_CONTEXT)),
    ]
}

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some((dialog, record)) = app.open_ask(cx) else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let tab = dialog.tab.min(record.questions.len().saturating_sub(1));
    let live = record.live();

    let tabs: Vec<Tab> = record
        .questions
        .iter()
        .map(|question| Tab::new(question.header.clone()))
        .collect();

    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_3()
        // The strip is flush to the modal's own edges, chrome-style: it separates the questions
        // rather than sitting among them.
        .child(div().flex().flex_none().child(tab_strip(
            "ask-tabs",
            tabs,
            tab,
            indexed(&view, |this, index, window, cx| {
                this.set_ask_tab(index, window, cx)
            }),
            None,
            None,
        )))
        // Its own scroll, under the strip rather than folded into the modal's whole-body one: a
        // tall question — four options, "Other" open under it, notes under that — must not push
        // the tab strip itself off screen with it. `feedback.rs`'s own body carries the same
        // `id` + `flex_1` + `min_h(0)` + `overflow_y_scroll` shape.
        .child(
            div()
                .id("ask-answers")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .gap_3()
                .overflow_y_scroll()
                .children(record.questions.get(tab).map(|question| match live {
                    true => asking(record, question, tab, dialog.cursor, app, window, cx),
                    false => answered(record, question, tab),
                })),
        )
        .into_any_element();

    let modal = modal_sized(
        "ask-modal",
        theme::accent(),
        ASK_WIDTH,
        None,
        "Ask for feedback",
        body,
        footer(record, cx),
        crate::ui::dismiss(&view, Layer::Ask, |this, window, cx| {
            this.close_ask(window, cx)
        }),
        window,
    );

    // The keyboard's own layer over the modal: a cursor for the option list, on `navigator.rs`'s
    // device rather than the modal's own focus, because `kit::modal` owns no focus of its own —
    // see `ui/kit/overlay.rs`'s own doc comment. `DialogConfirm` and `SubmitSearch` are already
    // global (`app::install_key_bindings`); intercepted here rather than left to whatever else
    // might answer them, on `new_agent.rs::confirmable`'s device.
    div()
        .key_context(CONTEXT)
        .track_focus(&app.ask_focus)
        .on_action(cx.listener(|this, _: &AskMoveUp, _, cx| this.move_ask_cursor(-1, cx)))
        .on_action(cx.listener(|this, _: &AskMoveDown, _, cx| this.move_ask_cursor(1, cx)))
        .on_action(cx.listener(|this, _: &AskToggle, _, cx| this.toggle_ask_cursor(cx)))
        .on_action(
            cx.listener(|this, _: &AskFieldNext, window, cx| this.ask_next_field(window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &DialogConfirm, window, cx| {
                this.advance_ask_question(window, cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &SubmitSearch, window, cx| this.confirm_ask_step(window, cx)),
        )
        .child(modal)
        .into_any_element()
}

/// One question, while it can still be answered: what was asked, what may be picked, and the notes
/// every question takes.
fn asking(
    record: &AskRecord,
    question: &AskQuestion,
    at: usize,
    cursor: usize,
    app: &AppState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let other_at = question.options.len();
    let picked_other = record.picked(at, other_at);

    let mut options: Vec<AnyElement> = question
        .options
        .iter()
        .enumerate()
        .map(|(ix, option)| {
            // Single-select only, by the wire's own rule: a preview belongs to *the* choice, and
            // several at once has nowhere to be drawn.
            let preview = match question.multi_select {
                true => None,
                false => option.preview.clone(),
            };
            option_card(
                at,
                ix,
                &option.label,
                &option.description,
                preview.as_deref(),
                record.picked(at, ix),
                ix == cursor,
                cx,
            )
        })
        .collect();

    // Always offered and never one of the agent's own — see `ubiq_proto::ask`.
    options.push(option_card(
        at,
        other_at,
        "Other",
        "Say it in your own words.",
        None,
        picked_other,
        other_at == cursor,
        cx,
    ));

    let other_focused = app
        .ask_other_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let notes_focused = app
        .ask_notes_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_3()
        .child(
            div()
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                .text_color(theme::text())
                .child(question.question.clone()),
        )
        .child(div().flex().flex_col().gap_1p5().children(options))
        // The free-text answer, under the card that turns it on. Drawn only while "Other" is
        // picked: a field that could not be sent is a field that lies about being read.
        .when(picked_other, |this| {
            this.child(
                crate::ui::kit::field(theme::border(), other_focused)
                    .h(px(28.))
                    .px_2()
                    .child(Input::new(&app.ask_other_input).appearance(false)),
            )
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Notes",
                    "Optional. Goes back with this question, whatever you picked.",
                ))
                .child(
                    crate::ui::kit::field(theme::border(), notes_focused)
                        .flex_col()
                        .items_stretch()
                        .child(
                            div().px_2().py_1p5().cursor_text().child(
                                Textarea::new(&app.ask_notes_input)
                                    .appearance(false)
                                    .bordered(false)
                                    .w_full()
                                    .text_size(theme::font(
                                        theme::Family::Chrome,
                                        theme::Role::Body,
                                    )),
                            ),
                        ),
                ),
        )
        .into_any_element()
}

/// One option, as a card that is its own click target.
///
/// **No marker beside the label.** The row is the whole target and the row is the whole answer:
/// picked reads as the card's own fill and edge (`kit::card`'s `selected`), and there is nothing
/// second to keep in step with it. `at_cursor` is the keyboard's own reading — up/down walk it,
/// space and enter act on it — drawn as a hover-toned card the same way a mouse resting on one
/// unpicked would be, so the keyboard's position on the list reads exactly like the mouse's would.
#[allow(clippy::too_many_arguments)]
fn option_card(
    question: usize,
    option: usize,
    label: &str,
    description: &str,
    preview: Option<&str>,
    picked: bool,
    at_cursor: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let mut root = card(
        eid2("ask-option", question, option),
        theme::border(),
        picked,
    );
    if at_cursor && !picked {
        root = root.bg(theme::hover());
    }

    root.px_2()
        .py_1p5()
        .flex()
        .items_start()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .gap_0p5()
                .child(
                    div()
                        .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                        .text_color(theme::text())
                        .child(label.to_string()),
                )
                .when(!description.trim().is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(theme::font(theme::Family::Chrome, theme::Role::Label))
                            .text_color(theme::text_muted())
                            .child(description.to_string()),
                    )
                })
                .children(preview.map(|preview| {
                    div()
                        .px_2()
                        .py_1()
                        .bg(theme::app_bg())
                        .border_l(px(theme::accent_edge()))
                        .border_color(theme::border())
                        .child(mono(preview.to_string(), theme::text_muted()).text_size(
                            theme::font(theme::Family::Conversation, theme::Role::Label),
                        ))
                })),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_ask_option(option, cx)))
        .into_any_element()
}

/// One question, once the ask has ended: what was asked, and what was said about it.
fn answered(record: &AskRecord, question: &AskQuestion, at: usize) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .child(
            div()
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Body))
                .text_color(theme::text())
                .child(question.question.clone()),
        )
        .child(said(record, at))
        .into_any_element()
}

/// What one question was answered with, as a paragraph. Shared by the read-only dialog and the
/// transcript entry, which say the same thing at two sizes.
pub fn said(record: &AskRecord, at: usize) -> AnyElement {
    let lines = said_lines(record, at);

    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(lines.into_iter().map(|line| {
            div()
                .text_size(theme::font(theme::Family::Chrome, theme::Role::Label))
                .text_color(theme::text_muted())
                .child(line)
        }))
        .into_any_element()
}

/// What [`said`] draws for one question, line by line: the answer where there is one, and
/// otherwise the single sentence that says why there is not.
fn said_lines(record: &AskRecord, at: usize) -> Vec<String> {
    match record
        .answers()
        .and_then(|answers| answers.iter().find(|answer| answer.question == at))
    {
        Some(answer) => answer_lines(answer),
        None => vec![match &record.stage {
            AskStage::Chatted => {
                "Nothing picked \u{2014} the user chose to talk about it instead.".to_string()
            }
            AskStage::Ended(AskClosed::Timeout) => "Nobody answered in time.".to_string(),
            AskStage::Ended(AskClosed::Gone) => "The conversation went away.".to_string(),
            _ => "Nothing answered.".to_string(),
        }],
    }
}

/// One answer, as the lines it is worth reading: the picks, what was written under "Other", and
/// the notes.
fn answer_lines(answer: &AskAnswer) -> Vec<String> {
    let mut lines = Vec::new();
    if !answer.chosen.is_empty() {
        lines.push(answer.chosen.join(", "));
    }
    if let Some(other) = &answer.other {
        lines.push(format!("Other \u{b7} {other}"));
    }
    if let Some(notes) = &answer.notes {
        lines.push(format!("Notes \u{b7} {notes}"));
    }
    if lines.is_empty() {
        lines.push("Nothing answered.".to_string());
    }
    lines
}

/// The footer: how the ask stands, and the two things that end it.
fn footer(record: &AskRecord, cx: &mut Context<AppState>) -> AnyElement {
    if !record.live() {
        let note = match &record.stage {
            AskStage::Answered(_) => "Answered.",
            AskStage::Chatted => "Ended \u{2014} you chose to talk about it.",
            AskStage::Ended(AskClosed::Timeout) => "Ended \u{2014} nobody answered in time.",
            AskStage::Ended(AskClosed::Gone) => "Ended \u{2014} the conversation is gone.",
            AskStage::Waiting => "",
        };
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_between()
            .gap_2()
            .child(modal_note(note))
            .child(ghost_button(
                "ask-done",
                None,
                "Close",
                cx.listener(|this, _, window, cx| this.close_ask(window, cx)),
            ))
            .into_any_element();
    }

    let ready = record.ready();
    let confirm = crate::ui::kit::primary_button(
        "ask-confirm",
        None,
        "Confirm",
        cx.listener(|this, _, window, cx| this.confirm_ask(window, cx)),
    );
    // `AskRecord::answered` rather than a count of its own: the note says what the dim Confirm is
    // waiting for, and two readings of "answered" would let it claim there is nothing left.
    let unanswered = (0..record.questions.len())
        .filter(|at| !record.answered(*at))
        .count();

    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_between()
        .gap_2()
        // What the dim Confirm is waiting for, said rather than left to be guessed at.
        .child(modal_note(&match unanswered {
            0 => "The agent is waiting on this.".to_string(),
            1 => "One question still to answer.".to_string(),
            many => format!("{many} questions still to answer."),
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                // Not a cancel: it ends the ask and tells the agent so, which is why it is worded
                // as what the user is about to do instead.
                .child(ghost_button(
                    "ask-chat",
                    None,
                    "Chat about this",
                    cx.listener(|this, _, window, cx| this.chat_about_ask(window, cx)),
                ))
                .child(match ready {
                    true => confirm,
                    false => confirm.opacity(0.5),
                }),
        )
        .into_any_element()
}

/// What [`transcript_entry`] costs whatever it is saying: the marker, the button row, the
/// padding, and the gaps between the three.
const ENTRY_CHROME: f32 = 64.0;
/// One line inside it — a question's header, a pick list, an `Other ·` or a `Notes ·` — and the
/// gap under it.
const ENTRY_LINE: f32 = 34.0;

/// How many lines [`transcript_entry`] draws: a header per question, and once the ask has ended
/// the lines [`said`] puts under each of them.
pub fn entry_lines(record: &AskRecord) -> usize {
    let live = record.live();
    (0..record.questions.len())
        .map(|at| match live {
            true => 1,
            false => 1 + said_lines(record, at).len(),
        })
        .sum()
}

/// How tall [`transcript_entry`] will draw.
///
/// Computed rather than measured because the transcript is a virtual list: it asks every row how
/// tall it is — including the rows it has never drawn — so there is nothing laid out to measure
/// at the moment the number is wanted. A count of the questions alone was the same number before
/// and after the ask ended, which clipped every answer that carried an `Other ·` or a `Notes ·`
/// line into a row reserved for its header alone.
pub fn entry_height(record: &AskRecord) -> gpui::Pixels {
    px(ENTRY_CHROME + ENTRY_LINE * entry_lines(record) as f32)
}

/// The entry an ask leaves in the transcript, and the button that opens its dialog.
///
/// Drawn by `ui::conversation` rather than here for the reason every other row of that transcript
/// is drawn there; it lives in this module because what it says is this dialog's, and the two
/// would drift if the wording were written twice.
pub fn transcript_entry(
    agent_id: ubiq_proto::work::AgentId,
    record: &AskRecord,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let ask_id = record.ask_id;
    let live = record.live();
    // Warning while it blocks the harness — the same visual language `ui::conversation::permission`
    // draws its own "NEEDS YOU" row in, because an unanswered ask is the same kind of thing a
    // reader has to notice. Answered, chatted away, timed out or the conversation gone, it reads
    // as an ordinary entry again: back to the neutral accent every ask started life in, before
    // this row existed to say it still needed someone.
    let (marker, bg, edge) = match live {
        true => ("ASK FOR FEEDBACK", theme::warning_soft(), theme::warning()),
        false => ("ASKED FOR FEEDBACK", theme::accent_soft(), theme::accent()),
    };

    // One block per question: its header, and — once the ask has ended — what was said about it.
    let questions: Vec<AnyElement> = record
        .questions
        .iter()
        .enumerate()
        .map(|(at, question)| {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    mono(question.header.clone(), theme::text())
                        .text_size(theme::font(theme::Family::Conversation, theme::Role::Label)),
                )
                .when(!live, |this| this.child(said(record, at)))
                .into_any_element()
        })
        .collect();

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_2()
        .bg(bg)
        .border_l(px(theme::accent_edge()))
        .border_color(edge)
        .child(
            mono(marker, edge)
                .text_size(theme::font(theme::Family::Conversation, theme::Role::Micro)),
        )
        .child(div().flex().flex_col().gap_1p5().children(questions))
        .child(div().flex().items_center().gap_2().child(ghost_button(
            eid("ask-open", ask_id),
            None,
            match live {
                true => "Answer\u{2026}",
                false => "Review",
            },
            cx.listener(move |this, _, window, cx| this.show_ask(agent_id, ask_id, window, cx)),
        )))
        .into_any_element()
}
