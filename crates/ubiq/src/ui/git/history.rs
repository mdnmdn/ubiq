//! The history: a search over the log, the graph's lanes, and one row per commit.
//!
//! **The uncommitted row is a commit row.** What has not been committed sits at the top of the
//! same list and is selected the same way, because "what have I got that is not in yet" is the
//! first question asked of a history and putting it somewhere else would make it the one thing on
//! the screen that is not where it belongs. It is the row the screen opens on.
//!
//! The graph is drawn from what a row carries — its lane, and the lanes that join it — rather than
//! computed here: the interface was not given a topology and does not invent one. Lanes are
//! stacked divs rather than a painted layer, because a lane is a straight line and a straight line
//! is not worth a canvas.
//!
//! The commits are the host's: `state::git::commit_rows` turns a `GitLogPage` reply into what
//! this module draws.

use gpui::{
    AnyElement, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px, uniform_list,
};
use gpui_component::input::Input;

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::git::{COMMIT_ROW, CommitRow, LANE_GUTTER, LANE_PITCH, RefSection};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::eid;
use crate::ui::kit::{
    Picker, PickerStyle, elided, filter_bar, ghost_button, mono, panel, pill, toggle_pill,
};
use crate::ui::{handler, indexed};

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let visible = git.visible_commits();
    let shown = visible.len();
    let focused = app.git_search.read(cx).focus_handle(cx).is_focused(window);

    // Only the indices cross into the list's own closure, which reads the rows themselves back out
    // of the view when it builds the handful that are on screen.
    let rows: Vec<usize> = visible.iter().map(|(index, _)| *index).collect();
    let lanes = git.lanes();
    let view = cx.entity();

    let mut list = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(uncommitted_row(app, lanes, cx))
        .child(
            uniform_list("git-history", shown, move |range, window, cx| {
                let Some(git) = view.read(cx).git_view(cx) else {
                    return Vec::new();
                };
                range
                    .filter_map(|slot| {
                        let index = *rows.get(slot)?;
                        let commit = git.commits.get(index)?;
                        Some(commit_row(
                            index,
                            commit,
                            git.selected_commit == Some(index),
                            lanes,
                            &view,
                            window,
                        ))
                    })
                    .collect::<Vec<AnyElement>>()
            })
            .track_scroll(&app.git_scroll)
            .flex_1()
            .min_h(px(0.)),
        );

    if !git.log_done {
        list = list.child(load_more_row(git.log_inflight.is_some(), cx));
    }

    panel()
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
                                format!("{} of {} commits", visible.len(), git.commits.len()),
                                theme::text_faint(),
                            )
                            .text_size(theme::font(Family::Chrome, Role::Meta)),
                        ),
                ),
        )
        .child(list)
        .into_any_element()
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
        .child(div().w(px(78.)).flex_none().child(
            mono("now", theme::text_faint()).text_size(theme::font(Family::Chrome, Role::Meta)),
        ))
        .child(div().w(px(70.)).flex_none())
        .on_click(cx.listener(|this, _, _, cx| this.select_git_commit(None, cx)))
        .into_any_element()
}

/// One commit: its lanes, whatever points at it, its summary, who wrote it, when, and its
/// abbreviated id.
fn commit_row(
    index: usize,
    commit: &CommitRow,
    selected: bool,
    lanes: usize,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    row_base(eid("git-commit", index), selected)
        .child(lane_gutter(commit, lanes))
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
                .w(px(150.))
                .flex_none()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_muted())
                .truncate()
                .child(commit.author.clone()),
        )
        .child(
            div().w(px(78.)).flex_none().child(
                mono(commit.when.clone(), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            ),
        )
        .child(
            div().w(px(70.)).flex_none().child(
                mono(commit.short_id.clone(), theme::text_muted())
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
            ),
        )
        .on_click(window.listener_for(view, move |this, _, _, cx| {
            this.select_git_commit(Some(index), cx)
        }))
        .into_any_element()
}

/// The trigger for the next page, at the bottom of the list — or its own loading state while that
/// page is in flight. Absent once `GitView::log_done` says there is nothing more.
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
        .hover(|this| this.bg(theme::hover()));

    if selected {
        row = row
            .bg(theme::accent_soft())
            .border_l_2()
            .border_color(theme::accent());
    }
    row
}

/// The graph beside one row: a hairline for every lane the history is that wide, and this commit's
/// own dot in the middle of its lane.
///
/// A lane cell is a column — line, dot, line — rather than a dot drawn inside a one-pixel line,
/// so the dot is laid out rather than overflowing what it sits in. A commit that something merges
/// into is drawn hollow, which is the one thing a lane says about a topology it did not compute.
fn lane_gutter(commit: &CommitRow, lanes: usize) -> AnyElement {
    div()
        .w(px(gutter_width(lanes)))
        .h_full()
        .flex()
        .flex_none()
        .items_center()
        .children((0..lanes).map(|lane| {
            let colour = lane_colour(lane);
            let mut cell = div()
                .w(px(LANE_PITCH))
                .h_full()
                .flex()
                .flex_none()
                .flex_col()
                .items_center();

            if lane == commit.lane {
                cell = cell
                    .child(div().w(px(1.)).flex_1().bg(colour))
                    .child(
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(if commit.merges.is_empty() {
                                colour
                            } else {
                                theme::pane_bg()
                            })
                            .border_1()
                            .border_color(colour),
                    )
                    .child(div().w(px(1.)).flex_1().bg(colour));
            } else {
                cell = cell.child(div().w(px(1.)).h_full().bg(colour));
            }
            cell
        }))
        .into_any_element()
}

/// How wide the graph's gutter draws for a given lane count — the minimum a real, multi-lane
/// history needs, never narrower than one lane's pitch. Shared by every row so the uncommitted
/// row's own placeholder gutter lines up with the commit rows below it.
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
