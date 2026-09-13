//! The panel beside the history: what the selected row is about.
//!
//! On the uncommitted row — the one the screen opens on — that is the working tree: the conflicted
//! paths first, then modified, then untracked, and the commit box under them. On a commit it is
//! what the log said about that commit, and nothing more: a commit's own file list needs the log
//! family, and inventing one here would be the one thing on this screen that is not true.
//!
//! **The lists are the host's answer.** Each path appears once — conflicted, modified or
//! untracked. `+` stages and `-` unstages; the letter is the worktree change when there is one,
//! otherwise the index change. The colour is the explorer's, so a path reads the same in both
//! places.
//!
//! **The commit box writes.** It commits when there is a message and something staged (or amend),
//! and draws the last write error under the button when there is one.

use gpui::{
    AnyElement, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px, uniform_list,
};
use gpui_component::input::Textarea;
use ubiq_proto::git::{GitEntry, GitPathChange};

use crate::app::AppState;
use crate::state::GitStatus;
use crate::state::git::{Side, can_stage, can_unstage, change_letter, group_changes, staged};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::explorer::git_colour;
use crate::ui::kit::{
    badge, check_box, elided_with, field, mono, panel, panel_header, primary_button, section_label,
};

/// Every row in the list is this tall, a list heading included, so the list is uniform and only
/// what is on screen is built.
const ROW: f32 = 24.0;
/// Click target for the stage / unstage glyphs.
const ACTION: f32 = 18.0;

/// One row of the flattened working tree: a list's heading, or one changed path in it as an index
/// into the project's entries.
enum Flat {
    Header(Side, usize),
    Row(Side, usize),
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };

    match git.selected_commit {
        None => working_tree(app, window, cx),
        Some(index) => commit(app, index, cx),
    }
}

/// The working tree: three lists and the box under them.
fn working_tree(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    if app.git_view(cx).is_none() {
        return div().into_any_element();
    }
    let entries = app.git_entries(cx).unwrap_or(&[]);
    let groups = group_changes(entries);
    let staged_count = staged(entries).len();

    // The three lists become one, so the panel is a single virtual list: only the rows on screen
    // are built, and a heading is a row like any other.
    let mut rows: Vec<Flat> = Vec::new();
    for (side, group) in [
        (Side::Conflicted, &groups.conflicted),
        (Side::Modified, &groups.modified),
        (Side::Untracked, &groups.untracked),
    ] {
        if group.is_empty() {
            continue;
        }
        rows.push(Flat::Header(side, group.len()));
        rows.extend(group.iter().map(|index| Flat::Row(side, *index)));
    }

    let count = rows.len();
    let view = cx.entity();
    let mut body = div().flex().flex_col().flex_1().min_h(px(0.)).child(
        uniform_list("git-changes", count, move |range, window, cx| {
            let app = view.read(cx);
            let entries = app.git_entries(cx).unwrap_or(&[]);
            let selected = app.git_view(cx).and_then(|git| git.path());
            range
                .filter_map(|slot| match rows.get(slot)? {
                    Flat::Header(side, count) => {
                        Some(list_header(*side, *count).into_any_element())
                    }
                    Flat::Row(side, index) => {
                        let entry = entries.get(*index)?;
                        Some(change_row(
                            *side,
                            entry,
                            selected == Some(&entry.rel_path),
                            &view,
                            window,
                        ))
                    }
                })
                .collect::<Vec<AnyElement>>()
        })
        .flex_1()
        .min_h(px(0.)),
    );

    // A working tree with nothing to say is a fact worth printing: it is the difference between
    // clean and not yet read, and only one of the two is worth being pleased about.
    if entries.is_empty() {
        body = body.child(div().px_3().py_2().child(mono(
            match app.open_project(cx).and_then(|open| open.git.as_ref()) {
                Some(_) => "Nothing to commit",
                None => "Not a repository",
            },
            theme::text_faint(),
        )));
    }

    panel()
        .child(panel_header(
            "Uncommitted changes",
            mono(format!("{} paths", entries.len()), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        ))
        .child(body)
        .child(commit_box(app, window, staged_count, cx))
        .into_any_element()
}

/// What the log said about the selected commit.
fn commit(app: &AppState, index: usize, cx: &mut Context<AppState>) -> AnyElement {
    let Some(commit) = app.git_view(cx).and_then(|git| git.commits.get(index)) else {
        return div().into_any_element();
    };

    panel()
        .child(panel_header(
            "Commit",
            mono(commit.short_id.clone(), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        ))
        .child(
            div()
                .px_3()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text())
                        .child(commit.summary.clone()),
                )
                .child(mono(
                    format!("{} \u{b7} {}", commit.author, commit.when),
                    theme::text_muted(),
                ))
                .children(
                    (!commit.refs.is_empty())
                        .then(|| mono(commit.refs.join(" \u{b7} "), theme::accent())),
                )
                .child(
                    mono(
                        "The files a commit touched need the log the git family does not carry yet",
                        theme::text_faint(),
                    )
                    .text_size(theme::font(Family::Chrome, Role::Meta)),
                ),
        )
        .into_any_element()
}

/// One list's heading.
fn list_header(side: Side, count: usize) -> impl IntoElement {
    div()
        .h(px(ROW))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(section_label(side.label()))
        .child(
            mono(format!("{count}"), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        )
}

/// One changed path. The letter is the worktree change when there is one, otherwise the index
/// change; conflicted rows say `!`. Non-conflicted rows carry right-justified `+` / `-`.
fn change_row(
    side: Side,
    entry: &GitEntry,
    selected: bool,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let change = entry.worktree.as_ref().or(entry.index.as_ref());
    let letter = match side {
        Side::Conflicted => "!",
        _ => change.map(change_letter).unwrap_or(" "),
    };
    let colour = row_colour(entry);
    let path = entry.rel_path.clone();
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let key = crate::ui::eid("git-change", &path);
    let stageable = can_stage(entry);
    let unstageable = can_unstage(entry);

    let mut row = div()
        .id(key)
        .h(px(ROW))
        .pr_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(div().w(px(8.)).flex_none())
        .child(badge(letter, colour))
        .child(elided_with(
            crate::ui::eid("git-change-name", &path),
            name,
            path.clone(),
            theme::text_muted(),
            theme::font(theme::Family::Chrome, theme::Role::Label),
        ))
        .children(
            // A rename is the one change whose old name is worth the width: the row's own name is
            // where the file went, and the pair says where it came from.
            match change {
                Some(GitPathChange::Renamed { from }) => Some(
                    mono(format!("\u{2190} {from}"), theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Micro)),
                ),
                _ => None,
            },
        );

    if side != Side::Conflicted {
        row = row
            .child(action_glyph(
                crate::ui::eid("git-stage", &path),
                "+",
                stageable,
                path.clone(),
                true,
                view,
                window,
            ))
            .child(action_glyph(
                crate::ui::eid("git-unstage", &path),
                "-",
                unstageable,
                path.clone(),
                false,
                view,
                window,
            ));
    }

    if selected {
        row = row
            .bg(theme::accent_soft())
            .border_l_2()
            .border_color(theme::accent());
    }

    row.on_click(window.listener_for(view, move |this, _, _, cx| {
        this.select_git_path(side, &path, cx)
    }))
    .into_any_element()
}

/// An 18px `+` or `-`. Enabled glyphs stage or unstage; disabled ones take no click.
fn action_glyph(
    id: impl Into<gpui::ElementId>,
    glyph: &'static str,
    enabled: bool,
    path: String,
    stage: bool,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let colour = if enabled {
        theme::text()
    } else {
        theme::text_faint()
    };
    let mut btn = div()
        .id(id)
        .size(px(ACTION))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .child(mono(glyph, colour));
    if enabled {
        btn = btn
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .on_click(window.listener_for(view, move |this, _, _, cx| {
                cx.stop_propagation();
                if stage {
                    this.stage_git_path(&path, cx);
                } else {
                    this.unstage_git_path(&path, cx);
                }
            }));
    }
    btn.into_any_element()
}

/// The colour a changed path takes, which is the explorer's for the same path: both are the same
/// projection of the same pair.
fn row_colour(entry: &GitEntry) -> Rgba {
    git_colour(entry.mark().map(GitStatus::from_mark))
}

/// The commit box: a message, whether it amends, and the button that commits.
fn commit_box(
    app: &AppState,
    window: &Window,
    staged_count: usize,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(git) = app.git_view(cx) else {
        return div();
    };
    let focused = app.git_message.read(cx).focus_handle(cx).is_focused(window);
    let amend = git.amend;
    let message = app.git_message.read(cx).value();
    let can_commit = (staged_count > 0 || amend) && !message.trim().is_empty();
    let last_error = git.last_error.clone();
    let label = match staged_count {
        1 => "Commit 1 file".to_string(),
        n => format!("Commit {n} files"),
    };

    let commit_control: AnyElement = if can_commit {
        primary_button(
            "git-commit",
            None,
            label,
            cx.listener(|this, _, _, cx| this.commit_git(cx)),
        )
        .into_any_element()
    } else {
        div()
            .h(px(26.))
            .px_2p5()
            .flex()
            .flex_none()
            .items_center()
            .bg(theme::surface())
            .border_l(px(theme::accent_edge()))
            .border_color(theme::border())
            .text_size(theme::font(Family::Chrome, Role::Body))
            .text_color(theme::text_faint())
            .child(label)
            .into_any_element()
    };

    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap_2()
        .p_3()
        .border_t_1()
        .border_color(theme::border())
        .child(
            field(theme::accent(), focused)
                .flex_col()
                .items_stretch()
                .child(
                    div()
                        .id("git-commit-box")
                        .px_2()
                        .py_1p5()
                        .cursor_text()
                        .child(
                            Textarea::new(&app.git_message)
                                .appearance(false)
                                .bordered(false)
                                .w_full()
                                .text_size(theme::font(Family::Chrome, Role::Body)),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            let input = this.git_message.clone();
                            input.update(cx, |state, cx| state.focus(window, cx));
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(check_box(
                    "git-amend",
                    amend,
                    cx.listener(|this, _, _, cx| this.toggle_git_amend(cx)),
                ))
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Label))
                        .text_color(theme::text_muted())
                        .child("amend"),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(commit_control),
        )
        .children(last_error.map(|reason| {
            mono(reason, theme::danger()).text_size(theme::font(Family::Chrome, Role::Micro))
        }))
}
