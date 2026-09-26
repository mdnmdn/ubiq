//! The host's half of the task-source wire family — the arms `D187` drew a page against and
//! `G364` recorded as missing.
//!
//! M7 built every surface and every message they send, and `crates/ubiq-host` answered none of
//! them: the settings page drew correctly and did nothing. This module is the other half. It holds
//! what the coordinator cannot lend out — the registry, the settings, the connector store, the
//! project directories and a [`crate::work::Handle`] clone — and answers each ask of §3.6.
//!
//! **Nothing here runs on the coordinator's thread except what touches no network.** That is
//! [`crate::repos`]' own division and for its own reason: the coordinator's thread carries
//! keystrokes, and a board listing is a round trip. So:
//!
//! | Answered here, inline | Answered on a thread of its own |
//! |---|---|
//! | `ListTaskProviders` — the registry, already in memory | `ListRemoteContainers`, `ListRemoteLanes` |
//! | `GetTaskSource` / `SetTaskSource` — one file | `TestTaskSource`, `ListRemoteItems` |
//! | `UnlinkTask` — one row of one file | `ImportRemoteItems`, `SyncTaskSource`, `ResolveTaskDrift` |
//!
//! **A query-shaped ask answers its asker; a project-shaped one answers everybody.** A listing is
//! for the window that typed the filter and carries a `TaskSrcQueryId` so a stale one is dropped;
//! a pass changes the project's tasks, and every window holding that project has to redraw. That
//! is also why a project-shaped failure comes back on `TaskSourceState` rather than on
//! `TaskSourceError`, which is keyed by a query id nobody minted.
//!
//! **The sidecar is written under one gate**, shared with the polling worker. Two passes over one
//! project's `tasksrc.toml` — the five-minute tick and a force button pressed during it — would
//! otherwise be last-writer-wins over the link table, which is the `G29` shape one directory down.
//!
//! **Nothing here holds credential material.** Every call resolves a [`Identity`] from a
//! `ConnectionId` for the length of that call, exactly as [`crate::repos`] does.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::Utc;
use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::{ConnectionId, ProjectId, TaskId, TaskSrcQueryId};
use ubiq_proto::messages::Message;
use ubiq_proto::tasksrc::{Binding, DriftSide, RemoteItemId, SyncScope, SyncState, TaskLink};

use super::store::{self, TasksrcFile};
use super::sync::{Pass, Reconcile, identity};
use super::{Identity, Registry, TaskProvider};
use crate::connectors::store::Store;
use crate::settings::Settings;
use crate::store::project_dir::ProjectDirs;

/// How many of a Test's matches come back beside the count.
///
/// `R7`'s whole point is that a bare number is a claim and a number with a few titles under it is
/// evidence. Five is enough to recognise the filter's shape and few enough that the reply stays a
/// reply rather than a listing.
const SAMPLE: usize = 5;

/// The task-source family, as the coordinator holds it.
pub struct TaskSrc {
    registry: Arc<Registry>,
    settings: Arc<Settings>,
    /// The connector family's own store, lent rather than opened again — a provider call borrows
    /// a connection's token, and that is the only place one lives.
    connections: Arc<Store>,
    dirs: ProjectDirs,
    work: crate::work::Handle,
    /// Held across every read-modify-write of a `tasksrc.toml`, by this family and by the polling
    /// worker both. One gate for every project rather than one per project: a pass is seconds of
    /// network and milliseconds of file, so the contention is nil and the alternative is a map
    /// whose entries nobody reaps.
    gate: Arc<Mutex<()>>,
}

impl TaskSrc {
    pub fn new(
        registry: Arc<Registry>,
        settings: Arc<Settings>,
        connections: Arc<Store>,
        dirs: ProjectDirs,
        work: crate::work::Handle,
        gate: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            registry,
            settings,
            connections,
            dirs,
            work,
            gate,
        }
    }

    // ── answered here, because none of it touches a network ──────────

    /// What this build can sync against. The one message a second edition's provider reaches the
    /// interface through at all (`R6`): the schema is declared in the host half and drawn in the
    /// interface half.
    pub fn providers(&self, query_id: TaskSrcQueryId, asker: &Mailbox) {
        asker.send(Message::TaskProviders {
            query_id,
            providers: self.registry.infos(),
        });
    }

    /// What this project is bound to, if anything, and what its binding is doing.
    pub fn get(&self, project_id: ProjectId, asker: &Mailbox) {
        let file = self.read(project_id);
        asker.send(source_message(project_id, &file));
        // The link rows too, unasked: the card badges are drawn from them and a window that has
        // just opened a project holds none. Cheap — a project has as many rows as it has linked
        // tasks, and they are already in memory.
        for row in &file.links {
            asker.send(Message::TaskLinkChanged {
                project_id,
                task_id: row.task,
                link: Some(Box::new(row.clone())),
            });
        }
    }

    /// Write the binding this project is configured with, or unbind it.
    ///
    /// **Unbinding deletes no task.** The binding and the link rows go and every task stays
    /// exactly as it is, with its content and its history (`R9`). A task that was linked simply
    /// stops being, which is what the badge disappearing means.
    ///
    /// A binding whose lane map is empty is seeded on a thread afterwards, because seeding needs
    /// the container's lanes and that is a round trip. The save itself is not made to wait on it:
    /// a binding that is written and then improved is better than a save that blocks on a board.
    pub fn set(
        &self,
        project_id: ProjectId,
        binding: Option<Binding>,
        asker: Mailbox,
        everyone: Mailbox,
    ) {
        let path = store::path(&self.dirs, project_id);
        let held = hold(&self.gate);
        let mut file = store::load(&path);
        let unbinding = binding.is_none();
        let keep = binding.as_ref().map(|held| held.id);
        match &binding {
            Some(binding) => file.bindings = vec![binding.clone()],
            None => file.bindings.clear(),
        }
        // Rows belonging to a binding that is gone are rows about nothing. The **tasks** they name
        // are untouched, which is the whole of `R9` here.
        let dropped: Vec<TaskId> = file
            .links
            .iter()
            .filter(|row| keep != Some(row.binding))
            .map(|row| row.task)
            .collect();
        file.links.retain(|row| keep == Some(row.binding));
        let saved = store::save(&path, &file);
        drop(held);

        if let Err(error) = saved {
            everyone.send(failed(project_id, &file, format!("{error}")));
            return;
        }
        let answer = source_message(project_id, &file);
        asker.send(answer.clone());
        everyone.send(answer);
        for task_id in dropped {
            everyone.send(Message::TaskLinkChanged {
                project_id,
                task_id,
                link: None,
            });
        }
        if unbinding {
            return;
        }
        let Some(binding) = binding.filter(|held| held.lanes.is_empty()) else {
            return;
        };
        spawn(
            "tasksrc-seed",
            self.calling_for(project_id),
            everyone,
            move |ctx| {
                let (who, provider) = ctx.provider(&binding)?;
                let lanes = provider.lanes(&who, &binding)?;
                let held = hold(&ctx.gate);
                let mut file = store::load(&ctx.path);
                if let Some(stored) = file.bindings.iter_mut().find(|held| held.id == binding.id)
                    && stored.lanes.is_empty()
                {
                    store::seed_lanes(stored, &lanes);
                }
                let saved = store::save(&ctx.path, &file);
                drop(held);
                saved?;
                Ok(vec![source_message(ctx.project, &file)])
            },
        );
    }

    /// Drop one task's link row. The task keeps its content and its history (`R9`).
    pub fn unlink(&self, project_id: ProjectId, task_id: TaskId, everyone: &Mailbox) {
        let path = store::path(&self.dirs, project_id);
        let held = hold(&self.gate);
        let mut file = store::load(&path);
        file.links.retain(|row| row.task != task_id);
        let saved = store::save(&path, &file);
        drop(held);
        if let Err(error) = saved {
            everyone.send(failed(project_id, &file, format!("{error}")));
            return;
        }
        everyone.send(Message::TaskLinkChanged {
            project_id,
            task_id,
            link: None,
        });
        everyone.send(state_message(project_id, &file, None));
    }

    // ── answered on a thread, because every one is a round trip ──────

    pub fn containers(
        &self,
        query_id: TaskSrcQueryId,
        provider: String,
        connection: ConnectionId,
        asker: Mailbox,
    ) {
        let registry = self.registry.clone();
        let settings = self.settings.clone();
        let connections = self.connections.clone();
        query_thread(query_id, asker, move || {
            let provider = registry
                .get(&provider)
                .with_context(|| format!("no task provider called {provider} in this build"))?;
            let who = identity(&settings, &connections, connection)?;
            Ok(Message::RemoteContainers {
                query_id,
                containers: provider.containers(&who)?,
            })
        });
    }

    /// The bound container's lanes **and** every facet the declared fields draw from, in one ask,
    /// because the provider answers both off one container description.
    pub fn lanes(&self, query_id: TaskSrcQueryId, binding: Binding, asker: Mailbox) {
        let ctx = self.calling_for(ProjectId::generate());
        query_thread(query_id, asker, move || {
            let (who, provider) = ctx.provider(&binding)?;
            Ok(Message::RemoteLanes {
                query_id,
                lanes: provider.lanes(&who, &binding)?,
                facets: provider.facets(&who, &binding)?,
            })
        });
    }

    /// **Run the filter** (`R7`). Read-only, and emphatically not a syntax check.
    pub fn test(&self, query_id: TaskSrcQueryId, binding: Binding, asker: Mailbox) {
        let ctx = self.calling_for(ProjectId::generate());
        query_thread(query_id, asker, move || {
            let (who, provider) = ctx.provider(&binding)?;
            let items = provider.query(&who, &binding)?;
            Ok(Message::TaskSourceTest {
                query_id,
                count: items.len(),
                sample: items.into_iter().take(SAMPLE).collect(),
            })
        });
    }

    /// Everything the filter currently offers, with what is already linked marked so the dialog
    /// says so rather than offering a second copy of a task that exists.
    pub fn items(
        &self,
        query_id: TaskSrcQueryId,
        project_id: ProjectId,
        binding: Binding,
        asker: Mailbox,
    ) {
        let ctx = self.calling_for(project_id);
        let path = store::path(&self.dirs, project_id);
        query_thread(query_id, asker, move || {
            let (who, provider) = ctx.provider(&binding)?;
            let items = provider.query(&who, &binding)?;
            let file = store::load(&path);
            let linked: Vec<RemoteItemId> = items
                .iter()
                .filter(|item| file.links.iter().any(|row| row.item == item.id))
                .map(|item| item.id.clone())
                .collect();
            Ok(Message::RemoteItems {
                query_id,
                items,
                linked,
            })
        });
    }

    /// Create a task per picked item and link it. **Import is explicit** (`R12`).
    pub fn import(&self, project_id: ProjectId, items: Vec<RemoteItemId>, everyone: Mailbox) {
        spawn(
            "tasksrc-import",
            self.calling_for(project_id),
            everyone,
            move |ctx| {
                let binding = ctx.binding()?;
                let (who, provider) = ctx.provider(&binding)?;
                // `fetch`, not `query`: the user picked these, and a pick is not the filter's to
                // second-guess between the listing and the click.
                let fetched = provider.fetch(&who, &binding, &items)?;
                let held = hold(&ctx.gate);
                let mut file = store::load(&ctx.path);
                let mut pass = Pass::default();
                let run = ctx.reconcile(provider, &who);
                let now = Utc::now();
                for item in &fetched {
                    if file.links.iter().any(|row| row.item == item.id) {
                        continue;
                    }
                    run.import(&binding, &mut file, item, now, &mut pass);
                }
                let saved = store::save(&ctx.path, &file);
                drop(held);
                saved?;
                Ok(ctx.announce(pass, &file))
            },
        );
    }

    /// Run a pass now rather than at the next interval — the board's force button and a card's.
    pub fn sync(&self, project_id: ProjectId, scope: SyncScope, everyone: Mailbox) {
        everyone.send(Message::TaskSourceState {
            project_id,
            state: SyncState::Running,
            last_sync: None,
            drifted: Vec::new(),
            error: None,
        });
        spawn(
            "tasksrc-force",
            self.calling_for(project_id),
            everyone,
            move |ctx| {
                let binding = ctx.binding()?;
                let (who, provider) = ctx.provider(&binding)?;
                let run = ctx.reconcile(provider, &who);
                let now = Utc::now();
                // The provider call is made with the gate **not** held: it is the whole of the pass's
                // latency, and the gate exists to order file writes, not round trips.
                let items = match scope {
                    SyncScope::All => Some(provider.query(&who, &binding)?),
                    SyncScope::Task(_) => None,
                };
                let held = hold(&ctx.gate);
                let mut file = store::load(&ctx.path);
                let pass = match scope {
                    SyncScope::All => {
                        run.reconcile(&binding, &mut file, &items.unwrap_or_default(), now)
                    }
                    SyncScope::Task(task) => run.one(&binding, &mut file, task, now)?,
                };
                let saved = store::save(&ctx.path, &file);
                drop(held);
                saved?;
                Ok(ctx.announce(pass, &file))
            },
        );
    }

    /// Settle one drifted task by hand, whatever the binding's switch says (`D188`).
    pub fn resolve(
        &self,
        project_id: ProjectId,
        task_id: TaskId,
        side: DriftSide,
        everyone: Mailbox,
    ) {
        spawn(
            "tasksrc-resolve",
            self.calling_for(project_id),
            everyone,
            move |ctx| {
                let binding = ctx.binding()?;
                let (who, provider) = ctx.provider(&binding)?;
                let held = hold(&ctx.gate);
                let mut file = store::load(&ctx.path);
                let pass = ctx.reconcile(provider, &who).resolve(
                    &binding,
                    &mut file,
                    task_id,
                    side,
                    Utc::now(),
                )?;
                let saved = store::save(&ctx.path, &file);
                drop(held);
                saved?;
                Ok(ctx.announce(pass, &file))
            },
        );
    }

    // ── the pieces every arm above is built out of ──────────────────

    fn read(&self, project: ProjectId) -> TasksrcFile {
        store::load(&store::path(&self.dirs, project))
    }

    fn calling_for(&self, project: ProjectId) -> Calling {
        Calling {
            registry: self.registry.clone(),
            settings: self.settings.clone(),
            connections: self.connections.clone(),
            work: self.work.clone(),
            gate: self.gate.clone(),
            path: store::path(&self.dirs, project),
            project,
        }
    }
}

/// What one call needs, gathered so a thread can own it.
///
/// A struct rather than six captured clones because every arm above needs the same six, and a
/// closure that captures them one at a time is six lines of noise per arm.
pub struct Calling {
    registry: Arc<Registry>,
    settings: Arc<Settings>,
    connections: Arc<Store>,
    work: crate::work::Handle,
    gate: Arc<Mutex<()>>,
    path: std::path::PathBuf,
    project: ProjectId,
}

impl Calling {
    /// The provider a binding names, and the identity to call it as.
    ///
    /// Both can fail for reasons a person can act on — a build with no such provider, a connection
    /// whose secret has gone from the store — so both are errors with sentences rather than
    /// `None`s the caller has to invent a message for.
    fn provider(&self, binding: &Binding) -> Result<(Identity, &dyn TaskProvider)> {
        let provider = self.registry.get(&binding.provider).with_context(|| {
            format!("no task provider called {} in this build", binding.provider)
        })?;
        let who = identity(&self.settings, &self.connections, binding.connection)?;
        Ok((who, provider))
    }

    fn binding(&self) -> Result<Binding> {
        store::load(&self.path)
            .only_binding()
            .cloned()
            .context("this project is not bound to a board")
    }

    fn reconcile<'a>(&'a self, provider: &'a dyn TaskProvider, who: &'a Identity) -> Reconcile<'a> {
        Reconcile {
            provider,
            who,
            work: &self.work,
            project: self.project,
        }
    }

    /// What a finished pass tells every window: the task changes, then each link row it rewrote,
    /// then the binding's own state.
    ///
    /// In that order on purpose — a badge that arrives before the task it is on has nothing to
    /// draw against, and the status item's count is the summary of everything above it.
    fn announce(&self, pass: Pass, file: &TasksrcFile) -> Vec<Message> {
        let mut out = pass.messages;
        for row in pass.links {
            out.push(Message::TaskLinkChanged {
                project_id: self.project,
                task_id: row.task,
                link: Some(Box::new(row)),
            });
        }
        for task_id in pass.unlinked {
            if file.link(task_id).is_none() {
                out.push(Message::TaskLinkChanged {
                    project_id: self.project,
                    task_id,
                    link: None,
                });
            }
        }
        out.push(state_message(self.project, file, Some(Utc::now())));
        out
    }
}

/// Take the sidecar gate, surviving a poisoning.
///
/// A panic in one pass must not stop every later pass from writing: the guarded resource is a file
/// this code re-reads from disk every time, so there is no invariant a poisoned lock is protecting.
fn hold(gate: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    gate.lock().unwrap_or_else(|held| held.into_inner())
}

/// A listing on a thread of its own: one message back to the asker, whichever way it went.
fn query_thread(
    query_id: TaskSrcQueryId,
    asker: Mailbox,
    run: impl FnOnce() -> Result<Message> + Send + 'static,
) {
    let refusal = asker.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("tasksrc-query".to_string())
        .spawn(move || {
            let message = run().unwrap_or_else(|error| Message::TaskSourceError {
                query_id,
                error: format!("{error:#}"),
            });
            asker.send(message);
        })
    {
        // A listing that never started still has to answer: the interface holds the query id and
        // draws a spinner against it, and a spinner nothing ever replaces is worse than a refusal.
        tracing::warn!("a task-source listing did not start: {error}");
        refusal.send(Message::TaskSourceError {
            query_id,
            error: "this listing could not be started".to_string(),
        });
    }
}

fn spawn(
    name: &'static str,
    ctx: Calling,
    everyone: Mailbox,
    run: impl FnOnce(&Calling) -> Result<Vec<Message>> + Send + 'static,
) {
    let project = ctx.project;
    if let Err(error) = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || match run(&ctx) {
            Ok(messages) => {
                for message in messages {
                    everyone.send(message);
                }
            }
            Err(error) => {
                tracing::warn!("a task-source pass failed: {error:#}");
                let file = store::load(&ctx.path);
                everyone.send(failed(project, &file, format!("{error:#}")));
            }
        })
    {
        tracing::warn!("a task-source pass did not start: {error}");
    }
}

/// The answer to `GetTaskSource` and to `SetTaskSource`.
fn source_message(project_id: ProjectId, file: &TasksrcFile) -> Message {
    Message::TaskSource {
        project_id,
        binding: file.only_binding().cloned().map(Box::new),
        state: binding_state(file),
    }
}

/// The board's status item, built from the link table rather than from anything remembered: the
/// file is the only thing that survives a restart, so it is the only honest source.
fn state_message(
    project_id: ProjectId,
    file: &TasksrcFile,
    last_sync: Option<chrono::DateTime<Utc>>,
) -> Message {
    Message::TaskSourceState {
        project_id,
        state: binding_state(file),
        last_sync: last_sync.or_else(|| file.links.iter().map(|row| row.synced_at).max()),
        drifted: drifted(file),
        error: None,
    }
}

fn failed(project_id: ProjectId, file: &TasksrcFile, error: String) -> Message {
    Message::TaskSourceState {
        project_id,
        state: SyncState::Failed,
        last_sync: file.links.iter().map(|row| row.synced_at).max(),
        drifted: drifted(file),
        error: Some(error),
    }
}

fn binding_state(file: &TasksrcFile) -> SyncState {
    if file.only_binding().is_some() {
        SyncState::Idle
    } else {
        SyncState::Unbound
    }
}

/// Every task whose two sides differ, for the status item's count and the drift overview's list.
///
/// Read off the link rows, which is where drift lives (`TaskLink::drift`) — nothing here keeps a
/// second copy that could disagree with the file.
fn drifted(file: &TasksrcFile) -> Vec<TaskId> {
    file.links
        .iter()
        .filter(|row: &&TaskLink| !row.drift.is_empty())
        .map(|row| row.task)
        .collect()
}
