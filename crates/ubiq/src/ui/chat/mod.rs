//! The chat panel.
//!
//! It is **editor-like tabs, not a chat of its own**: many instances may be open at once, each
//! attached to a conversation the host owns — or to none — and each free to move to any dockable
//! region. The set of conversations a tab may attach to is the host's, the same set the agents
//! screen draws as columns, so a conversation started here is visible there and the other way
//! round, and closing a tab ends nothing: the conversation, if it had one, keeps running.
//!
//! What a tab draws for its attachment is [`crate::ui::conversation`], the one conversation view
//! every surface shares. This module supplies only the frame: which tab, what it is attached to,
//! and the furniture around it.

pub mod sidebar;

use gpui::{Context, IntoElement, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::ChatId;
use crate::theme;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::empty;
use crate::ui::kit::panel;

pub fn render(
    app: &AppState,
    id: ChatId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    panel()
        .border_l_1()
        .border_color(theme::border())
        .child(sidebar::header(app, id, window, cx))
        // No status strip: the run pill, the context ring and the cost are the shared view's
        // footer, computed from what the harness actually reported. A second strip over it was a
        // fixture, and two answers about one conversation is one answer too many.
        .child(body(app, id, window, cx))
}

/// The attached conversation, or a note saying there is nothing to draw.
///
/// The transcript and the composer are the shared view's, not the tab's: a second transcript
/// would be a second answer about the same conversation, and the record is the host's alone.
fn body(
    app: &AppState,
    id: ChatId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let tab = app
        .open_project(cx)
        .and_then(|open| open.chats.iter().find(|tab| tab.id == id).copied());
    let attached = tab.and_then(|tab| {
        tab.attached
            .and_then(|agent| app.conversation(agent, cx).map(|conv| (conv, tab.slot)))
    });

    match attached {
        Some((conversation, slot)) => conversation::render(
            app,
            conversation,
            ConversationView {
                id: SharedString::from(format!("chat-{id}")),
                slot,
                footer: true,
                composer: true,
            },
            window,
            cx,
        ),
        // Nothing attached. The control that fixes it is named rather than left to be found, the
        // same way the agents screen's empty page names it.
        None => div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .child(empty::empty_page(
                "Nothing attached",
                "Attach a conversation, or start one with New chat.",
                IconName::Asterisk,
                None,
            ))
            .into_any_element(),
    }
}
