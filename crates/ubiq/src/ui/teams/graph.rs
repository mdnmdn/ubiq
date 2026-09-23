//! The Teams graph: cards on a dotted ground, boxed by the task they serve, joined to
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
//!
//! **What a block, a fence and the board they sit on look like is `ui::kit::blocks`'s**, not this
//! file's. This one measures the graph and says what each piece is; the kit stacks the layers in
//! the order a graph reads in and scales them by the one zoom. The sink's `teamsim` page draws the
//! same arrangement through the same module, which is the only way the two cannot drift apart.

use gpui::{
    App, AppContext as _, Context, DragMoveEvent, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div,
    point, px,
};
use gpui_component::{Icon, Sizable as _};

use ubiq_proto::ids::TaskId;
use ubiq_proto::work::{AgentId, TaskRecord, WorkAgent};

use crate::app::AppState;
use crate::state::conversation::{Conversation, SubagentTab, short_model_label};
use crate::state::status::Status;
use crate::state::teams::{
    CARD_WIDTH, GROUP_LABEL, GROUP_PAD, TEAMS_CARD_HEIGHT, TeamsSpan, agent_status,
    delegate_status, fence,
};
use crate::state::work;
use crate::state::{TeamsHeld, TeamsSelection};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::blocks::{self, Board, Fence, Look, Word};
use crate::ui::kit::canvas::{self, Link};
use crate::ui::kit::{elided_with, ghost_button, harness_icon, mono, progress_ring_in};
use crate::ui::mark;
use crate::ui::project_face::{ProjectFace, project_face};
use crate::ui::teams::status::{card_colour, project_chip, status_chip, status_mark};
use crate::ui::work::{activity_colour, lifecycle_colour};
use crate::ui::{eid, eid2};

/// What the pointer is carrying. It holds only what was picked up: where the thing is belongs to
/// the state, so a drag that is interrupted leaves it wherever the last move put it rather than in
/// a position only the drag knew about.
#[derive(Clone)]
pub struct Carried(pub TeamsHeld);

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

/// One card's delegates, measured: whose they are, where that card is, what the transcript says
/// about each of them, and where each is drawn.
type Ring = (AgentId, (f32, f32), Vec<SubagentTab>, Vec<(f32, f32)>);

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(graph)) = (app.teams_work(cx), app.teams(cx)) else {
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
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(theme::text_faint())
                    .child(if filtered {
                        "No agent matches the filters."
                    } else {
                        "No agent is running in this project."
                    }),
            );
        if filtered {
            said = said.child(ghost_button(
                "teams-empty-clear",
                None,
                "Show everything",
                cx.listener(|this, _, _, cx| this.clear_teams_filters(cx)),
            ));
        }
        return mark::backdrop(app, said.into_any_element(), cx);
    }

    // The containers that are actually drawn, measured once: the boxes decide the canvas size, take
    // the drags that move a whole task, and light up under a carried card. The record travels with
    // its box rather than a place in the vector — the projection is replaced whole every time the
    // host answers, and an index into a vector somebody else owns is not worth keeping.
    let boxes: Vec<TaskBox<'_>> = work
        .tasks
        .iter()
        .filter_map(|task| Some((task, graph.bounds_of(&work, task.id)?)))
        .collect();

    // Where every delegate is drawn, read once: the fence round a card, the connectors into it, the
    // cards themselves and the canvas extent are four readings of the same positions, and a second
    // reading of them is a chance to disagree.
    let rings: Vec<Ring> = visible
        .iter()
        .filter_map(|agent| {
            // Through the filter, not off the transcript: the finished delegates the toolbar's
            // tick box hides are hidden here and in `settle_teams`'s ring counts, which are two
            // readings of one answer.
            let delegates = graph.drawn_delegates(
                app.teams_conversation(agent.id, cx)
                    .map(|conversation| conversation.subagents())
                    .unwrap_or_default(),
            );
            if delegates.is_empty() {
                return None;
            }
            let at = graph.at(agent);
            let spots = delegates
                .iter()
                .enumerate()
                .map(|(ix, tab)| graph.sub_at(agent.id, at, &tab.id, ix))
                .collect();
            Some((agent.id, at, delegates, spots))
        })
        .collect();

    // The board is as big as what is on it, so scrolling reaches everything at any zoom — cards,
    // containers, and every delegate's card and fence. Nothing here measures that: the extent grows
    // as each piece is added, which is one fewer reading of the same geometry to disagree with.
    let mut board = Board::new(zoom, GRAPH_MARGIN);

    // What one delegate measures under the chosen arrangement's ring. Read once, so the card, its
    // connector and the fence round it can never disagree about the shape being drawn.
    let sub = graph.sub_box();

    // Whose work this is, under the window span only: the canvas is drawing several projects at
    // once, and a card that does not say which is a card a reader cannot place. Under the project
    // span the answer is the whole screen, and everything here is what it always was — both lists
    // below come back empty and nothing on the canvas changes.
    let span = app.teams_span();
    let spanning = span == TeamsSpan::Window;
    // Which container wears which project's colour, and which loose cards get a fence of their
    // own. Both answered by the state, so the rule — everything fenced once, in the colour of the
    // project it came from — is one reading rather than two the canvas could disagree about.
    let tinted: Vec<(TaskId, Option<Rgba>)> = graph
        .fenced_tasks(&work, span)
        .into_iter()
        .map(|(task, owner)| (task, card_tint(app, owner, cx)))
        .collect();
    let loose = graph.fenced_alone(&work, span);
    // Which task ids actually get a container fence, read once so the ring loop below can tell
    // whether a card's delegates are already inside one — a card no container or loose fence
    // reaches is the only case left needing an outline of its own (`T-105`).
    let boxed_tasks: std::collections::HashSet<TaskId> =
        boxes.iter().map(|(task, _)| task.id).collect();

    // The task containers, under everything: a dashed box round the cards serving one task, with
    // its shape and its title on the top edge. The box is computed from where its cards are, so a
    // card dragged out of one takes the outline with it.
    let over = graph.carry.as_ref().and_then(|c| c.over);
    let held = graph.carry.as_ref().map(|c| c.held.clone());
    for (task, (x, y, w, h)) in boxes {
        let id = task.id;
        let lit = over == Some(id);
        let carried = held == Some(TeamsHeld::Task(id));
        let view = view.clone();

        // The shape and the separator after it go together: a task nobody has shaped draws
        // neither, rather than a title behind a dot with nothing in front of it.
        let mut label = Vec::new();
        if let Some(shape) = task.shape {
            label.push(Word::new(shape.label(), theme::text_faint(), Role::Micro));
            label.push(Word::new("\u{b7}", theme::text_faint(), Role::Micro));
        }
        label.push(Word::new(
            task.title.clone(),
            theme::text_muted(),
            Role::Meta,
        ));

        let tint = tinted
            .iter()
            .find(|(task, _)| *task == id)
            .and_then(|(_, tint)| *tint);

        board.fence(
            Fence::new(
                (x, y, w, h),
                // The drop state still wins the outline: `accent` plus the heavier dashes is what
                // says "let go here", and a container that kept its project's colour while lit
                // would trade the answer to "where does this land" for one already on every card
                // inside it.
                if lit || carried {
                    theme::accent()
                } else {
                    tint.unwrap_or_else(theme::border)
                },
                lit || carried,
            )
            .labelled(label, (GROUP_PAD * 0.5, 4.0), GROUP_LABEL)
            // The empty ground inside a container is the handle for the container itself. The
            // cards are drawn after it and take their own drags, so grabbing a card moves one
            // agent and grabbing anywhere else in the box moves the whole task with everything in
            // it.
            .handle(
                blocks::handle(eid("teams-task", id), (x, y, w, h), zoom)
                    .on_drag(
                        Carried(TeamsHeld::Task(id)),
                        move |_, grab, _, cx: &mut App| {
                            let grab = (f32::from(grab.x), f32::from(grab.y));
                            view.update(cx, |this, cx| {
                                this.start_teams_carry(TeamsHeld::Task(id), grab, cx)
                            });
                            cx.new(|_| Empty)
                        },
                    )
                    .into_any_element(),
            ),
        );
    }

    // The loose cards' own fences, under the window span only: a card no container encloses still
    // belongs to a project, and a canvas where only the grouped work is coloured leaves the rest
    // of it unplaceable. One fence per card, never a second one round a card already inside a
    // container — two dashed outlines in the same colour say nothing the inner one did not.
    //
    // Colourless is not a case: a project the registry cannot face is one no fence can speak for,
    // so it draws none rather than a grey box round one card and a coloured one round the next.
    for agent in &visible {
        if !loose.contains(&agent.id) {
            continue;
        }
        let Some(tint) = card_tint(app, agent.id, cx) else {
            continue;
        };
        board.fence(Fence::new(
            graph.solo_bounds(agent.id, graph.at(agent)),
            tint,
            false,
        ));
    }

    // The connectors, over the containers and under the cards: parent's bottom edge to child's top.
    // A delegate is joined to the card that spawned it the same way one card is joined to another —
    // it is drawn inside its parent's fence, and the line is what says whose it is once it has been
    // dragged somewhere else in that fence.
    let mut links: Vec<Link> = visible
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
                    (from.1 + TEAMS_CARD_HEIGHT) * zoom,
                ),
                to: point((to.0 + CARD_WIDTH / 2.0) * zoom, to.1 * zoom),
                colour: theme::fade(activity_colour(agent.activity), 0.5),
            })
        })
        .collect();
    for (_, at, delegates, spots) in &rings {
        for (tab, spot) in delegates.iter().zip(spots) {
            links.push(Link {
                from: point(
                    (at.0 + CARD_WIDTH / 2.0) * zoom,
                    (at.1 + TEAMS_CARD_HEIGHT) * zoom,
                ),
                to: point((spot.0 + sub.0 / 2.0) * zoom, spot.1 * zoom),
                // The link is about whether the delegate is still going, which is the lifecycle
                // half — a completed delegate's green result belongs on its card, not on the
                // thread joining it to whoever spawned it.
                colour: theme::fade(lifecycle_colour(delegate_status(tab).lifecycle), 0.45),
            });
        }
    }
    for link in links {
        board.link(link);
    }

    // The delegate fences, under the cards and over the containers. **One fence, not two**: a
    // ringed card already sits inside its task's container or its own loose fence — both are
    // measured off `card_bounds`, which already wraps the ring — so drawing a second outline here
    // would be the same box again, a shade lighter (`T-105`). This one is only for a ringed card
    // neither reaches: no task on the canvas, and not spanning the window.
    let enclosed = |id: AgentId| {
        work.agent(id)
            .and_then(|agent| agent.task)
            .is_some_and(|task| boxed_tasks.contains(&task))
            || (spanning && loose.contains(&id))
    };
    for (id, at, delegates, spots) in &rings {
        if !enclosed(*id) {
            if let Some(rect) = fence(*at, spots) {
                board.inner_fence(Fence::new(
                    rect,
                    theme::fade(theme::accent_muted(), 0.8),
                    false,
                ));
            }
        }
        // The parent's transcript, read again at the grain a delegate's own spend is banked at —
        // `Conversation::subagent_tokens` keyed by type, the only grain the wire carries. The
        // `rings` pass above read the same conversation to name the delegates at all; this is a
        // second reading of it for a second fact, on the same rule every other loop in this
        // function follows.
        let conversation = app.teams_conversation(*id, cx);
        for (ix, (tab, spot)) in delegates.iter().zip(spots).enumerate() {
            board.block(
                (spot.0, spot.1, sub.0, sub.1),
                subagent_card(
                    *id,
                    work.agent(*id).map(|a| a.harness.as_str()).unwrap_or(""),
                    tab,
                    conversation,
                    ix,
                    *spot,
                    sub,
                    graph.subagent_in_focus() == Some(tab.id.as_str()),
                    held.as_ref()
                        == Some(&TeamsHeld::Subagent {
                            agent: *id,
                            subagent: tab.id.clone(),
                        }),
                    zoom,
                    &view,
                    cx,
                ),
            );
        }
    }

    for agent in &visible {
        let conversation = app.teams_conversation(agent.id, cx);
        let project = spanning
            .then(|| {
                app.project_of_agent(agent.id, cx)
                    .and_then(|project| project_face(project, cx))
            })
            .flatten();
        // How full the window is, on the same rule the status follows: the live conversation
        // first, because it is the stream itself, and the host's periodic reading of it after. No
        // ring at all where neither states one — no harness reports a window it was not given, and
        // a ratio with an invented denominator is worse than none (`G96`).
        let context = conversation
            .and_then(|conversation| conversation.context_pct())
            .or(Some(agent.context_pct).filter(|pct| *pct > 0));
        // What has been spent and how much of it came back from cache — the footer's own
        // `total_tokens`/`cached_tokens` read, at the card's grain. `None` total where the
        // conversation has not reported a spend yet, so the card draws no ring rather than one
        // over a made-up total.
        let spend = conversation
            .and_then(|conversation| conversation.total_tokens())
            .map(|total| {
                (
                    total,
                    conversation.and_then(|c| c.cached_tokens()).unwrap_or(0),
                )
            });
        let at = graph.at(agent);
        board.block(
            (at.0, at.1, CARD_WIDTH, TEAMS_CARD_HEIGHT),
            agent_card(
                agent,
                // The live conversation is the better witness of what a card is doing than the
                // host's periodic reading of it, and this mode draws no card without one.
                agent_status(agent, conversation),
                context,
                spend,
                project,
                at,
                graph.agent_in_focus() == Some(agent.id),
                held == Some(TeamsHeld::Agent(agent.id)),
                zoom,
                &view,
                cx,
            ),
        );
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
        board.over(canvas::sand(grains, theme::accent()));
        // The trail has to keep thinning after the pointer stops, so the window owes it frames
        // until the last grain is gone.
        window.request_animation_frame();
    }

    // The whole canvas is the drop target, so anything put down on it lands. Which task a card
    // landed in is worked out from where it is, not from what it was dropped on — a container is
    // an outline round some cards, and the outline is not what takes the drop.
    let content = board
        .content(window.viewport_size())
        .on_drag_move(
            cx.listener(move |this, event: &DragMoveEvent<Carried>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                let Some(graph) = this.teams(cx) else {
                    return;
                };
                let Some(carry) = graph.carry.as_ref() else {
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
                this.move_teams_carry(at, local, cx);
            }),
        )
        .on_drop(cx.listener(|this, _: &Carried, _, cx| this.end_teams_carry(cx)));

    blocks::scroller("teams-graph", &app.teams_scroll)
        .child(content)
        .into_any_element()
}

/// What a delegate's third row says it is doing: the permission wait where it has one, its own
/// last activity otherwise, or nothing where the transcript holds neither — drawn as no row rather
/// than an invented one.
fn delegate_activity_label(tab: &SubagentTab) -> Option<String> {
    if tab.waiting > 0 {
        return Some(if tab.waiting > 1 {
            format!("need you \u{d7}{}", tab.waiting)
        } else {
            "need you".to_string()
        });
    }
    tab.activity.clone()
}

/// One delegate, drawn as a card of its own inside its parent's fence.
///
/// **It is the agent card at a smaller scale, not a different thing.** Same slab, same left edge in
/// the state's colour, same selected fill, same grab — because a delegate is another agent doing
/// another piece of the work, and the one thing that tells it apart on the canvas is its size and
/// the line running back to whoever spawned it.
///
/// **It says only what the harness stamped.** `SubagentTab` carries a type, a model, a thinking
/// level and its current activity and nothing else — no branch, no note — so the card draws the
/// delegate's title over its type, the state's chip, the model, and the activity, and keeps the
/// rest for the tooltip.
///
/// **Four rows, top to bottom: mark and name, harness and model, the command it is running, and a
/// footer.** The same shape [`agent_card`] draws, at the delegate's grain — the one thing this mode
/// never draws twice differently. The footer's ring is the one fact a delegate's own transcript
/// can state about its spend: [`Conversation::subagent_tokens`] banks it by type, which is the only
/// grain the wire carries, so there is no per-delegate context ring beside it — a parent's ring
/// drawn under a delegate's card would be a number about somebody else (`G96`).
#[allow(clippy::too_many_arguments)]
fn subagent_card(
    agent: AgentId,
    harness: &str,
    tab: &SubagentTab,
    conversation: Option<&Conversation>,
    ix: usize,
    at: (f32, f32),
    sub: (f32, f32),
    selected: bool,
    carried: bool,
    zoom: f32,
    view: &Entity<AppState>,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let status = delegate_status(tab);
    let colour = card_colour(status);
    // One line, `\u{b7}`-separated, the way every other tooltip in the interface reads — and each part
    // is drawn only where the harness said it. The state leads it: what the delegate is doing is
    // what the reader came for, and the rest is detail.
    let tip = [
        Some(status.label().to_string()),
        Some(tab.name.clone()),
        tab.kind.clone(),
        tab.model.clone(),
        tab.thinking.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" \u{b7} ");
    let id = tab.id.clone();
    let held = TeamsHeld::Subagent {
        agent,
        subagent: tab.id.clone(),
    };
    let view = view.clone();

    // A card sits on the dotted ground, so it needs a ground of its own — except when it is the one
    // being read, where the selected fill is already behind it.
    let mut body = blocks::block(
        eid2("teams-subagent", agent, ix),
        (at.0, at.1, sub.0, sub.1),
        Look::new(colour)
            .selected(selected)
            .carried(carried)
            .opaque(true),
        zoom,
    )
    // Row 1: the mark, then the delegate's own name. The state's chip is the footer's, flush in
    // the card's bottom-right corner — this row says whose card it is, not what it is doing.
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.0 * zoom))
            .child(status_mark(
                status,
                18.0 * zoom,
                eid2("teams-subagent-mark", agent, ix),
            ))
            .child(elided_with(
                eid2("teams-subagent-name", agent, ix),
                tab.name.clone(),
                tip.clone(),
                theme::text(),
                theme::font(Family::Chrome, Role::Body) * zoom,
            )),
    )
    // Row 2: the harness it runs under, and the model — the alias, not the catalogue id, the same
    // cut the composer's chip makes, so one project never spells a model two ways. The tooltip
    // above still carries it in full.
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.0 * zoom))
            .children((!harness.is_empty()).then(|| {
                Icon::new(harness_icon(harness))
                    .with_size(theme::icon_sm() * zoom)
                    .flex_none()
                    .text_color(theme::text_muted())
            }))
            .children(tab.model.as_deref().map(|model| {
                mono(short_model_label(harness, model), theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom)
                    .truncate()
            }))
            .children(tab.kind.clone().map(|kind| {
                mono(kind.to_uppercase(), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom)
            })),
    );

    // Row 3: the command it is running — the permission wait where it has one, its own last
    // activity otherwise. Single line, clipped by the block rather than pushing the footer down.
    if let Some(activity) = delegate_activity_label(tab) {
        body = body.child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Label) * zoom)
                .text_color(theme::text_muted())
                .truncate()
                .child(activity),
        );
    }

    // The footer: what this delegate's type has spent, on the left — the state's chip is not here,
    // it is pinned to the card's own corner below, past this row's flow and its padding.
    let spend = tab
        .kind
        .as_deref()
        .and_then(|kind| conversation.and_then(|c| c.subagent_tokens(kind)));
    body = body.child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0 * zoom))
            .flex_wrap()
            .children(spend.and_then(|(total, cached)| token_ring(total, cached, zoom))),
    );

    // The activity indicator: the card's own bottom-right corner, flush with the block's edge —
    // past the padding every other row sits inside, because chrome does not pad (`ubiq-ui`).
    body = body.child(
        div()
            .absolute()
            .bottom_0()
            .right_0()
            .child(status_chip(status, zoom)),
    );

    body.on_click(
        cx.listener(move |this, _, _, cx| this.open_teams_subagent(agent, id.clone(), cx)),
    )
    .on_drag(Carried(held), move |carried, grab, _, cx: &mut App| {
        let grab = (f32::from(grab.x), f32::from(grab.y));
        let held = carried.0.clone();
        view.update(cx, |this, cx| this.start_teams_carry(held, grab, cx));
        cx.new(|_| Empty)
    })
    .into_any_element()
}

/// The consumed-token ring and its text: how much of what has been spent came back from cache,
/// against the total — the conversation footer's own `cache-ring` reading, at the card's grain.
/// `None` where nothing has been spent yet, on the rule every ring on this canvas follows: no
/// percentage over an invented denominator (`G96`).
fn token_ring(total: u64, cached: u64, zoom: f32) -> Option<gpui::AnyElement> {
    if total == 0 {
        return None;
    }
    let pct = ((cached as f64 / total as f64) * 100.0).round() as u8;
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.0 * zoom))
            .child(progress_ring_in(pct.min(100), 12.0 * zoom, theme::info()))
            .child(
                mono(
                    format!("{} tok", work::format_tokens(total)),
                    theme::text_faint(),
                )
                .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
            )
            .into_any_element(),
    )
}

/// The colour of the project one card belongs to, or `None` where the registry has never heard of
/// it — the same resolution the chip on the card and the pill over it draw from, so a fence, a chip
/// and a badge cannot disagree about what a project looks like.
fn card_tint(app: &AppState, agent: AgentId, cx: &App) -> Option<Rgba> {
    app.project_of_agent(agent, cx)
        .and_then(|project| project_face(project, cx))
        .map(|face| face.tint)
}

/// One card. Everything about it is either a fact the record carries or a colour from a token —
/// nothing on it is invented at draw time, and where it goes is handed in rather than read off it.
#[allow(clippy::too_many_arguments)]
fn agent_card(
    agent: &WorkAgent,
    status: Status,
    context: Option<u8>,
    spend: Option<(u64, u64)>,
    project: Option<ProjectFace>,
    at: (f32, f32),
    selected: bool,
    carried: bool,
    zoom: f32,
    view: &Entity<AppState>,
    cx: &mut Context<AppState>,
) -> gpui::AnyElement {
    let id = agent.id;
    // The card's edge, its mark and its chip all read the same state, so a card cannot say one
    // thing in colour and another in words.
    let colour = card_colour(status);
    let view = view.clone();

    let body = blocks::block(
        eid("teams-card", id),
        (at.0, at.1, CARD_WIDTH, TEAMS_CARD_HEIGHT),
        Look::new(colour).selected(selected).carried(carried),
        zoom,
    )
    // Row 1: the status mark, then whoever is answering — a persistent agent's own name where it
    // has one, the bare harness otherwise. The project chip, under the window span only, is the
    // one thing this row still owes to "whose card is this" beside what it is doing.
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.0 * zoom))
            .child(status_mark(status, 22.0 * zoom, eid("teams-card-mark", id)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(theme::font(Family::Chrome, Role::Body) * zoom)
                    .text_color(theme::text())
                    .truncate()
                    .child(SharedString::from(if agent.name.is_empty() {
                        agent.harness.clone()
                    } else {
                        agent.name.clone()
                    })),
            )
            .children(project.map(|face| {
                project_chip(eid("teams-card-project", id), &face, zoom).into_any_element()
            })),
    )
    // Row 2: what is answering, and its model — the harness is its mark and the model is its
    // name, because the mark is what tells two cards apart at a glance and the name is what
    // tells one card what it is.
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.0 * zoom))
            .children((!agent.harness.is_empty()).then(|| {
                Icon::new(harness_icon(&agent.harness))
                    .with_size(theme::icon_sm() * zoom)
                    .flex_none()
                    .text_color(theme::text_muted())
            }))
            .child(
                mono(
                    if agent.model.is_empty() {
                        agent.harness.clone()
                    } else {
                        short_model_label(&agent.harness, &agent.model)
                    },
                    theme::text_muted(),
                )
                .text_size(theme::font(Family::Chrome, Role::Micro) * zoom)
                .flex_1()
                .min_w(px(0.))
                .truncate(),
            ),
    )
    // Row 3: the title or the activity it is on — drawn only where there is one, on the same rule
    // the delegate card's own third row follows, so a card between commands does not carry a blank
    // line the footer would otherwise be stretched to clear.
    .children((!agent.note.is_empty()).then(|| {
        div()
            .flex_none()
            .text_size(theme::font(Family::Chrome, Role::Label) * zoom)
            .text_color(theme::text_muted())
            .truncate()
            .child(SharedString::from(agent.note.clone()))
    }))
    // The footer: how full the harness's window is and what it has spent — wrapping onto a second
    // row where the card is too narrow for it. The current activity chip is not in this flow; it
    // is pinned to the card's own corner below, so it never has to fight this row for space.
    //
    // **Flush under its own content, not stretched to the card's fixed height.** A card whose row 3
    // is empty used to leave this row's flex fill the rest of the box, so the ring drew flush with
    // the card's own bottom-right corner instead of under row 2 — the wasted middle the card's
    // geometry (`TEAMS_CARD_HEIGHT`) alone cannot answer for, since the box a packer reserves is
    // still one fixed size for every card.
    .child(
        div()
            .flex()
            .flex_none()
            .min_w(px(0.))
            .flex_wrap()
            .items_center()
            .gap(px(8.0 * zoom))
            // No ring where no harness stated a window: a ratio with an invented denominator is
            // worse than none.
            .children(context.map(|pct| {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.0 * zoom))
                    .child(progress_ring_in(
                        pct.min(100),
                        12.0 * zoom,
                        theme::usage_tone(pct),
                    ))
                    .child(
                        mono(format!("{pct}% ctx"), theme::text_faint())
                            .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
                    )
            }))
            .children(spend.and_then(|(total, cached)| token_ring(total, cached, zoom))),
    )
    // The activity indicator: the card's own bottom-right corner, flush with the block's edge —
    // past the padding every other row sits inside, because chrome does not pad (`ubiq-ui`).
    .child(
        div()
            .absolute()
            .bottom_0()
            .right_0()
            .child(status_chip(status, zoom)),
    )
    .on_click(
        cx.listener(move |this, _, _, cx| this.select_in_teams(TeamsSelection::Agent(id), cx)),
    )
    .on_drag(
        Carried(TeamsHeld::Agent(id)),
        move |_, grab, _, cx: &mut App| {
            // The grab point is where inside the card the pointer went down. Keeping it is what
            // stops the card jumping under the cursor on the first move.
            let grab = (f32::from(grab.x), f32::from(grab.y));
            view.update(cx, |this, cx| {
                this.start_teams_carry(TeamsHeld::Agent(id), grab, cx)
            });
            cx.new(|_| Empty)
        },
    );

    body.into_any_element()
}
