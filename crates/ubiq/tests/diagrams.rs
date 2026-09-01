//! Mermaid, drawn in the interface: the picture, its size, and the disk tier in front of it.
//!
//! What these assert is what a viewer is handed — bytes, a width and a height it can draw the
//! picture at — and never how merman got there. **Nothing here needs a frame**: the renderer is a
//! plain synchronous function and the cache is a directory, which is exactly what lets the window
//! hand both to a background thread.

use tempfile::TempDir;
use ubiq::state::diagrams::{self, DiagramPalette, Disk};

const FLOWCHART: &str =
    "flowchart TD\n  A[Start] --> B{Ready?}\n  B -->|yes| C[Go]\n  B -->|no| A\n";

const SEQUENCE: &str = "sequenceDiagram\n  Alice->>Bob: Hello\n  Bob-->>Alice: Hi\n";

/// Not a diagram, and not prose either: a flowchart whose arrow is nonsense.
const BROKEN: &str = "flowchart TD\n  A[Start] -%%-> ((( B\n";

fn svg_of(image: &diagrams::DiagramImage) -> String {
    String::from_utf8(image.bytes.clone()).expect("the picture is text")
}

/// A throwaway workarea, as the host would hand one over: an absolute path to a directory that is
/// the interface's alone.
fn workarea(root: &TempDir) -> String {
    root.path().to_string_lossy().into_owned()
}

// ── the render ──────────────────────────────────────────────────────

#[test]
fn a_flowchart_renders_to_svg_sized_by_its_view_box() {
    let image = diagrams::render(FLOWCHART, DiagramPalette::Light).expect("the flowchart");
    let svg = svg_of(&image);

    assert!(svg.starts_with("<svg"), "{}", &svg[..40.min(svg.len())]);
    assert!(svg.contains("</svg>"));
    // The labels survive: `resvg_safe` resolves `<foreignObject>` into real text and then strips
    // it, so a renderer with no HTML in it still draws the words.
    assert!(svg.contains("Ready?"), "the labels went missing");
    assert!(
        !svg.contains("<foreignObject"),
        "a foreignObject reached a renderer that cannot draw one"
    );

    // The size is the viewBox's, and it is a diagram-shaped number rather than a default.
    assert!(image.width > 50.0 && image.width < 10_000.0, "{image:?}");
    assert!(image.height > 50.0 && image.height < 10_000.0, "{image:?}");
    assert_eq!(
        diagrams::view_box(&svg),
        Some((image.width, image.height)),
        "the size did not come from the viewBox"
    );
}

#[test]
fn a_sequence_diagram_renders_too() {
    let image = diagrams::render(SEQUENCE, DiagramPalette::Light).expect("the sequence");
    let svg = svg_of(&image);

    assert!(svg.contains("Alice") && svg.contains("Bob"));
    assert!(image.width > 50.0 && image.height > 50.0, "{image:?}");
}

#[test]
fn the_white_card_is_rewritten_to_the_surface_underneath() {
    // merman hard-codes `background-color: white` in the root style in both themes, which would be
    // a white card in the middle of a dark window.
    for palette in [DiagramPalette::Light, DiagramPalette::Dark] {
        let svg = svg_of(&diagrams::render(FLOWCHART, palette).expect("the flowchart"));
        let root = svg.split_once('>').expect("a root tag").0;
        let style = root.replace(' ', "");
        assert!(
            style.contains("background-color:transparent"),
            "{palette:?} kept {root}"
        );
        assert!(!root.contains("white"), "{palette:?} kept {root}");
        // Only that declaration: the rest of the style attribute is merman's and stays.
        assert!(style.contains("max-width:"), "{palette:?} lost {root}");
    }
}

#[test]
fn a_broken_source_answers_a_sentence_rather_than_panicking() {
    let reason = diagrams::render(BROKEN, DiagramPalette::Light).expect_err("nonsense");
    assert!(!reason.is_empty());
    // A sentence a person can read, not a debug dump: it goes on screen above the source.
    assert!(!reason.starts_with('{'), "{reason}");
}

#[test]
fn prose_with_no_diagram_in_it_is_a_failure_and_not_a_picture() {
    let reason = diagrams::render("just some words\n", DiagramPalette::Light)
        .expect_err("there is no diagram here");
    assert!(!reason.is_empty());
}

// ── the size, read off the markup ───────────────────────────────────

#[test]
fn the_view_box_is_its_last_two_numbers_and_nothing_else_is_a_size() {
    // The first two are the origin and are not the size.
    assert_eq!(
        diagrams::view_box("<svg viewBox=\"0 0 320 240\" width=\"100%\">"),
        Some((320.0, 240.0))
    );
    assert_eq!(
        diagrams::view_box("<svg viewBox='-8 -8 320 240'>"),
        Some((320.0, 240.0))
    );
    assert_eq!(
        diagrams::view_box("<svg viewBox=\"0,0,320,240\">"),
        Some((320.0, 240.0))
    );

    // merman's own `width="100%"` is never mistaken for one, and neither is a degenerate box.
    assert_eq!(diagrams::view_box("<svg width=\"100%\">"), None);
    assert_eq!(diagrams::view_box("<svg viewBox=\"0 0 0 240\">"), None);
    assert_eq!(diagrams::view_box("not markup at all"), None);
}

// ── what a picture is filed under ───────────────────────────────────

#[test]
fn the_two_palettes_are_two_keys_and_two_pictures() {
    let light = diagrams::key(FLOWCHART, DiagramPalette::Light);
    let dark = diagrams::key(FLOWCHART, DiagramPalette::Dark);
    assert_ne!(light, dark, "the palette is not in the key");
    // A file name, and a content address rather than anything readable.
    assert_eq!(light.len(), 64);
    assert!(light.chars().all(|c| c.is_ascii_hexdigit()));

    let in_light = diagrams::render(FLOWCHART, DiagramPalette::Light).unwrap();
    let in_dark = diagrams::render(FLOWCHART, DiagramPalette::Dark).unwrap();
    assert_ne!(
        in_light.bytes, in_dark.bytes,
        "the palette is not reaching the renderer"
    );
}

#[test]
fn two_sources_are_two_keys_and_one_source_is_one() {
    assert_ne!(
        diagrams::key(FLOWCHART, DiagramPalette::Light),
        diagrams::key(SEQUENCE, DiagramPalette::Light)
    );
    // The whole point of a content address: asked for twice, filed once.
    assert_eq!(
        diagrams::key(FLOWCHART, DiagramPalette::Light),
        diagrams::key(FLOWCHART, DiagramPalette::Light)
    );
}

// ── the disk tier, in the project's workarea ────────────────────────

#[test]
fn a_picture_is_written_into_the_workarea_and_read_back_whole() {
    let root = TempDir::new().unwrap();
    let dir = diagrams::cache_dir(&workarea(&root));

    let first = diagrams::resolve(FLOWCHART, DiagramPalette::Light, Some(dir.clone()));
    let first = first.result.expect("the flowchart");

    // A fresh cache over the same directory — a restart, as far as this tier is concerned.
    let second = Disk::new(dir).read(&diagrams::key(FLOWCHART, DiagramPalette::Light));
    let second = second.expect("nothing was written down");
    assert_eq!(first.bytes, second.bytes);
    assert_eq!((first.width, first.height), (second.width, second.height));
}

#[test]
fn the_directory_is_made_only_when_there_is_something_to_put_in_it() {
    let root = TempDir::new().unwrap();
    let dir = diagrams::cache_dir(&workarea(&root));
    assert!(!dir.exists(), "a window that drew nothing left a directory");

    diagrams::resolve(SEQUENCE, DiagramPalette::Dark, Some(dir.clone()))
        .result
        .expect("the sequence");
    assert!(dir.is_dir());
}

#[test]
fn a_cache_that_cannot_be_read_is_a_re_render_rather_than_an_error() {
    let root = TempDir::new().unwrap();
    let dir = diagrams::cache_dir(&workarea(&root));
    diagrams::resolve(FLOWCHART, DiagramPalette::Light, Some(dir.clone()))
        .result
        .unwrap();

    // Whatever was written down is now rubbish — a half-written file, or somebody else's. The next
    // ask still answers a picture, and one with a real size.
    for entry in std::fs::read_dir(&dir).unwrap() {
        std::fs::write(entry.unwrap().path(), b"<svg width=\"100%\"").unwrap();
    }
    let image = diagrams::resolve(FLOWCHART, DiagramPalette::Light, Some(dir))
        .result
        .expect("a re-render, not an error");
    assert!(image.width > 50.0);
}

#[test]
fn a_source_that_will_not_render_is_never_written_down() {
    let root = TempDir::new().unwrap();
    let dir = diagrams::cache_dir(&workarea(&root));

    // Tried again each time, because the next ask may follow the edit that fixed it.
    assert!(
        diagrams::resolve(BROKEN, DiagramPalette::Light, Some(dir.clone()))
            .result
            .is_err()
    );
    assert!(
        diagrams::resolve(BROKEN, DiagramPalette::Light, Some(dir.clone()))
            .result
            .is_err()
    );

    let written = std::fs::read_dir(&dir).map(Iterator::count).unwrap_or(0);
    assert_eq!(written, 0, "a failure was cached");
}

#[test]
fn a_render_answers_the_key_it_will_be_filed_under() {
    let answer = diagrams::resolve(SEQUENCE, DiagramPalette::Dark, None);
    assert_eq!(answer.key, diagrams::key(SEQUENCE, DiagramPalette::Dark));
    assert!(answer.result.is_ok());
}

#[test]
fn a_window_with_no_workarea_yet_still_draws() {
    // The frames before the catalogue has arrived: no disk tier, and a picture all the same.
    let image = diagrams::resolve(FLOWCHART, DiagramPalette::Light, None)
        .result
        .expect("the flowchart");
    assert!(image.width > 50.0 && image.height > 50.0);
}

#[test]
fn no_leftover_temp_file_survives_a_write() {
    let root = TempDir::new().unwrap();
    let dir = diagrams::cache_dir(&workarea(&root));
    diagrams::resolve(FLOWCHART, DiagramPalette::Light, Some(dir.clone()))
        .result
        .unwrap();

    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].ends_with(".svg"), "{names:?}");
    assert!(!names[0].starts_with('.'), "{names:?}");
}
