//! The window registry, driven without a window.
//!
//! Everything the project picker's three groups do is decided here, and none of it needs a frame:
//! a `WindowId` is just a key. This is the half of the workbench that can be tested at all.

use gpui::WindowId;
use ubiq::state::windows::WindowRegistry;

/// Distinct, valid window keys. The low word is a version and is normalised to odd, so the
/// distinguishing part has to go in the high word.
fn id(n: u64) -> WindowId {
    WindowId::from((n << 32) | 1)
}

fn registry() -> WindowRegistry {
    WindowRegistry::default()
}

#[test]
fn windows_are_named_by_the_first_free_letter() {
    let mut registry = registry();
    assert_eq!(registry.next_label(), 'A');

    registry.register(id(1), 'A', 0);
    assert_eq!(registry.next_label(), 'B');

    registry.register(id(2), 'B', 1);
    assert_eq!(registry.next_label(), 'C');

    // A closed window gives its letter back, so the names stay as short as the set of windows.
    registry.unregister(id(1));
    assert_eq!(registry.next_label(), 'A');
}

#[test]
fn a_project_is_open_in_one_window_at_a_time() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.register(id(2), 'B', 1);

    // B takes A's project. A is left with nothing, so its slot goes and its ID comes back for the
    // caller to close.
    let emptied = registry.open_in(id(2), 0);

    assert_eq!(emptied, vec![id(1)]);
    assert!(registry.slot(id(1)).is_none());
    assert_eq!(registry.holder(0).map(|w| w.label), Some('B'));
    assert_eq!(registry.slot(id(2)).unwrap().projects, vec![1, 0]);
    // What a window takes is what it is pointed at.
    assert_eq!(registry.slot(id(2)).unwrap().active_project(), Some(0));
}

#[test]
fn a_window_keeping_a_project_is_not_closed() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.open_in(id(1), 1);
    registry.register(id(2), 'B', 2);

    let emptied = registry.open_in(id(2), 1);

    assert!(emptied.is_empty());
    assert_eq!(registry.slot(id(1)).unwrap().projects, vec![0]);
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(0));
}

#[test]
fn closing_the_last_project_closes_the_window() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.open_in(id(1), 1);

    assert!(registry.close(id(1), 1).is_empty());
    assert_eq!(registry.close(id(1), 0), vec![id(1)]);
    assert!(registry.slot(id(1)).is_none());
}

#[test]
fn a_closed_window_returns_its_projects_to_history() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.open_in(id(1), 1);

    registry.unregister(id(1));

    let groups = registry.groups(id(2), "");
    assert!(groups.here.is_empty());
    assert!(groups.elsewhere.is_empty());
    assert_eq!(groups.history.len(), registry.projects.len());
}

#[test]
fn the_three_groups_say_where_everything_is() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.register(id(2), 'B', 1);

    let groups = registry.groups(id(1), "");

    assert_eq!(groups.here, vec![0]);
    assert_eq!(groups.elsewhere, vec![(1, 'B', id(2))]);
    assert_eq!(groups.history, vec![2, 3, 4]);

    // The same registry, read from the other window, swaps the first two groups over.
    let groups = registry.groups(id(2), "");
    assert_eq!(groups.here, vec![1]);
    assert_eq!(groups.elsewhere, vec![(0, 'A', id(1))]);
}

#[test]
fn moving_a_project_moves_its_row_between_groups() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.register(id(2), 'B', 1);
    registry.open_in(id(2), 2);

    assert_eq!(registry.groups(id(1), "").elsewhere.len(), 2);

    registry.open_in(id(1), 2);

    let groups = registry.groups(id(1), "");
    assert_eq!(groups.here, vec![0, 2]);
    assert_eq!(groups.elsewhere, vec![(1, 'B', id(2))]);
    assert!(!groups.history.contains(&2));
}

#[test]
fn the_filter_matches_name_and_path_across_all_three_groups() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.register(id(2), 'B', 1);

    // A path fragment finds a project the same way its name does.
    let by_path = registry.groups(id(1), "~/dev/hire");
    assert!(by_path.here.is_empty());
    assert!(by_path.elsewhere.is_empty());
    assert_eq!(by_path.history.len(), 1);

    let by_name = registry.groups(id(1), "ubiq");
    assert!(by_name.here.is_empty());
    assert_eq!(by_name.elsewhere, vec![(1, 'B', id(2))]);
    assert!(by_name.history.is_empty());
}

#[test]
fn activating_a_project_only_repoints_the_window_that_holds_it() {
    let mut registry = registry();
    registry.register(id(1), 'A', 0);
    registry.open_in(id(1), 1);

    registry.activate(id(1), 0);
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(0));

    // A project the window does not hold is not something it can be pointed at.
    registry.activate(id(1), 2);
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(0));
}
