//! The log sink, driven through the same door every subsystem uses.
//!
//! Nothing here reaches into the ring: the test emits `tracing` events exactly as a subsystem
//! does, and reads them back the way the console does. One process, one global subscriber, so the
//! whole file is a single test.

use ubiq::log::{Filter, LogLevel, Subsystem, logs};
use ubiq::state::LogState;

#[test]
fn events_are_classified_filtered_and_read_back() {
    unsafe { std::env::remove_var("RUST_LOG") };
    ubiq::log::install();
    logs().clear();

    tracing::debug!(target: "ubiq::pty", "opened a pseudo-terminal");
    tracing::info!(target: "ubiq::orchestrator", "started a harness");
    tracing::warn!(target: "ubiq::ui::terminal", "the pane fell behind");
    tracing::error!(target: "agent_manager::provision", "the config directory is gone");
    tracing::error!(target: "wgpu_hal::metal", "no device");

    let everything = Filter {
        subsystem: None,
        min_level: LogLevel::Trace,
    };
    let all = logs().snapshot(everything);
    assert_eq!(all.len(), 5, "every event reaches the ring");

    // A target is a module path, and that is the whole classification rule.
    let subsystems: Vec<Subsystem> = all.iter().map(|record| record.subsystem).collect();
    assert_eq!(
        subsystems,
        vec![
            Subsystem::Pty,
            Subsystem::Coordinator,
            Subsystem::Ui,
            Subsystem::Harness,
            Subsystem::External,
        ]
    );

    // The message survives the trip, and the ring is in the order things happened.
    assert_eq!(all[0].message, "opened a pseudo-terminal");
    assert!(all[0].seq < all[4].seq);

    // The level filter is a floor, not a set.
    let loud = logs().snapshot(Filter {
        subsystem: None,
        min_level: LogLevel::Warn,
    });
    assert_eq!(loud.len(), 3);

    // The subsystem selector narrows to one, and the two filters compose.
    let harness = logs().snapshot(Filter {
        subsystem: Some(Subsystem::Harness),
        min_level: LogLevel::Trace,
    });
    assert_eq!(harness.len(), 1);
    assert_eq!(harness[0].level, LogLevel::Error);

    let quiet_harness = logs().snapshot(Filter {
        subsystem: Some(Subsystem::Ui),
        min_level: LogLevel::Error,
    });
    assert!(quiet_harness.is_empty());

    // The console's own state answers the pickers on the same indexing it draws them with.
    let mut state = LogState::default();
    assert_eq!(state.subsystem_label(), "All");
    state.pick_subsystem(3);
    assert_eq!(state.subsystem, Some(Subsystem::Pty));
    assert_eq!(state.subsystem_index(), 3);
    state.pick_subsystem(0);
    assert_eq!(state.filter().subsystem, None);
    state.pick_level(4);
    assert_eq!(state.min_level, LogLevel::Error);

    // The loudest level is what the dock's tab reports without the console being on screen.
    assert_eq!(logs().loudest(), Some(LogLevel::Error));

    // A console is nudged when a record arrives, and told nothing about what it was.
    let nudges = logs().subscribe();
    tracing::error!(target: "ubiq::app", "something to wake a window with");
    assert!(nudges.try_recv().is_ok());

    // Clearing empties the ring for every window at once, and wakes them to say so.
    logs().clear();
    assert!(nudges.try_recv().is_ok());
    let (kept, dropped) = logs().counts();
    assert_eq!((kept, dropped), (0, 0));
    assert_eq!(
        logs().loudest(),
        None,
        "a cleared ring holds nothing to report"
    );
}
