//! The harness-only body of `plan`: `Plans`, `Handle`, `Target` and `Saver`, split from
//! `super::blocks`, `super::lines` and `super::provenance` because those three read and match a
//! plan's sidecar with no dependency on `crate::work` at all, while everything here checks a
//! task's level through it — see the parent module's doc comment for what this is and why.
//!
//! Nested so `crate::plan::Target`, `crate::plan::Plans` and the rest keep their paths: this
//! module is declared `#[cfg(feature = "harness")]` and re-exported with `pub use` in
//! `super`, so nothing outside the crate can tell it moved.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;
use ubiq_proto::ids::{AnnotationId, BlockId, CommentId, ProjectId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::plan::{
    Annotation, AnnotationMark, AnnotationState, BlockHighlight, DocBinding, DocumentHandle, HighlightColour, PlanBlock, PlanChangeStats, PlanChangedRegion,
    PlanRevision, SaveOrigin,
};
use ubiq_proto::work::{Addressee, AgentId, CommentAuthor, Level};

use super::queue::{QueuedRef, ThreadView};
use crate::reply::Reply;
use crate::store::mission::MissionStore;
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
    /// A mission document beyond the plan (M8) — `name` is a bare document name, never a path.
    MissionDoc {
        project: ProjectId,
        task: TaskId,
        name: String,
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
            DocumentHandle::MissionDoc {
                project_id,
                task_id,
                name,
            } => {
                // Flat, and never a path: `docs/` holds no nesting (M8), so a name naming one is
                // refused here rather than resolving somewhere the mission's directory does not
                // reach.
                if name.trim().is_empty()
                    || name.contains(['/', '\\'])
                    || name == "."
                    || name == ".."
                {
                    return Err(
                        "a mission document's name may not contain a path separator".to_string()
                    );
                }
                Ok(Target::MissionDoc {
                    project: *project_id,
                    task: *task_id,
                    name: name.clone(),
                })
            }
        }
    }

    pub fn project(&self) -> ProjectId {
        match self {
            Target::Plan { project, .. }
            | Target::File { project, .. }
            | Target::MissionDoc { project, .. } => *project,
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
            Target::MissionDoc {
                project,
                task,
                name,
            } => DocumentHandle::MissionDoc {
                project_id: *project,
                task_id: *task,
                name: name.clone(),
            },
        }
    }

    fn placement(&self) -> Placement {
        match self {
            // Under the config root, exactly as a plan is — Ubiq owns the directories above it.
            Target::Plan { .. } | Target::MissionDoc { .. } => Placement::ConfigRoot,
            Target::File { .. } => Placement::InsideProject,
        }
    }
}

/// Move every thread whose block split into the part its lineage says it belongs in (`D208`): the
/// one part holding its quote, else the first part. Answers whether anything moved.
fn follow_lineage(
    annotations: &mut [Annotation],
    children: &HashMap<BlockId, Vec<BlockId>>,
    blocks: &[PlanBlock],
) -> bool {
    let normalise = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut moved = false;
    for annotation in annotations.iter_mut() {
        let Some(parts) = children.get(&annotation.block_id) else {
            continue;
        };
        let holding: Vec<BlockId> = match annotation.quote.as_deref().map(normalise) {
            Some(quote) if !quote.is_empty() => parts
                .iter()
                .copied()
                .filter(|part| {
                    blocks
                        .iter()
                        .any(|b| b.id == *part && normalise(&b.text).contains(&quote))
                })
                .collect(),
            _ => Vec::new(),
        };
        let to = match holding.as_slice() {
            [one] => *one,
            _ => parts[0],
        };
        if to != annotation.block_id || annotation.orphaned {
            annotation.block_id = to;
            annotation.orphaned = false;
            moved = true;
        }
    }
    moved
}

/// Re-anchor every thread that quotes a passage by **where the passage now is**, after a save's
/// block matching (`D208`). A block split in two keeps its id on one half only; a thread whose
/// quote sits in the other half moves there, and a thread orphaned because its block vanished is
/// brought back when its quote survives somewhere. **Only a unique match moves a thread** —
/// `annotate_doc`'s own rule for a quote: a quote two blocks hold is ambiguous, and a wrong anchor
/// is worse than an admitted missing one. A thread with no quote, or whose quote is gone or
/// ambiguous, keeps what the matching decided. Answers whether anything moved.
fn follow_quotes(annotations: &mut [Annotation], blocks: &[PlanBlock]) -> bool {
    let normalise = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut moved = false;
    for annotation in annotations.iter_mut() {
        let Some(quote) = annotation.quote.as_deref().map(normalise) else {
            continue;
        };
        if quote.is_empty() {
            continue;
        }
        let holds = |block: &PlanBlock| normalise(&block.text).contains(&quote);
        let here = !annotation.orphaned
            && blocks
                .iter()
                .any(|block| block.id == annotation.block_id && holds(block));
        if here {
            continue;
        }
        let mut found = blocks.iter().filter(|block| holds(block));
        if let (Some(block), None) = (found.next(), found.next()) {
            annotation.block_id = block.id;
            annotation.orphaned = false;
            moved = true;
        }
    }
    moved
}

/// Where a mission document's body sits, from the config root alone — the same `docs/` subtree
/// [`crate::store::mission::MissionStore::docs_dir`] computes, reached without holding a full
/// `MissionStore`: `Plans` already holds the config root through its own [`FilePlanStore`], and a
/// mission document is not otherwise this module's to store.
fn mission_doc_path(root: &Path, project: ProjectId, task: TaskId, name: &str) -> PathBuf {
    MissionStore::new(root.to_path_buf())
        .docs_dir(project, task)
        .join(format!("{name}.md"))
}

/// The opening line of a thread posted for an agent's passage a merge kept the user's text over.
const PROPOSED: &str = "Proposed (conflicted with your edit):";
/// The same, for a passage another window saved.
const FROM_ELSEWHERE: &str = "Saved elsewhere (conflicted with your edit):";
/// The same, for a passage nothing in Ubiq wrote — an editor outside it.
const FROM_OUTSIDE: &str = "Edited outside Ubiq (conflicted with your edit):";

/// An agent's write, merged and ready to commit ([`Plans::agent_commit`]).
pub struct AgentWrite {
    /// What will be written.
    pub body: String,
    /// The passages the user's text won, each carrying the agent's losing version.
    pub conflicts: Vec<ubiq_proto::merge::Conflict>,
    /// A base was on record, so the merge stands in for the revision check.
    pub based: bool,
}

impl AgentWrite {
    /// Merge `body` from `base` against `current` — pure, so it runs with the lock released.
    pub fn prepare(base: Option<&str>, current: Option<&str>, body: &str) -> Self {
        match (base, current) {
            (Some(base), Some(current)) => {
                let merged = ubiq_proto::merge::merge3(base, current, body);
                Self {
                    body: merged.text,
                    conflicts: merged.conflicts,
                    based: true,
                }
            }
            _ => Self {
                body: body.to_string(),
                conflicts: Vec::new(),
                based: false,
            },
        }
    }
}

/// What an agent's committed write came to.
pub struct AgentSaved {
    pub replies: Vec<Reply>,
    /// Threads made from the passages the user's text won.
    pub threads: Vec<AnnotationId>,
    /// What was saved is not what the agent sent — merged with newer edits.
    pub merged: bool,
}

/// One project's plans.
pub struct Plans {
    store: FilePlanStore,
    /// To check a task exists and carries a [`ubiq_proto::work::Level`] before a plan is loaded,
    /// saved or exported. See the module doc for why this does not make `crate::mcp::PlanReach`
    /// hold a work handle of its own.
    work: work::Handle,
    /// The body each agent last read or wrote, per document — the base an agent's next write is
    /// merged from (`agent_save`). Keyed by the agent's MCP key and `DocumentHandle::key`. In
    /// memory only: a host restart forgets it, and a write with no base is a plain save.
    agent_bases: HashMap<(String, String), String>,
    /// Each document's lineage codes by block id (`super::lineage`, `D208`), keyed by
    /// `DocumentHandle::key`. In memory only — minted flat when absent, carried across every save,
    /// and dropped by [`Self::reopen_lineage`] so the next answer mints them flat again.
    lineage: HashMap<String, HashMap<BlockId, String>>,
}

/// A cloneable handle to [`Plans`], on [`crate::work::Handle`]'s own footing: the coordinator
/// holds one clone, the MCP listener's `ubiq-plan` server holds another, and the lock covers one
/// call and is released before anything reaches the bus.
#[derive(Clone)]
pub struct Handle(Arc<Mutex<Plans>>);

impl Handle {
    /// An agent's write (`D208`), **merged outside the lock**: the base and the disk are read under
    /// it, the merge — the one slow step — runs with it released, and the commit takes it again and
    /// is refused if the disk moved meanwhile, in which case the merge is made again. A document
    /// that keeps moving gets the whole thing under one hold on the last attempt.
    pub fn agent_save(
        &self,
        target: &Target,
        body: &str,
        agent: &str,
        expected: Option<PlanRevision>,
    ) -> AgentSaved {
        for _ in 0..3 {
            let (base, current) = self.lock().agent_merge_inputs(target, agent);
            let prepared = AgentWrite::prepare(base.as_deref(), current.as_deref(), body);
            if let Some(saved) = self.lock().agent_commit(
                target,
                body,
                agent,
                expected,
                current.as_deref(),
                prepared,
            ) {
                return saved;
            }
        }
        self.lock()
            .agent_save(target, body.to_string(), agent, expected)
    }

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
        Self {
            store,
            work,
            agent_bases: HashMap::new(),
            lineage: HashMap::new(),
        }
    }

    /// Why this document may not be read or written, if there is a reason.
    ///
    /// A plan's reason is the task's: no such task, or a task whose `level` is `None`. A mission
    /// document's is narrower — no such task, or a task that is not **a mission**, since carrying
    /// some level is not enough for a document that lives under `missions/<TaskId>/docs/`. A file
    /// document has none left — being markdown and inside the project was settled when the
    /// [`Target`] was resolved, and a file that is not there yet reads as an empty body the same
    /// way an unplanned mission does.
    fn refusal(&mut self, target: &Target) -> Option<String> {
        let (project, task, needs_mission) = match target {
            Target::Plan { project, task } => (*project, *task, false),
            Target::MissionDoc { project, task, .. } => (*project, *task, true),
            Target::File { .. } => return None,
        };
        let (_, tasks) = self.work.lock().tasks(project);
        let Some(record) = tasks.iter().find(|t| t.id == task) else {
            return Some("no such task".to_string());
        };
        if needs_mission {
            if record.level != Some(Level::Mission) {
                return Some("only a mission can carry a mission document".to_string());
            }
        } else if record.level.is_none() {
            return Some("only a task with a level can carry a plan".to_string());
        }
        None
    }

    /// Where the body sits — the store's own path for a plan, the resolved file for a document in
    /// the project's tree, the mission's own `docs/` subtree for a mission document.
    fn body_path(&self, target: &Target) -> PathBuf {
        match target {
            Target::Plan { project, task } => self.store.path(*project, *task),
            Target::File { body, .. } => body.clone(),
            Target::MissionDoc {
                project,
                task,
                name,
            } => mission_doc_path(self.store.root(), *project, *task, name),
        }
    }

    /// Where the sidecar sits: `<TaskId>.annotations.json` for a plan, `<file>.md.annotation.json`
    /// beside the body for a document in the project's tree or a mission document alike — the
    /// mission document has no path of its own the way a plan's `<TaskId>.annotations.json` does,
    /// so it takes the file document's own naming instead of inventing a third spelling.
    fn sidecar_path(&self, target: &Target) -> PathBuf {
        match target {
            Target::Plan { project, task } => self.store.annotations_path(*project, *task),
            Target::File { body, .. } => sidecar_beside(body),
            Target::MissionDoc { .. } => sidecar_beside(&self.body_path(target)),
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

    /// Replace a document's sidecar, whole and atomically.
    ///
    /// **Unconditional, on purpose.** The block index this carries is what keeps a `BlockId`
    /// stable across two separate calls — `annotation_list()` computes it fresh from the body
    /// when none is on disk yet, and an `AnnotatePlan` that follows re-reads the sidecar to check
    /// the id it was given still names a block; skipping this write between the two would re-mint
    /// every id on the second read and refuse the very first annotation ever made on a document,
    /// every time. A save's revision and provenance are the same story: `Plans::save()`'s
    /// watermark and conflict arbitration are only as real as what a *second* call reads back, so
    /// they are only real if this wrote them. T-183's fix — no stale-source warning for a document
    /// with nothing annotated — reads real annotation content instead of this file's mere
    /// presence (`AppState::has_annotations`); it does not, and cannot without breaking the above,
    /// live in whether this write happens.
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

    /// Remember what an agent just read, as the base its next write is merged from.
    pub fn note_agent_read(&mut self, target: &Target, agent: &str, body: &str) {
        self.agent_bases
            .insert((agent.to_string(), target.handle().key()), body.to_string());
    }

    /// What an agent's write is merged against, read under the lock: the body the agent last read
    /// or wrote (`None` when nothing is on record — a host restart, a first write) and the body on
    /// disk now (`None` when the document may not be read).
    pub fn agent_merge_inputs(
        &mut self,
        target: &Target,
        agent: &str,
    ) -> (Option<String>, Option<String>) {
        let base = self
            .agent_bases
            .get(&(agent.to_string(), target.handle().key()))
            .cloned();
        let current = if self.refusal(target).is_none() {
            self.read_body(target).ok()
        } else {
            None
        };
        (base, current)
    }

    /// An agent's write, **merged rather than replacing** whatever landed since the agent last
    /// read or wrote the document (`D208`), all under one hold of the lock — what
    /// [`Handle::agent_save`] falls back to when the document keeps moving under its unlocked
    /// merge, and what a test drives directly.
    pub fn agent_save(
        &mut self,
        target: &Target,
        body: String,
        agent: &str,
        expected: Option<PlanRevision>,
    ) -> AgentSaved {
        let (base, current) = self.agent_merge_inputs(target, agent);
        let prepared = AgentWrite::prepare(base.as_deref(), current.as_deref(), &body);
        self.agent_commit(target, &body, agent, expected, current.as_deref(), prepared)
            .expect("nothing moved: the read and the commit share one hold of the lock")
    }

    /// Commit a write [`AgentWrite::prepare`] merged — **refused, writing nothing, when the disk is
    /// no longer `read`**, the body the merge was made against. Every write Ubiq makes to an
    /// annotated document holds this lock (an agent's here, a window's `SavePlan`, a tab's save
    /// through [`Self::file_save`]), so the check and the write cannot be split by another of them.
    ///
    /// Base is the body the agent last read or wrote, ours the disk, theirs the agent's body.
    /// Disjoint edits all land; where both changed the same words **the user's text wins** and the
    /// agent's version becomes a thread on that block, authored by the agent. With no base on
    /// record the write is the plain [`Self::save`] with `expected` as given.
    ///
    /// **The base kept afterwards is what the agent sent**, not what was saved: it is the text the
    /// agent believes the document holds, so its next write — built on its own text, or on the
    /// merged body it was handed back — merges the user's edits in again rather than reverting
    /// them. A read moves the base to the disk.
    pub fn agent_commit(
        &mut self,
        target: &Target,
        sent: &str,
        agent: &str,
        expected: Option<PlanRevision>,
        read: Option<&str>,
        prepared: AgentWrite,
    ) -> Option<AgentSaved> {
        let (_, current) = self.agent_merge_inputs(target, agent);
        if current.as_deref() != read {
            return None;
        }
        let expected = if prepared.based { None } else { expected };
        let merged = prepared.body != sent;
        let mut replies = self.save(target, prepared.body, &Saver::agent(agent), expected);
        let saved = replies
            .iter()
            .any(|reply| matches!(reply.message(), Message::Plan { .. }));
        if !saved {
            return Some(AgentSaved {
                replies,
                threads: Vec::new(),
                merged: false,
            });
        }
        self.agent_bases
            .insert((agent.to_string(), target.handle().key()), sent.to_string());
        let (posted, threads) = self.post_proposals(
            target,
            &prepared.conflicts,
            CommentAuthor::Agent,
            PROPOSED,
            false,
        );
        // Only the broadcast travels on: the asker of a write wants the body, not a thread list.
        replies.extend(
            posted
                .into_iter()
                .filter(|reply| matches!(reply, Reply::Everyone(_))),
        );
        Some(AgentSaved {
            replies,
            threads,
            merged,
        })
    }

    /// Post each contested passage's losing side as a thread on its block, unless that block
    /// already carries a thread saying exactly this — a merge made twice over the same edit must
    /// not say it twice. `by_theirs` anchors by the losing side's text (what is on disk, for a
    /// window's claim) rather than the winning side's.
    fn post_proposals(
        &mut self,
        target: &Target,
        conflicts: &[ubiq_proto::merge::Conflict],
        author: CommentAuthor,
        label: &str,
        by_theirs: bool,
    ) -> (Vec<Reply>, Vec<AnnotationId>) {
        let mut replies = Vec::new();
        let mut threads = Vec::new();
        for conflict in conflicts {
            let theirs = conflict.theirs.trim();
            if theirs.is_empty() {
                continue;
            }
            let (anchor, other) = if by_theirs {
                (&conflict.theirs, &conflict.ours)
            } else {
                (&conflict.ours, &conflict.theirs)
            };
            let Some(block) = self.anchor_block(target, anchor, other) else {
                continue;
            };
            let text = format!("{label}\n\n{theirs}");
            let before: Vec<Annotation> = self
                .annotation_list(target)
                .map(|(_, annotations, _)| annotations)
                .unwrap_or_default();
            let said = before.iter().any(|a| {
                a.block_id == block && a.thread.iter().any(|comment| comment.text == text)
            });
            if said {
                continue;
            }
            let made = self.annotate(target, block, None, author, text, Vec::new(), None);
            for reply in &made {
                if let Message::PlanAnnotations { annotations, .. } = reply.message() {
                    threads.extend(
                        annotations
                            .iter()
                            .map(|a| a.id)
                            .filter(|id| !before.iter().any(|a| a.id == *id)),
                    );
                }
            }
            replies.extend(made);
        }
        (replies, threads)
    }

    /// A window kept the user's words over passages another writer put on disk (`D208`,
    /// `Message::DocMergeConflicts`): post each such passage as a thread on its block.
    ///
    /// A claim is only honoured when its `theirs` is in the body as it stands on disk now — the
    /// window cannot invent text and have it posted under another author. **Who wrote it is read
    /// per passage** from the provenance stamps ([`Self::passage_origin`]): an agent's lines are
    /// posted as the agent's proposal, another window's as the user's, and a passage the stamps
    /// cannot vouch for (an edit made outside Ubiq) as an outside edit — never guessed at.
    pub fn disk_conflicts(
        &mut self,
        target: &Target,
        conflicts: Vec<ubiq_proto::merge::Conflict>,
    ) -> Vec<Reply> {
        let Ok(disk) = self.body(target) else {
            return Vec::new();
        };
        let disk = disk.replace("\r\n", "\n");
        let mut replies = Vec::new();
        for conflict in conflicts {
            let theirs = conflict.theirs.replace("\r\n", "\n");
            if theirs.trim().is_empty() || !disk.contains(theirs.trim()) {
                continue;
            }
            let (author, label) = match self.passage_origin(target, &disk, theirs.trim()) {
                Some(SaveOrigin::Agent) => (CommentAuthor::Agent, PROPOSED),
                Some(SaveOrigin::Human) => (CommentAuthor::User, FROM_ELSEWHERE),
                None => (CommentAuthor::User, FROM_OUTSIDE),
            };
            let (posted, _) = self.post_proposals(target, &[conflict], author, label, true);
            replies.extend(posted);
        }
        replies
    }

    /// Who last wrote every line of `passage`, as the provenance stamps say — `None` when the lines
    /// disagree, a line carries no stamp, or the disk is no longer the body the stamps were made
    /// for (its blocks differ from the last indexed ones: something wrote it outside Ubiq).
    fn passage_origin(&mut self, target: &Target, disk: &str, passage: &str) -> Option<SaveOrigin> {
        let sidecar = self.read_sidecar(target).ok()??;
        let now = super::blocks::blocks(disk);
        if now.len() != sidecar.blocks.len()
            || now
                .iter()
                .zip(&sidecar.blocks)
                .any(|(block, indexed)| block.text != indexed.text)
        {
            return None;
        }
        let at = disk.find(passage)?;
        let first = disk[..at].matches('\n').count() as u32 + 1;
        let last = first + passage.matches('\n').count() as u32;
        let mut origin = None;
        for line in first..=last {
            let run = sidecar
                .provenance
                .iter()
                .find(|run| run.first_line <= line && line <= run.last_line)?;
            match origin {
                None => origin = Some(run.origin),
                Some(seen) if seen != run.origin => return None,
                _ => {}
            }
        }
        origin
    }

    /// Whether this document has a sidecar — an annotated document, whose every write goes
    /// through this lock and stamps provenance (`D208`).
    pub fn is_annotated(&self, target: &Target) -> bool {
        self.sidecar_path(target).exists()
    }

    /// A window's ordinary tab save of an annotated markdown file (`WriteProjectFile`), made under
    /// this lock and stamped as the user's (`D208`): the file is written with the tab's own
    /// version check exactly as the file worker would, then re-indexed — so the block index, the
    /// threads and the provenance follow a tab save the way they follow a `SavePlan`, and an
    /// agent's merge can never land between this write's check and its write.
    pub fn file_save(
        &mut self,
        target: &Target,
        root: &Path,
        rel_path: &str,
        bytes: &[u8],
        expected: Option<ubiq_proto::files::FileVersion>,
        overwrite: bool,
    ) -> Result<(ubiq_proto::files::FileVersion, Vec<Reply>), ubiq_proto::files::FileError> {
        let previous = self.read_body(target).unwrap_or_default();
        let version = crate::files::save(root, rel_path, bytes, expected, overwrite)?;
        let body = String::from_utf8_lossy(bytes).into_owned();
        let mut replies = Vec::new();
        // The body is on disk either way; a sidecar that will not take the stamp leaves the index
        // where every pre-`D208` tab save left it.
        if let Ok((revision, changed)) = self.reindex(target, &previous, &body, &Saver::human()) {
            replies.push(Reply::Everyone(Message::PlanChanged {
                doc: target.handle(),
                revision,
                origin: SaveOrigin::Human,
            }));
            if changed {
                replies.push(Reply::Everyone(Message::PlanAnnotationsChanged {
                    doc: target.handle(),
                }));
            }
        }
        Ok((version, replies))
    }

    /// The block a contested region belongs to: the first whose text holds one of the lines only
    /// the user's side has (where the two actually disagree), then any of the region's lines, else
    /// the document's first block — a thread is never dropped for want of a place.
    fn anchor_block(&mut self, target: &Target, ours: &str, theirs: &str) -> Option<BlockId> {
        let normalise = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let (blocks, _, _) = self.annotation_list(target).ok()?;
        let theirs = normalise(theirs);
        let (only_ours, shared): (Vec<String>, Vec<String>) = ours
            .lines()
            .map(normalise)
            .filter(|line| !line.is_empty())
            .partition(|line| !theirs.contains(line.as_str()));
        only_ours
            .into_iter()
            .chain(shared)
            .find_map(|line| {
                blocks
                    .iter()
                    .find(|block| normalise(&block.text).contains(&line))
            })
            .or_else(|| blocks.first())
            .map(|block| block.id)
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
        let regions =
            super::provenance::regions(&sidecar.provenance, since, &body, &sidecar.blocks);
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
        // The codes follow the save, and a thread on a block that split follows its lineage into
        // the part that holds its quote — before the document-wide quote rule below.
        let previous_blocks = sidecar.blocks.clone();
        let codes = {
            let codes = self.lineage.entry(target.handle().key()).or_default();
            let order: Vec<BlockId> = previous_blocks.iter().map(|block| block.id).collect();
            super::lineage::fill(codes, &order);
            codes.clone()
        };
        let seen = |blocks: &[PlanBlock]| -> Vec<(BlockId, String)> {
            blocks.iter().map(|b| (b.id, b.text.clone())).collect()
        };
        let (prev, next) = (seen(&previous_blocks), seen(&matching.blocks));
        let carried = super::lineage::carry(
            &prev.iter().map(|(id, t)| (*id, t.as_str())).collect::<Vec<_>>(),
            &next.iter().map(|(id, t)| (*id, t.as_str())).collect::<Vec<_>>(),
            &codes,
        );
        changed |= follow_lineage(&mut sidecar.annotations, &carried.children, &matching.blocks);
        self.lineage.insert(target.handle().key(), carried.codes);
        changed |= follow_quotes(&mut sidecar.annotations, &matching.blocks);
        let before = sidecar.highlights.len();
        sidecar
            .highlights
            .retain(|highlight| !matching.vanished.contains(&highlight.block_id));
        changed |= sidecar.highlights.len() != before;
        changed |= sidecar.blocks != matching.blocks;
        sidecar.blocks = matching.blocks;

        let revision = sidecar.revision + 1;
        let diff = super::lines::diff(previous, body);
        sidecar.provenance =
            super::provenance::advance(&sidecar.provenance, &diff, revision, by.origin);
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
        if let Some(refusal) = self.refusal(target) {
            return vec![Reply::Asker(doc_error(target, refusal))];
        }
        match self.sidecar(target) {
            Ok(mut sidecar) => {
                self.code(target, &mut sidecar.blocks);
                vec![Reply::Asker(snapshot(target, sidecar))]
            }
            Err(error) => vec![Reply::Asker(doc_error(target, error))],
        }
    }

    /// The agent the document is bound to and its auto-send switch (`D207`), read without
    /// re-indexing anything. Unbound for a document with no sidecar yet.
    pub fn binding(&mut self, target: &Target) -> Result<DocBinding, String> {
        if let Some(refusal) = self.refusal(target) {
            return Err(refusal);
        }
        Ok(self
            .read_sidecar(target)?
            .map(|sidecar| sidecar.binding())
            .unwrap_or_default())
    }

    /// The blocks and the annotations themselves, for a caller that wants the records rather than
    /// a `Message` — `ubiq-plan`'s MCP tools, on [`Self::body`]'s own footing.
    #[allow(clippy::type_complexity)]
    pub fn annotation_list(
        &mut self,
        target: &Target,
    ) -> Result<(Vec<PlanBlock>, Vec<Annotation>, Vec<BlockHighlight>), String> {
        if let Some(refusal) = self.refusal(target) {
            return Err(refusal);
        }
        let mut sidecar = self.sidecar(target)?;
        self.code(target, &mut sidecar.blocks);
        Ok((sidecar.blocks, sidecar.annotations, sidecar.highlights))
    }

    /// Stamp each block with its lineage code (`D208`) — on a copy bound for an answer, never on
    /// the sidecar's own, which is written without them. Blocks with no code yet get one here.
    fn code(&mut self, target: &Target, blocks: &mut [PlanBlock]) {
        let codes = self.lineage.entry(target.handle().key()).or_default();
        let order: Vec<BlockId> = blocks.iter().map(|block| block.id).collect();
        super::lineage::fill(codes, &order);
        for block in blocks {
            block.lineage = codes.get(&block.id).cloned().unwrap_or_default();
        }
    }

    /// The document was opened again: its codes are forgotten, so the next answer mints them flat
    /// — the compaction a long editing session's `AAB.AB.AA` codes are owed.
    pub fn reopen_lineage(&mut self, target: &Target) {
        self.lineage.remove(&target.handle().key());
    }

    /// The one block whose text contains `quote`, whitespace-normalised on both sides. Refused
    /// when no block has it or several do: an agent anchoring by passage must say which block.
    pub fn block_for_quote(&mut self, target: &Target, quote: &str) -> Result<BlockId, String> {
        let normalise = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let wanted = normalise(quote);
        if wanted.is_empty() {
            return Err("quote must not be empty".to_string());
        }
        let (blocks, _, _) = self.annotation_list(target)?;
        let mut found = blocks
            .iter()
            .filter(|block| normalise(&block.text).contains(&wanted));
        match (found.next(), found.next()) {
            (Some(block), None) => Ok(block.id),
            (None, _) => Err("no block of the plan contains that quote".to_string()),
            _ => Err("more than one block contains that quote: quote more of the passage, or pass block_id".to_string()),
        }
    }

    /// Open an annotation on one block of a task's plan. The block must be one the last save
    /// indexed: an annotation that names a block the plan does not have would arrive orphaned,
    /// which is a state to report, never one to create.
    #[allow(clippy::too_many_arguments)]
    pub fn annotate(
        &mut self,
        target: &Target,
        block: BlockId,
        quote: Option<String>,
        author: CommentAuthor,
        text: String,
        marks: Vec<AnnotationMark>,
        to: Option<Addressee>,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err("an annotation needs some text".to_string());
            }
            if !sidecar.blocks.iter().any(|indexed| indexed.id == block) {
                return Err("no such block in this document".to_string());
            }
            let now = Utc::now();
            let mut annotation = Annotation::new(
                block,
                quote
                    .map(|quote| quote.trim().to_string())
                    .filter(|quote| !quote.is_empty()),
                author,
                text,
                now,
            );
            annotation.thread[0].to = to;
            for mark in marks {
                annotation.set_mark(mark, true);
            }
            if to == Some(Addressee::Agent) {
                annotation.set_mark(AnnotationMark::Agent, true);
            }
            sidecar.annotations.push(annotation);
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
        to: Option<Addressee>,
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
            found.reply_to(author, text, Utc::now(), to);
            Ok(())
        })
    }

    /// Set or clear one mark on an annotation.
    pub fn set_mark(
        &mut self,
        target: &Target,
        annotation: AnnotationId,
        mark: AnnotationMark,
        on: bool,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            let found = sidecar
                .annotations
                .iter_mut()
                .find(|existing| existing.id == annotation)
                .ok_or_else(|| "no such annotation".to_string())?;
            found.set_mark(mark, on);
            Ok(())
        })
    }

    /// Colour blocks, or clear them with `None`. Blocks the plan does not have are refused.
    pub fn set_highlights(
        &mut self,
        target: &Target,
        block_ids: &[BlockId],
        colour: Option<HighlightColour>,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            if let Some(missing) = block_ids
                .iter()
                .find(|id| !sidecar.blocks.iter().any(|indexed| indexed.id == **id))
            {
                return Err(format!("no such block in this document: {missing}"));
            }
            sidecar
                .highlights
                .retain(|highlight| !block_ids.contains(&highlight.block_id));
            if let Some(colour) = colour {
                sidecar
                    .highlights
                    .extend(block_ids.iter().map(|&block_id| BlockHighlight { block_id, colour }));
            }
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
            // A ruling either way answers the review an agent asked for.
            found.review = false;
            Ok(())
        })
    }

    /// An agent's "resolve": **it proposes, it never closes** (`D208`). The thread stays open,
    /// takes `review`, and the agent's closing note — when it gave one — is
    /// appended as its comment. Only the user's [`Self::resolve`] closes a thread.
    pub fn propose_resolution(
        &mut self,
        target: &Target,
        annotation: AnnotationId,
        note: Option<String>,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            let found = sidecar
                .annotations
                .iter_mut()
                .find(|existing| existing.id == annotation)
                .ok_or_else(|| "no such annotation".to_string())?;
            if let Some(note) = note.map(|note| note.trim().to_string()).filter(|n| !n.is_empty())
            {
                found.thread.push(ubiq_proto::work::Comment::new(
                    CommentAuthor::Agent,
                    note,
                    Utc::now(),
                ));
            }
            found.review = true;
            Ok(())
        })
    }

    /// Replace a comment's text and stamp `edited_at`. **Only a user's comment** — the interface
    /// is the only caller, and an agent's words are never rewritten by it.
    pub fn edit_comment(
        &mut self,
        target: &Target,
        annotation: AnnotationId,
        comment: CommentId,
        text: String,
    ) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err("a comment needs some text".to_string());
            }
            let found = sidecar
                .annotations
                .iter_mut()
                .find(|existing| existing.id == annotation)
                .ok_or_else(|| "no such annotation".to_string())?
                .thread
                .iter_mut()
                .find(|existing| existing.id == comment)
                .ok_or_else(|| "no such comment".to_string())?;
            if found.author != CommentAuthor::User {
                return Err("only your own comments can be edited".to_string());
            }
            if found.text != text {
                found.text = text;
                found.edited_at = Some(Utc::now());
            }
            Ok(())
        })
    }

    /// Bind the document to an agent, move it, or unbind it (`D207`).
    pub fn set_agent(&mut self, target: &Target, agent: Option<AgentId>) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            sidecar.agent = agent;
            Ok(())
        })
    }

    /// Turn auto-send on or off.
    pub fn set_auto_send(&mut self, target: &Target, auto_send: bool) -> Vec<Reply> {
        self.mutate(target, |sidecar| {
            sidecar.auto_send = auto_send;
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
        let mut answer = sidecar;
        self.code(target, &mut answer.blocks);
        vec![
            Reply::Asker(snapshot(target, answer)),
            Reply::Everyone(Message::PlanAnnotationsChanged {
                doc: target.handle(),
            }),
        ]
    }

    // ── agent delivery ─────────────────────────────────────────
    /// The threads the manual ask hands the doc queue, with the binding they go to — one sidecar
    /// read.
    ///
    /// `ids` empty is every open thread whose last word is the user's — what is waiting on the
    /// agent. Each carries the user's trailing comments, the ones since anybody else spoke.
    pub fn awaiting_threads(
        &mut self,
        target: &Target,
        ids: &[AnnotationId],
    ) -> Result<(DocBinding, Vec<QueuedRef>), String> {
        if let Some(refusal) = self.refusal(target) {
            return Err(refusal);
        }
        let sidecar = self.sidecar(target)?;
        let threads = sidecar
            .annotations
            .iter()
            .filter(|annotation| {
                // A thread awaiting the user's review is the user's move, never the agent's.
                if ids.is_empty() {
                    annotation.is_open()
                        && !annotation.review
                        && annotation
                            .thread
                            .last()
                            .is_some_and(|comment| comment.author == CommentAuthor::User)
                } else {
                    ids.contains(&annotation.id)
                }
            })
            .map(|annotation| {
                let mut comment_ids: Vec<CommentId> = annotation
                    .thread
                    .iter()
                    .rev()
                    .take_while(|comment| comment.author == CommentAuthor::User)
                    .map(|comment| comment.id)
                    .collect();
                comment_ids.reverse();
                QueuedRef {
                    doc: target.handle(),
                    annotation_id: annotation.id,
                    comment_ids,
                }
            })
            .collect();
        Ok((sidecar.binding(), threads))
    }

    /// A queued thread as the sidecar reads now, for the doc queue's delivery. `None` for a thread
    /// resolved, deleted or unreadable since it was queued — it is not delivered. The comments are
    /// the queued ones as they now read; one deleted since is left out.
    pub fn thread_view(&mut self, target: &Target, queued: &QueuedRef) -> Option<ThreadView> {
        let (blocks, annotations, _) = self.annotation_list(target).ok()?;
        let annotation = annotations
            .iter()
            .find(|annotation| annotation.id == queued.annotation_id)
            .filter(|annotation| annotation.is_open())?;
        let quote = annotation.quote.clone().or_else(|| {
            blocks
                .iter()
                .find(|block| block.id == annotation.block_id)
                .map(|block| block.text.clone())
        });
        let texts = annotation
            .thread
            .iter()
            .filter(|comment| queued.comment_ids.contains(&comment.id))
            .map(|comment| comment.text.clone())
            .collect();
        Some(ThreadView {
            doc: queued.doc.clone(),
            annotation_id: annotation.id,
            quote,
            texts,
        })
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

/// The agent a comment addressed to `Agent` goes to: the mission's coordinator, else the task's
/// assignee when that names an agent id. `None` is nobody to tell, and nothing is delivered.
pub fn choose_agent(
    coordinator: Option<ubiq_proto::work::AgentId>,
    assigned_to: Option<&str>,
) -> Option<ubiq_proto::work::AgentId> {
    coordinator.or_else(|| assigned_to.and_then(|who| who.trim().parse().ok()))
}

/// What a user comment just added hands the doc queue (`D207`), read off the mutation's own
/// `PlanAnnotations` reply so the sidecar is not read again: the document's binding and the thread
/// with the newest comment on it. `annotation` of `None` is the newest annotation, which is what a
/// just-created one is. `None` when the mutation failed.
pub fn queued_from(
    replies: &[Reply],
    annotation: Option<AnnotationId>,
) -> Option<(DocBinding, QueuedRef)> {
    replies.iter().find_map(|reply| match reply.message() {
        Message::PlanAnnotations {
            doc,
            annotations,
            binding,
            ..
        } => {
            let found = match annotation {
                Some(id) => annotations.iter().find(|existing| existing.id == id)?,
                None => annotations.last()?,
            };
            Some((
                *binding,
                QueuedRef {
                    doc: doc.clone(),
                    annotation_id: found.id,
                    comment_ids: vec![found.thread.last()?.id],
                },
            ))
        }
        _ => None,
    })
}

/// A document's annotation snapshot, binding included — what every read and mutation answers.
fn snapshot(target: &Target, sidecar: PlanSidecar) -> Message {
    let binding = sidecar.binding();
    Message::PlanAnnotations {
        doc: target.handle(),
        blocks: sidecar.blocks,
        annotations: sidecar.annotations,
        highlights: sidecar.highlights,
        binding,
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
            Vec::new(),
            None,
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
            None,
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
            Vec::new(),
            None,
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
            Vec::new(),
            None,
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
            Vec::new(),
            None,
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

    fn highlights_of(plans: &mut Plans, project: ProjectId, task: TaskId) -> Vec<BlockHighlight> {
        plans
            .annotation_list(&Target::plan(project, task))
            .expect("the sidecar reads")
            .2
    }

    #[test]
    fn marks_addressing_and_highlights_round_trip_and_vanish_with_their_block() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let target = Target::plan(project, task);
        plans.save(
            &target,
            "# Plan\n\nStep one.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        let ids = block_ids(&mut plans, project, task);

        // Addressing an agent sets the Agent mark; extra marks stack without duplicates.
        plans.annotate(
            &target,
            ids[1],
            None,
            CommentAuthor::User,
            "do it".to_string(),
            vec![AnnotationMark::Todo, AnnotationMark::Todo],
            Some(Addressee::Agent),
        );
        let a = annotations_of(&mut plans, project, task).remove(0);
        assert_eq!(a.thread[0].to, Some(Addressee::Agent));
        assert_eq!(a.marks, vec![AnnotationMark::Todo, AnnotationMark::Agent]);

        plans.reply_to(&target, a.id, CommentAuthor::User, "again".to_string(), None);
        plans.set_mark(&target, a.id, AnnotationMark::Todo, false);
        plans.set_mark(&target, a.id, AnnotationMark::Question, true);
        let a = annotations_of(&mut plans, project, task).remove(0);
        assert_eq!(a.thread[1].to, None);
        assert_eq!(a.marks, vec![AnnotationMark::Agent, AnnotationMark::Question]);

        // Highlights: set, recolour, clear; an unknown block is refused.
        plans.set_highlights(&target, &[ids[1], ids[2]], Some(HighlightColour::Green));
        plans.set_highlights(&target, &[ids[2]], Some(HighlightColour::Red));
        let h = highlights_of(&mut plans, project, task);
        assert_eq!(h.len(), 2);
        assert!(h.contains(&BlockHighlight { block_id: ids[2], colour: HighlightColour::Red }));
        let replies = plans.set_highlights(&target, &[BlockId::generate()], None);
        assert!(matches!(replies[0].message(), Message::PlanError { .. }));
        plans.set_highlights(&target, &[ids[1]], None);
        assert_eq!(highlights_of(&mut plans, project, task).len(), 1);

        // The highlighted block vanishes: its highlight goes with it, and the windows hear.
        let replies = plans.save(&target, "# Plan\n\nStep one.".to_string(), &Saver::human(), None);
        assert!(highlights_of(&mut plans, project, task).is_empty());
        assert!(replies.iter().any(|reply| matches!(
            reply,
            Reply::Everyone(Message::PlanAnnotationsChanged { .. })
        )));
    }

    #[test]
    fn a_save_that_only_reindexes_blocks_still_announces_it() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let target = Target::plan(project, task);
        plans.save(&target, "# Plan\n\nStep one.".to_string(), &Saver::human(), None);
        let replies = plans.save(
            &target,
            "# Plan\n\nStep one.\n\nStep two.".to_string(),
            &Saver::human(),
            None,
        );
        assert!(replies.iter().any(|reply| matches!(
            reply,
            Reply::Everyone(Message::PlanAnnotationsChanged { .. })
        )));
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
            Vec::new(),
            None,
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
            Vec::new(),
            None,
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
            Vec::new(),
            None,
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
        // And the sidecar is beside it, named by appending rather than replacing the extension —
        // written regardless of whether anything has been annotated, because the save it just
        // took a revision for is the same read a second call has to find again (T-183's note on
        // `Plans::write_sidecar`).
        let sidecar = repo.path().join("notes.md.annotation.json");
        assert!(sidecar.exists(), "the sidecar sits beside the file");
        assert!(
            !repo.path().join("notes.annotation.json").exists(),
            "the `.md` was not substituted away",
        );

        // Nothing of Ubiq's went into the config root for a file document.
        assert!(!_dir.path().join("projects").exists());

        let (blocks, _, _) = plans
            .annotation_list(&target)
            .expect("the block index reads");
        assert_eq!(blocks.len(), 2);
    }

    /// A document nobody has saved or read yet, and one whose body is empty, leave no sidecar —
    /// `Plans::sidecar()`'s own early return, unrelated to whether it carries an annotation: there
    /// is no block to index at all, so there is nothing yet worth writing down.
    #[test]
    fn an_untouched_or_empty_file_document_leaves_no_sidecar() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "").unwrap();
        let target = file_target(&repo, project, "notes.md");
        let sidecar = repo.path().join("notes.md.annotation.json");

        let (blocks, annotations, _) = plans
            .annotation_list(&target)
            .expect("an empty file still answers, with nothing in it");
        assert!(blocks.is_empty());
        assert!(annotations.is_empty());
        assert!(
            !sidecar.exists(),
            "reading an empty, never-saved file writes nothing beside it"
        );
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
        let (blocks, _, _) = plans.annotation_list(&target).unwrap();
        let second = blocks[2].id;
        plans.annotate(
            &target,
            second,
            Some("Second".to_string()),
            CommentAuthor::User,
            "is this still true?".to_string(),
            Vec::new(),
            None,
        );

        // The passage goes away, and the thread is flagged rather than dropped — the matcher is
        // the plan's own, so this is the plan's own behaviour.
        plans.save(
            &target,
            "# Notes\n\nFirst thought.".to_string(),
            &Saver::human(),
            None,
        );
        let (_, annotations, _) = plans.annotation_list(&target).unwrap();
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

    // ── mission documents (M8) ────────────────────────────────────────

    fn mission_doc(project: ProjectId, task: TaskId, name: &str) -> Target {
        Target::MissionDoc {
            project,
            task,
            name: name.to_string(),
        }
    }

    /// A mission document round-trips through save and load exactly as a plan does — the whole
    /// point of M8 being one more path the same family resolves, and nothing else.
    #[test]
    fn a_mission_document_round_trips_through_save_and_load() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let target = mission_doc(project, task, "notes");

        let replies = plans.save(
            &target,
            "# Notes\n\nFirst thought.".to_string(),
            &Saver::human(),
            None,
        );
        assert_eq!(revision_of(&replies), 1);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply, Reply::Everyone(Message::PlanChanged { .. })))
        );

        let replies = plans.load(&target);
        let Some(Message::Plan { body, doc, .. }) = replies
            .iter()
            .map(Reply::message)
            .find(|m| matches!(m, Message::Plan { .. }))
        else {
            panic!("expected a Plan reply, got {replies:?}");
        };
        assert_eq!(body, "# Notes\n\nFirst thought.");
        assert_eq!(*doc, target.handle());

        // Written under the mission's own `docs/` subtree, not beside the plan.
        assert!(
            _dir.path()
                .join("projects")
                .join(project.to_string())
                .join("missions")
                .join(task.to_string())
                .join("docs")
                .join("notes.md")
                .exists()
        );
    }

    /// A stale save on a mission document is refused exactly the way a plan's is — the same
    /// `expected`-revision arbitration, because it is the same code.
    #[test]
    fn a_mission_document_arbitrates_a_race_the_way_a_plan_does() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let target = mission_doc(project, task, "notes");

        let seeded = revision_of(&plans.save(&target, "mine".to_string(), &Saver::human(), None));
        let beaten = plans.save(&target, "also mine".to_string(), &Saver::human(), Some(0));
        assert_eq!(
            conflict_of(&beaten).map(|(revision, _)| revision),
            Some(seeded)
        );
    }

    /// Annotations and provenance work on a mission document exactly as they do on a plan's,
    /// because they are the same code reading the same sidecar shape.
    #[test]
    fn a_mission_documents_annotations_and_provenance_are_the_plans_own() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let target = mission_doc(project, task, "notes");

        plans.save(
            &target,
            "# Notes\n\nFirst thought.\n\nSecond thought.".to_string(),
            &Saver::agent("agent-1"),
            None,
        );
        let (blocks, _, _) = plans.annotation_list(&target).expect("the index reads");
        let second = blocks[2].id;

        let replies = plans.annotate(
            &target,
            second,
            Some("Second".to_string()),
            CommentAuthor::User,
            "is this still true?".to_string(),
            Vec::new(),
            None,
        );
        assert!(replies.iter().any(|reply| matches!(
            reply,
            Reply::Everyone(Message::PlanAnnotationsChanged { .. })
        )));

        let (_, annotations, _) = plans.annotation_list(&target).unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].quote.as_deref(), Some("Second"));
        assert!(!annotations[0].orphaned);

        // A human's edit is stamped, the way it is on a plan.
        plans.save(
            &target,
            "# Notes, revised.\n\nFirst thought.\n\nSecond thought.".to_string(),
            &Saver::human(),
            None,
        );
        let report = plans
            .change_report(&target, Some(1))
            .expect("the changes read");
        assert!(!report.regions.is_empty());
        assert_eq!(report.regions[0].origin, SaveOrigin::Human);
    }

    /// A task that carries no level at all is refused a mission document — no mission, no
    /// document.
    #[test]
    fn a_task_with_no_level_is_refused_a_mission_document() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_ordinary_task(&work, project);
        let target = mission_doc(project, task, "notes");

        let replies = plans.save(&target, "body".to_string(), &Saver::human(), None);
        let error = replies
            .iter()
            .map(Reply::message)
            .find_map(|m| match m {
                Message::PlanError { error, .. } => Some(error.clone()),
                _ => None,
            })
            .expect("a task with no level is refused");
        assert!(error.contains("mission"), "unexpected refusal: {error}");
    }

    /// A task that does not exist at all is refused the same way.
    #[test]
    fn a_missing_task_is_refused_a_mission_document() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let target = mission_doc(project, TaskId::generate(), "notes");

        let replies = plans.load(&target);
        assert!(
            replies
                .iter()
                .any(|reply| matches!(reply.message(), Message::PlanError { error, .. } if error == "no such task"))
        );
    }

    #[test]
    fn a_queued_thread_reads_the_annotation_the_block_and_the_comment_at_delivery() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let target = Target::plan(project, task);
        plans.save(&target, "# Plan\n\nShip the thing.".to_string(), &Saver::human(), None);
        let (blocks, _, _) = plans.annotation_list(&target).unwrap();
        let block = blocks.iter().find(|b| b.text.contains("Ship")).unwrap().id;
        let replies = plans.annotate(
            &target,
            block,
            None,
            CommentAuthor::User,
            "why now?".to_string(),
            Vec::new(),
            Some(Addressee::Agent),
        );
        let (_, annotations, _) = plans.annotation_list(&target).unwrap();
        let id = annotations[0].id;
        let (binding, queued) = queued_from(&replies, None).unwrap();
        assert_eq!(binding, DocBinding::default());
        assert_eq!(queued.annotation_id, id);
        assert!(queued_from(&replies, Some(AnnotationId::generate())).is_none());

        let view = plans.thread_view(&target, &queued).unwrap();
        assert_eq!(view.quote.as_deref(), Some("Ship the thing."));
        assert_eq!(view.texts, vec!["why now?".to_string()]);

        // Edited since it was queued: delivered as it now reads. Resolved: not delivered.
        plans.edit_comment(&target, id, queued.comment_ids[0], "why now, really?".to_string());
        assert_eq!(
            plans.thread_view(&target, &queued).unwrap().texts,
            vec!["why now, really?".to_string()]
        );
        plans.resolve(&target, id, true);
        assert!(plans.thread_view(&target, &queued).is_none());
    }

    /// A file document reaches its bound agent: the thread comes back with the binding, where it
    /// used to come back as nothing at all.
    #[test]
    fn a_bound_file_documents_thread_reaches_the_agent_and_the_snapshot_carries_the_binding() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("spec.md"), "# Spec\n\nA claim.").unwrap();
        let target = file_target(&repo, project, "spec.md");
        let agent = AgentId::generate();
        plans.set_agent(&target, Some(agent));
        let replies = plans.set_auto_send(&target, true);
        let carried = replies.iter().find_map(|reply| match reply.message() {
            Message::PlanAnnotations { binding, .. } => Some(*binding),
            _ => None,
        });
        assert_eq!(
            carried,
            Some(DocBinding {
                agent: Some(agent),
                auto_send: true
            })
        );
        let (blocks, _, _) = plans.annotation_list(&target).unwrap();
        let replies = plans.annotate(
            &target,
            blocks[1].id,
            Some("claim".to_string()),
            CommentAuthor::User,
            "source?".to_string(),
            Vec::new(),
            None,
        );
        let (binding, queued) = queued_from(&replies, None).unwrap();
        assert_eq!(binding.agent, Some(agent));
        assert_eq!(queued.doc, target.handle());
        assert!(plans.thread_view(&target, &queued).is_some());

        let (_, waiting) = plans.awaiting_threads(&target, &[]).unwrap();
        assert_eq!(waiting.len(), 1, "an open thread whose last word is the user's");

        plans.set_agent(&target, None);
        assert_eq!(plans.binding(&target).unwrap().agent, None);
    }

    #[test]
    fn only_a_users_comment_can_be_edited_and_an_edit_is_stamped() {
        let (mut plans, work, project, _dir) = plans_with_work();
        let task = make_mission(&work, project);
        let target = Target::plan(project, task);
        plans.save(&target, "# Plan\n\nShip it.".to_string(), &Saver::human(), None);
        let (blocks, _, _) = plans.annotation_list(&target).unwrap();
        plans.annotate(
            &target,
            blocks[1].id,
            None,
            CommentAuthor::User,
            "first".to_string(),
            Vec::new(),
            None,
        );
        let (_, annotations, _) = plans.annotation_list(&target).unwrap();
        let annotation = annotations[0].id;
        plans.reply_to(&target, annotation, CommentAuthor::Agent, "agent".to_string(), None);
        let (_, annotations, _) = plans.annotation_list(&target).unwrap();
        let (mine, theirs) = (annotations[0].thread[0].id, annotations[0].thread[1].id);

        plans.edit_comment(&target, annotation, mine, "first, edited".to_string());
        let refused = plans.edit_comment(&target, annotation, theirs, "hijack".to_string());
        assert!(refused.iter().any(|reply| matches!(
            reply.message(),
            Message::PlanError { error, .. } if error.contains("only your own")
        )));

        let (_, annotations, _) = plans.annotation_list(&target).unwrap();
        let thread = &annotations[0].thread;
        assert_eq!(thread[0].text, "first, edited");
        assert!(thread[0].edited_at.is_some());
        assert_eq!(thread[1].text, "agent");
        assert!(thread[1].edited_at.is_none());
    }

    #[test]
    fn the_coordinator_wins_over_the_assignee() {
        let coordinator = ubiq_proto::work::AgentId::generate();
        let assignee = ubiq_proto::work::AgentId::generate();
        let assigned = assignee.to_string();
        assert_eq!(choose_agent(Some(coordinator), Some(&assigned)), Some(coordinator));
        assert_eq!(choose_agent(None, Some(&assigned)), Some(assignee));
        assert_eq!(choose_agent(None, Some("a person")), None);
        assert_eq!(choose_agent(None, None), None);
    }

    // ── an agent's write merged over the user's newer text (`D208`) ─────

    const BASE: &str = "# T\n\nAlpha one. Alpha two.\n\nBeta.\n\nGamma old.\n";
    const SPLIT: &str = "# T\n\nAlpha one.\n\nAlpha two.\n\nBeta.\n\nGamma old.\n";

    /// A file document at [`BASE`], read by `agent`, then split by the user in its first block.
    fn read_then_split(agent: &str) -> (Plans, Target, tempfile::TempDir, tempfile::TempDir) {
        let (mut plans, _work, project, dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "").unwrap();
        let target = file_target(&repo, project, "notes.md");
        plans.save(&target, BASE.to_string(), &Saver::human(), None);
        plans.note_agent_read(&target, agent, BASE);
        plans.save(&target, SPLIT.to_string(), &Saver::human(), None);
        (plans, target, repo, dir)
    }

    fn on_disk(plans: &Plans, target: &Target) -> String {
        plans.read_body(target).unwrap()
    }

    #[test]
    fn a_split_and_a_later_agent_edit_both_land() {
        let (mut plans, target, _repo, _dir) = read_then_split("a1");
        let theirs = BASE.replace("Gamma old.", "Gamma new.");
        // A stale revision is no refusal when there is a base to merge from.
        let threads = plans.agent_save(&target, theirs, "a1", Some(1)).threads;
        assert!(threads.is_empty());
        assert_eq!(
            on_disk(&plans, &target),
            "# T\n\nAlpha one.\n\nAlpha two.\n\nBeta.\n\nGamma new.\n"
        );
    }

    #[test]
    fn an_agent_edit_inside_the_split_block_merges_by_word() {
        let (mut plans, target, _repo, _dir) = read_then_split("a1");
        let theirs = BASE.replace("Alpha two.", "Alpha 2.");
        let threads = plans.agent_save(&target, theirs, "a1", None).threads;
        assert!(threads.is_empty());
        assert_eq!(
            on_disk(&plans, &target),
            "# T\n\nAlpha one.\n\nAlpha 2.\n\nBeta.\n\nGamma old.\n"
        );
    }

    #[test]
    fn a_true_overlap_keeps_the_user_and_posts_the_agent_as_a_thread() {
        let (mut plans, target, _repo, _dir) = read_then_split("a1");
        plans.save(
            &target,
            SPLIT.replace("Alpha two.", "Alpha zwei."),
            &Saver::human(),
            None,
        );
        let theirs = BASE.replace("Alpha two.", "Alpha deux.");
        let threads = plans.agent_save(&target, theirs, "a1", None).threads;
        assert!(on_disk(&plans, &target).contains("Alpha zwei."));
        assert!(!on_disk(&plans, &target).contains("deux"));
        assert_eq!(threads.len(), 1);
        let (blocks, annotations, _) = plans.annotation_list(&target).unwrap();
        let thread = annotations.iter().find(|a| a.id == threads[0]).unwrap();
        assert_eq!(thread.thread[0].author, CommentAuthor::Agent);
        assert!(thread.thread[0].text.contains("Alpha deux."));
        let block = blocks.iter().find(|b| b.id == thread.block_id).unwrap();
        assert!(block.text.contains("zwei"), "anchored on the contested block");
    }

    #[test]
    fn a_thread_follows_its_quote_into_the_half_of_a_split_block() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "").unwrap();
        let target = file_target(&repo, project, "notes.md");
        plans.save(&target, BASE.to_string(), &Saver::human(), None);
        let block = plans.block_for_quote(&target, "Alpha two").unwrap();
        plans.annotate(
            &target,
            block,
            Some("Alpha two".to_string()),
            CommentAuthor::User,
            "why two?".to_string(),
            Vec::new(),
            None,
        );
        plans.save(&target, SPLIT.to_string(), &Saver::human(), None);
        let (blocks, annotations, _) = plans.annotation_list(&target).unwrap();
        let anchored = blocks
            .iter()
            .find(|b| b.id == annotations[0].block_id)
            .unwrap();
        assert!(!annotations[0].orphaned);
        assert_eq!(anchored.text.trim(), "Alpha two.");
    }

    /// Review fix 1: a second write built on the agent's own text, after a clean merge, keeps the
    /// user's edit — the base is what the agent sent, so the user's change is merged in again.
    #[test]
    fn a_second_agent_write_after_a_clean_merge_keeps_the_users_edit() {
        let (mut plans, target, _repo, _dir) = read_then_split("a1");
        let first = BASE.replace("Gamma old.", "Gamma new.");
        let saved = plans.agent_save(&target, first.clone(), "a1", None);
        assert!(saved.merged, "the split is in what stands, not in what was sent");
        // The agent ignores the note and writes again from its own text.
        let second = first.replace("Beta.", "Beta, expanded.");
        plans.agent_save(&target, second, "a1", None);
        assert_eq!(
            on_disk(&plans, &target),
            "# T\n\nAlpha one.\n\nAlpha two.\n\nBeta, expanded.\n\nGamma new.\n"
        );
    }

    /// Review fix 3: a tab save landing between an agent's unlocked merge and its commit refuses
    /// the commit, and the handle merges again over the user's newer text.
    #[test]
    fn a_tab_save_between_merge_and_commit_is_merged_not_overwritten() {
        let (mut plans, target, repo, _dir) = read_then_split("a1");
        let theirs = BASE.replace("Gamma old.", "Gamma new.");
        let (base, current) = plans.agent_merge_inputs(&target, "a1");
        let prepared = AgentWrite::prepare(base.as_deref(), current.as_deref(), &theirs);
        // The user's tab saves meanwhile, under the lock.
        let user = SPLIT.replace("Beta.", "Beta (user).");
        let version = crate::files::save(repo.path(), "notes.md", SPLIT.as_bytes(), None, true)
            .unwrap();
        plans
            .file_save(&target, repo.path(), "notes.md", user.as_bytes(), Some(version), false)
            .unwrap();
        assert!(
            plans
                .agent_commit(&target, &theirs, "a1", None, current.as_deref(), prepared)
                .is_none(),
            "the disk moved under the merge, so nothing is written"
        );
        let handle = Handle::new(plans);
        handle.agent_save(&target, &theirs, "a1", None);
        assert_eq!(
            handle.lock().read_body(&target).unwrap(),
            "# T\n\nAlpha one.\n\nAlpha two.\n\nBeta (user).\n\nGamma new.\n"
        );
    }

    /// Review fixes 2 and 10: a window's conflict claim is authored by who the stamps say wrote
    /// the passage, and the same claim twice makes one thread.
    #[test]
    fn a_window_claim_is_stamped_by_provenance_and_said_once() {
        let (mut plans, target, _repo, _dir) = read_then_split("a1");
        let theirs = SPLIT.replace("Gamma old.", "Gamma by agent.");
        plans.agent_save(&target, theirs, "a1", None);
        let claim = ubiq_proto::merge::Conflict {
            base: "Gamma old.\n".into(),
            ours: "Gamma by user.\n".into(),
            theirs: "Gamma by agent.\n".into(),
        };
        plans.disk_conflicts(&target, vec![claim.clone()]);
        plans.disk_conflicts(&target, vec![claim]);
        let (_, annotations, _) = plans.annotation_list(&target).unwrap();
        assert_eq!(annotations.len(), 1, "said once");
        assert_eq!(annotations[0].thread[0].author, CommentAuthor::Agent);
        // A passage not on disk is refused outright.
        let invented = ubiq_proto::merge::Conflict {
            base: String::new(),
            ours: String::new(),
            theirs: "Never written.".into(),
        };
        assert!(plans.disk_conflicts(&target, vec![invented]).is_empty());
    }

    /// Lineage (`D208`): the user splits a block while an agent's write is in flight; the parts
    /// take child codes, the agent's edit elsewhere keeps its block's code, and a thread on the
    /// split block follows its quote into the part holding it.
    #[test]
    fn a_split_during_an_agent_write_carries_codes_and_threads() {
        let (mut plans, _work, project, _dir) = plans_with_work();
        let repo = tempfile::tempdir().unwrap();
        std::fs::write(repo.path().join("notes.md"), "").unwrap();
        let target = file_target(&repo, project, "notes.md");
        plans.save(&target, BASE.to_string(), &Saver::human(), None);
        let code_of = |plans: &mut Plans, text: &str| {
            let (blocks, _, _) = plans.annotation_list(&target).unwrap();
            blocks
                .into_iter()
                .find(|b| b.text.trim() == text)
                .map(|b| b.lineage)
                .unwrap()
        };
        assert_eq!(code_of(&mut plans, "Alpha one. Alpha two."), "AAB");
        let alpha = plans.block_for_quote(&target, "Alpha two").unwrap();
        plans.annotate(
            &target,
            alpha,
            Some("Alpha two".to_string()),
            CommentAuthor::User,
            "why two?".to_string(),
            Vec::new(),
            None,
        );
        plans.note_agent_read(&target, "a1", BASE);
        // The user's split lands while the agent works on the last block.
        plans.save(&target, SPLIT.to_string(), &Saver::human(), None);
        plans.agent_save(&target, BASE.replace("Gamma old.", "Gamma new."), "a1", None);

        assert_eq!(code_of(&mut plans, "Alpha one."), "AAB.AA");
        assert_eq!(code_of(&mut plans, "Alpha two."), "AAB.AB");
        assert_eq!(code_of(&mut plans, "Gamma new."), "AAD");
        let (blocks, annotations, _) = plans.annotation_list(&target).unwrap();
        let anchored = blocks.iter().find(|b| b.id == annotations[0].block_id).unwrap();
        assert_eq!(anchored.lineage, "AAB.AB");
        // Nothing of it is written down: the sidecar's blocks carry no code.
        let sidecar = plans.read_sidecar(&target).unwrap().unwrap();
        assert!(sidecar.blocks.iter().all(|b| b.lineage.is_empty()));
        // Reopening compacts.
        plans.reopen_lineage(&target);
        assert_eq!(code_of(&mut plans, "Alpha two."), "AAC");
    }

    /// Review fix 6: a quote two blocks hold does not move a thread.
    #[test]
    fn an_ambiguous_quote_does_not_move_a_thread() {
        let mut annotation = Annotation::new(
            BlockId::generate(),
            Some("same".to_string()),
            CommentAuthor::User,
            "?".to_string(),
            Utc::now(),
        );
        annotation.orphaned = true;
        let block = |text: &str| PlanBlock {
            id: BlockId::generate(),
            kind: "paragraph".to_string(),
            text: text.to_string(),
            lineage: String::new(),
        };
        let mut annotations = vec![annotation];
        assert!(!follow_quotes(
            &mut annotations,
            &[block("the same one"), block("same again")]
        ));
        assert!(annotations[0].orphaned);
    }
}
