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
//! Pin/Unpin at the end; a terminal and a chat tab offer a rename, the two closes below, and the
//! same pin.
//!
//! **Hide and Close are two different endings, and both are offered.** A terminal or a chat tab is
//! a *view* onto something that lives past it — a harness the host is running, a conversation the
//! host owns — so taking the tab away and ending the thing behind it are separate decisions, and
//! the menu makes the user say which one they mean:
//!
//! - **Hide** takes the panel down and leaves everything else alone: the harness keeps running
//!   (`AppState::detach_pane`, reopened by `AppState::reattach_pane`), the conversation keeps
//!   running (`AppState::close_chat_tab`, reopened by attaching a chat tab to it again).
//! - **Close** is the real end: a terminal's kills the harness and drops its screen
//!   (`AppState::close_pane`, behind a confirm, because it is irreversible); a chat tab's deletes
//!   the conversation it is attached to, through the same confirm the conversation's own lifecycle
//!   menu raises. A chat tab attached to nothing has nothing to delete, so its Close only hides.
//!
//! **Hide is never offered on a pinned tab**, exactly as the single Close it replaced was not.
//! Suppressing the row rather than drawing a no-op is what keeps the pick a plain dispatch —
//! nothing downstream has to remember that pinned changes what index 0 means.
//!
//! **Close is offered on a pinned tab too.** Pinning is about the tab's place in the arrangement,
//! not about the harness's or the conversation's right to keep running underneath it, and ending
//! one is still the user's call to make regardless.

use gpui::{Context, IntoElement, SharedString, Window, div, point, px};

use crate::app::AppState;
use crate::state::PanelKind;
use crate::ui::{self, kit};

/// The rows a tab's right-click menu offers, by the panel's own kind, whether it is pinned, and
/// whether it is a stopped tool pane.
///
/// `restartable` is that last question, asked of the caller rather than computed here for the
/// reason `WorkbenchState::new_pane_rows` takes `has_project`: this module has no `AppState` and
/// a pane's origin is one. It is true only for a terminal whose pane was started by
/// [`ubiq_proto::messages::Message::RunTool`] and whose command has since ended — the "wait on
/// exit" case, where the tab is still up over output nothing is producing any more.
pub fn rows(kind: &PanelKind, pinned: bool, restartable: bool) -> Vec<&'static str> {
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
            // Above the endings, because it is the opposite of them: the row that puts the pane
            // back to work rather than taking it away. Offered only while there is nothing
            // running to restart.
            if restartable {
                rows.push("Restart");
            }
            if !pinned {
                rows.push("Hide");
            }
            rows.push("Close");
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
    let restartable = app.tab_restartable(&kind, cx);
    let items: Vec<_> = rows(&kind, pinned, restartable)
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
