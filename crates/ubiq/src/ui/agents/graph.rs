//! The orchestration graph: cards on a dotted ground, boxed by the task they serve, joined to
//! whoever spawned them — and picked up and put down with the pointer.
//!
//! **Nothing here knows where anything is.** A card's position comes from `state::layout`, which
//! holds it relative to the container it serves. That is what makes a container draggable: moving
//! its origin moves every card in it, and this file does not have to move any of them.
//!
//! **A card is carried, not previewed.** GPUI's own drag paints a ghost above the window while the
//! source sits still; here the ghost is empty and the real card follows the pointer, because the
//! thing being moved is a position on a canvas rather than a row being filed somewhere. The card
//! that moves *is* the answer, so there is nothing for a ghost to say.
//!
//! **A carried card leaves sand.** Each move lays a grain down where the pointer passed, and the
//! grains shrink, drift and fade over the next two-thirds of a second. It is the one piece of
//! motion on this screen and it earns its place: a card that jumps to a new position with no trace
//! reads as a redraw, and a card that leaves a track reads as something the user is holding. It is
//! skipped entirely when the system asks for reduced motion.
//!
//! Zoom scales positions, card size and type together, so the graph reads the same at every step
//! rather than turning into large cards on a small map.

use gpui::{
    App, AppContext as _, Context, DragMoveEvent, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, point,
    prelude::FluentBuilder, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::work::{TaskRecord, WorkAgent};

use crate::app::AppState;
use crate::state::agents::{CARD_HEIGHT, CARD_WIDTH, GROUP_LABEL, GROUP_PAD};
use crate::state::work;
use crate::state::{Held, Selection};
use crate::theme;
use crate::ui::agents::{activity_colour, role_mark};
use crate::ui::eid;
use crate::ui::kit::canvas::{self, Link};
use crate::ui::kit::{card, ghost_button, mono, state_chip};

/// What the pointer is carrying. It holds only what was picked up: where the thing is belongs to
/// the state, so a drag that is interrupted leaves it wherever the last move put it rather than in
/// a position only the drag knew about.
#[derive(Clone, Copy)]
pub struct Carried(pub Held);

/// GPUI wants a view for the drag preview. What is being dragged is already on the canvas and
/// already following the pointer, so this one draws nothing.
struct Empty;

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The margin past the outermost thing on the canvas, so something at the edge of the graph can
/// still be picked up and dropped without fighting the scroll.
const GRAPH_MARGIN: f32 = 60.0;

/// A task's outline: the record the box is drawn for, and the rectangle it came out as.
type TaskBox<'a> = (&'a TaskRecord, (f32, f32, f32, f32));

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(graph)) = (app.work(cx), app.graph(cx)) else {
        return div().into_any_element();
    };
    let zoom = graph.zoom;
    let view = cx.entity();

    let visible: Vec<&WorkAgent> = work.agents.iter().filter(|a| graph.visible(a)).collect();

    if visible.is_empty() {
        // Two different emptinesses, and saying which is the whole value of the message: a project
        // with no agents has nothing to offer, while a filter that hid them all has a way back.
        let filtered = graph.filtered() && !work.agents.is_empty();
        let mut said = div()
            .flex()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(theme::app_bg())
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(theme::text_faint())
                    .child(if filtered {
                        "No agent matches the filters."
                    } else {
                        "No agent is running in this project."
                    }),
            );
        if filtered {
            said = said.child(ghost_button(
                "agents-empty-clear",
                None,
                "Show everything",
                cx.listener(|this, _, _, cx| this.clear_graph_filters(cx)),
            ));
        }
        return said.into_any_element();
    }

    // The containers that are actually drawn, measured once: the boxes decide the canvas size, take
    // the drags that move a whole task, and light up under a carried card. The record travels with
    // its box rather than a place in the vector — the projection is replaced whole every time the
    // host answers, and an index into a vector somebody else owns is not worth keeping.
    let boxes: Vec<TaskBox<'_>> = work
        .tasks
        .iter()
        .filter_map(|task| Some((task, graph.bounds_of(work, task.id)?)))
        .collect();

    // The canvas is as big as what is on it, so scrolling reaches everything at any zoom.
    let mut extent = visible.iter().fold((0.0f32, 0.0f32), |(w, h), agent| {
        let at = graph.at(agent);
        (
            w.max(at.0 + CARD_WIDTH + GRAPH_MARGIN),
            h.max(at.1 + CARD_HEIGHT + GRAPH_MARGIN),
        )
    });
    for (_, (x, y, w, h)) in &boxes {
        extent = (
            extent.0.max(x + w + GRAPH_MARGIN),
            extent.1.max(y + h + GRAPH_MARGIN),
        );
    }

    let mut content = div()
        .relative()
        .w(px(extent.0 * zoom))
        .h(px(extent.1 * zoom))
        .child(canvas::dot_grid(
            theme::GRAPH_DOT_PITCH * zoom,
            point(0.0, 0.0),
        ));

    // The task containers, under everything: a dashed box round the cards serving one task, with
    // its shape and its title on the top edge. The box is computed from where its cards are, so a
    // card dragged out of one takes the outline with it.
    let over = graph.carry.and_then(|c| c.over);
    let held = graph.carry.map(|c| c.held);
    for (task, (x, y, w, h)) in boxes {
        let id = task.id;
        let lit = over == Some(id);
        let carried = held == Some(Held::Task(id));
        let view = view.clone();

        content = content
            .child(canvas::dashed_box(
                (x * zoom, y * zoom, w * zoom, h * zoom),
                if lit || carried {
                    theme::accent()
                } else {
                    theme::border()
                },
                lit || carried,
            ))
            // The empty ground inside a container is the handle for the container itself. The
            // cards are drawn after it and take their own drags, so grabbing a card moves one
            // agent and grabbing anywhere else in the box moves the whole task with everything in
            // it.
            .child(
                div()
                    .id(eid("agents-task", id))
                    .absolute()
                    .left(px(x * zoom))
                    .top(px(y * zoom))
                    .w(px(w * zoom))
                    .h(px(h * zoom))
                    .cursor_grab()
                    .on_drag(Carried(Held::Task(id)), move |_, grab, _, cx: &mut App| {
                        let grab = (f32::from(grab.x), f32::from(grab.y));
                        view.update(cx, |this, cx| {
                            this.start_graph_carry(Held::Task(id), grab, cx)
                        });
                        cx.new(|_| Empty)
                    }),
            )
            .child(
                div()
                    .absolute()
                    .left(px((x + GROUP_PAD * 0.5) * zoom))
                    .top(px((y + 4.0) * zoom))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .h(px(GROUP_LABEL * zoom))
                    .px(px(8.0 * zoom))
                    .bg(theme::pane_bg())
                    .child(mono(task.shape.label(), theme::text_faint()).text_size(px(9.5 * zoom)))
                    .child(mono("\u{b7}", theme::text_faint()).text_size(px(9.5 * zoom)))
                    .child(
                        mono(task.title.clone(), theme::text_muted()).text_size(px(11.0 * zoom)),
                    ),
            );
    }

    // The connectors, over the containers and under the cards: parent's bottom edge to child's top.
    let links: Vec<Link> = visible
        .iter()
        .filter_map(|agent| {
            let parent = work.agent(agent.parent?)?;
            if !graph.visible(parent) {
                return None;
            }
            let from = graph.at(parent);
            let to = graph.at(agent);
            Some(Link {
                from: point(
                    (from.0 + CARD_WIDTH / 2.0) * zoom,
                    (from.1 + CARD_HEIGHT) * zoom,
                ),
                to: point((to.0 + CARD_WIDTH / 2.0) * zoom, to.1 * zoom),
                colour: theme::fade(activity_colour(agent.activity), 0.5),
            })
        })
        .collect();
    content = content.child(canvas::links(links));

    for agent in &visible {
        content = content.child(agent_card(
            agent,
            graph.at(agent),
            graph.selection == Some(Selection::Agent(agent.id)),
            held == Some(Held::Agent(agent.id)),
            zoom,
            &view,
            cx,
        ));
    }

    // The sand goes over everything, including the card that is shedding it.
    if !graph.sand.is_empty() {
        let now = std::time::Instant::now();
        let grains = graph
            .sand
            .iter()
            .map(|grain| canvas::Grain {
                at: point(grain.at.0 + grain.spread.0, grain.at.1 + grain.spread.1),
                age: grain.age(now),
                size: grain.size,
            })
            .collect();
        content = content.child(canvas::sand(grains, theme::accent()));
        // The trail has to keep thinning after the pointer stops, so the window owes it frames
        // until the last grain is gone.
        window.request_animation_frame();
    }

    // The whole canvas is the drop target, so anything put down on it lands. Which task a card
    // landed in is worked out from where it is, not from what it was dropped on — a container is
    // an outline round some cards, and the outline is not what takes the drop.
    let content = content
        .on_drag_move(
            cx.listener(move |this, event: &DragMoveEvent<Carried>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                let Some(graph) = this.graph(cx) else {
                    return;
                };
                let Some(carry) = graph.carry else {
                    return;
                };
                let zoom = graph.zoom;
                let local = event.event.position - event.bounds.origin;
                let local = (f32::from(local.x), f32::from(local.y));
                // The grab point is where inside the card — or inside the container's box — the
                // pointer went down, so taking it off gives the top-left of whatever is held.
                let at = (
                    ((local.0 - carry.grab.0) / zoom).max(0.0),
                    ((local.1 - carry.grab.1) / zoom).max(0.0),
                );
                this.move_graph_carry(at, local, cx);
            }),
        )
        .on_drop(cx.listener(|this, _: &Carried, _, cx| this.end_graph_carry(cx)));

    div()
        .id("agents-graph")
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_scroll()
        .bg(theme::app_bg())
        .child(content)
        .into_any_element()
}

/// One card. Everything about it is either a fact the record carries or a colour from a token —
/// nothing on it is invented at draw time, and where it goes is handed in rather than read off it.
#[allow(clippy::too_many_arguments)]
fn agent_card(
    agent: &WorkAgent,
    at: (f32, f32),
    selected: bool,
    carried: bool,
    zoom: f32,
    view: &Entity<AppState>,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let id = agent.id;
    let colour = activity_colour(agent.activity);
    let view = view.clone();

    let body = card(eid("agents-card", id), colour, selected)
        .absolute()
        .left(px(at.0 * zoom))
        .top(px(at.1 * zoom))
        .w(px(CARD_WIDTH * zoom))
        .h(px(CARD_HEIGHT * zoom))
        .p(px(10.0 * zoom))
        .gap(px(6.0 * zoom))
        // The card is a fixed box on a canvas, so a long note is clipped by it rather than
        // spilling over the cards below.
        .overflow_hidden()
        // A card under the pointer is lifted off the ground: it goes opaque against the sand it is
        // dropping, and its edge takes the accent so it is the one thing in focus.
        .when(carried, |this| {
            this.bg(theme::surface_raised())
                .border_l(px(theme::ACCENT_EDGE * 2.0))
                .border_color(theme::accent())
        })
        .cursor_grab()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(7.0 * zoom))
                .child(role_mark(&agent.role, colour, 22.0 * zoom))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_size(px(13.0 * zoom))
                                .text_color(theme::text())
                                .child(SharedString::from(agent.name.clone())),
                        )
                        .child(
                            mono(agent.role.to_uppercase(), theme::text_faint())
                                .text_size(px(9.0 * zoom)),
                        ),
                )
                .child(state_chip(agent.activity.label(), colour, zoom)),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .text_size(px(11.5 * zoom))
                .text_color(theme::text_muted())
                .child(SharedString::from(agent.note.clone())),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.0 * zoom))
                .child(
                    Icon::new(IconName::Network)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .child(mono(agent.branch.clone(), theme::text_muted()).text_size(px(10.5 * zoom)))
                .child(
                    mono(work::tokens_label(agent), theme::text_faint()).text_size(px(10.5 * zoom)),
                )
                .child(div().flex_1().min_w(px(0.)))
                // The way into the conversation with this one agent: it selects the card and puts
                // the inspector on its thread, which is two clicks the card can save.
                .child(
                    div()
                        .id(eid("agents-card-chat", id))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4.0 * zoom))
                        .px(px(4.0 * zoom))
                        .cursor_pointer()
                        .hover(|this| this.bg(theme::hover()))
                        .child(
                            Icon::new(IconName::Inbox)
                                .with_size(Size::XSmall)
                                .text_color(theme::text_faint()),
                        )
                        .child(mono("chat", theme::text_muted()).text_size(px(10.5 * zoom)))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_agent_chat(id, cx))),
                ),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.select_in_graph(Selection::Agent(id), cx)))
        .on_drag(Carried(Held::Agent(id)), move |_, grab, _, cx: &mut App| {
            // The grab point is where inside the card the pointer went down. Keeping it is what
            // stops the card jumping under the cursor on the first move.
            let grab = (f32::from(grab.x), f32::from(grab.y));
            view.update(cx, |this, cx| {
                this.start_graph_carry(Held::Agent(id), grab, cx)
            });
            cx.new(|_| Empty)
        });

    body.into_any_element()
}
