//! An Excalidraw file turned into shapes: the rules a painter inherits and never re-decides.
//!
//! All of it without a frame. `state::scene` parses and orders; it does not draw, which is what
//! lets the whole vocabulary — colour, opacity, stack order, arrowheads, bounds, the Markdown
//! variant — be asserted here rather than looked at.
//!
//! The reference is `_tools/excalidraw.py`, which renders the same subset to SVG for the wireframes
//! under `_docs/design/`. Where a rule below looks arbitrary, that tool is where it came from.

use std::path::PathBuf;

use ubiq::state::scene::{ElementKind, Rgba8, Scene, SceneError, StrokeStyle, TextAlign};

/// A scene document with `elements` filled in, so a test says only what its own rule turns on.
fn scene(elements: &str) -> Scene {
    let json = format!(r#"{{"type":"excalidraw","version":2,"elements":[{elements}]}}"#);
    Scene::parse(json.as_bytes()).expect("a well-formed document parses")
}

/// One element of a type, with whatever extra fields the test cares about.
fn element(kind: &str, extra: &str) -> String {
    format!(r#"{{"id":"{kind}","type":"{kind}","x":0,"y":0,"width":10,"height":10{extra}}}"#)
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../_docs/design/_old")
        .join(name)
}

// --------------------------------------------------------------------------------------------
// Colour
// --------------------------------------------------------------------------------------------

/// `None` is transparent, and every way a file has of saying "no colour" arrives as `None` — an
/// unknown name included, because one exotic colour must not cost the scene.
#[test]
fn nothing_a_file_can_say_for_no_colour_becomes_a_colour() {
    for value in [
        "transparent",
        "none",
        "",
        "  ",
        "TRANSPARENT",
        "rebeccapurple",
        "#",
    ] {
        assert_eq!(Rgba8::parse(value), None, "{value:?}");
    }
}

/// Three, six and eight hex digits, with or without the `#` the file may have dropped.
#[test]
fn hex_is_read_at_every_length_and_with_or_without_its_hash() {
    assert_eq!(Rgba8::parse("#fff"), Some(Rgba8::opaque(255, 255, 255)));
    assert_eq!(Rgba8::parse("f00"), Some(Rgba8::opaque(255, 0, 0)));
    assert_eq!(Rgba8::parse("#1e1e1e"), Some(Rgba8::DEFAULT_STROKE));
    assert_eq!(Rgba8::parse("1e1e1e"), Some(Rgba8::DEFAULT_STROKE));
    assert_eq!(
        Rgba8::parse("#11223344"),
        Some(Rgba8 {
            r: 0x11,
            g: 0x22,
            b: 0x33,
            a: 0x44
        })
    );
    // A short digit doubles rather than shifting: `f` is `ff`, not `f0`.
    assert_eq!(Rgba8::parse("#369"), Some(Rgba8::opaque(0x33, 0x66, 0x99)));
    assert_eq!(Rgba8::parse("red"), Some(Rgba8::opaque(255, 0, 0)));
}

/// An absent `strokeColor` is Excalidraw's own `#1e1e1e`; an absent `backgroundColor` is
/// transparent; an explicit `"transparent"` beats the default rather than falling back to it.
#[test]
fn the_defaults_apply_to_an_absent_key_and_never_to_a_present_one() {
    let scene = scene(&format!(
        "{},{}",
        element("rectangle", ""),
        element(
            "ellipse",
            r##","strokeColor":"transparent","backgroundColor":"#00ff00""##
        ),
    ));

    assert_eq!(scene.elements[0].stroke, Some(Rgba8::DEFAULT_STROKE));
    assert_eq!(scene.elements[0].fill, None);
    assert_eq!(scene.elements[1].stroke, None);
    assert_eq!(scene.elements[1].fill, Some(Rgba8::opaque(0, 255, 0)));
}

/// The canvas colour comes off `appState`, and is transparent when the file names none — the
/// panel's own ground shows through rather than a guess.
#[test]
fn the_canvas_colour_comes_off_app_state() {
    let with =
        Scene::parse(br##"{"elements":[],"appState":{"viewBackgroundColor":"#ffffff"}}"##).unwrap();
    assert_eq!(with.background, Some(Rgba8::opaque(255, 255, 255)));

    let without = Scene::parse(br#"{"elements":[]}"#).unwrap();
    assert_eq!(without.background, None);
}

// --------------------------------------------------------------------------------------------
// The other defaults
// --------------------------------------------------------------------------------------------

/// The file writes `0` to `100`; the painter wants `0.0` to `1.0`, and an absent key is opaque.
#[test]
fn opacity_arrives_as_a_fraction() {
    let scene = scene(&format!(
        "{},{},{}",
        element("rectangle", r#","opacity":100"#),
        element("rectangle", r#","opacity":30"#),
        element("rectangle", ""),
    ));

    assert_eq!(scene.elements[0].opacity, 1.0);
    assert_eq!(scene.elements[1].opacity, 0.3);
    assert_eq!(scene.elements[2].opacity, 1.0);
}

/// Stroke width, font size, family and alignment all have a value before the file says anything.
#[test]
fn an_element_that_says_nothing_still_arrives_fully_specified() {
    let scene = scene(&format!(
        "{},{}",
        element("rectangle", ""),
        element("text", r#","text":"hello""#),
    ));

    assert_eq!(scene.elements[0].stroke_width, 2.0);
    assert_eq!(scene.elements[0].stroke_style, StrokeStyle::Solid);

    let ElementKind::Text {
        text,
        font_size,
        family,
        align,
    } = &scene.elements[1].kind
    else {
        panic!("the second element is text");
    };
    assert_eq!(text, "hello");
    assert_eq!(*font_size, 20.0);
    assert_eq!(*family, ubiq::state::scene::FontFamily::Normal);
    assert_eq!(*align, TextAlign::Left);
}

/// `roundness` is a whole object in the file, and all the painter needs from it is whether it is
/// there at all — a present `null` is not a rounded corner.
#[test]
fn roundness_is_a_yes_or_no() {
    let scene = scene(&format!(
        "{},{},{}",
        element("rectangle", r#","roundness":{"type":3}"#),
        element("rectangle", r#","roundness":null"#),
        element("rectangle", ""),
    ));

    let rounded = |i: usize| {
        matches!(
            scene.elements[i].kind,
            ElementKind::Rectangle { rounded: true }
        )
    };
    assert!(rounded(0));
    assert!(!rounded(1));
    assert!(!rounded(2));
}

// --------------------------------------------------------------------------------------------
// Paint order
// --------------------------------------------------------------------------------------------

/// **Order is by type, not by the file's own order or its `index`.** Frames sit under everything,
/// then shapes, then connectors, then text — so a scene drawn by walking the vec has its labels on
/// top whatever order the file listed them in.
#[test]
fn the_stack_is_ordered_by_type_and_not_by_the_file() {
    let scene = scene(&format!(
        "{},{},{},{}",
        element("text", r#","text":"label""#),
        element("arrow", ""),
        element("rectangle", ""),
        element("frame", ""),
    ));

    let types: Vec<&str> = scene.elements.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(types, ["frame", "rectangle", "arrow", "text"]);
}

/// Equal ranks keep the order the file wrote them in, which is the only thing left deciding which
/// of two overlapping boxes is on top.
#[test]
fn elements_of_equal_rank_keep_the_files_order() {
    let scene = scene(
        r#"{"id":"under","type":"rectangle"},{"id":"over","type":"ellipse"},{"id":"last","type":"diamond"}"#,
    );

    let ids: Vec<&str> = scene.elements.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["under", "over", "last"]);
}

// --------------------------------------------------------------------------------------------
// What is skipped
// --------------------------------------------------------------------------------------------

/// A deleted element is not in the scene at all — not hidden, not transparent, absent.
#[test]
fn a_deleted_element_is_not_in_the_scene() {
    let scene = scene(&format!(
        "{},{}",
        element("rectangle", r#","isDeleted":true"#),
        element("ellipse", ""),
    ));

    assert_eq!(scene.elements.len(), 1);
    assert_eq!(scene.elements[0].id, "ellipse");
}

/// **One unknown type is a missing shape, never a blank panel.** This is the failure rule the
/// proposal states, and it is asserted here because it is the one a future element type breaks.
#[test]
fn an_unknown_element_type_is_skipped_and_the_rest_of_the_scene_draws() {
    let scene = scene(&format!(
        "{},{},{}",
        element("rectangle", ""),
        element("embeddable", ""),
        element("ellipse", ""),
    ));

    let ids: Vec<&str> = scene.elements.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["rectangle", "ellipse"]);
}

// --------------------------------------------------------------------------------------------
// Connectors
// --------------------------------------------------------------------------------------------

/// Points are `[dx, dy]` from the element's own origin and stay that way — the painter adds `x, y`.
/// Storing them absolute here would be a second answer to where a shape is.
#[test]
fn connector_points_stay_relative_to_the_element() {
    let scene = scene(r#"{"id":"a","type":"line","x":100,"y":50,"points":[[0,0],[40,0],[40,25]]}"#);

    let element = &scene.elements[0];
    assert_eq!(element.x, 100.0);
    assert_eq!(element.y, 50.0);
    assert_eq!(
        element.kind.points(),
        [(0.0, 0.0), (40.0, 0.0), (40.0, 25.0)]
    );
}

/// The arrowhead default is the one thing the two connector types disagree on: **an `arrow` with no
/// `endArrowhead` key still has an arrowhead; a `line` does not.** An explicit `"none"` wins over
/// either default.
#[test]
fn an_arrow_points_by_default_and_a_line_does_not() {
    let scene = scene(&format!(
        "{},{},{},{}",
        element("arrow", ""),
        element("line", ""),
        element("arrow", r#","endArrowhead":"none""#),
        element(
            "line",
            r#","startArrowhead":"triangle","endArrowhead":"arrow""#
        ),
    ));

    let heads = |i: usize| match &scene.elements[i].kind {
        ElementKind::Arrow {
            start_arrow,
            end_arrow,
            ..
        }
        | ElementKind::Line {
            start_arrow,
            end_arrow,
            ..
        } => (*start_arrow, *end_arrow),
        other => panic!("{other:?} is not a connector"),
    };

    assert_eq!(heads(0), (false, true), "a bare arrow");
    assert_eq!(heads(1), (false, false), "a bare line");
    assert_eq!(heads(2), (false, false), "an arrow that declines its head");
    assert_eq!(heads(3), (true, true), "a line that asks for both");
}

// --------------------------------------------------------------------------------------------
// Bounds
// --------------------------------------------------------------------------------------------

/// The extent is the union of the element boxes, **unpadded** — the painter's margin is the
/// painter's business, and putting one here would apply it twice.
#[test]
fn the_bounds_are_the_union_of_the_boxes_with_no_margin() {
    let scene = scene(
        r#"{"id":"a","type":"rectangle","x":-20,"y":10,"width":50,"height":30},
           {"id":"b","type":"ellipse","x":100,"y":-5,"width":40,"height":80}"#,
    );

    assert_eq!(scene.bounds.min_x, -20.0);
    assert_eq!(scene.bounds.min_y, -5.0);
    assert_eq!(scene.bounds.max_x, 140.0);
    assert_eq!(scene.bounds.max_y, 75.0);
    assert_eq!(scene.bounds.width(), 160.0);
    assert_eq!(scene.bounds.height(), 80.0);
}

/// A scene with nothing in it has an empty extent rather than an infinite one, so the painter can
/// divide by it without checking.
#[test]
fn an_empty_scene_has_an_empty_extent() {
    let scene = Scene::parse(br#"{"elements":[]}"#).unwrap();
    assert!(scene.elements.is_empty());
    assert!(scene.bounds.is_empty());
}

// --------------------------------------------------------------------------------------------
// Embedded files
// --------------------------------------------------------------------------------------------

/// An image's bytes travel inside the file as a `data:` URI. They are decoded once, here, with the
/// mime type the URI declared — the painter is handed bytes, not a string to interpret.
#[test]
fn an_embedded_image_is_decoded_out_of_its_data_uri() {
    let scene = Scene::parse(
        br#"{"elements":[{"id":"i","type":"image","x":0,"y":0,"width":4,"height":4,"fileId":"pic"}],
             "files":{"pic":{"mimeType":"image/png","dataURL":"data:image/png;base64,aGVsbG8gd29ybGQ="}}}"#,
    )
    .unwrap();

    assert!(matches!(&scene.elements[0].kind, ElementKind::Image { file_id } if file_id == "pic"));
    let file = scene
        .files
        .get("pic")
        .expect("the file map carries the image");
    assert_eq!(file.mime, "image/png");
    assert_eq!(file.bytes, b"hello world");
}

/// A URI that is not base64 payload — a remote URL, say — is simply not a file this scene carries.
/// The image element stays; it has no bytes.
#[test]
fn a_file_that_is_not_inline_leaves_the_element_standing() {
    let scene = Scene::parse(
        br#"{"elements":[{"id":"i","type":"image","fileId":"pic"}],
             "files":{"pic":{"dataURL":"https://example.invalid/pic.png"}}}"#,
    )
    .unwrap();

    assert_eq!(scene.elements.len(), 1);
    assert!(scene.files.is_empty());
}

// --------------------------------------------------------------------------------------------
// The Markdown variant
// --------------------------------------------------------------------------------------------

/// Obsidian stores a scene inside a note. The drawing is the fence under `## Drawing`, and the
/// heading is what disambiguates it from any other JSON the prose happens to quote.
#[test]
fn a_markdown_note_yields_the_fence_under_its_drawing_heading() {
    let note = r#"---
excalidraw-plugin: parsed
---

# Excalidraw Data

## Text Elements
UBIQ ^mxPSyYNh

Some prose quoting an unrelated block:

```json
{"elements":[{"id":"decoy","type":"ellipse"}]}
```

## Drawing
```json
{"elements":[{"id":"real","type":"rectangle","x":0,"y":0,"width":8,"height":8}]}
```
"#;

    let scene = Scene::parse(note.as_bytes()).expect("the note holds a drawing");
    assert_eq!(scene.elements.len(), 1);
    assert_eq!(scene.elements[0].id, "real");
}

/// The other half of the variant: Obsidian's packed form. **Decompressing it is not implemented and
/// is not guessed at** — the viewer says the scene is stored compressed and stops.
#[test]
fn a_compressed_drawing_is_reported_rather_than_guessed_at() {
    let note = "## Drawing\n```compressed-json\nN4KAkARALgngDgUwgLgAQQQ\n```\n";
    assert_eq!(Scene::parse(note.as_bytes()), Err(SceneError::Compressed));
    assert_eq!(
        SceneError::Compressed.to_string(),
        "this scene is stored compressed"
    );
}

/// The real one, as `_docs/design/_old/` holds it.
#[test]
fn the_obsidian_fixture_is_reported_as_compressed() {
    let bytes = std::fs::read(fixture("scratchpad-features.excalidraw.md"))
        .expect("the fixture is in the tree");
    assert_eq!(Scene::parse(&bytes), Err(SceneError::Compressed));
}

/// Text that is neither JSON nor a note with a drawing in it is not a scene, and says so.
#[test]
fn something_that_is_not_a_scene_says_so() {
    assert_eq!(
        Scene::parse(b"# Just a document\n\nwith no drawing in it.\n"),
        Err(SceneError::NotAScene)
    );
    assert!(matches!(
        Scene::parse(b"{\"elements\": [,]}"),
        Err(SceneError::Json(_))
    ));
}

// --------------------------------------------------------------------------------------------
// The corpus
// --------------------------------------------------------------------------------------------

/// The wireframe the documentation is drawn from, parsed for real: this is the corpus the
/// reference renderer is tested against, and it is what keeps the subset honest.
#[test]
fn the_wireframe_fixture_parses_to_a_scene_worth_drawing() {
    let bytes = std::fs::read(fixture("wireframe.excalidraw")).expect("the fixture is in the tree");
    let scene = Scene::parse(&bytes).expect("the wireframe parses");

    assert!(
        scene.elements.len() > 20,
        "a real wireframe has plenty in it, got {}",
        scene.elements.len()
    );
    assert!(!scene.bounds.is_empty(), "and an extent to fit");

    // The ordering rule holds over a real file, not just a hand-built one.
    let ranks: Vec<u8> = scene.elements.iter().map(|e| e.kind.paint_rank()).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "the stack is ordered"
    );

    // And a real file exercises more than one type.
    assert!(
        scene
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Text { .. })),
        "a wireframe has labels"
    );
}
