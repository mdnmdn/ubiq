//! The "Connect to a remote host" modal's state, and the pure parsing it needs.
//!
//! Nothing here opens a socket. `crate::app::remote_connect` does that, off the GPUI thread, and
//! reports back through the same `cx.background_spawn` / `cx.spawn` pairing every other
//! long-running action in this crate uses — see that module for the transport. What lives here is
//! only what the modal draws, plus [`parse_connection_string`] and [`with_default_port`], which
//! are pure precisely so they can be pinned down by a test with no window, no host and no socket
//! at all.

use std::sync::atomic::{AtomicU64, Ordering};

/// The port a remote host listens on when nothing else is said. Matches
/// `ubiq_host::remote::serve`'s own default bind port — kept as a constant here rather than
/// imported, since `ubiq-host` is not a dependency of this crate and the number is a fact about
/// the wire protocol's convention, not about that crate's types.
pub const DEFAULT_PORT: u16 = 7420;

/// One attempt to reach a remote host.
///
/// Minted when a connect starts, and compared against on the way back: a background dial that is
/// still running when the user cancels or retries must not paint over whatever replaced it, so
/// every outcome the transport reports carries the id it was asked to answer, and a result naming
/// a stale one is discarded rather than drawn. The same discipline `ConnectId`, `RepoQueryId` and
/// every other in-flight id in this crate already follows — this one stays local to the interface
/// (nothing about a TCP dial belongs in the wire contract), so it is minted here rather than in
/// `ubiq_proto::ids`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AttemptId(u64);

impl AttemptId {
    pub fn generate() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Which stage the modal is showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteConnectStep {
    /// The two fields are being typed into. The modal's idle state.
    Editing,
    /// A dial is running in the background for `attempt`.
    Connecting { attempt: AttemptId },
    /// The dial answered with a `Client` and it was registered on the bus. `label` is the address
    /// that succeeded, shown back so the user can tell which host they just added.
    Connected { label: String },
    /// The dial failed, or the host was reached and refused the token. `reason` is worded for the
    /// modal to show directly — see `app::remote_connect::ConnectFailure`'s `Display`.
    Failed { reason: String },
}

/// The connect-to-a-remote-host modal, while it is up.
///
/// Composed into `ui/shell.rs` on the same terms as `SettingsState::connect`: an `Option` field,
/// `Some` only while the modal is on screen, painted from the window's root so it outlives
/// whatever raised it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteConnectState {
    pub step: RemoteConnectStep,
    /// Set when this dial was started from the Hosts section's "connect" row on a saved host,
    /// rather than typed fresh from the titlebar — the saved host's own name. Carried through so
    /// a successful attach is attributed to that name instead of the bare address it dialled
    /// under, the same way `saved` marks this as a reconnect worth reflecting in
    /// `SettingsState::failed_hosts` rather than an ordinary first dial.
    pub saved_name: Option<String>,
}

impl Default for RemoteConnectState {
    fn default() -> Self {
        Self {
            step: RemoteConnectStep::Editing,
            saved_name: None,
        }
    }
}

/// What pasting text into the address field yields.
///
/// A user reaching for this modal has one thing to paste: the whole `http://host:port?token=…`
/// string the host printed to its terminal. Asking them to split it into two fields by hand before
/// they can paste anything is the failure mode this type exists to avoid — so the address field
/// accepts either shape, and a paste that turns out to carry a token fills both fields at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPaste {
    /// The address part, with any scheme and trailing slash stripped. Never `None` — even a
    /// malformed paste is echoed back as an address, so nothing typed is silently thrown away.
    pub address: String,
    /// The token, when the paste carried a `token=` query parameter.
    pub token: Option<String>,
}

/// Parse whatever was pasted or typed into the address field.
///
/// Accepts a full connection string (`http://host:port?token=abc-_123`), the same without a
/// scheme, or a bare `host` or `host:port` with no query at all — in which case `token` comes back
/// `None` and whatever is already in the token field is left alone. This is deliberately lenient
/// rather than validating: a partial or malformed paste still yields *an* address, because the
/// alternative is a paste that visibly does nothing, which reads as broken.
pub fn parse_connection_string(input: &str) -> ParsedPaste {
    let trimmed = input.trim();
    let without_scheme = strip_scheme(trimmed);
    let (address, query) = match without_scheme.split_once('?') {
        Some((address, query)) => (address, Some(query)),
        None => (without_scheme, None),
    };
    let token = query
        .and_then(|query| {
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix("token="))
        })
        .filter(|token| !token.is_empty())
        .map(|token| token.trim().to_string());
    ParsedPaste {
        address: address.trim().trim_end_matches('/').to_string(),
        token,
    }
}

/// Strip a leading `http://` or `https://`, case-insensitively. The host only ever prints `http`,
/// but a user typing from memory is as likely to type `https` — both are the same thing to this
/// parser, since neither survives past the handshake's own upgrade anyway.
fn strip_scheme(input: &str) -> &str {
    for scheme in ["http://", "https://"] {
        if input.len() >= scheme.len()
            && input.as_bytes()[..scheme.len()].eq_ignore_ascii_case(scheme.as_bytes())
        {
            return &input[scheme.len()..];
        }
    }
    input
}

/// The address as the transport needs it: `host:port`, filling in [`DEFAULT_PORT`] when the typed
/// address carried none.
///
/// A trailing `:1234` is taken as a port already given; anything else — including an address with
/// no colon at all, and a malformed trailing colon with nothing after it — gets the default port
/// appended. This does not attempt to handle a bare IPv6 literal (`::1`) without brackets; nothing
/// upstream of it offers one today, and misreading a colon inside one as a port separator would be
/// the wrong kind of clever for the address this modal actually asks for.
pub fn with_default_port(address: &str) -> String {
    let address = address.trim();
    let has_port = address
        .rsplit_once(':')
        .is_some_and(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));
    if has_port {
        address.to_string()
    } else {
        format!("{address}:{DEFAULT_PORT}")
    }
}
