//! A project's knowledge base: the sources it is configured with, and what the host knows about
//! each one now.
//!
//! The list itself is [`store`]'s: one TOML file per project, on [`crate::store::file::FileTaskStore`]'s
//! own convention — missing is an empty list, not an error, and nothing here is written inside the
//! user's project folder unless the source itself asks for that (`KbStore::Project`), on `D30`'s
//! own exception. A fetched source lands under a directory of its own, resolved once, in
//! [`Kb::base_path`] — the interface never learns that path, and nothing else in this module
//! composes one beside it.
//!
//! **A source's state is derived, not stored**, except for the two states that disk cannot answer on
//! its own: `Syncing`, which is true only while a thread is doing it, and `Failed`, which a removed
//! clone directory (see [`sync`]) would otherwise read back as merely `Pending`. Those two live in
//! [`Kb::overrides`], in memory, and are gone on a restart — a source that failed last session simply
//! reads `Pending` again and is retried the next time its project's source list is written, on
//! [`Kb::set_sources`]'s own rule.
//!
//! **Every entry point takes the project's own path**, even the ones a folder or a `Cache`/`Internal`
//! git source never look at: the one origin that does — `KbOrigin::Git { store: KbStore::Project, .. }`,
//! and later `KbOrigin::Internal` sources that move to sit beside it — must never be a special path
//! through this module, or the day a source becomes writable, this is the boundary that would have
//! to be reopened.
//!
//! This is a concrete service, not a trait: there is one implementation and nothing substitutes it,
//! the same shape [`crate::store::usage::Usage`] takes and for the same reason.
//!
//! Writing to a source — [`ops`] — is a sibling rather than a method here: [`Kb`] answers "what are
//! the sources and where does each one live", and `ops` is the free functions that act once that is
//! known, so a caller with no coordinator around it (an MCP server, among others) can use the same
//! containment and writability rules without a bus.
//!
//! **A password-protected wiki** ([`vault`], `D206`) is an internal source whose documents are
//! sealed on disk. Its key is held here, in memory, once a window has supplied the password
//! ([`Kb::unlock`]) or the keychain had it filed ([`Kb::auto_unlock`]); until then the source reads
//! [`KbSourceState::Locked`] and [`Kb::vault_key`] refuses every request against it. The flag is
//! fixed when the source is created — [`Kb::set_sources`] keeps whatever the saved list said.

pub mod ops;
mod store;
#[cfg(feature = "git")]
mod sync;
pub mod vault;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use ubiq_proto::files::FileError;

use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::{KbSourceId, ProjectId};
use ubiq_proto::kb::{KbOrigin, KbSource, KbSourceState, KbSourceStatus, KbStore};
#[cfg(not(feature = "git"))]
use ubiq_proto::messages::Message;

/// The overrides map, shared with a sync thread — spelled out once, since [`SyncJob`] and
/// [`sync`] both hold one.
pub(crate) type Overrides = Arc<Mutex<HashMap<KbSourceId, KbSourceState>>>;

/// The knowledge-base family, as the coordinator holds it.
pub struct Kb {
    /// The config root, exactly what [`crate::store::file`] resolves every per-project path
    /// against.
    root: PathBuf,
    /// The state a source's own directory cannot answer: `Syncing` while [`sync`] is running for
    /// it, `Failed` after the last attempt did not leave a repository behind. Cleared once a sync
    /// succeeds — from there, [`Kb::state_of`] derives `Ready` from the directory itself, the same
    /// as it would have all along if nothing had gone wrong.
    ///
    /// `Arc` because a sync runs on a thread of its own (`sync::spawn`), on
    /// [`crate::store::harness::FileHarnessCache`]'s reason: the thread cannot borrow `&Kb`, only
    /// share what it has to change.
    overrides: Overrides,
    /// The keys of the protected wikis open this run. Never written anywhere.
    keys: Mutex<HashMap<KbSourceId, Arc<vault::Key>>>,
    /// The protected wikis the keychain has already been asked about this run, so a missing or
    /// stale password costs one keychain read and one derivation, not one per listing.
    asked: Mutex<HashSet<KbSourceId>>,
    keychain: Arc<dyn vault::WikiKeychain>,
    kdf: vault::KdfParams,
}

impl Kb {
    /// A knowledge base with no keychain: a protected wiki is asked for on every run.
    pub fn new(config_root: PathBuf) -> Self {
        Self::with_keychain(
            config_root,
            Arc::new(vault::NoKeychain),
            vault::KdfParams::default(),
        )
    }

    /// The same, filing protected wikis' passwords in `keychain` and deriving new keys at `kdf`'s
    /// cost — the OS keychain and the defaults in production, a [`vault::MemoryKeychain`] and
    /// [`vault::KdfParams::insecure_fast`] in a test.
    pub fn with_keychain(
        config_root: PathBuf,
        keychain: Arc<dyn vault::WikiKeychain>,
        kdf: vault::KdfParams,
    ) -> Self {
        Self {
            root: config_root,
            overrides: Arc::new(Mutex::new(HashMap::new())),
            keys: Mutex::new(HashMap::new()),
            asked: Mutex::new(HashSet::new()),
            keychain,
            kdf,
        }
    }

    /// A project's sources, each with what the host knows about it now.
    pub fn sources(&self, project: ProjectId, project_path: &Path) -> Vec<KbSourceStatus> {
        store::load(&self.list_path(project))
            .into_iter()
            .map(|source| {
                let state = self.state_of(project, &source, project_path);
                KbSourceStatus { source, state }
            })
            .collect()
    }

    /// Replace a project's whole source list, and fetch whatever git source is not yet ready — new
    /// this call or left `Pending`/`Failed` by a previous one.
    ///
    /// `asker` is the window's mailbox: a sync started from here reports its progress to whoever
    /// asked for the write, the same address `SyncKbSource` would use.
    pub fn set_sources(
        &self,
        project: ProjectId,
        sources: Vec<KbSource>,
        asker: Mailbox,
        project_path: &Path,
    ) -> Vec<KbSourceStatus> {
        // Protection is chosen at creation and never flipped: a source already saved keeps the
        // flag it was saved with, and only an internal one may carry it at all.
        let before: HashMap<KbSourceId, KbSource> = store::load(&self.list_path(project))
            .into_iter()
            .map(|source| (source.id, source))
            .collect();
        let sources: Vec<KbSource> = sources
            .into_iter()
            .map(|mut source| {
                source.protected = match before.get(&source.id) {
                    Some(saved) => saved.protected,
                    None => source.protected,
                } && source.origin.is_internal();
                source
            })
            .collect();
        // A protected wiki dropped from the list takes its key and its filed password with it.
        // Its sealed pages stay on disk, as an unprotected wiki's do.
        for gone in before
            .values()
            .filter(|old| old.is_protected() && !sources.iter().any(|source| source.id == old.id))
        {
            lock(&self.keys).remove(&gone.id);
            if let Err(error) = self.keychain.forget(gone.id) {
                tracing::warn!("a removed wiki's password could not be forgotten: {error}");
            }
        }

        if let Err(error) = store::save(&self.list_path(project), &sources) {
            // The session carries on from memory; the list this call answers with is what the
            // window sees, whether or not it is durable — the same degradation
            // `FileProjectStore` reports for the catalogue, minus the flag, because a lost source
            // list costs the user a re-typed URL and not a lost project.
            tracing::warn!(
                "the knowledge-base sources for project {project} could not be written: {error}"
            );
        }

        // An id no longer in the list has nothing left to say `Syncing` or `Failed` about.
        let keep: std::collections::HashSet<KbSourceId> =
            sources.iter().map(|source| source.id).collect();
        self.overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|id, _| keep.contains(id));

        let statuses: Vec<KbSourceStatus> = sources
            .into_iter()
            .map(|source| {
                let state = self.state_of(project, &source, project_path);
                KbSourceStatus { source, state }
            })
            .collect();

        for status in &statuses {
            if matches!(status.source.origin, KbOrigin::Git { .. }) && !status.state.is_ready() {
                self.sync_source(project, status.source.clone(), asker.clone(), project_path);
            }
        }

        statuses
    }

    /// One source of a project's list, for the file family's job to resolve against. `None` is
    /// every refusal the file worker needs: no such project has ever written a list, or this id is
    /// not in the one it wrote.
    pub fn find(&self, project: ProjectId, source_id: KbSourceId) -> Option<KbSource> {
        store::load(&self.list_path(project))
            .into_iter()
            .find(|source| source.id == source_id)
    }

    /// Where a source's documents actually are, resolved once and never composed a second time.
    ///
    /// A folder is read where the user pointed. A wiki is Ubiq's own, under the project's own area
    /// of the config root, created on demand so it is `Ready` the moment it is asked for. A git
    /// source is fetched onto a directory keyed on its own id, so two sources pointed at the same
    /// repository never collide — where, [`KbStore`] says: `Cache` under the machine's temporary
    /// area, `Internal` under the project's own area of the config root (the default, and `D30`'s
    /// guarantee), `Project` under `.ubiq/local/kb` inside the project itself — a clone is derived
    /// and re-fetchable wherever it lands, so it is on the ignored side of `.ubiq/` — the one arm that writes
    /// there. `project_path` is the project's own folder; every caller carries one because it is
    /// only ever looked at for `Project`, never conditionally fetched for it.
    pub fn base_path(&self, project: ProjectId, source: &KbSource, project_path: &Path) -> PathBuf {
        match &source.origin {
            KbOrigin::Folder { path } => PathBuf::from(path),
            KbOrigin::Git { store, .. } => match store {
                KbStore::Cache => std::env::temp_dir()
                    .join("ubiq-kb")
                    .join(project.to_string())
                    .join(source.id.to_string()),
                KbStore::Internal => {
                    crate::store::project_dir::ProjectData::under_config(&self.root, project)
                        .kb_clones()
                        .join(source.id.to_string())
                }
                KbStore::Project => {
                    crate::store::project_dir::ProjectData::in_project(project_path)
                        .kb_clones()
                        .join(source.id.to_string())
                }
            },
            KbOrigin::Internal => {
                let wiki = if source.is_protected() {
                    self.vault_dir(project, source.id).join(vault::PAGES)
                } else {
                    crate::store::project_dir::ProjectData::under_config(&self.root, project).wiki()
                };
                if let Err(error) = std::fs::create_dir_all(&wiki) {
                    tracing::warn!(
                        "the wiki directory for project {project} could not be created: {error}"
                    );
                }
                wiki
            }
        }
    }

    /// Fetch or refresh one source, on [`ubiq_proto::messages::Message::SyncKbSource`]'s own
    /// request: unlike the automatic fetch [`Kb::set_sources`] runs, this always tries a git
    /// source, ready or not, and does nothing at all for a folder or a wiki — a folder is already
    /// as current as the disk under it, and a wiki has nothing to fetch.
    pub fn sync(
        &self,
        project: ProjectId,
        source_id: KbSourceId,
        asker: Mailbox,
        project_path: &Path,
    ) {
        let Some(source) = self.find(project, source_id) else {
            // Nobody is watching a source that was never written down; there is nothing to answer
            // and no thread to spare on it.
            return;
        };
        if matches!(source.origin, KbOrigin::Git { .. }) {
            self.sync_source(project, source, asker, project_path);
        }
    }

    /// Start a fetch for one git source, unless one is already running for it.
    fn sync_source(
        &self,
        project: ProjectId,
        source: KbSource,
        asker: Mailbox,
        project_path: &Path,
    ) {
        let dest = self.base_path(project, &source, project_path);
        let mut overrides = self
            .overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(
            overrides.get(&source.id),
            Some(KbSourceState::Syncing { .. })
        ) {
            return;
        }
        overrides.insert(
            source.id,
            KbSourceState::Syncing {
                detail: "starting".to_string(),
            },
        );
        drop(overrides);

        spawn_sync(SyncJob {
            project_id: project,
            source,
            dest,
            asker,
            overrides: self.overrides.clone(),
        });
    }

    /// What the host knows about a source right now: an override in flight or left by the last
    /// attempt, or — the ordinary case — whatever the disk itself says.
    fn state_of(
        &self,
        project: ProjectId,
        source: &KbSource,
        project_path: &Path,
    ) -> KbSourceState {
        if let Some(state) = self
            .overrides
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&source.id)
        {
            return state.clone();
        }

        match &source.origin {
            KbOrigin::Folder { path } => {
                if Path::new(path).is_dir() {
                    KbSourceState::Ready
                } else {
                    KbSourceState::Failed {
                        error: format!("{path} does not exist, or is not a directory"),
                    }
                }
            }
            KbOrigin::Git { .. } => {
                let base = self.base_path(project, source, project_path);
                if looks_like_repo(&base) {
                    KbSourceState::Ready
                } else {
                    KbSourceState::Pending
                }
            }
            KbOrigin::Internal if source.is_protected() => {
                if lock(&self.keys).contains_key(&source.id) {
                    KbSourceState::Ready
                } else {
                    // A lost header over sealed pages is not a first use: no new password may be
                    // set over them, and the unlock that tries says why.
                    let dir = self.vault_dir(project, source.id);
                    KbSourceState::Locked {
                        first_use: !vault::is_created(&dir) && !vault::is_orphaned(&dir),
                    }
                }
            }
            KbOrigin::Internal => {
                // `base_path` already creates the wiki directory on demand, so reaching this arm
                // at all means it exists — a wiki is `Ready` the moment it is asked about.
                self.base_path(project, source, project_path);
                KbSourceState::Ready
            }
        }
    }

    /// Where a protected wiki's header and pages live.
    fn vault_dir(&self, project: ProjectId, source: KbSourceId) -> PathBuf {
        crate::store::project_dir::ProjectData::under_config(&self.root, project)
            .protected_wiki(source)
    }

    /// Open every protected wiki of a project whose password the keychain has filed, asking the
    /// keychain once per source per run. Called before a listing goes to a window — never from an
    /// agent's tools, which never reach a protected wiki at all.
    pub fn auto_unlock(&self, project: ProjectId) {
        for source in store::load(&self.list_path(project)) {
            if source.is_protected() {
                self.try_keychain(project, &source);
            }
        }
    }

    /// The key a request against `source` is sealed and opened with. `Ok(None)` is a source that
    /// is not protected; a protected one the host holds no key for — after one try of the keychain
    /// — is refused, on the same footing as every other refusal in this family.
    pub fn vault_key(
        &self,
        project: ProjectId,
        source: &KbSource,
    ) -> Result<Option<Arc<vault::Key>>, FileError> {
        if !source.is_protected() {
            return Ok(None);
        }
        self.try_keychain(project, source);
        lock(&self.keys)
            .get(&source.id)
            .cloned()
            .map(Some)
            .ok_or_else(|| FileError::Refused("this wiki is locked".to_string()))
    }

    fn try_keychain(&self, project: ProjectId, source: &KbSource) {
        if lock(&self.keys).contains_key(&source.id) || !lock(&self.asked).insert(source.id) {
            return;
        }
        let dir = self.vault_dir(project, source.id);
        if !vault::is_created(&dir) {
            return;
        }
        let Some(password) = self.keychain.password(source.id) else {
            return;
        };
        match vault::open(&dir, &password) {
            Ok(key) => {
                lock(&self.keys).insert(source.id, Arc::new(key));
            }
            // A filed password that no longer opens it — changed on another machine, say — is a
            // prompt, not an error.
            Err(error) => {
                tracing::info!("a protected wiki did not open from the keychain: {error}")
            }
        }
    }

    /// Open a protected wiki with a password a window supplied — or, the first time, set it.
    /// `remember` files the password in the keychain; `false` forgets one filed before. A keychain
    /// that will not take it is logged and does not fail the unlock: the wiki is open either way.
    pub fn unlock(
        &self,
        project: ProjectId,
        source_id: KbSourceId,
        password: &str,
        remember: bool,
    ) -> Result<(), String> {
        let source = self.protected(project, source_id)?;
        let dir = self.vault_dir(project, source.id);
        let key = if vault::is_created(&dir) {
            vault::open(&dir, password)
        } else {
            vault::create(&dir, password, self.kdf)
        }
        .map_err(|error| error.to_string())?;
        lock(&self.keys).insert(source.id, Arc::new(key));
        self.file_password(source.id, password, remember);
        Ok(())
    }

    /// Replace a protected wiki's password, re-sealing every document under the new key.
    pub fn change_password(
        &self,
        project: ProjectId,
        source_id: KbSourceId,
        old: &str,
        new: &str,
        remember: bool,
    ) -> Result<(), String> {
        let source = self.protected(project, source_id)?;
        let dir = self.vault_dir(project, source.id);
        let key = vault::change(&dir, old, new, self.kdf).map_err(|error| error.to_string())?;
        lock(&self.keys).insert(source.id, Arc::new(key));
        self.file_password(source.id, new, remember);
        Ok(())
    }

    fn protected(&self, project: ProjectId, source_id: KbSourceId) -> Result<KbSource, String> {
        match self.find(project, source_id) {
            Some(source) if source.is_protected() => Ok(source),
            Some(_) => Err("this source is not password-protected".to_string()),
            None => Err("no such knowledge-base source".to_string()),
        }
    }

    fn file_password(&self, source: KbSourceId, password: &str, remember: bool) {
        let filed = if remember {
            self.keychain.remember(source, password)
        } else {
            self.keychain.forget(source)
        };
        if let Err(error) = filed {
            tracing::warn!("a protected wiki's password could not be filed or forgotten: {error}");
        }
    }

    fn list_path(&self, project: ProjectId) -> PathBuf {
        store::path(&self.root, project)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether a directory is a git repository's working copy, cheaply: no library needed for this,
/// only for the fetch itself, so it answers the same whether or not this build has the `git`
/// feature.
fn looks_like_repo(path: &Path) -> bool {
    let dot_git = path.join(".git");
    dot_git.is_dir() || dot_git.is_file()
}

/// One fetch, addressed and equipped — [`sync::Job`] under the `git` feature, and the whole of what
/// the fallback below needs to answer with a sentence instead of a thread.
pub(crate) struct SyncJob {
    pub project_id: ProjectId,
    pub source: KbSource,
    /// Resolved once, by [`Kb::base_path`], and never recomposed here.
    pub dest: PathBuf,
    /// The window that asked — `SetKbSources`, or `SyncKbSource` directly.
    pub asker: Mailbox,
    pub overrides: Overrides,
}

#[cfg(feature = "git")]
fn spawn_sync(job: SyncJob) {
    sync::spawn(job);
}

/// Without `git` there is no library to fetch with. Said once, in place of the thread, the same way
/// [`crate::files::diff_answer`] says it for a diff.
#[cfg(not(feature = "git"))]
fn spawn_sync(job: SyncJob) {
    // Nothing here reads `dest`: without the library there is nowhere to write a clone to yet.
    let _ = &job.dest;
    let error =
        "this build has no version control in it, so a git knowledge-base source cannot be fetched"
            .to_string();
    job.overrides
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            job.source.id,
            KbSourceState::Failed {
                error: error.clone(),
            },
        );
    job.asker.send(Message::KbSourceChanged {
        project_id: job.project_id,
        source: job.source.id,
        state: KbSourceState::Failed { error },
    });
}
