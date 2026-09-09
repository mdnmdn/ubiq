//! The chat tab's own head: what the conversation is, what it is on, and what can be done to it.
//!
//! **One control answers both questions.** A tab either shows a conversation or it does not, and
//! the thing the user wants in each case is different — start something, or move to something
//! already running. Two controls side by side made the user pick the question before answering
//! it; one control whose first row starts and whose rest attach does not.
//!
//! **It is a bare chevron.** What the tab is showing is already said twice on this row — by the
//! dock's tab and by the state mark beside it — so the control that changes it says only that
//! there is a list, and its tooltip says what the list is about. A label here was a third copy of
//! the conversation's name wearing a control's clothes.
//!
//! **The row reads left to right in the order a reader asks.** The state mark says what the
//! conversation *is*, the control says what it is *on*, and the three-dots at the far end says
//! what can be *done to it* — so [`conversation::lifecycle_mark`] and
//! [`conversation::lifecycle_menu`] sit at opposite ends of the row rather than as a pair,
//! through the split fragments that module exposes for exactly this.
//!
//! **Nothing here names the tab.** The dock's tab already carries the conversation's name, and a
//! second copy of it directly under the first was the same answer twice.
//!
//! *New tab* is not here either — it is a `+` on the dock's own tab strip, beside the terminal
//! region's, because opening another view of the same kind is the tab strip's gesture in this
//! window and the chat panel is not an exception to it.

use crate::app::AppState;
use crate::state::ChatId;
use crate::state::conversation::Conversation;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::kit::Picker;
use crate::ui::{handler, indexed};
use gpui::{
    Context, ElementId, Focusable, IntoElement, ParentElement, SharedString, Styled, Window, div,
    px,
};

/// The row of controls above a chat tab's transcript.
pub fn header(
    app: &AppState,
    id: ChatId,
    attached: Option<(&Conversation, usize)>,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let view = attached.map(|(_, slot)| ConversationView {
        id: SharedString::from(format!("chat-{id}")),
        slot,
        footer: true,
        composer: true,
        header: false,
    });

    let mut left = div().flex().flex_none().items_center().gap_2();
    if let (Some((conversation, _)), Some(view)) = (attached, view.as_ref()) {
        left = left.child(conversation::lifecycle_mark(conversation, view));
    }
    left = left.child(start_control(app, id, window, cx));

    let mut right = div().flex().flex_none().items_center().gap_1();
    if let (Some((conversation, _)), Some(view)) = (attached, view.as_ref()) {
        right = right.child(conversation::lifecycle_menu(app, conversation, view, cx));
    }

    div()
        .h(px(38.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .child(left)
        .child(right)
}

/// The one control: start a conversation, or move to one already running.
///
/// **No text and no state of its own** — a chevron, and a tooltip. Its rows are
/// [`crate::state::chat_picks`], built once and read again by [`AppState::pick_chat_row`] when one
/// is clicked, so a position means the same row in both.
///
/// The first row raises the New agent form, which asks the harness, the identity, the model, the
/// level and the mode together. The flattened harness-and-identity list this control used to
/// carry above its conversations is gone with the row that started from it: it launched with every
/// question but the first skipped, and the form is what asks them.
///
/// **An attached tab is offered the start too.** It used to be denied one, on the grounds that
/// starting from a tab already showing a conversation would leave the first with no view; the
/// conversation it leaves is still on this very list, one click from coming back.
fn start_control(
    app: &AppState,
    id: ChatId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let entity = cx.entity();
    let open = app
        .open_project(cx)
        .and_then(|open| open.chats.iter().find(|tab| tab.id == id))
        .is_some_and(|tab| tab.picker_open);

    let search_focused = app
        .picker_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let picks = app.chat_picks(id, cx);

    // No label and no icon: the trigger's own chevron is the whole control.
    let mut picker = Picker::new(ElementId::Name(format!("chat-start-{id}").into()), "")
        .tooltip("change agent")
        .items(picks.labels)
        .disabled(picks.disabled)
        .separators(picks.separators)
        .open(open)
        .search(&app.picker_search, search_focused);
    if let Some(index) = picks.selected {
        picker = picker.selected(index);
    }
    picker
        .on_toggle(handler(&entity, move |this, window, cx| {
            this.toggle_chat_picker(id, window, cx)
        }))
        .on_pick(indexed(&entity, move |this, index, window, cx| {
            this.pick_chat_row(id, index, window, cx)
        }))
        .on_dismiss(handler(&entity, move |this, _, cx| {
            this.dismiss_chat_picker(id, cx)
        }))
}
