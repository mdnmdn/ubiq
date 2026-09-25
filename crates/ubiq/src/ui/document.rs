//! The annotated-document surface: a markdown preview with its threads beside it.
//!
//! **One surface, two frames.** `crate::ui::plan` hosts it in the plan dialog and
//! `crate::ui::viewer` hosts it as a markdown tab's fourth layout (`ViewLayout::Annotation`), and
//! nothing below knows which — it reads a `DocumentEditor`, whose `presentation` is the only thing
//! that differs. That is the whole reason this module exists: the rail, the gutter, the composer
//! and the section editor are the annotation machinery, and there is exactly one of them.
//!
//! **The document is read, not edited, until a section is opened for it.** There is no source view
//! here and no layout toggle: the body is drawn as an ordinary markdown preview, section by
//! section, with the right-hand gutter of each section carrying what is true about it — how many
//! threads it has, and, under the pointer, the two things that can be done to it. A double-click
//! opens one section as raw markdown in a field; confirming writes it back and saves, and the
//! preview returns. `_docs/wip/planning-system.md` decision 6 — the writing surface is markdown
//! source, never WYSIWYG — still holds: what changed is that the source is opened one section at
//! a time rather than for the whole document at once.
//!
//! **The buffer is still `app.plan_editor`** and still the document's one text: a section edit
//! splices into it and saves it whole, so dirtiness, the revision watermark, the overwrite
//! question and the host's block matching all work exactly as they did. It is simply never drawn.
//!
//! *A thread* lives in the rail. There is no popover anchored into the text — a preview has no
//! document offsets to anchor one to — and a section reports its threads in its gutter.

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ListState,
    MouseButton, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div,
    list, px,
};
use gpui_component::input::{Input, Textarea};
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::{AppState, SubmitSearch};
use crate::state::document::{
    AnnotationsBody, ComposerTarget, DocumentBody, DocumentEditor, MinimapBlockKind, Notice,
    heading_sections, is_frontmatter_fields, minimap_rows, thread_marks,
};
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::{
    MdNavEntry, MinimapMark, MinimapTick, MinimapViewport, UbiqIcon, choice_pill, ghost_button,
    icon_button, md_navigator, minimap, mono, primary_button, slab, status_dot,
};
use crate::ui::viewer::markdown;
use crate::ui::{eid, eid2, indexed, scrub};
use ubiq_proto::ids::BlockId;
use ubiq_proto::plan::{Annotation, AnnotationState, PlanBlock};
use ubiq_proto::work::{Comment, CommentAuthor};

/// The rail is a fixed column: a thread is short, and a wider one would only narrow the document.
pub const RAIL_WIDTH: f32 = 320.0;
/// The minimap beside the document — a thin strip, not a panel. The same figure the standard
/// viewer's own minimap draws at.
pub const MINIMAP_WIDTH: f32 = 72.0;
/// The chrome strip above the document, the file viewer's own header height.
const HEADER: f32 = 32.0;
/// The strip down the right of the document where each section reports its threads and offers its
/// two actions. Wide enough for the count and both buttons, and the same width for every section
/// so the prose keeps one measure.
const GUTTER_WIDTH: f32 = 96.0;
/// How far past the viewport the section list keeps sections rendered, so a scroll has something
/// already laid out to reveal. The board's lanes use the same figure for the same reason; a
/// section is taller than a card, so this is fewer rows than it looks.
pub const PLAN_OVERDRAW: f32 = 600.0;

/// The whole surface below whatever chrome its frame drew: the notices, then the three columns.
///
/// The minimap's side is the reading setting T-118 persists, and it lands *between* the document
/// and the thread rail when it is on the right rather than past it — the rail stays the outermost
/// column either way, so the minimap and the text it maps stay adjacent.
pub fn surface(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    let ui = &app.workbench.settings.ui;
    let side = ui.md_minimap_side;
    let minimap_el = ui.md_minimap.then(|| document_minimap(app, doc, &view));

    let document_col = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(document(app, doc, cx));
    let rail_col = div()
        .flex_none()
        .w(px(RAIL_WIDTH))
        .min_h(px(0.))
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(theme::border())
        .child(rail(app, doc, cx));

    let columns = div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .border_t_1()
        .border_color(theme::border());
    let columns = match side {
        theme::MdMinimapSide::Left => columns.children(minimap_el).child(document_col),
        theme::MdMinimapSide::Right => columns.child(document_col).children(minimap_el),
    }
    .child(rail_col);

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .children(doc.notice.as_ref().map(banner))
        .children(state_banner(doc, cx))
        .child(columns)
        .into_any_element()
}

/// The heading navigator for an open document: the document's own structure, each row carrying
/// how many threads sit under it — which is also how many threads are open at a glance.
///
/// Both frames draw it in their own chrome, so it is built here rather than twice: `above_modal`
/// is the only thing that differs, and it is exactly the plan dialog's own reason
/// ([`crate::ui::kit::md_navigator`]).
pub fn navigator(doc: &DocumentEditor, above_modal: bool, view: &Entity<AppState>) -> AnyElement {
    let open_threads = doc
        .annotations
        .annotations()
        .iter()
        .filter(|annotation| annotation.is_open())
        .count();
    let entries: Vec<MdNavEntry> =
        heading_sections(doc.annotations.blocks(), doc.annotations.annotations())
            .into_iter()
            .map(|entry| MdNavEntry::new(entry.level, entry.label, entry.open, entry.resolved))
            .collect();

    md_navigator(
        eid("plan-nav", doc.surface_key()),
        format!("{open_threads} threads open"),
        doc.nav_open,
        &entries,
        above_modal,
        crate::ui::handler(view, |this, _, cx| this.open_plan_nav(cx)),
        std::rc::Rc::new(indexed(view, |this, index, _, cx| {
            this.select_plan_nav_heading(index, cx)
        })),
        crate::ui::handler(view, |this, _, cx| this.close_plan_nav(cx)),
    )
}

/// The minimap beside the surface: the document's own layout in miniature
/// (`_docs/inbox/markdown-improvement-proposal.md` §8.2) plus one edge tick per thread, in the
/// colour of whether it is still open, and the translucent viewport rectangle §8.3 asks for.
///
/// **The strip is block space, not pixel space** (T-150). It used to place each mark by
/// `ScrollHandle::bounds_for_item`, which answered from what the preview had painted — but the
/// section list is virtualized now, so the only sections with a measured height are the ones
/// somebody has scrolled past, and there is no total document height to divide a pixel offset by.
/// Mixing measured pixels for those against [`proportional_fraction`]'s even spread for the rest
/// would put the two halves of the strip in different coordinate systems and make a mark jump as
/// its block came into view. Every mark, every thread tick and the viewport rectangle are
/// therefore placed by block index over block count, which is one space, stable from the frame
/// the document opens in, and the same space [`AppState::scrub_plan_minimap`] reads a scrub back
/// into.
///
/// What that costs is proposal §8.3's 1:1 scale for a short document — the strip always spans the
/// whole document now — and the marks no longer being proportional to how tall a section really
/// draws. A row inside a block (several to a paragraph or a table) never had a position of its
/// own (§8.5 rules out a second layout pass), which is why [`MinimapRow`] carries
/// `row_index`/`row_count`; they are spread evenly inside the block's own slice as before.
///
/// [`MinimapRow`]: crate::state::document::MinimapRow
/// [`AppState::scrub_plan_minimap`]: crate::app::AppState::scrub_plan_minimap
fn document_minimap(app: &AppState, doc: &DocumentEditor, view: &Entity<AppState>) -> AnyElement {
    let blocks = doc.annotations.blocks();
    let len = blocks.len();

    let rows = minimap_rows(blocks);
    let kit_marks: Vec<MinimapMark> = rows
        .iter()
        .map(|row| {
            let (top, bottom) = block_span(row.block_index, len);
            let span = (bottom - top).max(0.0);
            let row_top = top + span * (row.row_index as f32 / row.row_count as f32);
            let row_height = span / row.row_count as f32;
            let (colour, dotted) = mark_style(row.kind);
            MinimapMark::new(row_top, row_height, row.length, dotted, colour)
        })
        .collect();

    let marks = thread_marks(blocks, doc.annotations.annotations());
    let ticks: Vec<MinimapTick> = marks
        .iter()
        .map(|mark| {
            let (top, _) = block_span(mark.block_index, len);
            let colour = if mark.open {
                theme::info()
            } else {
                theme::success()
            };
            MinimapTick::new(top, colour)
        })
        .collect();

    let viewport =
        visible_blocks(&app.plan_preview_list, len).map(|(first, count)| MinimapViewport {
            top: (first as f32 / len as f32).clamp(0.0, 1.0),
            height: (count as f32 / len as f32).clamp(0.02, 1.0),
        });

    minimap(
        eid("plan-minimap", doc.surface_key()),
        MINIMAP_WIDTH,
        &kit_marks,
        &ticks,
        viewport,
        std::rc::Rc::new(indexed(view, |this, index, _, cx| {
            this.select_plan_minimap_mark(index, cx)
        })),
        scrub(view, |this, fraction, _, cx| {
            this.scrub_plan_minimap(fraction, cx)
        }),
    )
}

/// A colour and a dotted flag per §8.2's table — the block-kind facts a minimap mark carries,
/// resolved to a palette token here so `kit::minimap` never has to know what a block kind is.
pub(crate) fn mark_style(kind: MinimapBlockKind) -> (gpui::Rgba, bool) {
    match kind {
        MinimapBlockKind::Heading => (theme::text(), false),
        MinimapBlockKind::Paragraph => (theme::text_faint(), false),
        MinimapBlockKind::Code => (theme::border(), false),
        MinimapBlockKind::Table => (theme::border(), true),
        MinimapBlockKind::Image => (theme::fade(theme::text_faint(), 0.5), false),
    }
}

/// A block's own slice of the strip, `0.0..=1.0` — its index over the block count. See
/// [`document_minimap`] for why this is not measured in pixels.
fn block_span(index: usize, len: usize) -> (f32, f32) {
    (
        proportional_fraction(index, len),
        proportional_fraction(index + 1, len),
    )
}

/// A block's own position down the strip, spread evenly across the document's blocks.
fn proportional_fraction(index: usize, len: usize) -> f32 {
    if len == 0 {
        0.0
    } else {
        (index as f32 / len as f32).clamp(0.0, 1.0)
    }
}

/// The first section on screen and how many are, or `None` when the whole document already fits
/// and there is no viewport smaller than the strip to draw.
///
/// Counted off the list rather than computed: `ListState::bounds_for_item` answers for exactly
/// the sections it rendered this frame, which is the set that is on screen plus the overdraw, so
/// walking forward from the scroll top until a section starts below the viewport is a walk over
/// what is already measured and nothing else.
fn visible_blocks(list: &ListState, len: usize) -> Option<(usize, usize)> {
    if len == 0 {
        return None;
    }
    let viewport = list.viewport_bounds();
    if viewport.size.height <= px(0.) {
        return None;
    }
    let first = list.logical_scroll_top().item_ix.min(len - 1);
    let mut count = 0usize;
    for index in first..len {
        match list.bounds_for_item(index) {
            Some(bounds) if bounds.top() < viewport.bottom() => count += 1,
            _ => break,
        }
    }
    let count = count.max(1);
    (first > 0 || count < len).then_some((first, count))
}

/// The document itself — always the preview, because there is no other view of it.
fn document(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    match &doc.body {
        DocumentBody::Loading => note("Reading\u{2026}", theme::text_faint()),
        DocumentBody::Failed(reason) => note(reason.clone(), theme::danger()),
        DocumentBody::Loaded(_) => preview(app, doc, cx),
    }
}

/// The rendered document. Blocks the host has indexed are drawn one by one so each can carry its
/// own gutter and its own affordances — the preview has no document offsets, so a block *is* its
/// granularity here — and an unindexed document is drawn whole, with nothing to annotate yet.
///
/// **The sections are flush.** No card, no gap, no per-block padding: the padding belongs to the
/// document, so what the reader sees is an ordinary markdown page with a gutter down its right.
///
/// **The list is virtualized** (T-150). Every section is a whole `TextView`, and drawing them all
/// meant laying out the whole document every frame to show a screenful of it — a 400-block
/// document cost roughly forty times what a ten-block one did, and the annotation surface got
/// slower the longer the document was, which is the report. `gpui::list` hands the row builder
/// below only the sections between the scroll top and the bottom of the viewport plus
/// [`PLAN_OVERDRAW`]; everything else contributes its cached height and nothing more.
fn preview(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let key = doc.surface_key();
    let blocks = doc.annotations.blocks();
    if blocks.is_empty() {
        let source = app.plan_editor.read(cx).value().to_string();
        return div()
            .id(eid("plan-preview", &key))
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_y_scroll()
            .p_4()
            .child(markdown::render(app, &key, &source, false, cx))
            .into_any_element();
    }

    // A thread's "Show" button, the heading navigator and the minimap all bring a section into
    // view by index into this list — see `AppState::plan_preview_list`.
    let list_state = app.plan_preview_list.clone();
    if list_state.item_count() != blocks.len() {
        // `reset` is the only way to change a list's length, and it drops the scroll position with
        // the measurements — so the reader's place is taken before and put back after. A block
        // count changes when the host re-indexes the document, which is routinely *while the
        // reader is part-way down it* (a section edit that splits a block is the common case), and
        // being thrown back to the first line every time would be worse than the old column's
        // cost. `AppState::open_document` resets to the top for the one case where starting over
        // is right.
        let at = list_state
            .logical_scroll_top()
            .item_ix
            .min(blocks.len().saturating_sub(1));
        list_state.reset(blocks.len());
        if at > 0 {
            list_state.scroll_to(gpui::ListOffset {
                item_ix: at,
                offset_in_item: px(0.),
            });
        }
    }
    let view = cx.entity();

    div()
        .id(eid("plan-preview", &key))
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .p_4()
        .child(
            list(list_state, move |index, window, cx| {
                section_row(index, &view, window, cx)
            })
            .flex_1()
            .min_h(px(0.)),
        )
        .into_any_element()
}

/// The one section `gpui::list` asked for, at the index it asked for.
///
/// Kept for the life of the list's `ListState` rather than for one render, so nothing here
/// borrows a particular frame's `AppState` — the document is looked up fresh off `view`, the same
/// shape `ui::board`'s own `render_row` takes. There is one annotated document per window
/// (`AppState::workbench.plan`, and `DocumentEditor::surface_key`'s note), so the document the
/// list is drawing is always that one.
fn section_row(
    index: usize,
    view: &Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let app = view.read(cx);
    let Some(doc) = app.workbench.plan.as_ref() else {
        return div().into_any_element();
    };
    let blocks = doc.annotations.blocks();
    if blocks.get(index).is_none() {
        return div().into_any_element();
    }
    section(app, doc, blocks, index, view, window)
}

/// One section of the document: the rendered markdown, and the gutter that says what is true of
/// it.
///
/// A click selects it — which is what sets a fresh thread up, the composer opening in the rail on
/// this block — and a double-click opens it as raw markdown. Hovering lifts it slightly and is
/// what reveals the two buttons; the thread count is drawn whether or not the pointer is there,
/// because it is a fact about the document rather than an action.
fn section(
    app: &AppState,
    doc: &DocumentEditor,
    blocks: &[PlanBlock],
    index: usize,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let block = &blocks[index];
    let block_id = block.id;
    if doc.editing_block() == Some(block_id) {
        return section_editor(doc, block_id, view, window);
    }

    // Marked either because a thread is being written about it, or because the rail was asked to
    // show a thread that is anchored here.
    let shown = doc
        .thread
        .and_then(|id| doc.annotation(id))
        .map(|annotation| annotation.block_id);
    let selected = doc.composer_block() == Some(block_id) || shown == Some(block_id);
    let key = format!("{}:{}", doc.doc.key(), block_id);
    let group = SharedString::from(format!("plan-section-{block_id}"));

    let mut root = div()
        .id(eid("plan-block", block_id))
        .group(group.clone())
        .flex()
        .items_start()
        .gap_2()
        .px_1()
        .cursor_pointer()
        .hover(|this| this.bg(theme::hover()))
        .on_click(
            window.listener_for(view, move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.begin_section_edit(block_id, window, cx);
                } else {
                    this.compose_annotation(block_id, window, cx);
                }
            }),
        );
    if selected {
        root = root.bg(theme::selected());
    }

    // T-153: a document opening with `---` has no `frontmatter` block kind to read at
    // `ParseOptions::gfm()` (see `is_frontmatter_fields`) — the host hands this surface a
    // mis-parsed heading instead, and drawing it through the same renderer as prose reproduced
    // the misparse a second time. The preview never shows this at all, because it splits
    // frontmatter out of the source before any Markdown parse runs over it
    // (`ui::viewer::markdown::split_frontmatter`); the closest this surface can get without that
    // same pre-parse step is the preview's own collapsed-frontmatter typography — monospace,
    // faint, dense — rather than another pass through the block renderer.
    let content = if is_frontmatter_fields(blocks, index) {
        mono(block.text.clone(), theme::text_faint())
            .text_size(theme::font(Family::Content, Role::Dense))
            .into_any_element()
    } else {
        markdown::render_block(app, &key, &block.text)
    };

    root.child(div().flex_1().min_w(px(0.)).child(content))
        .child(gutter(doc, block_id, group, view, window))
        .into_any_element()
}

/// The right-hand strip beside a section: its threads, and — under the pointer — the two things
/// that can be done to it.
fn gutter(
    doc: &DocumentEditor,
    block_id: BlockId,
    group: SharedString,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let mut total = 0usize;
    let mut open = 0usize;
    for annotation in doc.annotations_for(block_id) {
        total += 1;
        if annotation.is_open() {
            open += 1;
        }
    }

    div()
        .flex_none()
        .w(px(GUTTER_WIDTH))
        .flex()
        .items_center()
        .justify_end()
        .gap_1()
        .children((total > 0).then(|| thread_count(total, open)))
        .child(
            div()
                .flex()
                .items_center()
                .invisible()
                .group_hover(group, |this| this.visible())
                .child(
                    icon_button(
                        eid("plan-section-annotate", block_id),
                        UbiqIcon::BoardComment,
                        false,
                        window.listener_for(view, move |this, _, window, cx| {
                            this.compose_annotation(block_id, window, cx)
                        }),
                    )
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Start a thread on this section")
                            .build(window, cx)
                    }),
                )
                .child(
                    icon_button(
                        eid("plan-section-edit", block_id),
                        UbiqIcon::ToolEdit,
                        false,
                        window.listener_for(view, move |this, _, window, cx| {
                            this.begin_section_edit(block_id, window, cx)
                        }),
                    )
                    .tooltip(|window, cx| {
                        gpui_component::tooltip::Tooltip::new("Edit this section as markdown")
                            .build(window, cx)
                    }),
                ),
        )
        .into_any_element()
}

/// How many threads a section carries, in the colour of what they are: still asking, or settled.
fn thread_count(total: usize, open: usize) -> AnyElement {
    let colour = if open > 0 {
        theme::info()
    } else {
        theme::success()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(
            Icon::new(UbiqIcon::BoardComment)
                .with_size(Size::XSmall)
                .text_color(colour),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(SharedString::from(format!("{total}"))),
        )
        .into_any_element()
}

/// One section opened as raw markdown. Confirming writes it back into the document and saves;
/// the host re-indexes from there, so text that has become several sections simply becomes
/// several sections, with the threads still on the one they were about.
fn section_editor(
    doc: &DocumentEditor,
    block_id: BlockId,
    view: &Entity<AppState>,
    window: &Window,
) -> AnyElement {
    let Some(edit) = doc.section_edit.as_ref() else {
        return div().into_any_element();
    };
    slab(theme::accent())
        .id(eid("plan-section-edit-body", block_id))
        .w_full()
        .min_w(px(0.))
        .p_2()
        .gap_1p5()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        // T-134: ⌘⏎ (⌃⏎ off macOS) confirms and leaves edit mode, the same key every other field
        // in the window answers to for "confirm this from inside a field" — `SubmitSearch` is
        // bound at the window and at the field's own depth (`app/mod.rs`), so it reaches here
        // whichever has the keyboard. Bare Enter stays a newline: a section is prose.
        .on_action(
            window.listener_for(view, move |this, _: &SubmitSearch, window, cx| {
                this.confirm_section_edit(window, cx)
            }),
        )
        .child(
            Textarea::new(&edit.input)
                .appearance(false)
                .bordered(false)
                .w_full()
                .text_size(theme::font(Family::Content, Role::Body)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_1()
                .child(ghost_button(
                    eid2("plan-section-cancel", block_id, "btn"),
                    None,
                    "Cancel",
                    window.listener_for(view, |this, _, _, cx| this.cancel_section_edit(cx)),
                ))
                .child(primary_button(
                    eid2("plan-section-confirm", block_id, "btn"),
                    Some(IconName::Check),
                    "Confirm",
                    window.listener_for(view, |this, _, window, cx| {
                        this.confirm_section_edit(window, cx)
                    }),
                )),
        )
        .into_any_element()
}

/// The thread rail: every annotation on the document, and the one field answering whichever of
/// them the composer is pointed at.
fn rail(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let key = doc.surface_key();
    let focused = doc.hides_other_threads();
    let body = match &doc.annotations {
        AnnotationsBody::Loading => note("Reading annotations\u{2026}", theme::text_faint()),
        AnnotationsBody::Failed(reason) => note(reason.clone(), theme::danger()),
        AnnotationsBody::Loaded { annotations, .. } => {
            let composer = doc
                .composer_block()
                .map(|_| new_thread_composer(app, doc, cx));
            let shown: Vec<&Annotation> = if focused {
                Vec::new()
            } else {
                annotations
                    .iter()
                    .filter(|annotation| doc.show_resolved || annotation.is_open())
                    .collect()
            };
            if shown.is_empty() && composer.is_none() {
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .p_3()
                    .child(note(
                        "No threads here yet. Select a passage and annotate it.",
                        theme::text_faint(),
                    ))
                    .into_any_element()
            } else {
                div()
                    .id(eid("plan-annotations-list", &key))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .gap_2()
                    .p_3()
                    .children(composer)
                    .children(
                        shown
                            .into_iter()
                            .map(|annotation| annotation_card(app, doc, annotation, cx))
                            .collect::<Vec<_>>(),
                    )
                    .into_any_element()
            }
        }
    };

    let resolved = doc
        .annotations
        .annotations()
        .iter()
        .filter(|annotation| !annotation.is_open())
        .count();

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(
            div()
                .flex_none()
                .h(px(HEADER))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text_muted())
                .border_b_1()
                .border_color(theme::border())
                .child("Threads")
                .child(div().flex_1().min_w(px(0.)))
                .children(focused.then(|| {
                    ghost_button(
                        eid("plan-show-all-threads", &key),
                        None,
                        "Show all threads",
                        cx.listener(|this, _, _, cx| this.show_all_threads(cx)),
                    )
                }))
                .children((!focused && resolved > 0).then(|| {
                    choice_pill(
                        eid("plan-show-resolved", &key),
                        format!("{resolved} resolved"),
                        doc.show_resolved,
                        cx.listener(|this, _, _, cx| this.toggle_resolved_annotations(cx)),
                    )
                    .h_full()
                })),
        )
        .child(body)
        .into_any_element()
}

/// The field claimed for a fresh thread — shown above the rail's list while it is open, and
/// nowhere else: one composer at a time.
fn new_thread_composer(
    app: &AppState,
    doc: &DocumentEditor,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let block_id = doc.composer_block();
    let passage = doc.composer_quote.clone().or_else(|| {
        block_id.and_then(|id| {
            doc.annotations
                .blocks()
                .iter()
                .find(|block| block.id == id)
                .map(|block| block.text.clone())
        })
    });

    slab(theme::accent())
        .p_2()
        .gap_1p5()
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_faint())
                .child("New thread"),
        )
        .children(passage.map(preview_line))
        .child(composer_field(app, doc, cx))
        .into_any_element()
}

/// One thread: the passage it names, whether that passage is still in the document, its comments
/// and its state — `orphaned` and `state` drawn as two separate facts, because a thread can be
/// both closed and orphaned, or open and orphaned, and neither implies the other.
fn annotation_card(
    app: &AppState,
    doc: &DocumentEditor,
    annotation: &Annotation,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let annotation_id = annotation.id;
    let edge = match annotation.state {
        AnnotationState::Open => theme::info(),
        AnnotationState::Resolved => theme::success(),
    };
    let passage = annotation.quote.clone().or_else(|| {
        doc.annotations
            .blocks()
            .iter()
            .find(|block| block.id == annotation.block_id)
            .map(|block| block.text.clone())
    });
    let replying = matches!(doc.composer, Some(ComposerTarget::Reply(id)) if id == annotation_id);

    let mut root = slab(edge).p_2().gap_1p5();

    if annotation.orphaned {
        root = root.child(orphan_banner());
    }
    root = root
        .children(passage.map(preview_line))
        .child(state_row(annotation));

    for comment in &annotation.thread {
        root = root.child(comment_row(comment));
    }

    // Show me where this is: the thread is marked, and the section it is anchored to is marked
    // with it. There is no popover over the text any more — a preview has no offsets to anchor
    // one to — so the highlight in the document *is* the answer.
    let actions = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            eid2("plan-reply", annotation_id, "btn"),
            None,
            "Reply",
            cx.listener(move |this, _, window, cx| this.compose_reply(annotation_id, window, cx)),
        ))
        .child(resolve_button(annotation, cx))
        .child(ghost_button(
            eid2("plan-thread-show", annotation_id, "btn"),
            Some(IconName::ArrowRight),
            "Show",
            cx.listener(move |this, _, _, cx| this.open_annotation_thread(annotation_id, cx)),
        ));
    root = root.child(actions);

    if replying {
        root = root.child(composer_field(app, doc, cx));
    }

    root.into_any_element()
}

fn resolve_button(annotation: &Annotation, cx: &mut Context<AppState>) -> AnyElement {
    let annotation_id = annotation.id;
    match annotation.state {
        AnnotationState::Open => ghost_button(
            eid2("plan-resolve", annotation_id, "btn"),
            Some(IconName::Check),
            "Resolve",
            cx.listener(move |this, _, _, cx| {
                this.set_annotation_resolved(annotation_id, true, cx)
            }),
        )
        .into_any_element(),
        AnnotationState::Resolved => ghost_button(
            eid2("plan-reopen", annotation_id, "btn"),
            None,
            "Reopen",
            cx.listener(move |this, _, _, cx| {
                this.set_annotation_resolved(annotation_id, false, cx)
            }),
        )
        .into_any_element(),
    }
}

fn state_row(annotation: &Annotation) -> AnyElement {
    let (label, colour) = match annotation.state {
        AnnotationState::Open => ("open", theme::info()),
        AnnotationState::Resolved => ("resolved", theme::success()),
    };
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(status_dot(colour, theme::transparent()))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(label),
        )
        .into_any_element()
}

/// An annotation whose block is gone from the saved document — kept and shown, never dropped,
/// because the thread surviving the loss is exactly what the interface must not hide.
fn orphan_banner() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1p5()
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(Size::XSmall)
                .text_color(theme::warning()),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::warning())
                .child("This passage is no longer in the document."),
        )
        .into_any_element()
}

fn preview_line(text: String) -> AnyElement {
    div()
        .text_size(theme::font(Family::Content, Role::Meta))
        .text_color(theme::text_muted())
        .child(SharedString::from(text))
        .into_any_element()
}

fn comment_row(comment: &Comment) -> AnyElement {
    let author = match comment.author {
        CommentAuthor::User => "user",
        CommentAuthor::Agent => "agent",
    };
    let when = comment.created_at.format("%Y-%m-%d %H:%M").to_string();
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .font_family(theme::MONO_FONT)
                        .text_size(theme::font(Family::Chrome, Role::Meta))
                        .text_color(theme::text_muted())
                        .child(author),
                )
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                        .text_color(theme::text_faint())
                        .child(when),
                ),
        )
        .child(
            div()
                .text_size(theme::font(Family::Content, Role::Body))
                .text_color(theme::text())
                .child(SharedString::from(comment.text.clone())),
        )
        .into_any_element()
}

/// The one composer field, wherever it is currently shown — a fresh thread's or a reply's,
/// `DocumentEditor::composer` says which.
fn composer_field(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let can_send = !doc.composer_text.trim().is_empty();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .px_2()
                .py_1()
                .bg(theme::surface())
                .border_l(px(theme::accent_edge()))
                .border_color(theme::border())
                .child(Input::new(&app.annotation_composer_input).appearance(false)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_1()
                .child(ghost_button(
                    "plan-composer-cancel",
                    None,
                    "Cancel",
                    cx.listener(|this, _, window, cx| this.cancel_annotation_composer(window, cx)),
                ))
                .child(send_button(can_send, cx)),
        )
        .into_any_element()
}

fn send_button(enabled: bool, cx: &mut Context<AppState>) -> AnyElement {
    let colour = if enabled {
        theme::accent()
    } else {
        theme::text_faint()
    };
    let mut root = div()
        .id("plan-composer-send")
        .h(px(26.))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .text_size(theme::font(Family::Chrome, Role::Body))
        .text_color(colour);
    if enabled {
        root = root
            .cursor_pointer()
            .hover(|this| this.bg(theme::hover()))
            .on_click(
                cx.listener(|this, _, window, cx| this.submit_annotation_composer(window, cx)),
            );
    }
    root.child("Send").into_any_element()
}

/// What the surface has to say about itself before anything else: the host's copy moved under an
/// unsaved edit, or Escape was pressed over one. Both are questions, and both keep the edit.
fn state_banner(doc: &DocumentEditor, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let key = doc.surface_key();
    if doc.stale {
        let who = match doc.stale_origin {
            Some(ubiq_proto::plan::SaveOrigin::Agent) => "An agent",
            Some(ubiq_proto::plan::SaveOrigin::Human) => "Someone else",
            None => "Something else",
        };
        // The second Save is what goes through, and it says so before it does: the host cannot
        // refuse a save made against an old revision, so this sentence is the guard.
        let text = if doc.confirm_overwrite {
            format!(
                "{who} saved revision {} while you were editing. Save again to replace it with yours \u{2014} their edits are lost.",
                doc.host_revision,
            )
        } else {
            format!(
                "{who} saved revision {} while you were editing; you are at {}. Save asks again before replacing it.",
                doc.host_revision, doc.revision,
            )
        };
        return Some(warning_row(&key, text, None, cx));
    }
    if doc.confirm_close {
        return Some(warning_row(
            &key,
            "Unsaved changes. Save, or close again to discard.".to_string(),
            Some("Discard"),
            cx,
        ));
    }
    None
}

/// A sentence the surface has to put in front of the reader, in the warning colour — the stale
/// banner, the close question, and the viewer's own "editing the source can orphan a thread".
///
/// `discard` names the one button it may carry; `None` is a statement rather than a question.
/// `key` scopes the discard button's id to whichever surface raised it — T-125, so two banners
/// (one per surface) drawn in the same frame do not collide.
pub fn warning_row(
    key: &str,
    text: String,
    discard: Option<&'static str>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .bg(theme::warning_soft())
        .border_l(px(theme::accent_edge()))
        .border_color(theme::warning())
        .child(
            Icon::new(IconName::TriangleAlert)
                .with_size(Size::XSmall)
                .text_color(theme::warning()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text())
                .child(SharedString::from(text)),
        )
        .children(discard.map(|label| {
            ghost_button(
                eid("plan-discard", key),
                None,
                label,
                cx.listener(|this, _, _, cx| this.discard_plan_edits(cx)),
            )
        }))
        .into_any_element()
}

pub fn note(text: impl Into<SharedString>, colour: gpui::Rgba) -> AnyElement {
    div()
        .p_5()
        .text_size(theme::font(Family::Content, Role::Body))
        .text_color(colour)
        .child(text.into())
        .into_any_element()
}

/// The last one-shot action's outcome — an export that landed, a save the host refused — said
/// where the user is looking, in the colour of what it reports.
fn banner(notice: &Notice) -> AnyElement {
    let (text, bg, edge, icon) = match notice {
        Notice::Ok(said) => (
            said.clone(),
            theme::success_soft(),
            theme::success(),
            IconName::Check,
        ),
        Notice::Err(reason) => (
            reason.clone(),
            theme::danger_soft(),
            theme::danger(),
            IconName::TriangleAlert,
        ),
    };
    div()
        .px_2()
        .py_1()
        .flex_none()
        .flex()
        .items_center()
        .gap_1p5()
        .bg(bg)
        .border_l(px(theme::accent_edge()))
        .border_color(edge)
        .child(Icon::new(icon).with_size(Size::XSmall).text_color(edge))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(theme::font(Family::Chrome, Role::Label))
                .text_color(theme::text())
                .child(SharedString::from(text)),
        )
        .into_any_element()
}
