//! The "Remote hosts" manager panel, raised from the titlebar's network icon.
//!
//! One [`modal`] listing every saved host and every live connection: what each is doing, what
//! its host said about itself, and a row of buttons — test, connect or reconnect, disconnect,
//! rename, forget — plus a "New connection" footer that raises the connect modal on top. The
//! rename prompt paints over it, the same way settings paints its account dialogs over the page
//! that raised them.

use gpui::{AnyElement, Context, ElementId, IntoElement, ParentElement, SharedString, Styled, Window, div};

use crate::app::{AppState, HostRef, LiveRemote};
use crate::theme;
use crate::ui::kit::{
    badge, elided, ghost_button, label_block, modal, modal_note, primary_button, prompt_modal,
    setting_row,
};
use ubiq_proto::settings::{RemoteScheme, SavedRemoteHost};

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    if !app.workbench.remote_manager.open {
        return div().into_any_element();
    }
    if let Some(element) = rename_prompt(app, window, cx) {
        return element;
    }

    let saved = app.workbench.settings.host.remote_hosts.clone();
    let live = app.live_remotes();
    let mut body = div().flex().flex_col().gap_3().pt_3();
    if saved.is_empty() && live.is_empty() {
        body = body.child(modal_note(
            "No remote hosts yet. A host becomes reachable from here the first time a dial to \
             it succeeds; its token is kept in the keychain, so reconnecting never asks again.",
        ));
    }
    for host in &saved {
        body = body.child(saved_row(app, host, &live, cx));
    }
    for conn in live
        .iter()
        .filter(|conn| !saved.iter().any(|host| owns_connection(host, conn)))
    {
        body = body.child(orphan_row(app, conn, cx));
    }

    let footer = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            "remote-hosts-close",
            None,
            "Close",
            cx.listener(|this, _, _, cx| this.close_remote_manager(cx)),
        ))
        .child(primary_button(
            "remote-hosts-new",
            None,
            "New connection",
            cx.listener(|this, _, window, cx| this.open_remote_connect(window, cx)),
        ))
        .into_any_element();

    modal(
        "remote-hosts-modal",
        theme::accent(),
        "Remote hosts",
        body.into_any_element(),
        footer,
        crate::ui::handler(&cx.entity(), |this, _, cx| {
            this.close_remote_manager(cx)
        }),
        window,
    )
}

/// Whether a live connection belongs to a saved entry: by stable id, or — for entries written
/// before ids existed — by the address a one-off dial lands under.
fn owns_connection(host: &SavedRemoteHost, conn: &LiveRemote) -> bool {
    if !host.id.is_empty() {
        conn.save_id == host.id
    } else {
        conn.save_id.is_empty() && conn.address == host.address
    }
}

/// One saved host: its name and address, what it is doing, what its host said about itself, and
/// the row of buttons that acts on it.
fn saved_row(
    app: &AppState,
    host: &SavedRemoteHost,
    live: &[LiveRemote],
    cx: &mut Context<AppState>,
) -> AnyElement {
    let key = crate::app::host_secrets::key_for(&host.id, &host.address);
    let conn = live.iter().find(|conn| owns_connection(host, conn));
    let reconnect = app.workbench.settings.reconnects.get(&key).cloned();
    let testing = app.workbench.remote_manager.testing.contains(&key);
    let test = app.workbench.remote_manager.tests.get(&key).cloned();

    let (chip, colour) = if conn.is_some() {
        ("attached", theme::success())
    } else if reconnect.is_some() {
        ("reconnecting", theme::warning())
    } else if app.workbench.settings.failed_hosts.contains(&host.address) {
        ("last attempt failed", theme::danger())
    } else {
        ("saved", theme::text_faint())
    };

    let mut note = match host.scheme {
        RemoteScheme::Http => format!("http://{}", host.address),
        RemoteScheme::Https => format!("https://{}", host.address),
    };
    if host.trust_insecure {
        note.push_str(" · certificate not verified");
    }
    if let Some(conn) = conn
        && let Some(meta) = app.remote_host_meta(HostRef::Remote(conn.id))
    {
        note.push_str(&format!(" · {}", meta.summary()));
    }
    if let Some(state) = &reconnect
        && !state.error.is_empty()
    {
        note.push_str(&format!(" · {}", state.error));
    }
    if let Some(outcome) = &test {
        note.push_str(&format!(" · test: {}", outcome.report));
    }

    let save_id = host.id.clone();
    let save_id2 = host.id.clone();
    let save_id3 = host.id.clone();
    let key_clone = key.clone();
    let key_test = key.clone();
    let live_id = conn.map(|conn| conn.id);

    let mut buttons = div().flex().items_center().gap_2();
    buttons = buttons.child(badge(chip, colour));
    if testing {
        buttons = buttons.child(badge("testing…", theme::text_faint()));
    } else {
        buttons = buttons.child(ghost_button(
            element_name(&key, "test"),
            None,
            "Test",
            cx.listener(move |this, _, _, cx| this.test_saved_host(key_test.clone(), cx)),
        ));
    }
    if let Some(id) = live_id {
        buttons = buttons.child(ghost_button(
            element_name(&key, "disconnect"),
            None,
            "Disconnect",
            cx.listener(move |this, _, _, cx| {
                this.disconnect_host(id, cx);
            }),
        ));
    } else if reconnect.is_some() {
        buttons = buttons.child(ghost_button(
            element_name(&key, "reconnect"),
            None,
            "Reconnect now",
            cx.listener(move |this, _, _, cx| this.start_reconnect(&key_clone, cx)),
        ));
    } else {
        buttons = buttons.child(ghost_button(
            element_name(&key, "connect"),
            None,
            "Connect",
            cx.listener(move |this, _, window, cx| {
                this.connect_saved_host(save_id.clone(), window, cx)
            }),
        ));
    }
    buttons = buttons
        .child(ghost_button(
            element_name(&key, "rename"),
            None,
            "Rename",
            cx.listener(move |this, _, window, cx| {
                this.begin_rename_remote_host(save_id2.clone(), window, cx)
            }),
        ))
        .child(ghost_button(
            element_name(&key, "forget"),
            None,
            "Forget",
            cx.listener(move |this, _, _, cx| {
                this.forget_remote_host(save_id3.clone(), cx)
            }),
        ));

    setting_row(
        &host.name,
        &note,
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                elided(
                    element_name(&key, "name"),
                    SharedString::from(host.name.clone()),
                    theme::text(),
                    theme::font(theme::Family::Chrome, theme::Role::Body),
                ),
            )
            .child(buttons)
            .into_any_element(),
    )
}

/// A live connection with no saved entry behind it — a one-off dial, or an entry forgotten
/// while attached. Disconnect is all there is to do with it; forgetting already happened.
fn orphan_row(app: &AppState, conn: &LiveRemote, cx: &mut Context<AppState>) -> AnyElement {
    let mut note = conn.address.clone();
    if let Some(meta) = app.remote_host_meta(HostRef::Remote(conn.id)) {
        note.push_str(&format!(" · {}", meta.summary()));
    }
    let id = conn.id;
    setting_row(
        &conn.label,
        &note,
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(badge("attached", theme::success()))
            .child(ghost_button(
                element_name(&conn.address, "disconnect"),
                None,
                "Disconnect",
                cx.listener(move |this, _, _, cx| {
                    this.disconnect_host(id, cx);
                }),
            ))
            .into_any_element(),
    )
}

/// The rename prompt, over the panel. The entry's own name is the starting text; an empty or
/// unchanged name dims the confirm rather than refusing.
fn rename_prompt(
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let key = app.workbench.remote_manager.renaming.clone()?;
    let host = app
        .workbench
        .settings
        .host
        .remote_hosts
        .iter()
        .find(|host| crate::app::host_secrets::key_for(&host.id, &host.address) == key)?;
    let value = app.remote_rename_input.read(cx).value().to_string();
    let enabled = !value.trim().is_empty() && value.trim() != host.name;
    let view = cx.entity();
    Some(prompt_modal(
        "remote-hosts-rename",
        "Rename host",
        Some("The address, scheme and saved token stay exactly as they are."),
        "Name",
        &app.remote_rename_input,
        "Rename",
        enabled,
        crate::ui::handler(&view, |this, _, cx| this.confirm_rename_remote_host(cx)),
        crate::ui::handler(&view, |this, _, cx| this.cancel_rename_remote_host(cx)),
        window,
        cx,
    ))
}

fn element_name(key: &str, what: &str) -> ElementId {
    ElementId::Name(format!("remote-hosts-{key}-{what}").into())
}

/// What the settings page's Hosts section links to — the panel, not the connect modal.
pub fn manager_link(cx: &mut Context<AppState>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .pt_4()
        .child(crate::ui::kit::section_label("Remote hosts"))
        .child(label_block(
            "Connections",
            "Saved hosts, live connections and reconnects live in the manager panel.",
        ))
        .child(ghost_button(
            "app-settings-remote-hosts-open",
            None,
            "Open remote hosts",
            cx.listener(|this, _, _, cx| this.open_remote_manager(cx)),
        ))
        .into_any_element()
}
