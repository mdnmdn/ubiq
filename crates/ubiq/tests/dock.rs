//! Where a panel may sit, and what a saved arrangement is keyed by.
//!
//! The tree, the drag and the serialisation are the component library's and are its own tests'
//! business. What is Ubiq's is the policy over them: which regions each kind of panel is allowed
//! in, where one dropped somewhere else goes back to, and the names a saved layout is rebuilt
//! from — which may never change, because they are the keys.

use ubiq::state::RailMode;
use ubiq::state::dock::{ChatId, PanelKind, Region, Visibility};
use ubiq::ui::dock::{
    chat_from_payload, chat_payload, file_from_payload, file_payload, pane_from_payload,
    pane_payload,
};
use ubiq_proto::ids::PaneId;

const REGIONS: [Region; 4] = [Region::Centre, Region::Left, Region::Right, Region::Bottom];

fn every_kind() -> Vec<PanelKind> {
    vec![
        PanelKind::Terminal(PaneId::generate()),
        PanelKind::Logs,
        PanelKind::Explorer,
        PanelKind::Chat(ChatId::generate()),
        PanelKind::Centre,
        PanelKind::File("crates/ubiq/src/app.rs".to_string()),
        PanelKind::GitRefs,
        PanelKind::GitChanges,
        PanelKind::GitHistory,
        PanelKind::GitDiff,
        PanelKind::KbExplorer,
        PanelKind::Kb("kb:01J0:notes.md".to_string()),
        PanelKind::Task,
        PanelKind::AgentsExplorer,
    ]
}

/// The window doing nothing in particular: no project, no IDE, nothing open. Every test says only
/// the fields its rule turns on, so a rule that starts reading a new one shows up as a failure
/// rather than as a silent pass.
fn nothing() -> Visibility {
    Visibility::default()
}

/// The rule that keeps the explorer on a border: one home, not four. An Edge panel dragged over
/// the centre or the bottom is refused and returns.
#[test]
fn an_edge_panel_lives_on_a_border_and_nowhere_else() {
    for kind in [
        PanelKind::Explorer,
        PanelKind::KbExplorer,
        PanelKind::Task,
        PanelKind::AgentsExplorer,
    ] {
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

/// A terminal, the console and a chat tab go wherever the user puts them: nothing about any of
/// the three is tied to a border or to the centre, so every region is fair game.
#[test]
fn a_free_panel_takes_any_region() {
    for kind in [
        PanelKind::Terminal(PaneId::generate()),
        PanelKind::Logs,
        PanelKind::Search,
        PanelKind::Chat(ChatId::generate()),
        PanelKind::GitRefs,
        PanelKind::GitChanges,
        PanelKind::GitHistory,
        PanelKind::GitDiff,
    ] {
        for region in REGIONS {
            assert!(kind.class().allows(region), "{kind:?} in {region:?}");
        }
    }
}

#[test]
fn a_centre_panel_takes_only_the_centre() {
    for kind in [
        PanelKind::Centre,
        PanelKind::File("justfile".to_string()),
        PanelKind::Kb("kb:01J0:notes.md".to_string()),
    ] {
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
    assert_eq!(PanelKind::Chat(ChatId::generate()).name(), "ubiq.chat");
    assert_eq!(PanelKind::Centre.name(), "ubiq.centre");
    assert_eq!(PanelKind::File("justfile".to_string()).name(), "ubiq.file");
    assert_eq!(PanelKind::GitRefs.name(), "ubiq.git.refs");
    assert_eq!(PanelKind::GitChanges.name(), "ubiq.git.changes");
    assert_eq!(PanelKind::GitHistory.name(), "ubiq.git.history");
    assert_eq!(PanelKind::GitDiff.name(), "ubiq.git.diff");
    assert_eq!(PanelKind::GitRefs.home(), Region::Left);
    assert_eq!(PanelKind::GitChanges.home(), Region::Right);
    assert_eq!(PanelKind::GitHistory.home(), Region::Centre);
    assert_eq!(PanelKind::GitDiff.home(), Region::Centre);
    assert_eq!(PanelKind::KbExplorer.name(), "ubiq.kb.explorer");
    assert_eq!(PanelKind::KbExplorer.home(), Region::Left);
    assert_eq!(
        PanelKind::Kb("kb:01J0:notes.md".to_string()).name(),
        "ubiq.kb.doc"
    );
    assert_eq!(
        PanelKind::Kb("kb:01J0:notes.md".to_string()).home(),
        Region::Centre
    );
    assert_eq!(PanelKind::Task.name(), "ubiq.task");
    assert_eq!(PanelKind::Task.home(), Region::Right);
    assert_eq!(PanelKind::AgentsExplorer.name(), "ubiq.agents.explorer");
    assert_eq!(PanelKind::AgentsExplorer.home(), Region::Left);
}

/// **The mode is part of the placement policy**, even though a chat answers it the same way in
/// every mode: a chat tab is the agents' side of the conversation and holds the right dock whatever
/// the mode, Teams included, where the graph and its own inspector take the centre and stay inline
/// rather than in the dock. Every other kind answers its one home whatever the mode.
#[test]
fn a_chat_homes_right_in_every_mode() {
    let chat = PanelKind::Chat(ChatId::generate());
    for mode in [
        RailMode::IDE,
        RailMode::TASKS,
        RailMode::GIT,
        RailMode::KB,
        RailMode::TEAMS,
        RailMode::TEAMS_ALL,
        RailMode::TEAMS_OLD,
    ] {
        assert_eq!(chat.home_in(mode), Region::Right, "{mode:?}");
        assert_eq!(PanelKind::chat_home(mode), Region::Right, "{mode:?}");
    }
    for kind in every_kind() {
        if kind.chat_id().is_some() {
            continue;
        }
        for mode in [RailMode::IDE, RailMode::TEAMS, RailMode::TASKS] {
            assert_eq!(kind.home_in(mode), kind.home(), "{kind:?} in {mode:?}");
        }
    }
}

/// The board's task and the agents list are their own mode's furniture, the way the knowledge
/// base's explorer is KB's: a project, and that mode, and nothing else draws either.
#[test]
fn the_board_s_task_and_the_agents_list_belong_to_their_modes() {
    let tasks = Visibility {
        has_project: true,
        rail_mode: Some(RailMode::TASKS),
        ..nothing()
    };
    let agents = Visibility {
        rail_mode: Some(RailMode::AGENTS),
        ..tasks
    };

    assert!(PanelKind::Task.is_drawn(tasks));
    assert!(!PanelKind::Task.is_drawn(agents));
    assert!(!PanelKind::Task.is_drawn(Visibility {
        rail_mode: Some(RailMode::TASKS),
        ..nothing()
    }));

    assert!(PanelKind::AgentsExplorer.is_drawn(agents));
    assert!(!PanelKind::AgentsExplorer.is_drawn(tasks));
    assert!(!PanelKind::AgentsExplorer.is_drawn(Visibility {
        rail_mode: Some(RailMode::AGENTS),
        ..nothing()
    }));

    // A conversation is furniture in three modes now, not one, and wants a project in each.
    let chat = PanelKind::Chat(ChatId::generate());
    assert!(chat.is_drawn(tasks));
    assert!(chat.is_drawn(Visibility {
        rail_mode: Some(RailMode::TEAMS),
        ..tasks
    }));
    assert!(!chat.is_drawn(agents));
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

/// Every panel a saved layout can carry is rebuilt from its name — except a terminal, a file and
/// a chat tab, each rebuilt from the payload beside it instead: a terminal because a saved one
/// names no pane this build can trust, a file and a chat tab because their name is the same for
/// every one of them.
#[test]
fn every_name_but_a_terminal_a_file_and_a_chat_rebuilds() {
    for kind in every_kind() {
        match kind {
            PanelKind::Terminal(_) | PanelKind::File(_) | PanelKind::Kb(_) | PanelKind::Chat(_) => {
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

/// Closing a tab means killing a harness, closing a file, closing a chat tab, putting the
/// console away, or dismissing a git panel. The explorer and the centre are the window's own
/// furniture: they are hidden and brought back, never closed.
#[test]
fn a_pane_a_file_a_chat_tab_and_the_console_close_and_nothing_else_does() {
    for kind in every_kind() {
        let closes = matches!(
            kind,
            PanelKind::Logs
                | PanelKind::GitRefs
                | PanelKind::GitChanges
                | PanelKind::GitHistory
                | PanelKind::GitDiff
        ) || kind.pane().is_some()
            || kind.tab_key().is_some()
            || kind.kb_key().is_some()
            || kind.chat_id().is_some();
        assert_eq!(kind.closable(), closes, "{kind:?}");
    }
}

/// Closing a chat tab is never refused for being the last one — there is no last-tab guard
/// anywhere in this tree, and a chat tab is no exception.
#[test]
fn a_chat_tab_is_closable_even_as_the_last_one() {
    assert!(PanelKind::Chat(ChatId::generate()).closable());
}

/// A terminal panel writes down which pane it draws, which is what lets a rebuilt arrangement put
/// a pane back where the user left it. It is still not rebuilt from its name — a pane exists
/// because the coordinator says so — so the payload is the whole of the claim.
#[test]
fn a_terminal_panel_writes_down_the_pane_it_draws() {
    let pane_id = PaneId::generate();
    let payload = pane_payload(pane_id);
    assert_eq!(
        pane_from_payload(&payload),
        Some(PanelKind::Terminal(pane_id))
    );

    // A payload from a build that wrote none, or one naming something that is not an id, names no
    // pane — and a leaf that names no pane is dropped rather than drawn empty.
    assert_eq!(pane_from_payload(&serde_json::json!({})), None);
    assert_eq!(
        pane_from_payload(&serde_json::json!({ "pane": "not-an-id" })),
        None
    );

    // The name is a constant because a saved leaf has to be recognised before there is an id to
    // build the kind with. It never changes: it is the key the rebuild reads.
    assert_eq!(PanelKind::TERMINAL, "ubiq.terminal");
    assert_eq!(PanelKind::Terminal(pane_id).name(), PanelKind::TERMINAL);
}

/// A chat tab's id round-trips through the dock's payload exactly the way a terminal's pane does
/// — carried in the payload rather than the name, because the name below is the same for every
/// chat tab there is.
#[test]
fn a_chat_tab_s_id_round_trips_through_the_payload() {
    let id = ChatId::generate();
    let payload = chat_payload(id);
    assert_eq!(chat_from_payload(&payload), Some(PanelKind::Chat(id)));

    // A payload from a build that wrote none, or one naming something that is not an id, names no
    // tab — and a leaf that names no tab is dropped rather than drawn empty.
    assert_eq!(chat_from_payload(&serde_json::json!({})), None);
    assert_eq!(
        chat_from_payload(&serde_json::json!({ "chat": "not-an-id" })),
        None
    );

    assert_eq!(PanelKind::CHAT, "ubiq.chat");
    assert_eq!(PanelKind::Chat(id).name(), PanelKind::CHAT);
}

/// The explorer leaves with IDE mode. **A chat is furniture on every screen about a project** —
/// IDE, Git, Kb, Tasks and Teams — and leaves only where the screen already is the conversation
/// (Agents) or is about no project at all (Control, the sink); it always wants a project. A
/// terminal is hidden while its project is not the one on screen — hidden, so it keeps its place
/// and its harness keeps running. The console is always drawn.
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

    let chat = PanelKind::Chat(ChatId::generate());
    // IDE is the one screen `settle_visibility` names with no mode at all — `rail_mode` is `None`
    // exactly when `is_ide` is set — so it is said that way here rather than as a mode.
    assert!(chat.is_drawn(Visibility {
        is_ide: true,
        has_project: true,
        ..nothing()
    }));
    assert!(!chat.is_drawn(Visibility {
        is_ide: true,
        ..nothing()
    }));
    for mode in [
        RailMode::GIT,
        RailMode::KB,
        RailMode::TASKS,
        RailMode::TEAMS,
    ] {
        assert!(
            chat.is_drawn(Visibility {
                has_project: true,
                rail_mode: Some(mode),
                ..nothing()
            }),
            "a chat is furniture in {mode:?}"
        );
    }
    for mode in [RailMode::AGENTS, RailMode::CONTROL, RailMode::SINK] {
        assert!(
            !chat.is_drawn(Visibility {
                has_project: true,
                rail_mode: Some(mode),
                ..nothing()
            }),
            "a chat is not drawn in {mode:?}: the columns are the conversation there, and Control \
             and the sink are about no project"
        );
    }
    // A conversation about nothing is a fiction, in every mode that would otherwise draw one.
    for mode in [
        RailMode::GIT,
        RailMode::KB,
        RailMode::TASKS,
        RailMode::TEAMS,
    ] {
        assert!(
            !chat.is_drawn(Visibility {
                rail_mode: Some(mode),
                ..nothing()
            }),
            "a chat wants a project in {mode:?}"
        );
    }

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

    let git = Visibility {
        has_project: true,
        rail_mode: Some(RailMode::GIT),
        ..nothing()
    };
    for kind in [
        PanelKind::GitRefs,
        PanelKind::GitChanges,
        PanelKind::GitHistory,
        PanelKind::GitDiff,
    ] {
        assert!(kind.is_drawn(git), "{kind:?} in Git");
        assert!(
            !kind.is_drawn(Visibility {
                is_ide: true,
                has_project: true,
                ..nothing()
            }),
            "{kind:?} in IDE"
        );
    }
    assert!(!PanelKind::Centre.is_drawn(git));
    assert!(PanelKind::Centre.is_drawn(Visibility {
        rail_mode: Some(RailMode::GIT),
        ..nothing()
    }));

    // The knowledge base's explorer wants its own mode and a project, and nothing else draws it.
    let kb = Visibility {
        has_project: true,
        rail_mode: Some(RailMode::KB),
        ..nothing()
    };
    assert!(PanelKind::KbExplorer.is_drawn(kb));
    assert!(!PanelKind::KbExplorer.is_drawn(Visibility {
        rail_mode: Some(RailMode::KB),
        ..nothing()
    }));
    assert!(!PanelKind::KbExplorer.is_drawn(git));
    // The centre stays in KB mode: it is the document the explorer selects.
    assert!(PanelKind::Centre.is_drawn(kb));
}

/// A panel that belongs to a rail mode rather than to the window is never put back into another
/// mode's tree by the leftover restore — it would open that mode's edges for a panel it hides.
/// The IDE's file explorer is one of them: it is that mode's left-hand furniture, drawn nowhere
/// else, so it travels in IDE's own blob and in nothing else's.
#[test]
fn a_mode_s_own_panels_say_so() {
    for kind in every_kind() {
        let owned = matches!(
            kind,
            PanelKind::Explorer
                | PanelKind::GitRefs
                | PanelKind::GitChanges
                | PanelKind::GitHistory
                | PanelKind::GitDiff
                | PanelKind::KbExplorer
                | PanelKind::Task
                | PanelKind::AgentsExplorer
        );
        assert_eq!(kind.is_mode_owned(), owned, "{kind:?}");
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

/// **A viewer's payload is what it is looking at, not what it drew.** The tab key and nothing
/// else: a parsed scene, a computed diff and a rendered diagram are all functions of bytes the
/// host will send again, and none of them belongs in a saved arrangement.
///
/// **The view mode is not in it either, since T-202.** The default view mode persists — that is
/// the `markdown_open` setting — and a per-document override is remembered in memory only, on
/// `OpenFile::layout`, for as long as the tab is open. A saved arrangement is on disk, so a mode
/// written here would be an override outliving the tab that chose it.
///
/// The shape is pinned here as well as the round trip, because it is on disk in every user's
/// preferences: changing a field name is losing every arrangement written before the change, the
/// same cost a panel's name carries.
#[test]
fn a_file_panel_s_payload_is_the_tab_and_nothing_else() {
    let payload = file_payload("diff:head:crates/ubiq/src/app.rs");
    assert_eq!(
        payload,
        serde_json::json!({ "key": "diff:head:crates/ubiq/src/app.rs" })
    );

    let kind = file_from_payload(&payload).expect("rebuilds");
    assert_eq!(
        kind,
        PanelKind::File("diff:head:crates/ubiq/src/app.rs".to_string())
    );
}

/// A payload from a build that wrote something else. A key is the whole payload, so a payload
/// with one rebuilds and a payload without one names no tab at all.
///
/// A `layout` left over from a build before T-202 is read straight past rather than honoured —
/// that is the upgrade path, and it is what makes an old arrangement open its documents in the
/// default instead of in whichever mode they were abandoned in.
#[test]
fn a_payload_this_build_cannot_read_whole_is_not_half_read() {
    assert_eq!(
        file_from_payload(&serde_json::json!({ "key": "justfile" })).expect("rebuilds"),
        PanelKind::File("justfile".to_string())
    );

    assert_eq!(
        file_from_payload(&serde_json::json!({ "key": "justfile", "layout": "source" }))
            .expect("rebuilds"),
        PanelKind::File("justfile".to_string()),
        "a pre-T-202 layout field is ignored, not a reason to drop the tab"
    );

    assert_eq!(file_from_payload(&serde_json::json!({})), None);
    assert_eq!(
        file_from_payload(&serde_json::json!({ "layout": "source" })),
        None
    );
}
