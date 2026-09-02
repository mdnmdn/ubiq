//! The Git screen: what the repository is, what it has done, and what has not been committed yet.
//!
//! Four areas, one file each. The refs down the left are [`refs`] — branches, remotes, tags,
//! stashes and submodules, each section shutting on its own. The history in the middle is
//! [`history`], searched and filtered, with the graph's lanes drawn beside it. The uncommitted
//! changes on the right are [`changes`] — the staged and unstaged lists, and the commit box under
//! them. The comparison under both is [`diff`], which is what the whole screen is read for.
//!
//! **The screen is honest about what is answered and what is drawn.** The branch, the tracking
//! counts, the in-progress operation, the working-tree totals, the changed paths and the diff are
//! the host's; the branch list, the tags, the stashes, the submodules and the history are fixtures
//! until the git family carries a refs list and a log — `G70`. What the toolbar's write actions
//! and the commit box would do is nobody's yet: **Ubiq observes a repository and never writes into
//! it**, so they draw the shape the screen will have and take no clicks, and the toolbar says so.
//!
//! This is the screen about *what version control knows*. The badges on the explorer's rows are
//! the same facts at a glance, and the two never disagree, because both are projections of the one
//! working-tree map the host sent.

pub mod changes;
pub mod diff;
pub mod history;
pub mod refs;

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::IconName;
use ubiq_proto::git::{GitCounts, GitHead, RepoOverview};

use crate::app::AppState;
use crate::state::git::{CHANGES_WIDTH, SIDEBAR_WIDTH};
use crate::theme;
use crate::ui::kit::{icon_button, mono, pill, section_label};
use crate::ui::status_bar::{capped, operation_label};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The screen is a view of one project's repository, and the shell keeps a window with no
    // project off it entirely — so there is nothing here to draw rather than an empty history.
    if app.git_view(cx).is_none() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .child(toolbar(app, cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .child(
                    div()
                        .w(px(SIDEBAR_WIDTH))
                        .flex()
                        .flex_none()
                        .border_r_1()
                        .border_color(theme::border())
                        .child(refs::render(app, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_h(px(0.))
                                .child(history::render(app, window, cx))
                                .child(
                                    div()
                                        .w(px(CHANGES_WIDTH))
                                        .flex()
                                        .flex_none()
                                        .border_l_1()
                                        .border_color(theme::border())
                                        .child(changes::render(app, window, cx)),
                                ),
                        )
                        .child(diff::render(app, cx)),
                ),
        )
        .into_any_element()
}

/// The strip over the screen: which repository this is and what its HEAD is doing, the actions a
/// write version would offer, and how much the working tree has to say.
fn toolbar(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let name = app
        .project_snapshot(cx)
        .map(|project| project.record.name.clone())
        .unwrap_or_else(|| "no project".to_string());
    let overview = app.open_project(cx).and_then(|open| open.git.as_ref());

    div()
        .h(px(theme::TITLEBAR_HEIGHT))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        .child(mono(SharedString::from(name), theme::text()).text_size(px(12.5)))
        .child(head_pill(overview))
        .child(div().w(px(12.)).flex_none())
        // What a write version does, drawn as the shape it will take. Inert on purpose: nothing
        // here writes, and a control that looks live and does nothing is worse than one that says
        // what it is.
        .child(inert("Fetch"))
        .child(inert("Pull"))
        .child(inert("Push"))
        .child(inert("Branch"))
        .child(inert("Stash"))
        .child(inert("Undo"))
        .child(
            pill(theme::border())
                .h(px(22.))
                .px_2()
                .child(mono("read-only", theme::text_faint()).text_size(px(11.))),
        )
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
fn head_pill(overview: Option<&RepoOverview>) -> AnyElement {
    let Some(overview) = overview else {
        // Not a repository is an ordinary answer, and the strip says so rather than drawing a
        // branch nobody can name.
        return pill(theme::border())
            .h(px(24.))
            .px_2()
            .child(mono("not a repository", theme::text_faint()).text_size(px(11.5)))
            .into_any_element();
    };

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
        .children(overview.operation.map(|operation| {
            mono(operation_label(operation), theme::warning()).text_size(px(11.5))
        }))
        .child(mono(head, theme::text()).text_size(px(11.5)))
        .children(tracking.map(|text| mono(text, theme::text_muted()).text_size(px(11.5))))
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

/// An action the write version will have. It is drawn the way a ghost button is and takes no
/// click, because there is nothing behind it yet.
fn inert(label: &'static str) -> impl IntoElement {
    div()
        .h(px(26.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .child(section_label(label))
}
