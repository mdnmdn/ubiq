//! Cloning a repository into a project: the listings that find one, and the clones themselves.
//!
//! A clone is how a project enters the catalogue from somewhere other than a folder the user
//! already has, and it ends where an `AddProject` ends — a registered project. There is no
//! clone-success message for that reason: `ProjectAdded` is the signal.
//!
//! **Nothing in this family runs on the coordinator's thread.** A listing is one HTTP round trip
//! and a clone is minutes of transfer, and the coordinator's thread carries keystrokes. So the
//! shape is [`crate::connectors`]' shape: a thread per operation, a mailbox it answers through,
//! and a coordinator that holds a sender and no more. A listing is fire and forget — it answers
//! [`Message::Repos`] or [`Message::RepoError`] and ends — while a clone is kept, because it can
//! be cancelled and because the folder it produces has to reach [`crate::projects::Projects`].
//!
//! The one thing that cannot be done from a clone's own thread is registering the result: the
//! catalogue lives on the coordinator's thread. So a finished clone posts a [`Registered`], and
//! [`Repos::registered`] is what the coordinator drains.
//!
//! - `list`: the provider calls — which repositories, and which branches
//! - `clone`: one clone on a thread, its progress, and its cancel

pub mod clone;
pub mod list;

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use ubiq_proto::bus::{ClientId, Mailbox};
use ubiq_proto::ids::{CloneId, ConnectionId, RepoQueryId};
use ubiq_proto::messages::Message;
use ubiq_proto::repos::{CloneError, CloneRequest, RepoSource};

use crate::connectors::store::Store;
use crate::settings::Settings;

use self::list::Identity;

/// The repository family, as the coordinator holds it.
pub struct Repos {
    settings: Arc<Settings>,
    /// The connector family's own store, lent rather than opened again — a clone borrows a
    /// connection's token, and that is the only place one lives.
    store: Arc<Store>,
    /// The clones in flight. Dropping an entry cancels the clone it names.
    running: HashMap<CloneId, Running>,
    finished: flume::Sender<Registered>,
    done: flume::Receiver<Registered>,
}

/// One running clone, from the coordinator's side: who is watching it, and the one way to stop it.
struct Running {
    client: ClientId,
    /// Never sent on. Dropping it is what a cancel is.
    answers: flume::Sender<()>,
}

/// A clone that finished, waiting to be taken into the catalogue.
///
/// The clone's own thread cannot do it — `Projects` is the coordinator's — so this crosses back
/// and the coordinator adds the folder exactly as it would a folder the user picked.
pub struct Registered {
    pub clone_id: CloneId,
    pub client: ClientId,
    pub path: String,
    pub name: String,
    pub ephemeral: bool,
}

impl Repos {
    pub fn new(settings: Arc<Settings>, store: Arc<Store>) -> Self {
        let (finished, done) = flume::unbounded();
        Self {
            settings,
            store,
            running: HashMap::new(),
            finished,
            done,
        }
    }

    /// The repositories a connection can see. Answers nothing here: the thread does.
    pub fn list(
        &self,
        query_id: RepoQueryId,
        connection: ConnectionId,
        query: Option<String>,
        asker: Mailbox,
    ) {
        let (settings, store) = (self.settings.clone(), self.store.clone());
        query_thread(
            query_id,
            move || match identity(&settings, &store, connection) {
                Ok(who) => match list::repos(&who, query.as_deref()) {
                    Ok((repos, truncated)) => Message::Repos {
                        query_id,
                        repos,
                        truncated,
                    },
                    Err(error) => Message::RepoError { query_id, error },
                },
                Err(error) => Message::RepoError { query_id, error },
            },
            asker,
        );
    }

    /// One repository's branches, however the user got to it.
    ///
    /// A connection asks the provider; a pasted URL has no provider to ask and is answered by git
    /// itself, anonymously.
    pub fn branches(&self, query_id: RepoQueryId, source: RepoSource, asker: Mailbox) {
        let (settings, store) = (self.settings.clone(), self.store.clone());
        query_thread(
            query_id,
            move || {
                let found = match &source {
                    RepoSource::Url(url) => list::remote_branches(url),
                    RepoSource::Connection {
                        connection, repo, ..
                    } => identity(&settings, &store, *connection)
                        .and_then(|who| list::branches(&who, repo)),
                };
                match found {
                    Ok((branches, default)) => Message::RepoBranches {
                        query_id,
                        branches,
                        default,
                    },
                    Err(error) => Message::RepoError { query_id, error },
                }
            },
            asker,
        );
    }

    /// Start a clone. Everything after this — every stage, the failure, the folder — comes from
    /// the thread.
    pub fn clone(&mut self, client: ClientId, request: CloneRequest, asker: Mailbox) {
        // Reaped where they are minted, so a finished clone's row never outlives the next start.
        self.running
            .retain(|_, running| !running.answers.is_disconnected());
        let clone_id = request.clone_id;
        let (answers, queue) = flume::bounded::<()>(1);
        self.running.insert(clone_id, Running { client, answers });
        clone::spawn(clone::Job {
            clone_id,
            client,
            request,
            answers: queue,
            settings: self.settings.clone(),
            store: self.store.clone(),
            asker,
            done: self.finished.clone(),
        });
    }

    /// Stop a clone. Dropping the sender is the whole operation: the transfer callback's next
    /// answer is `false`, and the thread removes what it had written.
    pub fn cancel(&mut self, clone_id: CloneId) {
        self.running.remove(&clone_id);
    }

    /// A window went. Every clone it was watching goes with it.
    pub fn client_gone(&mut self, client: ClientId) {
        self.running.retain(|_, running| running.client != client);
    }

    /// The clones that finished since this was last asked, for the coordinator to register.
    pub fn registered(&mut self) -> Vec<Registered> {
        let found: Vec<Registered> = self.done.try_iter().collect();
        for done in &found {
            self.running.remove(&done.clone_id);
        }
        found
    }

    /// Whether a clone is in flight, so the run loop knows to keep waking to collect one.
    pub fn busy(&self) -> bool {
        !self.running.is_empty()
    }
}

/// A connection, resolved into what a provider call needs.
///
/// On the query's own thread, not the coordinator's: reading the token is a keychain call, and
/// this family's rule is that nothing it does happens where keystrokes are carried.
fn identity(
    settings: &Settings,
    store: &Store,
    connection: ConnectionId,
) -> Result<Identity, CloneError> {
    let host = settings.host();
    let Some(record) = host
        .connections
        .iter()
        .find(|held| held.id == connection)
        .cloned()
    else {
        return Err(CloneError::Refused("no such connection".to_string()));
    };
    // The pin the user vouched for at this instance, looked up the way a flow looks one up.
    let pin =
        crate::connectors::providers::instance_origin(record.provider, record.instance.as_deref())
            .ok()
            .and_then(|origin| {
                host.trusted_certs
                    .iter()
                    .find(|held| held.origin == origin)
                    .map(|held| held.sha256.clone())
            });
    Ok(Identity {
        token: store
            .token(record.provider, connection)
            .map(|token| token.access_token),
        provider: record.provider,
        instance: record.instance,
        account: record.account,
        pin,
    })
}

/// One listing on a thread of its own, answering the window that asked and then ending.
fn query_thread(
    query_id: RepoQueryId,
    ask: impl FnOnce() -> Message + Send + 'static,
    asker: Mailbox,
) {
    let name = format!("ubiq-repos-{query_id}");
    let sink = asker.clone();
    if let Err(error) = thread::Builder::new().name(name).spawn(move || {
        sink.send(ask());
    }) {
        tracing::error!("a repository listing did not start: {error}");
        asker.send(Message::RepoError {
            query_id,
            error: CloneError::Refused(error.to_string()),
        });
    }
}
