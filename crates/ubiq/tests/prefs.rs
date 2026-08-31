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
    };

    let back: ViewPrefs = prefs::decode(&prefs::encode(&view)).expect("decodes");
    assert_eq!(back, view);
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
