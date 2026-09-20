//! Forcing a viewer onto an open tab from the status bar's file-kind readout.
//!
//! The override is deliberately not a fact about the file: nothing is written to the view prefs or
//! the dock's payload, so it lives exactly as long as the tab does. That is the whole contract, and
//! it is what these assert — the kind changes, the layout re-settles onto one the new kind offers,
//! and a tab closed and reopened is back on `ViewerKind::of`.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use ubiq::app::{AppState, BusHub};
use ubiq::state::WindowRegistry;
use ubiq::state::editor::{FileLanguage, ViewLayout, ViewerKind};
use ubiq_proto::bus;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// A window on one project. The host end is held only so the bus stays connected — nothing here
/// needs an answer, because a tab exists before its bytes do.
struct Fixture {
    state: Entity<AppState>,
    _window: WindowHandle<gpui_component::Root>,
    _host: bus::HostEnd,
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
            gpui_component::Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            _window: window,
            _host: host,
        }
    }

    /// What one open tab is drawn by and in which layout, `None` if it is not open at all.
    fn view_of(&self, key: &str, cx: &mut TestAppContext) -> Option<(ViewerKind, ViewLayout)> {
        self.state.read_with(cx, |state, cx| {
            state
                .editor(cx)
                .and_then(|editor| editor.open.get(editor.index_of_key(key)?))
                .map(|file| (file.viewer, file.layout))
        })
    }

    /// The language a tab is highlighted as, `None` if it is not open at all.
    fn language_of(&self, key: &str, cx: &mut TestAppContext) -> Option<FileLanguage> {
        self.state.read_with(cx, |state, cx| {
            state.file(key, cx).map(|file| file.language)
        })
    }

    fn close(&self, key: &str, cx: &mut TestAppContext) {
        self.state.update(cx, |state, cx| {
            let Some(index) = state.editor(cx).and_then(|editor| editor.index_of_key(key)) else {
                return;
            };
            state.close_editor_tab(index, cx);
        });
        cx.run_until_parked();
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
            initials: String::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

/// The whole round trip: a markdown tab forced onto the Excalidraw viewer, whose toggle holds
/// neither `Source` nor `Split`, and then closed and reopened.
#[gpui::test]
fn a_forced_viewer_re_settles_the_layout_and_dies_with_the_tab(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture.state.update(cx, |state, cx| {
        state.select_file("README.md".to_string(), cx)
    });
    cx.run_until_parked();
    fixture.state.update(cx, |state, cx| {
        state.set_view_layout("README.md", ViewLayout::Source, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.view_of("README.md", cx),
        Some((ViewerKind::Markdown, ViewLayout::Source)),
        "the extension picks the viewer, and the tab is left showing its source"
    );

    // Excalidraw offers Edit and Preview and nothing else, so the layout cannot stay on Source.
    fixture.state.update(cx, |state, cx| {
        state.set_viewer_kind("README.md", ViewerKind::Excalidraw, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.view_of("README.md", cx),
        Some((ViewerKind::Excalidraw, ViewLayout::Edit)),
        "the forced viewer takes, and the layout falls back to the first one it offers"
    );

    // A layout the new kind already offers is left where it is.
    fixture.state.update(cx, |state, cx| {
        state.set_view_layout("README.md", ViewLayout::Preview, cx)
    });
    cx.run_until_parked();
    fixture.state.update(cx, |state, cx| {
        state.set_viewer_kind("README.md", ViewerKind::Mermaid, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.view_of("README.md", cx),
        Some((ViewerKind::Mermaid, ViewLayout::Preview)),
        "Mermaid offers Preview, so re-settling has nothing to do"
    );

    fixture.close("README.md", cx);
    assert_eq!(
        fixture.view_of("README.md", cx),
        None,
        "the tab is gone, and with it everything that was only true of the tab"
    );

    fixture.state.update(cx, |state, cx| {
        state.select_file("README.md".to_string(), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.view_of("README.md", cx).map(|(viewer, _)| viewer),
        Some(ViewerKind::of("README.md")),
        "nothing was written down, so the reopened tab starts from the extension again"
    );
}

/// Forcing the Markdown viewer onto a tab whose extension named no language must make the source
/// highlight as Markdown too — T-13. `language` used to lag behind `viewer`: the chip's label and
/// the tree-sitter grammar `attach_file` bakes into the buffer both read `file.language`, and
/// `set_viewer_kind` used to leave it exactly where the extension put it, so a `.txt` file forced
/// onto Markdown drew the Markdown viewer over an unhighlighted source. Closing the tab and
/// reopening it must drop the override, the same as the viewer itself does.
#[gpui::test]
fn forcing_the_markdown_viewer_forces_markdown_highlighting_too(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    fixture.state.update(cx, |state, cx| {
        state.select_file("notes.txt".to_string(), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.language_of("notes.txt", cx),
        Some(FileLanguage::Plain),
        "a `.txt` file has no grammar of its own"
    );

    fixture.state.update(cx, |state, cx| {
        state.set_viewer_kind("notes.txt", ViewerKind::Markdown, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.view_of("notes.txt", cx).map(|(viewer, _)| viewer),
        Some(ViewerKind::Markdown),
        "the forced kind takes"
    );
    assert_eq!(
        fixture.language_of("notes.txt", cx),
        Some(FileLanguage::Markdown),
        "the source must highlight as what the forced viewer draws, not as what the extension named"
    );

    // Forcing the plain Editor back does not un-force the language: the picker offers viewers,
    // not languages, and there is no "Plain Text" entry to force it back with. That is unchanged
    // by this fix — it is `ViewerKind::forced_language` returning `None` for `Editor`, on purpose.
    fixture.state.update(cx, |state, cx| {
        state.set_viewer_kind("notes.txt", ViewerKind::Editor, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.language_of("notes.txt", cx),
        Some(FileLanguage::Markdown),
        "Editor names no language of its own, so the last forced one stands"
    );

    fixture.close("notes.txt", cx);
    fixture.state.update(cx, |state, cx| {
        state.select_file("notes.txt".to_string(), cx)
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.language_of("notes.txt", cx),
        Some(FileLanguage::Plain),
        "nothing was written down, so the reopened tab starts from the extension again"
    );
}
