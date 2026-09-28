//! The chevron beside New agent: the agent panes a tab's `Hide` took off the screen (`T-266`).
//!
//! A `Hide` close detaches a pane and nothing more — the harness keeps running and its screen
//! stays live — so an agent hidden that way has no tab and no panel, and until now no way back
//! except the new-pane menu's Detached group, which mixes it in with shells. This is the same
//! set, narrowed to agents, one click from the control that starts one.
//!
//! Painted over the window for the reason [`super::overflow_menu`] is. The rows are
//! `ui::menus::hidden_agents`, through the `ext::menu` container; a pick **reattaches** the live
//! pane and never respawns it.

use gpui::{Context, IntoElement, Window, div};

use crate::app::AppState;
use crate::ext::ids;
use crate::ui::menus;

/// Draw the open hidden-agents menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.hidden_agents_menu else {
        return div().into_any_element();
    };
    menus::overlay(
        &cx.entity(),
        "hidden-agents-menu",
        at,
        menus::entries(ids::MENU_HIDDEN_AGENTS, app, cx),
        |this, _, cx| this.dismiss_hidden_agents_menu(cx),
    )
}
