//! The Git screen: what the repository is, what it has done, and what has not been committed yet.
//!
//! Four movable panels, one file each, under a chrome strip that names the repository and offers
//! the write actions. The refs on the left are [`refs`] — branches, remotes, tags, stashes and
//! submodules, each section shutting on its own. The history in the centre is [`history`],
//! searched and filtered, with the graph's lanes drawn beside it. The uncommitted changes on the
//! right are [`changes`] — modified / untracked / conflicted lists, `+` / `-`, and the commit box
//! under them. The comparison under the history is [`diff`]. A project with more than one
//! repository picks which one the screen shows in the Repositories section atop [`refs`].
//!
//! **Every action is live** — fetch all, pull, push, branch, stash, undo, commit, checkout and
//! stage / unstage — and drawn as an icon with its name in a tooltip; all but refresh go faint
//! with no repository, and `Checkout` also faint with nothing selected in the refs panel.
//! `Branch` opens a small popover for the new branch's name. The branch, the tracking counts, the
//! in-progress operation, the working-tree totals, the changed paths and the diff are the host's.
//!
//! A write that could lose something asks first: a forced checkout, a discard, a reset or
//! deleting a ref raises [`crate::state::git::GitConfirm`] on `GitView`, and the window's shared
//! overlay stack (`Layer::GitConfirm`) is what dismisses it — `ui::shell::git_confirm` paints it,
//! not this module, so Escape peels it the same way it peels every other confirm.
//!
//! This is the screen about *what version control knows*. The badges on the explorer's rows are
//! the same facts at a glance, and the two never disagree, because both are projections of the one
//! working-tree map the host sent.

pub mod changes;
pub mod diff;
pub mod history;
pub mod refs;

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled,
    anchored, deferred, div, point, prelude::FluentBuilder, px,
};
use gpui_component::IconName;
use gpui_component::input::Input;
use ubiq_proto::git::{GitCounts, GitHead, RepoOverview};

use crate::app::AppState;
use crate::state::MenuId;
use crate::state::git::RefSection;
use crate::theme;
use crate::ui::kit::menu::MENU_LAYER;
use crate::ui::kit::{UbiqIcon, icon_button_tip, mono, pill, primary_button};
use crate::ui::status_bar::{capped, operation_label};

/// The strip over the Git panels: which repository this is, what HEAD is doing, the write
/// actions, and how much the working tree has to say.
pub fn toolbar(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let overview = app.git_overview(cx);
    let live = overview.is_some();
    let checkoutable = live
        && app.git_view(cx).is_some_and(|git| {
            git.selected_ref.is_some_and(|index| {
                git.refs.get(index).is_some_and(|row| {
                    matches!(
                        row.section,
                        RefSection::Local | RefSection::Remotes | RefSection::Tags
                    )
                })
            })
        });

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
        .children(overview.map(head_pill))
        .child(div().w(px(12.)).flex_none())
        .child(icon_button_tip(
            "git-checkout",
            UbiqIcon::GitBranch,
            "Checkout the selected ref",
            checkoutable,
            cx.listener(|this, _, _, cx| this.checkout_selected_git_ref(cx)),
        ))
        .child(icon_button_tip(
            "git-fetch-all",
            UbiqIcon::GitFetch,
            "Fetch all",
            live,
            cx.listener(|this, _, _, cx| this.fetch_all_git(cx)),
        ))
        .child(icon_button_tip(
            "git-pull",
            IconName::ArrowDown,
            "Pull",
            live,
            cx.listener(|this, _, _, cx| this.pull_git(cx)),
        ))
        .child(icon_button_tip(
            "git-push",
            IconName::ArrowUp,
            "Push",
            live,
            cx.listener(|this, _, _, cx| this.push_git(cx)),
        ))
        .child(branch_button(app, live, cx))
        .child(icon_button_tip(
            "git-stash",
            UbiqIcon::GitStash,
            "Stash changes",
            live,
            cx.listener(|this, _, _, cx| this.stash_git(cx)),
        ))
        .child(icon_button_tip(
            "git-undo",
            IconName::Undo,
            "Undo last commit",
            live,
            cx.listener(|this, _, _, cx| this.undo_git_commit(cx)),
        ))
        .child(div().flex_1().min_w(px(0.)))
        .children(
            app.git_view(cx)
                .and_then(|git| git.in_progress())
                .map(|label| mono(label, theme::warning())),
        )
        .children(changed_label(overview).map(|label| mono(label, theme::text_muted())))
        .child(icon_button_tip(
            "git-refresh",
            IconName::RotateCw,
            "Refresh",
            true,
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

/// How wide the `Branch` popover is.
const BRANCH_PANEL_WIDTH: f32 = 260.0;

/// The `Branch` button and, while it is open, the popover that names the new branch: a text field
/// (Enter submits) and a `Create` button. Esc or a click outside closes it.
fn branch_button(app: &AppState, live: bool, cx: &mut Context<AppState>) -> AnyElement {
    let open = live && app.workbench.open_menu == Some(MenuId::GitNewBranch);
    div()
        .relative()
        .flex_none()
        .child(icon_button_tip(
            "git-branch-new",
            UbiqIcon::GitBranchNew,
            "New branch",
            live,
            cx.listener(|this, _, window, cx| this.open_git_branch_popover(window, cx)),
        ))
        .when(open, |this| {
            this.child(
                deferred(
                    anchored()
                        // Dropped by the strip's height, so it opens under the row.
                        .offset(point(px(0.), px(theme::titlebar_height())))
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id("git-branch-panel")
                                .w(px(BRANCH_PANEL_WIDTH))
                                .p_2()
                                .flex()
                                .items_center()
                                .gap_2()
                                .occlude()
                                .bg(theme::surface_raised())
                                .border_l(px(theme::accent_edge()))
                                .border_color(theme::accent())
                                .shadow_lg()
                                // The same stop the repo selector makes: the panel is painted
                                // over the strip, and a click on it must not reach the button.
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_mouse_down_out(
                                    cx.listener(|this, _, _, cx| this.close_menu(cx)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .child(Input::new(&app.git_branch_name).appearance(false)),
                                )
                                .child(primary_button(
                                    "git-branch-create",
                                    None,
                                    "Create",
                                    cx.listener(|this, _, window, cx| {
                                        this.submit_git_branch(window, cx)
                                    }),
                                )),
                        ),
                )
                .priority(MENU_LAYER),
            )
        })
        .into_any_element()
}
