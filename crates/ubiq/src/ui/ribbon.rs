//! The build-channel ribbon: a diagonal band across the window's bottom-left corner reading
//! `alpha` or `beta`.
//!
//! Which word it carries is the bundle's version: a released version is named `vX.Y` and is beta;
//! anything else — a bare `cargo build`'s `dev` included — is alpha.
//!
//! It is drawn as an SVG picture rather than styled markup because GPUI rotates images and not
//! boxes, and a ribbon is a rotation. `Image::from_bytes` identifies a picture by the hash of its
//! bytes, so rebuilding the same markup every frame hits the window's image cache rather than the
//! renderer.

use gpui::{
    Image, ImageFormat, ImageSource, IntoElement, ParentElement, Rgba, Styled, div, img, px,
};
use std::sync::Arc;

use crate::{theme, version};

/// How large the corner box is drawn, in pixels. The band lies across its diagonal.
const SIZE: f32 = 96.0;

pub fn render() -> impl IntoElement {
    let beta = version::FULL.starts_with('v');
    let (word, band) = if beta {
        ("beta", theme::ribbon_beta())
    } else {
        ("alpha", theme::ribbon_alpha())
    };

    div().absolute().bottom_0().left_0().size(px(SIZE)).child(
        img(ImageSource::Image(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            markup(word, band, theme::ribbon_ink()).into_bytes(),
        ))))
        .size(px(SIZE)),
    )
}

/// The picture, in a 100×100 box whose bottom-left corner the band crosses.
///
/// The band lies between the lines `y = x + 45` and `y = x + 72`; the word sits on the midline
/// between them, rotated onto it: the band runs down-and-right from the left edge to the bottom one, so the word
/// turns with it.
fn markup(word: &str, band: Rgba, ink: Rgba) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
<polygon points="0,45 55,100 28,100 0,72" fill="{band}"/>
<text x="20.75" y="79.25" transform="rotate(45 20.75 79.25)" fill="{ink}"
 font-family="sans-serif" font-size="11" font-weight="700" letter-spacing="1"
 text-anchor="middle" dominant-baseline="central">{word}</text>
</svg>"##,
        band = hex(band),
        ink = hex(ink),
    )
}

/// A token as SVG writes colours. Alpha is dropped: both ribbon tokens are opaque.
fn hex(colour: Rgba) -> String {
    let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(colour.r),
        channel(colour.g),
        channel(colour.b)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_writes_six_digits() {
        assert_eq!(
            hex(Rgba {
                r: 1.0,
                g: 0.0,
                b: 0.5,
                a: 1.0
            }),
            "#ff0080"
        );
    }

    #[test]
    fn markup_carries_the_word_and_the_colours() {
        let svg = markup(
            "alpha",
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            },
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        );
        assert!(svg.contains(">alpha</text>"));
        assert!(svg.contains("fill=\"#ffff00\""));
        assert!(svg.contains("fill=\"#000000\""));
    }
}
