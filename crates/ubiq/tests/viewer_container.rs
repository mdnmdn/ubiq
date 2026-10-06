//! The viewer container (`D204`): a contributed viewer is a first-class editor in the IDE.
//!
//! What is pinned, with one viewer registered the way a second edition registers its own:
//!
//! 1. **A `Default` claim opens the path in it** — by path when the tab opens, and by the head of
//!    the text when the bytes arrive, where the extension left the tab on the editor.
//! 2. **An `Offer` is listed under "Open with" and opens nothing unasked** — in the explorer's
//!    right-click and in the editor top row's `⋯`.
//! 3. **A pick wins**: a viewer chosen from "Open with" is never re-decided by the head of the
//!    text.
//! 4. **The file stays the IDE's**: `reload_file_if_clean` re-reads a clean tab and leaves a dirty
//!    one, and a viewer that asked for `live_reload` has its on-screen tab re-read on a disk change.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::Utc;
use gpui::{AnyElement, AppContext as _, Entity, IntoElement, TestAppContext, WindowHandle, div};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::ext::SlotId;
use ubiq::ext::viewer::{self, Claim, ViewerAction, ViewerSpec};
use ubiq::state::editor::{FileLanguage, ViewLayout, ViewerKind};
use ubiq::state::explorer::{ExplorerAction, menu_entries};
use ubiq::state::{ViewerMenuRow, WindowRegistry};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::{DirEntry, DirListing, EntryKind, FileContents, FileVersion};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

const PATIENCE: Duration = Duration::from_millis(300);

/// A second edition's id: a string literal in its own code, never a base `const` (`D178`).
const DEMO: SlotId = SlotId::new("test.viewer.demo");

static PICKED: AtomicBool = AtomicBool::new(false);

/// `.vdemo` is the viewer's own; a `.json` is offered, and claimed outright when its head carries
/// the viewer's signature.
fn claims(path: &str, head: Option<&str>) -> Claim {
    if path.ends_with(".vdemo") {
        Claim::Default
    } else if path.ends_with(".json") {
        match head.is_some_and(|head| head.contains("\"vdemo\"")) {
            true => Claim::Default,
            false => Claim::Offer,
        }
    } else {
        Claim::No
    }
}

fn render(
    _: &viewer::ViewerCtx<'_>,
    _: &mut gpui::Window,
    _: &mut gpui::Context<AppState>,
) -> AnyElement {
    div().into_any_element()
}

/// Installed once per test binary: the container is resolved once per process.
fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut reg = viewer::base_registry();
        viewer::register(
            &mut reg,
            ViewerSpec {
                id: DEMO,
                label: "Demo",
                claims,
                layouts: &[ViewLayout::Preview, ViewLayout::Source],
                default_layout: ViewLayout::Preview,
                language: Some(FileLanguage::Json),
                live_reload: true,
                render,
                menu: Some(|_, _, _| {
                    vec![ViewerAction {
                        label: "Re-layout".into(),
                        enabled: true,
                        run: |_, _, _, _| PICKED.store(true, Ordering::SeqCst),
                    }]
                }),
                header_left: None,
                header_right: None,
            },
        );
        viewer::install(reg);
    });
}

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        install();
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });

        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            window,
            host,
            project,
        }
    }

    /// Everything the window has said so far, in order.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    fn deliver(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, message);
        cx.run_until_parked();
    }

    fn select(&self, path: &str, cx: &mut TestAppContext) {
        self.state
            .update(cx, |state, cx| state.select_file(path.to_string(), cx));
        cx.run_until_parked();
    }

    /// The bytes the host would have sent for one path.
    fn arrive(&self, path: &str, text: &str, cx: &mut TestAppContext) {
        self.deliver(
            Message::ProjectFileContents {
                project_id: self.project,
                rel_path: path.to_string(),
                contents: FileContents {
                    bytes: text.as_bytes().to_vec(),
                    len: text.len() as u64,
                    truncated: false,
                    is_binary: false,
                    version: Some(FileVersion {
                        len: text.len() as u64,
                        modified: None,
                    }),
                },
            },
            cx,
        );
    }

    fn view_of(&self, key: &str, cx: &mut TestAppContext) -> Option<(ViewerKind, ViewLayout)> {
        self.state.read_with(cx, |state, cx| {
            state.file(key, cx).map(|file| (file.viewer, file.layout))
        })
    }

    fn reads_of(&self, path: &str) -> usize {
        self.said()
            .into_iter()
            .filter(|message| {
                matches!(message, Message::ReadProjectFile { rel_path, .. } if rel_path == path)
            })
            .count()
    }
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: Default::default(),
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
            definitions_use_global: true,
            definitions_allowed: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

// ── 1. A default claim opens the path in it ─────────────────────────

#[test]
fn the_dispatch_consults_a_default_claim_where_the_extension_table_falls_through() {
    install();
    let demo = ViewerKind::Contributed(DEMO);
    assert_eq!(ViewerKind::of("diagram.vdemo"), demo);
    assert_eq!(demo.label(), "Demo");
    assert_eq!(demo.layouts(), &[ViewLayout::Preview, ViewLayout::Source]);
    assert!(demo.has_preview());
    assert!(demo.shows_buffer(ViewLayout::Source));
    assert!(!demo.shows_buffer(ViewLayout::Preview));
    assert_eq!(demo.forced_language(), Some(FileLanguage::Json));
    assert!(
        ViewerKind::all().contains(&demo),
        "the status bar's picker offers it"
    );

    // An offer opens nothing, and a built-in extension is never second-guessed.
    assert_eq!(ViewerKind::of("data.json"), ViewerKind::Editor);
    assert_eq!(ViewerKind::of("README.md"), ViewerKind::Markdown);
}

#[gpui::test]
fn a_claimed_path_opens_in_the_viewer_at_its_default_layout(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.select("diagram.vdemo", cx);
    assert_eq!(
        fixture.view_of("diagram.vdemo", cx),
        Some((ViewerKind::Contributed(DEMO), ViewLayout::Preview))
    );
    let language = fixture.state.read_with(cx, |state, cx| {
        state.file("diagram.vdemo", cx).map(|file| file.language)
    });
    assert_eq!(language, Some(FileLanguage::Json), "the spec's language");
}

#[gpui::test]
fn the_head_of_the_text_claims_a_tab_the_extension_left_on_the_editor(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.select("graph.json", cx);
    assert_eq!(
        fixture.view_of("graph.json", cx).map(|(viewer, _)| viewer),
        Some(ViewerKind::Editor),
        "with the path alone the viewer only offers itself"
    );
    fixture.arrive("graph.json", r#"{ "type": "vdemo" }"#, cx);
    assert_eq!(
        fixture.view_of("graph.json", cx),
        Some((ViewerKind::Contributed(DEMO), ViewLayout::Preview)),
        "the signature in the head makes it the default"
    );
}

// ── 2. An offer is listed under "Open with" ─────────────────────────

#[test]
fn an_offer_appears_in_the_explorer_open_with_list() {
    install();
    assert_eq!(
        ViewerKind::open_with("data.json", None),
        [ViewerKind::Editor, ViewerKind::Contributed(DEMO)]
    );
    let actions: Vec<_> = menu_entries(Some("data.json"), false, true, false, false)
        .into_iter()
        .map(|entry| entry.action)
        .collect();
    assert_eq!(
        &actions[..3],
        [
            ExplorerAction::Open,
            ExplorerAction::OpenWith,
            ExplorerAction::OpenDiff
        ]
    );

    // A file only the editor can draw keeps the menu it always had.
    let plain: Vec<_> = menu_entries(Some("src/main.rs"), false, true, false, false)
        .into_iter()
        .map(|entry| entry.action)
        .collect();
    assert!(!plain.contains(&ExplorerAction::OpenWith));
    assert_eq!(
        ViewerKind::open_with("src/main.rs", None),
        [ViewerKind::Editor]
    );
}

#[gpui::test]
fn the_explorer_open_with_stage_opens_the_file_in_the_picked_viewer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    // The tree has to hold the row: a path it has never listed gets an unreadable row's menu.
    fixture.deliver(
        Message::ProjectTreeListing {
            project_id: fixture.project,
            rel_path: String::new(),
            listings: vec![DirListing {
                rel_path: String::new(),
                entries: vec![DirEntry {
                    name: "data.json".to_string(),
                    rel_path: "data.json".to_string(),
                    kind: EntryKind::File,
                    size: Some(1),
                    symlink: false,
                }],
                truncated: false,
            }],
        },
        cx,
    );
    fixture.state.update(cx, |state, cx| {
        state.open_explorer_menu(Some("data.json".to_string()), (10.0, 10.0), cx)
    });
    let rows = |fixture: &Fixture, cx: &mut TestAppContext| {
        fixture.state.read_with(cx, |state, cx| {
            state
                .explorer(cx)
                .and_then(|explorer| explorer.menu.as_ref())
                .map(|menu| menu.entries())
                .unwrap_or_default()
        })
    };
    let first = rows(&fixture, cx);
    let open_with = first
        .iter()
        .position(|entry| entry.action == ExplorerAction::OpenWith)
        .expect("offered");
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.pick_explorer_action(open_with, window, cx)
            });
        })
        .expect("the window is open");
    let second = rows(&fixture, cx);
    assert_eq!(
        second.iter().map(|entry| entry.action).collect::<Vec<_>>(),
        [
            ExplorerAction::OpenWithViewer(ViewerKind::Editor),
            ExplorerAction::OpenWithViewer(ViewerKind::Contributed(DEMO)),
        ],
        "the same menu, on its second stage"
    );
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.pick_explorer_action(1, window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(
        fixture.view_of("data.json", cx),
        Some((ViewerKind::Contributed(DEMO), ViewLayout::Preview))
    );
}

#[gpui::test]
fn the_top_row_menu_offers_open_with_and_the_viewer_rows(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.select("diagram.vdemo", cx);
    fixture.arrive("diagram.vdemo", "{}", cx);
    let rows = fixture.state.read_with(cx, |state, cx| {
        state.viewer_menu_rows("diagram.vdemo", false, cx)
    });
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], ViewerMenuRow::OpenWith);
    assert_eq!(rows[1], ViewerMenuRow::Separator);
    assert!(
        matches!(&rows[2], ViewerMenuRow::Action { label, .. } if label.as_ref() == "Re-layout")
    );

    let stage = fixture.state.read_with(cx, |state, cx| {
        state.viewer_menu_rows("diagram.vdemo", true, cx)
    });
    assert_eq!(
        stage,
        [
            ViewerMenuRow::Viewer {
                kind: ViewerKind::Editor,
                current: false
            },
            ViewerMenuRow::Viewer {
                kind: ViewerKind::Contributed(DEMO),
                current: true
            },
        ]
    );

    // The viewer's own row runs its `fn` with the tab's key.
    fixture.state.update(cx, |state, cx| {
        state.open_viewer_more("diagram.vdemo", (0.0, 0.0), cx)
    });
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.pick_viewer_more(2, window, cx));
        })
        .expect("the window is open");
    assert!(PICKED.load(Ordering::SeqCst));
}

// ── 3. A pick wins ──────────────────────────────────────────────────

#[gpui::test]
fn a_picked_viewer_is_not_re_decided_by_the_head(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.select("other.json", cx);
    fixture.state.update(cx, |state, cx| {
        state.set_viewer_kind("other.json", ViewerKind::Editor, cx)
    });
    fixture.arrive("other.json", r#"{ "type": "vdemo" }"#, cx);
    assert_eq!(
        fixture.view_of("other.json", cx).map(|(viewer, _)| viewer),
        Some(ViewerKind::Editor),
        "the user chose the editor, and the signature does not take it back"
    );

    // And the reverse: picking the contributed viewer from "Open with" on a plain `.vdemo`
    // tab's editor and back.
    fixture.select("diagram.vdemo", cx);
    fixture.state.update(cx, |state, cx| {
        state.set_viewer_kind("diagram.vdemo", ViewerKind::Editor, cx)
    });
    assert_eq!(
        fixture
            .view_of("diagram.vdemo", cx)
            .map(|(viewer, _)| viewer),
        Some(ViewerKind::Editor)
    );
}

// ── 4. The file stays the IDE's ─────────────────────────────────────

#[gpui::test]
fn reload_file_if_clean_rereads_a_clean_tab_and_leaves_a_missing_one(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.select("diagram.vdemo", cx);
    fixture.arrive("diagram.vdemo", "{}", cx);
    fixture.said();

    let reloaded = fixture.state.update(cx, |state, cx| {
        state.reload_file_if_clean("diagram.vdemo", cx)
    });
    assert!(reloaded);
    assert_eq!(fixture.reads_of("diagram.vdemo"), 1);

    let absent = fixture.state.update(cx, |state, cx| {
        state.reload_file_if_clean("nowhere.vdemo", cx)
    });
    assert!(!absent, "a path with no tab is nobody's to reload");
}

#[gpui::test]
fn a_live_viewer_has_its_on_screen_tab_reread_on_a_disk_change(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.select("diagram.vdemo", cx);
    fixture.arrive("diagram.vdemo", "{}", cx);
    fixture.said();

    fixture.deliver(
        Message::ProjectFilesChanged {
            project_id: fixture.project,
            changed: vec!["diagram.vdemo".to_string()],
            truncated: false,
            repository: false,
        },
        cx,
    );
    assert_eq!(
        fixture.reads_of("diagram.vdemo"),
        1,
        "the tab on screen is re-read, because its viewer asked to follow the file"
    );
}
