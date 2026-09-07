//! The capture editor's buffer: a scene over bytes, undone as elements, saved as PNG.
//!
//! All of it without a window. `state::image_edit` holds no frame and no focus handle, which
//! is what lets the edit stack, the crop and the flatten be asserted here rather than looked at.

use ubiq::state::editor::{OpenFile, ViewLayout};
use ubiq::state::image_edit::{ImageEdit, ImageTool, ShapeKind};

/// A flat PNG of one colour, the capture stand-in.
fn png(width: u32, height: u32, pixel: [u8; 4]) -> Vec<u8> {
    let picture = image::RgbaImage::from_pixel(width, height, image::Rgba(pixel));
    let mut out = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("an in-memory encode succeeds");
    out
}

fn dims(png: &[u8]) -> (u32, u32) {
    let decoded = image::load_from_memory(png).expect("the flatten is a PNG");
    (decoded.width(), decoded.height())
}

fn pixel(png: &[u8], x: u32, y: u32) -> [u8; 4] {
    image::load_from_memory(png)
        .expect("the flatten is a PNG")
        .to_rgba8()
        .get_pixel(x, y)
        .0
}

fn version() -> ubiq_proto::files::FileVersion {
    ubiq_proto::files::FileVersion {
        len: 0,
        modified: None,
    }
}

// --------------------------------------------------------------------------------------------
// The scene
// --------------------------------------------------------------------------------------------

/// Bytes become a scene with the picture under `capture`, at pixel size and dirty nowhere —
/// dirty is the tab's, not the buffer's.
#[test]
fn bytes_become_a_scene_over_the_picture() {
    let edit = ImageEdit::new(&png(12, 10, [255, 255, 255, 255]), None).expect("decodable");
    assert_eq!((edit.width, edit.height), (12, 10));
    assert_eq!(edit.scene.bounds.width(), 12.0);
    assert_eq!(edit.scene.bounds.height(), 10.0);
    assert_eq!(edit.scene.elements.len(), 1);
    assert!(matches!(
        edit.scene.elements[0].kind,
        ubiq::state::scene::ElementKind::Image { .. }
    ));
    assert!(edit.scene.files.contains_key("capture"));
}

/// Bytes with no picture in them are not a scene, and the caller keeps them read-only.
#[test]
fn garbage_is_not_a_scene() {
    assert!(ImageEdit::new(b"not a picture", None).is_none());
}

// --------------------------------------------------------------------------------------------
// The edit stack
// --------------------------------------------------------------------------------------------

/// An annotation is one element appended; undo removes it, redo restores it, and a new edit
/// clears the redo.
#[test]
fn annotations_are_elements_on_a_stack() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    assert!(edit.commit_shape(ShapeKind::Rect, (2.0, 2.0), (8.0, 8.0)));
    assert_eq!(edit.scene.elements.len(), 2);
    assert!(edit.undo());
    assert_eq!(edit.scene.elements.len(), 1);
    assert!(edit.redo());
    assert_eq!(edit.scene.elements.len(), 2);
    assert!(edit.undo());
    assert!(edit.commit_shape(ShapeKind::Ellipse, (1.0, 1.0), (5.0, 5.0)));
    assert!(!edit.redo(), "a new edit clears the redo");
}

/// A nothing-drag annotates nothing.
#[test]
fn a_point_is_not_a_rectangle() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    assert!(!edit.commit_shape(ShapeKind::Rect, (4.0, 4.0), (4.0, 4.0)));
    assert_eq!(edit.scene.elements.len(), 1);
}

/// Crop narrows the bounds, undo restores them, and a wider drag brings back what an earlier
/// crop took — the crop measures against the whole picture, never the current view.
#[test]
fn crop_is_undoable_and_recroppable() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    assert!(edit.set_crop(2.0, 2.0, 10.0, 10.0));
    assert_eq!(edit.scene.bounds.width(), 8.0);
    assert!(edit.undo());
    assert_eq!(edit.scene.bounds.width(), 20.0);
    assert!(edit.redo());
    assert!(edit.set_crop(0.0, 0.0, 18.0, 18.0));
    assert_eq!(edit.scene.bounds.width(), 18.0);
}

/// A crop dragged to nothing is ignored.
#[test]
fn a_crop_to_nothing_is_ignored() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    assert!(!edit.set_crop(5.0, 5.0, 5.0, 5.0));
    assert_eq!(edit.scene.bounds.width(), 20.0);
}

/// The topmost annotation wins the hit, and the base image never does — it is not moved,
/// recoloured or deleted.
#[test]
fn hit_testing_skips_the_base() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    assert!(edit.hit_test(10.0, 10.0).is_none(), "the base is not hit");
    edit.commit_shape(ShapeKind::Rect, (2.0, 2.0), (8.0, 8.0));
    let hit = edit.hit_test(5.0, 5.0).expect("the rectangle is hit");
    assert_ne!(hit, "capture");
    assert!(edit.hit_test(15.0, 15.0).is_none());
}

/// A move commits as one step and undoes as one.
#[test]
fn a_move_is_one_step() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    edit.commit_shape(ShapeKind::Rect, (2.0, 2.0), (8.0, 8.0));
    let id = edit.selected.clone().expect("the shape selects itself");
    edit.live_move(&id, 4.0, 0.0, (2.0, 2.0));
    edit.commit_move(&id, (2.0, 2.0));
    assert!(edit.hit_test(10.0, 5.0).is_some());
    assert!(edit.undo());
    assert!(edit.hit_test(10.0, 5.0).is_none());
    assert!(edit.hit_test(5.0, 5.0).is_some());
}

/// Delete removes the selection; undo brings it back.
#[test]
fn delete_and_its_undo() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    edit.commit_shape(ShapeKind::Rect, (2.0, 2.0), (8.0, 8.0));
    assert!(edit.delete_selected());
    assert_eq!(edit.scene.elements.len(), 1);
    assert!(edit.undo());
    assert_eq!(edit.scene.elements.len(), 2);
}

// --------------------------------------------------------------------------------------------
// The flatten
// --------------------------------------------------------------------------------------------

/// A save is a PNG at the bounds' size, with the annotations over the base pixels.
#[test]
fn flatten_draws_annotations_over_base_pixels() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    edit.fill = edit.stroke;
    assert!(edit.commit_shape(ShapeKind::Rect, (2.0, 2.0), (8.0, 8.0)));
    let flat = edit.flatten().expect("a flatten succeeds");
    assert_eq!(&flat[0..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(dims(&flat), (20, 20));
    let inside = pixel(&flat, 5, 5);
    assert_eq!(inside, [30, 30, 30, 255], "the default dark fill lands");
    let outside = pixel(&flat, 15, 15);
    assert_eq!(outside, [255, 255, 255, 255]);
}

/// The crop decides the output size.
#[test]
fn flatten_is_cropped_to_the_bounds() {
    let mut edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    assert!(edit.set_crop(4.0, 4.0, 14.0, 12.0));
    assert_eq!(dims(&edit.flatten().expect("a flatten succeeds")), (10, 8));
}

/// A region flattens to its own size.
#[test]
fn a_region_flattens_to_its_size() {
    let edit = ImageEdit::new(&png(20, 20, [255, 255, 255, 255]), None).expect("decodable");
    let region = edit
        .flatten_rect(2.0, 2.0, 6.0, 4.0)
        .expect("a region succeeds");
    assert_eq!(dims(&region), (6, 4));
}

/// Typed text survives the flatten as pixels, not as silence.
#[test]
fn text_flattens_to_pixels() {
    let mut edit = ImageEdit::new(&png(40, 30, [255, 255, 255, 255]), None).expect("decodable");
    assert!(edit.append_text("hi".to_string(), (2.0, 2.0)));
    let flat = edit.flatten().expect("a flatten succeeds");
    let decoded = image::load_from_memory(&flat).unwrap().to_rgba8();
    assert!(
        decoded
            .pixels()
            .any(|pixel| pixel.0 != [255, 255, 255, 255]),
        "the text drew something"
    );
}

// --------------------------------------------------------------------------------------------
// The tab around it
// --------------------------------------------------------------------------------------------

/// An untitled capture opens dirty and editable, and unsavable until it is written once.
#[test]
fn an_untitled_capture_is_dirty_and_editable() {
    let mut tab = OpenFile::untitled("capture-1.png", ViewLayout::Preview);
    tab.set_image_untitled(png(12, 10, [255, 255, 255, 255]));
    assert!(tab.dirty());
    assert!(tab.editable_image());
    assert!(!tab.savable(), "no version yet");
    assert!(tab.ensure_image_edit().is_some());
}

/// A PNG from the explorer stays read-only: never editable, never a scene.
#[test]
fn an_explorer_png_stays_read_only() {
    let mut tab = OpenFile::opening(
        "shots/a.png",
        ubiq::state::editor::Subject::File,
        ViewLayout::Preview,
    );
    tab.set_bytes(png(12, 10, [255, 255, 255, 255]));
    assert!(!tab.editable_image());
    assert!(tab.ensure_image_edit().is_none());
}

/// Bytes that never became a scene still upgrade on first use — the tab Phase 2 leaves behind.
#[test]
fn untitled_bytes_upgrade_on_first_use() {
    let mut tab = OpenFile::untitled("capture-2.png", ViewLayout::Preview);
    tab.set_bytes_untitled(png(12, 10, [255, 255, 255, 255]));
    assert!(tab.editable_image());
    let edit = tab.ensure_image_edit().expect("bytes upgrade");
    assert_eq!((edit.width, edit.height), (12, 10));
}

/// A written capture clears dirty, carries its version, and is savable from there on.
#[test]
fn a_written_capture_clears_dirty() {
    let mut tab = OpenFile::untitled("capture-1.png", ViewLayout::Preview);
    tab.set_image_untitled(png(12, 10, [255, 255, 255, 255]));
    tab.mark_saving(String::new());
    tab.saved(version(), "");
    assert!(!tab.dirty());
    assert!(tab.savable());
}

/// The toolbar's default is Select, and every tool has a label.
#[test]
fn every_tool_has_a_label() {
    assert_eq!(ImageTool::default(), ImageTool::Select);
    for tool in ImageTool::all() {
        assert!(!tool.label().is_empty());
    }
}
