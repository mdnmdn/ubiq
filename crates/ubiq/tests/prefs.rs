//! The view-state blob: the interface owns this schema, so the interface versions it.

use ubiq::state::RailMode;
use ubiq::state::editor::{Subject, from_tab_key, tab_key};
use ubiq::state::prefs::{self, InterfacePrefs, ModeLayout, ViewPrefs};
use ubiq::theme::ThemeId;
use ubiq_proto::files::DiffBase;

#[test]
fn a_blob_survives_the_round_trip() {
    let view = ViewPrefs {
        schema: prefs::SCHEMA,
        rail_mode: RailMode::Agents,
        modes: [(
            RailMode::Agents,
            ModeLayout {
                show_left: false,
                show_bottom: true,
                show_right: false,
                layout: Some(
                    serde_json::json!({"version": prefs::SCHEMA, "center": {"panel_name": "TabPanel"}}),
                ),
            },
        )]
        .into(),
        open_files: Vec::new(),
        active_file: None,
        expanded: Vec::new(),
        selected: None,
        file_filter: "main".to_string(),
        ui_font_size: Some(16.0),
        editor_wrap: Some(false),
    };

    let back: ViewPrefs = prefs::decode(&prefs::encode(&view)).expect("decodes");
    assert_eq!(back, view);
}

/// What the project was looking at travels in the same blob as where its furniture was: the tabs
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

/// **The open files are tab keys, not paths.** A file and its diff are two tabs on one path, so a
/// path could not say which of them was open — and an unprefixed key *is* the path, which is what
/// the file itself is remembered under.
///
/// The prefix is what a restore splits back out with `from_tab_key`, so a remembered diff reopens
/// as a diff rather than taking over the tab holding the file.
#[test]
fn the_tabs_a_project_remembers_are_keys_rather_than_paths() {
    let path = "crates/ubiq/src/app.rs";
    let view = ViewPrefs {
        open_files: vec![
            tab_key(path, Subject::File),
            tab_key(path, Subject::Diff(DiffBase::Head)),
            tab_key(path, Subject::Diff(DiffBase::Index)),
        ],
        active_file: Some(tab_key(path, Subject::Diff(DiffBase::Head))),
        ..ViewPrefs::default()
    };

    // Three tabs on one path, and the blob keeps them three.
    assert_eq!(view.open_files[0], path);
    assert_eq!(view.open_files.len(), 3);

    let back: ViewPrefs = prefs::decode(&prefs::encode(&view)).expect("decodes");
    assert_eq!(back, view);

    for key in &back.open_files {
        assert_eq!(from_tab_key(key).0, path, "{key} splits back to its path");
    }
    assert_eq!(
        from_tab_key(back.active_file.as_deref().expect("an active tab")).1,
        Subject::Diff(DiffBase::Head)
    );
}

/// Every field added after the first release is defaulted, so a blob written before the file set
/// was remembered — or before the window's arrangement was a per-mode record — still opens at the
/// same schema, with its rail mode intact and the fields it never carried empty.
///
/// The window's arrangement is remembered per rail mode, so a blob that never carried a mode's
/// flags opens with an empty `modes` map and each mode is arranged the way a fresh one is.
///
/// Bumping the schema for an *added* field would have discarded all of it for nothing. What does
/// move the schema is a field already being written changing meaning, which no default can rescue.
#[test]
fn a_blob_missing_the_fields_a_later_build_added_still_opens() {
    let before = format!(
        r#"{{"schema":{},"rail_mode":"Ide","show_left":true,"show_bottom":true,
             "show_right":false,"explorer_width":280.0,"chat_width":null,
             "dock_height":220.5}}"#,
        prefs::SCHEMA
    );
    let view: ViewPrefs = prefs::decode(&before).expect("decodes");

    assert_eq!(view.rail_mode, RailMode::Ide);
    // The flat show flags and the three sizes belong to a frame this build no longer has: they are
    // written into the per-mode record instead, so nothing here reads them and the window opens
    // each mode the way a fresh one does.
    assert!(view.modes.is_empty());
    let arranged = ModeLayout::default_for(RailMode::Ide);
    assert!(arranged.show_left && arranged.show_bottom && arranged.show_right);

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

/// The schema moved when a remembered file became a tab key rather than a path, when the saved
/// arrangement gained one panel per open file, and when `rail_mode: "Agents"` stopped naming the
/// graph and started naming the columns. All three are fields an older build already wrote, so a
/// blob from before a move is discarded whole rather than read as though it meant this.
#[test]
fn a_blob_from_a_previous_schema_is_discarded() {
    for schema in 1..prefs::SCHEMA {
        let before = format!(
            r#"{{"schema":{schema},"rail_mode":"Ide","show_left":true,"show_bottom":true,
                 "show_right":true,"open_files":["justfile"],"active_file":"justfile"}}"#
        );
        assert!(
            prefs::decode::<ViewPrefs>(&before).is_none(),
            "schema {schema} is not this build's and must be discarded"
        );
    }
}

/// The mode the graph screen answers to. `Agents` is still a name the blob carries and it now
/// names a different screen, which is the reason the schema moved — so both names have to survive
/// a round trip, or a window would come back on the wrong one.
#[test]
fn both_screens_over_the_work_survive_a_round_trip() {
    for mode in [RailMode::Agents, RailMode::Orchestration] {
        let out = ViewPrefs {
            schema: prefs::SCHEMA,
            rail_mode: mode,
            ..ViewPrefs::default()
        };
        let back: ViewPrefs = prefs::decode(&prefs::encode(&out)).expect("decodes");
        assert_eq!(back.rail_mode, mode);
    }
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

/// The dock's saved arrangement is versioned on the same number, so the blob and the arrangement
/// inside it are discarded together rather than one half at a time.
#[test]
fn the_arrangement_travels_on_the_blob_s_own_version() {
    assert_eq!(ubiq::ui::dock::LAYOUT_VERSION, prefs::SCHEMA as usize);
}

#[test]
fn nonsense_is_discarded_rather_than_panicking() {
    assert!(prefs::decode::<ViewPrefs>("").is_none());
    assert!(prefs::decode::<ViewPrefs>("not json at all").is_none());
    assert!(prefs::decode::<ViewPrefs>("{}").is_none());
    // Right schema, wrong shape.
    let bare = format!(r#"{{"schema":{}}}"#, prefs::SCHEMA);
    assert!(prefs::decode::<ViewPrefs>(&bare).is_none());
}

#[test]
fn the_arrangement_may_be_absent() {
    // A blob written before the window remembered one still opens, and the window arranges itself
    // the way a fresh one does: the mode has no record, so it falls to `default_for`.
    let without = format!(
        r#"{{"schema":{},"rail_mode":"Ide","open_files":[],"active_file":null}}"#,
        prefs::SCHEMA
    );
    let view: ViewPrefs = prefs::decode(&without).expect("decodes");

    assert!(view.modes.is_empty());
    let fresh = ModeLayout::default_for(RailMode::Ide);
    assert!(fresh.show_left && fresh.show_bottom && fresh.show_right);
}
