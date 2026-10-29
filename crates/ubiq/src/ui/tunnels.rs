//! The tunnel editor (`D227`), shared by every form that edits a [`TunnelSpec`] list — the Drones
//! section's per-host Tunnels for a saved SSH host, and any embedder's own carrier form.
//!
//! What is shared is the row and its inputs, the typed-field parse and the status lines; where the
//! list lives is the caller's. A row reports its switches as a [`TunnelEdit`] through the caller's
//! callback, and [`TunnelEdit::apply`] performs it on the caller's two parallel vectors.
//!
//! Text inputs are built on a click — the only place a `&mut Window` is at hand — so a form is
//! opened, never drawn before it exists.

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, Focusable,
    IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use ubiq_proto::settings::{TunnelKind, TunnelSpec};
use ubiq_tunnel::Live;

use crate::app::AppState;
use crate::theme::{self, Family, Role};
use crate::ui::kit::{RowAction, check_box, choice_pill, field, row_action};

/// A tunnel row's text fields; its switches live on the spec itself. TCP reads `host` and `port`,
/// HTTP reads `url` and `content_types`; both sets exist, so switching the kind loses nothing typed.
#[derive(Clone)]
pub struct TunnelInputs {
    local_port: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    url: Entity<InputState>,
    /// HTTP only, with the Host rewrite on: an override of the URL's host.
    host_header: Entity<InputState>,
    content_types: Entity<TextareaState>,
}

impl TunnelInputs {
    pub fn new(spec: &TunnelSpec, window: &mut Window, cx: &mut Context<AppState>) -> Self {
        let port = |p: u16| if p == 0 { String::new() } else { p.to_string() };
        Self {
            local_port: input(window, cx, "local", &port(spec.local_port)),
            host: input(window, cx, "remote host", &spec.host),
            port: input(window, cx, "port", &port(spec.port)),
            url: {
                let url = input(window, cx, "https://host:port/", &spec.remote_url());
                // The certificate switch follows the scheme as it is typed.
                cx.subscribe(&url, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
                .detach();
                url
            },
            host_header: input(
                window,
                cx,
                "Host header value (default: from URL)",
                spec.host_rewrite.as_deref().unwrap_or(""),
            ),
            content_types: {
                let area = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("text/html, application/*+json \u{2014} one per line")
                        .auto_grow(3, 8)
                });
                let value = spec.content_types.join("\n");
                area.update(cx, |area, cx| area.set_value(value, window, cx));
                area
            },
        }
    }

    /// What is typed, now.
    pub fn fields(&self, cx: &App) -> TunnelFields {
        let read = |e: &Entity<InputState>| e.read(cx).value().to_string();
        TunnelFields {
            local_port: read(&self.local_port),
            host: read(&self.host),
            port: read(&self.port),
            url: read(&self.url),
            host_header: read(&self.host_header),
            content_types: self.content_types.read(cx).value().to_string(),
        }
    }
}

fn input(
    window: &mut Window,
    cx: &mut Context<AppState>,
    placeholder: &'static str,
    value: &str,
) -> Entity<InputState> {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    if !value.is_empty() {
        input.update(cx, |input, cx| input.set_value(value.to_string(), window, cx));
    }
    input
}

/// A tunnel row's text fields, as typed.
#[derive(Clone, Debug, Default)]
pub struct TunnelFields {
    pub local_port: String,
    /// TCP only.
    pub host: String,
    /// TCP only.
    pub port: String,
    /// HTTP only: `http[s]://host[:port]/`.
    pub url: String,
    /// HTTP only: the `Host` override; empty is the URL's.
    pub host_header: String,
    /// HTTP only: patterns separated by newlines or commas.
    pub content_types: String,
}

/// A number field: empty is `None`.
pub fn optional<T: std::str::FromStr>(text: &str, what: &str) -> Result<Option<T>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse()
        .map(Some)
        .map_err(|_| format!("{what} is not a number"))
}

/// Put the typed fields into the specs, in the same order, and validate what a row alone can.
pub fn apply_fields(specs: &mut [TunnelSpec], fields: &[TunnelFields]) -> Result<(), String> {
    for (n, (spec, typed)) in specs.iter_mut().zip(fields).enumerate() {
        let what = |part: &str| format!("Tunnel {}'s {part}", n + 1);
        spec.local_port = optional(&typed.local_port, &what("local port"))?.unwrap_or(0);
        match spec.kind {
            TunnelKind::Tcp => {
                spec.host = typed.host.trim().to_string();
                spec.port = optional(&typed.port, &what("remote port"))?.unwrap_or(0);
                spec.upstream_tls = false;
            }
            TunnelKind::Http => {
                spec.set_remote_url(&typed.url)
                    .map_err(|e| format!("{}: {e}", what("remote URL")))?;
                spec.host_rewrite =
                    Some(typed.host_header.trim().to_string()).filter(|h| !h.is_empty());
                spec.content_types = typed
                    .content_types
                    .split([',', '\n'])
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(String::from)
                    .collect();
            }
        }
    }
    Ok(())
}

/// What a whole list must hold: both ports set, a remote host, no local port forwarded twice.
pub fn check_tunnels(tunnels: &[TunnelSpec]) -> Result<(), String> {
    let mut ports = std::collections::HashSet::new();
    for (n, t) in tunnels.iter().enumerate() {
        let n = n + 1;
        if t.local_port == 0 || t.port == 0 {
            return Err(format!("Tunnel {n}: a port is outside 1-65535"));
        }
        if t.host.trim().is_empty() {
            return Err(format!("Tunnel {n} needs a remote host"));
        }
        if t.enabled && !ports.insert(t.local_port) {
            return Err(format!(
                "Tunnel {n}: local port {} is already forwarded",
                t.local_port
            ));
        }
    }
    Ok(())
}

/// What the Add button appends: TCP to `localhost`, enabled, ports still to type.
pub fn new_spec() -> TunnelSpec {
    TunnelSpec {
        id: ubiq_proto::ids::HostSaveId::generate().to_string(),
        kind: TunnelKind::Tcp,
        local_port: 0,
        host: "localhost".into(),
        port: 0,
        upstream_tls: false,
        insecure: false,
        enabled: true,
        rewrite_host: true,
        host_rewrite: None,
        rewrite_content: false,
        content_types: TunnelSpec::default_content_types(),
    }
}

/// One of a row's switches, reported to the form that owns the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelEdit {
    Kind(TunnelKind),
    Enabled,
    RewriteHost,
    RewriteContent,
    Insecure,
    Remove,
}

impl TunnelEdit {
    /// Perform it on the spec `id` of a form's two parallel lists.
    pub fn apply(self, specs: &mut Vec<TunnelSpec>, inputs: &mut Vec<TunnelInputs>, id: &str) {
        let Some(at) = specs.iter().position(|s| s.id == id) else {
            return;
        };
        let spec = &mut specs[at];
        match self {
            Self::Kind(kind) => spec.kind = kind,
            Self::Enabled => spec.enabled = !spec.enabled,
            Self::RewriteHost => spec.rewrite_host = !spec.rewrite_host,
            Self::RewriteContent => spec.rewrite_content = !spec.rewrite_content,
            Self::Insecure => spec.insecure = !spec.insecure,
            Self::Remove => {
                specs.remove(at);
                if at < inputs.len() {
                    inputs.remove(at);
                }
            }
        }
    }
}

/// How a row reports a switch: the spec's id and the edit.
pub type OnEdit = Rc<dyn Fn(&mut AppState, &str, TunnelEdit, &mut Context<AppState>)>;

fn click(
    view: &Entity<AppState>,
    on_edit: &OnEdit,
    id: &str,
    edit: TunnelEdit,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let (view, on_edit, id) = (view.clone(), on_edit.clone(), id.to_string());
    move |_, _, cx| {
        view.update(cx, |this, cx| on_edit(this, &id, edit, cx));
    }
}

/// One tunnel of a form: kind, `local → host:port` (TCP) or `local → URL` (HTTP), on, remove.
/// HTTP adds the two rewrite switches — and the certificate one for an `https` URL — on a second
/// line, and the content types when the content rewrite is on. `prefix` keeps element ids unique
/// across forms.
pub fn tunnel_row(
    view: &Entity<AppState>,
    prefix: &str,
    spec: &TunnelSpec,
    inputs: &TunnelInputs,
    on_edit: &OnEdit,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let sid = spec.id.as_str();
    let eid = |part: &str| ElementId::Name(format!("{prefix}-tunnel-{part}-{sid}").into());
    let http = spec.kind == TunnelKind::Http;
    let kind = |label: &'static str, kind: TunnelKind| {
        choice_pill(
            eid(label),
            label,
            spec.kind == kind,
            click(view, on_edit, sid, TunnelEdit::Kind(kind)),
        )
    };
    let toggle = |part: &str, on: bool, label: &'static str, edit: TunnelEdit| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(check_box(eid(part), on, click(view, on_edit, sid, edit)))
            .child(small(label))
    };
    let mut first = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(kind("TCP", TunnelKind::Tcp))
        .child(kind("HTTP", TunnelKind::Http))
        .child(sized_field(&inputs.local_port, 80., window, cx))
        .child(small("\u{2192}"));
    first = if http {
        first.child(sized_field(&inputs.url, 260., window, cx))
    } else {
        first
            .child(sized_field(&inputs.host, 160., window, cx))
            .child(small(":"))
            .child(sized_field(&inputs.port, 80., window, cx))
    };
    let first = first
        .child(toggle("on", spec.enabled, "Enabled", TunnelEdit::Enabled))
        .child(row_action(
            eid("del"),
            RowAction::Delete,
            click(view, on_edit, sid, TunnelEdit::Remove),
        ));
    let mut rows = div().flex().flex_col().gap_1().py_1().child(first);
    if http {
        let https = inputs
            .url
            .read(cx)
            .value()
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("https:");
        let mut second = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(toggle(
                "host",
                spec.rewrite_host,
                "Rewrite Host header",
                TunnelEdit::RewriteHost,
            ));
        if spec.rewrite_host {
            second = second.child(sized_field(&inputs.host_header, 220., window, cx));
        }
        second = second.child(toggle(
            "content",
            spec.rewrite_content,
            "Rewrite host in contents (experimental)",
            TunnelEdit::RewriteContent,
        ));
        if https {
            second = second.child(toggle(
                "insecure",
                spec.insecure,
                "Skip certificate check",
                TunnelEdit::Insecure,
            ));
        }
        rows = rows.child(second);
        if spec.rewrite_content {
            rows = rows.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(small("Content types processed"))
                    .child(sized_area(&inputs.content_types, 260., window, cx)),
            );
        }
    }
    rows.into_any_element()
}

/// One line per enabled tunnel: where it goes, whether it listens, its traffic and last error.
pub fn status_lines(specs: &[TunnelSpec], live: &Live) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if let Some(error) = &live.error {
        out.push(line(format!("Tunnels: {error}"), theme::danger()));
    }
    for s in &live.status {
        let to = specs
            .iter()
            .find(|spec| spec.id == s.id)
            .map(|spec| match spec.kind {
                TunnelKind::Tcp => format!("{}:{}", spec.host, spec.port),
                TunnelKind::Http => spec.remote_url(),
            })
            .unwrap_or_default();
        let state = match (s.listening, live.linked) {
            (false, _) => "not listening",
            (true, true) => "linked",
            (true, false) => "waiting for the drone",
        };
        out.push(line(
            format!(
                "127.0.0.1:{} \u{2192} {to} \u{b7} {state} \u{b7} {} open, {} total \u{b7} \u{2191}{} \u{2193}{}",
                s.local_port,
                s.active,
                s.conns,
                bytes(s.bytes_up),
                bytes(s.bytes_down),
            ),
            theme::text_faint(),
        ));
        if let Some(error) = &s.last_error {
            out.push(line(error.clone(), theme::danger()));
        }
    }
    out
}

/// A byte count, binary-scaled.
pub fn bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KiB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

fn line(text: String, colour: gpui::Rgba) -> AnyElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(colour)
        .child(SharedString::from(text))
        .into_any_element()
}

fn small(text: &'static str) -> AnyElement {
    div()
        .text_size(theme::font(Family::Chrome, Role::Meta))
        .text_color(theme::text_muted())
        .child(text)
        .into_any_element()
}

/// A single-line input of a fixed width.
pub fn sized_field(input: &Entity<InputState>, width: f32, window: &Window, cx: &App) -> AnyElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    field(theme::border(), focused)
        .w(px(width))
        .h(px(30.))
        .px_2()
        .child(Input::new(input).appearance(false))
        .into_any_element()
}

fn sized_area(input: &Entity<TextareaState>, width: f32, window: &Window, cx: &App) -> AnyElement {
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    field(theme::border(), focused)
        .w(px(width))
        .px_2()
        .py_1()
        .child(
            Textarea::new(input)
                .appearance(false)
                .bordered(false)
                .w_full()
                .text_size(theme::font(Family::Chrome, Role::Body)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_parse_per_kind() {
        let mut specs = vec![new_spec()];
        specs[0].kind = TunnelKind::Http;
        let row = |local: &str, url: &str| TunnelFields {
            local_port: local.into(),
            url: url.into(),
            content_types: "text/html,\n application/json".into(),
            ..TunnelFields::default()
        };
        apply_fields(&mut specs, &[row("8080", " https://web:8443/ ")]).unwrap();
        let s = &specs[0];
        assert_eq!((s.local_port, s.host.as_str(), s.port), (8080, "web", 8443));
        assert!(s.upstream_tls);
        assert_eq!(s.content_types, ["text/html", "application/json"]);
        assert!(apply_fields(&mut specs, &[row("x", "http://web")]).is_err());

        specs[0].kind = TunnelKind::Tcp;
        let tcp = TunnelFields {
            local_port: "5433".into(),
            host: " db ".into(),
            port: "5432".into(),
            ..TunnelFields::default()
        };
        apply_fields(&mut specs, &[tcp]).unwrap();
        let s = &specs[0];
        assert_eq!((s.host.as_str(), s.port, s.upstream_tls), ("db", 5432, false));
    }
}
