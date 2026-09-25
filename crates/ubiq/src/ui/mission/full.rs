//! The mission full view — §6.2's *every detail*, in two shapes.
//!
//! **One view, two frames.** [`modal`] is the overlay raised by the side panel's `⤢` outside IDE
//! mode (`Layer::Mission`), [`tab`] is the same thing as a document in the centre region
//! (`PanelKind::MissionView`). Neither knows which one it is: both call [`body`], and what they
//! add is the frame — a title bar and *Open as tab* for the modal, a header row for the document.
//! That is the plan surface's own split between its dialog and a markdown tab's annotation
//! layout, and it is why moving between them loses nothing: the tab, the filter and the selection
//! live on the project's `state::mission::MissionView`, which both read.
//!
//! **Seven tabs.** The strip is built from [`TABS`], and what each draws is what is true today —
//! the phase history, the anchor task's own brief (M7), the work-state counts (M26), the
//! prerequisite graph laid out by dependency level ([`super::wbs`], M20), the roster with its four
//! live actions and the agent-kinds table beneath it (M13), the plan on the existing surface (M9),
//! the journal (M12), and every scheduler setting on the record ([`super::settings`], M22, M23).
//! Spend per mission is not measured anywhere, so it is absent rather than zero.

use chrono::Utc;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::IconName;

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{Actor, ExecutionMode, MissionRecord};
use ubiq_proto::work::{TaskRecord, WorkAgent};

use crate::app::AppState;
use crate::state::Layer;
use crate::state::mission::{MissionTab, MissionView};
use crate::state::work::{WorkProjection, WorkState};
use crate::theme::{self, Family, Role};
use crate::ui::kit::{
    Tab, UbiqIcon, card, elided, ghost_button, hex_mark, mono, panel, section_label, state_chip,
    tab_strip,
};
use crate::ui::mission::panel::{feedback, nothing, phase_colour, section_bar};
use crate::ui::work::{activity_colour, work_state_colour};
use crate::ui::{eid, eid2, empty, indexed};

/// The tabs this build draws, in the strip's order — all seven of them, which is
/// [`MissionTab::all`]'s own order.
pub const TABS: [MissionTab; 7] = MissionTab::all();

/// Wide enough for the tasks list and the agents table without either wrapping.
const MODAL_WIDTH: f32 = 880.0;
/// The tab strip and the body need a real height to scroll inside — `modal_sized`'s `fill_height`.
const MODAL_HEIGHT: f32 = 640.0;

// ── the two frames ──────────────────────────────────────────────────

/// The modal shape, painted at the window root by `ui::shell`.
pub fn modal(
    app: &AppState,
    task_id: TaskId,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let title = match app.work(cx).and_then(|work| work.task(task_id)) {
        Some(task) => match task.key.clone() {
            Some(key) => format!("{} {key} \u{2014} {}", app.mission_term(cx), task.title),
            None => format!("{} \u{2014} {}", app.mission_term(cx), task.title),
        },
        None => app.mission_term(cx),
    };
    let edge = app
        .mission(task_id, cx)
        .map(|mission| phase_colour(mission.phase))
        .unwrap_or_else(theme::accent);
    let view = cx.entity();
    // Built before the modal, because `modal_sized` takes the window mutably and the body reads
    // it to ask which field holds the keyboard.
    let content = body(app, task_id, window, cx);

    crate::ui::kit::modal_sized(
        "mission-full",
        edge,
        MODAL_WIDTH,
        Some(MODAL_HEIGHT),
        &title,
        content,
        footer(app, task_id, cx),
        crate::ui::dismiss(&view, Layer::Mission, |this, _, cx| {
            this.close_mission_full(cx)
        }),
        window,
    )
}

/// The document shape, drawn by the dock in the centre region.
pub fn tab(
    app: &AppState,
    task_id: TaskId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(work) = app.work(cx) else {
        return missing();
    };
    let (Some(task), Some(mission)) = (work.task(task_id), app.mission(task_id, cx)) else {
        return missing();
    };
    let term = app.mission_term(cx);

    panel()
        .child(header(work, task_id, &term, task, mission))
        .child(body(app, task_id, window, cx))
        .into_any_element()
}

fn missing() -> AnyElement {
    empty::empty_page(
        "No mission here",
        "This window is not holding the project this view was opened for.",
        UbiqIcon::ModeTasks,
        None,
    )
    .into_any_element()
}

/// The document shape's own header — what the modal's title bar says, said in a row.
fn header(
    work: &WorkProjection,
    task_id: TaskId,
    term: &str,
    task: &TaskRecord,
    mission: &MissionRecord,
) -> impl IntoElement {
    let colour = phase_colour(mission.phase);

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
        .child(crate::ui::mission::panel::mission_hex(
            work,
            task_id,
            mission,
            eid("mission-full-hex", task_id),
            14.,
        ))
        .child(section_label(term))
        .when_some(task.key.clone(), |row, key| {
            row.child(mono(key, theme::text_muted()))
        })
        .child(elided(
            eid("mission-full-title", task_id),
            task.title.clone(),
            theme::text(),
            theme::font(Family::Chrome, Role::Label),
        ))
        .child(state_chip(mission.phase.label(), colour, 1.0))
}

/// The modal's foot: how the mission is run, and the one control that changes its shape.
fn footer(app: &AppState, task_id: TaskId, cx: &mut Context<AppState>) -> AnyElement {
    let run = app
        .mission(task_id, cx)
        .map(execution_label)
        .unwrap_or_default();

    div()
        .flex()
        .flex_1()
        .items_center()
        .gap_2()
        .child(mono(run, theme::text_faint()))
        .child(div().flex_1().min_w(px(0.)))
        .child(ghost_button(
            "mission-open-as-tab",
            Some(IconName::PanelRight),
            "Open as tab",
            cx.listener(move |this, _, _, cx| this.open_mission_tab(task_id, cx)),
        ))
        .into_any_element()
}

fn execution_label(mission: &MissionRecord) -> String {
    let execution = match mission.execution {
        ExecutionMode::Auto => format!("auto \u{00d7}{}", mission.parallelism),
        ExecutionMode::Manual => "manual".to_string(),
    };
    format!("{} \u{00b7} {execution}", mission.phase.label())
}

// ── the strip and what is under it ──────────────────────────────────

/// The tab strip and the tab's own body, shared by both frames.
fn body(
    app: &AppState,
    task_id: TaskId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(work) = app.work(cx) else {
        return missing();
    };
    let Some(mission) = app.mission(task_id, cx) else {
        return missing();
    };
    let view = app.mission_view(cx).cloned().unwrap_or_default();
    let active = TABS.iter().position(|tab| *tab == view.tab).unwrap_or(0);
    let tabs: Vec<Tab> = TABS.iter().map(|tab| Tab::new(tab.label())).collect();

    let term = app.mission_term(cx);
    let content = match TABS[active] {
        MissionTab::Wbs => super::wbs::render(app, work, task_id, &view, window, cx),
        MissionTab::Settings => super::settings::render(task_id, mission, cx),
        MissionTab::Tasks => tasks(work, task_id, &view, cx),
        MissionTab::Agents => agents(app, work, task_id, mission, cx),
        MissionTab::Docs => docs(task_id, cx),
        MissionTab::Activity => activity(app, task_id, &term, window, cx),
        _ => overview(app, work, task_id, mission, cx),
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(tab_strip(
            "mission-tabs",
            tabs,
            active,
            indexed(&cx.entity(), |this, index, _window, cx| {
                if let Some(tab) = TABS.get(index) {
                    this.set_mission_tab(*tab, cx);
                }
            }),
            None,
            None,
        ))
        .child(
            div()
                .id(eid("mission-scroll", task_id))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(content),
        )
        // The same two menus the side panel raises — *Spawn ▾* and the kind picker are one
        // control each, drawn wherever the surface that opened them is.
        .children(super::menu::spawn_overlay(app, task_id, cx))
        .children(super::menu::kind_overlay(app, task_id, cx))
        .into_any_element()
}

// ── Overview ────────────────────────────────────────────────────────

/// The phase history, the brief, the counts, the agents and elapsed time, and *Needs you*.
///
/// **Spend is not here.** Nothing measures a mission's, and a zero would read as a fact.
fn overview(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let task = work.task(task_id);
    let members = on_mission(work, task_id, mission).len();

    div()
        .flex()
        .flex_col()
        .child(section_bar("Phase", Some(execution_label(mission))))
        // The live stepper over the history that explains it: the dots move the mission (M5), the
        // rows under them say when each phase was entered and who moved it. Drawn in every phase,
        // `Abandoned` included — a mission the user wants back is moved from here.
        .child(
            div()
                .px_3()
                .pt_2()
                .flex()
                .flex_none()
                .child(crate::ui::mission::panel::phase_steps(task_id, mission, cx)),
        )
        .child(phase_history(mission))
        .child(section_bar("Brief", None))
        .children(task.map(|task| brief(work, task)))
        .child(section_bar("Progress", None))
        .child(counts(work, task_id, cx))
        .child(section_bar(
            "Agents",
            Some(format!("{members} \u{00b7} running {}", elapsed(mission))),
        ))
        .child(agent_lines(app, work, task_id, mission))
        // The whole list, with its actions — the side panel shows the first of these and a count.
        .child(crate::ui::mission::panel::needs_you(
            app, task_id, mission, false, cx,
        ))
        .into_any_element()
}

/// Every phase this mission has been in, with when it moved and who moved it — `phase_history`
/// carries both, which is why this is a list rather than the panel's four dots.
fn phase_history(mission: &MissionRecord) -> impl IntoElement {
    let rows: Vec<AnyElement> = mission
        .phase_history
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let colour = phase_colour(entry.phase);
            let current = entry.phase == mission.phase && index + 1 == mission.phase_history.len();
            div()
                .h(px(26.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(if current { 9. } else { 7. }))
                        .flex_none()
                        .rounded_full()
                        .bg(colour),
                )
                .child(
                    div()
                        .w(px(96.))
                        .flex_none()
                        .text_size(theme::font(Family::Chrome, Role::Label))
                        .text_color(if current {
                            theme::text()
                        } else {
                            theme::text_muted()
                        })
                        .child(entry.phase.label()),
                )
                .child(mono(
                    entry.at.format("%Y-%m-%d %H:%M").to_string(),
                    theme::text_faint(),
                ))
                .child(mono(actor(entry.by), theme::text_faint()))
                .into_any_element()
        })
        .collect();

    div().flex().flex_col().flex_none().py_1().children(rows)
}

/// Who moved something. Three answers, never two: "the host did it on its own" and "the person at
/// the keyboard did it" are different facts (`ubiq_proto::mission::Actor`).
pub(super) fn actor(by: Actor) -> String {
    match by {
        Actor::User => "you".to_string(),
        Actor::Host => "the host".to_string(),
        Actor::Agent(id) => format!("agent {id}"),
    }
}

/// **The brief IS the anchor task's fields** (M7): its description, its attachments and the tasks
/// it references. Nothing new is stored and nothing here is editable — the task panel and the
/// board's form are where a task is written.
fn brief(work: &WorkProjection, task: &TaskRecord) -> AnyElement {
    let references: Vec<AnyElement> = task
        .references
        .iter()
        .filter_map(|id| work.task(*id))
        .map(|other| {
            state_chip(
                other.key.clone().unwrap_or_else(|| other.title.clone()),
                theme::text_muted(),
                1.0,
            )
            .into_any_element()
        })
        .collect();

    let attachments: Vec<AnyElement> = task
        .attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            let label = attachment
                .label
                .clone()
                .unwrap_or_else(|| attachment.target.clone());
            div()
                .h(px(22.))
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .child(elided(
                    eid2("mission-attachment", task.id, index),
                    label,
                    theme::text_muted(),
                    theme::font(Family::Chrome, Role::Meta),
                ))
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
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(if task.description.is_empty() {
                    theme::text_faint()
                } else {
                    theme::text()
                })
                .child(match task.description.is_empty() {
                    true => SharedString::from("Nothing written yet."),
                    false => SharedString::from(task.description.clone()),
                }),
        )
        .when(!attachments.is_empty(), |body| {
            body.child(div().flex().flex_col().children(attachments))
        })
        .when(!references.is_empty(), |body| {
            body.child(div().flex().flex_wrap().gap_1p5().children(references))
        })
        .into_any_element()
}

/// One chip per work state with a count (M26). Each is a way into the Tasks tab, filtered — the
/// same gesture the side panel's progress segments are.
fn counts(work: &WorkProjection, task_id: TaskId, cx: &mut Context<AppState>) -> AnyElement {
    let counts = work.work_state_counts(task_id);
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return nothing("No tasks yet.").into_any_element();
    }

    let chips: Vec<AnyElement> = counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(state, n)| {
            let state = *state;
            div()
                .id(eid2("mission-count", task_id, state.label()))
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_mission_tasks(task_id, state, cx)),
                )
                .child(state_chip(
                    format!("{n} {}", state.label()),
                    work_state_colour(state),
                    1.0,
                ))
                .into_any_element()
        })
        .collect();

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_wrap()
        .gap_1p5()
        .children(chips)
        .into_any_element()
}

/// How long the mission has been going, from the record's own stamp.
fn elapsed(mission: &MissionRecord) -> String {
    let span = Utc::now() - mission.created_at;
    let hours = span.num_hours();
    match hours {
        h if h < 1 => format!("{}m", span.num_minutes().max(0)),
        h if h < 48 => format!("{h}h"),
        h => format!("{}d", h / 24),
    }
}

// ── Tasks ───────────────────────────────────────────────────────────

/// The mission's own tasks, filtered to one work state when the view says so.
///
/// The filter is `MissionView::state_filter`, which the counts above and the side panel's
/// progress segments set — this tab is the surface those clicks land on.
fn tasks(
    work: &WorkProjection,
    task_id: TaskId,
    view: &MissionView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let counts = work.work_state_counts(task_id);
    let filters: Vec<AnyElement> = counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(state, n)| {
            let state = *state;
            let on = view.state_filter == Some(state);
            crate::ui::kit::choice_pill(
                eid2("mission-filter", task_id, state.label()),
                format!("{n} {}", state.label()),
                on,
                cx.listener(move |this, _, _, cx| this.toggle_mission_state_filter(state, cx)),
            )
            .into_any_element()
        })
        .collect();

    let rows: Vec<AnyElement> = work
        .children_of(task_id)
        .map(|task| (task, work.work_state(task)))
        .filter(|(_, state)| view.state_filter.is_none_or(|want| want == *state))
        .map(|(task, state)| task_row(work, task, state, cx))
        .collect();

    div()
        .flex()
        .flex_col()
        .child(
            div()
                .px_3()
                .py_2()
                .flex()
                .flex_none()
                .flex_wrap()
                .gap_1p5()
                .children(filters),
        )
        .when(rows.is_empty(), |body| {
            body.child(nothing("No tasks in this state."))
        })
        .children(rows)
        .into_any_element()
}

/// One task, as the board reads one: the state's colour on the left edge, the key, the title, and
/// what it is still waiting on (M20's derived readiness, `TaskRecord::waiting_on`).
fn task_row(
    work: &WorkProjection,
    task: &TaskRecord,
    state: WorkState,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = task.id;
    let waiting = task.waiting_on(&work.tasks).len();

    card(eid("mission-task", id), work_state_colour(state), false)
        .h(px(30.))
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .on_click(cx.listener(move |this, _, _, cx| this.select_task(id, cx)))
        .when_some(task.key.clone(), |row, key| {
            row.child(mono(key, theme::text_muted()))
        })
        .child(elided(
            eid("mission-task-title", id),
            task.title.clone(),
            theme::text(),
            theme::font(Family::Chrome, Role::Body),
        ))
        .when(waiting > 0, |row| {
            row.child(mono(
                format!("waits on {waiting}"),
                work_state_colour(WorkState::Waiting),
            ))
        })
        .child(state_chip(state.label(), work_state_colour(state), 1.0))
        .into_any_element()
}

// ── Agents ──────────────────────────────────────────────────────────

/// Which agents are on this mission.
///
/// **The record's roster is the membership** (M11): assigned to the mission's task or one of its
/// children, *or* spawned by a member — and the second half cannot be recomputed, which is why the
/// host writes it down as it happens.
///
/// The derived half is kept beside it rather than dropped: an assignment is what the roster is
/// settled *from*, so an agent just pointed at one of the mission's tasks is on the mission a
/// frame before the record says so, and a list that waited would flicker.
pub(super) fn on_mission<'a>(
    work: &'a WorkProjection,
    task_id: TaskId,
    mission: &MissionRecord,
) -> Vec<&'a WorkAgent> {
    let children: Vec<TaskId> = work.children_of(task_id).map(|task| task.id).collect();
    work.agents
        .iter()
        .filter(|agent| {
            mission.member(agent.id).is_some()
                || agent
                    .task
                    .is_some_and(|held| held == task_id || children.contains(&held))
        })
        .collect()
}

/// The roster as a table: role, kind, model, current task, affinity labels, tasks held,
/// lifecycle.
///
/// Affinity labels are the labels of the tasks an agent holds here — the accumulated set on
/// `RosterEntry::labels` is S3's, so what is drawn is what the current task says.
fn agents(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let rows: Vec<AnyElement> = on_mission(work, task_id, mission)
        .into_iter()
        .map(|agent| agent_row(app, work, task_id, mission, agent, Some(cx)))
        .collect();

    div()
        .flex()
        .flex_col()
        .child(crate::ui::mission::panel::section_bar_with(
            "Roster",
            Some(format!("{}", rows.len())),
            Some(crate::ui::mission::panel::spawn_button(
                app,
                task_id,
                "mission-full-spawn",
                cx,
            )),
        ))
        .when(rows.is_empty(), |body| {
            body.child(nothing("No agents on this mission."))
        })
        .when(!rows.is_empty(), |body| body.child(agents_head()))
        .children(rows)
        .child(agent_kinds(task_id, mission, cx))
        .into_any_element()
}

/// The agent-kinds table (M13): what a mission may spawn, what each resolves to, and what each is
/// good at.
///
/// **A short list edited as a set** — `MissionField::AgentKinds` replaces the whole table, which is
/// `TaskField::Labels`' own posture, so every control here sends one message carrying the list as
/// it now is. Seeded from the project's profiles by the new-mission dialog; a row's profile is
/// re-pointed from the same picker a pending spawn's kind is changed with.
pub(super) fn agent_kinds(
    task_id: TaskId,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let rows: Vec<AnyElement> = mission
        .agent_kinds
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            let labels = match kind.labels.is_empty() {
                true => String::new(),
                false => kind.labels.join(", "),
            };
            div()
                .h(px(28.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(
                    elided(
                        eid2("mission-kind-name", task_id, index),
                        kind.name.clone(),
                        theme::text(),
                        theme::font(Family::Chrome, Role::Meta),
                    )
                    .w(px(120.))
                    .flex_none(),
                )
                .child(
                    elided(
                        eid2("mission-kind-desc", task_id, index),
                        kind.description.clone(),
                        theme::text_muted(),
                        theme::font(Family::Chrome, Role::Meta),
                    )
                    .flex_1()
                    .min_w(px(0.)),
                )
                .child(
                    elided(
                        eid2("mission-kind-labels", task_id, index),
                        labels,
                        theme::text_faint(),
                        theme::font(Family::Chrome, Role::Meta),
                    )
                    .w(px(120.))
                    .flex_none(),
                )
                // The profile it resolves to — a control, because that is the one field of a kind
                // the interface can answer from what the host already lists.
                .child(
                    div()
                        .id(eid2("mission-kind-profile", task_id, index))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                            let at = event.position();
                            this.open_mission_kind_menu(
                                task_id,
                                crate::state::mission::KindTarget::Row(index),
                                (at.x.into(), at.y.into()),
                                cx,
                            );
                        }))
                        .tooltip(|window, cx| {
                            gpui_component::tooltip::Tooltip::new("Which profile this resolves to")
                                .build(window, cx)
                        })
                        .child(state_chip(
                            kind.profile.clone().unwrap_or_else(|| "no profile".into()),
                            match kind.profile.is_some() {
                                true => theme::accent(),
                                false => theme::warning(),
                            },
                            1.0,
                        )),
                )
                .child(ghost_button(
                    eid2("mission-kind-remove", task_id, index),
                    None,
                    "Remove",
                    cx.listener(move |this, _, _, cx| {
                        this.remove_mission_agent_kind(task_id, index, cx)
                    }),
                ))
                .into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .child(crate::ui::mission::panel::section_bar_with(
            "Agent kinds",
            (!rows.is_empty()).then(|| rows.len().to_string()),
            Some(
                div()
                    .id(eid("mission-kind-add", task_id))
                    .h(px(20.))
                    .px_2()
                    .flex()
                    .flex_none()
                    .items_center()
                    .cursor_pointer()
                    .border_1()
                    .border_color(theme::border())
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                    .text_color(theme::text())
                    .child("Add from profile \u{25be}")
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        let at = event.position();
                        this.open_mission_kind_menu(
                            task_id,
                            crate::state::mission::KindTarget::Add,
                            (at.x.into(), at.y.into()),
                            cx,
                        );
                    }))
                    .into_any_element(),
            ),
        ))
        .when(rows.is_empty(), |body| {
            body.child(nothing(
                "No kinds yet \u{2014} an agent asking for one gets whatever profile it names.",
            ))
        })
        .children(rows)
        .into_any_element()
}

fn agents_head() -> impl IntoElement {
    let cell = |label: &'static str, width: f32| {
        div()
            .w(px(width))
            .flex_none()
            .text_size(theme::font(Family::Chrome, Role::Micro))
            .text_color(theme::text_faint())
            .child(label)
    };

    div()
        .h(px(22.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(cell("ROLE", 110.))
        .child(cell("KIND", 90.))
        .child(cell("MODEL", 110.))
        .child(div().flex_1().min_w(px(0.)).child(cell("ON", 40.)))
        .child(cell("LABELS", 140.))
        .child(cell("HELD", 40.))
        .child(cell("LIFECYCLE", 80.))
}

/// One roster row. `cx` present is the Agents tab, which draws the four live actions; the
/// Overview passes `None` and gets the same row without them — the short list there is a reading,
/// and the place a member is acted on is the tab that is about them.
fn agent_row(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionRecord,
    agent: &WorkAgent,
    cx: Option<&mut Context<AppState>>,
) -> AnyElement {
    let colour = activity_colour(agent.activity);
    let id = agent.id;
    let is_coordinator = mission.coordinator == Some(id);
    let live = app.conversation_live(id);
    // How many of *this mission's* tasks the agent is on. A `WorkAgent` points at one task, so
    // this is 0 or 1 today; the accumulated count over a mission's life is the stored roster's,
    // which is S3's to write.
    let held = work
        .children_of(task_id)
        .filter(|task| agent.task == Some(task.id))
        .count();
    let on = agent.task.and_then(|id| work.task(id));
    let labels: String = on
        .map(|task| {
            task.labels
                .iter()
                .map(|label| label.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let cell = |id: gpui::ElementId, text: String, width: f32| {
        elided(
            id,
            text,
            theme::text_muted(),
            theme::font(Family::Chrome, Role::Meta),
        )
        .w(px(width))
        .flex_none()
    };

    div()
        .h(px(28.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(hex_mark(
            eid("mission-roster-hex", agent.id),
            colour,
            Some(theme::fade(colour, 0.25)),
            11.,
            false,
        ))
        .child(cell(
            eid("mission-roster-role", agent.id),
            match (is_coordinator, agent.role.is_empty()) {
                // The mission's own role outranks the harness's own word for itself: which agent
                // plans and asks is the one fact this table exists to say.
                (true, _) => format!("coordinator \u{00b7} {}", app.agent_title(agent)),
                (false, true) => app.agent_title(agent).to_string(),
                (false, false) => agent.role.clone(),
            },
            96.,
        ))
        .child(cell(
            eid("mission-roster-kind", agent.id),
            agent.harness.clone(),
            90.,
        ))
        .child(cell(
            eid("mission-roster-model", agent.id),
            agent.model.clone(),
            110.,
        ))
        .child(elided(
            eid("mission-roster-on", agent.id),
            match on {
                Some(task) => task.key.clone().unwrap_or_else(|| task.title.clone()),
                None => agent.note.clone(),
            },
            theme::text(),
            theme::font(Family::Chrome, Role::Meta),
        ))
        .child(cell(eid("mission-roster-labels", agent.id), labels, 140.))
        .child(cell(
            eid("mission-roster-held", agent.id),
            held.to_string(),
            40.,
        ))
        .child(state_chip(agent.activity.label(), colour, 1.0))
        // The four actions, live (§6.2). `Chat` becomes `Resume` for an agent whose harness has
        // been unloaded — the conversation is still there, so the honest offer is to start it
        // again rather than to open a panel onto nothing.
        .when_some(cx, |row, cx| {
            row.child(match live {
                true => ghost_button(
                    eid("mission-roster-chat", id),
                    None,
                    "Chat",
                    cx.listener(move |this, _, _, cx| this.chat_with_mission_agent(id, cx)),
                ),
                false => ghost_button(
                    eid("mission-roster-resume", id),
                    None,
                    "Resume",
                    cx.listener(move |this, _, _, cx| this.resume_mission_agent(id, cx)),
                ),
            })
            .child(ghost_button(
                eid("mission-roster-stop", id),
                None,
                "Stop",
                cx.listener(move |this, _, _, cx| this.stop_mission_agent(id, cx)),
            ))
            .child(ghost_button(
                eid("mission-roster-detach", id),
                None,
                "Detach",
                cx.listener(move |this, _, _, cx| this.detach_mission_agent(id, cx)),
            ))
            // M10's handoff: the previous coordinator is put back to worker and *told*, never
            // killed — which is the host's to do, in one `SetMissionField`.
            .when(!is_coordinator, |row| {
                row.child(ghost_button(
                    eid("mission-roster-crown", id),
                    None,
                    "Make coordinator",
                    cx.listener(move |this, _, _, cx| {
                        this.make_mission_coordinator(task_id, id, cx)
                    }),
                ))
            })
        })
        .into_any_element()
}

/// The Overview's short version of the same list — who is on it, and what each is doing.
fn agent_lines(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionRecord,
) -> AnyElement {
    let rows: Vec<AnyElement> = on_mission(work, task_id, mission)
        .into_iter()
        .map(|agent| agent_row(app, work, task_id, mission, agent, None))
        .collect();

    match rows.is_empty() {
        true => nothing("No agents on this mission.").into_any_element(),
        false => div().flex().flex_col().children(rows).into_any_element(),
    }
}

// ── Plan & docs ─────────────────────────────────────────────────────

/// The plan, on the surface it already has (M9) — `ui::document`'s columns, minimap, gutter and
/// thread rail, raised as the plan dialog the way the task panel raises it. **Unchanged by this
/// proposal**: nothing here is a second plan editor.
///
/// Mission documents (`DocumentHandle::MissionDoc`, M8) share exactly that surface — one row per
/// document, opened the way the plan's own row is. **The row beneath the plan's is honestly
/// empty**: the path under `missions/<TaskId>/docs/` is real and readable this wave, but nothing
/// writes one yet — `ubiq-mission::write_document` is a later wave — so there is nothing here to
/// list, not a missing feature to hide.
fn docs(task_id: TaskId, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(section_bar("Documents", None))
        .child(doc_row(
            "mission-open-plan",
            "Plan",
            cx.listener(move |this, _, _, cx| this.open_plan(task_id, cx)),
        ))
        .child(nothing("No documents yet."))
        .into_any_element()
}

/// One row of the *Plan & docs* list: a name and the "Open" that raises it on the document
/// surface. The plan's own row and a mission document's row are this, called the same way — the
/// document family draws neither differently from the other.
fn doc_row(
    id: &'static str,
    name: &'static str,
    on_open: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    div()
        .h(px(30.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child(name),
        )
        .child(ghost_button(id, Some(IconName::FileText), "Open", on_open))
        .into_any_element()
}

// ── Activity ────────────────────────────────────────────────────────

/// The mission's journal, page after page (M12), and the live feedback composer under it.
///
/// **The two kinds M13 adds are drawn like every other one** — `spawn_requested` and
/// `spawn_answered` are rows here because `JournalEvent::kind` is a closed set and this reads it,
/// so a kind added later needs a colour and nothing else.
fn activity(
    app: &AppState,
    task_id: TaskId,
    term: &str,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let journal = app.mission_journal(task_id);
    let rows: Vec<AnyElement> = journal
        .map(|held| {
            held.entries
                .iter()
                .map(crate::ui::mission::panel::journal_row)
                .collect()
        })
        .unwrap_or_default();
    let more = journal.is_some_and(|held| held.more);

    div()
        .flex()
        .flex_col()
        .child(section_bar(
            "Journal",
            (!rows.is_empty()).then(|| rows.len().to_string()),
        ))
        .when(rows.is_empty(), |body| {
            body.child(nothing("No activity yet."))
        })
        .children(rows)
        // The cursor is the oldest sequence held, so a step back never skips or repeats a line.
        .when(more, |body| {
            body.child(div().px_3().py_2().flex().flex_none().child(ghost_button(
                eid("mission-journal-more", task_id),
                None,
                "Older",
                cx.listener(move |this, _, _, cx| this.load_more_mission_journal(task_id, cx)),
            )))
        })
        .child(feedback(app, task_id, term, false, window, cx))
        .into_any_element()
}
