//! The knowledge base's app-level wiring: what a click on the explorer asks the host for, what a
//! stale reply is discarded rather than drawn, and what the settings dialog's folder chooser turns
//! into a `SetKbSources`.
//!
//! The state arithmetic itself — merging a listing, deciding what a click presses — is
//! `state::kb`'s own tests. What is tested here is the layer above it: that `AppState` asks the
//! bus exactly what the state answered with, and that a reply naming a path the user has since
//! clicked past never reaches the document on screen.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::sink::ProjectNav;
use ubiq::state::{
    FileBody, KbAction, KbKind, KbMenuRow, SaveState, WindowRegistry, kb_menu_entries, kb_tab_key,
};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::{DirEntry, DirListing, EntryKind, FileContents, FileError, HostDirEntry};
use ubiq_proto::ids::{KbSourceId, ProjectId, RepoQueryId};
use ubiq_proto::kb::{KbAccess, KbOrigin, KbSource, KbSourceState, KbSourceStatus, KbStore};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::repos::RepoSource;

const PATIENCE: Duration = Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
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

    /// Do something to the window with a window in hand, which is what a gesture always has.
    fn with<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut AppState, &mut gpui::Window, &mut gpui::Context<AppState>) -> R,
    ) -> R {
        let out = self
            .window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| f(state, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
        out
    }

    /// One source, ready, with no tree yet — the shape `KbSourcesListed` answers a fresh
    /// configuration with.
    fn seed_source(&self, name: &str, cx: &mut TestAppContext) -> KbSourceId {
        let id = KbSourceId::generate();
        self.deliver(
            Message::KbSourcesListed {
                project_id: self.project,
                sources: vec![KbSourceStatus {
                    source: KbSource {
                        id,
                        name: name.to_string(),
                        origin: KbOrigin::Folder {
                            path: format!("/{name}"),
                        },
                        filter: String::new(),
                        access: Default::default(),
                    },
                    state: KbSourceState::Ready,
                }],
            },
            cx,
        );
        id
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
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn dir(parent: &str, name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        rel_path: rel(parent, name),
        kind: EntryKind::Dir,
        size: None,
        symlink: false,
    }
}

fn file(parent: &str, name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        rel_path: rel(parent, name),
        kind: EntryKind::File,
        size: Some(1),
        symlink: false,
    }
}

fn rel(parent: &str, name: &str) -> String {
    match parent.is_empty() {
        true => name.to_string(),
        false => format!("{parent}/{name}"),
    }
}

fn listing(rel_path: &str, entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        rel_path: rel_path.to_string(),
        entries,
        truncated: false,
    }
}

fn trees_asked(said: &[Message]) -> Vec<(KbSourceId, String)> {
    said.iter()
        .filter_map(|message| match message {
            Message::KbTree {
                source, rel_path, ..
            } => Some((*source, rel_path.clone())),
            _ => None,
        })
        .collect()
}

/// A folder shut over an unlisted tree asks for its listing on the first click; once it has one,
/// neither collapsing nor reopening it asks again.
#[gpui::test]
fn clicking_a_folder_asks_for_its_listing_once(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let source = fixture.seed_source("docs", cx);
    fixture.deliver(
        Message::KbTreeListing {
            project_id: fixture.project,
            source,
            rel_path: String::new(),
            listings: vec![listing("", vec![dir("", "notes")])],
        },
        cx,
    );
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(source, "notes".to_string(), cx)
    });
    assert_eq!(
        trees_asked(&fixture.said()),
        vec![(source, "notes".to_string())],
        "a shut, unlisted folder asks once"
    );

    fixture.deliver(
        Message::KbTreeListing {
            project_id: fixture.project,
            source,
            rel_path: "notes".to_string(),
            listings: vec![listing("notes", vec![file("notes", "a.md")])],
        },
        cx,
    );

    // Shut, then open again: the listing is already held, so nothing is asked for twice.
    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(source, "notes".to_string(), cx)
    });
    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(source, "notes".to_string(), cx)
    });
    assert!(
        trees_asked(&fixture.said()).is_empty(),
        "a folder already listed is never asked for again"
    );
}

/// Two documents clicked in turn are two tabs, and each reply fills the tab it names — a reply
/// that arrives after the user has moved on is no longer discarded, because the document it
/// names is still open beside the one they moved to (`T-31`).
#[gpui::test]
fn two_documents_open_as_two_tabs_each_filled_by_its_own_reply(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let source = fixture.seed_source("docs", cx);
    fixture.deliver(
        Message::KbTreeListing {
            project_id: fixture.project,
            source,
            rel_path: String::new(),
            listings: vec![listing("", vec![file("", "a.md"), file("", "b.md")])],
        },
        cx,
    );

    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(source, "a.md".to_string(), cx)
    });
    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(source, "b.md".to_string(), cx)
    });

    // The reply for "a.md" lands after the user has already moved on to "b.md" — its own tab is
    // still open and is what it fills.
    fixture.deliver(
        Message::KbFileContents {
            project_id: fixture.project,
            source,
            rel_path: "a.md".to_string(),
            contents: FileContents {
                bytes: b"stale".to_vec(),
                len: 5,
                truncated: false,
                is_binary: false,
                version: None,
            },
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let kb = state.kb(cx).expect("a knowledge base");
        assert_eq!(kb.docs.len(), 2, "two clicks opened two tabs");
        assert!(
            matches!(
                kb.doc(&kb_tab_key(source, "b.md")).map(|doc| &doc.body),
                Some(FileBody::Loading)
            ),
            "a reply for another tab must not fill this one"
        );
        let _ = cx;
    });

    // The reply for "b.md" fills the other tab.
    fixture.deliver(
        Message::KbFileContents {
            project_id: fixture.project,
            source,
            rel_path: "b.md".to_string(),
            contents: FileContents {
                bytes: b"current".to_vec(),
                len: 7,
                truncated: false,
                is_binary: false,
                version: None,
            },
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let kb = state.kb(cx).expect("a knowledge base");
        for (path, text) in [("a.md", "stale"), ("b.md", "current")] {
            let doc = kb
                .doc(&kb_tab_key(source, path))
                .unwrap_or_else(|| panic!("{path} is open"));
            let buffer = doc
                .buffer()
                .unwrap_or_else(|| panic!("{path} has its bytes"));
            assert_eq!(buffer.read(cx).value(), text);
        }
    });
}

/// Every source list the window wrote, in order — `SetKbSources` is the one message any settings
/// edit sends, so this is the whole of what a gesture committed.
fn sources_written(said: &[Message]) -> Vec<Vec<KbSource>> {
    said.iter()
        .filter_map(|message| match message {
            Message::SetKbSources { sources, .. } => Some(sources.clone()),
            _ => None,
        })
        .collect()
}

/// Raise the project settings dialog and the "Add source" modal over it, and swallow whatever the
/// opening said — every test below is about what a *gesture* writes, not about the dialog opening.
fn open_form(fixture: &Fixture, cx: &mut TestAppContext) {
    fixture.with(cx, |state, _, cx| state.open_kb_settings(cx));
    fixture.with(cx, |state, window, cx| {
        state.open_kb_source_form(window, cx)
    });
    let _ = fixture.said();
}

/// The form is answered one kind at a time: choosing a different kind forgets what the previous
/// one filled in, and Confirm writes nothing until whichever kind is chosen has what it needs.
#[gpui::test]
fn a_kind_switch_clears_what_the_other_kind_filled(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    open_form(&fixture, cx);

    // Nothing chosen at all: Confirm is refused rather than writing an empty source.
    fixture.with(cx, |state, _, cx| state.confirm_kb_source(cx));
    assert!(
        sources_written(&fixture.said()).is_empty(),
        "a form with no kind chosen writes nothing"
    );

    fixture.with(cx, |state, window, cx| {
        state.pick_kb_source_kind(KbKind::Folder, window, cx)
    });
    fixture.with(cx, |state, window, cx| {
        state.accept_kb_source_folder("/srv/docs".to_string(), window, cx)
    });
    fixture.with(cx, |state, _, _| {
        let form = state.workbench.kb_source.as_ref().expect("the form is up");
        assert_eq!(form.name, "docs", "the name is seeded from the folder");
        assert!(form.is_answerable(), "a folder source needs only a folder");
    });

    // Switching to a repository forgets the folder — and a URL nobody has checked is not an
    // answer, so the form stops being answerable rather than carrying the folder over.
    fixture.with(cx, |state, window, cx| {
        state.pick_kb_source_kind(KbKind::Git, window, cx)
    });
    fixture.with(cx, |state, window, cx| {
        state.retype_kb_source_url("https://example.com/org/wiki.git".to_string(), window, cx)
    });
    fixture.with(cx, |state, _, _| {
        let form = state.workbench.kb_source.as_ref().expect("the form is up");
        assert!(form.path.is_none(), "the folder went with the kind");
        assert!(
            !form.is_answerable(),
            "a URL the host has never reached is not an answer"
        );
    });
    fixture.with(cx, |state, _, cx| state.confirm_kb_source(cx));
    assert!(
        sources_written(&fixture.said()).is_empty(),
        "Confirm is refused until the chosen kind has what it needs"
    );
}

/// Check is what makes a git source answerable: it asks the host for the repository's branches,
/// and an answer of any kind is what says the URL is real. The branch preselects on the host's
/// default, then `main`, then `master`.
#[gpui::test]
fn a_checked_url_is_what_makes_a_repository_answerable(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    open_form(&fixture, cx);
    fixture.with(cx, |state, window, cx| {
        state.pick_kb_source_kind(KbKind::Git, window, cx)
    });
    fixture.with(cx, |state, window, cx| {
        state.retype_kb_source_url("https://example.com/org/wiki.git".to_string(), window, cx)
    });
    fixture.with(cx, |state, _, cx| state.check_kb_source_url(cx));

    let query_id = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::ListRepoBranches {
                query_id,
                source: RepoSource::Url(url),
            } => {
                assert_eq!(url, "https://example.com/org/wiki.git");
                Some(query_id)
            }
            _ => None,
        })
        .expect("Check asks the host for the repository's branches");

    // An answer for a query nobody is waiting on changes nothing — the clone modal's own
    // discipline, kept here because both surfaces ask with the same message.
    fixture.deliver(
        Message::RepoBranches {
            query_id: RepoQueryId::generate(),
            branches: vec!["stale".to_string()],
            default: None,
        },
        cx,
    );
    fixture.with(cx, |state, _, _| {
        let form = state.workbench.kb_source.as_ref().expect("the form is up");
        assert!(form.branches.is_empty(), "a stale listing is discarded");
        assert!(!form.is_answerable());
    });

    fixture.deliver(
        Message::RepoBranches {
            query_id,
            branches: vec!["dev".to_string(), "main".to_string()],
            default: None,
        },
        cx,
    );
    fixture.with(cx, |state, _, _| {
        let form = state.workbench.kb_source.as_ref().expect("the form is up");
        assert_eq!(
            form.branch.as_deref(),
            Some("main"),
            "`main` is preferred where the host names no default"
        );
        assert_eq!(form.name, "wiki", "the name is seeded from the URL");
        assert!(form.is_answerable());
    });
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| state.confirm_kb_source(cx));
    let written = sources_written(&fixture.said());
    let sources = written.first().expect("Confirm writes the whole list");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0].origin,
        KbOrigin::Git {
            url: "https://example.com/org/wiki.git".to_string(),
            branch: Some("main".to_string()),
            store: KbStore::Internal,
        }
    );
    assert_eq!(
        sources[0].access,
        KbAccess::ReadOnly,
        "read-only by default"
    );
    fixture.with(cx, |state, _, _| {
        assert!(
            state.workbench.kb_source.is_none(),
            "a confirmed form takes itself down"
        );
    });
}

/// The name is seeded from whatever the form points at only until the user names it themselves.
/// After that, picking a folder must not rename what they wrote.
#[gpui::test]
fn the_name_stops_being_seeded_once_it_is_typed(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    open_form(&fixture, cx);
    fixture.with(cx, |state, window, cx| {
        state.pick_kb_source_kind(KbKind::Folder, window, cx)
    });
    fixture.with(cx, |state, _, cx| {
        state.retype_kb_source_name("Design notes".to_string(), cx)
    });
    fixture.with(cx, |state, window, cx| {
        state.accept_kb_source_folder("/srv/docs".to_string(), window, cx)
    });

    fixture.with(cx, |state, _, cx| state.confirm_kb_source(cx));
    let written = sources_written(&fixture.said());
    let sources = written.first().expect("Confirm writes the whole list");
    assert_eq!(
        sources[0].name, "Design notes",
        "a name the user typed is never seeded over"
    );
    assert_eq!(
        sources[0].origin,
        KbOrigin::Folder {
            path: "/srv/docs".to_string()
        }
    );
}

/// A wiki has nothing to point at and nothing to fetch, so it is answerable the moment it is
/// chosen — and it is always writable, whatever the access row was left showing.
#[gpui::test]
fn a_wiki_needs_nothing_but_its_kind_and_is_always_writable(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    open_form(&fixture, cx);
    fixture.with(cx, |state, window, cx| {
        state.pick_kb_source_kind(KbKind::Wiki, window, cx)
    });
    fixture.with(cx, |state, _, cx| {
        state.pick_kb_source_access(KbAccess::ReadOnly, cx)
    });
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| state.confirm_kb_source(cx));
    let written = sources_written(&fixture.said());
    let sources = written.first().expect("Confirm writes the whole list");
    assert_eq!(sources[0].origin, KbOrigin::Internal);
    assert_eq!(sources[0].name, "Wiki");
    assert_eq!(
        sources[0].access,
        KbAccess::ReadWrite,
        "an internal wiki is always writable"
    );
}

// ── The right-click menu ────────────────────────────────────────────────────

/// A source row offers what its origin and its access earn it, and nothing else. `Rename source`
/// is on every one of them: a source's name is Ubiq's own label for it, so a read-only source is
/// still renameable.
#[test]
fn a_source_row_offers_new_entries_only_where_it_is_writable() {
    assert_eq!(
        kb_menu_entries(KbMenuRow::Root, false, false),
        vec![
            KbAction::OpenInSystem,
            KbAction::CopyPath,
            KbAction::Separator,
            KbAction::RenameSource,
        ]
    );
    assert_eq!(
        kb_menu_entries(KbMenuRow::Root, true, false),
        vec![
            KbAction::OpenInSystem,
            KbAction::CopyPath,
            KbAction::Separator,
            KbAction::NewFile,
            KbAction::NewFolder,
            KbAction::Separator,
            KbAction::RenameSource,
        ]
    );
}

/// There is nothing to fetch for a folder or a wiki, and no state a user could reach that would
/// give it something — so the entry is absent rather than drawn dead.
#[test]
fn get_latest_version_is_absent_for_a_source_that_is_not_fetched() {
    for writable in [false, true] {
        assert!(
            !kb_menu_entries(KbMenuRow::Root, writable, false).contains(&KbAction::Sync),
            "a folder source has no latest version to get"
        );
        assert!(
            kb_menu_entries(KbMenuRow::Root, writable, true).contains(&KbAction::Sync),
            "a repository does"
        );
    }
    // And it is a source's row only: a folder inside a repository is not separately fetchable.
    assert!(!kb_menu_entries(KbMenuRow::Dir, true, true).contains(&KbAction::Sync));
    assert!(!kb_menu_entries(KbMenuRow::File, true, true).contains(&KbAction::Sync));
}

/// A folder takes New file, New folder, Rename and Delete when the source is writable; a file
/// takes Rename and Delete. Neither offers anything but the two path commands otherwise.
#[test]
fn a_folder_and_a_file_offer_what_the_sources_access_allows() {
    let read_only = vec![KbAction::OpenInSystem, KbAction::CopyPath];
    assert_eq!(kb_menu_entries(KbMenuRow::Dir, false, false), read_only);
    assert_eq!(kb_menu_entries(KbMenuRow::File, false, false), read_only);

    assert_eq!(
        kb_menu_entries(KbMenuRow::Dir, true, false),
        vec![
            KbAction::OpenInSystem,
            KbAction::CopyPath,
            KbAction::Separator,
            KbAction::NewFile,
            KbAction::NewFolder,
            KbAction::Separator,
            KbAction::Rename,
            KbAction::Delete,
        ]
    );
    assert_eq!(
        kb_menu_entries(KbMenuRow::File, true, false),
        vec![
            KbAction::OpenInSystem,
            KbAction::CopyPath,
            KbAction::Separator,
            KbAction::Rename,
            KbAction::Delete,
        ],
        "a file is not a folder to make anything in"
    );
}

/// `KbChanged` re-lists the directory it names — including a source's own top level, which has no
/// node of its own — and says nothing about a folder the tree has never opened.
#[gpui::test]
fn kb_changed_re_lists_the_directory_it_names(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let source = fixture.seed_source("docs", cx);
    fixture.deliver(
        Message::KbTreeListing {
            project_id: fixture.project,
            source,
            rel_path: String::new(),
            listings: vec![listing("", vec![dir("", "notes")])],
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(source, "notes".to_string(), cx)
    });
    fixture.deliver(
        Message::KbTreeListing {
            project_id: fixture.project,
            source,
            rel_path: "notes".to_string(),
            listings: vec![listing("notes", vec![file("notes", "a.md")])],
        },
        cx,
    );
    let _ = fixture.said();

    fixture.deliver(
        Message::KbChanged {
            project_id: fixture.project,
            source,
            rel_path: "notes".to_string(),
        },
        cx,
    );
    assert_eq!(
        trees_asked(&fixture.said()),
        vec![(source, "notes".to_string())],
        "the named folder is asked for again rather than guessed at"
    );

    fixture.deliver(
        Message::KbChanged {
            project_id: fixture.project,
            source,
            rel_path: String::new(),
        },
        cx,
    );
    assert_eq!(
        trees_asked(&fixture.said()),
        vec![(source, String::new())],
        "a source's own top level is a directory like any other"
    );

    fixture.deliver(
        Message::KbChanged {
            project_id: fixture.project,
            source,
            rel_path: "never-opened".to_string(),
        },
        cx,
    );
    assert!(
        trees_asked(&fixture.said()).is_empty(),
        "a folder the tree has never held has nothing to re-list"
    );
}

/// The knowledge base section of the project settings dialog opens like Tools and Remote do: it
/// hangs off an existing project's record, so an edit dialog answers to it and a create one does
/// not.
#[gpui::test]
fn the_settings_dialogs_kb_section_opens_on_an_existing_project(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, _, cx| state.open_edit_project(cx));
    fixture.with(cx, |state, _, cx| {
        state.set_sink_project_nav(ProjectNav::Kb, cx)
    });
    fixture.with(cx, |state, _, _| {
        assert_eq!(
            state
                .workbench
                .project_settings
                .as_ref()
                .map(|settings| settings.nav),
            Some(ProjectNav::Kb),
            "the KB row is one an edit dialog answers to"
        );
    });
}

/// The folder field is answered by Ubiq's own host-backed picker: the icon asks the host for a
/// listing, the answer fills the dialog, and the folder chosen there lands on the form rather than
/// being sent anywhere — the source is written when the form is confirmed.
#[gpui::test]
fn the_folder_chooser_browses_the_host_and_answers_the_form(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    open_form(&fixture, cx);
    fixture.with(cx, |state, window, cx| {
        state.pick_kb_source_kind(KbKind::Folder, window, cx)
    });
    let _ = fixture.said();

    fixture.with(cx, |state, window, cx| {
        state.browse_kb_source_folder(window, cx)
    });
    assert!(
        fixture
            .said()
            .iter()
            .any(|message| matches!(message, Message::BrowseHostDir { .. })),
        "the chooser asks the host the project runs on for a listing"
    );

    fixture.deliver(
        Message::HostDirListing {
            path: "/srv".to_string(),
            parent: Some("/".to_string()),
            entries: vec![HostDirEntry {
                name: "docs".to_string(),
                kind: EntryKind::Dir,
                hidden: false,
                readable: true,
            }],
            truncated: false,
        },
        cx,
    );
    fixture.with(cx, |state, window, cx| {
        let rows = state
            .file_picker
            .as_ref()
            .expect("the dialog is up")
            .rows()
            .len();
        assert_eq!(rows, 1, "the host's listing is what the dialog draws");
        state.click_picker_row("/srv/docs".to_string(), window, cx);
        state.commit_file_picker(window, cx);
    });
    fixture.with(cx, |state, _, _| {
        let form = state.workbench.kb_source.as_ref().expect("the form is up");
        assert_eq!(
            form.path.as_deref(),
            Some("/srv/docs"),
            "the folder lands on the form the chooser was raised from"
        );
    });
    assert!(
        sources_written(&fixture.said()).is_empty(),
        "a folder chosen is not a source written: Confirm is what writes one"
    );
}

/// A wiki source's file opens exactly the way a folder's or a repository's does: the click asks
/// for the same `ReadKbFile`, and the reply fills its tab's buffer — `KbOrigin::Internal`
/// is not a special case anywhere in this path (`T-23`).
#[gpui::test]
fn a_wiki_sources_file_opens_like_any_other(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = KbSourceId::generate();
    fixture.deliver(
        Message::KbSourcesListed {
            project_id: fixture.project,
            sources: vec![KbSourceStatus {
                source: KbSource {
                    id,
                    name: "Wiki".to_string(),
                    origin: KbOrigin::Internal,
                    filter: String::new(),
                    access: KbAccess::ReadWrite,
                },
                state: KbSourceState::Ready,
            }],
        },
        cx,
    );
    fixture.deliver(
        Message::KbTreeListing {
            project_id: fixture.project,
            source: id,
            rel_path: String::new(),
            listings: vec![listing("", vec![file("", "notes.md")])],
        },
        cx,
    );
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(id, "notes.md".to_string(), cx)
    });
    let said = fixture.said();
    assert!(
        said.iter().any(|m| matches!(
            m,
            Message::ReadKbFile { source, rel_path, .. }
                if *source == id && rel_path == "notes.md"
        )),
        "a click on a wiki file asks to read it, the same as any other source"
    );

    fixture.deliver(
        Message::KbFileContents {
            project_id: fixture.project,
            source: id,
            rel_path: "notes.md".to_string(),
            contents: FileContents {
                bytes: b"hello".to_vec(),
                len: 5,
                truncated: false,
                is_binary: false,
                version: None,
            },
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let doc = state
            .kb(cx)
            .unwrap()
            .doc(&kb_tab_key(id, "notes.md"))
            .expect("a document opened");
        assert_eq!(doc.path, "notes.md");
        assert_eq!(
            doc.buffer().expect("the bytes arrived").read(cx).value(),
            "hello",
            "the document drew the wiki file's contents"
        );
    });
}

/// One source list naming a read-write source and a read-only one, side by side — a single
/// `KbSourcesListed` neither test below has to repeat.
fn a_writable_and_a_read_only_source() -> (KbSourceId, KbSourceId, Vec<KbSourceStatus>) {
    let writable = KbSourceId::generate();
    let read_only = KbSourceId::generate();
    let sources = vec![
        KbSourceStatus {
            source: KbSource {
                id: writable,
                name: "rw".to_string(),
                origin: KbOrigin::Folder {
                    path: "/rw".to_string(),
                },
                filter: String::new(),
                access: KbAccess::ReadWrite,
            },
            state: KbSourceState::Ready,
        },
        KbSourceStatus {
            source: KbSource {
                id: read_only,
                name: "ro".to_string(),
                origin: KbOrigin::Folder {
                    path: "/ro".to_string(),
                },
                filter: String::new(),
                access: KbAccess::ReadOnly,
            },
            state: KbSourceState::Ready,
        },
    ];
    (writable, read_only, sources)
}

fn a_text_reply(project: ProjectId, source: KbSourceId, rel_path: &str, text: &str) -> Message {
    Message::KbFileContents {
        project_id: project,
        source,
        rel_path: rel_path.to_string(),
        contents: FileContents {
            bytes: text.as_bytes().to_vec(),
            len: text.len() as u64,
            truncated: false,
            is_binary: false,
            version: None,
        },
    }
}

/// Every document opens in the same editor, whatever its source's access says; what
/// `KbSource::is_writable` decides is whether a save is offered. `savable_kb_doc` is the one
/// place it is read — before `T-31` a read-only source's document got no buffer at all, which
/// cost it the IDE's highlighting to say something a refused save already says (`T-29`).
#[gpui::test]
fn every_document_opens_in_the_editor_and_only_a_writable_one_saves(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let (writable, read_only, sources) = a_writable_and_a_read_only_source();
    fixture.deliver(
        Message::KbSourcesListed {
            project_id: fixture.project,
            sources,
        },
        cx,
    );
    for source in [writable, read_only] {
        fixture.deliver(
            Message::KbTreeListing {
                project_id: fixture.project,
                source,
                rel_path: String::new(),
                listings: vec![listing("", vec![file("", "notes.md")])],
            },
            cx,
        );
    }

    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(writable, "notes.md".to_string(), cx)
    });
    fixture.deliver(
        a_text_reply(fixture.project, writable, "notes.md", "hello"),
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let key = kb_tab_key(writable, "notes.md");
        let doc = state.kb(cx).unwrap().doc(&key).expect("a document opened");
        assert_eq!(
            doc.buffer().expect("a buffer").read(cx).value(),
            "hello",
            "a read-write source's document opens in the editor"
        );
        assert!(state.savable_kb_doc(&key, cx), "and it can be written back");
    });

    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(read_only, "notes.md".to_string(), cx)
    });
    fixture.deliver(
        a_text_reply(fixture.project, read_only, "notes.md", "hello"),
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let key = kb_tab_key(read_only, "notes.md");
        let doc = state.kb(cx).unwrap().doc(&key).expect("a document opened");
        assert_eq!(
            doc.buffer().expect("a buffer").read(cx).value(),
            "hello",
            "a read-only source's document opens in the same editor"
        );
        assert!(
            !state.savable_kb_doc(&key, cx),
            "but it is never written back"
        );
    });
}

/// A dirty buffer over a writable source asks the host with `WriteKbFile`; a clean one asks for
/// nothing. A refusal leaves the buffer exactly as typed rather than discarding the document —
/// `KbFileError` otherwise reads as "the document is gone", which is right for a failed *read*
/// and wrong for a failed *write* — and `KbChanged` for the file's own directory is what confirms
/// the write and clears dirty (`T-29`).
#[gpui::test]
fn a_dirty_writable_document_saves_and_a_refusal_keeps_the_buffer(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let source = KbSourceId::generate();
    fixture.deliver(
        Message::KbSourcesListed {
            project_id: fixture.project,
            sources: vec![KbSourceStatus {
                source: KbSource {
                    id: source,
                    name: "rw".to_string(),
                    origin: KbOrigin::Folder {
                        path: "/rw".to_string(),
                    },
                    filter: String::new(),
                    access: KbAccess::ReadWrite,
                },
                state: KbSourceState::Ready,
            }],
        },
        cx,
    );
    fixture.deliver(
        Message::KbTreeListing {
            project_id: fixture.project,
            source,
            rel_path: String::new(),
            listings: vec![listing("", vec![file("", "notes.md")])],
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        state.click_kb_row(source, "notes.md".to_string(), cx)
    });
    fixture.deliver(
        a_text_reply(fixture.project, source, "notes.md", "hello"),
        cx,
    );
    let _ = fixture.said();

    let key = kb_tab_key(source, "notes.md");

    // Nothing typed yet: a save asks for nothing.
    fixture.with(cx, |state, _, cx| state.save_kb_doc(&key, cx));
    assert!(
        fixture.said().is_empty(),
        "a clean buffer has nothing to write"
    );

    // Marked dirty directly, on `an_unsaved_tab_is_asked_about_before_it_closes`'s own precedent
    // for the file editor: what is under test is the save wiring, not the widget's keystroke path.
    fixture.with(cx, |state, _, cx| {
        state
            .kb_mut(cx)
            .unwrap()
            .doc_mut(&key)
            .expect("open")
            .refresh_dirty("changed");
    });
    fixture.with(cx, |state, _, cx| state.save_kb_doc(&key, cx));
    let said = fixture.said();
    assert!(
        said.iter().any(|m| matches!(
            m,
            Message::WriteKbFile { source: s, rel_path, .. }
                if *s == source && rel_path == "notes.md"
        )),
        "a dirty buffer over a writable source is written back: {said:?}"
    );

    // The host refuses it. The document, and what was typed, must both survive: only the save's
    // own state is allowed to change.
    fixture.deliver(
        Message::KbFileError {
            project_id: fixture.project,
            source,
            rel_path: "notes.md".to_string(),
            error: FileError::Refused("this knowledge-base source is read-only".to_string()),
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let doc = state.kb(cx).unwrap().doc(&key).expect("still open");
        assert_eq!(
            doc.path, "notes.md",
            "a refused save must not discard the document"
        );
        assert!(
            doc.buffer().is_some(),
            "the buffer must survive a refused save"
        );
        assert!(matches!(&doc.save, SaveState::Failed(_)));
        assert!(doc.dirty(), "what was typed is still unsaved");
    });

    // Retried, and this time the host confirms with `KbChanged` for the file's own directory.
    fixture.with(cx, |state, _, cx| state.save_kb_doc(&key, cx));
    let _ = fixture.said();
    fixture.deliver(
        Message::KbChanged {
            project_id: fixture.project,
            source,
            rel_path: String::new(),
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let doc = state.kb(cx).unwrap().doc(&key).expect("still open");
        assert!(matches!(doc.save, SaveState::Idle));
        assert!(
            !doc.dirty(),
            "a `KbChanged` for the file's own directory confirms the save"
        );
    });
}
