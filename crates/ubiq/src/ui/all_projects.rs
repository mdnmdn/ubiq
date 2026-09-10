//! The "All projects" modal.
//!
//! The picker's History group shows nine rows and hands off the rest here — a menu is a glance,
//! not a browse, and past nine it stops being faster than opening a searchable list. This is that
//! list: every project the catalogue holds, filtered by name or path, each row carrying the same
//! three actions the picker's own History rows do.
//!
//! Its own search field rather than a share of the picker's: the two can be on screen at once —
//! this modal is raised over the row that opened it — and a field drawn twice is a field in two
//! places.

use gpui::{
    AnyElement, Context, ElementId, Focusable, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::ProjectSnapshot;

use crate::app::{AppState, open_project_window};
use crate::state::WindowRegistry;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{field, modal_sized, mono};
use crate::ui::project_menu::{action, path_tail};

/// Wide enough for a name and its folder on the same line, and tall enough that the list reads
/// as a list rather than a sliver — the modal's whole point is browsing more than the picker's
/// History group can hold.
const MODAL_WIDTH: f32 = 480.0;
const MODAL_HEIGHT: f32 = 520.0;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(state) = app.workbench.all_projects.as_ref() else {
        return div().into_any_element();
    };
    let filter = state.filter.clone();
    let registry = WindowRegistry::read(cx);
    let here = app.project_groups(cx).here;
    let projects: Vec<ProjectSnapshot> = registry
        .all_matching(&filter)
        .into_iter()
        .filter_map(|id| registry.project(id).cloned())
        .collect();

    let focused = app
        .all_projects_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let body = div()
        .id("all-projects-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .gap_2()
        .child(
            field(theme::accent(), focused)
                .h(px(34.))
                .flex_none()
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
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .child(Input::new(&app.all_projects_search).appearance(false)),
                ),
        )
        .child(
            div()
                .id("all-projects-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .children(
                    projects
                        .iter()
                        .map(|entry| row(entry, here.contains(&entry.record.id), cx)),
                )
                .children((projects.is_empty()).then(|| {
                    div()
                        .px_3()
                        .py_4()
                        .text_size(theme::font(Family::Chrome, Role::Body))
                        .text_color(theme::text_muted())
                        .child("No projects match.")
                        .into_any_element()
                })),
        )
        .into_any_element();

    let view = cx.entity();
    modal_sized(
        "all-projects-modal",
        theme::accent(),
        MODAL_WIDTH,
        Some(MODAL_HEIGHT),
        "All projects",
        body,
        div().into_any_element(),
        crate::ui::handler(&view, |this, _, cx| this.close_all_projects(cx)),
        window,
    )
}

fn row(entry: &ProjectSnapshot, in_this_window: bool, cx: &mut Context<AppState>) -> AnyElement {
    let project: ProjectId = entry.record.id;
    let colour = theme::project_tint(
        entry.record.temporary,
        entry.record.colour,
        entry.record.custom_colour,
    );
    let full_path = entry.record.path.clone();
    let tail = path_tail(&full_path);
    let tooltip_name = full_path.clone();
    let tooltip_path = full_path.clone();

    div()
        .id(ElementId::Name(format!("all-projects-{project}").into()))
        .h(px(34.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .hover(|this| this.bg(theme::hover()))
        .child(div().size(px(8.)).flex_none().rounded_full().bg(colour))
        .child(
            div()
                .id(ElementId::Name(
                    format!("all-projects-name-{project}").into(),
                ))
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Body))
                .text_color(theme::text())
                .truncate()
                .child(entry.record.name.clone())
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip_name.clone()).build(window, cx)
                }),
        )
        .child(
            div()
                .id(ElementId::Name(
                    format!("all-projects-path-{project}").into(),
                ))
                .flex_none()
                .child(
                    mono(tail, theme::text_faint())
                        .text_size(theme::font(Family::Chrome, Role::Micro)),
                )
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(tooltip_path.clone()).build(window, cx)
                }),
        )
        .child(action(
            format!("all-projects-open-here-{project}"),
            IconName::ArrowLeft,
            "Open in this window",
            cx.listener(move |this, _, _, cx| {
                if in_this_window {
                    this.activate_project(project, cx);
                } else {
                    this.take_project(project, cx);
                }
                this.close_all_projects(cx);
            }),
        ))
        .child(action(
            format!("all-projects-open-window-{project}"),
            IconName::ExternalLink,
            "Open in a new window",
            cx.listener(move |this, _, _, cx| {
                open_project_window(Some(project), cx);
                this.close_all_projects(cx);
            }),
        ))
        .child(action(
            format!("all-projects-forget-{project}"),
            IconName::Delete,
            "Forget",
            cx.listener(move |this, _, _, cx| this.forget_project(project, cx)),
        ))
        .into_any_element()
}
