//! The "Connect to a remote host" modal, raised from the titlebar's network icon.
//!
//! One [`modal`] with four steps drawn as its body and footer change — [`RemoteConnectStep`] says
//! which. The shape is `ui/settings.rs`'s `connect` function's own: a `match` picking a title, a
//! body and a footer, with the fields living in `AppState` rather than mirrored into
//! `RemoteConnectState`, exactly as that flow's fields do.

use gpui::{AnyElement, Context, Focusable, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::input::Input;

use crate::app::AppState;
use crate::state::remote::RemoteConnectStep;
use crate::theme;
use crate::ui::kit::{
    check_box, field, ghost_button, label_block, modal, modal_note, primary_button, toggle_pill,
};
use ubiq_proto::settings::RemoteScheme;

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(state) = &app.workbench.remote_connect else {
        return div().into_any_element();
    };

    let (title, body, footer) = match &state.step {
        RemoteConnectStep::Editing => (
            "Connect to a remote host",
            editing_body(app, window, cx),
            editing_footer(app, cx),
        ),
        RemoteConnectStep::Connecting { .. } => (
            "Connecting",
            div()
                .pt_3()
                .child(modal_note("Reaching that host\u{2026}"))
                .into_any_element(),
            cancel_footer("remote-connect-cancel-connecting", cx),
        ),
        RemoteConnectStep::Connected { label } => (
            "Connected",
            div()
                .pt_3()
                .child(modal_note(&format!(
                    "Attached to {label}. Its projects join the ones already listed; the menus \
                     for what a new pane can run stay this machine's."
                )))
                .into_any_element(),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "remote-connect-close-connected",
                    None,
                    "Close",
                    cx.listener(|this, _, window, cx| this.cancel_remote_connect(window, cx)),
                ))
                .into_any_element(),
        ),
        RemoteConnectStep::Failed { reason } => (
            "Not connected",
            div().pt_3().child(modal_note(reason)).into_any_element(),
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(ghost_button(
                    "remote-connect-close-failed",
                    None,
                    "Close",
                    cx.listener(|this, _, window, cx| this.cancel_remote_connect(window, cx)),
                ))
                .child(primary_button(
                    "remote-connect-retry",
                    None,
                    "Try again",
                    cx.listener(|this, _, _, cx| this.retry_remote_connect(cx)),
                ))
                .into_any_element(),
        ),
    };

    modal(
        "remote-connect-modal",
        theme::accent(),
        title,
        body,
        footer,
        crate::ui::handler(&cx.entity(), |this, window, cx| {
            this.cancel_remote_connect(window, cx)
        }),
        window,
    )
}

/// Both fields plus the protocol: an address — which absorbs a whole pasted connection
/// string, see `AppState::apply_remote_address_input` — a token, and whether the dial wraps the
/// socket in TLS first.
fn editing_body(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let address_focused = app
        .remote_address_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let token_focused = app
        .remote_token_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let scheme = app
        .workbench
        .remote_connect
        .as_ref()
        .map(|state| state.scheme)
        .unwrap_or(RemoteScheme::Http);
    let trust_insecure = app
        .workbench
        .remote_connect
        .as_ref()
        .map(|state| state.trust_insecure)
        .unwrap_or(false);

    div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(modal_note(
            "Paste the connection string a running `ubiq --serve` printed, or type the address \
             and token separately.",
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Address",
                    "A host, or host:port. Pasting the whole connection string also fills the \
                     token below.",
                ))
                .child(
                    field(theme::border(), address_focused)
                        .h(px(30.))
                        .px_2()
                        .child(Input::new(&app.remote_address_input).appearance(false)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block("Token", ""))
                .child(
                    field(theme::border(), token_focused)
                        .h(px(30.))
                        .px_2()
                        .child(Input::new(&app.remote_token_input).appearance(false)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Protocol",
                    "Plaintext unless the host was started with `--tls-cert` and `--tls-key`.",
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(toggle_pill(
                            "remote-connect-scheme-http",
                            "http",
                            theme::accent(),
                            scheme == RemoteScheme::Http,
                            cx.listener(|this, _, _, cx| {
                                this.set_remote_scheme(RemoteScheme::Http, cx)
                            }),
                        ))
                        .child(toggle_pill(
                            "remote-connect-scheme-https",
                            "https",
                            theme::accent(),
                            scheme == RemoteScheme::Https,
                            cx.listener(|this, _, _, cx| {
                                this.set_remote_scheme(RemoteScheme::Https, cx)
                            }),
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(check_box(
                    "remote-connect-trust",
                    trust_insecure,
                    cx.listener(|this, _, _, cx| {
                        let trust = this
                            .workbench
                            .remote_connect
                            .as_ref()
                            .map(|state| !state.trust_insecure)
                            .unwrap_or(false);
                        this.set_remote_trust(trust, cx)
                    }),
                ))
                .child(label_block(
                    "Trust certificate",
                    "Skip verification for a self-signed host certificate. Only ever needed \
                     with https, and shown on the saved entry afterwards.",
                )),
        )
        .into_any_element()
}

fn editing_footer(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let ready = !app.remote_address_input.read(cx).value().trim().is_empty()
        && !app.remote_token_input.read(cx).value().trim().is_empty();
    let connect = primary_button(
        "remote-connect-submit",
        None,
        "Connect",
        cx.listener(|this, _, _, cx| this.try_connect_remote(cx)),
    );
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "remote-connect-cancel-editing",
            None,
            "Cancel",
            cx.listener(|this, _, window, cx| this.cancel_remote_connect(window, cx)),
        ))
        .child(if ready { connect } else { connect.opacity(0.5) })
        .into_any_element()
}

fn cancel_footer(id: &'static str, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            id,
            None,
            "Cancel",
            cx.listener(|this, _, window, cx| this.cancel_remote_connect(window, cx)),
        ))
        .into_any_element()
}
