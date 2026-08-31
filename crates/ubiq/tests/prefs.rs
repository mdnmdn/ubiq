//! The view-state blob: the interface owns this schema, so the interface versions it.

use ubiq::state::RailMode;
use ubiq::state::prefs::{self, InterfacePrefs, ViewPrefs};
use ubiq::theme::ThemeId;

#[test]
fn a_blob_survives_the_round_trip() {
    let view = ViewPrefs {
        schema: prefs::SCHEMA,
        rail_mode: RailMode::Agents,
        show_left: false,
        show_bottom: true,
        show_right: false,
        explorer_width: Some(311.5),
        chat_width: Some(420.0),
        dock_height: None,
        open_files: Vec::new(),
        active_file: None,
        expanded: Vec::new(),
        selected: None,
    };

    let back: ViewPrefs = prefs::decode(&prefs::encode(&view)).expect("decodes");
    assert_eq!(back, view);
}

/// What the project was looking at travels in the same blob as where its furniture was: the files
/// that were open, which of them was in front, the folders that were expanded, and the row that was
/// selected.
#[test]
fn a_blob_carries_what_the_project_was_looking_at() {
    let view = ViewPrefs {
        open_files: vec!["justfile".to_string(), "crates/ubiq/src/app.rs".to_string()],
        active_file: Some("crates/ubiq/src/app.rs".to_string()),
        expanded: vec!["crates".to_string(), "crates/ubiq".to_string()],
        selected: Some("crates/ubiq/src/app.rs".to_string()),
        ..ViewPrefs::default()
    };

    let back: ViewPrefs = prefs::decode(&prefs::encode(&view)).expect("decodes");
    assert_eq!(back, view);
}

/// Every field added after the first release is defaulted, so a blob written before the file set
/// was remembered still opens — at the same schema, with its panel sizes and rail mode intact.
///
/// Bumping the schema instead would have discarded both for nothing.
#[test]
fn a_blob_written_before_the_file_set_still_opens() {
    let before = r#"{"schema":1,"rail_mode":"Ide","show_left":true,"show_bottom":true,
                     "show_right":false,"explorer_width":280.0,"chat_width":null,
                     "dock_height":220.5}"#;
    let view: ViewPrefs = prefs::decode(before).expect("decodes");

    assert_eq!(view.explorer_width, Some(280.0));
    assert_eq!(view.dock_height, Some(220.5));
    assert!(!view.show_right);

    assert!(view.open_files.is_empty());
    assert_eq!(view.active_file, None);
    assert!(view.expanded.is_empty());
    assert_eq!(view.selected, None);
}

#[test]
fn the_interface_blob_carries_the_palette() {
    let prefs_in = InterfacePrefs {
        schema: prefs::SCHEMA,
        theme: ThemeId::Light,
    };
    let back: InterfacePrefs = prefs::decode(&prefs::encode(&prefs_in)).expect("decodes");
    assert_eq!(back.theme, ThemeId::Light);
}

#[test]
fn a_blob_from_another_schema_is_discarded_whole() {
    // Not half-applied: the window opens on defaults instead.
    let newer =
        r#"{"schema":99,"rail_mode":"Ide","show_left":true,"show_bottom":true,"show_right":true}"#;
    assert!(prefs::decode::<ViewPrefs>(newer).is_none());

    let older =
        r#"{"schema":0,"rail_mode":"Ide","show_left":true,"show_bottom":true,"show_right":true}"#;
    assert!(prefs::decode::<ViewPrefs>(older).is_none());
}

#[test]
fn nonsense_is_discarded_rather_than_panicking() {
    assert!(prefs::decode::<ViewPrefs>("").is_none());
    assert!(prefs::decode::<ViewPrefs>("not json at all").is_none());
    assert!(prefs::decode::<ViewPrefs>("{}").is_none());
    // Right schema, wrong shape.
    assert!(prefs::decode::<ViewPrefs>(r#"{"schema":1}"#).is_none());
}

#[test]
fn the_optional_sizes_may_be_absent() {
    // A blob written before sizes were remembered still opens.
    let without =
        r#"{"schema":1,"rail_mode":"Ide","show_left":true,"show_bottom":false,"show_right":true}"#;
    let view: ViewPrefs = prefs::decode(without).expect("decodes");

    assert_eq!(view.explorer_width, None);
    assert!(!view.show_bottom);
}
