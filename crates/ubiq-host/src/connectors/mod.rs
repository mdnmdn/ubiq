//! Connections to external services: the records, the secret store, and the flows that fill them.
//!
//! Everything answerable from a file is answered here, on the coordinator's thread — a list, a
//! rename, a delete, a stored expiry. Everything that touches a network is a [`flow`], on a thread
//! of its own, because the coordinator's thread carries keystrokes and must never wait on a
//! handshake. That is why a *probe* is a flow too: checking a connection is a network call like any
//! other, and giving it a `ConnectId` means it reports its stages, can be cancelled, and can stop on
//! a certificate exactly the way a fresh connection does.
//!
//! The records live in the host settings blob and move only through
//! [`crate::settings::Settings::update_host`], which is what lets a flow thread write one while a
//! settings dialog is open in a window.
//!
//! - `providers`: the endpoint table, one `const` row per provider
//! - `tls`: certificate verification, the machine's rules first and one pin second
//! - `http`: the requests a flow makes, one agent per request
//! - `flow`: one authentication on a thread, and the channel that is its whole cancel story
//! - `app`: which OAuth application a flow authenticates as
//! - `store`: where a token lives, and what it says about itself

pub mod app;
pub mod flow;
pub mod http;
pub mod providers;
pub mod store;
pub mod tls;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ubiq_proto::bus::{ClientId, Mailbox};
use ubiq_proto::connectors::{AuthKind, ConnectError, OauthApp, ProviderId, origin as as_origin};
use ubiq_proto::ids::{ConnectId, ConnectionId, OauthAppId};
use ubiq_proto::messages::{ConnectionInfo, Message, Secret};
use ubiq_proto::settings::{HostSettings, SettingsLayer};

use crate::reply::Reply;
use crate::settings::Settings;

pub use flow::Answer;
use store::Store;

/// The connector family, as the coordinator holds it.
pub struct Connectors {
    settings: Arc<Settings>,
    /// The platform's secret store. Constructing it does no I/O; whether it *works* is
    /// [`Store::usable`], asked before any flow opens anything.
    store: Arc<Store>,
    flows: HashMap<ConnectId, Flow>,
}

/// One running flow, from the coordinator's side: who is watching it, and the one way to talk to it.
///
/// Dropping the sender is how a flow is cancelled — the thread's receive fails and it stops.
struct Flow {
    client: ClientId,
    answers: flume::Sender<Answer>,
}

impl Connectors {
    pub fn new(settings: Arc<Settings>, root: &Path) -> Self {
        Self {
            settings,
            store: Arc::new(Store::open(root)),
            flows: HashMap::new(),
        }
    }

    /// Every connection, with what its stored token claims about itself.
    pub fn list(&self) -> Vec<Reply> {
        vec![Reply::Asker(Message::Connections {
            connections: self.connections(),
            bundled: providers::bundled(),
        })]
    }

    /// The user's name for a connection. Nothing else references it, so nothing else changes.
    pub fn rename(&self, connection: ConnectionId, label: String) -> Vec<Reply> {
        self.change(|host| {
            match host
                .connections
                .iter_mut()
                .find(|held| held.id == connection)
            {
                Some(held) => {
                    held.label = label;
                    Ok(())
                }
                None => Err("no such connection".to_string()),
            }
        })
    }

    /// A connection and its token, together. The pinned certificate is left alone: it belongs to
    /// the instance, and may be why another connection still works.
    pub fn delete(&self, connection: ConnectionId) -> Vec<Reply> {
        let Some(record) = self
            .settings
            .host()
            .connections
            .into_iter()
            .find(|held| held.id == connection)
        else {
            return vec![error("no such connection")];
        };
        if let Err(reason) = self.store.delete(record.provider, connection) {
            // The record goes anyway: a connection whose token could not be removed is still a
            // connection the user asked to be rid of, and leaving the row would strand it.
            tracing::warn!("a connection's token was not deleted: {reason}");
        }
        self.change(|host| {
            host.connections.retain(|held| held.id != connection);
            Ok(())
        })
    }

    /// What a connection's token says about itself.
    ///
    /// `probe: false` reads the stored expiry and calls nobody. `probe: true` is a flow, for the
    /// same reason every other network call here is.
    pub fn check(
        &mut self,
        client: ClientId,
        connection: ConnectionId,
        probe: bool,
        asker: Mailbox,
        everyone: Mailbox,
    ) -> Vec<Reply> {
        let host = self.settings.host();
        let Some(record) = host.connections.iter().find(|held| held.id == connection) else {
            return vec![error("no such connection")];
        };
        if !probe {
            return vec![Reply::Asker(Message::ConnectionStatus {
                connection,
                status: self
                    .store
                    .status(record.provider, connection, flow::now_ms()),
            })];
        }
        let provider = record.provider;
        let instance = record.instance.clone();
        let label = record.label.clone();
        let client_id = record.client_id.clone();
        // The registration the connection was made under, so a probe authenticates as the same
        // application the user first consented to.
        let oauth_app = record.oauth_app;
        self.start(
            client,
            ConnectId::generate(),
            provider,
            instance,
            label,
            AuthKind::Probe,
            client_id,
            oauth_app,
            Some(connection),
            asker,
            everyone,
        )
    }

    /// Drop a pin. The next request to that origin validates normally.
    pub fn forget_cert(&self, origin: String) -> Vec<Reply> {
        self.change(|host| {
            host.trusted_certs.retain(|held| held.origin != origin);
            Ok(())
        })
    }

    /// Create or rewrite one named application registration.
    ///
    /// The id is minted here and not in the interface, for [`ConnectionId`]'s reason: a
    /// registration exists once it is written, and a form the user abandoned leaves nothing behind.
    /// A rewrite keeps the id, so the secret filed under it and the connections that name it are
    /// undisturbed by a rename or a change of instance.
    pub fn save_app(
        &self,
        id: Option<OauthAppId>,
        provider: ProviderId,
        name: String,
        origin: Option<String>,
        client_id: String,
    ) -> Vec<Reply> {
        let name = name.trim().to_string();
        let client_id = client_id.trim().to_string();
        if name.is_empty() {
            return vec![error("a registration needs a name")];
        }
        if client_id.is_empty() {
            return vec![error("a registration needs a client id")];
        }
        // Normalised here as well as in the interface, because this is the boundary: what a
        // registration stores has to be the same string a certificate pin and a settings match are
        // keyed by, and an unparseable URL is a refusal rather than a row nothing will ever match.
        let origin = match origin.as_deref().map(str::trim).filter(|at| !at.is_empty()) {
            None => None,
            Some(at) => match as_origin(at) {
                Some(origin) => Some(origin),
                None => return vec![error("that instance is not an absolute http or https URL")],
            },
        };
        let has_secret = id.is_some_and(|id| self.store.app_secret(id).is_some());
        self.change(move |host| {
            let held = id.and_then(|id| host.oauth_apps.iter_mut().find(|app| app.id == id));
            match held {
                Some(app) => {
                    app.provider = provider;
                    app.name = name;
                    app.origin = origin;
                    app.client_id = client_id;
                    app.has_secret = has_secret;
                    Ok(())
                }
                None if id.is_some() => Err("no such registration".to_string()),
                None => {
                    host.oauth_apps.push(OauthApp {
                        id: OauthAppId::generate(),
                        provider,
                        name,
                        origin,
                        client_id,
                        has_secret: false,
                    });
                    Ok(())
                }
            }
        })
    }

    /// Delete a registration and the client secret filed under it.
    ///
    /// Connections made under it are left alone: each holds its own client id, and the id it names
    /// simply matches nothing after this.
    pub fn delete_app(&self, id: OauthAppId) -> Vec<Reply> {
        if let Err(reason) = self.store.clear_app_secret(id) {
            // The row goes anyway, on [`Self::delete`]'s reasoning: leaving it would strand a
            // registration the user asked to be rid of.
            tracing::warn!("a registration's client secret was not deleted: {reason}");
        }
        self.change(|host| {
            let before = host.oauth_apps.len();
            host.oauth_apps.retain(|app| app.id != id);
            if host.oauth_apps.len() == before {
                return Err("no such registration".to_string());
            }
            Ok(())
        })
    }

    /// Store a registration's client secret, and record that there is one.
    ///
    /// The `has_secret` flag lives on the [`OauthApp`] row rather than being asked of the keychain
    /// on every read, so the two are written together or not at all.
    pub fn set_app_secret(&self, app: OauthAppId, secret: Secret) -> Vec<Reply> {
        if let Err(reason) = self.store.set_app_secret(app, secret.expose()) {
            return vec![error(&reason)];
        }
        self.change(|host| app_row(host, app).map(|row| row.has_secret = true))
    }

    /// Forget a stored client secret. Its own operation rather than an empty one, because clearing
    /// a credential should not look like setting one.
    pub fn clear_app_secret(&self, app: OauthAppId) -> Vec<Reply> {
        if let Err(reason) = self.store.clear_app_secret(app) {
            return vec![error(&reason)];
        }
        self.change(|host| app_row(host, app).map(|row| row.has_secret = false))
    }

    /// Start authenticating. Mints a flow, not a connection — an abandoned flow leaves nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        client: ClientId,
        connect_id: ConnectId,
        provider: ProviderId,
        instance: Option<String>,
        label: String,
        auth: AuthKind,
        client_id: Option<String>,
        oauth_app: Option<OauthAppId>,
        asker: Mailbox,
        everyone: Mailbox,
    ) -> Vec<Reply> {
        self.start(
            client, connect_id, provider, instance, label, auth, client_id, oauth_app, None, asker,
            everyone,
        )
    }

    /// Abandon a flow. Dropping the sender is the whole operation: the thread's next receive fails.
    pub fn cancel(&mut self, connect_id: ConnectId) {
        self.flows.remove(&connect_id);
    }

    /// Hand a waiting flow what it asked for.
    pub fn answer(&mut self, connect_id: ConnectId, answer: Answer) -> Vec<Reply> {
        match self.flows.get(&connect_id) {
            // The channel holds one; a flow that is not waiting is a flow that already moved on.
            Some(flow) => match flow.answers.try_send(answer) {
                Ok(()) => Vec::new(),
                Err(_) => vec![error("that flow is no longer waiting")],
            },
            None => vec![error("no such connect flow")],
        }
    }

    /// A window went. Every flow it was watching goes with it.
    pub fn client_gone(&mut self, client: ClientId) {
        self.flows.retain(|_, flow| flow.client != client);
    }

    /// The secret store, for the one other family that needs a connection's token.
    ///
    /// Lent rather than opened a second time: two stores over one directory would be two answers
    /// to [`Store::usable`], and the whole point of that probe is that there is one.
    pub fn store(&self) -> Arc<Store> {
        self.store.clone()
    }

    /// Every connection as the interface is told about it.
    pub fn connections(&self) -> Vec<ConnectionInfo> {
        infos(&self.settings.host(), Some(&self.store), flow::now_ms())
    }

    /// The one path that starts a thread, shared by [`Self::begin`] and a probe.
    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        client: ClientId,
        connect_id: ConnectId,
        provider: ProviderId,
        instance: Option<String>,
        label: String,
        auth: AuthKind,
        client_id: Option<String>,
        oauth_app: Option<OauthAppId>,
        connection: Option<ConnectionId>,
        asker: Mailbox,
        everyone: Mailbox,
    ) -> Vec<Reply> {
        // Reaped where they are minted, so a finished flow's row never outlives the next start.
        self.flows.retain(|_, flow| !flow.answers.is_disconnected());
        // Asked before anything opens: the application never writes a bearer token to a plaintext
        // file, so a machine with no usable secret store is refused here rather than after a
        // browser has already been sent somewhere.
        if let Err(reason) = self.store.usable() {
            tracing::warn!("no secure credential store: {reason}");
            return vec![Reply::Asker(Message::ConnectFailed {
                connect_id,
                error: ConnectError::NoSecureStore,
            })];
        }
        let (answers, queue) = flume::bounded::<Answer>(1);
        self.flows.insert(connect_id, Flow { client, answers });
        flow::spawn(flow::Job {
            connect_id,
            provider,
            instance,
            label,
            auth,
            client_id,
            oauth_app,
            connection,
            answers: queue,
            settings: self.settings.clone(),
            store: self.store.clone(),
            asker,
            everyone,
        });
        vec![Reply::Asker(Message::ConnectPending {
            connect_id,
            stage: ubiq_proto::connectors::ConnectStage::Opening,
        })]
    }

    /// Change a record and tell everybody. Every write in this file ends the same way: the settings
    /// blob the records live in, then the list drawn from it.
    fn change(&self, edit: impl FnOnce(&mut HostSettings) -> Result<(), String>) -> Vec<Reply> {
        let mut refused = None;
        let written = self.settings.update_host(|host| {
            refused = edit(host).err();
        });
        if let Some(reason) = refused {
            return vec![error(&reason)];
        }
        match written {
            Err(reason) => vec![error(&reason)],
            Ok(host) => vec![
                Reply::Everyone(Message::Settings {
                    layer: SettingsLayer::Host,
                    value: serde_json::to_string(&host).ok(),
                }),
                Reply::Everyone(Message::Connections {
                    connections: infos(&host, Some(&self.store), flow::now_ms()),
                    bundled: providers::bundled(),
                }),
            ],
        }
    }
}

/// The records, with what the store and the pin list add to them.
///
/// Shared with the flow threads, which announce the same list after a write — one place that
/// decides what a `ConnectionInfo` is.
pub fn infos(settings: &HostSettings, store: Option<&Store>, now_ms: i64) -> Vec<ConnectionInfo> {
    settings
        .connections
        .iter()
        .map(|connection| ConnectionInfo {
            status: match store {
                Some(store) => store.status(connection.provider, connection.id, now_ms),
                None => ubiq_proto::messages::LoginStatus::Missing,
            },
            pinned: connection
                .instance
                .as_deref()
                .and_then(ubiq_proto::connectors::origin)
                .is_some_and(|origin| {
                    settings
                        .trusted_certs
                        .iter()
                        .any(|held| held.origin == origin)
                }),
            connection: connection.clone(),
        })
        .collect()
}

/// One registration by id. A secret can only be set on a registration that exists, since the id it
/// would be filed under is the row's.
fn app_row(settings: &mut HostSettings, id: OauthAppId) -> Result<&mut OauthApp, String> {
    settings
        .oauth_apps
        .iter_mut()
        .find(|app| app.id == id)
        .ok_or_else(|| "no such registration".to_string())
}

fn error(reason: &str) -> Reply {
    Reply::Asker(Message::ConnectorError {
        error: reason.to_string(),
    })
}
