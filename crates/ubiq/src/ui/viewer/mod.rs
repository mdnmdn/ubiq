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
pub mod zoom_modal;

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
use crate::ui::kit::{choice_pill, mono, status_dot};
use crate::ui::mdview::view::MdView;
use crate::ui::{eid, eid2};

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
        // The navigator is offered wherever the document is drawn rather than its source: over the
        // annotation surface's own block index, or over the markdown view's.
        .children(markdown.then(|| navigator(app, file, cx)).flatten())
        .child(div().flex_1().min_w(px(0.)))
        .children(markdown.then(|| edit_chip(file, cx)).flatten())
        .when(markdown && current != ViewLayout::Source, |this| {
            this.child(md_options::control(app, file, cx))
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

/// The header's heading navigator: the markdown view's own jump popover over the parsed document,
/// in every layout that draws the view — `Preview`, `Split` and `Annotation`, where the view is the
/// annotation surface's page. `Source` draws no document, so it offers none.
fn navigator(_app: &AppState, file: &OpenFile, cx: &mut Context<AppState>) -> Option<AnyElement> {
    if !draws_view(file.layout) {
        return None;
    }
    let md = file.md.clone()?;
    Some(crate::ui::document::heading_control(
        eid("md-nav", file.key()),
        &md,
        cx,
    ))
}

/// The layouts a markdown tab draws its `MdView` in.
fn draws_view(layout: ViewLayout) -> bool {
    matches!(
        layout,
        ViewLayout::Preview | ViewLayout::Split | ViewLayout::Annotation
    )
}

/// The markdown view's read-only / editable switch: on, a double-click on a block opens the block
/// editor in its place. Offered where the view is drawn.
fn edit_chip(file: &OpenFile, cx: &mut Context<AppState>) -> Option<AnyElement> {
    if !draws_view(file.layout) {
        return None;
    }
    let md = file.md.clone()?;
    Some(crate::ui::document::edit_chip(
        eid("md-edit", file.key()),
        &md,
        cx,
    ))
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
                markdown::render(app, &key, &source, false, cx)
            }
        };
    }

    let mut preview = || match file.viewer {
        ViewerKind::Markdown => match &file.md {
            Some(md) => document(app, md, cx),
            // The frame between a viewer switch and the view being built.
            None => note("Reading\u{2026}", theme::text_faint()),
        },
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
        ViewLayout::Preview => preview(),
        // Only Excalidraw and Drawio offer it, and only they have a component to host.
        ViewLayout::Edit => web::render(app, file, cx),
        // Only Markdown offers it, and only a file in the project's tree has a document handle.
        ViewLayout::Annotation => annotation(app, file, cx),
        // A markdown view scroll-links itself to the buffer beside it, by block index
        // (`MdView::set_linked`, pushed from `AppState::set_view_layout`); the diagram viewers have
        // no text worth linking a picture to. Either way this is two halves.
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

/// A markdown file's preview: its `MdView`, with the window's diagram cache published for the
/// fences it draws this frame. The view owns its scroll, minimap, navigator popover and block
/// editor; the host hands it nothing else.
fn document(app: &AppState, md: &Entity<MdView>, cx: &gpui::App) -> AnyElement {
    md.read(cx).publish_fences(app);
    md.clone().into_any_element()
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

/// A picture with the small zoom button (T-185) in its top-right corner — what turns a scaled-down
/// image or diagram into one the reader can raise near-fullscreen. `target.key` is what keys the
/// zoom modal's own camera, distinct from any camera the inline picture is drawn on.
pub(crate) fn with_zoom_button(
    picture: impl IntoElement,
    target: crate::state::zoom::ImageZoom,
) -> AnyElement {
    let id = eid("zoom-open", &target.key);
    div()
        .relative()
        .flex_none()
        .child(picture)
        .child(
            div()
                .absolute()
                .top_1()
                .right_1()
                .child(zoom_modal::zoom_button(id, target)),
        )
        .into_any_element()
}
