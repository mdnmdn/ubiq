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
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, point, px,
};
use gpui_component::IconName;

use ubiq_proto::work::Bucket;

use crate::app::AppState;
use crate::state::teams::{Algo, TeamsSpan, ZOOM_STEP};
use crate::state::{MenuId, TeamsCreateStage, TeamsSelection};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    self, MultiPicker, Picker, check_box, ghost_button, icon_button, section_label, stepper,
};
use crate::ui::project_face::project_face;
use crate::ui::work::bucket_colour;
use crate::ui::{handler, indexed};

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

/// The strip over the graph: which sessions it is drawing, which states it is showing, and how far
/// in.
///
/// Both filters clear. A row with nothing ticked is not filtering — so a graph emptied by a filter
/// is always one click from being full again, and the control at the end of the row does both at
/// once.
///
/// **The two filters are the same shape**, and both are a `kit::MultiPicker`: sessions and states
/// are each a set, several of them on at once, so a chip saying what is ticked and a list that
/// stays down while a second is ticked answers both. Under the window span a session *is* a
/// project — this is the "projects" filter the toolbar reads as, and each row's dot is the owning
/// project's tint (`T-142`).
fn toolbar(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(graph)) = (app.teams_work(cx), app.teams(cx)) else {
        return div().into_any_element();
    };
    let view = cx.entity();
    let spanning = app.teams_span() == TeamsSpan::Window;

    // The sessions filter: a set, several on at once, exactly the buckets' shape below. Under the
    // window span each session is a project's, so a dot in that project's tint is what a project
    // chip elsewhere on this screen already carries; under the project span every session is the
    // one project's and a dot would tell nothing apart.
    let session_lit: Vec<usize> = work
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| graph.sessions.contains(&session.id))
        .map(|(ix, _)| ix)
        .collect();
    let session_dots: Vec<Rgba> = if spanning {
        work.sessions
            .iter()
            .map(|session| {
                work.agents
                    .iter()
                    .find(|a| a.session == session.id)
                    .and_then(|a| app.project_of_agent(a.id, cx))
                    .and_then(|project| project_face(project, cx))
                    .map(|face| face.tint)
                    .unwrap_or_else(theme::text_faint)
            })
            .collect()
    } else {
        Vec::new()
    };
    let sessions_picker = MultiPicker::new(
        "teams-sessions",
        if spanning {
            "all projects"
        } else {
            "all sessions"
        },
    )
    .items(work.sessions.iter().map(|session| session.name.clone()))
    .dots(session_dots)
    .selected(session_lit)
    .open(app.workbench.open_menu == Some(MenuId::TeamsSessions))
    .on_toggle(handler(&view, |this, _, cx| {
        this.open_menu(MenuId::TeamsSessions, cx)
    }))
    .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
    // The menu stays down: narrowing to two sessions is two clicks, and a list that shut after the
    // first would make the second a reopen. The list is read again here, exactly as it was drawn —
    // the rule every position-matched menu in this window follows.
    .on_pick(indexed(&view, |this, index, _, cx| {
        let id = this
            .teams_work(cx)
            .and_then(|work| work.sessions.get(index).map(|session| session.id));
        if let Some(id) = id {
            this.toggle_teams_session(id, cx);
        }
    }))
    // The row's second target: point the canvas and the tasks drawer at this one session without
    // touching the filter (`T-146`) — ticking narrows the set, this looks at one of them.
    .on_select(
        if spanning {
            "Look at just this project"
        } else {
            "Look at just this session"
        },
        indexed(&view, |this, index, _, cx| {
            let id = this
                .teams_work(cx)
                .and_then(|work| work.sessions.get(index).map(|session| session.id));
            if let Some(id) = id {
                this.select_in_teams(TeamsSelection::Session(id), cx);
            }
        }),
    );

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
        .child(section_label(if spanning { "Projects" } else { "Session" }))
        .child(sessions_picker)
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
        .child(create_split(app, cx))
        .children(teams_create_overlay(app, cx))
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

/// The one control on the row that *makes* something rather than narrowing what is drawn: a split
/// button (M15, §9) — a `+` and a chevron, the titlebar's own pattern (`ui::titlebar`'s
/// new-terminal `+` and its chevron) reused rather than built again.
///
/// It sits past the flexible gap, beside `Show everything` and before the view controls, because
/// an action is not a filter: the pills to the left of the gap all answer "what is on screen", and
/// this one answers "what is there to be on screen". Last in the action group rather than first,
/// so its distance from the zoom stepper does not move when `Show everything` comes and goes.
///
/// **The `+` does what the toolbar's `+ Add agent` always did** — New agent, asking which project
/// when the canvas spans more than one ([`AppState::open_teams_add_agent`]). **The chevron's menu
/// offers New agent and New mission**, each asking the same project question in its own second
/// stage ([`AppState::open_teams_create_menu`], [`AppState::pick_teams_create_menu`]) — *New
/// mission* raises the same dialog the board and the side docks' `+` already do
/// ([`AppState::open_new_mission`]).
fn create_split(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .child(
            icon_button(
                "teams-create",
                IconName::Plus,
                false,
                cx.listener(|this, event: &ClickEvent, window, cx| {
                    let at = (f32::from(event.position().x), f32::from(event.position().y));
                    this.open_teams_add_agent(at, window, cx);
                }),
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("New agent").build(window, cx)
            }),
        )
        .child(
            icon_button(
                "teams-create-menu",
                IconName::ChevronDown,
                app.workbench.open_menu == Some(MenuId::TeamsCreate),
                cx.listener(|this, event: &ClickEvent, _, cx| {
                    let at = (f32::from(event.position().x), f32::from(event.position().y));
                    this.open_teams_create_menu(at, cx);
                }),
            )
            .tooltip(|window, cx| {
                gpui_component::tooltip::Tooltip::new("New agent or mission").build(window, cx)
            }),
        )
        .into_any_element()
}

/// The split button's chevron menu, at whichever of its stages is down (§9's last bullet, M15):
/// the fixed *New agent* / *New mission* choice, or either row's own project question. A
/// `kit::context_menu` rather than a `kit::Picker` — the trigger is the chevron alone, not a
/// labelled control the panel can hang from, so the anchored click point
/// [`crate::app::TeamsCreateMenu::at`] carries is what the fixed-rows menu already uses
/// (`ui::agents::new_agent_menu`'s own first stage).
fn teams_create_overlay(app: &AppState, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let menu = app.workbench.teams_create_menu?;
    let view = cx.entity();
    let at = point(px(menu.at.0), px(menu.at.1));

    let items = match menu.stage {
        TeamsCreateStage::Kind => vec![
            kit::ContextItem::new("New agent"),
            kit::ContextItem::new("New mission"),
        ],
        // A row is the project's name in full, resolved through `project_face` so the rail's
        // badges, a card's chip and this list cannot disagree about which project is which.
        TeamsCreateStage::AgentProject | TeamsCreateStage::MissionProject => app
            .window_projects(cx)
            .into_iter()
            .map(|id| {
                kit::ContextItem::new(
                    project_face(id, cx)
                        .map(|face| face.name.to_string())
                        .unwrap_or_else(|| "\u{2026}".to_string()),
                )
            })
            .collect(),
    };

    Some(
        kit::context_menu(
            "teams-create-overlay",
            at,
            items,
            indexed(&view, |this, index, window, cx| {
                this.pick_teams_create_menu(index, window, cx);
            }),
            handler(&view, |this, _, cx| this.close_menu(cx)),
        )
        .into_any_element(),
    )
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
