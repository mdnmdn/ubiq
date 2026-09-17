//! The titlebar's run chevron menu: every runnable tool this project offers, machine-wide rows
//! and the project's own together, in the order the host listed them.
//!
//! Painted over the window for the reason [`super::new_project_menu`] is: the titlebar does not
//! name `AppState` beyond what it already reads, and this is one more menu the window draws the
//! same way. The rows are `WorkbenchState::run_tool_rows`, read again here exactly as
//! `AppState::pick_run_tool_menu` reads them — the rule every position-matched menu in this
//! window follows.
//!
//! **This is where a tool is reached now.** It used to be a group inside the new-pane menu,
//! under the shells; a tool is not a shell and a project's tools are the thing a user reaches
//! for most often, so they moved out to a control of their own beside the project's settings.
//! The rows themselves are unchanged — the same `ListedTool` list, the same `RunTool` behind a
//! pick.

use gpui::{Context, IntoElement, SharedString, Window, div, point, px};
use gpui_component::IconName;

use crate::app::AppState;
use crate::ui::{self, kit};

/// What the menu says when this host has no tool it could run here. Drawn disabled, the way
/// `ui::new_pane_menu`'s detached heading is: a menu that opened onto nothing at all would read
/// as a control that did not work.
pub const EMPTY_ROW: &str = "No tools for this project";

/// Draw the open run menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.run_tool_menu else {
        return div().into_any_element();
    };

    let rows = app.workbench.run_tool_rows();
    let items: Vec<_> = if rows.is_empty() {
        vec![kit::ContextItem::new(EMPTY_ROW).disabled()]
    } else {
        rows.iter()
            .map(|&at| {
                let listed = &app.workbench.tools[at];
                kit::ContextItem::new(SharedString::from(listed.tool.name.clone()))
                    .icon(IconName::Play)
            })
            .collect()
    };

    kit::context_menu(
        "run-tool-menu",
        point(px(at.0), px(at.1)),
        items,
        ui::indexed(&cx.entity(), |this, index, window, cx| {
            this.pick_run_tool_menu(index, window, cx);
        }),
        ui::handler(&cx.entity(), |this, _, cx| this.dismiss_run_tool_menu(cx)),
    )
    .into_any_element()
}
