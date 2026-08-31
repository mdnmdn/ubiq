//! The log sink, driven through the same door every subsystem uses.
//!
//! Nothing here reaches into the ring: the test emits `tracing` events exactly as a subsystem
//! does, and reads them back the way the console does. One process, one global subscriber, so the
//! whole file is a single test.

use ubiq_proto::log::{Filter, LogLevel, Subsystem, logs};

#[test]
fn events_are_classified_filtered_and_read_back() {
    unsafe { std::env::remove_var("RUST_LOG") };
    ubiq_proto::log::install();
    logs().clear();

    // These targets are the crate-qualified module paths records actually arrive with. They are
    // spelled out because the classification is by prefix: a crate rename that is not mirrored in
    // `Subsystem::of` files every host record under `External` and compiles perfectly.
    tracing::debug!(target: "ubiq_host::pty", "opened a pseudo-terminal");
    tracing::info!(target: "ubiq_host::coordinator", "started a harness");
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

    // The bus is the coordinator's, and it now lives in this crate rather than beside it.
    logs().clear();
    tracing::info!(target: "ubiq_proto::bus", "a client attached");
    let bus = logs().snapshot(everything);
    assert_eq!(bus[0].subsystem, Subsystem::Coordinator);

    logs().clear();
    tracing::debug!(target: "ubiq_host::pty", "opened a pseudo-terminal");
    tracing::info!(target: "ubiq_host::coordinator", "started a harness");
    tracing::warn!(target: "ubiq::ui::terminal", "the pane fell behind");
    tracing::error!(target: "agent_manager::provision", "the config directory is gone");
    tracing::error!(target: "wgpu_hal::metal", "no device");
    let all = logs().snapshot(everything);

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
