//! The annotated-document surface's actions: open a document, save it, and drive the threads
//! anchored to its blocks.
//!
//! **The document is drawn by an `MdView` with its annotation layer on**, and everything the view's
//! margin does arrives here as an `MdViewEvent` ([`AppState::document_md_event`]): a new thread, a
//! mark, a resolve, a highlight, a block commit, a reparse, a scroll. The view mutates no decor of
//! its own; this file acts on the host's state and pushes the result back through
//! `MdView::set_decor` ([`AppState::refresh_document_decor`]).
//!
//! **`Message::SavePlan` is sent from one place**, [`AppState::save_document`]: the body is
//! replaced whole, the host re-indexes the blocks, and the annotations that survive come back
//! through `PlanAnnotationsChanged` — which is why a save never touches a thread here. A block
//! committed in the dialog's view is such a save; a block committed in a markdown tab's view is an
//! edit to the tab, which saves the way every tab does.
//!
//! **Nothing in this file names a task except the plan's own entry points.** Everything between
//! opening and saving reads a [`DocumentHandle`], and the handle is what turns into a message.

use super::*;

use std::ops::Range;

use crate::state::document::{
    AnnotationsBody, ComposerTarget, DocumentEditor, DocumentHandle, Notice, Presentation,
    addressed, change_ranges, follow_target, mark_target, resolve_target, target_block,
};
use crate::state::plan::{file_document, mission_doc_document, plan_document};
use crate::state::workbench::FileDialog;
use crate::ui::mdview::events::MdViewEvent;
use crate::ui::mdview::view::MdView;
use ubiq_proto::ids::{AnnotationId, BlockId, CommentId, TaskId};
use ubiq_proto::plan::{
    AnnotationMark, HighlightColour, PlanChangeStats, PlanChangedRegion, PlanRevision, SaveOrigin,
};
use ubiq_proto::work::{AgentId, Addressee, CommentAuthor};

/// The wire, for any annotated document. The interface never invents a route: every method here
/// is a message that already exists in `crates/ubiq-proto/src/messages.rs`, and the handle is the
/// whole of what it names — the *host* is what answers a handle with a place on disk.
///
/// A trait rather than an inherent `impl` only because [`DocumentHandle`] is the contract's type
/// and this crate does not own it.
pub(crate) trait DocumentWire {
    fn load(&self) -> Message;
    fn list_annotations(&self) -> Message;
    /// `expected` is the revision this body is meant to replace, and the host refuses the write if
    /// the document no longer stands there. A confirmed overwrite names the newest revision the
    /// host has stated instead of the one the buffer was seeded from.
    fn save(&self, body: String, expected: PlanRevision) -> Message;
    fn annotate(
        &self,
        block_id: BlockId,
        text: String,
        marks: Vec<AnnotationMark>,
        to: Option<Addressee>,
    ) -> Message;
    fn reply(&self, annotation_id: AnnotationId, text: String, to: Option<Addressee>) -> Message;
    /// Set (`on`) or clear one flag on a thread.
    fn mark(&self, annotation_id: AnnotationId, mark: AnnotationMark, on: bool) -> Message;
    /// Colour blocks, or clear their colour with `None`.
    fn highlight(&self, block_ids: Vec<BlockId>, colour: Option<HighlightColour>) -> Message;
    /// Where the document has been edited, and by whom. `since` absent asks for the whole history.
    fn list_changes(&self, since: Option<PlanRevision>) -> Message;
    fn resolve(&self, annotation_id: AnnotationId, resolved: bool) -> Message;
}

impl DocumentWire for DocumentHandle {
    fn load(&self) -> Message {
        Message::LoadPlan { doc: self.clone() }
    }

    fn list_annotations(&self) -> Message {
        Message::ListPlanAnnotations {
            doc: self.clone(),
            open: false,
        }
    }

    fn save(&self, body: String, expected: PlanRevision) -> Message {
        Message::SavePlan {
            doc: self.clone(),
            body,
            expected,
        }
    }

    fn annotate(
        &self,
        block_id: BlockId,
        text: String,
        marks: Vec<AnnotationMark>,
        to: Option<Addressee>,
    ) -> Message {
        // No quote until the preview has a text selection (T-305): a thread is about its block.
        Message::AnnotatePlan {
            doc: self.clone(),
            block_id,
            quote: None,
            text,
            marks,
            to,
        }
    }

    fn reply(&self, annotation_id: AnnotationId, text: String, to: Option<Addressee>) -> Message {
        Message::ReplyToAnnotation {
            doc: self.clone(),
            annotation_id,
            text,
            to,
        }
    }

    fn mark(&self, annotation_id: AnnotationId, mark: AnnotationMark, on: bool) -> Message {
        Message::MarkAnnotation {
            doc: self.clone(),
            annotation_id,
            mark,
            on,
        }
    }

    fn highlight(&self, block_ids: Vec<BlockId>, colour: Option<HighlightColour>) -> Message {
        Message::SetBlockHighlight {
            doc: self.clone(),
            block_ids,
            colour,
        }
    }

    fn list_changes(&self, since: Option<PlanRevision>) -> Message {
        Message::ListPlanChanges {
            doc: self.clone(),
            since_revision: since,
        }
    }

    fn resolve(&self, annotation_id: AnnotationId, resolved: bool) -> Message {
        Message::ResolveAnnotation {
            doc: self.clone(),
            annotation_id,
            resolved,
        }
    }
}

impl AppState {
    /// Open a task's plan as an editable document. The affordance that calls this is only ever
    /// drawn for a task carrying a `level` — the host refuses the whole family for one with none.
    pub fn open_plan(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.open_document(plan_document(project_id, task_id), cx);
    }

    /// Open one of a mission's own documents (M8), on the plan dialog's own footing.
    pub fn open_mission_doc(&mut self, task_id: TaskId, name: &str, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.open_document(mission_doc_document(project_id, task_id, name), cx);
    }

    /// Open any annotated document in the dialog: ask for its body and its threads in the same
    /// breath, over a view built for it on the dialog's buffer.
    pub fn open_document(&mut self, doc: DocumentHandle, cx: &mut Context<Self>) {
        let config = self.md_view_config();
        let buffer = self.plan_editor.clone();
        let md = cx.new(|cx| {
            let mut view = MdView::new(buffer, config, cx);
            view.set_annotating(true, cx);
            view
        });
        let events = cx.subscribe(&md, |this, md, event: &MdViewEvent, cx| {
            this.document_md_event(&md, event, cx)
        });
        self.open_with(doc, Presentation::Modal, md, Some(events), cx);
    }

    /// Open a project markdown file as an annotated document over the tab's own view, for a tab
    /// put into `ViewLayout::Annotation` — [`crate::state::plan::file_document`]'s one caller.
    ///
    /// Idempotent: a tab redrawn, or the layout pressed twice, must not re-ask for a document that
    /// is already open.
    pub fn open_file_document(
        &mut self,
        rel_path: &str,
        md: Entity<MdView>,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let doc = file_document(project_id, rel_path);
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|open| open.doc == doc && open.md == md)
        {
            return;
        }
        // A different tab's view goes back to reading before this one takes the surface.
        self.release_file_view(cx);
        md.update(cx, |view, cx| view.set_annotating(true, cx));
        self.open_with(doc, Presentation::Viewer, md, None, cx);
    }

    fn open_with(
        &mut self,
        doc: DocumentHandle,
        presentation: Presentation,
        md: Entity<MdView>,
        events: Option<Subscription>,
        cx: &mut Context<Self>,
    ) {
        self.bus.send(doc.load());
        // Opening, not refreshing: the host mints the document's lineage codes afresh (`D208`).
        self.bus.send(Message::ListPlanAnnotations {
            doc: doc.clone(),
            open: true,
        });
        self.workbench.plan = Some(DocumentEditor::loading(doc, presentation, md, events));
        // A different document's rail starts at its own top.
        self.plan_thread_list.reset(0);
        cx.notify();
    }

    /// Hand a tab's view back to reading — no annotation layer, no margin decor — when the open
    /// document is a tab's. Reports whether it was.
    fn release_file_view(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(md) = self
            .workbench
            .plan
            .as_ref()
            .filter(|doc| !doc.is_modal())
            .map(|doc| doc.md.clone())
        else {
            return false;
        };
        md.update(cx, |view, cx| {
            view.set_annotating(false, cx);
            view.set_decor(Vec::new(), cx);
        });
        true
    }

    /// The annotation rail's *Agent* button: the New agent form, aimed at the chat dock beside the
    /// document, opened with the collaboration servers ticked and an opening turn naming this
    /// file. **The ordinary start path** — the user picks the harness or definition and presses
    /// Start; nothing here chooses a harness. A plan or mission document has its own coordinator
    /// and is left alone.
    pub fn start_doc_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rel_path) = self
            .workbench
            .plan
            .as_ref()
            .and_then(|doc| doc.doc.rel_path().map(str::to_string))
        else {
            return;
        };
        self.aim_start(NewAgentSurface::Chat, cx);
        // After `aim_start`, which clears the aims: the start binds the new agent to this document.
        self.new_agent_for_doc = self.workbench.plan.as_ref().map(|doc| doc.doc.clone());
        self.open_new_agent(
            crate::state::new_agent::NewAgentOpen {
                initial_prompt: Some(crate::state::plan::doc_agent_prompt(&rel_path)),
                mcps: crate::state::plan::DOC_AGENT_MCPS
                    .iter()
                    .map(|it| it.to_string())
                    .collect(),
                ..Default::default()
            },
            window,
            cx,
        );
    }

    /// Bind the open document to `agent` — or unbind it with `None`. The host's answer
    /// (`PlanAnnotations`) is what the surface reads back.
    pub fn set_doc_agent(&mut self, agent_id: Option<AgentId>, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        self.bus.send(Message::SetDocAgent {
            doc: doc.doc.clone(),
            agent_id,
        });
        self.close_menu(cx);
    }

    /// The header's auto-send switch.
    pub fn toggle_doc_auto_send(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        self.bus.send(Message::SetDocAutoSend {
            doc: doc.doc.clone(),
            auto_send: !doc.binding.auto_send,
        });
        cx.notify();
    }

    /// The header's *Ask agent*: queue every open thread waiting on the agent.
    pub fn ask_doc_agent(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        self.bus.send(Message::AskDocAgent {
            doc: doc.doc.clone(),
            annotation_ids: Vec::new(),
        });
        cx.notify();
    }

    /// The ownership question answered yes: the agent that asked takes the document.
    pub fn confirm_doc_conflict(&mut self, cx: &mut Context<Self>) {
        if let Some(conflict) = self.workbench.doc_conflict.take() {
            self.bus.send(Message::SetDocAgent {
                doc: conflict.doc,
                agent_id: Some(conflict.requester),
            });
        }
        cx.notify();
    }

    pub fn dismiss_doc_conflict(&mut self, cx: &mut Context<Self>) {
        if let Some(conflict) = self.workbench.doc_conflict.take() {
            self.workbench
                .doc_conflict_refused
                .push((conflict.doc, conflict.requester));
        }
        cx.notify();
    }

    /// Put away a document a tab opened, when the tab leaves annotation mode. The dialog's own
    /// document is never touched by this: closing the plan editor is [`Self::close_plan`]'s.
    pub fn close_file_document(&mut self, cx: &mut Context<Self>) {
        if self.release_file_view(cx) {
            self.workbench.plan = None;
            cx.notify();
        }
    }

    /// Escape, and the modal's own close. **An unsaved edit is not discarded on the first ask** —
    /// the surface says what it is holding and the second Escape means it.
    pub fn close_plan(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        if doc.dirty && !doc.confirm_close {
            doc.confirm_close = true;
            cx.notify();
            return;
        }
        self.release_file_view(cx);
        self.workbench.plan = None;
        cx.notify();
    }

    /// Discard the edit and go, for the button that says so.
    pub fn discard_plan_edits(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.dirty = false;
            doc.confirm_close = true;
        }
        self.close_plan(cx);
    }

    /// Write the dialog's buffer back, whole. The host answers with the document's own body, which
    /// is what clears `saving` and re-seeds the buffer; the block re-index arrives separately.
    ///
    /// **A stale buffer does not save on the first ask.** The buffer was seeded at `revision` and
    /// the host has moved past it, so this write would replace a save the user has not read. The
    /// first ask raises the question and keeps every character typed; the second one means it.
    /// **That question is a courtesy, not the guard**: every `SavePlan` names the revision it
    /// expects to replace and the host refuses it with `PlanConflict` if the document has moved.
    pub fn save_document(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_open_document(cx);
    }

    fn save_open_document(&mut self, cx: &mut Context<Self>) {
        let body = self.plan_editor.read(cx).value().to_string();
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        // Read from the buffer rather than trusting the flag: ⌘S is the one action that must not
        // depend on a change event having been seen.
        doc.refresh_dirty(&body);
        if !doc.dirty && !doc.stale {
            return;
        }
        if doc.stale && !doc.confirm_overwrite {
            doc.confirm_overwrite = true;
            doc.confirm_close = false;
            cx.notify();
            return;
        }
        let expected = if doc.stale {
            doc.host_revision
        } else {
            doc.revision
        };
        doc.confirm_overwrite = false;
        doc.saving = true;
        doc.confirm_close = false;
        doc.notice = None;
        let handle = doc.doc.clone();
        self.bus.send(handle.save(body, expected));
        cx.notify();
    }

    /// The host refused the save: the document had moved past the revision it named, and nothing
    /// was written. The buffer is untouched, and the overwrite question is asked again.
    pub(crate) fn plan_save_refused(&mut self, revision: PlanRevision, origin: SaveOrigin) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.save_refused(revision, origin);
        }
    }

    /// The host stated a document's body. Read against what the buffer is holding — see
    /// [`DocumentEditor::set_loaded`].
    pub(crate) fn plan_body_arrived(
        &mut self,
        body: String,
        revision: PlanRevision,
        cx: &Context<Self>,
    ) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        // A markdown tab's document is over the tab's own buffer, which follows the file itself
        // and merges whatever moved it (`D208`) — so the body is taken as stated, and the dialog's
        // buffer (which is not this document's) is never read against it. Reading it was what
        // put a stale banner over every agent write in annotation mode.
        let typed = if doc.is_modal() {
            self.plan_editor.read(cx).value().to_string()
        } else {
            body.clone()
        };
        doc.set_loaded(body, &typed, revision);
    }

    /// Ask where the document has been edited, for the body that just arrived. **Nothing is asked
    /// for a document that is not tracking updates** (T-183).
    pub(crate) fn ask_for_plan_changes(&mut self) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        if !doc.track_updates {
            return;
        }
        self.bus.send(doc.doc.list_changes(None));
    }

    /// The host stated where the document was edited and by whom.
    pub(crate) fn plan_changes_arrived(
        &mut self,
        regions: Vec<PlanChangedRegion>,
        stats: PlanChangeStats,
    ) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.set_changes(regions, stats);
        }
    }

    /// Where each changed run sits in the dialog's buffer, paired with who left it.
    pub fn change_ranges(&self, cx: &App) -> Vec<(SaveOrigin, Range<usize>)> {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return Vec::new();
        };
        let body = self.plan_editor.read(cx).value().to_string();
        change_ranges(&body, &doc.changes)
    }

    /// Keep the one annotated document in step with what the editor's active tab is asking for
    /// (T-124). In `render`, because entering the layout, leaving it, switching tab and closing
    /// the tab are four routes to the same fact, and the tab's own layout is the fact.
    ///
    /// **The dialog wins while it is up.** A plan raised over a tab in annotation mode replaces
    /// the document; the tab's own is asked for again on the frame after the dialog goes. A tab
    /// whose bytes have not arrived has no view yet, and waits for it.
    pub(crate) fn settle_annotation_document(&mut self, cx: &mut Context<Self>) {
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| doc.is_modal())
        {
            return;
        }
        let wanted = self
            .editor(cx)
            .and_then(|editor| editor.active_file())
            .filter(|file| file.layout.is_annotation() && file.annotatable())
            .and_then(|file| Some((file.path.clone(), file.md.clone()?)));
        match wanted {
            Some((path, md)) => self.open_file_document(&path, md, cx),
            None => self.close_file_document(cx),
        }
    }

    /// In `render`, because seeding a buffer and focusing a field both need a `Window`, and the
    /// host's answer (or the view's event) arrives without one.
    pub fn settle_plan_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Dirtiness is the buffer against the host's last word, refreshed here as well as on the
        // buffer's own change event: a value written by anything but a keystroke raises no event.
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| doc.is_modal())
        {
            let typed = self.plan_editor.read(cx).value().to_string();
            if let Some(doc) = self.workbench.plan.as_mut()
                && !doc.needs_seed
            {
                doc.refresh_dirty(&typed);
            }
        }
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        let seed = doc.needs_seed.then(|| doc.saved.clone());
        doc.needs_seed = false;
        let focus = std::mem::take(&mut doc.composer_needs_focus);
        let modal = doc.is_modal();
        let md = doc.md.clone();
        // A tab's view is over the tab's own buffer; only the dialog's buffer is seeded from the
        // host's body.
        if let Some(text) = seed
            && modal
        {
            let editor = self.plan_editor.clone();
            editor.update(cx, |state, cx| state.set_value(&text, window, cx));
            if let Some(doc) = self.workbench.plan.as_mut() {
                doc.dirty = false;
            }
            // `set_value` raises no change event, so the view is told to follow at once.
            md.update(cx, |view, cx| view.resync(cx));
        }
        if focus {
            let input = self.annotation_composer_input.clone();
            input.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
        }
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| doc.decor_stale)
        {
            self.refresh_document_decor(cx);
        }
    }

    /// Re-place the host's blocks in the view's rows and push the margin decor that follows — the
    /// answer to a reparse, a fresh block index, and a focus change alike.
    pub(crate) fn refresh_document_decor(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        let md = doc.md.clone();
        doc.remap(md.read(cx).doc());
        let decor = doc.decor();
        md.update(cx, |view, cx| view.set_decor(decor, cx));
    }

    /// The dialog's buffer changed: dirty, or clean again.
    pub(crate) fn plan_editor_changed(&mut self, typed: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut().filter(|doc| doc.is_modal()) else {
            return;
        };
        doc.refresh_dirty(typed);
        doc.confirm_close = false;
        cx.notify();
    }

    /// What the open document's view said. A view that is not the open document's — a tab whose
    /// document was put away, a dialog's view outliving its document by a frame — is ignored.
    pub(crate) fn document_md_event(
        &mut self,
        md: &Entity<MdView>,
        event: &MdViewEvent,
        cx: &mut Context<Self>,
    ) {
        /// What the event comes to, worked out while the document is borrowed.
        enum Act {
            Compose(Option<BlockId>, Vec<AnnotationMark>),
            Focus(AnnotationId),
            Follow(AnnotationId),
            Mark(AnnotationId, AnnotationMark),
            Resolve(AnnotationId, bool),
            Highlight(Vec<BlockId>, Option<HighlightColour>),
            Save,
            Remap,
            Nothing,
        }

        let Some(doc) = self.workbench.plan.as_ref().filter(|doc| doc.md == *md) else {
            return;
        };
        let (rows, focused) = (&doc.rows, doc.focused);
        let annotations = doc.annotations.annotations();
        let act = match event {
            MdViewEvent::AddThread { row } => {
                Act::Compose(target_block(rows, *row, None), Vec::new())
            }
            MdViewEvent::ThreadsClicked { row } => {
                mark_target(rows, annotations, *row, focused).map_or(Act::Nothing, Act::Focus)
            }
            MdViewEvent::MarkRequested { row, mark } => {
                match mark_target(rows, annotations, *row, focused) {
                    Some(id) => Act::Mark(id, *mark),
                    // No thread to flag: marking makes one, with the mark preset — never an empty
                    // thread on the wire.
                    None => Act::Compose(target_block(rows, *row, None), vec![*mark]),
                }
            }
            MdViewEvent::ResolveRequested { row, .. } => {
                resolve_target(rows, annotations, *row, focused)
                    .map_or(Act::Nothing, |(id, resolved)| Act::Resolve(id, resolved))
            }
            MdViewEvent::HighlightRequested { row, colour } => {
                let blocks = rows.blocks_in(*row).to_vec();
                if blocks.is_empty() {
                    Act::Nothing
                } else {
                    Act::Highlight(blocks, *colour)
                }
            }
            // The dialog's commit is a save; a tab's commit is an edit to the tab, saved the way
            // every tab is.
            MdViewEvent::BlockEdited { .. } if doc.is_modal() => Act::Save,
            MdViewEvent::DocumentChanged { .. } => Act::Remap,
            MdViewEvent::Scrolled { rows: visible } => {
                match follow_target(rows, annotations, visible.clone(), focused) {
                    Some(next) if Some(next) != focused => Act::Follow(next),
                    _ => Act::Nothing,
                }
            }
            _ => Act::Nothing,
        };

        match act {
            Act::Compose(block, marks) => self.compose_on_row(block, marks, cx),
            Act::Focus(id) => self.focus_thread(id, cx),
            Act::Follow(id) => {
                if let Some(doc) = self.workbench.plan.as_mut() {
                    doc.focused = Some(id);
                }
                self.reveal_focused_in_rail();
                self.refresh_document_decor(cx);
                cx.notify();
            }
            Act::Mark(id, mark) => self.toggle_annotation_mark(id, mark, cx),
            Act::Resolve(id, resolved) => self.set_annotation_resolved(id, resolved, cx),
            Act::Highlight(blocks, colour) => {
                if let Some(doc) = self.workbench.plan.as_ref() {
                    self.bus.send(doc.doc.highlight(blocks, colour));
                }
            }
            Act::Save => self.save_open_document(cx),
            Act::Remap => self.refresh_document_decor(cx),
            Act::Nothing => {}
        }
    }

    /// Open the composer on a row's target block, or say why there is none: a block typed since
    /// the last save has no host id yet.
    fn compose_on_row(
        &mut self,
        block: Option<BlockId>,
        marks: Vec<AnnotationMark>,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        let Some(block_id) = block else {
            doc.notice = Some(Notice::Err(
                "This block is not in the saved document yet \u{2014} save before annotating it."
                    .to_string(),
            ));
            cx.notify();
            return;
        };
        doc.composer = Some(ComposerTarget::Block(block_id));
        doc.composer_marks = marks;
        doc.composer_text.clear();
        doc.composer_needs_focus = true;
        doc.notice = None;
        cx.notify();
    }

    /// Expand a thread in the rail and bring its block into view — a click on a collapsed thread,
    /// or on a row's count marker.
    pub fn focus_thread(&mut self, annotation_id: AnnotationId, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        doc.focused = Some(annotation_id);
        let row = doc.row_of_annotation(annotation_id);
        let md = doc.md.clone();
        if let Some(row) = row {
            md.update(cx, |view, cx| view.reveal_block(row, cx));
        }
        self.reveal_focused_in_rail();
        self.refresh_document_decor(cx);
        cx.notify();
    }

    /// Collapse the expanded thread.
    pub fn collapse_thread(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.focused = None;
        }
        self.refresh_document_decor(cx);
        cx.notify();
    }

    /// Scroll the rail so the focused thread is on screen.
    fn reveal_focused_in_rail(&self) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let Some(focused) = doc.focused else {
            return;
        };
        if let Some(index) = doc.rail_threads().iter().position(|id| *id == focused)
            && index < self.plan_thread_list.item_count()
        {
            self.plan_thread_list.scroll_to_reveal_item(index);
        }
    }

    pub fn toggle_resolved_annotations(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.show_resolved = !doc.show_resolved;
        }
        cx.notify();
    }

    /// Flip whether this document tracks who has edited it (T-183). Turning it on asks the host at
    /// once; turning it off drops what was already shown.
    pub fn toggle_track_updates(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        doc.track_updates = !doc.track_updates;
        let tracking = doc.track_updates;
        if tracking {
            self.ask_for_plan_changes();
        } else if let Some(doc) = self.workbench.plan.as_mut() {
            doc.set_changes(Vec::new(), PlanChangeStats::default());
        }
        cx.notify();
    }

    /// Claim a whole block for a fresh thread.
    pub fn compose_annotation(
        &mut self,
        block_id: BlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.composer_marks.clear();
        }
        self.open_composer(ComposerTarget::Block(block_id), window, cx);
    }

    /// Open the reply field under an existing thread.
    pub fn compose_reply(
        &mut self,
        annotation_id: AnnotationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_composer(ComposerTarget::Reply(annotation_id), window, cx);
    }

    /// Edit one of the user's own comments in place: the composer opens on it, seeded with its
    /// text, and sending answers `EditAnnotationComment`.
    pub fn compose_edit(
        &mut self,
        annotation_id: AnnotationId,
        comment_id: CommentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self
            .workbench
            .plan
            .as_ref()
            .and_then(|doc| doc.annotation(annotation_id))
            .and_then(|annotation| annotation.thread.iter().find(|c| c.id == comment_id))
            .filter(|comment| comment.author == CommentAuthor::User)
            .map(|comment| comment.text.clone())
        else {
            return;
        };
        self.open_composer(ComposerTarget::Edit(annotation_id, comment_id), window, cx);
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.composer_text = text.clone();
        }
        let input = self.annotation_composer_input.clone();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn open_composer(
        &mut self,
        target: ComposerTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        doc.composer = Some(target);
        doc.composer_text.clear();
        doc.notice = None;
        let input = self.annotation_composer_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// The composer's "@agent" chip.
    pub fn toggle_composer_agent(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.composer_to_agent = !doc.composer_to_agent;
        }
        cx.notify();
    }

    /// One of a fresh thread's mark chips.
    pub fn toggle_composer_mark(&mut self, mark: AnnotationMark, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            match doc.composer_marks.iter().position(|m| *m == mark) {
                Some(at) => {
                    doc.composer_marks.remove(at);
                }
                None => doc.composer_marks.push(mark),
            }
        }
        cx.notify();
    }

    /// Close the composer without sending anything.
    pub fn cancel_annotation_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        doc.composer = None;
        doc.composer_marks.clear();
        doc.composer_text.clear();
        let input = self.annotation_composer_input.clone();
        input.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// Send whatever the composer is holding — a fresh thread on a block, or a reply — and close
    /// it. A leading `@agent` or the "@agent" chip addresses it to the agent ([`addressed`]).
    /// Empty text sends nothing.
    pub fn submit_annotation_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let Some(target) = doc.composer else {
            return;
        };
        // An edit keeps what was typed verbatim — no addressing, never queued for an agent — and
        // sends nothing when it is empty or says what the comment already said.
        if let ComposerTarget::Edit(annotation_id, comment_id) = target {
            let typed = doc.composer_text.trim().to_string();
            let unchanged = doc
                .annotation(annotation_id)
                .and_then(|a| a.thread.iter().find(|c| c.id == comment_id))
                .is_none_or(|c| c.text.trim() == typed);
            if typed.is_empty() {
                return;
            }
            if !unchanged {
                self.bus.send(Message::EditAnnotationComment {
                    doc: doc.doc.clone(),
                    annotation_id,
                    comment_id,
                    text: typed,
                });
            }
            self.cancel_annotation_composer(window, cx);
            return;
        }
        let (to, text) = addressed(&doc.composer_text, doc.composer_to_agent);
        if text.is_empty() {
            return;
        }
        let handle = doc.doc.clone();
        let message = match target {
            ComposerTarget::Edit(..) => return,
            ComposerTarget::Block(block_id) => {
                handle.annotate(block_id, text, doc.composer_marks.clone(), to)
            }
            ComposerTarget::Reply(annotation_id) => handle.reply(annotation_id, text, to),
        };
        self.bus.send(message);
        self.cancel_annotation_composer(window, cx);
    }

    /// Resolve or reopen a thread. **Anyone may — a user ruling, no author check on this side
    /// either.**
    pub fn set_annotation_resolved(
        &mut self,
        annotation_id: AnnotationId,
        resolved: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        self.bus.send(doc.doc.resolve(annotation_id, resolved));
        cx.notify();
    }

    /// The review's Reopen (`D208`): the agent's proposal is turned down — the thread is reopened
    /// (which clears `review` on the host) and a reply addressed to the agent says why, which
    /// queues it back through the doc queue (`D207`).
    pub fn reopen_for_agent(&mut self, annotation_id: AnnotationId, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let handle = doc.doc.clone();
        self.bus.send(handle.resolve(annotation_id, false));
        // Said in the thread and addressed to the agent, so the doc queue delivers it with the
        // user's latest comment before it — the reason travels with the thread, not beside it.
        self.bus.send(handle.reply(
            annotation_id,
            "Reopened: this needs more work.".to_string(),
            Some(Addressee::Agent),
        ));
        cx.notify();
    }

    /// Flip one flag on a thread: set it when the thread lacks it, clear it when it has it.
    pub fn toggle_annotation_mark(
        &mut self,
        annotation_id: AnnotationId,
        mark: AnnotationMark,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let on = !doc
            .annotation(annotation_id)
            .is_some_and(|annotation| annotation.marks.contains(&mark));
        self.bus.send(doc.doc.mark(annotation_id, mark, on));
        cx.notify();
    }

    /// Re-ask for a document's annotations, for a surface that is open and cares —
    /// `Message::PlanAnnotationsChanged`'s handler, on `Message::PlanChanged`'s own economy.
    pub(crate) fn reload_plan_annotations(&mut self, changed: &DocumentHandle) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        if doc.doc != *changed {
            return;
        }
        // A stale `Loaded` index must not sit under the decor while the fresh one is in flight.
        doc.annotations = AnnotationsBody::Loading;
        let handle = doc.doc.clone();
        self.bus.send(handle.list_annotations());
    }

    /// Raise the Export question over the document — an explicit, one-shot write into the
    /// project's working tree, never a continuous mirror.
    pub fn open_export_plan_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = self.workbench.plan.as_ref().and_then(|doc| doc.task_id()) else {
            return;
        };
        self.open_file_dialog(FileDialog::ExportPlan { task_id }, "", window, cx);
    }
}
