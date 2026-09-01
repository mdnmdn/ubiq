//! The Agents screen: an orchestration graph, an inspector for whatever is selected in it, and
//! the tasks belonging to that selection.
//!
//! It is a placeholder in one specific sense — the graph has no transport family behind it, so its
//! sessions, agents and tasks come from [`crate::state::sample`] — and in no other. Everything on
//! it is live: the filters filter, the zoom zooms, a card is picked up and put down, and what is
//! selected is what the inspector and the tasks drawer are about.
//!
//! Selection has two scales, and they answer the same questions at each. A **session** is a named
//! piece of work; an **agent** is one workspace inside it — one running harness, one terminal.
//! Picking a session points the graph at it and lists its tasks; picking a card narrows both to
//! that agent.
//!
//! Three files: the graph is [`graph`], the panel beside it is [`inspector`], the drawer under it
//! is [`tasks`]. This module is the frame and the one place an activity is turned into a colour.

pub mod graph;
pub mod inspector;
pub mod tasks;

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::{Activity, Bucket, Selection};
use crate::theme;
use crate::ui::kit::{icon_button, mono, section_label, stepper, toggle_pill};

/// What an activity reads as. The four buckets share the four status tokens, and the three ways of
/// working share the one that means "moving", so the graph never asks the user to learn a colour
/// that means nothing anywhere else in the window.
pub fn activity_colour(activity: Activity) -> Rgba {
    bucket_colour(activity.bucket())
}

pub fn bucket_colour(bucket: Bucket) -> Rgba {
    match bucket {
        Bucket::Running => theme::success(),
        Bucket::Waiting => theme::info(),
        Bucket::Ended => theme::text_faint(),
        Bucket::Error => theme::danger(),
    }
}

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let agents = &app.agents;

    let mut body = div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .child(graph::render(app, window, cx).into_any_element());

    if agents.show_inspector {
        body = body.child(
            div()
                .w(px(theme::INSPECTOR_WIDTH))
                .flex()
                .flex_none()
                .border_l_1()
                .border_color(theme::border())
                .child(inspector::render(app, cx).into_any_element()),
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
}

/// The strip over the graph: which session it is on, which states it is showing, and how far in.
fn toolbar(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let agents = &app.agents;
    let active = agents.active_session();

    let sessions: Vec<_> = agents
        .sessions
        .iter()
        .map(|session| {
            let id = session.id;
            let is_active = Some(id) == active;
            let count = agents.agents.iter().filter(|a| a.session == id).count();
            div()
                .id(("agents-session", id))
                .h(px(26.))
                .px_2()
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .bg(if is_active {
                    theme::accent_soft()
                } else {
                    theme::pane_bg()
                })
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(if is_active {
                    theme::accent()
                } else {
                    theme::border()
                })
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(if is_active {
                            theme::text()
                        } else {
                            theme::text_muted()
                        })
                        .child(SharedString::from(session.name.clone())),
                )
                .child(mono(format!("{count}"), theme::text_faint()).text_size(px(11.)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_in_graph(Selection::Session(id), cx)
                }))
                .into_any_element()
        })
        .collect();

    let filters: Vec<_> = Bucket::all()
        .into_iter()
        .map(|bucket| {
            toggle_pill(
                ("agents-filter", bucket as u32),
                bucket.label(),
                bucket_colour(bucket),
                agents.showing(bucket),
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
        .children(sessions)
        .child(div().w(px(12.)).flex_none())
        .children(filters)
        .child(div().flex_1().min_w(px(0.)))
        .child(stepper(
            "agents-zoom",
            format!("{}%", agents.zoom_pct()),
            cx.listener(|this, _, _, cx| this.zoom_graph(-crate::state::agents::ZOOM_STEP, cx)),
            cx.listener(|this, _, _, cx| this.zoom_graph(crate::state::agents::ZOOM_STEP, cx)),
        ))
        .child(icon_button(
            "agents-fit",
            IconName::Maximize,
            false,
            cx.listener(|this, _, _, cx| this.reset_graph_zoom(cx)),
        ))
        .child(icon_button(
            "agents-inspector",
            IconName::PanelRight,
            app.agents.show_inspector,
            cx.listener(|this, _, _, cx| this.toggle_inspector(cx)),
        ))
}

/// The glyph on a card and at the top of the inspector. Ubiq ships no icon set, so a role borrows
/// the nearest thing in the component library's bundle.
pub fn role_icon(role: &str) -> IconName {
    match role.to_lowercase().as_str() {
        "project manager" | "activity coordinator" => IconName::Asterisk,
        "analyst" | "investigator" => IconName::Search,
        "verifier" => IconName::CircleCheck,
        "documentation" => IconName::BookOpen,
        _ => IconName::SquareTerminal,
    }
}

/// A role's glyph, at the size a card and the inspector both draw it.
pub fn role_mark(role: &str, colour: Rgba, side: f32) -> impl IntoElement {
    div()
        .size(px(side))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(theme::surface_raised())
        .child(
            Icon::new(role_icon(role))
                .with_size(Size::XSmall)
                .text_color(colour),
        )
}
