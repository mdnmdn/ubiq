//! The new-pane control's chevron menu: which shell a pane runs, and the console.
//!
//! The "+" itself opens the platform's default shell and needs no menu. This is what else can be
//! started here — every shell the host found on the machine, the default one marked — painted over
//! the window for the reason [`super::file_tab_menu`] is: the dock's skin does not name
//! `AppState`, so it says a menu was wanted and the window draws it.
//!
//! The rows are the shells in the order the host listed them, then a separator, then the console.
//! The index a row is picked at is its index in that list — the separator included, because it is a
//! row — which is what `AppState::pick_new_pane_menu` matches on: keep the two in step.

use gpui::{Context, IntoElement, SharedString, Window, div, point, px};

use crate::app::AppState;
use crate::state::NewPaneRow;
use crate::ui::{self, kit};

/// The trailing row, which is not a shell: it brings the console back on screen.
pub const CONSOLE_ROW: &str = "Logs";

/// Draw the open new-pane menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.new_pane_menu else {
        return div().into_any_element();
    };

    let has_project = app.project(cx).is_some();
    let items: Vec<_> = app
        .workbench
        .new_pane_rows(has_project)
        .into_iter()
        .map(|row| match row {
            NewPaneRow::Shell(shell) => {
                let shell = &app.workbench.shells[shell];
                // The default is marked rather than reordered: the row a bare click on "+"
                // already runs has to be findable, and the host's order is the one shown.
                let label = if shell.is_default {
                    format!("{} (default)", shell.label)
                } else {
                    shell.label.clone()
                };
                kit::ContextItem::new(SharedString::from(label))
            }
            // The console is not a pane, and the line says so: everything above it starts
            // something.
            NewPaneRow::Separator => kit::ContextItem::separator(),
            NewPaneRow::Console => kit::ContextItem::new(CONSOLE_ROW),
        })
        .collect();

    kit::context_menu(
        "new-pane-menu",
        point(px(at.0), px(at.1)),
        items,
        ui::indexed(&cx.entity(), |this, index, window, cx| {
            this.pick_new_pane_menu(index, window, cx);
        }),
        ui::handler(&cx.entity(), |this, _, cx| this.dismiss_new_pane_menu(cx)),
    )
    .into_any_element()
}
