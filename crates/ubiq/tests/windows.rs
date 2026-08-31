//! The window registry, driven without a window.
//!
//! Everything the project picker's three groups do is decided here, and none of it needs a frame:
//! a `WindowId` is just a key. This is the half of the workbench that can be tested at all.
//!
//! The catalogue is the host's, so the registry is seeded the way the host seeds it — with
//! snapshots pushed through `replace_all`, which is exactly what `ProjectList` does.

use chrono::{DateTime, TimeZone, Utc};
use gpui::WindowId;
use ubiq::state::windows::WindowRegistry;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};

/// Distinct, valid window keys. The low word is a version and is normalised to odd, so the
/// distinguishing part has to go in the high word.
fn id(n: u64) -> WindowId {
    WindowId::from((n << 32) | 1)
}

/// Fixed, ordered project ids, so a test can name the same project twice.
///
/// Minted in sequence and kept, because ids sort by creation time and several tests assert on the
/// order the registry hands them back.
fn ids(n: usize) -> Vec<ProjectId> {
    let mut ids: Vec<ProjectId> = (0..n).map(|_| ProjectId::generate()).collect();
    ids.sort();
    ids
}

fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, day, 9, 0, 0).unwrap()
}

fn snapshot(id: ProjectId, name: &str, path: &str, opened: Option<u32>) -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id,
            name: name.to_string(),
            path: path.to_string(),
            colour: 0,
            created_at: at(1),
            last_opened_at: opened.map(at),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
    }
}

/// A registry holding `n` projects, named `p0`, `p1`… under `~/dev/`.
fn registry(n: usize) -> (WindowRegistry, Vec<ProjectId>) {
    let ids = ids(n);
    let mut registry = WindowRegistry::default();
    registry.replace_all(
        ids.iter()
            .enumerate()
            // Descending `last_opened_at`, so history comes back in the same order the ids do and
            // the assertions below read naturally.
            .map(|(index, id)| {
                snapshot(
                    *id,
                    &format!("p{index}"),
                    &format!("~/dev/p{index}"),
                    Some(28 - index as u32),
                )
            })
            .collect(),
    );
    (registry, ids)
}

#[test]
fn windows_are_named_by_the_first_free_letter() {
    let (mut registry, p) = registry(5);
    assert_eq!(registry.next_label(), 'A');

    registry.register(id(1), 'A', Some(p[0]));
    assert_eq!(registry.next_label(), 'B');

    registry.register(id(2), 'B', Some(p[1]));
    assert_eq!(registry.next_label(), 'C');

    // A closed window gives its letter back, so the names stay as short as the set of windows.
    registry.unregister(id(1));
    assert_eq!(registry.next_label(), 'A');
}

#[test]
fn a_project_is_open_in_one_window_at_a_time() {
    let (mut registry, p) = registry(5);
    registry.register(id(1), 'A', Some(p[0]));
    registry.register(id(2), 'B', Some(p[1]));

    // B takes A's project. A is left with nothing, so its slot goes and its ID comes back for the
    // caller to close.
    let emptied = registry.open_in(id(2), p[0]);

    assert_eq!(emptied, vec![id(1)]);
    assert!(registry.slot(id(1)).is_none());
    assert_eq!(registry.holder(p[0]).map(|w| w.label), Some('B'));
    assert_eq!(registry.slot(id(2)).unwrap().projects, vec![p[1], p[0]]);
    // What a window takes is what it is pointed at.
    assert_eq!(registry.slot(id(2)).unwrap().active_project(), Some(p[0]));
}

#[test]
fn a_window_keeping_a_project_is_not_closed() {
    let (mut registry, p) = registry(5);
    registry.register(id(1), 'A', Some(p[0]));
    registry.open_in(id(1), p[1]);
    registry.register(id(2), 'B', Some(p[2]));

    let emptied = registry.open_in(id(2), p[1]);

    assert!(emptied.is_empty());
    assert_eq!(registry.slot(id(1)).unwrap().projects, vec![p[0]]);
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(p[0]));
}

#[test]
fn closing_the_last_project_closes_the_window() {
    let (mut registry, p) = registry(5);
    registry.register(id(1), 'A', Some(p[0]));
    registry.open_in(id(1), p[1]);

    assert!(registry.close(id(1), p[1]).is_empty());
    assert_eq!(registry.close(id(1), p[0]), vec![id(1)]);
    assert!(registry.slot(id(1)).is_none());
}

#[test]
fn a_closed_window_returns_its_projects_to_history() {
    let (mut registry, p) = registry(5);
    registry.register(id(1), 'A', Some(p[0]));
    registry.open_in(id(1), p[1]);

    registry.unregister(id(1));

    let groups = registry.groups(id(2), "");
    assert!(groups.here.is_empty());
    assert!(groups.elsewhere.is_empty());
    assert_eq!(groups.history.len(), registry.len());
}

#[test]
fn the_three_groups_say_where_everything_is() {
    let (mut registry, p) = registry(5);
    registry.register(id(1), 'A', Some(p[0]));
    registry.register(id(2), 'B', Some(p[1]));

    let groups = registry.groups(id(1), "");

    assert_eq!(groups.here, vec![p[0]]);
    assert_eq!(groups.elsewhere, vec![(p[1], 'B', id(2))]);
    assert_eq!(groups.history, vec![p[2], p[3], p[4]]);

    // The same registry, read from the other window, swaps the first two groups over.
    let groups = registry.groups(id(2), "");
    assert_eq!(groups.here, vec![p[1]]);
    assert_eq!(groups.elsewhere, vec![(p[0], 'A', id(1))]);
}

#[test]
fn history_is_ordered_by_when_a_project_was_last_opened() {
    let mut registry = WindowRegistry::default();
    let p = ids(3);
    registry.replace_all(vec![
        snapshot(p[0], "oldest", "~/dev/a", Some(2)),
        snapshot(p[1], "newest", "~/dev/b", Some(20)),
        // Never opened, so it sorts last however recently it was added.
        snapshot(p[2], "unopened", "~/dev/c", None),
    ]);

    let groups = registry.groups(id(1), "");

    assert_eq!(groups.history, vec![p[1], p[0], p[2]]);
}

#[test]
fn moving_a_project_moves_its_row_between_groups() {
    let (mut registry, p) = registry(5);
    registry.register(id(1), 'A', Some(p[0]));
    registry.register(id(2), 'B', Some(p[1]));
    registry.open_in(id(2), p[2]);

    assert_eq!(registry.groups(id(1), "").elsewhere.len(), 2);

    registry.open_in(id(1), p[2]);

    let groups = registry.groups(id(1), "");
    assert_eq!(groups.here, vec![p[0], p[2]]);
    assert_eq!(groups.elsewhere, vec![(p[1], 'B', id(2))]);
    assert!(!groups.history.contains(&p[2]));
}

#[test]
fn the_filter_matches_name_and_path_across_all_three_groups() {
    let mut registry = WindowRegistry::default();
    let p = ids(3);
    registry.replace_all(vec![
        snapshot(p[0], "agent-manager", "~/dev/agent-manager", Some(28)),
        snapshot(p[1], "ubiq", "~/dev/ubiq", Some(27)),
        snapshot(p[2], "hire-mate", "~/dev/hire-mate", Some(26)),
    ]);
    registry.register(id(1), 'A', Some(p[0]));
    registry.register(id(2), 'B', Some(p[1]));

    // A path fragment finds a project the same way its name does.
    let by_path = registry.groups(id(1), "~/dev/hire");
    assert!(by_path.here.is_empty());
    assert!(by_path.elsewhere.is_empty());
    assert_eq!(by_path.history.len(), 1);

    let by_name = registry.groups(id(1), "ubiq");
    assert!(by_name.here.is_empty());
    assert_eq!(by_name.elsewhere, vec![(p[1], 'B', id(2))]);
    assert!(by_name.history.is_empty());
}

#[test]
fn activating_a_project_only_repoints_the_window_that_holds_it() {
    let (mut registry, p) = registry(5);
    registry.register(id(1), 'A', Some(p[0]));
    registry.open_in(id(1), p[1]);

    registry.activate(id(1), p[0]);
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(p[0]));

    // A project the window does not hold is not something it can be pointed at.
    registry.activate(id(1), p[2]);
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(p[0]));
}

// ── the projection ──────────────────────────────────────────────────

#[test]
fn a_window_on_an_empty_catalogue_is_not_closed() {
    // The rule "a window with no project closes" would quit the application at boot, when there is
    // nothing to open and the picker is the only thing worth showing.
    let mut registry = WindowRegistry::default();
    assert!(registry.is_empty());

    let emptied = registry.register(id(1), 'A', None);

    assert!(emptied.is_empty());
    assert!(registry.slot(id(1)).is_some());
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), None);
}

#[test]
fn once_a_project_exists_the_ordinary_rule_is_back() {
    let (mut registry, p) = registry(2);
    registry.register(id(1), 'A', Some(p[0]));

    assert_eq!(registry.close(id(1), p[0]), vec![id(1)]);
}

#[test]
fn forgetting_the_only_project_leaves_the_window_open() {
    let mut registry = WindowRegistry::default();
    let p = ids(1);
    registry.replace_all(vec![snapshot(p[0], "only", "~/dev/only", Some(1))]);
    registry.register(id(1), 'A', Some(p[0]));

    let emptied = registry.forget(p[0]);

    // Nothing is left to open, so the window stays and offers to add one.
    assert!(emptied.is_empty());
    assert!(registry.slot(id(1)).is_some());
    assert!(registry.is_empty());
}

#[test]
fn a_catalogue_that_loses_a_project_drops_it_from_every_window() {
    let (mut registry, p) = registry(3);
    registry.register(id(1), 'A', Some(p[0]));
    registry.open_in(id(1), p[1]);

    // The host says two of the three are left. A window holding the third loses it, but the
    // catalogue arriving is not the user closing anything, so nothing is reaped.
    registry.replace_all(vec![
        snapshot(p[1], "p1", "~/dev/p1", Some(27)),
        snapshot(p[2], "p2", "~/dev/p2", Some(26)),
    ]);

    assert_eq!(registry.slot(id(1)).unwrap().projects, vec![p[1]]);
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(p[1]));
    assert!(registry.project(p[0]).is_none());
}

#[test]
fn applying_a_snapshot_changes_it_in_place_and_keeps_it_where_it_is() {
    let (mut registry, p) = registry(3);
    registry.register(id(1), 'A', Some(p[0]));

    let mut changed = snapshot(p[0], "renamed", "~/dev/p0", Some(28));
    changed.open_panes = 3;
    changed.health = ProjectHealth::Missing;
    registry.apply(changed);

    let got = registry.project(p[0]).unwrap();
    assert_eq!(got.record.name, "renamed");
    assert_eq!(got.open_panes, 3);
    assert_eq!(got.health, ProjectHealth::Missing);
    // Still open where it was: a change is not a move.
    assert_eq!(registry.slot(id(1)).unwrap().active_project(), Some(p[0]));
}

#[test]
fn a_window_opens_on_the_project_used_most_recently() {
    let mut registry = WindowRegistry::default();
    let p = ids(3);
    registry.replace_all(vec![
        snapshot(p[0], "a", "~/dev/a", Some(2)),
        snapshot(p[1], "b", "~/dev/b", Some(20)),
        snapshot(p[2], "c", "~/dev/c", None),
    ]);

    assert_eq!(registry.most_recent(), Some(p[1]));
}

#[test]
fn an_empty_catalogue_has_nothing_to_open_on() {
    assert_eq!(WindowRegistry::default().most_recent(), None);
}

#[test]
fn a_project_the_catalogue_does_not_hold_cannot_be_opened() {
    let (mut registry, p) = registry(2);
    let stranger = ProjectId::generate();

    // Asked for a project that is not there, with a catalogue that has others: the window has
    // nothing to show and is reaped like any other empty one.
    let emptied = registry.register(id(1), 'A', Some(stranger));
    assert_eq!(emptied, vec![id(1)]);
    assert!(registry.slot(id(1)).is_none());

    // A window that does exist cannot be pointed at one either.
    registry.register(id(2), 'B', Some(p[0]));
    assert!(registry.open_in(id(2), stranger).is_empty());
    assert_eq!(registry.slot(id(2)).unwrap().projects, vec![p[0]]);
}
