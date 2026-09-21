//! The clipboard's image arm: what ⌘N and paste read, and what copy-region writes.
//!
//! GPUI normalises every platform's pasteboard into `Image { format, bytes }` before Ubiq sees
//! it — a macOS TIFF arrives as `ImageFormat::Tiff`, a Windows DIB as `Bmp` — so there is
//! nothing per-platform here, and no image feature to grow: the viewer already names every
//! format but `Pnm`, and GPUI decodes them. Text wins over an image: with a string on the
//! board ⌘N keeps meaning a text file.
//!
//! The composer reads the same board by a different rule — [`clipboard_attachment`] — because a
//! paste into a field is a different question from `⌘N`: a copied file arrives with its own path
//! *and* a string beside it, and the interesting half is the path.

use std::path::PathBuf;

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

/// What a paste into the composer found on the board, where it found something to attach.
///
/// Two shapes, because a copied *file* and a copied *picture* are two different things and only
/// one of them already exists on a disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PastedAttachment {
    /// A file the operating system named — a Finder copy, a file manager's copy. It is already on
    /// a disk under a path, so attaching it writes nothing.
    Path(PathBuf),
    /// A picture with no path behind it — a screenshot, an image copied out of a browser. Nothing
    /// names it, so attaching it has to put it somewhere first.
    Image { bytes: Vec<u8>, format: ImageFormat },
}

/// What a paste into the composer takes off the board, when it is something to attach.
///
/// **This is deliberately not [`clipboard_image`], and the two must not be merged.** That one
/// answers `None` the moment any string is on the board, which is right for `⌘N` — with text
/// there, the key keeps meaning *a text file*. A file copied in Finder puts an `ExternalPaths`
/// entry **and** a `String` of its path on the board at once, so the same rule would make every
/// copied file invisible here. This reads the board in the order the composer cares about
/// instead: a path first, then a picture.
///
/// **A board carrying only text answers `None`**, and the keystroke goes back to the field, where
/// it pastes as text exactly as it always has. That is the whole of what keeps this gesture from
/// taking ordinary paste away from the one control the user types into.
///
/// Only the first path is taken. A multi-file copy is a real gesture, but each file is its own
/// tag and its own `@path`, and taking them one keystroke at a time is not what the user asked
/// for — see `G106`.
pub fn clipboard_attachment(cx: &App) -> Option<PastedAttachment> {
    let item = cx.read_from_clipboard()?;
    // Paths first: a copied file is a file, whatever else the platform put beside it.
    let path = item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::ExternalPaths(paths) => paths.paths().first().cloned(),
        _ => None,
    });
    if let Some(path) = path {
        return Some(PastedAttachment::Path(path));
    }
    item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) if decodable(image.format) && !image.bytes.is_empty() => {
            Some(PastedAttachment::Image {
                bytes: image.bytes.clone(),
                format: image.format,
            })
        }
        _ => None,
    })
}

/// Where a pasted picture is written inside the project: `.ubiq/pasted/`.
///
/// **Attaching a picture writes a file into the user's project**, and that is a product decision
/// rather than an implementation detail: an attachment is an `@path` mention (`G171`), so a
/// picture that is nowhere cannot be attached at all. `.ubiq/` is already Ubiq's own folder inside
/// a project — the knowledge base writes there under `.ubiq/kb` — so a second scratch folder
/// beside it is the shape that already exists rather than a new one invented here.
///
/// **It is not the same choice `.ubiq/kb` is, though.** A project-stored knowledge base is
/// something the user asked for by adding a source; a pasted screenshot is a side effect of a
/// keystroke, and a folder of untracked binaries nobody chose has no business turning up in
/// `git status`. So the folder ignores itself — see [`PASTED_IGNORE`].
pub const PASTED_DIR: &str = ".ubiq/pasted";

/// The ignore file written beside the first pasted picture, and what goes in it.
///
/// **The folder ignores itself; the user's own `.gitignore` is never touched.** A rule appended
/// to the repository's ignore file is a change to a tracked file the user did not ask for, and it
/// would have to be merged, deduplicated and removed again. A `.gitignore` holding `*` inside
/// `.ubiq/pasted/` covers exactly this folder, is invisible in every other one, and goes away
/// when the folder does. It is written through the same `WriteProjectFile` every other pasted
/// picture takes — the interface names a project-relative path and no filesystem path of its own.
pub const PASTED_IGNORE: (&str, &str) = (".ubiq/pasted/.gitignore", "*\n");

/// The project-relative path a pasted picture is written to, and the extension it keeps.
///
/// The format here is the one [`transcodable_to_png`] settled on, not necessarily the board's:
/// the name has to match the bytes, because `viewer::image::format` and `ViewerKind::of` both
/// read the extension and never the bytes (`G178`).
///
/// **Named by the moment it was pasted, not counted.** `next_capture_name`'s numbering is right
/// for tabs, which are all open at once and can be counted; these are written once and referred
/// to for the life of a transcript, so a number reused in a later session would silently repoint
/// an older turn's chip at a newer picture. `stamp` is milliseconds since the epoch.
pub fn pasted_image_path(format: ImageFormat, stamp: u128) -> String {
    format!("{PASTED_DIR}/pasted-{stamp}.{}", extension(format))
}

/// Whether this build can re-encode `format` as PNG, and the `image` format to decode it with.
///
/// **This is what the build actually has, not what the crate offers.** `image` is compiled
/// `default-features = false` here, so only the decoders named in `crates/ubiq/Cargo.toml` exist
/// — `png`, plus `tiff` and `bmp`, which are in that list for this function. Anything else
/// answers `None` and its bytes go down untouched.
fn transcodable_to_png(format: ImageFormat) -> Option<image::ImageFormat> {
    match format {
        ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
        ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
        _ => None,
    }
}

/// The format and bytes a pasted picture is actually written as.
///
/// **A pasted picture exists to be read by a harness, so the format is not the board's to
/// choose.** The whole of the attachment is the `@path` the harness opens, and harness image
/// support is png/jpeg/gif/webp — a macOS screenshot arrives as TIFF and a Windows DIB as BMP,
/// and either lands as a path the harness cannot read at all. Those two are decoded here and
/// re-encoded as PNG, which is lossless in and lossless out, so nothing is given up for it.
///
/// A format with no decoder in this build, or bytes that will not decode, is written exactly as
/// it arrived under its own extension: a file the harness may decline is still better than a
/// `.png` that is not one. What that leaves unreadable is `G249`.
pub fn pasted_image_bytes(format: ImageFormat, bytes: Vec<u8>) -> (ImageFormat, Vec<u8>) {
    let Some(from) = transcodable_to_png(format) else {
        return (format, bytes);
    };
    match to_png(&bytes, from) {
        Some(png) => (ImageFormat::Png, png),
        None => (format, bytes),
    }
}

/// Decode with one named decoder and re-encode as PNG, or answer `None` and leave the caller
/// with what it had. Nothing here reports: a picture that will not decode is a picture written
/// under its own name, which is the fallback the caller already has.
fn to_png(bytes: &[u8], from: image::ImageFormat) -> Option<Vec<u8>> {
    let decoded = image::load_from_memory_with_format(bytes, from).ok()?;
    let mut png = Vec::new();
    decoded
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// The file extension for a clipboard image's format — what `viewer::image::format` reads back.
fn extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Ico => "ico",
        // `Pnm` never reaches here — `decodable` declines it — and a format added upstream is
        // written under a name the viewer will decline rather than one it would misread.
        _ => "bin",
    }
}

/// The byte a harness that reads the system clipboard itself waits for: `Ctrl+V`, `0x16`.
///
/// Claude Code takes an image by pressing `Ctrl+V` and reading the pasteboard on its own — the
/// image never crosses the wire as a path or as bytes, which is what lets this work with no
/// temp file, no new message and nothing local named in UI code.
const CTRL_V: u8 = 0x16;

/// What a focused terminal is sent for the platform paste chord, given what is on the board.
///
/// With an image and no text there is nothing to bracket-paste, and the chord means "give the
/// harness its image": `Ctrl+V` goes down the pane's own byte stream and the harness reads the
/// pasteboard itself. Anything else answers `None` and the emulator's own paste — a bracketed
/// paste of the clipboard's text — runs unchanged.
pub fn terminal_paste_bytes(keystroke: &gpui::Keystroke, has_image: bool) -> Option<Vec<u8>> {
    (has_image && gpui_terminal::input::is_paste_shortcut(keystroke)).then(|| vec![CTRL_V])
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

    /// An image on the board turns the terminal's paste chord into `Ctrl+V` for the harness,
    /// which is how Claude Code is asked to read the pasteboard itself.
    #[test]
    fn a_terminal_paste_of_an_image_sends_ctrl_v() {
        let paste = if cfg!(target_os = "macos") {
            "cmd-v"
        } else {
            "ctrl-shift-v"
        };
        let keystroke = gpui::Keystroke::parse(paste).unwrap();
        assert_eq!(terminal_paste_bytes(&keystroke, true), Some(vec![0x16]));
    }

    /// Text, or an empty board, is the emulator's own paste: this hands the chord back.
    #[test]
    fn a_terminal_paste_of_text_is_left_to_the_emulator() {
        let paste = if cfg!(target_os = "macos") {
            "cmd-v"
        } else {
            "ctrl-shift-v"
        };
        let keystroke = gpui::Keystroke::parse(paste).unwrap();
        assert_eq!(terminal_paste_bytes(&keystroke, false), None);
    }

    /// Every other key is untouched, image or not — including `Ctrl+V`, which already reaches
    /// the harness as `0x16` through the emulator's ordinary keystroke path.
    #[test]
    fn only_the_paste_chord_is_translated() {
        for key in ["ctrl-v", "cmd-c", "ctrl-c", "a"] {
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            assert_eq!(terminal_paste_bytes(&keystroke, true), None, "{key}");
        }
    }

    /// A format no harness reads is re-encoded on the way to disk, and the name follows the
    /// bytes. A BMP stands in for the Windows DIB; the macOS TIFF takes the same path.
    #[test]
    fn a_format_no_harness_reads_is_written_as_png() {
        let mut bmp = Vec::new();
        image::RgbaImage::new(2, 2)
            .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
            .expect("the build encodes bmp");

        let (format, bytes) = pasted_image_bytes(ImageFormat::Bmp, bmp.clone());
        assert_eq!(format, ImageFormat::Png);
        assert_ne!(bytes, bmp, "re-encoded rather than renamed");
        assert!(pasted_image_path(format, 1).ends_with(".png"));
        assert_eq!(
            image::guess_format(&bytes).ok(),
            Some(image::ImageFormat::Png),
            "the bytes really are a PNG"
        );
    }

    /// A format already readable, and one with no decoder in this build, both go down exactly as
    /// they arrived: a file the harness may decline beats a `.png` that is not one.
    #[test]
    fn anything_else_keeps_the_bytes_it_arrived_with() {
        for format in [ImageFormat::Png, ImageFormat::Gif, ImageFormat::Ico] {
            let arrived = vec![7u8, 7, 7];
            let (kept, bytes) = pasted_image_bytes(format, arrived.clone());
            assert_eq!(kept, format, "{format:?}");
            assert_eq!(bytes, arrived, "{format:?}");
        }
    }

    /// Bytes that will not decode are not thrown away — the name still matches what went down.
    #[test]
    fn bytes_that_will_not_decode_are_written_unchanged() {
        let junk = vec![0u8, 1, 2, 3];
        let (format, bytes) = pasted_image_bytes(ImageFormat::Tiff, junk.clone());
        assert_eq!(format, ImageFormat::Tiff);
        assert_eq!(bytes, junk);
        assert!(pasted_image_path(format, 1).ends_with(".tiff"));
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
