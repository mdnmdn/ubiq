//! The contract's identifiers: that they sort, that they print, and that they survive the wire.
//!
//! Sorting by creation time is most of why these are ULIDs rather than UUIDs, and it is a property
//! of the *generator*, not of the type — a bare `Ulid::new()` does not have it. That is what the
//! first test is guarding.

use std::str::FromStr;

use ubiq_proto::ids::{PaneId, ProjectId, SessionId};

#[test]
fn ids_minted_together_still_sort_in_the_order_they_were_minted() {
    // Far more than one millisecond's worth, so the run is dominated by ids sharing a timestamp.
    // Those are exactly the ones a non-monotonic generator would order arbitrarily.
    let minted: Vec<PaneId> = (0..10_000).map(|_| PaneId::generate()).collect();

    let mut sorted = minted.clone();
    sorted.sort();

    assert_eq!(
        minted, sorted,
        "ids must sort in the order they were minted"
    );
}

#[test]
fn every_id_is_distinct() {
    let mut minted: Vec<PaneId> = (0..10_000).map(|_| PaneId::generate()).collect();
    minted.sort();
    let before = minted.len();
    minted.dedup();
    assert_eq!(minted.len(), before, "no id is minted twice");
}

#[test]
fn an_id_prints_as_twenty_six_characters_and_reads_back() {
    let id = ProjectId::generate();
    let text = id.to_string();

    assert_eq!(
        text.len(),
        26,
        "the canonical form is 26 characters: {text}"
    );
    assert!(
        text.chars().all(|c| c.is_ascii_alphanumeric()),
        "no hyphens, nothing to escape in a directory name: {text}"
    );
    assert_eq!(ProjectId::from_str(&text).unwrap(), id);
}

#[test]
fn the_wire_form_is_that_same_bare_string() {
    let id = SessionId::generate();

    // A newtype must not wrap the value in a struct on the wire; the contract says a string.
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, format!("\"{id}\""));
    assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), id);
}

#[test]
fn debug_says_which_kind_of_id_it_is() {
    let id = PaneId::generate();
    assert_eq!(format!("{id:?}"), format!("PaneId({id})"));
}

#[test]
fn an_id_carries_the_time_it_was_minted() {
    let before = std::time::SystemTime::now();
    let id = PaneId::generate();

    // The stamp has millisecond resolution, so it may round down below `before`.
    let skew = before
        .duration_since(id.created_at())
        .unwrap_or_default()
        .as_millis();
    assert!(skew <= 1, "the stamp should be the moment it was minted");
}
