//! The tasks board: every task in the project as a card, in the column that says how far along it
//! is, with the one that is selected reported beside it.
//!
//! It is the third view of the work, beside the agents screen's columns and the orchestration
//! graph. The graph answers "who is doing what"; the board answers "what is there, and where has it
//! got to" — the same tasks, in [`crate::state::work`], read at the scale of the project rather
//! than of one session. Which is why a card carries an agent's name and a state: the screens are
//! three questions about one set of facts, not three sets.
//!
//! **A card draws what it has.** A key, a kind, labels, a colour, a shape, a session, a link, a
//! comment count — each is drawn where somebody filled it in and takes no space where nobody did,
//! down to the whole bottom row going away. A row saying a task has none of those would take the
//! same space as one saying it has all of them, and say nothing.
//!
//! Three things on it are live. **A task is asked for** from the filter field — one field to find
//! work and to name it — and lands in the backlog. **A card is dragged**, and unlike the graph's
//! canvas the target is a box rather than a point: a task is filed somewhere rather than placed
//! anywhere. The column it lands in lights up and the gap it lands in is marked with a bar, and
//! the gap is a card's answer — each card claims the pointer over its own half before the column
//! behind it does, which leaves the column meaning the end of itself. **A wheel over a lane moves
//! that lane**, not the board sideways.
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
    AnyElement, App, AppContext as _, Context, DragMoveEvent, Entity, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, Rgba, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement, Styled, Window, div, list, prelude::FluentBuilder, px, rems,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{MissionRecord, Phase};
use ubiq_proto::work::{Level, Status, TaskRecord};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::work;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::eid;
use crate::ui::empty;
use crate::ui::kit::{
    MultiPicker, Picker, PickerStyle, UbiqIcon, card, field, ghost_button, icon_button, meter,
    mono, pill, primary_button, section_label, tag, toggle_pill,
};
use crate::ui::work::{activity_colour, bucket_colour};
use crate::ui::{handler, indexed};

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
        Status::Blocked => theme::danger(),
        Status::InProgress => theme::success(),
        Status::InReview => theme::warning(),
        Status::Done => theme::accent_muted(),
        Status::Abandoned => theme::text_muted(),
    }
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The board is a view of one project's work, and the shell keeps a window with no project off
    // it entirely — so there is nothing here to draw rather than an empty board to explain.
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };

    let body = div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .child(columns(app, cx).into_any_element());

    // The popup toggle is the project's own choice of *shape* for the same task: the docked panel
    // ([`panel`], in the window's right region) and the modal draw the same report and controls off
    // the same `selected`/`editing` fields, so exactly one of the two is on screen at once.
    let popup = board.popup;
    let open_task = board.open_task(work);

    let mut root = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(toolbar(app, window, cx))
        .child(body);

    if board.draft && popup {
        root = root.child(form::draft_popup(app, window, cx));
    } else if let Some(task) = open_task
        && popup
        // A carry selects the card it lifts, the way a dragged agent card does, so that what
        // moves is what the panel reports — but the popup is a modal over the whole board, and a
        // drag (or the drop that ends it) opening one under the pointer is the bug this guard
        // exists to stop. `suppress_popup` outlives the carry itself, through the drop, until a
        // click with no drag behind it puts it back — the side panel needs no such guard: it was
        // already on screen, so a drag just changes what it reports.
        && !board.suppress_popup
    {
        root = root.child(detail::popup(app, task, window, cx));
    }

    root.into_any_element()
}

/// The board's side panel, drawn in the window's right region: the task being written, the task
/// selected, or the page saying neither.
///
/// One slot for the two, because a draft answers `open_task` as nothing — the form and the report
/// can never both be on screen. `board.popup` is the *shape* toggle over the same bodies: with it
/// on, both draw as a modal over the columns ([`render`]) and this panel says where they went
/// rather than emptying, so the toggle back is always in reach.
pub fn panel(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };

    if board.popup {
        return empty::empty_page(
            "Shown as a popup",
            "The task is drawn over the columns. The toolbar's popup switch brings it back here.",
            IconName::WindowRestore,
            None,
        )
        .into_any_element();
    }
    if board.draft {
        return form::draft(app, window, cx);
    }
    match board.open_task(work) {
        Some(task) => detail::render(app, task, window, cx).into_any_element(),
        None => empty::empty_page(
            "No task open",
            "Pick a card to report on it, or add one with New task.",
            UbiqIcon::ModeTasks,
            None,
        )
        .into_any_element(),
    }
}

/// One row of the board toolbar's mission filter (M27).
struct MissionRow {
    id: TaskId,
    /// The mission term, its key and its title, and its phase — the row's own text, searched by
    /// key and title (see [`board_mission_rows`]).
    label: SharedString,
    /// The key and title alone, without the term or the phase — what the closed trigger names.
    name: SharedString,
    dot: Rgba,
    /// Completed or abandoned: sorted last, drawn muted, still pickable.
    dim: bool,
}

/// The board toolbar's mission filter rows: the project's missions, searched by key and title,
/// completed and abandoned sorted last. Read again by the picker's own `on_pick`, exactly as it
/// was drawn — the rule every position-matched menu in this window follows.
fn board_mission_rows(
    app: &AppState,
    work: &work::WorkProjection,
    query: &str,
    cx: &App,
) -> Vec<MissionRow> {
    let term = app.mission_term(cx);
    let empty = std::collections::HashMap::new();
    let missions: &std::collections::HashMap<TaskId, MissionRecord> = app
        .open_project(cx)
        .map(|open| &open.missions)
        .unwrap_or(&empty);
    let mut rows: Vec<(&TaskRecord, &MissionRecord)> = missions
        .values()
        .filter_map(|record| work.task(record.task_id).map(|task| (task, record)))
        .filter(|(task, _)| {
            query.is_empty()
                || task.title.to_lowercase().contains(query)
                || task
                    .key
                    .as_deref()
                    .is_some_and(|key| key.to_lowercase().contains(query))
        })
        .collect();
    rows.sort_by(|(a_task, a_rec), (b_task, b_rec)| {
        let rank = |phase: Phase| matches!(phase, Phase::Completed | Phase::Abandoned) as u8;
        rank(a_rec.phase)
            .cmp(&rank(b_rec.phase))
            .then_with(|| a_task.title.cmp(&b_task.title))
    });
    rows.into_iter()
        .map(|(task, record)| {
            let name = match &task.key {
                Some(key) => SharedString::from(format!("{key} — {}", task.title)),
                None => SharedString::from(task.title.clone()),
            };
            let label = SharedString::from(format!("{term} {name} · {}", record.phase.label()));
            MissionRow {
                id: task.id,
                dim: matches!(record.phase, Phase::Completed | Phase::Abandoned),
                dot: mission_phase_colour(record.phase),
                name,
                label,
            }
        })
        .collect()
}

/// What a mission's phase reads as — the path colour while it is being walked, success once it
/// is finished, and the muted token for one nobody is taking further.
fn mission_phase_colour(phase: Phase) -> Rgba {
    match phase {
        Phase::Requirements | Phase::Refining => theme::info(),
        Phase::InProgress => theme::accent(),
        Phase::Completed => theme::success(),
        Phase::Abandoned => theme::text_muted(),
    }
}

/// The strip over the columns: what is being looked for, and the way to add one.
///
/// Every filter on it clears. A label row with nothing lit is not filtering — so a board emptied
/// by a filter is always one click from being full again, and the control at the end does the
/// whole row at once. It is drawn only while something is being hidden: a reset with nothing to
/// reset is a button that lies.
fn toolbar(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };

    // The tags filter: several labels on at once, and a card has to carry every one that is lit —
    // the same set shape Teams' states filter is, so it is the same `kit::MultiPicker` rather than
    // a second row-of-chips implementation (`T-142`). Each row's dot is the swatch that label was
    // given, the way the pill it replaces carried it.
    let all_labels = work.labels();
    let view = cx.entity();
    let labels_lit: Vec<usize> = all_labels
        .iter()
        .enumerate()
        .filter(|(_, label)| board.is_label_on(&label.name))
        .map(|(ix, _)| ix)
        .collect();
    let tags = MultiPicker::new("board-labels", "all tags")
        .items(all_labels.iter().map(|label| label.name.clone()))
        .dots(
            all_labels
                .iter()
                .map(|label| theme::project_colour(label.colour)),
        )
        .selected(labels_lit)
        .open(app.workbench.open_menu == Some(MenuId::BoardLabels))
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::BoardLabels, cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        // The list is read again here, exactly as it was drawn — the rule every position-matched
        // menu in this window follows.
        .on_pick(indexed(&view, |this, index, _, cx| {
            let name = this
                .work(cx)
                .and_then(|work| work.labels().get(index).map(|label| label.name.clone()));
            if let Some(name) = name {
                this.toggle_board_label(&name, cx);
            }
        }));

    // The mission filter (M27): single choice, unlike the tags picker beside it — a task belongs
    // to at most one mission, so ticking two would mean OR while the labels picker means AND, and
    // a `kit::MultiPicker` here would read two ways in one toolbar.
    let mission_query = if app.workbench.open_menu == Some(MenuId::BoardMission) {
        app.picker_search.read(cx).value().trim().to_lowercase()
    } else {
        String::new()
    };
    let mission_rows = board_mission_rows(app, work, &mission_query, cx);
    let mission_items: Vec<SharedString> = std::iter::once(SharedString::from("All tasks"))
        .chain(mission_rows.iter().map(|row| row.label.clone()))
        .collect();
    let mission_dots: Vec<Option<Rgba>> = std::iter::once(None)
        .chain(mission_rows.iter().map(|row| Some(row.dot)))
        .collect();
    let mission_dim: Vec<usize> = mission_rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.dim)
        .map(|(ix, _)| ix + 1)
        .collect();
    let mission_selected = match board.mission {
        None => Some(0),
        Some(id) => mission_rows
            .iter()
            .position(|row| row.id == id)
            .map(|ix| ix + 1),
    };
    let mission_trigger = match board.mission {
        None => SharedString::from("all missions"),
        Some(id) => mission_rows
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.name.clone())
            .unwrap_or_else(|| SharedString::from("all missions")),
    };
    let mission_search_focused = app
        .picker_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let mut mission_picker = Picker::new("board-mission", mission_trigger)
        .style(PickerStyle::Chip)
        .items(mission_items)
        .dots(mission_dots)
        .dim(mission_dim)
        .open(app.workbench.open_menu == Some(MenuId::BoardMission))
        .search(&app.picker_search, mission_search_focused)
        .on_toggle(handler(&view, |this, window, cx| {
            this.open_board_mission_menu(window, cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        // The list is read again here, exactly as it was drawn — the rule every
        // position-matched menu in this window follows.
        .on_pick(indexed(&view, |this, index, _, cx| {
            if index == 0 {
                this.pick_board_mission(None, cx);
                return;
            }
            let query = this.picker_search.read(cx).value().trim().to_lowercase();
            let id = this.work(cx).and_then(|work| {
                board_mission_rows(this, work, &query, cx)
                    .get(index - 1)
                    .map(|row| row.id)
            });
            if let Some(id) = id {
                this.pick_board_mission(Some(id), cx);
            }
        }));
    if let Some(ix) = mission_selected {
        mission_picker = mission_picker.selected(ix);
    }

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
                .items_center()
                .gap_2()
                .child(tags)
                .child(mission_picker),
        )
        .child(toggle_pill(
            "board-ready-only",
            "Ready only",
            theme::warning(),
            board.ready_only,
            cx.listener(|this, _, _, cx| this.toggle_board_ready_only(cx)),
        ))
        .children(board.filtering().then(|| {
            ghost_button(
                "board-show-all",
                None,
                "Show everything",
                cx.listener(|this, _, _, cx| this.clear_board_filters(cx)),
            )
        }))
        .child(icon_button(
            "board-popup-toggle",
            IconName::Maximize,
            board.popup,
            cx.listener(|this, _, _, cx| this.toggle_board_popup(cx)),
        ))
        // Straight to the form, the way Teams' own `+ Add agent` goes — the `+` menu's second row
        // offers conversations to attach to a surface, and this board draws no agent to attach one
        // to. The aim is `NewAgentSurface::Chat` (`T-109`): what it starts opens as a chat tab in
        // the right dock, beside this board, rather than jumping the window to the agents screen.
        .child(ghost_button(
            "board-new-agent",
            Some(IconName::Plus),
            "New agent",
            cx.listener(|this, _, window, cx| this.open_new_agent_direct(window, cx)),
        ))
        // The mission dialog's own entry point, beside the ordinary ways a task is made — labelled
        // with the project's own word for a mission, the same reading `ui::board::detail` already
        // gives the level chip.
        .child(ghost_button(
            "board-new-mission",
            Some(IconName::Plus),
            format!("New {}", app.mission_term(cx).to_lowercase()),
            cx.listener(|this, _, window, cx| this.open_new_mission(window, cx)),
        ))
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
    // Which lanes there are is settled before any of them is built: `column` takes the context
    // mutably, so the question cannot be asked inside the same iterator that answers it.
    let drawn: Vec<Status> = Status::all()
        .into_iter()
        .filter(|status| app.lane_drawn(*status, cx))
        .collect();
    div()
        .id("board-columns")
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .p_3()
        .gap_2()
        .overflow_x_scroll()
        // A lane the project has hidden is not drawn at all. It still holds whatever work is in
        // it — hiding is about the board, not about the tasks — which is why nothing is filtered
        // here beyond the column itself.
        .children(drawn.into_iter().map(|status| column(app, status, cx)))
}

fn column(app: &AppState, status: Status, cx: &mut Context<AppState>) -> AnyElement {
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };
    let tasks = board.column(work, status);
    let count = tasks.len();
    // Shut by hand, or shut because the project asked this lane to shut itself when it is empty.
    let shut = app.lane_shut(status, count, cx);
    let lit = board.carry.is_some_and(|carry| carry.over == Some(status));
    let colour = status_colour(status);
    // The three ids on a column key off the enum's discriminant rather than an id: a column is one
    // of the stages, not a record, so there is nothing here for a ULID to name.
    let key = status as u32;

    let mut root = div()
        .id(("board-column", key))
        .w(px(if shut {
            theme::column_shut()
        } else {
            theme::column_width()
        }))
        .flex()
        .when(shut, |this| this.flex_none())
        .when(!shut, |this| this.flex_1().min_w(px(160.)))
        .flex_col()
        .min_h(px(0.))
        .bg(theme::pane_bg())
        .border_l(px(theme::accent_edge()))
        .border_color(if lit { theme::accent() } else { colour })
        // The column a drop would file the card into says so by lighting up, which is the only
        // answer the user gets before letting go.
        .when(lit, |this| this.bg(theme::accent_soft()))
        // Entering the column names the column, not a place in it. A card overwrites `before`
        // with the gap it is over; the space under the cards claims the end. Resetting `before`
        // here on every move is what put every drop at the bottom of the lane.
        .on_drag_move(
            cx.listener(move |this, event: &DragMoveEvent<Dragged>, _, cx| {
                if event.bounds.contains(&event.event.position) {
                    this.drag_task_column(status, cx);
                }
            }),
        )
        .on_drop(cx.listener(move |this, _: &Dragged, _, cx| {
            let before = this.board(cx).and_then(|board| board.carry?.before);
            this.drop_task(status, before, cx);
        }));

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

    // Flattened once, up front: a bar where a drop would land, one row per task, then the tail —
    // so the list below has a single index space and never has to lay a card out just to learn
    // it scrolled off screen. See `T-108`: at 80+ cards a lane built every one of them on every
    // frame, and this is what made collapsing a lane the only way back to a responsive board.
    let mut rows: Vec<Row> = Vec::with_capacity(ids.len() + 2);
    for (ix, id) in ids.iter().enumerate() {
        if gap == Some(Some(*id)) {
            rows.push(Row::Marker);
        }
        // Which card a drop past this one's midpoint lands in front of. Past the last card there
        // is none, which is the end of the column.
        rows.push(Row::Card(*id, ids.get(ix + 1).copied()));
    }
    if gap == Some(None) {
        rows.push(Row::Marker);
    }
    rows.push(Row::Tail);

    // The lane's own list state, kept across renders — see `BoardState::lane_list`. Only what is
    // between `logical_scroll_top` and the bottom of the viewport (plus overdraw) is ever handed
    // to `render_row` below; everything else contributes its cached height and nothing more.
    let list_state = board.lane_list(status, rows.len());
    let view = cx.entity();

    let body = if empty {
        div()
            .id(("board-column-body", key))
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
            .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| cx.stop_propagation()))
            // `gpui::list` sizes itself to its rows' measured height, not the flex space it is
            // given (`T-108`'s change, see `column_tail`), so on a lane shorter than its column
            // this body div's own bounds are the only ones reaching the bottom of the lane —
            // `list`'s bounds stop where its content does. `column_tail`, a row inside the list,
            // still answers for the strip right under the last card; this answers for everything
            // below that, which is what restores the old flex_1 tail's full-height drop target
            // (`T-131`). A card or `column_tail` claims the pointer first and stops propagation,
            // so this only fires once the pointer is past all of them.
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<Dragged>, _, cx| {
                    if event.bounds.contains(&event.event.position) {
                        this.drag_task_over(status, None, cx);
                    }
                }),
            )
            .on_drop(cx.listener(move |this, _: &Dragged, _, cx| {
                cx.stop_propagation();
                this.drop_task(status, None, cx);
            }))
            .child(
                list(list_state, move |ix, window, cx| {
                    render_row(&rows, ix, status, &view, window, cx)
                })
                .flex_1()
                .min_h(px(0.)),
            )
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

/// One row inside a lane's virtualized list: a marker bar, a card by id, or the drop space under
/// them. Flattened once by [`column`] into one index space, which is what lets `gpui::list` below
/// ask for a row by number without knowing anything about drop gaps or task order.
#[derive(Clone, Copy)]
enum Row {
    Marker,
    /// A task's id, and the id of the card a drop past its midpoint lands in front of.
    Card(TaskId, Option<TaskId>),
    Tail,
}

/// The one row `gpui::list` asked for, at the index it asked for. Kept for the life of the lane's
/// `ListState` rather than one render, so nothing here borrows a particular frame's `AppState` —
/// everything it needs is looked up fresh off `view`.
///
/// `spaced` puts back the gap the old, non-virtualized column drew with `gap_2` on its flex
/// container: a fixed-stack list has no gap of its own, so every row but the last carries its own.
///
/// **Padding, not margin.** `gpui::list` measures a row by calling `element.layout_as_root`, which
/// reports the row's own border-box size — a root element's margin never enters that box, so a
/// margin-bottom here is silently dropped and every card ends up flush against the next (`T-132`).
/// Padding is part of the border box, so it is what the list actually counts.
fn render_row(
    rows: &[Row],
    ix: usize,
    status: Status,
    view: &Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(row) = rows.get(ix).copied() else {
        return div().into_any_element();
    };
    let last = ix + 1 == rows.len();
    let spaced = |el: AnyElement| -> AnyElement {
        if last {
            el
        } else {
            div().pb(rems(0.5)).child(el).into_any_element()
        }
    };
    match row {
        Row::Marker => spaced(marker()),
        Row::Tail => column_tail(status, view, window),
        Row::Card(id, next) => {
            let app = view.read(cx);
            let Some(work) = app.work(cx) else {
                return div().into_any_element();
            };
            match work.task(id) {
                Some(task) => spaced(task_card(app, task, next, view, window, cx)),
                None => div().into_any_element(),
            }
        }
    }
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

/// The strip right under the last card: a drop here is the end of the column.
fn column_tail(status: Status, view: &Entity<AppState>, window: &Window) -> AnyElement {
    // `gpui::list` measures every row at its intrinsic height rather than flexing it against the
    // column's remaining space — `flex_1` did that in the old, non-virtualized column, but has
    // nothing to answer to here — so this claims a fixed strip rather than the rest of the lane.
    // That is fine: it is a row *inside* the list, so its own bounds are real regardless of the
    // lane's height. The list's own bounds are not — `list` sizes itself to its rows' measured
    // height, not the flex space it is handed, so on a lane shorter than its column there is real
    // empty space below this row that belongs to no row at all. `column`'s body div covers that
    // (`T-131`): its own bounds always reach the bottom of the lane, so it is where the "rest of
    // the lane" drop target now lives. A lane that ends well above the bottom of the column is
    // `T-108`'s empty-column case, which the `Nothing here.` branch already draws.
    div()
        .id(("board-column-tail", status as u32))
        .min_h(px(40.))
        .w_full()
        .on_drag_move(window.listener_for(
            view,
            move |this, event: &DragMoveEvent<Dragged>, _, cx| {
                if event.bounds.contains(&event.event.position) {
                    this.drag_task_over(status, None, cx);
                }
            },
        ))
        .on_drop(window.listener_for(view, move |this, _: &Dragged, _, cx| {
            cx.stop_propagation();
            this.drop_task(status, None, cx);
        }))
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

/// A not-ready card's mark — M20's derived readiness, `TaskRecord::waiting_on`, not
/// `Status::Blocked`: a card can be both, and this carries no opinion about the other. Muted
/// (`warning`/`warning_soft`) and gains no pulse of its own — the left edge still owns the
/// card's one colour statement. `kit::tag`, whose click the card's own click already covers: the
/// tooltip names what it waits on, and opening the task shows the same keys in full on its
/// Prerequisites fact.
fn waits_on_chip(id: TaskId, waiting: &[TaskId], work: &work::WorkProjection) -> AnyElement {
    let keys: Vec<String> = waiting
        .iter()
        .filter_map(|wid| work.task(*wid))
        .map(|task| task.key.clone().unwrap_or_else(|| task.title.clone()))
        .collect();
    tag(
        eid("board-task-waits-on", id),
        format!("waits on {}", waiting.len()),
        format!("waits on {}", keys.join(", ")),
        theme::warning_soft(),
        theme::warning(),
        theme::warning(),
        false,
        |_, _, cx| cx.stop_propagation(),
    )
    .into_any_element()
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
    view: &Entity<AppState>,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let (Some(work), Some(board)) = (app.work(cx), app.board(cx)) else {
        return div().into_any_element();
    };
    let id = task.id;
    let colour = task
        .colour
        .map(theme::project_colour)
        .unwrap_or_else(|| bucket_colour(work.pulse(task)));
    let selected = board.selected == Some(id) && board.show_detail;
    let folded = board.is_folded(id);
    let carried = board.carry.is_some_and(|carry| carry.task == id);
    // Not-ready, M20's derived readiness — not `Status::Blocked`, which is what a person says
    // rather than what the prerequisite graph says. A card can carry both marks at once.
    let waiting = task.waiting_on(&work.tasks);
    // A drop the host has not answered yet. The card goes muted rather than moving, because the
    // column it is in is the host's answer and this one has not arrived.
    let moving = board.is_moving(id);
    let title = SharedString::from(task.title.clone());
    let ghost = title.clone();

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
                // A mission leads the row, in the accent colour and the project's own word for it,
                // so it reads as a mission before anything else on the card is read.
                .children(
                    (task.level == Some(Level::Mission))
                        .then(|| chip(app.mission_term(cx), theme::accent())),
                )
                // How many children it has, beside the mission chip that makes it eligible to
                // have any — a mission with none draws no chip, the same as every other mark
                // here that has nothing to say.
                .children({
                    let count = work.child_count(id);
                    (count > 0).then(|| chip(format!("{count}"), theme::text_muted()))
                })
                // Not ready, ahead of the labels: what a person reads first is what nobody can
                // start on yet.
                .children((!waiting.is_empty()).then(|| waits_on_chip(id, &waiting, &work)))
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
                        .on_click(window.listener_for(view, move |this, _, _, cx| {
                            this.toggle_task_fold(id, cx)
                        })),
                ),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(if waiting.is_empty() {
                    theme::text()
                } else {
                    theme::text_muted()
                })
                .child(title),
        )
        .children(shape_line(app, task, view, window, cx));

    if !folded {
        if !task.steps.is_empty() {
            root = root.child(meter(work::fraction(task), colour));
        }
        root = root.child(now_line(app, task, view, window, cx));
        root = root.children(comment_mark(task));
    }

    let status = task.status;

    root.on_click(window.listener_for(view, move |this, _, _, cx| this.select_task(id, cx)))
        // Above the midpoint is the gap in front of this card, below it the gap behind — which is
        // in front of the next one, or the end of the column when there is none. The card answers
        // before the column does, so the pointer is never told only which column it is in.
        .on_drag_move(window.listener_for(
            view,
            move |this, event: &DragMoveEvent<Dragged>, _, cx| {
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
            },
        ))
        // A drop carries no bounds, and the pointer has not moved since the last drag-move, so the
        // gap that one settled on is the gap this lands in. Without this the drop would fall
        // through to the column and mean the end of it.
        .on_drop(window.listener_for(view, move |this, _: &Dragged, _, cx| {
            cx.stop_propagation();
            let before = this.board(cx).and_then(|board| board.carry?.before);
            this.drop_task(status, before, cx);
        }))
        .on_drag(Dragged(id, ghost.clone()), {
            let view = view.clone();
            move |_, _, _, cx: &mut App| {
                let ghost = ghost.clone();
                view.update(cx, |this, cx| this.start_task_carry(id, cx));
                cx.new(|_| Ghost(ghost))
            }
        })
        .into_any_element()
}

/// How the agents on the task are arranged, whose work it is, and the issue it stands for.
///
/// Three optional facts on one line, and **no line at all** when none of them is set. A card draws
/// what it has: a row saying a task has no shape, no session and no link would take the same space
/// as one saying it has all three, and say nothing. Which session a task belongs to is offered in
/// the panel's picker, which is where a task is handed back to nobody.
fn shape_line(
    app: &AppState,
    task: &TaskRecord,
    view: &Entity<AppState>,
    window: &Window,
    cx: &App,
) -> Option<AnyElement> {
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
            .children(link.map(|url| link_chip(task.id, url, view, window)))
            .into_any_element(),
    )
}

/// The issue the task stands for, as the tracker's glyph and its name.
///
/// Clicking it hands the URL to the operating system, the same thing a link in a rendered
/// description does — the interface has no browser of its own to open it in.
fn link_chip(
    task: TaskId,
    url: String,
    view: &Entity<AppState>,
    window: &Window,
) -> impl IntoElement {
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
        .on_click(window.listener_for(view, move |_, _, _, cx| {
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

/// How many comments the task has, as a balloon and a count. Absent when there are none, so a
/// card without comments does not spend a row saying so.
fn comment_mark(task: &TaskRecord) -> Option<AnyElement> {
    let count = task.comments.len();
    if count == 0 {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(
                Icon::new(UbiqIcon::BoardComment)
                    .with_size(Size::XSmall)
                    .text_color(theme::text_faint()),
            )
            .child(
                mono(format!("{count}"), theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            )
            .into_any_element(),
    )
}

/// The bottom line of a card: the agent holding the task and what it is saying, or — when nobody
/// is — how many sub-tasks there are to be done.
fn now_line(
    app: &AppState,
    task: &TaskRecord,
    view: &Entity<AppState>,
    window: &Window,
    cx: &App,
) -> AnyElement {
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
                .on_click(
                    window.listener_for(view, move |this, _, _, cx| this.open_task_chat(id, cx)),
                ),
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
