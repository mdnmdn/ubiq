//! The mission side panel — the cockpit.
//!
//! Top to bottom: the header, the phase stepper (display only, no click-to-move), the progress bar
//! with the driver switch (chrome, never folds), then the **sections** — *Needs you*, *Running*,
//! *Next* (manual driver only), *Documents* — each opened or shut on its own through
//! [`kit::disclosure`], and the guidance composer fixed at the foot. In **Breakdown** the middle
//! shows the drafted task list and **Start mission** instead of Running and Next.
//!
//! **The section content scrolls; the chrome around it does not.** A section's open/shut state is
//! remembered per mission (`MissionView::shut_sections`) — UI-local bookkeeping, so two side
//! panels open on different missions never share a shape.
//!
//! Everything drawn is the parent task, its subtasks and the host's `MissionSnapshot`; every button
//! is one message (`MissionControl`, `MissionTask`, `AddGuidance`, `SetMissionField`).

use chrono::Utc;
use gpui::{
    AnyElement, Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder, px,
    relative,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{
    Driver, MissionAction, MissionSnapshot, MissionTaskAction, NeedsYou, Phase,
};
use ubiq_proto::work::{Activity, Status, TaskRecord};

use crate::app::AppState;
use crate::state::mission::{MissionSection, NoteKind};
use crate::state::work::WorkProjection;
use crate::theme::{self, Family, Role};
use crate::ui::empty;
use crate::ui::kit::{
    UbiqIcon, check_box, choice_pill, disclosure, elided, ghost_button, hex_mark, icon_button,
    mono, panel, primary_button, section_label, state_chip,
};
use crate::ui::mission::full::doc_row;
use crate::ui::work::work_state_colour;
use crate::ui::{eid, eid2};

/// The five steps of the stepper. `Paused` sits on *Running* and `Abandoned` on none of them —
/// [`Phase::step`] says which.
pub(super) const STEPS: [Phase; 5] = [
    Phase::Planning,
    Phase::Breakdown,
    Phase::Running,
    Phase::Review,
    Phase::Done,
];

pub fn render(
    app: &AppState,
    task_id: TaskId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let (Some(work), Some(mission)) = (app.work(cx), app.mission(task_id, cx)) else {
        return empty::empty_page(
            "No mission here",
            "This window is not holding the project this panel was opened for.",
            UbiqIcon::ModeTasks,
            None,
        )
        .into_any_element();
    };
    let Some(task) = work.task(task_id) else {
        // The snapshot outlives a window's projection of the task for as long as one `WorkList` is
        // in flight, so this is a frame, not a state.
        return empty::empty_page(
            "Loading",
            "The task this mission is has not arrived yet.",
            UbiqIcon::ModeTasks,
            None,
        )
        .into_any_element();
    };

    let term = app.mission_term(cx);
    let view = app.mission_view(cx).cloned().unwrap_or_default();
    let breakdown = mission.phase == Phase::Breakdown;
    let manual = mission.settings.driver == Driver::Manual;

    let mut sections = div()
        .id(eid("mission-panel-scroll", task_id))
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .child(needs_you_section(
            app,
            work,
            task_id,
            mission,
            view.section_open(task_id, MissionSection::NeedsYou),
            cx,
        ));
    if breakdown {
        sections = sections.child(draft_list(work, task_id, mission, cx));
    } else {
        sections = sections.child(running(
            app,
            work,
            task_id,
            mission,
            view.section_open(task_id, MissionSection::Running),
            cx,
        ));
        if manual {
            sections = sections.child(next(
                work,
                task_id,
                mission,
                view.section_open(task_id, MissionSection::Next),
                cx,
            ));
        }
    }
    sections = sections.child(documents(
        task_id,
        mission,
        view.section_open(task_id, MissionSection::Documents),
        cx,
    ));

    panel()
        .child(header(app, task_id, work, &term, task, mission, cx))
        .child(phase_line(mission))
        .child(progress(work, task_id, mission, cx))
        .child(sections)
        .child(guidance(app, task_id, view.guidance_to_running, window, cx))
        .children(super::menu::overlay(app, task_id, cx))
        .into_any_element()
}

/// One section's fold: the chevron bar, and the body only while it is open.
fn fold(
    task_id: TaskId,
    section: MissionSection,
    title: &str,
    summary: impl IntoElement,
    open: bool,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    disclosure(
        eid2("mission-section", task_id, section.label()),
        title,
        summary,
        open,
        cx.listener(move |this, _, _, cx| this.toggle_mission_section(task_id, section, cx)),
    )
}

// ── the header ──────────────────────────────────────────────────────

/// The mission term, the key, the title and the hexagon, then the two controls.
///
/// `⤢` opens the full view — the document tab in IDE mode, the modal everywhere else. `⋯` is the
/// mission's own menu, `ui::mission::menu`, whose rows are `state::mission::MissionMenuRow`.
fn header(
    app: &AppState,
    task_id: TaskId,
    work: &WorkProjection,
    term: &str,
    task: &TaskRecord,
    mission: &MissionSnapshot,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let menu_open = app
        .workbench
        .mission_menu
        .is_some_and(|(open, _)| open == task_id);

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
        .child(mission_hex(
            work,
            mission,
            eid("mission-hex", task_id),
            14.,
            cx,
        ))
        .child(section_label(term))
        .when_some(task.key.clone(), |row, key| {
            row.child(mono(key, theme::text_muted()))
        })
        .child(elided(
            eid("mission-title", task_id),
            task.title.clone(),
            theme::text(),
            theme::font(Family::Chrome, Role::Label),
        ))
        .child(
            icon_button(
                eid("mission-expand", task_id),
                IconName::Maximize,
                false,
                cx.listener(move |this, _, _, cx| this.open_mission_full(task_id, cx)),
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Open the full view").build(window, cx)
            }),
        )
        .child(
            icon_button(
                eid("mission-more", task_id),
                IconName::Ellipsis,
                menu_open,
                cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                    let at = event.position();
                    this.open_mission_menu(task_id, (at.x.into(), at.y.into()), cx);
                }),
            )
            .tooltip(|window, cx| gpui_component::tooltip::Tooltip::new("More").build(window, cx)),
        )
}

/// The mission's hexagon.
///
/// **The outer ring is the worst thing happening among its live agents** —
/// `ui::work::worst_bucket`, the one rule a session group's left bar already follows. A mission
/// nobody is on falls back to the phase's own colour. **The inner mark is the attention flag**:
/// anything in *Needs you* draws `NeedsYou` in the dictionary's own colour and pulses.
pub fn mission_hex(
    work: &WorkProjection,
    mission: &MissionSnapshot,
    id: gpui::ElementId,
    side: f32,
    cx: &gpui::App,
) -> AnyElement {
    let members: Vec<_> = mission
        .agents
        .iter()
        .filter_map(|agent| work.agent(*agent))
        .collect();
    let ring = match members.is_empty() {
        true => phase_colour(mission.phase),
        false => crate::ui::work::bucket_colour(crate::ui::work::worst_bucket(
            members.iter().map(|agent| agent.activity.bucket()),
        )),
    };
    let attention = !mission.needs_you.is_empty();
    let fill = match attention {
        true => crate::ui::work::doing_colour(crate::state::status::Doing::NeedsYou),
        false => theme::fade(phase_colour(mission.phase), 0.25),
    };
    hex_mark(id, ring, Some(fill), side, attention && !cx.reduce_motion())
}

// ── the phase line ──────────────────────────────────────────────────

/// The five-step stepper — **display only**. A phase moves through the gates (Approve plan, Start
/// mission, …) and the `⋯` menu, never by clicking a dot. A paused mission lights *Running* and
/// says so; an abandoned one is drawn as one chip, having walked no path.
pub(super) fn phase_steps(phase: Phase) -> impl IntoElement {
    let reached = phase.step();
    let steps: Vec<AnyElement> = STEPS
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let passed = reached.is_some_and(|at| i <= at);
            let current = reached == Some(i);
            let colour = match passed {
                true => phase_colour(phase),
                false => theme::text_faint(),
            };
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(i > 0, |row| {
                    row.child(div().w(px(8.)).h(px(1.)).bg(theme::border()))
                })
                .child(
                    div()
                        .size(px(if current { 9. } else { 7. }))
                        .flex_none()
                        .rounded_full()
                        .bg(colour),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                        .text_color(match current {
                            true => theme::text(),
                            false => theme::text_faint(),
                        })
                        .child(step.label()),
                )
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .children(steps)
}

fn phase_line(mission: &MissionSnapshot) -> impl IntoElement {
    let phase = mission.phase;
    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(match phase {
            Phase::Abandoned => {
                state_chip(phase.label(), phase_colour(phase), 1.0).into_any_element()
            }
            _ => phase_steps(phase).into_any_element(),
        })
        .when(phase == Phase::Paused, |row| {
            row.child(state_chip(phase.label(), phase_colour(phase), 1.0))
        })
}

/// The driver switch, usable mid-run: `Auto` or `Manual`, one `SetMissionField`.
pub(super) fn driver_switch(
    task_id: TaskId,
    driver: Driver,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(mono("Driver", theme::text_faint()))
        .child(choice_pill(
            eid("mission-driver-auto", task_id),
            "Auto",
            driver == Driver::Auto,
            cx.listener(move |this, _, _, cx| this.set_mission_driver(task_id, Driver::Auto, cx)),
        ))
        .child(choice_pill(
            eid("mission-driver-manual", task_id),
            "Manual",
            driver == Driver::Manual,
            cx.listener(move |this, _, _, cx| this.set_mission_driver(task_id, Driver::Manual, cx)),
        ))
}

// ── the progress bar ────────────────────────────────────────────────

/// One segmented bar over the mission's subtasks in the work-state colours, `done/total` and the
/// driver switch beside it.
///
/// **A segment and its count are one control**: either opens the full view's Tasks tab filtered
/// to that state, which is the whole reason both surfaces share one `MissionView`.
fn progress(
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let counts = work.work_state_counts(task_id);
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    let done = work
        .children_of(task_id)
        .filter(|task| task.status == Status::Done)
        .count();

    let head = div()
        .flex()
        .items_center()
        .gap_2()
        .child(mono(format!("{done}/{total}"), theme::text_muted()))
        .child(div().flex_1().min_w(px(0.)))
        .child(driver_switch(task_id, mission.settings.driver, cx));

    if total == 0 {
        return div()
            .px_3()
            .pb_2()
            .flex()
            .flex_none()
            .flex_col()
            .gap_1p5()
            .child(head)
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text_faint())
                    .child("No tasks yet."),
            )
            .into_any_element();
    }

    let segments: Vec<AnyElement> = counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(state, n)| {
            let state = *state;
            div()
                .id(eid2("mission-seg", task_id, state.label()))
                .h_full()
                .w(relative(*n as f32 / total as f32))
                .bg(work_state_colour(state))
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_mission_tasks(task_id, state, cx)),
                )
                .into_any_element()
        })
        .collect();

    let legend: Vec<AnyElement> = counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(state, n)| {
            let state = *state;
            div()
                .id(eid2("mission-legend", task_id, state.label()))
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_mission_tasks(task_id, state, cx)),
                )
                .child(
                    div()
                        .size(px(6.))
                        .flex_none()
                        .rounded_full()
                        .bg(work_state_colour(state)),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .text_color(theme::text_muted())
                        .child(format!("{n} {}", state.label())),
                )
                .into_any_element()
        })
        .collect();

    div()
        .px_3()
        .pb_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1p5()
        .child(head)
        .child(
            div()
                .h(px(6.))
                .w_full()
                .flex()
                .bg(theme::fade(theme::text_faint(), 0.35))
                .children(segments),
        )
        .child(div().flex().flex_wrap().gap_2().children(legend))
        .into_any_element()
}

// ── shared bits ─────────────────────────────────────────────────────

/// A section's bar: the heading, and whatever the section wants readable beside it.
pub(super) fn section_bar(title: &str, note: Option<String>) -> impl IntoElement {
    div()
        .h(px(28.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(section_label(title))
        .when_some(note, |row, note| row.child(mono(note, theme::text_faint())))
}

/// One line of honest nothing.
pub(super) fn nothing(text: &'static str) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_faint())
        .child(text)
}

/// `T-207 Payment form` — the key where there is one, then the title.
pub(super) fn task_label(work: &WorkProjection, id: TaskId) -> String {
    match work.task(id) {
        Some(task) => match task.key.as_deref() {
            Some(key) => format!("{key} {}", task.title),
            None => task.title.clone(),
        },
        None => "(task gone)".to_string(),
    }
}

/// What a phase reads as. The path's own colour while it is being walked, the success token when
/// it is finished, the warning one while paused and the muted one for a mission nobody is taking
/// further.
pub fn phase_colour(phase: Phase) -> gpui::Rgba {
    match phase {
        Phase::Planning | Phase::Breakdown | Phase::Review => theme::info(),
        Phase::Running => theme::accent(),
        Phase::Paused => theme::warning(),
        Phase::Done => theme::success(),
        Phase::Abandoned => theme::text_muted(),
    }
}

// ── Needs you ───────────────────────────────────────────────────────

/// One *Needs you* row: a glyph in the attention colour, what it is about, and its actions.
fn need_row(
    id: gpui::ElementId,
    icon: IconName,
    colour: gpui::Rgba,
    text: impl Into<SharedString>,
    actions: Vec<AnyElement>,
) -> AnyElement {
    div()
        .px_3()
        .py_1p5()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(Icon::new(icon).with_size(Size::XSmall).text_color(colour))
                .child(elided(
                    id,
                    text,
                    theme::text(),
                    theme::font(Family::Chrome, Role::Body),
                )),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap_1p5()
                .children(actions),
        )
        .into_any_element()
}

/// Every item the user owes this mission, in the table's order: the window-derived ones (an agent
/// asking, a worker's permission request) first, then the host's `needs_you`.
///
/// The host's list is authoritative; *Planner is asking* and *Worker permission request* are the
/// window's, read off each agent's own `Activity::NeedsYou`.
pub(super) fn needs_you_rows(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    cx: &mut Context<AppState>,
) -> Vec<AnyElement> {
    let attention = crate::ui::work::doing_colour(crate::state::status::Doing::NeedsYou);
    let project_id = mission.project_id;
    let mut rows: Vec<AnyElement> = Vec::new();

    for agent_id in &mission.agents {
        let Some(agent) = work.agent(*agent_id) else {
            continue;
        };
        if agent.activity != Activity::NeedsYou {
            continue;
        }
        let agent_id = *agent_id;
        let planner = mission.settings.planner_agent == Some(agent_id);
        let text = match planner {
            true => "Planner is asking".to_string(),
            false => format!("{} needs permission", app.agent_label(agent).title),
        };
        rows.push(need_row(
            eid("mission-need-agent", agent_id),
            IconName::Bot,
            attention,
            text,
            vec![
                ghost_button(
                    eid("mission-need-agent-chat", agent_id),
                    None,
                    "Open chat",
                    cx.listener(move |this, _, _, cx| this.chat_with_mission_agent(agent_id, cx)),
                )
                .into_any_element(),
            ],
        ));
    }

    for (index, item) in mission.needs_you.iter().enumerate() {
        let key = eid2("mission-need", task_id, index);
        let row = match item {
            NeedsYou::PlanReady { open_threads } => {
                let text = match open_threads {
                    0 => "Plan ready \u{2014} approve".to_string(),
                    n => format!("Plan ready \u{2014} {n} open thread(s)"),
                };
                need_row(
                    key,
                    IconName::FileText,
                    match open_threads {
                        0 => attention,
                        _ => theme::warning(),
                    },
                    text,
                    vec![
                        ghost_button(
                            eid2("mission-need-plan", task_id, index),
                            None,
                            "Open plan",
                            cx.listener(move |this, _, _, cx| this.open_plan(task_id, cx)),
                        )
                        .into_any_element(),
                        primary_button(
                            eid2("mission-need-approve", task_id, index),
                            None,
                            "Approve plan",
                            cx.listener(move |this, _, _, cx| {
                                this.mission_control(task_id, MissionAction::ApprovePlan, cx)
                            }),
                        )
                        .into_any_element(),
                    ],
                )
            }
            NeedsYou::TasksDrafted => need_row(
                key,
                IconName::Inbox,
                attention,
                "Tasks drafted \u{2014} review and start",
                vec![
                    ghost_button(
                        eid2("mission-need-tasks", task_id, index),
                        None,
                        "Open tasks",
                        cx.listener(move |this, _, _, cx| this.open_mission_tasks_tab(task_id, cx)),
                    )
                    .into_any_element(),
                    primary_button(
                        eid2("mission-need-start", task_id, index),
                        None,
                        "Start mission",
                        cx.listener(move |this, _, _, cx| {
                            this.mission_control(task_id, MissionAction::StartMission, cx)
                        }),
                    )
                    .into_any_element(),
                ],
            ),
            NeedsYou::PlannerStopped => need_row(
                key,
                IconName::Bot,
                theme::warning(),
                "Planner stopped",
                vec![
                    primary_button(
                        eid2("mission-need-resume", task_id, index),
                        None,
                        "Resume planner",
                        cx.listener(move |this, _, _, cx| {
                            this.mission_control(task_id, MissionAction::ResumePlanner, cx)
                        }),
                    )
                    .into_any_element(),
                ],
            ),
            NeedsYou::AwaitingApproval { task } => {
                let task = *task;
                need_row(
                    key,
                    IconName::CircleCheck,
                    attention,
                    task_label(work, task),
                    vec![
                        ghost_button(
                            eid2("mission-need-reject", task_id, index),
                            None,
                            "Reject\u{2026}",
                            cx.listener(move |this, _, _, cx| {
                                this.open_mission_note(NoteKind::Reject(task), cx)
                            }),
                        )
                        .into_any_element(),
                        primary_button(
                            eid2("mission-need-confirm", task_id, index),
                            None,
                            "Confirm",
                            cx.listener(move |this, _, _, cx| {
                                this.mission_task_action(
                                    project_id,
                                    task,
                                    MissionTaskAction::Confirm,
                                    cx,
                                )
                            }),
                        )
                        .into_any_element(),
                    ],
                )
            }
            NeedsYou::HumanTask { task } => {
                let task = *task;
                // Fail needs a task to send its feedback to: only a review has one.
                let reviews = work.task(task).is_some_and(|t| t.review_of.is_some());
                let mut actions = Vec::new();
                if reviews {
                    actions.push(
                        ghost_button(
                            eid2("mission-need-fail", task_id, index),
                            None,
                            "Fail\u{2026}",
                            cx.listener(move |this, _, _, cx| {
                                this.open_mission_note(NoteKind::Fail(task), cx)
                            }),
                        )
                        .into_any_element(),
                    );
                }
                actions.push(
                    primary_button(
                        eid2("mission-need-done", task_id, index),
                        None,
                        "Done",
                        cx.listener(move |this, _, _, cx| {
                            this.mission_task_action(project_id, task, MissionTaskAction::Done, cx)
                        }),
                    )
                    .into_any_element(),
                );
                need_row(
                    key,
                    IconName::User,
                    attention,
                    task_label(work, task),
                    actions,
                )
            }
            NeedsYou::WorkerGone { task } => {
                let task = *task;
                need_row(
                    key,
                    IconName::TriangleAlert,
                    theme::danger(),
                    format!("{} \u{2014} worker gone", task_label(work, task)),
                    vec![
                        ghost_button(
                            eid2("mission-need-open", task_id, index),
                            None,
                            "Open task",
                            cx.listener(move |this, _, _, cx| this.select_task(task, cx)),
                        )
                        .into_any_element(),
                        primary_button(
                            eid2("mission-need-retry", task_id, index),
                            None,
                            "Retry",
                            cx.listener(move |this, _, _, cx| {
                                this.mission_task_action(
                                    project_id,
                                    task,
                                    MissionTaskAction::Retry,
                                    cx,
                                )
                            }),
                        )
                        .into_any_element(),
                    ],
                )
            }
            NeedsYou::ReviewFailedTwice { task } => {
                let task = *task;
                need_row(
                    key,
                    IconName::TriangleAlert,
                    theme::warning(),
                    format!(
                        "{} \u{2014} agent review failed twice",
                        task_label(work, task)
                    ),
                    vec![
                        ghost_button(
                            eid2("mission-need-open", task_id, index),
                            None,
                            "Open task",
                            cx.listener(move |this, _, _, cx| this.select_task(task, cx)),
                        )
                        .into_any_element(),
                        ghost_button(
                            eid2("mission-need-reject", task_id, index),
                            None,
                            "Reject\u{2026}",
                            cx.listener(move |this, _, _, cx| {
                                this.open_mission_note(NoteKind::Reject(task), cx)
                            }),
                        )
                        .into_any_element(),
                        // The normal Confirm: the host accepts it while `auto_fails >= 2`.
                        primary_button(
                            eid2("mission-need-confirm", task_id, index),
                            None,
                            "Confirm anyway",
                            cx.listener(move |this, _, _, cx| {
                                this.mission_task_action(
                                    project_id,
                                    task,
                                    MissionTaskAction::Confirm,
                                    cx,
                                )
                            }),
                        )
                        .into_any_element(),
                    ],
                )
            }
            NeedsYou::ReadyForReview { left_in_review } => {
                let left: Vec<String> = left_in_review
                    .iter()
                    .map(|id| task_label(work, *id))
                    .collect();
                let text = match left.is_empty() {
                    true => "Mission ready for review".to_string(),
                    false => format!(
                        "Mission ready for review \u{2014} still in review: {}",
                        left.join(", ")
                    ),
                };
                need_row(
                    key,
                    IconName::Eye,
                    attention,
                    text,
                    vec![
                        ghost_button(
                            eid2("mission-need-agent-review", task_id, index),
                            None,
                            "Run agent review",
                            cx.listener(move |this, _, _, cx| {
                                this.mission_control(task_id, MissionAction::RunAgentReview, cx)
                            }),
                        )
                        .into_any_element(),
                        ghost_button(
                            eid2("mission-need-reject", task_id, index),
                            None,
                            "Reject\u{2026}",
                            cx.listener(move |this, _, _, cx| {
                                this.open_mission_note(NoteKind::RejectMission(task_id), cx)
                            }),
                        )
                        .into_any_element(),
                        primary_button(
                            eid2("mission-need-complete", task_id, index),
                            None,
                            "Mark reviewed",
                            cx.listener(move |this, _, _, cx| {
                                this.mission_control(task_id, MissionAction::Complete, cx)
                            }),
                        )
                        .into_any_element(),
                    ],
                )
            }
        };
        rows.push(row);
    }
    rows
}

/// The full view's always-open *Needs you*: every row.
pub(super) fn needs_you(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let rows = needs_you_rows(app, work, task_id, mission, cx);
    let total = rows.len();
    div()
        .flex()
        .flex_none()
        .flex_col()
        .child(section_bar(
            "Needs you",
            (total > 0).then(|| total.to_string()),
        ))
        .when(total == 0, |body| body.child(nothing("Nothing needs you.")))
        .children(rows)
        .into_any_element()
}

/// The side panel's own *Needs you*: the same rows under a fold.
fn needs_you_section(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    open: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let rows = needs_you_rows(app, work, task_id, mission, cx);
    let total = rows.len();
    let bar = fold(
        task_id,
        MissionSection::NeedsYou,
        "Needs you",
        mono(
            (total > 0).then(|| total.to_string()).unwrap_or_default(),
            theme::text_faint(),
        ),
        open,
        cx,
    );
    let body = div().flex().flex_none().flex_col().child(bar);
    if !open {
        return body.into_any_element();
    }
    body.when(total == 0, |body| body.child(nothing("Nothing needs you.")))
        .children(rows)
        .into_any_element()
}

// ── Running and Next ────────────────────────────────────────────────

/// One row per live worker: its task, elapsed time, and **Chat**.
fn running(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    open: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let total = mission.running.len();
    let bar = fold(
        task_id,
        MissionSection::Running,
        "Running",
        mono(
            (total > 0).then(|| total.to_string()).unwrap_or_default(),
            theme::text_faint(),
        ),
        open,
        cx,
    );
    let body = div().flex().flex_none().flex_col().child(bar);
    if !open {
        return body.into_any_element();
    }

    let rows: Vec<AnyElement> = mission
        .running
        .iter()
        .map(|worker| {
            let agent_id = worker.agent;
            let agent = work.agent(agent_id);
            let colour = agent
                .map(|agent| crate::ui::work::activity_colour(agent.activity))
                .unwrap_or_else(theme::text_faint);
            let name = agent
                .map(|agent| app.agent_label(agent).title.to_string())
                .unwrap_or_else(|| "worker".to_string());
            let secs = (Utc::now() - worker.since).num_seconds().max(0);
            div()
                .h(px(26.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(hex_mark(
                    eid("mission-run-hex", agent_id),
                    colour,
                    Some(theme::fade(colour, 0.25)),
                    11.,
                    false,
                ))
                .child(mono(name, theme::text_muted()))
                .child(elided(
                    eid("mission-run-task", agent_id),
                    task_label(work, worker.task),
                    theme::text(),
                    theme::font(Family::Chrome, Role::Meta),
                ))
                .child(mono(
                    format!("{:02}:{:02}", secs / 60, secs % 60),
                    theme::text_faint(),
                ))
                .child(ghost_button(
                    eid("mission-run-chat", agent_id),
                    None,
                    "Chat",
                    cx.listener(move |this, _, _, cx| this.chat_with_mission_agent(agent_id, cx)),
                ))
                .into_any_element()
        })
        .collect();

    body.when(rows.is_empty(), |body| {
        body.child(nothing("No workers running."))
    })
    .children(rows)
    .into_any_element()
}

/// **Manual driver only**: the ready tasks, each with ▶ to start it.
fn next(
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    open: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let project_id = mission.project_id;
    let total = mission.next.len();
    let bar = fold(
        task_id,
        MissionSection::Next,
        "Next",
        mono(
            (total > 0).then(|| total.to_string()).unwrap_or_default(),
            theme::text_faint(),
        ),
        open,
        cx,
    );
    let body = div().flex().flex_none().flex_col().child(bar);
    if !open {
        return body.into_any_element();
    }

    let rows: Vec<AnyElement> = mission
        .next
        .iter()
        .map(|task| {
            let task = *task;
            div()
                .h(px(26.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(elided(
                    eid("mission-next-task", task),
                    task_label(work, task),
                    theme::text(),
                    theme::font(Family::Chrome, Role::Meta),
                ))
                .child(
                    icon_button(
                        eid("mission-next-start", task),
                        IconName::Play,
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.mission_task_action(project_id, task, MissionTaskAction::Start, cx)
                        }),
                    )
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Start this task now")
                            .build(window, cx)
                    }),
                )
                .into_any_element()
        })
        .collect();

    body.when(rows.is_empty(), |body| {
        body.child(nothing("No task is ready."))
    })
    .children(rows)
    .into_any_element()
}

/// **In Breakdown**: the drafted task list, in place of Running and Next, with **Start mission**.
/// The draft is edited on the full view's Tasks tab.
fn draft_list(
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let _ = mission;
    let drafted: Vec<&TaskRecord> = work.children_of(task_id).collect();
    let rows: Vec<AnyElement> = drafted
        .iter()
        .map(|task| {
            let id = task.id;
            div()
                .id(eid("mission-draft", id))
                .h(px(26.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.select_task(id, cx)))
                .child(elided(
                    eid("mission-draft-title", id),
                    task_label(work, id),
                    theme::text(),
                    theme::font(Family::Chrome, Role::Meta),
                ))
                .children(task.approve.then(|| {
                    Icon::new(IconName::CircleCheck)
                        .with_size(Size::XSmall)
                        .text_color(theme::warning())
                }))
                .children(
                    task.review_of
                        .is_some()
                        .then(|| mono("review", theme::text_faint())),
                )
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_none()
        .flex_col()
        .child(section_bar("Draft tasks", Some(drafted.len().to_string())))
        .when(rows.is_empty(), |body| {
            body.child(nothing("The planner has not drafted any tasks yet."))
        })
        .children(rows)
        .child(
            div()
                .px_3()
                .py_2()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .gap_1p5()
                .child(ghost_button(
                    eid("mission-draft-open", task_id),
                    None,
                    "Edit tasks",
                    cx.listener(move |this, _, _, cx| this.open_mission_tasks_tab(task_id, cx)),
                ))
                .child(primary_button(
                    eid("mission-draft-start", task_id),
                    None,
                    "Start mission",
                    cx.listener(move |this, _, _, cx| {
                        this.mission_control(task_id, MissionAction::StartMission, cx)
                    }),
                )),
        )
        .into_any_element()
}

// ── Documents ───────────────────────────────────────────────────────

/// The plan, with its open-thread count, and the mission's documents (`guidance` included), each a
/// button onto the document surface [`super::full::doc_row`] draws for the full view's *Plan &
/// docs* tab.
fn documents(
    task_id: TaskId,
    mission: &MissionSnapshot,
    open: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let total = 1 + mission.documents.len();
    let bar = fold(
        task_id,
        MissionSection::Documents,
        "Documents",
        mono(total.to_string(), theme::text_faint()),
        open,
        cx,
    );
    let body = div().flex().flex_none().flex_col().child(bar);
    if !open {
        return body.into_any_element();
    }

    let mut body = body.child(doc_row(
        eid("mission-open-plan", task_id),
        plan_label(mission.open_threads),
        cx.listener(move |this, _, _, cx| this.open_plan(task_id, cx)),
    ));
    for name in &mission.documents {
        let doc_name = name.clone();
        body = body.child(doc_row(
            eid2("mission-open-doc", task_id, name),
            format!("{name}.md"),
            cx.listener(move |this, _, _, cx| this.open_mission_doc(task_id, &doc_name, cx)),
        ));
    }
    body.into_any_element()
}

/// `plan.md`, with how many threads on it are still open.
pub(super) fn plan_label(open_threads: usize) -> String {
    match open_threads {
        0 => "plan.md".to_string(),
        1 => "plan.md (1 open thread)".to_string(),
        n => format!("plan.md ({n} open threads)"),
    }
}

// ── Guidance ────────────────────────────────────────────────────────

/// The guidance composer at the foot: a note for the whole mission, appended to `guidance.md`, and
/// the *also send to running workers* box.
///
/// **Enter and Send are the same path** — `AppState::add_guidance`. Every worker started from then
/// on gets the note in its prompt; with the box ticked the running ones get it as a message too.
fn guidance(
    app: &AppState,
    task_id: TaskId,
    to_running: bool,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let input = app.mission_guidance_input.clone();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1p5()
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    crate::ui::kit::field(theme::border(), focused)
                        .flex_1()
                        .min_w(px(0.))
                        .h(px(26.))
                        .px_2()
                        .child(
                            gpui_component::input::Input::new(&input)
                                .appearance(false)
                                .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
                )
                .child(ghost_button(
                    eid("mission-guidance-send", task_id),
                    None,
                    "Send",
                    cx.listener(move |this, _, window, cx| {
                        this.submit_mission_guidance(window, cx)
                    }),
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(check_box(
                    eid("mission-guidance-running", task_id),
                    to_running,
                    cx.listener(|this, _, _, cx| this.toggle_guidance_to_running(cx)),
                ))
                .child(mono("also send to running workers", theme::text_faint())),
        )
}
