//! The chat panel's own head: T-102's one first line — the three-dots menu, the agent-switch
//! chevron, the focus button, and the current-action chip flush right. An agents column draws the
//! same row without the chevron, its tab strip being what switches agents there.
//!
//! **It is a bare chevron.** What the tab is showing is already said twice on this row — by the
//! dock's tab and by the hexagon it now wears (T-99) — so the control that changes it says only
//! that there is a list, and its tooltip says what the list is about. A label here was a third
//! copy of the conversation's name wearing a control's clothes.
//!
//! **One control answers both questions.** A tab either shows a conversation or it does not, and
//! the thing the user wants in each case is different — start something, or move to something
//! already running. Two controls side by side made the user pick the question before answering
//! it; one control whose first row starts and whose rest attach does not.
//!
//! **Nothing here names the tab.** The dock's tab already carries the conversation's name, and a
//! second copy of it directly under the first was the same answer twice.
//!
//! *New tab* is not here either — it is a `+` on the dock's own tab strip, beside the terminal
//! region's, because opening another view of the same kind is the tab strip's gesture in this
//! window and the chat panel is not an exception to it.
//!
//! **An unattached tab draws the row anyway.** [`conversation::lifecycle_header`] wants a
//! conversation to read the menu and the chip off; a tab with nothing attached has neither, so
//! this draws the chevron alone rather than call it with nothing to say.

use crate::app::AppState;
use crate::state::conversation::Conversation;
use crate::state::{ChatHost, ChatId};
use crate::theme;
use crate::ui::conversation::{self, ConversationView};
use crate::ui::kit::Picker;
use crate::ui::{handler, indexed};
use gpui::{Context, ElementId, Focusable, IntoElement, ParentElement, Styled, Window, div, px};

/// The row of controls above a chat panel's transcript.
///
/// **A column gets the same row without the chevron**: which agent it shows is its tab strip's
/// to say, so only the menu, the focus button and the chip draw. A column with nothing attached
/// draws no row at all — its body already says why.
pub fn header(
    app: &AppState,
    host: ChatHost,
    attached: Option<(&Conversation, usize)>,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let view = attached.map(|(_, slot)| ConversationView {
        id: super::view_id(host),
        slot,
        footer: true,
        composer: true,
        header: false,
    });

    let mut chevron = match host {
        ChatHost::Tab(id) => Some(start_control(app, id, window, cx).into_any_element()),
        ChatHost::Column(_) => None,
    };
    if attached.is_some() {
        let focused = app.workbench.chat_focus == Some(host);
        chevron = Some(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .children(chevron)
                .child(
                    crate::ui::kit::icon_button_tip(
                        ElementId::Name(format!("{}-focus-toggle", super::view_id(host)).into()),
                        if focused {
                            gpui_component::IconName::Minimize
                        } else {
                            gpui_component::IconName::Maximize
                        },
                        if focused {
                            "Leave focus mode (\u{2318}\u{21e7}\u{23ce})"
                        } else {
                            "Focus this chat (\u{2318}\u{21e7}\u{23ce})"
                        },
                        true,
                        cx.listener(move |this, _, window, cx| {
                            this.toggle_chat_focus(host, window, cx)
                        }),
                    )
                    .size(px(28.)),
                )
                .into_any_element(),
        );
    }
    match (attached, view.as_ref()) {
        (Some((conversation, _)), Some(view)) => {
            conversation::lifecycle_header(app, conversation, view, chevron, cx).into_any_element()
        }
        (_, _) if matches!(host, ChatHost::Column(_)) => div().into_any_element(),
        // Nothing attached: the menu and the chip have nothing to read, so only the chevron that
        // starts or attaches something draws — same row height as the attached case, so nothing
        // moves when a pick lands.
        _ => div()
            .h(px(28.))
            .pl_1p5()
            .flex()
            .flex_none()
            .items_center()
            .border_b_1()
            .border_color(theme::border())
            .children(chevron)
            .into_any_element(),
    }
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
        .details(picks.details.into_iter().map(Some))
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
