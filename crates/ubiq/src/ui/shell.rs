//! The window's skeleton: titlebar, rail, the three resizable panels, and the status bar.
//!
//! Only IDE mode fills the middle. The chat panel is the one piece of furniture that survives a
//! rail-mode switch, so it sits outside the mode branch.
//!
//! A window holding no project keeps its frame and empties what a project would have filled. The
//! dock stays, because the log console is a tab in it and a window with nothing open is exactly
//! when the console is worth reaching.

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
    let has_project = app.project(cx).is_some();

    let centre = if ide {
        // With no project there is nothing to edit, and the empty page says so where the editor
        // would have been. The dock below it is unchanged: the console is reachable either way.
        let above = if has_project {
            editor::render(app, cx).into_any_element()
        } else {
            empty::no_project(cx)
        };

        // The group's state is the window's, not the frame's, so a dragged size can be read back
        // and remembered.
        v_resizable("workbench-v")
            .with_state(&app.centre)
            // A drag is remembered. The host debounces, so this may fire as freely as it likes.
            .on_resize(cx.listener(|this, _, _, cx| this.remember_view(cx)))
            .child(resizable_panel().child(above))
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
            Some(empty::not_built()),
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
        .border_color(app.project_tint(cx))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .child(rail::mark(app, cx))
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
                            .with_state(&app.columns)
                            .on_resize(cx.listener(|this, _, _, cx| this.remember_view(cx)))
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
                                    // The chat is a conversation about a project. With none open
                                    // it has nothing to be about, so it goes rather than sitting
                                    // there as the one populated panel on an empty screen.
                                    .visible(ide && wb.show_right && has_project)
                                    .child(chat::render(app, cx).into_any_element()),
                            ),
                    ),
                ),
        )
        .child(status_bar::render(app, cx))
}
