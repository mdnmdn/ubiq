//! The WBS tab: the mission's prerequisite graph, laid out by dependency level (§6.2, M20, M21).
//!
//! **The arrangement is [`crate::state::wbs`]'s and the drawing is the Teams canvas's.** Nothing
//! here is a graph engine: the levels come from a topological layering that knows nothing about
//! pixels, and what turns them into a picture is `kit::blocks` — one [`Fence`] per level, one
//! [`blocks::block`] per task, the prerequisite curves through [`Board::link`], all on the same
//! dotted ground and the same scroller the Teams graph uses.
//!
//! **One data set, two shapes.** *Graph* and *Table* say the same thing — the table's columns are
//! the node's own facts spelled out (key, title, level, state, waits on, blocks, labels,
//! assignee), and both honour `MissionView::state_filter`, so the narrowing a progress segment set
//! survives the move between tabs.
//!
//! **Selecting a node opens the board's own detail** beside it (`ui/board/detail.rs`, whose
//! controls are `ui/board/form.rs`): a task is edited here exactly as it is on the board,
//! prerequisites included. There is deliberately no mission-specific editor, and no edge dragging
//! — changing a prerequisite is the detail's Prerequisites chip list, and the drag is a later
//! refinement.
//!
//! **The two shapes that are not a chain read as themselves.** A mission with no prerequisites at
//! all is one level holding every task; a disconnected graph is several chains against the same
//! levels, with no edge between them. Neither collapses, and a mission with no tasks says so.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, Window, div, point, prelude::FluentBuilder as _, px,
};

use ubiq_proto::ids::TaskId;
use ubiq_proto::work::TaskRecord;

use crate::app::AppState;
use crate::state::mission::MissionView;
use crate::state::wbs::{self, Wbs};
use crate::state::work::{WorkProjection, WorkState};
use crate::theme::{self, Family, Role};
use crate::ui::kit::blocks::{self, Board, Fence, Look, Word};
use crate::ui::kit::canvas::Link;
use crate::ui::kit::{choice_pill, elided, mono, state_chip, stepper, toggle_pill};
use crate::ui::mission::panel::nothing;
use crate::ui::work::work_state_colour;
use crate::ui::{eid, eid2};

/// One node at 100% zoom, and what the arrangement leaves round it. The node is three lines: the
/// key, the title, and the state with what it waits on.
const NODE: (f32, f32) = (196.0, 64.0);
const NODE_GAP: f32 = 26.0;
const LEVEL_GAP: f32 = 52.0;
/// What a level's fence leaves round its nodes, and the band its label sits in.
const LEVEL_PAD: f32 = 14.0;
const LEVEL_LABEL: f32 = 22.0;
/// Room past the outermost node, so the edge of the graph is still reachable.
const MARGIN: f32 = 28.0;

/// The tab: the toolbar, the graph or the table, and the selected task's detail beside it.
pub fn render(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    view: &MissionView,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let tasks: Vec<&TaskRecord> = work.children_of(task_id).collect();
    let arranged = wbs::layers(&tasks);

    let body = match tasks.is_empty() {
        true => nothing("No tasks yet \u{2014} the breakdown is the planner's to write (M21).")
            .into_any_element(),
        false => match view.wbs_table {
            true => table(work, &tasks, &arranged, view, cx),
            false => graph(
                work,
                &tasks,
                &arranged,
                view,
                &app.mission_scroll,
                window,
                cx,
            ),
        },
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(toolbar(work, task_id, &arranged, view, cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(body)
                .children(detail(app, work, task_id, view, window, cx)),
        )
        .into_any_element()
}

/// *Graph | Table*, the critical-path switch, the zoom, and the same state filter the Tasks tab
/// carries — one narrowing, read by both tabs.
fn toolbar(
    work: &WorkProjection,
    task_id: TaskId,
    arranged: &Wbs,
    view: &MissionView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let filters: Vec<AnyElement> = work
        .work_state_counts(task_id)
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(state, n)| {
            let on = view.state_filter == Some(state);
            choice_pill(
                eid2("wbs-filter", task_id, state.label()),
                format!("{n} {}", state.label()),
                on,
                cx.listener(move |this, _, _, cx| this.toggle_mission_state_filter(state, cx)),
            )
            .into_any_element()
        })
        .collect();

    div()
        .h(px(34.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .border_b_1()
        .border_color(theme::border())
        .child(choice_pill(
            "wbs-shape-graph",
            "Graph",
            !view.wbs_table,
            cx.listener(|this, _, _, cx| this.toggle_mission_wbs_table(false, cx)),
        ))
        .child(choice_pill(
            "wbs-shape-table",
            "Table",
            view.wbs_table,
            cx.listener(|this, _, _, cx| this.toggle_mission_wbs_table(true, cx)),
        ))
        // The chain that decides how many rounds the mission takes. Off by default: it is a
        // reading of the graph, not a property of it.
        .child(toggle_pill(
            "wbs-critical",
            format!("critical path \u{00b7} {}", arranged.critical.len()),
            theme::accent(),
            view.wbs_critical,
            cx.listener(|this, _, _, cx| this.toggle_mission_critical_path(cx)),
        ))
        .child(div().flex_1().min_w(px(0.)))
        .children(filters)
        .when(!view.wbs_table, |row| {
            row.child(stepper(
                "wbs-zoom",
                format!("{}%", (view.zoom * 100.0).round() as i32),
                cx.listener(|this, _, _, cx| {
                    this.zoom_mission_wbs(-crate::state::teams::ZOOM_STEP, cx)
                }),
                cx.listener(|this, _, _, cx| {
                    this.zoom_mission_wbs(crate::state::teams::ZOOM_STEP, cx)
                }),
            ))
        })
        .into_any_element()
}

// ── the graph ───────────────────────────────────────────────────────

/// The layered graph on the Teams canvas: a fence per level, a block per task, a curve per
/// prerequisite.
fn graph(
    work: &WorkProjection,
    tasks: &[&TaskRecord],
    arranged: &Wbs,
    view: &MissionView,
    scroll: &gpui::ScrollHandle,
    window: &Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let zoom = view.zoom;
    let mut board = Board::new(zoom, MARGIN);

    // Where every node sits at 100% zoom, so the fences, the blocks and the curves are all read
    // off one arrangement rather than three that could disagree.
    let mut at: Vec<(TaskId, (f32, f32))> = Vec::new();
    let mut y = 0.0f32;
    for (level, row) in arranged.rows.iter().enumerate() {
        let width = match row.len() {
            0 => NODE.0,
            n => n as f32 * NODE.0 + (n - 1) as f32 * NODE_GAP,
        };
        let height = NODE.1 + LEVEL_PAD * 2.0 + LEVEL_LABEL;
        let rect = (0.0, y, width + LEVEL_PAD * 2.0, height);

        // A level with nothing on it cannot happen — a level exists because a task is on it — so
        // the fence is always a fence round something.
        board.fence(Fence::new(rect, theme::border(), false).labelled(
            vec![
                Word::new(format!("L{level}"), theme::text_muted(), Role::Meta),
                Word::new(
                    match row.len() {
                        1 => "1 task".to_string(),
                        n => format!("{n} tasks"),
                    },
                    theme::text_faint(),
                    Role::Micro,
                ),
            ],
            (LEVEL_PAD, 0.0),
            LEVEL_LABEL,
        ));

        for (place, id) in row.iter().enumerate() {
            at.push((
                *id,
                (
                    LEVEL_PAD + place as f32 * (NODE.0 + NODE_GAP),
                    y + LEVEL_PAD + LEVEL_LABEL,
                ),
            ));
        }
        y += height + LEVEL_GAP;
    }

    let held: Vec<TaskId> = tasks.iter().map(|task| task.id).collect();
    let point_of = |id: TaskId| at.iter().find(|(held, _)| *held == id).map(|(_, p)| *p);

    // An edge leaves its prerequisite's bottom and arrives at the top of what waits on it — the
    // curve's control points are vertical, which is why the levels run down the canvas.
    for task in tasks {
        let Some(to) = point_of(task.id) else {
            continue;
        };
        for prereq in &task.prerequisites {
            if !held.contains(prereq) {
                continue;
            }
            let Some(from) = point_of(*prereq) else {
                continue;
            };
            let on_chain =
                view.wbs_critical && arranged.is_critical(task.id) && arranged.is_critical(*prereq);
            let colour = match on_chain {
                true => theme::accent(),
                false => theme::fade(
                    work.task(*prereq)
                        .map(|record| work_state_colour(work.work_state(record)))
                        .unwrap_or_else(theme::text_faint),
                    0.45,
                ),
            };
            board.link(Link {
                from: point((from.0 + NODE.0 / 2.0) * zoom, (from.1 + NODE.1) * zoom),
                to: point((to.0 + NODE.0 / 2.0) * zoom, to.1 * zoom),
                colour,
            });
        }
    }

    for task in tasks {
        let Some(spot) = point_of(task.id) else {
            continue;
        };
        board.block(
            (spot.0, spot.1, NODE.0, NODE.1),
            node(work, task, arranged, view, (spot.0, spot.1), zoom, cx),
        );
    }

    blocks::scroller("mission-wbs", scroll)
        .child(board.content(window.viewport_size()))
        .into_any_element()
}

/// One task as a block: the key, the title, its work state and what it is still waiting on.
fn node(
    work: &WorkProjection,
    task: &TaskRecord,
    arranged: &Wbs,
    view: &MissionView,
    at: (f32, f32),
    zoom: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = task.id;
    let state = work.work_state(task);
    let colour = work_state_colour(state);
    let waiting = task.waiting_on(&work.tasks).len();
    let selected = view.selected == Some(id);
    // A node outside the filter keeps its place and its edges — the shape of the graph is what the
    // tab is for — and reads back rather than disappearing.
    let dimmed = view.state_filter.is_some_and(|want| want != state);
    // The chain is drawn as the accent on the node's own edge, which is the one place a block says
    // what it is part of.
    let edge = match view.wbs_critical && arranged.is_critical(id) {
        true => theme::accent(),
        false => colour,
    };

    blocks::block(
        eid("wbs-node", id),
        (at.0, at.1, NODE.0, NODE.1),
        Look::new(match dimmed {
            true => theme::fade(edge, 0.35),
            false => edge,
        })
        .selected(selected)
        .opaque(true),
        zoom,
    )
    .cursor_pointer()
    .on_click(cx.listener(move |this, _, _, cx| this.select_wbs_task(id, cx)))
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0 * zoom))
            .children(task.key.clone().map(|key| {
                mono(key, theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom)
            }))
            .child(
                mono(format!("L{}", arranged.level_of(id)), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
            ),
    )
    .child(
        elided(
            eid("wbs-node-title", id),
            task.title.clone(),
            match dimmed {
                true => theme::text_muted(),
                false => theme::text(),
            },
            theme::font(Family::Chrome, Role::Body) * zoom,
        )
        .w(px((NODE.0 - 20.0) * zoom)),
    )
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0 * zoom))
            .child(state_chip(state.label(), colour, zoom))
            .when(waiting > 0, |row| {
                row.child(
                    mono(
                        format!("waits on {waiting}"),
                        work_state_colour(WorkState::Waiting),
                    )
                    .text_size(theme::font(Family::Chrome, Role::Micro) * zoom),
                )
            }),
    )
    .into_any_element()
}

// ── the table ───────────────────────────────────────────────────────

/// The same data, spelled out: key, title, level, state, waits on, blocks, labels, assignee.
fn table(
    work: &WorkProjection,
    tasks: &[&TaskRecord],
    arranged: &Wbs,
    view: &MissionView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let mut rows: Vec<&TaskRecord> = tasks
        .iter()
        .copied()
        .filter(|task| {
            view.state_filter
                .is_none_or(|want| want == work.work_state(task))
        })
        .collect();
    rows.sort_by_key(|task| (arranged.level_of(task.id), arranged.place_of(task.id)));

    let body: Vec<AnyElement> = rows
        .iter()
        .map(|task| row(work, tasks, task, arranged, view, cx))
        .collect();

    div()
        .id("mission-wbs-table")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .child(head())
        .when(body.is_empty(), |body| {
            body.child(nothing("No tasks in this state."))
        })
        .children(body)
        .into_any_element()
}

fn head() -> AnyElement {
    let cell = |label: &'static str, width: f32| {
        div()
            .w(px(width))
            .flex_none()
            .text_size(theme::font(Family::Chrome, Role::Micro))
            .text_color(theme::text_faint())
            .child(label)
    };

    div()
        .h(px(22.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .child(cell("KEY", 74.))
        .child(div().flex_1().min_w(px(0.)).child(cell("TITLE", 60.)))
        .child(cell("LEVEL", 44.))
        .child(cell("STATE", 70.))
        .child(cell("WAITS ON", 64.))
        .child(cell("BLOCKS", 56.))
        .child(cell("LABELS", 130.))
        .child(cell("ASSIGNEE", 110.))
        .into_any_element()
}

fn row(
    work: &WorkProjection,
    tasks: &[&TaskRecord],
    task: &TaskRecord,
    arranged: &Wbs,
    view: &MissionView,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let id = task.id;
    let state = work.work_state(task);
    let waiting = task.waiting_on(&work.tasks).len();
    let blocks = tasks
        .iter()
        .filter(|other| other.prerequisites.contains(&id))
        .count();
    let labels = task
        .labels
        .iter()
        .map(|label| label.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let assignee = work
        .now(task)
        .map(|agent| match agent.role.is_empty() {
            true => agent.harness.clone(),
            false => agent.role.clone(),
        })
        .unwrap_or_default();
    let on_chain = view.wbs_critical && arranged.is_critical(id);

    let cell = |element: gpui::ElementId, text: String, width: f32| {
        elided(
            element,
            text,
            theme::text_muted(),
            theme::font(Family::Chrome, Role::Meta),
        )
        .w(px(width))
        .flex_none()
    };

    crate::ui::kit::card(
        eid("wbs-row", id),
        match on_chain {
            true => theme::accent(),
            false => work_state_colour(state),
        },
        view.selected == Some(id),
    )
    .h(px(28.))
    .pr_3()
    .flex()
    .flex_none()
    .flex_row()
    .items_center()
    .gap_2()
    .on_click(cx.listener(move |this, _, _, cx| this.select_wbs_task(id, cx)))
    .child(cell(
        eid("wbs-row-key", id),
        task.key.clone().unwrap_or_default(),
        74.,
    ))
    .child(
        elided(
            eid("wbs-row-title", id),
            task.title.clone(),
            theme::text(),
            theme::font(Family::Chrome, Role::Body),
        )
        .flex_1()
        .min_w(px(0.)),
    )
    .child(cell(
        eid("wbs-row-level", id),
        format!("L{}", arranged.level_of(id)),
        44.,
    ))
    .child(div().w(px(70.)).flex_none().child(state_chip(
        state.label(),
        work_state_colour(state),
        1.0,
    )))
    .child(cell(eid("wbs-row-waits", id), waiting.to_string(), 64.))
    .child(cell(eid("wbs-row-blocks", id), blocks.to_string(), 56.))
    .child(cell(eid("wbs-row-labels", id), labels, 130.))
    .child(cell(eid("wbs-row-assignee", id), assignee, 110.))
    .into_any_element()
}

// ── the detail beside it ────────────────────────────────────────────

/// The selected task, on the board's own detail surface. Nothing here is a second editor: the
/// selection is the board's, so what is open and which field is being edited is one state.
fn detail(
    app: &AppState,
    work: &WorkProjection,
    task_id: TaskId,
    view: &MissionView,
    window: &Window,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let selected = view.selected?;
    // Only a task of this mission: a selection left behind by the board is not this tab's.
    let task = work
        .children_of(task_id)
        .find(|task| task.id == selected)?
        .clone();

    Some(
        div()
            .w(px(theme::task_panel_width()))
            .flex()
            .flex_none()
            .flex_col()
            .border_l_1()
            .border_color(theme::border())
            .child(crate::ui::board::detail::render(app, &task, window, cx))
            .into_any_element(),
    )
}
