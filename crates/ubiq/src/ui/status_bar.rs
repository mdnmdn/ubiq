//! The bottom strip: which file is open, where the caret is, and what the composer is set to — or,
//! on the agents screen, how many agents there are and what they are doing.
//!
//! It reports facts, never intentions, and an absent fact is drawn as absent. It reports on
//! whatever is on screen, which is why the rail mode picks which set of facts it has: a caret in a
//! screen with no buffer is not a fact. Nothing reads version control, so there is no branch and no
//! working-tree count here — a readout nobody can answer for is worse than none.

use gpui::{Context, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::agents::Bucket;
use crate::state::{OpenFile, RailMode, SaveState};
use crate::theme;
use crate::ui::agents::bucket_colour;
use crate::ui::kit::mono;

/// Where this run writes everything down, when that is not `~/.config/ubiq`.
fn config_root(app: &AppState) -> Option<impl IntoElement> {
    if app.workbench.config_root_is_default {
        return None;
    }
    let root = app.workbench.config_root.clone()?;
    let shown = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => root.replace(&home, "~"),
        _ => root,
    };

    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::Inspector)
                    .with_size(Size::XSmall)
                    .text_color(theme::warning()),
            )
            .child(mono(shown, theme::warning())),
    )
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

    // On the agents screen there is no file and no caret to report, so the strip reports what is
    // on screen instead: how many sessions and agents there are, and how the agents are spread
    // across the four states. A count of zero is drawn as zero rather than dropped — "no agent is
    // failing" is a fact, and it is the one the user is checking for.
    if app.workbench.rail_mode == RailMode::Agents {
        let agents = &app.agents;
        return strip
            .child(mono(
                format!(
                    "{} sessions \u{b7} {} agents",
                    agents.sessions.len(),
                    agents.agents.len()
                ),
                theme::text_muted(),
            ))
            .children(Bucket::all().into_iter().map(|bucket| {
                let n = agents.count(bucket);
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
            .children(config_root(app));
    }

    // A window holding no project has one fact to report, and reports only that. A config root
    // pointed anywhere but the usual place still says so: it is true whatever is open.
    if app.project(cx).is_none() {
        return strip
            .child(mono("no project", theme::text_faint()))
            .child(div().flex_1().min_w(px(0.)))
            .children(config_root(app));
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
        // A config root you cannot see is a foot-gun, so a run pointed anywhere but the usual
        // place says so.
        .children(config_root(app))
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
}
