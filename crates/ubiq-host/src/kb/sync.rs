//! Fetching one knowledge-base git source, on a thread of its own.
//!
//! The shape [`crate::repos::clone`] set, for the same reason: a fetch is minutes of network, and
//! the coordinator's thread carries keystrokes. Unlike a project clone there is nothing here for a
//! coordinator to register afterwards — a knowledge-base source has no catalogue entry — so a
//! finished sync only has to report itself, directly into [`super::Kb::overrides`] and onto the
//! asker's mailbox, and there is no cancel: a fetch that is already running is left to finish
//! rather than torn down under the working tree it might be resetting.
//!
//! **A clone leaves either a repository or nothing**, on [`crate::repos::clone`]'s own rule: a
//! failure removes the destination it was writing. A *refresh* does not — the directory already
//! held a working tree before this call, and a network blip must not cost the user the one they
//! had; its failure is only ever a sentence, left for the next attempt to clear.

use std::fs;
use std::time::{Duration, Instant};

use ubiq_proto::ids::KbSourceId;
use ubiq_proto::kb::{KbOrigin, KbSourceState};
use ubiq_proto::messages::Message;

use super::SyncJob as Job;

/// The floor between two progress messages, on [`crate::repos::clone::THROTTLE`]'s own reasoning:
/// nobody can read a message a network thread can produce thousands of a second.
const THROTTLE: Duration = Duration::from_millis(250);

/// Start a fetch. It ends by itself; nothing waits on it, because nothing here is cancellable.
pub fn spawn(job: Job) {
    let name = format!("ubiq-kb-sync-{}", job.source.id);
    if let Err(error) = std::thread::Builder::new()
        .name(name)
        .spawn(move || run(job))
    {
        tracing::error!("a knowledge-base sync did not start: {error}");
    }
}

fn run(job: Job) {
    let KbOrigin::Git { url, branch, .. } = job.source.origin.clone() else {
        // `Kb::sync_source` never calls this for a folder or a wiki; a dead branch that must
        // not silently become a stuck `Syncing` state if the rule above ever changes.
        return clear_and_post(&job, KbSourceState::Ready);
    };

    let existing = super::looks_like_repo(&job.dest);
    let result = if existing {
        refresh(&job, &url)
    } else {
        clone(&job, &url, branch.as_deref())
    };

    match result {
        Ok(()) => clear_and_post(&job, KbSourceState::Ready),
        Err(error) => {
            if !existing {
                // Whatever this attempt wrote was written by this attempt and only this attempt.
                if let Err(remove_error) = fs::remove_dir_all(&job.dest)
                    && job.dest.exists()
                {
                    tracing::warn!(
                        "a failed knowledge-base clone left {} behind: {remove_error}",
                        job.dest.display()
                    );
                }
            }
            set_and_post(&job, KbSourceState::Failed { error });
        }
    }
}

/// Clone a git source that has never been fetched, or whose last attempt left nothing behind.
fn clone(job: &Job, url: &str, branch: Option<&str>) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!(
            "{url:?} is not a URL this build can fetch (https only)"
        ));
    }
    // A directory that exists but is not a repository is a previous attempt's litter, not
    // something to clone into — the same call `occupied` in `repos::clone` decides against, minus
    // the "somebody else's folder" case a knowledge-base directory can never be, since nothing but
    // a sync ever writes here.
    if job.dest.exists() {
        fs::remove_dir_all(&job.dest).map_err(|error| error.to_string())?;
    }
    if let Some(parent) = job.dest.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    stage(job, "counting objects".to_string());
    let mut fetch = git2::FetchOptions::new();
    fetch.remote_callbacks(progress_callbacks(job));

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch);
    if let Some(branch) = branch.filter(|b| !b.is_empty()) {
        builder.branch(branch);
    }
    builder
        .clone(url, &job.dest)
        .map_err(|error| error.message().to_string())?;
    Ok(())
}

/// Refresh a source that is already a repository: fetch, then hard-reset the branch that is
/// checked out to what the remote now holds. The branch is read off `HEAD` rather than off the
/// source's own `branch` field, because that is what is actually on disk — a source edited to
/// name a different branch is a new checkout, which [`clone`] gives it once this directory is
/// gone.
fn refresh(job: &Job, url: &str) -> Result<(), String> {
    stage(job, "checking for updates".to_string());
    let repo = git2::Repository::open(&job.dest).map_err(|error| error.message().to_string())?;

    let head = repo.head().map_err(|error| error.message().to_string())?;
    let head_name = head
        .name()
        .map_err(|error| format!("the checked-out branch is not valid UTF-8: {error}"))?;
    let branch_name = head_name
        .strip_prefix("refs/heads/")
        .unwrap_or(head_name)
        .to_string();

    let mut remote = repo
        .find_remote("origin")
        .or_else(|_| repo.remote_anonymous(url))
        .map_err(|error| error.message().to_string())?;

    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.remote_callbacks(progress_callbacks(job));
    let refspec = format!("+refs/heads/{branch_name}:refs/remotes/origin/{branch_name}");
    remote
        .fetch(&[refspec.as_str()], Some(&mut fetch_opts), None)
        .map_err(|error| error.message().to_string())?;

    stage(job, "updating the working tree".to_string());
    let refname = format!("refs/remotes/origin/{branch_name}");
    let oid = repo
        .refname_to_id(&refname)
        .map_err(|error| error.message().to_string())?;
    let object = repo
        .find_object(oid, None)
        .map_err(|error| error.message().to_string())?;
    repo.reset(&object, git2::ResetType::Hard, None)
        .map_err(|error| error.message().to_string())?;
    Ok(())
}

/// Callbacks that turn libgit2's own progress into a throttled [`KbSourceState::Syncing`], on
/// [`crate::repos::clone::THROTTLE`]'s own rule: `transfer_progress` fires per object, and nobody
/// can read that many messages a second.
fn progress_callbacks(job: &Job) -> git2::RemoteCallbacks<'static> {
    let mut callbacks = git2::RemoteCallbacks::new();
    let (asker, project_id, source_id) = (job.asker.clone(), job.project_id, job.source.id);
    let overrides = job.overrides.clone();
    let mut spoke = Instant::now() - THROTTLE;
    callbacks.transfer_progress(move |progress| {
        if spoke.elapsed() >= THROTTLE {
            spoke = Instant::now();
            let detail = format!(
                "receiving {}/{} objects",
                progress.received_objects(),
                progress.total_objects()
            );
            let state = KbSourceState::Syncing { detail };
            set_state(&overrides, source_id, state.clone());
            asker.send(Message::KbSourceChanged {
                project_id,
                source: source_id,
                state,
            });
        }
        true
    });
    callbacks
}

fn stage(job: &Job, detail: String) {
    set_and_post(job, KbSourceState::Syncing { detail });
}

fn set_and_post(job: &Job, state: KbSourceState) {
    set_state(&job.overrides, job.source.id, state.clone());
    post(job, state);
}

fn clear_and_post(job: &Job, state: KbSourceState) {
    job.overrides
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&job.source.id);
    post(job, state);
}

fn set_state(overrides: &super::Overrides, id: KbSourceId, state: KbSourceState) {
    overrides
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(id, state);
}

fn post(job: &Job, state: KbSourceState) {
    job.asker.send(Message::KbSourceChanged {
        project_id: job.project_id,
        source: job.source.id,
        state,
    });
}
