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

use ubiq_proto::work::{CommentAuthor, Level, Status, StepState, TaskRecord};

use crate::app::AppState;
use crate::state::board::Field;
use crate::state::status::Reach;
use crate::state::work;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::board::{form, status_colour};
use crate::ui::eid2;
use crate::ui::handler;
use crate::ui::kit::{
    ghost_button, icon_button, meter, modal_sized, mono, panel, pill, section_label,
};
use crate::ui::work::{bucket_colour, lifecycle_colour};

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
                .h(px(theme::titlebar_height()))
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
                .child(empty_fields_toggle(app, cx))
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

/// The same report and controls as [`render`], centred over the window instead of hung off the
/// columns — what `board.popup` draws. Both read `board.selected` and `board.editing` straight, so
/// there is no second copy of the task to keep in step: the toggle only moves where the panel is
/// drawn, never what is open in it.
pub fn popup(
    app: &AppState,
    task: &TaskRecord,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let colour = app
        .work(cx)
        .map(|work| bucket_colour(work.pulse(task)))
        .unwrap_or_else(theme::text_faint);
    let title = task.title.clone();
    let entity = cx.entity();

    modal_sized(
        "board-task-modal",
        colour,
        theme::task_panel_width(),
        None,
        &title,
        Some(empty_fields_toggle(app, cx).into_any_element()),
        body(app, task, window, cx),
        footer(app, task, cx).into_any_element(),
        handler(&entity, |this, _, cx| this.close_task_detail(cx)),
        window,
    )
}

/// The eye beside the panel's close: show the fields nobody has filled in, or put them away.
///
/// One answer for every task the window opens and for the sitting only — the state is
/// [`AppState::task_show_empty`], and hidden is where every start finds it. It hides *emptiness*,
/// never content: a field carrying a value draws either way, so nothing the task says can be put
/// out of reach by it.
fn empty_fields_toggle(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let showing = app.task_show_empty;
    icon_button(
        "board-detail-empty-fields",
        if showing {
            IconName::Eye
        } else {
            IconName::EyeOff
        },
        showing,
        cx.listener(|this, _, _, cx| this.toggle_task_empty_fields(cx)),
    )
    .tooltip(move |window, cx| {
        gpui_component::tooltip::Tooltip::new(if showing {
            "Hide empty fields"
        } else {
            "Show empty fields"
        })
        .build(window, cx)
    })
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

    let worktree = task
        .session
        .and_then(|id| work.session(id))
        .is_some_and(|session| session.worktree);

    // Which facts a task has nothing to say about, and whether the panel is saying so. The eye in
    // the bar is one answer for the whole window (T-255); a fact with a value is never subject to
    // it. `Blocks` is derived rather than typed in (M19), so what makes it empty is that no other
    // task names this one as a prerequisite.
    let blocks_any = work
        .tasks
        .iter()
        .any(|other| other.prerequisites.contains(&task.id));
    let drawn = |filled: bool| app.task_show_empty || filled;

    let steps: Vec<AnyElement> = task
        .steps
        .iter()
        .map(|step| {
            let owner = step.owner.and_then(|id| work.agent(id));
            let state = bucket_colour(step.state.bucket());
            let idle = step.state == StepState::Idle;
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
                                .text_size(theme::font(Family::Chrome, Role::Body))
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
                                    mono(app.agent_label(owner).title, theme::text_muted())
                                        .text_size(theme::font(Family::Chrome, Role::Meta))
                                }))
                                .children(owner.map(|_| {
                                    mono("\u{b7}", theme::text_faint())
                                        .text_size(theme::font(Family::Chrome, Role::Meta))
                                }))
                                // Idle is the absence of news, and a list that writes it once per
                                // line has to be read to find the line that is not idle. So a
                                // sub-task nobody has started says nothing, and every dot on the
                                // list is one worth looking at.
                                .children((!idle).then(|| {
                                    div().size(px(6.)).flex_none().rounded_full().bg(state)
                                }))
                                .children((!idle).then(|| {
                                    mono(step.state.label(), state)
                                        .text_size(theme::font(Family::Chrome, Role::Meta))
                                })),
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
        // Said once, over the whole panel, rather than beside each locked field: the reason is the
        // same for all of them, and a control that only greys out leaves the user guessing.
        .children(crate::ui::tasksrc::pull_only_notice(app, task.id))
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
                .children(task.blocked().then(|| tag("BLOCKED", theme::danger())))
                // What the task is, left; how urgent it is, right. The priority row carries no
                // heading: three words in a row with one of them lit need none, and the space it
                // took is what lets the two facts share one line.
                .child(div().flex_1().min_w(px(0.)))
                .child(form::priority_pills(app, task, cx)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(fact("Key", form::key(app, task, window, cx)))
                .children(
                    drawn(filled(task.link.as_deref())).then(|| {
                        fact("Link", form::link(app, task, window, cx)).into_any_element()
                    }),
                )
                .child(fact(
                    "Level",
                    form::level_pill(task, &app.mission_term(cx), cx),
                ))
                // The task panel keeps working for a mission's anchor task and gains exactly one
                // row: the way into the mission's own surface (`mission-proposal.md` §6.1). Drawn
                // only on a task that *is* a mission, because on any other it opens nothing.
                .children(open_mission(app, task, cx))
                .children(
                    drawn(task.parent.is_some())
                        .then(|| fact("Parent", form::parent(app, task, cx)).into_any_element()),
                )
                .child(fact("Kind", form::kind_pills(app, task, cx)))
                .child(fact("Complexity", form::complexity_pills(task, cx)))
                .children(drawn(filled(task.assigned_to.as_deref())).then(|| {
                    fact("Assigned to", form::assigned_to(app, task, window, cx)).into_any_element()
                }))
                .children(
                    drawn(!task.labels.is_empty())
                        .then(|| fact("Labels", form::labels(app, task, cx)).into_any_element()),
                )
                .children(drawn(!task.references.is_empty()).then(|| {
                    fact("References", form::references(app, task, window, cx)).into_any_element()
                }))
                .children(drawn(!task.prerequisites.is_empty()).then(|| {
                    fact("Prerequisites", form::prerequisites(app, task, window, cx))
                        .into_any_element()
                }))
                .children(
                    drawn(blocks_any)
                        .then(|| fact("Blocks", form::blocks(app, task, cx)).into_any_element()),
                )
                .children(drawn(!task.attachments.is_empty()).then(|| {
                    fact("Attachments", form::attachments(app, task, cx)).into_any_element()
                }))
                .children(
                    drawn(task.colour.is_some())
                        .then(|| fact("Colour", form::colour(task, cx)).into_any_element()),
                )
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
                .child(mono(format!("{done}/{total}"), theme::text_muted()))
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(section_label("Sub-tasks"))
                .child(div().flex_1().min_w(px(0.)))
                .children((total > 0).then(|| {
                    mono(format!("{done}/{total}"), theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                })),
        )
        .children((total == 0).then(|| {
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_faint())
                .child("No sub-tasks yet.")
        }))
        .children(steps)
        .child(form::new_step(app, window, cx))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(section_label("Comments"))
                .child(div().flex_1().min_w(px(0.)))
                .children((!task.comments.is_empty()).then(|| {
                    mono(format!("{}", task.comments.len()), theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                })),
        )
        .children(task.comments.is_empty().then(|| {
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_faint())
                .child("No comments yet.")
        }))
        .children(task.comments.iter().map(|comment| {
            let author = match comment.author {
                CommentAuthor::User => "user",
                CommentAuthor::Agent => "agent",
            };
            let when = comment.created_at.format("%Y-%m-%d %H:%M").to_string();
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .py_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(mono(author, theme::text_muted()))
                        .child(mono("\u{b7}", theme::text_faint()))
                        .child(
                            mono(when, theme::text_faint())
                                .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(SharedString::from(comment.text.clone())),
                )
                .into_any_element()
        }))
        .child(form::new_comment(app, window, cx))
        // How the work will be done, and who has it — under the work itself, because both are
        // claims about a task that is already described. The note says what the shape means: the
        // word alone says how the agents are arranged only to somebody who already knows, and
        // there is nothing to say for a task nobody has shaped.
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(section_label("Shape"))
                .child(form::shape_pills(task, cx))
                .children(task.shape.map(|shape| {
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text_muted())
                        .child(shape.note())
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        // Which session is a picker; whether that session is a worktree is a fact
                        // about it, and stays one — reported beside the control rather than
                        // offered as a choice.
                        .child(form::session(app, task, cx))
                        .children(worktree.then(|| {
                            mono("(worktree)", theme::text_faint())
                                .text_size(theme::font(Family::Chrome, Role::Meta))
                        })),
                ),
        )
        .into_any_element()
}

/// One labelled fact, in the two columns the panel reads in.
/// The one row a mission's anchor task gains: *Open mission*, which reveals that mission's side
/// panel in its home region. Named with the project's own word for one (M18).
fn open_mission(
    app: &AppState,
    task: &TaskRecord,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    if task.level != Some(Level::Mission) {
        return None;
    }
    let term = app.mission_term(cx);
    let id = task.id;
    Some(
        fact(
            &term.clone(),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    ghost_button(
                        "board-open-mission",
                        Some(IconName::PanelRight),
                        format!("Open {}", term.to_lowercase()),
                        cx.listener(move |this, _, _, cx| this.open_mission_panel(id, cx)),
                    )
                    .into_any_element(),
                )
                // The board toolbar's mission filter (M27), reached straight off the mission's
                // own card rather than found again in the picker.
                .child(
                    ghost_button(
                        "board-show-only-mission",
                        None,
                        "Show only this mission",
                        cx.listener(move |this, _, _, cx| this.pick_board_mission(Some(id), cx)),
                    )
                    .into_any_element(),
                )
                .into_any_element(),
        )
        .into_any_element(),
    )
}

/// Whether a free-text fact — a link, an assignee — carries anything. Whitespace is not a value,
/// which is the same reading [`form::typed_fact`](super::form) gives it when it decides whether to
/// draw the word for its absence.
fn filled(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty())
}

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
        .child(mono(label, colour).text_size(theme::font(Family::Chrome, Role::Micro)))
}

/// The way out of a task: the thread of whoever is doing it, on the screen over the agents.
///
/// A conversation is what a user who wants to intervene is after, and the columns are where one is
/// had. The graph is the map, and a button that only moved the selection onto it answered a
/// question nobody had asked from here.
///
/// **Two lines, by the task's status** (T-321). The first is the linked agent, when there is one —
/// the shared hexagon, its status and its name, and `Open` — which is the task's lifecycle
/// tracking. The second is what the status asks for next: an assignment, or *Continue with an
/// agent* when the linked one is not running, while the task is to do or under way; *Complete*
/// and *Feedback to an agent* while it is in review; nothing more once it is blocked, done or
/// abandoned. Open plan and Delete stand on every status.
fn footer(app: &AppState, task: &TaskRecord, cx: &mut Context<AppState>) -> impl IntoElement {
    let task_id = task.id;
    let link = app
        .task_agent_link(task_id, cx)
        .map(|(agent, status, reach)| {
            let name = app
                .work(cx)
                .and_then(|work| work.agent(agent))
                .map(|record| app.agent_label(record).title)
                .unwrap_or_else(|| "an agent".into());
            (agent, status, reach, name)
        });
    let reach = link.as_ref().map(|(_, _, reach, _)| *reach);

    let mut actions: Vec<AnyElement> = Vec::new();
    match task.status {
        Status::Backlog | Status::Ready | Status::InProgress => match reach {
            Some(Reach::Live) => {}
            // A linked agent that is not running: carried on where it can be relaunched, a new
            // assignment where it cannot — the button is the same either way.
            Some(Reach::Resumable | Reach::Gone) => actions.push(
                ghost_button(
                    "board-continue-agent",
                    Some(IconName::Play),
                    "Continue with an agent",
                    cx.listener(move |this, _, window, cx| {
                        this.continue_task_with_agent(task_id, window, cx)
                    }),
                )
                .into_any_element(),
            ),
            // Nobody is on this task yet — the way in is the New agent dialog, pre-filled with
            // this task's key, the board and feedback MCPs, and the two checkboxes `T-64` asks for.
            None => actions.push(
                ghost_button(
                    "board-assign-agent",
                    Some(IconName::Plus),
                    "Assign to an agent",
                    cx.listener(move |this, _, window, cx| {
                        this.assign_task_to_agent(task_id, window, cx)
                    }),
                )
                .into_any_element(),
            ),
        },
        Status::InReview => {
            actions.push(
                ghost_button(
                    "board-complete-task",
                    Some(IconName::Check),
                    "Complete",
                    cx.listener(move |this, _, _, cx| this.complete_task(task_id, cx)),
                )
                .into_any_element(),
            );
            actions.push(
                ghost_button(
                    "board-feedback-agent",
                    Some(IconName::Undo),
                    "Feedback to an agent",
                    cx.listener(move |this, _, window, cx| {
                        this.feedback_task_to_agent(task_id, window, cx)
                    }),
                )
                .into_any_element(),
            );
        }
        Status::Blocked | Status::Done | Status::Abandoned => {}
    }

    div()
        .flex()
        .flex_col()
        .flex_none()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .children(link.map(|(agent, status, _, name)| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .pt_2()
                .child(crate::ui::teams::status::status_mark(
                    status,
                    14.0,
                    crate::ui::eid("board-agent-mark", agent),
                ))
                .child(div().flex_1().min_w(px(0.)).child(crate::ui::kit::elided(
                    crate::ui::eid("board-agent-name", agent),
                    name,
                    theme::text(),
                    theme::font(Family::Chrome, Role::Label),
                )))
                .child(mono(status.label(), lifecycle_colour(status.lifecycle)))
                .child(ghost_button(
                    "board-open-chat",
                    Some(IconName::Inbox),
                    "Open",
                    cx.listener(move |this, _, _, cx| this.open_task_chat(agent, cx)),
                ))
        }))
        .child(footer_actions(app, task, actions, cx))
}

/// The footer's second line: the status's own offer, then Open plan, then Delete at the far end.
fn footer_actions(
    app: &AppState,
    task: &TaskRecord,
    actions: Vec<AnyElement>,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .children(actions)
        // A plan belongs to any task carrying a `level` — not to an ordinary task, and not to
        // some special mission subtype. A task with no `level` has no plan and offers none, the
        // affordance's own posture rather than a click the host would refuse.
        .children(task.level.is_some().then(|| {
            let task_id = task.id;
            ghost_button(
                "board-open-plan",
                Some(IconName::FileText),
                "Open plan",
                cx.listener(move |this, _, _, cx| this.open_plan(task_id, cx)),
            )
        }))
        .child(div().flex_1().min_w(px(0.)))
        .child(form::delete(app, cx))
}
