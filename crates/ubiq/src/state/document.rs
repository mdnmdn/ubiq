//! The annotated-document surface: the document's markdown, the threads anchored to its blocks,
//! and the handle that says *which* document is on screen.
//!
//! **The surface is an [`MdView`] with its annotation layer on.** The body is drawn by
//! `crate::ui::mdview` — one row per `ubiq_md` root block — and written through that view's own
//! block editor, which commits into the same buffer the whole document is saved from. Every fact
//! below about revisions, staleness and dirtiness is unchanged by that: a block commit is an
//! ordinary whole-document save.
//!
//! **Two block models meet here, and [`RowMap`] is the join.** The host indexes a document into
//! `PlanBlock`s and owns their ids — a window never mints one, because matching blocks across a
//! save is what keeps a thread anchored. The view lays out `ubiq_md` rows. [`row_map`] places each
//! host block in the row holding the start of its source range ([`block_ranges`]), and everything
//! the margin draws ([`row_decor`]) and every intent it raises ([`target_block`], [`mark_target`],
//! [`resolve_target`]) goes through that map. Thread notes live only in the rail, never inline.
//!
//! **This module is generic over a document on purpose.** [`DocumentEditor`] names a
//! [`DocumentHandle`] rather than a `TaskId`: a plan, a mission document and an ordinary markdown
//! file in the project's tree are the handle's variants, and the surface cannot tell them apart.
//! The handle lives in the contract, because every message in the family carries it. What any
//! variant answers for is the same three things:
//!
//! 1. **A body source and a save target** — `LoadPlan`/`SavePlan`, naming the handle.
//! 2. **An annotation source** — the blocks, their threads and highlights, and the verbs that open,
//!    answer, mark, colour and close them.
//! 3. **A store for both, on the host side.** A plan's is under the config root; a file's body is
//!    the file itself, with its sidecar at `<file>.md.annotation.json` beside it.
//!
//! Nothing here renders, and nothing here builds a [`ubiq_proto::messages::Message`]: the wire is
//! `crate::app::plan`'s, which carries the `impl DocumentHandle` that turns a handle into the
//! family's messages.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use gpui::{Entity, Subscription};
use ubiq_proto::ids::{AnnotationId, BlockId, CommentId, ProjectId, TaskId};
use ubiq_proto::plan::{
    Annotation, AnnotationMark, AnnotationState, BlockHighlight, DocBinding, HighlightColour,
    PlanBlock,
    PlanChangeStats, PlanChangedRegion, PlanRevision, SaveOrigin,
};
use ubiq_proto::work::Addressee;

use crate::ui::mdview::view::MdView;

pub use ubiq_proto::plan::DocumentHandle;

/// What the host has said about the document's body, since the surface asked.
pub enum DocumentBody {
    /// The read was sent, nothing back yet.
    Loading,
    /// The document, whole — an empty string is a document nobody has written yet, which is not
    /// an error.
    Loaded(String),
    /// The host refused, and said why.
    Failed(String),
}

/// What the host has said about the document's annotations — [`DocumentBody`]'s own shape, for
/// the threads rather than the text.
pub enum AnnotationsBody {
    Loading,
    /// The block index and every thread on it, open, resolved and orphaned alike.
    Loaded {
        blocks: Vec<PlanBlock>,
        annotations: Vec<Annotation>,
        /// The colours on whole blocks, independent of any thread.
        highlights: Vec<BlockHighlight>,
    },
    Failed(String),
}

impl AnnotationsBody {
    pub fn blocks(&self) -> &[PlanBlock] {
        match self {
            AnnotationsBody::Loaded { blocks, .. } => blocks,
            _ => &[],
        }
    }

    pub fn annotations(&self) -> &[Annotation] {
        match self {
            AnnotationsBody::Loaded { annotations, .. } => annotations,
            _ => &[],
        }
    }

    pub fn highlights(&self) -> &[BlockHighlight] {
        match self {
            AnnotationsBody::Loaded { highlights, .. } => highlights,
            _ => &[],
        }
    }
}

/// The outcome of a one-shot action the surface reports where the user is looking — a save that
/// was refused, an export that landed — rather than as a toast.
pub enum Notice {
    Ok(String),
    Err(String),
}

/// What the composer in the rail is answering — one at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ComposerTarget {
    /// A fresh annotation on this block.
    Block(BlockId),
    /// A reply appended to this thread.
    Reply(AnnotationId),
    /// New text for one of the user's own comments on this thread. The composer's field is
    /// seeded with the old text, and "send" is `EditAnnotationComment`.
    Edit(AnnotationId, CommentId),
}

/// Which frame the one annotated-document surface is drawn in.
///
/// **One document is open at a time per window**, so this is a property of that document rather
/// than two states: a markdown tab put into `ViewLayout::Annotation` opens the file's document
/// here, and raising the plan dialog over it replaces it, exactly as opening a second plan does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Presentation {
    /// `ui/plan.rs`'s dialog — the plan editor, settled as a modal.
    Modal,
    /// Inside a markdown tab's viewer, as its fourth layout.
    Viewer,
}

/// Whether a freshly opened document of this kind asks the host where it was edited (T-183).
///
/// A file document is markdown in the user's own repository, most often prose nobody but its one
/// editor ever touches; a plan (or a mission document, on the same footing) is the kind this
/// surface exists to referee between a human and an agent, so it defaults to knowing which lines
/// are whose.
pub fn tracks_updates_by_default(doc: &DocumentHandle) -> bool {
    !matches!(doc, DocumentHandle::File { .. })
}

/// One annotated document, while its surface is open. One at a time per window: opening another
/// replaces it, the way the rest of the window's overlays behave.
pub struct DocumentEditor {
    pub doc: DocumentHandle,
    /// Where the surface is drawn. **The surface itself is the same either way** — the view, the
    /// rail and every action on them are `crate::ui::document`'s. What this decides is who frames
    /// it, and whose buffer [`Self::md`] is over: [`Presentation::Modal`] is the plan dialog over
    /// `AppState::plan_editor`, [`Presentation::Viewer`] is a markdown tab's own view over the
    /// tab's own buffer.
    pub presentation: Presentation,
    /// The markdown view the document is drawn in, its annotation layer on. A dialog's is built
    /// for the document and dropped with it; a tab's is the tab's `OpenFile::md`, borrowed.
    pub md: Entity<MdView>,
    /// The view's events, for a view built for this document — `None` for a tab's view, which
    /// the tab already subscribes to (`app::editor::attach_md_view`).
    pub md_events: Option<Subscription>,
    /// The host blocks placed in the view's rows, as of the last remap — rebuilt whenever either
    /// side moves: a reparse, or a fresh block index.
    pub rows: RowMap,
    pub body: DocumentBody,
    /// The text as the host last stated it. The buffer is seeded from this, and `dirty` is the
    /// buffer differing from it.
    pub saved: String,
    /// The buffer has not been seeded from `saved` yet — the frame after a load does it, where
    /// there is a `Window` to write a buffer with.
    pub needs_seed: bool,
    /// The buffer differs from `saved`.
    pub dirty: bool,
    /// The host's copy moved under an unsaved edit — another window's save, or an agent's write.
    /// The surface says so and keeps what was typed; it never silently overwrites either side.
    pub stale: bool,
    /// The revision of the last body the *buffer* was seeded from — the watermark a save is made
    /// against. `0` is a document whose body has never been written.
    pub revision: PlanRevision,
    /// The newest revision the host has stated, seeded or not. Ahead of `revision` exactly when
    /// somebody else's save landed under an unsaved edit, which is what `stale` reports.
    pub host_revision: PlanRevision,
    /// Who made the save that moved the host's copy, as `PlanChanged` stated it. What the stale
    /// banner names, so the user is told whether they are about to overwrite an agent or a person.
    pub stale_origin: Option<SaveOrigin>,
    /// The user has been shown what a stale save would overwrite and asked again anyway.
    ///
    /// **This is a confirmation, not the guard.** The guard is on the wire: `SavePlan` names the
    /// revision it expects to replace and the host refuses it with `PlanConflict` otherwise. What
    /// this decides is only whether the next press names `revision` or `host_revision`. Cleared
    /// whenever the host's copy moves again, so one confirmation covers one revision.
    pub confirm_overwrite: bool,
    /// A save is in flight: the surface waits for the host's answer before calling itself clean.
    pub saving: bool,
    /// Escape was pressed over an unsaved edit once. The second one means it.
    pub confirm_close: bool,
    /// The margin decor no longer matches the threads — remapped in the frame that notices.
    pub decor_stale: bool,
    /// Where the document was edited since the beginning, and by whom — the host's answer to
    /// `ListPlanChanges`, in *current* line numbers.
    pub changes: Vec<PlanChangedRegion>,
    /// How much changed, for the footer. Zeroed until the host has answered once.
    pub change_stats: PlanChangeStats,
    pub annotations: AnnotationsBody,
    /// The thread the rail has expanded — set by a click in the rail or on a row's count marker,
    /// and moved by the preview's scroll to the first open thread on screen ([`follow_target`]).
    /// Never a resolved thread for long: the rail draws resolved threads collapsed.
    pub focused: Option<AnnotationId>,
    /// Resolved threads are hidden in the rail until this is on. They are never dropped.
    pub show_resolved: bool,
    pub composer: Option<ComposerTarget>,
    /// The marks a fresh thread is opened with — preset by marking a row that has no thread yet
    /// (`MdViewEvent::MarkRequested`), toggled by the composer's own chips.
    pub composer_marks: Vec<AnnotationMark>,
    /// The composer's "@agent" toggle: the next post is addressed to the agent. **Sticky** — it
    /// stays as the user left it across posts, cancels and new threads, for the life of this
    /// document's surface.
    pub composer_to_agent: bool,
    /// The agent this document is bound to and its auto-send switch, as the host last stated them
    /// (`D207`) — `PlanAnnotations`' `binding`, the answer to every mutation in the family.
    pub binding: DocBinding,
    /// Mirrored out of `AppState::annotation_composer_input`: the entity behind the field is the
    /// window's, what was typed is the document's.
    pub composer_text: String,
    /// The composer was opened from somewhere with no `Window` (a view event); the next frame
    /// clears and focuses the field.
    pub composer_needs_focus: bool,
    pub notice: Option<Notice>,
    /// Whether the surface asks the host where this document has been edited and shows the
    /// footer's line counts and human/agent split from the answer — `ListPlanChanges`,
    /// `AppState::ask_for_plan_changes` (T-183). **Per document**, defaulted by
    /// [`tracks_updates_by_default`]; `AppState::toggle_track_updates` changes only this one's.
    pub track_updates: bool,
}

impl DocumentEditor {
    /// A freshly opened surface over `md`, waiting on the host's two answers.
    pub fn loading(
        doc: DocumentHandle,
        presentation: Presentation,
        md: Entity<MdView>,
        md_events: Option<Subscription>,
    ) -> Self {
        let track_updates = tracks_updates_by_default(&doc);
        Self {
            doc,
            presentation,
            md,
            md_events,
            rows: RowMap::default(),
            body: DocumentBody::Loading,
            saved: String::new(),
            needs_seed: false,
            dirty: false,
            stale: false,
            revision: 0,
            host_revision: 0,
            stale_origin: None,
            confirm_overwrite: false,
            saving: false,
            confirm_close: false,
            decor_stale: false,
            changes: Vec::new(),
            change_stats: PlanChangeStats::default(),
            annotations: AnnotationsBody::Loading,
            focused: None,
            show_resolved: false,
            composer: None,
            composer_marks: Vec::new(),
            composer_to_agent: false,
            binding: DocBinding::default(),
            composer_text: String::new(),
            composer_needs_focus: false,
            notice: None,
            track_updates,
        }
    }

    pub fn project_id(&self) -> ProjectId {
        self.doc.project_id()
    }

    pub fn task_id(&self) -> Option<TaskId> {
        self.doc.task_id()
    }

    /// The stable string that scopes every element id this surface draws — `ubiq_proto`'s own
    /// `DocumentHandle::key` (T-125). An id with a unique part of its own (a block id, an
    /// annotation id) needs no scope; the surface's chrome does.
    pub fn surface_key(&self) -> String {
        self.doc.key()
    }

    /// Whether this document is the plan dialog's rather than a markdown tab's.
    pub fn is_modal(&self) -> bool {
        self.presentation == Presentation::Modal
    }

    /// The host stated the document's body, and `typed` is what the buffer holds as it arrives.
    ///
    /// A clean surface takes the body; a surface holding an unsaved edit keeps it and is told its
    /// copy moved, because throwing away an unsaved edit to win a race is the one thing an editor
    /// may never do. **The buffer is the truth here rather than the `dirty` flag**: the answer to
    /// the window's own save carries exactly what the buffer holds, so comparing against `typed`
    /// is what tells a save's echo apart from somebody else's write.
    /// `revision` is the watermark that body carries. A buffer that takes the body takes the
    /// watermark with it; a buffer that keeps an unsaved edit does **not** — it stays behind at
    /// the revision it was seeded from, which is exactly what makes the save that follows a save
    /// over somebody else's work and what the confirmation is asked about.
    pub fn set_loaded(&mut self, body: String, typed: &str, revision: PlanRevision) {
        self.saving = false;
        let moved = revision != self.host_revision;
        self.host_revision = revision;
        // The dialog's buffer is shared across whichever document is open, so on a document's
        // *first* answer it still holds whatever the surface last showed. Nothing has been typed
        // into a document that has not loaded yet, so this body is taken unconditionally.
        let edited = !matches!(self.body, DocumentBody::Loading) && typed != self.saved;
        if edited && typed != body {
            // A *newer* third-party save is a new question: what was confirmed was confirmed
            // about the revision before this one. The same revision restated is not.
            if moved {
                self.confirm_overwrite = false;
            }
            self.stale = true;
            self.dirty = true;
            self.saved = body;
            return;
        }
        self.revision = revision;
        self.stale_origin = None;
        self.confirm_overwrite = false;
        // Only re-seed when the buffer does not already hold it: writing the same text back would
        // cost the undo history for nothing.
        self.needs_seed = typed != body;
        self.saved = body.clone();
        self.body = DocumentBody::Loaded(body);
        self.dirty = false;
        self.stale = false;
        self.confirm_close = false;
        self.decor_stale = true;
    }

    /// The host refused a save because the document had moved past the revision it named. Nothing
    /// was written, so **the buffer is left exactly as it is** — that is the difference between
    /// being refused and silently losing the work.
    ///
    /// The surface is put back where a third-party save would have put it: stale, holding an
    /// unsaved edit, naming who moved the copy, and with the confirmation cleared so the question
    /// is asked again about *this* revision.
    pub fn save_refused(&mut self, revision: PlanRevision, origin: SaveOrigin) {
        self.saving = false;
        self.host_revision = revision;
        self.stale = true;
        self.dirty = true;
        self.stale_origin = Some(origin);
        self.confirm_overwrite = false;
    }

    /// The host restated the block index, the threads and the highlights: the row map and the
    /// decor are the join of those with the view's rows, so they are recomputed.
    pub fn set_annotations(&mut self, annotations: AnnotationsBody) {
        self.annotations = annotations;
        // A thread that went away takes the focus with it.
        if let Some(id) = self.focused
            && self.annotation(id).is_none()
        {
            self.focused = None;
        }
        // An edit whose comment went away has nothing left to edit.
        if let Some(ComposerTarget::Edit(annotation, comment)) = self.composer
            && !self
                .annotation(annotation)
                .is_some_and(|a| a.thread.iter().any(|c| c.id == comment))
        {
            self.composer = None;
            self.composer_text.clear();
        }
        self.decor_stale = true;
    }

    /// The host stated where the document has been edited and by how much — the answer to
    /// `ListPlanChanges`.
    pub fn set_changes(&mut self, regions: Vec<PlanChangedRegion>, stats: PlanChangeStats) {
        self.changes = regions;
        self.change_stats = stats;
    }

    /// Whether any run in the current answer was left by the origin asked about — what the
    /// footer's legend is drawn from, so a hue is only ever offered when it is on screen.
    pub fn has_changes_by(&self, origin: SaveOrigin) -> bool {
        self.changes.iter().any(|region| region.origin == origin)
    }

    /// What the buffer now holds, against what the host last stated.
    pub fn refresh_dirty(&mut self, typed: &str) {
        self.dirty = typed != self.saved;
        if !self.dirty {
            self.stale = false;
            self.stale_origin = None;
            self.confirm_overwrite = false;
        }
    }

    /// The block the composer is drafting a fresh annotation about.
    pub fn composer_block(&self) -> Option<BlockId> {
        match self.composer {
            Some(ComposerTarget::Block(id)) => Some(id),
            _ => None,
        }
    }

    /// The threads the rail lists, in document order ([`ordered`]) — every open thread, plus every
    /// resolved one once `show_resolved` is on. A pure read, so the rail's virtualized list asks
    /// it fresh for every index (T-152).
    pub fn rail_threads(&self) -> Vec<AnnotationId> {
        ordered(&self.rows, self.annotations.annotations())
            .into_iter()
            .filter(|annotation| self.show_resolved || annotation.is_open())
            .map(|annotation| annotation.id)
            .collect()
    }

    /// How many open threads end on the user's own comment — what the host's *Ask agent* would
    /// queue (`AskDocAgent` with no ids).
    pub fn awaiting_agent(&self) -> usize {
        self.annotations
            .annotations()
            .iter()
            .filter(|annotation| {
                annotation.is_open()
                    && !annotation.review
                    && annotation
                        .thread
                        .last()
                        .is_some_and(|c| c.author == ubiq_proto::work::CommentAuthor::User)
            })
            .count()
    }

    /// Every annotation naming a given block, open and resolved alike.
    pub fn annotations_for(&self, block_id: BlockId) -> impl Iterator<Item = &Annotation> {
        self.annotations
            .annotations()
            .iter()
            .filter(move |annotation| annotation.block_id == block_id)
    }

    pub fn annotation(&self, annotation_id: AnnotationId) -> Option<&Annotation> {
        self.annotations
            .annotations()
            .iter()
            .find(|annotation| annotation.id == annotation_id)
    }

    /// The view row a thread sits in, `None` when it is orphaned or its block is not in the body.
    pub fn row_of_annotation(&self, annotation_id: AnnotationId) -> Option<usize> {
        row_of_annotation(&self.rows, self.annotation(annotation_id)?)
    }

    /// Rebuild the row map against the view's current parse.
    pub fn remap(&mut self, parsed: &ubiq_md::Document) {
        self.rows = row_map(parsed, self.annotations.blocks(), &parsed.source);
        self.decor_stale = false;
    }

    /// What the view's margins carry, as of the last [`Self::remap`].
    pub fn decor(&self) -> Vec<RowDecor> {
        row_decor(
            &self.rows,
            self.annotations.annotations(),
            self.annotations.highlights(),
            self.focused,
        )
    }
}

/// The composer's text read as an addressed comment: a leading `@agent` (then `:`, whitespace or
/// the end) addresses it to the agent and is stripped, and so does the composer's "@agent" toggle
/// (`to_agent`). Returns the addressee and the body left to post — possibly empty.
pub fn addressed(text: &str, to_agent: bool) -> (Option<Addressee>, String) {
    let text = text.trim();
    let tagged = text
        .strip_prefix("@agent")
        .filter(|rest| rest.is_empty() || rest.starts_with(|c: char| c == ':' || c.is_whitespace()))
        .map(|rest| rest.trim_start_matches([':', ' ', '\t', '\n']).trim());
    match tagged {
        Some(rest) => (Some(Addressee::Agent), rest.to_string()),
        None => (to_agent.then_some(Addressee::Agent), text.to_string()),
    }
}

/// Where each of the host's blocks sits in the text on screen.
///
/// The ids are the host's and the offsets are the buffer's, so something has to join them, and a
/// second parse in the window would only agree with the host's by luck. The blocks arrive in
/// document order with their own markdown, so a forward scan is both the simplest join and the
/// one that cannot cross two blocks with the same text: each search starts where the previous
/// block ended. A block whose text the editor has since changed simply finds nothing and is left
/// undecorated until the next save re-indexes it.
pub fn block_ranges(body: &str, blocks: &[PlanBlock]) -> Vec<(BlockId, Range<usize>)> {
    let mut found = Vec::new();
    let mut from = 0usize;
    for block in blocks {
        let needle = block.text.trim();
        if needle.is_empty() || from >= body.len() {
            continue;
        }
        let hit = body[from..]
            .find(needle)
            .map(|at| (from + at, needle.len()))
            .or_else(|| {
                // A block the editor has half-edited still anchors by its first line, which is
                // what a heading or a list item mostly is.
                let first = needle.lines().next()?.trim();
                if first.is_empty() {
                    return None;
                }
                body[from..].find(first).map(|at| (from + at, first.len()))
            });
        let Some((start, len)) = hit else {
            continue;
        };
        found.push((block.id, start..start + len));
        from = start + len;
    }
    found
}

/// Where each changed run sits in the text on screen, paired with who left it.
///
/// A [`PlanChangedRegion`] names 1-based inclusive line numbers into the body **as it is now**,
/// which is the body this was loaded with — so the join is arithmetic over line starts and never
/// a search, unlike the block one above. A run is clamped to the document and a run that starts
/// past its end is dropped: the host's numbering and the buffer's agree only until the buffer is
/// edited, and the library moves the decorations from there.
///
/// The range ends at the last line's last character, never at its newline: a decoration that
/// swallowed the line break would underline the gutter of the line below.
pub fn change_ranges(body: &str, regions: &[PlanChangedRegion]) -> Vec<(SaveOrigin, Range<usize>)> {
    // Byte offset of every line start, plus the end of the document as a sentinel.
    let mut starts = vec![0usize];
    starts.extend(
        body.match_indices('\n')
            .map(|(at, _)| at + 1)
            .filter(|at| *at <= body.len()),
    );

    regions
        .iter()
        .filter_map(|region| {
            let first = region.first_line.max(1) as usize - 1;
            let last = (region.last_line.max(region.first_line) as usize - 1).min(starts.len() - 1);
            let start = *starts.get(first)?;
            let end = match starts.get(last + 1) {
                // The next line's start, less the newline that separates them.
                Some(next) => next.saturating_sub(1),
                None => body.len(),
            };
            // A run clamped to the end of a document that ends in a newline would otherwise take
            // the newline with it.
            let end = body
                .get(..end)
                .map_or(end, |head| head.trim_end_matches('\n').len());
            (start < end).then_some((region.origin, start..end))
        })
        .collect()
}

/// One entry in the `/` menu.
///
/// `insert` is what replaces the typed `/word`; `caret` is where the cursor wants to be inside it
/// once it lands, counted in bytes from the start of the insertion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SlashCommand {
    pub label: &'static str,
    pub detail: &'static str,
    pub insert: &'static str,
}

/// The whole `/` menu, in the order it is offered.
///
/// Markdown a planner types over and over, and nothing else: the menu is a shortcut, not a
/// feature surface. It is plain data so the provider stays a mapping and the test stays a
/// comparison.
pub const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        label: "/heading",
        detail: "A section heading",
        insert: "## ",
    },
    SlashCommand {
        label: "/step",
        detail: "A numbered step",
        insert: "1. ",
    },
    SlashCommand {
        label: "/task",
        detail: "A checklist item",
        insert: "- [ ] ",
    },
    SlashCommand {
        label: "/bullet",
        detail: "A bullet",
        insert: "- ",
    },
    SlashCommand {
        label: "/code",
        detail: "A fenced code block",
        insert: "```\n\n```",
    },
    SlashCommand {
        label: "/table",
        detail: "A two-column table",
        insert: "| What | Why |\n|---|---|\n|  |  |",
    },
    SlashCommand {
        label: "/quote",
        detail: "A block quote",
        insert: "> ",
    },
    SlashCommand {
        label: "/rule",
        detail: "A horizontal rule",
        insert: "\n---\n",
    },
    SlashCommand {
        label: "/question",
        detail: "An open question for the reader",
        insert: "**Open question:** ",
    },
];

/// The commands whose label starts with what has been typed — `/` alone offers all of them.
pub fn slash_commands(typed: &str) -> Vec<&'static SlashCommand> {
    let typed = typed.trim();
    SLASH_COMMANDS
        .iter()
        .filter(|command| command.label.starts_with(typed))
        .collect()
}

/// The `/word` immediately behind a byte offset, or `None` when the caret is not in one.
///
/// The menu is a line-start affordance in spirit but not in rule: what matters is that a `/` with
/// no whitespace after it sits behind the caret, which is the same thing the library's own
/// trigger asks about.
pub fn slash_prefix(text: &str, offset: usize) -> Option<&str> {
    let head = text.get(..offset)?;
    let start = head.rfind('/')?;
    let word = &head[start..];
    if word.chars().skip(1).any(|ch| ch.is_whitespace()) {
        return None;
    }
    // A `/` inside a word — a path, a date — is not a command.
    if head[..start]
        .chars()
        .next_back()
        .is_some_and(|ch| !ch.is_whitespace())
    {
        return None;
    }
    Some(word)
}

// ── Rows: the join between the host's blocks and the renderer's ─────

/// Which `ubiq_md` root row each of the host's blocks sits in, and the reverse.
///
/// The host indexes blocks its own way (`PlanBlock`), the renderer lays out rows its own way
/// (`ubiq_md::Document::blocks`); the join is [`block_ranges`] for block → source range and
/// `Document::block_at_offset` for offset → row. Several host blocks can share a row (a list is
/// one row and each item a block), and a block whose text the body no longer holds belongs to no
/// row and is simply left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowMap {
    /// Per row, its host blocks in document order. Always as long as the document's row count.
    row_blocks: Vec<Vec<BlockId>>,
    /// Each mapped block's row and source range.
    placed: HashMap<BlockId, (usize, Range<usize>)>,
}

impl RowMap {
    /// The row a block sits in, or `None` for a block the body does not hold.
    pub fn row_of(&self, block: BlockId) -> Option<usize> {
        self.placed.get(&block).map(|(row, _)| *row)
    }

    /// The host blocks in a row, in document order.
    pub fn blocks_in(&self, row: usize) -> &[BlockId] {
        self.row_blocks.get(row).map_or(&[], Vec::as_slice)
    }

    pub fn rows(&self) -> usize {
        self.row_blocks.len()
    }
}

/// Map each host block to the row containing the start of its source range.
pub fn row_map(doc: &ubiq_md::Document, blocks: &[PlanBlock], body: &str) -> RowMap {
    let mut map = RowMap {
        row_blocks: vec![Vec::new(); doc.blocks.len()],
        placed: HashMap::new(),
    };
    for (id, range) in block_ranges(body, blocks) {
        let Some(row) = doc.block_at_offset(range.start) else {
            continue;
        };
        map.row_blocks[row].push(id);
        map.placed.insert(id, (row, range));
    }
    map
}

/// What a row carries in the margin: its threads, the flags standing on its open ones, its
/// highlight, and whether the focused thread is one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowDecor {
    pub row: usize,
    pub open: usize,
    pub resolved: usize,
    /// The union of the marks on the row's open threads, in `AnnotationMark` declaration order.
    pub marks: Vec<AnnotationMark>,
    pub highlight: Option<HighlightColour>,
    pub focused: bool,
}

/// One entry per row that has a thread, a highlight or the focus, in row order. A row sums the
/// threads of every block in it; orphaned threads and those on unmapped blocks have no row.
pub fn row_decor(
    map: &RowMap,
    annotations: &[Annotation],
    highlights: &[BlockHighlight],
    focused: Option<AnnotationId>,
) -> Vec<RowDecor> {
    let mut rows: BTreeMap<usize, RowDecor> = BTreeMap::new();
    fn entry(rows: &mut BTreeMap<usize, RowDecor>, row: usize) -> &mut RowDecor {
        rows.entry(row).or_insert(RowDecor {
            row,
            open: 0,
            resolved: 0,
            marks: Vec::new(),
            highlight: None,
            focused: false,
        })
    }
    for annotation in annotations.iter().filter(|a| !a.orphaned) {
        let Some(row) = map.row_of(annotation.block_id) else {
            continue;
        };
        let decor = entry(&mut rows, row);
        match annotation.state {
            AnnotationState::Open => {
                decor.open += 1;
                for mark in &annotation.marks {
                    if !decor.marks.contains(mark) {
                        decor.marks.push(*mark);
                    }
                }
            }
            AnnotationState::Resolved => decor.resolved += 1,
        }
        if focused == Some(annotation.id) {
            decor.focused = true;
        }
    }
    for highlight in highlights {
        if let Some(row) = map.row_of(highlight.block_id) {
            entry(&mut rows, row).highlight = Some(highlight.colour);
        }
    }
    let mut out: Vec<RowDecor> = rows.into_values().collect();
    for decor in &mut out {
        decor.marks.sort_by_key(|mark| match mark {
            AnnotationMark::Agent => 0,
            AnnotationMark::Todo => 1,
            AnnotationMark::Question => 2,
        });
    }
    out
}

/// The host block a new thread on `row` targets: the one whose source range contains `offset`,
/// else the row's first. `None` for a row with no host block.
pub fn target_block(map: &RowMap, row: usize, offset: Option<usize>) -> Option<BlockId> {
    let blocks = map.blocks_in(row);
    offset
        .and_then(|at| {
            blocks.iter().find(|id| {
                map.placed
                    .get(id)
                    .is_some_and(|(_, range)| range.contains(&at))
            })
        })
        .or_else(|| blocks.first())
        .copied()
}

/// The row of an annotation, `None` when it is orphaned or its block is not in the body.
fn row_of_annotation(map: &RowMap, annotation: &Annotation) -> Option<usize> {
    if annotation.orphaned {
        return None;
    }
    map.row_of(annotation.block_id)
}

/// The threads on a row, oldest first.
fn threads_on<'a>(map: &RowMap, annotations: &'a [Annotation], row: usize) -> Vec<&'a Annotation> {
    let mut here: Vec<&Annotation> = annotations
        .iter()
        .filter(|a| row_of_annotation(map, a) == Some(row))
        .collect();
    here.sort_by_key(|a| a.created_at);
    here
}

/// Every thread in document order, by row then creation, which is the side list's order so that
/// scrolling the document walks the list. Threads with no row sort last.
pub fn ordered<'a>(map: &RowMap, annotations: &'a [Annotation]) -> Vec<&'a Annotation> {
    let mut out: Vec<&Annotation> = annotations.iter().collect();
    out.sort_by_key(|a| {
        (
            row_of_annotation(map, a).unwrap_or(usize::MAX),
            a.created_at,
        )
    });
    out
}

/// Which thread the side list focuses, given the rows now on screen: `current` while its row is
/// still visible, else the first open thread (in [`ordered`] order) on a visible row. `None`
/// leaves the focus where it is. Resolved threads are never focused.
pub fn follow_target(
    map: &RowMap,
    annotations: &[Annotation],
    visible: Range<usize>,
    current: Option<AnnotationId>,
) -> Option<AnnotationId> {
    let here = |a: &Annotation| {
        a.state == AnnotationState::Open
            && row_of_annotation(map, a).is_some_and(|row| visible.contains(&row))
    };
    current
        .filter(|id| annotations.iter().any(|a| a.id == *id && here(a)))
        .or_else(|| {
            ordered(map, annotations)
                .into_iter()
                .find(|a| here(a))
                .map(|a| a.id)
        })
}

/// Which thread marking `row` toggles: the focused one when it sits on the row, else the row's
/// first. `None` when the row has no thread, where marking makes one.
pub fn mark_target(
    map: &RowMap,
    annotations: &[Annotation],
    row: usize,
    focused: Option<AnnotationId>,
) -> Option<AnnotationId> {
    let here = threads_on(map, annotations, row);
    focused
        .filter(|id| here.iter().any(|a| a.id == *id))
        .or_else(|| here.first().map(|a| a.id))
}

/// What a row's resolve control does: resolve (`true`) the focused thread if it is open on the
/// row, else the row's first open one; with none open, reopen (`false`) the
/// [`mark_target`]. `None` when the row has no thread.
pub fn resolve_target(
    map: &RowMap,
    annotations: &[Annotation],
    row: usize,
    focused: Option<AnnotationId>,
) -> Option<(AnnotationId, bool)> {
    let open: Vec<&Annotation> = threads_on(map, annotations, row)
        .into_iter()
        .filter(|a| a.state == AnnotationState::Open)
        .collect();
    focused
        .and_then(|id| open.iter().find(|a| a.id == id))
        .or_else(|| open.first())
        .map(|a| (a.id, true))
        .or_else(|| mark_target(map, annotations, row, focused).map(|id| (id, false)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ubiq_proto::plan::AnnotationState;
    use ubiq_proto::work::CommentAuthor;

    fn block(text: &str) -> PlanBlock {
        PlanBlock {
            id: BlockId::generate(),
            kind: "paragraph".to_string(),
            text: text.to_string(),
            lineage: String::new(),
        }
    }

    fn heading(depth: u8, text: &str) -> PlanBlock {
        PlanBlock {
            id: BlockId::generate(),
            kind: format!("heading:{depth}"),
            text: text.to_string(),
            lineage: String::new(),
        }
    }

    fn annotation(block_id: BlockId, open: bool) -> Annotation {
        let mut annotation = Annotation::new(
            block_id,
            None,
            CommentAuthor::User,
            "is that real?".to_string(),
            Utc::now(),
        );
        if !open {
            annotation.state = AnnotationState::Resolved;
        }
        annotation
    }

    #[test]
    fn blocks_are_found_in_document_order() {
        let body = "# Title\n\nFirst para.\n\nFirst para.\n";
        let blocks = vec![block("# Title"), block("First para."), block("First para.")];
        let ranges = block_ranges(body, &blocks);
        assert_eq!(ranges.len(), 3);
        assert_eq!(&body[ranges[0].1.clone()], "# Title");
        // The two identical paragraphs take different ranges: the scan never goes backwards.
        assert_eq!(ranges[1].1.start, 9);
        assert_eq!(ranges[2].1.start, 22);
    }

    #[test]
    fn an_edited_block_anchors_by_its_first_line() {
        let body = "## Scope\nsomething else entirely\n";
        let blocks = vec![block("## Scope\nthe original second line")];
        let ranges = block_ranges(body, &blocks);
        assert_eq!(&body[ranges[0].1.clone()], "## Scope");
    }

    #[test]
    fn a_vanished_block_takes_no_range() {
        let body = "nothing of the sort\n";
        let ranges = block_ranges(body, &[block("gone")]);
        assert!(ranges.is_empty());
    }

    fn region(first: u32, last: u32, origin: SaveOrigin) -> PlanChangedRegion {
        PlanChangedRegion {
            first_line: first,
            last_line: last,
            revision: 2,
            origin,
            block_id: None,
            text: String::new(),
        }
    }

    #[test]
    fn changed_lines_become_ranges_that_stop_at_the_line_end() {
        let body = "# Title\n\nAgent wrote this.\nAnd this.\n\nA human wrote this.\n";
        let ranges = change_ranges(
            body,
            &[
                region(3, 4, SaveOrigin::Agent),
                region(6, 6, SaveOrigin::Human),
            ],
        );
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].0, SaveOrigin::Agent);
        assert_eq!(&body[ranges[0].1.clone()], "Agent wrote this.\nAnd this.");
        assert_eq!(ranges[1].0, SaveOrigin::Human);
        assert_eq!(&body[ranges[1].1.clone()], "A human wrote this.");
    }

    #[test]
    fn a_run_past_the_end_of_the_document_is_dropped() {
        let body = "one\ntwo\n";
        // The buffer shrank under an answer counted against a longer body.
        let ranges = change_ranges(body, &[region(9, 12, SaveOrigin::Human)]);
        assert!(ranges.is_empty());
        // A run ending past the end is clamped rather than dropped.
        let clamped = change_ranges(body, &[region(2, 12, SaveOrigin::Human)]);
        assert_eq!(&body[clamped[0].1.clone()], "two");
    }

    #[test]
    fn slash_filters_by_what_was_typed() {
        assert_eq!(slash_commands("/").len(), SLASH_COMMANDS.len());
        let narrowed = slash_commands("/ta");
        assert_eq!(narrowed.len(), 2);
        assert!(narrowed.iter().any(|command| command.label == "/task"));
        assert!(narrowed.iter().any(|command| command.label == "/table"));
        assert!(slash_commands("/zzz").is_empty());
    }

    #[test]
    fn a_slash_prefix_is_a_word_at_a_boundary() {
        assert_eq!(slash_prefix("/he", 3), Some("/he"));
        assert_eq!(slash_prefix("write /co", 9), Some("/co"));
        assert_eq!(slash_prefix("write /co then", 14), None);
        // Not a command: a path.
        assert_eq!(slash_prefix("src/state", 9), None);
    }

    /// T-183: a file document defaults away from tracking updates, a plan defaults into it.
    #[test]
    fn track_updates_defaults_by_document_kind() {
        let project = ProjectId::generate();
        assert!(!tracks_updates_by_default(&DocumentHandle::File {
            project_id: project,
            rel_path: "notes.md".to_string(),
        }));
        assert!(tracks_updates_by_default(&DocumentHandle::Plan {
            project_id: project,
            task_id: TaskId::generate(),
        }));
    }

    #[test]
    fn addressed_reads_the_tag_and_the_toggle() {
        assert_eq!(
            addressed("@agent: look here", false),
            (Some(Addressee::Agent), "look here".to_string())
        );
        assert_eq!(
            addressed("@agent", false),
            (Some(Addressee::Agent), String::new())
        );
        assert_eq!(addressed("plain", false), (None, "plain".to_string()));
        assert_eq!(
            addressed(" plain ", true),
            (Some(Addressee::Agent), "plain".to_string())
        );
        // Not a tag: a word that merely starts with it.
        assert_eq!(addressed("@agents", false), (None, "@agents".to_string()));
    }

    // ── Rows ────────────────────────────────────────────────────────

    fn threaded(map_block: &PlanBlock, secs: i64) -> Annotation {
        let mut a = annotation(map_block.id, true);
        a.created_at = Utc::now() + chrono::Duration::seconds(secs);
        a
    }

    #[test]
    fn a_list_row_sums_its_items_threads() {
        let body = "- one\n- two\n\nafter\n";
        let (one, two, after) = (block("one"), block("two"), block("after"));
        let doc = ubiq_md::parse(body);
        let map = row_map(&doc, &[one.clone(), two.clone(), after.clone()], body);
        assert_eq!(map.row_of(one.id), Some(0));
        assert_eq!(map.row_of(two.id), Some(0));
        assert_eq!(map.row_of(after.id), Some(1));
        assert_eq!(map.blocks_in(0), &[one.id, two.id]);

        let mut done = annotation(two.id, false);
        done.marks = vec![AnnotationMark::Question];
        let mut open = annotation(one.id, true);
        open.marks = vec![AnnotationMark::Agent, AnnotationMark::Question];
        let decor = row_decor(
            &map,
            &[open.clone(), done, annotation(one.id, true)],
            &[],
            Some(open.id),
        );
        assert_eq!(decor.len(), 1);
        assert_eq!((decor[0].open, decor[0].resolved), (2, 1));
        assert_eq!(
            decor[0].marks,
            vec![AnnotationMark::Agent, AnnotationMark::Question]
        );
        assert!(decor[0].focused);
    }

    #[test]
    fn heading_and_paragraph_are_separate_rows_with_highlights() {
        let body = "# Title\n\nBody text.\n";
        let (h, p) = (heading(1, "# Title"), block("Body text."));
        let doc = ubiq_md::parse(body);
        let map = row_map(&doc, &[h.clone(), p.clone()], body);
        let highlights = [BlockHighlight {
            block_id: h.id,
            colour: HighlightColour::Green,
        }];
        let decor = row_decor(&map, &[annotation(p.id, true)], &highlights, None);
        assert_eq!(decor.len(), 2);
        assert_eq!(
            (decor[0].row, decor[0].highlight, decor[0].open),
            (0, Some(HighlightColour::Green), 0)
        );
        assert_eq!(
            (decor[1].row, decor[1].highlight, decor[1].open),
            (1, None, 1)
        );
        assert!(!decor[1].focused);
    }

    #[test]
    fn an_edited_body_shifts_the_rows() {
        let blocks = [block("First."), block("Second.")];
        let before = "First.\n\nSecond.\n";
        let after = "New intro.\n\nFirst.\n\nSecond.\n";
        let a = row_map(&ubiq_md::parse(before), &blocks, before);
        let b = row_map(&ubiq_md::parse(after), &blocks, after);
        assert_eq!(a.row_of(blocks[1].id), Some(1));
        assert_eq!(b.row_of(blocks[1].id), Some(2));
        assert_eq!(b.rows(), 3);
    }

    #[test]
    fn a_missing_block_has_no_row_and_no_decor() {
        let body = "Only this.\n";
        let (here, gone) = (block("Only this."), block("Vanished entirely."));
        let map = row_map(&ubiq_md::parse(body), &[here.clone(), gone.clone()], body);
        assert_eq!(map.row_of(gone.id), None);
        let mut orphan = annotation(here.id, true);
        orphan.orphaned = true;
        let decor = row_decor(&map, &[annotation(gone.id, true), orphan], &[], None);
        assert!(decor.is_empty());
        assert_eq!(row_map(&ubiq_md::Document::default(), &[], "").rows(), 0);
    }

    #[test]
    fn target_block_prefers_the_offset_then_the_first() {
        let body = "- one\n- two\n";
        let (one, two) = (block("one"), block("two"));
        let map = row_map(&ubiq_md::parse(body), &[one.clone(), two.clone()], body);
        assert_eq!(
            target_block(&map, 0, Some(body.find("two").unwrap())),
            Some(two.id)
        );
        assert_eq!(target_block(&map, 0, Some(0)), Some(one.id));
        assert_eq!(target_block(&map, 0, None), Some(one.id));
        assert_eq!(target_block(&map, 5, None), None);
    }

    #[test]
    fn follow_and_resolve_targets() {
        let body = "a\n\nb\n\nc\n";
        let blocks = [block("a"), block("b"), block("c")];
        let map = row_map(&ubiq_md::parse(body), &blocks, body);
        let c = threaded(&blocks[2], 0);
        let b1 = threaded(&blocks[1], 1);
        let b2 = threaded(&blocks[1], 2);
        let all = vec![c.clone(), b1.clone(), b2.clone()];

        let order: Vec<_> = ordered(&map, &all).iter().map(|a| a.id).collect();
        assert_eq!(order, vec![b1.id, b2.id, c.id]);

        assert_eq!(follow_target(&map, &all, 0..3, None), Some(b1.id));
        assert_eq!(follow_target(&map, &all, 2..3, None), Some(c.id));
        assert_eq!(follow_target(&map, &all, 0..3, Some(c.id)), Some(c.id));
        assert_eq!(follow_target(&map, &all, 0..1, Some(c.id)), None);

        assert_eq!(mark_target(&map, &all, 1, Some(b2.id)), Some(b2.id));
        assert_eq!(mark_target(&map, &all, 1, Some(c.id)), Some(b1.id));
        assert_eq!(mark_target(&map, &all, 0, None), None);

        assert_eq!(
            resolve_target(&map, &all, 1, Some(b2.id)),
            Some((b2.id, true))
        );
        assert_eq!(resolve_target(&map, &all, 1, None), Some((b1.id, true)));
        let mut closed = all.clone();
        closed[1].state = AnnotationState::Resolved;
        closed[2].state = AnnotationState::Resolved;
        assert_eq!(resolve_target(&map, &closed, 1, None), Some((b1.id, false)));
        assert_eq!(resolve_target(&map, &closed, 0, None), None);
        // Resolved threads are never followed.
        assert_eq!(follow_target(&map, &closed, 1..2, None), None);
    }
}
