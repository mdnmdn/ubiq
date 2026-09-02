//! Application settings, over the window: a nav, a column of rows, a fixed-size panel.
//!
//! **Not the kit's one-question modal.** A settings page is worked in rather than answered, so it
//! follows the project-settings overlay: scrim, coloured left edge, left nav, scrolling body.
//! The size is fixed — switching sections must not resize the panel.

use gpui::{
    AnyElement, Context, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px,
};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::settings::{MarkdownOpen, SettingsSection};
use crate::theme;
use crate::ui::kit::{
    check_box, choice_pill, column, heading, icon_button, nav_item, primary_button, setting_row,
};

pub fn overlay(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> AnyElement {
    let viewport = window.viewport_size();
    let panel = dialog(app, window, cx).on_mouse_down_out(cx.listener(|this, _, _, cx| {
        this.close_settings(cx);
    }));

    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div()
                .id("app-settings")
                .w(viewport.width)
                .h(viewport.height)
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::scrim())
                .occlude()
                .child(panel),
        ),
    )
    .priority(2)
    .into_any_element()
}

fn dialog(
    app: &AppState,
    window: &Window,
    cx: &mut Context<AppState>,
) -> gpui::Stateful<gpui::Div> {
    let viewport = window.viewport_size();

    div()
        .id("app-settings-dialog")
        .w(px(theme::SETTINGS_WIDTH))
        .h(px(theme::SETTINGS_HEIGHT))
        .max_w(viewport.width)
        .max_h(viewport.height)
        .flex()
        .flex_col()
        .bg(theme::surface_raised())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::accent())
        .shadow_lg()
        .child(header(cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .child(nav(app, cx))
                .child(body(app, cx)),
        )
}

fn header(cx: &mut Context<AppState>) -> AnyElement {
    div()
        .h(px(52.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .child("Settings"),
        )
        .child(icon_button(
            "app-settings-close",
            IconName::Close,
            false,
            cx.listener(|this, _, _, cx| this.close_settings(cx)),
        ))
        .into_any_element()
}

fn nav(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let current = app.workbench.settings.nav;
    let items: Vec<AnyElement> = SettingsSection::all()
        .iter()
        .copied()
        .map(|item| {
            nav_item(
                ElementId::Name(format!("app-settings-nav-{}", item.label()).into()),
                nav_icon(item),
                item.label(),
                None,
                item == current,
                true,
                cx.listener(move |this, _, _, cx| this.set_settings_nav(item, cx)),
            )
        })
        .collect();

    div()
        .id("app-settings-nav")
        .w(px(220.))
        .flex()
        .flex_none()
        .flex_col()
        .gap_1()
        .px_2()
        .py_3()
        .bg(theme::pane_bg())
        .border_r_1()
        .border_color(theme::border())
        .children(items)
        .into_any_element()
}

fn nav_icon(item: SettingsSection) -> IconName {
    match item {
        SettingsSection::FileExplorer => IconName::Folder,
        SettingsSection::Editor => IconName::File,
        SettingsSection::Harnesses => IconName::Asterisk,
    }
}

fn body(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let content = match app.workbench.settings.nav {
        SettingsSection::FileExplorer => file_explorer(app, cx),
        SettingsSection::Editor => editor(app, cx),
        SettingsSection::Harnesses => harnesses(),
    };

    div()
        .id("app-settings-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .px_6()
        .py_5()
        .child(content)
        .into_any_element()
}

fn file_explorer(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let on = app.workbench.settings.ui.explorer_preview;
    column(vec![
        heading(
            "File explorer",
            "How a click on a file in the tree opens it.",
        ),
        setting_row(
            "Open files in previews",
            "A single click opens a temporary tab, replaced by the next preview. Double-click, \
             Shift-click and Shift-Enter open permanently either way.",
            check_box(
                "app-settings-explorer-preview",
                on,
                cx.listener(|this, _, _, cx| this.toggle_explorer_preview(cx)),
            )
            .into_any_element(),
        ),
    ])
}

fn editor(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let current = app.workbench.settings.ui.markdown_open;
    let pills: Vec<AnyElement> = MarkdownOpen::all()
        .iter()
        .copied()
        .map(|choice| {
            choice_pill(
                ElementId::Name(format!("app-settings-markdown-{}", choice.label()).into()),
                choice.label(),
                choice == current,
                cx.listener(move |this, _, _, cx| this.set_markdown_open(choice, cx)),
            )
            .into_any_element()
        })
        .collect();

    column(vec![
        heading(
            "Editor",
            "How a file opens. Already-open tabs keep the layout they were left in.",
        ),
        setting_row(
            "Markdown opens in",
            "New markdown files start in this layout. Source, preview and split remain available \
             on the tab.",
            div().flex().gap_1().children(pills).into_any_element(),
        ),
    ])
}

fn harnesses() -> AnyElement {
    column(vec![
        heading(
            "Harnesses",
            "Every agent runs on a harness. Register as many as you like — the same tool twice \
             with different credentials is normal, and each entry carries its own defaults.",
        ),
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(primary_button(
                "app-settings-add-harness",
                Some(IconName::Plus),
                "Add harness",
                |_, _, _| {},
            ))
            .into_any_element(),
        div()
            .px_3()
            .py_8()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::text_muted())
                    .child(SharedString::from("No harnesses registered.")),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::text_faint())
                    .child(SharedString::from(
                        "Definitions live with the harness library, not in this file.",
                    )),
            )
            .into_any_element(),
    ])
}
