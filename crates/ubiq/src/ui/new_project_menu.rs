//! The titlebar's new-project chevron menu: the same three ways into a project the picker's foot
//! offers — add, clone, remote — reached without opening the picker first.
//!
//! Painted over the window for the reason [`super::overflow_menu`] is: the titlebar does not name
//! `AppState` beyond what it already reads, and this is a third menu the window draws the same
//! way.
//!
//! **Reused, not restated.** The actions behind each row are the same ones `ui::project_menu`'s
//! `add_row`, `clone_row` and `remote_row` already run — see `ui::menus::new_project`.

use gpui::{Context, IntoElement, Window, div};

use crate::app::AppState;
use crate::ext::ids;
use crate::ui::menus;

/// Draw the open new-project menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.new_project_menu else {
        return div().into_any_element();
    };
    menus::overlay(
        &cx.entity(),
        "new-project-menu",
        at,
        menus::entries(ids::MENU_NEW_PROJECT, app, cx),
        |this, _, cx| this.dismiss_new_project_menu(cx),
    )
}
