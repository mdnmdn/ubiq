//! **The markdown view entity:** one `gpui::list()` row per root block of a document whose text
//! lives in a shared `Entity<EditorState>` — the file tab's own buffer, never a copy.
//!
//! **The block table follows the buffer, behind [`REPARSE_DEBOUNCE`].** An edit — typed in the
//! source editor or committed by the block editor (`blockedit.rs`) — emits `InputEvent::Change`,
//! which arms a timer; the timer's turn reparses, splices the row range the delta names, and carries
//! the viewport's anchor across it. Everything downstream reads [`MdView::doc`] and is therefore
//! correct as of the last quiet moment.
//!
//! **The split's scroll link is the view's own.** When the host lays the buffer beside this view it
//! calls [`MdView::set_linked`]; from then on the list's wheel scroll moves the editor and the
//! editor's scroll moves the list, by block index (`sync.rs`), with [`ScrollLink`] stopping the
//! echo. The host draws the editor and holds none of that.
//!
//! **What needs `AppState` is the host's to call.** A row is handed no `AppState`, so the diagram
//! cache is published into `fences.rs`'s frame-local hand-off before the view draws: the host calls
//! [`MdView::publish_fences`] at render time, with the column width the view laid out at last frame.
//!
//! Ported from the markdown-viewer spike's `DocTab` + `AppState::reparse_tab` (mission T-298).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ListAlignment, ListOffset, ListState, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, canvas, div, list,
    point, px,
};
use gpui_component::input::{EditorState, InputEvent};
use ubiq_md::{BlockKind, Document};
use ubiq_proto::plan::{AnnotationMark, HighlightColour};

use super::MdViewConfig;
use super::annotation::{RowActions, RowMenu};
use super::blockedit::{self, BlockEdit};
use super::blocks::Metrics;
use super::events::MdViewEvent;
use super::fences::{self, Format};
use super::minimap::{self, Minimap};
use super::outline::{Jump, JumpHooks};
use super::prose::LinkHandler;
use super::search::Find;
use super::structure::{Outline, Target};
use super::sync::{self, BlockPoint, Half, ScrollLink};
use crate::app::AppState;
use crate::state::document::RowDecor;
use crate::theme;

/// How quiet the buffer has to go before its edit reaches the block table. A typing pause, not a
/// parse budget: reparsing between two keystrokes of the same word is work nobody sees.
pub const REPARSE_DEBOUNCE: Duration = Duration::from_millis(200);

/// How far past the viewport the list keeps rows measured, so a scroll reveals rows that already
/// have a height rather than popping.
const OVERDRAW: f32 = 400.0;

pub struct MdView {
    /// The document's text — the file tab's buffer, shared. The only thing a block commit writes.
    pub(super) buffer: Entity<EditorState>,
    pub(super) doc: Document,
    /// The rows' measured heights. On the view, not in the element tree: a `ListState` rebuilt per
    /// frame has measured nothing.
    pub(super) list: ListState,
    /// The width the rows were last measured at; a different one invalidates every cached height.
    measured_at: Cell<f32>,
    /// The pane's width as of the last prepaint. The element tree is built before anything knows
    /// its bounds, so the view lays out against last frame's width and re-renders when it moves.
    width: Rc<Cell<f32>>,
    /// The text column the view laid out at last frame — what [`Self::publish_fences`] caps
    /// pictures at.
    column: Cell<f32>,
    /// What the minimap strip took out of the pane last frame — `0` when none was drawn.
    reserved: Cell<f32>,
    config: MdViewConfig,
    /// Set while the host draws the buffer beside this view (the split layout).
    linked: bool,
    /// Which half is driving a linked scroll, while it drives it.
    link: ScrollLink,
    /// The editor's scroll offset the last time the link looked. The editor emits no scroll event,
    /// so source → preview rides its `notify`, which also fires for every caret move; comparing
    /// against this is what tells a scroll apart from typing.
    source_scroll_at: Cell<f32>,
    /// The one row that is an editor rather than a rendered block, when there is one.
    pub(super) edit: Option<BlockEdit>,
    /// Whether a double-click opens the block editor.
    pub(super) editable: bool,
    /// The block the reader last clicked, as a root block index.
    pub(super) selected_block: Option<usize>,
    /// Whether the front matter row is expanded. Per view, in memory; every document opens collapsed.
    pub(super) front_open: bool,
    /// What the host says each row carries — threads, marks, highlight, focus — in row order.
    /// Pushed whole by [`Self::set_decor`]; the view never edits it.
    decor: Vec<RowDecor>,
    /// Annotation mode: the far-margin stack and the selection bar are drawn, and the margin is
    /// floored at `theme::MD_ACTION_MARGIN`.
    annotating: bool,
    /// The one row menu (`⚑` or `●`) that is open.
    menu: Option<RowMenu>,
    pub(super) find: Find,
    /// The structural minimap down the page's edge: its measurement cache and the drag in flight.
    minimap: Minimap,
    /// The heading navigator's popover, drawn over this view and opened by the host's header.
    jump: Jump,
    pub(super) focus: FocusHandle,
    /// The debounce's generation: a timer whose value is no longer current was overtaken by a
    /// later keystroke and parses nothing. No timer to cancel, so none can outlive the view.
    reparse_at: Cell<u64>,
    _watch_source: Subscription,
    _watch_edits: Subscription,
}

impl EventEmitter<MdViewEvent> for MdView {}

impl Focusable for MdView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MdView {
    /// A view over `buffer`, parsed now.
    pub fn new(buffer: Entity<EditorState>, config: MdViewConfig, cx: &mut Context<Self>) -> Self {
        let doc = ubiq_md::parse(&buffer.read(cx).value());
        let list = ListState::new(doc.blocks.len(), ListAlignment::Top, px(OVERDRAW));

        // Preview → source. `list()`'s scroll handler fires on the wheel path only, never on
        // `scroll_to`, and with the state's `RefCell` borrowed — so the work is deferred a turn.
        let view = cx.entity().downgrade();
        list.set_scroll_handler(move |_event, window, cx| {
            let view = view.clone();
            window.defer(cx, move |_window, cx| {
                view.update(cx, |this, cx| this.preview_scrolled(cx)).ok();
            });
        });

        // Source → preview: the editor's only signal is its own `notify`.
        let watch_source = cx.observe(&buffer, |this, buffer, cx| {
            this.source_scrolled(&buffer, cx)
        });
        // The reparse trigger: `Change` is the editor's one exact edit signal.
        let watch_edits = cx.subscribe(&buffer, |this, _buffer, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.source_edited(cx);
            }
        });

        MdView {
            buffer,
            doc,
            list,
            measured_at: Cell::new(0.0),
            width: Rc::new(Cell::new(0.0)),
            column: Cell::new(0.0),
            reserved: Cell::new(0.0),
            config,
            linked: false,
            link: ScrollLink::default(),
            source_scroll_at: Cell::new(f32::NAN),
            edit: None,
            editable: false,
            selected_block: None,
            front_open: false,
            decor: Vec::new(),
            annotating: false,
            menu: None,
            find: Find::default(),
            minimap: Minimap::default(),
            jump: Jump::default(),
            focus: cx.focus_handle(),
            reparse_at: Cell::new(0),
            _watch_source: watch_source,
            _watch_edits: watch_edits,
        }
    }

    // ── What the host reads ─────────────────────────────────────────

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn buffer(&self) -> &Entity<EditorState> {
        &self.buffer
    }

    pub fn list(&self) -> &ListState {
        &self.list
    }

    pub fn config(&self) -> MdViewConfig {
        self.config
    }

    pub fn selected_block(&self) -> Option<usize> {
        self.selected_block
    }

    pub fn editable(&self) -> bool {
        self.editable
    }

    /// The open block editor, if a row is one.
    pub fn editing(&self) -> Option<&BlockEdit> {
        self.edit.as_ref()
    }

    pub fn find_mut(&mut self) -> &mut Find {
        &mut self.find
    }

    pub fn annotating(&self) -> bool {
        self.annotating
    }

    pub fn decor(&self) -> &[RowDecor] {
        &self.decor
    }

    /// The decor the host pushed for `row`, if it pushed any.
    pub fn decor_at(&self, row: usize) -> Option<&RowDecor> {
        self.decor
            .binary_search_by_key(&row, |decor| decor.row)
            .ok()
            .map(|at| &self.decor[at])
    }

    /// What `row` draws in its margins this frame (`annotation::RowActions`).
    pub fn row_actions(&self, row: usize) -> RowActions {
        RowActions::of(
            self.decor_at(row),
            self.selected_block == Some(row),
            self.annotating,
            self.editable,
        )
    }

    /// The row menu that is open, if one is.
    pub fn open_menu(&self) -> Option<RowMenu> {
        self.menu
    }

    // ── What the host pushes in ─────────────────────────────────────

    /// The window's width, density, reading options and minimap settings changed. Every measured
    /// height was measured under the old ones.
    pub fn set_config(&mut self, config: MdViewConfig, cx: &mut Context<Self>) {
        if self.config == config {
            return;
        }
        self.config = config;
        self.list.remeasure();
        cx.notify();
    }

    /// Replace what the rows carry — the host's answer to every annotation change, and to every
    /// [`MdViewEvent::DocumentChanged`] (rows move). Sorted here, so a row lookup is a search.
    /// Absolute layers only: no row is remeasured.
    pub fn set_decor(&mut self, mut decor: Vec<RowDecor>, cx: &mut Context<Self>) {
        decor.sort_by_key(|d| d.row);
        if self.decor != decor {
            self.decor = decor;
            cx.notify();
        }
    }

    /// Enter or leave annotation mode. The margin floor may move the column, so every row is
    /// remeasured; an open row menu closes.
    pub fn set_annotating(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.annotating == on {
            return;
        }
        self.annotating = on;
        self.menu = None;
        self.list.remeasure();
        cx.notify();
    }

    // ── Annotation intents ──────────────────────────────────────────
    //
    // What the margin's controls call. Each emits, and none touches the decor: the host acts on
    // its own state and pushes the result back through `set_decor`.

    /// `+`: a new thread on `row`.
    pub fn add_thread(&mut self, row: usize, cx: &mut Context<Self>) {
        self.menu = None;
        cx.emit(MdViewEvent::AddThread { row });
        cx.notify();
    }

    /// The count marker: select `row` and ask for its threads.
    pub fn threads_clicked(&mut self, row: usize, cx: &mut Context<Self>) {
        self.menu = None;
        self.select_block(row, cx);
        cx.emit(MdViewEvent::ThreadsClicked { row });
        cx.notify();
    }

    /// A pick from `row`'s `⚑` menu. Closes the menu.
    pub fn request_mark(&mut self, row: usize, mark: AnnotationMark, cx: &mut Context<Self>) {
        self.menu = None;
        cx.emit(MdViewEvent::MarkRequested { row, mark });
        cx.notify();
    }

    /// `✓` / `↺` on `row`: resolve while any thread on it is open, reopen when all are resolved.
    /// Nothing when the row has no thread.
    pub fn request_resolve(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(resolved) = self
            .decor_at(row)
            .filter(|d| d.open + d.resolved > 0)
            .map(|d| d.open > 0)
        else {
            return;
        };
        self.menu = None;
        cx.emit(MdViewEvent::ResolveRequested { row, resolved });
        cx.notify();
    }

    /// A pick from `row`'s `●` menu; `None` clears. Closes the menu.
    pub fn request_highlight(
        &mut self,
        row: usize,
        colour: Option<HighlightColour>,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        cx.emit(MdViewEvent::HighlightRequested { row, colour });
        cx.notify();
    }

    /// Open one row menu, closing any other — one menu at a time.
    pub fn open_row_menu(&mut self, menu: RowMenu, cx: &mut Context<Self>) {
        if self.menu != Some(menu) {
            self.menu = Some(menu);
            cx.notify();
        }
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// Link (or unlink) the list's scroll with the buffer's — the host lays them side by side.
    pub fn set_linked(&mut self, on: bool) {
        if self.linked != on {
            self.linked = on;
            self.source_scroll_at.set(f32::NAN);
        }
    }

    /// The read-only / editable switch. Going read-only commits an open editor first; a refused
    /// commit keeps it open, and the switch with it.
    pub fn set_editable(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        blockedit::set_editable(self, on, window, cx);
    }

    /// Resolve the document's mermaid fences against `app`'s diagram cache and publish the
    /// measure pictures scale to, for this frame's rows. Called by the host at render time, before
    /// the view draws.
    pub fn publish_fences(&self, app: &AppState) {
        let column = self.column.get();
        fences::publish(app, &self.doc, (column > 0.0).then_some(column));
    }

    /// A diagram or image finished loading: the rows that draw one were measured at the
    /// placeholder's height. Splices exactly those rows, keeping every other measured height.
    pub fn remeasure_fences(&self, cx: &mut Context<Self>) {
        for (ix, block) in self.doc.blocks.iter().enumerate() {
            let pictured = match &block.kind {
                BlockKind::Code { language, .. } => Format::of(language.as_deref()).is_some(),
                BlockKind::Image { .. } => true,
                _ => false,
            };
            if pictured {
                self.list.splice(ix..ix + 1, 1);
            }
        }
        cx.notify();
    }

    /// Expand or collapse the front matter at root row `ix`; the row's height changes, so it is
    /// remeasured alone and the minimap's cache is dropped.
    pub(super) fn toggle_front(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.front_open = !self.front_open;
        self.list.splice(ix..ix + 1, 1);
        self.minimap.invalidate();
        cx.notify();
    }

    // ── Navigation and selection ────────────────────────────────────

    /// Put root block `ix` on screen. A root index is a `list()` row, so this is one call.
    pub fn reveal_block(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.doc.blocks.len() {
            self.list.scroll_to_reveal_item(ix);
            cx.notify();
        }
    }

    /// Select root block `ix`, emitting [`MdViewEvent::BlockSelected`] when it changed.
    pub fn select_block(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.doc.blocks.len() {
            return;
        }
        if self.selected_block.replace(ix) != Some(ix) {
            cx.emit(MdViewEvent::BlockSelected { block: ix });
            cx.notify();
        }
    }

    // ── The navigator ───────────────────────────────────────────────

    /// How the jump popover reaches this view: a picked heading is revealed, and a closed popover
    /// hands focus back to the page so its keys work.
    const JUMP: JumpHooks<MdView> = JumpHooks {
        jump: |this| &mut this.jump,
        pick: |this, target, _window, cx| match target {
            Target::Block(ix) => this.reveal_block(ix, cx),
        },
        closed: |this, window, cx| window.focus(&this.focus, cx),
    };

    /// Open the heading navigator over this view — the host's header control calls this.
    pub fn open_navigator(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        Self::JUMP.open(self, window, cx);
    }

    pub fn navigator_open(&self) -> bool {
        self.jump.is_open()
    }

    /// How many headings the navigator lists — what the host's control says on its tooltip.
    pub fn heading_count(&self) -> usize {
        self.doc.outline().len()
    }

    // ── The minimap drag ────────────────────────────────────────────

    /// A press or a drag on the strip: scroll the list to the pointer, under last frame's metrics.
    fn scrub(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let metrics = self.metrics(window, self.page_width());
        if self
            .minimap
            .scrub(&self.doc, &self.list, &metrics, position)
            .is_some()
        {
            cx.notify();
        }
    }

    /// The width the rows get: the pane's, less the strip when one was reserved last frame.
    fn page_width(&self) -> f32 {
        (self.width.get() - self.reserved.get()).max(0.0)
    }

    // ── The linked split ────────────────────────────────────────────

    /// The root blocks on screen: from the scroll top down to the first row that starts below the
    /// viewport. Counted off what the list measured this frame, so never past what it drew.
    pub fn visible_rows(&self) -> std::ops::Range<usize> {
        let len = self.doc.blocks.len();
        if len == 0 {
            return 0..0;
        }
        let first = self.list.logical_scroll_top().item_ix.min(len - 1);
        let bottom = self.list.viewport_bounds().bottom();
        let mut end = first + 1;
        while end < len
            && self
                .list
                .bounds_for_item(end)
                .is_some_and(|bounds| bounds.top() < bottom)
        {
            end += 1;
        }
        first..end
    }

    /// **Preview → source.** Put the editor on the line the list's top block and offset point at.
    /// Every wheel scroll also tells the host which rows are on screen.
    fn preview_scrolled(&mut self, cx: &mut Context<Self>) {
        cx.emit(MdViewEvent::Scrolled {
            rows: self.visible_rows(),
        });
        if !self.linked {
            return;
        }
        let Some(_gesture) = self.link.begin(Half::Preview) else {
            return;
        };
        let top = self.list.logical_scroll_top();
        let row_height = self
            .list
            .bounds_for_item(top.item_ix)
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let fraction = sync::fraction_in_row(f32::from(top.offset_in_item), row_height);
        let Some(offset) =
            sync::block_and_fraction_to_offset(&self.doc, BlockPoint::new(top.item_ix, fraction))
        else {
            return;
        };
        let line = sync::line_of_offset(&self.doc.source, offset);

        // The editor takes a pixel offset and has no line-scroll call.
        let scrolled_to = self.buffer.update(cx, |state, cx| {
            let line_height = f32::from(state.line_height().unwrap_or(px(0.)));
            let y = px(-(line_height * line as f32));
            state.set_scroll_offset(point(state.scroll_offset().x, y), cx);
            f32::from(y)
        });
        // Recorded inside the gesture so the observer this write wakes sees no change.
        self.source_scroll_at.set(scrolled_to);
    }

    /// **Source → preview.** Put the list on the block the editor's top line is in.
    fn source_scrolled(&mut self, buffer: &Entity<EditorState>, cx: &mut Context<Self>) {
        if !self.linked {
            return;
        }
        let state = buffer.read(cx);
        let scroll_y = f32::from(state.scroll_offset().y);
        // An edit, a caret move or a selection, not a scroll.
        if (scroll_y - self.source_scroll_at.get()).abs() < 0.5 {
            return;
        }
        self.source_scroll_at.set(scroll_y);
        let Some(top_row) = state.visible_row_range().map(|rows| rows.start) else {
            return;
        };
        let Some(_gesture) = self.link.begin(Half::Source) else {
            return;
        };
        let offset = sync::offset_of_line(&self.doc.source, top_row);
        let Some(at) = sync::offset_to_block_and_fraction(&self.doc, offset) else {
            return;
        };
        self.scroll_to_point(at);
        cx.notify();
    }

    /// Land the list on `at`: interpolated inside the row when it is measured, revealed when not
    /// (the next event interpolates against the height that layout produces).
    fn scroll_to_point(&self, at: BlockPoint) {
        match self.list.bounds_for_item(at.block) {
            Some(bounds) => self.list.scroll_to(ListOffset {
                item_ix: at.block,
                offset_in_item: px(sync::pixels_in_row(
                    at.fraction,
                    f32::from(bounds.size.height),
                )),
            }),
            None => self.list.scroll_to_reveal_item(at.block),
        }
    }

    // ── Reparse on edit ─────────────────────────────────────────────

    /// **An edit landed in the buffer.** Arm the debounce; parse nothing.
    fn source_edited(&mut self, cx: &mut Context<Self>) {
        let generation = self.reparse_at.get().wrapping_add(1);
        self.reparse_at.set(generation);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REPARSE_DEBOUNCE).await;
            this.update(cx, |this, cx| this.reparse(generation, cx))
                .ok();
        })
        .detach();
    }

    /// Reparse now, without waiting out the debounce — for a host that wrote the buffer
    /// programmatically (`set_value` raises no `Change`) and wants the rows to follow.
    pub fn resync(&mut self, cx: &mut Context<Self>) {
        let generation = self.reparse_at.get().wrapping_add(1);
        self.reparse_at.set(generation);
        self.reparse(generation, cx);
    }

    /// **The reparse**, one quiet moment after the last keystroke of a burst.
    fn reparse(&mut self, generation: u64, cx: &mut Context<Self>) {
        // Overtaken by a later keystroke, which armed its own timer.
        if self.reparse_at.get() != generation {
            return;
        }
        let source = self.buffer.read(cx).value();
        if self.doc.source.as_str() == &*source {
            return;
        }

        // The anchor, off the old table and the layout that measured it.
        let top = self.list.logical_scroll_top();
        let row_height = self
            .list
            .bounds_for_item(top.item_ix)
            .map_or(0.0, |bounds| f32::from(bounds.size.height));
        let anchor = BlockPoint::new(
            top.item_ix,
            sync::fraction_in_row(f32::from(top.offset_in_item), row_height),
        );

        let delta = self.doc.reparse(&source);
        let (rows, count) = sync::splice_range(delta);
        // Splice, not reset: every row outside `rows` keeps its measured height. Unconditional —
        // the list's row count has to agree with the block table.
        self.list.splice(rows, count);

        // The anchor rewrite is a scroll, so it is behind the link's guard.
        if let Some(carried) = sync::carry_anchor(anchor, delta, self.doc.blocks.len())
            && carried.rewrite
            && let Some(_gesture) = self.link.begin_reparse()
        {
            self.scroll_to_point(carried.at);
        }

        self.selected_block = self.selected_block.filter(|ix| *ix < self.doc.blocks.len());
        // A row menu names a row of the old table; the decor waits for the host's next push.
        self.menu = None;
        if !self.find.open {
            self.find.forget();
        }
        // An open block editor is checked against the table that just replaced its own.
        blockedit::note_stale(self);

        cx.emit(MdViewEvent::DocumentChanged {
            blocks: self.doc.blocks.len(),
        });
        cx.notify();
    }

    // ── Layout ──────────────────────────────────────────────────────

    /// The page metrics for a pane `width` wide. The minimap's reserve comes out of `width` before
    /// this is called, so the column is centred in what the rows actually get.
    pub fn metrics(&self, window: &Window, width: f32) -> Metrics {
        let viewport_height = f32::from(window.viewport_size().height);
        let min_margin = if self.annotating {
            theme::MD_ACTION_MARGIN
        } else {
            0.0
        };
        Metrics::measure(window, &self.config, width, viewport_height, min_margin)
    }

    /// What a link click in the prose does: tell the host.
    fn link_handler(cx: &mut Context<Self>) -> LinkHandler {
        let view = cx.entity().downgrade();
        Rc::new(move |dest: &SharedString, _window, cx| {
            let dest = dest.clone();
            view.update(cx, |_, cx| cx.emit(MdViewEvent::LinkClicked(dest)))
                .ok();
        })
    }
}

impl Render for MdView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Last frame's width, or the window's on the first frame — a one-frame estimate.
        let pane = match self.width.get() {
            w if w > 0.0 => w,
            _ => f32::from(window.viewport_size().width),
        };
        // The strip's cost comes out of the pane before the column is centred, so it can never
        // overlap prose (`minimap::reserve`). Not beside a linked buffer: half a pane is cramped.
        let full = self.metrics(window, pane);
        let reserved = minimap::reserve(
            self.config.minimap && !self.linked,
            pane,
            full.column,
            full.margin,
            full.body,
        );
        self.reserved.set(reserved.unwrap_or(0.0));
        let width = pane - reserved.unwrap_or(0.0);
        let metrics = self
            .metrics(window, width)
            .with_link_handler(Self::link_handler(cx));
        self.column.set(metrics.column);

        // Reflow: a resize invalidates every cached row height, and only this comparison knows.
        if (self.measured_at.get() - width).abs() > 0.5 {
            self.measured_at.set(width);
            self.list.remeasure();
        }

        let view = cx.entity().downgrade();
        let strip = reserved.map(|reserved| {
            let press = view.clone();
            minimap::strip(
                &mut self.minimap,
                &self.doc,
                &self.list,
                &metrics,
                reserved,
                move |event, window, cx| {
                    let position = event.position;
                    press
                        .update(cx, |this, cx| {
                            this.minimap.dragging = true;
                            this.scrub(position, window, cx);
                        })
                        .ok();
                },
            )
        });

        let rows = list(self.list.clone(), {
            let view = view.clone();
            move |ix, _window, cx: &mut App| {
                let Some(this) = view.upgrade() else {
                    return div().into_any_element();
                };
                blockedit::row(&view, this.read(cx), ix, &metrics)
            }
        })
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.));

        // Records the pane's width once it has bounds, and re-renders when it moved.
        let seen = self.width.clone();
        let measure = canvas(
            move |bounds, _window, cx| {
                let w = f32::from(bounds.size.width);
                if (seen.get() - w).abs() > 0.5 {
                    seen.set(w);
                    view.update(cx, |_, cx| cx.notify()).ok();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let top = self.list.logical_scroll_top().item_ix;
        let outline = Outline::new(&self.doc, Some(top));
        let navigator = Self::JUMP.overlay(self, Some(&outline), window, cx);
        let (before, after) = match self.config.minimap_side {
            theme::MdMinimapSide::Left => (strip, None),
            theme::MdMinimapSide::Right => (None, strip),
        };

        div()
            .id("mdview")
            .key_context("MdView")
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_row()
            .flex_1()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .bg(theme::pane_bg())
            // The strip's drag, tracked on the whole view: its first pixel is usually off the strip.
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if !this.minimap.dragging {
                    return;
                }
                if event.pressed_button == Some(MouseButton::Left) {
                    this.scrub(event.position, window, cx);
                } else {
                    this.minimap.dragging = false;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.minimap.dragging = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.minimap.dragging = false),
            )
            .child(measure)
            .children(before)
            .child(rows)
            .children(after)
            .children(navigator)
    }
}
