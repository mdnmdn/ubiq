//! The ⌘N clipboard question: Escape takes the text file.
//!
//! With no project on screen there is no tab to open, so what is asserted is the peel — the
//! question goes away and nothing else moves. The tab halves live in `app::clipboard`'s own
//! unit tests, which need no window.

use ubiq::app::{AppState, BusHub, DialogCancel};
use ubiq::state::{FileDialog, WindowRegistry};

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
