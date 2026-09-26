//! The pass: what a bound board and a project's tasks do to each other, and the thread that runs
//! it.
//!
//! One named thread for the whole host, not one per binding. It walks the bound projects, ticks
//! each on its own interval, and finishes one binding's pass before starting another — which is
//! also the whole of "a pass never overlaps itself" (`R14`). It holds a [`crate::work::Handle`]
//! clone and writes through [`crate::work::Work`] exactly as an MCP tool does, so every window
//! redraws and a pulled task is an ordinary task from the moment it lands (`D120`, `R2`).
//!
//! **The lock is taken for one method and never across a network call.** The provider is asked
//! first, with nothing held; every write afterwards is one [`crate::work::Work`] method under the
//! lock and released before the message is posted. That rule is the entire mitigation for
//! `D120`'s stated cost.
//!
//! Two behaviours here are rules of the **layer**, not of any provider, and both are pinned by a
//! test of their own:
//!
//! - **An unmapped lane parks the task.** A remote lane the binding's map does not name leaves the
//!   task exactly where it is and files the link row as [`LinkState::Parked`], which is the badge
//!   a card draws. A wrong guess would move somebody's card silently, and that is the failure this
//!   exists to prevent — the settled Trello map binds three lanes and leaves four unbound on
//!   purpose.
//! - **An item that leaves the filter keeps its task, its content and its history, and loses only
//!   its link** (`R9`). The row becomes [`LinkState::Unlinked`] and nothing else happens.
//!   **Nothing in this module deletes anything** — there is no call to
//!   [`crate::work::Work::delete`] here, and there never should be. "No longer in the fetch" is
//!   not "gone".
//!
//! The **full** pass reads the filter through [`TaskProvider::query`], never
//! [`TaskProvider::fetch`]: the filter is what decides which items are the binding's, and an item
//! outside it is one the user narrowed away. [`Reconcile::one`] is the other shape — a deliberate
//! single-item sync, which reads through `fetch` precisely so that a task the user has since
//! narrowed the filter past can still be synced on purpose.
//!
//! **Both directions, since `D188`.** [`super::outbound`] holds the conflict table and everything
//! that decides; this module is where it is executed. A field only one side changed goes that way;
//! a field both sides changed is settled by the binding's [`Authority`](super::Authority), the
//! loser's value is kept in a comment, and the task is filed [`LinkState::Drifted`] until somebody
//! looks. A push the provider refuses is [`LinkState::Conflict`] and is **never** retried by
//! overwriting. A [`Direction::Pull`] binding still decides every field and still records the
//! divergence; it simply sends nothing (`R13`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::{ConnectionId, ProjectId, TaskId};
use ubiq_proto::messages::{Message, TaskField};
use ubiq_proto::tasksrc::{DriftSide, FieldDrift, RemoteItem};
use ubiq_proto::work::{CommentAuthor, Label, TaskRecord};

use super::outbound::{self, Decision};
use super::store::{self, TasksrcFile};
use super::{Binding, Direction, Identity, LinkState, Registry, SyncState, TaskLink, TaskProvider};
use crate::connectors::store::Store;
use crate::reply::Reply;
use crate::settings::Settings;
use crate::store::project_dir::ProjectDirs;
use crate::work::Work;

// ── the per-field hash keys ──────────────────────────────────────────

/// The fields a pass compares, one hash each in [`TaskLink::hash`].
///
/// Per field rather than per record because the question a pass asks is *which field* the remote
/// changed: a whole-record hash can only answer "something did", and pulling every field because
/// one moved is how a local edit is lost.
pub const FIELD_TITLE: &str = "title";
pub const FIELD_BODY: &str = "body";
pub const FIELD_LANE: &str = "lane";
pub const FIELD_LABELS: &str = "labels";
pub const FIELD_ASSIGNEES: &str = "assignees";
pub const FIELD_KEY: &str = "key";
pub const FIELD_URL: &str = "url";
/// The **mapped** kind, not the hint: a change to the binding's kind map has to pull too, and
/// hashing the answer rather than the input is what makes that true for nothing.
pub const FIELD_KIND: &str = "kind";
/// The mapped priority, on [`FIELD_KIND`]'s terms.
pub const FIELD_PRIORITY: &str = "priority";

/// What one item's read hashes to, field by field.
fn hashes(binding: &Binding, item: &RemoteItem) -> BTreeMap<String, String> {
    BTreeMap::from([
        (FIELD_TITLE.to_string(), digest(&item.title)),
        (FIELD_BODY.to_string(), digest(&item.body)),
        (FIELD_LANE.to_string(), digest(item.lane.as_str())),
        (
            FIELD_LABELS.to_string(),
            digest(&item.labels.join("\u{1f}")),
        ),
        (
            FIELD_ASSIGNEES.to_string(),
            digest(&item.assignees.join("\u{1f}")),
        ),
        (
            FIELD_KEY.to_string(),
            digest(item.key.as_deref().unwrap_or_default()),
        ),
        (FIELD_URL.to_string(), digest(&item.url)),
        (
            FIELD_KIND.to_string(),
            digest(&format!("{:?}", binding.kind_of(item))),
        ),
        (
            FIELD_PRIORITY.to_string(),
            digest(&format!("{:?}", binding.priority_of(item))),
        ),
    ])
}

/// A short digest of one field's value.
///
/// **Not a cryptographic hash and not meant to be.** Equal or not equal is the only question ever
/// asked of it, the inputs are a tracker's own strings, and nothing here is a security boundary —
/// the same reasoning [`ubiq_proto::tasksrc::Revision`] is opaque for.
///
/// Public so [`super::outbound`] hashes the task's side with the same function. Two digests of one
/// value have to be the same digest or the dirty test compares nothing.
pub fn digest(value: &str) -> String {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(value.as_bytes());
    format!("{:016x}", hasher.finish())
}

// ── what a pass did ──────────────────────────────────────────────────

/// What one inbound pass changed, for the caller to report and for a test to assert on.
///
/// Four disjoint lists and the messages they produced. A task can be in both [`Self::created`] and
/// [`Self::parked`] — a card imported out of an unmapped lane is exactly that — but never in both
/// [`Self::created`] and [`Self::updated`].
#[derive(Debug, Default)]
pub struct Pass {
    /// Tasks this pass made, which happens only where the binding auto-imports (`R12`).
    pub created: Vec<TaskId>,
    /// Tasks a field of which the remote had changed.
    pub updated: Vec<TaskId>,
    /// Tasks whose remote lane the binding's map does not name. Left where they are, with the link
    /// row filed as [`LinkState::Parked`].
    pub parked: Vec<TaskId>,
    /// Tasks whose item is no longer under the filter. **The task is untouched** — only its link
    /// row moves to [`LinkState::Unlinked`] (`R9`).
    pub unlinked: Vec<TaskId>,
    /// Tasks whose local value was written to the remote item (`D188`).
    pub pushed: Vec<TaskId>,
    /// Tasks whose two sides differ, or differed and were settled by the authority switch. What
    /// each one differs *in* is on its link row — [`TaskLink::drift`].
    pub drifted: Vec<TaskId>,
    /// Tasks whose push the provider refused because somebody wrote between the read and the
    /// write. **Nothing was overwritten on either side** and nothing is retried (`R8`).
    pub conflicted: Vec<TaskId>,
    /// Every link row this pass rewrote, for the `TaskLinkChanged` the caller posts.
    pub links: Vec<TaskLink>,
    /// What every window should be told: the same `TaskCreated` / `TaskChanged` a click produces.
    /// Posted by the caller, after the lock is gone.
    pub messages: Vec<Message>,
}

impl Pass {
    /// Whether anything at all happened, which is what decides whether the sidecar is rewritten.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty()
            && self.updated.is_empty()
            && self.parked.is_empty()
            && self.unlinked.is_empty()
            && self.pushed.is_empty()
            && self.drifted.is_empty()
            && self.conflicted.is_empty()
    }
}

/// What one item's pass did to one task.
struct Applied {
    messages: Vec<Message>,
    /// Whether the remote lane is one the binding's map does not name.
    parked: bool,
    /// Whether a local value reached the remote.
    pushed: bool,
    /// Whether the two sides differ, or differed and were settled.
    drifted: bool,
    /// Whether the provider refused the write.
    conflicted: bool,
}

// ── the pass itself ──────────────────────────────────────────────────

/// One binding's pass, against one project's board — **both directions** since `D188`.
///
/// Borrowed rather than owned so the thread builds one per tick and a test builds one over a
/// provider serving fixtures — which is the only way the filter is exercised by its *effect*
/// rather than by its configuration.
///
/// Named for what it does rather than for which way the work travels: `D185` called it `Inbound`
/// when pulling was all it did, and a type that now pushes, decides a conflict table and files
/// drift cannot keep that name and stay readable.
pub struct Reconcile<'a> {
    pub provider: &'a dyn TaskProvider,
    pub who: &'a Identity,
    pub work: &'a crate::work::Handle,
    pub project: ProjectId,
}

impl Reconcile<'_> {
    /// Fetch under the filter, then reconcile what came back against the link table.
    ///
    /// The provider call happens first and with no lock held (`R2`).
    pub fn run(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        now: DateTime<Utc>,
    ) -> Result<Pass> {
        let items = self
            .provider
            .query(self.who, binding)
            .with_context(|| format!("the {} board could not be read", binding.provider))?;
        Ok(self.reconcile(binding, file, &items, now))
    }

    /// What the fetched set means for the project's tasks and the binding's link rows.
    ///
    /// Split from [`Self::run`] because what a pass *decides* is a pure function of what came
    /// back, and because a caller that already has the items — an explicit import — needs this
    /// half and not the round trip.
    pub fn reconcile(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        items: &[RemoteItem],
        now: DateTime<Utc>,
    ) -> Pass {
        let mut pass = Pass::default();
        let tasks = {
            let mut work = self.work.lock();
            let (replies, tasks) = work.tasks(self.project);
            warn_only(&replies);
            tasks
        };

        // `R1`'s stated cost: a task deleted outside the layer leaves an orphan row, so the worker
        // prunes on load. Only this side can see the task list, which is why the store cannot.
        let present: HashSet<TaskId> = tasks.iter().map(|task| task.id).collect();
        file.links
            .retain(|row| row.binding != binding.id || present.contains(&row.task));

        let linked: HashMap<String, TaskId> = file
            .links
            .iter()
            .filter(|row| row.binding == binding.id)
            .map(|row| (row.item.to_string(), row.task))
            .collect();

        for item in items {
            match linked.get(item.id.as_str()) {
                Some(task) => {
                    let Some(record) = tasks.iter().find(|held| held.id == *task) else {
                        continue;
                    };
                    let applied = self.apply(binding, file, item, record, now);
                    if !applied.messages.is_empty() {
                        pass.updated.push(record.id);
                    }
                    record_applied(&mut pass, file, record.id, applied);
                }
                // Import is explicit, and the filter governs what is *offered* rather than what is
                // created — unless the binding says otherwise, which is the one switch `R12`
                // leaves for the user who wants the whole board on the board.
                None if binding.auto_import => self.import(binding, file, item, now, &mut pass),
                None => {}
            }
        }

        // `R9`. An item that left the filter keeps its task, its content and its history, and
        // loses only its link. Nothing below deletes anything, and nothing below touches the task.
        let fetched: HashSet<&str> = items.iter().map(|item| item.id.as_str()).collect();
        for row in file
            .links
            .iter_mut()
            .filter(|row| row.binding == binding.id)
        {
            if fetched.contains(row.item.as_str()) || row.state == LinkState::Unlinked {
                continue;
            }
            row.state = LinkState::Unlinked;
            row.synced_at = now;
            pass.unlinked.push(row.task);
        }

        pass
    }

    /// Make a task for an item nothing is linked to, and fill it from that item.
    ///
    /// Public so an explicit import — the dialog's picks — is the same code path as the
    /// auto-import switch, rather than a second one that can disagree with it.
    pub fn import(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        item: &RemoteItem,
        now: DateTime<Utc>,
        pass: &mut Pass,
    ) {
        let project = self.project;
        let title = item.title.clone();
        let messages = mutate(self.work, |work| work.create(project, title, None));
        let Some(record) = messages.iter().find_map(|message| match message {
            Message::TaskCreated { task, .. } => Some(task.clone()),
            _ => None,
        }) else {
            return;
        };
        pass.created.push(record.id);
        pass.messages.extend(messages);

        let applied = self.apply(binding, file, item, &record, now);
        record_applied(pass, file, record.id, applied);
    }
    /// One task's own pass, whatever the filter says — the card's *Sync now* and the board's
    /// force button over a single scope.
    ///
    /// **Reads through [`TaskProvider::fetch`] rather than [`TaskProvider::query`]**, which is the
    /// whole difference: a task the user has since narrowed the filter past is still linked, and a
    /// deliberate single-item sync has to reach it. Nothing here unlinks against a filter — a
    /// filter's opinion is the full pass's business — but an item the remote no longer has is
    /// still an unlink, because that is not the filter talking.
    pub fn one(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        task: TaskId,
        now: DateTime<Utc>,
    ) -> Result<Pass> {
        self.one_forcing(binding, file, task, None, now)
    }

    /// Settle one drifted task by hand, one way or the other (`ResolveTaskDrift`, `D188`).
    ///
    /// **The switch is overridden, not consulted.** This is somebody looking at the two values and
    /// choosing, which is the case the drift overview exists for — so `Push` writes the task's
    /// values to the remote and `Pull` writes the item's to the task, whatever [`Authority`] says.
    ///
    /// **Only the fields already on the row's drift list move.** A resolution answers a divergence
    /// a person is looking at; it is not a blanket overwrite of one side by the other, and a field
    /// nobody was shown is not a field anybody chose.
    ///
    /// A forced push is made even on a [`Direction::Pull`] binding. `R13` exists to stop a first
    /// binding writing to somebody's board *silently*, and a button pressed beside the two values
    /// is the opposite of silent.
    pub fn resolve(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        task: TaskId,
        side: DriftSide,
        now: DateTime<Utc>,
    ) -> Result<Pass> {
        let fields: Vec<String> = file
            .link(task)
            .map(|row| row.drift.iter().map(|d| d.field.clone()).collect())
            .unwrap_or_default();
        if fields.is_empty() {
            return Ok(Pass::default());
        }
        let mut binding = binding.clone();
        if side == DriftSide::Push {
            binding.direction = Direction::TwoWay;
        }
        self.one_forcing(&binding, file, task, Some((side, fields)), now)
    }

    fn one_forcing(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        task: TaskId,
        force: Option<(DriftSide, Vec<String>)>,
        now: DateTime<Utc>,
    ) -> Result<Pass> {
        let mut pass = Pass::default();
        let Some(row) = file.link(task).cloned() else {
            return Ok(pass);
        };
        let items = self
            .provider
            .fetch(self.who, binding, std::slice::from_ref(&row.item))
            .with_context(|| format!("the {} item could not be read", binding.provider))?;
        let Some(item) = items.into_iter().find(|held| held.id == row.item) else {
            // Gone from the remote. `R9`: the task keeps everything it had and loses its link.
            if let Some(held) = file.links.iter_mut().find(|held| held.task == task) {
                held.state = LinkState::Unlinked;
                held.synced_at = now;
                pass.unlinked.push(task);
                pass.links.push(held.clone());
            }
            return Ok(pass);
        };
        let record = {
            let mut work = self.work.lock();
            let (replies, tasks) = work.tasks(self.project);
            warn_only(&replies);
            tasks.into_iter().find(|held| held.id == task)
        };
        let Some(record) = record else {
            return Ok(pass);
        };
        let applied = self.apply_forcing(binding, file, &item, &record, force.as_ref(), now);
        if !applied.messages.is_empty() {
            pass.updated.push(task);
        }
        record_applied(&mut pass, file, task, applied);
        Ok(pass)
    }

    /// Reconcile one item against one task, with the table deciding every field.
    fn apply(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        item: &RemoteItem,
        record: &TaskRecord,
        now: DateTime<Utc>,
    ) -> Applied {
        self.apply_forcing(binding, file, item, record, None, now)
    }

    /// Reconcile one item against one task: decide every field, push what the task changed, pull
    /// what the remote changed, and file what this pass saw on both sides.
    ///
    /// **This is `D188`'s conflict table, executed.** [`super::outbound::decide`] is where the
    /// table itself is written down; everything here is the consequence — the patch, the round
    /// trip, the local writes and the row.
    ///
    /// `force` is a hand resolution: those fields take that side whatever the table says, and
    /// every other field is decided as usual. It is a parameter rather than a second pass because
    /// two write paths over one provider is two places for the same rule to be written differently.
    ///
    /// The order is forced by the one rule the whole layer rests on: **the lock is taken for one
    /// method and never across a network call**. So the push goes first, with nothing held, and
    /// every local write happens afterwards.
    fn apply_forcing(
        &self,
        binding: &Binding,
        file: &mut TasksrcFile,
        item: &RemoteItem,
        record: &TaskRecord,
        force: Option<&(DriftSide, Vec<String>)>,
        now: DateTime<Utc>,
    ) -> Applied {
        let caps = self.provider.caps();
        let stored = file
            .link(record.id)
            .map(|row| (row.hash.clone(), row.local.clone()))
            .unwrap_or_default();
        let (stored_remote, stored_local) = &stored;
        let fresh_remote = hashes(binding, item);
        let fresh_local = outbound::local_hashes(record);

        // A row written before outbound existed carries no local hashes at all. That reads as
        // *the task has not changed*, which pulls — exactly what `D185` did, and the safe answer:
        // the alternative would declare every linked task drifted the first time this build runs.
        let knows_local = !stored_local.is_empty();

        let mut decisions: Vec<(&'static str, Decision)> = Vec::new();
        for field in outbound::FIELDS {
            let remote_changed = stored_remote.get(*field) != fresh_remote.get(*field);
            let local_changed = knows_local && stored_local.get(*field) != fresh_local.get(*field);
            let decision = match force {
                // A hand resolution overrides the table for the fields somebody was shown, and
                // for those only.
                Some((side, forced)) if forced.iter().any(|held| held == field) => match side {
                    DriftSide::Push => Decision::Push,
                    DriftSide::Pull => Decision::Pull,
                },
                _ => outbound::decide(local_changed, remote_changed, binding.authority),
            };
            if decision != Decision::Nothing {
                decisions.push((field, decision));
            }
        }

        // ── the push, first, with no lock held ──────────────────────
        //
        // `R13`: a `Pull` binding never sends anything. The fields are still decided above and
        // still recorded below, so turning the switch to two-way pushes them on the next pass
        // rather than having quietly forgotten them.
        let wanted: Vec<&str> = decisions
            .iter()
            .filter(|(_, decision)| matches!(decision.side(), Some(DriftSide::Push)))
            .map(|(field, _)| *field)
            .collect();
        let planned = outbound::push_patch(binding, &caps, record, &wanted);
        let mut pushed = false;
        let mut conflicted = false;
        // **Before any write, fetch.** `item` *is* that read — a pass has just queried and a
        // single-item sync has just fetched — so the revision handed over as the precondition is
        // the one this decision was made against. Where the provider honours no precondition it
        // re-reads and compares instead, which narrows the window to a round trip and does not
        // close it (`R8`), and the settings surface says exactly that beside the switch.
        let seen = if binding.writes() && !planned.patch.is_empty() {
            match self
                .provider
                .update(self.who, binding, &item.id, &planned.patch, &item.revision)
            {
                Ok(updated) => {
                    pushed = true;
                    updated
                }
                Err(error) => {
                    // Never retried by overwriting. Nothing was written on either side, the task
                    // keeps every byte it had, and a human decides.
                    tracing::warn!("a task-source push was refused: {error:#}");
                    conflicted = true;
                    item.clone()
                }
            }
        } else {
            item.clone()
        };
        let fresh_remote = if pushed {
            hashes(binding, &seen)
        } else {
            fresh_remote
        };

        // ── the drift rows, and the comment the loser's value goes in ──
        let mut drift: Vec<FieldDrift> = Vec::new();
        let mut losers: Vec<String> = Vec::new();
        for (field, decision) in &decisions {
            let row = outbound::row(binding, &caps, record, item, field, *decision);
            let stands = match decision {
                // Both sides moved: the switch settled it, and the row is the record of what was
                // replaced — which is what makes the force buttons safe to press.
                Decision::Both(side) => {
                    losers.push(format!(
                        "Both sides changed `{field}`. {} The value replaced was:\n\n{}",
                        match side {
                            DriftSide::Pull => "The remote won.",
                            DriftSide::Push => "Ubiq won.",
                        },
                        match side {
                            DriftSide::Pull => row.local.clone(),
                            DriftSide::Push => row.remote.clone(),
                        }
                    ));
                    true
                }
                // A push that was made is resolved, not drift. One that could not be made *by a
                // binding that writes* is a standing divergence and says so rather than vanishing.
                //
                // A pull-only binding records nothing here on purpose: a local edit is the user's
                // to keep, nothing will ever be done about it, and flagging it would leave every
                // edited card on every pull-only board permanently drifted — which is the dot
                // nobody reads. It still counts as a local change, so if the remote later moves
                // that same field the table reaches its fourth row and the switch decides.
                Decision::Push => binding.writes() && (row.settles.is_none() || !pushed),
                _ => false,
            };
            if stands {
                drift.push(row);
            }
        }

        // ── the pulls, under the lock, one method at a time ──────────
        let project = self.project;
        let task = record.id;
        let mut messages = Vec::new();
        let pulls: Vec<&str> = decisions
            .iter()
            .filter(|(_, decision)| matches!(decision.side(), Some(DriftSide::Pull)))
            .map(|(field, _)| *field)
            .collect();
        let pulling = |field: &str| pulls.contains(&field);

        // The title, the description and the priority travel together because `Work::update` is
        // one method over the three, and `None` there already means "leave alone".
        if pulling(FIELD_TITLE) || pulling(FIELD_BODY) || pulling(FIELD_PRIORITY) {
            let title = pulling(FIELD_TITLE).then(|| seen.title.clone());
            let body = pulling(FIELD_BODY).then(|| seen.body.clone());
            let priority = pulling(FIELD_PRIORITY)
                .then(|| binding.priority_of(&seen))
                .flatten();
            messages.extend(mutate(self.work, |work| {
                work.update(project, task, title, body, priority)
            }));
        }

        // `R11`: `key` and `link` are written from the remote and are **not** the sync identity.
        // The link table is. A user who clears either has not unlinked the task.
        if pulling(FIELD_KEY)
            && let Some(key) = seen.key.clone()
        {
            messages.extend(set(self.work, project, task, TaskField::Key(Some(key))));
        }
        if pulling(FIELD_URL) {
            messages.extend(set(
                self.work,
                project,
                task,
                TaskField::Link(Some(seen.url.clone())),
            ));
        }
        if pulling(FIELD_LABELS) {
            // A label list is edited as a set on both sides, so the remote's set replaces the
            // local one whole. Colour is the interface's alone and is never carried (`D113`).
            //
            // **That is a real loss and `D185` records it as one**: a label added here and
            // nowhere else does not survive a pass in which the remote's labels moved. The
            // per-field conflict table narrows it — the set is only replaced where the *remote*
            // changed it — and does not end it, because a set has no per-element history to
            // reconcile against. `G366`.
            let labels = seen
                .labels
                .iter()
                .map(|name| Label::new(name.clone(), 0))
                .collect();
            messages.extend(set(self.work, project, task, TaskField::Labels(labels)));
        }
        if pulling(FIELD_ASSIGNEES) {
            // Free text on both sides, and one name: a task has one `assigned_to` where a tracker
            // may have several, so the first is the one that lands and the rest are not invented
            // a field for.
            let who = seen.assignees.first().cloned();
            messages.extend(set(self.work, project, task, TaskField::AssignedTo(who)));
        }
        if pulling(FIELD_KIND)
            && let Some(kind) = binding.kind_of(&seen)
        {
            messages.extend(set(self.work, project, task, TaskField::Kind(Some(kind))));
        }

        // The lane, which is the one that parks.
        let mapped = binding.status_of(&seen.lane);
        let parked = mapped.is_none();
        if let Some(status) = mapped
            && pulling(FIELD_LANE)
            && status != record.status
        {
            messages.extend(mutate(self.work, |work| {
                work.move_task(project, task, status, None)
            }));
        }

        // The loser's value, kept where a person will find it. Posted after every field write, so
        // the comment sits below the change it explains.
        for text in losers {
            messages.extend(mutate(self.work, |work| {
                work.add_comment(
                    project,
                    task,
                    CommentAuthor::Agent,
                    format!("Task sync: {text}"),
                )
            }));
        }

        // ── the row ─────────────────────────────────────────────────
        //
        // The local hashes are re-taken from the task **as it now is**, which is the task plus
        // every pull just made. Re-using the pre-pull snapshot would declare the task changed on
        // the next pass against a change this pass made itself.
        let after = {
            let mut work = self.work.lock();
            let (replies, tasks) = work.tasks(project);
            warn_only(&replies);
            tasks.into_iter().find(|held| held.id == task)
        };
        let local = outbound::local_hashes(after.as_ref().unwrap_or(record));

        let mut row = file.link(task).cloned().unwrap_or(TaskLink {
            binding: binding.id,
            task,
            item: seen.id.clone(),
            container: binding.container.clone(),
            revision: seen.revision.clone(),
            hash: BTreeMap::new(),
            local: BTreeMap::new(),
            drift: Vec::new(),
            synced_at: now,
            state: LinkState::Linked,
        });
        row.binding = binding.id;
        row.item = seen.id.clone();
        row.container = binding.container.clone();
        // **Only where nothing was refused.** A conflicted push means the remote moved under us,
        // so the revision this pass read is already stale; keeping the stored one makes the next
        // pass re-read rather than believe a token it never saw honoured.
        if !conflicted {
            row.revision = seen.revision.clone();
        }
        row.hash = fresh_remote;
        // **A refused push leaves the local side reading as changed**, on the revision's own
        // reasoning above: the edit has not reached the remote, so the next pass must still see it
        // as pending and try again against a fresh read. That is not "retrying by overwriting" —
        // the precondition is honoured every time, and the write only lands if nobody else has
        // written since. Re-taking the hashes here would quietly forget the edit instead.
        row.local = if conflicted {
            stored_local.clone()
        } else {
            local
        };
        let drifted = !drift.is_empty();
        row.drift = drift;
        if parked {
            // The lane's hash is deliberately **not** kept for a parked item: the next pass has to
            // re-ask, so binding that lane later moves the task at once rather than waiting for
            // the remote to touch it.
            row.hash.remove(FIELD_LANE);
        }
        row.synced_at = now;
        // Conflict first because it is the one a person must act on; drift next because it is a
        // divergence in content; parked last because it is a gap in the lane map and the task is
        // otherwise in step.
        row.state = match (conflicted, drifted, parked) {
            (true, _, _) => LinkState::Conflict,
            (_, true, _) => LinkState::Drifted,
            (_, _, true) => LinkState::Parked,
            _ => LinkState::Linked,
        };
        file.put_link(row);

        Applied {
            messages,
            parked,
            pushed,
            drifted,
            conflicted,
        }
    }
}

/// File one task's outcome on the pass, and carry its link row out for `TaskLinkChanged`.
///
/// One place, so the four lists and the row can never be filled from three call sites in three
/// slightly different ways.
fn record_applied(pass: &mut Pass, file: &TasksrcFile, task: TaskId, applied: Applied) {
    pass.messages.extend(applied.messages);
    if applied.parked {
        pass.parked.push(task);
    }
    if applied.pushed {
        pass.pushed.push(task);
    }
    if applied.drifted {
        pass.drifted.push(task);
    }
    if applied.conflicted {
        pass.conflicted.push(task);
    }
    if let Some(row) = file.link(task) {
        pass.links.push(row.clone());
    }
}

/// One [`Work`] method under the lock, with the lock released before anything is said.
fn mutate(work: &crate::work::Handle, run: impl FnOnce(&mut Work) -> Vec<Reply>) -> Vec<Message> {
    let replies = {
        let mut held = work.lock();
        run(&mut held)
    };
    let mut out = Vec::new();
    for reply in replies {
        let message = reply.into_message();
        match &message {
            Message::WorkError { error, .. } => tracing::warn!("a task-source pass: {error}"),
            Message::TaskCreated { .. } | Message::TaskChanged { .. } => out.push(message),
            _ => {}
        }
    }
    out
}

fn set(
    work: &crate::work::Handle,
    project: ProjectId,
    task: TaskId,
    field: TaskField,
) -> Vec<Message> {
    mutate(work, |held| held.set_field(project, task, field))
}

fn warn_only(replies: &[Reply]) {
    for reply in replies {
        if let Message::WorkError { error, .. } = reply.message() {
            tracing::warn!("a task-source pass: {error}");
        }
    }
}

// ── the identity ─────────────────────────────────────────────────────

/// A connection, resolved into what one provider call needs.
///
/// `repos::identity`'s own shape and for its own reason — reading a token is a keychain call, so
/// it happens on this thread and never where keystrokes are carried. `G363` is the lift of the two
/// into the connector family.
pub fn identity(settings: &Settings, store: &Store, connection: ConnectionId) -> Result<Identity> {
    let host = settings.host();
    let record = host
        .connections
        .iter()
        .find(|held| held.id == connection)
        .cloned()
        .context("no such connection")?;
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

// ── the thread ───────────────────────────────────────────────────────

/// How often the worker wakes to see which bindings are due. Not the poll interval — that is per
/// binding and at least [`super::POLL_FLOOR`]; this is only the resolution the floor is measured
/// at.
const TICK: Duration = Duration::from_secs(10);

/// What the worker is built with. The [`crate::work::Handle`] is the coordinator's own, which is
/// the whole point (`D120`).
pub struct Job {
    pub work: crate::work::Handle,
    /// Where a pulled task's `TaskCreated` / `TaskChanged` goes. Every window, because there is no
    /// asking client.
    pub everyone: Mailbox,
    /// How a project's data directory is found — never composed, because a project may keep its
    /// data in its own folder (`D173`).
    pub dirs: ProjectDirs,
    pub registry: Arc<Registry>,
    pub settings: Arc<Settings>,
    pub connections: Arc<Store>,
    /// Held across every read-modify-write of a `tasksrc.toml`, and **shared with
    /// [`super::service::TaskSrc`]**: the poll and a force button pressed during it are two passes
    /// over one file, and without one gate the second overwrites the first's link table.
    pub gate: Arc<Mutex<()>>,
}

/// Which projects the worker walks. The coordinator owns the catalogue and this is the one fact it
/// hands over — the worker holds no `Projects` and cannot, because that never leaves its thread.
#[derive(Default)]
pub struct Roster {
    projects: Mutex<Vec<ProjectId>>,
}

impl Roster {
    fn read(&self) -> Vec<ProjectId> {
        self.projects
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default()
    }
}

/// The worker's handle. **Dropping it ends the thread** — `watch/`'s own shape, here by way of a
/// `Weak` on the roster rather than a channel, because the roster is the only thing the two share.
pub struct Sync {
    roster: Arc<Roster>,
}

impl Sync {
    /// Tell the worker which projects exist. Cheap and idempotent: the coordinator says it on the
    /// same tick it re-reads the task files, and a list that has not changed costs a comparison.
    pub fn watch(&self, projects: Vec<ProjectId>) {
        let Ok(mut held) = self.roster.projects.lock() else {
            return;
        };
        if *held != projects {
            *held = projects;
        }
    }
}

/// Start the sync worker.
///
/// A failure to spawn is said once and leaves a host whose boards do not sync, which is the same
/// posture the MCP listener takes: no feature is worth refusing to start the application for.
pub fn start(job: Job) -> Sync {
    let roster = Arc::new(Roster::default());
    let weak = Arc::downgrade(&roster);
    if let Err(error) = std::thread::Builder::new()
        .name("ubiq-tasksrc".to_string())
        .spawn(move || run(job, weak))
    {
        tracing::error!("the task-source sync worker did not start: {error}");
    }
    Sync { roster }
}

fn run(job: Job, roster: Weak<Roster>) {
    let mut due: HashMap<ProjectId, Instant> = HashMap::new();
    loop {
        std::thread::sleep(TICK);
        let Some(roster) = roster.upgrade() else {
            return;
        };
        let projects = roster.read();
        drop(roster);
        due.retain(|project, _| projects.contains(project));
        for project in projects {
            // One binding's whole pass before the next is started, on one thread: that is the
            // whole of "a pass never overlaps itself" (`R14`), with no flag to get wrong.
            tick(&job, project, &mut due);
        }
    }
}

fn tick(job: &Job, project: ProjectId, due: &mut HashMap<ProjectId, Instant>) {
    let path = store::path(&job.dirs, project);
    let mut file = store::load(&path);
    let Some(binding) = file.only_binding().cloned() else {
        due.remove(&project);
        return;
    };
    let now = Instant::now();
    if due.get(&project).is_some_and(|next| *next > now) {
        return;
    }
    // `R14`: the floor is applied on read and the value the user asked for is never rewritten.
    due.insert(project, now + Duration::from_secs(binding.poll_seconds()));

    let Some(provider) = job.registry.get(&binding.provider) else {
        // A binding written by a build that had a provider this one does not. It parks rather than
        // failing the project.
        tracing::warn!("no task provider called {} in this build", binding.provider);
        return;
    };
    let who = match identity(&job.settings, &job.connections, binding.connection) {
        Ok(who) => who,
        Err(error) => {
            tracing::warn!("a task source could not be authenticated: {error:#}");
            return;
        }
    };

    let run = Reconcile {
        provider,
        who: &who,
        work: &job.work,
        project,
    };
    // The round trip happens with the gate **not** held — it is the whole of a pass's latency,
    // and the gate orders file writes rather than network calls. The file is then re-read under
    // the gate, because a force button pressed during the fetch has written to it since.
    let items = match provider.query(&who, &binding) {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!("a task-source pass failed: {error:#}");
            job.everyone.send(Message::TaskSourceState {
                project_id: project,
                state: SyncState::Failed,
                last_sync: file.links.iter().map(|row| row.synced_at).max(),
                drifted: drifted(&file),
                error: Some(format!("{error:#}")),
            });
            return;
        }
    };
    let held = job
        .gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    file = store::load(&path);
    let pass = run.reconcile(&binding, &mut file, &items, Utc::now());
    if let Err(error) = store::save(&path, &file) {
        tracing::warn!("a task source could not be written: {error}");
    }
    drop(held);

    for message in pass.messages {
        job.everyone.send(message);
    }
    for row in pass.links {
        job.everyone.send(Message::TaskLinkChanged {
            project_id: project,
            task_id: row.task,
            link: Some(Box::new(row)),
        });
    }
    job.everyone.send(Message::TaskSourceState {
        project_id: project,
        state: SyncState::Idle,
        last_sync: Some(Utc::now()),
        drifted: drifted(&file),
        error: None,
    });
}

/// Every task whose two sides differ, for the status item's count.
fn drifted(file: &TasksrcFile) -> Vec<TaskId> {
    file.links
        .iter()
        .filter(|row| !row.drift.is_empty())
        .map(|row| row.task)
        .collect()
}
