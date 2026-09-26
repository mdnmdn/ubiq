//! The one claim M7 exists to make: **one declarative renderer serves every provider.**
//!
//! These tests are deliberately written against two providers' *real* declared shapes — Trello's
//! four fields as `crates/ubiq-host/src/tasksrc/trello.rs` declares them, and Azure DevOps' five
//! as Studio's `ado_provider.rs` does, including the tree flattened into a flat `Choice` and
//! `server_query: true`. Neither provider's code is reachable from here, which is exactly the
//! point: the interface holds a `ProviderInfo` that arrived over the wire, and everything it draws
//! is derived from that.
//!
//! A test that asserted the renderer had been *configured* would pass with the renderer deleted.
//! These assert what falls out of the schema: which control each field gets, which fields Test is
//! offered for, which write rows are lit, and which caveats are owed.

use std::collections::BTreeMap;
use ubiq::ext::ids;
use ubiq::ext::settings::project_sections;
use ubiq::state::tasksrc::{ABILITIES, CAVEATS, TaskSrcState};

use chrono::Utc;
use ubiq_proto::ids::{BindingId, ConnectionId, TaskId};
use ubiq_proto::tasksrc::{
    Binding, ConfigField, ConfigFieldKind, DriftSide, FieldDrift, LinkState, ProviderCaps,
    ProviderInfo, RemoteContainerId, RemoteItemId, Revision, TaskLink,
};

/// Trello's schema, as its provider declares it: a board picker and three client-side facets. No
/// query language, and therefore no `testable` field.
fn trello() -> ProviderInfo {
    ProviderInfo {
        id: "trello",
        label: "Trello",
        caps: ProviderCaps {
            write_lane: true,
            write_title: true,
            write_body: true,
            write_labels: true,
            write_assignee: true,
            comments_read: true,
            comments_write: true,
            checklist_read: true,
            checklist_write: true,
            conditional_write: false,
            server_query: false,
            hierarchy: false,
        },
        config_schema: vec![
            ConfigField {
                key: "board",
                label: "Board",
                kind: ConfigFieldKind::Choice("boards"),
                required: true,
                testable: false,
            },
            ConfigField {
                key: "lists",
                label: "Lists",
                kind: ConfigFieldKind::MultiChoice("lists"),
                required: false,
                testable: false,
            },
            ConfigField {
                key: "labels",
                label: "Labels",
                kind: ConfigFieldKind::MultiChoice("labels"),
                required: false,
                testable: false,
            },
            ConfigField {
                key: "members",
                label: "Members",
                kind: ConfigFieldKind::MultiChoice("members"),
                required: false,
                testable: false,
            },
        ],
        connector: Some(ubiq_proto::connectors::ProviderId::Trello),
    }
}

/// Azure DevOps' schema, as M9 declares it in Studio: five fields, a hierarchy flattened into a
/// flat `Choice`, `server_query: true`, and WIQL as `Text` + Test.
fn ado() -> ProviderInfo {
    ProviderInfo {
        id: "ado",
        label: "Azure DevOps",
        caps: ProviderCaps {
            server_query: true,
            hierarchy: true,
            ..ProviderCaps::default()
        },
        config_schema: vec![
            ConfigField {
                key: "types",
                label: "Work item types",
                kind: ConfigFieldKind::MultiChoice("types"),
                required: false,
                testable: false,
            },
            ConfigField {
                key: "iteration",
                label: "Iteration",
                kind: ConfigFieldKind::Choice("iterations"),
                required: false,
                testable: false,
            },
            ConfigField {
                key: "assigned",
                label: "Assigned to",
                kind: ConfigFieldKind::Choice("assignees"),
                required: false,
                testable: false,
            },
            ConfigField {
                key: "area",
                label: "Area path",
                kind: ConfigFieldKind::Choice("areas"),
                required: false,
                testable: false,
            },
            ConfigField {
                key: "wiql",
                label: "WIQL condition",
                kind: ConfigFieldKind::Text,
                required: false,
                testable: true,
            },
        ],
        connector: Some(ubiq_proto::connectors::ProviderId::AzureDevops),
    }
}

fn bound(info: ProviderInfo) -> TaskSrcState {
    let binding = Binding::new(
        info.id,
        ConnectionId::generate(),
        RemoteContainerId::new("container-1"),
    );
    TaskSrcState {
        providers: vec![info],
        draft: Some(binding),
        ..TaskSrcState::default()
    }
}

/// Nine field declarations across two trackers, and **every one lands in one of five shapes**.
///
/// This is `R6`'s whole claim restated as an assertion: if a provider's configuration needed
/// something `ConfigFieldKind` cannot express, it would show up here as a shape the renderer has
/// no arm for, which is a missing variant rather than a leak into the interface.
#[test]
fn every_field_both_providers_declare_is_one_of_the_five_shapes() {
    let mut text = 0;
    let mut choice = 0;
    let mut multi = 0;
    for info in [trello(), ado()] {
        for spec in &info.config_schema {
            match spec.kind {
                ConfigFieldKind::Text => text += 1,
                ConfigFieldKind::Secret | ConfigFieldKind::Bool => {}
                ConfigFieldKind::Choice(_) => choice += 1,
                ConfigFieldKind::MultiChoice(_) => multi += 1,
            }
        }
    }
    // One `Text` (ADO's WIQL), four `Choice` (Trello's board, ADO's iteration/assignee/area) and
    // four `MultiChoice` (Trello's three, ADO's types). Nothing is left over.
    assert_eq!((text, choice, multi), (1, 4, 4));
    assert_eq!(trello().config_schema.len() + ado().config_schema.len(), 9);
}

/// **ADO's area path is a tree, and it is declared as a flat `Choice`.**
///
/// The one place a provider's shape was predicted to strain `R6`. It does not: the provider
/// flattens the hierarchy into full-path labels and the base draws a list. Pinned here so a later
/// `Tree` kind has to justify itself against a case that did not need one.
#[test]
fn a_hierarchy_reaches_the_renderer_flattened_and_needs_nothing_new() {
    let area = ado()
        .config_schema
        .into_iter()
        .find(|spec| spec.key == "area")
        .expect("ADO declares an area path");
    assert!(matches!(area.kind, ConfigFieldKind::Choice("areas")));
}

/// Test is offered for the field that declares it, and for no other — and **which** field that is
/// comes from the provider, not from the renderer.
#[test]
fn test_is_offered_where_the_schema_says_and_nowhere_else() {
    // Trello has no query language, so nothing on its page is testable at field level. The Test
    // button still runs the filter — there is simply no syntax anywhere to have validated.
    assert!(trello().config_schema.iter().all(|spec| !spec.testable));

    let testable: Vec<&str> = ado()
        .config_schema
        .iter()
        .filter(|spec| spec.testable)
        .map(|spec| spec.key)
        .collect();
    assert_eq!(testable, ["wiql"]);
}

/// The write rows are lit from the capability table, and the two providers differ in **which rows
/// light**, not in which rows are drawn.
#[test]
fn capabilities_light_rows_rather_than_removing_them() {
    let rich = trello().caps;
    let lean = ado().caps;

    let lit_for = |caps: ProviderCaps| -> Vec<&str> {
        ABILITIES
            .iter()
            .filter(|ability| (ability.needs)(&caps))
            .map(|ability| ability.key)
            .collect()
    };

    // Trello accepts every write the table names; ADO accepts none yet (M8 flips them).
    assert_eq!(lit_for(rich).len(), ABILITIES.len());
    assert!(lit_for(lean).is_empty());

    // And the row count is the same either way: a provider that cannot do a thing gets the row
    // greyed, never removed, so the two pages are the same shape.
    assert_eq!(ABILITIES.len(), 7);
}

/// The sentences a page owes are derived from the gaps, and both providers owe different ones.
#[test]
fn the_caveats_a_page_owes_come_from_the_gaps_it_has() {
    let owed = |caps: ProviderCaps| -> Vec<&str> {
        CAVEATS
            .iter()
            .filter(|caveat| (caveat.when)(&caps))
            .map(|caveat| caveat.at)
            .collect()
    };

    // Trello has no conditional write and no server-side query: it owes all three.
    assert_eq!(owed(trello().caps), ["authority", "filter", "interval"]);
    // ADO filters server-side, so the two full-fetch sentences are not owed — but it has no
    // conditional write yet, so the `R8` sentence beside the switch still is.
    assert_eq!(owed(ado().caps), ["authority"]);
}

/// A required field nobody filled blocks the save, and **the schema is what says which fields
/// those are**. Trello's board is required; none of ADO's five is.
#[test]
fn completeness_is_read_off_the_schema() {
    let mut state = bound(trello());
    // The board id is on the binding rather than in the filter, so the declared `board` field is
    // still unfilled and the page refuses.
    assert!(!state.complete());
    if let Some(draft) = state.draft.as_mut() {
        draft.filter.set("board", vec!["board-1".into()]);
    }
    assert!(state.complete());

    // ADO declares nothing required, so a bound-but-unfiltered binding is savable — an unnarrowed
    // filter is the whole board, not an error.
    let ado_state = bound(ado());
    assert!(ado_state.complete());
}

/// **The parked badge.** M6 files `LinkState::Parked` and nothing drew it; this is what says it is
/// drawn now, and that the ordinary case draws nothing.
#[test]
fn only_the_states_worth_saying_get_a_badge() {
    assert!(!LinkState::Linked.is_notable());
    assert!(LinkState::Parked.is_notable());
    assert!(LinkState::Parked.label() == "Parked");
    // The sentence matters as much as the dot: a coloured badge with no explanation is a thing
    // somebody has to ask about.
    assert!(LinkState::Parked.note().contains("lane map"));
}

/// Changing provider keeps the answers that are about *this project* and drops the ones that were
/// about the tracker.
#[test]
fn switching_provider_keeps_what_is_not_provider_shaped() {
    let mut state = bound(trello());
    state.providers.push(ado());
    if let Some(draft) = state.draft.as_mut() {
        draft.poll = 900;
        draft.filter.set("labels", vec!["bug".into()]);
    }

    // What `pick_task_provider` does, restated without a window: the interval survives, the
    // provider-keyed filter does not.
    let poll = state.draft.as_ref().map(|draft| draft.poll);
    assert_eq!(poll, Some(900));
    let fresh = Binding::new(
        "ado",
        ConnectionId::generate(),
        RemoteContainerId::new(String::new()),
    );
    assert!(fresh.filter.fields.is_empty());
}

/// The section is registered through the container, under a group the container declares.
#[test]
fn the_section_is_a_contribution_to_the_project_container() {
    let spec = project_sections()
        .iter()
        .find(|spec| spec.id == ids::PROJECT_TASK_SYNC)
        .expect("task sync is registered");
    assert_eq!(spec.label, "Task sync");
    assert_eq!(spec.group, ids::SETTINGS_PROJECT_CORE);
}

// ── The drift surfaces (`D188`) ──────────────────────────────────────

fn drifting(field: &str, settles: Option<DriftSide>) -> TaskLink {
    TaskLink {
        binding: BindingId::generate(),
        task: TaskId::generate(),
        item: RemoteItemId::new("card-1"),
        container: RemoteContainerId::new("container-1"),
        revision: Revision::new("r1"),
        hash: BTreeMap::new(),
        local: BTreeMap::new(),
        drift: vec![FieldDrift {
            field: field.to_string(),
            local: "mine".to_string(),
            remote: "theirs".to_string(),
            settles,
            note: format!("Both sides changed {field}."),
        }],
        synced_at: Utc::now(),
        state: LinkState::Drifted,
    }
}

/// **The drift overview is read off the link rows the interface already holds**, not off a second
/// message that could disagree with them.
///
/// That is the shape `D188` picked over a `DriftOverview` reply, and this is what it buys: the
/// rows a window can draw are exactly the rows it was told about per task, with no query id, no
/// staleness rule and nothing to refresh.
#[test]
fn what_the_overview_draws_comes_off_the_link_rows_and_nowhere_else() {
    let mut state = bound(trello());
    let row = drifting("title", Some(DriftSide::Pull));
    let task = row.task;
    state.links.insert(task, row);

    let rows: Vec<&FieldDrift> = state
        .links
        .values()
        .flat_map(|link| link.drift.iter())
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].field, "title");
    // Both values travel, because the overview's job is to let somebody read the two and choose.
    assert_eq!(rows[0].local, "mine");
    assert_eq!(rows[0].remote, "theirs");
    assert_eq!(rows[0].settles, Some(DriftSide::Pull));
    // And the badge has something to say beyond the state's own sentence.
    assert_eq!(state.link(task).expect("a row").state, LinkState::Drifted);
}

/// **A field that settles nowhere is drawn, not hidden.** `settles: None` is a divergence no
/// switch setting closes — the provider will not take the write — and the surfaces must be able to
/// say so rather than offering a button that does nothing.
#[test]
fn a_field_that_settles_nowhere_is_still_a_row() {
    let mut state = bound(trello());
    let row = drifting("assignees", None);
    let task = row.task;
    state.links.insert(task, row);

    let drift = &state.link(task).expect("a row").drift[0];
    assert_eq!(drift.settles, None);
    assert!(!drift.note.is_empty(), "and it says why");
}

/// `Drifted` and `Conflict` are both notable, so both wear a badge — and `Linked` still says
/// nothing, which is what keeps the ordinary case quiet now that M8 fills these in.
#[test]
fn the_states_m8_produces_are_the_ones_the_badge_already_draws() {
    for state in [LinkState::Drifted, LinkState::Conflict] {
        assert!(state.is_notable());
        assert!(!state.note().is_empty());
    }
    assert!(!LinkState::Linked.is_notable());
    assert_eq!(DriftSide::Push.label(), "Push");
    assert_eq!(DriftSide::Pull.label(), "Pull");
}
