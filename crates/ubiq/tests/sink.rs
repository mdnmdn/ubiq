//! The kitchen sink: the fixtures it draws, and the rules the page holds them under.
//!
//! **Almost none of it needs a frame.** The sink's documents are constants and its state is a
//! handful of fields, which is exactly what lets the fixtures be checked against the parsers and the
//! renderer they will be handed to — a fixture that stopped parsing is a page that draws an error,
//! and a page that draws an error is not a test bench.
//!
//! What is asserted about the rail is the one thing the sink claims structurally: it sits under
//! `APP`, with the application's own destinations, and not under `PROJECT` with the ones that need
//! a folder open.
//!
//! The last test is the exception, and it is the one thing the others cannot reach: **every page
//! actually drawn, in a window with no graphics device and no project.** The style reference builds
//! more element ids than the rest of the window put together, and a collision or a panic in one of
//! them is only reachable by drawing it.

use ubiq::state::RailMode;
use ubiq::state::diagrams::{self, DiagramPalette};
use ubiq::state::editor::{FileLanguage, ViewLayout, ViewerKind};
use ubiq::state::scene::{ElementKind, Scene};
use ubiq::state::sink::{self, SinkSection, SinkState};

// ── the rail ────────────────────────────────────────────────────────

/// The sink is the application's, not a project's. A window with an empty catalogue can open it,
/// which is only true while it is grouped with the destinations that need nothing open.
#[test]
fn the_sink_is_an_app_destination_and_never_a_project_one() {
    let groups = RailMode::groups();

    let app = groups
        .iter()
        .find(|(label, _)| *label == "APP")
        .expect("the APP group");
    assert!(app.1.contains(&RailMode::Sink), "the sink left APP");

    for (label, modes) in groups {
        if *label != "APP" {
            assert!(
                !modes.contains(&RailMode::Sink),
                "the sink is also under {label}"
            );
        }
    }
}

// ── the pages ───────────────────────────────────────────────────────

/// Four pages hold a document and the fifth holds the style reference. A page with neither would
/// draw nothing at all.
#[test]
fn every_page_holds_a_document_or_is_the_style_reference() {
    for section in SinkSection::all() {
        match section {
            SinkSection::Style => assert!(section.doc().is_none(), "{section:?} holds a document"),
            _ => assert!(section.doc().is_some(), "{section:?} holds nothing"),
        }
    }
    assert_eq!(SinkSection::all().len(), 5);
}

/// The page and its viewer cannot disagree, because neither is written down twice: the document's
/// name carries an extension, and the extension is what picks the viewer.
#[test]
fn a_documents_name_picks_the_viewer_its_page_is_named_for() {
    let expected = [
        (SinkSection::Editor, ViewerKind::Editor, FileLanguage::Rust),
        (
            SinkSection::Markdown,
            ViewerKind::Markdown,
            FileLanguage::Markdown,
        ),
        (
            SinkSection::Mermaid,
            ViewerKind::Mermaid,
            FileLanguage::Plain,
        ),
        (
            SinkSection::Excalidraw,
            ViewerKind::Excalidraw,
            FileLanguage::Plain,
        ),
    ];

    for (section, viewer, language) in expected {
        let doc = section.doc().expect("a document");
        assert_eq!(doc.viewer(), viewer, "{} draws wrong", doc.name);
        assert_eq!(doc.language(), language, "{} highlights wrong", doc.name);
    }
}

/// A key is what the buffer, the layout and every element id are looked up by, so two documents
/// sharing one would be two documents in one buffer.
#[test]
fn every_document_has_a_key_of_its_own() {
    let keys: Vec<&str> = sink::docs().iter().map(|doc| doc.key).collect();
    let mut unique = keys.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(keys.len(), unique.len(), "{keys:?}");
}

// ── the fixtures ────────────────────────────────────────────────────

/// The Markdown fixture is there to exercise the fence renderers, which it only does while it
/// carries a fence of each kind.
#[test]
fn the_markdown_fixture_carries_a_fence_of_each_diagram_kind() {
    let source = SinkSection::Markdown.doc().expect("the document").source;

    assert!(source.contains("```mermaid"), "no Mermaid fence");
    assert!(source.contains("```excalidraw"), "no Excalidraw fence");
    // The things a Markdown viewer is worth testing against at all.
    assert!(source.contains("| Token group |"), "no table");
    assert!(source.contains("- [x]"), "no task list");
    assert!(source.contains("```rust"), "no code fence");
}

/// The scene fixture parses, and holds one element of each kind the painter draws — which is what
/// makes the page a test of the painter rather than a picture of one rectangle.
#[test]
fn the_scene_fixture_parses_into_every_kind_the_painter_draws() {
    let source = SinkSection::Excalidraw.doc().expect("the document").source;
    let scene = Scene::parse(source.as_bytes()).expect("the fixture parses");

    let mut seen = [false; 6];
    for element in &scene.elements {
        match element.kind {
            ElementKind::Frame { .. } => seen[0] = true,
            ElementKind::Rectangle { .. } => seen[1] = true,
            ElementKind::Ellipse => seen[2] = true,
            ElementKind::Diamond => seen[3] = true,
            ElementKind::Arrow { .. } | ElementKind::Line { .. } => seen[4] = true,
            ElementKind::Text { .. } => seen[5] = true,
            _ => {}
        }
    }
    assert_eq!(seen, [true; 6], "{:?}", scene.elements.len());

    // Paint order is by kind, and the fixture is written in a different order on purpose.
    let ranks: Vec<u8> = scene
        .elements
        .iter()
        .map(|element| element.kind.paint_rank())
        .collect();
    assert!(ranks.windows(2).all(|pair| pair[0] <= pair[1]), "{ranks:?}");
    assert!(!scene.bounds.is_empty());
}

/// The Mermaid fixture is a diagram rather than a plausible-looking string. The renderer is the
/// only thing that can say so.
#[test]
fn the_mermaid_fixture_is_a_diagram_the_renderer_draws() {
    let source = SinkSection::Mermaid.doc().expect("the document").source;
    let image = diagrams::render(source, DiagramPalette::Dark).expect("the fixture draws");

    assert!(image.width > 50.0 && image.height > 50.0, "{image:?}");

    let svg = String::from_utf8(image.bytes).expect("the picture is text");
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains("coordinator"), "the labels went missing");
}

/// The plain buffer's fixture is code, and long enough to have something to scroll and something to
/// fold. A one-liner would test neither.
#[test]
fn the_editor_fixture_is_a_file_worth_scrolling() {
    let source = SinkSection::Editor.doc().expect("the document").source;
    assert!(source.lines().count() > 40, "{}", source.lines().count());
    assert!(source.contains("pub fn resize"));
}

// ── the state ───────────────────────────────────────────────────────

/// A document opens on what its viewer has to show: the drawing where there is one, the source
/// where there is not. The same rule an open file follows.
#[test]
fn a_document_opens_on_what_its_viewer_has_to_show() {
    let sink = SinkState::default();
    for doc in sink::docs() {
        let expected = match doc.viewer().has_preview() {
            true => ViewLayout::Preview,
            false => ViewLayout::Source,
        };
        assert_eq!(sink.layout(doc), expected, "{}", doc.name);
    }
}

/// The layout toggle reaches only the documents that have something to toggle between. The plain
/// buffer keeps its source however hard it is asked.
#[test]
fn only_a_viewer_with_a_preview_can_be_put_into_one() {
    let mut sink = SinkState::default();

    let markdown = SinkSection::Markdown.doc().expect("the document");
    sink.set_layout(markdown, ViewLayout::Split);
    assert_eq!(sink.layout(markdown), ViewLayout::Split);

    let editor = SinkSection::Editor.doc().expect("the document");
    sink.set_layout(editor, ViewLayout::Preview);
    assert_eq!(sink.layout(editor), ViewLayout::Source);
}

/// One value drives the stepper, the meter and the ring, so it has to stay inside what all three
/// can report.
#[test]
fn the_level_stays_inside_what_a_meter_can_report() {
    let mut sink = SinkState::default();

    sink.nudge(1_000);
    assert_eq!(sink.level, 100);
    assert_eq!(sink.fraction(), 1.0);

    sink.nudge(-1_000);
    assert_eq!(sink.level, 0);
    assert_eq!(sink.fraction(), 0.0);
}

/// Each page's own note, because the strip prints it and an empty one would print nothing.
#[test]
fn every_page_says_what_it_is_for() {
    for section in SinkSection::all() {
        assert!(!section.label().is_empty(), "{section:?}");
        assert!(!section.note().is_empty(), "{section:?}");
    }
}

// ── drawn ───────────────────────────────────────────────────────────

/// Every page drawn, and every modal raised, in a window with no project.
///
/// This is the sink's own claim under test: it opens on an empty catalogue and asks the host for
/// nothing. What it guards is what only a frame can catch — a duplicate element id, a `.id()` a
/// scroll needs and does not have, a panel that reads the window from inside its own render — across
/// the page that draws more primitives than any other in the application.
#[gpui::test]
fn every_page_draws_in_a_window_with_no_project(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    use ubiq::app::{AppState, BusHub};
    use ubiq::state::sink::SinkModal;
    use ubiq::state::{MenuId, RailMode, WindowRegistry};

    // The host end is held for the test: a bus with no reader left is a different path from one
    // whose reader never answers, and nothing here needs an answer.
    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();

    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    state.update(cx, |state, cx| state.set_rail_mode(RailMode::Sink, cx));
    cx.run_until_parked();

    for section in SinkSection::all() {
        state.update(cx, |state, cx| state.set_sink_section(*section, cx));
        cx.run_until_parked();

        // Both halves of a document, where it has two: the source, and what the viewer drew.
        if let Some(doc) = section.doc() {
            for layout in ViewLayout::all() {
                state.update(cx, |state, cx| state.set_sink_layout(doc, layout, cx));
                cx.run_until_parked();
            }
        }
    }

    // The style reference's own controls, and each modal over the page that raised it.
    state.update(cx, |state, cx| {
        state.set_sink_section(SinkSection::Style, cx)
    });
    state.update(cx, |state, cx| {
        state.toggle_sink_facet(0, cx);
        state.set_sink_choice(2, cx);
        state.nudge_sink(-20, cx);
        state.toggle_sink_disclosure(cx);
        state.open_menu(MenuId::SinkPicker, cx);
    });
    cx.run_until_parked();

    state.update(cx, |state, cx| state.pick_sink_menu(3, cx));
    cx.run_until_parked();

    for modal in [SinkModal::Confirm, SinkModal::Form, SinkModal::Danger] {
        state.update(cx, |state, cx| state.open_sink_modal(modal, cx));
        cx.run_until_parked();
        state.update(cx, |state, cx| state.close_sink_modal(cx));
        cx.run_until_parked();
    }

    // The window is still there, and it is still on the page it was left on.
    state.read_with(cx, |state, _| {
        assert_eq!(state.workbench.rail_mode, RailMode::Sink);
        assert_eq!(state.sink.section, SinkSection::Style);
        assert_eq!(state.sink.picked, 3);
        assert_eq!(state.sink.level, 40);
        assert!(state.sink.modal.is_none());
    });
}
