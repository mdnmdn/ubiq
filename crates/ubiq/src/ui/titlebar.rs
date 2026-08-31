//! The top row: what is open, where it lives, and the switches for the panels around it.

use gpui::{Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::MenuId;
use crate::theme;
use crate::ui::kit::{Picker, icon_button, mono};
use crate::ui::{handler, indexed, project_menu};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let view = cx.entity();
    let wb = &app.workbench;

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
        .child(project_menu::render(app, cx))
        .child(
            Picker::new("branch-picker", wb.branch_name().to_string())
                .icon(IconName::Network)
                .items(&wb.branches)
                .selected(wb.branch)
                .open(wb.open_menu == Some(MenuId::Branch))
                .on_toggle(handler(&view, |this, _, cx| {
                    this.open_menu(MenuId::Branch, cx)
                }))
                .on_pick(indexed(&view, |this, index, _, cx| {
                    this.workbench.branch = index;
                    this.close_menu(cx);
                }))
                .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
        )
        .child(div().flex_1().min_w(px(0.)))
        .child(command_field(app))
        .child(div().flex_1().min_w(px(0.)))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .child(icon_button(
                    "toggle-left",
                    IconName::PanelLeft,
                    wb.show_left,
                    cx.listener(|this, _, _, cx| {
                        this.workbench.show_left = !this.workbench.show_left;
                        this.remember_view(cx);
                        cx.notify();
                    }),
                ))
                .child(icon_button(
                    "toggle-bottom",
                    IconName::PanelBottom,
                    wb.show_bottom,
                    cx.listener(|this, _, _, cx| {
                        this.workbench.show_bottom = !this.workbench.show_bottom;
                        this.remember_view(cx);
                        cx.notify();
                    }),
                ))
                .child(icon_button(
                    "toggle-right",
                    IconName::PanelRight,
                    wb.show_right,
                    cx.listener(|this, _, _, cx| {
                        this.workbench.show_right = !this.workbench.show_right;
                        this.remember_view(cx);
                        cx.notify();
                    }),
                ))
                .child(
                    div()
                        .w(px(1.))
                        .h(px(18.))
                        .mx_1()
                        .flex_none()
                        .bg(theme::border()),
                )
                .child(icon_button("search", IconName::Search, false, |_, _, _| {}))
                .child(icon_button("bell", IconName::Bell, false, |_, _, _| {}))
                .child(icon_button(
                    "settings",
                    IconName::Settings2,
                    false,
                    |_, _, _| {},
                ))
                .child(icon_button(
                    "theme",
                    if app.workbench.theme_id == crate::theme::ThemeId::Dark {
                        IconName::Sun
                    } else {
                        IconName::Moon
                    },
                    false,
                    cx.listener(|this, _, _, cx| this.toggle_theme(cx)),
                )),
        )
}

/// The middle of the titlebar: one field for finding a file and for running a command.
fn command_field(app: &AppState) -> impl IntoElement {
    div()
        .w(px(420.))
        .h(px(28.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
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
                .child(Input::new(&app.command_input).appearance(false)),
        )
        .child(mono("\u{2318}K", theme::text_faint()).text_size(px(10.5)))
}
