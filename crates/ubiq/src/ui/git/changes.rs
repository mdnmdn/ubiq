//! The panel beside the history: what the selected row is about.
//!
//! On the uncommitted row — the one the screen opens on — that is the working tree: conflicted
//! paths first when any exist, then staged, then unstaged, and the commit box under them. A path
//! that is both staged and modified appears in both lists. Two commits selected for a range list
//! the paths that differ between them. One commit shows what the log said about it.
//!
//! **The lists are the host's answer.** Conflicted rows have no `+` / `-`. Staged rows unstage,
//! unstaged rows stage; the section headers do the same to every path in the list. The letter is
//! the index change on a staged row and the worktree change on an unstaged one. The colour is
//! the explorer's, so a path reads the same in both places.
//!
//! **The commit box writes.** It commits when there is a message and something staged (or amend),
//! and draws the last write error under the button when there is one.

use gpui::{
    AnyElement, Context, Entity, Focusable, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Rgba, StatefulInteractiveElement, Styled, Window, div, point,
    px, uniform_list,
};
use gpui_component::input::Textarea;
use ubiq_proto::git::{GitChangedPath, GitEntry, GitPathChange};

use crate::app::AppState;
use crate::state::GitStatus;
use crate::state::git::{
    ChangeSection, GitMenuKind, Side, can_stage, can_unstage, change_letter, group_changes, staged,
};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::explorer::git_colour;
use crate::ui::kit::{
    ContextItem, badge, check_box, context_menu, elided_with, field, mono, panel, panel_header,
    primary_button, section_label,
};
use crate::ui::{handler, indexed};

/// Every row in the list is this tall, a list heading included, so the list is uniform and only
/// what is on screen is built.
const ROW: f32 = 24.0;
/// Click target for the stage / unstage glyphs.
const ACTION: f32 = 18.0;

/// One row of the flattened working tree: a list's heading, or one changed path in it as an index
/// into the project's entries.
enum Flat {
    Header {
        section: ChangeSection,
        count: usize,
        open: bool,
    },
    Row(Side, usize),
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let menu = git.menu.clone();

    let body = match (git.selected_commit, git.compare_commit) {
        (Some(_), Some(_)) => range_files(app, window, cx),
        (Some(index), None) => commit(app, index, cx),
        (None, _) => working_tree(app, window, cx),
    };

    let Some(menu) = menu.filter(|menu| matches!(menu.kind, GitMenuKind::Change { .. })) else {
        return body;
    };
    let epoch = menu.epoch;
    let (stageable, unstageable) = match &menu.kind {
        GitMenuKind::Change { path, .. } => app
            .git_entries(cx)
            .unwrap_or(&[])
            .iter()
            .find(|entry| &entry.rel_path == path)
            .map(|entry| (can_stage(entry), can_unstage(entry)))
            .unwrap_or((false, false)),
        _ => (false, false),
    };
    let items: Vec<ContextItem> = menu
        .entries(stageable, unstageable)
        .into_iter()
        .map(|entry| {
            if entry.is_separator() {
                return ContextItem::separator();
            }
            let item = ContextItem::new(entry.label());
            if entry.enabled { item } else { item.disabled() }
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(body)
        .child(context_menu(
            "git-change-menu",
            point(px(menu.x), px(menu.y)),
            items,
            indexed(&cx.entity(), |this, index, _, cx| {
                this.pick_git_menu_action(index, cx)
            }),
            handler(&cx.entity(), move |this, _, cx| {
                this.dismiss_git_menu(epoch, cx)
            }),
        ))
        .into_any_element()
}

/// The working tree: three lists and the box under them.
fn working_tree(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let conflicted_open = git.is_change_open(ChangeSection::Conflicted);
    let staged_open = git.is_change_open(ChangeSection::Staged);
    let unstaged_open = git.is_change_open(ChangeSection::Unstaged);
    let entries = app.git_entries(cx).unwrap_or(&[]);
    let groups = group_changes(entries);
    let staged_count = staged(entries).len();

    // The three lists become one, so the panel is a single virtual list: only the rows on screen
    // are built, and a heading is a row like any other. Staged and Unstaged stay visible at
    // zero so the split is on screen; Conflicted hides when it has nothing to say.
    let mut rows: Vec<Flat> = Vec::new();
    for (side, group, always) in [
        (Side::Conflicted, groups.conflicted.as_slice(), false),
        (Side::Staged, groups.staged.as_slice(), true),
        (Side::Unstaged, groups.unstaged.as_slice(), true),
    ] {
        if !always && group.is_empty() {
            continue;
        }
        let open = match side {
            Side::Conflicted => conflicted_open,
            Side::Staged => staged_open,
            Side::Unstaged => unstaged_open,
        };
        rows.push(Flat::Header {
            section: side.section(),
            count: group.len(),
            open,
        });
        if open {
            rows.extend(group.iter().map(|index| Flat::Row(side, *index)));
        }
    }

    let count = rows.len();
    let view = cx.entity();
    let mut body = div().flex().flex_col().flex_1().min_h(px(0.)).child(
        uniform_list("git-changes", count, move |range, window, cx| {
            let app = view.read(cx);
            let entries = app.git_entries(cx).unwrap_or(&[]);
            let selected = app.git_view(cx).and_then(|git| git.selected_path.clone());
            range
                .filter_map(|slot| match rows.get(slot)? {
                    Flat::Header {
                        section,
                        count,
                        open,
                    } => Some(
                        list_header(*section, *count, *open, &view, window).into_any_element(),
                    ),
                    Flat::Row(side, index) => {
                        let entry = entries.get(*index)?;
                        let picked = selected.as_ref().is_some_and(|(held, path)| {
                            *held == *side && path == &entry.rel_path
                        });
                        Some(change_row(*side, entry, picked, &view, window))
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

/// The paths that differ between the two selected commits. No `+` / `-`: a range is a read.
fn range_files(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let files = git.range_files.clone();
    let selected = git.path().map(str::to_string);
    let count = files.len();
    let view = cx.entity();

    panel()
        .child(panel_header(
            "Comparing",
            mono(format!("{count} paths"), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        ))
        .child(
            div().flex().flex_col().flex_1().min_h(px(0.)).child(
                uniform_list("git-range-files", count, move |range, window, cx| {
                    range
                        .filter_map(|slot| {
                            let file = files.get(slot)?;
                            Some(range_row(
                                file,
                                selected.as_deref() == Some(file.rel_path.as_str()),
                                &view,
                                window,
                            ))
                        })
                        .collect::<Vec<AnyElement>>()
                })
                .flex_1()
                .min_h(px(0.)),
            ),
        )
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

/// One list's heading. Staged carries a right-justified `-` (unstage all), Unstaged a `+`
/// (stage all). A click on the heading itself shuts the section; the glyph is its own target.
fn list_header(
    section: ChangeSection,
    count: usize,
    _open: bool,
    view: &Entity<AppState>,
    window: &Window,
) -> impl IntoElement {
    let mut row = div()
        .id(crate::ui::eid("git-change-header", section.label()))
        .h(px(ROW))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(section_label(section.label()))
        .child(
            mono(format!("{count}"), theme::text_faint())
                .text_size(theme::font(Family::Chrome, Role::Meta)),
        )
        .child(div().flex_1().min_w(px(0.)));

    match section {
        ChangeSection::Staged => {
            row = row.child(header_glyph(
                crate::ui::eid("git-unstage-all", section.label()),
                "-",
                count > 0,
                false,
                view,
                window,
            ));
        }
        ChangeSection::Unstaged => {
            row = row.child(header_glyph(
                crate::ui::eid("git-stage-all", section.label()),
                "+",
                count > 0,
                true,
                view,
                window,
            ));
        }
        ChangeSection::Conflicted => {}
    }

    row.on_click(window.listener_for(view, move |this, _, _, cx| {
        this.toggle_git_change_section(section, cx)
    }))
}

/// One changed path. The letter is the index change on a staged row and the worktree change on an
/// unstaged one; conflicted rows say `!`. Non-conflicted rows carry right-justified `+` / `-`.
fn change_row(
    side: Side,
    entry: &GitEntry,
    selected: bool,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let change = match side {
        Side::Staged => entry.index.as_ref(),
        Side::Unstaged | Side::Conflicted => entry.worktree.as_ref().or(entry.index.as_ref()),
    };
    let letter = match side {
        Side::Conflicted => "!",
        _ => change.map(change_letter).unwrap_or(" "),
    };
    let colour = row_colour(entry);
    let path = entry.rel_path.clone();
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let tag = side.label();
    let stageable = can_stage(entry);
    let unstageable = can_unstage(entry);

    let mut row = div()
        .id(crate::ui::eid2("git-change", tag, &path))
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
            crate::ui::eid2("git-change-name", tag, &path),
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
                crate::ui::eid2("git-stage", tag, &path),
                "+",
                stageable,
                path.clone(),
                true,
                view,
                window,
            ))
            .child(action_glyph(
                crate::ui::eid2("git-unstage", tag, &path),
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

    let menu_path = path.clone();
    row.on_click(window.listener_for(view, move |this, _, _, cx| {
        this.select_git_path(side, &path, cx)
    }))
    .on_mouse_down(
        MouseButton::Right,
        window.listener_for(view, move |this, event: &MouseDownEvent, _, cx| {
            this.open_git_menu(
                GitMenuKind::Change {
                    path: menu_path.clone(),
                    side,
                },
                (f32::from(event.position.x), f32::from(event.position.y)),
                cx,
            );
        }),
    )
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

/// An 18px `+` or `-` on a section heading. Stages or unstages every path; stops the click so
/// the heading itself can still shut the section.
fn header_glyph(
    id: impl Into<gpui::ElementId>,
    glyph: &'static str,
    enabled: bool,
    stage_all: bool,
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
                if stage_all {
                    this.stage_all_git(cx);
                } else {
                    this.unstage_all_git(cx);
                }
            }));
    }
    btn.into_any_element()
}

/// One path in a two-commit range. The letter is the change between those revs; there is no
/// `+` / `-` because a range is not the working tree.
fn range_row(
    file: &GitChangedPath,
    selected: bool,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let path = file.rel_path.clone();
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let letter = change_letter(&file.change);
    let colour = range_colour(&file.change);

    let mut row = div()
        .id(crate::ui::eid("git-range", &path))
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
            crate::ui::eid("git-range-name", &path),
            name,
            path.clone(),
            theme::text_muted(),
            theme::font(theme::Family::Chrome, theme::Role::Label),
        ))
        .children(match &file.change {
            GitPathChange::Renamed { from } => Some(
                mono(format!("\u{2190} {from}"), theme::text_faint())
                    .text_size(theme::font(Family::Chrome, Role::Micro)),
            ),
            _ => None,
        });

    if selected {
        row = row
            .bg(theme::accent_soft())
            .border_l_2()
            .border_color(theme::accent());
    }

    row.on_click(window.listener_for(view, move |this, _, _, cx| {
        this.select_git_path(Side::Unstaged, &path, cx)
    }))
    .into_any_element()
}

/// The colour a changed path takes, which is the explorer's for the same path: both are the same
/// projection of the same pair.
fn row_colour(entry: &GitEntry) -> Rgba {
    git_colour(entry.mark().map(GitStatus::from_mark))
}

fn range_colour(change: &GitPathChange) -> Rgba {
    match change {
        GitPathChange::Added | GitPathChange::Untracked => theme::success(),
        GitPathChange::Deleted => theme::danger(),
        GitPathChange::Modified | GitPathChange::Renamed { .. } | GitPathChange::TypeChange => {
            theme::warning()
        }
    }
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
