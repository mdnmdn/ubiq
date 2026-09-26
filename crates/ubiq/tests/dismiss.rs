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
use ubiq::state::sink::{ColourField, DroneField};
use ubiq::state::sink::{ProjectNav, SinkModal};
use ubiq::state::workbench::{FileDialog, ProjectSettings, ProjectSettingsMode};
use ubiq::state::{MenuId, WindowRegistry};
use ubiq_proto::ids::PaneId;
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
            drone: DroneField::default(),
            nav: ProjectNav::default(),
            definitions_use_global: true,
        });
        state.workbench.settings.open = true;
        state.workbench.confirm_end_conversation = Some(AgentId::generate());
        state.conversation_info = Some(AgentId::generate());
        state.workbench.file_dialog = Some(FileDialog::New {
            parent: String::new(),
            dir: false,
            ext: None,
        });
        state.workbench.confirm_close_pane = Some(PaneId::generate());
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
            state.workbench.confirm_end_conversation.is_some(),
            "the menu took the confirm under it"
        );
    });

    // The two destructive closes, both painted at the window root just over the file question and
    // in reverse paint order: the conversation's is drawn after the pane's, so Escape peels it
    // first. They sit this high because Escape is the answer a destructive confirm should be
    // easiest of all to give.
    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.confirm_end_conversation.is_none());
        assert!(
            state.workbench.confirm_close_pane.is_some(),
            "the conversation's confirm took the pane's under it"
        );
    });

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.confirm_close_pane.is_none());
        assert!(
            state.workbench.file_dialog.is_some(),
            "the confirm took the dialog under it"
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
        assert!(state.conversation_info.is_some());
    });

    // The conversation's Info panel: still raised from a dock panel rather than the window root,
    // because it reads the live conversation — so it sits under everything above, next to the
    // sink's fixture modal that had no Escape at all before one handler owned the key.
    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.conversation_info.is_none());
        assert!(
            state.sink.modal.is_some(),
            "the info panel took the modal under it"
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
            storage: Default::default(),
            temporary: false,
            created_at: chrono::Utc::now(),
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
            RailMode::SINK,
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

/// The other gesture over the same stack: **an outside click belongs to the topmost layer.**
///
/// `on_mouse_down_out` is a capture-phase handler over one panel's own bounds, so a click inside a
/// layer painted above it reads to every layer below as a click outside *them* — left alone, one
/// click peels the whole stack. Every such dismissal asks `AppState::covered` first, and that
/// answer is what this asserts: the rung order in `state::overlay`, read the way an outside click
/// reads it, rather than each pair of overlays hard-coding the other.
#[gpui::test]
fn an_outside_click_in_a_higher_layer_leaves_the_layer_under_it_up(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    use ubiq::state::Layer;
    use ubiq::state::kb::{KbList, KbSourceForm};

    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let _handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    // Nothing up: no layer is covered, so every dismissal still answers its own outside click.
    state.read_with(cx, |state, _| {
        assert!(state.top_layer().is_none());
        assert!(!state.covered(Layer::ProjectSettings));
    });

    // The triple this rule was found on: the project settings page, the "Add source" modal over
    // it, and one of that modal's own lists over both.
    state.update(cx, |state, _| {
        state.workbench.project_settings = Some(ProjectSettings {
            mode: ProjectSettingsMode::Create {
                path: "/tmp/x".to_string(),
            },
            colour: ColourField::default(),
            drone: DroneField::default(),
            nav: ProjectNav::default(),
            definitions_use_global: true,
        });
        state.workbench.kb_source = Some(KbSourceForm::default());
    });
    state.read_with(cx, |state, _| {
        assert!(
            state.covered(Layer::ProjectSettings),
            "a click in the modal over the page would have closed the page under it"
        );
        assert!(
            !state.covered(Layer::KbSource),
            "the modal on top still answers its own outside click"
        );
    });

    state.update(cx, |state, _| {
        if let Some(form) = state.workbench.kb_source.as_mut() {
            form.open = Some(KbList::Kind);
        }
    });
    state.read_with(cx, |state, _| {
        assert!(
            state.covered(Layer::KbSource),
            "a click in the list is the list's, not the modal's under it"
        );
        assert!(state.covered(Layer::ProjectSettings));
    });

    // And the pair the hand-patched triple never knew about: the settings page under a file
    // question, which is the same hazard one rung further up the stack.
    state.update(cx, |state, _| {
        state.workbench.project_settings = None;
        state.workbench.kb_source = None;
        state.workbench.settings.open = true;
    });
    state.read_with(cx, |state, _| assert!(!state.covered(Layer::Settings)));
    state.update(cx, |state, _| {
        state.workbench.file_dialog = Some(FileDialog::New {
            parent: String::new(),
            dir: false,
            ext: None,
        });
    });
    state.read_with(cx, |state, _| {
        assert!(
            state.covered(Layer::Settings),
            "a click in the file question would have taken the page under it"
        );
        assert!(!state.covered(Layer::FileDialog));
    });
}

/// In-place help is the top rung, above the menus and the modals both.
///
/// The mode is deliberately able to cover a dialog and point at its controls — that is what makes
/// it an inspector rather than another modal — so Escape has to mean "stop pointing" while it is
/// up, before the menu it was drawn over. The rest of the stack has to be exactly where it was
/// when the mode closes: pointing at a dialog must not cost the reader the dialog.
#[gpui::test]
fn escape_takes_in_place_help_before_anything_under_it(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    use ubiq::state::overlay::Layer;

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
        state.workbench.settings.open = true;
        state.open_menu(MenuId::SinkPicker, cx);
        state.open_help_target(cx);
    });
    cx.run_until_parked();

    state.read_with(cx, |state, _| {
        assert_eq!(state.top_layer(), Some(Layer::HelpTarget));
        assert!(
            state.covered(Layer::Menu),
            "a click in the targeting overlay is not the menu's"
        );
    });

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

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.help_target.is_none(), "the mode is down");
        assert_eq!(
            state.workbench.open_menu,
            Some(MenuId::SinkPicker),
            "and it took neither the menu"
        );
        assert!(state.workbench.settings.open, "nor the page under it");
    });

    // From there the stack peels in its usual order.
    escape(&state, cx);
    state.read_with(cx, |state, _| assert!(state.workbench.open_menu.is_none()));
    escape(&state, cx);
    state.read_with(cx, |state, _| assert!(!state.workbench.settings.open));
}

/// The pointer's position is the mode's, and only the mode's.
#[gpui::test]
fn the_cursor_is_only_remembered_while_the_mode_is_up(cx: &mut gpui::TestAppContext) {
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
    let _handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    // A move with the mode down raises nothing: a stray pointer is not a gesture.
    state.update(cx, |state, cx| state.move_help_target(10., 10., cx));
    state.read_with(cx, |state, _| {
        assert!(state.workbench.help_target.is_none())
    });

    // Up, and it opens with no cursor — so the overlay shows its hint rather than a rectangle
    // around whatever happens to be at the window origin.
    state.update(cx, |state, cx| state.open_help_target(cx));
    state.read_with(cx, |state, _| {
        assert_eq!(state.workbench.help_target.map(|m| m.cursor), Some(None));
    });

    state.update(cx, |state, cx| state.move_help_target(120., 48., cx));
    state.read_with(cx, |state, _| {
        assert_eq!(
            state.workbench.help_target.and_then(|m| m.cursor),
            Some((120., 48.))
        );
    });

    // Asking again while it is up keeps the cursor it already had: a second ⇧F1 must not blank
    // the highlight the reader is looking at.
    state.update(cx, |state, cx| state.open_help_target(cx));
    state.read_with(cx, |state, _| {
        assert_eq!(
            state.workbench.help_target.and_then(|m| m.cursor),
            Some((120., 48.))
        );
    });

    // And closing forgets it, because the type does not let the mode be down with a cursor.
    state.update(cx, |state, cx| state.close_help_target(cx));
    state.read_with(cx, |state, _| {
        assert!(state.workbench.help_target.is_none())
    });
}

/// The mission full view's modal takes a rung of its own, under the plan and the file question.
///
/// A modal raised in `ui::shell` without one is a modal Escape walks past, which is what this
/// file exists to catch — and the mission's is the one whose *Plan & docs* tab raises the plan
/// surface over it, so the pair's order is the part worth asserting.
#[gpui::test]
fn escape_peels_the_mission_full_view(cx: &mut gpui::TestAppContext) {
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

    let task_id = ubiq_proto::ids::TaskId::generate();
    state.update(cx, |state, cx| {
        state.open_mission_modal(task_id, cx);
        state.workbench.file_dialog = Some(FileDialog::New {
            parent: String::new(),
            dir: false,
            ext: None,
        });
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

    escape(&state, cx);
    state.read_with(cx, |state, _| {
        assert!(state.workbench.file_dialog.is_none());
        assert_eq!(
            state.workbench.mission,
            Some(task_id),
            "the file question took the mission under it"
        );
    });

    escape(&state, cx);
    state.read_with(cx, |state, _| assert!(state.workbench.mission.is_none()));
}

/// *Open as tab* is a move, not a copy: the modal goes and a centre-region document takes its
/// place, so the mission is never on screen twice.
#[gpui::test]
fn open_as_tab_moves_the_mission_into_the_centre(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    use ubiq::state::dock::{PanelClass, PanelKind};

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
    let _handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    let task_id = ubiq_proto::ids::TaskId::generate();
    state.update(cx, |state, cx| {
        state.open_mission_modal(task_id, cx);
        state.open_mission_tab(task_id, cx);
    });
    cx.run_until_parked();

    state.read_with(cx, |state, _| {
        assert!(
            state.workbench.mission.is_none(),
            "the modal stayed up beside the document"
        );
    });
    // The document's class is what puts it beside the open files rather than in a side region.
    assert_eq!(
        PanelKind::MissionView(task_id).class(),
        PanelClass::Centre,
        "the full view is a centre-region document"
    );
}
