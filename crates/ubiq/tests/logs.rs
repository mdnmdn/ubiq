//! The log console's own state: the two pickers, on the same indexing the console draws them with.
//!
//! The sink itself is `ubiq-proto`'s and is tested there. What is left here is the half that
//! belongs to the window — which subsystem and which level the console is filtered to — and it is
//! pure, so it needs neither a frame nor a subscriber.

use ubiq::state::LogState;
use ubiq_proto::log::{LogLevel, Subsystem};

#[test]
fn the_console_pickers_answer_on_the_indexing_they_are_drawn_with() {
    let mut state = LogState::default();
    assert_eq!(state.subsystem_label(), "All");

    state.pick_subsystem(3);
    assert_eq!(state.subsystem, Some(Subsystem::Pty));
    assert_eq!(state.subsystem_index(), 3);

    // Index zero is "All", which is an absent filter rather than a subsystem.
    state.pick_subsystem(0);
    assert_eq!(state.filter().subsystem, None);
    assert_eq!(state.subsystem_index(), 0);

    state.pick_level(4);
    assert_eq!(state.min_level, LogLevel::Error);
    assert_eq!(state.filter().min_level, LogLevel::Error);
}
