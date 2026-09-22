//! The annotated-document surface: an editable markdown buffer, the threads anchored to passages
//! of it, and the handle that says *which* document is on screen.
//!
//! **This module is generic over a document on purpose.** Slice 6 builds one writing surface, not
//! a plan screen: the editor, the decorations, the thread popover and the `/` menu all read
//! [`DocumentEditor`], which names a [`DocumentHandle`] rather than a `TaskId`. A plan is the
//! handle's first — and, this wave, only — variant.
//!
//! What a second variant has to supply is exactly the three things the handle answers for:
//!
//! 1. **A body source and a save target** — a message that reads the document's markdown and one
//!    that replaces it whole. `crate::state::plan`'s is `LoadPlan`/`SavePlan`.
//! 2. **An annotation source** — a message that lists the blocks and their threads, and the three
//!    verbs that open, answer and close one. The block ids are the *host's*: a window never mints
//!    one, because matching blocks across a save is what keeps a thread anchored.
//! 3. **A store for both, on the host side.** For an arbitrary project `.md` file that is the open
//!    question — a sidecar inside the user's git repository is a decision nobody has taken — and
//!    it is why this wave stops at the seam.
//!
//! Nothing here renders, and nothing here builds a [`ubiq_proto::messages::Message`]: the wire is
//! `crate::app::plan`'s, which carries the `impl DocumentHandle` that turns a handle into the
//! family's messages.

use std::ops::Range;

use ubiq_proto::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use ubiq_proto::plan::{
    Annotation, PlanBlock, PlanChangeStats, PlanChangedRegion, PlanRevision, SaveOrigin,
};

use crate::state::editor::ViewLayout;

/// Which document an annotated surface is showing.
///
/// One variant today. A second one is a storage decision before it is a line of code — see the
/// module documentation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DocumentHandle {
    /// A task's plan, stored beside the project's tasks in the config root and reached through
    /// the plan family on the wire.
    Plan {
        project_id: ProjectId,
        task_id: TaskId,
    },
}

impl DocumentHandle {
    pub fn project_id(&self) -> ProjectId {
        match self {
            DocumentHandle::Plan { project_id, .. } => *project_id,
        }
    }

    /// The task a plan belongs to. `None` for a document that is not a plan — which is the whole
    /// point of the handle: no caller outside `state::plan` may assume a task is there.
    pub fn task_id(&self) -> Option<TaskId> {
        match self {
            DocumentHandle::Plan { task_id, .. } => Some(*task_id),
        }
    }

    /// What the surface calls this kind of document in its chrome.
    pub fn kind_label(&self) -> &'static str {
        match self {
            DocumentHandle::Plan { .. } => "Plan",
        }
    }

    /// A stable string for element ids and buffer keys — one document, one key.
    pub fn key(&self) -> String {
        match self {
            DocumentHandle::Plan { task_id, .. } => format!("plan:{task_id}"),
        }
    }
}

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
}

/// The outcome of a one-shot action the surface reports where the user is looking — a save that
/// was refused, an export that landed — rather than as a toast.
pub enum Notice {
    Ok(String),
    Err(String),
}

/// What the composer beside the document is answering — one at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ComposerTarget {
    /// A fresh annotation on this block, quoting whatever passage of it was selected.
    Block(BlockId),
    /// A reply appended to this thread.
    Reply(AnnotationId),
}

/// One annotated document, while its surface is open. One at a time per window: opening another
/// replaces it, the way the rest of the window's overlays behave.
pub struct DocumentEditor {
    pub doc: DocumentHandle,
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
    /// revision it expects to replace and the host refuses it with `PlanConflict` otherwise, so a
    /// save landing between the question and the answer cannot be overwritten by it. What this
    /// decides is only whether the next press names `revision` or `host_revision`. Cleared
    /// whenever the host's copy moves again, so one confirmation covers one revision and never
    /// the next one.
    pub confirm_overwrite: bool,
    /// A save is in flight: the surface waits for the host's answer before calling itself clean.
    pub saving: bool,
    /// Escape was pressed over an unsaved edit once. The second one means it.
    pub confirm_close: bool,
    /// The decorations no longer match the text or the threads — recomputed in the frame that
    /// notices, never on every frame, because the join is a scan over the whole document.
    pub decor_stale: bool,
    /// Where the document was edited since the beginning, and by whom — the host's answer to
    /// `ListPlanChanges`, in *current* line numbers, painted as the second decoration layer.
    pub changes: Vec<PlanChangedRegion>,
    /// How much changed, for the footer. Zeroed until the host has answered once.
    pub change_stats: PlanChangeStats,
    /// The change decorations no longer match the text — `decor_stale`'s twin, kept apart because
    /// the two layers are refreshed by different answers.
    pub change_decor_stale: bool,
    /// Source, preview, or both — the file editor's own toggle, on the same enum, because this is
    /// the same kind of surface and not a dialog with a preview bolted on.
    pub layout: ViewLayout,
    pub annotations: AnnotationsBody,
    /// A thread shown in the popover anchored to its passage, rather than only in the rail.
    pub thread: Option<AnnotationId>,
    /// Resolved threads are hidden in the rail until this is on. They are never dropped.
    pub show_resolved: bool,
    pub composer: Option<ComposerTarget>,
    /// The passage the composer is quoting, when the selection gave one.
    pub composer_quote: Option<String>,
    /// Mirrored out of `AppState::annotation_composer_input`: the entity behind the field is the
    /// window's, what was typed is the document's.
    pub composer_text: String,
    pub notice: Option<Notice>,
}

impl DocumentEditor {
    /// A freshly opened surface, waiting on the host's two answers.
    pub fn loading(doc: DocumentHandle) -> Self {
        Self {
            doc,
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
            change_decor_stale: false,
            layout: ViewLayout::Source,
            annotations: AnnotationsBody::Loading,
            thread: None,
            show_resolved: false,
            composer: None,
            composer_quote: None,
            composer_text: String::new(),
            notice: None,
        }
    }

    pub fn project_id(&self) -> ProjectId {
        self.doc.project_id()
    }

    pub fn task_id(&self) -> Option<TaskId> {
        self.doc.task_id()
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
        let edited = typed != self.saved;
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
    /// is asked again about *this* revision. `revision` is taken as the host's word, so the press
    /// that follows names it rather than something already overtaken.
    pub fn save_refused(&mut self, revision: PlanRevision, origin: SaveOrigin) {
        self.saving = false;
        self.host_revision = revision;
        self.stale = true;
        self.dirty = true;
        self.stale_origin = Some(origin);
        self.confirm_overwrite = false;
    }

    /// The host restated the block index and the threads: the decorations are the join of the
    /// two, so they are recomputed on the next frame.
    pub fn set_annotations(&mut self, annotations: AnnotationsBody) {
        self.annotations = annotations;
        self.decor_stale = true;
    }

    /// The host stated where the document has been edited and by how much — the answer to
    /// `ListPlanChanges`. The regions are in current line numbers, so they are a decoration the
    /// next frame paints rather than anything to reconcile here.
    pub fn set_changes(&mut self, regions: Vec<PlanChangedRegion>, stats: PlanChangeStats) {
        self.changes = regions;
        self.change_stats = stats;
        self.change_decor_stale = true;
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

/// The range a thread points at: its quoted passage where it has one and the passage is still
/// there, and the whole block otherwise.
pub fn annotation_range(
    body: &str,
    ranges: &[(BlockId, Range<usize>)],
    annotation: &Annotation,
) -> Option<Range<usize>> {
    let block = ranges
        .iter()
        .find(|(id, _)| *id == annotation.block_id)
        .map(|(_, range)| range.clone())?;
    let Some(quote) = annotation.quote.as_deref() else {
        return Some(block);
    };
    let quote = quote.trim();
    if quote.is_empty() {
        return Some(block);
    }
    match body.get(block.clone()).and_then(|text| text.find(quote)) {
        Some(at) => Some(block.start + at..block.start + at + quote.len()),
        None => Some(block),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ubiq_proto::work::CommentAuthor;

    fn block(text: &str) -> PlanBlock {
        PlanBlock {
            id: BlockId::generate(),
            kind: "paragraph".to_string(),
            text: text.to_string(),
        }
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

    #[test]
    fn a_quote_narrows_the_range_to_the_passage() {
        let body = "Ship the thing by Friday.\n";
        let blocks = vec![block("Ship the thing by Friday.")];
        let ranges = block_ranges(body, &blocks);
        let annotation = Annotation::new(
            blocks[0].id,
            Some("by Friday".to_string()),
            CommentAuthor::User,
            "is that real?".to_string(),
            Utc::now(),
        );
        let range = annotation_range(body, &ranges, &annotation).expect("the block is there");
        assert_eq!(&body[range], "by Friday");
    }

    #[test]
    fn a_quote_that_is_gone_falls_back_to_the_block() {
        let body = "Ship the thing.\n";
        let blocks = vec![block("Ship the thing.")];
        let ranges = block_ranges(body, &blocks);
        let annotation = Annotation::new(
            blocks[0].id,
            Some("by Friday".to_string()),
            CommentAuthor::User,
            "is that real?".to_string(),
            Utc::now(),
        );
        let range = annotation_range(body, &ranges, &annotation).expect("the block is there");
        assert_eq!(&body[range], "Ship the thing.");
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
}
