//! The file gestures, over the bus: what the window says when a path is created, moved or removed,
//! and what it does with the answer.
//!
//! Every one of them is the same round trip — a menu pick or a drop raises a question, confirming it
//! sends one `EditProjectPath`, and `ProjectPathEdited` is where the tabs and the tree are settled.
//! A window is needed here rather than in `tests/explorer.rs` because the dialogs are the window's
//! state and the field they type into is the window's entity.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::{FileDialog, WindowRegistry};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::{DirEntry, DirListing, EntryKind, PathOp};
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
        let fixture = Self {
            state,
            window,
            host,
            project,
        };
        // A root and one folder under it, so every path these tests name is one the tree holds.
        fixture.deliver(
            Message::ProjectTreeListing {
                project_id: project,
                rel_path: String::new(),
                listings: vec![listing(
                    "",
                    vec![dir("", "src"), dir("", "docs"), file("", "justfile")],
                )],
            },
            cx,
        );
        fixture.deliver(
            Message::ProjectTreeListing {
                project_id: project,
                rel_path: "src".to_string(),
                listings: vec![listing("src", vec![file("src", "main.rs")])],
            },
            cx,
        );
        fixture
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

    /// Right-click a row (or the empty panel) and pick the row with this label.
    fn pick(&self, path: Option<&str>, label: &str, cx: &mut TestAppContext) {
        let path = path.map(str::to_string);
        self.with(cx, |state, window, cx| {
            state.open_explorer_menu(path, (0.0, 0.0), cx);
            let at = state
                .explorer(cx)
                .and_then(|explorer| explorer.menu.clone())
                .expect("the menu is up")
                .entries()
                .iter()
                .position(|entry| entry.label() == label)
                .expect("the menu offers it");
            state.pick_explorer_action(at, window, cx);
        });
    }

    fn dialog(&self, cx: &mut TestAppContext) -> Option<FileDialog> {
        self.with(cx, |state, _, _| state.workbench.file_dialog.clone())
    }

    /// Type a name into the dialog's field and confirm it.
    fn confirm(&self, typed: &str, cx: &mut TestAppContext) {
        let typed = typed.to_string();
        self.with(cx, |state, window, cx| {
            let field = state.file_name.clone();
            field.update(cx, |input, cx| input.set_value(typed.clone(), window, cx));
            state.confirm_file_dialog(window, cx);
        });
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

/// Every path edit the window asked for, in order.
fn edits(said: &[Message]) -> Vec<(String, Option<String>, PathOp)> {
    said.iter()
        .filter_map(|message| match message {
            Message::EditProjectPath {
                rel_path, to, op, ..
            } => Some((rel_path.clone(), to.clone(), *op)),
            _ => None,
        })
        .collect()
}

fn open_paths(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<String> {
    fixture.with(cx, |state, _, cx| {
        state
            .editor(cx)
            .map(|editor| editor.open.iter().map(|f| f.path.clone()).collect())
            .unwrap_or_default()
    })
}

/// New file asks for a name inside the row's folder, sends one `Create`, and the answer opens the
/// file where the user made it.
#[gpui::test]
fn a_new_file_is_named_then_created_then_opened(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.pick(Some("src/main.rs"), "New file", cx);
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::New {
            parent: "src".to_string(),
            dir: false,
        }),
        "the folder holding the row is what a new file lands in"
    );

    fixture.confirm("notes.md", cx);
    assert_eq!(
        edits(&fixture.said()),
        vec![(
            "src/notes.md".to_string(),
            None,
            PathOp::Create { dir: false }
        )]
    );
    assert_eq!(fixture.dialog(cx), None, "confirming closes the question");

    fixture.deliver(
        Message::ProjectPathEdited {
            project_id: fixture.project,
            rel_path: "src/notes.md".to_string(),
            to: None,
            op: PathOp::Create { dir: false },
        },
        cx,
    );
    assert_eq!(
        open_paths(&fixture, cx),
        vec!["src/notes.md".to_string()],
        "a created file opens where it was made"
    );
    // The folder it landed in is re-listed rather than waited for.
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::ProjectTree { rel_path, .. } if rel_path == "src"
        )),
        "the gesture asks for the folder it changed: {said:?}"
    );
}

/// A new folder is the same gesture with `dir` set, and nothing is opened for it.
#[gpui::test]
fn a_new_folder_carries_dir_and_opens_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.pick(None, "New folder", cx);
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::New {
            parent: String::new(),
            dir: true,
        }),
        "the empty panel is the project's root"
    );
    fixture.confirm("notes", cx);
    assert_eq!(
        edits(&fixture.said()),
        vec![("notes".to_string(), None, PathOp::Create { dir: true })]
    );

    fixture.deliver(
        Message::ProjectPathEdited {
            project_id: fixture.project,
            rel_path: "notes".to_string(),
            to: None,
            op: PathOp::Create { dir: true },
        },
        cx,
    );
    assert!(
        open_paths(&fixture, cx).is_empty(),
        "there is nothing to open in a folder"
    );
}

/// Delete asks first, and the wording is the op: Shift is what makes it permanent, and the two are
/// two different messages.
#[gpui::test]
fn delete_asks_before_it_sends_and_trash_is_the_default(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.pick(Some("src/main.rs"), "Delete", cx);
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::Remove {
            path: "src/main.rs".to_string(),
            dir: false,
            trash: true,
        }),
        "no modifier held means the platform's Trash"
    );
    assert!(
        edits(&fixture.said()).is_empty(),
        "nothing is sent before the question is answered"
    );

    fixture.confirm("", cx);
    assert_eq!(
        edits(&fixture.said()),
        vec![("src/main.rs".to_string(), None, PathOp::Trash)]
    );
}

/// A removed path takes its tab with it, and what a Copy remembered about it is forgotten.
#[gpui::test]
fn a_removed_path_closes_its_tabs_and_clears_the_clipboard(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, _, cx| {
        state.select_file("src/main.rs".to_string(), cx)
    });
    fixture.pick(Some("src"), "Copy", cx);
    let _ = fixture.said();

    fixture.deliver(
        Message::ProjectPathEdited {
            project_id: fixture.project,
            rel_path: "src".to_string(),
            to: None,
            op: PathOp::Delete,
        },
        cx,
    );

    assert!(
        open_paths(&fixture, cx).is_empty(),
        "the tab under the folder that went is gone too"
    );
    assert_eq!(
        fixture.with(cx, |state, _, cx| state
            .explorer(cx)
            .and_then(|e| e.copied.clone())),
        None,
        "a path that is not there is not worth remembering"
    );
}

/// Paste and Duplicate are the same message: a copy into a folder, under a name it does not hold.
#[gpui::test]
fn paste_and_duplicate_copy_under_a_free_name(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.pick(Some("src/main.rs"), "Copy", cx);
    fixture.pick(Some("docs"), "Paste", cx);
    assert_eq!(
        edits(&fixture.said()),
        vec![(
            "src/main.rs".to_string(),
            Some("docs/main.rs".to_string()),
            PathOp::Copy
        )],
        "another folder has room for the name as it is"
    );

    fixture.pick(Some("src/main.rs"), "Duplicate", cx);
    assert_eq!(
        edits(&fixture.said()),
        vec![(
            "src/main.rs".to_string(),
            Some("src/main copy.rs".to_string()),
            PathOp::Copy
        )],
        "in place, the collision is certain and does not need a refusal to discover"
    );
}

/// A rename is a move, and every tab at or under the old path follows it — buffer and all.
#[gpui::test]
fn a_rename_moves_the_path_and_its_tabs_follow(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, _, cx| {
        state.select_file("src/main.rs".to_string(), cx)
    });
    let _ = fixture.said();

    fixture.pick(Some("src"), "Rename", cx);
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::Rename {
            path: "src".to_string()
        })
    );
    fixture.confirm("lib", cx);
    assert_eq!(
        edits(&fixture.said()),
        vec![("src".to_string(), Some("lib".to_string()), PathOp::Move)]
    );

    fixture.deliver(
        Message::ProjectPathEdited {
            project_id: fixture.project,
            rel_path: "src".to_string(),
            to: Some("lib".to_string()),
            op: PathOp::Move,
        },
        cx,
    );
    assert_eq!(
        open_paths(&fixture, cx),
        vec!["lib/main.rs".to_string()],
        "the tab under the renamed folder is pointed at the new name"
    );
}

/// A file dropped on a folder moves with no question. A folder raises one — and the ten-minute
/// window the dialog's checkbox opens is what stops it asking the second time.
#[gpui::test]
fn a_dropped_file_moves_and_a_dropped_folder_asks_once(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| {
        state.drop_path_on("src/main.rs".to_string(), "docs".to_string(), cx)
    });
    assert_eq!(fixture.dialog(cx), None, "a file is not asked about");
    assert_eq!(
        edits(&fixture.said()),
        vec![(
            "src/main.rs".to_string(),
            Some("docs/main.rs".to_string()),
            PathOp::Move
        )]
    );

    // A folder is the gesture with something under it, so it asks.
    fixture.with(cx, |state, _, cx| {
        state.drop_path_on("src".to_string(), "docs".to_string(), cx)
    });
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::Move {
            path: "src".to_string(),
            into: "docs".to_string(),
        })
    );
    assert!(edits(&fixture.said()).is_empty());

    // Ticked, then confirmed: the move goes out and the window opens.
    fixture.with(cx, |state, window, cx| {
        state.toggle_move_unasked(cx);
        state.confirm_file_dialog(window, cx);
    });
    assert_eq!(
        edits(&fixture.said()),
        vec![(
            "src".to_string(),
            Some("docs/src".to_string()),
            PathOp::Move
        )]
    );

    fixture.with(cx, |state, _, cx| {
        state.drop_path_on("docs".to_string(), "src".to_string(), cx)
    });
    assert_eq!(
        fixture.dialog(cx),
        None,
        "inside the ten minutes the second folder drag moves silently"
    );
    assert_eq!(
        edits(&fixture.said()),
        vec![(
            "docs".to_string(),
            Some("src/docs".to_string()),
            PathOp::Move
        )]
    );
}

/// A drop that would change nothing, or that would put a folder inside itself, is refused here —
/// so the gesture never raises a question about a move that could not happen.
#[gpui::test]
fn a_drop_that_changes_nothing_is_refused_without_asking(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    for (path, into) in [
        ("src", "src"),         // onto itself
        ("src/main.rs", "src"), // already there
        ("src", ""),            // already at the root
        ("src", "src/deep"),    // a folder into its own child
    ] {
        fixture.with(cx, |state, _, cx| {
            state.drop_path_on(path.to_string(), into.to_string(), cx)
        });
        assert_eq!(fixture.dialog(cx), None, "{path} onto {into} asks nothing");
        assert!(
            edits(&fixture.said()).is_empty(),
            "{path} onto {into} sends nothing"
        );
    }
}

/// A new buffer has nowhere to go, so a save asks where — and the tab takes the name the user chose
/// before the host has answered, so a refusal on that path lands on a tab that reads correctly.
#[gpui::test]
fn an_untitled_buffer_asks_where_to_be_saved(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.with(cx, |state, window, cx| {
        state.new_untitled_file(&ubiq::app::NewFile, window, cx)
    });
    assert_eq!(open_paths(&fixture, cx), vec!["untitled-1".to_string()]);
    assert!(
        fixture
            .said()
            .iter()
            .all(|message| !matches!(message, Message::ReadProjectFile { .. })),
        "there is nothing to read for a buffer that was never on disk"
    );

    fixture.with(cx, |state, window, cx| {
        state.save_active_file(&ubiq::app::SaveFile, window, cx)
    });
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::SaveAs {
            key: "untitled-1".to_string()
        }),
        "the save is a question first"
    );
    assert!(fixture.said().is_empty(), "and nothing is sent yet");

    fixture.confirm("docs/notes.md", cx);
    let said = fixture.said();
    let written: Vec<&Message> = said
        .iter()
        .filter(|message| matches!(message, Message::WriteProjectFile { .. }))
        .collect();
    assert!(
        matches!(
            written.as_slice(),
            [Message::WriteProjectFile {
                rel_path,
                expected: None,
                ..
            }] if rel_path == "docs/notes.md"
        ),
        "an absent version already means create, and refuse if anything is there: {written:?}"
    );
    assert_eq!(
        open_paths(&fixture, cx),
        vec!["docs/notes.md".to_string()],
        "the tab is retitled on the click, the same bet opening one makes"
    );
}

/// A dirty tab asks before it is dropped, and the answer is the dialog's — the same one Enter and
/// Escape already reach. The window's own close counts what would go with it, per project.
#[gpui::test]
fn an_unsaved_tab_is_asked_about_before_it_closes(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.with(cx, |state, _, cx| {
        state.select_file("src/main.rs".to_string(), cx)
    });
    fixture.deliver(
        Message::ProjectFileContents {
            project_id: fixture.project,
            rel_path: "src/main.rs".to_string(),
            contents: ubiq_proto::files::FileContents {
                bytes: b"fn main() {}\n".to_vec(),
                len: 13,
                truncated: false,
                is_binary: false,
                version: Some(ubiq_proto::files::FileVersion {
                    len: 13,
                    modified: None,
                }),
            },
        },
        cx,
    );
    let key = fixture.with(cx, |state, _, cx| {
        let open = state.open_project_mut(cx).expect("the project is open");
        let tab = open
            .editor
            .find_mut("src/main.rs")
            .expect("the tab is open");
        tab.refresh_dirty("fn main() { typed }\n");
        assert!(tab.dirty(), "the tab holds an unsaved edit");
        tab.key()
    });

    // A clean close is refused: the question goes up instead, and the tab stays.
    fixture.with(cx, |state, _, cx| state.close_editor_tab(0, cx));
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::DiscardChanges { key: key.clone() })
    );
    assert_eq!(open_paths(&fixture, cx), vec!["src/main.rs".to_string()]);

    // The window's close counts it, alongside nothing else running here.
    assert_eq!(
        fixture.with(cx, |state, _, cx| state.unsaved_summary(cx)),
        vec!["ubiq — 1 unsaved file".to_string()]
    );

    // Answered yes, the buffer goes.
    fixture.with(cx, |state, window, cx| {
        state.confirm_file_dialog(window, cx)
    });
    assert_eq!(fixture.dialog(cx), None);
    assert!(open_paths(&fixture, cx).is_empty(), "the tab was dropped");
}
