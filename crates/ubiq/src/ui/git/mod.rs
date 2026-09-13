//! The Git screen: what the repository is, what it has done, and what has not been committed yet.
//!
//! Four movable panels, one file each, under a chrome strip that names the repository and offers
//! the write actions. The refs on the left are [`refs`] — branches, remotes, tags, stashes and
//! submodules, each section shutting on its own. The history in the centre is [`history`],
//! searched and filtered, with the graph's lanes drawn beside it. The uncommitted changes on the
//! right are [`changes`] — modified / untracked / conflicted lists, `+` / `-`, and the commit box
//! under them. The comparison under the history is [`diff`]. [`repo_selector`] is the strip's
//! control that picks which repository a project with more than one is showing.
//!
//! **Fetch all, pull, push, commit and stage / unstage are live.** Branch, stash and undo stay
//! inert. The branch, the tracking counts, the in-progress operation, the working-tree totals,
//! the changed paths and the diff are the host's.
//!
//! This is the screen about *what version control knows*. The badges on the explorer's rows are
//! the same facts at a glance, and the two never disagree, because both are projections of the one
//! working-tree map the host sent.

pub mod changes;
pub mod diff;
pub mod history;
pub mod refs;
pub mod repo_selector;

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::IconName;
use ubiq_proto::git::{GitCounts, GitHead, RepoOverview};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::{ghost_button, icon_button, mono, pill, section_label};
use crate::ui::status_bar::{capped, operation_label};

/// The strip over the Git panels: which repository this is, what HEAD is doing, the write
/// actions, and how much the working tree has to say.
pub fn toolbar(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let overview = app.open_project(cx).and_then(|open| open.git.as_ref());
    let live = overview.is_some();

    div()
        .h(px(theme::titlebar_height()))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(repo_selector::render(app, window, cx))
        .children(overview.map(head_pill))
        .child(div().w(px(12.)).flex_none())
        .child(if live {
            ghost_button(
                "git-fetch-all",
                None,
                "Fetch all",
                cx.listener(|this, _, _, cx| this.fetch_all_git(cx)),
            )
            .into_any_element()
        } else {
            inert("Fetch all").into_any_element()
        })
        .child(if live {
            ghost_button(
                "git-pull",
                None,
                "Pull",
                cx.listener(|this, _, _, cx| this.pull_git(cx)),
            )
            .into_any_element()
        } else {
            inert("Pull").into_any_element()
        })
        .child(if live {
            ghost_button(
                "git-push",
                None,
                "Push",
                cx.listener(|this, _, _, cx| this.push_git(cx)),
            )
            .into_any_element()
        } else {
            inert("Push").into_any_element()
        })
        .child(inert("Branch"))
        .child(inert("Stash"))
        .child(inert("Undo"))
        .child(div().flex_1().min_w(px(0.)))
        .children(changed_label(overview).map(|label| mono(label, theme::text_muted())))
        .child(icon_button(
            "git-refresh",
            IconName::RotateCw,
            false,
            cx.listener(|this, _, _, cx| this.refresh_git(cx)),
        ))
}

/// What HEAD is, and what it is doing: the operation first, because a repository mid-rebase is the
/// most useful thing this strip can say.
fn head_pill(overview: &RepoOverview) -> AnyElement {
    let head = match &overview.head {
        GitHead::Branch(name) => name.clone(),
        GitHead::Detached { short_id } => format!("detached {short_id}"),
        GitHead::Unborn(name) => format!("{name} (unborn)"),
    };
    let tracking = match (overview.ahead, overview.behind) {
        (Some(ahead), Some(behind)) if ahead > 0 || behind > 0 => Some(format!(
            "\u{2191}{} \u{2193}{}",
            capped(ahead),
            capped(behind)
        )),
        _ => None,
    };

    pill(theme::accent())
        .h(px(24.))
        .px_2()
        .children(
            overview
                .operation
                .map(|operation| mono(operation_label(operation), theme::warning())),
        )
        .child(mono(head, theme::text()))
        .children(tracking.map(|text| mono(text, theme::text_muted())))
        .into_any_element()
}

/// How many paths the working tree has something to say about, or nothing at all when no walk has
/// answered yet. Absent rather than zero, on the rule the status bar follows.
fn changed_label(overview: Option<&RepoOverview>) -> Option<String> {
    let counts = overview?.counts?;
    let GitCounts {
        staged,
        modified,
        untracked,
        conflicted,
    } = counts;
    let total = staged + modified + untracked + conflicted;
    Some(match total {
        0 => "nothing changed".to_string(),
        1 => "1 changed".to_string(),
        n => format!("{n} changed"),
    })
}

/// An action that is not live yet. Drawn the way a ghost button is and takes no click.
fn inert(label: &'static str) -> impl IntoElement {
    div()
        .h(px(26.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .child(section_label(label))
}
