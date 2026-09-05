//! The chat panel.
//!
//! It is the one panel that stays on screen in every rail mode, and it is **a view onto the
//! project's conversations, not a chat of its own**. The set it lists is the host's — the same set
//! the agents screen draws as columns — so a conversation started here is visible there and the
//! other way round, and closing this panel ends nothing.
//!
//! What it draws for the selected one is [`crate::ui::conversation`], the one conversation view
//! every surface shares. The panel supplies only the frame: which conversation, and the furniture
//! around it.

pub mod sidebar;
pub mod transcript;

use gpui::{Context, IntoElement, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::agents::CHAT_SLOT;
use crate::theme;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::empty;
use crate::ui::kit::panel;

pub fn render(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let mut root = panel()
        .border_l_1()
        .border_color(theme::border())
        .child(sidebar::header(app, cx));

    if !app.chat.collapsed {
        root = root.child(sidebar::chat_list(app, cx));
    }

    // No status strip: the run pill, the context ring and the cost are the shared view's footer,
    // computed from what the harness actually reported. A second strip over it was a fixture, and
    // two answers about one conversation is one answer too many.
    root.child(body(app, window, cx))
}

/// The selected conversation, or a note saying there is nothing selected to draw.
///
/// The transcript and the composer are the shared view's, not the panel's: a second transcript
/// would be a second answer about the same conversation, and the record is the host's alone.
fn body(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> impl IntoElement {
    let selected = app.chat.selected.and_then(|id| {
        app.conversation(id, cx)
            .map(|conversation| (id, conversation))
    });

    match selected {
        Some((_, conversation)) => conversation::render(
            app,
            conversation,
            ConversationView {
                id: SharedString::from("chat-panel"),
                slot: CHAT_SLOT,
                footer: true,
                composer: true,
            },
            window,
            cx,
        ),
        // Nothing selected. The control that starts one is named rather than left to be found,
        // the same way the agents screen's empty page names it.
        None => div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.))
            .child(empty::empty_page(
                "No conversation",
                "Start one with New chat, or pick one from the list.",
                IconName::Asterisk,
                None,
            ))
            .into_any_element(),
    }
}
