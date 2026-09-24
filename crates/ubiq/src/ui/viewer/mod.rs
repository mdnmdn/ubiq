//! What draws a file when it is not plain text.
//!
//! **A viewer is a pure function of bytes and a kind.** Nothing here opens a file, resolves a path
//! or spawns anything: a viewer is handed content and draws it. Usually that content is the file's
//! own bytes; sometimes — a diff, a diagram — it is something the host made from them, which
//! changes where the work happens and not the rule.
//!
//! Which viewer draws a path is [`crate::state::editor::ViewerKind`]'s answer, and anything with no
//! viewer of its own is the editor — the general case rather than a fallback. The one piece of
//! state a viewer keeps is which of its layouts is on screen, which lives on the open file beside
//! its buffer rather than in here. A diagram or a scene also has a camera — pan and zoom — which
//! lives on the window, keyed by the tab, because it is not a property of the file.

pub mod diagram;
pub mod diff;
pub mod image;
pub mod image_edit;
pub mod markdown;
pub mod md_options;
pub mod scene;
pub mod viewport;
pub mod web;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, Entity, InteractiveElement, IntoElement, ParentElement, Rgba,
    SharedString, StatefulInteractiveElement, Styled, div, px, relative,
};
use gpui_component::input::{Editor, EditorState};

use crate::app::AppState;
use crate::state::editor::{ViewLayout, ViewerKind};
use crate::state::{FileBody, OpenFile};
use crate::theme;
use crate::ui::kit::{
    MdNavEntry, MinimapMark, choice_pill, md_navigator, minimap, mono, status_dot,
};
use crate::ui::{eid, eid2, indexed, scrub};

/// The strip the layout toggle sits in, above whatever the viewer drew.
const HEADER: f32 = 32.0;

/// One open file, drawn by the viewer its kind names.
///
/// This is the whole of what a file panel shows. The header comes first and is the viewer's only
/// chrome — a viewer with more than one layout says which one it is in, and one with a single
/// layout says nothing at all, because the editor and the image have nothing to toggle between.
pub fn render(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    let mut root = surface();
    if file.viewer.has_preview() || file.editable_image() {
        root = root.child(header(app, file, cx));
    }
    root.child(body(app, file, cx)).into_any_element()
}

/// The layout toggle: the positions the file's own viewer offers, and no others.
///
/// Which one is on screen belongs to the file rather than to this row, so the click goes to
/// `AppState` and comes back as the file's own `layout` — which is also what the panel writes into
/// the dock's saved arrangement, so a document reopens as it was left.
fn header(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> impl IntoElement {
    // A picture's strip is the annotation toolbar; every other viewer keeps the layout toggle.
    if file.editable_image() {
        return image_edit::toolbar(app, file, cx).into_any_element();
    }
    let key = file.key();
    let current = file.layout;
    let markdown = file.viewer == ViewerKind::Markdown;
    // The badge is a hint about the *other* mode: while the reader is already in it, the threads
    // themselves are on screen and a dot beside the button says nothing new.
    let badge = markdown && !current.is_annotation() && app.has_annotations(file, cx);

    div()
        .h(px(HEADER))
        .px_2()
        .flex()
        .flex_none()
        .items_center()
        .justify_end()
        .gap_2()
        .bg(theme::pane_bg())
        .border_b_1()
        .border_color(theme::border())
        // The navigator is offered in all four of markdown's positions (T-124): the document's
        // structure is a fact about the file, not about how it is being drawn.
        .when(markdown, |this| this.child(navigator(app, file, cx)))
        .child(div().flex_1().min_w(px(0.)))
        .when(markdown && current != ViewLayout::Source, |this| {
            this.child(md_options::control(app, cx))
        })
        .child(div().flex().items_center().gap_1().children(
            file.viewer.layouts().iter().copied().map(|layout| {
                let key = key.clone();
                let pill = choice_pill(
                    eid2("view-layout", &key, layout.label()),
                    layout.label(),
                    current == layout,
                    cx.listener(move |this, _, _, cx| this.set_view_layout(&key, layout, cx)),
                )
                .h_full();
                if badge && layout.is_annotation() {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .h_full()
                        .child(pill)
                        .child(status_dot(theme::info(), theme::transparent()))
                        .into_any_element()
                } else {
                    pill.into_any_element()
                }
            }),
        ))
        .into_any_element()
}

/// The header's heading navigator.
///
/// Two sources, one control. In [`ViewLayout::Annotation`] the document the surface holds is the
/// better answer — the host's own block index, with each heading's thread counts beside it — and
/// picking a row scrolls the section into view. In the other three positions there is no indexed
/// document, so the headings are parsed out of the buffer and a row scrolls the preview by the
/// heading's own proportion down the source, which is what the minimap already does.
fn navigator(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    let view = cx.entity();
    if let Some(doc) = app.annotation_document(file) {
        return crate::ui::document::navigator(doc, false, &view);
    }

    let source = match &file.body {
        FileBody::Text { state, .. } => state.read(cx).value().to_string(),
        _ => String::new(),
    };
    let entries: Vec<MdNavEntry> = markdown::heading_marks(&source)
        .into_iter()
        .map(|heading| MdNavEntry::new(heading.level, heading.label, 0, 0))
        .collect();
    let key = file.key();
    let pick_key = key.clone();

    md_navigator(
        eid("md-nav", &key),
        format!("{} headings", entries.len()),
        app.workbench.open_menu == Some(crate::state::MenuId::MdNavigator),
        &entries,
        false,
        crate::ui::handler(&view, |this, _, cx| this.open_md_navigator(cx)),
        std::rc::Rc::new(indexed(&view, move |this, index, _, cx| {
            this.select_md_nav_heading(&pick_key, index, cx)
        })),
        crate::ui::handler(&view, |this, _, cx| this.close_menu(cx)),
    )
}

/// What the file is showing, which is not always what its viewer draws: a tab exists before its
/// bytes do, and a read that failed has to say so somewhere.
fn body(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    match &file.body {
        FileBody::Loading => note("Reading\u{2026}", theme::text_faint()),
        FileBody::Failed(reason) => note(reason.clone(), theme::danger()),
        FileBody::Binary => note("Not text \u{b7} nothing to show", theme::text_faint()),
        FileBody::Diff(diff) => diff::render(diff, file.layout),
        FileBody::Bytes(bytes) => image::render(bytes, &file.path),
        FileBody::ImageEdit(_) => image_edit::render(app, file, cx),
        FileBody::Text { state, .. } => drawn(app, file, state, cx),
    }
}

/// The bytes, once they are here: the buffer, what the viewer made of it, or the two side by side.
fn drawn(
    app: &AppState,
    file: &OpenFile,
    state: &Entity<EditorState>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    // The editor is the general case rather than a fallback: an extension with no viewer of its
    // own lands here and gets the highlighted buffer.
    let key = file.key();

    let buf = || buffer(state);

    // A viewer with no source/preview toggle draws one thing only. The editor is the general
    // case; an image's bytes are its body. The buffer is only cloned out in the branches that
    // actually read it — the general case (`Editor`) never does, and cloning the whole file for
    // it every frame was pure waste.
    if !file.viewer.has_preview() {
        return match file.viewer {
            ViewerKind::Editor => buf(),
            ViewerKind::Image => note("Nothing to draw", theme::text_faint()),
            // Markdown, Mermaid, Excalidraw and Drawio do have the toggle, so these are
            // unreachable here.
            ViewerKind::Markdown
            | ViewerKind::Mermaid
            | ViewerKind::Excalidraw
            | ViewerKind::Drawio => {
                let source = state.read(cx).value().to_string();
                markdown::render(app, &key, &source, file.frontmatter_open, cx)
            }
        };
    }

    let mut preview = || match file.viewer {
        ViewerKind::Markdown => {
            let source = state.read(cx).value().to_string();
            markdown::render(app, &key, &source, file.frontmatter_open, cx)
        }
        ViewerKind::Mermaid => {
            let source = state.read(cx).value().to_string();
            diagram::render(app, &key, &source, cx)
        }
        ViewerKind::Excalidraw => {
            let source = state.read(cx).value().to_string();
            scene::live(app, &key, &source, cx)
        }
        // draw.io has no native painter: the picture is the panel's own SVG export, kept in the
        // diagram cache and read back from the workarea. See `app/web_panel.rs`.
        ViewerKind::Drawio => {
            let source = state.read(cx).value().to_string();
            diagram::exported(app, &key, &source, cx)
        }
        // `has_preview` names Markdown, Mermaid, Excalidraw and Drawio and nothing else.
        ViewerKind::Editor | ViewerKind::Image => note("Nothing to draw", theme::text_faint()),
    };

    match file.layout {
        ViewLayout::Source => warned(app, file, buf(), cx),
        // The minimap only draws for a markdown file's own full-pane preview — cramped in half a
        // `Split`, and Mermaid/Excalidraw/Drawio have no headings to mark in the first place.
        ViewLayout::Preview if file.viewer == ViewerKind::Markdown => {
            markdown_preview(app, file, state, cx)
        }
        ViewLayout::Preview => preview(),
        // Only Excalidraw and Drawio offer it, and only they have a component to host.
        ViewLayout::Edit => web::render(app, file, cx),
        // Only Markdown offers it, and only a file in the project's tree has a document handle.
        ViewLayout::Annotation => annotation(app, file, cx),
        ViewLayout::Split => warned(
            app,
            file,
            div()
                .flex()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(half(buf()).border_r_1().border_color(theme::border()))
                .child(half(preview()))
                .into_any_element(),
            cx,
        ),
    }
}

/// The annotated-document surface, inside the tab — the same one the plan dialog frames, pointed
/// at this file through `crate::state::plan::file_document`.
///
/// **One document is open at a time per window**, so a tab is only ever handed the surface when
/// the document on the window *is* this file's; `AppState::settle_annotation_document` is what
/// keeps the two in step, following the editor's active tab. A second markdown tab left in this
/// layout, or a plan dialog raised over one, says so rather than drawing somebody else's threads.
fn annotation(app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> AnyElement {
    if !file.annotatable() {
        return note(
            "Only a markdown file in the project's own tree can carry annotations",
            theme::text_faint(),
        );
    }
    match app.annotation_document(file) {
        // **The surface reads the file from disk, not from this tab's buffer.** They are two
        // buffers over one file, so a tab holding unsaved edits is told: a section confirmed here
        // saves the host's copy and the tab's own edits are not in it.
        Some(doc) if file.dirty() => div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .child(crate::ui::document::warning_row(
                &file.key(),
                "This tab has unsaved edits \u{2014} the annotation surface is showing the saved \
                 file. Save the tab first."
                    .to_string(),
                None,
                cx,
            ))
            .child(crate::ui::document::surface(app, doc, cx))
            .into_any_element(),
        Some(doc) => crate::ui::document::surface(app, doc, cx),
        None => note(
            "The annotation surface is showing another document",
            theme::text_faint(),
        ),
    }
}

/// A view that shows the buffer for editing, with the warning above it when the file carries
/// threads.
///
/// **Editing the source can orphan an annotation** — a thread is anchored to a block the host
/// matched at the last save, and text rewritten out from under it comes back as a new block with a
/// new id (`ubiq-host`'s matcher orphans one way, deliberately). The thread is kept and flagged
/// rather than lost, which is exactly why this is a warning and not a refusal.
fn warned(
    app: &AppState,
    file: &OpenFile,
    view: AnyElement,
    cx: &mut Context<AppState>,
) -> AnyElement {
    if !app.has_annotations(file, cx) {
        return view;
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .child(crate::ui::document::warning_row(
            &file.key(),
            "This file carries annotations \u{2014} editing the source here can orphan a thread."
                .to_string(),
            None,
            cx,
        ))
        .child(view)
        .into_any_element()
}

/// The width the markdown minimap draws at, wherever it is drawn — the same figure `ui/plan.rs`'s
/// own `MINIMAP_WIDTH` uses, kept in step by hand rather than shared, because a screen's own
/// furniture is not `theme.rs`'s to own (`kit-and-theme.md`'s "Not here" note).
const MINIMAP_WIDTH: f32 = 72.0;

/// The standard viewer's markdown preview, with the minimap beside it when the setting asks for
/// one — T-118, proposal §8 reduced to what a single `TextView` (no per-block layout, unlike the
/// plan surface) can honestly offer: one mark per top-level block, positioned by its proportional
/// offset in the source rather than a measured pixel position. **The same shapes the annotation
/// surface draws** (T-134) — [`markdown::structure_marks`] is `ui::document::document_minimap`'s
/// own shapes computed off the raw source instead of a host block index, drawn through the same
/// `minimap` primitive and the same `ui::document::mark_style` palette, so a file looks like one
/// minimap whichever surface it is open in.
fn markdown_preview(
    app: &AppState,
    file: &OpenFile,
    state: &Entity<EditorState>,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let key = file.key();
    let source = state.read(cx).value().to_string();
    let ui = &app.workbench.settings.ui;
    // T-126: always the external-scroll-handle rendering — the one path that gives a properly
    // sized, natural-height document plus a scrollbar drawn as the pane's own edge, rather than
    // TextView's internal virtualised-list scroller, which clipped the last line, left dead
    // space below it, and drew its own scrollbar off the reading column. The minimap needed this
    // path already; the fix is offering it whether or not the minimap is on screen.
    let document = markdown::render_scrollable(
        app,
        &key,
        &source,
        file.frontmatter_open,
        &file.md_scroll,
        cx,
    );
    if !ui.md_minimap {
        return document;
    }

    let side = ui.md_minimap_side;
    // One mark per top-level block, in `ui::document::mark_style`'s own palette — see the note
    // above. A mark never draws shorter than `MIN_MARK_HEIGHT_PX` (`ui/kit/minimap.rs`), which is
    // what keeps a document with many blocks from thinning its marks into a barcode: each still
    // reads as its own bar rather than a hairline tick.
    let structure = markdown::structure_marks(&source);
    let mark_gap = if structure.is_empty() {
        0.0
    } else {
        (1.0 / structure.len() as f32) * 0.6
    };
    let marks: Vec<MinimapMark> = structure
        .iter()
        .map(|block| {
            let (colour, dotted) = crate::ui::document::mark_style(block.kind);
            MinimapMark::new(block.fraction, mark_gap, block.length, dotted, colour)
        })
        .collect();

    let view = cx.entity();
    let scroll = file.md_scroll.clone();
    let strip = minimap(
        eid("md-minimap", &key),
        MINIMAP_WIDTH,
        &marks,
        &[],
        None,
        std::rc::Rc::new(indexed(&view, |_, _, _, _| {})),
        scrub(&view, move |_, fraction, _, cx| {
            let max_offset = scroll.max_offset();
            scroll.set_offset(gpui::point(px(0.), -max_offset.y * fraction));
            cx.notify();
        }),
    );

    let row = div().flex().flex_1().min_w(px(0.)).min_h(px(0.));
    match side {
        theme::MdMinimapSide::Left => row.child(strip).child(document),
        theme::MdMinimapSide::Right => row.child(document).child(strip),
    }
    .into_any_element()
}

/// The file's own buffer. Never a copy of it: the source half of a split is the same entity the
/// source layout draws, so a toggle costs nothing and loses no undo history. It draws at the
/// content family's body size, which already carries the user's zoom.
pub(crate) fn buffer(state: &Entity<EditorState>) -> AnyElement {
    Editor::new(state)
        .h(relative(1.))
        .p_0()
        .border_0()
        .text_size(theme::font(theme::Family::Content, theme::Role::Body))
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

/// What a viewer says when it has nothing to draw yet, or cannot draw what it was given.
///
/// Centred and quiet: a viewer that failed says why in the same place the drawing would have been,
/// rather than leaving an empty box the reader has to interpret.
pub fn note(text: impl Into<SharedString>, colour: Rgba) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .p_4()
        .child(mono(text, colour))
        .into_any_element()
}

/// The frame every viewer's body is drawn in: it fills its panel, scrolls nothing by itself, and
/// carries the application's ground rather than the surface a panel header sits on.
pub fn surface() -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .bg(theme::app_bg())
}

/// The frame a fenced diagram (Mermaid or Excalidraw) draws in inside a Markdown document —
/// T-126: the picture is drawn at its own natural size, which can be wider than the reading
/// column, so this scrolls it horizontally instead of letting it spill past the viewport. The
/// element id only needs to be unique per fence in the document, which the fence's own source is.
///
/// **Left-aligned, not centred** (T-134): a paragraph's own text starts at the column's left
/// edge, and a diagram narrower than the column centred under it read as adrift from the prose
/// around it. `items_start` keeps the picture's own left edge flush with the paragraph's.
pub(crate) fn diagram_frame(key: &str, picture: impl IntoElement) -> AnyElement {
    div()
        .id(eid("md-diagram", key))
        .w_full()
        .min_w(px(0.))
        .overflow_x_scroll()
        .flex()
        .flex_col()
        .items_start()
        .py_3()
        .child(picture)
        .into_any_element()
}
