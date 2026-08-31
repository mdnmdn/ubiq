//! The composer: what is typed, what it will be sent to, and the button that sends it.

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    div, px,
};
use gpui_component::input::Textarea;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::{HARNESSES, MODELS, MODES, MenuId, THINKING};
use crate::theme;
use crate::ui::kit::menu::MENU_ANCHOR_UP;
use crate::ui::kit::{Picker, PickerStyle, icon_button, mono};
use crate::ui::{handler, indexed};

pub fn render(app: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    let view = cx.entity();
    let open = app.workbench.open_menu;
    let can_send = !app.chat.draft.trim().is_empty();

    div()
        .flex()
        .flex_none()
        .flex_col()
        .m_3()
        .bg(theme::surface())
        .border_l(px(theme::ACCENT_EDGE))
        .border_color(theme::accent())
        .child(
            div()
                .id("chat-composer")
                .px_3()
                .pt_2()
                .cursor_text()
                .child(
                    Textarea::new(&app.chat_input)
                        .appearance(false)
                        .bordered(false)
                        .w_full()
                        .text_size(px(13.5)),
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    let input = this.chat_input.clone();
                    input.update(cx, |state, cx| state.focus(window, cx));
                })),
        )
        .child(
            div()
                .px_2()
                .pb_2()
                .pt_1()
                .flex()
                .items_center()
                .gap_2()
                .child(icon_button(
                    "chat-attach",
                    IconName::Plus,
                    app.chat.attachment,
                    cx.listener(|this, _, _, cx| {
                        this.chat.attachment = !this.chat.attachment;
                        cx.notify();
                    }),
                ))
                .child(
                    Picker::new("harness-picker", app.chat.harness_label())
                        .icon(IconName::Asterisk)
                        .style(PickerStyle::Chip)
                        .anchor(MENU_ANCHOR_UP)
                        .items(HARNESSES)
                        .selected(app.chat.harness)
                        .open(open == Some(MenuId::Harness))
                        .on_toggle(handler(&view, |this, _, cx| {
                            this.open_menu(MenuId::Harness, cx)
                        }))
                        .on_pick(indexed(&view, |this, index, _, cx| {
                            this.chat.harness = index;
                            this.close_menu(cx);
                        }))
                        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
                )
                .child(
                    Picker::new("model-picker", app.chat.model_label())
                        .style(PickerStyle::Chip)
                        .anchor(MENU_ANCHOR_UP)
                        .items(MODELS)
                        .selected(app.chat.model)
                        .open(open == Some(MenuId::Model))
                        .on_toggle(handler(&view, |this, _, cx| {
                            this.open_menu(MenuId::Model, cx)
                        }))
                        .on_pick(indexed(&view, |this, index, _, cx| {
                            this.chat.model = index;
                            this.close_menu(cx);
                        }))
                        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
                )
                .child(
                    Picker::new("thinking-picker", app.chat.thinking_label())
                        .icon(IconName::Cpu)
                        .style(PickerStyle::Chip)
                        .anchor(MENU_ANCHOR_UP)
                        .items(THINKING)
                        .selected(app.chat.thinking)
                        .open(open == Some(MenuId::Thinking))
                        .on_toggle(handler(&view, |this, _, cx| {
                            this.open_menu(MenuId::Thinking, cx)
                        }))
                        .on_pick(indexed(&view, |this, index, _, cx| {
                            this.chat.thinking = index;
                            this.close_menu(cx);
                        }))
                        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
                )
                .child(
                    Picker::new("mode-picker", app.chat.mode_label())
                        .icon(IconName::Menu)
                        .style(PickerStyle::Chip)
                        .anchor(MENU_ANCHOR_UP)
                        .items(MODES)
                        .selected(app.chat.mode)
                        .open(open == Some(MenuId::Mode))
                        .on_toggle(handler(&view, |this, _, cx| {
                            this.open_menu(MenuId::Mode, cx)
                        }))
                        .on_pick(indexed(&view, |this, index, _, cx| {
                            this.chat.mode = index;
                            this.close_menu(cx);
                        }))
                        .on_dismiss(handler(&view, |this, _, cx| this.close_menu(cx))),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    mono(
                        "\u{23ce} send \u{b7} \u{21e7}\u{23ce} newline",
                        theme::text_faint(),
                    )
                    .text_size(px(10.5)),
                )
                .child(send_button(can_send, cx)),
        )
}

fn send_button(enabled: bool, cx: &mut Context<AppState>) -> impl IntoElement {
    let (bg, fg) = if enabled {
        (theme::accent(), theme::on_accent())
    } else {
        (theme::surface_raised(), theme::text_faint())
    };

    div()
        .id("chat-send")
        .h(px(28.))
        .px_3()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .bg(bg)
        .text_size(px(12.5))
        .text_color(fg)
        .cursor_pointer()
        .child("Send")
        .child(Icon::new(IconName::ArrowRight).with_size(Size::XSmall))
        .on_click(cx.listener(move |this, _, window, cx| {
            if enabled {
                this.send_chat(window, cx);
            }
        }))
}
