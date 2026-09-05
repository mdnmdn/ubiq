//! Connections to external services: what a connection is, which providers exist, and what a
//! certificate confirmation carries.
//!
//! A **connection** is an authenticated identity at a provider — a GitHub login, a GitLab account
//! on a company's own install, a Google Workspace grant. Several per provider is the ordinary case,
//! not a feature: nothing in the record is unique per provider, and every consumer takes a
//! connection id rather than a provider name.
//!
//! Nothing here holds material. The token a flow obtains lives in the harness library's
//! `SecretStore` and reaches this crate never; [`crate::messages::Secret`] is the only shape
//! material takes on the bus, and it travels in exactly two variants.
//!
//! The provider set is closed and compiled in. What lives here is the half both crates need — the
//! names, and which flows work where — so the interface can draw a picker without asking. Endpoint
//! paths, embedded client ids and poll intervals are the host's alone.

use serde::{Deserialize, Serialize};

use crate::ids::ConnectionId;

/// The six services Ubiq can hold an identity at.
///
/// Closed, and a seventh is a change to this enum and to the host's table beside it. Forgejo is a
/// Gitea fork with the same API surface and the same `/api/v1` base, so it connects as
/// [`ProviderId::Gitea`] against its own instance rather than earning a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Github,
    Gitlab,
    Gitea,
    AzureDevops,
    Atlassian,
    Google,
}

/// Which flow produced a connection, and which one a picker is offering.
///
/// [`AuthKind::Probe`] is not a way to authenticate: it is an existing connection being checked
/// against the network, which runs as a flow for the same reason the other three do — a handshake
/// must not happen on the thread that carries keystrokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// A pasted personal access token. One HTTP round trip, no registered application, and the
    /// only flow every provider but Google supports — which is why a self-hosted install is usable
    /// without an administrator.
    Token,
    /// The device flow: a code the user types into a browser, polled until they finish.
    Device,
    /// An authorization code with PKCE, returned to a loopback listener.
    Oauth,
    /// Not authentication — an existing connection, checked against its instance.
    Probe,
}

/// Whether a provider needs to be told where it lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceNeed {
    /// Cloud only. Asking would be a field with one correct answer.
    Never,
    /// A public cloud exists, and so do self-managed installs.
    Optional,
    /// Self-hosted only: there is no cloud to fall back to.
    Required,
}

impl ProviderId {
    /// Every provider, in the order a picker offers them.
    pub fn all() -> &'static [ProviderId] {
        &[
            ProviderId::Github,
            ProviderId::Gitlab,
            ProviderId::Gitea,
            ProviderId::AzureDevops,
            ProviderId::Atlassian,
            ProviderId::Google,
        ]
    }

    /// The provider's own name for itself.
    pub fn label(self) -> &'static str {
        match self {
            ProviderId::Github => "GitHub",
            ProviderId::Gitlab => "GitLab",
            ProviderId::Gitea => "Gitea",
            ProviderId::AzureDevops => "Azure DevOps",
            ProviderId::Atlassian => "Atlassian",
            ProviderId::Google => "Google Workspace",
        }
    }

    /// Two letters standing in for a logo. The interface ships no provider artwork, and a mark a
    /// user would recognise is a licensing question rather than a drawing one.
    pub fn glyph(self) -> &'static str {
        match self {
            ProviderId::Github => "GH",
            ProviderId::Gitlab => "GL",
            ProviderId::Gitea => "GT",
            ProviderId::AzureDevops => "AZ",
            ProviderId::Atlassian => "AT",
            ProviderId::Google => "GO",
        }
    }

    /// Whether this provider must be told where it lives.
    pub fn instance_need(self) -> InstanceNeed {
        match self {
            // Gitea and Forgejo ship no hosted service, so an instance is the whole address.
            ProviderId::Gitea => InstanceNeed::Required,
            ProviderId::Google => InstanceNeed::Never,
            _ => InstanceNeed::Optional,
        }
    }

    /// Which flows work against this provider, given whether the user named an instance.
    ///
    /// The split is the point. Azure DevOps *Services* authenticates through Entra ID; Azure DevOps
    /// *Server* has no browser flow at all, so an on-premises connection is simply never offered
    /// one — the interface reads this list rather than special-casing a provider.
    pub fn flows(self, self_hosted: bool) -> &'static [AuthKind] {
        use AuthKind::{Device, Oauth, Token};
        match (self, self_hosted) {
            (ProviderId::Github, _) => &[Token, Device],
            (ProviderId::Gitlab, _) => &[Token, Oauth],
            (ProviderId::Gitea, true) => &[Token, Oauth],
            (ProviderId::AzureDevops, false) => &[Token, Oauth],
            (ProviderId::AzureDevops, true) => &[Token],
            (ProviderId::Atlassian, false) => &[Token, Oauth],
            (ProviderId::Atlassian, true) => &[Token],
            (ProviderId::Google, false) => &[Oauth],
            // Gitea has no cloud, and Google no self-hosted install.
            (ProviderId::Gitea, false) | (ProviderId::Google, true) => &[],
        }
    }

    /// Whether the interface must ask for a client id before opening anything.
    ///
    /// A browser flow needs a registered application, and an application on a self-hosted install
    /// is registered *on that install* by whoever administers it — so there is no built-in id to
    /// fall back to and asking is the normal case, not the exception.
    pub fn needs_client_id(self, kind: AuthKind, self_hosted: bool) -> bool {
        matches!(kind, AuthKind::Oauth | AuthKind::Device) && self_hosted
    }
}

/// The scheme, host and port an instance lives at — the triple a browser scopes anything to, and
/// what a pinned certificate is keyed by.
///
/// The default port for the scheme is dropped, so `https://host` and `https://host:443` are one
/// origin. Returns `None` for anything that is not an absolute `http`/`https` URL, which is what
/// turns a bare host name typed into the instance field into a refusal rather than a guess.
pub fn origin(instance: &str) -> Option<String> {
    let (scheme, rest) = instance.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    // Everything from the first `/`, `?` or `#` is a path, not an authority.
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|a| !a.is_empty())?;
    // Credentials in a URL are not part of its origin.
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = match authority.rsplit_once(':') {
        // A colon inside brackets is an IPv6 address, not a port separator.
        Some((host, port)) if !host.ends_with(']') => (host, port.parse::<u16>().ok()?),
        _ => (authority, default_port),
    };
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    if port == default_port {
        Some(format!("{scheme}://{host}"))
    } else {
        Some(format!("{scheme}://{host}:{port}"))
    }
}

/// One authenticated identity, as it is written down and as the interface draws it.
///
/// Carries no material: an id, what the user called it, where it lives, and what the provider said
/// its own name for the identity was. The log sink listens to the same bus the record travels on,
/// so a token here would be a token in a log a user might paste into an issue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub provider: ProviderId,
    /// The user's name for it — "work", "personal", "client-x". Freely renamable.
    pub label: String,
    /// The base URL this identity lives at, exactly as the user typed it. `None` is the provider's
    /// public cloud. Stored rather than reconstructed: an on-premises install can live under a path
    /// (`example.com/gitlab`, `server/tfs/DefaultCollection`), and rebuilding a URL from a host name
    /// loses it.
    pub instance: Option<String>,
    pub auth: AuthKind,
    /// What was asked for, as returned. Non-secret, and the honest answer to "what can this do".
    #[serde(default)]
    pub scopes: Vec<String>,
    /// The provider's own display name for the identity — a login, an email. Fetched once when the
    /// connection was made and cached; nothing re-fetches it.
    pub account: String,
    /// An application registered on this instance, when the connection was made against one. Public
    /// by construction — a client id travels in the query string of every authorization URL.
    #[serde(default)]
    pub client_id: Option<String>,
}

/// An OAuth application Ubiq authenticates *as*, configured rather than built in.
///
/// Distinct from a connection: a connection credential identifies the user and is theirs; this
/// identifies Ubiq to the provider and is the same for every user of a build. `has_secret` is
/// derived from the secret store rather than stored, so this whole record is safe in a file the
/// user can open and hand to a colleague.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthApp {
    pub provider: ProviderId,
    /// Which instance this application is registered on. `None` is the provider's cloud.
    #[serde(default)]
    pub origin: Option<String>,
    pub client_id: String,
    #[serde(default)]
    pub has_secret: bool,
}

/// A certificate the user has vouched for, at one origin.
///
/// Keyed by origin rather than by connection, because the certificate is a property of the server
/// and the user's answer is about the server: a company's GitLab has one certificate whoever is
/// logging in, and asking the same question twice teaches the user to click through it. The
/// consequence is that a pin outlives the connection that created it — deliberately, since a user
/// who deletes and re-adds a connection should not be asked again about a certificate they already
/// approved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedCert {
    /// Scheme, host and port — see [`origin`].
    pub origin: String,
    /// Lowercase hex of the SHA-256 of the leaf certificate's DER encoding.
    pub sha256: String,
    /// Kept for the list to draw, so revoking a pin does not need a live handshake to describe it.
    pub subject: String,
    pub issuer: String,
    /// Epoch seconds. The interface owns how a date is written; the host does not.
    pub not_after: i64,
}

/// Why a certificate did not validate — what the confirmation dialog needs to say *why* it is
/// asking.
///
// ponytail: four values against rustls' much larger `CertificateError` set; anything not named
// reads as `UnknownIssuer`, the honest default for "this chain did not check out". Widen it only
// when a dialog sentence turns out to be wrong, not because it could be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertReason {
    UnknownIssuer,
    HostnameMismatch,
    Expired,
    NotYetValid,
}

/// A leaf certificate as the user is shown it, so they can check it against what their
/// administrator told them.
///
/// Everything here is public: a certificate is what a server hands to anyone who connects. It is
/// the one payload in this family that exists to be read carefully rather than merely displayed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertInfo {
    pub subject: String,
    /// The subject alternative names, which is what a browser actually matches a host against.
    #[serde(default)]
    pub sans: Vec<String>,
    pub issuer: String,
    /// Epoch seconds.
    pub not_before: i64,
    pub not_after: i64,
    /// Lowercase hex of the SHA-256 of the DER encoding — the exact string a
    /// [`crate::messages::Message::TrustCertificate`] must carry back.
    pub sha256: String,
    pub self_signed: bool,
    pub reason: CertReason,
}

/// How far a connect flow has got. One `ConnectPending` per change, terminated by exactly one
/// `ConnectCaptured` or `ConnectFailed`.
///
/// Typed rather than a string because "which of these is happening" is exactly what the interface
/// draws: a spinner, a code to type, a field to paste into, a question to answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectStage {
    /// Started; nothing to show yet.
    Opening,
    /// The user types `user_code` at `verification_url`. `expires_in` is seconds.
    DeviceCode {
        user_code: String,
        verification_url: String,
        expires_in: u64,
    },
    /// The browser has been sent to `url` and the loopback listener is bound on `port`. The URL
    /// travels so the interface can offer it as a link when opening the browser did not work.
    AwaitingCallback { port: u16, url: String },
    /// Trading a code, or a pasted token, for an identity.
    Exchanging,
    /// Waiting for the user to paste something. `prompt` says what.
    NeedSecret { prompt: String },
    /// Stopped on a certificate. A `ConfirmCertificate` carrying the details is on its way, or has
    /// already arrived.
    AwaitingCertificate,
}

/// Why a flow ended without a connection.
///
/// Typed for the same reason [`ConnectStage`] is: each of these needs a different sentence, and
/// several name something the user can fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectError {
    /// The provider refused, or the user declined in the browser.
    Denied,
    /// The device code or the callback ran out of time.
    Expired,
    /// Port 47821 was in use. Deliberately not retried on another port: another port is a redirect
    /// URI no provider will accept, so failing here is clearer than failing later.
    PortBusy,
    /// A callback arrived whose `state` was not the one sent, or a certificate was confirmed with a
    /// fingerprint other than the one offered. Either way nothing is exchanged and nothing stored.
    StateMismatch,
    /// No secure credential store is available. Refused before any browser opens, because the
    /// application never writes a bearer token to a plaintext file.
    NoSecureStore,
    /// The instance URL is not a URL, or is not the product it claims — a Gitea URL typed into a
    /// GitLab connection fails here rather than at first use.
    BadInstance,
    /// No application is configured for this provider, so there is no browser flow to open.
    NoApplication,
    /// The handshake failed for something other than a certificate: a reset, a protocol mismatch,
    /// no route. Nothing to confirm and nothing to pin.
    Tls(String),
    /// Anything else the provider said.
    Http(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_origin_is_scheme_host_and_a_port_that_is_not_the_default() {
        assert_eq!(
            origin("https://Example.com/gitlab").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            origin("https://example.com:443/gitlab").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            origin("https://server:8443/tfs/DefaultCollection").as_deref(),
            Some("https://server:8443")
        );
        assert_eq!(
            origin("http://localhost:3000").as_deref(),
            Some("http://localhost:3000")
        );
        // Credentials are not part of an origin.
        assert_eq!(
            origin("https://user@example.com/x").as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn only_an_absolute_http_url_has_an_origin() {
        // A bare host name is the mistake this catches: there is no scheme to assume.
        assert_eq!(origin("example.com"), None);
        assert_eq!(origin("ftp://example.com"), None);
        assert_eq!(origin("https://"), None);
        assert_eq!(origin(""), None);
        assert_eq!(origin("https://example.com:not-a-port"), None);
    }

    #[test]
    fn a_self_hosted_instance_is_offered_only_the_flows_that_work() {
        // Azure DevOps Server authenticates with a token and nothing else.
        assert_eq!(ProviderId::AzureDevops.flows(true), &[AuthKind::Token]);
        assert!(
            ProviderId::AzureDevops
                .flows(false)
                .contains(&AuthKind::Oauth)
        );
        // Gitea ships no cloud, Google no self-hosted install.
        assert!(ProviderId::Gitea.flows(false).is_empty());
        assert!(ProviderId::Google.flows(true).is_empty());
        // Google is the one provider with no token flow, which is why it needs the browser one.
        assert_eq!(ProviderId::Google.flows(false), &[AuthKind::Oauth]);
    }

    #[test]
    fn every_provider_is_reachable_somehow() {
        for &provider in ProviderId::all() {
            let cloud = provider.flows(false);
            let hosted = provider.flows(true);
            assert!(
                !cloud.is_empty() || !hosted.is_empty(),
                "{provider:?} offers no flow at all"
            );
            assert!(!provider.label().is_empty());
            assert_eq!(
                provider.glyph().len(),
                2,
                "{provider:?} glyph is two letters"
            );
            // A provider with no cloud must be told where it lives, and one with no self-hosted
            // form must not be asked.
            match provider.instance_need() {
                InstanceNeed::Required => assert!(cloud.is_empty()),
                InstanceNeed::Never => assert!(hosted.is_empty()),
                InstanceNeed::Optional => assert!(!cloud.is_empty() && !hosted.is_empty()),
            }
        }
    }

    #[test]
    fn a_browser_flow_on_a_self_hosted_instance_asks_for_a_client_id() {
        // The application is registered on that install, so there is no built-in id to use.
        assert!(ProviderId::Gitlab.needs_client_id(AuthKind::Oauth, true));
        assert!(!ProviderId::Gitlab.needs_client_id(AuthKind::Oauth, false));
        // A pasted token needs no application at all, wherever it is going.
        assert!(!ProviderId::Gitlab.needs_client_id(AuthKind::Token, true));
    }
}
