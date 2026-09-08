//! The chat tab's own head: what the conversation is, what it is on, and what can be done to it.
//!
//! **One control answers both questions.** A tab either shows a conversation or it does not, and
//! the thing the user wants in each case is different — start something, or move to something
//! already running. Two controls side by side made the user pick the question before answering
//! it; one control that changes its icon and its offer does not. An empty tab is offered both
//! halves, and an attached tab only the second: starting from a tab that already shows a
//! conversation would leave the first with no view and no way back to it.
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

use gpui::{
    Context, ElementId, Focusable, IntoElement, ParentElement, SharedString, Styled, Window, div,
    px,
};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::ChatId;
use crate::state::conversation::Conversation;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::kit::Picker;
use crate::ui::{handler, indexed};

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
    left = left.child(start_control(app, id, attached.is_some(), window, cx));

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
/// **The icon says which question it is asking.** An empty tab wears `Play` — there is something
/// to begin — and an attached one wears the harness glyph, because the question has become which
/// conversation rather than whether to have one.
///
/// Its rows are [`crate::state::chat_picks`], built once and read again by
/// [`AppState::pick_chat_row`] when one is clicked, so a position means the same row in both. The
/// harness half is grouped exactly as the agents screen's own menu groups it — the offers come
/// from [`crate::ui::agents::harness_offers`], which is that menu's labelling, not a second copy.
///
/// An empty tab opens on the last harness anything was started on
/// ([`AppState::remembered_choice`]), so the common case is one click away and the list is still
/// there for every other case.
fn start_control(
    app: &AppState,
    id: ChatId,
    attached: bool,
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
    let label = picks
        .selected
        .and_then(|ix| picks.labels.get(ix))
        .cloned()
        .unwrap_or_else(|| "Start or attach".to_string());

    // The remembered harness is only a preselection, and only where there is nothing attached to
    // preselect instead: a tab showing a conversation is already sitting on its own answer.
    let selected = picks.selected.or_else(|| {
        if attached {
            return None;
        }
        let remembered = app.remembered_choice()?;
        picks
            .rows
            .iter()
            .position(|row| *row == crate::state::ChatPick::Start(remembered))
    });

    let mut picker = Picker::new(ElementId::Name(format!("chat-start-{id}").into()), label)
        .icon(if attached {
            IconName::Asterisk
        } else {
            IconName::Play
        })
        .items(picks.labels)
        .disabled(picks.disabled)
        .separators(picks.separators)
        .open(open)
        .search(&app.picker_search, search_focused);
    if let Some(index) = selected {
        picker = picker.selected(index);
    }
    picker
        .on_toggle(handler(&entity, move |this, window, cx| {
            this.toggle_chat_picker(id, window, cx)
        }))
        .on_pick(indexed(&entity, move |this, index, _, cx| {
            this.pick_chat_row(id, index, cx)
        }))
        .on_dismiss(handler(&entity, move |this, _, cx| {
            this.dismiss_chat_picker(id, cx)
        }))
}
