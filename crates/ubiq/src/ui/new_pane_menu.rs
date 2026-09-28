//! The new-pane control's chevron menu: which shell a pane runs, and the console.
//!
//! The "+" itself opens the platform's default shell and needs no menu. This is what else can be
//! started here — every shell the host found on the machine, the default marked — painted over the
//! window for the reason [`super::tab_menu`] is: the dock's skin does not name `AppState`, so it
//! says a menu was wanted and the window draws it.
//!
//! **The rows are data**: `ui::menus::new_pane`, through the `ext::menu` container — any pane
//! still running with no panel drawing it, then the shells, then the console. A runnable tool is
//! not a row here (`super::run_tool_menu` is where a project's tools are read at once), and
//! neither is a harness: starting an agent is the New agent form's job.

use gpui::{Context, IntoElement, Window, div};

use crate::app::AppState;
use crate::ext::ids;
use crate::ui::menus;

/// Draw the open new-pane menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.new_pane_menu else {
        return div().into_any_element();
    };
    menus::overlay(
        &cx.entity(),
        "new-pane-menu",
        at,
        menus::entries(ids::MENU_NEW_PANE, app, cx),
        |this, _, cx| this.dismiss_new_pane_menu(cx),
    )
}
