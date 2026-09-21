//! What the clipboard means to the window: the ⌘N question, and a paste into a composer.
//!
//! With no project on screen there is no tab to open, so what the first test asserts is the peel
//! — the question goes away and nothing else moves. The tab halves live in `app::clipboard`'s own
//! unit tests, which need no window.
//!
//! The paste tests drive `paste_into_composer` against the platform's own pasteboard, which
//! `TestAppContext` holds in memory. **What is not tested here is the keystroke**: which binding
//! wins `⌘V` between the field's own paste and this one is `install_key_bindings`' registration
//! order against a live focus tree, and it is compile-checked and exercised by hand rather than
//! asserted — the state side of the gesture is what every one of these pins.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use chrono::Utc;
use gpui::{AppContext as _, ClipboardEntry, ClipboardItem, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub, DialogCancel};
use ubiq::state::{FileDialog, WindowRegistry};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::files::{FileContents, FileError};
use ubiq_proto::ids::{ProjectId, SessionId};
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

/// Long enough for a message to cross a channel in the same process.
const PATIENCE: Duration = Duration::from_millis(500);

#[gpui::test]
fn escape_on_the_paste_question_takes_the_text_file(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;

    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        ubiq::app::install_key_bindings(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    state.update(cx, |state, cx| {
        state.workbench.file_dialog = Some(FileDialog::PasteImage);
        cx.notify();
    });
    cx.run_until_parked();

    handle
        .update(cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                state.cancel_dialog(&DialogCancel, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    // No project, so the text half is a no-op — but the question is gone rather than walked
    // past, which is the rung `dismiss.rs` pins for every other file question.
    state.read_with(cx, |state, _| {
        assert!(state.workbench.file_dialog.is_none());
    });
}

// ── Pasting into a composer ─────────────────────────────────────

const ROOT: &str = "/tmp/ubiq-paste";

struct Composer {
    state: Entity<AppState>,
    _window: WindowHandle<Root>,
    host: bus::HostEnd,
    agent: AgentId,
    project: ProjectId,
}

impl Composer {
    /// A window with one project open and one live conversation in it — the least that makes a
    /// composer something a paste can land in.
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

        let agent = AgentId::generate();
        let record = an_agent(agent);
        let session = WorkSession {
            id: record.session,
            name: "the project".to_string(),
            branch: String::new(),
            worktree: false,
        };
        host.send(
            To::Everyone,
            Message::ConversationStarted {
                project_id: project,
                agent: Box::new(record),
                session,
                accepts_input: true,
            },
        );
        cx.run_until_parked();

        Self {
            state,
            _window: window,
            host,
            agent,
            project,
        }
    }

    /// Put a second project in this window's hands, as a window holding two really does. The
    /// conversation stays in the first — which is the whole point of the cross-project case.
    fn also_holding(&self, root: &str, cx: &mut TestAppContext) -> ProjectId {
        let snapshot = a_project_at(root);
        let id = snapshot.record.id;
        cx.update(|cx| {
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });
        self._window
            .update(cx, |_, _, cx| {
                self.state
                    .update(cx, |state, cx| state.take_project(id, cx));
            })
            .expect("the window is open");
        cx.run_until_parked();
        id
    }

    /// Push one message at the window as the host would.
    fn tell(&self, message: Message, cx: &mut TestAppContext) {
        self.host.send(To::Everyone, message);
        cx.run_until_parked();
    }

    /// Put an item on the platform's pasteboard and paste it into the conversation's composer.
    fn paste(&self, item: ClipboardItem, cx: &mut TestAppContext) {
        let agent = self.agent;
        self._window
            .update(cx, |_, window, cx| {
                cx.write_to_clipboard(item);
                self.state.update(cx, |state, cx| {
                    state.paste_into_composer(agent, 0, window, cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// The tags the composer is holding, by path.
    fn attached(&self, cx: &TestAppContext) -> Vec<String> {
        self.state.read_with(cx, |state, cx| {
            state
                .teams_conversation(self.agent, cx)
                .map(|conversation| {
                    conversation
                        .attached
                        .iter()
                        .map(|file| file.path.clone())
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }
}

fn a_project() -> ProjectSnapshot {
    a_project_at(ROOT)
}

fn a_project_at(root: &str) -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: root.to_string(),
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
            initials: String::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-paste-workarea".to_string(),
    }
}

fn an_agent(id: AgentId) -> WorkAgent {
    WorkAgent {
        id,
        session: SessionId::generate(),
        task: None,
        parent: None,
        name: "Claude Code".to_string(),
        summary: None,
        role: "Implementer".to_string(),
        activity: Activity::Ended,
        note: String::new(),
        branch: "main".to_string(),
        tokens: 0.0,
        harness: "Claude Code".to_string(),
        account: "work".to_string(),
        model: String::new(),
        context_pct: 0,
        persistent: false,
        accept_all: false,
        debug_dump: None,
        run_dir: None,
        config_dir: None,
        thread: Vec::new(),
    }
}

/// A file copied in Finder: an `ExternalPaths` entry **and** a `String` of the same path, which
/// is exactly what the platform puts there and exactly what `clipboard_image` refuses.
fn a_copied_file(path: &str) -> ClipboardItem {
    ClipboardItem {
        entries: vec![
            ClipboardEntry::ExternalPaths(gpui::ExternalPaths(
                std::iter::once(PathBuf::from(path)).collect(),
            )),
            ClipboardEntry::String(gpui::ClipboardString::new(path.to_string())),
        ],
    }
}

/// A file copied from inside the project attaches under the path the `+` picker would have given
/// it — project-relative, so it is the `@path` the harness reads and one tag rather than two.
#[gpui::test]
fn a_copied_file_inside_the_project_attaches_project_relative(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    composer.paste(a_copied_file(&format!("{ROOT}/src/main.rs")), cx);

    assert_eq!(composer.attached(cx), vec!["src/main.rs".to_string()]);
}

/// A file from outside every project keeps its absolute path: the harness is running somewhere,
/// and that is the only shape that still names the file from there.
#[gpui::test]
fn a_copied_file_outside_every_project_attaches_absolutely(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    composer.paste(a_copied_file("/elsewhere/notes.md"), cx);

    assert_eq!(
        composer.attached(cx),
        vec!["/elsewhere/notes.md".to_string()]
    );
}

/// A picture has no path, so one is made for it: the bytes are written into the project under
/// `.ubiq/pasted/` and the tag names where they went. The size is the board's own byte count,
/// which is the one attachment nothing had to guess at.
#[gpui::test]
fn a_copied_image_is_written_into_the_project_and_attached(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    let bytes = vec![1u8, 2, 3, 4];
    composer.paste(
        ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            bytes.clone(),
        )),
        cx,
    );

    let attached = composer.attached(cx);
    assert_eq!(attached.len(), 1, "one tag for the picture");
    assert!(
        attached[0].starts_with(".ubiq/pasted/") && attached[0].ends_with(".png"),
        "written where Ubiq puts a pasted picture, got {}",
        attached[0]
    );

    // The folder's own `.gitignore` goes down beside the picture, so the picture's write is the
    // one the tag names rather than merely the first one.
    let wrote = composer
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::WriteProjectFile {
                rel_path, bytes, ..
            } if rel_path == attached[0] => Some((rel_path, bytes)),
            _ => None,
        });
    let (rel_path, written) = wrote.expect("the picture was written");
    assert_eq!(rel_path, attached[0], "the tag names what was written");
    // PNG is already a format a harness reads, so nothing was re-encoded on the way down.
    assert_eq!(written, bytes, "the board's bytes, not a re-encoding");
}

/// **A window holding two projects must not confuse them.** The conversation is in the first, the
/// file was copied out of the second, and `src/main.rs` exists in both — so the relative form
/// would name a different, existing file and the harness would read it without a word. The
/// absolute path is the only shape that still names what the user copied.
#[gpui::test]
fn a_file_from_another_project_in_the_same_window_attaches_absolutely(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    const OTHER: &str = "/tmp/ubiq-paste-other";
    composer.also_holding(OTHER, cx);

    composer.paste(a_copied_file(&format!("{OTHER}/src/main.rs")), cx);

    assert_eq!(
        composer.attached(cx),
        vec![format!("{OTHER}/src/main.rs")],
        "the agent's own project is the only one it may be relative to"
    );
}

/// A pasted picture's chip goes up before the write is answered, so a refused write has to take
/// it back off — otherwise the turn carries an `@path` to a file that was never written, and the
/// only trace of it is a log line.
#[gpui::test]
fn a_failed_pasted_write_detaches_the_chip_and_says_so(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    composer.paste(
        ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            vec![1u8, 2, 3, 4],
        )),
        cx,
    );
    let rel_path = composer.attached(cx).pop().expect("the picture attached");

    composer.tell(
        Message::ProjectFileError {
            project_id: composer.project,
            rel_path,
            error: FileError::Denied("read-only volume".to_string()),
        },
        cx,
    );

    assert!(
        composer.attached(cx).is_empty(),
        "the chip came back off again"
    );
    assert!(
        composer
            .said()
            .iter()
            .any(|message| matches!(message, Message::RaiseNotification { .. })),
        "the user was told, since there is no other surface for it"
    );
}

/// A file larger than the read ceiling arrives as a prefix, and a prefix of a PNG is not a PNG:
/// handing it to the image renderer draws an empty box that explains nothing. The panel says why
/// instead. The chip for such a file is already drawn in the danger tokens, so this is the
/// likeliest one anybody clicks.
#[gpui::test]
fn a_truncated_read_draws_the_note_rather_than_a_blank_box(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    let agent = composer.agent;
    composer
        ._window
        .update(cx, |_, _, cx| {
            composer.state.update(cx, |state, cx| {
                state.open_attachment_preview(
                    agent,
                    1,
                    ".ubiq/pasted/pasted-1.png".to_string(),
                    Some(9_000_000),
                    cx,
                );
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    composer.tell(
        Message::ProjectFileContents {
            project_id: composer.project,
            rel_path: ".ubiq/pasted/pasted-1.png".to_string(),
            contents: FileContents {
                bytes: vec![0x89, b'P', b'N', b'G'],
                len: 9_000_000,
                truncated: true,
                is_binary: true,
                version: None,
            },
        },
        cx,
    );

    composer.state.read_with(cx, |state, _| {
        let preview = state
            .workbench
            .attachment_preview
            .as_ref()
            .expect("the panel is still up");
        assert!(preview.bytes.is_none(), "a prefix is not a picture");
        assert_eq!(
            preview.failed.as_deref(),
            Some("larger than the read ceiling")
        );
    });
}

/// A late answer for the same path in *another* project is not this panel's file. Guarding on the
/// path alone would fill the popover with the wrong project's bytes.
#[gpui::test]
fn a_same_path_answer_from_another_project_does_not_fill_the_preview(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    let other = composer.also_holding("/tmp/ubiq-paste-other", cx);
    let agent = composer.agent;
    composer
        ._window
        .update(cx, |_, _, cx| {
            composer.state.update(cx, |state, cx| {
                state.open_attachment_preview(agent, 1, "src/main.rs".to_string(), None, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    composer.tell(
        Message::ProjectFileContents {
            project_id: other,
            rel_path: "src/main.rs".to_string(),
            contents: FileContents {
                bytes: b"the other project's file".to_vec(),
                len: 24,
                truncated: false,
                is_binary: false,
                version: None,
            },
        },
        cx,
    );

    composer.state.read_with(cx, |state, _| {
        let preview = state
            .workbench
            .attachment_preview
            .as_ref()
            .expect("the panel is still up");
        assert!(preview.bytes.is_none(), "still reading its own project's");
    });
}

/// A read that fails leaves the panel saying `Reading…` for as long as it is up unless the
/// failure reaches it — the editor's own `file_failed` never touches a path with no tab behind it.
#[gpui::test]
fn a_failed_read_gives_the_preview_its_cannot_preview_state(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    let agent = composer.agent;
    composer
        ._window
        .update(cx, |_, _, cx| {
            composer.state.update(cx, |state, cx| {
                state.open_attachment_preview(
                    agent,
                    1,
                    ".ubiq/pasted/pasted-1.png".to_string(),
                    None,
                    cx,
                );
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    composer.tell(
        Message::ProjectFileError {
            project_id: composer.project,
            rel_path: ".ubiq/pasted/pasted-1.png".to_string(),
            error: FileError::Missing,
        },
        cx,
    );

    composer.state.read_with(cx, |state, _| {
        let preview = state
            .workbench
            .attachment_preview
            .as_ref()
            .expect("the panel is still up");
        assert!(
            preview.failed.is_some(),
            "the panel stopped waiting and said why"
        );
    });
}

/// The pasted folder ignores itself, so the untracked binaries this gesture writes never turn up
/// in the user's `git status`. The user's own `.gitignore` is not touched.
#[gpui::test]
fn the_pasted_folder_writes_its_own_gitignore(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    composer.paste(
        ClipboardItem::new_image(&gpui::Image::from_bytes(
            gpui::ImageFormat::Png,
            vec![1u8, 2, 3, 4],
        )),
        cx,
    );

    let writes: Vec<(String, Vec<u8>)> = composer
        .said()
        .into_iter()
        .filter_map(|message| match message {
            Message::WriteProjectFile {
                rel_path, bytes, ..
            } => Some((rel_path, bytes)),
            _ => None,
        })
        .collect();

    let ignore = writes
        .iter()
        .find(|(path, _)| path == ".ubiq/pasted/.gitignore")
        .expect("the folder ignores itself");
    assert_eq!(String::from_utf8_lossy(&ignore.1).trim(), "*");
    assert!(
        !writes.iter().any(|(path, _)| path == ".gitignore"),
        "the user's own ignore file is never written"
    );
}

/// **Text is still text.** A board with nothing but a string on it attaches nothing and writes
/// nothing — the keystroke goes back to the field, which is what keeps this gesture from taking
/// ordinary paste away from the one control the user types into.
#[gpui::test]
fn a_text_board_attaches_nothing(cx: &mut TestAppContext) {
    let composer = Composer::open(cx);
    composer.paste(ClipboardItem::new_string("just words".to_string()), cx);

    assert!(composer.attached(cx).is_empty());
    assert!(
        !composer
            .said()
            .iter()
            .any(|message| matches!(message, Message::WriteProjectFile { .. })),
        "nothing was written for a text paste"
    );
}
