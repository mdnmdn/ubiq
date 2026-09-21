//! The multi-select picker: what a closed one says, what order its rows come in, and what a pick
//! does to the menu it was made in.
//!
//! `kit::MultiPicker` is a `RenderOnce` and holds nothing, so the two decisions worth asserting are
//! the two free functions behind it — `multi_label` and `multi_order` — plus the one claim that
//! separates it from `kit::Picker`: **a pick toggles and leaves the list down**. The last is a
//! statement about the caller's mutator, not about pixels, which is why it is driven through
//! `AppState` like every other test in this directory.
//!
//! What only a frame can catch is caught by `sink.rs`'s `every_page_draws_in_a_window_with_no_project`,
//! which draws the style reference with the specimen on it: the elision, the hover tooltip and the
//! outside-click dismissal are compile- and render-checked there, never asserted here.

use gpui::SharedString;
use ubiq::ui::kit::{multi_label, multi_order};

fn items(labels: &[&str]) -> Vec<SharedString> {
    labels.iter().map(|s| SharedString::from(*s)).collect()
}

// ── the closed label ────────────────────────────────────────────────

/// What the trigger says is every ticked value, comma-separated. The same string is what
/// `kit::elided` hangs on the hover, so the truncated label and the full list are one value and
/// cannot disagree.
#[test]
fn the_closed_label_is_the_ticked_values_comma_separated() {
    let list = items(&["Claude Code", "Codex", "Gemini CLI", "opencode"]);
    let placeholder = SharedString::from("nothing chosen");

    assert_eq!(
        multi_label(&list, &[0, 2], &placeholder).as_ref(),
        "Claude Code, Gemini CLI"
    );
    assert_eq!(
        multi_label(&list, &[0, 1, 2, 3], &placeholder).as_ref(),
        "Claude Code, Codex, Gemini CLI, opencode"
    );
}

/// Nothing ticked reads as the placeholder, which for a filter is what an empty selection *shows*
/// rather than the word "none".
#[test]
fn nothing_ticked_reads_as_the_placeholder() {
    let list = items(&["running", "waiting"]);
    let placeholder = SharedString::from("all states");

    assert_eq!(multi_label(&list, &[], &placeholder).as_ref(), "all states");
}

/// The label is in the list's order, not in the order the user ticked — a summary that reshuffles
/// as it grows is one nobody can read twice. An index naming no row is dropped rather than
/// panicking: the selection and the list are two values, and a frame can arrive holding an old one.
#[test]
fn the_label_follows_the_list_and_ignores_indices_that_name_nothing() {
    let list = items(&["alpha", "beta", "gamma"]);
    let placeholder = SharedString::from("none");

    assert_eq!(
        multi_label(&list, &[2, 0], &placeholder).as_ref(),
        "alpha, gamma"
    );
    assert_eq!(
        multi_label(&list, &[1, 9], &placeholder).as_ref(),
        "beta",
        "an index past the end is dropped"
    );
}

// ── the order rows are drawn in ─────────────────────────────────────

/// With the field empty, the ticked rows come first — in the list's own order, and so does
/// everything left.
#[test]
fn an_empty_query_draws_the_ticked_rows_first() {
    assert_eq!(multi_order(5, &[3, 1], Some("")), vec![1, 3, 0, 2, 4]);
    assert_eq!(multi_order(3, &[], Some("")), vec![0, 1, 2]);
    assert_eq!(multi_order(3, &[0, 1, 2], Some("")), vec![0, 1, 2]);
}

/// Once something is typed the list is the search result in the search's own order: a ticked row
/// lifted above better matches reads as a match it is not.
#[test]
fn a_query_leaves_the_search_result_in_its_own_order() {
    assert_eq!(multi_order(5, &[3, 1], Some("co")), vec![0, 1, 2, 3, 4]);
}

/// A list with no search field has no query to be empty, and keeps the order the caller chose —
/// the Teams states filter, whose four rows would otherwise jump under the pointer as they are
/// ticked.
#[test]
fn a_list_with_no_search_field_is_never_reordered() {
    assert_eq!(multi_order(4, &[2, 3], None), vec![0, 1, 2, 3]);
}

// ── a pick toggles, and the menu stays down ─────────────────────────

/// The difference between this control and `kit::Picker`, asserted on the style reference's own
/// specimen: ticking adds, ticking again removes, and neither closes the menu.
///
/// The preselection is the same field the trigger reads back, so what goes in comes out — which is
/// what lets a form hand it a record's current values and a filter hand it the screen's.
#[gpui::test]
fn ticking_adds_and_removes_without_closing_the_menu(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    use ubiq::app::{AppState, BusHub};
    use ubiq::state::{MenuId, RailMode, WindowRegistry};

    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let _window = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();

    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    state.update(cx, |state, cx| state.set_rail_mode(RailMode::Sink, cx));
    cx.run_until_parked();

    // What the specimen is preselected with, before anything is clicked.
    state.read_with(cx, |state, _| {
        assert_eq!(
            state.sink.multi,
            vec![0, 2],
            "the preselection is the value"
        );
    });

    state.update(cx, |state, cx| state.open_menu(MenuId::SinkMulti, cx));
    cx.run_until_parked();

    state.update(cx, |state, cx| state.toggle_sink_multi(1, cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert_eq!(state.sink.multi, vec![0, 2, 1], "ticking adds");
        assert_eq!(
            state.workbench.open_menu,
            Some(MenuId::SinkMulti),
            "the list is still down"
        );
    });

    state.update(cx, |state, cx| state.toggle_sink_multi(0, cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert_eq!(state.sink.multi, vec![2, 1], "ticking again removes");
        assert_eq!(
            state.workbench.open_menu,
            Some(MenuId::SinkMulti),
            "and still does not close it"
        );
    });

    // Dismissal is the caller's, exactly as it is for every other menu in the window.
    state.update(cx, |state, cx| state.close_menu(cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert_eq!(state.workbench.open_menu, None);
        assert_eq!(state.sink.multi, vec![2, 1], "closing keeps the selection");
    });
}
