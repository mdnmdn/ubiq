//! The panel beside the history: what the selected row is about.
//!
//! On the uncommitted row — the one the screen opens on — that is the working tree: the conflicted
//! paths first, then what is staged, then what is not, and the commit box under them. On a commit
//! it is what the log said about that commit, and nothing more: a commit's own file list needs the
//! log family, and inventing one here would be the one thing on this screen that is not true.
//!
//! **The lists are the host's answer.** Each row is one [`ubiq_proto::git::GitEntry`] — the pair,
//! not the projection — so a path both staged and modified appears in both lists, which is what
//! the pair is for and what a single badge on an explorer row cannot say. The letter is the change
//! on that side; the colour is the explorer's, so a path reads the same in both places.
//!
//! **Nothing here commits.** The box keeps what is typed so the thought is not lost, and the
//! button says why it cannot be pressed.

use gpui::{
    AnyElement, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement, Styled, Window, div, px, uniform_list,
};
use gpui_component::input::Textarea;
use ubiq_proto::git::{GitEntry, GitPathChange};

use crate::app::AppState;
use crate::state::GitStatus;
use crate::state::git::{Side, change_letter, group_changes};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::explorer::git_colour;
use crate::ui::kit::{
    badge, check_box, elided_with, field, mono, panel, panel_header, section_label,
};

/// Every row in the list is this tall, a list heading included, so the list is uniform and only
/// what is on screen is built.
const ROW: f32 = 24.0;

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
    // Grouped once here instead of filtered three times below, plus a fourth pass for the staged
    // count `commit_box` used to take on its own — `staged.len()` is that count.
    let groups = group_changes(entries);
    let staged_count = groups.staged.len();

    // The three lists become one, so the panel is a single virtual list: only the rows on screen
    // are built, and a heading is a row like any other.
    let mut rows: Vec<Flat> = Vec::new();
    for (side, group) in [
        (Side::Conflicted, &groups.conflicted),
        (Side::Staged, &groups.staged),
        (Side::Unstaged, &groups.unstaged),
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

/// One list's heading, with what a write version's bulk action would be beside it.
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

/// One changed path. The letter is the change on this list's own side of the pair; the colour is
/// the one the explorer paints the same path in.
fn change_row(
    side: Side,
    entry: &GitEntry,
    selected: bool,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let change = match side {
        Side::Staged => entry.index.as_ref(),
        Side::Unstaged | Side::Conflicted => entry.worktree.as_ref(),
    };
    let letter = match side {
        Side::Conflicted => "!",
        _ => change.map(change_letter).unwrap_or(" "),
    };
    let colour = row_colour(entry);
    let path = entry.rel_path.clone();
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let key = crate::ui::eid("git-change", &path);

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

/// The colour a changed path takes, which is the explorer's for the same path: both are the same
/// projection of the same pair.
fn row_colour(entry: &GitEntry) -> Rgba {
    git_colour(entry.mark().map(GitStatus::from_mark))
}

/// The commit box: a message, whether it would amend, and the button that would do it.
///
/// Inert, and it says so. A message is kept because the thought is worth keeping even when the
/// action is a version away.
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
                // The one obvious action, drawn where it will be and drained of the accent,
                // because nothing behind it writes.
                .child(
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
                        .child(match staged_count {
                            1 => "Commit 1 file".to_string(),
                            n => format!("Commit {n} files"),
                        }),
                ),
        )
        .child(
            mono(
                "Ubiq observes this repository and never writes into it",
                theme::text_faint(),
            )
            .text_size(theme::font(Family::Chrome, Role::Micro)),
        )
}
