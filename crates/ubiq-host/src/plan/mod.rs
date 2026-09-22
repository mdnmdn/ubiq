//! A task's plan: a markdown document, kept beside its tasks and read or written whole.
//!
//! **A plan belongs to any task carrying a [`ubiq_proto::work::Level`]** — not to ordinary tasks,
//! and not only to some special mission subtype. Every method here that reads or writes a plan
//! checks that first, against [`crate::work::Work`] through the same [`crate::work::Handle`] the
//! coordinator and the MCP listener already share, and refuses with [`Message::PlanError`] where
//! it does not hold — the same posture [`crate::work::Work::parent_refusal`] takes with a parent
//! that is not itself allowed to be one.
//!
//! [`Handle`] mirrors [`crate::work::Handle`] on purpose: one project's plans are cheap enough that
//! no in-memory cache sits in front of [`crate::store::plan::FilePlanStore`] the way `Work` caches
//! tasks — a plan is read or written on request, never on every keystroke — but the sharing shape
//! (an `Arc<Mutex<_>>` clone held by the coordinator and by the MCP listener's `ubiq-plan` server)
//! is the same, for the same reason: an agent and a window must never disagree about what the file
//! holds.
//!
//! **The MCP reach type holds the plan store, not this module's [`Handle`] paired with a work
//! handle of its own** — `crate::mcp::PlanReach` carries only [`Handle`]; the level check still
//! happens, because [`Plans`] itself holds `crate::work::Handle` internally. That is what the
//! staging card asks for: the *reach struct* an agent's tool call is handed stays a plan store and
//! nothing else, while the service behind it is free to depend on `Work` the way any other
//! in-process collaborator would.
//!
//! **Annotations hang off blocks, and the blocks are re-matched on every save.** The body on disk
//! stays plain markdown with nothing of Ubiq's in it; [`blocks`] holds the id assignment, and this
//! module is what calls it — a save re-indexes the document, carries the ids it can, and flags
//! every annotation whose block is gone as orphaned rather than deleting it. **Anyone may resolve
//! an annotation, including an agent**, so there is no author check anywhere below.

pub mod blocks;
pub mod lines;
pub mod provenance;

use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;
use ubiq_proto::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::plan::{
    Annotation, AnnotationState, PlanBlock, PlanChangeStats, PlanChangedRegion, PlanRevision,
    SaveOrigin,
};
use ubiq_proto::work::CommentAuthor;

use crate::reply::Reply;
use crate::store::plan::{FilePlanStore, PlanSidecar};
use crate::work;

/// Who is saving a plan's body, established where the save enters the host and carried unchanged
/// from there.
///
/// **This type exists so the origin cannot be guessed downstream.** Its only constructors are
/// [`Saver::human`] and [`Saver::agent`], and there are exactly two call sites: the coordinator's
/// [`Message::SavePlan`] arm, which only the interface sends, and `ubiq-plan`'s `write_plan`,
/// which only a hosted agent can call. Nothing between those points and the sidecar has to decide
/// anything, and nothing below has the information to decide it correctly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saver {
    origin: SaveOrigin,
    author: Option<String>,
}

impl Saver {
    /// A person, through the interface. Unattributed on purpose: there is one person at the
    /// keyboard and the interface does not name them.
    pub fn human() -> Self {
        Self {
            origin: SaveOrigin::Human,
            author: None,
        }
    }

    /// An agent, by the key its MCP calls arrive under — so `plan_changes` can later default its
    /// watermark to *this* agent's last write rather than some other agent's.
    pub fn agent(key: impl Into<String>) -> Self {
        Self {
            origin: SaveOrigin::Agent,
            author: Some(key.into()),
        }
    }

    pub fn origin(&self) -> SaveOrigin {
        self.origin
    }
}

/// What [`Plans::change_report`] answers: where the plan changed, the counts, and the block index
/// those regions point into.
///
/// The block index rides along for the same reason it rides on [`Message::PlanAnnotations`] — a
/// [`BlockId`] means nothing without it, and `ubiq-plan` folds the block's text into its answer
/// rather than handing an agent a bare id to guess at. The [`Message::PlanChanges`] path drops it,
/// because a window annotating a plan already holds the index.
pub struct ChangeReport {
    pub regions: Vec<PlanChangedRegion>,
    pub stats: PlanChangeStats,
    pub blocks: Vec<PlanBlock>,
}

/// One project's plans.
pub struct Plans {
    store: FilePlanStore,
    /// To check a task exists and carries a [`ubiq_proto::work::Level`] before a plan is loaded,
    /// saved or exported. See the module doc for why this does not make `crate::mcp::PlanReach`
    /// hold a work handle of its own.
    work: work::Handle,
}

/// A cloneable handle to [`Plans`], on [`crate::work::Handle`]'s own footing: the coordinator
/// holds one clone, the MCP listener's `ubiq-plan` server holds another, and the lock covers one
/// call and is released before anything reaches the bus.
#[derive(Clone)]
pub struct Handle(Arc<Mutex<Plans>>);

impl Handle {
    pub fn new(plans: Plans) -> Self {
        Self(Arc::new(Mutex::new(plans)))
    }

    /// A poisoned lock is taken back rather than propagated — [`crate::work::Handle::lock`]'s own
    /// reasoning: refusing every plan tool because one call panicked once would be the worse
    /// failure.
    pub fn lock(&self) -> MutexGuard<'_, Plans> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Plans {
    pub fn open(store: FilePlanStore, work: work::Handle) -> Self {
        Self { store, work }
    }

    /// Why `task` may not carry a plan, if there is a reason: no such task in `project`, or a task
    /// whose `level` is `None`.
    fn refusal(&mut self, project: ProjectId, task: TaskId) -> Option<String> {
        let (_, tasks) = self.work.lock().tasks(project);
        let Some(record) = tasks.iter().find(|t| t.id == task) else {
            return Some("no such task".to_string());
        };
        if record.level.is_none() {
            return Some("only a task with a level can carry a plan".to_string());
        }
        None
    }

    /// A task's plan, whole. Answered with [`Message::Plan`], carrying an empty body for a task
    /// that has none yet — a mission nobody has planned is not an error.
    pub fn load(&mut self, project: ProjectId, task: TaskId) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(plan_error(project, Some(task), refusal))];
        }
        let body = match self.store.load(project, task) {
            Ok(body) => body.unwrap_or_default(),
            Err(error) => {
                return vec![Reply::Asker(plan_error(
                    project,
                    Some(task),
                    error.to_string(),
                ))];
            }
        };
        vec![Reply::Asker(Message::Plan {
            project_id: project,
            task_id: task,
            body,
            revision: self.revision(project, task),
        })]
    }

    /// The plan's current revision, or `0` when it has none — a plan never saved, a sidecar
    /// written before the provenance layer existed, or a sidecar that will not read at all. A
    /// watermark is a convenience and never the reason a load fails, so this swallows the error
    /// the load path would already have reported.
    fn revision(&mut self, project: ProjectId, task: TaskId) -> PlanRevision {
        self.store
            .load_sidecar(project, task)
            .ok()
            .flatten()
            .map(|sidecar| sidecar.revision)
            .unwrap_or_default()
    }

    /// Who made the plan's most recent save — what a refused save names so the banner can say
    /// whether an agent or a person moved the copy. [`SaveOrigin::Human`] when there is no history
    /// to read, on [`SaveOrigin`]'s own default: attributing an unknown save to the person is the
    /// reading that never tells an agent it wrote something it did not.
    fn latest_origin(&mut self, project: ProjectId, task: TaskId) -> SaveOrigin {
        self.store
            .load_sidecar(project, task)
            .ok()
            .flatten()
            .and_then(|sidecar| sidecar.history.last().map(|entry| entry.origin))
            .unwrap_or_default()
    }

    /// Replace a task's plan, whole. The asker gets the body back as confirmation; every window
    /// hears [`Message::PlanChanged`] so a viewer already open on this plan knows to re-ask.
    ///
    /// `by` says whose save this is, and is the only thing that ever decides it — see [`Saver`].
    ///
    /// `expected` is the revision the body was written against, and **this is where a race is
    /// arbitrated**: a save naming a revision the plan has already moved past writes nothing and
    /// answers [`Message::PlanConflict`] with where the plan actually stands. The host decides it
    /// rather than the window because two windows cannot see each other, and because the gap
    /// between a window asking its user "overwrite?" and the user answering is exactly long enough
    /// for a third save to land. `None` skips the check, for a caller that has no watermark to
    /// name — see `crate::mcp::plan::write_plan`.
    ///
    /// **The previous body is read before the new one is written**, because both the block
    /// matching and the line diff compare against it. That read is the one ordering constraint in
    /// here: everything else could happen in any order, and this could not.
    pub fn save(
        &mut self,
        project: ProjectId,
        task: TaskId,
        body: String,
        by: &Saver,
        expected: Option<PlanRevision>,
    ) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(plan_error(project, Some(task), refusal))];
        }
        if let Some(expected) = expected {
            let current = self.revision(project, task);
            if current != expected {
                return vec![Reply::Asker(Message::PlanConflict {
                    project_id: project,
                    task_id: task,
                    revision: current,
                    origin: self.latest_origin(project, task),
                })];
            }
        }
        let previous = match self.store.load(project, task) {
            Ok(previous) => previous.unwrap_or_default(),
            Err(error) => {
                return vec![Reply::Asker(plan_error(
                    project,
                    Some(task),
                    error.to_string(),
                ))];
            }
        };
        if let Err(error) = self.store.save(project, task, &body) {
            return vec![Reply::Asker(plan_error(
                project,
                Some(task),
                error.to_string(),
            ))];
        }
        // The body is on disk either way: a sidecar that will not be read or written leaves the
        // block index stale, which is worth saying out loud, but it is not a failed save.
        let (revision, after) = match self.reindex(project, task, &previous, &body, by) {
            Ok((revision, true)) => (
                revision,
                vec![Reply::Everyone(Message::PlanAnnotationsChanged {
                    project_id: project,
                    task_id: task,
                })],
            ),
            Ok((revision, false)) => (revision, Vec::new()),
            // The sidecar did not take the stamp, so there is no revision to name. Reporting the
            // save at its previous revision would hand out a watermark that never existed.
            Err(error) => (
                self.revision(project, task),
                vec![Reply::Asker(plan_error(project, Some(task), error))],
            ),
        };
        let mut replies = vec![
            Reply::Asker(Message::Plan {
                project_id: project,
                task_id: task,
                body,
                revision,
            }),
            Reply::Everyone(Message::PlanChanged {
                project_id: project,
                task_id: task,
                revision,
                origin: by.origin,
            }),
        ];
        replies.extend(after);
        replies
    }

    /// Where a task's plan has been edited since `since`, and by how much.
    ///
    /// `since` absent means from the beginning, which for a plan with provenance is every line
    /// ever stamped. The regions describe the body as it stands now — see
    /// [`provenance::regions`].
    pub fn changes(
        &mut self,
        project: ProjectId,
        task: TaskId,
        since: Option<PlanRevision>,
    ) -> Vec<Reply> {
        match self.change_report(project, task, since) {
            Ok(report) => vec![Reply::Asker(Message::PlanChanges {
                project_id: project,
                task_id: task,
                regions: report.regions,
                stats: report.stats,
            })],
            Err(error) => vec![Reply::Asker(plan_error(project, Some(task), error))],
        }
    }

    /// The regions and the stats themselves, for a caller that wants the records rather than a
    /// `Message` — `ubiq-plan`'s `plan_changes`, on [`Self::annotation_list`]'s own footing. Both
    /// paths come through here, so a window's decoration and an agent's report can never disagree
    /// about what changed.
    pub fn change_report(
        &mut self,
        project: ProjectId,
        task: TaskId,
        since: Option<PlanRevision>,
    ) -> Result<ChangeReport, String> {
        if let Some(refusal) = self.refusal(project, task) {
            return Err(refusal);
        }
        let sidecar = self
            .store
            .load_sidecar(project, task)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        let body = self
            .store
            .load(project, task)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();

        // A watermark past the end is the caller's own revision arriving before it hears its own
        // save echoed, or a stale handle. Clamping answers "nothing since then", which is true,
        // rather than reporting every line because the comparison went negative.
        let since = since.unwrap_or_default().min(sidecar.revision);
        let regions = provenance::regions(&sidecar.provenance, since, &body, &sidecar.blocks);
        let stats = provenance::stats(&sidecar.history, since, sidecar.revision, &regions);
        Ok(ChangeReport {
            regions,
            stats,
            blocks: sidecar.blocks,
        })
    }

    /// The revision an agent last wrote this plan at — `plan_changes`'s default watermark.
    pub fn last_written_by(
        &mut self,
        project: ProjectId,
        task: TaskId,
        author: &str,
    ) -> Option<PlanRevision> {
        self.store
            .load_sidecar(project, task)
            .ok()
            .flatten()
            .and_then(|sidecar| sidecar.last_written_by(author))
    }

    /// Re-assign the plan's block ids against the previous save's, stamp the lines this save
    /// changed, and flag every annotation whose block is gone. Answers the new revision, and
    /// whether any annotation changed so the caller knows whether to tell the other windows.
    ///
    /// **Both layers are computed against the same `previous` and written in the same sidecar
    /// save**, which is what keeps the block index and the line provenance from ever being a save
    /// apart — `store/plan.rs`'s reason for the sidecar being one file.
    ///
    /// **Orphaning is one-way.** A vanished block's id leaves the index, so a passage the user
    /// deletes and later retypes comes back as a new block with a new id: the annotation stays
    /// flagged rather than being silently re-anchored to something that only looks like what it
    /// was about.
    ///
    /// **Every save takes a revision**, including one that writes the bytes already there: the
    /// counter follows saves, not content, so a watermark handed out by one save is never
    /// invalidated by the next. Such a save records a [`provenance::RevisionEntry`] with all
    /// three counts at zero and stamps no line, so it costs a number and changes no answer.
    fn reindex(
        &mut self,
        project: ProjectId,
        task: TaskId,
        previous: &str,
        body: &str,
        by: &Saver,
    ) -> Result<(PlanRevision, bool), String> {
        let mut sidecar = self
            .store
            .load_sidecar(project, task)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();

        let matching = blocks::match_blocks(&sidecar.blocks, &blocks::blocks(body));
        let mut changed = false;
        for annotation in &mut sidecar.annotations {
            if !annotation.orphaned && matching.vanished.contains(&annotation.block_id) {
                annotation.orphaned = true;
                changed = true;
            }
        }
        sidecar.blocks = matching.blocks;

        let revision = sidecar.revision + 1;
        let diff = lines::diff(previous, body);
        sidecar.provenance = provenance::advance(&sidecar.provenance, &diff, revision, by.origin);
        sidecar.history.push(provenance::RevisionEntry {
            revision,
            origin: by.origin,
            author: by.author.clone(),
            at: Utc::now(),
            added: diff.added,
            removed: diff.removed,
            modified: diff.modified,
        });
        if sidecar.history.len() > provenance::HISTORY_LIMIT {
            let over = sidecar.history.len() - provenance::HISTORY_LIMIT;
            sidecar.history.drain(..over);
        }
        sidecar.revision = revision;

        sidecar.version = crate::store::plan::ANNOTATIONS_VERSION;
        self.store
            .save_sidecar(project, task, &sidecar)
            .map_err(|error| error.to_string())?;
        Ok((revision, changed))
    }

    /// Drop a task's plan. **No level check here**, unlike [`Self::load`] and [`Self::save`]: this
    /// is also how [`crate::work::Work::delete`] keeps a deleted task's plan from being left
    /// orphaned on disk, and a task already gone from the board cannot be asked whether it still
    /// carries a level. Broadcast only when a file actually went away, so deleting the great
    /// majority of tasks — which never had a plan — costs nothing on the bus.
    pub fn delete(&mut self, project: ProjectId, task: TaskId) -> Vec<Reply> {
        let existed = self.store.path(project, task).exists();
        match self.store.delete(project, task) {
            Ok(()) if existed => vec![Reply::Everyone(Message::PlanDeleted {
                project_id: project,
                task_id: task,
            })],
            Ok(()) => Vec::new(),
            Err(error) => vec![Reply::Asker(plan_error(
                project,
                Some(task),
                error.to_string(),
            ))],
        }
    }

    /// A plan's sidecar, with its block index brought up to date if it has none yet — a plan
    /// written before annotations existed, or one saved by a build that did not index it. The
    /// index is what an annotation names, so a plan with a body and no index cannot be annotated
    /// at all, and re-deriving it here costs one parse on the first annotation instead of a
    /// migration.
    fn sidecar(&mut self, project: ProjectId, task: TaskId) -> Result<PlanSidecar, String> {
        let sidecar = self
            .store
            .load_sidecar(project, task)
            .map_err(|error| error.to_string())?;
        if let Some(sidecar) = sidecar
            .as_ref()
            .filter(|sidecar| !sidecar.blocks.is_empty())
        {
            return Ok(sidecar.clone());
        }
        let body = self
            .store
            .load(project, task)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        if body.trim().is_empty() {
            return Ok(sidecar.unwrap_or_default());
        }
        let mut sidecar = sidecar.unwrap_or_default();
        sidecar.blocks = blocks::match_blocks(&[], &blocks::blocks(&body)).blocks;
        sidecar.version = crate::store::plan::ANNOTATIONS_VERSION;
        self.store
            .save_sidecar(project, task, &sidecar)
            .map_err(|error| error.to_string())?;
        Ok(sidecar)
    }

    /// Every annotation on a task's plan — open, resolved and orphaned alike — with the block
    /// index they anchor to. The filtering is the interface's; see
    /// [`Message::ListPlanAnnotations`].
    pub fn annotations(&mut self, project: ProjectId, task: TaskId) -> Vec<Reply> {
        match self.annotation_list(project, task) {
            Ok((blocks, annotations)) => vec![Reply::Asker(Message::PlanAnnotations {
                project_id: project,
                task_id: task,
                blocks,
                annotations,
            })],
            Err(error) => vec![Reply::Asker(plan_error(project, Some(task), error))],
        }
    }

    /// The blocks and the annotations themselves, for a caller that wants the records rather than
    /// a `Message` — `ubiq-plan`'s MCP tools, on [`Self::body`]'s own footing.
    #[allow(clippy::type_complexity)]
    pub fn annotation_list(
        &mut self,
        project: ProjectId,
        task: TaskId,
    ) -> Result<(Vec<PlanBlock>, Vec<Annotation>), String> {
        if let Some(refusal) = self.refusal(project, task) {
            return Err(refusal);
        }
        let sidecar = self.sidecar(project, task)?;
        Ok((sidecar.blocks, sidecar.annotations))
    }

    /// Open an annotation on one block of a task's plan. The block must be one the last save
    /// indexed: an annotation that names a block the plan does not have would arrive orphaned,
    /// which is a state to report, never one to create.
    pub fn annotate(
        &mut self,
        project: ProjectId,
        task: TaskId,
        block: BlockId,
        quote: Option<String>,
        author: CommentAuthor,
        text: String,
    ) -> Vec<Reply> {
        self.mutate(project, task, |sidecar| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err("an annotation needs some text".to_string());
            }
            if !sidecar.blocks.iter().any(|indexed| indexed.id == block) {
                return Err("no such block in this plan".to_string());
            }
            sidecar.annotations.push(Annotation::new(
                block,
                quote
                    .map(|quote| quote.trim().to_string())
                    .filter(|quote| !quote.is_empty()),
                author,
                text,
                Utc::now(),
            ));
            Ok(())
        })
    }

    /// Append to an annotation's thread.
    pub fn reply_to(
        &mut self,
        project: ProjectId,
        task: TaskId,
        annotation: AnnotationId,
        author: CommentAuthor,
        text: String,
    ) -> Vec<Reply> {
        self.mutate(project, task, |sidecar| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err("a reply needs some text".to_string());
            }
            let found = sidecar
                .annotations
                .iter_mut()
                .find(|existing| existing.id == annotation)
                .ok_or_else(|| "no such annotation".to_string())?;
            found.reply(author, text, Utc::now());
            Ok(())
        })
    }

    /// Close an annotation, or reopen one. **No author check**: anyone may resolve an annotation,
    /// including the agent that answered it.
    pub fn resolve(
        &mut self,
        project: ProjectId,
        task: TaskId,
        annotation: AnnotationId,
        resolved: bool,
    ) -> Vec<Reply> {
        self.mutate(project, task, |sidecar| {
            let found = sidecar
                .annotations
                .iter_mut()
                .find(|existing| existing.id == annotation)
                .ok_or_else(|| "no such annotation".to_string())?;
            found.state = if resolved {
                AnnotationState::Resolved
            } else {
                AnnotationState::Open
            };
            Ok(())
        })
    }

    /// The level check, the sidecar read, the change, the write and the two replies — every
    /// annotation mutation is these in this order, the way `Work::with_task` is for a task. The
    /// asker gets the whole set back; every other window hears that it changed.
    fn mutate(
        &mut self,
        project: ProjectId,
        task: TaskId,
        change: impl FnOnce(&mut PlanSidecar) -> Result<(), String>,
    ) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(project, task) {
            return vec![Reply::Asker(plan_error(project, Some(task), refusal))];
        }
        let mut sidecar = match self.sidecar(project, task) {
            Ok(sidecar) => sidecar,
            Err(error) => return vec![Reply::Asker(plan_error(project, Some(task), error))],
        };
        if let Err(error) = change(&mut sidecar) {
            return vec![Reply::Asker(plan_error(project, Some(task), error))];
        }
        sidecar.version = crate::store::plan::ANNOTATIONS_VERSION;
        if let Err(error) = self.store.save_sidecar(project, task, &sidecar) {
            return vec![Reply::Asker(plan_error(
                project,
                Some(task),
                error.to_string(),
            ))];
        }
        vec![
            Reply::Asker(Message::PlanAnnotations {
                project_id: project,
                task_id: task,
                blocks: sidecar.blocks,
                annotations: sidecar.annotations,
            }),
            Reply::Everyone(Message::PlanAnnotationsChanged {
                project_id: project,
                task_id: task,
            }),
        ]
    }

    /// A task's plan body, or the reason it may not be read — for a caller that wants the string
    /// itself rather than a `Message`: the MCP `read_plan` tool, and the coordinator's export
    /// handler.
    pub fn body(&mut self, project: ProjectId, task: TaskId) -> Result<String, String> {
        if let Some(refusal) = self.refusal(project, task) {
            return Err(refusal);
        }
        self.store
            .load(project, task)
            .map(|body| body.unwrap_or_default())
            .map_err(|error| error.to_string())
    }
}

fn plan_error(project: ProjectId, task: Option<TaskId>, error: impl Into<String>) -> Message {
    let error = error.into();
    tracing::warn!("{project}'s plan: {error}");
    Message::PlanError {
        project_id: project,
        task_id: task,
        error,
    }
}

#[cfg(test)]
mod tests {
    use ubiq_proto::messages::TaskField;
    use ubiq_proto::work::Level;

    use super::*;
    use crate::store::memory::MemoryTaskStore;

    /// The `TempDir` rides along so it is not dropped — and its directory removed — before the
    /// test that owns it is done with the store pointed at it.
    fn plans_with_work() -> (Plans, work::Handle, ProjectId, tempfile::TempDir) {
        let work = work::Handle::new(work::Work::open(Box::new(MemoryTaskStore::new())));
        let dir = tempfile::tempdir().unwrap();
        let store = FilePlanStore::new(dir.path().to_path_buf());
        let plans = Plans::open(store, work.clone());
        (plans, work, ProjectId::generate(), dir)
    }

    fn make_mission(work: &work::Handle, project: ProjectId) -> TaskId {
        let mut work = work.lock();
        let replies = work.create(project, "a mission".to_string(), None);
        let id = replies
            .iter()
            .find_map(|reply| match reply.message() {
                Message::TaskCreated { task, .. } => Some(task.id),
                _ => None,
            })
            .expect("the task was created");
        work.set_field(project, id, TaskField::Level(Some(Level::Mission)));
        id
    }

    fn make_ordinary_task(work: &work::Handle, project: ProjectId) -> TaskId {
        let mut work = work.lock();
        let replies = work.create(project, "an ordinary task".to_string(), None);
        replies
            .iter()
            .find_map(|reply| match reply.message() {
                Message::TaskCreated { task, .. } => Some(task.id),
                _ => None,
            })
            .expect("the task was created")
    }

    #[test]
    fn a_plan_round_trips_through_save_and_load() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        let replies = plans.save(
            project,
            task,
            "# Plan\n\nStep one.".to_string(),
            &Saver::human(),
            None,
        );
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::Plan { .. }))
        );
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply, Reply::Everyone(Message::PlanChanged { .. })))
        );

        let replies = plans.load(project, task);
        let Some(Message::Plan { body, .. }) = replies
            .iter()
            .map(Reply::message)
            .find(|m| matches!(m, Message::Plan { .. }))
        else {
            panic!("expected a Plan reply, got {replies:?}");
        };
        assert_eq!(body, "# Plan\n\nStep one.");
    }

    #[test]
    fn a_task_with_no_level_is_refused_a_plan() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_ordinary_task(&work, project);

        let replies = plans.save(project, task, "body".to_string(), &Saver::human(), None);
        let error = replies
            .iter()
            .map(Reply::message)
            .find_map(|m| match m {
                Message::PlanError { error, .. } => Some(error.clone()),
                _ => None,
            })
            .expect("a task with no level is refused");
        assert!(error.contains("level"), "unexpected refusal: {error}");

        let replies = plans.load(project, task);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::PlanError { .. }))
        );
    }

    /// The block ids of a saved plan, in document order.
    fn block_ids(plans: &mut Plans, project: ProjectId, task: TaskId) -> Vec<BlockId> {
        plans
            .sidecar(project, task)
            .expect("the sidecar reads")
            .blocks
            .iter()
            .map(|block| block.id)
            .collect()
    }

    fn annotations_of(plans: &mut Plans, project: ProjectId, task: TaskId) -> Vec<Annotation> {
        plans
            .annotation_list(project, task)
            .expect("the annotations read")
            .1
    }

    #[test]
    fn an_annotation_round_trips_through_the_sidecar() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(
            project,
            task,
            "# Plan\n\nStep one.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[1];

        let replies = plans.annotate(
            project,
            task,
            block,
            Some("Step one".to_string()),
            CommentAuthor::User,
            "  Which one? ".to_string(),
        );
        assert!(replies.iter().any(|reply| matches!(
            reply,
            Reply::Everyone(Message::PlanAnnotationsChanged { .. })
        )));

        let annotations = annotations_of(&mut plans, project, task);
        assert_eq!(annotations.len(), 1);
        let annotation = &annotations[0];
        assert_eq!(annotation.block_id, block);
        assert_eq!(annotation.quote.as_deref(), Some("Step one"));
        assert_eq!(annotation.state, AnnotationState::Open);
        assert!(!annotation.orphaned);
        assert_eq!(annotation.thread.len(), 1);
        assert_eq!(annotation.thread[0].text, "Which one?");

        // An agent replies and closes it: anyone may resolve an annotation.
        plans.reply_to(
            project,
            task,
            annotation.id,
            CommentAuthor::Agent,
            "The first one.".to_string(),
        );
        plans.resolve(project, task, annotation.id, true);

        let annotations = annotations_of(&mut plans, project, task);
        assert_eq!(annotations[0].thread.len(), 2);
        assert_eq!(annotations[0].thread[1].author, CommentAuthor::Agent);
        assert_eq!(annotations[0].state, AnnotationState::Resolved);
    }

    #[test]
    fn the_markdown_file_holds_no_annotation_of_any_kind() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let body = "# Plan\n\nStep one.";
        plans.save(project, task, body.to_string(), &Saver::human(), None);
        let block = block_ids(&mut plans, project, task)[1];
        plans.annotate(
            project,
            task,
            block,
            None,
            CommentAuthor::User,
            "a comment".to_string(),
        );

        let on_disk = plans.store.load(project, task).unwrap().unwrap();
        assert_eq!(on_disk, body, "the plan on disk is still plain markdown");
        assert!(
            plans.store.annotations_path(project, task).exists(),
            "the annotations went to the sidecar beside it",
        );
    }

    #[test]
    fn an_edit_elsewhere_keeps_an_annotation_anchored() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(
            project,
            task,
            "# Plan\n\nStep one.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[2];
        plans.annotate(
            project,
            task,
            block,
            None,
            CommentAuthor::User,
            "about step two".to_string(),
        );

        // A paragraph inserted above, and step two itself reworded: neither moves the anchor.
        plans.save(
            project,
            task,
            "# Plan\n\nA preamble.\n\nStep one.\n\nStep two, revised.".to_string(),
            &Saver::human(),
            None,
        );

        let annotations = annotations_of(&mut plans, project, task);
        assert_eq!(annotations.len(), 1);
        assert!(!annotations[0].orphaned, "the block is still there");
        assert_eq!(
            annotations[0].block_id, block,
            "the anchor is the id, not an offset",
        );
        assert_eq!(
            block_ids(&mut plans, project, task)[3],
            block,
            "and the block it names is the edited paragraph, now one row lower",
        );
    }

    #[test]
    fn a_vanished_block_orphans_its_annotation_rather_than_dropping_it() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(
            project,
            task,
            "# Plan\n\nStep one.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[2];
        plans.annotate(
            project,
            task,
            block,
            Some("Step two".to_string()),
            CommentAuthor::User,
            "about step two".to_string(),
        );

        let replies = plans.save(
            project,
            task,
            "# Plan\n\nStep one.".to_string(),
            &Saver::human(),
            None,
        );
        assert!(
            replies.iter().any(|reply| matches!(
                reply,
                Reply::Everyone(Message::PlanAnnotationsChanged { .. })
            )),
            "the windows are told the annotation set changed",
        );

        let annotations = annotations_of(&mut plans, project, task);
        assert_eq!(annotations.len(), 1, "it was flagged, not deleted");
        assert!(annotations[0].orphaned);
        assert_eq!(
            annotations[0].block_id, block,
            "it still says what it was about"
        );
        assert_eq!(annotations[0].quote.as_deref(), Some("Step two"));
        assert_eq!(annotations[0].thread.len(), 1, "the thread survived");
    }

    #[test]
    fn an_annotation_on_a_block_the_plan_does_not_have_is_refused() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(project, task, "# Plan".to_string(), &Saver::human(), None);

        let replies = plans.annotate(
            project,
            task,
            BlockId::generate(),
            None,
            CommentAuthor::User,
            "about nothing".to_string(),
        );
        let error = replies
            .iter()
            .map(Reply::message)
            .find_map(|m| match m {
                Message::PlanError { error, .. } => Some(error.clone()),
                _ => None,
            })
            .expect("an unknown block is refused");
        assert!(error.contains("block"), "unexpected refusal: {error}");
        assert!(annotations_of(&mut plans, project, task).is_empty());
    }

    #[test]
    fn a_task_with_no_level_is_refused_its_annotations() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_ordinary_task(&work, project);

        let replies = plans.annotations(project, task);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::PlanError { .. }))
        );
    }

    #[test]
    fn deleting_a_plan_takes_its_annotations_with_it() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(
            project,
            task,
            "# Plan\n\nStep one.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[1];
        plans.annotate(
            project,
            task,
            block,
            None,
            CommentAuthor::User,
            "a comment".to_string(),
        );

        plans.delete(project, task);
        assert!(!plans.store.annotations_path(project, task).exists());
        assert!(annotations_of(&mut plans, project, task).is_empty());
    }

    #[test]
    fn deleting_a_task_with_no_plan_broadcasts_nothing() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        let replies = plans.delete(project, task);
        assert!(replies.is_empty());
    }

    #[test]
    fn deleting_a_task_with_a_plan_broadcasts_that_it_is_gone() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(project, task, "body".to_string(), &Saver::human(), None);

        let replies = plans.delete(project, task);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply, Reply::Everyone(Message::PlanDeleted { .. })))
        );

        let replies = plans.load(project, task);
        let Some(Message::Plan { body, .. }) = replies
            .iter()
            .map(Reply::message)
            .find(|m| matches!(m, Message::Plan { .. }))
        else {
            panic!("expected a Plan reply, got {replies:?}");
        };
        assert_eq!(
            body, "",
            "the plan is gone, so load answers empty rather than erroring"
        );
    }

    // ── the edit-provenance layer ───────────────────────────────────

    /// The plan every provenance test starts from: five lines, two of which a human will edit.
    const PLANNED: &str = "# Plan\n\nStep one.\n\nStep two.";

    fn revision_of(replies: &[Reply]) -> PlanRevision {
        replies
            .iter()
            .find_map(|reply| match reply.message() {
                Message::Plan { revision, .. } => Some(*revision),
                _ => None,
            })
            .expect("a save answers with the revision it took")
    }

    /// **The sequence the whole layer exists for**: an agent writes a plan, a human edits it in
    /// two separate places, and the agent comes back and is told exactly those two places — not
    /// the three lines between them, and not the document.
    #[test]
    fn an_agents_plan_edited_by_a_human_in_two_places_reports_exactly_those_two() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        let written = revision_of(&plans.save(
            project,
            task,
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));
        assert_eq!(written, 1, "the first save is revision 1");

        // A person reworks line 3 and line 5, and leaves lines 1, 2 and 4 alone.
        plans.save(
            project,
            task,
            "# Plan\n\nStep one, revised.\n\nStep two, also revised.".to_string(),
            &Saver::human(),
            None,
        );

        let report = plans
            .change_report(project, task, Some(written))
            .expect("the changes read");

        let places: Vec<(u32, u32, SaveOrigin)> = report
            .regions
            .iter()
            .map(|region| (region.first_line, region.last_line, region.origin))
            .collect();
        assert_eq!(
            places,
            vec![(3, 3, SaveOrigin::Human), (5, 5, SaveOrigin::Human)],
            "exactly the two lines the human touched",
        );
        assert_eq!(report.regions[0].text, "Step one, revised.");
        assert_eq!(report.regions[1].text, "Step two, also revised.");

        assert_eq!(report.stats.lines_modified, 2);
        assert_eq!(
            (report.stats.lines_added, report.stats.lines_removed),
            (0, 0)
        );
        assert_eq!(report.stats.human_revisions, 1);
        assert_eq!(
            report.stats.agent_revisions, 0,
            "the agent's own write is below its watermark",
        );
        assert_eq!(report.stats.blocks_touched, 2);
        assert_eq!(report.stats.since_revision, 1);
        assert_eq!(report.stats.revision, 2);

        // Each region names the block it fell in, so the agent is never handed a bare id.
        let named: Vec<BlockId> = report
            .regions
            .iter()
            .filter_map(|region| region.block_id)
            .collect();
        assert_eq!(named.len(), 2, "both runs sit inside a block");
        assert!(
            named
                .iter()
                .all(|id| report.blocks.iter().any(|block| block.id == *id)),
            "and the ids are ones the index holds",
        );
    }

    #[test]
    fn the_agents_own_write_is_the_default_watermark() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        plans.save(
            project,
            task,
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        );
        plans.save(
            project,
            task,
            "# Plan\n\nStep one, revised.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );

        assert_eq!(
            plans.last_written_by(project, task, "agent-1"),
            Some(1),
            "the revision this agent last wrote at",
        );
        assert_eq!(
            plans.last_written_by(project, task, "agent-2"),
            None,
            "an agent that never wrote this plan has no watermark of its own",
        );
    }

    #[test]
    fn a_human_save_and_an_agent_save_are_told_apart_on_disk() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        plans.save(
            project,
            task,
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        );
        plans.save(
            project,
            task,
            "# Plan\n\nStep one, revised.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );

        let raw = std::fs::read_to_string(plans.store.annotations_path(project, task)).unwrap();
        let sidecar: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(sidecar["revision"], 2);
        let history = sidecar["history"].as_array().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["origin"], "agent");
        assert_eq!(history[0]["author"], "agent-1");
        assert_eq!(history[1]["origin"], "human");
        assert!(
            history[1].get("author").is_none(),
            "a human save names nobody",
        );

        // The line the human rewrote is stamped as theirs; the rest is still the agent's.
        let runs = sidecar["provenance"].as_array().unwrap();
        let human: Vec<&serde_json::Value> =
            runs.iter().filter(|run| run["origin"] == "human").collect();
        assert_eq!(human.len(), 1);
        assert_eq!(human[0]["first_line"], 3);
        assert_eq!(human[0]["last_line"], 3);
    }

    #[test]
    fn a_save_reports_its_revision_and_origin_to_every_window() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        let replies = plans.save(
            project,
            task,
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        );
        let announced = replies
            .iter()
            .find_map(|reply| match reply {
                Reply::Everyone(Message::PlanChanged {
                    revision, origin, ..
                }) => Some((*revision, *origin)),
                _ => None,
            })
            .expect("every window hears the save");
        assert_eq!(announced, (1, SaveOrigin::Agent));
    }

    #[test]
    fn a_load_carries_the_revision_as_a_watermark() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        let replies = plans.load(project, task);
        assert_eq!(
            revision_of(&replies),
            0,
            "a plan nobody has written stands at revision 0",
        );

        plans.save(project, task, PLANNED.to_string(), &Saver::human(), None);
        assert_eq!(revision_of(&plans.load(project, task)), 1);
    }

    #[test]
    fn a_watermark_at_the_current_revision_reports_no_change() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let written = revision_of(&plans.save(
            project,
            task,
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));

        let report = plans
            .change_report(project, task, Some(written))
            .expect("the changes read");
        assert!(report.regions.is_empty());
        assert!(report.stats.is_empty());
        assert_eq!(report.stats.since_revision, report.stats.revision);
    }

    #[test]
    fn a_deletion_is_reported_where_it_happened_rather_than_not_at_all() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let written = revision_of(&plans.save(
            project,
            task,
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));

        // The human deletes "Step one." and the blank line under it.
        plans.save(
            project,
            task,
            "# Plan\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );

        let report = plans
            .change_report(project, task, Some(written))
            .expect("the changes read");
        assert_eq!(report.stats.lines_removed, 2);
        assert_eq!(
            report.regions.len(),
            1,
            "the deletion is one place, not nowhere",
        );
        assert_eq!(report.regions[0].origin, SaveOrigin::Human);
    }

    /// A sidecar from before this layer existed: the envelope is unchanged, so it loads, keeps
    /// its annotations, and simply reports that nothing is *known* to have changed.
    #[test]
    fn a_sidecar_written_before_the_provenance_layer_still_loads() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        // Write the body and let the block index be built, then hand-write the older sidecar
        // shape over the top of it: version, blocks and annotations, and nothing else.
        plans.save(project, task, PLANNED.to_string(), &Saver::human(), None);
        let block = block_ids(&mut plans, project, task)[1];
        plans.annotate(
            project,
            task,
            block,
            None,
            CommentAuthor::User,
            "a comment".to_string(),
        );
        let sidecar = plans.store.load_sidecar(project, task).unwrap().unwrap();
        let older = serde_json::json!({
            "version": 1,
            "blocks": sidecar.blocks,
            "annotations": sidecar.annotations,
        });
        std::fs::write(
            plans.store.annotations_path(project, task),
            serde_json::to_string_pretty(&older).unwrap(),
        )
        .unwrap();

        let reloaded = plans
            .store
            .load_sidecar(project, task)
            .expect("the older sidecar reads")
            .expect("it is there");
        assert_eq!(reloaded.revision, 0, "no revision was ever recorded");
        assert!(reloaded.provenance.is_empty());
        assert!(reloaded.history.is_empty());
        assert_eq!(
            reloaded.annotations.len(),
            1,
            "and nothing was lost on the way",
        );

        let report = plans
            .change_report(project, task, None)
            .expect("the changes read");
        assert!(
            report.regions.is_empty(),
            "an unstamped document is not reported as freshly written",
        );

        // The next save stamps only what it actually changed, and does not claim the rest.
        plans.save(
            project,
            task,
            "# Plan\n\nStep one, revised.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        let report = plans
            .change_report(project, task, Some(0))
            .expect("the changes read");
        assert_eq!(report.regions.len(), 1);
        assert_eq!(report.regions[0].first_line, 3);
        assert_eq!(
            report.stats.revision, 1,
            "the counter starts from the 0 the older sidecar carried",
        );
    }

    #[test]
    fn a_save_that_changes_nothing_still_takes_a_revision_and_reports_no_change() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let first = revision_of(&plans.save(
            project,
            task,
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));
        let second =
            revision_of(&plans.save(project, task, PLANNED.to_string(), &Saver::human(), None));

        assert_eq!(
            (first, second),
            (1, 2),
            "the counter follows saves, so a watermark once handed out stays meaningful",
        );
        let report = plans
            .change_report(project, task, Some(first))
            .expect("the changes read");
        assert!(report.regions.is_empty(), "nothing was rewritten");
        assert!(report.stats.is_empty(), "and nothing is counted");
        assert_eq!(
            report.stats.human_revisions, 1,
            "though the save itself is still on the record",
        );
    }

    // ── the expected revision ───────────────────────────────────────

    fn conflict_of(replies: &[Reply]) -> Option<(PlanRevision, SaveOrigin)> {
        replies.iter().find_map(|reply| match reply.message() {
            Message::PlanConflict {
                revision, origin, ..
            } => Some((*revision, *origin)),
            _ => None,
        })
    }

    /// Two windows editing the same plan from the same watermark. The first save lands; the
    /// second, made against a revision that no longer stands, is **refused** rather than written
    /// over the first — which is the whole difference between losing the work and being told.
    ///
    /// And the loser is not stuck: naming the revision the refusal reported is what a confirmed
    /// *Overwrite* does, and it wins. There is no force flag anywhere in this test because there
    /// is none on the wire.
    #[test]
    fn two_windows_racing_one_save_wins_and_the_other_is_refused() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let seeded = revision_of(&plans.save(
            project,
            task,
            "# Plan\n\nAs it was.".to_string(),
            &Saver::human(),
            None,
        ));

        // Both windows hold `seeded` and both mean to replace it.
        let first = plans.save(
            project,
            task,
            "# Plan\n\nThe first window's.".to_string(),
            &Saver::human(),
            Some(seeded),
        );
        let landed = revision_of(&first);
        assert_eq!(landed, seeded + 1, "the first save is an ordinary one");
        assert!(conflict_of(&first).is_none());

        let second = plans.save(
            project,
            task,
            "# Plan\n\nThe second window's.".to_string(),
            &Saver::human(),
            Some(seeded),
        );
        assert_eq!(
            conflict_of(&second),
            Some((landed, SaveOrigin::Human)),
            "the loser is told where the plan actually stands, and who moved it",
        );
        assert!(
            !second
                .iter()
                .any(|reply| matches!(reply, Reply::Everyone(_))),
            "a refused save is nobody else's business — nothing was written",
        );
        assert_eq!(
            plans.body(project, task).expect("the body reads"),
            "# Plan\n\nThe first window's.",
            "and the winner's body is untouched",
        );

        // The second window is shown that, presses Overwrite, and names what it was told.
        let again = plans.save(
            project,
            task,
            "# Plan\n\nThe second window's.".to_string(),
            &Saver::human(),
            Some(landed),
        );
        assert!(
            conflict_of(&again).is_none(),
            "a determined user gets there"
        );
        assert_eq!(revision_of(&again), landed + 1);
        assert_eq!(
            plans.body(project, task).expect("the body reads"),
            "# Plan\n\nThe second window's.",
        );
    }

    /// An agent's `write_plan` is the other racer, and the refusal names it as one — the banner
    /// the window draws says "an agent" or "somebody else", and it reads that from here.
    #[test]
    fn a_refusal_names_an_agent_that_moved_the_copy() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let seeded =
            revision_of(&plans.save(project, task, PLANNED.to_string(), &Saver::human(), None));
        plans.save(
            project,
            task,
            "# Plan\n\nThe agent's.".to_string(),
            &Saver::agent("agent-1"),
            None,
        );

        let refused = plans.save(
            project,
            task,
            "mine".to_string(),
            &Saver::human(),
            Some(seeded),
        );
        assert_eq!(
            conflict_of(&refused),
            Some((seeded + 1, SaveOrigin::Agent)),
            "overwriting an agent and overwriting a colleague are not the same decision",
        );
    }

    /// A first save against a plan nobody has written names `0`, and is refused if somebody got
    /// there first — the watermark meaning "nothing yet" is a watermark like any other.
    #[test]
    fn a_first_save_names_the_empty_watermark_and_is_refused_if_beaten_to_it() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        let fresh = plans.save(project, task, "mine".to_string(), &Saver::human(), Some(0));
        assert!(conflict_of(&fresh).is_none(), "nothing stood there");
        assert_eq!(revision_of(&fresh), 1);

        let beaten = plans.save(
            project,
            task,
            "also mine".to_string(),
            &Saver::human(),
            Some(0),
        );
        assert_eq!(conflict_of(&beaten).map(|(revision, _)| revision), Some(1));
    }

    #[test]
    fn a_task_with_no_level_is_refused_its_changes() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_ordinary_task(&work, project);

        let replies = plans.changes(project, task, None);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::PlanError { .. }))
        );
    }
}
