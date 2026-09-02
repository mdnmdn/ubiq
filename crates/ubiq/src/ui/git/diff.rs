//! The comparison under the history: what the selected path actually changed.
//!
//! The hunks are the host's — the same `DiffProjectFile` the editor's diff tabs ask for — and they
//! are drawn by the same renderer, [`crate::ui::viewer::diff`], so a change reads identically
//! whether it is opened as a tab or picked from the list beside this. No diff library entered the
//! interface to make either.
//!
//! The pane shuts rather than the history shrinking: the history is the screen's subject, and a
//! reader who wants the whole log wants the whole height for it.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::editor::ViewLayout;
use crate::state::git::DIFF_HEIGHT;
use crate::theme;
use crate::ui::kit::{choice_pill, icon_button, mono};
use crate::ui::viewer;

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };

    let head = header(app, cx);
    if !git.diff_open {
        return div()
            .flex()
            .flex_none()
            .flex_col()
            .border_t_1()
            .border_color(theme::border())
            .child(head)
            .into_any_element();
    }

    let body = match (git.path(), &git.diff) {
        (None, _) => viewer::note("Pick a changed path to compare it", theme::text_faint()),
        (Some(_), None) => viewer::note("Reading\u{2026}", theme::text_faint()),
        (Some(_), Some(diff)) => viewer::diff::render(
            diff,
            if git.split {
                ViewLayout::Split
            } else {
                ViewLayout::Preview
            },
        ),
    };

    div()
        .h(px(DIFF_HEIGHT))
        .flex()
        .flex_none()
        .flex_col()
        .bg(theme::pane_bg())
        .border_t_1()
        .border_color(theme::border())
        .child(head)
        .child(div().flex().flex_1().min_h(px(0.)).child(body))
        .into_any_element()
}

/// What is being compared, and how it is drawn. The chevron is what shuts the pane.
fn header(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(git) = app.git_view(cx) else {
        return div().into_any_element();
    };
    let path = git.path().unwrap_or("no path selected").to_string();

    div()
        .h(px(32.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::pane_bg())
        .child(mono(path, theme::text_muted()).text_size(px(11.5)))
        .child(div().flex_1().min_w(px(0.)))
        .child(choice_pill(
            "git-diff-split",
            "split",
            git.split,
            cx.listener(|this, _, _, cx| this.set_git_split(true, cx)),
        ))
        .child(choice_pill(
            "git-diff-unified",
            "unified",
            !git.split,
            cx.listener(|this, _, _, cx| this.set_git_split(false, cx)),
        ))
        .child(icon_button(
            "git-diff-open",
            if git.diff_open {
                IconName::ChevronDown
            } else {
                IconName::ChevronUp
            },
            false,
            cx.listener(|this, _, _, cx| this.toggle_git_diff_pane(cx)),
        ))
        .into_any_element()
}
