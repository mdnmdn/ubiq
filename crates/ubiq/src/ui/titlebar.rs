//! The top row: what is open, where it lives, and the switches for the dock's three edge regions.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, Focusable, IntoElement, ParentElement, StatefulInteractiveElement as _, Styled,
    Window, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::{field, icon_button, mono};
use crate::ui::project_menu;

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    // The three switches report the dock's own regions rather than a flag beside it: a region the
    // user collapsed by dragging its last panel out has to read as closed here too.
    let (left, bottom, right) = app.regions_open(cx);
    // A pane runs in a project's folder, so with none open there is nothing a new one could be
    // started in and the action is not offered.
    let has_project = app.project(cx).is_some();
    // A temporary project (dropped in from outside the catalogue) offers "keep" rather than
    // settings — there is nothing to rename or recolour yet, only a decision to make it real.
    let temporary = app
        .project_snapshot(cx)
        .is_some_and(|snapshot| snapshot.record.temporary);

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
        // The window's letter sits before the picker rather than inside it: one says which
        // window, the other which project.
        .children(project_menu::window_badge(app, cx))
        .child(project_menu::render(app, window, cx))
        .when(has_project, |this| {
            let (icon, label) = if temporary {
                (IconName::Plus, "Keep this project")
            } else {
                (IconName::EllipsisVertical, "Project settings")
            };
            this.child(
                icon_button(
                    "project-settings",
                    icon,
                    app.workbench.project_settings.is_some(),
                    cx.listener(|this, _, _, cx| this.open_edit_project(cx)),
                )
                .h_full()
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(label).build(window, cx)
                }),
            )
        })
        .child(div().flex_1().min_w(px(0.)))
        .child(command_field(app, window, cx))
        .child(div().flex_1().min_w(px(0.)))
        .child(
            div()
                .h_full()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                // The side regions are IDE furniture: in any other rail mode they are disabled, so
                // their switches are not offered. The bottom region stays openable in every mode.
                .when(app.workbench.is_ide(), |this| {
                    this.child(
                        icon_button(
                            "toggle-left",
                            IconName::PanelLeft,
                            left,
                            cx.listener(|this, _, window, cx| {
                                this.toggle_region(crate::state::Region::Left, window, cx)
                            }),
                        )
                        .h_full(),
                    )
                })
                .child(
                    icon_button(
                        "toggle-bottom",
                        IconName::PanelBottom,
                        bottom,
                        cx.listener(|this, _, window, cx| {
                            this.toggle_region(crate::state::Region::Bottom, window, cx)
                        }),
                    )
                    .h_full(),
                )
                .when(app.workbench.is_ide(), |this| {
                    this.child(
                        icon_button(
                            "toggle-right",
                            IconName::PanelRight,
                            right,
                            cx.listener(|this, _, window, cx| {
                                this.toggle_region(crate::state::Region::Right, window, cx)
                            }),
                        )
                        .h_full(),
                    )
                })
                .child(
                    div()
                        .w(px(1.))
                        .h_full()
                        .mx_1()
                        .flex_none()
                        .bg(theme::border()),
                )
                .child(
                    icon_button(
                        "search",
                        IconName::Search,
                        false,
                        cx.listener(|this, _, window, cx| this.reveal_search(window, cx)),
                    )
                    .h_full(),
                )
                .child(icon_button("bell", IconName::Bell, false, |_, _, _| {}).h_full())
                .when(has_project, |this| {
                    this.child(
                        icon_button(
                            "web-export",
                            IconName::Globe,
                            false,
                            cx.listener(|this, _, window, cx| this.open_web_export(window, cx)),
                        )
                        .h_full()
                        .tooltip(move |window, cx| {
                            gpui_component::tooltip::Tooltip::new("Open in browser")
                                .build(window, cx)
                        }),
                    )
                })
                .child(
                    icon_button(
                        "settings",
                        IconName::Settings2,
                        app.workbench.settings.open,
                        cx.listener(|this, _, _, cx| this.toggle_settings(cx)),
                    )
                    .h_full()
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new("Settings").build(window, cx)
                    }),
                )
                .child(
                    icon_button(
                        "theme",
                        if app.workbench.theme_id == crate::theme::ThemeId::Dark {
                            IconName::Sun
                        } else {
                            IconName::Moon
                        },
                        false,
                        cx.listener(|this, _, _, cx| this.toggle_theme(cx)),
                    )
                    .h_full(),
                ),
        )
}

/// The middle of the titlebar: one field for finding a file and for running a command.
fn command_field(app: &AppState, window: &Window, cx: &App) -> impl IntoElement {
    let focused = app
        .command_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    field(theme::border(), focused)
        .w(px(420.))
        .h_full()
        .px_2()
        .flex_none()
        .gap_2()
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
                .child(Input::new(&app.command_input).appearance(false)),
        )
        .child(mono("\u{2318}K", theme::text_faint()).text_size(px(10.5)))
}
