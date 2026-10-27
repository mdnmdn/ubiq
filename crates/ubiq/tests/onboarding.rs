//! The onboarding wizard, at the state level.
//!
//! A run is a flow plus the flows it chains, and the tour is a sub-run launched from one of the
//! welcome's steps; the dialog is a rung of the overlay stack only while
//! it is on screen, Escape does not take it, and only its own navigation — Next past the end, or
//! Skip — closes it, marking what it walked through done.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{Entity, TestAppContext, WindowHandle};
use ubiq::app::{AppState, BusHub, DialogCancel};
use ubiq::state::WindowRegistry;
use ubiq::state::onboarding::{StepBody, TOUR, WELCOME};
use ubiq::state::overlay::Layer;
use ubiq::state::ui_id::MarkRect;

type Window = WindowHandle<gpui_component::Root>;

fn window(cx: &mut TestAppContext) -> (Entity<AppState>, Window) {
    use gpui::AppContext as _;

    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        ubiq::app::install_key_bindings(cx);
    });
    let held: Rc<RefCell<Option<Entity<AppState>>>> = Default::default();
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
    // No host answers in a test; stand in for the interface preferences having arrived.
    state.update(cx, |state, _| state.workbench.prefs_loaded = true);
    (state, handle)
}

fn escape(state: &Entity<AppState>, handle: &Window, cx: &mut TestAppContext) {
    handle
        .update(cx, |_, window, cx| {
            state.update(cx, |state, cx| {
                state.cancel_dialog(&DialogCancel, window, cx)
            });
        })
        .expect("the window is open");
    cx.run_until_parked();
}

fn next(state: &Entity<AppState>, cx: &mut TestAppContext) {
    state.update(cx, |state, cx| state.onboarding_next(cx));
    cx.run_until_parked();
}

#[gpui::test]
fn the_dialog_is_the_top_layer_and_escape_leaves_it(cx: &mut TestAppContext) {
    let (state, handle) = window(cx);
    state.update(cx, |state, cx| state.open_onboarding(WELCOME, cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert_eq!(state.top_layer(), Some(Layer::Onboarding));
        assert!(state.covered(Layer::Notifications));
    });

    escape(&state, &handle, cx);
    state.read_with(cx, |state, _| {
        let run = state
            .workbench
            .onboarding
            .as_ref()
            .expect("Escape left it open");
        assert_eq!(run.index, 0);
    });
}

/// The welcome's step that stands for the tour.
const TOUR_STEP: usize = 2;

fn to_tour_step(state: &Entity<AppState>, cx: &mut TestAppContext) {
    for _ in 0..TOUR_STEP {
        next(state, cx);
    }
    state.read_with(cx, |state, _| {
        let run = state.workbench.onboarding.as_ref().unwrap();
        assert_eq!(run.index, TOUR_STEP);
        assert!(matches!(
            run.current().body,
            StepBody::SubFlow { flow: TOUR, .. }
        ));
    });
}

fn start_tour(state: &Entity<AppState>, cx: &mut TestAppContext) {
    state.update(cx, |state, cx| state.onboarding_start_sub(cx));
    cx.run_until_parked();
}

#[gpui::test]
fn next_and_back_move_and_the_tour_is_one_step(cx: &mut TestAppContext) {
    let (state, _handle) = window(cx);
    state.update(cx, |state, cx| state.open_onboarding(WELCOME, cx));
    cx.run_until_parked();
    state.read_with(cx, |state, cx| {
        let run = state.workbench.onboarding.as_ref().unwrap();
        assert_eq!(run.flows, vec![WELCOME]);
        assert!(run.steps.iter().all(|(flow, _)| *flow == WELCOME));
        assert_eq!(
            run.steps
                .iter()
                .filter(|(_, step)| matches!(step.body, StepBody::SubFlow { .. }))
                .count(),
            1,
            "the tour is one step of the welcome"
        );
        assert!(!state.onboarding_back_open(cx), "no Back on the first step");
    });

    next(&state, cx);
    state.read_with(cx, |state, _| {
        assert_eq!(state.workbench.onboarding.as_ref().unwrap().index, 1)
    });
    state.update(cx, |state, cx| state.onboarding_back(cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert_eq!(state.workbench.onboarding.as_ref().unwrap().index, 0)
    });
}

#[gpui::test]
fn a_highlight_step_takes_no_rung_and_lets_escape_through(cx: &mut TestAppContext) {
    let (state, handle) = window(cx);
    state.update(cx, |state, cx| state.open_onboarding(WELCOME, cx));
    // The tour's first step is a highlight.
    to_tour_step(&state, cx);
    start_tour(&state, cx);
    state.read_with(cx, |state, _| {
        let run = state.workbench.onboarding.as_ref().unwrap();
        assert!(run.hidden());
        assert_eq!(state.top_layer(), None);
    });
    escape(&state, &handle, cx);
    state.read_with(cx, |state, _| {
        assert!(
            state.workbench.onboarding.is_some(),
            "a hidden run stays open"
        );
    });
}

#[gpui::test]
fn the_tour_is_a_sub_run_that_hands_back(cx: &mut TestAppContext) {
    let (state, _handle) = window(cx);
    state.update(cx, |state, cx| state.open_onboarding(WELCOME, cx));
    to_tour_step(&state, cx);
    start_tour(&state, cx);
    let tour_len = state.read_with(cx, |state, cx| {
        let run = state.workbench.onboarding.as_ref().unwrap();
        assert!(run.is_sub());
        assert_eq!(run.flows, vec![TOUR]);
        assert_eq!(run.progress().0, 0, "the tour keeps its own progress");
        assert!(state.onboarding_back_open(cx), "Back backs out of the tour");
        run.steps.len()
    });
    assert!(tour_len > 1);

    // Back from the tour's first step: the welcome, on the tour's step.
    state.update(cx, |state, cx| state.onboarding_back(cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        let run = state.workbench.onboarding.as_ref().unwrap();
        assert!(!run.is_sub());
        assert_eq!(run.index, TOUR_STEP);
    });

    // Through to the end: the tour is done and the welcome moves past its step.
    start_tour(&state, cx);
    for _ in 0..tour_len {
        next(&state, cx);
    }
    state.read_with(cx, |state, _| {
        let run = state.workbench.onboarding.as_ref().unwrap();
        assert!(!run.is_sub());
        assert_eq!(run.index, TOUR_STEP + 1);
        assert!(state.workbench.onboarding_done.contains(TOUR));
        assert!(!state.workbench.onboarding_done.contains(WELCOME));
    });

    // Skip inside the tour skips only the tour.
    state.update(cx, |state, cx| state.onboarding_back(cx));
    start_tour(&state, cx);
    next(&state, cx);
    state.update(cx, |state, cx| state.skip_onboarding(cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        let run = state
            .workbench
            .onboarding
            .as_ref()
            .expect("the welcome is still open");
        assert!(!run.is_sub());
        assert_eq!(run.index, TOUR_STEP + 1);
    });
}

#[gpui::test]
fn finishing_marks_every_flow_of_the_run_done(cx: &mut TestAppContext) {
    let (state, _handle) = window(cx);
    state.update(cx, |state, cx| state.open_onboarding(WELCOME, cx));
    let len = state.read_with(cx, |state, _| {
        state.workbench.onboarding.as_ref().unwrap().steps.len()
    });
    state.read_with(cx, |state, _| {
        assert!(!state.workbench.onboarding.as_ref().unwrap().is_last())
    });
    for _ in 0..len {
        next(&state, cx);
    }
    state.read_with(cx, |state, _| {
        assert!(state.workbench.onboarding.is_none(), "Finish closed it");
        assert!(state.workbench.onboarding_done.contains(WELCOME));
        assert!(
            !state.workbench.onboarding_done.contains(TOUR),
            "Next passed the tour's step without running it"
        );
    });
}

#[gpui::test]
fn skip_marks_the_run_done_and_a_manual_open_still_works(cx: &mut TestAppContext) {
    let (state, _handle) = window(cx);
    state.update(cx, |state, cx| state.open_onboarding(WELCOME, cx));
    state.update(cx, |state, cx| state.skip_onboarding(cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert!(state.workbench.onboarding.is_none());
        assert!(state.workbench.onboarding_done.contains(WELCOME));
    });

    state.update(cx, |state, cx| state.open_onboarding(WELCOME, cx));
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert_eq!(state.top_layer(), Some(Layer::Onboarding));
    });
}

#[gpui::test]
fn nothing_starts_before_the_preferences_answer(cx: &mut TestAppContext) {
    let (state, _handle) = window(cx);
    state.update(cx, |state, cx| {
        state.workbench.prefs_loaded = false;
        state.check_onboarding(cx);
        // Nor by hand: a run finished now would save a done set missing what is stored.
        state.open_onboarding(WELCOME, cx);
    });
    state.read_with(cx, |state, _| assert!(state.workbench.onboarding.is_none()));
}

#[test]
fn the_callout_stays_on_screen() {
    use ubiq::ui::onboarding::place_callout;
    let (vw, vh, width) = (1200., 800., 300.);
    let room = ubiq::theme::onboarding_callout_room();
    let gap = ubiq::theme::help_balloon_gap();
    // Beside a narrow target on the left.
    let (left, _) = place_callout(Some(MarkRect::new(0., 40., 48., 700.)), width, vw, vh);
    assert_eq!(left, 48. + gap);
    // Above a full-width strip at the foot.
    let (_, top) = place_callout(Some(MarkRect::new(0., 776., 1200., 24.)), width, vw, vh);
    assert_eq!(top, 776. - room - gap);
    // Centred when the target is not on screen.
    let (left, _) = place_callout(None, width, vw, vh);
    assert_eq!(left, (vw - width) / 2.);
    // Inside the window for a target covering all of it.
    let (left, top) = place_callout(Some(MarkRect::new(0., 0., vw, vh)), width, vw, vh);
    assert!(left >= gap && left + width <= vw && top >= gap && top + room <= vh);
}
