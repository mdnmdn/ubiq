//! The annotated-document surface: the document's markdown, the threads anchored to passages of
//! it, the section opened for editing, and the handle that says *which* document is on screen.
//!
//! **The surface reads the document and writes one section at a time.** There is no source view
//! and no layout: the body is drawn as a markdown preview, a section opened for editing is a field
//! over that section alone ([`SectionEdit`]), and confirming it splices back into the same buffer
//! the whole document is saved from. Every fact below about revisions, staleness and dirtiness is
//! unchanged by that — a section edit is an ordinary whole-document save.
//!
//! **This module is generic over a document on purpose.** Slice 6 builds one writing surface, not
//! a plan screen: the editor, the decorations, the thread popover and the `/` menu all read
//! [`DocumentEditor`], which names a [`DocumentHandle`] rather than a `TaskId`. A plan is the
//! handle's first variant and an ordinary markdown file in the project's tree is the second; the
//! surface cannot tell them apart, which is the point.
//!
//! **The handle is [`ubiq_proto::plan::DocumentHandle`], and it lives in the contract** rather
//! than here: every message in the family carries it, so a type the window owned would have to be
//! translated on the way out and matched back on the way in. Two variants today — a task's plan,
//! and an ordinary markdown file in the project's tree, annotated in place. What either one
//! answers for is the same three things:
//!
//! 1. **A body source and a save target** — `LoadPlan`/`SavePlan`, naming the handle.
//! 2. **An annotation source** — the blocks and their threads, and the three verbs that open,
//!    answer and close one. The block ids are the *host's*: a window never mints one, because
//!    matching blocks across a save is what keeps a thread anchored.
//! 3. **A store for both, on the host side.** A plan's is under the config root; a file's body is
//!    the file itself, with its sidecar at `<file>.md.annotation.json` beside it.
//!
//! Nothing here renders, and nothing here builds a [`ubiq_proto::messages::Message`]: the wire is
//! `crate::app::plan`'s, which carries the `impl DocumentHandle` that turns a handle into the
//! family's messages.

use std::ops::Range;

use gpui::Entity;
use gpui_component::input::TextareaState;
use ubiq_proto::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use ubiq_proto::plan::{
    Annotation, PlanBlock, PlanChangeStats, PlanChangedRegion, PlanRevision, SaveOrigin,
};

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

/// One section opened for editing in place, and the field holding its raw markdown.
///
/// The field's entity lives here rather than on `AppState` because **the widget's state is the
/// model**: a section edit exists only while a document is open, and which section is being edited
/// is the same fact as which buffer is on screen. It is built when the edit starts and dropped
/// when it is confirmed or cancelled, so nothing subscribes to it — the confirming press reads the
/// value.
pub struct SectionEdit {
    /// The block the edit replaces. The host's id, never one the window minted.
    pub block_id: BlockId,
    /// The section's markdown source, as typed.
    pub input: Entity<TextareaState>,
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

/// One annotated document, while its surface is open. One at a time per window: opening another
/// replaces it, the way the rest of the window's overlays behave.
pub struct DocumentEditor {
    pub doc: DocumentHandle,
    /// Where the surface is drawn. **The surface itself is the same either way** — the columns, the
    /// rail, the gutter and every action on them are `crate::ui::document`'s, and neither arm knows
    /// which one it is in. What this decides is who frames it: [`Presentation::Modal`] is the plan
    /// dialog (`ui/plan.rs`, raised over the window, peeled by Escape at `Layer::Plan`), and
    /// [`Presentation::Viewer`] is a markdown tab in `ViewLayout::Annotation`, drawn inside the
    /// panel with no scrim and no rung of its own.
    pub presentation: Presentation,
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
    /// The section opened for editing in place, if any. **The surface has no source view**: the
    /// document is read as a markdown preview and written one section at a time, so this is the
    /// only writing surface it offers.
    pub section_edit: Option<SectionEdit>,
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
    /// The heading navigator's own dropdown, down or not — a fact about this document alone, the
    /// same way `composer` and `thread` are, rather than the window's single `MenuId`: it opens off
    /// the document's own chrome, not the titlebar or a panel header the rest of that system reads.
    pub nav_open: bool,
    /// While composing a *new* thread, the rail hides every other thread so the one being written
    /// is the only thing beside the field — the user pressed the section's own affordance for a
    /// reason, and a long list of settled threads is noise against that. `true` overrides the hide,
    /// which is what the rail's "Show all threads" button sets; never persisted, and reset the
    /// moment the composer leaves the new-thread shape (cancelled, sent, or a reply opened instead).
    pub thread_focus_override: bool,
}

impl DocumentEditor {
    /// A freshly opened surface, waiting on the host's two answers.
    pub fn loading(doc: DocumentHandle, presentation: Presentation) -> Self {
        Self {
            doc,
            presentation,
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
            section_edit: None,
            annotations: AnnotationsBody::Loading,
            thread: None,
            show_resolved: false,
            composer: None,
            composer_quote: None,
            composer_text: String::new(),
            notice: None,
            nav_open: false,
            thread_focus_override: false,
        }
    }

    pub fn project_id(&self) -> ProjectId {
        self.doc.project_id()
    }

    pub fn task_id(&self) -> Option<TaskId> {
        self.doc.task_id()
    }

    /// The stable string that scopes every element id this surface draws — `ubiq_proto`'s own
    /// `DocumentHandle::key`, named again here because it is `ui::document` that reads it (T-125).
    /// Every `plan-*` id that has no other unique part of its own (a block id, an annotation id —
    /// both host-minted and already unique across documents) is scoped by this, so a second
    /// surface open on a different document does not collide with the first one's chrome.
    ///
    /// **This does not yet make two surfaces independently editable.** `AppState` still holds one
    /// `workbench.plan` slot, one shared buffer (`plan_editor`) and one shared composer field —
    /// opening a second annotated document still replaces the first rather than adding beside it.
    /// Only the id collision this method fixes is what's done; `_docs/backlog.md` carries the rest
    /// of "a `DocumentEditor` per surface rather than per window" as its own row.
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
        // The buffer is shared across whichever document is open (`ui::document`'s note on
        // `plan_editor`), so on a document's *first* answer it still holds whatever the surface
        // last showed — a previous document's text, or the launch default — and comparing that
        // against `self.saved` (always `""` for a document that has never loaded) reads as an
        // edit that was never made. Nothing has been typed into a document that has not loaded
        // yet, so there is nothing to protect: this body is taken unconditionally instead, and
        // the buffer is reseeded from it below like any other fresh load.
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

    /// The block currently open as a raw-markdown field, if any.
    pub fn editing_block(&self) -> Option<BlockId> {
        self.section_edit.as_ref().map(|edit| edit.block_id)
    }

    /// The block the composer is drafting a fresh annotation about.
    pub fn composer_block(&self) -> Option<BlockId> {
        match self.composer {
            Some(ComposerTarget::Block(id)) => Some(id),
            _ => None,
        }
    }

    /// Whether the rail should hide every thread but the one being composed — a new thread in
    /// progress, and the user has not asked to see the rest anyway.
    pub fn hides_other_threads(&self) -> bool {
        self.composer_block().is_some() && !self.thread_focus_override
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

    /// A section edit was confirmed and saved: patch the cached block(s) so the preview shows
    /// them now, rather than waiting on the host.
    ///
    /// **The host only restates the block index — `Message::PlanAnnotationsChanged` — when a save
    /// orphans a thread.** An edit that keeps every anchor (the ordinary case) never re-sends it,
    /// so `preview`'s per-section render, which reads this cache rather than the live buffer, kept
    /// showing what the section said before the edit until the surface was closed and reopened.
    /// This is the fix: the window already knows exactly what it just wrote, so it says so here
    /// instead of waiting for a round trip that may never come. The host's own word, whenever it
    /// does arrive, overwrites this again — this is only ever ahead of it, never in conflict.
    ///
    /// `parsed` is the edited text's own blocks — [`parse_section_blocks`]'s answer — so **a section
    /// that parsed into several blocks (a paragraph split by a blank line, say) is cached as several
    /// blocks**, never as one block holding embedded newlines: the gutter, the double-click-to-edit
    /// affordance and every other per-block thing in `ui::plan` reads this cache at block
    /// granularity, and a single lumped block would answer all of them for the whole span at once.
    /// An empty `parsed` is a section edited down to nothing, and the block is dropped outright — the
    /// preview should say so immediately rather than keep drawing a row for text no longer there.
    ///
    /// The id `block_id` carried is kept on the **first** resulting block and nowhere else, so a
    /// thread anchored here stays anchored to the part of the split it is still about, the same
    /// rule the host's own matcher applies when a block turns into several.
    pub fn replace_cached_block(&mut self, block_id: BlockId, parsed: Vec<(String, String)>) {
        let AnnotationsBody::Loaded { blocks, .. } = &mut self.annotations else {
            return;
        };
        let Some(pos) = blocks.iter().position(|block| block.id == block_id) else {
            return;
        };
        if parsed.is_empty() {
            blocks.remove(pos);
            return;
        }
        let replacement: Vec<PlanBlock> = parsed
            .into_iter()
            .enumerate()
            .map(|(index, (kind, text))| PlanBlock {
                id: if index == 0 {
                    block_id
                } else {
                    BlockId::generate()
                },
                kind,
                text,
            })
            .collect();
        blocks.splice(pos..=pos, replacement);
    }
}

/// The blocks a section's edited markdown parses into, kind and text alike — the same walk
/// `ubiq-host`'s own indexer runs (`crates/ubiq-host/src/plan/blocks.rs`), mirrored here rather
/// than shared because the window does not depend on the host crate. It exists only to patch the
/// local block cache optimistically, ahead of the host's own re-index, which remains authoritative
/// and overwrites whatever this guessed the moment it answers.
pub fn parse_section_blocks(text: &str) -> Vec<(String, String)> {
    use markdown::mdast::Node;

    fn is_container(node: &Node) -> bool {
        matches!(
            node,
            Node::Root(_)
                | Node::Blockquote(_)
                | Node::List(_)
                | Node::ListItem(_)
                | Node::FootnoteDefinition(_)
        )
    }

    fn kind_of(node: &Node) -> Option<String> {
        Some(match node {
            Node::Paragraph(_) => "paragraph".to_string(),
            Node::Heading(heading) => format!("heading:{}", heading.depth),
            Node::Code(_) => "code".to_string(),
            Node::Math(_) => "math".to_string(),
            Node::Table(_) => "table".to_string(),
            Node::ThematicBreak(_) => "break".to_string(),
            Node::Html(_) => "html".to_string(),
            Node::Definition(_) => "definition".to_string(),
            Node::Toml(_) | Node::Yaml(_) => "frontmatter".to_string(),
            _ => return None,
        })
    }

    fn walk(node: &Node, source: &str, out: &mut Vec<(String, String)>) {
        if is_container(node) {
            if let Some(children) = node.children() {
                for child in children {
                    walk(child, source, out);
                }
            }
            return;
        }
        let (Some(kind), Some(position)) = (kind_of(node), node.position()) else {
            return;
        };
        let Some(text) = source.get(position.start.offset..position.end.offset) else {
            return;
        };
        let text = text.trim();
        if !text.is_empty() {
            out.push((kind, text.to_string()));
        }
    }

    let Ok(ast) = markdown::to_mdast(text, &markdown::ParseOptions::gfm()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(&ast, text, &mut out);
    out
}

/// One heading in a document, indented by its own depth, and how many threads sit under it — open
/// against settled — for a navigator that lists a document's structure hierarchically.
///
/// **"Under it" is every block from this heading up to, but not including, the next heading of any
/// depth.** Not a nested rollup: a `##`'s count including every `###` beneath it would count the
/// same thread again for a second row on screen, and a flat count is what each row's own label
/// actually names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingEntry {
    pub block_id: BlockId,
    /// `1` for `#`, up to `6`.
    pub level: u8,
    /// The heading's own text, with its `#`s taken off — what the row says, not what it was
    /// written as.
    pub label: String,
    pub open: usize,
    pub resolved: usize,
}

/// The document's headings, in order, each carrying the open and resolved counts of the section it
/// starts — the data a markdown navigator draws, kept apart from the drawing so it can be tested
/// on its own.
pub fn heading_sections(blocks: &[PlanBlock], annotations: &[Annotation]) -> Vec<HeadingEntry> {
    fn heading_level(kind: &str) -> Option<u8> {
        kind.strip_prefix("heading:")?.parse().ok()
    }

    let mut out: Vec<HeadingEntry> = Vec::new();
    for block in blocks {
        match heading_level(&block.kind) {
            Some(level) => {
                let label = block.text.trim_start_matches('#').trim().to_string();
                let mut entry = HeadingEntry {
                    block_id: block.id,
                    level,
                    label,
                    open: 0,
                    resolved: 0,
                };
                count_into(&mut entry, block.id, annotations);
                out.push(entry);
            }
            // A block before the first heading has nowhere to add its count — a document that
            // opens with prose rather than a title, which the navigator simply has nothing to say
            // about yet.
            None => {
                if let Some(last) = out.last_mut() {
                    count_into(last, block.id, annotations);
                }
            }
        }
    }
    out
}

fn count_into(entry: &mut HeadingEntry, block_id: BlockId, annotations: &[Annotation]) {
    for annotation in annotations {
        if annotation.block_id != block_id {
            continue;
        }
        if annotation.is_open() {
            entry.open += 1;
        } else {
            entry.resolved += 1;
        }
    }
}

/// What the minimap draws a block as — a shape rather than shrunken text, in the spirit of
/// `_docs/inbox/markdown-improvement-proposal.md` §8.2's table. Every block kind the host indexes
/// today (`crates/ubiq-host/src/plan/blocks.rs::kind_of`) resolves to one of these except a
/// thematic break, raw HTML, a definition and frontmatter, which draw nothing — there is no shape
/// in the proposal's table for them and a mark for every block would be noise, not orientation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MinimapBlockKind {
    Heading,
    Paragraph,
    Code,
    Table,
    /// A paragraph that is a single image reference and nothing else — the host's parser has no
    /// block kind of its own for an image (it is phrasing inside a paragraph, not a block level
    /// node), so this is a heuristic over a paragraph's text rather than a fact the host states.
    Image,
}

/// One row the minimap draws for a block — exactly one per block, of every kind (T-134). A
/// paragraph or a table once drew one row per real source line; against the reference minimap's
/// own look that read as a barcode, a wall of same-height ticks with nothing to tell one block
/// from the next. One mark per block, sized to its widest line, is what keeps "short line, short
/// mark" (T-110) without losing the block-level shape a minimap is for. `row_index`/`row_count`
/// stay on the type for `ui/document.rs`'s own use — every row built today carries `(0, 1)`.
///
/// **`length` is measured in characters, not pixels.** There is no second layout pass to place a
/// mark by real glyph widths (proposal §8.5 asks that there not be one), and the preview's own
/// renderer (`ui::viewer::markdown`) exposes no per-line fragment geometry to measure instead —
/// character count against [`LINE_LENGTH_CHARS`] is the real content the line carries, only
/// approximated by count rather than by width. This is the divergence from proposal §8.2's own
/// wording ("shapes are legible at any scale", said of a native text system with line fragments
/// on hand); a monospace-ish approximation is what is reachable here.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimapRow {
    pub block_index: usize,
    /// Which row of this block this is, and how many the block has. There is no per-line pixel
    /// position either — `ui/document.rs` spreads `row_count` rows evenly across the block's own
    /// *measured* height (from `ScrollHandle::bounds_for_item`), so the rows stay anchored to the
    /// block's real vertical span even though their position inside it is interpolated.
    pub row_index: usize,
    pub row_count: usize,
    pub kind: MinimapBlockKind,
    /// `0.0..=1.0`, this row's own share of a full column width.
    pub length: f32,
}

/// Roughly how many characters of body text fill the document column at its own width
/// (`ui::document::DOC_WIDTH`-ish, at the content body size) — the normalising constant a real
/// line's character count is measured against, so "short line, short mark" is relative to what
/// the column can actually hold rather than to the longest line in the file.
pub(crate) const LINE_LENGTH_CHARS: f32 = 90.0;

/// The document's blocks, drawn as the minimap sees them — one entry per block for a heading, a
/// code block or an image, one per real line for a paragraph or a table row.
pub fn minimap_rows(blocks: &[PlanBlock]) -> Vec<MinimapRow> {
    let mut out = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        if block.kind.starts_with("heading:") {
            out.push(MinimapRow {
                block_index,
                row_index: 0,
                row_count: 1,
                kind: MinimapBlockKind::Heading,
                length: 0.85,
            });
            continue;
        }
        match block.kind.as_str() {
            "paragraph" if is_image_reference(&block.text) => out.push(MinimapRow {
                block_index,
                row_index: 0,
                row_count: 1,
                kind: MinimapBlockKind::Image,
                length: 0.55,
            }),
            "paragraph" => out.extend(text_rows(block_index, &block.text, MinimapBlockKind::Paragraph)),
            "code" | "math" => out.push(MinimapRow {
                block_index,
                row_index: 0,
                row_count: 1,
                kind: MinimapBlockKind::Code,
                length: 1.0,
            }),
            "table" => out.extend(table_rows(block_index, &block.text)),
            _ => {}
        }
    }
    out
}

/// A paragraph that is nothing but a Markdown image reference — `![alt](url)` alone on the line,
/// no surrounding prose. The host's parser folds an image into its paragraph as phrasing content
/// rather than giving it a block kind of its own, so this is the only way to tell one from an
/// ordinary paragraph at this layer.
pub(crate) fn is_image_reference(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("![") && t.ends_with(')') && t.matches("![").count() == 1
}

/// A paragraph's or a heading-less prose block's shape, as **one** mark rather than one per source
/// line (T-134). A mark per real line read as a barcode — a wall of identical-height ticks with no
/// air between them — against the reference minimap's own look (a handful of legible bars with
/// room around each). One mark per block, sized to its longest line, keeps the shape a block-level
/// map is for: where the headings, the prose and the code sit, not a transcription of every line.
fn text_rows(block_index: usize, text: &str, kind: MinimapBlockKind) -> Vec<MinimapRow> {
    let widest = text
        .lines()
        .map(|line| line.trim().len())
        .max()
        .unwrap_or(0);
    let length = if widest == 0 {
        0.3
    } else {
        (widest as f32 / LINE_LENGTH_CHARS).clamp(0.08, 1.0)
    };
    vec![MinimapRow {
        block_index,
        row_index: 0,
        row_count: 1,
        kind,
        length,
    }]
}

/// A table's shape, as **one** dotted mark rather than one per row — the same barcode problem
/// [`text_rows`] fixes, for the same reason. Sized to the widest row, so a wide table still reads
/// as wider than a narrow one.
fn table_rows(block_index: usize, text: &str) -> Vec<MinimapRow> {
    let widest = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_separator_row(line))
        .map(|line| line.trim_matches('|').len())
        .max();
    let Some(widest) = widest else {
        return Vec::new();
    };
    vec![MinimapRow {
        block_index,
        row_index: 0,
        row_count: 1,
        kind: MinimapBlockKind::Table,
        length: (widest as f32 / LINE_LENGTH_CHARS).clamp(0.15, 1.0),
    }]
}

fn is_separator_row(line: &str) -> bool {
    line.trim_matches('|')
        .chars()
        .all(|c| matches!(c, '-' | ':' | '|' | ' '))
}

/// One thread, positioned by where its block sits among the document's blocks — the data a
/// minimap draws, kept apart from the drawing on `heading_sections`'s own rule so it can be
/// tested on its own and so a mark clicked in the strip can be resolved back to an annotation the
/// same way a picked row in the navigator resolves back to a heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadMark {
    pub annotation_id: AnnotationId,
    /// The mark's position among `blocks` — an index, not a pixel: the caller with a real
    /// `ScrollHandle` turns it into one, and a caller with none (a test) can still assert on the
    /// ordering alone.
    pub block_index: usize,
    pub open: bool,
}

/// Every thread whose block is still in the document, in the blocks' own order — **an orphaned
/// thread (`annotation.block_id` matching no block) carries no position and is left out**, the
/// same as the gutter's own count: there is nothing on the page to mark it against.
pub fn thread_marks(blocks: &[PlanBlock], annotations: &[Annotation]) -> Vec<ThreadMark> {
    annotations
        .iter()
        .filter_map(|annotation| {
            let block_index = blocks
                .iter()
                .position(|block| block.id == annotation.block_id)?;
            Some(ThreadMark {
                annotation_id: annotation.id,
                block_index,
                open: annotation.is_open(),
            })
        })
        .collect()
}

/// Splice a section's edited text into the body. **An all-whitespace replacement removes the
/// section outright** — its blank-line gap with it — rather than saving an empty paragraph where
/// it stood: item 2 of the section-edit revamp is "a block edited down to nothing goes away", not
/// "a block edited down to nothing is kept, empty".
///
/// The gap absorbed is whichever side has one: forward first, so a section removed from the middle
/// of the document closes up against the one that follows it, and backward only when there is
/// nothing after it to close up against — the last block in the document.
pub fn splice_section(body: &str, range: Range<usize>, typed: &str) -> String {
    let typed = typed.trim();
    if typed.is_empty() {
        let mut start = range.start;
        let mut end = range.end;
        let kept_after = body[end..].trim_start_matches('\n').len();
        let gap_after = body[end..].len() - kept_after;
        if gap_after > 0 {
            end += gap_after;
        } else {
            start = body[..start].trim_end_matches('\n').len();
        }
        let mut next = String::with_capacity(body.len() - (end - start));
        next.push_str(&body[..start]);
        next.push_str(&body[end..]);
        return next;
    }
    let mut next = String::with_capacity(body.len() + typed.len());
    next.push_str(&body[..range.start]);
    next.push_str(typed);
    next.push_str(&body[range.end..]);
    next
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
    use ubiq_proto::plan::AnnotationState;
    use ubiq_proto::work::CommentAuthor;

    fn block(text: &str) -> PlanBlock {
        PlanBlock {
            id: BlockId::generate(),
            kind: "paragraph".to_string(),
            text: text.to_string(),
        }
    }

    fn heading(depth: u8, text: &str) -> PlanBlock {
        PlanBlock {
            id: BlockId::generate(),
            kind: format!("heading:{depth}"),
            text: text.to_string(),
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

    #[test]
    fn headings_carry_their_own_depth_and_the_hash_marks_are_taken_off() {
        let title = heading(1, "# Title");
        let scope = heading(2, "## Scope");
        let blocks = vec![title.clone(), scope.clone()];
        let entries = heading_sections(&blocks, &[]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, 1);
        assert_eq!(entries[0].label, "Title");
        assert_eq!(entries[1].level, 2);
        assert_eq!(entries[1].label, "Scope");
    }

    #[test]
    fn a_headings_count_is_its_own_section_only() {
        let title = heading(1, "# Title");
        let intro = block("Intro paragraph.");
        let scope = heading(2, "## Scope");
        let detail = block("A detail.");
        let blocks = vec![title.clone(), intro.clone(), scope.clone(), detail.clone()];
        let annotations = vec![
            annotation(intro.id, true),
            annotation(scope.id, false),
            annotation(detail.id, true),
            annotation(detail.id, false),
        ];
        let entries = heading_sections(&blocks, &annotations);

        assert_eq!(entries[0].open, 1, "the paragraph under Title counts there");
        assert_eq!(entries[0].resolved, 0);
        // Scope's own thread, plus the detail beneath it — flat, not rolled up into Title.
        assert_eq!(entries[1].open, 1);
        assert_eq!(entries[1].resolved, 2);
    }

    #[test]
    fn a_block_before_the_first_heading_has_nowhere_to_add_its_count() {
        let intro = block("No title yet.");
        let entries = heading_sections(&[intro.clone()], &[annotation(intro.id, true)]);
        assert!(entries.is_empty());
    }

    #[test]
    fn thread_marks_position_by_block_index_and_carry_open_state() {
        let first = block("First.");
        let second = block("Second.");
        let third = block("Third.");
        let blocks = vec![first.clone(), second.clone(), third.clone()];
        let open = annotation(second.id, true);
        let resolved = annotation(third.id, false);
        let annotations = vec![open.clone(), resolved.clone()];

        let marks = thread_marks(&blocks, &annotations);

        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].annotation_id, open.id);
        assert_eq!(marks[0].block_index, 1);
        assert!(marks[0].open);
        assert_eq!(marks[1].annotation_id, resolved.id);
        assert_eq!(marks[1].block_index, 2);
        assert!(!marks[1].open);
    }

    #[test]
    fn an_orphaned_thread_carries_no_mark() {
        let kept = block("Still here.");
        let annotations = vec![annotation(BlockId::generate(), true)];
        let marks = thread_marks(&[kept], &annotations);
        assert!(marks.is_empty());
    }

    #[test]
    fn an_emptied_section_closes_the_gap_it_leaves() {
        let body = "# Title\n\nFirst.\n\nSecond.\n\nThird.\n";
        let start = body.find("Second.").unwrap();
        let range = start..start + "Second.".len();
        let next = splice_section(body, range, "   \n  ");
        assert_eq!(next, "# Title\n\nFirst.\n\nThird.\n");
    }

    #[test]
    fn emptying_the_last_section_closes_the_gap_behind_it() {
        let body = "# Title\n\nFirst.\n\nSecond.\n";
        let start = body.find("Second.").unwrap();
        let range = start..start + "Second.".len();
        let next = splice_section(body, range, "\n");
        assert_eq!(next, "# Title\n\nFirst.\n\n");
    }

    #[test]
    fn an_ordinary_edit_still_just_replaces_the_range() {
        let body = "First.\n\nSecond.\n";
        let start = body.find("Second.").unwrap();
        let range = start..start + "Second.".len();
        let next = splice_section(body, range, "Rewritten.");
        assert_eq!(next, "First.\n\nRewritten.\n");
    }
}
