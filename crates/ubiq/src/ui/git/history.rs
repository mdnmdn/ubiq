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
//! The commits are a fixture until the git family carries a log — `G70`.

use gpui::{
    AnyElement, Context, Focusable, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::Input;

use crate::app::AppState;
use crate::state::git::{COMMIT_ROW, CommitRow, LANE_GUTTER, LANE_PITCH};
use crate::theme;
use crate::ui::eid;
use crate::ui::kit::{elided, filter_bar, ghost_button, mono, panel, pill, toggle_pill};

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let visible = git.visible_commits();
    let focused = app.git_search.read(cx).focus_handle(cx).is_focused(window);

    let mut list = div()
        .id("git-history")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .child(uncommitted_row(app, cx));

    for (index, commit) in visible.iter() {
        list = list.child(commit_row(
            *index,
            commit,
            git.selected_commit == Some(*index),
            git.lanes(),
            cx,
        ));
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
                            .text_size(px(11.)),
                        ),
                ),
        )
        .child(list)
        .into_any_element()
}

/// The working tree, at the top of the log. Selected is what the panel beside the history is
/// about, so this row and a commit row are the same choice.
fn uncommitted_row(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let selected = app
        .git_view(cx)
        .map(|git| git.selected_commit.is_none())
        .unwrap_or(false);
    let changed = app
        .git_entries(cx)
        .map(|entries| entries.len())
        .unwrap_or(0);

    row_base("git-commit-uncommitted", selected)
        .child(div().w(px(LANE_GUTTER)).flex_none())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.5))
                .text_color(theme::text())
                .child("Uncommitted changes"),
        )
        .child(mono(format!("{changed} paths"), theme::text_muted()).text_size(px(11.)))
        .child(
            div()
                .w(px(78.))
                .flex_none()
                .child(mono("now", theme::text_faint()).text_size(px(11.))),
        )
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
    cx: &mut Context<AppState>,
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
                    pill(theme::accent())
                        .h(px(16.))
                        .px_1()
                        .child(mono(name.clone(), theme::text()).text_size(px(10.5)))
                })),
        )
        .child(elided(
            eid("git-commit-summary", index),
            commit.summary.clone(),
            theme::text(),
            12.5,
        ))
        .child(
            div()
                .w(px(150.))
                .flex_none()
                .text_size(px(11.5))
                .text_color(theme::text_muted())
                .truncate()
                .child(commit.author.clone()),
        )
        .child(
            div()
                .w(px(78.))
                .flex_none()
                .child(mono(commit.when.clone(), theme::text_faint()).text_size(px(11.))),
        )
        .child(
            div()
                .w(px(70.))
                .flex_none()
                .child(mono(commit.short_id.clone(), theme::text_muted()).text_size(px(11.))),
        )
        .on_click(cx.listener(move |this, _, _, cx| this.select_git_commit(Some(index), cx)))
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
    let width = (lanes as f32 * LANE_PITCH).max(LANE_PITCH);

    div()
        .w(px(LANE_GUTTER.max(width)))
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
