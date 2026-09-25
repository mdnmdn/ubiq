//! The bottom strip: which file is open and where the caret is — or,
//! on the agents screen, how the columns are filled and what is on the bench, or, on the
//! orchestration screen, how many agents there are and what they are doing, or, on the board, how
//! much work there is and where it has got to.
//!
//! It reports facts, never intentions, and an absent fact is drawn as absent. It reports on
//! whatever is on screen, which is why the rail mode picks which set of facts it has: a caret in a
//! screen with no buffer is not a fact. A project in a repository prints its branch; a project
//! that is not one prints nothing git-related.

use gpui::{
    Anchor, App, Context, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use ubiq_proto::git::{AHEAD_BEHIND_CAP, GitCounts, GitOperation};
use ubiq_proto::work::Bucket;

use crate::app::AppState;
use crate::state::editor::ViewerKind;
use crate::state::navigator::subsequence;
use crate::state::vim::VimMode;
use crate::state::{MenuId, RailMode};
use crate::theme;
use crate::ui::board::status_colour;
use crate::ui::kit::{Picker, PickerStyle, UbiqIcon, icon_button, mono};
use crate::ui::work::bucket_colour;
use crate::ui::{handler, indexed, size};
use crate::version;

/// The bundle version, at the left edge of the strip's right-justified half. Fixed at build time
/// by `_devops/scripts/bundle-version.sh` — see `crate::version`. Elided rather than wrapped or
/// dropped, the way every other value in this strip is, so it never grows the strip; the full
/// string is the tooltip.
fn version_label() -> impl IntoElement {
    mono(version::short(), theme::text_faint())
        .id("bundle-version")
        .tooltip(|window, cx| {
            gpui_component::tooltip::Tooltip::new(version::FULL).build(window, cx)
        })
}

/// The attribution just after the version. The heart never gives up its room — the words around
/// it do, clipping away first when the strip runs out of space, so a narrow window still shows
/// the heart and the full line stays one hover away.
fn made_with_love() -> impl IntoElement {
    div()
        .id("made-with-love")
        .flex()
        .items_center()
        .min_w(px(0.))
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .child(mono("Made with ", theme::text_faint())),
        )
        .child(mono("\u{2665}", theme::danger()))
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .child(mono(" in Turin", theme::text_faint())),
        )
        .tooltip(|window, cx| {
            gpui_component::tooltip::Tooltip::new("Made with \u{2665} in Turin").build(window, cx)
        })
}

/// The vim chip: what mode the focused input is in, and the switch that turns modal editing on and
/// off. One element for both, because a readout the user cannot act on and a switch that does not
/// say what it did are the same slot asked twice.
///
/// Present whether or not vim is on — off it reads `VIM`, faint. A chip that appeared only once the
/// feature was enabled would be a switch nobody could find.
fn vim_chip(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let on = app.workbench.settings.ui.vim_mode;
    let (text, colour) = if !on {
        ("VIM".to_string(), theme::text_faint())
    } else if app.vim.mode == VimMode::Normal {
        (app.vim.label(), theme::accent())
    } else {
        (app.vim.label(), theme::text_muted())
    };

    mono(text, colour)
        .id("vim-mode")
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| this.toggle_vim_mode(cx)))
        .tooltip(move |window, cx| {
            let note = if on {
                "Vim mode is on \u{2014} click to turn it off"
            } else {
                "Vim mode is off \u{2014} click to turn it on"
            };
            gpui_component::tooltip::Tooltip::new(note).build(window, cx)
        })
}

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let strip = div()
        .h(px(theme::status_bar_height()))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_4()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border());

    // A window holding no project has one fact to report, and reports only that. A config root
    // pointed anywhere but the usual place still says so: it is true whatever is open.
    if app.project(cx).is_none() {
        return strip
            .child(mono("no project", theme::text_faint()))
            .child(div().flex_1().min_w(px(0.)))
            .child(size_control(app, cx))
            .child(made_with_love())
            .child(version_label());
    }

    // The agents screen is a screen about columns, so the strip counts columns: how many there are,
    // how many agents are in them, how many of those columns are grouped, how many the user has put
    // back on the bench, how the agents on the field are spread across the four states, and which
    // harnesses are behind them. It counts the field rather than the project on purpose — the strip
    // reports on what is on screen, and the bench is exactly the difference.
    if app.workbench.rail_mode == RailMode::Agents
        && let (Some(work), Some(agents)) = (app.work(cx), app.agents(cx))
    {
        let bench = agents.benched(work).len();
        // Which harnesses are behind the columns, each named once. A row of columns is often a row
        // of different tools, and it is the one fact about them the columns' own footers say only
        // one at a time.
        let mut harnesses: Vec<String> = Vec::new();
        for name in agents
            .columns
            .iter()
            .flat_map(|column| column.tabs.iter())
            .filter_map(|id| work.agent(*id))
            .map(|agent| agent.harness.clone())
        {
            if !harnesses.contains(&name) {
                harnesses.push(name);
            }
        }

        return strip
            .child(mono(
                format!(
                    "{} columns \u{b7} {} agents \u{b7} {} grouped \u{b7} {bench} on the bench",
                    agents.columns.len(),
                    agents.on_the_field(),
                    agents.grouped()
                ),
                theme::text_muted(),
            ))
            .children(Bucket::all().into_iter().map(|bucket| {
                let n = agents.count(work, bucket);
                mono(
                    format!("{n} {}", bucket.label()),
                    if n == 0 {
                        theme::text_faint()
                    } else {
                        bucket_colour(bucket)
                    },
                )
            }))
            .child(div().flex_1().min_w(px(0.)))
            .child(size_control(app, cx))
            .child(version_label())
            .child(made_with_love())
            .child(mono(harnesses.join(" \u{b7} "), theme::text_muted()));
    }

    // On the orchestration screen there is no file and no caret to report, so the strip reports
    // what is on screen instead: how many sessions and agents there are, and how the agents are
    // spread across the four states. A count of zero is drawn as zero rather than dropped — "no
    // agent is failing" is a fact, and it is the one the user is checking for.
    if app.workbench.rail_mode == RailMode::TeamsOld
        && let Some(work) = app.work(cx)
    {
        return strip
            .child(mono(
                format!(
                    "{} sessions \u{b7} {} agents",
                    work.sessions.len(),
                    work.agents.len()
                ),
                theme::text_muted(),
            ))
            .children(Bucket::all().into_iter().map(|bucket| {
                let n = work.count(bucket);
                mono(
                    format!("{n} {}", bucket.label()),
                    if n == 0 {
                        theme::text_faint()
                    } else {
                        bucket_colour(bucket)
                    },
                )
            }))
            .child(div().flex_1().min_w(px(0.)))
            .child(size_control(app, cx))
            .child(version_label())
            .child(made_with_love());
    }

    // The board is a screen about work rather than about a file, so the strip counts the work: how
    // many cards are in each column, how many sub-tasks are done across them, and how many of them
    // nobody can finish without the user. A count of zero is drawn as zero, for the reason the
    // two screens over the agents do.
    if app.workbench.rail_mode == RailMode::Tasks
        && let (Some(work), Some(board)) = (app.work(cx), app.board(cx))
    {
        let (done, total) = board.steps(work);
        let blocked = board.blocked(work);
        return strip
            .children(board.counts(work).into_iter().map(|(status, n)| {
                mono(
                    format!("{n} {}", status.label()),
                    if n == 0 {
                        theme::text_faint()
                    } else {
                        status_colour(status)
                    },
                )
            }))
            .child(div().flex_1().min_w(px(0.)))
            .child(size_control(app, cx))
            .child(version_label())
            .child(made_with_love())
            .child(mono(
                format!("{done}/{total} sub-tasks done"),
                theme::text_muted(),
            ))
            .child(mono(
                format!("{blocked} blocked"),
                if blocked == 0 {
                    theme::text_faint()
                } else {
                    theme::danger()
                },
            ));
    }

    let active = app.editor(cx).and_then(|editor| editor.active_file());
    let where_it_is = match active {
        Some(file) => file.name.clone(),
        None => "no file open".to_string(),
    };

    strip
        .child(mono(where_it_is, theme::text_muted()))
        .children(viewer_picker(app, window, cx))
        .child(div().flex_1().min_w(px(0.)))
        .children(git_readout(app, cx))
        .child(vim_chip(app, cx))
        .child(size_control(app, cx))
        .child(version_label())
        .child(made_with_love())
}

/// The file-kind chip, just after the file name: what the open tab is, and the way to draw it as
/// something else.
///
/// It reads as the *language* — Rust, Markdown, TypeScript — because that is the fact the strip is
/// reporting, and its list offers *viewers*, because that is the only part of it the user can
/// change. There is nothing to report with no file open, so the chip is absent rather than empty,
/// the way every other value in this strip is.
///
/// Whatever is picked lasts as long as the tab does; `AppState::set_viewer_kind` says why nothing
/// is written down.
fn viewer_picker(
    app: &AppState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> Option<impl IntoElement> {
    let file = app.editor(cx)?.active_file()?;
    let key = file.key();
    let current = file.viewer;
    let label = file.language.label();

    let open = app.workbench.open_menu == Some(MenuId::ViewerKind);
    let needle = app.picker_search.read(cx).value().trim().to_lowercase();
    let shown: Vec<ViewerKind> = if open && !needle.is_empty() {
        ViewerKind::all()
            .into_iter()
            .filter(|kind| subsequence(&needle, kind.label()))
            .collect()
    } else {
        ViewerKind::all().to_vec()
    };

    let view = cx.entity();
    let focused = app
        .picker_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let picked = shown.clone();

    let mut picker = Picker::new("file-kind", label)
        .items(shown.iter().map(|kind| kind.label()))
        .open(open)
        .anchor(Anchor::BottomLeft)
        .style(PickerStyle::Chip)
        .search(&app.picker_search, focused)
        .on_toggle(handler(&view, |this, window, cx| {
            this.open_viewer_menu(window, cx)
        }))
        .on_pick(indexed(&view, move |this, index, _, cx| {
            if let Some(kind) = picked.get(index).copied() {
                this.set_viewer_kind(&key, kind, cx);
            }
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)));
    if let Some(selected) = shown.iter().position(|kind| *kind == current) {
        picker = picker.selected(selected);
    }
    Some(picker.into_any_element())
}

/// The size control, at the bottom-right of the strip — an icon and the panel it opens.
///
/// **It replaced a dropdown that said `13 px`.** That control named a point size, changed four
/// unrelated surfaces with it and moved nothing they sat in; what is here moves the two axes
/// everything follows, and says so with two icons and a track rather than a number (`D151`).
/// Icon-only, so the tooltip is not optional: it names the preset the axes spell, or *Custom*.
fn size_control(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let open = size::is_open(app);
    let tip: SharedString = size::trigger_tooltip(app).into();

    let mut trigger = icon_button(
        "size-control",
        UbiqIcon::SizeTextLarge,
        open,
        cx.listener(|this, _, window, cx| this.open_size_menu(window, cx)),
    )
    .tooltip(move |window, cx| {
        gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
    });
    if open {
        trigger = trigger.child(size::panel(app, cx));
    }
    trigger
}

/// Branch, tracking, working-tree totals — or nothing, when the project is not a repository.
fn git_readout(app: &AppState, cx: &App) -> Option<impl IntoElement> {
    let open = app.open_project(cx)?;
    let overview = open.git.as_ref()?;
    let mut parts = Vec::new();
    if let Some(operation) = overview.operation {
        parts.push(operation_label(operation).to_string());
    }
    parts.push(crate::state::git::head_label(&overview.head));
    match (overview.ahead, overview.behind) {
        (Some(ahead), Some(behind)) if ahead > 0 || behind > 0 => {
            parts.push(format!("↑{} ↓{}", capped(ahead), capped(behind)));
        }
        _ => {}
    }
    if let Some(counts) = overview.counts {
        let label = counts_label(counts);
        if !label.is_empty() {
            parts.push(label);
        }
    }
    if open.git_truncated {
        parts.push("…".to_string());
    }
    Some(mono(parts.join("  "), theme::text_muted()))
}

/// The word for an in-progress operation. Shared with the Git screen's toolbar, so the two say the
/// same thing about the same repository.
pub fn operation_label(operation: GitOperation) -> &'static str {
    match operation {
        GitOperation::Merge => "merge",
        GitOperation::Rebase | GitOperation::RebaseInteractive => "rebase",
        GitOperation::CherryPick => "cherry-pick",
        GitOperation::Revert => "revert",
        GitOperation::Bisect => "bisect",
        GitOperation::ApplyMailbox => "am",
    }
}

/// Ahead or behind, as the number or the cap. Shared with the Git screen for the reason above.
pub fn capped(n: u32) -> String {
    if n >= AHEAD_BEHIND_CAP {
        "99+".to_string()
    } else {
        n.to_string()
    }
}

fn counts_label(counts: GitCounts) -> String {
    let mut parts = Vec::new();
    if counts.conflicted > 0 {
        parts.push(format!("{}!", counts.conflicted));
    }
    if counts.modified > 0 {
        parts.push(format!("{}M", counts.modified));
    }
    if counts.staged > 0 {
        parts.push(format!("{}S", counts.staged));
    }
    if counts.untracked > 0 {
        parts.push(format!("{}U", counts.untracked));
    }
    parts.join(" ")
}
