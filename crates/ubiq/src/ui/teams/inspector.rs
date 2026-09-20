//! The panel beside the graph: what the selection is, what it is doing, and what has been said to
//! it.
//!
//! Selection is what this panel is a function of, and both scales are drawn by the same frame.
//! With a **session** selected it reports the session — its branch, how its agents are spread
//! across the four states, and its tasks. With an **agent** selected it reports that one workspace
//! — its harness, its model, how much context it has left, its thread, and a composer.
//!
//! **The thread is [`crate::ui::conversation`]'s, not this panel's.** There is one interface for
//! talking to an agent and this is its third host, beside the chat panel and the agents columns:
//! the transcript, the footer and the composer all arrive with it, driven by a
//! [`ConversationView`] naming this surface's element ids and its composer slot. Nothing here
//! draws a transcript of its own, because two renderings of one conversation are two answers to
//! one question.

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::ids::SessionId;
use ubiq_proto::work::{AgentId, Bucket};

use crate::app::AppState;
use crate::state::agents::TEAMS_SLOT;
use crate::state::{TeamsInspectorTab, TeamsSelection};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::conversation::{self, ConversationView};
use crate::ui::indexed;
use crate::ui::kit::{
    Tab, UbiqIcon, icon_button, mono, panel, pill, progress_ring, state_chip, tab_strip,
};
use crate::ui::work::{activity_colour, bucket_colour, role_mark};

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let Some(graph) = app.teams(cx) else {
        return div().into_any_element();
    };

    let Some(selection) = graph.selection.clone() else {
        return panel()
            .child(header_bar("Nothing selected", "", theme::text_faint(), cx))
            .child(note(
                "Pick a session in the toolbar, or a card in the graph.",
            ))
            .into_any_element();
    };

    match selection {
        TeamsSelection::Session(id) => session_view(app, id, cx),
        // A delegate is read in its parent's panel: it has no workspace, no branch and no
        // conversation of its own, and which of its parent's turns are on screen is already the
        // shared view's own question. So both selections draw the same frame.
        TeamsSelection::Agent(id) | TeamsSelection::Subagent { agent: id, .. } => {
            agent_view(app, id, window, cx)
        }
    }
}

/// The header every shape of this panel wears: a mark, a name, what it is, and the way out.
fn header_bar(
    name: &str,
    kind: &str,
    colour: gpui::Rgba,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
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
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Title))
                .text_color(theme::text())
                .child(SharedString::from(name.to_string())),
        )
        .child(mono(kind.to_string(), theme::text_muted()))
        .child(div().flex_1().min_w(px(0.)))
        .child(icon_button(
            "teams-inspector-close",
            IconName::Close,
            false,
            cx.listener(|this, _, _, cx| this.toggle_teams_inspector(cx)),
        ))
}

/// A session: how its agents are spread across the four states, and the tasks it holds.
fn session_view(app: &AppState, id: SessionId, cx: &mut Context<AppState>) -> gpui::AnyElement {
    let Some(work) = app.teams_work(cx) else {
        return div().into_any_element();
    };
    let Some(session) = work.session(id) else {
        return div().into_any_element();
    };

    let counts: Vec<_> = Bucket::all()
        .into_iter()
        .map(|bucket| {
            let n = work
                .agents
                .iter()
                .filter(|a| a.session == id && a.activity.bucket() == bucket)
                .count();
            pill(bucket_colour(bucket))
                .h(px(24.))
                .px_2()
                .child(mono(format!("{n}"), theme::text()))
                .child(
                    mono(bucket.label(), theme::text_muted())
                        .text_size(theme::font(Family::Chrome, Role::Meta)),
                )
                .into_any_element()
        })
        .collect();

    panel()
        .child(header_bar(&session.name, "Session", theme::accent(), cx))
        .child(
            div()
                .px_3()
                .py_2()
                .flex()
                .flex_none()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    pill(theme::accent())
                        .h(px(24.))
                        .px_2()
                        .child(
                            Icon::new(UbiqIcon::GitBranch)
                                .with_size(Size::XSmall)
                                .text_color(theme::text_faint()),
                        )
                        .child(mono(session.branch.clone(), theme::text())),
                )
                .children(counts),
        )
        .child(super::tasks::list(app, cx))
        .into_any_element()
}

/// One agent — that is, one workspace: one harness, one terminal, one thread.
fn agent_view(
    app: &AppState,
    id: AgentId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let view = cx.entity();
    let (Some(work), Some(graph)) = (app.teams_work(cx), app.teams(cx)) else {
        return div().into_any_element();
    };
    let Some(agent) = work.agent(id) else {
        return div().into_any_element();
    };
    let colour = activity_colour(agent.activity);

    let owned = work
        .tasks
        .iter()
        .flat_map(|t| t.steps.iter())
        .filter(|s| s.owner == Some(id))
        .count();
    let open = work
        .tasks
        .iter()
        .flat_map(|t| t.steps.iter())
        .filter(|s| s.owner == Some(id) && !s.done())
        .count();

    let tabs = vec![
        Tab::new("chat"),
        Tab::new(format!("tasks {open}\u{b7}{owned}")),
    ];
    let active = match graph.tab {
        TeamsInspectorTab::Chat => 0,
        TeamsInspectorTab::Tasks => 1,
    };

    let body = match graph.tab {
        TeamsInspectorTab::Chat => chat(app, id, window, cx),
        TeamsInspectorTab::Tasks => super::tasks::list(app, cx),
    };

    let mut root = panel().child(header_bar(&agent.name, &agent.role, colour, cx));

    // The vitals strip belongs to the tasks tab alone. On the chat tab the shared view's own
    // footer reports the run, the context left and what the turn cost — from what the harness
    // said, rather than from the work record — and a second strip over it would be two answers
    // about one conversation.
    if graph.tab == TeamsInspectorTab::Tasks {
        root = root.child(
            div()
                .px_3()
                .py_2()
                .flex()
                .flex_none()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .border_b_1()
                .border_color(theme::border())
                .child(state_chip(agent.activity.label(), colour, 1.0))
                .child(
                    pill(theme::accent())
                        .h(px(24.))
                        .px_2()
                        .child(role_mark(&agent.role, theme::accent(), 16.))
                        .child(mono(agent.harness.clone(), theme::text())),
                )
                .child(
                    pill(theme::border())
                        .h(px(24.))
                        .px_2()
                        .child(mono(agent.model.clone(), theme::text())),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(progress_ring(agent.context_pct, 13.))
                .child(mono(format!("{}%", agent.context_pct), theme::text())),
        );
    }

    root.child(tab_strip(
        "teams-inspector-tabs",
        tabs,
        active,
        indexed(&view, |this, ix, _, cx| {
            this.select_teams_inspector_tab(ix, cx)
        }),
        None,
        None,
    ))
    .child(body)
    .into_any_element()
}

/// The conversation with one agent, drawn by the one component every surface that shows one uses.
///
/// Footer and composer come with it, and the composer is the window's own pooled field at
/// [`TEAMS_SLOT`] — so what is typed here is addressed at whichever card the canvas has selected,
/// and nothing this panel owns has to send it. The header is the lifecycle strip: this surface has
/// no toolbar of its own to hang that menu in, which is the difference between it and the chat
/// panel.
fn chat(app: &AppState, id: AgentId, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(conversation) = app.teams_conversation(id, cx) else {
        // A card is only drawn for an agent this window holds a conversation with, so this is the
        // frame between an agent ending and the canvas hearing about it.
        return note("This agent is no longer running.");
    };
    conversation::render(
        app,
        conversation,
        ConversationView {
            id: SharedString::from(format!("teams-{id}")),
            slot: TEAMS_SLOT,
            footer: true,
            composer: true,
            header: true,
        },
        window,
        cx,
    )
}

/// The panel's one register for saying there is nothing to draw: centred, faint, one line.
fn note(text: &str) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .px_3()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text_faint())
                .child(SharedString::from(text.to_string())),
        )
        .into_any_element()
}
