//! The tasks board: every task in the project as a card, in the column that says how far along it
//! is, with the one that is selected reported beside it.
//!
//! It is the third view of the work, beside the agents screen's columns and the orchestration
//! graph. The graph answers "who is doing what"; the board answers "what is there, and where has it
//! got to" — the same tasks, in [`crate::state::work`], read at the scale of the project rather
//! than of one session. Which is why a card carries an agent's name and a state: the screens are
//! three questions about one set of facts, not three sets.
//!
//! **A card draws what it has.** A key, a kind, labels, a shape, a session, a link — each is drawn
//! where somebody filled it in and takes no space where nobody did, down to the whole bottom row
//! going away. A row saying a task has none of those would take the same space as one saying it
//! has all of them, and say nothing.
//!
//! Three things on it are live. **A task is asked for** from the filter field — one field to find
//! work and to name it — and lands in the backlog. **A card is dragged**, and unlike the graph's
//! canvas the target is a box rather than a point: a task is filed somewhere rather than placed
//! anywhere. The column it lands in lights up and the gap it lands in is marked with a bar, and
//! the gap is a card's answer — each card claims the pointer over its own half before the column
//! behind it does, which leaves the column meaning the end of itself.
//! **A column and a card both shut**, to a strip and to a title, because a board is read by
//! ignoring most of it.
//!
//! What a column a card is in is the host's, so a drag asks rather than moves, and the card is
//! drawn muted until the answer comes back — a slow host must not read as a drag that failed.
//!
//! Three files: this one is the toolbar, the columns and the cards; [`detail`] is the panel that
//! reports one task, and [`form`] is the controls that change it.

pub mod detail;
pub mod form;

use gpui::{
    AnyElement, App, AppContext as _, Context, DragMoveEvent, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled,
    Window, div, prelude::FluentBuilder, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::ids::TaskId;
use ubiq_proto::work::{Status, TaskRecord};

use crate::app::AppState;
use crate::state::work;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::eid;
use crate::ui::kit::{
    card, choice_pill, field, ghost_button, meter, mono, pill, primary_button, section_label,
    toggle_pill,
};
use crate::ui::work::{activity_colour, bucket_colour};

/// The task under the pointer. It carries the id alone: where the task belongs is the column's
/// answer, not the drag's.
#[derive(Clone)]
pub struct Dragged(pub TaskId, pub SharedString);

/// What follows the pointer during a drag. The card stays where it is and a label travels, because
/// a task being filed is going somewhere rather than moving somewhere.
struct Ghost(SharedString);

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(theme::surface_raised())
            .border_l(px(theme::accent_edge()))
            .border_color(theme::accent())
            .text_size(theme::font(Family::Chrome, Role::Label))
            .text_color(theme::text())
            .child(self.0.clone())
    }
}

/// What a column's dot reads in. A column is a stage rather than a state, so it borrows the token
/// that means the same thing: nothing yet, queued, moving, waiting on a person, over.
pub fn status_colour(status: Status) -> Rgba {
    match status {
        Status::Backlog => theme::text_faint(),
        Status::Ready => theme::info(),
        Status::InProgress => theme::success(),
        Status::InReview => theme::warning(),
        Status::Done => theme::accent_muted(),
    }
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The board is a view of one project's work, and the shell keeps a window with no project off
    // it entirely — so there is nothing here to draw rather than an empty board to explain.
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };

    let mut body = div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .child(columns(app, cx).into_any_element());

    if let Some(task) = board.open_task(work) {
        body = body.child(
            div()
                .w(px(theme::TASK_PANEL_WIDTH))
                .flex()
                .flex_none()
                .border_l_1()
                .border_color(theme::border())
                .child(detail::render(app, task, window, cx)),
        );
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(toolbar(app, window, cx))
        .child(body)
        .into_any_element()
}

/// The strip over the columns: what is being looked for, whose work it is, and the way to add one.
///
/// Every filter on it clears. The session row leads with an `all` that draws every session, and a
/// label row with nothing lit is not filtering — so a board emptied by a filter is always one click
/// from being full again, and the control at the end does the whole row at once. It is drawn only
/// while something is being hidden: a reset with nothing to reset is a button that lies.
fn toolbar(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };

    let sessions: Vec<AnyElement> = work
        .sessions
        .iter()
        .map(|session| {
            let id = session.id;
            choice_pill(
                eid("board-session", id),
                session.name.clone(),
                board.session == Some(id),
                cx.listener(move |this, _, _, cx| this.pick_board_session(Some(id), cx)),
            )
            .into_any_element()
        })
        .collect();

    // One pill per label anybody in the project has used, in the swatch that label was given. The
    // pills narrow together rather than in turn, which is why they are switches and the session row
    // beside them is a choice.
    let labels: Vec<AnyElement> = work
        .labels()
        .into_iter()
        .map(|label| {
            let name = label.name.clone();
            let lit = board.is_label_on(&name);
            toggle_pill(
                eid("board-label", name.clone()),
                label.name.clone(),
                theme::project_colour(label.colour),
                lit,
                cx.listener(move |this, _, _, cx| this.toggle_board_label(&name, cx)),
            )
            .into_any_element()
        })
        .collect();

    div()
        .min_h(px(theme::titlebar_height()))
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(filter_field(app, window, cx))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .child(choice_pill(
                    "board-session-all",
                    "all sessions",
                    board.session.is_none(),
                    cx.listener(|this, _, _, cx| this.pick_board_session(None, cx)),
                ))
                .children(sessions)
                .children(labels),
        )
        .children(board.filtering().then(|| {
            ghost_button(
                "board-show-all",
                None,
                "Show everything",
                cx.listener(|this, _, _, cx| this.clear_board_filters(cx)),
            )
        }))
        .child(primary_button(
            "board-new-task",
            Some(IconName::Plus),
            "New task",
            cx.listener(|this, _, window, cx| this.new_task(window, cx)),
        ))
        .into_any_element()
}

/// One field, doing both jobs: it filters the cards, and what is in it names the next one.
fn filter_field(app: &AppState, window: &Window, cx: &App) -> impl IntoElement {
    let focused = app.task_filter.read(cx).focus_handle(cx).is_focused(window);
    field(theme::border(), focused)
        .w(px(260.))
        .h(px(28.))
        .px_2()
        .gap_2()
        .child(
            Icon::new(IconName::Search)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .child(Input::new(&app.task_filter).appearance(false)),
        )
}

fn columns(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .id("board-columns")
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .p_3()
        .gap_2()
        .overflow_x_scroll()
        .children(
            Status::all()
                .into_iter()
                .map(|status| column(app, status, cx)),
        )
}

fn column(app: &AppState, status: Status, cx: &mut Context<AppState>) -> AnyElement {
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };
    let tasks = board.column(work, status);
    let count = tasks.len();
    let shut = board.is_shut(status);
    let lit = board.carry.is_some_and(|carry| carry.over == Some(status));
    let colour = status_colour(status);
    // The three ids on a column key off the enum's discriminant rather than an id: a column is one
    // of five stages, not a record, so there is nothing here for a ULID to name.
    let key = status as u32;

    let mut root = div()
        .id(("board-column", key))
        .w(px(if shut {
            theme::COLUMN_SHUT
        } else {
            theme::COLUMN_WIDTH
        }))
        .flex()
        .flex_none()
        .flex_col()
        .bg(theme::pane_bg())
        .border_l(px(theme::accent_edge()))
        .border_color(if lit { theme::accent() } else { colour })
        // The column a drop would file the card into says so by lighting up, which is the only
        // answer the user gets before letting go.
        .when(lit, |this| this.bg(theme::accent_soft()))
        // A drag that never enters a column changes nothing: the pointer has to be inside this
        // box for it to claim the drop. The column means the *end* of itself — a card claims the
        // gap it is over before this sees the pointer, so what is left is the space under them all,
        // and the empty column.
        .on_drag_move(
            cx.listener(move |this, event: &DragMoveEvent<Dragged>, _, cx| {
                if event.bounds.contains(&event.event.position) {
                    this.drag_task_over(status, None, cx);
                }
            }),
        )
        .on_drop(cx.listener(move |this, _: &Dragged, _, cx| this.drop_task(status, None, cx)));

    if shut {
        // Shut, a column is a strip that still counts and still takes a drop. The name is written
        // downwards a letter at a time, which is the only way a 44px column can carry it.
        return root
            .items_center()
            .py_2()
            .gap_1p5()
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .child(
                Icon::new(IconName::ChevronRight)
                    .with_size(Size::XSmall)
                    .text_color(theme::text_faint()),
            )
            .child(div().size(px(7.)).flex_none().rounded_full().bg(colour))
            .child(
                mono(format!("{count}"), theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            )
            .children(
                status
                    .label()
                    .to_uppercase()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .map(|c| {
                        mono(c.to_string(), theme::text_faint())
                            .text_size(theme::font(Family::Chrome, Role::Micro))
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_board_column(status, cx)))
            .into_any_element();
    }

    // The gap a drop would land in, while there is a carry over this column. `None` is the end of
    // it, so the bar goes under the last card rather than in front of any of them.
    let gap = board
        .carry
        .filter(|carry| carry.over == Some(status))
        .map(|carry| carry.before);
    let empty = tasks.is_empty();
    let ids: Vec<TaskId> = tasks.iter().map(|task| task.id).collect();

    let mut cards: Vec<AnyElement> = Vec::new();
    for (ix, task) in tasks.into_iter().enumerate() {
        if gap == Some(Some(task.id)) {
            cards.push(marker());
        }
        // Which card a drop past this one's midpoint lands in front of. Past the last card there
        // is none, which is the end of the column.
        cards.push(task_card(app, task, ids.get(ix + 1).copied(), cx));
    }
    if gap == Some(None) {
        cards.push(marker());
    }

    let body = if empty {
        div()
            .flex()
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Label))
                    .text_color(theme::text_faint())
                    .child("Nothing here."),
            )
            .into_any_element()
    } else {
        div()
            .id(("board-column-body", key))
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .p_2()
            .gap_2()
            .overflow_y_scroll()
            .children(cards)
            .into_any_element()
    };

    root = root
        .child(
            div()
                .id(("board-column-head", key))
                .h(px(34.))
                .px_2p5()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(div().size(px(7.)).flex_none().rounded_full().bg(colour))
                .child(section_label(status.label()))
                .child(div().flex_1().min_w(px(0.)))
                .child(mono(format!("{count}"), theme::text_muted()))
                .child(
                    Icon::new(IconName::ChevronLeft)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_board_column(status, cx))),
        )
        .child(body);

    root.into_any_element()
}

/// The bar in the gap a drop would land in.
///
/// The lit column answers "which column"; this answers "where in it", which is the other half of
/// what the user has to know before letting go. It is the accent at the width every edge in the
/// interface is drawn at, so the gap reads as the same kind of mark as a card's own edge.
fn marker() -> AnyElement {
    div()
        .w_full()
        .h(px(theme::accent_edge()))
        .flex_none()
        .bg(theme::accent())
        .into_any_element()
}

/// A word on a card, in the colour of whatever it is a word about.
///
/// Smaller than the panel's tag: a card carries several of these at once and is read from across a
/// column, so they are marks rather than labels.
fn chip(label: impl Into<SharedString>, colour: Rgba) -> impl IntoElement {
    pill(colour)
        .h(px(18.))
        .px_1p5()
        .gap_1()
        .child(mono(label, colour).text_size(theme::font(Family::Chrome, Role::Micro)))
}

/// One card. Its left edge carries the worst thing happening in the task, because that is what is
/// read from across a column; everything finer than that is the panel's job.
///
/// `next` is the card under this one, which is what a drop below its midpoint lands in front of.
/// The card claims the drop before the column behind it does, so a pointer inside a column always
/// has a gap to answer with rather than only the end of the list.
fn task_card(
    app: &AppState,
    task: &TaskRecord,
    next: Option<TaskId>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };
    let id = task.id;
    let colour = bucket_colour(work.pulse(task));
    let selected = board.selected == Some(id) && board.show_detail;
    let folded = board.is_folded(id);
    let carried = board.carry.is_some_and(|carry| carry.task == id);
    // A drop the host has not answered yet. The card goes muted rather than moving, because the
    // column it is in is the host's answer and this one has not arrived.
    let moving = board.is_moving(id);
    let title = SharedString::from(task.title.clone());
    let ghost = title.clone();
    let view = cx.entity();

    let mut root = card(eid("board-task", id), colour, selected)
        .w_full()
        .p_2p5()
        .gap_1p5()
        .cursor_grab()
        // A card in the air goes opaque, the way a carried agent card does.
        .when(carried, |this| this.bg(theme::surface_raised()))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                // What the task is called elsewhere, what kind of work it is and what it is
                // labelled: three facts a card draws only where somebody has filled them in. A
                // card with none of them is a title and its marks, which is what most of them are.
                .children(task.key.as_deref().map(|key| {
                    mono(key.to_string(), theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                }))
                .children(
                    task.kind
                        .map(|kind| chip(kind.label(), theme::text_muted())),
                )
                .children(
                    task.labels
                        .iter()
                        .map(|label| chip(label.name.clone(), theme::project_colour(label.colour))),
                )
                .children(task.blocked().then(|| {
                    Icon::new(IconName::TriangleAlert)
                        .with_size(Size::XSmall)
                        .text_color(theme::danger())
                }))
                // That a description exists is a fact about the task at card scale, and one mark is
                // all a card can honestly take: what a card carries is fixed, and a folded one
                // keeps its title and the marks on this row and drops everything under them.
                .children((!task.description.trim().is_empty()).then(|| {
                    Icon::new(IconName::BookOpen)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint())
                }))
                // The drop the host has not answered yet, said in the faintest token there is: the
                // card is still in its old column and saying so is the whole point.
                .children(moving.then(|| {
                    mono("moving\u{2026}", theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                }))
                .child(div().flex_1().min_w(px(0.)))
                .children(task.priority.label().map(|label| {
                    mono(
                        label,
                        if label == "high" {
                            theme::danger()
                        } else {
                            theme::text_faint()
                        },
                    )
                    .text_size(theme::font(Family::Chrome, Role::Meta))
                }))
                .child(
                    div()
                        .id(eid("board-task-fold", id))
                        .flex()
                        .flex_none()
                        .items_center()
                        .px_1()
                        .cursor_pointer()
                        .hover(|this| this.bg(theme::hover()))
                        .child(
                            Icon::new(if folded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronUp
                            })
                            .with_size(Size::XSmall)
                            .text_color(theme::text_faint()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle_task_fold(id, cx))),
                ),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child(title),
        )
        .children(shape_line(app, task, cx));

    if !folded {
        if !task.steps.is_empty() {
            root = root.child(meter(work::fraction(task), colour));
        }
        root = root.child(now_line(app, task, cx));
    }

    let status = task.status;

    root.on_click(cx.listener(move |this, _, _, cx| this.select_task(id, cx)))
        // Above the midpoint is the gap in front of this card, below it the gap behind — which is
        // in front of the next one, or the end of the column when there is none. The card answers
        // before the column does, so the pointer is never told only which column it is in.
        .on_drag_move(
            cx.listener(move |this, event: &DragMoveEvent<Dragged>, _, cx| {
                if !event.bounds.contains(&event.event.position) {
                    return;
                }
                cx.stop_propagation();
                let before = if event.event.position.y < event.bounds.center().y {
                    Some(id)
                } else {
                    next
                };
                this.drag_task_over(status, before, cx);
            }),
        )
        // A drop carries no bounds, and the pointer has not moved since the last drag-move, so the
        // gap that one settled on is the gap this lands in. Without this the drop would fall
        // through to the column and mean the end of it.
        .on_drop(cx.listener(move |this, _: &Dragged, _, cx| {
            cx.stop_propagation();
            let before = this.board(cx).and_then(|board| board.carry?.before);
            this.drop_task(status, before, cx);
        }))
        .on_drag(Dragged(id, ghost.clone()), move |_, _, _, cx: &mut App| {
            let ghost = ghost.clone();
            view.update(cx, |this, cx| this.start_task_carry(id, cx));
            cx.new(|_| Ghost(ghost))
        })
        .into_any_element()
}

/// How the agents on the task are arranged, whose work it is, and the issue it stands for.
///
/// Three optional facts on one line, and **no line at all** when none of them is set. A card draws
/// what it has: a row saying a task has no shape, no session and no link would take the same space
/// as one saying it has all three, and say nothing. Which session a task belongs to is offered in
/// the panel's picker, which is where a task is handed back to nobody.
fn shape_line(app: &AppState, task: &TaskRecord, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let session = app
        .work(cx)
        .and_then(|work| task.session.and_then(|id| work.session(id)))
        .map(|session| session.name.clone());
    let link = task.link.clone();

    if task.shape.is_none() && session.is_none() && link.is_none() {
        return None;
    }

    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1p5()
            .children(task.shape.map(|shape| {
                div().px_1().border_1().border_color(theme::border()).child(
                    mono(shape.label(), theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Micro)),
                )
            }))
            .children(session.map(|name| {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .child(
                        Icon::new(IconName::Network)
                            .with_size(Size::XSmall)
                            .text_color(theme::text_faint()),
                    )
                    .child(
                        mono(name, theme::text_muted())
                            .text_size(theme::font(Family::Chrome, Role::Meta)),
                    )
            }))
            .children(link.map(|url| link_chip(task.id, url, cx)))
            .into_any_element(),
    )
}

/// The issue the task stands for, as the tracker's glyph and its name.
///
/// Clicking it hands the URL to the operating system, the same thing a link in a rendered
/// description does — the interface has no browser of its own to open it in.
fn link_chip(task: TaskId, url: String, cx: &mut Context<AppState>) -> impl IntoElement {
    let (icon, provider) = issue_provider(&url);
    let tooltip = SharedString::from(url.clone());

    div()
        .id(eid("board-task-link", task))
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(icon)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .child(
            mono(provider, theme::text_muted()).text_size(theme::font(Family::Chrome, Role::Meta)),
        )
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
        })
        .on_click(cx.listener(move |_, _, _, cx| {
            cx.stop_propagation();
            cx.open_url(&url);
        }))
}

/// Which tracker a link points at, read off the host in the URL and nothing more.
///
/// The interface does this rather than the host, which stores the string and never parses it:
/// nothing here is configured and nothing is fetched, and a URL nobody recognises is still a link.
/// Only GitHub has a mark in the icon set, so what says which of the others it is is the word
/// beside the glyph rather than the glyph.
fn issue_provider(url: &str) -> (IconName, &'static str) {
    let host = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if host.ends_with("github.com") {
        (IconName::Github, "GitHub")
    } else if host.ends_with("dev.azure.com") || host.ends_with("visualstudio.com") {
        (IconName::Globe, "Azure DevOps")
    } else if host.ends_with("atlassian.net") || host.contains("jira") {
        (IconName::Globe, "Jira")
    } else {
        (IconName::ExternalLink, "link")
    }
}

/// The bottom line of a card: the agent holding the task and what it is saying, or — when nobody
/// is — how many sub-tasks there are to be done.
fn now_line(app: &AppState, task: &TaskRecord, cx: &mut Context<AppState>) -> AnyElement {
    let Some(agent) = app.work(cx).and_then(|work| work.now(task)) else {
        let total = task.steps.len();
        let text = if total == 0 {
            "no sub-tasks yet".to_string()
        } else {
            format!("{}/{total} sub-tasks", task.done())
        };
        return mono(text, theme::text_muted())
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .into_any_element();
    };

    let colour = activity_colour(agent.activity);
    let id = agent.id;

    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        // Only the name takes the click. The line beside it is what the agent is saying, and a
        // sentence that changes the screen when you touch it is a trap.
        .child(
            div()
                .id(eid("board-task-now", task.id))
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .px_1()
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(div().size(px(6.)).flex_none().rounded_full().bg(colour))
                .child(
                    mono(agent.name.clone(), colour)
                        .text_size(theme::font(Family::Chrome, Role::Meta)),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.open_task_chat(id, cx))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .truncate()
                .child(SharedString::from(format!("\u{2014} {}", agent.note))),
        )
        .into_any_element()
}
