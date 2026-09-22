//! The annotated document: an editable markdown surface with its threads beside it.
//!
//! **This is the file editor, raised over the window.** The buffer is the tree's own
//! `EditorState` (`app.plan_editor`), drawn by `viewer::buffer` — the same element a `.md` tab
//! gets, with the same highlighter, line numbers, soft wrap, undo history and vim-less key map —
//! and the preview is `viewer::markdown`, the same renderer. Source, Preview and Split are the
//! file viewer's own `ViewLayout` on the same pills. Nothing here is a second editor built for a
//! modal, and nothing here is WYSIWYG: the writing surface is markdown source with a preview,
//! which `_docs/wip/planning-system.md` decision 6 accepts by name.
//!
//! **Three things sit on top of the editor, and each is one seam of the library's:**
//!
//! - *Annotated passages* are a `TextDecorationCollection` over the live text, painted by
//!   `app::plan::paint_annotation_marks` — bookmarks' own machinery. Room is deliberately left
//!   for a second collection beside it, for the per-line edit provenance being built host-side.
//! - *A thread* opens where its passage is, anchored by `EditorState::range_to_bounds`, and a
//!   click inside a decorated range is what opens it.
//! - *`/`* is a `CompletionProvider` (`ui::editor::SlashCommands`) against the library's own
//!   completion popover, so this module draws no menu at all.
//!
//! **Nothing here names a plan.** The surface reads a `DocumentEditor`, whose handle says which
//! document is open; the title is the one place a plan's task is looked up, and it asks the
//! handle rather than assuming.

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, px,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _, Size};

use crate::app::AppState;
use crate::state::Layer;
use crate::state::document::{
    AnnotationsBody, ComposerTarget, DocumentBody, DocumentEditor, Notice,
};
use crate::state::editor::ViewLayout;
use crate::theme;
use crate::theme::{Family, Role};
use crate::ui::kit::menu::MODAL_MENU_LAYER;
use crate::ui::kit::{
    card, choice_pill, ghost_button, modal_sized, primary_button, slab, status_dot,
};
use crate::ui::viewer::{self, markdown};
use crate::ui::{eid, eid2};
use ubiq_proto::plan::{Annotation, AnnotationState, PlanBlock, SaveOrigin};
use ubiq_proto::work::{Comment, CommentAuthor};

/// Wide enough to write in beside the thread rail — a writing surface, not a dialog.
const DOC_WIDTH: f32 = 860.0;
/// The rail is a fixed column: a thread is short, and a wider one would only narrow the document.
const RAIL_WIDTH: f32 = 320.0;
/// Both columns need a real height to scroll inside — `modal_sized`'s `fill_height`.
const DOC_HEIGHT: f32 = 700.0;
/// The chrome strip above the document, the file viewer's own header height.
const HEADER: f32 = 32.0;
/// The thread popover's narrowest.
const THREAD_WIDTH: f32 = 300.0;

/// The layouts this surface offers. `Edit` is the web-tenant layout and has no component here.
const LAYOUTS: [ViewLayout; 3] = [ViewLayout::Source, ViewLayout::Split, ViewLayout::Preview];

pub fn render(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let Some(doc) = app.workbench.plan.as_ref() else {
        return div().into_any_element();
    };

    let title = match doc.task_id().and_then(|id| {
        app.work(cx)
            .and_then(|work| work.task(id))
            .map(|task| task.title.clone())
    }) {
        Some(task) => format!("{} \u{2014} {task}", doc.doc.kind_label()),
        None => doc.doc.kind_label().to_string(),
    };

    let columns = div()
        .flex()
        .flex_1()
        .min_h(px(0.))
        .border_t_1()
        .border_color(theme::border())
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(document(app, doc, cx)),
        )
        .child(
            div()
                .flex_none()
                .w(px(RAIL_WIDTH))
                .min_h(px(0.))
                .flex()
                .flex_col()
                .border_l_1()
                .border_color(theme::border())
                .child(rail(app, doc, cx)),
        )
        .into_any_element();

    let body = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .children(doc.notice.as_ref().map(banner))
        .children(state_banner(doc, cx))
        .child(chrome(doc, cx))
        .child(columns)
        .children(thread_popover(app, doc, cx))
        .into_any_element();

    let view = cx.entity();

    modal_sized(
        "plan-modal",
        theme::accent(),
        DOC_WIDTH + RAIL_WIDTH,
        Some(DOC_HEIGHT),
        &title,
        body,
        footer(doc, cx),
        crate::ui::dismiss(&view, Layer::Plan, |this, _, cx| this.close_plan(cx)),
        window,
    )
}

/// The strip above the document: what it is on the left, how it is drawn on the right. Flush,
/// full height, separated by a rule — a chrome row, not a toolbar with margins.
fn chrome(doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let current = doc.layout;
    let annotating = doc.composer_block().is_some();
    let open_threads = doc
        .annotations
        .annotations()
        .iter()
        .filter(|annotation| annotation.is_open())
        .count();

    div()
        .h(px(HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .bg(theme::pane_bg())
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_faint())
                .child(SharedString::from(format!("{} threads open", open_threads))),
        )
        .child(div().flex_1().min_w(px(0.)))
        .child(ghost_button(
            "plan-annotate-selection",
            Some(IconName::Plus),
            "Annotate selection",
            cx.listener(|this, _, window, cx| this.annotate_selection(window, cx)),
        ))
        .children(annotating.then(|| {
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::accent())
                .child("writing a thread")
        }))
        .children(LAYOUTS.iter().copied().map(|layout| {
            choice_pill(
                eid2("plan-layout", doc.doc.key(), layout.label()),
                layout.label(),
                current == layout,
                cx.listener(move |this, _, _, cx| this.set_document_layout(layout, cx)),
            )
            .h_full()
            .into_any_element()
        }))
        .into_any_element()
}

/// The document itself: the buffer, the preview, or both.
fn document(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    match &doc.body {
        DocumentBody::Loading => note("Reading\u{2026}", theme::text_faint()),
        DocumentBody::Failed(reason) => note(reason.clone(), theme::danger()),
        DocumentBody::Loaded(_) => {
            let source = app.plan_editor.read(cx).value().to_string();
            match doc.layout {
                ViewLayout::Preview => preview(app, doc, &source, cx),
                ViewLayout::Split => div()
                    .flex()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .child(
                        half(buffer(app, cx))
                            .border_r_1()
                            .border_color(theme::border()),
                    )
                    .child(half(preview(app, doc, &source, cx)))
                    .into_any_element(),
                // `Edit` is the web tenant's layout and has no component here; the source is what
                // this surface means by editing.
                ViewLayout::Source | ViewLayout::Edit => buffer(app, cx),
            }
        }
    }
}

/// The buffer, on the window's one document `EditorState`.
///
/// The mouse-up is how a decorated passage is clicked: the editor has moved the caret by then,
/// so the handler reads the caret and asks which thread it landed in. There is no click target
/// on a decoration itself — a decoration is paint — and adding an invisible overlay for one
/// would break selection everywhere it covered.
fn buffer(app: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .id("plan-buffer")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.document_clicked(cx)),
        )
        .child(viewer::buffer(&app.plan_editor))
        .into_any_element()
}

/// The rendered half. Blocks the host has indexed are drawn one by one so a click on a passage
/// still claims it for a thread — the preview has no document offsets, so a block *is* its
/// granularity there — and an unindexed document is drawn whole.
fn preview(
    app: &AppState,
    doc: &DocumentEditor,
    source: &str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let key = doc.doc.key();
    let blocks = doc.annotations.blocks();
    if blocks.is_empty() {
        return div()
            .id("plan-preview")
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_y_scroll()
            .child(markdown::render(app, &key, source, false, cx))
            .into_any_element();
    }
    div()
        .id("plan-preview")
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_y_scroll()
        .gap_2()
        .p_4()
        .children(
            blocks
                .iter()
                .map(|block| block_card(app, doc, block, cx))
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

/// One block of the preview, hit-testable as a whole: a click claims it for a fresh thread. Its
/// edge reports what it carries — accent while it is being annotated, the open colour for a
/// thread still asking something, the resolved colour when every thread on it is closed.
fn block_card(
    app: &AppState,
    doc: &DocumentEditor,
    block: &PlanBlock,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let block_id = block.id;
    let selected = doc.composer_block() == Some(block_id);
    let mut total = 0usize;
    let mut open = 0usize;
    for annotation in doc.annotations_for(block_id) {
        total += 1;
        if annotation.is_open() {
            open += 1;
        }
    }
    let edge = if selected {
        theme::accent()
    } else if open > 0 {
        theme::info()
    } else if total > 0 {
        theme::success()
    } else {
        theme::border()
    };

    let key = format!("{}:{}", doc.doc.key(), block_id);
    let rendered = markdown::render_block(app, &key, &block.text);

    card(eid("plan-block", block_id), edge, selected)
        .p_2()
        .gap_1()
        .child(rendered)
        .children((total > 0).then(|| thread_count_chip(total, open)))
        .on_click(
            cx.listener(move |this, _, window, cx| this.compose_annotation(block_id, window, cx)),
        )
        .into_any_element()
}

fn thread_count_chip(total: usize, open: usize) -> AnyElement {
    let colour = if open > 0 {
        theme::info()
    } else {
        theme::success()
    };
    let label = if total == 1 {
        "1 thread".to_string()
    } else {
        format!("{total} threads")
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(
            Icon::new(IconName::Check)
                .with_size(Size::XSmall)
                .text_color(colour),
        )
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_faint())
                .child(label),
        )
        .into_any_element()
}

/// The thread rail: every annotation on the document, and the one field answering whichever of
/// them the composer is pointed at.
fn rail(app: &AppState, doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let body = match &doc.annotations {
        AnnotationsBody::Loading => note("Reading annotations\u{2026}", theme::text_faint()),
        AnnotationsBody::Failed(reason) => note(reason.clone(), theme::danger()),
        AnnotationsBody::Loaded { annotations, .. } => {
            let composer = doc
                .composer_block()
                .map(|_| new_thread_composer(app, doc, cx));
            let shown: Vec<&Annotation> = annotations
                .iter()
                .filter(|annotation| doc.show_resolved || annotation.is_open())
                .collect();
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
                    .id("plan-annotations-list")
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
                            .map(|annotation| annotation_card(app, doc, annotation, false, cx))
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
                .children((resolved > 0).then(|| {
                    choice_pill(
                        "plan-show-resolved",
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

/// The thread anchored over its own passage, where `range_to_bounds` says the passage is drawn.
///
/// Absent when the range is not laid out — a passage scrolled out of view, or a block the editor
/// has changed past recognition — and the rail still has the thread either way.
fn thread_popover(
    app: &AppState,
    doc: &DocumentEditor,
    cx: &mut Context<AppState>,
) -> Option<AnyElement> {
    let annotation_id = doc.thread?;
    let annotation = doc.annotation(annotation_id)?;
    // The preview alone has no offsets to anchor to; the rail is where the thread lives then.
    if doc.layout == ViewLayout::Preview {
        return None;
    }
    let range = app
        .annotation_ranges(cx)
        .into_iter()
        .find(|(id, _)| *id == annotation_id)
        .map(|(_, range)| range)?;
    let bounds = app.plan_editor.read(cx).range_to_bounds(&range)?;

    let panel = div()
        .id("plan-thread-popover")
        .w(px(THREAD_WIDTH))
        .flex()
        .flex_col()
        .flex_none()
        .bg(theme::surface_raised())
        .shadow_lg()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(annotation_card(app, doc, annotation, true, cx));

    Some(
        deferred(
            anchored()
                .position(bounds.bottom_left())
                .snap_to_window_with_margin(px(8.))
                .child(panel),
        )
        .priority(MODAL_MENU_LAYER)
        .into_any_element(),
    )
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
    in_popover: bool,
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

    let mut actions = div()
        .flex()
        .items_center()
        .gap_2()
        .child(ghost_button(
            eid2("plan-reply", annotation_id, "btn"),
            None,
            "Reply",
            cx.listener(move |this, _, window, cx| this.compose_reply(annotation_id, window, cx)),
        ))
        .child(resolve_button(annotation, cx));
    actions = if in_popover {
        actions.child(ghost_button(
            eid2("plan-thread-close", annotation_id, "btn"),
            None,
            "Close",
            cx.listener(|this, _, _, cx| this.close_annotation_thread(cx)),
        ))
    } else {
        // In the rail: show me where this is. The passage is selected and the popover opens over
        // it, which is the same gesture as clicking the decoration in the text.
        actions.child(ghost_button(
            eid2("plan-thread-show", annotation_id, "btn"),
            Some(IconName::ArrowRight),
            "Show",
            cx.listener(move |this, _, _, cx| this.open_annotation_thread(annotation_id, cx)),
        ))
    };
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

/// The foot of the surface: what the buffer is, then what can be done to it. Save is the primary
/// action here for the same reason ⌘S reaches it — this is an editor.
fn footer(doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    let (label, colour) = if doc.saving {
        ("Saving\u{2026}", theme::text_faint())
    } else if doc.dirty {
        ("Unsaved changes", theme::warning())
    } else {
        ("Saved", theme::text_faint())
    };

    div()
        .flex()
        .flex_1()
        .items_center()
        .gap_2()
        .child(status_dot(colour, theme::transparent()))
        .child(
            div()
                .text_size(theme::font(Family::Chrome, Role::Meta))
                .text_color(theme::text_muted())
                .child(label),
        )
        .children(change_summary(doc))
        .child(div().flex_1().min_w(px(0.)))
        .child(ghost_button(
            "plan-export",
            Some(IconName::FileText),
            "Export\u{2026}",
            cx.listener(|this, _, window, cx| this.open_export_plan_dialog(window, cx)),
        ))
        .child(save_button(doc, cx))
        .into_any_element()
}

/// What the edit-provenance layer has to say, in one line of the footer: how much changed, and
/// which of the two underline hues in the buffer are on screen.
///
/// Counts, not a report. `PlanChangeStats` carries more than this — the revisions in the window,
/// split by who made them — and a footer that showed all of it would be a dashboard in a status
/// bar. Absent entirely for a document nothing has happened to, which is the ordinary case.
fn change_summary(doc: &DocumentEditor) -> Option<AnyElement> {
    let stats = &doc.change_stats;
    if stats.is_empty() && stats.blocks_touched == 0 {
        return None;
    }
    let counts = format!(
        "+{} \u{2212}{} \u{223c}{} lines \u{00b7} {} block{}",
        stats.lines_added,
        stats.lines_removed,
        stats.lines_modified,
        stats.blocks_touched,
        if stats.blocks_touched == 1 { "" } else { "s" },
    );
    let mut row = div().flex().items_center().gap_1p5().child(
        div()
            .text_size(theme::font(Family::Chrome, Role::Meta))
            .text_color(theme::text_faint())
            .child(SharedString::from(counts)),
    );
    for (origin, label) in [
        (SaveOrigin::Human, "you"),
        (SaveOrigin::Agent, SaveOrigin::Agent.label()),
    ] {
        if !doc.has_changes_by(origin) {
            continue;
        }
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(status_dot(
                    theme::edit_origin(origin.is_human()),
                    theme::transparent(),
                ))
                .child(
                    div()
                        .text_size(theme::font(Family::Chrome, Role::Micro))
                        .text_color(theme::text_faint())
                        .child(label),
                ),
        );
    }
    Some(row.into_any_element())
}

fn save_button(doc: &DocumentEditor, cx: &mut Context<AppState>) -> AnyElement {
    if doc.dirty || doc.stale {
        // The button says what the next press does. A stale buffer's first press only asks.
        let label = if doc.confirm_overwrite {
            "Overwrite"
        } else {
            "Save"
        };
        primary_button(
            "plan-save",
            Some(IconName::Check),
            label,
            cx.listener(|this, _, window, cx| this.save_document(window, cx)),
        )
        .into_any_element()
    } else {
        ghost_button("plan-save", Some(IconName::Check), "Save", |_, _, _| {}).into_any_element()
    }
}

/// What the surface has to say about itself before anything else: the host's copy moved under an
/// unsaved edit, or Escape was pressed over one. Both are questions, and both keep the edit.
fn state_banner(doc: &DocumentEditor, cx: &mut Context<AppState>) -> Option<AnyElement> {
    if doc.stale {
        let who = match doc.stale_origin {
            Some(SaveOrigin::Agent) => "An agent",
            Some(SaveOrigin::Human) => "Someone else",
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
        return Some(warning_row(text, None, cx));
    }
    if doc.confirm_close {
        return Some(warning_row(
            "Unsaved changes. Save, or close again to discard.".to_string(),
            Some("Discard"),
            cx,
        ));
    }
    None
}

fn warning_row(
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
                "plan-discard",
                None,
                label,
                cx.listener(|this, _, _, cx| this.discard_plan_edits(cx)),
            )
        }))
        .into_any_element()
}

fn note(text: impl Into<SharedString>, colour: gpui::Rgba) -> AnyElement {
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

/// One side of a split, each taking half and neither pushing the other out.
fn half(child: AnyElement) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(child)
}
