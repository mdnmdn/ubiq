//! The build-channel ribbon: a diagonal band across the window's bottom-left corner reading
//! `alpha` or `beta`.
//!
//! Which word it carries is the bundle's version: a released version is named `vX.Y` and is beta;
//! anything else — a bare `cargo build`'s `dev` included — is alpha.
//!
//! Clicking it swaps the dock icon for the yellow mark, and clicking it again puts the bundle's
//! own back — a one-gesture way to tell two running builds apart in the dock. macOS only:
//! nothing else has an icon a running process may change.
//!
//! It is drawn as an SVG picture rather than styled markup because GPUI rotates images and not
//! boxes, and a ribbon is a rotation. `Image::from_bytes` identifies a picture by the hash of its
//! bytes, so rebuilding the same markup every frame hits the window's image cache rather than the
//! renderer.

use gpui::{
    Image, ImageFormat, ImageSource, InteractiveElement as _, IntoElement, ParentElement, Rgba,
    StatefulInteractiveElement as _, Styled, div, img, px,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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

    div()
        .absolute()
        .bottom_0()
        .left_0()
        .size(px(SIZE))
        .child(
            img(ImageSource::Image(Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                markup(word, band, theme::ribbon_ink()).into_bytes(),
            ))))
            .size(px(SIZE)),
        )
        // The band is a diagonal and the box around it is not: only the corner the band actually
        // crosses takes the click, or the ribbon would swallow every press in the bottom-left of
        // the window.
        .child(
            div()
                .id("build-ribbon")
                .absolute()
                .bottom_0()
                .left_0()
                .size(px(SIZE * BAND))
                .cursor_pointer()
                .on_click(|_, _, _| toggle_dock_icon()),
        )
}

/// How much of the corner box the band crosses — the click target, as a fraction of [`SIZE`].
const BAND: f32 = 0.42;

/// The yellow mark, worn by the dock while the swap is on.
#[cfg(target_os = "macos")]
const MARK: &[u8] = include_bytes!("../../../../assets/logo-white-on-yellow.png");

/// How much of the icon's square the artwork fills. macOS draws its own icons on an 824-in-1024
/// grid, and a full-bleed picture handed to the dock beside them reads as oversized.
#[cfg(target_os = "macos")]
const GRID: f32 = 824.0 / 1024.0;

/// The mark on a transparent square, at the size macOS draws an icon at.
///
/// The asset is full-bleed, which is right for a logo and wrong for a dock icon: the margin is
/// added here rather than kept as a second checked-in picture.
#[cfg(target_os = "macos")]
fn padded_mark() -> Option<Vec<u8>> {
    use image::{ImageFormat, RgbaImage, imageops};

    let mark = image::load_from_memory(MARK).ok()?.to_rgba8();
    let side = mark.width().max(mark.height());
    let inner = (side as f32 * GRID).round() as u32;
    let mark = imageops::resize(&mark, inner, inner, imageops::FilterType::Lanczos3);

    let mut canvas = RgbaImage::new(side, side);
    let offset = i64::from(side.saturating_sub(inner) / 2);
    imageops::overlay(&mut canvas, &mark, offset, offset);

    let mut png = Vec::new();
    canvas
        .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// Whether the dock is wearing the mark rather than the bundle's own icon. Process-wide, because
/// the dock icon is: every window's ribbon toggles the same thing.
static SWAPPED: AtomicBool = AtomicBool::new(false);

/// Put the mark on the dock, or give the bundle's own icon back.
///
/// `None` is not "no icon": AppKit reads the icon out of the bundle again, which is the whole
/// undo. Off the main thread there is no marker and nothing happens — a click is on it.
#[cfg(target_os = "macos")]
fn toggle_dock_icon() {
    use objc2::{AnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let on = !SWAPPED.fetch_xor(true, Ordering::Relaxed);
    let image = on
        .then(padded_mark)
        .flatten()
        .and_then(|png| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png)));
    // Safe here: the icon is AppKit's to own, and this is the main thread — `mtm` is the proof.
    unsafe {
        NSApplication::sharedApplication(mtm).setApplicationIconImage(image.as_deref());
    }
}

#[cfg(not(target_os = "macos"))]
fn toggle_dock_icon() {
    SWAPPED.fetch_xor(true, Ordering::Relaxed);
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
