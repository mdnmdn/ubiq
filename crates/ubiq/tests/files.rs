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
use gpui_component::input::InputEvent;
use ubiq::app::{AppState, BusHub, CloseEditor};
use ubiq::state::{FileDialog, WindowRegistry};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::{DirEntry, DirListing, EntryKind, PathOp, RelatedFile};
use ubiq_proto::git::{GitHead, GitNested};
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

    /// Type into one of the window's fields and press Enter, which is what commits it.
    fn type_into(
        &self,
        pick: impl Fn(&AppState) -> Entity<gpui_component::input::InputState>,
        text: &str,
        cx: &mut TestAppContext,
    ) {
        let text = text.to_string();
        self.with(cx, |state, window, cx| {
            let input = pick(state);
            input.update(cx, |field, cx| {
                field.set_value(text.clone(), window, cx);
                cx.emit(InputEvent::PressEnter {
                    shift: false,
                    secondary: false,
                });
            });
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
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
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
            ext: None,
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
            carry_related: false,
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
            ext: None,
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
            carry_related: false,
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
            related: Vec::new(),
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

/// Rename and Delete both ask the host what else a markdown file carries, and the question's own
/// checkbox — default checked — is what tells the host to carry it along.
#[gpui::test]
fn a_rename_asks_what_it_carries_and_the_checkbox_defaults_checked(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.deliver(
        Message::ProjectTreeListing {
            project_id: fixture.project,
            rel_path: "docs".to_string(),
            listings: vec![listing("docs", vec![file("docs", "spec.md")])],
        },
        cx,
    );
    let _ = fixture.said();

    fixture.pick(Some("docs/spec.md"), "Rename", cx);
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::Rename {
            path: "docs/spec.md".to_string(),
            related: Vec::new(),
        }),
        "the question is up before the host answers what else the path carries"
    );
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::RelatedProjectFiles { rel_path, .. } if rel_path == "docs/spec.md"
        )),
        "the question asks the host what else the path carries: {said:?}"
    );

    let related = vec![RelatedFile {
        rel_path: "docs/spec.md.annotation.json".to_string(),
        label: "annotations".to_string(),
    }];
    fixture.deliver(
        Message::ProjectFileRelated {
            project_id: fixture.project,
            rel_path: "docs/spec.md".to_string(),
            related: related.clone(),
        },
        cx,
    );
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::Rename {
            path: "docs/spec.md".to_string(),
            related: related.clone(),
        }),
        "the answer fills in the question that is still up"
    );

    fixture.confirm("notes.md", cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::EditProjectPath {
                carry_related: true,
                op: PathOp::Move,
                ..
            }
        )),
        "the checkbox defaults to checked: {said:?}"
    );
}

/// Unchecking the box before confirming is what leaves a related file behind.
#[gpui::test]
fn unchecking_carry_related_leaves_the_related_file_behind(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.deliver(
        Message::ProjectTreeListing {
            project_id: fixture.project,
            rel_path: "docs".to_string(),
            listings: vec![listing("docs", vec![file("docs", "spec.md")])],
        },
        cx,
    );
    let _ = fixture.said();

    fixture.pick(Some("docs/spec.md"), "Delete", cx);
    fixture.deliver(
        Message::ProjectFileRelated {
            project_id: fixture.project,
            rel_path: "docs/spec.md".to_string(),
            related: vec![RelatedFile {
                rel_path: "docs/spec.md.annotation.json".to_string(),
                label: "annotations".to_string(),
            }],
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| state.toggle_carry_related(cx));

    fixture.confirm("", cx);
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::EditProjectPath {
                carry_related: false,
                op: PathOp::Trash,
                ..
            }
        )),
        "unchecking the box leaves the sidecar behind: {said:?}"
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
            carry_related: false,
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
            path: "src".to_string(),
            related: Vec::new(),
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
            carry_related: false,
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

/// A pasted picture is a file that has never been on disk, so its save is a creation — even when
/// the path the user types names a folder that is not there yet.
///
/// The bug: the save came back `Missing`, drawn as "no longer there" in the **Not saved** modal,
/// over a picture that had never been anywhere. The interface's half of that round trip is this
/// one message; the host's half — making the folders a creation names — is in
/// `crates/ubiq-host/tests/files.rs`.
#[gpui::test]
fn a_pasted_picture_is_saved_as_a_creation(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    let picture = image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
    let mut png = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("an in-memory encode succeeds");
    cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            png,
        )));
    });
    fixture.with(cx, |state, window, cx| {
        state.paste_clipboard_image(&ubiq::app::PasteClipboardImage, window, cx)
    });
    assert_eq!(open_paths(&fixture, cx), vec!["capture-1.png".to_string()]);

    fixture.with(cx, |state, window, cx| {
        state.save_active_file(&ubiq::app::SaveFile, window, cx)
    });
    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::SaveAs {
            key: "capture-1.png".to_string()
        }),
        "a picture that was never on disk is asked where to go"
    );
    assert!(
        writes(&fixture.said()).is_empty(),
        "and nothing is written yet"
    );

    fixture.confirm("shots/new/capture-1.png", cx);
    let written = writes(&fixture.said());
    assert!(
        matches!(
            written.as_slice(),
            [Message::WriteProjectFile {
                rel_path,
                expected: None,
                overwrite: false,
                ..
            }] if rel_path == "shots/new/capture-1.png"
        ),
        "the flatten goes out as a creation, whatever folders the path names: {written:?}"
    );
    assert_eq!(
        open_paths(&fixture, cx),
        vec!["shots/new/capture-1.png".to_string()],
        "the tab is retitled on the click, the same bet opening one makes"
    );
}

/// Every `WriteProjectFile` the window has said, in order.
fn writes(said: &[Message]) -> Vec<Message> {
    said.iter()
        .filter(|message| matches!(message, Message::WriteProjectFile { .. }))
        .cloned()
        .collect()
}

/// Save an untitled buffer onto `docs/notes.md` and have the host refuse it as taken.
fn save_onto_a_taken_path(fixture: &Fixture, cx: &mut TestAppContext) {
    fixture.with(cx, |state, window, cx| {
        state.new_untitled_file(&ubiq::app::NewFile, window, cx)
    });
    fixture.with(cx, |state, window, cx| {
        state.save_active_file(&ubiq::app::SaveFile, window, cx)
    });
    fixture.confirm("docs/notes.md", cx);
    let _ = fixture.said();
    fixture.deliver(
        Message::ProjectFileError {
            project_id: fixture.project,
            rel_path: "docs/notes.md".to_string(),
            error: ubiq_proto::files::FileError::Conflict,
        },
        cx,
    );
}

/// A save onto a path that is taken asks, rather than failing where nobody can see it.
///
/// The write names no version, so the host refuses it as a creation that landed on something.
/// That refusal used to reach the user as the colour of a dot the next keystroke cleared.
#[gpui::test]
fn a_save_onto_a_path_that_is_taken_asks_before_it_overwrites(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();
    save_onto_a_taken_path(&fixture, cx);

    assert_eq!(
        fixture.dialog(cx),
        Some(FileDialog::OverwriteFile {
            key: "docs/notes.md".to_string()
        }),
        "a create that landed on something is the one refusal with a question in it"
    );
    assert_eq!(
        open_paths(&fixture, cx),
        vec!["docs/notes.md".to_string()],
        "and nothing was dropped on the way"
    );

    // Answered yes: the same bytes again, this time allowed to land on what is there.
    fixture.with(cx, |state, window, cx| {
        state.confirm_file_dialog(window, cx)
    });
    let written = writes(&fixture.said());
    assert!(
        matches!(
            written.as_slice(),
            [Message::WriteProjectFile {
                rel_path,
                expected: None,
                overwrite: true,
                ..
            }] if rel_path == "docs/notes.md"
        ),
        "only a confirmed overwrite ever carries the flag: {written:?}"
    );
    assert_eq!(fixture.dialog(cx), None, "and the question is answered");
}

/// A refused save leaves the tab savable, and a second refusal is said out loud.
///
/// The tab was retargeted on the click, so after a refusal it is no longer untitled and still has
/// no version — which is what used to make it unsavable for the rest of the session: every later
/// ⌘S sent nothing and said nothing, which is the bug as the user met it.
#[gpui::test]
fn a_refused_save_can_be_taken_again_and_never_loses_the_edits(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();
    save_onto_a_taken_path(&fixture, cx);

    // Not overwriting: the question is dismissed, and the keystroke taken again.
    fixture.with(cx, |state, _, cx| state.close_file_dialog(cx));
    let _ = fixture.said();
    fixture.with(cx, |state, window, cx| {
        state.save_active_file(&ubiq::app::SaveFile, window, cx)
    });
    let written = writes(&fixture.said());
    assert!(
        matches!(
            written.as_slice(),
            [Message::WriteProjectFile {
                rel_path,
                expected: None,
                overwrite: false,
                ..
            }] if rel_path == "docs/notes.md"
        ),
        "a buffer that was never read from disk is written as a creation: {written:?}"
    );

    // A refusal with nothing to ask about is reported instead, and the buffer keeps everything.
    fixture.deliver(
        Message::ProjectFileError {
            project_id: fixture.project,
            rel_path: "docs/notes.md".to_string(),
            error: ubiq_proto::files::FileError::Denied("read-only".to_string()),
        },
        cx,
    );
    assert!(
        matches!(
            fixture.dialog(cx),
            Some(FileDialog::SaveFailed { ref key, .. }) if key == "docs/notes.md"
        ),
        "a save that did not happen says so"
    );
    let state = fixture.with(cx, |state, _, cx| {
        state
            .file("docs/notes.md", cx)
            .map(|file| matches!(file.save, ubiq::state::SaveState::Failed(_)))
    });
    assert_eq!(
        state,
        Some(true),
        "the tab is still open and still holds an unwritten buffer"
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

/// A tab that lands on a Markdown preview by a close — not a click — still answers `cmd-w`.
///
/// Before the fix, the buffer nobody drew (`Preview` shows no `Input`) still took the window's
/// focus, which put it on a node this frame never painted. GPUI's key dispatch falls back to the
/// window's own root when that happens, and the app's whole `"Workbench"` key context — every
/// binding in it, `CloseEditor` included — is unreachable from there.
#[gpui::test]
fn closing_a_tab_still_lets_the_markdown_tab_behind_it_close(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    for (path, bytes) in [
        ("src/main.rs", &b"fn main() {}\n"[..]),
        ("docs/notes.md", b"# Notes\n"),
        ("justfile", b"default:\n\techo hi\n"),
    ] {
        fixture.with(cx, |state, _, cx| state.select_file(path.to_string(), cx));
        fixture.deliver(
            Message::ProjectFileContents {
                project_id: fixture.project,
                rel_path: path.to_string(),
                contents: ubiq_proto::files::FileContents {
                    bytes: bytes.to_vec(),
                    len: bytes.len() as u64,
                    truncated: false,
                    is_binary: false,
                    version: Some(ubiq_proto::files::FileVersion {
                        len: bytes.len() as u64,
                        modified: None,
                    }),
                },
            },
            cx,
        );
    }
    assert_eq!(
        open_paths(&fixture, cx),
        vec![
            "src/main.rs".to_string(),
            "docs/notes.md".to_string(),
            "justfile".to_string(),
        ]
    );

    // "justfile" is the active tab; closing it lands the editor back on "docs/notes.md" by the
    // close itself, exactly the way the bug was reported — never by a click on that tab.
    fixture.with(cx, |state, _, cx| state.close_editor_tab(2, cx));
    assert_eq!(
        fixture.with(cx, |state, _, cx| state
            .editor(cx)
            .and_then(|editor| editor.active_file())
            .map(|file| file.path.clone())),
        Some("docs/notes.md".to_string()),
        "the close itself put the Markdown tab in front"
    );

    // `cmd-w`, from here, still closes it — the same dispatch a real keystroke takes.
    cx.dispatch_action(fixture.window.into(), CloseEditor);
    assert_eq!(
        open_paths(&fixture, cx),
        vec!["src/main.rs".to_string()],
        "the Markdown preview tab closed like any other"
    );
}

/// "Exclude from search" adds the folder to the project's own excludes and sends the whole list,
/// touching nothing else on the record; picking it again is now offered as "Add to search".
#[gpui::test]
fn exclude_from_search_adds_the_path_then_offers_to_add_it_back(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.pick(Some("src"), "Exclude from search", cx);
    let sent = fixture.said().into_iter().find_map(|m| match m {
        Message::UpdateProject {
            project_id,
            name,
            colour,
            custom_colour,
            search_excludes,
            index,
            mission_term,
            tools,
            managed_repos,
            lanes,
            runs_on,
        } => Some((
            project_id,
            name,
            colour,
            custom_colour,
            search_excludes,
            index,
            mission_term,
            tools,
            managed_repos,
            lanes,
            runs_on,
        )),
        _ => None,
    });
    assert_eq!(
        sent,
        Some((
            fixture.project,
            None,
            None,
            None,
            Some(vec!["src".to_string()]),
            None,
            None,
            None,
            None,
            None,
            None
        )),
        "only the excludes change"
    );

    let labels = fixture.with(cx, |state, _, cx| {
        state.open_explorer_menu(Some("src".to_string()), (0.0, 0.0), cx);
        state
            .explorer(cx)
            .and_then(|explorer| explorer.menu.clone())
            .expect("the menu is up")
            .entries()
            .iter()
            .map(|e| e.label())
            .collect::<Vec<_>>()
    });
    assert!(labels.contains(&"Add to search"));
    assert!(!labels.contains(&"Exclude from search"));
}

/// The project settings dialog's repository tick box sends the whole `managed_repos` list at
/// once, the way the exclude field does — ticking it on adds the path, ticking it off again drops
/// it.
#[gpui::test]
fn ticking_a_repository_sends_the_managed_list_then_clears_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.deliver(
        Message::GitWorkingTree {
            project_id: fixture.project,
            generation: 1,
            entries: Vec::new(),
            rollups: Vec::new(),
            repos: vec![GitNested {
                rel_path: "vendor/lib".to_string(),
                head: GitHead::Branch("main".to_string()),
                submodule: false,
                counts: None,
                managed: false,
            }],
            truncated: false,
        },
        cx,
    );

    fixture.with(cx, |state, _, cx| state.open_edit_project(cx));
    fixture.with(cx, |state, _, cx| {
        state.toggle_project_managed_repo(fixture.project, "vendor/lib".to_string(), cx)
    });
    let sent = fixture.said().into_iter().find_map(|m| match m {
        Message::UpdateProject { managed_repos, .. } => managed_repos,
        _ => None,
    });
    assert_eq!(
        sent,
        Some(vec!["vendor/lib".to_string()]),
        "the tick added the repository"
    );

    fixture.with(cx, |state, _, cx| {
        state.toggle_project_managed_repo(fixture.project, "vendor/lib".to_string(), cx)
    });
    let sent = fixture.said().into_iter().find_map(|m| match m {
        Message::UpdateProject { managed_repos, .. } => managed_repos,
        _ => None,
    });
    assert_eq!(sent, Some(Vec::new()), "the tick removed it again");
}

/// The project settings dialog's own exclude field adds a pattern on Enter, sending the whole
/// list the same way the explorer menu's toggle does; the remove control clears it again.
#[gpui::test]
fn project_settings_search_exclude_field_adds_then_removes_a_pattern(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    fixture.with(cx, |state, _, cx| state.open_edit_project(cx));

    fixture.type_into(|state| state.project_exclude_input.clone(), "*.log", cx);
    let sent = fixture.said().into_iter().find_map(|m| match m {
        Message::UpdateProject {
            search_excludes, ..
        } => search_excludes,
        _ => None,
    });
    assert_eq!(
        sent,
        Some(vec!["*.log".to_string()]),
        "the pattern was added"
    );
    assert_eq!(
        fixture.with(cx, |state, _, cx| state
            .project_exclude_input
            .read(cx)
            .value()
            .to_string()),
        "",
        "the field is cleared after adding"
    );

    fixture.with(cx, |state, _, cx| {
        state.remove_project_search_exclude(fixture.project, "*.log".to_string(), cx)
    });
    let sent = fixture.said().into_iter().find_map(|m| match m {
        Message::UpdateProject {
            search_excludes, ..
        } => search_excludes,
        _ => None,
    });
    assert_eq!(sent, Some(Vec::new()), "the pattern was removed");
}

/// The project settings dialog's path field abbreviates the user's home directory to `~`, the way
/// a shell prompt does — but only the home directory itself, or a path under it. A sibling whose
/// name merely starts with the same characters is not a child of it.
#[test]
fn home_abbreviated_replaces_only_the_home_directory_prefix() {
    let home = std::env::var("HOME")
        .expect("HOME must be set to run this test")
        .trim_end_matches('/')
        .to_string();

    assert_eq!(ubiq::ui::sink::project::home_abbreviated(&home), "~");
    assert_eq!(
        ubiq::ui::sink::project::home_abbreviated(&format!("{home}/code/ubiq")),
        "~/code/ubiq"
    );
    assert_eq!(
        ubiq::ui::sink::project::home_abbreviated("/var/empty/not-home"),
        "/var/empty/not-home"
    );
    let false_prefix = format!("{home}other");
    assert_eq!(
        ubiq::ui::sink::project::home_abbreviated(&false_prefix),
        false_prefix,
        "a sibling that merely starts with the home path is not a child of it"
    );
}

/// A guest tab — a file opened from outside every project (`OpenFile::guest`) — saves over
/// `Message::WriteHostFile`, an absolute path with no project id, rather than
/// `Message::WriteProjectFile`. `open_guest_file` reads straight off disk (`D54`), so this drives
/// a real temp file rather than delivering a fake `ReadProjectFile` reply the way every other test
/// in this file does.
#[gpui::test]
fn a_guest_tab_saves_over_write_host_file(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let _ = fixture.said();

    let dir = tempfile::TempDir::new().unwrap();
    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, "hello\n").unwrap();
    let abs = outside.to_string_lossy().into_owned();

    fixture.with(cx, |state, _, cx| state.open_guest_file(&outside, cx));
    assert_eq!(
        open_paths(&fixture, cx),
        vec![abs.clone()],
        "the tab's key is the absolute path itself"
    );
    // The read never touches the bus (`D54`) — nothing about it is a `ReadProjectFile`.
    assert!(
        fixture
            .said()
            .iter()
            .all(|message| !matches!(message, Message::ReadProjectFile { .. })),
    );

    // Marked dirty directly rather than by typing, on `an_unsaved_tab_is_asked_about_before_it_closes`'s
    // own precedent: what is under test is the save wiring, not the widget's keystroke path.
    fixture.with(cx, |state, _, cx| {
        let open = state.open_project_mut(cx).expect("the project is open");
        let tab = open.editor.find_mut(&abs).expect("the guest tab is open");
        assert!(
            tab.savable(),
            "an untruncated, versioned guest read is savable"
        );
        tab.refresh_dirty("hello, edited\n");
        assert!(tab.dirty());
    });

    fixture.with(cx, |state, window, cx| {
        state.save_active_file(&ubiq::app::SaveFile, window, cx)
    });
    let said = fixture.said();
    let written: Vec<&Message> = said
        .iter()
        .filter(|message| matches!(message, Message::WriteHostFile { .. }))
        .collect();
    assert!(
        matches!(
            written.as_slice(),
            [Message::WriteHostFile { path, .. }] if *path == abs
        ),
        "a dirty, savable guest tab writes over WriteHostFile: {written:?}"
    );
    assert!(
        said.iter()
            .all(|message| !matches!(message, Message::WriteProjectFile { .. })),
        "a guest tab never writes as a project file"
    );

    // The host answers, and the tab's version and baseline land.
    let version = ubiq_proto::files::FileVersion {
        len: 6,
        modified: None,
    };
    fixture.deliver(
        Message::HostFileWritten {
            path: abs.clone(),
            version,
        },
        cx,
    );
    fixture.with(cx, |state, _, cx| {
        let open = state.open_project_mut(cx).expect("the project is open");
        let tab = open.editor.find_mut(&abs).expect("still open");
        assert_eq!(tab.version(), Some(version));
    });
}
