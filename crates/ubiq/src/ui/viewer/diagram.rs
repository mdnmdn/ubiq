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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gpui::{AnyElement, Context, ImageSource, IntoElement, ParentElement, Styled, div, img, px};

use crate::app::{AppState, DiagramEntry};
use crate::state::viewport::Content;
use crate::state::zoom::ImageZoom;
use crate::theme;
use crate::ui::kit::mono;

/// The panel's diagram: whatever the window holds for this source, asked for if it holds nothing.
pub fn render(app: &AppState, key: &str, source: &str, cx: &mut Context<AppState>) -> AnyElement {
    match app.diagram(source) {
        DiagramEntry::Pending => super::note("Drawing\u{2026}", theme::text_faint()),
        DiagramEntry::Failed(reason) => failed(&reason, source),
        DiagramEntry::Ready(picture) => surface(app, key, picture, cx),
    }
}

/// The Preview position of a document only a web panel can draw — a `.drawio` file. Same picture,
/// same camera; the difference is where it came from, and what a miss means. **Nothing is
/// rendered here or anywhere else in the interface**: a miss is a document the panel has not
/// exported yet, and the way to fill it in is to open the editor once.
pub fn exported(app: &AppState, key: &str, source: &str, cx: &mut Context<AppState>) -> AnyElement {
    match app.exported_preview(source) {
        DiagramEntry::Pending => super::note("\u{2026}", theme::text_faint()),
        DiagramEntry::Failed(_) => {
            super::note("Open the editor once to draw this.", theme::text_faint())
        }
        DiagramEntry::Ready(picture) => surface(app, key, picture, cx),
    }
}

/// One picture on the shared camera: fitted to start, wheel to zoom, drag to pan.
fn surface(
    app: &AppState,
    key: &str,
    picture: crate::app::DiagramPicture,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let content = Content::from_size(picture.width, picture.height);
    let camera = {
        let vp = app.viewport(key);
        vp.camera(content, vp.panel_w, vp.panel_h)
    };
    super::viewport::surface(
        app,
        key,
        theme::app_bg(),
        content,
        img(ImageSource::Image(picture.image))
            .absolute()
            .left(px(camera.offset_x))
            .top(px(camera.offset_y))
            .w(px(picture.width * camera.scale))
            .h(px(picture.height * camera.scale)),
        div().into_any_element(),
        cx,
    )
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
    /// The reading measure the document publishing right now is capped at, in pixels — `None` for
    /// no cap (the `Full` width preset, or a caller that does not cap at all, like the split
    /// layout's own preview pane). A fence's block renderer is handed no `AppState`, so this is
    /// the same hand-off `RESOLVED` already is: `markdown::render_linked_scrollable` publishes it
    /// before the text view it is about to build lays out, and [`draw`] reads it back for every
    /// fence that document holds (T-185).
    static MEASURE: Cell<Option<f32>> = const { Cell::new(None) };
}

/// Publish the measure a document's fences should scale down to fit, or `None` for no cap.
/// Published unconditionally by every markdown render, so a stale value from a previous document
/// can never bleed into this one's.
pub fn publish_measure(measure: Option<f32>) {
    MEASURE.with(|cell| cell.set(measure));
}

pub(crate) fn current_measure() -> Option<f32> {
    MEASURE.with(|cell| cell.get())
}

/// A dimension scaled down to fit `max`, never up — a diagram or an image smaller than the reading
/// column keeps its own size, one bigger is shrunk to it, aspect preserved.
pub(crate) fn scale_to_measure(width: f32, height: f32, max: Option<f32>) -> (f32, f32) {
    match max {
        Some(max) if max > 0.0 && width > max => {
            let factor = max / width;
            (max, height * factor)
        }
        _ => (width, height),
    }
}

/// One diagram, in whichever of its three states it is in. Used by a fence, which has no camera.
fn draw(entry: DiagramEntry, source: &str) -> AnyElement {
    match entry {
        // A viewer whose picture has not arrived draws an empty body until it does.
        DiagramEntry::Pending => super::note("Drawing\u{2026}", theme::text_faint()),
        // Drawn at the size the renderer measured — the SVG's own viewBox — scaled down to the
        // document's reading measure when it is wider than that (T-185): stretching a diagram
        // past its own size is still never done, only shrinking one down that overruns the
        // column. `img` and never `svg().data()`, which reduces the markup to an alpha mask and
        // would draw every diagram in one colour. A diagram still wider than the reading column
        // once at that ceiling (the `Full` preset, which caps at nothing) scrolls inside
        // `super::diagram_frame` (T-126) rather than spilling past the viewport.
        DiagramEntry::Ready(picture) => {
            let (width, height) =
                scale_to_measure(picture.width, picture.height, current_measure());
            let target = ImageZoom {
                key: source.to_string(),
                title: "Diagram".to_string(),
                image: picture.image.clone(),
                width: picture.width,
                height: picture.height,
            };
            super::diagram_frame(
                source,
                super::with_zoom_button(
                    img(ImageSource::Image(picture.image))
                        .flex_none()
                        .w(px(width))
                        .h(px(height)),
                    target,
                ),
            )
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture narrower than the measure keeps its own size — never blown up (T-185).
    #[test]
    fn a_smaller_picture_is_not_upscaled() {
        assert_eq!(scale_to_measure(200.0, 100.0, Some(400.0)), (200.0, 100.0));
    }

    /// A picture wider than the measure shrinks to it, aspect preserved.
    #[test]
    fn a_wider_picture_shrinks_to_the_measure_keeping_aspect() {
        let (w, h) = scale_to_measure(800.0, 400.0, Some(400.0));
        assert_eq!(w, 400.0);
        assert_eq!(h, 200.0);
    }

    /// No measure — the `Full` width preset, or a caller that never caps at all — draws at the
    /// picture's own size, exactly as before T-185.
    #[test]
    fn no_measure_is_no_cap() {
        assert_eq!(scale_to_measure(800.0, 400.0, None), (800.0, 400.0));
    }
}
