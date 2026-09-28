//! The titlebar's run chevron menu: every runnable tool this project offers, machine-wide rows
//! and the project's own together, in the order the host listed them.
//!
//! Painted over the window for the reason [`super::new_project_menu`] is: the titlebar does not
//! name `AppState` beyond what it already reads, and this is one more menu the window draws the
//! same way.
//!
//! **This is where a tool is reached now.** It used to be a group inside the new-pane menu, under
//! the shells; a tool is not a shell and a project's tools are the thing a user reaches for most
//! often, so they moved out to a control of their own beside the project's settings. The rows
//! themselves are `ui::menus::run_tool` — the same `ListedTool` list, the same `RunTool` behind a
//! pick.

use gpui::{Context, IntoElement, Window, div};

use crate::app::AppState;
use crate::ext::ids;
use crate::ui::menus;

/// Draw the open run menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.run_tool_menu else {
        return div().into_any_element();
    };
    menus::overlay(
        &cx.entity(),
        "run-tool-menu",
        at,
        menus::entries(ids::MENU_RUN_TOOL, app, cx),
        |this, _, cx| this.dismiss_run_tool_menu(cx),
    )
}
