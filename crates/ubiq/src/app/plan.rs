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

use gpui_component::input::{TextDecoration, TextDecorationCollection, TextareaState};

use crate::state::document::{
    AnnotationsBody, ComposerTarget, DocumentEditor, DocumentHandle, Notice, Presentation,
    SectionEdit, annotation_range, block_ranges, change_ranges, heading_sections,
    parse_section_blocks, splice_section, thread_marks,
};
use crate::state::plan::{file_document, mission_doc_document, plan_document};
use crate::state::workbench::FileDialog;
use ubiq_proto::ids::{AnnotationId, BlockId, TaskId};
use ubiq_proto::plan::{PlanChangeStats, PlanChangedRegion, PlanRevision, SaveOrigin};

/// The wire, for any annotated document. The interface never invents a route: every method here
/// is a message that already exists in `crates/ubiq-proto/src/messages.rs`, and the handle is the
/// whole of what it names — there is no per-kind arm left to write, because the *host* is what
/// answers a handle with a place on disk.
///
/// A trait rather than an inherent `impl` only because [`DocumentHandle`] is the contract's type
/// and this crate does not own it.
pub(crate) trait DocumentWire {
    fn load(&self) -> Message;
    fn list_annotations(&self) -> Message;
    /// `expected` is the revision this body is meant to replace, and the host refuses the write if
    /// the document no longer stands there. There is no force flag to leave out: an overwrite the
    /// user has confirmed names the newest revision the host has stated instead of the one the
    /// buffer was seeded from, so it is still an honest expectation and still refused if a third
    /// save landed while the question was on screen.
    fn save(&self, body: String, expected: PlanRevision) -> Message;
    fn annotate(&self, block_id: BlockId, quote: Option<String>, text: String) -> Message;
    fn reply(&self, annotation_id: AnnotationId, text: String) -> Message;
    /// Where the document has been edited, and by whom. `since` absent asks for the whole history,
    /// which is what an editor opening a document wants: a plan an agent wrote from nothing reads
    /// as agent lines throughout, and the human lines stand out against them.
    fn list_changes(&self, since: Option<PlanRevision>) -> Message;
    fn resolve(&self, annotation_id: AnnotationId, resolved: bool) -> Message;
}

impl DocumentWire for DocumentHandle {
    fn load(&self) -> Message {
        Message::LoadPlan { doc: self.clone() }
    }

    fn list_annotations(&self) -> Message {
        Message::ListPlanAnnotations { doc: self.clone() }
    }

    fn save(&self, body: String, expected: PlanRevision) -> Message {
        Message::SavePlan {
            doc: self.clone(),
            body,
            expected,
        }
    }

    fn annotate(&self, block_id: BlockId, quote: Option<String>, text: String) -> Message {
        Message::AnnotatePlan {
            doc: self.clone(),
            block_id,
            quote,
            text,
        }
    }

    fn reply(&self, annotation_id: AnnotationId, text: String) -> Message {
        Message::ReplyToAnnotation {
            doc: self.clone(),
            annotation_id,
            text,
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
    /// drawn for a task carrying a `level` — the host refuses the whole family for one with none
    /// — so this never has to check that itself.
    pub fn open_plan(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.open_document(plan_document(project_id, task_id), Presentation::Modal, cx);
    }

    /// Open one of a mission's own documents (M8), on the plan dialog's own footing — the *Plan &
    /// docs* tab's "Open" beside a document row is this function's one caller, the way the plan's
    /// own "Open" is [`Self::open_plan`]'s.
    pub fn open_mission_doc(&mut self, task_id: TaskId, name: &str, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.open_document(
            mission_doc_document(project_id, task_id, name),
            Presentation::Modal,
            cx,
        );
    }

    /// Open a project markdown file as an annotated document, for a tab put into
    /// `ViewLayout::Annotation` — [`crate::state::plan::file_document`]'s one caller.
    ///
    /// Idempotent: a tab redrawn, or the layout pressed twice, must not re-ask for a document that
    /// is already open, because the second `LoadPlan` would arrive while a section edit was in the
    /// buffer.
    pub fn open_file_document(&mut self, rel_path: &str, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let doc = file_document(project_id, rel_path);
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|open| open.doc == doc)
        {
            return;
        }
        self.open_document(doc, Presentation::Viewer, cx);
    }

    /// Put away a document a tab opened, when the tab leaves annotation mode. The dialog's own
    /// document is never touched by this: closing the plan editor is [`Self::close_plan`]'s.
    pub fn close_file_document(&mut self, cx: &mut Context<Self>) {
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| !doc.is_modal())
        {
            self.workbench.plan = None;
            self.plan_marks = None;
            self.plan_change_marks = None;
            cx.notify();
        }
    }

    /// Open any annotated document: ask for its body and its threads in the same breath.
    pub fn open_document(
        &mut self,
        doc: DocumentHandle,
        presentation: Presentation,
        cx: &mut Context<Self>,
    ) {
        self.bus.send(doc.load());
        self.bus.send(doc.list_annotations());
        self.workbench.plan = Some(DocumentEditor::loading(doc, presentation));
        // A different document starts at its own top. The surface's section list is one
        // `ListState` for the window (`plan_preview_list`), and `ui::document::preview` only ever
        // *re-syncs* it to a new block count — which it does while keeping the reader's place, so
        // a section edit that splits a block does not throw them back to the first line. Opening
        // a document is the case where keeping the place would be wrong, so it is said here.
        self.plan_preview_list.reset(0);
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

    /// Open one section as raw markdown — what a double-click on a rendered section does, and the
    /// edit affordance beside it.
    ///
    /// The field is seeded from the **document**, not from the block index: the slice of the body
    /// the block occupies is what the reader is looking at, and the index is only how it is found.
    /// A section the host has not indexed cannot be located in the body and is refused where the
    /// user is looking, the way an unanchorable annotation is.
    pub fn begin_section_edit(
        &mut self,
        block_id: BlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let body = self.plan_editor.read(cx).value().to_string();
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let found = block_ranges(&body, doc.annotations.blocks())
            .into_iter()
            .find(|(id, _)| *id == block_id)
            .map(|(_, range)| range);
        let Some(range) = found else {
            if let Some(doc) = self.workbench.plan.as_mut() {
                doc.notice = Some(Notice::Err(
                    "This section is not in the saved document yet \u{2014} save the plan before \
                     editing it."
                        .to_string(),
                ));
            }
            cx.notify();
            return;
        };
        let text = body[range].to_string();
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 24));
        input.update(cx, |state, cx| {
            state.set_value(&text, window, cx);
            state.focus(window, cx);
        });
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.section_edit = Some(SectionEdit { block_id, input });
            doc.notice = None;
            // The first press of a double-click claimed the section for a thread. Editing it is
            // the answer the second press gave, so the composer that opened on the way here goes.
            doc.composer = None;
            doc.composer_quote = None;
        }
        cx.notify();
    }

    /// Leave the section as it was. The field is dropped with the edit: nothing survives a cancel.
    pub fn cancel_section_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.section_edit = None;
        }
        cx.notify();
    }

    /// Put the typed markdown back into the document, and save.
    ///
    /// **A section edit is an ordinary whole-document save**, and that is what keeps every piece
    /// of tracking working. The buffer's slice for this block is replaced by what was typed, the
    /// body goes out as `SavePlan`, and the host does the rest:
    ///
    /// - it re-indexes the blocks and **matches them against the previous save** — an unchanged
    ///   block keeps its id, an edited one keeps its id while it still resembles itself, and text
    ///   that has become several blocks leaves the anchor on the one that still matches while the
    ///   rest are minted fresh. So a split keeps the annotations on the part they were about
    ///   rather than orphaning them;
    /// - it records the save's provenance as a human edit, which is what an agent reads back
    ///   through `ListPlanChanges` and what the change underlines draw.
    ///
    /// The window invents none of that: it never mints a block id and never writes a section on
    /// its own. **An all-whitespace section is removed rather than saved as an empty one** —
    /// [`splice_section`] closes the gap it leaves — and either way the block cache is patched
    /// immediately: see [`DocumentEditor::replace_cached_block`] for why the preview cannot simply
    /// wait for the host's answer the way everything else here does, and for how a section that
    /// parsed into several blocks is cached as several rather than as one holding embedded
    /// newlines.
    pub fn confirm_section_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.plan_editor.read(cx).value().to_string();
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let Some(edit) = doc.section_edit.as_ref() else {
            return;
        };
        let block_id = edit.block_id;
        let typed = edit.input.read(cx).value().to_string();
        let found = block_ranges(&body, doc.annotations.blocks())
            .into_iter()
            .find(|(id, _)| *id == block_id)
            .map(|(_, range)| range);
        let Some(range) = found else {
            if let Some(doc) = self.workbench.plan.as_mut() {
                doc.section_edit = None;
                doc.notice = Some(Notice::Err(
                    "This section has moved in the document \u{2014} the edit was not applied."
                        .to_string(),
                ));
            }
            cx.notify();
            return;
        };
        let emptied = typed.trim().is_empty();
        let next = splice_section(&body, range, &typed);
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.section_edit = None;
        }
        if next == body {
            cx.notify();
            return;
        }
        let editor = self.plan_editor.clone();
        editor.update(cx, |state, cx| state.set_value(&next, window, cx));
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.refresh_dirty(&next);
            let parsed = if emptied {
                Vec::new()
            } else {
                parse_section_blocks(typed.trim())
            };
            doc.replace_cached_block(block_id, parsed);
        }
        self.save_document(window, cx);
    }

    /// Write the buffer back, whole. The host answers with the document's own body, which is what
    /// clears `saving` and re-seeds the buffer; the block re-index arrives separately.
    ///
    /// **A stale buffer does not save on the first ask.** The buffer was seeded at `revision` and
    /// the host has moved past it, so this write would replace a save the user has not read. The
    /// first ask raises the question and keeps every character typed; the second one means it.
    /// Whoever moved the copy is named in the banner, because overwriting an agent and overwriting
    /// a colleague are not the same decision.
    ///
    /// **That question is a courtesy and no longer the guard.** Every `SavePlan` names the
    /// revision it expects to replace, and the host refuses it with `PlanConflict` if the document
    /// has moved: a save landing between the question and the confirming press is refused rather
    /// than overwritten, and two windows both confirming an overwrite cannot both win. What the
    /// confirmation still decides is *which* revision this window is willing to replace — the one
    /// it was seeded from, or the newer one it has been shown.
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
        // A confirmed overwrite replaces the copy the user was just shown, which is the newest one
        // the host has stated — not the one the buffer was seeded from, which nothing would accept
        // any more. An ordinary save has the two equal.
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
    /// was written. The buffer is untouched — that is the whole point — and the surface asks the
    /// overwrite question again about the revision it has now been told, so the next press names
    /// it and either wins or is refused in turn.
    pub(crate) fn plan_save_refused(&mut self, revision: PlanRevision, origin: SaveOrigin) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.save_refused(revision, origin);
        }
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

    /// Keep the one annotated document in step with what the editor's active tab is asking for
    /// (T-124). In `render`, because entering the layout, leaving it, switching tab and closing
    /// the tab are four routes to the same fact, and the tab's own layout is the fact.
    ///
    /// **The dialog wins while it is up.** A plan raised over a tab in annotation mode replaces
    /// the document, exactly as opening a second plan does; the tab's own document is asked for
    /// again on the frame after the dialog goes.
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
            .map(|file| file.path.clone());
        match wanted {
            Some(path) => self.open_file_document(&path, cx),
            None => self.close_file_document(cx),
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

    /// Show a thread's own passage — the "Show" button beside it in the rail. The preview draws a
    /// section at a time rather than the buffer itself, so naming the passage is not enough on its
    /// own any more: the section also has to be scrolled into view, which is what
    /// `plan_preview_scroll` is for. The buffer's selection is still set alongside it — a section
    /// opened for editing right after is a field over the live buffer, and that is what the
    /// selection is read against.
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
        let block_index = self.workbench.plan.as_ref().and_then(|doc| {
            let block_id = doc.annotation(annotation_id)?.block_id;
            doc.annotations
                .blocks()
                .iter()
                .position(|block| block.id == block_id)
        });
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.thread = Some(annotation_id);
        }
        if let Some(index) = block_index {
            self.plan_preview_list.scroll_to_reveal_item(index);
        }
        cx.notify();
    }

    /// The heading navigator's own dropdown — opened and closed the way every other trigger in the
    /// window is: a press opens it (never toggles, so a race with the panel's own outside-click
    /// dismissal cannot reopen what the user meant to close), and the panel's outside click or a
    /// row picked closes it.
    pub fn open_plan_nav(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.nav_open = !doc.nav_open;
        }
        cx.notify();
    }

    pub fn close_plan_nav(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.nav_open = false;
        }
        cx.notify();
    }

    /// A row picked in the heading navigator: scroll the preview to that heading's own section and
    /// put the composer's selection down, then close the list the way any dropdown row does.
    pub fn select_plan_nav_heading(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let entries = heading_sections(doc.annotations.blocks(), doc.annotations.annotations());
        if let Some(entry) = entries.get(index) {
            let block_id = entry.block_id;
            if let Some(block_index) = doc
                .annotations
                .blocks()
                .iter()
                .position(|block| block.id == block_id)
            {
                self.plan_preview_list.scroll_to_reveal_item(block_index);
            }
        }
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.nav_open = false;
        }
        cx.notify();
    }

    /// A mark picked in the minimap: resolve it back to the thread it stands for — the same index
    /// into [`thread_marks`]'s own answer the strip was drawn from — and show it exactly the way
    /// the rail's own "Show" button does.
    pub fn select_plan_minimap_mark(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let marks = thread_marks(doc.annotations.blocks(), doc.annotations.annotations());
        let Some(mark) = marks.get(index) else {
            return;
        };
        self.open_annotation_thread(mark.annotation_id, cx);
    }

    /// The minimap strip scrubbed or clicked, `fraction` `0.0` at its top and `1.0` at its
    /// bottom (`kit::minimap`'s `on_scrub`) — scroll the preview to the section at that point.
    /// One formula serves both a click ("jump to here") and a drag ("keep following the
    /// pointer"): every scrub recomputes the target from scratch, there is no drag anchor to lose
    /// track of.
    ///
    /// **The strip is block space, not pixel space** (T-150). The section list is virtualized, so
    /// a block nobody has scrolled past has no measured height and the document has no total
    /// height to divide a pixel offset by — the strip therefore maps a fraction onto a block
    /// index, which is exactly what [`crate::ui::document::document_minimap`] draws its marks in.
    pub fn scrub_plan_minimap(&mut self, fraction: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.workbench.plan.as_ref() else {
            return;
        };
        let len = doc.annotations.blocks().len();
        if len == 0 {
            return;
        }
        let index = ((fraction.clamp(0.0, 1.0) * len as f32) as usize).min(len - 1);
        self.plan_preview_list.scroll_to(gpui::ListOffset {
            item_ix: index,
            offset_in_item: gpui::px(0.),
        });
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
        // A fresh composer starts the rail's focus mode over — the override from a previous thread
        // does not carry into this one.
        doc.thread_focus_override = false;
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
        doc.thread_focus_override = false;
        let input = self.annotation_composer_input.clone();
        input.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// The rail's "Show all threads" button, up while a new thread's focus mode is hiding the
    /// rest. View state only — never sent anywhere, and reset the moment the composer that put it
    /// up leaves.
    pub fn show_all_threads(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.workbench.plan.as_mut() {
            doc.thread_focus_override = true;
        }
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
        let handle = doc.doc.clone();
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
    pub(crate) fn reload_plan_annotations(&mut self, changed: &DocumentHandle) {
        let Some(doc) = self.workbench.plan.as_mut() else {
            return;
        };
        if doc.doc != *changed {
            return;
        }
        // A stale `Loaded` index must not sit under the decorations while the fresh one is in
        // flight.
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
