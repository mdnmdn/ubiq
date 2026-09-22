//! The Teams screen: a graph of who is working on what, and the tasks belonging to whatever is
//! selected in it.
//!
//! A clone of [`crate::ui::orchestration`] under the new rail mode — same shape, same rules,
//! independent state. The sessions, agents and tasks it draws are the host's, projected into
//! [`crate::state::work`]; what is selected in them, which states are showing and how far in it is
//! zoomed are this window's, in [`crate::state::teams`]. Everything on it is live: the
//! filters filter, the zoom zooms, a card is picked up and put down, and what is selected is what
//! the tasks drawer is about — and what the right dock opens a conversation on.
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
//! Two files: the graph is [`graph`], the drawer under it is [`tasks`]. This module is the frame;
//! what a state reads as is [`crate::ui::work`]'s.
//!
//! **There is no inline detail area.** A card used to have a third column beside the graph
//! reporting on whatever was selected; that panel is gone, and selecting a block now opens that
//! agent's conversation in the right dock instead ([`crate::app::AppState::select_in_teams`]) —
//! the same surface the chat tabs and the agents screen already share, so a card is a map pin
//! rather than a page of its own.

pub mod graph;
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
    MultiPicker, Picker, check_box, ghost_button, icon_button, mono, section_label, stepper,
};
use crate::ui::project_face::{ProjectFace, project_face};
use crate::ui::teams::status::project_chip;
use crate::ui::work::bucket_colour;
use crate::ui::{eid, handler, indexed};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The screen is a view of one project's work, and the shell keeps a window with no project off
    // it entirely — so there is nothing here to draw rather than an empty graph to explain.
    if app.teams(cx).is_none() {
        return div().into_any_element();
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(toolbar(app, cx))
        .child(
            div()
                .flex()
                .flex_1()
                // Both axes, not just the vertical one: the graph's own scroller already grows
                // wider than the viewport once a wide arrangement is on it, and a flex item with
                // no `min_w` refuses to shrink below its content's width — the standard
                // refuses-to-shrink bug, on the axis that usually escapes notice because most
                // panes only fill downward. Left off, the wide content pushes this row out
                // instead of scrolling inside it, and a trackpad's horizontal gesture has nothing
                // to act on.
                .min_w(px(0.))
                .min_h(px(0.))
                .child(graph::render(app, window, cx).into_any_element()),
        )
        .child(tasks::render(app, cx))
        .into_any_element()
}

/// The strip over the graph: which session it is drawing, which states it is showing, and how far
/// in.
///
/// Both filters clear. The session row leads with an `all` that draws every session, and a states
/// control with nothing ticked is not filtering — so a graph emptied by a filter is always one
/// click from being full again, and the control at the end of the row does both at once.
///
/// **The two filters have different shapes because they are different questions.** A session is a
/// choice of one, and a row of pills is the report and the control at once; the states are a set,
/// several of them on at once, and that is a `kit::MultiPicker` — one chip saying what is ticked,
/// a list that stays down while the user ticks a second.
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

    // The states filter: four values, any number of them on at once, which is what makes it the
    // one filter on this row that is a set rather than a choice. It reads `buckets` rather than
    // `showing`, because "every bucket lit" and "none lit" draw the same canvas but are not the
    // same answer to give a control — ticking a row off the full set is how the user narrows it.
    let buckets = Bucket::all();
    let lit: Vec<usize> = buckets
        .iter()
        .enumerate()
        .filter(|(_, bucket)| graph.buckets.contains(bucket))
        .map(|(ix, _)| ix)
        .collect();
    let states = MultiPicker::new("teams-buckets", "all states")
        .items(buckets.map(|bucket| bucket.label()))
        // The colour the pills carried comes with them: a state is read by colour before it is
        // read by name, on this screen and on every other.
        .dots(buckets.map(bucket_colour))
        .selected(lit)
        .open(app.workbench.open_menu == Some(MenuId::TeamsBuckets))
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::TeamsBuckets, cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        // The menu stays down: narrowing to two states is two clicks, and a list that shut after
        // the first would make the second a reopen.
        .on_pick(indexed(&view, |this, index, _, cx| {
            if let Some(&bucket) = Bucket::all().get(index) {
                this.toggle_teams_bucket(bucket, cx);
            }
        }));

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
        .child(section_label("States"))
        .child(states)
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
        .child(
            icon_button(
                "teams-rearrange",
                IconName::RotateCw,
                false,
                cx.listener(|this, _, _, cx| this.tidy_teams(cx)),
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("Rearrange \u{2014} lay the graph out again")
                    .build(window, cx)
            }),
        )
        .child(div().w(px(12.)).flex_none())
        .child(
            // The right dock's own `+`: the chat panel a card's selection opens
            // (`AppState::open_teams_agent_panel`) has always offered *New agent* or *attach
            // existing* through its own header the moment it is on screen — the IDE and KB modes
            // get one for free because a persistent agent's tab joins that dock at project entry.
            // Teams has nothing there until a card is clicked, so this reveals the same panel with
            // no agent pointed at yet, through `AppState::open_teams_agent_panel`'s own
            // reuse-or-mint step.
            icon_button(
                "teams-new-agent-panel",
                IconName::Plus,
                false,
                cx.listener(|this, _, _, cx| this.open_teams_new_agent_panel(cx)),
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new(
                    "Open the agent panel \u{2014} new agent or attach existing",
                )
                .build(window, cx)
            }),
        )
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
