//! The clipboard's image arm: what ⌘N and paste read, and what copy-region writes.
//!
//! GPUI normalises every platform's pasteboard into `Image { format, bytes }` before Ubiq sees
//! it — a macOS TIFF arrives as `ImageFormat::Tiff`, a Windows DIB as `Bmp` — so there is
//! nothing per-platform here, and no image feature to grow: the viewer already names every
//! format but `Pnm`, and GPUI decodes them. Text wins over an image: with a string on the
//! board ⌘N keeps meaning a text file.

use gpui::{App, ClipboardEntry, ClipboardItem, Image, ImageFormat};

use crate::state::editor::EditorPaneState;

/// The clipboard's image, when it holds one this build draws.
///
/// Text, nothing, and a format the viewer does not name all answer `None`, and the caller
/// takes the text path unchanged.
pub fn clipboard_image(cx: &App) -> Option<Vec<u8>> {
    let item = cx.read_from_clipboard()?;
    if item
        .entries()
        .iter()
        .any(|entry| matches!(entry, ClipboardEntry::String(_)))
    {
        return None;
    }
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) if decodable(image.format) && !image.bytes.is_empty() => {
            Some(image.bytes.clone())
        }
        _ => None,
    })
}

/// Whether the image viewer draws this format — every `ImageFormat` but `Pnm`, which
/// `ViewerKind::of` and `viewer::image` both decline.
fn decodable(format: ImageFormat) -> bool {
    !matches!(format, ImageFormat::Pnm)
}

/// The clipboard item copy-region writes: already-flattened PNG bytes as an image.
pub fn clipboard_image_item(png: Vec<u8>) -> ClipboardItem {
    ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, png))
}

/// The name the next untitled picture takes: numbered past whatever is already open, with the
/// extension the viewer and the save-as suggestion both read.
pub fn next_capture_name(open: &EditorPaneState) -> String {
    (1..)
        .map(|n| format!("capture-{n}.png"))
        .find(|path| open.index_of(path).is_none())
        .expect("the numbering grows without bound")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::editor::{OpenFile, ViewLayout, ViewerKind};

    /// What copy-region hands the platform: one PNG image entry carrying its bytes.
    #[test]
    fn copy_region_writes_a_png_image() {
        let item = clipboard_image_item(vec![1, 2, 3]);
        assert_eq!(item.entries().len(), 1);
        match &item.entries()[0] {
            ClipboardEntry::Image(image) => {
                assert_eq!(image.format, ImageFormat::Png);
                assert_eq!(image.bytes, vec![1, 2, 3]);
            }
            other => panic!("expected an image, got {other:?}"),
        }
    }

    /// The numbering skips what is already open, the way the untitled text names do.
    #[test]
    fn capture_names_skip_what_is_open() {
        let mut open = EditorPaneState::empty();
        open.open
            .push(OpenFile::untitled("capture-1.png", ViewLayout::Preview));
        assert_eq!(next_capture_name(&open), "capture-2.png");
    }

    /// The name carries the extension the viewer reads, so the tab draws rather than says it
    /// cannot.
    #[test]
    fn a_capture_name_draws_an_image() {
        assert_eq!(
            ViewerKind::of(&next_capture_name(&EditorPaneState::empty())),
            ViewerKind::Image
        );
    }
}
