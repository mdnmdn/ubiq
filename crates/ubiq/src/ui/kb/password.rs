//! The password question a protected wiki asks (`D206`): unlock, set the first password, or change it.
//!
//! **One modal, three shapes** ([`KbPasswordMode`]): the fields drawn are the ones the mode asks
//! for, all masked, and none is ever seeded — what is typed lives in the fields until it is sent,
//! and is emptied as it goes. The host's refusal (a wrong password, say) is drawn under the fields
//! and the dialog stays up for another try; only an acceptance takes it down.
//!
//! The shape is the database password prompt's (`ui::db::explorer::password_prompt`), on purpose.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Entity, Focusable as _, InteractiveElement, IntoElement,
    ParentElement, Styled, Window, div, px,
};
use gpui_component::input::{Input, InputState};

use crate::app::AppState;
use crate::state::kb::KbPasswordMode;
use crate::state::overlay::Layer;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{check_box, ghost_button, modal, modal_note, primary_button};

const CONTEXT: &str = "KbPassword";
const FIELD_CONTEXT: &str = "KbPassword > Input";

gpui::actions!(ubiq_kb_password, [KbPasswordSubmit]);

/// Enter sends, at the dialog's depth and at its fields' — the component library binds Enter in
/// the deepest node, so the second is the one that is reached with a caret in a field.
pub fn key_bindings() -> Vec<gpui::KeyBinding> {
    vec![
        gpui::KeyBinding::new("enter", KbPasswordSubmit, Some(CONTEXT)),
        gpui::KeyBinding::new("enter", KbPasswordSubmit, Some(FIELD_CONTEXT)),
    ]
}

/// The modal, painted at the window root.
pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(dialog) = app.kb(cx).and_then(|kb| kb.password.clone()) else {
        return div().into_any_element();
    };
    let name = app
        .kb(cx)
        .and_then(|kb| kb.source(dialog.source))
        .map(|view| view.name().to_string())
        .unwrap_or_default();
    let view = cx.entity();

    let (title, note, confirm_label) = match dialog.mode {
        KbPasswordMode::Set => (
            format!("Set a password for {name}"),
            "This wiki is password protected and has no password yet. Choose one: its documents \
             are encrypted under it.",
            "Set password",
        ),
        KbPasswordMode::Unlock => (
            format!("Unlock {name}"),
            "This wiki is locked. Enter its password to read and edit its documents.",
            "Unlock",
        ),
        KbPasswordMode::Change => (
            format!("Change the password of {name}"),
            "Every document is re-encrypted under the new password. The current one is asked for \
             even though the wiki is open.",
            "Change password",
        ),
    };

    let mut body = div()
        .key_context(CONTEXT)
        .on_action(cx.listener(|this, _: &KbPasswordSubmit, window, cx| {
            this.submit_kb_password(window, cx)
        }))
        .flex()
        .flex_col()
        .gap_2()
        .pt_3()
        .child(modal_note(note));

    if dialog.mode.asks_current() {
        body = body.child(field(&dialog.current, window, cx));
    }
    if dialog.mode.asks_new() {
        body = body
            .child(field(&dialog.new, window, cx))
            .child(field(&dialog.confirm, window, cx));
    }
    if dialog.mode == KbPasswordMode::Set {
        body = body.child(
            div()
                .w_full()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::warning())
                .child("If you forget this password the wiki cannot be recovered."),
        );
    }
    body = body.child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(check_box(
                "kb-password-remember",
                dialog.remember,
                cx.listener(|this, _, _, cx| this.toggle_kb_password_remember(cx)),
            ))
            .child(
                div()
                    .text_size(theme::font(Family::Chrome, Role::Body))
                    .text_color(theme::text())
                    .child("Remember in keychain"),
            ),
    );
    if let Some(error) = dialog.error.clone() {
        body = body.child(
            div()
                .w_full()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::danger())
                .child(error),
        );
    }

    let busy = dialog.busy;
    let footer = div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .justify_end()
        .gap_2()
        .child(ghost_button(
            "kb-password-cancel",
            None,
            "Cancel",
            cx.listener(|this, _, _, cx| this.cancel_kb_password(cx)),
        ))
        .child(
            primary_button(
                "kb-password-confirm",
                None,
                confirm_label,
                cx.listener(|this, _, window, cx| this.submit_kb_password(window, cx)),
            )
            .when(busy, |button| button.opacity(0.5)),
        )
        .into_any_element();

    modal(
        "kb-password",
        theme::accent(),
        &title,
        body.into_any_element(),
        footer,
        crate::ui::dismiss(&view, Layer::KbPassword, |this, _, cx| {
            this.cancel_kb_password(cx)
        }),
        window,
    )
}

/// One masked field, framed as the DB prompt's is. The focus ring follows the field's own handle.
fn field(input: &Entity<InputState>, window: &Window, cx: &App) -> AnyElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    crate::ui::kit::field(theme::border(), focused)
        .h(px(30.))
        .px_2()
        .child(Input::new(input).appearance(false).mask_toggle())
        .into_any_element()
}
