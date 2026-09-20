//! The Teams screen: a graph of who is working on what, an inspector for whatever is selected in
//! it, and the tasks belonging to that selection.
//!
//! A clone of [`crate::ui::orchestration`] under the new rail mode — same shape, same rules,
//! independent state. The sessions, agents and tasks it draws are the host's, projected into
//! [`crate::state::work`]; what is selected in them, which states are showing and how far in it is
//! zoomed are this window's, in [`crate::state::teams`]. Everything on it is live: the
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
//! the whole graph on its own and hands a card its point. The toolbar's arrangement control chooses
//! which of [`crate::state::layout::Algo`] does that, and throws every hand-placed position away to
//! ask for it.
//!
//! Three files: the graph is [`graph`], the panel beside it is [`inspector`], the drawer under it
//! is [`tasks`]. This module is the frame; what a state reads as is [`crate::ui::work`]'s.

pub mod graph;
pub mod inspector;
pub mod status;
pub mod tasks;

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::IconName;

use ubiq_proto::work::Bucket;

use crate::app::AppState;
use crate::state::teams::{Algo, TeamsSpan, ZOOM_STEP};
use crate::state::{MenuId, TeamsSelection};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    Picker, check_box, ghost_button, icon_button, mono, section_label, stepper, toggle_pill,
};
use crate::ui::project_face::{ProjectFace, project_face};
use crate::ui::teams::status::project_chip;
use crate::ui::work::bucket_colour;
use crate::ui::{eid, handler, indexed};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The screen is a view of one project's work, and the shell keeps a window with no project off
    // it entirely — so there is nothing here to draw rather than an empty graph to explain.
    let Some(graph) = app.teams(cx) else {
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
                .w(px(theme::inspector_width()))
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
    let (Some(work), Some(graph)) = (app.teams_work(cx), app.teams(cx)) else {
        return div().into_any_element();
    };
    let view = cx.entity();
    // The lit pill is the one being *drawn*, not the one selected: `all` is a real state of the
    // row, and a session can be selected while every session is on screen.
    let showing = graph.session;
    let spanning = app.teams_span() == TeamsSpan::Window;

    let all = session_pill(
        "teams-session-all",
        "all",
        work.agents.len(),
        showing.is_none(),
        None,
        cx.listener(|this, _, _, cx| this.show_teams_session(None, cx)),
    );

    let sessions: Vec<_> = work
        .sessions
        .iter()
        .map(|session| {
            let id = session.id;
            let count = work.agents.iter().filter(|a| a.session == id).count();
            // Whose session this is, read off one of its cards: a session is minted inside a
            // project and every agent under it is that project's, so the first one answers for
            // the row. None under the project span, where the answer is the whole canvas.
            let project = spanning
                .then(|| {
                    work.agents
                        .iter()
                        .find(|a| a.session == id)
                        .and_then(|a| app.project_of_agent(a.id, cx))
                        .and_then(|project| project_face(project, cx))
                })
                .flatten();
            session_pill(
                eid("teams-session", id),
                session.name.clone(),
                count,
                showing == Some(id),
                project.map(|face| (eid("teams-session-project", id), face)),
                cx.listener(move |this, _, _, cx| {
                    // Narrowing to a session is also picking it: the inspector reporting on one the
                    // canvas is not drawing would be two answers to "which session".
                    this.show_teams_session(Some(id), cx);
                    this.select_in_teams(TeamsSelection::Session(id), cx);
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
                ("teams-filter", bucket as u32),
                bucket.label(),
                bucket_colour(bucket),
                graph.showing(bucket),
                cx.listener(move |this, _, _, cx| this.toggle_teams_bucket(bucket, cx)),
            )
            .into_any_element()
        })
        .collect();

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
        .child(section_label("Session"))
        .child(all)
        .children(sessions)
        .child(div().w(px(12.)).flex_none())
        .children(filters)
        .child(div().w(px(12.)).flex_none())
        .child(hide_done_check(graph.hide_done, cx))
        .child(div().flex_1().min_w(px(0.)))
        .children(graph.filtered().then(|| {
            ghost_button(
                "teams-show-all",
                None,
                "Show everything",
                cx.listener(|this, _, _, cx| this.clear_teams_filters(cx)),
            )
        }))
        .child(add_agent(app, cx))
        .child(div().w(px(12.)).flex_none())
        .child(stepper(
            "teams-zoom",
            format!("{}%", graph.zoom_pct()),
            cx.listener(|this, _, _, cx| this.zoom_teams(-ZOOM_STEP, cx)),
            cx.listener(|this, _, _, cx| this.zoom_teams(ZOOM_STEP, cx)),
        ))
        .child(
            Picker::new("teams-layout", graph.algo.label())
                .icon(IconName::LayoutDashboard)
                // A row carries the arrangement's hint as well as its name: "Packed" and "Tree"
                // say nothing about what a pick would do to the canvas, and the trigger goes on
                // reading as the bare name because that is what the control is reporting.
                .items(Algo::ALL.map(|a| format!("{} \u{2014} {}", a.label(), a.hint())))
                .selected(Algo::ALL.iter().position(|&a| a == graph.algo).unwrap_or(0))
                .open(app.workbench.open_menu == Some(MenuId::GraphLayout))
                .on_toggle(handler(&view, |this, _, cx| {
                    this.open_menu(MenuId::GraphLayout, cx)
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
                .on_pick(indexed(&view, |this, index, _, cx| {
                    this.set_teams_layout(index, cx)
                })),
        )
        .child(icon_button(
            "teams-fit",
            IconName::Maximize,
            false,
            cx.listener(|this, _, _, cx| this.reset_teams_zoom(cx)),
        ))
        .child(icon_button(
            "teams-inspector",
            IconName::PanelRight,
            graph.show_inspector,
            cx.listener(|this, _, _, cx| this.toggle_teams_inspector(cx)),
        ))
        .into_any_element()
}

/// The one control on the row that *makes* something rather than narrowing what is drawn.
///
/// It sits past the flexible gap, beside `Show everything` and before the view controls, because
/// an action is not a filter: the pills to the left of the gap all answer "what is on screen", and
/// this one answers "what is there to be on screen". Last in the action group rather than first,
/// so its distance from the zoom stepper does not move when `Show everything` comes and goes.
///
/// **Its shape is the question it asks.** A canvas spanning several projects gets a picker — a
/// start raised from a canvas about all of them has to name which one it is for — and anything
/// else gets a plain button, because a list of one row is a decision already made.
fn add_agent(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    if !app.teams_project_choice(cx) {
        return ghost_button(
            "teams-add-agent",
            Some(IconName::Plus),
            "Add agent",
            cx.listener(|this, _, window, cx| this.open_teams_add_agent(window, cx)),
        )
        .into_any_element();
    }
    let view = cx.entity();
    // A row is the project's name in full, resolved through `project_face` so the rail's badges,
    // a card's chip and this list cannot disagree about which project is which. The initials and
    // the tint are what a project wears where there is no room for its name; a menu row has the
    // room, and the name is the thing a choice is made on.
    let names: Vec<String> = app
        .window_projects(cx)
        .into_iter()
        .map(|id| {
            project_face(id, cx)
                .map(|face| face.name.to_string())
                // The registry is what `window_projects` filtered against, so this is unreachable
                // in practice — but a row dropped here would shift every index below it, and the
                // pick is matched by position.
                .unwrap_or_else(|| "\u{2026}".to_string())
        })
        .collect();
    Picker::new("teams-add-agent", "Add agent")
        .icon(IconName::Plus)
        .items(names)
        .open(app.workbench.open_menu == Some(MenuId::TeamsAddAgent))
        .on_toggle(handler(&view, |this, window, cx| {
            this.open_teams_add_agent(window, cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        .on_pick(indexed(&view, |this, index, window, cx| {
            this.pick_teams_add_agent(index, window, cx)
        }))
        .into_any_element()
}

/// The one control on the row about *delegates* rather than cards: whether a card goes on drawing
/// the boxes for the delegates that have finished.
///
/// A tick box rather than a pill, because it is not one more bucket — the pills narrow the canvas
/// to a set of states, and this drops one kind of thing out of every card that has any. `Show
/// everything` clears it with the rest.
fn hide_done_check(hidden: bool, cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(check_box(
            "teams-hide-done",
            hidden,
            cx.listener(|this, _, _, cx| this.toggle_teams_hide_done(cx)),
        ))
        .child(
            div()
                .id("teams-hide-done-label")
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(if hidden {
                    theme::text()
                } else {
                    theme::text_muted()
                })
                .cursor_pointer()
                .child("Hide done")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_teams_hide_done(cx))),
        )
}

/// One pill in the session row: a name, how many agents are under it, and whether it is the one
/// being drawn. `all` is one of these rather than a control of its own, because it answers the same
/// question the others do.
/// Under the window span it leads with the project's chip: two projects can name a session the
/// same thing, and a row of bare names would be two pills that look like one.
fn session_pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    count: usize,
    active: bool,
    project: Option<(ElementId, ProjectFace)>,
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
        .border_l(px(theme::accent_edge()))
        .border_color(if active {
            theme::accent()
        } else {
            theme::border()
        })
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .children(project.map(|(chip, face)| project_chip(chip, &face, 1.0).into_any_element()))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(if active {
                    theme::text()
                } else {
                    theme::text_muted()
                })
                .child(label.into()),
        )
        .child(
            mono(format!("{count}"), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        )
        .on_click(on_click)
        .into_any_element()
}
