//! A drawn diagram, and the source that would not draw.
//!
//! **Nothing is rendered here.** Mermaid is a language rather than a data format, and turning it
//! into a picture is seconds of layout in the worst case; [`crate::state::diagrams`] owns the
//! renderer and the background thread it runs on, and this draws what came back. What this viewer
//! adds is the asking: a diagram whose picture is not in the window's cache is asked for the first
//! time it is drawn, and the panel says so until it lands.
//!
//! In a panel the picture sits on the same camera a scene does: fitted to start, wheel to zoom,
//! drag to pan. A fence inside a Markdown document is drawn at the SVG's own size instead, because
//! a fence is a block in a document and the document is what scrolls.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui::{
    AnyElement, Context, ImageSource, IntoElement, ParentElement, Styled, div, img, px,
};

use crate::app::{AppState, DiagramEntry};
use crate::state::viewport::Content;
use crate::theme;
use crate::ui::kit::mono;

/// The panel's diagram: whatever the window holds for this source, asked for if it holds nothing.
pub fn render(
    app: &AppState,
    key: &str,
    source: &str,
    cx: &mut Context<AppState>,
) -> AnyElement {
    match app.diagram(source) {
        DiagramEntry::Pending => super::note("Drawing\u{2026}", theme::text_faint()),
        DiagramEntry::Failed(reason) => failed(&reason, source),
        DiagramEntry::Ready(picture) => {
            let content = Content::from_size(picture.width, picture.height);
            let camera = {
                let vp = app.viewport(key);
                vp.camera(content, vp.panel_w, vp.panel_h)
            };
            super::viewport::surface(app, key, theme::app_bg(), content, cx)
                .child(
                    img(ImageSource::Image(picture.image))
                        .absolute()
                        .left(px(camera.offset_x))
                        .top(px(camera.offset_y))
                        .w(px(picture.width * camera.scale))
                        .h(px(picture.height * camera.scale)),
                )
                .into_any_element()
        }
    }
}

/// A fenced diagram inside a Markdown document, from what the document resolved for it.
///
/// The Markdown view's block renderer is handed a window and nothing else — no `AppState`, and no
/// way to reach the one whose frame it is drawing inside — so the document resolves every fence it
/// is about to draw through [`publish`], and this reads that back. **One renderer, two call
/// sites**: the same cache and the same background render the panel uses, so a document with
/// several fences fills in as each of them lands.
pub fn drawn(source: &str) -> AnyElement {
    match RESOLVED.with_borrow(|resolved| resolved.get(source).cloned()) {
        Some(entry) => draw(entry, source),
        // A fence the document did not resolve, which is the frame before it did.
        None => super::note("\u{2026}", theme::text_faint()),
    }
}

/// Resolve one fence against the window's cache, for the renderer that cannot reach it.
pub fn publish(app: &AppState, source: &str) {
    let entry = app.diagram(source);
    RESOLVED.with_borrow_mut(|resolved| resolved.insert(source.to_string(), entry));
}

thread_local! {
    /// What each fence drawn so far resolved to. The window is single-threaded and the block
    /// renderer runs on it, so this is a hand-off between two points in one frame rather than
    /// state of its own — the cache it is copied from is `AppState`'s, and this holds nothing that
    /// is not already there.
    static RESOLVED: RefCell<HashMap<String, DiagramEntry>> = RefCell::new(HashMap::new());
}

/// One diagram, in whichever of its three states it is in. Used by a fence, which has no camera.
fn draw(entry: DiagramEntry, source: &str) -> AnyElement {
    match entry {
        // A viewer whose picture has not arrived draws an empty body until it does.
        DiagramEntry::Pending => super::note("Drawing\u{2026}", theme::text_faint()),
        // Drawn at the size the renderer measured, which is the size the SVG's own viewBox gives:
        // stretching a diagram to whatever box it landed in is what that field exists to prevent.
        // `img` and never `svg().data()`, which reduces the markup to an alpha mask and would draw
        // every diagram in one colour.
        DiagramEntry::Ready(picture) => div()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .p_3()
            .child(
                img(ImageSource::Image(picture.image))
                    .flex_none()
                    .w(px(picture.width))
                    .h(px(picture.height)),
            )
            .into_any_element(),
        DiagramEntry::Failed(reason) => failed(&reason, source),
    }
}

/// A source that will not draw shows the renderer's own words above it. The words are about the
/// source and are no use apart from it.
fn failed(reason: &str, source: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .p_3()
        .child(mono(reason.to_string(), theme::danger()))
        .child(mono(source.to_string(), theme::text_muted()))
        .into_any_element()
}
