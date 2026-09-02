//! The list down the side of the agents screen: every session, every agent in it, and what each
//! one is doing.
//!
//! It lists **everything the host reports**, not what is on screen. That is the point of it: a
//! column is one conversation and there are only ever a few of them, so the list is the one place
//! the whole project is visible at once — and an agent the user has benched is still here, marked,
//! rather than gone.
//!
//! One click reveals: an agent already in a column comes to the front of it, and a benched one
//! opens a column of its own. A session's row folds it away.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::scroll::Scrollbar;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::work::{Bucket, WorkAgent, WorkSession};

use crate::app::AppState;
use crate::state::work::WorkProjection;
use crate::theme;
use crate::ui::eid;
use crate::ui::kit::{badge, elided, mono, panel, panel_header, section_label, status_dot};
use crate::ui::work::activity_colour;

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let mut groups: Vec<AnyElement> = Vec::new();
    if let (Some(work), Some(agents)) = (app.work(cx), app.agents(cx)) {
        for session in &work.sessions {
            let members: Vec<&WorkAgent> = work
                .agents
                .iter()
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
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(worst_of(&members))
                .child(session_row(session, members.len(), shut, cx));

            if !shut {
                group = group.child(note_row(work, session)).children(
                    members
                        .into_iter()
                        .map(|agent| agent_row(agent, !agents.on_screen(agent.id), cx)),
                );
            }
            groups.push(group.into_any_element());
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
                        .children(if groups.is_empty() {
                            vec![
                                div()
                                    .p_3()
                                    .child(mono("nothing running", theme::text_faint()))
                                    .into_any_element(),
                            ]
                        } else {
                            groups
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

/// The one control in the header: fold every session, or open every one.
///
/// It reads as a switch rather than two buttons because the answer is one bit — with any session
/// open it folds them all, and with all of them folded it opens them.
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
            13.,
        ))
        .children(session.worktree.then(|| section_label("worktree")))
        .child(mono(format!("{members}"), theme::text_faint()).text_size(px(11.)))
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
            12.,
        ))
        .into_any_element()
}

/// One agent: what it is doing, and whether it is on the bench.
fn agent_row(agent: &WorkAgent, benched: bool, cx: &mut Context<AppState>) -> AnyElement {
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
        .child(elided(
            eid("agents-row-name", id),
            agent.name.clone(),
            if benched {
                theme::text_muted()
            } else {
                theme::text()
            },
            13.,
        ))
        // The one mark on the row that is about this window rather than about the agent: it is not
        // on screen, and clicking the row is what puts it back.
        .children(benched.then(|| badge("bench", theme::text_faint())))
        .child(mono(agent.activity.label().to_lowercase(), colour).text_size(px(11.)))
        .on_click(cx.listener(move |this, _, _, cx| this.reveal_agent(id, cx)))
        .into_any_element()
}

/// The worst thing happening in a group: an error over a wait over movement over nothing. A folded
/// session is read at a glance, so its edge says what the user would want to be told first — the
/// same rule `WorkProjection::pulse` follows for a task's card, which is why a failing agent stays
/// visible with its group shut.
fn worst_of(members: &[&WorkAgent]) -> gpui::Rgba {
    let mut worst = Bucket::Ended;
    for bucket in members.iter().map(|agent| agent.activity.bucket()) {
        match bucket {
            Bucket::Error => {
                worst = Bucket::Error;
                break;
            }
            Bucket::Waiting => worst = Bucket::Waiting,
            Bucket::Running if worst != Bucket::Waiting => worst = Bucket::Running,
            _ => {}
        }
    }
    crate::ui::work::bucket_colour(worst)
}
