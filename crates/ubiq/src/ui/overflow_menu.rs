//! The titlebar's overflow chevron menu: the rarely-used commands moved off the strip.
//!
//! Remote connect, web export, window capture, help and settings are all things the user reaches
//! for occasionally rather than every session, so they read as one family behind a chevron rather
//! than five permanent squares — painted over the window for the reason [`super::new_pane_menu`]
//! is: the titlebar does not name `AppState` beyond what it already reads, and this is a second
//! menu the window draws the same way.
//!
//! **The rows are data**: `ui::menus::overflow`, through the `ext::menu` container. Nothing here
//! matches a row index against a second list — each row carries its own action.

use gpui::{Context, IntoElement, Window, div};

use crate::app::AppState;
use crate::ext::ids;
use crate::ui::menus;

/// Draw the open overflow menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some(at) = app.workbench.overflow_menu else {
        return div().into_any_element();
    };
    menus::overlay(
        &cx.entity(),
        "overflow-menu",
        at,
        menus::entries(ids::MENU_OVERFLOW, app, cx),
        |this, _, cx| this.dismiss_overflow_menu(cx),
    )
}
