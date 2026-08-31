//! The project picker.
//!
//! Richer than the shared `Picker`, because a project is not just a value: it is open or only
//! remembered, it can be closed, and it can be sent to a window of its own.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::{AppState, open_project_window};
use crate::state::MenuId;
use crate::theme;
use crate::ui::kit::{mono, section_label};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let wb = &app.workbench;
    let colour = theme::project_colour(wb.project_colour());
    let open = wb.open_menu == Some(MenuId::Project);

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
        .cursor_pointer()
        .child(wb.project_name().to_string())
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
    let mut open_rows: Vec<AnyElement> = Vec::new();
    for (index, _) in app.workbench.filtered(true) {
        open_rows.push(row(app, index, true, cx));
    }

    let mut recent_rows: Vec<AnyElement> = Vec::new();
    for (index, _) in app.workbench.filtered(false) {
        recent_rows.push(row(app, index, false, cx));
    }

    let mut body = div()
        .w(px(340.))
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::project_colour(app.workbench.project_colour()))
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

    if !open_rows.is_empty() {
        body = body
            .child(div().px_2().pt_2().pb_1().child(section_label("Open")))
            .children(open_rows);
    }

    if !recent_rows.is_empty() {
        body = body
            .child(div().px_2().pt_2().pb_1().child(section_label("Recent")))
            .children(recent_rows);
    }

    deferred(
        anchored()
            .snap_to_window_with_margin(px(8.))
            .child(body.on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_menu(cx)))),
    )
    .priority(1)
}

fn row(app: &AppState, index: usize, is_open: bool, cx: &mut Context<AppState>) -> AnyElement {
    let project = &app.workbench.projects[index];
    let colour = theme::project_colour(project.colour);
    let is_current = app.workbench.project == index;

    if app.workbench.pending_close == Some(index) {
        return confirm_row(project.terminals, index, cx);
    }

    let mut line = div()
        .id(ElementId::Name(format!("project-{index}").into()))
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
                        .child(project.name.clone()),
                )
                .child(mono(project.path.clone(), theme::text_faint()).text_size(px(10.5))),
        );

    if !is_open {
        line = line.child(mono(project.when.clone(), theme::text_faint()).text_size(px(10.5)));
    }

    line = line.child(action(
        format!("project-window-{index}"),
        IconName::ExternalLink,
        cx.listener(move |_, _, _, cx| open_project_window(index, cx)),
    ));

    if is_open {
        line = line.child(action(
            format!("project-close-{index}"),
            IconName::Close,
            cx.listener(move |this, _, _, cx| this.close_project(index, false, cx)),
        ));
    }

    line.on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
        .into_any_element()
}

/// Closing a project with terminals running is a question, not a click.
fn confirm_row(terminals: usize, index: usize, cx: &mut Context<AppState>) -> AnyElement {
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
            cx.listener(move |this, _, _, cx| this.close_project(index, true, cx)),
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
