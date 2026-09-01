//! The panel beside the graph: what the selection is, what it is doing, and what has been said to
//! it.
//!
//! Selection is what this panel is a function of, and both scales are drawn by the same frame.
//! With a **session** selected it reports the session — its branch, how its agents are spread
//! across the four states, and its tasks. With an **agent** selected it reports that one workspace
//! — its harness, its model, how much context it has left, its thread, and a composer.
//!
//! The composer is real: what is typed lands in the agent's thread. Nothing answers, and the panel
//! says so rather than inventing a reply, because a fabricated answer is the one thing a screen
//! with no transport family behind it must not draw.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::input::Textarea;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::agents::{Bucket, Speaker};
use crate::state::{InspectorTab, Selection};
use crate::theme;
use crate::ui::agents::{activity_colour, bucket_colour, role_mark};
use crate::ui::indexed;
use crate::ui::kit::{
    Tab, ghost_button, icon_button, mono, panel, pill, progress_ring, section_label, state_chip,
    tab_strip,
};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let agents = &app.agents;

    let Some(selection) = agents.selection else {
        return panel()
            .child(header_bar("Nothing selected", "", theme::text_faint(), cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .px_3()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(theme::text_faint())
                            .child("Pick a session in the toolbar, or a card in the graph."),
                    ),
            )
            .into_any_element();
    };

    match selection {
        Selection::Session(id) => session_view(app, id, cx),
        Selection::Agent(id) => agent_view(app, id, cx),
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
        .child(
            div()
                .text_size(px(14.))
                .text_color(theme::text())
                .child(SharedString::from(name.to_string())),
        )
        .child(mono(kind.to_string(), theme::text_muted()).text_size(px(11.5)))
        .child(div().flex_1().min_w(px(0.)))
        .child(icon_button(
            "inspector-close",
            IconName::Close,
            false,
            cx.listener(|this, _, _, cx| this.toggle_inspector(cx)),
        ))
}

/// A session: how its agents are spread across the four states, and the tasks it holds.
fn session_view(
    app: &AppState,
    id: crate::state::agents::SessionId,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let agents = &app.agents;
    let Some(session) = agents.session(id) else {
        return div().into_any_element();
    };

    let counts: Vec<_> = Bucket::all()
        .into_iter()
        .map(|bucket| {
            let n = agents
                .agents
                .iter()
                .filter(|a| a.session == id && a.activity.bucket() == bucket)
                .count();
            pill(bucket_colour(bucket))
                .h(px(24.))
                .px_2()
                .child(mono(format!("{n}"), theme::text()).text_size(px(11.5)))
                .child(mono(bucket.label(), theme::text_muted()).text_size(px(11.)))
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
                            Icon::new(IconName::Network)
                                .with_size(Size::XSmall)
                                .text_color(theme::text_faint()),
                        )
                        .child(mono(session.branch.clone(), theme::text()).text_size(px(11.5))),
                )
                .children(counts),
        )
        .child(super::tasks::list(app, cx))
        .into_any_element()
}

/// One agent — that is, one workspace: one harness, one terminal, one thread.
fn agent_view(
    app: &AppState,
    id: crate::state::agents::AgentId,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let view = cx.entity();
    let agents = &app.agents;
    let Some(agent) = agents.agent(id) else {
        return div().into_any_element();
    };
    let colour = activity_colour(agent.activity);

    let owned = agents
        .tasks
        .iter()
        .flat_map(|t| t.steps.iter())
        .filter(|s| s.owner == Some(id))
        .count();
    let open = agents
        .tasks
        .iter()
        .flat_map(|t| t.steps.iter())
        .filter(|s| s.owner == Some(id) && !s.done)
        .count();

    let tabs = vec![
        Tab::new("chat"),
        Tab::new(format!("tasks {open}\u{b7}{owned}")),
    ];
    let active = match agents.tab {
        InspectorTab::Chat => 0,
        InspectorTab::Tasks => 1,
    };

    let body = match agents.tab {
        InspectorTab::Chat => thread(app, id, cx),
        InspectorTab::Tasks => super::tasks::list(app, cx),
    };

    let mut root = panel()
        .child(header_bar(&agent.name, &agent.role, colour, cx))
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
                .child(state_chip(agent.activity.label(), colour, 1.0))
                .child(
                    pill(theme::accent())
                        .h(px(24.))
                        .px_2()
                        .child(role_mark(&agent.role, theme::accent(), 16.))
                        .child(mono(agent.harness.clone(), theme::text()).text_size(px(11.5))),
                )
                .child(
                    pill(theme::border())
                        .h(px(24.))
                        .px_2()
                        .child(mono(agent.model.clone(), theme::text()).text_size(px(11.5))),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(progress_ring(agent.context_pct, 13.))
                .child(mono(format!("{}%", agent.context_pct), theme::text()).text_size(px(11.5)))
                .child(mono(agent.tokens_label(), theme::text_muted()).text_size(px(11.5))),
        )
        .child(tab_strip(
            "inspector-tabs",
            tabs,
            active,
            indexed(&view, |this, ix, _, cx| this.select_inspector_tab(ix, cx)),
            None,
            None,
        ))
        .child(body);

    if agents.tab == InspectorTab::Chat {
        root = root.child(composer(app, cx));
    }

    root.into_any_element()
}

/// What has been said to and by one agent, oldest first.
fn thread(
    app: &AppState,
    id: crate::state::agents::AgentId,
    _cx: &mut Context<AppState>,
) -> AnyElement {
    let Some(agent) = app.agents.agent(id) else {
        return div().into_any_element();
    };

    let turns: Vec<_> = agent
        .thread
        .iter()
        .map(|turn| match turn.from {
            // What the user said sits in the accent, indented from the left, the way the chat
            // panel draws the same thing.
            Speaker::You => div()
                .pl_6()
                .child(
                    div()
                        .p_2()
                        .bg(theme::accent_soft())
                        .border_l(px(theme::ACCENT_EDGE))
                        .border_color(theme::accent())
                        .text_size(px(13.))
                        .text_color(theme::text())
                        .child(SharedString::from(turn.text.clone())),
                )
                .into_any_element(),
            Speaker::Agent => div()
                .text_size(px(13.))
                .text_color(theme::text())
                .child(SharedString::from(turn.text.clone()))
                .into_any_element(),
        })
        .collect();

    div()
        .id("inspector-thread")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .px_3()
        .py_2()
        .gap_3()
        .overflow_y_scroll()
        .children(turns)
        .child(
            div()
                .pt_1()
                .text_size(px(11.5))
                .text_color(theme::text_faint())
                .child(
                    "Nothing is listening yet \u{2014} the graph has no transport family, so what \
                     you send is kept and not answered.",
                ),
        )
        .into_any_element()
}

/// The composer, in the shape the chat's is: what is typed, what it will reach, and the button.
fn composer(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let can_send = !app.agents.draft.trim().is_empty();
    let target = app
        .agents
        .selected_agent()
        .map(|a| a.name.clone())
        .unwrap_or_default();

    div()
        .flex()
        .flex_none()
        .flex_col()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::accent())
        .child(
            div()
                .id("inspector-composer")
                .px_3()
                .pt_2()
                .cursor_text()
                .child(
                    Textarea::new(&app.agent_input)
                        .appearance(false)
                        .bordered(false)
                        .w_full()
                        .text_size(px(13.5)),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    let input = this.agent_input.clone();
                    input.update(cx, |state, cx| state.focus(window, cx));
                })),
        )
        .child(
            div()
                .px_2()
                .pb_2()
                .pt_1()
                .flex()
                .items_center()
                .gap_2()
                .child(section_label("to"))
                .child(mono(target, theme::text_muted()).text_size(px(11.5)))
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    ghost_button(
                        "inspector-send",
                        Some(IconName::ArrowUp),
                        "Send",
                        cx.listener(|this, _, window, cx| this.send_to_agent(window, cx)),
                    )
                    .text_color(if can_send {
                        theme::accent()
                    } else {
                        theme::text_faint()
                    }),
                ),
        )
}
