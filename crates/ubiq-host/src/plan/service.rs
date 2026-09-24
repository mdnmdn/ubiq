//! The harness-only body of `plan`: `Plans`, `Handle`, `Target` and `Saver`, split from
//! `super::blocks`, `super::lines` and `super::provenance` because those three read and match a
//! plan's sidecar with no dependency on `crate::work` at all, while everything here checks a
//! task's level through it — see the parent module's doc comment for what this is and why.
//!
//! Nested so `crate::plan::Target`, `crate::plan::Plans` and the rest keep their paths: this
//! module is declared `#[cfg(feature = "harness")]` and re-exported with `pub use` in
//! `super`, so nothing outside the crate can tell it moved.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;
use ubiq_proto::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::plan::{
    Annotation, AnnotationState, DocumentHandle, PlanBlock, PlanChangeStats, PlanChangedRegion,
    PlanRevision, SaveOrigin,
};
use ubiq_proto::work::CommentAuthor;

use crate::reply::Reply;
use crate::store::plan::{FilePlanStore, Placement, PlanSidecar, sidecar_beside};
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

/// A [`DocumentHandle`] with the host's own answer to it: where the body is, and therefore where
/// the sidecar goes.
///
/// **The wire handle names no path and this one does.** Resolving a project-relative path against
/// the project's root — and refusing one that does not land inside it — is
/// [`crate::files::path`]'s job and the coordinator's to call, so the resolution happens once, at
/// the edge, and nothing below this type ever sees a `rel_path` again. A plan resolves to nothing
/// at all: its path is the store's, computed from two ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Plan {
        project: ProjectId,
        task: TaskId,
    },
    File {
        project: ProjectId,
        rel_path: String,
        /// Absolute, canonical, and already contained inside the project.
        body: PathBuf,
    },
}

impl Target {
    /// A task's plan — what every `ubiq-plan` MCP call resolves to, and what nothing has to
    /// contain.
    pub fn plan(project: ProjectId, task: TaskId) -> Self {
        Target::Plan { project, task }
    }

    /// A wire handle against the project's own root.
    ///
    /// **Markdown only.** An annotation names a [`BlockId`], a block comes out of a markdown
    /// parse, and a sidecar beside a `.png` would anchor to blocks that cannot exist — so the
    /// extension is checked here rather than discovered as an empty block index later.
    pub fn resolve(doc: &DocumentHandle, root: &Path) -> Result<Self, String> {
        match doc {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Ok(Target::plan(*project_id, *task_id)),
            DocumentHandle::File {
                project_id,
                rel_path,
            } => {
                if !rel_path.to_ascii_lowercase().ends_with(".md") {
                    return Err("only a markdown file can carry annotations".to_string());
                }
                let body = crate::files::path::resolve_for_write(root, rel_path)
                    .map_err(|error| error.to_string())?;
                Ok(Target::File {
                    project: *project_id,
                    rel_path: rel_path.clone(),
                    body,
                })
            }
        }
    }

    pub fn project(&self) -> ProjectId {
        match self {
            Target::Plan { project, .. } | Target::File { project, .. } => *project,
        }
    }

    /// The handle this was resolved from — what every reply names, so a window matches an answer
    /// against the document it has open without the host ever sending a path back.
    pub fn handle(&self) -> DocumentHandle {
        match self {
            Target::Plan { project, task } => DocumentHandle::Plan {
                project_id: *project,
                task_id: *task,
            },
            Target::File {
                project, rel_path, ..
            } => DocumentHandle::File {
                project_id: *project,
                rel_path: rel_path.clone(),
            },
        }
    }

    fn placement(&self) -> Placement {
        match self {
            Target::Plan { .. } => Placement::ConfigRoot,
            Target::File { .. } => Placement::InsideProject,
        }
    }
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

    /// Why this document may not be read or written, if there is a reason.
    ///
    /// A plan's reason is the task's: no such task, or a task whose `level` is `None`. A file
    /// document has none left — being markdown and inside the project was settled when the
    /// [`Target`] was resolved, and a file that is not there yet reads as an empty body the same
    /// way an unplanned mission does.
    fn refusal(&mut self, target: &Target) -> Option<String> {
        let Target::Plan { project, task } = target else {
            return None;
        };
        let (_, tasks) = self.work.lock().tasks(*project);
        let Some(record) = tasks.iter().find(|t| t.id == *task) else {
            return Some("no such task".to_string());
        };
        if record.level.is_none() {
            return Some("only a task with a level can carry a plan".to_string());
        }
        None
    }

    /// Where the body sits — the store's own path for a plan, the resolved file for a document in
    /// the project's tree.
    fn body_path(&self, target: &Target) -> PathBuf {
        match target {
            Target::Plan { project, task } => self.store.path(*project, *task),
            Target::File { body, .. } => body.clone(),
        }
    }

    /// Where the sidecar sits: `<TaskId>.annotations.json` for a plan, `<file>.md.annotation.json`
    /// for a document in the project's tree. **Two spellings, deliberately** — the plan's is the
    /// one already on disk in every config root, and renaming it would be a migration for no gain.
    fn sidecar_path(&self, target: &Target) -> PathBuf {
        match target {
            Target::Plan { project, task } => self.store.annotations_path(*project, *task),
            Target::File { body, .. } => sidecar_beside(body),
        }
    }

    fn read_body(&self, target: &Target) -> Result<String, String> {
        crate::store::plan::load_body(&self.body_path(target))
            .map(|body| body.unwrap_or_default())
            .map_err(|error| error.to_string())
    }

    fn write_body(&self, target: &Target, body: &str) -> Result<(), String> {
        crate::store::plan::save_body(&self.body_path(target), body, target.placement())
            .map_err(|error| error.to_string())
    }

    fn read_sidecar(&self, target: &Target) -> Result<Option<PlanSidecar>, String> {
        crate::store::plan::load_sidecar(&self.sidecar_path(target)).map_err(|e| e.to_string())
    }

    fn write_sidecar(&self, target: &Target, sidecar: &PlanSidecar) -> Result<(), String> {
        crate::store::plan::save_sidecar(&self.sidecar_path(target), sidecar, target.placement())
            .map_err(|error| error.to_string())
    }

    /// A task's plan, whole. Answered with [`Message::Plan`], carrying an empty body for a task
    /// that has none yet — a mission nobody has planned is not an error.
    pub fn load(&mut self, target: &Target) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(target) {
            return vec![Reply::Asker(doc_error(target, refusal))];
        }
        let body = match self.read_body(target) {
            Ok(body) => body,
            Err(error) => return vec![Reply::Asker(doc_error(target, error))],
        };
        vec![Reply::Asker(Message::Plan {
            doc: target.handle(),
            body,
            revision: self.revision(target),
        })]
    }

    /// The plan's current revision, or `0` when it has none — a plan never saved, a sidecar
    /// written before the provenance layer existed, or a sidecar that will not read at all. A
    /// watermark is a convenience and never the reason a load fails, so this swallows the error
    /// the load path would already have reported.
    fn revision(&mut self, target: &Target) -> PlanRevision {
        self.read_sidecar(target)
            .ok()
            .flatten()
            .map(|sidecar| sidecar.revision)
            .unwrap_or_default()
    }

    /// Who made the plan's most recent save — what a refused save names so the banner can say
    /// whether an agent or a person moved the copy. [`SaveOrigin::Human`] when there is no history
    /// to read, on [`SaveOrigin`]'s own default: attributing an unknown save to the person is the
    /// reading that never tells an agent it wrote something it did not.
    fn latest_origin(&mut self, target: &Target) -> SaveOrigin {
        self.read_sidecar(target)
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
        target: &Target,
        body: String,
        by: &Saver,
        expected: Option<PlanRevision>,
    ) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(target) {
            return vec![Reply::Asker(doc_error(target, refusal))];
        }
        if let Some(expected) = expected {
            let current = self.revision(target);
            if current != expected {
                return vec![Reply::Asker(Message::PlanConflict {
                    doc: target.handle(),
                    revision: current,
                    origin: self.latest_origin(target),
                })];
            }
        }
        let previous = match self.read_body(target) {
            Ok(previous) => previous,
            Err(error) => return vec![Reply::Asker(doc_error(target, error))],
        };
        if let Err(error) = self.write_body(target, &body) {
            return vec![Reply::Asker(doc_error(target, error))];
        }
        // The body is on disk either way: a sidecar that will not be read or written leaves the
        // block index stale, which is worth saying out loud, but it is not a failed save.
        let (revision, after) = match self.reindex(target, &previous, &body, by) {
            Ok((revision, true)) => (
                revision,
                vec![Reply::Everyone(Message::PlanAnnotationsChanged {
                    doc: target.handle(),
                })],
            ),
            Ok((revision, false)) => (revision, Vec::new()),
            // The sidecar did not take the stamp, so there is no revision to name. Reporting the
            // save at its previous revision would hand out a watermark that never existed.
            Err(error) => (
                self.revision(target),
                vec![Reply::Asker(doc_error(target, error))],
            ),
        };
        let mut replies = vec![
            Reply::Asker(Message::Plan {
                doc: target.handle(),
                body,
                revision,
            }),
            Reply::Everyone(Message::PlanChanged {
                doc: target.handle(),
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
    /// [`super::provenance::regions`].
    pub fn changes(&mut self, target: &Target, since: Option<PlanRevision>) -> Vec<Reply> {
        match self.change_report(target, since) {
            Ok(report) => vec![Reply::Asker(Message::PlanChanges {
                doc: target.handle(),
                regions: report.regions,
                stats: report.stats,
            })],
            Err(error) => vec![Reply::Asker(doc_error(target, error))],
        }
    }

    /// The regions and the stats themselves, for a caller that wants the records rather than a
    /// `Message` — `ubiq-plan`'s `plan_changes`, on [`Self::annotation_list`]'s own footing. Both
    /// paths come through here, so a window's decoration and an agent's report can never disagree
    /// about what changed.
    pub fn change_report(
        &mut self,
        target: &Target,
        since: Option<PlanRevision>,
    ) -> Result<ChangeReport, String> {
        if let Some(refusal) = self.refusal(target) {
            return Err(refusal);
        }
        let sidecar = self.read_sidecar(target)?.unwrap_or_default();
        let body = self.read_body(target)?;

        // A watermark past the end is the caller's own revision arriving before it hears its own
        // save echoed, or a stale handle. Clamping answers "nothing since then", which is true,
        // rather than reporting every line because the comparison went negative.
        let since = since.unwrap_or_default().min(sidecar.revision);
        let regions = super::provenance::regions(&sidecar.provenance, since, &body, &sidecar.blocks);
        let stats = super::provenance::stats(&sidecar.history, since, sidecar.revision, &regions);
        Ok(ChangeReport {
            regions,
            stats,
            blocks: sidecar.blocks,
        })
    }

    /// The revision an agent last wrote this plan at — `plan_changes`'s default watermark.
    pub fn last_written_by(&mut self, target: &Target, author: &str) -> Option<PlanRevision> {
        self.read_sidecar(target)
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
    /// invalidated by the next. Such a save records a [`super::provenance::RevisionEntry`] with all
    /// three counts at zero and stamps no line, so it costs a number and changes no answer.
    fn reindex(
        &mut self,
        target: &Target,
        previous: &str,
        body: &str,
        by: &Saver,
    ) -> Result<(PlanRevision, bool), String> {
        let mut sidecar = self.read_sidecar(target)?.unwrap_or_default();

        let matching = super::blocks::match_blocks(&sidecar.blocks, &super::blocks::blocks(body));
        let mut changed = false;
        for annotation in &mut sidecar.annotations {
            if !annotation.orphaned && matching.vanished.contains(&annotation.block_id) {
                annotation.orphaned = true;
                changed = true;
            }
        }
        sidecar.blocks = matching.blocks;

        let revision = sidecar.revision + 1;
        let diff = super::lines::diff(previous, body);
        sidecar.provenance = super::provenance::advance(&sidecar.provenance, &diff, revision, by.origin);
        sidecar.history.push(super::provenance::RevisionEntry {
            revision,
            origin: by.origin,
            author: by.author.clone(),
            at: Utc::now(),
            added: diff.added,
            removed: diff.removed,
            modified: diff.modified,
        });
        if sidecar.history.len() > super::provenance::HISTORY_LIMIT {
            let over = sidecar.history.len() - super::provenance::HISTORY_LIMIT;
            sidecar.history.drain(..over);
        }
        sidecar.revision = revision;

        sidecar.version = crate::store::plan::ANNOTATIONS_VERSION;
        self.write_sidecar(target, &sidecar)?;
        Ok((revision, changed))
    }

    /// Drop a task's plan. **No level check here**, unlike [`Self::load`] and [`Self::save`]: this
    /// is also how [`crate::work::Work::delete`] keeps a deleted task's plan from being left
    /// orphaned on disk, and a task already gone from the board cannot be asked whether it still
    /// carries a level. Broadcast only when a file actually went away, so deleting the great
    /// majority of tasks — which never had a plan — costs nothing on the bus.
    /// **A file document is refused outright**: its body is the user's own file, and the message
    /// that annotates a document may not be the one that deletes what the repository owns. The
    /// explorer's own delete is how a project file goes away, sidecar and all.
    pub fn delete(&mut self, target: &Target) -> Vec<Reply> {
        let Target::Plan { project, task } = target else {
            return vec![Reply::Asker(doc_error(
                target,
                "a project's own file is not deleted through the annotation family",
            ))];
        };
        let (project, task) = (*project, *task);
        let existed = self.store.path(project, task).exists();
        match self.store.delete(project, task) {
            Ok(()) if existed => vec![Reply::Everyone(Message::PlanDeleted {
                doc: target.handle(),
            })],
            Ok(()) => Vec::new(),
            Err(error) => vec![Reply::Asker(doc_error(target, error.to_string()))],
        }
    }

    /// A plan's sidecar, with its block index brought up to date if it has none yet — a plan
    /// written before annotations existed, or one saved by a build that did not index it. The
    /// index is what an annotation names, so a plan with a body and no index cannot be annotated
    /// at all, and re-deriving it here costs one parse on the first annotation instead of a
    /// migration.
    fn sidecar(&mut self, target: &Target) -> Result<PlanSidecar, String> {
        let sidecar = self.read_sidecar(target)?;
        if let Some(sidecar) = sidecar
            .as_ref()
            .filter(|sidecar| !sidecar.blocks.is_empty())
        {
            return Ok(sidecar.clone());
        }
        let body = self.read_body(target)?;
        if body.trim().is_empty() {
            return Ok(sidecar.unwrap_or_default());
        }
        let mut sidecar = sidecar.unwrap_or_default();
        sidecar.blocks = super::blocks::match_blocks(&[], &super::blocks::blocks(&body)).blocks;
        sidecar.version = crate::store::plan::ANNOTATIONS_VERSION;
        self.write_sidecar(target, &sidecar)?;
        Ok(sidecar)
    }

    /// Every annotation on a task's plan — open, resolved and orphaned alike — with the block
    /// index they anchor to. The filtering is the interface's; see
    /// [`Message::ListPlanAnnotations`].
    pub fn annotations(&mut self, target: &Target) -> Vec<Reply> {
        match self.annotation_list(target) {
            Ok((blocks, annotations)) => vec![Reply::Asker(Message::PlanAnnotations {
                doc: target.handle(),
                blocks,
                annotations,
            })],
            Err(error) => vec![Reply::Asker(doc_error(target, error))],
        }
    }

    /// The blocks and the annotations themselves, for a caller that wants the records rather than
    /// a `Message` — `ubiq-plan`'s MCP tools, on [`Self::body`]'s own footing.
    #[allow(clippy::type_complexity)]
    pub fn annotation_list(
        &mut self,
        target: &Target,
    ) -> Result<(Vec<PlanBlock>, Vec<Annotation>), String> {
        if let Some(refusal) = self.refusal(target) {
            return Err(refusal);
        }
        let sidecar = self.sidecar(target)?;
        Ok((sidecar.blocks, sidecar.annotations))
    }

    /// Open an annotation on one block of a task's plan. The block must be one the last save
    /// indexed: an annotation that names a block the plan does not have would arrive orphaned,
    /// which is a state to report, never one to create.
    pub fn annotate(
        &mut self,
        target: &Target,
        block: BlockId,
        quote: Option<String>,
        author: CommentAuthor,
        text: String,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err("an annotation needs some text".to_string());
            }
            if !sidecar.blocks.iter().any(|indexed| indexed.id == block) {
                return Err("no such block in this document".to_string());
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
        target: &Target,
        annotation: AnnotationId,
        author: CommentAuthor,
        text: String,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
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
        target: &Target,
        annotation: AnnotationId,
        resolved: bool,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
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
        target: &Target,
        change: impl FnOnce(&mut PlanSidecar) -> Result<(), String>,
    ) -> Vec<Reply> {
        if let Some(refusal) = self.refusal(target) {
            return vec![Reply::Asker(doc_error(target, refusal))];
        }
        let mut sidecar = match self.sidecar(target) {
            Ok(sidecar) => sidecar,
            Err(error) => return vec![Reply::Asker(doc_error(target, error))],
        };
        if let Err(error) = change(&mut sidecar) {
            return vec![Reply::Asker(doc_error(target, error))];
        }
        sidecar.version = crate::store::plan::ANNOTATIONS_VERSION;
        if let Err(error) = self.write_sidecar(target, &sidecar) {
            return vec![Reply::Asker(doc_error(target, error))];
        }
        vec![
            Reply::Asker(Message::PlanAnnotations {
                doc: target.handle(),
                blocks: sidecar.blocks,
                annotations: sidecar.annotations,
            }),
            Reply::Everyone(Message::PlanAnnotationsChanged {
                doc: target.handle(),
            }),
        ]
    }

    /// A task's plan body, or the reason it may not be read — for a caller that wants the string
    /// itself rather than a `Message`: the MCP `read_plan` tool, and the coordinator's export
    /// handler.
    pub fn body(&mut self, target: &Target) -> Result<String, String> {
        if let Some(refusal) = self.refusal(target) {
            return Err(refusal);
        }
        self.read_body(target)
    }
}

/// One failure, named against the document it is about.
pub fn doc_error(target: &Target, error: impl Into<String>) -> Message {
    let error = error.into();
    let project = target.project();
    tracing::warn!("{project}'s document: {error}");
    Message::PlanError {
        project_id: project,
        doc: Some(target.handle()),
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
            &Target::plan(project, task),
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

        let replies = plans.load(&Target::plan(project, task));
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

        let replies = plans.save(
            &Target::plan(project, task),
            "body".to_string(),
            &Saver::human(),
            None,
        );
        let error = replies
            .iter()
            .map(Reply::message)
            .find_map(|m| match m {
                Message::PlanError { error, .. } => Some(error.clone()),
                _ => None,
            })
            .expect("a task with no level is refused");
        assert!(error.contains("level"), "unexpected refusal: {error}");

        let replies = plans.load(&Target::plan(project, task));
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::PlanError { .. }))
        );
    }

    /// The block ids of a saved plan, in document order.
    fn block_ids(plans: &mut Plans, project: ProjectId, task: TaskId) -> Vec<BlockId> {
        plans
            .sidecar(&Target::plan(project, task))
            .expect("the sidecar reads")
            .blocks
            .iter()
            .map(|block| block.id)
            .collect()
    }

    fn annotations_of(plans: &mut Plans, project: ProjectId, task: TaskId) -> Vec<Annotation> {
        plans
            .annotation_list(&Target::plan(project, task))
            .expect("the annotations read")
            .1
    }

    #[test]
    fn an_annotation_round_trips_through_the_sidecar() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(
            &Target::plan(project, task),
            "# Plan\n\nStep one.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[1];

        let replies = plans.annotate(
            &Target::plan(project, task),
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
            &Target::plan(project, task),
            annotation.id,
            CommentAuthor::Agent,
            "The first one.".to_string(),
        );
        plans.resolve(&Target::plan(project, task), annotation.id, true);

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
        plans.save(
            &Target::plan(project, task),
            body.to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[1];
        plans.annotate(
            &Target::plan(project, task),
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
            &Target::plan(project, task),
            "# Plan\n\nStep one.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[2];
        plans.annotate(
            &Target::plan(project, task),
            block,
            None,
            CommentAuthor::User,
            "about step two".to_string(),
        );

        // A paragraph inserted above, and step two itself reworded: neither moves the anchor.
        plans.save(
            &Target::plan(project, task),
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
            &Target::plan(project, task),
            "# Plan\n\nStep one.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[2];
        plans.annotate(
            &Target::plan(project, task),
            block,
            Some("Step two".to_string()),
            CommentAuthor::User,
            "about step two".to_string(),
        );

        let replies = plans.save(
            &Target::plan(project, task),
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
        plans.save(
            &Target::plan(project, task),
            "# Plan".to_string(),
            &Saver::human(),
            None,
        );

        let replies = plans.annotate(
            &Target::plan(project, task),
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

        let replies = plans.annotations(&Target::plan(project, task));
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
            &Target::plan(project, task),
            "# Plan\n\nStep one.".to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[1];
        plans.annotate(
            &Target::plan(project, task),
            block,
            None,
            CommentAuthor::User,
            "a comment".to_string(),
        );

        plans.delete(&Target::plan(project, task));
        assert!(!plans.store.annotations_path(project, task).exists());
        assert!(annotations_of(&mut plans, project, task).is_empty());
    }

    #[test]
    fn deleting_a_task_with_no_plan_broadcasts_nothing() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        let replies = plans.delete(&Target::plan(project, task));
        assert!(replies.is_empty());
    }

    #[test]
    fn deleting_a_task_with_a_plan_broadcasts_that_it_is_gone() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        plans.save(
            &Target::plan(project, task),
            "body".to_string(),
            &Saver::human(),
            None,
        );

        let replies = plans.delete(&Target::plan(project, task));
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply, Reply::Everyone(Message::PlanDeleted { .. })))
        );

        let replies = plans.load(&Target::plan(project, task));
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
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));
        assert_eq!(written, 1, "the first save is revision 1");

        // A person reworks line 3 and line 5, and leaves lines 1, 2 and 4 alone.
        plans.save(
            &Target::plan(project, task),
            "# Plan\n\nStep one, revised.\n\nStep two, also revised.".to_string(),
            &Saver::human(),
            None,
        );

        let report = plans
            .change_report(&Target::plan(project, task), Some(written))
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
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        );
        plans.save(
            &Target::plan(project, task),
            "# Plan\n\nStep one, revised.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );

        assert_eq!(
            plans.last_written_by(&Target::plan(project, task), "agent-1"),
            Some(1),
            "the revision this agent last wrote at",
        );
        assert_eq!(
            plans.last_written_by(&Target::plan(project, task), "agent-2"),
            None,
            "an agent that never wrote this plan has no watermark of its own",
        );
    }

    #[test]
    fn a_human_save_and_an_agent_save_are_told_apart_on_disk() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);

        plans.save(
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        );
        plans.save(
            &Target::plan(project, task),
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
            &Target::plan(project, task),
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

        let replies = plans.load(&Target::plan(project, task));
        assert_eq!(
            revision_of(&replies),
            0,
            "a plan nobody has written stands at revision 0",
        );

        plans.save(
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::human(),
            None,
        );
        assert_eq!(revision_of(&plans.load(&Target::plan(project, task),)), 1);
    }

    #[test]
    fn a_watermark_at_the_current_revision_reports_no_change() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let written = revision_of(&plans.save(
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));

        let report = plans
            .change_report(&Target::plan(project, task), Some(written))
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
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));

        // The human deletes "Step one." and the blank line under it.
        plans.save(
            &Target::plan(project, task),
            "# Plan\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );

        let report = plans
            .change_report(&Target::plan(project, task), Some(written))
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
        plans.save(
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::human(),
            None,
        );
        let block = block_ids(&mut plans, project, task)[1];
        plans.annotate(
            &Target::plan(project, task),
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
            .change_report(&Target::plan(project, task), None)
            .expect("the changes read");
        assert!(
            report.regions.is_empty(),
            "an unstamped document is not reported as freshly written",
        );

        // The next save stamps only what it actually changed, and does not claim the rest.
        plans.save(
            &Target::plan(project, task),
            "# Plan\n\nStep one, revised.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        let report = plans
            .change_report(&Target::plan(project, task), Some(0))
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
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::agent("agent-1"),
            None,
        ));
        let second = revision_of(&plans.save(
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::human(),
            None,
        ));

        assert_eq!(
            (first, second),
            (1, 2),
            "the counter follows saves, so a watermark once handed out stays meaningful",
        );
        let report = plans
            .change_report(&Target::plan(project, task), Some(first))
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
            &Target::plan(project, task),
            "# Plan\n\nAs it was.".to_string(),
            &Saver::human(),
            None,
        ));

        // Both windows hold `seeded` and both mean to replace it.
        let first = plans.save(
            &Target::plan(project, task),
            "# Plan\n\nThe first window's.".to_string(),
            &Saver::human(),
            Some(seeded),
        );
        let landed = revision_of(&first);
        assert_eq!(landed, seeded + 1, "the first save is an ordinary one");
        assert!(conflict_of(&first).is_none());

        let second = plans.save(
            &Target::plan(project, task),
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
            plans
                .body(&Target::plan(project, task),)
                .expect("the body reads"),
            "# Plan\n\nThe first window's.",
            "and the winner's body is untouched",
        );

        // The second window is shown that, presses Overwrite, and names what it was told.
        let again = plans.save(
            &Target::plan(project, task),
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
            plans
                .body(&Target::plan(project, task),)
                .expect("the body reads"),
            "# Plan\n\nThe second window's.",
        );
    }

    /// An agent's `write_plan` is the other racer, and the refusal names it as one — the banner
    /// the window draws says "an agent" or "somebody else", and it reads that from here.
    #[test]
    fn a_refusal_names_an_agent_that_moved_the_copy() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let seeded = revision_of(&plans.save(
            &Target::plan(project, task),
            PLANNED.to_string(),
            &Saver::human(),
            None,
        ));
        plans.save(
            &Target::plan(project, task),
            "# Plan\n\nThe agent's.".to_string(),
            &Saver::agent("agent-1"),
            None,
        );

        let refused = plans.save(
            &Target::plan(project, task),
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

        let fresh = plans.save(
            &Target::plan(project, task),
            "mine".to_string(),
            &Saver::human(),
            Some(0),
        );
        assert!(conflict_of(&fresh).is_none(), "nothing stood there");
        assert_eq!(revision_of(&fresh), 1);

        let beaten = plans.save(
            &Target::plan(project, task),
            "also mine".to_string(),
            &Saver::human(),
            Some(0),
        );
        assert_eq!(conflict_of(&beaten).map(|(revision, _)| revision), Some(1));
    }

    // ── a project's own markdown file, annotated in place ─────────────

    /// A project root with `notes.md` in it, and the target that names it. The plan store's own
    /// temp directory rides along unused: a file document never touches it.
    fn file_target(dir: &tempfile::TempDir, project: ProjectId, rel: &str) -> Target {
        let doc = DocumentHandle::File {
            project_id: project,
            rel_path: rel.to_string(),
        };
        Target::resolve(&doc, dir.path()).expect("the path resolves inside the project")
    }

    #[test]
    fn a_file_document_is_saved_in_place_with_its_sidecar_beside_it() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "").unwrap();
        let target = file_target(&repo, project, "notes.md");

        let replies = plans.save(
            &target,
            "# Notes\n\nFirst thought.".to_string(),
            &Saver::human(),
            Some(0),
        );
        assert_eq!(revision_of(&replies), 1);

        // The body is the user's own file, written where it already was.
        assert_eq!(
            std::fs::read_to_string(repo.path().join("notes.md")).unwrap(),
            "# Notes\n\nFirst thought."
        );
        // And the sidecar is beside it, named by appending rather than replacing the extension.
        let sidecar = repo.path().join("notes.md.annotation.json");
        assert!(sidecar.exists(), "the sidecar sits beside the file");
        assert!(
            !repo.path().join("notes.annotation.json").exists(),
            "the `.md` was not substituted away",
        );

        // Nothing of Ubiq's went into the config root for a file document.
        assert!(!_dir.path().join("projects").exists());

        let (blocks, _) = plans
            .annotation_list(&target)
            .expect("the block index reads");
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn a_file_documents_annotation_orphans_on_the_same_rule_as_a_plans() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "").unwrap();
        let target = file_target(&repo, project, "notes.md");

        plans.save(
            &target,
            "# Notes\n\nFirst thought.\n\nSecond thought.".to_string(),
            &Saver::human(),
            None,
        );
        let (blocks, _) = plans.annotation_list(&target).unwrap();
        let second = blocks[2].id;
        plans.annotate(
            &target,
            second,
            Some("Second".to_string()),
            CommentAuthor::User,
            "is this still true?".to_string(),
        );

        // The passage goes away, and the thread is flagged rather than dropped — the matcher is
        // the plan's own, so this is the plan's own behaviour.
        plans.save(
            &target,
            "# Notes\n\nFirst thought.".to_string(),
            &Saver::human(),
            None,
        );
        let (_, annotations) = plans.annotation_list(&target).unwrap();
        assert_eq!(annotations.len(), 1);
        assert!(annotations[0].orphaned);
        assert_eq!(annotations[0].quote.as_deref(), Some("Second"));

        // The provenance layer rode in the same sidecar and answers for the same file.
        let report = plans.change_report(&target, Some(1)).unwrap();
        assert!(!report.regions.is_empty());
    }

    #[test]
    fn a_file_document_arbitrates_a_race_the_way_a_plan_does() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "").unwrap();
        let target = file_target(&repo, project, "notes.md");

        plans.save(&target, "mine".to_string(), &Saver::human(), Some(0));
        let beaten = plans.save(&target, "also mine".to_string(), &Saver::human(), Some(0));
        assert_eq!(conflict_of(&beaten).map(|(revision, _)| revision), Some(1));
    }

    #[test]
    fn only_a_markdown_file_inside_the_project_resolves() {
        let repo = tempfile::tempdir().unwrap();
        let project = ProjectId::generate();

        let image = DocumentHandle::File {
            project_id: project,
            rel_path: "logo.png".to_string(),
        };
        assert!(Target::resolve(&image, repo.path()).is_err());

        let outside = DocumentHandle::File {
            project_id: project,
            rel_path: "../elsewhere.md".to_string(),
        };
        assert!(Target::resolve(&outside, repo.path()).is_err());
    }

    #[test]
    fn a_file_document_is_never_deleted_through_the_family() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "# Notes").unwrap();
        let target = file_target(&repo, project, "notes.md");

        let replies = plans.delete(&target);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::PlanError { .. }))
        );
        assert!(
            repo.path().join("notes.md").exists(),
            "the user's file is still there",
        );
    }

    #[test]
    fn a_task_with_no_level_is_refused_its_changes() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_ordinary_task(&work, project);

        let replies = plans.changes(&Target::plan(project, task), None);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::PlanError { .. }))
        );
    }
}
