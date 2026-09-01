//! The chat panel's head: the conversation list and the run/context strip under it.

use gpui::{
    AnyElement, Context, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::RunState;
use crate::theme;
use crate::ui::kit::{
    ghost_button, icon_button, mono, pill, progress_ring, section_label, state_chip,
};

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
            cx.listener(|this, _, _, cx| this.new_chat(cx)),
        ))
}

pub fn chat_list(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let active = app.chat.active;

    let mut rows: Vec<AnyElement> = Vec::new();
    for (ix, chat) in app.chat.chats.iter().enumerate() {
        {
            let is_active = ix == active;
            let mut row = div()
                .id(ElementId::Name(format!("chat-row-{}", chat.id).into()))
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
                        .child(SharedString::from(chat.title.clone())),
                )
                .child(mono(chat.when.clone(), theme::text_faint()).text_size(px(11.)));

            if is_active {
                row = row
                    .bg(theme::accent_soft())
                    .border_l(px(theme::ACCENT_EDGE))
                    .border_color(theme::accent());
            }

            rows.push(
                row.on_click(cx.listener(move |this, _, _, cx| this.select_chat(ix, cx)))
                    .into_any_element(),
            );
        }
    }

    div()
        .id("chat-list")
        .flex()
        .flex_col()
        .flex_none()
        .max_h(px(160.))
        .overflow_y_scroll()
        .children(rows)
}

/// The run state and the context window, side by side. Status is shown by colour, never by
/// wording alone.
pub fn status_strip(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let run = app.chat.run;
    let dot = match run {
        RunState::Idle => theme::text_faint(),
        RunState::Working => theme::warning(),
    };
    let pct = app.chat.context_pct();

    div()
        .px_3()
        .py_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .border_t_1()
        .border_b_1()
        .border_color(theme::border())
        .child(state_chip(run.label(), dot, 1.0))
        .child(
            pill(theme::accent())
                .child(progress_ring(pct, 13.0))
                .child(mono(format!("{pct}%"), theme::text()).text_size(px(11.5)))
                .child(
                    mono(
                        format!("{:.1}K tok", app.chat.tokens / 1000.0),
                        theme::text_muted(),
                    )
                    .text_size(px(11.5)),
                ),
        )
        .child(div().flex_1().min_w(px(0.)))
        .child(icon_button(
            "chat-history",
            IconName::Calendar,
            false,
            cx.listener(|this, _, _, _cx| {
                this.chat_scroll.set_offset(gpui::point(px(0.), px(0.)));
            }),
        ))
}
