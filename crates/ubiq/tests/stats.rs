//! The Control screen: where it sits in the rail, and the two pages it holds.
//!
//! **Neither needs a frame.** What the screen claims structurally is that it belongs to the
//! application rather than to a project — the host is one process behind every window, so its
//! memory and its uptime are true with an empty catalogue — and that is a fact about the rail's
//! grouping, not about anything drawn.
//!
//! The tabs are checked the way the sink's pages are: the strip prints a label and a note for each
//! one, so a variant that lost either would print nothing where something belongs.

use ubiq::state::RailMode;
use ubiq::state::stats::{StatsState, StatsTab};

/// Control reports on the host, which runs whether or not this window has a folder open. A window
/// with an empty catalogue can open it, which is only true while it is grouped with the
/// destinations that need nothing open.
#[test]
fn control_is_an_app_destination_and_never_a_project_one() {
    let groups = RailMode::groups();

    let app = groups
        .iter()
        .find(|(label, _)| *label == "APP")
        .expect("the APP group");
    assert!(app.1.contains(&RailMode::Control), "Control left APP");

    for (label, modes) in groups {
        if *label != "APP" {
            assert!(
                !modes.contains(&RailMode::Control),
                "Control is also under {label}"
            );
        }
    }
}

/// Two pages, each with a label the strip prints and a note beside it. The copy is a const array
/// indexed by variant, so a variant added without a row would be the one this catches.
#[test]
fn every_page_says_what_it_is_and_what_it_is_for() {
    assert_eq!(StatsTab::all(), &[StatsTab::General, StatsTab::Ai]);
    assert_eq!(StatsTab::General.label(), "General");
    assert_eq!(StatsTab::Ai.label(), "AI");

    for tab in StatsTab::all() {
        assert!(!tab.label().is_empty(), "{tab:?}");
        assert!(!tab.note().is_empty(), "{tab:?}");
    }
}

/// The screen opens on General with nothing read yet. `None` rather than a default reading: a zero
/// the host never sent would be drawn as a fact it never stated.
#[test]
fn nothing_is_known_until_the_host_has_answered() {
    let stats = StatsState::default();
    assert_eq!(stats.tab, StatsTab::General);
    assert!(stats.host.is_none());
    assert!(!stats.polling);
}
