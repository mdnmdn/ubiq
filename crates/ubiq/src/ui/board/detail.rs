//! The panel beside the columns: one task, reported whole.
//!
//! The card says what is read from across a column — the shape, the state, how far along. This
//! says the rest: whose session it is, what its shape means, who is holding it now, and every
//! sub-task with the agent that has it and where that has got to.
//!
//! Two things on it act. A checkbox ticks a sub-task, which is the one place the board changes the
//! work rather than the view of it; and the two buttons at the bottom leave for the agents screen,
//! because a task the user wants to intervene in is a conversation with an agent, and that lives
//! there.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::agents::Task;
use crate::theme;
use crate::ui::agents::{activity_colour, bucket_colour};
use crate::ui::board::status_colour;
use crate::ui::kit::{ghost_button, icon_button, meter, mono, panel, pill, section_label};

pub fn render(app: &AppState, task: &Task, cx: &mut Context<AppState>) -> impl IntoElement {
    let agents = &app.agents;
    let colour = bucket_colour(agents.pulse(task));

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
        .child(body(app, task, cx))
        .child(footer(app, task, cx))
}

fn body(app: &AppState, task: &Task, cx: &mut Context<AppState>) -> AnyElement {
    let agents = &app.agents;
    let colour = bucket_colour(agents.pulse(task));
    let done = task.done();
    let total = task.steps.len();

    let session = match task.session.and_then(|id| agents.session(id)) {
        Some(session) => div()
            .flex()
            .items_center()
            .gap_1p5()
            .child(mono(session.name.clone(), theme::text()).text_size(px(12.)))
            .children(
                session
                    .worktree
                    .then(|| mono("(worktree)", theme::text_faint()).text_size(px(11.))),
            )
            .into_any_element(),
        None => mono("no session yet", theme::warning())
            .text_size(px(12.))
            .into_any_element(),
    };

    let now = match agents.now(task) {
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
        .enumerate()
        .map(|(ix, step)| {
            let owner = step.owner.and_then(|id| agents.agent(id));
            let state = bucket_colour(step.state.bucket());
            let done = step.done();
            let task_id = task.id;
            let key = (task_id as u64) << 32 | ix as u64;

            div()
                .flex()
                .items_start()
                .gap_2p5()
                .py_1()
                .child(
                    div()
                        .id(("board-step", key))
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
                            this.toggle_task_step(task_id, ix, cx)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(if done {
                                    theme::text_muted()
                                } else {
                                    theme::text()
                                })
                                // A ticked sub-task is struck through as well as greyed: the list
                                // is read at a glance, and one signal is not enough for "over".
                                .when(done, |this| this.line_through())
                                .child(SharedString::from(step.title.clone())),
                        )
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
        .child(
            div()
                .text_size(px(17.))
                .text_color(theme::text())
                .child(SharedString::from(task.title.clone())),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .child(tag(task.shape.label(), theme::text_muted()))
                .child(tag(
                    task.status.label().to_uppercase(),
                    status_colour(task.status),
                ))
                .children(task.priority.label().map(|label| {
                    tag(
                        label.to_uppercase(),
                        if label == "high" {
                            theme::danger()
                        } else {
                            theme::text_faint()
                        },
                    )
                }))
                .children(task.blocked().then(|| tag("BLOCKED", theme::danger()))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(fact("Session", session))
                .child(fact(
                    "Shape",
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::text_muted())
                        .child(task.shape.note())
                        .into_any_element(),
                ))
                .child(fact("Now", now)),
        )
        .children((total > 0).then(|| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(meter(task.fraction(), colour)),
                )
                .child(mono(format!("{done}/{total}"), theme::text_muted()).text_size(px(11.5)))
        }))
        .children((total == 0).then(|| {
            div()
                .text_size(px(12.5))
                .text_color(theme::text_faint())
                .child("No sub-tasks yet.")
        }))
        .children(steps)
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

/// The two ways out of a task, both of them onto the agents screen: the graph pointed at whoever
/// is doing it, or that agent's thread.
fn footer(app: &AppState, task: &Task, cx: &mut Context<AppState>) -> impl IntoElement {
    let id = task.id;
    let now = app
        .agents
        .now(task)
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
}
