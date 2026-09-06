//! Escape, over a stack of overlays.
//!
//! **The peel order is the whole of it.** Every modal in the window is raised from
//! `WorkbenchState` and answered by one handler — `AppState::cancel_dialog` — reading the paint
//! order from the top. Which surface Escape means is therefore a list, and a list is the one thing
//! worth asserting: a modal added to `ui::shell` without a rung here is a modal Escape walks past.
//!
//! A window with no project and no host, because none of these overlays needs either.

use ubiq::app::{AppState, BusHub, DialogCancel};
use ubiq::state::sink::ColourField;
use ubiq::state::sink::SinkModal;
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
