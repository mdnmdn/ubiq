//! The chat tab's own head: what it is attached to, and the two controls that start something
//! new.
//!
//! *New chat* starts a conversation, through the same menu the agents screen uses, and attaches
//! this tab to it. *New tab* starts nothing — it opens another view, attached to nothing, beside
//! this one. One adds a harness to have a view on; the other adds the view.

use gpui::{Context, ElementId, Focusable, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::IconName;

use crate::app::AppState;
use crate::state::{ChatId, attach_choices};
use crate::ui::kit::{Picker, ghost_button};
use crate::ui::{handler, indexed};

/// The row of controls above a chat tab's transcript: what it is attached to, on the left, and
/// the two ways to start something new, on the right.
pub fn header(
    app: &AppState,
    id: ChatId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    div()
        .h(px(38.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .child(attach_picker(app, id, window, cx))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .child(ghost_button(
                    "chat-new",
                    Some(IconName::Plus),
                    "New chat",
                    cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        let at = event.position();
                        this.new_chat(id, (at.x.into(), at.y.into()), cx);
                    }),
                ))
                .child(ghost_button(
                    "chat-new-tab",
                    Some(IconName::Copy),
                    "New tab",
                    cx.listener(|this, _, _, cx| this.new_chat_tab(cx)),
                )),
        )
}

/// What this tab is attached to, and the picker that changes it.
///
/// The picker's items are the project's conversations — the same registry the agents sidebar
/// lists. A row already attached to a *different* chat tab draws disabled and cannot be picked;
/// this tab's own current row stays selectable, and no row is ever dropped from the list, because
/// a row that vanishes reads as a conversation that ended rather than one taken.
fn attach_picker(
    app: &AppState,
    id: ChatId,
    window: &Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let entity = cx.entity();
    let picker_open = app
        .open_project(cx)
        .and_then(|open| open.chats.iter().find(|tab| tab.id == id))
        .is_some_and(|tab| tab.picker_open);

    let query = app.picker_search.read(cx).value().to_string();
    let search_focused = app
        .picker_search
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);

    let choices = match (app.open_project(cx), app.work(cx)) {
        (Some(open), Some(work)) => attach_choices(&open.chats, id, &work.agents, &query),
        _ => attach_choices(&[], id, &[], &query),
    };
    let names: Vec<String> = choices.items.iter().map(|(_, name)| name.clone()).collect();
    let agent_ids: Vec<_> = choices.items.iter().map(|(agent, _)| *agent).collect();
    let label = choices
        .selected
        .and_then(|ix| choices.items.get(ix))
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| "Attach a conversation".to_string());

    let mut picker = Picker::new(ElementId::Name(format!("chat-attach-{id}").into()), label)
        .icon(IconName::Asterisk)
        .items(names)
        .disabled(choices.disabled)
        .open(picker_open)
        .search(&app.picker_search, search_focused);
    if let Some(index) = choices.selected {
        picker = picker.selected(index);
    }
    picker
        .on_toggle(handler(&entity, move |this, window, cx| {
            this.toggle_chat_picker(id, window, cx)
        }))
        .on_pick(indexed(&entity, move |this, index, _, cx| {
            if let Some(agent) = agent_ids.get(index).copied() {
                this.attach_chat(id, Some(agent), cx);
            }
        }))
        .on_dismiss(handler(&entity, move |this, _, cx| {
            this.dismiss_chat_picker(id, cx)
        }))
}
