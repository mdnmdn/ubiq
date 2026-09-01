//! The project picker.
//!
//! Richer than the shared `Picker`, because a project is not just a value: it is open in some
//! window or only remembered, it can be closed, forgotten, and — when its folder has gone —
//! pointed somewhere else. Rename and recolour live in project settings, next to the title chip.
//!
//! Three groups, top to bottom — open in this window, open in another window, history — so the
//! picker is the one place that answers "where is everything I have open?". A row moves between
//! groups as the project moves between windows; the projection behind that is
//! `crates/ubiq/src/state/windows.rs`, and the catalogue it projects belongs to the host.

use chrono::Utc;
use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, WindowId, anchored, deferred, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::ProjectSnapshot;

use crate::app::{AppState, focus_window, open_project_window};
use crate::state::{MenuId, RowAction, when};
use crate::theme;
use crate::ui::kit::{mono, section_label};

/// Where a row sits, which is what decides the actions it carries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    /// Open in this window.
    Here,
    /// Open in another window, named by its letter.
    Elsewhere(char, WindowId),
    /// Not open anywhere.
    History,
}

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let colour = app.project_tint(cx);
    let open = app.workbench.open_menu == Some(MenuId::Project);
    let label = app.window_label(cx);
    let name = app.project_name(cx);

    let mut trigger = div()
        .id("project-picker")
        .relative()
        .h(px(26.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        // The project's own colour, filled: the window says which project it is at a glance.
        .bg(colour)
        .text_color(theme::on_accent())
        .text_size(px(13.))
        // The window's letter, so the user knows which window they are typing into.
        .child(window_mark(label, theme::on_accent(), true))
        .child(name)
        .child(
            Icon::new(IconName::ChevronDown)
                .with_size(Size::XSmall)
                .text_color(theme::on_accent()),
        )
        .on_click(cx.listener(|this, _, _, cx| this.open_menu(MenuId::Project, cx)));

    if open {
        trigger = trigger.child(panel(app, cx));
    }

    trigger
}

fn panel(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let groups = app.project_groups(cx);
    let empty = crate::state::WindowRegistry::read(cx).is_empty();

    let mut body = div()
        .w(px(340.))
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(app.project_tint(cx))
        .shadow_lg()
        .child(
            div()
                .h(px(34.))
                .px_2()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    Icon::new(IconName::Search)
                        .with_size(Size::XSmall)
                        .text_color(theme::text_faint()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(12.5))
                        .child(Input::new(&app.project_search).appearance(false)),
                ),
        );

    if let Some(error) = &app.workbench.project_error {
        body = body.child(banner(error.clone(), cx));
    }

    if empty {
        // A first run has nothing to list, and saying so is better than three empty headings.
        body = body.child(
            div()
                .px_3()
                .py_4()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child("No projects yet."),
        );
    } else {
        let here_label = app.window_label(cx);
        body = group(
            body,
            &format!("This window \u{b7} {here_label}"),
            groups
                .here
                .iter()
                .map(|project| row(app, *project, Group::Here, cx))
                .collect(),
        );

        body = group(
            body,
            "Other windows",
            groups
                .elsewhere
                .iter()
                .map(|(project, label, id)| row(app, *project, Group::Elsewhere(*label, *id), cx))
                .collect(),
        );

        body = group(
            body,
            "History",
            groups
                .history
                .iter()
                .map(|project| row(app, *project, Group::History, cx))
                .collect(),
        );
    }

    // Always drawn, because on a first run it is the only thing there is to do.
    body = body.child(add_row(cx));

    deferred(
        anchored()
            .snap_to_window_with_margin(px(8.))
            .child(body.on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))),
    )
    .priority(1)
    .into_any_element()
}

/// A heading and its rows, or nothing at all. An empty group is not drawn: the filter would
/// otherwise leave three headings over one row.
fn group(body: gpui::Div, label: &str, rows: Vec<AnyElement>) -> gpui::Div {
    if rows.is_empty() {
        return body;
    }
    body.child(div().px_2().pt_2().pb_1().child(section_label(label)))
        .children(rows)
}

fn row(app: &AppState, project: ProjectId, group: Group, cx: &mut Context<AppState>) -> AnyElement {
    let registry = crate::state::WindowRegistry::read(cx);
    let Some(entry) = registry.project(project) else {
        return div().into_any_element();
    };
    let entry: ProjectSnapshot = entry.clone();
    let colour = theme::project_colour(entry.record.colour);
    let is_current = group == Group::Here && app.project(cx) == Some(project);

    // One row at a time expands into a Forget confirmation, and it takes the row's place while
    // it is open. Rename and recolour live in project settings, not on the row.
    if let Some((id, RowAction::ConfirmForget)) = app.workbench.row_action
        && id == project
    {
        return forget_row(&entry, project, cx);
    }

    if app.workbench.pending_close == Some(project) && group == Group::Here {
        return confirm_row(entry.open_panes, project, cx);
    }

    let healthy = entry.health.is_ok();
    let full_path = entry.record.path.clone();
    let path_colour = if healthy {
        theme::text_faint()
    } else {
        theme::warning()
    };

    let mut line = div()
        .id(ElementId::Name(format!("project-{project}").into()))
        .h(px(30.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(div().size(px(8.)).flex_none().rounded_full().bg(colour))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.5))
                .text_color(if is_current {
                    theme::text()
                } else {
                    theme::text_muted()
                })
                .truncate()
                .child(entry.record.name.clone()),
        )
        .child(
            div()
                .id(ElementId::Name(format!("project-path-{project}").into()))
                .flex_none()
                .child(mono(path_tail(&full_path), path_colour).text_size(px(10.5)))
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(full_path.clone()).build(window, cx)
                }),
        );

    // A folder that is not there is marked, never quietly repaired and never removed.
    if !healthy {
        line = line.child(
            Icon::new(IconName::TriangleAlert)
                .with_size(Size::XSmall)
                .text_color(theme::warning()),
        );
    }

    // Every open project prints the window holding it; a remembered one prints how long ago.
    line = match group {
        Group::Here => line.child(window_mark(app.window_label(cx), colour, false)),
        Group::Elsewhere(label, _) => line.child(window_mark(label, theme::text_muted(), false)),
        Group::History => line.child(
            // Rendered now rather than stored: how long ago something was is a fact about the
            // moment it is drawn.
            mono(
                when::relative_opt(entry.record.last_opened_at, Utc::now()),
                theme::text_faint(),
            )
            .text_size(px(10.5)),
        ),
    };

    // A project whose folder has gone offers the two actions that can help.
    if !healthy {
        line = line.child(action(
            format!("project-locate-{project}"),
            IconName::FolderOpen,
            "Locate",
            cx.listener(move |this, _, _, cx| this.choose_folder(Some(project), cx)),
        ));
    }

    // Take it from the window that holds it: the one action a project in another window needs.
    if matches!(group, Group::Elsewhere(..)) {
        line = line.child(action(
            format!("project-take-{project}"),
            IconName::ArrowLeft,
            "Take into this window",
            cx.listener(move |this, _, _, cx| this.take_project(project, cx)),
        ));
    }

    // Send it to a window of its own. It leaves this window, and this window closes if it held
    // nothing else.
    if !matches!(group, Group::Elsewhere(..)) {
        line = line.child(action(
            format!("project-window-{project}"),
            IconName::ExternalLink,
            "Open in a new window",
            cx.listener(move |_, _, _, cx| open_project_window(Some(project), cx)),
        ));
    }

    if group == Group::Here {
        line = line.child(action(
            format!("project-close-{project}"),
            IconName::Close,
            "Close in this window",
            cx.listener(move |this, _, _, cx| this.close_project(project, false, cx)),
        ));
    } else {
        line = line.child(action(
            format!("project-forget-{project}"),
            IconName::Delete,
            "Forget",
            cx.listener(move |this, _, _, cx| {
                this.set_row_action(Some((project, RowAction::ConfirmForget)), cx)
            }),
        ));
    }

    // Clicking the row does the obvious thing for where it sits: point this window at it, go to the
    // window that has it, or open it here.
    let click = cx.listener(move |this, _, _, cx| match group {
        Group::Here => this.activate_project(project, cx),
        Group::Elsewhere(_, id) => {
            this.close_menu(cx);
            focus_window(id, cx);
        }
        Group::History => this.take_project(project, cx),
    });

    line.on_click(click).into_any_element()
}

/// The last component of a path, with a leading `.../` when the path has a parent. A picker row
/// is one line; the full path is the tooltip.
pub(crate) fn path_tail(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rsplit_once(['/', '\\']) {
        Some((_, leaf)) if !leaf.is_empty() => format!(".../{leaf}"),
        _ => path.to_string(),
    }
}

/// Forgetting is not deleting, and the confirmation says so.
fn forget_row(
    entry: &ProjectSnapshot,
    project: ProjectId,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let name = entry.record.name.clone();
    div()
        .px_2()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::warning_soft())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::warning())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.))
                .text_color(theme::text())
                .child(format!(
                    "Forget {name}? Its view state goes; the folder is untouched."
                )),
        )
        .child(small_button(
            "forget-cancel",
            "Cancel",
            theme::text_muted(),
            cx.listener(|this, _, _, cx| this.set_row_action(None, cx)),
        ))
        .child(small_button(
            "forget-confirm",
            "Forget",
            theme::danger(),
            cx.listener(move |this, _, _, cx| this.forget_project(project, cx)),
        ))
        .into_any_element()
}

/// The always-present way in for a project Ubiq has never seen. It opens the platform's own folder
/// dialog rather than a browser of Ubiq's; the folder is not added until project settings confirms.
fn add_row(cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .id("project-add")
        .h(px(34.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .border_t_1()
        .border_color(theme::border())
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(
            Icon::new(IconName::Plus)
                .with_size(Size::XSmall)
                .text_color(theme::text_muted()),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(theme::text_muted())
                .child("Add a project\u{2026}"),
        )
        .on_click(cx.listener(|this, _, _, cx| this.choose_folder(None, cx)))
}

/// What the host last refused to do. Dismissible, because it is history the moment it is read.
fn banner(error: String, cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .px_2()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::danger_soft())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::danger())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(11.5))
                .text_color(theme::text())
                .child(error),
        )
        .child(small_button(
            "project-error-dismiss",
            "Dismiss",
            theme::text_muted(),
            cx.listener(|this, _, _, cx| this.dismiss_project_error(cx)),
        ))
}

/// A window's letter, in the small square the picker prints beside every open project.
fn window_mark(label: char, colour: gpui::Rgba, filled: bool) -> impl IntoElement {
    let mut mark = div()
        .size(px(16.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colour)
        .child(label.to_string());

    if !filled {
        mark = mark.border_1().border_color(colour);
    }

    mark
}

/// Closing a project with terminals running is a question, not a click.
fn confirm_row(panes: usize, project: ProjectId, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .px_2()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::warning_soft())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::warning())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.))
                .text_color(theme::text())
                .child(format!(
                    "{panes} terminal{} still running. Close anyway?",
                    if panes == 1 { "" } else { "s" }
                )),
        )
        .child(small_button(
            "confirm-cancel",
            "Cancel",
            theme::text_muted(),
            cx.listener(|this, _, _, cx| this.cancel_close(cx)),
        ))
        .child(small_button(
            "confirm-close",
            "Close",
            theme::danger(),
            cx.listener(move |this, _, _, cx| this.close_project(project, true, cx)),
        ))
        .into_any_element()
}

fn action(
    id: String,
    icon: IconName,
    tooltip: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(ElementId::Name(id.into()))
        .size(px(22.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tooltip).build(window, cx))
        .child(
            Icon::new(icon)
                .with_size(Size::XSmall)
                .text_color(theme::text_faint()),
        )
        .on_click(on_click)
}

fn small_button(
    id: &'static str,
    label: &'static str,
    colour: gpui::Rgba,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .h(px(22.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .bg(theme::surface())
        .text_size(px(11.5))
        .text_color(colour)
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .child(label)
        .on_click(on_click)
}

#[cfg(test)]
mod tests {
    use super::path_tail;

    #[test]
    fn a_nested_path_shows_the_leaf_with_a_leading_ellipsis() {
        assert_eq!(path_tail("/Users/mdn/works/ubiq"), ".../ubiq");
    }

    #[test]
    fn a_leaf_is_shown_whole() {
        assert_eq!(path_tail("ubiq"), "ubiq");
    }

    #[test]
    fn a_trailing_separator_does_not_invent_an_empty_leaf() {
        assert_eq!(path_tail("/Users/mdn/works/ubiq/"), ".../ubiq");
    }
}
