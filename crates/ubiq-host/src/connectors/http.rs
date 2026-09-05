//! The requests a flow makes, and what a failure means.
//!
//! An agent is built **per request** and never pooled. A pooled agent outlives the pin it was
//! built with, and the whole point of a confirmation is that the next call takes effect: a user who
//! vouches for a certificate and is told "still refused" because a connection was kept alive has
//! been lied to. Building one is cheap next to a TLS handshake.
//!
//! [`Failure`] is host-private and wider than [`ConnectError`] on purpose. A refused certificate is
//! not something the interface should be told as a `ConnectFailed` — it is a question, and the flow
//! asks it. A cancellation is not a failure at all: nobody is left to hear about it.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use ubiq_proto::connectors::{CertInfo, ConnectError};

use super::tls::{PinVerifier, Seen, lock};

/// Everything a request can end as, including the two things the bus has no word for.
#[derive(Debug)]
pub enum Failure {
    /// Something the interface is told, as a `ConnectFailed`.
    Connect(ConnectError),
    /// The server offered a certificate nothing trusts. A question, not a failure.
    Certificate(CertInfo),
    /// The user abandoned the flow, or the window went. Nothing is said, because there is nothing
    /// left to say it to.
    Cancelled,
}

impl From<ConnectError> for Failure {
    fn from(error: ConnectError) -> Self {
        Failure::Connect(error)
    }
}

/// One request's client, with one pin and one slot to leave a refused leaf in.
///
/// Every request a flow makes goes through here, so there is exactly one place that decides what
/// verifies a certificate.
pub fn agent(pin: Option<&str>, seen: &Seen) -> ureq::Agent {
    let config = rustls::ClientConfig::builder_with_provider(super::tls::provider())
        .with_safe_default_protocol_versions()
        // The provider names both TLS 1.2 and 1.3; a version set it cannot serve would be a build
        // error, not a runtime one.
        .expect("the tls versions ring supports")
        .dangerous()
        .with_custom_certificate_verifier(PinVerifier::new(pin.map(str::to_string), seen.clone()))
        .with_no_client_auth();
    ureq::AgentBuilder::new()
        // A flow is a person waiting at a dialog. A request that has not answered in half a minute
        // is not going to.
        .timeout(Duration::from_secs(30))
        .tls_config(Arc::new(config))
        .build()
}

/// A GET that answers JSON, with an optional bearer token.
pub fn get_json(pin: Option<&str>, url: &str, token: Option<&str>) -> Result<Value, Failure> {
    let seen = super::tls::seen();
    let mut request = agent(pin, &seen)
        .get(url)
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT);
    if let Some(token) = token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    read(request.call(), &seen)
}

/// A POST of form fields that answers JSON — every OAuth endpoint in this family.
pub fn post_form(pin: Option<&str>, url: &str, fields: &[(&str, &str)]) -> Result<Value, Failure> {
    let seen = super::tls::seen();
    let response = agent(pin, &seen)
        .post(url)
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT)
        .send_form(fields);
    match response {
        // An OAuth endpoint says why it refused in a JSON body, and providers disagree about
        // whether that body comes with a 200 or a 400 — GitHub's device flow reports
        // `authorization_pending` as a success. So a body that parses is an answer whatever the
        // status line said, and only one that does not is a failure.
        Err(ureq::Error::Status(code, response)) if lock(&seen).is_none() => {
            let body = response.into_string().unwrap_or_default();
            serde_json::from_str(&body).map_err(|_| {
                Failure::Connect(ConnectError::Http(match body.trim() {
                    "" => format!("HTTP {code}"),
                    body => format!(
                        "HTTP {code}: {}",
                        body.chars().take(300).collect::<String>()
                    ),
                }))
            })
        }
        other => read(other, &seen),
    }
}

/// The name the provider sees. GitHub refuses a request without one.
const USER_AGENT: &str = concat!("ubiq/", env!("CARGO_PKG_VERSION"));

fn read(response: Result<ureq::Response, ureq::Error>, seen: &Seen) -> Result<Value, Failure> {
    match response {
        Ok(response) => response
            .into_json()
            .map_err(|error| Failure::Connect(ConnectError::Http(error.to_string()))),
        Err(error) => Err(map_error(error, seen)),
    }
}

/// What a failed request means.
///
/// A captured leaf **outranks** whatever ureq called the failure. ureq sees a closed connection and
/// says so; the verifier saw the certificate and knows why it closed. The one the user can act on
/// wins.
pub fn map_error(error: ureq::Error, seen: &Seen) -> Failure {
    if let Some(cert) = lock(seen).take() {
        return Failure::Certificate(cert);
    }
    match error {
        // A provider that says no in a body says it better than a status line does.
        ureq::Error::Status(code, response) => {
            let detail = response.into_string().unwrap_or_default();
            Failure::Connect(ConnectError::Http(match detail.trim() {
                "" => format!("HTTP {code}"),
                body => format!(
                    "HTTP {code}: {}",
                    body.chars().take(300).collect::<String>()
                ),
            }))
        }
        ureq::Error::Transport(transport) => match transport.kind() {
            // Nothing to confirm and nothing to pin: the handshake never got as far as a
            // certificate, or never got as far as a server.
            ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Dns | ureq::ErrorKind::Io => {
                Failure::Connect(ConnectError::Tls(transport.to_string()))
            }
            _ => Failure::Connect(ConnectError::Http(transport.to_string())),
        },
    }
}
