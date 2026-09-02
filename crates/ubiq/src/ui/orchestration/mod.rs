//! The Orchestration screen: an orchestration graph, an inspector for whatever is selected in it,
//! and the tasks belonging to that selection.
//!
//! The sessions, agents and tasks it draws are the host's, projected into
//! [`crate::state::work`]; what is selected in them, which states are showing and how far in it is
//! zoomed are this window's, in [`crate::state::orchestration`]. Everything on it is live: the
//! filters filter, the zoom zooms, a card is picked up and put down, and what is selected is what
//! the inspector and the tasks drawer are about.
//!
//! This is the screen about *how the work is arranged* — who spawned whom, which task a card
//! serves, what a hand-off looks like. The screen about *talking to the agents* is
//! [`crate::ui::agents`], and the two never share a view: a graph is a map and a column is a
//! conversation.
//!
//! Selection has two scales, and they answer the same questions at each. A **session** is a named
//! piece of work; an **agent** is one workspace inside it — one running harness, one terminal.
//! Picking a session points the graph at it and lists its tasks; picking a card narrows both to
//! that agent.
//!
//! Nothing on the graph carries its own position. What an agent or a task *is* lives in
//! [`crate::state::work`]; where it is drawn lives in [`crate::state::layout`], which arranges
//! the whole graph on its own and hands a card its point. The toolbar's tidy control throws every
//! hand-placed position away and asks for that arrangement again.
//!
//! Three files: the graph is [`graph`], the panel beside it is [`inspector`], the drawer under it
//! is [`tasks`]. This module is the frame; what a state reads as is [`crate::ui::work`]'s.

pub mod graph;
pub mod inspector;
pub mod tasks;

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::IconName;

use ubiq_proto::work::Bucket;

use crate::app::AppState;
use crate::state::Selection;
use crate::state::orchestration::ZOOM_STEP;
use crate::theme;
use crate::ui::eid;
use crate::ui::kit::{ghost_button, icon_button, mono, section_label, stepper, toggle_pill};
use crate::ui::work::bucket_colour;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The screen is a view of one project's work, and the shell keeps a window with no project off
    // it entirely — so there is nothing here to draw rather than an empty graph to explain.
    let Some(graph) = app.graph(cx) else {
        return div().into_any_element();
    };

    let mut body = div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .child(graph::render(app, window, cx).into_any_element());

    if graph.show_inspector {
        body = body.child(
            div()
                .w(px(theme::INSPECTOR_WIDTH))
                .flex()
                .flex_none()
                .border_l_1()
                .border_color(theme::border())
                .child(inspector::render(app, window, cx).into_any_element()),
        );
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(toolbar(app, cx))
        .child(body)
        .child(tasks::render(app, cx))
        .into_any_element()
}

/// The strip over the graph: which session it is drawing, which states it is showing, and how far
/// in.
///
/// Both filters clear. The session row leads with an `all` that draws every session, and a bucket
/// row with nothing lit is not filtering — so a graph emptied by a filter is always one click from
/// being full again, and the control at the end of the row does both at once.
fn toolbar(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(graph)) = (app.work(cx), app.graph(cx)) else {
        return div().into_any_element();
    };
    // The lit pill is the one being *drawn*, not the one selected: `all` is a real state of the
    // row, and a session can be selected while every session is on screen.
    let showing = graph.session;

    let all = session_pill(
        "orch-session-all",
        "all",
        work.agents.len(),
        showing.is_none(),
        cx.listener(|this, _, _, cx| this.show_graph_session(None, cx)),
    );

    let sessions: Vec<_> = work
        .sessions
        .iter()
        .map(|session| {
            let id = session.id;
            let count = work.agents.iter().filter(|a| a.session == id).count();
            session_pill(
                eid("orch-session", id),
                session.name.clone(),
                count,
                showing == Some(id),
                cx.listener(move |this, _, _, cx| {
                    // Narrowing to a session is also picking it: the inspector reporting on one the
                    // canvas is not drawing would be two answers to "which session".
                    this.show_graph_session(Some(id), cx);
                    this.select_in_graph(Selection::Session(id), cx);
                }),
            )
        })
        .collect();

    let filters: Vec<_> = Bucket::all()
        .into_iter()
        .map(|bucket| {
            toggle_pill(
                // Keyed off the enum's discriminant rather than an id: there is one pill per
                // bucket and no record behind it, so there is nothing here for a ULID to name.
                ("orch-filter", bucket as u32),
                bucket.label(),
                bucket_colour(bucket),
                graph.showing(bucket),
                cx.listener(move |this, _, _, cx| this.toggle_agent_bucket(bucket, cx)),
            )
            .into_any_element()
        })
        .collect();

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
        .child(section_label("Session"))
        .child(all)
        .children(sessions)
        .child(div().w(px(12.)).flex_none())
        .children(filters)
        .child(div().flex_1().min_w(px(0.)))
        .children(graph.filtered().then(|| {
            ghost_button(
                "orch-show-all",
                None,
                "Show everything",
                cx.listener(|this, _, _, cx| this.clear_graph_filters(cx)),
            )
        }))
        .child(stepper(
            "orch-zoom",
            format!("{}%", graph.zoom_pct()),
            cx.listener(|this, _, _, cx| this.zoom_graph(-ZOOM_STEP, cx)),
            cx.listener(|this, _, _, cx| this.zoom_graph(ZOOM_STEP, cx)),
        ))
        .child(icon_button(
            "orch-tidy",
            IconName::LayoutDashboard,
            false,
            cx.listener(|this, _, _, cx| this.tidy_graph(cx)),
        ))
        .child(icon_button(
            "orch-fit",
            IconName::Maximize,
            false,
            cx.listener(|this, _, _, cx| this.reset_graph_zoom(cx)),
        ))
        .child(icon_button(
            "orch-inspector",
            IconName::PanelRight,
            graph.show_inspector,
            cx.listener(|this, _, _, cx| this.toggle_inspector(cx)),
        ))
        .into_any_element()
}

/// One pill in the session row: a name, how many agents are under it, and whether it is the one
/// being drawn. `all` is one of these rather than a control of its own, because it answers the same
/// question the others do.
fn session_pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    count: usize,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .h(px(26.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .bg(if active {
            theme::accent_soft()
        } else {
            theme::pane_bg()
        })
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(if active {
            theme::accent()
        } else {
            theme::border()
        })
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            div()
                .text_size(px(12.))
                .text_color(if active {
                    theme::text()
                } else {
                    theme::text_muted()
                })
                .child(label.into()),
        )
        .child(mono(format!("{count}"), theme::text_faint()).text_size(px(11.)))
        .on_click(on_click)
        .into_any_element()
}
