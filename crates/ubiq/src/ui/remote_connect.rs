//! The "Connect to a remote host" modal, raised from the titlebar's network icon.
//!
//! One [`modal`] with four steps drawn as its body and footer change — [`RemoteConnectStep`] says
//! which. The shape is `ui/settings.rs`'s `connect` function's own: a `match` picking a title, a
//! body and a footer, with the fields living in `AppState` rather than mirrored into
//! `RemoteConnectState`, exactly as that flow's fields do.

use gpui::{AnyElement, Context, Focusable, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::input::Input;

use crate::app::AppState;
use crate::state::remote::{ConnectMode, RemoteConnectStep};
use crate::theme;
use crate::ui::kit::{
    check_box, field, ghost_button, label_block, modal, modal_note, primary_button, toggle_pill,
};
use ubiq_proto::settings::{RemoteScheme, SshAuth};

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
        RemoteConnectStep::Connecting { .. } if state.mode == ConnectMode::Ssh => (
            "Connecting",
            div()
                .pt_3()
                .child(modal_note("Starting a drone over ssh\u{2026}"))
                .into_any_element(),
            cancel_footer("remote-connect-cancel-connecting", cx),
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

/// The mode picker, then whichever half it chose.
///
/// Two carriers, one modal: a `ubiq --serve` host is an address and a token, and a drone is a
/// saved SSH profile and a folder on the far machine. They share nothing but the buttons under
/// them, which is why this is a pick at the top rather than two entries in the titlebar — what
/// the user is doing is the same thing, and only the way the bytes travel differs (`D116`).
fn editing_body(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let mode = app
        .workbench
        .remote_connect
        .as_ref()
        .map(|state| state.mode)
        .unwrap_or_default();

    div()
        .flex()
        .flex_col()
        .gap_3()
        .pt_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(toggle_pill(
                    "remote-connect-mode-socket",
                    "Ubiq host",
                    theme::accent(),
                    mode == ConnectMode::Socket,
                    cx.listener(|this, _, _, cx| this.set_remote_mode(ConnectMode::Socket, cx)),
                ))
                .child(toggle_pill(
                    "remote-connect-mode-ssh",
                    "Drone over ssh",
                    theme::accent(),
                    mode == ConnectMode::Ssh,
                    cx.listener(|this, _, _, cx| this.set_remote_mode(ConnectMode::Ssh, cx)),
                )),
        )
        .child(match mode {
            ConnectMode::Socket => socket_body(app, window, cx),
            ConnectMode::Ssh => ssh_body(app, window, cx),
        })
        .into_any_element()
}

/// The SSH half: which saved profile to reach the far machine with, and which folder the drone
/// there should serve.
///
/// The profiles are the settings page's list, shown here and never edited here — a target is a
/// thing the user keeps, and this modal is where one is used.
fn ssh_body(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let picked = app
        .workbench
        .remote_connect
        .as_ref()
        .and_then(|state| state.profile);
    let root_focused = app
        .remote_root_input
        .read(cx)
        .focus_handle(cx)
        .is_focused(window);
    let profiles = &app.workbench.settings.host.ssh_profiles;

    let mut rows = div().flex().flex_col().gap_1();
    if profiles.is_empty() {
        rows = rows.child(modal_note(
            "No SSH profiles yet. Add one in Settings \u{203a} SSH profiles, then come back here.",
        ));
    }
    for profile in profiles {
        let id = profile.id;
        let method = match &profile.auth {
            SshAuth::Agent => "agent".to_string(),
            SshAuth::KeyFile { path, .. } => format!("key {path}"),
            SshAuth::Password { .. } => "password".to_string(),
            SshAuth::ConfigAlias => "ssh config".to_string(),
        };
        let where_to = if matches!(profile.auth, SshAuth::ConfigAlias) {
            profile.host.clone()
        } else if profile.user.trim().is_empty() {
            format!("{}:{}", profile.host, profile.port)
        } else {
            format!("{}@{}:{}", profile.user, profile.host, profile.port)
        };
        rows = rows.child(toggle_pill(
            crate::ui::eid("remote-connect-profile", id),
            format!("{} \u{2014} {where_to} ({method})", profile.name),
            theme::accent(),
            picked == Some(id),
            cx.listener(move |this, _, _, cx| this.set_remote_profile(id, cx)),
        ));
    }

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(modal_note(
            "Ubiq runs `ubiq-drone --stdio` on the far machine over ssh, and speaks to it the \
             way it speaks to any other host.",
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "SSH profile",
                    "Where to connect, and how it authenticates.",
                ))
                .child(rows),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(label_block(
                    "Folder",
                    "The folder on that machine the drone serves. Empty is the login directory.",
                ))
                .child(
                    field(theme::border(), root_focused)
                        .h(px(30.))
                        .px_2()
                        .child(Input::new(&app.remote_root_input).appearance(false)),
                ),
        )
        .into_any_element()
}

/// Both fields plus the protocol: an address — which absorbs a whole pasted connection
/// string, see `AppState::apply_remote_address_input` — a token, and whether the dial wraps the
/// socket in TLS first.
fn socket_body(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
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
    let state = app.workbench.remote_connect.as_ref();
    let ready = match state.map(|state| state.mode).unwrap_or_default() {
        // A drone needs a profile and nothing else: the folder is optional, and the profile
        // carries everything a dial would otherwise have to be typed.
        ConnectMode::Ssh => state.is_some_and(|state| state.profile.is_some()),
        ConnectMode::Socket => {
            !app.remote_address_input.read(cx).value().trim().is_empty()
                && !app.remote_token_input.read(cx).value().trim().is_empty()
        }
    };
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
