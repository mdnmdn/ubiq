//! The bottom strip: which file is open, where the caret is, and what the composer is set to — or,
//! on the agents screen, how the columns are filled and what is on the bench, or, on the
//! orchestration screen, how many agents there are and what they are doing, or, on the board, how
//! much work there is and where it has got to.
//!
//! It reports facts, never intentions, and an absent fact is drawn as absent. It reports on
//! whatever is on screen, which is why the rail mode picks which set of facts it has: a caret in a
//! screen with no buffer is not a fact. A project in a repository prints its branch; a project
//! that is not one prints nothing git-related.

use gpui::{
    Anchor, App, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};

use ubiq_proto::git::{AHEAD_BEHIND_CAP, GitCounts, GitHead, GitOperation};
use ubiq_proto::work::Bucket;

use crate::app::AppState;
use crate::state::vim::VimMode;
use crate::state::{MenuId, OpenFile, RailMode, SaveState};
use crate::theme;
use crate::ui::board::status_colour;
use crate::ui::kit::{Picker, mono};
use crate::ui::work::bucket_colour;
use crate::ui::{handler, indexed};
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

/// What the active file's save is doing, when it is doing anything worth a word.
fn save_state(file: &OpenFile) -> Option<impl IntoElement> {
    let (text, colour) = match (&file.save, file.dirty()) {
        (SaveState::Failed(reason), _) => (format!("save failed: {reason}"), theme::danger()),
        (SaveState::Saving(_), _) => ("saving\u{2026}".to_string(), theme::info()),
        (SaveState::Idle, true) => ("unsaved".to_string(), theme::warning()),
        (SaveState::Idle, false) => return None,
    };
    Some(mono(text, colour))
}

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let strip = div()
        .h(px(theme::STATUS_BAR_HEIGHT))
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
            .child(version_label())
            .child(made_with_love())
            .child(mono(harnesses.join(" \u{b7} "), theme::text_muted()));
    }

    // On the orchestration screen there is no file and no caret to report, so the strip reports
    // what is on screen instead: how many sessions and agents there are, and how the agents are
    // spread across the four states. A count of zero is drawn as zero rather than dropped — "no
    // agent is failing" is a fact, and it is the one the user is checking for.
    if app.workbench.rail_mode == RailMode::Orchestration
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
        Some(file) => file.path.clone(),
        None => "no file open".to_string(),
    };
    let language = active.map(|file| file.language.label());

    strip
        .child(mono(where_it_is, theme::text_muted()))
        // What a save is doing, in the one place that reports on the file as a whole. A failure
        // takes the danger colour, because it is the only thing here the user has to act on.
        .children(active.and_then(save_state))
        .child(div().flex_1().min_w(px(0.)))
        .child(version_label())
        .child(made_with_love())
        .children(git_readout(app, cx))
        .child(vim_chip(app, cx))
        // A caret in a buffer nobody is looking at is not a fact, so the readout goes with the
        // file rather than reporting a position in nothing.
        .children(
            app.cursor_line_column(cx).map(|(line, column)| {
                mono(format!("Ln {line}, Col {column}"), theme::text_muted())
            }),
        )
        .children(language.map(|language| {
            mono(
                format!("{language} \u{b7} UTF-8 \u{b7} LF"),
                theme::text_muted(),
            )
        }))
        .child(mono(
            SharedString::from(format!(
                "{} \u{b7} {}",
                app.chat.harness_label(),
                app.chat.mode_label()
            )),
            theme::text_muted(),
        ))
        .child(font_size_dropdown(app, cx))
}

/// Every text size the status bar's dropdown offers, in points. A hand-picked ladder rather than
/// every integer: a font size is chosen by eye, and the spread of sizes the same knob wants on an
/// editor, a terminal and a tree is the ladder rather than a grid.
const FONT_SIZES: &[f32] = &[
    10.0, 11.0, 12.0, 13.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0,
];

/// The text-size dropdown, at the bottom-right of the strip. A project's text size scales the
/// editor, the terminal panes and the explorer tree together, and it is remembered with the
/// project, so the dropdown is where the whole window's zoom lives. The nearest ladder entry is
/// shown when a nudge-landed size is not one of the ladder's.
fn font_size_dropdown(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let view = cx.entity();
    let current = app.ui_font_size_or_default(cx);
    let index = nearest_font_index(current);

    Picker::new("font-size", format!("{current:.0}\u{2009}px"))
        .items(
            FONT_SIZES
                .iter()
                .map(|n| format!("{n:.0}"))
                .collect::<Vec<_>>(),
        )
        .selected(index)
        .open(app.workbench.open_menu == Some(MenuId::FontSize))
        .anchor(Anchor::BottomLeft)
        .on_toggle(handler(&view, |this, _, cx| {
            this.open_menu(MenuId::FontSize, cx)
        }))
        .on_pick(indexed(&view, |this, index, _, cx| {
            this.set_ui_font_size(FONT_SIZES[index], cx)
        }))
        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx)))
        .into_any_element()
}

/// The ladder entry nearest a size, for the trigger's `selected` mark. Only meaningful when the
/// current size is a nudge-landed value that is not itself on the ladder.
fn nearest_font_index(size: f32) -> usize {
    let mut best = 0;
    let mut best_delta = f32::INFINITY;
    for (i, &candidate) in FONT_SIZES.iter().enumerate() {
        let delta = (candidate - size).abs();
        if delta < best_delta {
            best_delta = delta;
            best = i;
        }
    }
    best
}

/// Branch, tracking, working-tree totals — or nothing, when the project is not a repository.
fn git_readout(app: &AppState, cx: &App) -> Option<impl IntoElement> {
    let open = app.open_project(cx)?;
    let overview = open.git.as_ref()?;
    let mut parts = Vec::new();
    if let Some(operation) = overview.operation {
        parts.push(operation_label(operation).to_string());
    }
    parts.push(match &overview.head {
        GitHead::Branch(name) => name.clone(),
        GitHead::Detached { short_id } => format!("detached {short_id}"),
        GitHead::Unborn(name) => name.clone(),
    });
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
