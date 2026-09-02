//! The right-click menu of a file tab, painted over the window.
//!
//! The dock's skin does not name `AppState`, so it cannot draw a menu with state in it. Instead a
//! right-click on a file tab hands `AppState` the tab's key and the click's point, and this module
//! paints the menu here — at the window root, on top of the dock that the tab lives in. One menu at
//! a time, exactly like `WorkbenchState::open_menu` already is.

use gpui::{Context, IntoElement, SharedString, Window, div, point, px};

use crate::app::AppState;
use crate::ui::{self, kit};

/// The rows, in the order they appear: what closes the clicked tab first, then the bulk closes, then
/// the actions. The index a row is picked at is the index in this list, which is what
/// `AppState::pick_file_tab_menu` matches on — keep the two in step.
const ITEMS: &[&str] = &[
    "Close",
    "Close Others",
    "Close Left",
    "Close Right",
    "Close All",
    "Copy Full Path",
    "Open in Finder",
    "Save",
    "Word Wrap",
];

/// The "reveal in the system file manager" row's label, named after the platform.
fn open_in_system_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Open in Finder"
    } else if cfg!(target_os = "windows") {
        "Open in Explorer"
    } else {
        "Open in File Manager"
    }
}

/// Draw the open file-tab menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some((_, at)) = app.workbench.file_tab_menu else {
        return div().into_any_element();
    };
    let items: Vec<_> = ITEMS
        .iter()
        .map(|label| {
            let label = if *label == "Open in Finder" {
                open_in_system_label()
            } else {
                label
            };
            kit::ContextItem::new(SharedString::from(label))
        })
        .collect();
    kit::context_menu(
        "file-tab-menu",
        point(px(at.0), px(at.1)),
        items,
        ui::indexed(&cx.entity(), |this, index, window, cx| {
            this.pick_file_tab_menu(index, window, cx);
        }),
        ui::handler(&cx.entity(), |this, _, cx| this.dismiss_file_tab_menu(cx)),
    )
    .into_any_element()
}
