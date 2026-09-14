//! The history: a search over the log, the graph's lanes, and one row per commit.
//!
//! **The uncommitted row is a commit row.** What has not been committed sits at the top of the
//! same list and is selected the same way, because "what have I got that is not in yet" is the
//! first question asked of a history and putting it somewhere else would make it the one thing on
//! the screen that is not where it belongs. It is the row the screen opens on.
//!
//! The graph is drawn from a row's own cell — the lanes alive above and below it, and the lanes
//! that end at its dot — as a painted layer, because once lines can turn: a merge's extra-parent
//! lane is born at the merge and an elbow joins two columns at a row, which is no longer a stack
//! of straight hairlines. The cells themselves come ready-made from `state::git::graph_cells`,
//! a projection of the host's lane data; the interface draws, it does not relayout.
//!
//! The commits are the host's: `state::git::commit_rows` turns a `GitLogPage` reply into what
//! this module draws.

use gpui::{
    AnyElement, ClickEvent, Context, Entity, Focusable, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, PathBuilder, Pixels, Point, Rgba,
    StatefulInteractiveElement, Styled, Window, canvas, div, point, px, uniform_list,
};
use gpui_component::input::Input;

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::git::{
    AUTHOR_COL, COMMIT_ROW, CommitRow, GitMenuKind, GraphCell, LANE_GUTTER, LANE_PITCH, RefSection,
    SHA_COL, WHEN_COL,
};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::eid;
use crate::ui::kit::{
    ContextItem, Picker, PickerStyle, context_menu, elided, filter_bar, ghost_button, mono, panel,
    pill, toggle_pill,
};
use crate::ui::{handler, indexed};

/// How close the rendered range has to come to the end of what is loaded before the next page is
/// asked for — a handful of rows of runway so the page lands before the user's scroll catches up
/// to it, not a wait for the last row to actually paint.
const NEAR_END: usize = 20;

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let history = git.history();
    let shown = history.len();
    let focused = app.git_search.read(cx).focus_handle(cx).is_focused(window);

    // Only the commit indices cross into the list's own closure, which reads the rows themselves
    // back out of the view when it builds the handful that are on screen.
    let rows: Vec<usize> = history.iter().map(|(index, _, _)| *index).collect();
    let lanes = git.lanes();
    let view = cx.entity();

    let mut list = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .children((!git.commits.is_empty()).then(|| header_row(lanes)))
        .child(uncommitted_row(app, lanes, cx))
        .child(
            uniform_list("git-history", shown, move |range, window, cx| {
                let Some(git) = view.read(cx).git_view(cx) else {
                    return Vec::new();
                };
                // Within `NEAR_END` rows of the bottom of what is loaded, with nothing already in
                // flight and more left to page in, the next page is worth asking for now rather
                // than waiting for a click the scrollbar's own travel already promised. The
                // request cannot be sent from here — this runs mid-render, with the view already
                // read above — so it is deferred, the same turn-later pattern every other
                // during-render app write in this dock takes.
                let near_end = range.end + NEAR_END >= rows.len();
                let should_load = near_end && git.log_inflight.is_none() && !git.log_done;
                let elements = range
                    .filter_map(|slot| {
                        let index = *rows.get(slot)?;
                        let commit = git.commits.get(index)?;
                        let cell = git.graph.get(index)?;
                        Some(commit_row(
                            index,
                            commit,
                            cell,
                            git.commit_selected(index),
                            lanes,
                            &view,
                            window,
                        ))
                    })
                    .collect::<Vec<AnyElement>>();
                if should_load {
                    let view = view.clone();
                    cx.defer(move |cx| {
                        view.update(cx, |this, cx| this.load_more_git_log(cx));
                    });
                }
                elements
            })
            .track_scroll(&app.git_scroll)
            .flex_1()
            .min_h(px(0.)),
        );

    if !git.log_done {
        list = list.child(load_more_row(git.log_inflight.is_some(), cx));
    }

    let menu = git.menu.clone();

    let mut root = panel()
        .flex_1()
        .child(
            div()
                .pt_2()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(div().flex_1().min_w(px(0.)).child(filter_bar(
                    Input::new(&app.git_search).appearance(false),
                    div(),
                    focused,
                )))
                .child(branch_picker(app, window, cx))
                .child(
                    div()
                        .pr_3()
                        .pb_1()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .child(toggle_pill(
                            "git-mine",
                            "my commits",
                            theme::accent(),
                            git.mine_only,
                            cx.listener(|this, _, _, cx| this.toggle_git_mine(cx)),
                        ))
                        .children(git.filtered().then(|| {
                            ghost_button(
                                "git-show-all",
                                None,
                                "Show everything",
                                cx.listener(|this, _, _, cx| this.clear_git_filters(cx)),
                            )
                        }))
                        .child(
                            mono(
                                format!("{} of {} commits", history.len(), git.commits.len()),
                                theme::text_faint(),
                            )
                            .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
                ),
        )
        .child(list);

    if let Some(menu) = menu
        && matches!(menu.kind, GitMenuKind::Commit { .. })
    {
        let epoch = menu.epoch;
        let items: Vec<ContextItem> = menu
            .entries(false, false)
            .into_iter()
            .map(|entry| {
                if entry.is_separator() {
                    return ContextItem::separator();
                }
                let item = ContextItem::new(entry.label());
                if entry.enabled { item } else { item.disabled() }
            })
            .collect();
        root = root.child(context_menu(
            "git-commit-menu",
            point(px(menu.x), px(menu.y)),
            items,
            indexed(&cx.entity(), |this, index, _, cx| {
                this.pick_git_menu_action(index, cx)
            }),
            handler(&cx.entity(), move |this, _, cx| {
                this.dismiss_git_menu(epoch, cx)
            }),
        ));
    }

    root.into_any_element()
}

/// Which ref the history is walking. `All branches` is HEAD; any other row re-asks the host for
/// that ref's log. The list is filtered as it is typed, the same split every searchable picker
/// follows.
fn branch_picker(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let names: Vec<String> = git
        .refs
        .iter()
        .filter(|row| matches!(row.section, RefSection::Local | RefSection::Remotes))
        .map(|row| row.name.clone())
        .collect();
    let current = git.branch_filter.clone();
    let needle = app.git_branch_query.read(cx).value().trim().to_lowercase();
    let focused = app
        .git_branch_query
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let mut items = vec!["All branches".to_string()];
    let mut ids: Vec<Option<String>> = vec![None];
    for name in names {
        if needle.is_empty() || name.to_lowercase().contains(&needle) {
            items.push(name.clone());
            ids.push(Some(name));
        }
    }
    let selected = match &current {
        None => 0,
        Some(name) => items.iter().position(|item| item == name).unwrap_or(0),
    };
    let label = current
        .clone()
        .unwrap_or_else(|| "All branches".to_string());
    let view = cx.entity();

    Picker::new("git-branch-pick", label)
        .items(items)
        .selected(selected)
        .style(PickerStyle::Chip)
        .search(&app.git_branch_query, focused)
        .open(app.workbench.open_menu == Some(MenuId::GitBranch))
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::GitBranch, cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        .on_pick(indexed(&view, move |this, index, _, cx| {
            let name = ids.get(index).cloned().flatten();
            this.set_git_branch_filter(name, cx);
        }))
        .into_any_element()
}

/// The working tree, at the top of the log. Selected is what the panel beside the history is
/// about, so this row and a commit row are the same choice. `lanes` is the graph's width, the same
/// count every commit row below it draws with, so a wide graph does not misalign this row's own
/// columns against theirs.
fn uncommitted_row(app: &AppState, lanes: usize, cx: &mut Context<AppState>) -> AnyElement {
    let selected = app
        .git_view(cx)
        .map(|git| git.selected_commit.is_none())
        .unwrap_or(false);
    let changed = app
        .git_entries(cx)
        .map(|entries| entries.len())
        .unwrap_or(0);

    row_base("git-commit-uncommitted", selected)
        .child(div().w(px(gutter_width(lanes))).flex_none())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .child("Uncommitted changes"),
        )
        .child(
            mono(format!("{changed} paths"), theme::text_muted())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        )
        .child(div().w(px(WHEN_COL)).flex_none().child(
            mono("now", theme::text_faint()).text_size(theme::font(Family::Chrome, Role::Meta)),
        ))
        .child(div().w(px(SHA_COL)).flex_none())
        .on_click(cx.listener(|this, _, _, cx| this.select_git_commit(None, cx)))
        .into_any_element()
}

/// One commit: its lane graph, whatever points at it, its summary, who wrote it, when, and its
/// abbreviated id. A plain click selects it; cmd/ctrl-click makes it the other end of a two-commit
/// comparison, drawn under the history once both ends are set. A right-click raises the row's
/// context menu.
fn commit_row(
    index: usize,
    commit: &CommitRow,
    cell: &GraphCell,
    selected: bool,
    lanes: usize,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    row_base(eid("git-commit", index), selected)
        .child(graph_gutter(commit, cell, lanes))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .children(commit.refs.iter().map(|name| {
                    pill(theme::accent()).h(px(16.)).px_1().child(
                        mono(name.clone(), theme::text())
                            .text_size(theme::font(Family::Chrome, Role::Micro)),
                    )
                })),
        )
        .child(elided(
            eid("git-commit-summary", index),
            commit.summary.clone(),
            theme::text(),
            theme::font(theme::Family::Chrome, theme::Role::Body),
        ))
        .child(
            div()
                .w(px(AUTHOR_COL))
                .flex_none()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_muted())
                .truncate()
                .child(commit.author.clone()),
        )
        .child(
            div().w(px(WHEN_COL)).flex_none().child(
                mono(commit.when.clone(), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            ),
        )
        .child(
            div().w(px(SHA_COL)).flex_none().child(
                mono(commit.short_id.clone(), theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            ),
        )
        .on_click(
            window.listener_for(view, move |this, event: &ClickEvent, _, cx| {
                if event.modifiers().platform {
                    this.toggle_git_compare(index, cx);
                } else {
                    this.select_git_commit(Some(index), cx);
                }
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            window.listener_for(view, move |this, event: &MouseDownEvent, _, cx| {
                this.open_git_menu(
                    GitMenuKind::Commit { index },
                    (f32::from(event.position.x), f32::from(event.position.y)),
                    cx,
                );
            }),
        )
        .into_any_element()
}

/// The footer under the list: a loading word while the next page the scroll already asked for is
/// in flight, or a manual fallback button for the rare frame nothing has scrolled near the end yet
/// — a history short enough to show past its own bottom without ever nearing it, say. Absent once
/// `GitView::log_done` says there is nothing more.
fn load_more_row(loading: bool, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .h(px(COMMIT_ROW))
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .children((!loading).then(|| {
            ghost_button(
                "git-load-more",
                None,
                "Load more commits",
                cx.listener(|this, _, _, cx| this.load_more_git_log(cx)),
            )
        }))
        .children(loading.then(|| {
            mono("Loading more\u{2026}", theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta))
        }))
        .into_any_element()
}

/// The shape every row in the list has: one line, selectable, marked on its left edge the way the
/// file lists mark theirs.
///
/// The left border is drawn on every row, transparent when not selected — the same 2px it becomes
/// on a selected row, so a selection never shifts the row's own content over by the border's
/// width the way an edge that only appears when picked would.
fn row_base(id: impl Into<gpui::ElementId>, selected: bool) -> gpui::Stateful<gpui::Div> {
    let mut row = div()
        .id(id)
        .h(px(COMMIT_ROW))
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .border_l_2()
        .border_color(theme::transparent())
        .hover(|this| this.bg(theme::hover()));

    if selected {
        row = row.bg(theme::accent_soft()).border_color(theme::accent());
    }
    row
}

/// The graph beside one row, painted as one canvas: a line per lane that is alive above this row,
/// below it, or turns at it, and this commit's own dot in its lane.
///
/// A lane line is drawn where it is real, not for every column of the gutter —
/// [`GraphCell::above`]`/below/joins` say which lanes exist around this row. A lane that *joins*
/// (an above-lane the host matched to this commit's id) comes down and elbows into the dot; a
/// lane this commit's merge *born* elbows out of the dot and continues down; a lane that is alive
/// on both sides passes straight through. The dot is drawn on top of both, and a merge's dot is a
/// hollow ring rather than a filled one — a lane does not say a topology only the host saw, but
/// the hollow dot is what the eye has always used.
fn graph_gutter(commit: &CommitRow, cell: &GraphCell, lanes: usize) -> AnyElement {
    let width = gutter_width(lanes);
    // The constants and the facts a row draws with, captured once — the canvas painter outlives
    // this call, so they are owned rather than borrowed.
    let pitch = LANE_PITCH;
    let cell_h = COMMIT_ROW;
    let dot = commit.lane;
    let merge = !commit.merges.is_empty();
    let merges = commit.merges.clone();
    let cell = cell.clone();
    let stroke = 1.5;
    let radius = 3.0;

    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let left = f32::from(bounds.origin.x);
            let top = f32::from(bounds.origin.y);
            let mid = top + cell_h / 2.0;
            let dot_x = left + dot as f32 * pitch + pitch / 2.0;
            let gap = radius + 1.0;
            let lane_x = |lane: usize| left + lane as f32 * pitch + pitch / 2.0;

            for lane in 0..lanes {
                let colour = lane_colour(lane);
                let x = lane_x(lane);
                let join = cell.joins.contains(&lane);
                let born = merges.contains(&lane);
                let has_above = cell.above.contains(&lane);
                let has_below = cell.below.contains(&lane);

                let mut builder = PathBuilder::stroke(px(stroke));
                let mut strokes = 0;
                if join || has_above {
                    let stop = if lane == dot { mid - gap } else { mid };
                    builder.move_to(point(px(x), px(top + 1.)));
                    builder.line_to(point(px(x), px(stop)));
                    strokes += 1;
                }
                if born || has_below {
                    let start = if lane == dot { mid + gap } else { mid };
                    builder.move_to(point(px(x), px(start)));
                    builder.line_to(point(px(x), px(top + cell_h)));
                    strokes += 1;
                }
                if strokes > 0
                    && let Ok(path) = builder.build()
                {
                    window.paint_path(path, colour);
                }
                if (join && lane != dot) || born {
                    let mut elbow = PathBuilder::stroke(px(stroke));
                    elbow.move_to(point(px(x), px(mid)));
                    elbow.line_to(point(px(dot_x), px(mid)));
                    if let Ok(path) = elbow.build() {
                        window.paint_path(path, colour);
                    }
                }
            }

            let dots: Vec<Point<Pixels>> = (0..20)
                .map(|step| {
                    let angle = std::f32::consts::TAU * step as f32 / 20.0;
                    point(
                        px(dot_x + radius * angle.cos()),
                        px(mid + radius * angle.sin()),
                    )
                })
                .collect();
            let colour = lane_colour(dot);
            if merge {
                let mut ring = PathBuilder::stroke(px(stroke));
                ring.add_polygon(&dots, true);
                if let Ok(path) = ring.build() {
                    window.paint_path(path, colour);
                }
            } else {
                let mut fill = PathBuilder::fill();
                fill.add_polygon(&dots, true);
                if let Ok(path) = fill.build() {
                    window.paint_path(path, colour);
                }
            }
        },
    )
    .w(px(width))
    .h_full()
    .flex_none()
    .into_any_element()
}

/// The column labels over the history: the same gutter, message, author, when and id columns the
/// rows draw with, each at its own fixed width, so the aligned table the rows form has a header
/// that names what a column is. `Message` takes the flexible rule, exactly as a row's summary
/// does.
fn header_row(lanes: usize) -> AnyElement {
    div()
        .h(px(COMMIT_ROW))
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .border_l_2()
        .border_color(theme::transparent())
        .child(div().w(px(gutter_width(lanes))).flex_none())
        .child(div().flex_1().min_w(px(0.)).child(header_label("Message")))
        .child(
            div()
                .w(px(AUTHOR_COL))
                .flex_none()
                .child(header_label("Author")),
        )
        .child(
            div()
                .w(px(WHEN_COL))
                .flex_none()
                .child(header_label("When")),
        )
        .child(div().w(px(SHA_COL)).flex_none().child(header_label("SHA")))
        .into_any_element()
}

/// One header label: the smallest chrome type in the footer's faint, matching the column values
/// it names without competing with the data beneath it.
fn header_label(text: &'static str) -> gpui::Div {
    div()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_faint())
        .child(text)
}

/// How wide the graph's gutter draws for a given lane count — the minimum a real, multi-lane
/// history needs, never narrower than one lane's pitch. Shared by every row so the header's and
/// the uncommitted row's own placeholder gutters line up with the commit rows below them.
fn gutter_width(lanes: usize) -> f32 {
    LANE_GUTTER.max(lanes as f32 * LANE_PITCH)
}

/// The colour a lane draws in. Four tokens, cycled — a lane is not a state, so it borrows the
/// palette rather than meaning anything by it.
fn lane_colour(lane: usize) -> Rgba {
    match lane % 4 {
        0 => theme::accent(),
        1 => theme::success(),
        2 => theme::info(),
        _ => theme::warning(),
    }
}
