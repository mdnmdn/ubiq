//! One clone, on a thread of its own.
//!
//! The shape [`crate::connectors::flow`] set, for the same reason: a clone is minutes of network,
//! and the coordinator's thread carries keystrokes. The coordinator holds nothing but a sender,
//! and **dropping it is the whole cancel story** — the transfer callback sees the channel
//! disconnected and answers `false`, which is how libgit2 is told to stop mid-transfer.
//!
//! **A clone leaves either a repository or nothing.** Every way out that is not success removes
//! the destination it was writing, cancellation included, so a half-fetched tree never becomes a
//! project. The one exception is [`CloneError::Exists`], which is decided *before* anything is
//! written precisely so the removal can never reach a folder that was already there.
//!
//! Registering the result is not done here. `Projects` lives on the coordinator's thread and is
//! not this thread's to touch, so a finished clone posts a [`Registered`] and the coordinator
//! takes the folder into the catalogue — the ordinary `ProjectAdded` is the success signal, and
//! there is no clone-success message.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ubiq_proto::bus::{ClientId, Mailbox};
use ubiq_proto::connectors::ProviderId;
use ubiq_proto::ids::CloneId;
use ubiq_proto::messages::Message;
use ubiq_proto::repos::{CloneError, CloneRequest, CloneStage, RepoSource};

use crate::connectors::store::Store;
use crate::settings::Settings;

use super::Registered;

/// The floor between two progress messages.
///
/// `transfer_progress` fires per object, which on a large repository is tens of thousands of
/// messages a second onto a bus whose whole contract is that it never blocks the host. The
/// interface cannot draw them and the user cannot read them, so they are dropped here rather than
/// queued somewhere.
const THROTTLE: Duration = Duration::from_millis(250);

/// One clone, addressed and equipped. Everything it needs, so the thread borrows nothing.
pub struct Job {
    pub clone_id: CloneId,
    /// The window that asked, carried through so the coordinator can answer it when the folder
    /// comes back to be registered.
    pub client: ClientId,
    pub request: CloneRequest,
    /// Cancellation and nothing else: the coordinator holds the sender, and dropping it stops the
    /// transfer. Never sent on.
    pub answers: flume::Receiver<()>,
    pub settings: Arc<Settings>,
    pub store: Arc<Store>,
    /// The window that asked. Stages and the failure go here.
    pub asker: Mailbox,
    /// Where a finished clone is posted, for the coordinator to register.
    pub done: flume::Sender<Registered>,
}

/// Start a clone. It ends by itself, and the coordinator learns so when its sender disconnects.
pub fn spawn(job: Job) {
    let name = format!("ubiq-clone-{}", job.clone_id);
    if let Err(error) = thread::Builder::new().name(name).spawn(move || run(job)) {
        tracing::error!("a clone did not start: {error}");
    }
}

fn run(job: Job) {
    let destination = match destination(&job.request) {
        Ok(destination) => destination,
        Err(error) => return fail(&job, error),
    };
    // Decided before a byte is written, so the cleanup below can never reach a folder that was
    // already somebody's.
    if occupied(&destination) {
        return fail(&job, CloneError::Exists);
    }
    match fetch(&job, &destination) {
        Ok(()) => {}
        Err(error) => {
            // Whatever was written was written by this clone and only this clone.
            if let Err(error) = std::fs::remove_dir_all(&destination)
                && destination.exists()
            {
                tracing::warn!(
                    "a failed clone left {} behind: {error}",
                    destination.display()
                );
            }
            // Nobody is waiting to be told a clone they cancelled has stopped.
            if job.answers.is_disconnected() {
                tracing::debug!("clone {} was cancelled", job.clone_id);
            } else {
                fail(&job, error);
            }
        }
    }
}

/// The clone itself: resolve, fetch, check out, and hand the folder on to be registered.
fn fetch(job: &Job, destination: &Path) -> Result<(), CloneError> {
    stage(job, CloneStage::Resolving);
    let (url, credential) = resolve(job)?;
    if !url.starts_with("https://") {
        return Err(CloneError::Unsupported(url));
    }
    if let Some(parent) = destination.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return Err(CloneError::Refused(format!(
            "{}: {error}",
            parent.display()
        )));
    }

    let mut callbacks = git2::RemoteCallbacks::new();
    if let Some((user, secret)) = credential {
        callbacks.credentials(move |_, _, _| git2::Cred::userpass_plaintext(&user, &secret));
    }
    let (asker, clone_id) = (job.asker.clone(), job.clone_id);
    let answers = job.answers.clone();
    let mut spoke = Instant::now() - THROTTLE;
    callbacks.transfer_progress(move |progress| {
        // The one place a cancel takes effect: `false` aborts the transfer, and everything after
        // it is the failure path, which removes what was written.
        if answers.is_disconnected() {
            return false;
        }
        if spoke.elapsed() >= THROTTLE {
            spoke = Instant::now();
            let received = progress.received_objects();
            // Counting has no total to divide by yet, and is its own stage for that reason.
            let stage = if received == 0 {
                CloneStage::Counting
            } else {
                CloneStage::Receiving {
                    received: received as u32,
                    total: progress.total_objects() as u32,
                    bytes: progress.received_bytes() as u64,
                }
            };
            asker.send(Message::ClonePending { clone_id, stage });
        }
        true
    });

    let mut fetch = git2::FetchOptions::new();
    fetch.remote_callbacks(callbacks);
    if job.request.shallow {
        fetch.depth(1);
    }

    let asker = job.asker.clone();
    let mut spoke = Instant::now() - THROTTLE;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.progress(move |_, done, total| {
        if spoke.elapsed() >= THROTTLE {
            spoke = Instant::now();
            asker.send(Message::ClonePending {
                clone_id,
                stage: CloneStage::CheckingOut {
                    done: done as u32,
                    total: total as u32,
                },
            });
        }
    });

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch).with_checkout(checkout);
    if let Some(branch) = job.request.branch.as_deref().filter(|b| !b.is_empty()) {
        builder.branch(branch);
    }
    builder.clone(&url, destination).map_err(error)?;

    // Asked immediately before the folder is handed on, on the flow family's rule: a clone that
    // finished after the user cancelled leaves nothing rather than a project they did not ask for.
    if job.answers.is_disconnected() {
        return Err(CloneError::Refused("cancelled".to_string()));
    }
    stage(job, CloneStage::Registering);
    let _ = job.done.send(Registered {
        clone_id: job.clone_id,
        client: job.client,
        path: destination.to_string_lossy().into_owned(),
        name: job.request.name.clone(),
        ephemeral: job.request.ephemeral,
    });
    Ok(())
}

/// Where the clone comes from, and what it presents to get there.
///
/// A pasted URL is anonymous by definition. A connection lends its token, and the username half of
/// the basic-auth pair is the provider's own convention — GitLab reads `oauth2`, and everyone else
/// takes the token as the user.
fn resolve(job: &Job) -> Result<(String, Option<(String, String)>), CloneError> {
    match &job.request.source {
        RepoSource::Url(url) => Ok((url.clone(), None)),
        RepoSource::Connection {
            connection,
            clone_url,
            ..
        } => {
            let Some(record) = job
                .settings
                .host()
                .connections
                .into_iter()
                .find(|held| held.id == *connection)
            else {
                return Err(CloneError::Refused("no such connection".to_string()));
            };
            let credential =
                job.store
                    .token(record.provider, *connection)
                    .map(|token| match record.provider {
                        ProviderId::Gitlab => ("oauth2".to_string(), token.access_token),
                        _ => (token.access_token, "x-oauth-basic".to_string()),
                    });
            Ok((clone_url.clone(), credential))
        }
    }
}

/// `parent/name`, once the name is one.
///
/// The name comes from a text field, so it is checked rather than joined: anything with a
/// separator or a `..` in it would put the clone somewhere the user did not choose, and the
/// removal on failure would follow it there.
fn destination(request: &CloneRequest) -> Result<PathBuf, CloneError> {
    let name = request.name.trim();
    let sane = !name.is_empty()
        && Path::new(name).components().count() == 1
        && !matches!(name, "." | "..")
        && !name.contains(std::path::is_separator);
    if !sane {
        return Err(CloneError::Refused(format!(
            "{name:?} is not a folder name"
        )));
    }
    Ok(Path::new(&request.parent).join(name))
}

/// Whether something is already there. An empty directory is not: a folder the user made to clone
/// into is exactly what they meant.
fn occupied(destination: &Path) -> bool {
    match std::fs::read_dir(destination) {
        Ok(mut entries) => entries.next().is_some(),
        // Not a directory at all: a file with that name is still something in the way.
        Err(_) => destination.exists(),
    }
}

/// What libgit2 said, in the vocabulary the interface draws.
///
/// Shared with [`super::list`], whose anonymous ref listing fails the same ways for the same
/// reasons.
pub fn error(error: git2::Error) -> CloneError {
    match error.code() {
        git2::ErrorCode::Auth => CloneError::Auth,
        git2::ErrorCode::NotFound => CloneError::NotFound,
        git2::ErrorCode::Exists => CloneError::Exists,
        _ => CloneError::Network(error.message().to_string()),
    }
}

fn stage(job: &Job, stage: CloneStage) {
    job.asker.send(Message::ClonePending {
        clone_id: job.clone_id,
        stage,
    });
}

fn fail(job: &Job, error: CloneError) {
    job.asker.send(Message::CloneFailed {
        clone_id: job.clone_id,
        error,
    });
}
