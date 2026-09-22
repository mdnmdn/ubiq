//! The annotated-document surface's actions: open a document, edit it, save it, and drive the
//! threads anchored to its passages.
//!
//! **This is `Message::SavePlan`'s first caller.** Slice 3 put the write half on the wire and left
//! it to slice 6; [`AppState::save_document`] is it. The body is replaced whole, the host
//! re-indexes the blocks, and the annotations that survive the re-index come back through
//! `PlanAnnotationsChanged` — which is why a save never touches a thread here.
//!
//! **Nothing in this file names a task except the plan's own entry points.** Everything between
//! opening and saving reads a [`DocumentHandle`], and the handle is what turns into a message.
//! That is the whole of the seam a second kind of annotated document would arrive through: a new
//! variant, the five arms below, and a host-side store for the body and the sidecar.

use super::*;

use std::ops::Range;

use gpui_component::input::{TextDecoration, TextDecorationCollection};

use crate::state::document::{
    AnnotationsBody, ComposerTarget, DocumentEditor, DocumentHandle, Notice, annotation_range,
    block_ranges, change_ranges,
};
use crate::state::plan::plan_document;
use crate::state::workbench::FileDialog;
use ubiq_proto::ids::{AnnotationId, BlockId, ProjectId, TaskId};
use ubiq_proto::plan::{PlanChangeStats, PlanChangedRegion, PlanRevision, SaveOrigin};

/// The wire, per document kind. The interface never invents a route: every arm here is a message
/// that already exists in `crates/ubiq-proto/src/messages.rs`.
impl DocumentHandle {
    pub(crate) fn load(&self) -> Message {
        match *self {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Message::LoadPlan {
                project_id,
                task_id,
            },
        }
    }

    pub(crate) fn list_annotations(&self) -> Message {
        match *self {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Message::ListPlanAnnotations {
                project_id,
                task_id,
            },
        }
    }

    pub(crate) fn save(&self, body: String) -> Message {
        match *self {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Message::SavePlan {
                project_id,
                task_id,
                body,
            },
        }
    }

    pub(crate) fn annotate(
        &self,
        block_id: BlockId,
        quote: Option<String>,
        text: String,
    ) -> Message {
        match *self {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Message::AnnotatePlan {
                project_id,
                task_id,
                block_id,
                quote,
                text,
            },
        }
    }

    pub(crate) fn reply(&self, annotation_id: AnnotationId, text: String) -> Message {
        match *self {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Message::ReplyToAnnotation {
                project_id,
                task_id,
                annotation_id,
                text,
            },
        }
    }

    /// Where the document has been edited, and by whom. `since` absent asks for the whole history,
    /// which is what an editor opening a document wants: a plan an agent wrote from nothing reads
    /// as agent lines throughout, and the human lines stand out against them.
    pub(crate) fn list_changes(&self, since: Option<PlanRevision>) -> Message {
        match *self {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Message::ListPlanChanges {
                project_id,
                task_id,
                since_revision: since,
            },
        }
    }

    pub(crate) fn resolve(&self, annotation_id: AnnotationId, resolved: bool) -> Message {
        match *self {
            DocumentHandle::Plan {
                project_id,
                task_id,
            } => Message::ResolveAnnotation {
                project_id,
                task_id,
                annotation_id,
                resolved,
            },
        }
    }
}

impl AppState {
    /// Open a task's plan as an editable document. The affordance that calls this is only ever
    /// drawn for a task carrying a `level` — the host refuses the whole family for one with none
    /// — so this never has to check that itself.
    pub fn open_plan(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.open_document(plan_document(project_id, task_id), cx);
    }

    /// Open any annotated document: ask for its body and its threads in the same breath.
    pub fn open_document(&mut self, doc: DocumentHandle, cx: &mut Context<Self>) {
        self.workbench.plan = Some(DocumentEditor::loading(doc));
        self.bus.send(doc.load());
        self.bus.send(doc.list_annotations());
        cx.notify();
    }

    /// Escape, and the modal's own close. **An unsaved edit is not discarded on the first ask** —
    /// the surface says what it is holding and the second Escape means it. `⌘S` clears the
    /// question by making it moot.
    pub fn close_plan(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        if doc.dirty && !doc.confirm_close {
            doc.confirm_close = true;
            cx.notify();
            return;
        }
        self.workbench.plan = None;
        self.plan_marks = None;
        self.plan_change_marks = None;
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

    /// Which of source, preview and both the document is drawn in — the file editor's own toggle,
    /// on the same enum.
    pub fn set_document_layout(&mut self, layout: ViewLayout, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.layout = layout;
        }
        cx.notify();
    }

    /// Write the buffer back, whole. The host answers with the document's own body, which is what
    /// clears `saving` and re-seeds the buffer; the block re-index arrives separately.
    ///
    /// **A stale buffer does not save on the first ask.** The buffer was seeded at `revision` and
    /// the host has moved past it, so this write would replace a save the user has not read —
    /// `SavePlan` is a whole-body replacement and carries no expected revision, so nothing
    /// downstream can refuse it. The first ask raises the question and keeps every character
    /// typed; the second one means it. Whoever moved the copy is named in the banner, because
    /// overwriting an agent and overwriting a colleague are not the same decision.
    pub fn save_document(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
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
        doc.confirm_overwrite = false;
        doc.saving = true;
        doc.confirm_close = false;
        doc.notice = None;
        let handle = doc.doc;
        self.bus.send(handle.save(body));
        cx.notify();
    }

    /// The host stated a document's body. Handled here rather than assigned in `wire` because the
    /// answer has to be read against what the buffer is holding — see
    /// [`DocumentEditor::set_loaded`].
    pub(crate) fn plan_body_arrived(
        &mut self,
        body: String,
        revision: PlanRevision,
        cx: &Context<Self>,
    ) {
        let typed = self.plan_editor.read(cx).value().to_string();
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        doc.set_loaded(body, &typed, revision);
    }

    /// Ask where the document has been edited, for the body that just arrived. Asked with every
    /// body and nowhere else: the regions are in the *current* body's line numbers, so an answer
    /// is only usable against the body it was asked alongside.
    pub(crate) fn ask_for_plan_changes(&mut self) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
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

    /// In `render`, for the reason `attach_arrived_files` is: seeding a buffer and measuring one
    /// both need a `Window`, and the host's answer arrives without one.
    pub fn settle_plan_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Dirtiness is the buffer against the host's last word, refreshed here as well as on the
        // buffer's own change event: a value written into the buffer by anything other than a
        // keystroke raises no event, and a surface that thinks it is clean would lose that edit.
        if self.workbench.plan.is_some() {
            let typed = self.plan_editor.read(cx).value().to_string();
            if let Some(doc) = self.workbench.plan.as_mut()
                && !doc.needs_seed
            {
                doc.refresh_dirty(&typed);
            }
        }
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        if doc.needs_seed {
            let text = doc.saved.clone();
            let editor = self.plan_editor.clone();
            editor.update(cx, |state, cx| state.set_value(&text, window, cx));
            if let Some(doc) = self.workbench.plan.as_mut() {
                doc.needs_seed = false;
                doc.dirty = false;
                doc.decor_stale = true;
            }
        }
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| doc.decor_stale)
        {
            self.paint_annotation_marks(cx);
            if let Some(doc) = self.workbench.plan.as_mut() {
                doc.decor_stale = false;
            }
        }
        // The change marks are painted against the body the host counted them over, so an answer
        // that lands while the buffer holds an unsaved edit waits: the library moves the marks it
        // already has with every keystroke, which is the better of the two wrong answers.
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| doc.change_decor_stale && !doc.dirty)
        {
            self.paint_change_marks(cx);
            if let Some(doc) = self.workbench.plan.as_mut() {
                doc.change_decor_stale = false;
            }
        }
    }

    /// Where each thread sits in the text on screen — the join between the host's block ids and
    /// the buffer's offsets, and the one place that join is made.
    pub fn annotation_ranges(&self, cx: &App) -> Vec<(AnnotationId, Range<usize>)> {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return Vec::new();
        };
        let body = self.plan_editor.read(cx).value().to_string();
        let blocks = block_ranges(&body, doc.annotations.blocks());
        doc.annotations
            .annotations()
            .iter()
            .filter(|annotation| !annotation.orphaned)
            .filter_map(|annotation| {
                annotation_range(&body, &blocks, annotation).map(|range| (annotation.id, range))
            })
            .collect()
    }

    /// The block a byte offset falls in, for a selection that wants to become a thread.
    fn block_at(&self, offset: usize, cx: &App) -> Option<BlockId> {
        let doc = self.workbench.plan.as_ref()?;
        let body = self.plan_editor.read(cx).value().to_string();
        block_ranges(&body, doc.annotations.blocks())
            .into_iter()
            .find(|(_, range)| range.contains(&offset) || range.end == offset)
            .map(|(id, _)| id)
    }

    /// Where each changed run sits in the text on screen, paired with who left it — the join
    /// between the host's line numbers and the buffer's offsets.
    pub fn change_ranges(&self, cx: &App) -> Vec<(SaveOrigin, Range<usize>)> {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return Vec::new();
        };
        let body = self.plan_editor.read(cx).value().to_string();
        change_ranges(&body, &doc.changes)
    }

    /// Edit provenance, drawn over the live text as the second layer.
    ///
    /// An underline rather than a fill: the first layer is already painting backgrounds, the
    /// library lets the first collection win a property they share, and a changed line inside an
    /// annotated passage has to read as both. The two origins differ by hue *and* by whether the
    /// underline waves, so the distinction survives a reader who cannot tell the hues apart.
    fn paint_change_marks(&mut self, cx: &mut Context<Self>) {
        let marks: Vec<TextDecoration> = self
            .change_ranges(cx)
            .into_iter()
            .map(|(origin, range)| {
                TextDecoration::new(
                    range,
                    gpui::HighlightStyle {
                        underline: Some(gpui::UnderlineStyle {
                            thickness: gpui::px(1.),
                            color: Some(crate::theme::edit_origin(origin.is_human()).into()),
                            wavy: !origin.is_human(),
                        }),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let collection = match self.plan_change_marks.clone() {
            Some(collection) => collection,
            None => {
                let collection: TextDecorationCollection =
                    self.plan_editor.update(cx, |state, cx| {
                        state.create_decorations_collection(Vec::new(), cx)
                    });
                self.plan_change_marks = Some(collection.clone());
                collection
            }
        };
        collection.set(marks, cx);
    }

    /// The annotated passages, drawn over the live text.
    ///
    /// One collection for the buffer, kept: the library hands a collection out once and it lives
    /// as long as the buffer it was made from, and this buffer outlives every document opened in
    /// it. [`AppState::paint_change_marks`] is the second layer, over the same text.
    fn paint_annotation_marks(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let open: std::collections::HashSet<AnnotationId> = doc
            .annotations
            .annotations()
            .iter()
            .filter(|annotation| annotation.is_open())
            .map(|annotation| annotation.id)
            .collect();
        let marks: Vec<TextDecoration> = self
            .annotation_ranges(cx)
            .into_iter()
            .map(|(id, range)| {
                let colour = if open.contains(&id) {
                    crate::theme::info_soft()
                } else {
                    crate::theme::success_soft()
                };
                TextDecoration::new(
                    range,
                    gpui::HighlightStyle {
                        background_color: Some(colour.into()),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let collection = match self.plan_marks.clone() {
            Some(collection) => collection,
            None => {
                let collection: TextDecorationCollection =
                    self.plan_editor.update(cx, |state, cx| {
                        state.create_decorations_collection(Vec::new(), cx)
                    });
                self.plan_marks = Some(collection.clone());
                collection
            }
        };
        collection.set(marks, cx);
    }

    /// The buffer changed: dirty, and the decorations have moved with the text.
    pub(crate) fn plan_editor_changed(&mut self, typed: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        doc.refresh_dirty(typed);
        doc.confirm_close = false;
        doc.decor_stale = true;
        cx.notify();
    }

    /// A click landed in the document: if it landed inside an annotated passage, that thread is
    /// what the reader means, and the popover opens on it.
    pub fn document_clicked(&mut self, cx: &mut Context<Self>) {
        if self.workbench.plan.is_none() {
            return;
        }
        let at = self.plan_editor.read(cx).cursor();
        let hit = self
            .annotation_ranges(cx)
            .into_iter()
            .find(|(_, range)| range.contains(&at))
            .map(|(id, _)| id);
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.thread = hit;
        }
        cx.notify();
    }

    /// Show a thread's popover over its own passage, and put the selection there so the passage
    /// is what the eye lands on.
    pub fn open_annotation_thread(&mut self, annotation_id: AnnotationId, cx: &mut Context<Self>) {
        if self.workbench.plan.is_none() {
            return;
        }
        let range = self
            .annotation_ranges(cx)
            .into_iter()
            .find(|(id, _)| *id == annotation_id)
            .map(|(_, range)| range);
        if let Some(range) = range {
            self.plan_editor
                .update(cx, |state, cx| state.set_selected_range(range, cx));
        }
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.thread = Some(annotation_id);
        }
        cx.notify();
    }

    pub fn close_annotation_thread(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.thread = None;
        }
        cx.notify();
    }

    pub fn toggle_resolved_annotations(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.show_resolved = !doc.show_resolved;
        }
        cx.notify();
    }

    /// Claim whatever is selected in the document for a fresh thread. The block is the anchor —
    /// block ids are the host's, and only a block has one — and the selected passage rides along
    /// as the quote, which is what survives the block being edited around it.
    ///
    /// A selection the host has no block for (a passage typed since the last save) cannot be
    /// annotated, and the surface says so rather than posting a thread that would be orphaned on
    /// arrival.
    pub fn annotate_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.plan.is_none() {
            return;
        }
        let (selection, quote) = {
            let state = self.plan_editor.read(cx);
            let range = state.selected_range();
            let quote = state.selected_text().to_string();
            (range, quote)
        };
        let Some(block_id) = self.block_at(selection.start, cx) else {
            if let Some(doc) = self.workbench.plan.as_mut() {
                doc.notice = Some(Notice::Err(
                    "Save the plan before annotating this passage \u{2014} it has no block yet."
                        .to_string(),
                ));
            }
            cx.notify();
            return;
        };
        let quote = quote.trim().to_string();
        let quote = (!quote.is_empty()).then_some(quote);
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.composer_quote = quote;
        }
        self.open_composer(ComposerTarget::Block(block_id), window, cx);
    }

    /// Claim a whole block, for a caller that has one already — a click on the preview's own
    /// block, where there are no document offsets to select with.
    pub fn compose_annotation(
        &mut self,
        block_id: BlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.composer_quote = None;
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

    /// Close the composer without sending anything.
    pub fn cancel_annotation_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        doc.composer = None;
        doc.composer_quote = None;
        doc.composer_text.clear();
        let input = self.annotation_composer_input.clone();
        input.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// Send whatever the composer is holding — a fresh thread on a block, or a reply — and close
    /// it. Empty text sends nothing, `add_task_comment`'s own guard.
    pub fn submit_annotation_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let Some(target) = doc.composer else {
            return;
        };
        let text = doc.composer_text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let handle = doc.doc;
        let quote = doc.composer_quote.clone();
        let message = match target {
            ComposerTarget::Block(block_id) => handle.annotate(block_id, quote, text),
            ComposerTarget::Reply(annotation_id) => handle.reply(annotation_id, text),
        };
        self.bus.send(message);
        self.cancel_annotation_composer(window, cx);
    }

    /// Resolve or reopen a thread. **Anyone may — a user ruling, no author check on this side
    /// either** (`_docs/wip/planning-system.md`, decision — "who may resolve").
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

    /// Re-ask for a document's annotations, for a surface that is open and cares —
    /// `Message::PlanAnnotationsChanged`'s handler, on `Message::PlanChanged`'s own economy.
    pub(crate) fn reload_plan_annotations(&mut self, project_id: ProjectId, task_id: TaskId) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        if doc.project_id() != project_id || doc.task_id() != Some(task_id) {
            return;
        }
        // A stale `Loaded` index must not sit under the decorations while the fresh one is in
        // flight.
        doc.annotations = AnnotationsBody::Loading;
        let handle = doc.doc;
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
