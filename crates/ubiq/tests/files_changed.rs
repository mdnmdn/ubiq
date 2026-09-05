//! What a window does when the host says the disk moved under it.
//!
//! `ProjectFilesChanged` carries paths and never content, so everything the window does with it is
//! a fresh request: one listing per affected folder the tree already holds, a git refresh when the
//! repository's plumbing moved, and a re-read for every open tab with nothing unsaved in it. A
//! dirty tab is the one thing left alone — what has been typed into it is on disk nowhere.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::{DirEntry, DirListing, EntryKind, FileContents, FileVersion};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

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

    /// Open a tab and fill it with the bytes the host would have sent.
    fn open_file(&self, path: &str, text: &str, cx: &mut TestAppContext) {
        self.window
            .update(cx, |_, _window, cx| {
                self.state
                    .update(cx, |state, cx| state.select_file(path.to_string(), cx));
            })
            .expect("the window is open");
        cx.run_until_parked();
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
            no_local_index: false,
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn dir(name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        rel_path: name.to_string(),
        kind: EntryKind::Dir,
        size: None,
        symlink: false,
    }
}

fn file(parent: &str, name: &str) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        rel_path: format!("{parent}/{name}"),
        kind: EntryKind::File,
        size: Some(1),
        symlink: false,
    }
}

fn listing(rel_path: &str, entries: Vec<DirEntry>) -> DirListing {
    DirListing {
        rel_path: rel_path.to_string(),
        entries,
        truncated: false,
    }
}

fn listings_asked(said: &[Message]) -> Vec<String> {
    said.iter()
        .filter_map(|message| match message {
            Message::ProjectTree { rel_path, .. } => Some(rel_path.clone()),
            _ => None,
        })
        .collect()
}

fn reads_asked(said: &[Message]) -> Vec<String> {
    said.iter()
        .filter_map(|message| match message {
            Message::ReadProjectFile { rel_path, .. } => Some(rel_path.clone()),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn a_disk_change_relists_known_folders_and_rereads_only_clean_background_tabs(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::open(cx);

    // A root and one listed folder under it. `vendor` is named but never listed, so the tree does
    // not hold what is inside it.
    fixture.deliver(
        Message::ProjectTreeListing {
            project_id: fixture.project,
            rel_path: String::new(),
            listings: vec![listing("", vec![dir("src"), dir("vendor")])],
        },
        cx,
    );
    fixture.deliver(
        Message::ProjectTreeListing {
            project_id: fixture.project,
            rel_path: "src".to_string(),
            listings: vec![listing(
                "src",
                vec![
                    file("src", "main.rs"),
                    file("src", "lib.rs"),
                    file("src", "util.rs"),
                ],
            )],
        },
        cx,
    );

    // `main.rs` and `lib.rs` open first and end up in the background; `util.rs` opens last and is
    // the tab on screen.
    fixture.open_file("src/main.rs", "fn main() {}\n", cx);
    fixture.open_file("src/lib.rs", "pub fn one() {}\n", cx);
    fixture.open_file("src/util.rs", "pub fn two() {}\n", cx);

    // One background tab has unsaved edits in it, which is what the change must not touch.
    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture.state.update(cx, |state, cx| {
                let open = state.open_project_mut(cx).expect("the project is open");
                let tab = open
                    .editor
                    .find_mut("src/main.rs")
                    .expect("the tab is open");
                tab.refresh_dirty("fn main() { typed }\n");
                assert!(tab.dirty(), "the tab holds an unsaved edit");
            });
        })
        .expect("the window is open");

    // The other background tab's cursor is not at the top, which is where a careless reread would
    // put it.
    let moved_cursor = 5..5;
    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture.state.update(cx, |state, cx| {
                let open = state.open_project_mut(cx).expect("the project is open");
                let buffer = open
                    .editor
                    .find_mut("src/lib.rs")
                    .expect("the tab is open")
                    .buffer()
                    .expect("the tab has a buffer")
                    .clone();
                buffer.update(cx, |buffer, cx| {
                    buffer.set_selected_range(moved_cursor.clone(), cx);
                });
            });
        })
        .expect("the window is open");

    let _ = fixture.said();

    fixture.deliver(
        Message::ProjectFilesChanged {
            project_id: fixture.project,
            changed: vec![
                "src/main.rs".to_string(),
                "src/lib.rs".to_string(),
                "src/util.rs".to_string(),
                "vendor/thing.rs".to_string(),
            ],
            truncated: false,
            repository: true,
        },
        cx,
    );

    let said = fixture.said();
    let listings = listings_asked(&said);
    assert_eq!(
        listings.iter().filter(|path| *path == "src").count(),
        1,
        "three changes in one folder are one listing: {said:?}"
    );
    assert!(
        !listings.iter().any(|path| path == "vendor"),
        "a folder the tree has not listed is not asked about: {said:?}"
    );

    let reads = reads_asked(&said);
    assert_eq!(
        reads,
        vec!["src/lib.rs".to_string()],
        "the clean background tab is read again; the dirty background tab and the tab on \
         screen are both left alone: {said:?}"
    );
    assert!(
        said.iter()
            .any(|message| matches!(message, Message::RefreshProjectGit { .. })),
        "the repository moved, so the git overview is asked for again: {said:?}"
    );

    // The reread's answer rebuilds the buffer; the cursor it lands with is the one the old
    // buffer had, not the top of the file.
    let text = "pub fn one() {}\n";
    fixture.deliver(
        Message::ProjectFileContents {
            project_id: fixture.project,
            rel_path: "src/lib.rs".to_string(),
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

    fixture
        .window
        .update(cx, |_, _window, cx| {
            fixture.state.update(cx, |state, cx| {
                let open = state.open_project_mut(cx).expect("the project is open");
                let buffer = open
                    .editor
                    .find_mut("src/lib.rs")
                    .expect("the tab is open")
                    .buffer()
                    .expect("the reread filled a fresh buffer")
                    .clone();
                assert_eq!(
                    buffer.read(cx).selected_range(),
                    moved_cursor,
                    "the fresh buffer keeps the cursor the old one had, not the top of the file"
                );
            });
        })
        .expect("the window is open");
}

#[gpui::test]
fn a_change_this_window_just_wrote_is_not_reread(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture.deliver(
        Message::ProjectTreeListing {
            project_id: fixture.project,
            rel_path: String::new(),
            listings: vec![listing("", vec![dir("src")])],
        },
        cx,
    );
    fixture.deliver(
        Message::ProjectTreeListing {
            project_id: fixture.project,
            rel_path: "src".to_string(),
            listings: vec![listing(
                "src",
                vec![
                    file("src", "main.rs"),
                    file("src", "lib.rs"),
                    file("src", "extra.rs"),
                ],
            )],
        },
        cx,
    );

    // Two background tabs, both clean, so a careless handler would reread both. A third stays on
    // screen, which is where the tab-on-screen exclusion is not what this test is about.
    fixture.open_file("src/main.rs", "fn main() {}\n", cx);
    fixture.open_file("src/lib.rs", "pub fn one() {}\n", cx);
    fixture.open_file("src/extra.rs", "pub fn extra() {}\n", cx);

    // `main.rs` is the one this window just wrote — the host's acknowledgement of a save this
    // window made, not something typed elsewhere.
    fixture.deliver(
        Message::ProjectFileWritten {
            project_id: fixture.project,
            rel_path: "src/main.rs".to_string(),
            version: FileVersion {
                len: 13,
                modified: None,
            },
        },
        cx,
    );

    let _ = fixture.said();

    // The watcher echoes that same write back, bundled with a real external change to the other
    // file.
    fixture.deliver(
        Message::ProjectFilesChanged {
            project_id: fixture.project,
            changed: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            truncated: false,
            repository: false,
        },
        cx,
    );

    let reads = reads_asked(&fixture.said());
    assert_eq!(
        reads,
        vec!["src/lib.rs".to_string()],
        "the write this window made is not reread; the other file's real change still is: \
         {reads:?}"
    );
}
