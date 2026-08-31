//! The project picker.
//!
//! Richer than the shared `Picker`, because a project is not just a value: it is open in some
//! window or only remembered, it can be closed, it can be sent to a window of its own, and it can
//! be taken from the window that holds it.
//!
//! Three groups, top to bottom — open in this window, open in another window, history — so the
//! picker is the one place that answers "where is everything I have open?". A row moves between
//! groups as the project moves between windows; the registry behind that is
//! `crates/ubiq/src/state/windows.rs`.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, WindowId, anchored, deferred, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::{AppState, focus_window, open_project_window};
use crate::state::MenuId;
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
    let colour = theme::project_colour(app.project_colour(cx));
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

fn panel(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let groups = app.project_groups(cx);

    let mut body = div()
        .w(px(340.))
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::project_colour(app.project_colour(cx)))
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

    let here_label = app.window_label(cx);
    body = group(
        body,
        &format!("This window · {here_label}"),
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

    deferred(
        anchored()
            .snap_to_window_with_margin(px(8.))
            .child(body.on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))),
    )
    .priority(1)
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

fn row(app: &AppState, project: usize, group: Group, cx: &mut Context<AppState>) -> AnyElement {
    let registry = crate::state::WindowRegistry::read(cx);
    let Some(entry) = registry.project(project) else {
        return div().into_any_element();
    };
    let (name, path, when, terminals) = (
        entry.name.clone(),
        entry.path.clone(),
        entry.when.clone(),
        entry.terminals,
    );
    let colour = theme::project_colour(entry.colour);
    let is_current = group == Group::Here && app.project(cx) == project;

    if app.workbench.pending_close == Some(project) && group == Group::Here {
        return confirm_row(terminals, project, cx);
    }

    let mut line = div()
        .id(ElementId::Name(format!("project-{project}").into()))
        .h(px(38.))
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
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(if is_current {
                            theme::text()
                        } else {
                            theme::text_muted()
                        })
                        .child(name),
                )
                .child(mono(path, theme::text_faint()).text_size(px(10.5))),
        );

    // Every open project prints the window holding it; a remembered one prints how long ago.
    line = match group {
        Group::Here => line.child(window_mark(app.window_label(cx), colour, false)),
        Group::Elsewhere(label, _) => line.child(window_mark(label, theme::text_muted(), false)),
        Group::History => line.child(mono(when, theme::text_faint()).text_size(px(10.5))),
    };

    // Take it from the window that holds it: the one action a project in another window needs.
    if matches!(group, Group::Elsewhere(..)) {
        line = line.child(action(
            format!("project-take-{project}"),
            IconName::ArrowLeft,
            cx.listener(move |this, _, _, cx| this.take_project(project, cx)),
        ));
    }

    // Send it to a window of its own. It leaves this window, and this window closes if it held
    // nothing else.
    if !matches!(group, Group::Elsewhere(..)) {
        line = line.child(action(
            format!("project-window-{project}"),
            IconName::ExternalLink,
            cx.listener(move |_, _, _, cx| open_project_window(project, cx)),
        ));
    }

    if group == Group::Here {
        line = line.child(action(
            format!("project-close-{project}"),
            IconName::Close,
            cx.listener(move |this, _, _, cx| this.close_project(project, false, cx)),
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
fn confirm_row(terminals: usize, project: usize, cx: &mut Context<AppState>) -> AnyElement {
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
                    "{terminals} terminal{} still running. Close anyway?",
                    if terminals == 1 { "" } else { "s" }
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
