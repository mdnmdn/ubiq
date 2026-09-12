//! `Svg` — the one component Ubiq's own A2UI catalog adds.
//!
//! **A picture an agent wrote, drawn as a picture.** The markup goes through
//! [`crate::state::a2ui::svg::sanitize`] and then to `Image::from_bytes` as an SVG, which is how
//! `ui/ribbon.rs` draws the build band and how `ui/viewer/diagram.rs` draws a rendered Mermaid
//! picture. `img` and never `svg().data()` — the latter reduces markup to a single-colour alpha
//! mask, which the diagram viewer's own comment already explains. Rebuilding the same bytes every
//! frame is free: `Image::from_bytes` identifies a picture by their hash, so it hits the window's
//! image cache.
//!
//! **The sanitizer is the boundary and it is in `state/`**, so it can be tested with no window and
//! so nothing about what is refused depends on how it is drawn. What is left here is the two
//! decisions drawing makes: how large to draw a picture that declared its own size, and what
//! `currentColor` means — which is `theme::text()`, passed down as a hex string so that `state/`
//! never names a colour.
//!
//! **A refusal is drawn, never swallowed.** A blank box cannot be told apart from an empty drawing,
//! and a security boundary nobody can see is not one.

use std::sync::Arc;

use gpui::{AnyElement, Rgba};
use gpui::{Image, ImageFormat, ImageSource, IntoElement, Styled, img, px};

use crate::state::a2ui::svg::sanitize;
use crate::theme;

use super::registry::ExtCtx;
use super::{note, refused};

/// Draw an `Svg`, or say why it is not drawn.
pub fn draw(ext: &ExtCtx) -> AnyElement {
    let markup = ext.text("markup");
    if markup.trim().is_empty() {
        return note("Svg with no markup");
    }

    let clean = match sanitize(&markup) {
        Ok(clean) => clean,
        Err(reason) => return refused(format!("Svg refused: {reason}")),
    };

    let (width, height) = fit(clean.width, clean.height);
    img(ImageSource::Image(Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        clean.bytes(&hex(theme::text())),
    ))))
    .flex_none()
    .w(px(width))
    .h(px(height))
    .into_any_element()
}

/// A picture's own size, fitted into the box a surface gives one.
///
/// The aspect ratio is the picture's: a `viewBox` exists precisely so a renderer does not have to
/// guess, and stretching a drawing to whatever space it landed in is what that field prevents. A
/// picture smaller than the box is left alone rather than blown up.
fn fit(width: f32, height: f32) -> (f32, f32) {
    let longest = width.max(height);
    if longest <= theme::A2UI_SVG_MAX || longest <= 0.0 {
        return (width, height);
    }
    let scale = theme::A2UI_SVG_MAX / longest;
    (width * scale, height * scale)
}

/// A token as SVG writes colours. Alpha is dropped: `currentColor` is a colour, not a compositing
/// instruction, and the markup's own `opacity` is what an author reaches for instead.
fn hex(colour: Rgba) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(colour.r),
        channel(colour.g),
        channel(colour.b)
    )
}
