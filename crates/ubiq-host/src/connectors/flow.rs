//! One authentication, on a thread of its own.
//!
//! Every flow here talks to a network, and nothing that talks to a network may run on the thread
//! that carries keystrokes — so a flow is a thread, and the coordinator holds nothing but a sender.
//!
//! **The channel is the whole synchronisation story.** One `flume::bounded(1)` per flow: the
//! receiver lives in the thread, the sender in the coordinator's map. Every way a flow can stop —
//! the user cancelling, the window closing, a deadline passing — arrives as a failed receive, so
//! there is no flag to check, no flag to forget to check, and no window between checking one and
//! writing a token. The three helpers ([`ask`], [`nap`], [`wanted`]) are the only places that
//! touch it.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::connectors::{
    AuthKind, CertInfo, ConnectError, ConnectStage, Connection, ProviderId, TrustedCert,
};
use ubiq_proto::ids::{ConnectId, ConnectionId};
use ubiq_proto::messages::{Message, Secret};
use ubiq_proto::settings::{HostSettings, SettingsLayer};

use crate::settings::Settings;

use super::http::{self, Failure};
use super::store::{Store, Token};
use super::{app, providers};

/// The loopback port an authorization code comes back on.
///
/// One port, never another: a different port is a different redirect URI, and no provider accepts
/// one it was not registered with. Failing on a busy port is clearer than succeeding into a
/// redirect the provider will refuse.
pub const PORT: u16 = 47821;

/// The redirect URI registered against every application this build talks to.
pub const REDIRECT: &str = "http://127.0.0.1:47821/callback";

/// How long a flow waits for a person. Longer than a person takes, shorter than forever.
const PATIENCE: Duration = Duration::from_secs(600);

/// What the coordinator can tell a waiting flow.
#[derive(Debug)]
pub enum Answer {
    /// A pasted token, or an application's client secret.
    Secret(Secret),
    /// The user vouched for a certificate. Carries the fingerprint back so the flow can check it is
    /// the one it offered.
    Certificate { origin: String, sha256: String },
    /// Stop. Dropping the sender says the same thing, and is what a closed window does.
    Cancel,
}

/// One flow, addressed and equipped. Everything it needs, so the thread borrows nothing.
pub struct Job {
    pub connect_id: ConnectId,
    pub provider: ProviderId,
    pub instance: Option<String>,
    pub label: String,
    pub auth: AuthKind,
    /// The client id the interface was told to ask for, when it asked.
    pub client_id: Option<String>,
    /// Set only for a probe: the connection being checked.
    pub connection: Option<ConnectionId>,
    pub answers: flume::Receiver<Answer>,
    pub settings: Arc<Settings>,
    pub store: Arc<Store>,
    /// The window that asked. Stages and the outcome go here.
    pub asker: Mailbox,
    /// Every window. A record that changed is everyone's news.
    pub everyone: Mailbox,
}

/// Start a flow. It ends by itself, and the coordinator learns so when its sender disconnects.
pub fn spawn(job: Job) {
    let name = format!("ubiq-connect-{}", job.connect_id);
    if let Err(error) = thread::Builder::new().name(name).spawn(move || run(job)) {
        tracing::error!("a connect flow did not start: {error}");
    }
}

fn run(job: Job) {
    let outcome = match job.auth {
        AuthKind::Token => token(&job),
        AuthKind::Device => device(&job),
        AuthKind::Oauth => pkce(&job),
        AuthKind::Probe => probe(&job),
    };
    match outcome {
        Ok(()) => {}
        // Nobody is waiting to be told a flow they abandoned has stopped.
        Err(Failure::Cancelled) => tracing::debug!("connect flow {} was abandoned", job.connect_id),
        Err(Failure::Certificate(cert)) => {
            // A certificate that reached here was refused a second time, after the user vouched
            // for it — the honest reading is that the server is not offering what they approved.
            fail(
                &job,
                ConnectError::Tls(format!("certificate {}", cert.sha256)),
            );
        }
        Err(Failure::Connect(error)) => fail(&job, error),
    }
}

// ── the three helpers ────────────────────────────────────────────────

/// Say how far the flow has got, then wait for an answer.
///
/// `None` is every way of not getting one: the user cancelled, the window went, or the deadline
/// passed. A caller that turns `None` into anything but "stop" is a bug.
pub fn ask(job: &Job, stage: ConnectStage, deadline: Instant) -> Option<Answer> {
    pending(job, stage);
    match job.answers.recv_deadline(deadline) {
        Ok(Answer::Cancel) | Err(_) => None,
        Ok(answer) => Some(answer),
    }
}

/// Wait out a poll interval, and answer whether the flow should still be polling.
///
/// The device flow's sleep and its cancel check are one call, because a flow that sleeps first and
/// checks afterwards keeps a cancelled poll alive for one more interval.
pub fn nap(job: &Job, interval: Duration) -> bool {
    matches!(
        job.answers.recv_timeout(interval),
        Err(flume::RecvTimeoutError::Timeout)
    )
}

/// Whether anybody is still waiting for this flow.
///
/// Checked immediately before a token is written, and that is the point: a token that arrives after
/// a cancel is discarded rather than stored, so an abandoned flow really does leave nothing behind.
pub fn wanted(job: &Job) -> bool {
    !job.answers.is_disconnected()
}

// ── certificates ─────────────────────────────────────────────────────

/// Run a request, and on a certificate stop, ask once and retry once.
///
/// The retry is deliberately singular: a server that refuses the certificate the user just vouched
/// for is not going to be talked round by a third attempt.
pub fn with_trust<T>(
    job: &Job,
    origin: &str,
    call: impl Fn(Option<&str>) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let refused = match call(pin(job, origin).as_deref()) {
        Err(Failure::Certificate(cert)) => cert,
        other => return other,
    };
    confirm(job, origin, &refused)?;
    call(Some(&refused.sha256))
}

/// Offer a certificate, wait for the answer, and pin it.
fn confirm(job: &Job, origin: &str, cert: &CertInfo) -> Result<(), Failure> {
    job.asker.send(Message::ConfirmCertificate {
        connect_id: job.connect_id,
        origin: origin.to_string(),
        cert: cert.clone(),
    });
    let Some(answer) = ask(job, ConnectStage::AwaitingCertificate, deadline()) else {
        return Err(Failure::Cancelled);
    };
    let Answer::Certificate {
        origin: vouched,
        sha256,
    } = answer
    else {
        return Err(Failure::Cancelled);
    };
    // The confirmation is meaningful only if it is about the certificate that was offered.
    // Anything else and the flow stays stopped with nothing pinned.
    if vouched != origin || sha256 != cert.sha256 {
        return Err(ConnectError::StateMismatch.into());
    }
    // Written *before* the retry, not after success: the user's answer was about the server, so the
    // pin outlives a flow they then abandon, and a second connection to the same server is not
    // asked again.
    let record = TrustedCert {
        origin: origin.to_string(),
        sha256,
        subject: cert.subject.clone(),
        issuer: cert.issuer.clone(),
        not_after: cert.not_after,
    };
    let settings = job
        .settings
        .update_host(|host| {
            host.trusted_certs
                .retain(|held| held.origin != record.origin);
            host.trusted_certs.push(record);
        })
        .map_err(|error| Failure::Connect(ConnectError::Http(error)))?;
    announce(job, &settings);
    Ok(())
}

/// The fingerprint already vouched for at this origin, if any.
fn pin(job: &Job, origin: &str) -> Option<String> {
    job.settings
        .host()
        .trusted_certs
        .into_iter()
        .find(|held| held.origin == origin)
        .map(|held| held.sha256)
}

// ── the four flows ───────────────────────────────────────────────────

/// A pasted personal access token: ask, check it against the instance, write it down.
fn token(job: &Job) -> Result<(), Failure> {
    let prompt = providers::of(job.provider).secret_prompt.to_string();
    let Some(Answer::Secret(secret)) = ask(job, ConnectStage::NeedSecret { prompt }, deadline())
    else {
        return Err(Failure::Cancelled);
    };
    pending(job, ConnectStage::Exchanging);
    let account = whoami(job, secret.expose())?;
    capture(
        job,
        Token {
            access_token: secret.expose().to_string(),
            ..Token::default()
        },
        account,
        Vec::new(),
    )
}

/// The device flow: a code the user types into a browser, then a poll.
fn device(job: &Job) -> Result<(), Failure> {
    let row = providers::of(job.provider);
    if row.device.is_empty() {
        return Err(ConnectError::NoApplication.into());
    }
    let client_id = application(job)?;
    let origin = providers::instance_origin(job.provider, job.instance.as_deref())?;
    let start = providers::web_url(job.provider, job.instance.as_deref(), row.device)?;
    let opened = with_trust(job, &origin, |pin| {
        http::post_form(
            pin,
            &start,
            &[("client_id", client_id.as_str()), ("scope", row.scopes)],
        )
    })?;
    let device_code = string(&opened, "device_code").ok_or(ConnectError::Denied)?;
    let user_code = string(&opened, "user_code").ok_or(ConnectError::Denied)?;
    // Providers disagree on the spelling; the specification says `verification_uri`.
    let verification_url = string(&opened, "verification_uri")
        .or_else(|| string(&opened, "verification_url"))
        .ok_or(ConnectError::Denied)?;
    let expires_in = opened
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(900);
    // Five seconds is the floor the specification sets; a provider asking for less is not obeyed.
    let mut interval = Duration::from_secs(
        opened
            .get("interval")
            .and_then(Value::as_u64)
            .map_or(5, |asked| asked.max(5)),
    );

    pending(
        job,
        ConnectStage::DeviceCode {
            user_code,
            verification_url,
            expires_in,
        },
    );

    let exchange = providers::web_url(job.provider, job.instance.as_deref(), row.token)?;
    let until = Instant::now() + Duration::from_secs(expires_in);
    while nap(job, interval) {
        if Instant::now() >= until {
            return Err(ConnectError::Expired.into());
        }
        let answer = with_trust(job, &origin, |pin| {
            http::post_form(
                pin,
                &exchange,
                &[
                    ("client_id", client_id.as_str()),
                    ("device_code", device_code.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ],
            )
        })?;
        match string(&answer, "error").as_deref() {
            None => {
                let token = token_from(&answer).ok_or(ConnectError::Denied)?;
                let account = whoami(job, &token.access_token)?;
                let scopes = scopes(&token);
                return capture(job, token, account, scopes);
            }
            Some("authorization_pending") => {}
            // The provider is telling us the interval it wants; taking it is what keeps a flow
            // from being throttled off entirely.
            Some("slow_down") => interval += Duration::from_secs(5),
            Some("access_denied") => return Err(ConnectError::Denied.into()),
            Some("expired_token") => return Err(ConnectError::Expired.into()),
            Some(other) => return Err(ConnectError::Http(other.to_string()).into()),
        }
    }
    Err(Failure::Cancelled)
}

/// An authorization code with PKCE, returned to a loopback listener.
fn pkce(job: &Job) -> Result<(), Failure> {
    let row = providers::of(job.provider);
    let client_id = application(job)?;
    let origin = providers::instance_origin(job.provider, job.instance.as_deref())?;

    let verifier = nonce(32);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = nonce(16);

    // Bound before a URL is offered, so a busy port is a refusal rather than a browser sent
    // somewhere nothing is listening.
    let listener = tiny_http::Server::http(("127.0.0.1", PORT))
        .map_err(|_| Failure::Connect(ConnectError::PortBusy))?;

    let url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        providers::web_url(job.provider, job.instance.as_deref(), row.authorize)?,
        encode(&client_id),
        encode(REDIRECT),
        encode(row.scopes),
        encode(&state),
        encode(&challenge),
    );
    // The host does not open a browser. The URL travels, and the interface opens it — which is also
    // what lets the interface offer it as a link when opening one did not work.
    pending(job, ConnectStage::AwaitingCallback { port: PORT, url });

    let returned = wait_for_callback(job, &listener)?;
    // Checked before the code is exchanged, not after: a code from a callback nobody asked for is
    // never traded for anything.
    if returned.get("state").map(String::as_str) != Some(state.as_str()) {
        return Err(ConnectError::StateMismatch.into());
    }
    if returned.contains_key("error") {
        return Err(ConnectError::Denied.into());
    }
    let code = returned.get("code").ok_or(ConnectError::Denied)?.clone();

    pending(job, ConnectStage::Exchanging);
    let exchange = providers::web_url(job.provider, job.instance.as_deref(), row.token)?;
    let configured = job.instance.as_deref().and_then(as_origin);
    let secret = job
        .store
        .app_secret(job.provider, configured.as_deref())
        .unwrap_or_default();
    let answer = with_trust(job, &origin, |pin| {
        let mut fields = vec![
            ("grant_type", "authorization_code"),
            ("client_id", client_id.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier.as_str()),
        ];
        // A public client has none, and sending an empty one is not the same as sending nothing.
        if !secret.is_empty() {
            fields.push(("client_secret", secret.as_str()));
        }
        http::post_form(pin, &exchange, &fields)
    })?;
    if let Some(error) = string(&answer, "error") {
        return Err(ConnectError::Http(error).into());
    }
    let token = token_from(&answer).ok_or(ConnectError::Denied)?;
    let account = whoami(job, &token.access_token)?;
    let scopes = scopes(&token);
    capture(job, token, account, scopes)
}

/// An existing connection, checked against its instance.
///
/// Not authentication — but a flow all the same, because it is a handshake, and this is the one
/// place an existing connection's certificate can be confirmed or replaced.
fn probe(job: &Job) -> Result<(), Failure> {
    let id = job.connection.ok_or(ConnectError::BadInstance)?;
    let stored = job
        .store
        .token(job.provider, id)
        .ok_or(ConnectError::NoSecureStore)?;
    pending(job, ConnectStage::Exchanging);
    whoami(job, &stored.access_token)?;
    let status = job.store.status(job.provider, id, now_ms());
    job.asker.send(Message::ConnectionStatus {
        connection: id,
        status,
    });
    let host = job.settings.host();
    let info = super::infos(&host, Some(&job.store), now_ms())
        .into_iter()
        .find(|info| info.connection.id == id)
        .ok_or(ConnectError::BadInstance)?;
    job.asker.send(Message::ConnectCaptured {
        connect_id: job.connect_id,
        connection: info,
    });
    Ok(())
}

// ── the pieces the flows share ───────────────────────────────────────

/// Who the provider says this token belongs to.
///
/// A body with none of the provider's account keys is [`ConnectError::BadInstance`]: it answered,
/// but it is not this product — a Gitea URL typed into a GitLab connection fails here.
fn whoami(job: &Job, token: &str) -> Result<String, Failure> {
    let row = providers::of(job.provider);
    let url = providers::api_url(job.provider, job.instance.as_deref(), row.whoami)?;
    let origin = providers::instance_origin(job.provider, job.instance.as_deref())?;
    let body = with_trust(job, &origin, |pin| http::get_json(pin, &url, Some(token)))?;
    providers::account_name(job.provider, &body).ok_or(ConnectError::BadInstance.into())
}

/// Store the token and write the connection down. The last thing every flow but a probe does.
fn capture(job: &Job, token: Token, account: String, scopes: Vec<String>) -> Result<(), Failure> {
    // The one check that matters: a token that arrived after the user cancelled is discarded, never
    // stored. Everything before this point is reversible by doing nothing.
    if !wanted(job) {
        return Err(Failure::Cancelled);
    }
    let id = ConnectionId::generate();
    job.store
        .put(job.provider, id, &token)
        .map_err(|error| Failure::Connect(ConnectError::Http(error)))?;
    let connection = Connection {
        id,
        provider: job.provider,
        label: job.label.clone(),
        instance: job.instance.clone(),
        auth: job.auth,
        scopes,
        account,
        client_id: job.client_id.clone(),
    };
    let settings = job
        .settings
        .update_host(|host| host.connections.push(connection.clone()))
        .map_err(|error| Failure::Connect(ConnectError::Http(error)))?;
    let infos = super::infos(&settings, Some(&job.store), now_ms());
    let captured = infos
        .iter()
        .find(|info| info.connection.id == id)
        .cloned()
        .ok_or(ConnectError::BadInstance)?;
    job.asker.send(Message::ConnectCaptured {
        connect_id: job.connect_id,
        connection: captured,
    });
    announce(job, &settings);
    Ok(())
}

/// The application this flow authenticates as, or the refusal that says there is none.
fn application(job: &Job) -> Result<String, Failure> {
    let origin = job.instance.as_deref().and_then(as_origin);
    app::client_id(
        &job.settings.host(),
        job.provider,
        origin.as_deref(),
        job.client_id.as_deref(),
    )
    .ok_or(ConnectError::NoApplication.into())
}

/// Wait for the one request this listener exists for.
///
/// `recv_timeout` is the deadline and the cancel check in one: every tick asks whether anybody is
/// still waiting, and the listener is dropped the moment this returns.
fn wait_for_callback(
    job: &Job,
    listener: &tiny_http::Server,
) -> Result<HashMap<String, String>, Failure> {
    let until = Instant::now() + PATIENCE;
    loop {
        if !wanted(job) {
            return Err(Failure::Cancelled);
        }
        if Instant::now() >= until {
            return Err(ConnectError::Expired.into());
        }
        match listener.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(request)) => {
                let returned = query(request.url());
                let _ = request.respond(
                    tiny_http::Response::from_string(DONE).with_header(
                        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..])
                            .expect("a constant header"),
                    ),
                );
                return Ok(returned);
            }
            Ok(None) => {}
            Err(error) => return Err(ConnectError::Tls(error.to_string()).into()),
        }
    }
}

/// What the browser is left showing. Deliberately tiny: it is a full stop, not a page.
const DONE: &str =
    "<!doctype html><meta charset=utf-8><title>Signed in</title><p>You can close this tab.";

// ── saying things ────────────────────────────────────────────────────

fn pending(job: &Job, stage: ConnectStage) {
    job.asker.send(Message::ConnectPending {
        connect_id: job.connect_id,
        stage,
    });
}

fn fail(job: &Job, error: ConnectError) {
    job.asker.send(Message::ConnectFailed {
        connect_id: job.connect_id,
        error,
    });
}

/// A record changed, so every window hears both halves: the settings blob it lives in, then the
/// list drawn from it.
fn announce(job: &Job, settings: &HostSettings) {
    let value = serde_json::to_string(settings).ok();
    job.everyone.send(Message::Settings {
        layer: SettingsLayer::Host,
        value,
    });
    job.everyone.send(Message::Connections {
        connections: super::infos(settings, Some(&job.store), now_ms()),
    });
}

// ── small things ─────────────────────────────────────────────────────

fn deadline() -> Instant {
    Instant::now() + PATIENCE
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}

fn string(body: &Value, key: &str) -> Option<String> {
    body.get(key)?.as_str().map(str::to_string)
}

fn as_origin(instance: &str) -> Option<String> {
    ubiq_proto::connectors::origin(instance)
}

/// A token response, in whichever of the two shapes the provider answered.
fn token_from(body: &Value) -> Option<Token> {
    let access_token = string(body, "access_token")?;
    Some(Token {
        access_token,
        refresh_token: string(body, "refresh_token"),
        // Providers give a lifetime, not a moment. The moment is what survives a restart.
        expires_at: body
            .get("expires_in")
            .and_then(Value::as_i64)
            .map(|seconds| now_ms() / 1000 + seconds),
        token_type: string(body, "token_type"),
        scope: string(body, "scope"),
    })
}

/// What was actually granted, as the provider said it — not what was asked for.
fn scopes(token: &Token) -> Vec<String> {
    token
        .scope
        .as_deref()
        .unwrap_or_default()
        .split([' ', ','])
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect()
}

/// `count` random bytes, base64url with no padding — a PKCE verifier and a `state` nonce are the
/// same shape and the same requirement.
fn nonce(count: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; count];
    rand::thread_rng().fill_bytes(&mut bytes);
    b64url(&bytes)
}

/// Base64url without padding, as RFC 7636 wants it. Six lines against a dependency.
fn b64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let packed = u32::from_be_bytes([0, block[0], block[1], block[2]]);
        for index in 0..chunk.len() + 1 {
            let shift = 18 - index * 6;
            out.push(ALPHABET[((packed >> shift) & 0x3f) as usize] as char);
        }
    }
    out
}

/// Percent-encoding for a query value. The unreserved set of RFC 3986, and everything else escaped.
///
/// Shared with [`crate::repos::list`], whose provider searches put user text in a query string for
/// the same reason and would otherwise grow a second copy of this.
pub(crate) fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// The query of the one request the loopback listener answers.
fn query(url: &str) -> HashMap<String, String> {
    url.split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (decode(key), decode(value)))
        .collect()
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&value[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
