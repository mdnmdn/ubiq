//! The mission side panel — §6.1's fast overview.
//!
//! Top to bottom: the header, the phase line and the progress bar (chrome, never folds), then four
//! **sections** — *Needs you*, *Documents*, the agents, *Latest* (T-184) — each opened or shut on
//! its own through [`kit::disclosure`], and the feedback composer fixed at the foot. Short on
//! purpose: **nothing here needs reading twice**, and every detail the panel leaves out is the full
//! view's.
//!
//! **The section content scrolls; the chrome around it does not.** A mission with everything open
//! and a long roster no longer clips against the panel's bottom (T-184) — the sections sit in their
//! own `overflow_y_scroll` region between the fixed top and the fixed composer.
//!
//! **A section's open/shut state is remembered per mission** (`MissionView::shut_sections`,
//! `AppState::toggle_mission_section`) — UI-local bookkeeping, not a fact the host has an opinion
//! about, so two side panels open on different missions never share a shape.
//!
//! All of it is live. *Needs you* draws the pending phase move and every pending spawn off the
//! record; *Documents* draws the plan and `MissionRecord::documents`, each a button onto the
//! document surface; *Latest* draws the three newest journal lines; *Spawn ▾* is the one control
//! that puts an agent on a mission; and the composer sends the mission's feedback to its
//! coordinator.

use gpui::{
    AnyElement, Context, Focusable as _, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder, px,
    relative,
};
use gpui_component::IconName;

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{ExecutionMode, MissionRecord, Phase};
use ubiq_proto::work::{TaskRecord, WorkAgent};

use crate::app::AppState;
use crate::state::mission::MissionSection;
use crate::state::work::WorkProjection;
use crate::theme::{self, Family, Role};
use crate::ui::empty;
use crate::ui::kit::{
    UbiqIcon, disclosure, elided, ghost_button, hex_mark, icon_button, mono, panel, primary_button,
    section_label, state_chip,
};
use crate::ui::mission::full::doc_row;
use crate::ui::work::{activity_colour, work_state_colour};
use crate::ui::{eid, eid2};

/// How many roster rows are drawn before the panel says *+n more* (§6.1).
const AGENT_ROWS: usize = 5;

/// The phases the stepper walks. `Abandoned` is not on it — it is terminal from anywhere, so a
/// mission in it is drawn as one chip instead of as a step on a path it never reached.
pub(super) const STEPS: [Phase; 4] = [
    Phase::Requirements,
    Phase::Refining,
    Phase::InProgress,
    Phase::Completed,
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
        // The record outlives a window's projection of the task for as long as one `WorkList` is
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

    panel()
        .child(header(app, task_id, work, &term, task, mission, cx))
        .child(phase_line(task_id, mission, cx))
        .child(progress(work, task_id, cx))
        .child(
            div()
                .id(eid("mission-panel-scroll", task_id))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(needs_you_section(
                    app,
                    task_id,
                    mission,
                    view.section_open(task_id, MissionSection::NeedsYou),
                    cx,
                ))
                .child(documents(
                    task_id,
                    mission,
                    view.section_open(task_id, MissionSection::Documents),
                    cx,
                ))
                .child(agents(
                    app,
                    work,
                    task_id,
                    view.section_open(task_id, MissionSection::Agents),
                    cx,
                ))
                .child(latest(
                    app,
                    task_id,
                    view.section_open(task_id, MissionSection::Latest),
                    cx,
                )),
        )
        .child(feedback(app, task_id, &term, true, window, cx))
        .children(super::menu::overlay(app, task_id, cx))
        .children(super::menu::spawn_overlay(app, task_id, cx))
        .children(super::menu::kind_overlay(app, task_id, cx))
        .into_any_element()
}

/// One section's fold: the chevron bar, and the body only while it is open — every section of the
/// panel is this shape (T-184).
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

/// The mission term, the key, the title and the hexagon (M6, M18), then the two controls.
///
/// `⤢` opens the full view — the document tab in IDE mode, the modal everywhere else, which is
/// `AppState::open_mission_full`'s judgement rather than this row's. `⋯` is the mission's own
/// menu, `ui::mission::menu`, whose rows are `state::mission::MissionMenuRow`; the ones no
/// message stands behind are drawn dead rather than left out.
fn header(
    app: &AppState,
    task_id: TaskId,
    work: &WorkProjection,
    term: &str,
    task: &TaskRecord,
    mission: &MissionRecord,
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
            task_id,
            mission,
            eid("mission-hex", task_id),
            14.,
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

// ── the phase line ──────────────────────────────────────────────────

/// The mission's hexagon (M6).
///
/// **The outer ring is the worst thing happening in its roster** — `ui::work::worst_bucket`, the
/// one rule a session group's left bar already follows, read here rather than written a second
/// time. A mission nobody is on has no roster to read, so the ring falls back to the phase's own
/// colour instead of reporting "ended" about agents that never existed.
///
/// **The inner mark is the attention flag**, not the phase: anything pending draws `NeedsYou` in
/// the dictionary's own colour and pulses, exactly as a card's does, and a mission with nothing
/// outstanding draws the phase faded. Attention is a flag, not a phase — so it never displaces
/// what the ring or the stepper say.
pub fn mission_hex(
    work: &WorkProjection,
    task_id: TaskId,
    mission: &MissionRecord,
    id: gpui::ElementId,
    side: f32,
) -> AnyElement {
    let members = super::full::on_mission(work, task_id, mission);
    let ring = match members.is_empty() {
        true => phase_colour(mission.phase),
        false => crate::ui::work::bucket_colour(crate::ui::work::worst_bucket(
            members.iter().map(|agent| agent.activity.bucket()),
        )),
    };
    let attention = mission.pending_phase.is_some() || !mission.pending_spawns.is_empty();
    let fill = match attention {
        true => crate::ui::work::doing_colour(crate::state::status::Doing::NeedsYou),
        false => theme::fade(phase_colour(mission.phase), 0.25),
    };
    hex_mark(id, ring, Some(fill), side, attention)
}

/// A compact stepper over the phases, then how the mission is run (M22).
///
/// **Every step is live** (M5): clicking one sends `SetPhase` — a forward step the coordinator
/// asked for confirms its request, a step back or an abandon is the user's own move and is allowed
/// from any phase. The plan gate is the host's refusal to make, not this row's to pre-empt.
///
/// A step the coordinator is waiting on is drawn in the attention colour, so the thing to click is
/// the thing the *Needs you* row below is about.
pub(super) fn phase_steps(
    task_id: TaskId,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let phase = mission.phase;
    let reached = STEPS.iter().position(|p| *p == phase);
    let asked = mission.pending_phase.as_ref().map(|pending| pending.phase);

    let steps: Vec<AnyElement> = STEPS
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let step = *step;
            let passed = reached.is_some_and(|at| i <= at);
            let colour = match (asked == Some(step), passed) {
                (true, _) => crate::ui::work::doing_colour(crate::state::status::Doing::NeedsYou),
                (false, true) => phase_colour(phase),
                (false, false) => theme::text_faint(),
            };
            let tooltip: SharedString = match asked == Some(step) {
                true => format!("Confirm the move to {}", step.label()).into(),
                false => format!("Move to {}", step.label()).into(),
            };
            div()
                .id(eid2("mission-step", task_id, step.label()))
                .flex()
                .items_center()
                .cursor_pointer()
                .when(i > 0, |row| {
                    row.child(div().w(px(10.)).h(px(1.)).bg(theme::border()))
                })
                .child(
                    div()
                        .size(px(if reached == Some(i) { 9. } else { 7. }))
                        .flex_none()
                        .rounded_full()
                        .bg(colour),
                )
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_mission_phase(task_id, step, cx)),
                )
                .into_any_element()
        })
        .collect();

    div().flex().items_center().children(steps)
}

fn phase_line(
    task_id: TaskId,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let phase = mission.phase;
    let execution = match mission.execution {
        ExecutionMode::Auto => format!("auto ×{}", mission.parallelism),
        ExecutionMode::Manual => "manual".to_string(),
    };

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        // A mission that was abandoned never walked the path, so the dots are dropped for the one
        // word that is true. Its way back is the full view's Overview, whose stepper is drawn in
        // every phase.
        .when(phase != Phase::Abandoned, |row| {
            row.child(phase_steps(task_id, mission, cx))
        })
        .child(mono(
            format!("{} · {execution}", phase.label()),
            theme::text_muted(),
        ))
}

// ── the progress bar ────────────────────────────────────────────────

/// One segmented bar over the mission's children, in M26's colours, with the counts under it.
///
/// **A segment and its count are one control**: either opens the full view's Tasks tab filtered
/// to that state (§6.1), which is the whole reason both surfaces share one `MissionView`.
fn progress(
    work: &WorkProjection,
    task_id: TaskId,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let counts = work.work_state_counts(task_id);
    let total: usize = counts.iter().map(|(_, n)| n).sum();

    if total == 0 {
        return div()
            .px_3()
            .pb_2()
            .flex()
            .flex_none()
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

// ── the sections ────────────────────────────────────────────────────

/// A section's bar: the heading, and whatever the section wants readable beside it.
pub(super) fn section_bar(title: &str, note: Option<String>) -> impl IntoElement {
    section_bar_with(title, note, None)
}

/// The same bar with a control at its right end — *Spawn ▾*, and the kinds table's `+`.
///
/// Chrome is flush: the control sits in the bar's own height with no margin of its own.
pub(super) fn section_bar_with(
    title: &str,
    note: Option<String>,
    action: Option<AnyElement>,
) -> impl IntoElement {
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
        .when_some(action, |row, action| {
            row.child(div().flex_1().min_w(px(0.))).child(action)
        })
}

/// The mission's *Spawn ▾* — the one control that puts an agent on a mission (§6.1).
pub(super) fn spawn_button(
    app: &AppState,
    task_id: TaskId,
    id: &'static str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let open = app
        .workbench
        .mission_spawn_menu
        .is_some_and(|menu| menu.task == task_id);
    div()
        .id(eid(id, task_id))
        .h(px(20.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .cursor_pointer()
        .border_1()
        .border_color(match open {
            true => theme::accent(),
            false => theme::border(),
        })
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text())
        .child("Spawn \u{25be}")
        .tooltip(|window, cx| {
            gpui_component::tooltip::Tooltip::new("Put an agent on this mission").build(window, cx)
        })
        .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
            let at = event.position();
            this.open_mission_spawn_menu(task_id, (at.x.into(), at.y.into()), cx);
        }))
        .into_any_element()
}

/// One line of honest nothing — what a section owned by later work draws.
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

/// What is waiting on the user (M6), drawn from the record and nothing else.
///
/// **Two sources.** The pending phase move, and every agent a member has asked for and nobody has
/// launched (M13). Both ride `MissionChanged` on the record — `pending_phase` and
/// `pending_spawns` — so this section is a read: no second store, no query.
///
/// The spawns are **plural** where the phase is singular, and that is the whole reason the rows
/// are a vector: two members each waiting on a worker are two questions, and answering one must
/// not throw the other away.
///
/// The rows themselves — the pending phase move, then every pending spawn — shared by the side
/// panel's foldable section ([`needs_you_section`]) and the full view's always-open one below.
fn needs_you_rows(
    app: &AppState,
    task_id: TaskId,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> Vec<AnyElement> {
    let mut rows: Vec<AnyElement> = Vec::new();
    if let Some(pending) = mission.pending_phase.as_ref() {
        rows.push(phase_request_row(task_id, mission, pending, cx));
    }
    for spawn in &mission.pending_spawns {
        rows.push(spawn_request_row(app, task_id, spawn, cx));
    }
    rows
}

/// `compact` is the side panel: the first row and a count of the rest (§6.1). The full view's
/// Overview passes `false` and gets the lot.
pub(super) fn needs_you(
    app: &AppState,
    task_id: TaskId,
    mission: &MissionRecord,
    compact: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let rows = needs_you_rows(app, task_id, mission, cx);
    let total = rows.len();
    let shown = match compact {
        true => rows.into_iter().take(1).collect::<Vec<_>>(),
        false => rows,
    };

    div()
        .flex()
        .flex_none()
        .flex_col()
        .child(section_bar(
            "Needs you",
            (total > 0).then(|| total.to_string()),
        ))
        .when(total == 0, |body| body.child(nothing("Nothing needs you.")))
        .children(shown)
        .when(compact && total > 1, |body| {
            body.child(nothing_count(total - 1))
        })
        .into_any_element()
}

/// The side panel's own *Needs you* (T-184): the same rows as [`needs_you`] with `compact: true`,
/// under a fold rather than a plain bar, and drawing nothing beneath it while shut.
fn needs_you_section(
    app: &AppState,
    task_id: TaskId,
    mission: &MissionRecord,
    open: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let rows = needs_you_rows(app, task_id, mission, cx);
    let total = rows.len();
    let shown: Vec<AnyElement> = rows.into_iter().take(1).collect();

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
        .children(shown)
        .when(total > 1, |body| body.child(nothing_count(total - 1)))
        .into_any_element()
}

/// One pending phase request: what was asked for, why, who asked — and the two answers.
///
/// **Both answers are `SetPhase`** (`AppState::set_mission_phase`'s doc gives the rule): Confirm
/// names the phase asked for, Decline names the phase the mission is already in. There is no third
/// message and no "dismiss" — a request the user walks away from stays pending, which is what a
/// question nobody answered is.
fn phase_request_row(
    task_id: TaskId,
    mission: &MissionRecord,
    pending: &ubiq_proto::mission::PendingPhase,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let asked = pending.phase;
    let summary: SharedString = match pending.summary.trim().is_empty() {
        true => format!(
            "{} asks to move this to {}.",
            super::full::actor(pending.by),
            asked.label()
        )
        .into(),
        false => pending.summary.clone().into(),
    };
    let held = mission.phase;

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(state_chip(
                    asked.label(),
                    crate::ui::work::doing_colour(crate::state::status::Doing::NeedsYou),
                    1.0,
                ))
                .child(elided(
                    eid("mission-pending-summary", task_id),
                    summary,
                    theme::text(),
                    theme::font(Family::Chrome, Role::Body),
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(mono(
                    format!(
                        "asked by {} \u{00b7} {}",
                        super::full::actor(pending.by),
                        pending.at.format("%Y-%m-%d %H:%M")
                    ),
                    theme::text_faint(),
                ))
                .child(div().flex_1().min_w(px(0.)))
                .child(ghost_button(
                    eid("mission-pending-decline", task_id),
                    None,
                    "Decline",
                    cx.listener(move |this, _, _, cx| this.decline_mission_phase(task_id, cx)),
                ))
                .child(primary_button(
                    eid("mission-pending-confirm", task_id),
                    None,
                    "Confirm",
                    cx.listener(move |this, _, _, cx| this.set_mission_phase(task_id, asked, cx)),
                )),
        )
        // The phase the decline puts it back in, said where the decline is — the one fact the two
        // buttons' labels have no room for.
        .child(mono(
            format!("declining keeps it in {}", held.label()),
            theme::text_faint(),
        ))
        .into_any_element()
}

/// One pending spawn: what was asked for, by whom and why — and the three controls (M13).
///
/// **The kind is a control, not a label.** The user may change what will actually be launched
/// before allowing it, which is exactly why `SpawnOutcome::Launched` carries the kind at all: what
/// the requester is told is what ran, not what it asked for. The pick is the window's
/// (`MissionView::spawn_picks`) and never touches the record.
fn spawn_request_row(
    app: &AppState,
    task_id: TaskId,
    spawn: &ubiq_proto::mission::PendingSpawn,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = spawn.id;
    let pick = app.mission_spawn_pick(task_id, spawn);
    let changed = pick.kind != spawn.kind || pick.definition != spawn.definition;
    let chip: SharedString = match &pick.definition {
        Some(definition) => format!("{} \u{00b7} {definition}", pick.kind).into(),
        None => pick.kind.clone().into(),
    };
    let reason: SharedString = match spawn.reason.trim().is_empty() {
        true => format!(
            "{} asks for a {} agent.",
            super::full::actor(spawn.by),
            spawn.kind
        )
        .into(),
        false => spawn.reason.clone().into(),
    };

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                // The kind, as a control: clicking it offers the mission's kinds and, for the
                // `custom` case, a definition named outright.
                .child(
                    div()
                        .id(eid2("mission-spawn-kind", task_id, id))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                            let at = event.position();
                            this.open_mission_kind_menu(
                                task_id,
                                crate::state::mission::KindTarget::Pending(id),
                                (at.x.into(), at.y.into()),
                                cx,
                            );
                        }))
                        .tooltip(|window, cx| {
                            gpui_component::tooltip::Tooltip::new(
                                "Change what will be launched before allowing it",
                            )
                            .build(window, cx)
                        })
                        .child(state_chip(
                            chip,
                            match changed {
                                true => theme::accent(),
                                false => crate::ui::work::doing_colour(
                                    crate::state::status::Doing::NeedsYou,
                                ),
                            },
                            1.0,
                        )),
                )
                .child(elided(
                    eid2("mission-spawn-reason", task_id, id),
                    reason,
                    theme::text(),
                    theme::font(Family::Chrome, Role::Body),
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(mono(
                    format!(
                        "asked by {} \u{00b7} {}",
                        super::full::actor(spawn.by),
                        spawn.at.format("%Y-%m-%d %H:%M")
                    ),
                    theme::text_faint(),
                ))
                .child(div().flex_1().min_w(px(0.)))
                .child(ghost_button(
                    eid2("mission-spawn-decline", task_id, id),
                    None,
                    "Decline",
                    cx.listener(move |this, _, _, cx| this.refuse_mission_spawn(task_id, id, cx)),
                ))
                .child(primary_button(
                    eid2("mission-spawn-allow", task_id, id),
                    None,
                    "Launch",
                    cx.listener(move |this, _, _, cx| this.allow_mission_spawn(task_id, id, cx)),
                )),
        )
        .when(changed, |body| {
            body.child(mono(
                format!("asked for {}", spawn.kind),
                theme::text_faint(),
            ))
        })
        .into_any_element()
}

/// The mission's own documents, and its plan (T-184, M8): one row per name in
/// `MissionRecord::documents`, plus the plan's own row, each a button onto the document surface
/// [`super::full::doc_row`] already draws for the full view's *Plan & docs* tab — one row shape,
/// drawn from both surfaces rather than redrawn for this one.
///
/// **`documents` is a read, never a query.** The host derives it from `missions/<TaskId>/docs/` on
/// every `MissionChanged`, so the row list here is exactly what a fresh directory listing would be
/// — nothing here asks the host to enumerate anything a second time.
fn documents(
    task_id: TaskId,
    mission: &MissionRecord,
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
        "Plan",
        cx.listener(move |this, _, _, cx| this.open_plan(task_id, cx)),
    ));
    for name in &mission.documents {
        let doc_name = name.clone();
        body = body.child(doc_row(
            eid2("mission-open-doc", task_id, name),
            name.clone(),
            cx.listener(move |this, _, _, cx| this.open_mission_doc(task_id, &doc_name, cx)),
        ));
    }
    body.into_any_element()
}

/// Who is on the mission, and the one control that puts somebody on it (§6.1).
///
/// **The record's roster is the membership** (M11) — assigned to the mission or a child, *or*
/// spawned by a member, which cannot be recomputed and so is written down. Agents merely pointing
/// at one of the mission's tasks that the roster has not caught up with yet are drawn too, because
/// the assignment is what the roster is settled from and the two are a frame apart.
fn agents(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    open: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(record) = app.mission(task_id, cx) else {
        return div().flex().flex_none().flex_col().into_any_element();
    };
    let on_mission = super::full::on_mission(work, task_id, record);
    let total = on_mission.len();

    let bar = fold(
        task_id,
        MissionSection::Agents,
        "Agents",
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

    let rows: Vec<AnyElement> = on_mission
        .iter()
        .take(AGENT_ROWS)
        .map(|agent| agent_row(app, work, task_id, agent, cx))
        .collect();

    body.child(div().px_3().py_1().flex().flex_none().child(spawn_button(
        app,
        task_id,
        "mission-spawn",
        cx,
    )))
    .when(total == 0, |body| {
        body.child(nothing("No agents on this mission."))
    })
    .children(rows)
    .when(total > AGENT_ROWS, |body| {
        body.child(nothing_count(total - AGENT_ROWS))
    })
    .into_any_element()
}

fn nothing_count(more: usize) -> impl IntoElement {
    div()
        .px_3()
        .pb_2()
        .flex()
        .flex_none()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_faint())
        .child(format!("+{more} more"))
}

/// One roster row: the hexagon in the agent's own activity colour, its name, its model, the task
/// it is on, and the one action a panel row has room for.
///
/// **`Chat` becomes `Resume` when nothing is loaded behind the agent.** A conversation whose
/// harness has been unloaded is still a conversation — its transcript and its run directory are
/// there — so the honest offer is to start it again, not to open a panel onto a dead one.
fn agent_row(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    agent: &WorkAgent,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let _ = task_id;
    let id = agent.id;
    let live = app.conversation_live(id);
    let colour = activity_colour(agent.activity);
    let on: SharedString = agent
        .task
        .and_then(|id| work.task(id))
        .map(|task| match task.key.clone() {
            Some(key) => SharedString::from(key),
            None => SharedString::from(task.title.clone()),
        })
        .unwrap_or_else(|| SharedString::from(agent.note.clone()));

    div()
        .h(px(26.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(hex_mark(
            eid("mission-agent", agent.id),
            colour,
            Some(theme::fade(colour, 0.25)),
            11.,
            false,
        ))
        .child(elided(
            eid("mission-agent-name", agent.id),
            app.agent_title(agent),
            theme::text(),
            theme::font(Family::Chrome, Role::Label),
        ))
        .when(!agent.model.is_empty(), |row| {
            row.child(mono(agent.model.clone(), theme::text_faint()))
        })
        .child(mono(on, theme::text_muted()))
        .child(div().flex_1().min_w(px(0.)))
        .child(match live {
            true => ghost_button(
                eid("mission-agent-chat", id),
                None,
                "Chat",
                cx.listener(move |this, _, _, cx| this.chat_with_mission_agent(id, cx)),
            ),
            false => ghost_button(
                eid("mission-agent-resume", id),
                None,
                "Resume",
                cx.listener(move |this, _, _, cx| this.resume_mission_agent(id, cx)),
            ),
        })
        .into_any_element()
}

/// The last three journal lines (M12). The page is asked for once, where the panel is opened —
/// see `AppState::load_mission_journal` — and kept current by `JournalAppended`.
fn latest(app: &AppState, task_id: TaskId, open: bool, cx: &mut Context<AppState>) -> AnyElement {
    let bar = fold(task_id, MissionSection::Latest, "Latest", div(), open, cx);
    let body = div().flex().flex_none().flex_col().child(bar);
    if !open {
        return body.into_any_element();
    }

    let entries = app.mission_journal(task_id);
    let rows: Vec<AnyElement> = entries
        .map(|journal| {
            journal
                .entries
                .iter()
                .take(3)
                .map(journal_row)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    body.when(rows.is_empty(), |body| {
        body.child(nothing("No activity yet."))
    })
    .children(rows)
    .into_any_element()
}

/// One journal line: what kind it was, when, and the sentence the host wrote beside it.
///
/// **The sentence is `JournalEntry::text`, never composed here.** It is written where the event
/// was, so a window and an agent reading the same line read the same words.
pub(super) fn journal_row(entry: &ubiq_proto::mission::JournalEntry) -> AnyElement {
    let kind = entry.event.kind();
    div()
        .px_3()
        .py_1()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(
            div()
                .size(px(6.))
                .flex_none()
                .rounded_full()
                .bg(journal_colour(kind)),
        )
        .child(mono(
            entry.at.format("%H:%M").to_string(),
            theme::text_faint(),
        ))
        .child(elided(
            eid("mission-journal", entry.seq),
            entry.text.clone(),
            theme::text_muted(),
            theme::font(Family::Chrome, Role::Meta),
        ))
        .into_any_element()
}

/// What a journal kind reads as. Status rides colour, never wording alone — and the two kinds M13
/// adds are drawn in the attention colour, because a spawn is the one event on this list the user
/// is ever asked about.
pub(super) fn journal_colour(kind: &str) -> gpui::Rgba {
    match kind {
        "spawn_requested" | "spawn_answered" | "ask_raised" => {
            crate::ui::work::doing_colour(crate::state::status::Doing::NeedsYou)
        }
        "phase_requested" | "phase_changed" => theme::info(),
        "task_finished" | "ask_answered" => theme::success(),
        "agent_joined" | "agent_left" | "agent_role_changed" => theme::accent(),
        "feedback" => theme::warning(),
        _ => theme::text_muted(),
    }
}

/// The one-line composer, live (M12).
///
/// **Enter and Send are the same path** — `AppState::send_mission_feedback`, which journals the
/// line through `SendToAgent` and puts it in front of the coordinator as a turn, or on
/// `Conversation::queued` while one is running. `panel` picks which of the window's two fields
/// this is, because the side panel and the *Activity* tab can be on screen at once and one
/// `InputState` drawn twice is one caret in two places.
///
/// A mission with no coordinator draws the field faint and says so: there is nobody to tell, and
/// a Send that quietly did nothing would be the interface pretending.
pub(super) fn feedback(
    app: &AppState,
    task_id: TaskId,
    term: &str,
    panel: bool,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let input = match panel {
        true => app.mission_feedback_input.clone(),
        false => app.mission_feedback_tab_input.clone(),
    };
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    let has_coordinator = app
        .mission(task_id, cx)
        .is_some_and(|record| record.coordinator.is_some());
    let id = match panel {
        true => "mission-feedback-send",
        false => "mission-feedback-tab-send",
    };

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .border_t_1()
        .border_color(theme::border())
        .child(
            crate::ui::kit::field(
                match has_coordinator {
                    true => theme::accent(),
                    false => theme::border(),
                },
                focused,
            )
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
        .child(match has_coordinator {
            true => ghost_button(
                eid(id, task_id),
                None,
                "Send",
                cx.listener(move |this, _, window, cx| {
                    this.submit_mission_feedback(panel, window, cx)
                }),
            )
            .into_any_element(),
            false => mono(
                format!("no {} coordinator to tell", term.to_lowercase()),
                theme::text_faint(),
            )
            .into_any_element(),
        })
}

/// What a phase reads as. The path's own colour while it is being walked, the success token when
/// it is finished, and the muted one for a mission nobody is taking further.
pub fn phase_colour(phase: Phase) -> gpui::Rgba {
    match phase {
        Phase::Requirements | Phase::Refining => theme::info(),
        Phase::InProgress => theme::accent(),
        Phase::Completed => theme::success(),
        Phase::Abandoned => theme::text_muted(),
    }
}
