//! The mission full view — every detail, in two shapes.
//!
//! **One view, two frames.** [`modal`] is the overlay raised by the side panel's `⤢` outside IDE
//! mode (`Layer::Mission`), [`tab`] is the same thing as a document in the centre region
//! (`PanelKind::MissionView`). Neither knows which one it is: both call [`body`], and what they
//! add is the frame — a title bar and *Open as tab* for the modal, a header row for the document.
//! Moving between them loses nothing: the tab, the filter and the selection live on the project's
//! `state::mission::MissionView`, which both read.
//!
//! **Five tabs** ([`MissionTab`]): *Overview* (brief, phase history, counts, the full Needs you
//! list), *Tasks* (the WBS with its Graph/Table toggle, and in Breakdown the draft's editing),
//! *Plan & docs*, *History* (computed by the host, filtered here) and *Settings* (the five
//! fields).

use chrono::Utc;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::IconName;

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{
    Driver, HistoryEntry, HistoryKind, MissionAction, MissionSnapshot, Phase,
};
use ubiq_proto::work::TaskRecord;

use crate::app::AppState;
use crate::state::Layer;
use crate::state::mission::{HISTORY_KINDS, MissionTab, MissionView, NoteKind, history_kind_label};
use crate::state::work::WorkProjection;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{
    Tab, UbiqIcon, choice_pill, elided, ghost_button, mono, panel, primary_button, section_label,
    state_chip, tab_strip,
};
use crate::ui::mission::panel::{
    needs_you, nothing, phase_colour, plan_label, section_bar, task_label,
};
use crate::ui::work::work_state_colour;
use crate::ui::{eid, eid2, empty, indexed};

/// The tabs, in the strip's order — [`MissionTab::all`]'s own.
pub const TABS: [MissionTab; 5] = MissionTab::all();

/// Wide enough for the WBS table without it wrapping.
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
        None,
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
        .child(header(work, task_id, &term, task, mission, cx))
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
    mission: &MissionSnapshot,
    cx: &gpui::App,
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
            mission,
            eid("mission-full-hex", task_id),
            14.,
            cx,
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
    let run = app.mission(task_id, cx).map(run_label).unwrap_or_default();

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

fn run_label(mission: &MissionSnapshot) -> String {
    let driver = match mission.settings.driver {
        Driver::Auto => format!("auto \u{00d7}{}", mission.settings.parallelism()),
        Driver::Manual => "manual".to_string(),
    };
    format!("{} \u{00b7} {driver}", mission.phase.label())
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

    let content = match TABS[active] {
        MissionTab::Tasks => super::wbs::render(app, work, task_id, mission, &view, window, cx),
        MissionTab::Settings => super::settings::render(app, task_id, &mission.settings, cx),
        MissionTab::PlanDocs => plan_docs(task_id, mission, cx),
        MissionTab::History => history(app, work, task_id, &view, cx),
        MissionTab::Overview => overview(app, work, task_id, mission, &view, cx),
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
        .into_any_element()
}

// ── Overview ────────────────────────────────────────────────────────

/// The phase stepper and its history, the brief, the counts, and the full *Needs you* list.
fn overview(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionSnapshot,
    view: &MissionView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let _ = view;
    let task = work.task(task_id);
    let history = app.mission_history(task_id);

    div()
        .flex()
        .flex_col()
        .child(section_bar("Phase", Some(run_label(mission))))
        .child(
            div()
                .px_3()
                .pt_2()
                .flex()
                .flex_none()
                .child(crate::ui::mission::panel::phase_steps(mission.phase)),
        )
        .child(phase_history(history))
        .child(section_bar("Brief", None))
        .children(task.map(|task| brief(work, task)))
        .child(section_bar(
            "Progress",
            task.map(|task| format!("running {}", elapsed(task))),
        ))
        .child(counts(work, task_id, cx))
        .child(needs_you(app, work, task_id, mission, cx))
        .into_any_element()
}

/// Every phase the mission has been in, when it moved — read off the computed history's `Phase`
/// entries, oldest first. Empty until the History has been asked for (opening the History tab
/// asks), and says so rather than drawing a guess.
fn phase_history(history: Option<&Vec<HistoryEntry>>) -> AnyElement {
    let Some(entries) = history else {
        return nothing("Open the History tab to load the phase history.").into_any_element();
    };
    let phases: Vec<&HistoryEntry> = entries
        .iter()
        .filter(|entry| entry.kind == HistoryKind::Phase)
        .rev()
        .collect();
    if phases.is_empty() {
        return nothing("No phase changes yet.").into_any_element();
    }
    let last = phases.len() - 1;
    let rows: Vec<AnyElement> = phases
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            div()
                .h(px(26.))
                .px_3()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .size(px(if index == last { 9. } else { 7. }))
                        .flex_none()
                        .rounded_full()
                        .bg(theme::info()),
                )
                .child(mono(
                    entry.at.format("%Y-%m-%d %H:%M").to_string(),
                    theme::text_faint(),
                ))
                .child(elided(
                    eid2("mission-phase-row", index, entry.at.timestamp()),
                    entry.text.clone(),
                    theme::text_muted(),
                    theme::font(Family::Chrome, Role::Meta),
                ))
                .into_any_element()
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .py_1()
        .children(rows)
        .into_any_element()
}

/// **The brief IS the parent task's fields**: its description, its attachments and the tasks it
/// references. Nothing new is stored and nothing here is editable — the task panel and the
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

/// One chip per work state with a count. Each is a way into the Tasks tab, filtered — the same
/// gesture the side panel's progress segments are.
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

/// How long the mission has been going, from its parent task's own stamp.
fn elapsed(task: &TaskRecord) -> String {
    let span = Utc::now() - task.created_at;
    let hours = span.num_hours();
    match hours {
        h if h < 1 => format!("{}m", span.num_minutes().max(0)),
        h if h < 48 => format!("{h}h"),
        h => format!("{}d", h / 24),
    }
}

// ── Plan & docs ─────────────────────────────────────────────────────

/// The plan and the mission's documents, on the surface they already have — `ui::document`'s
/// columns, minimap, gutter and thread rail, raised as the plan dialog the way the task panel
/// raises it. **Approve plan** and **Replan…** are the two gates that act on it: approval is open
/// while the mission is planning, a replan once it is running or in review.
fn plan_docs(task_id: TaskId, mission: &MissionSnapshot, cx: &mut Context<AppState>) -> AnyElement {
    let planning = mission.phase == Phase::Planning;
    let can_replan = matches!(
        mission.phase,
        Phase::Running | Phase::Paused | Phase::Review
    );

    let mut body = div()
        .flex()
        .flex_col()
        .child(
            div()
                .px_3()
                .py_2()
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .gap_1p5()
                .child(
                    ghost_button(
                        eid("mission-docs-replan", task_id),
                        None,
                        "Replan\u{2026}",
                        cx.listener(move |this, _, _, cx| {
                            this.open_mission_note(NoteKind::Replan(task_id), cx)
                        }),
                    )
                    .opacity(if can_replan { 1.0 } else { 0.5 }),
                )
                .child(
                    primary_button(
                        eid("mission-docs-approve", task_id),
                        None,
                        "Approve plan",
                        cx.listener(move |this, _, _, cx| {
                            this.mission_control(task_id, MissionAction::ApprovePlan, cx)
                        }),
                    )
                    .opacity(if planning { 1.0 } else { 0.5 }),
                ),
        )
        .child(section_bar("Documents", None))
        .child(doc_row(
            eid("mission-open-plan", task_id),
            plan_label(mission.open_threads),
            cx.listener(move |this, _, _, cx| this.open_plan(task_id, cx)),
        ));
    if mission.documents.is_empty() {
        body = body.child(nothing("No documents yet."));
    } else {
        for name in &mission.documents {
            let doc_name = name.clone();
            body = body.child(doc_row(
                eid2("mission-open-doc", task_id, name),
                format!("{name}.md"),
                cx.listener(move |this, _, _, cx| this.open_mission_doc(task_id, &doc_name, cx)),
            ));
        }
    }
    body.into_any_element()
}

/// One row of the *Plan & docs* list: a name and the "Open" that raises it on the document
/// surface. The plan's own row and a mission document's row are this, called the same way — the
/// document family draws neither differently from the other. `pub(super)` because the side
/// panel's *Documents* section draws the same rows (T-184).
pub(super) fn doc_row(
    id: gpui::ElementId,
    name: impl Into<SharedString>,
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
                .child(name.into()),
        )
        .child(ghost_button(id, Some(IconName::FileText), "Open", on_open))
        .into_any_element()
}

// ── History ─────────────────────────────────────────────────────────

/// The mission's one timeline, newest first, as the host computed it. Filters by kind, task and
/// agent; a row opens its task, its plan revision or its chat.
fn history(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    view: &MissionView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(entries) = app.mission_history(task_id) else {
        return nothing("Loading the history\u{2026}").into_any_element();
    };
    let filter = view.history;

    let kinds: Vec<AnyElement> = std::iter::once(None)
        .chain(HISTORY_KINDS.into_iter().map(Some))
        .map(|kind| {
            choice_pill(
                eid2(
                    "mission-hist-kind",
                    task_id,
                    kind.map(history_kind_label).unwrap_or("all"),
                ),
                kind.map(history_kind_label).unwrap_or("all"),
                filter.kind == kind,
                cx.listener(move |this, _, _, cx| this.set_history_kind(kind, cx)),
            )
            .into_any_element()
        })
        .collect();

    // The tasks and agents that actually appear, so every pill filters to something.
    let mut tasks: Vec<TaskId> = Vec::new();
    let mut agents: Vec<ubiq_proto::work::AgentId> = Vec::new();
    for entry in entries {
        if let Some(task) = entry.task
            && !tasks.contains(&task)
        {
            tasks.push(task);
        }
        if let Some(agent) = entry.agent
            && !agents.contains(&agent)
        {
            agents.push(agent);
        }
    }
    let task_pills: Vec<AnyElement> = tasks
        .into_iter()
        .take(8)
        .map(|task| {
            let on = filter.task == Some(task);
            choice_pill(
                eid2("mission-hist-task", task_id, task),
                task_label(work, task),
                on,
                cx.listener(move |this, _, _, cx| this.set_history_task((!on).then_some(task), cx)),
            )
            .into_any_element()
        })
        .collect();
    let agent_pills: Vec<AnyElement> = agents
        .into_iter()
        .take(6)
        .map(|agent| {
            let on = filter.agent == Some(agent);
            let label = work
                .agent(agent)
                .map(|record| app.agent_label(record).title.to_string())
                .unwrap_or_else(|| "agent".to_string());
            choice_pill(
                eid2("mission-hist-agent", task_id, agent),
                label,
                on,
                cx.listener(move |this, _, _, cx| {
                    this.set_history_agent((!on).then_some(agent), cx)
                }),
            )
            .into_any_element()
        })
        .collect();

    let rows: Vec<AnyElement> = entries
        .iter()
        .filter(|entry| filter.admits(entry))
        .enumerate()
        .map(|(index, entry)| history_row(app, work, task_id, index, entry, cx))
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
                .flex_col()
                .gap_1p5()
                .child(div().flex().flex_wrap().gap_1p5().children(kinds))
                .when(!task_pills.is_empty(), |col| {
                    col.child(div().flex().flex_wrap().gap_1p5().children(task_pills))
                })
                .when(!agent_pills.is_empty(), |col| {
                    col.child(div().flex().flex_wrap().gap_1p5().children(agent_pills))
                }),
        )
        .when(rows.is_empty(), |body| {
            body.child(nothing("Nothing here yet."))
        })
        .children(rows)
        .into_any_element()
}

/// One timeline row: when, what kind, the sentence, and the thing it opens.
fn history_row(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    index: usize,
    entry: &HistoryEntry,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let _ = app;
    let task = entry.task;
    let agent = entry.agent;
    let revision = entry.plan_revision.is_some();
    let colour = match entry.kind {
        HistoryKind::Phase => theme::info(),
        HistoryKind::Plan => theme::accent(),
        HistoryKind::Agent => theme::accent(),
        HistoryKind::Review => theme::success(),
        HistoryKind::Feedback => theme::warning(),
        HistoryKind::Guidance => theme::warning(),
        HistoryKind::Status => theme::text_muted(),
        HistoryKind::Comment => theme::text_muted(),
    };
    let text: SharedString = match task {
        Some(task) => format!("{} \u{2014} {}", task_label(work, task), entry.text).into(),
        None => entry.text.clone().into(),
    };

    div()
        .id(eid2("mission-hist-row", task_id, index))
        .h(px(26.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|row| row.bg(theme::hover()))
        .on_click(cx.listener(move |this, _, _, cx| {
            // The plan revision, then the task, then the agent's chat — the most specific thing
            // the row names.
            if revision {
                this.open_plan(task_id, cx);
            } else if let Some(task) = task {
                this.select_task(task, cx);
            } else if let Some(agent) = agent {
                this.chat_with_mission_agent(agent, cx);
            }
        }))
        .child(div().size(px(6.)).flex_none().rounded_full().bg(colour))
        .child(mono(
            entry.at.format("%m-%d %H:%M").to_string(),
            theme::text_faint(),
        ))
        .child(
            div()
                .w(px(64.))
                .flex_none()
                .child(mono(history_kind_label(entry.kind), colour)),
        )
        .child(elided(
            eid2("mission-hist-text", task_id, index),
            text,
            theme::text(),
            theme::font(Family::Chrome, Role::Meta),
        ))
        .into_any_element()
}
