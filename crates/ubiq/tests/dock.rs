//! Where a panel may sit, and what a saved arrangement is keyed by.
//!
//! The tree, the drag and the serialisation are the component library's and are its own tests'
//! business. What is Ubiq's is the policy over them: which regions each kind of panel is allowed
//! in, where one dropped somewhere else goes back to, and the names a saved layout is rebuilt
//! from — which may never change, because they are the keys.

use ubiq::state::dock::{PanelKind, Region, Visibility};
use ubiq::state::editor::ViewLayout;
use ubiq::ui::dock::{file_from_payload, file_payload};
use ubiq_proto::ids::PaneId;

const REGIONS: [Region; 4] = [Region::Centre, Region::Left, Region::Right, Region::Bottom];

fn every_kind() -> Vec<PanelKind> {
    vec![
        PanelKind::Terminal(PaneId::generate()),
        PanelKind::Logs,
        PanelKind::Explorer,
        PanelKind::Chat,
        PanelKind::Centre,
        PanelKind::File("crates/ubiq/src/app.rs".to_string()),
    ]
}

/// The window doing nothing in particular: no project, no IDE, nothing open. Every test says only
/// the fields its rule turns on, so a rule that starts reading a new one shows up as a failure
/// rather than as a silent pass.
fn nothing() -> Visibility {
    Visibility::default()
}

/// The rule that keeps the explorer and the chat on a border: two homes, not four. An Edge panel
/// dragged over the centre or the bottom is refused and returns.
#[test]
fn an_edge_panel_lives_on_a_border_and_nowhere_else() {
    for kind in [PanelKind::Explorer, PanelKind::Chat] {
        assert!(kind.class().allows(Region::Left), "{kind:?} in the left");
        assert!(kind.class().allows(Region::Right), "{kind:?} in the right");
        assert!(
            !kind.class().allows(Region::Centre),
            "{kind:?} in the centre"
        );
        assert!(
            !kind.class().allows(Region::Bottom),
            "{kind:?} in the bottom"
        );
    }
}

/// A terminal and the console go where the user puts them, as long as it is somewhere a terminal
/// is worth reading: the centre or the bottom.
#[test]
fn a_free_panel_takes_the_centre_or_the_bottom() {
    for kind in [PanelKind::Terminal(PaneId::generate()), PanelKind::Logs] {
        assert!(kind.class().allows(Region::Centre));
        assert!(kind.class().allows(Region::Bottom));
        assert!(!kind.class().allows(Region::Left));
        assert!(!kind.class().allows(Region::Right));
    }
}

#[test]
fn a_centre_panel_takes_only_the_centre() {
    for kind in [PanelKind::Centre, PanelKind::File("justfile".to_string())] {
        assert!(kind.class().allows(Region::Centre), "{kind:?}");
        for region in [Region::Left, Region::Right, Region::Bottom] {
            assert!(!kind.class().allows(region), "{kind:?} in {region:?}");
        }
    }
}

/// A file panel is one open tab, and the tab is what it is keyed by: a file and its diff are two
/// tabs on one path, so a bare path could not tell them apart. Nothing else answers a key.
#[test]
fn a_file_panel_names_the_tab_it_draws() {
    let file = PanelKind::File("crates/ubiq/src/app.rs".to_string());
    let diff = PanelKind::File("diff:head:crates/ubiq/src/app.rs".to_string());

    assert_eq!(file.tab_key(), Some("crates/ubiq/src/app.rs"));
    assert_eq!(diff.tab_key(), Some("diff:head:crates/ubiq/src/app.rs"));
    assert_ne!(file, diff);
    assert_eq!(file.home(), Region::Centre);

    for kind in every_kind() {
        assert_eq!(
            kind.tab_key().is_some(),
            matches!(kind, PanelKind::File(_)),
            "{kind:?}"
        );
    }
}

/// Where a panel opens, and where one put back goes, has to satisfy its own policy — otherwise a
/// refused drop would be moved somewhere that refuses it again.
#[test]
fn every_kind_is_allowed_in_its_own_home() {
    for kind in every_kind() {
        assert!(
            kind.class().allows(kind.home()),
            "{kind:?} opens in {:?}, which its class forbids",
            kind.home()
        );
    }
}

/// Every kind has somewhere to go. A class that allowed nothing would be a panel that could be
/// dragged out of the window.
#[test]
fn every_kind_has_at_least_one_region() {
    for kind in every_kind() {
        assert!(
            REGIONS.iter().any(|region| kind.class().allows(*region)),
            "{kind:?} is allowed nowhere"
        );
    }
}

/// **A panel's name is permanent**: it is the key a saved layout is rebuilt from, so changing one
/// is losing every arrangement written before the change. This test is what makes that cost
/// visible at the moment somebody edits the string.
#[test]
fn the_names_a_saved_layout_is_keyed_by_are_fixed() {
    assert_eq!(
        PanelKind::Terminal(PaneId::generate()).name(),
        "ubiq.terminal"
    );
    assert_eq!(PanelKind::Logs.name(), "ubiq.logs");
    assert_eq!(PanelKind::Explorer.name(), "ubiq.explorer");
    assert_eq!(PanelKind::Chat.name(), "ubiq.chat");
    assert_eq!(PanelKind::Centre.name(), "ubiq.centre");
    assert_eq!(PanelKind::File("justfile".to_string()).name(), "ubiq.file");
}

/// **Every file panel answers the same name**, whichever tab it is. A name is a `&'static str` and
/// a tab key is not, so what tells one file panel from another is the payload beside it — which is
/// also why a saved layout cannot rebuild one from its name.
#[test]
fn a_file_panel_s_name_is_the_same_for_every_file() {
    let one = PanelKind::File("justfile".to_string());
    let two = PanelKind::File("diff:index:crates/ubiq/src/app.rs".to_string());
    assert_eq!(one.name(), two.name());
    assert_eq!(PanelKind::from_name("ubiq.file"), None);
}

/// Every panel a saved layout can carry is rebuilt from its name — except a terminal, which is
/// dropped on purpose: layout persists, harnesses do not.
#[test]
fn every_name_but_a_terminal_s_rebuilds() {
    for kind in every_kind() {
        match kind {
            // A terminal is dropped on purpose, and a file is rebuilt from its payload instead.
            PanelKind::Terminal(_) | PanelKind::File(_) => {
                assert_eq!(PanelKind::from_name(kind.name()), None, "{kind:?}")
            }
            kind => assert_eq!(PanelKind::from_name(kind.name()), Some(kind)),
        }
    }
    assert_eq!(
        PanelKind::from_name("ubiq.something-a-later-build-added"),
        None
    );
}

/// Closing a tab means killing a harness or closing a file. Every other panel is the window's own
/// furniture: it is hidden and brought back, never closed.
#[test]
fn only_a_terminal_s_and_a_file_s_tab_close() {
    for kind in every_kind() {
        let closes = kind.pane().is_some() || kind.tab_key().is_some();
        assert_eq!(kind.closable(), closes, "{kind:?}");
    }
}

/// The explorer and the chat leave with IDE mode; the chat also wants a project. A terminal is
/// hidden while its project is not the one on screen — hidden, so it keeps its place and its
/// harness keeps running. The console is always drawn.
#[test]
fn what_is_drawn_follows_the_mode_and_the_project() {
    let pane = PanelKind::Terminal(PaneId::generate());

    assert!(PanelKind::Explorer.is_drawn(Visibility {
        is_ide: true,
        ..nothing()
    }));
    assert!(!PanelKind::Explorer.is_drawn(Visibility {
        has_project: true,
        ..nothing()
    }));

    assert!(PanelKind::Chat.is_drawn(Visibility {
        is_ide: true,
        has_project: true,
        ..nothing()
    }));
    assert!(!PanelKind::Chat.is_drawn(Visibility {
        is_ide: true,
        ..nothing()
    }));
    assert!(!PanelKind::Chat.is_drawn(Visibility {
        has_project: true,
        ..nothing()
    }));

    assert!(pane.is_drawn(Visibility {
        pane_on_screen: true,
        ..nothing()
    }));
    assert!(!pane.is_drawn(Visibility {
        is_ide: true,
        has_project: true,
        ..nothing()
    }));

    for is_ide in [true, false] {
        for has_project in [true, false] {
            assert!(PanelKind::Logs.is_drawn(Visibility {
                is_ide,
                has_project,
                ..nothing()
            }));
        }
    }
}

/// **In IDE mode the open files are the centre.** A file panel is drawn while its tab is open, and
/// the centre panel — which in that mode is only the page saying no file is open — steps aside for
/// as long as one is.
///
/// It steps aside rather than leaving, which is the whole point of saying it this way: the
/// hidden-not-removed machinery is what brings it back where it was left when the last tab closes.
#[test]
fn the_centre_steps_aside_for_the_open_files_and_comes_back() {
    let ide = Visibility {
        is_ide: true,
        has_project: true,
        ..nothing()
    };

    // No file open: the centre is the page that says so, and there is no file panel to draw.
    assert!(PanelKind::Centre.is_drawn(ide));

    // One open: the file is the centre, and the centre panel is hidden behind it.
    let with_file = Visibility {
        file_open: true,
        any_file_open: true,
        ..ide
    };
    assert!(!PanelKind::Centre.is_drawn(with_file));
    assert!(PanelKind::File("justfile".to_string()).is_drawn(with_file));

    // The last tab closes and the centre is back, without having been rebuilt.
    assert!(PanelKind::Centre.is_drawn(ide));

    // Another mode's screen is the centre whatever the IDE's files are doing.
    let agents = Visibility {
        is_ide: false,
        has_project: true,
        file_open: true,
        any_file_open: true,
        ..nothing()
    };
    assert!(PanelKind::Centre.is_drawn(agents));
    assert!(!PanelKind::File("justfile".to_string()).is_drawn(agents));
}

/// A file panel whose tab has been closed is hidden rather than drawn — and one belonging to a
/// project this window has switched away from is too, which is the same fact: the tab is not among
/// the ones the project on screen holds.
#[test]
fn a_file_panel_is_drawn_only_while_its_tab_is_open() {
    let other_tabs_open = Visibility {
        is_ide: true,
        has_project: true,
        file_open: false,
        any_file_open: true,
        ..nothing()
    };
    assert!(!PanelKind::File("justfile".to_string()).is_drawn(other_tabs_open));
    // And the centre stays aside, because the project on screen still has files open.
    assert!(!PanelKind::Centre.is_drawn(other_tabs_open));
}

/// **A viewer's payload is what it is looking at, not what it drew.** The tab key and the layout
/// mode, and nothing else: a parsed scene, a computed diff and a rendered diagram are all functions
/// of bytes the host will send again, and none of them belongs in a saved arrangement.
///
/// The shape is pinned here as well as the round trip, because it is on disk in every user's
/// preferences: changing a field name is losing every arrangement written before the change, the
/// same cost a panel's name carries.
#[test]
fn a_file_panel_s_payload_is_the_tab_and_the_layout() {
    let payload = file_payload("diff:head:crates/ubiq/src/app.rs", ViewLayout::Split);
    assert_eq!(
        payload,
        serde_json::json!({
            "key": "diff:head:crates/ubiq/src/app.rs",
            "layout": "split",
        })
    );

    let (kind, layout) = file_from_payload(&payload).expect("rebuilds");
    assert_eq!(
        kind,
        PanelKind::File("diff:head:crates/ubiq/src/app.rs".to_string())
    );
    assert_eq!(layout, ViewLayout::Split);
}

/// Every layout survives the trip, so a document reopens as it was left rather than in whichever
/// mode happens to be the default.
#[test]
fn every_layout_survives_the_payload() {
    for layout in ViewLayout::all() {
        let payload = file_payload("README.md", layout);
        let (kind, back) = file_from_payload(&payload).expect("rebuilds");
        assert_eq!(kind, PanelKind::File("README.md".to_string()));
        assert_eq!(back, layout, "{layout:?}");
    }
}

/// A payload from a build that wrote something else. A missing layout is the viewer's default,
/// because which mode a document was left in is not worth losing the document over; a missing key
/// names no tab at all, so there is nothing to rebuild.
#[test]
fn a_payload_this_build_cannot_read_whole_is_not_half_read() {
    let (kind, layout) =
        file_from_payload(&serde_json::json!({ "key": "justfile" })).expect("rebuilds");
    assert_eq!(kind, PanelKind::File("justfile".to_string()));
    assert_eq!(layout, ViewLayout::default());

    let (_, layout) =
        file_from_payload(&serde_json::json!({ "key": "justfile", "layout": "kaleidoscope" }))
            .expect("rebuilds");
    assert_eq!(layout, ViewLayout::default());

    assert_eq!(file_from_payload(&serde_json::json!({})), None);
    assert_eq!(
        file_from_payload(&serde_json::json!({ "layout": "source" })),
        None
    );
}
