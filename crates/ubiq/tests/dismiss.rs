//! Escape, over a stack of overlays.
//!
//! **The peel order is the whole of it.** Every modal in the window is raised from
//! `WorkbenchState` and answered by one handler — `AppState::cancel_dialog` — reading the paint
//! order from the top. Which surface Escape means is therefore a list, and a list is the one thing
//! worth asserting: a modal added to `ui::shell` without a rung here is a modal Escape walks past.
//!
//! A window with no project and no host, because none of the overlays in the peel order needs
//! either. The second test does open one: the file picker is the window's dialog that a project's
//! own surfaces raise, and where it is *mounted* is the other half of what makes a rung in that
//! list mean anything.

use ubiq::app::{AppState, BusHub, DialogCancel};
use ubiq::state::new_agent::OpenList;
use ubiq::state::sink::ColourField;
use ubiq::state::sink::{ProjectNav, SinkModal};
use ubiq::state::workbench::{FileDialog, ProjectSettings, ProjectSettingsMode};
use ubiq::state::{MenuId, WindowRegistry};
use ubiq_proto::work::AgentId;

#[gpui::test]
fn escape_peels_one_layer_at_a_time(cx: &mut gpui::TestAppContext) {
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

    // A stack, bottom to top, of the layers that need neither a project nor an answer from the
    // host: the sink's fixture modal, project settings, the settings page, a file question — and
    // a dropdown over all of it.
    state.update(cx, |state, cx| {
        state.sink.modal = Some(SinkModal::Confirm);
        state.workbench.project_settings = Some(ProjectSettings {
            mode: ProjectSettingsMode::Create {
                path: "/tmp/x".to_string(),
            },
            colour: ColourField::default(),
            nav: ProjectNav::General,
        });
        state.workbench.settings.open = true;
        state.workbench.confirm_end_conversation = Some(AgentId::generate());
        state.workbench.file_dialog = Some(FileDialog::New {
            parent: String::new(),
            dir: false,
        });
        state.open_menu(MenuId::SinkPicker, cx);
    });
    cx.run_until_parked();

    let escape = |state: &gpui::Entity<AppState>, cx: &mut gpui::TestAppContext| {
        handle
            .update(cx, |_, window, cx| {
                state.update(cx, |state, cx| {
                    state.cancel_dialog(&DialogCancel, window, cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    };

    // The menu goes first and takes nothing with it — that is the rule the clone modal used to be
    // alone in knowing.
    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.open_menu.is_none(), "the menu stayed down");
        assert!(
            state.workbench.file_dialog.is_some(),
            "the menu took the dialog under it"
        );
    });

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.file_dialog.is_none());
        assert!(state.workbench.settings.open, "the dialog took settings");
    });

    // The New agent modal, and its own picker above it: a list down over a form is peeled before
    // the form under it, which is the rung a picker inside a modal needs and `open_menu` cannot
    // give it.
    handle
        .update(cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                state.open_new_agent(window, cx);
                state.toggle_new_agent_list(OpenList::Target, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(
            state
                .workbench
                .new_agent
                .as_ref()
                .is_some_and(|form| form.open.is_none()),
            "the list went down"
        );
        assert!(
            state.workbench.new_agent.is_some(),
            "the list took the modal with it"
        );
    });

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.new_agent.is_none());
        assert!(state.workbench.settings.open, "the modal took settings");
    });

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(!state.workbench.settings.open);
        assert!(state.workbench.project_settings.is_some());
    });

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.project_settings.is_none());
        assert!(state.workbench.confirm_end_conversation.is_some());
    });

    // The two that had no Escape at all before one handler owned the key: the destructive
    // conversation confirm, and the sink's fixture modal under it.
    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.confirm_end_conversation.is_none());
        assert!(
            state.sink.modal.is_some(),
            "the confirm took the modal under it"
        );
    });

    escape(&state, cx);
    state.read_with(cx, |state, _| assert!(state.sink.modal.is_none()));
}

/// The other half of the same claim: **a modal is drawn where it is mounted, and the picker is
/// mounted at the window root.**
///
/// The file picker is the window's one dialog, raised by a composer's `+`, by an explorer gesture
/// and by a remote project's Open — and for a while it was painted only by the kitchen-sink page,
/// so every other caller set `AppState::file_picker` and no dialog appeared. Escape would have
/// peeled a layer nobody could see. So this asks the composer's own entry point for a picker with
/// the rail anywhere but the sink, and looks for the dialog in the frame the window drew.
#[gpui::test]
fn the_composers_picker_draws_with_the_rail_off_the_sink(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    use ubiq::state::RailMode;
    use ubiq_proto::bus::To;
    use ubiq_proto::files::{DirEntry, DirListing, EntryKind};
    use ubiq_proto::ids::ProjectId;
    use ubiq_proto::messages::Message;
    use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

    // A project, because the composer's picker is raised over the open project's explorer tree and
    // does nothing at all without one.
    let project = ProjectId::generate();
    let snapshot = ProjectSnapshot {
        record: ProjectRecord {
            id: project,
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            temporary: false,
            created_at: chrono::Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            tools: Vec::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    };

    let (hub, host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        ubiq::app::install_key_bindings(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    // The tree the picker will be built from, arriving the way the host sends one.
    host.send(
        To::Everyone,
        Message::ProjectTreeListing {
            project_id: project,
            rel_path: String::new(),
            listings: vec![DirListing {
                rel_path: String::new(),
                entries: vec![
                    DirEntry {
                        name: "crates".to_string(),
                        rel_path: "crates".to_string(),
                        kind: EntryKind::Dir,
                        size: None,
                        symlink: false,
                    },
                    DirEntry {
                        name: "README.md".to_string(),
                        rel_path: "README.md".to_string(),
                        kind: EntryKind::File,
                        size: Some(12),
                        symlink: false,
                    },
                ],
                truncated: false,
            }],
        },
    );
    cx.run_until_parked();

    // The rail is on the IDE, which is the point: the sink's page is not in the tree, so nothing
    // but the window root can be drawing this dialog.
    state.read_with(cx, |state, _| {
        assert_ne!(
            state.workbench.rail_mode,
            RailMode::Sink,
            "the sink's page would paint its own picker"
        );
    });

    let agent = AgentId::generate();
    handle
        .update(cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                state.raise_composer_picker(agent, 0, window, cx)
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    state.read_with(cx, |state, _| {
        assert!(
            state.file_picker.is_some(),
            "the composer's + never raised one"
        );
    });

    let mut vcx = gpui::VisualTestContext::from_window(handle.into(), cx);
    assert!(
        vcx.debug_bounds("file-picker").is_some(),
        "the picker is in the window's state and not in its tree — `ui::shell` lost the mount"
    );
}

/// A list going down inside the New agent form gives the keyboard back to the form.
///
/// The filter field every one of the form's lists carries holds the keyboard while that list is
/// open, and it is unmounted with the list. Leaving the focus on it left the keyboard with an
/// element nothing draws any more — and a window key dispatched from there reaches nothing, so
/// Escape stopped closing the form and ⌘⏎ stopped starting it. This drives a **real keystroke**
/// rather than `cancel_dialog`, because the whole failure was in the dispatch and not in the peel
/// order the first test asserts.
#[gpui::test]
fn a_list_going_down_gives_the_form_the_keyboard(cx: &mut gpui::TestAppContext) {
    use gpui::{AppContext as _, Focusable, VisualTestContext};
    use std::ops::Deref as _;

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

    handle
        .update(cx, |_, window, cx| {
            state.update(cx, |state, cx| state.open_new_agent(window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let mut cx = VisualTestContext::from_window(*handle.deref(), cx);
    let prompt_focused = |cx: &mut VisualTestContext| {
        handle
            .update(cx, |_, window, cx| {
                state
                    .read(cx)
                    .new_agent_prompt
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            })
            .expect("the window is open")
    };
    assert!(
        prompt_focused(&mut cx),
        "the form opens with the keyboard in the opening prompt"
    );

    // A list down, and then picked from — the filter field takes the keyboard on the way in and
    // has to hand it back on the way out.
    handle
        .update(&mut cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                state.toggle_new_agent_list(OpenList::Target, window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    handle
        .update(&mut cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                state.dismiss_new_agent_list(window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
    assert!(
        prompt_focused(&mut cx),
        "the list handed the keyboard back to the prompt"
    );

    // Which is what makes the key work: dispatched from the prompt, not from a handle nothing
    // draws.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    state.read_with(&cx, |state, _| {
        assert!(
            state.workbench.new_agent.is_none(),
            "Escape closed the form"
        );
    });
}
