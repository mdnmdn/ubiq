//! The chat panel's head: the control that starts a conversation, and the list of them.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::theme;
use crate::ui::kit::{ghost_button, mono, section_label};

pub fn header(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    div()
        .h(px(38.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            div()
                .id("chat-collapse")
                .flex()
                .items_center()
                .gap_2()
                .px_1()
                .cursor_pointer()
                .hover(|this| this.bg(theme::hover()))
                .child(
                    Icon::new(if app.chat.collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(Size::XSmall)
                    .text_color(theme::text_muted()),
                )
                .child(section_label("Chats"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.chat.collapsed = !this.chat.collapsed;
                    cx.notify();
                })),
        )
        .child(ghost_button(
            "chat-new",
            Some(IconName::Plus),
            "New chat",
            cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                let at = event.position();
                this.new_chat((at.x.into(), at.y.into()), cx);
            }),
        ))
}

/// The project's conversations, newest activity first as the host reports them.
///
/// The same set the agents screen lists in its own sidebar — one registry, two views — so a row
/// here names a conversation that exists whether or not this panel is open.
pub fn chat_list(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let selected = app.chat.selected;
    // One registry, two views: this is the same set the agents screen's sidebar lists.
    let Some(work) = app.work(cx) else {
        return div().flex().flex_none().flex_col();
    };

    let mut rows: Vec<AnyElement> = Vec::new();
    for agent in &work.agents {
        let id = agent.id;
        let is_active = selected == Some(id);
        let mut row = div()
            .id(ElementId::Name(format!("chat-row-{id}").into()))
            .h(px(38.))
            .px_3()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap_2()
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(13.))
                    .text_color(if is_active {
                        theme::text()
                    } else {
                        theme::text_muted()
                    })
                    .child(SharedString::from(agent.name.clone())),
            )
            // Which harness and identity, rather than a timestamp: two rows of the same name are
            // told apart by what they run as, and the host reports no time.
            .child(
                mono(
                    if agent.account.is_empty() {
                        agent.harness.clone()
                    } else {
                        format!("{} · {}", agent.harness, agent.account)
                    },
                    theme::text_faint(),
                )
                .text_size(px(11.)),
            );

        if is_active {
            row = row
                .bg(theme::accent_soft())
                .border_l(px(theme::ACCENT_EDGE))
                .border_color(theme::accent());
        }

        rows.push(
            row.on_click(cx.listener(move |this, _, _, cx| this.select_chat(id, cx)))
                .into_any_element(),
        );
    }

    if rows.is_empty() {
        rows.push(
            div()
                .px_3()
                .py_2()
                .text_size(px(12.))
                .text_color(theme::text_faint())
                .child(SharedString::from("Nothing running in this project."))
                .into_any_element(),
        );
    }

    div().flex().flex_none().flex_col().children(rows)
}
