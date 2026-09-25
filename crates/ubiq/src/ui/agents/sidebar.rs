//! The list down the side of the agents screen: every mission, every session, every agent in it,
//! and what each one is doing.
//!
//! It lists **every conversation this window holds**, not what is on screen. That is the point of
//! it: a column is one conversation and there are only ever a few of them, so the list is the one
//! place the whole project is visible at once — and an agent the user has benched is still here,
//! marked, rather than gone. What it does not list is an agent this window cannot talk to, which
//! is what `AgentsView::live_agents` answers for every reader on this screen.
//!
//! **The Missions section sits above the sessions (M14)**, one row per mission not `Completed` or
//! `Abandoned`, and folds to a roster rather than to nothing — a mission is what the user came for,
//! a session is what carries it. **The session groups below stay exactly as they are.** An agent
//! inside a mission carries a small chip there rather than being lifted out: one conversation is
//! never listed as two different things, and that rule is the whole point of the section above it.
//!
//! One click reveals: an agent already in a column comes to the front of it, and a benched one
//! opens a column of its own. A session's row folds it away; a mission's own chevron folds it to
//! its roster, and clicking the rest of its row opens the mission panel instead.

use std::collections::HashMap;

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::scroll::Scrollbar;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::mission::{MissionRecord, Phase};
use ubiq_proto::work::{AgentId, WorkAgent, WorkSession};

use crate::app::AppState;
use crate::state::agents::AgentsView;
use crate::state::work::WorkProjection;
use crate::theme;
use crate::ui::eid;
use crate::ui::kit::{
    badge, elided, elided_with, icon_button, mono, panel, panel_header, section_label, state_chip,
    status_dot,
};
use crate::ui::mission::panel::{mission_hex, phase_colour};
use crate::ui::work::activity_colour;

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    if let (Some(work), Some(agents)) = (app.work(cx), app.agents(cx)) {
        rows.extend(missions_section(app, work, agents, cx));

        // Which mission each agent belongs to, by its own active roster — built once here rather
        // than searched per row, since every agent row asks. A chip rather than a second listing:
        // the agent stays right where its session already puts it.
        let chips = mission_chips(app, work, cx);

        for session in &work.sessions {
            let members: Vec<&WorkAgent> = agents
                .live_agents(work)
                .into_iter()
                .filter(|agent| agent.session == session.id)
                .collect();
            // A session nobody is working in is not drawn: the list is about agents, and a header
            // with nothing under it says only that the session exists.
            if members.is_empty() {
                continue;
            }
            let shut = agents.is_collapsed(session.id);

            // **The edge belongs to the group, not to its header.** It carries the worst thing
            // happening in the session, and a bar down the whole group is what makes the rows
            // under it read as that session's rather than as a flat list with a heading in it.
            let mut group = div()
                .flex()
                .flex_none()
                .flex_col()
                .border_l(px(theme::accent_edge()))
                .border_color(worst_of(&members))
                .child(session_row(session, members.len(), shut, cx));

            if !shut {
                group = group
                    .child(note_row(work, session))
                    .children(members.into_iter().map(|agent| {
                        agent_row(
                            agent,
                            !agents.on_screen(agent.id),
                            chips.get(&agent.id).cloned(),
                            cx,
                        )
                    }));
            }
            rows.push(group.into_any_element());
        }
    }

    panel()
        .child(panel_header("Agents", collapse_all(app, cx)))
        .child(
            div()
                .relative()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .border_t_1()
                .border_color(theme::border())
                .child(
                    div()
                        .id("agents-sidebar")
                        .size_full()
                        .flex()
                        .flex_col()
                        .overflow_y_scroll()
                        .track_scroll(&app.agents_scroll)
                        .children(if rows.is_empty() {
                            vec![
                                div()
                                    .p_3()
                                    .child(mono("nothing running", theme::text_faint()))
                                    .into_any_element(),
                            ]
                        } else {
                            rows
                        }),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Scrollbar::vertical(&app.agents_scroll)),
                ),
        )
}

/// The one control in the header: fold every session, or open every one. The Missions section
/// folds separately, row by row — it is a short list read at a glance, not a wall of sessions this
/// switch exists to tame.
fn collapse_all(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let sessions: Vec<_> = app
        .work(cx)
        .map(|work| work.sessions.iter().map(|s| s.id).collect::<Vec<_>>())
        .unwrap_or_default();
    let all_shut = app.agents(cx).is_some_and(|agents| {
        !sessions.is_empty() && sessions.iter().all(|id| agents.is_collapsed(*id))
    });

    div()
        .id("agents-fold-all")
        .size(px(22.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(IconName::ChevronsUpDown)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            for id in &sessions {
                let shut = this
                    .agents(cx)
                    .is_some_and(|agents| agents.is_collapsed(*id));
                if shut == all_shut {
                    this.toggle_agents_session(*id, cx);
                }
            }
        }))
        .into_any_element()
}

// ── the Missions section (M14) ────────────────────────────────────────

/// Every mission not `Completed` or `Abandoned`, most recently active first — closed ones sorted
/// last and drawn only once [`AgentsView::show_closed_missions`] is on, the same "most recently
/// active work" judgement [`AppState::open_missions`] and the board's own mission filter already
/// make.
fn missions_section(
    app: &AppState,
    work: &WorkProjection,
    agents: &AgentsView,
    cx: &mut Context<AppState>,
) -> Vec<AnyElement> {
    let Some(open) = app.open_project(cx) else {
        return Vec::new();
    };
    let has_closed = open
        .missions
        .values()
        .any(|record| matches!(record.phase, Phase::Completed | Phase::Abandoned));
    let show_closed = agents.show_closed_missions;
    let mut missions: Vec<(&ubiq_proto::work::TaskRecord, &MissionRecord)> = open
        .missions
        .values()
        .filter(|record| {
            show_closed || !matches!(record.phase, Phase::Completed | Phase::Abandoned)
        })
        .filter_map(|record| work.task(record.task_id).map(|task| (task, record)))
        .collect();
    // The header — and `New mission` on it — is drawn whenever a project is open, whether or not
    // it has a mission yet: that button is how the first one gets made.
    missions.sort_by(|(a_task, a_rec), (b_task, b_rec)| {
        let rank = |phase: Phase| matches!(phase, Phase::Completed | Phase::Abandoned) as u8;
        rank(a_rec.phase)
            .cmp(&rank(b_rec.phase))
            .then_with(|| b_task.updated_at.cmp(&a_task.updated_at))
    });

    let mut rows: Vec<AnyElement> = vec![missions_header(has_closed, show_closed, cx)];
    for (task, record) in missions {
        rows.push(mission_row(app, work, agents, task, record, cx));
        if agents.is_mission_expanded(task.id) {
            rows.extend(mission_roster(work, agents, record, cx));
        }
    }
    rows
}

/// The section's own header: the label, `New agent` beside `New mission` (M14's own last line),
/// and the *show closed* toggle — drawn only when there is something behind it to show.
fn missions_header(has_closed: bool, show_closed: bool, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .h(px(26.))
        .pl_3()
        .pr_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(section_label("Missions"))
        .child(div().flex_1().min_w(px(0.)))
        .children(has_closed.then(|| {
            div()
                .id("agents-missions-show-closed")
                .h(px(20.))
                .px_1p5()
                .flex()
                .flex_none()
                .items_center()
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(
                    mono(
                        if show_closed {
                            "hide closed"
                        } else {
                            "show closed"
                        },
                        theme::text_faint(),
                    )
                    .text_size(theme::font(theme::Family::Conversation, theme::Role::Meta)),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_show_closed_missions(cx)))
                .into_any_element()
        }))
        .child(
            icon_button(
                "agents-missions-new-agent",
                IconName::Bot,
                false,
                cx.listener(|this, _, window, cx| this.open_new_agent_direct(window, cx)),
            )
            .size(px(22.))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("New agent").build(window, cx)
            }),
        )
        .child(
            icon_button(
                "agents-missions-new",
                IconName::Plus,
                false,
                cx.listener(|this, _, window, cx| this.open_new_mission(window, cx)),
            )
            .size(px(22.))
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("New mission").build(window, cx)
            }),
        )
        .into_any_element()
}

/// One mission's row: the hexagon [`mission_hex`] draws everywhere else, a phase chip, the key and
/// title, the roster's size and a *needs you* dot from `pending_phase`. The chevron alone expands
/// to the roster; the rest of the row opens the mission panel, exactly as a session's members open
/// a column.
fn mission_row(
    app: &AppState,
    work: &WorkProjection,
    agents: &AgentsView,
    task: &ubiq_proto::work::TaskRecord,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let task_id = task.id;
    let expanded = agents.is_mission_expanded(task_id);
    let roster_size = roster_of(work, agents, mission).len();
    let needs_you = mission.pending_phase.is_some();
    let term = app.mission_term(cx);

    div()
        .id(eid("agents-mission-row", task_id))
        .h(px(28.))
        .pl_1()
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .hover(|this| this.bg(theme::hover()))
        .child(
            div()
                .id(eid("agents-mission-fold", task_id))
                .size(px(16.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .with_size(Size::XSmall)
                    .text_color(theme::text_muted()),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_mission_row(task_id, cx))),
        )
        .child(mission_hex(
            work,
            task_id,
            mission,
            eid("agents-mission-hex", task_id),
            13.,
        ))
        .child(state_chip(
            mission.phase.label(),
            phase_colour(mission.phase),
            0.85,
        ))
        .child(
            div()
                .id(eid("agents-mission-open", task_id))
                .flex()
                .flex_1()
                .min_w(px(0.))
                .items_center()
                .gap_1p5()
                .cursor_pointer()
                .children(task.key.clone().map(|key| mono(key, theme::text_muted())))
                .child(elided(
                    eid("agents-mission-title", task_id),
                    task.title.clone(),
                    theme::text(),
                    theme::font(theme::Family::Conversation, theme::Role::Body),
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.open_mission_panel(task_id, cx))),
        )
        .children(needs_you.then(|| {
            status_dot(
                crate::ui::work::doing_colour(crate::state::status::Doing::NeedsYou),
                theme::pane_bg(),
            )
        }))
        .child(
            mono(format!("{roster_size}"), theme::text_faint())
                .text_size(theme::font(theme::Family::Conversation, theme::Role::Meta)),
        )
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(term.clone()).build(window, cx)
        })
        .into_any_element()
}

/// The mission's roster, drawn while its row is expanded — the same click [`agent_row`]'s own
/// session members answer with, opening a column.
fn mission_roster(
    work: &WorkProjection,
    agents: &AgentsView,
    mission: &MissionRecord,
    cx: &mut Context<AppState>,
) -> Vec<AnyElement> {
    let roster = roster_of(work, agents, mission);
    if roster.is_empty() {
        return vec![
            div()
                .pl_5()
                .pr_3()
                .pb_1()
                .flex()
                .flex_none()
                .child(mono("no agents yet", theme::text_faint()))
                .into_any_element(),
        ];
    }
    roster
        .into_iter()
        .map(|agent| agent_row(agent, !agents.on_screen(agent.id), None, cx))
        .collect()
}

/// A mission's active roster, resolved to the agents this window can actually talk to — the same
/// narrowing every reader on this screen goes through, `AgentsView::live_agents`.
fn roster_of<'a>(
    work: &'a WorkProjection,
    agents: &AgentsView,
    mission: &MissionRecord,
) -> Vec<&'a WorkAgent> {
    mission
        .roster
        .iter()
        .filter(|entry| entry.left_at.is_none())
        .filter_map(|entry| work.agent(entry.agent))
        .filter(|agent| agents.is_live(agent.id))
        .collect()
}

/// Which mission each live agent belongs to — its label (key, or title) and the phase colour the
/// rest of this window's mission rows already wear — read once from every mission's active roster
/// rather than searched per agent row.
fn mission_chips(
    app: &AppState,
    work: &WorkProjection,
    cx: &Context<AppState>,
) -> HashMap<AgentId, (String, Rgba)> {
    let Some(open) = app.open_project(cx) else {
        return HashMap::new();
    };
    open.missions
        .values()
        .flat_map(|record| {
            let colour = phase_colour(record.phase);
            let label = work
                .task(record.task_id)
                .map(|task| task.key.clone().unwrap_or_else(|| task.title.clone()))
                .unwrap_or_default();
            record
                .roster
                .iter()
                .filter(|entry| entry.left_at.is_none())
                .map(move |entry| (entry.agent, (label.clone(), colour)))
                .collect::<Vec<_>>()
        })
        .collect()
}

// ── sessions (unchanged in shape) ──────────────────────────────────────

/// A session's header: whether it is folded, its name, whether it has a worktree of its own, and
/// how many agents are in it. The whole row folds the group.
fn session_row(
    session: &WorkSession,
    members: usize,
    shut: bool,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = session.id;

    div()
        .id(eid("agents-session", id))
        .h(px(28.))
        .pl_1()
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        // The whole row folds, so the chevron is a mark rather than a control of its own.
        .child(
            div()
                .size(px(16.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(if shut {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(Size::XSmall)
                    .text_color(theme::text_muted()),
                ),
        )
        .child(elided(
            eid("agents-session-name", id),
            session.name.clone(),
            theme::text(),
            theme::font(theme::Family::Conversation, theme::Role::Body),
        ))
        .children(session.worktree.then(|| section_label("worktree")))
        .child(
            mono(format!("{members}"), theme::text_faint())
                .text_size(theme::font(theme::Family::Conversation, theme::Role::Meta)),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_agents_session(id, cx)))
        .into_any_element()
}

/// What the session is for, in one line: the title of the task being worked on in it.
///
/// Read off the work rather than carried on the session, because a session has no description on
/// the wire and inventing one would be a field nobody wrote. A session with no task in flight
/// draws no line at all.
fn note_row(work: &WorkProjection, session: &WorkSession) -> AnyElement {
    let Some(task) = work
        .tasks
        .iter()
        .find(|task| task.session == Some(session.id))
    else {
        return div().into_any_element();
    };

    div()
        .pl_5()
        .pr_3()
        .pb_1()
        .flex()
        .flex_none()
        .child(elided(
            eid("agents-session-note", session.id),
            task.title.clone(),
            theme::text_muted(),
            theme::font(theme::Family::Conversation, theme::Role::Label),
        ))
        .into_any_element()
}

/// One agent: what it is doing, whether it is on the bench, and — inside a mission (M14) — a small
/// chip naming it. The chip is the whole answer to "one conversation is never listed as two
/// different things": the agent stays right here, under its session, and the mission is a fact
/// about it rather than a second place it lives.
fn agent_row(
    agent: &WorkAgent,
    benched: bool,
    mission: Option<(String, Rgba)>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = agent.id;
    let colour = activity_colour(agent.activity);

    div()
        .id(eid("agents-row", id))
        .h(px(26.))
        .pl_5()
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(status_dot(colour, theme::pane_bg()))
        // What the conversation is about, where something has named it — otherwise the name in
        // full, which is what an elided row says on hover anyway.
        .child(elided_with(
            eid("agents-row-name", id),
            agent.name.clone(),
            agent.summary.clone().unwrap_or_else(|| agent.name.clone()),
            if benched {
                theme::text_muted()
            } else {
                theme::text()
            },
            theme::font(theme::Family::Conversation, theme::Role::Body),
        ))
        .children(mission.map(|(label, colour)| badge(&label, colour).into_any_element()))
        // The one mark on the row that is about this window rather than about the agent: it is not
        // on screen, and clicking the row is what puts it back.
        .children(benched.then(|| badge("bench", theme::text_faint())))
        .child(
            mono(agent.activity.label().to_lowercase(), colour)
                .text_size(theme::font(theme::Family::Conversation, theme::Role::Meta)),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.reveal_agent(id, cx)))
        .into_any_element()
}

/// The worst thing happening in a group: an error over a wait over movement over nothing. A folded
/// session is read at a glance, so its edge says what the user would want to be told first — the
/// same rule `WorkProjection::pulse` follows for a task's card, which is why a failing agent stays
/// visible with its group shut.
fn worst_of(members: &[&WorkAgent]) -> gpui::Rgba {
    crate::ui::work::bucket_colour(crate::ui::work::worst_bucket(
        members.iter().map(|agent| agent.activity.bucket()),
    ))
}
