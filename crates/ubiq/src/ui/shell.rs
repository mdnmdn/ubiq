//! The window's skeleton: titlebar, rail, the three resizable panels, and the status bar.
//!
//! Only IDE mode fills the middle. The chat panel is the one piece of furniture that survives a
//! rail-mode switch, so it sits outside the mode branch.

use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::resizable::{h_resizable, resizable_panel, v_resizable};

use crate::app::AppState;
use crate::theme;
use crate::ui::{chat, editor, empty, explorer, rail, status_bar, terminal, titlebar};

pub fn render(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let wb = &app.workbench;
    let ide = wb.is_ide();

    let centre = if ide {
        v_resizable("workbench-v")
            .child(resizable_panel().child(editor::render(app, cx).into_any_element()))
            .child(
                resizable_panel()
                    .size(px(theme::DOCK_HEIGHT))
                    .size_range(px(theme::DOCK_MIN)..px(theme::DOCK_MAX))
                    .visible(wb.show_bottom)
                    .child(terminal::render(app, cx).into_any_element()),
            )
            .into_any_element()
    } else {
        empty::empty_page(
            wb.rail_mode.label(),
            wb.rail_mode.note(),
            rail::mode_icon(wb.rail_mode),
        )
        .into_any_element()
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(theme::app_bg())
        .text_color(theme::text())
        // The window wears its project's colour down its whole left edge.
        .border_l(px(theme::ACCENT_EDGE * 2.0))
        .border_color(theme::project_colour(wb.project_colour()))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .child(
                    // The mark sits above the rail, so the two read as one column.
                    div().w(px(theme::RAIL_WIDTH)).flex_none(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(titlebar::render(app, cx)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .child(rail::render(app, cx))
                .child(
                    div().flex_1().min_w(px(0.)).child(
                        h_resizable("workbench-h")
                            .child(
                                resizable_panel()
                                    .size(px(theme::EXPLORER_WIDTH))
                                    .size_range(px(theme::EXPLORER_MIN)..px(theme::EXPLORER_MAX))
                                    .visible(ide && wb.show_left)
                                    .child(explorer::render(app, cx).into_any_element()),
                            )
                            .child(resizable_panel().child(centre))
                            .child(
                                resizable_panel()
                                    .size(px(theme::CHAT_WIDTH))
                                    .size_range(px(theme::CHAT_MIN)..px(theme::CHAT_MAX))
                                    .visible(ide && wb.show_right)
                                    .child(chat::render(app, cx).into_any_element()),
                            ),
                    ),
                ),
        )
        .child(status_bar::render(app, cx))
}
