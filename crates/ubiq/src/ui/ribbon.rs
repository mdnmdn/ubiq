//! The ribbons the window and Git mode pin in a corner.
//!
//! The picture itself is [`crate::ui::kit::ribbon`]: a word across a corner. This file is which
//! ribbons the shell draws, and the dock-icon swap a click on the build band performs.
//!
//! The build-channel ribbon sits in the window's bottom-left, reading `alpha` or `beta`. Which
//! word it carries is the bundle's version: a released version is named `vX.Y` and is beta;
//! anything else — a bare `cargo build`'s `dev` included — is alpha. Clicking it swaps the dock
//! icon for the yellow mark, and clicking it again puts the bundle's own back — a one-gesture way
//! to tell two running builds apart in the dock. macOS only: nothing else has an icon a running
//! process may change.
//!
//! Git mode draws a second band, red, across its own top-left, reading `experimental`.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::IntoElement;

use crate::theme;
use crate::ui::kit::{RibbonCorner, ribbon};
use crate::version;

pub fn render() -> impl IntoElement {
    let beta = version::FULL.starts_with('v');
    let (word, band) = if beta {
        ("beta", theme::ribbon_beta())
    } else {
        ("alpha", theme::ribbon_alpha())
    };

    ribbon(word, RibbonCorner::BottomLeft, band, theme::ribbon_ink())
        .on_click("build-ribbon", |_, _, _| toggle_dock_icon())
}

/// A red `experimental` band across Git mode's top-left corner. It takes no click: the git
/// toolbar sits under it. Larger than the build ribbon because the word is longer.
pub fn experimental() -> impl IntoElement {
    ribbon(
        "experimental",
        RibbonCorner::TopLeft,
        theme::ribbon_experimental(),
        theme::ribbon_experimental_ink(),
    )
    .size(128.0)
}

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
