//! The panel beside the columns: one task, reported whole and edited in place.
//!
//! The card says what is read from across a column — the shape, the state, how far along. This
//! says the rest: whose session it is, what its shape means, who is holding it now, and every
//! sub-task with the agent that has it and where that has got to.
//!
//! This is the report; the controls that change a task are [`super::form`], drawn into the same
//! column. Everything either of them does asks the host and waits, so what is on screen is always
//! the task the host last confirmed.
//!
//! The two buttons at the bottom leave for the screens over the agents, because a task the user
//! wants to intervene in is a conversation with an agent, and that lives there.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::work::TaskRecord;

use crate::app::AppState;
use crate::state::board::Field;
use crate::state::work;
use crate::theme;
use crate::ui::board::{form, status_colour};
use crate::ui::eid2;
use crate::ui::kit::{ghost_button, icon_button, meter, mono, panel, pill, section_label};
use crate::ui::work::{activity_colour, bucket_colour};

pub fn render(
    app: &AppState,
    task: &TaskRecord,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let colour = app
        .work(cx)
        .map(|work| bucket_colour(work.pulse(task)))
        .unwrap_or_else(theme::text_faint);

    panel()
        .child(
            div()
                .h(px(theme::TITLEBAR_HEIGHT))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .bg(theme::pane_bg())
                .border_b_1()
                .border_color(theme::border())
                .child(div().size(px(8.)).flex_none().rounded_full().bg(colour))
                .child(section_label("Task"))
                .child(div().flex_1().min_w(px(0.)))
                .child(icon_button(
                    "board-detail-close",
                    IconName::Close,
                    false,
                    cx.listener(|this, _, _, cx| this.close_task_detail(cx)),
                )),
        )
        .child(body(app, task, window, cx))
        .child(footer(app, task, cx))
}

fn body(
    app: &AppState,
    task: &TaskRecord,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(work) = app.work(cx) else {
        return div().into_any_element();
    };
    let colour = bucket_colour(work.pulse(task));
    let done = task.done();
    let total = task.steps.len();

    // Which session is a picker; whether that session is a worktree is a fact about it, and stays a
    // fact — the panel reports it beside the control rather than offering it as a choice.
    let worktree = task
        .session
        .and_then(|id| work.session(id))
        .is_some_and(|session| session.worktree);

    let now = match work.now(task) {
        Some(agent) => {
            let agent_colour = activity_colour(agent.activity);
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .size(px(6.))
                        .flex_none()
                        .rounded_full()
                        .bg(agent_colour),
                )
                .child(mono(agent.name.clone(), agent_colour).text_size(px(12.)))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.))
                        .text_color(theme::text_muted())
                        .child(SharedString::from(format!("\u{2014} {}", agent.note))),
                )
                .into_any_element()
        }
        None => mono("nobody has started this", theme::text_faint())
            .text_size(px(12.))
            .into_any_element(),
    };

    let steps: Vec<AnyElement> = task
        .steps
        .iter()
        .map(|step| {
            let owner = step.owner.and_then(|id| work.agent(id));
            let state = bucket_colour(step.state.bucket());
            let done = step.done();
            let task_id = task.id;
            let step_id = step.id;
            let renaming = app
                .board(cx)
                .is_some_and(|board| board.is_editing(Field::Step(step_id)));

            div()
                .flex()
                .items_start()
                .gap_2p5()
                .py_1()
                .child(
                    div()
                        .id(eid2("board-step", task_id, step_id))
                        .size(px(16.))
                        .mt(px(2.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .border_color(if done { theme::success() } else { state })
                        .cursor_pointer()
                        .when(done, |this| this.bg(theme::success()))
                        .hover(|this| this.border_color(theme::accent()))
                        .children(done.then(|| {
                            Icon::new(IconName::Check)
                                .with_size(Size::XSmall)
                                .text_color(theme::on_accent())
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_task_step(task_id, step_id, cx)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(if renaming {
                            form::step_field(app, window, cx)
                        } else {
                            div()
                                .id(eid2("board-step-title", task_id, step_id))
                                .text_size(px(13.))
                                .text_color(if done {
                                    theme::text_muted()
                                } else {
                                    theme::text()
                                })
                                // A ticked sub-task is struck through as well as greyed: the list
                                // is read at a glance, and one signal is not enough for "over".
                                .when(done, |this| this.line_through())
                                .cursor_text()
                                .hover(|this| this.text_color(theme::accent()))
                                .child(SharedString::from(step.title.clone()))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.begin_task_edit(Field::Step(step_id), window, cx)
                                }))
                                .into_any_element()
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .children(owner.map(|owner| {
                                    mono(owner.name.clone(), theme::text_muted()).text_size(px(11.))
                                }))
                                .children(owner.map(|_| {
                                    mono("\u{b7}", theme::text_faint()).text_size(px(11.))
                                }))
                                .child(div().size(px(6.)).flex_none().rounded_full().bg(state))
                                .child(mono(step.state.label(), state).text_size(px(11.))),
                        ),
                )
                .child(form::step_controls(app, task, step_id, cx))
                .into_any_element()
        })
        .collect();

    div()
        .id("board-detail")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .px_3()
        .py_3()
        .gap_3()
        .overflow_y_scroll()
        .children(form::refusal(app))
        .child(form::title(app, task, window, cx))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                // The status is drawn and not offered: a column is a stage, and a card only ever
                // changes column by being moved. `BLOCKED` is derived from the steps, so there is
                // nothing to offer there either.
                .child(tag(
                    task.status.label().to_uppercase(),
                    status_colour(task.status),
                ))
                .children(task.blocked().then(|| tag("BLOCKED", theme::danger()))),
        )
        .child(form::pills(task, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(fact(
                    "Session",
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(form::session(app, task, cx))
                        .children(
                            worktree.then(|| {
                                mono("(worktree)", theme::text_faint()).text_size(px(11.))
                            }),
                        )
                        .into_any_element(),
                ))
                .child(fact("Now", now)),
        )
        .child(form::description(app, task, window, cx))
        .children((total > 0).then(|| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(meter(work::fraction(task), colour)),
                )
                .child(mono(format!("{done}/{total}"), theme::text_muted()).text_size(px(11.5)))
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(section_label("Sub-tasks"))
                .child(div().flex_1().min_w(px(0.)))
                .children((total > 0).then(|| {
                    mono(format!("{done}/{total}"), theme::text_faint()).text_size(px(11.))
                })),
        )
        .children((total == 0).then(|| {
            div()
                .text_size(px(12.5))
                .text_color(theme::text_faint())
                .child("No sub-tasks yet.")
        }))
        .children(steps)
        .child(form::new_step(app, window, cx))
        .into_any_element()
}

/// One labelled fact, in the two columns the panel reads in.
fn fact(label: &str, value: AnyElement) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap_3()
        .py_1()
        .child(
            div()
                .w(px(76.))
                .flex_none()
                .pt(px(1.))
                .child(section_label(label)),
        )
        .child(div().flex_1().min_w(px(0.)).child(value))
}

/// A word about the task, in the colour of whatever it is a word about.
fn tag(label: impl Into<SharedString>, colour: gpui::Rgba) -> impl IntoElement {
    pill(colour)
        .h(px(22.))
        .px_2()
        .child(mono(label, colour).text_size(px(10.5)))
}

/// The two ways out of a task, one onto each screen over the agents: the graph pointed at whoever
/// is doing it, or that agent's thread.
fn footer(app: &AppState, task: &TaskRecord, cx: &mut Context<AppState>) -> impl IntoElement {
    let id = task.id;
    let now = app
        .work(cx)
        .and_then(|work| work.now(task))
        .map(|agent| (agent.id, agent.name.clone()));

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(ghost_button(
            "board-show-in-graph",
            Some(IconName::Network),
            "Show in graph",
            cx.listener(move |this, _, _, cx| this.show_task_in_graph(id, cx)),
        ))
        .children(now.map(|(agent, name)| {
            ghost_button(
                "board-open-chat",
                Some(IconName::Inbox),
                format!("Open {name}'s chat"),
                cx.listener(move |this, _, _, cx| this.open_task_chat(agent, cx)),
            )
        }))
        .child(div().flex_1().min_w(px(0.)))
        .child(form::delete(app, cx))
}
