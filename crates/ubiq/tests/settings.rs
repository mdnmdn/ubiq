//! The interface-owned settings blob: the interface versions it, so a schema it does not know is
//! discarded rather than half-applied.

use ubiq::state::editor::{OpenFile, ViewLayout, ViewerKind};
use ubiq::state::settings::{self, MarkdownOpen, UiSettings};

#[test]
fn a_blob_survives_the_round_trip() {
    let settings = UiSettings {
        schema: settings::SCHEMA,
        explorer_preview: false,
        markdown_open: MarkdownOpen::Source,
    };
    let back = settings::decode(&settings::encode(&settings)).expect("decodes");
    assert_eq!(back, settings);
}

#[test]
fn missing_fields_open_on_defaults() {
    let blob = r#"{"schema":1}"#;
    let back = settings::decode(blob).expect("decodes");
    assert!(back.explorer_preview);
    assert_eq!(back.markdown_open, MarkdownOpen::Preview);
}

#[test]
fn a_newer_schema_is_discarded() {
    let blob = r#"{"schema":99,"explorer_preview":false}"#;
    assert!(settings::decode(blob).is_none());
}

#[test]
fn unreadable_is_discarded() {
    assert!(settings::decode("not json").is_none());
}

#[test]
fn markdown_open_picks_the_layout_a_new_tab_starts_in() {
    let preview = OpenFile::opening(
        "README.md",
        ubiq::state::editor::Subject::File,
        ViewLayout::Preview,
    );
    assert_eq!(preview.viewer, ViewerKind::Markdown);
    assert_eq!(preview.layout, ViewLayout::Preview);

    let source = OpenFile::opening(
        "README.md",
        ubiq::state::editor::Subject::File,
        ViewLayout::Source,
    );
    assert_eq!(source.layout, ViewLayout::Source);

    // A mermaid file still opens in preview: the setting is markdown's.
    let mermaid = OpenFile::opening(
        "flow.mmd",
        ubiq::state::editor::Subject::File,
        ViewLayout::Source,
    );
    assert_eq!(mermaid.layout, ViewLayout::Preview);

    // Plain text has no preview.
    let rust = OpenFile::opening(
        "main.rs",
        ubiq::state::editor::Subject::File,
        ViewLayout::Preview,
    );
    assert_eq!(rust.layout, ViewLayout::Source);
}
