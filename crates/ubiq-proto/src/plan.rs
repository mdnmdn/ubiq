//! A plan's annotations: what a human or an agent has written *about* a passage of the plan,
//! never inside it.
//!
//! The plan body on disk stays plain markdown — that is what makes export a copy rather than a
//! render, and what lets an agent rewrite a plan with ordinary file tools. So an annotation cannot
//! live in the document, and it cannot name a character offset either: an insertion anywhere above
//! a range silently moves every annotation below it, and a rewritten section invalidates all of
//! them with no signal. It names a [`BlockId`] instead, and optionally quotes a span inside that
//! block for the interface to highlight (`_docs/wip/planning-system.md`, decision 4).
//!
//! Two states are orthogonal and both are kept:
//!
//! - [`AnnotationState`] is what the *thread* is — open, or answered and closed. **Anyone may
//!   resolve one, including an agent**: a planning assistant that answers a comment can close it.
//! - [`Annotation::orphaned`] is what the *anchor* is. A block that vanished from a saved plan
//!   leaves its annotations flagged rather than deleted — a reportable state the interface can
//!   show, not a silent loss. An orphaned annotation keeps its thread and its quoted span, which
//!   is all the user needs to decide whether it still matters.
//!
//! **Edit provenance is the third layer here, and it is about lines rather than blocks.** An
//! annotation says what someone wrote *about* a passage; provenance says who last *rewrote* one.
//! An agent that wrote a plan and comes back to it needs to be told where a human has been since,
//! and a block id cannot say that: a block whose wording a human tightened is the same block, with
//! the same id, and the matcher's whole job is to keep it that way. So provenance is carried per
//! line, stamped with the [`PlanRevision`] and the [`SaveOrigin`] that last touched it, and
//! reported back as [`PlanChangedRegion`] runs with a [`PlanChangeStats`] summary.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use crate::work::{Comment, CommentAuthor};

/// Which document the annotation family is talking about.
///
/// **The whole family is keyed by this and by nothing else.** A plan and an ordinary markdown file
/// in a project's tree are annotated by the same messages, matched by the same block matcher and
/// orphaned by the same rule; all that differs is where the body and its sidecar sit on disk, and
/// that is the host's answer to this handle rather than a second set of messages. The names in the
/// family stayed `Plan*` because the records they carry are ([`PlanBlock`], [`PlanRevision`]) and
/// renaming half a vocabulary is not what makes a document generic.
///
/// **It carries no absolute path.** `rel_path` is project-relative, the way every file-family
/// message's is: the interface never learns where a project lives, and the host resolves and
/// contains the path before it touches anything.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum DocumentHandle {
    /// A task's plan, under the config root at `projects/<ProjectId>/plans/<TaskId>.md`, with its
    /// sidecar at `<TaskId>.annotations.json` beside it.
    Plan {
        project_id: ProjectId,
        task_id: TaskId,
    },
    /// An ordinary markdown file in the project's own working tree, with its sidecar at
    /// `<file>.md.annotation.json` beside it — inside the user's repository, because an annotation
    /// on a file the repository owns belongs with the file and travels with it.
    File {
        project_id: ProjectId,
        /// Project-relative, and refused by the host unless it lands inside the project.
        rel_path: String,
    },
    /// A mission document beyond the plan (M8), at
    /// `<config root>/projects/<ProjectId>/missions/<TaskId>/docs/<name>.md`, with its sidecar
    /// beside it. **Flat**: `name` is a bare document name, never a path — there is no nesting
    /// under `docs/`. This is the whole of what a second kind of mission document costs the wire:
    /// the family is keyed by the document, not by the plan, so a mission doc inherits
    /// annotations, provenance and the conflict rule for free.
    MissionDoc {
        project_id: ProjectId,
        task_id: TaskId,
        name: String,
    },
}

impl DocumentHandle {
    pub fn project_id(&self) -> ProjectId {
        match self {
            DocumentHandle::Plan { project_id, .. }
            | DocumentHandle::File { project_id, .. }
            | DocumentHandle::MissionDoc { project_id, .. } => *project_id,
        }
    }

    /// The task a plan or a mission document belongs to. `None` for a file document, which
    /// carries no task — the point of the handle: no caller may assume a task is there.
    pub fn task_id(&self) -> Option<TaskId> {
        match self {
            DocumentHandle::Plan { task_id, .. } | DocumentHandle::MissionDoc { task_id, .. } => {
                Some(*task_id)
            }
            DocumentHandle::File { .. } => None,
        }
    }

    /// The project-relative path of a file document. `None` for a plan or a mission document,
    /// neither of which has one: both live under the config root and the interface never learns
    /// where.
    pub fn rel_path(&self) -> Option<&str> {
        match self {
            DocumentHandle::Plan { .. } | DocumentHandle::MissionDoc { .. } => None,
            DocumentHandle::File { rel_path, .. } => Some(rel_path),
        }
    }

    /// What a surface calls this kind of document in its chrome.
    pub fn kind_label(&self) -> &'static str {
        match self {
            DocumentHandle::Plan { .. } => "Plan",
            DocumentHandle::File { .. } => "Document",
            DocumentHandle::MissionDoc { .. } => "Mission document",
        }
    }

    /// A stable string for element ids and buffer keys — one document, one key.
    pub fn key(&self) -> String {
        match self {
            DocumentHandle::Plan { task_id, .. } => format!("plan:{task_id}"),
            DocumentHandle::File {
                project_id,
                rel_path,
            } => format!("file:{project_id}:{rel_path}"),
            DocumentHandle::MissionDoc { task_id, name, .. } => {
                format!("missiondoc:{task_id}:{name}")
            }
        }
    }
}

/// A plan's version counter: how many times its body has been saved.
///
/// Monotonic within one plan and meaningless across two. `0` is a plan whose body has never been
/// written, so the first save is revision 1 and a watermark of `0` means "everything".
pub type PlanRevision = u64;

/// Who saved a plan's body.
///
/// **Established at the entry point, never inferred.** A `SavePlan` off the bus is
/// [`SaveOrigin::Human`] because only the interface sends one; a `write_plan` through the
/// `ubiq-plan` MCP server is [`SaveOrigin::Agent`] because only a hosted agent can call it. There
/// is no third path to a plan's body and no downstream code that guesses — the distinction is the
/// entire point of the layer, and a guess would be worse than no answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveOrigin {
    /// A person, typing in the plan editor. The default, because an unstamped save in an older
    /// sidecar is one nobody can attribute to an agent, and attributing it to the human is the
    /// reading that never tells an agent "you wrote this" about something it did not.
    #[default]
    Human,
    /// An agent, through `ubiq-plan`'s `write_plan`.
    Agent,
}

impl SaveOrigin {
    pub fn label(self) -> &'static str {
        match self {
            SaveOrigin::Human => "human",
            SaveOrigin::Agent => "agent",
        }
    }

    pub fn is_human(self) -> bool {
        self == SaveOrigin::Human
    }
}

/// One contiguous run of lines in the plan's *current* body that changed since some watermark,
/// with the text now standing there.
///
/// Line numbers are 1-based and inclusive at both ends, the numbering a person reads off an
/// editor's gutter. They index the body as it is **now**, not as it was at `revision`, which is
/// what makes them usable for decoration without a second round trip: the interface already holds
/// the current body.
///
/// A run is split wherever the revision or the origin changes, so `origin` and `revision` describe
/// every line in it and never an average of two.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanChangedRegion {
    pub first_line: u32,
    pub last_line: u32,
    /// The revision that last touched these lines.
    pub revision: PlanRevision,
    pub origin: SaveOrigin,
    /// The block these lines fall in, where one contains the whole run — absent for a run that
    /// straddles two blocks or falls in the blank space between them. Best effort by design: it
    /// is here so an agent can say "you changed the *Rollout* section" without a second call, and
    /// the line numbers are the authoritative answer either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<BlockId>,
    /// The lines themselves, as they now read, newline-joined and without a trailing newline.
    pub text: String,
}

impl PlanChangedRegion {
    /// How many lines the run covers.
    pub fn line_count(&self) -> u32 {
        self.last_line.saturating_sub(self.first_line) + 1
    }
}

/// What changed in a plan between a watermark and now, counted.
///
/// The three line counts are **summed over the revisions in the window**, not derived from the
/// regions: a line a human rewrote twice is two modifications and one region, and an agent asking
/// "how much churn has there been" wants the former. `blocks_touched` and the regions do agree,
/// because both are read off the current body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanChangeStats {
    /// The watermark this was counted from. Changes at or below it are not in the window.
    pub since_revision: PlanRevision,
    /// The plan's revision now. Equal to `since_revision` when nothing has been saved since.
    pub revision: PlanRevision,
    pub lines_added: u32,
    pub lines_removed: u32,
    pub lines_modified: u32,
    /// Blocks of the current body that hold at least one changed line.
    pub blocks_touched: u32,
    /// Saves in the window, split by who made them. Their sum is the number of revisions in the
    /// window, which is also `revision - since_revision`.
    pub human_revisions: u32,
    pub agent_revisions: u32,
}

impl PlanChangeStats {
    /// Nothing changed at all — no region, no count. What a caller whose watermark is already the
    /// current revision gets.
    pub fn unchanged(revision: PlanRevision) -> Self {
        Self {
            since_revision: revision,
            revision,
            ..Self::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines_added == 0 && self.lines_removed == 0 && self.lines_modified == 0
    }
}

/// One block of a saved plan: the unit an annotation anchors to.
///
/// The ids are the host's — assigned on save by matching the parsed document against the previous
/// version — so a window that wants to annotate a passage has to be **told** which block is which
/// rather than deriving it. It travels beside the annotations for that reason: an annotation names
/// a block id, and a block id means nothing without this.
///
/// `text` is the block's own markdown, trimmed. A window pairs it against its own parse of the
/// same body, which is the same parser at the same options, so the two agree block for block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanBlock {
    pub id: BlockId,
    /// The mdast node kind, with a heading's depth folded in — `paragraph`, `heading:2`, `code`.
    /// Two blocks of different kinds never match across a save.
    pub kind: String,
    pub text: String,
}

/// Whether an annotation's thread is still asking for something.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnotationState {
    /// Nobody has closed it.
    #[default]
    Open,
    /// Answered, and closed by whoever decided it was. Not author-only: see the module doc.
    Resolved,
}

impl AnnotationState {
    pub fn label(self) -> &'static str {
        match self {
            AnnotationState::Open => "open",
            AnnotationState::Resolved => "resolved",
        }
    }
}

/// One annotation on a plan, with its whole thread.
///
/// The thread is never empty: the comment that opened the annotation is its first entry, and a
/// reply appends. The author of each comment is host-stamped from the path it arrived on, `D121`'s
/// rule for a task's comments applied unchanged here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: AnnotationId,
    /// The block this is *about*. Carried forward across saves by block matching; when the block
    /// goes, this id stays and `orphaned` is set, so the annotation still says what it was about.
    pub block_id: BlockId,
    /// The passage inside the block the user selected, where the interface had one to give. The
    /// text itself, not a range — a quote survives the block being edited around it, and a quote
    /// that no longer appears in the block is still readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    pub state: AnnotationState,
    /// The block this names is gone from the plan. Retained and flagged, never deleted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orphaned: bool,
    pub thread: Vec<Comment>,
    pub created_at: DateTime<Utc>,
}

impl Annotation {
    pub fn new(
        block_id: BlockId,
        quote: Option<String>,
        author: CommentAuthor,
        text: String,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: AnnotationId::generate(),
            block_id,
            quote,
            state: AnnotationState::Open,
            orphaned: false,
            thread: vec![Comment::new(author, text, now)],
            created_at: now,
        }
    }

    /// Append to the thread.
    pub fn reply(&mut self, author: CommentAuthor, text: String, now: DateTime<Utc>) {
        self.thread.push(Comment::new(author, text, now));
    }

    /// What the annotation asked, for a list that draws one line per thread.
    pub fn opening(&self) -> Option<&Comment> {
        self.thread.first()
    }

    pub fn is_open(&self) -> bool {
        self.state == AnnotationState::Open
    }
}
