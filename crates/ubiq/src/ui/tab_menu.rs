//! The right-click menu of a tab — a file, a terminal or a chat — painted over the window.
//!
//! The dock's skin does not name `AppState`, so it cannot draw a menu with state in it. Instead a
//! right-click on a tab hands `AppState` the panel's kind and the click's point, and this module
//! paints the menu here — at the window root, on top of the dock that the tab lives in. One menu at
//! a time, exactly like `WorkbenchState::open_menu` already is.
//!
//! [`rows`] is what a file, a terminal or a chat tab offers, read by both the frame that draws the
//! menu and `AppState::pick_tab_menu` — the pick is matched by position, so the two have to read
//! the same list. A file's is unchanged from before the menu widened to every kind of tab, plus
//! Pin/Unpin at the end; a terminal and a chat tab offer a rename, a close and the same pin.
//!
//! **Close is never offered on a pinned tab.** Suppressing the row rather than drawing a no-op is
//! what keeps the pick a plain dispatch — nothing downstream has to remember that pinned changes
//! what index 0 means.

use gpui::{Context, IntoElement, SharedString, Window, div, point, px};

use crate::app::AppState;
use crate::state::PanelKind;
use crate::ui::{self, kit};

/// The rows a tab's right-click menu offers, by the panel's own kind and whether it is pinned.
pub fn rows(kind: &PanelKind, pinned: bool) -> Vec<&'static str> {
    let pin_row = if pinned { "Unpin" } else { "Pin" };
    match kind {
        PanelKind::File(_) => {
            let mut rows = Vec::new();
            if !pinned {
                rows.push("Close");
            }
            rows.extend([
                "Close Others",
                "Close Left",
                "Close Right",
                "Close All",
                "Copy Full Path",
                "Copy link",
                "Open in Finder",
                "Save",
                "Word Wrap",
            ]);
            rows.push(pin_row);
            rows
        }
        PanelKind::Terminal(_) | PanelKind::Chat(_) => {
            let mut rows = vec!["Rename…"];
            if !pinned {
                rows.push("Close");
            }
            rows.push(pin_row);
            rows
        }
        _ => Vec::new(),
    }
}

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

/// Draw the open tab menu, or nothing when there is none. Called from the window root.
pub fn overlay(
    app: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let Some((kind, at)) = app.workbench.tab_menu.clone() else {
        return div().into_any_element();
    };
    let pinned = app.tab_pinned(&kind, cx);
    let items: Vec<_> = rows(&kind, pinned)
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
        "tab-menu",
        point(px(at.0), px(at.1)),
        items,
        ui::indexed(&cx.entity(), |this, index, window, cx| {
            this.pick_tab_menu(index, window, cx);
        }),
        ui::handler(&cx.entity(), |this, _, cx| this.dismiss_tab_menu(cx)),
    )
    .into_any_element()
}
