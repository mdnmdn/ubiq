//! The Trello client, against recorded responses.
//!
//! **A client that can only be tested against a live board will not be tested.** Everything the
//! client *knows* — what a card means, which filter matches, which checklist counts — is a pure
//! function of a JSON body, so it is exercised here against fixtures under `fixtures/trello/`
//! rather than against somebody's board. The network wrappers beside those functions are one round
//! trip each and are not unit-tested, the same as every other provider's network calls.
//!
//! The fixtures are synthetic but shaped like Trello's real answers — `idShort`, `shortUrl`,
//! `dateLastActivity`, `pos` as a rebalancing float, labels as objects *and* as `idLabels`, a
//! closed board, an archived list and an archived card. They are the same set a second provider's
//! tests copy.

use ubiq_host::tasksrc::store::{TasksrcFile, seed_lanes};
use ubiq_host::tasksrc::trello::{
    self, FACET_LABELS, FACET_LISTS, FACET_MEMBERS, FIELD_LABELS, FIELD_LISTS, FIELD_MEMBERS,
};
use ubiq_host::tasksrc::{Binding, Registry};
use ubiq_proto::connectors::ProviderId;
use ubiq_proto::ids::ConnectionId;
use ubiq_proto::tasksrc::{ConfigFieldKind, RemoteContainerId, RemoteLaneId};
use ubiq_proto::work::Status;

const BOARDS: &str = include_str!("fixtures/trello/boards.json");
const LISTS: &str = include_str!("fixtures/trello/lists.json");
const LABELS: &str = include_str!("fixtures/trello/labels.json");
const MEMBERS: &str = include_str!("fixtures/trello/members.json");
const CARDS: &str = include_str!("fixtures/trello/cards.json");
const ACTIONS: &str = include_str!("fixtures/trello/actions.json");
const CHECKLISTS: &str = include_str!("fixtures/trello/checklists.json");

fn json(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).expect("a fixture is valid JSON")
}

const BOARD: &str = "5f2a7b1c9d0e4a0012345601";
const TO_DO: &str = "6a1b2c3d4e5f60007000a101";
const DOING: &str = "6a1b2c3d4e5f60007000a102";
const BUG: &str = "7b2c3d4e5f60700080001201";
const ADA: &str = "8c3d4e5f607008000900e301";

fn binding() -> Binding {
    Binding::new(
        trello::ID,
        ConnectionId::generate(),
        RemoteContainerId::new(BOARD),
    )
}

#[test]
fn a_closed_board_is_not_something_to_bind_to() {
    let boards = trello::parse_boards(&json(BOARDS));
    assert_eq!(boards.len(), 2);
    assert_eq!(boards[0].name, "Ubiq");
    assert_eq!(boards[0].url, "https://trello.com/b/aBcDeFgH");
    assert!(
        !boards.iter().any(|board| board.name == "Last year"),
        "a closed board is not a container"
    );
}

/// Trello's `pos` is a float it rebalances, and the answer does not arrive sorted.
#[test]
fn lists_come_back_in_the_boards_own_order_with_the_archived_one_left_out() {
    let lanes = trello::parse_lists(&json(LISTS));
    let names: Vec<&str> = lanes.iter().map(|lane| lane.name.as_str()).collect();
    assert_eq!(names, ["To Do", "Doing", "Done", "Icebox"]);
    assert!(!names.contains(&"Archived ideas"));
    assert_eq!(lanes[0].id, RemoteLaneId::new(TO_DO));
}

/// A label's id is a swatch key and its name is what is written on the card — which is why a
/// facet value carries both, and why a label with no name still has something to show.
#[test]
fn a_facet_value_carries_the_id_to_store_and_the_label_to_show() {
    let facets = trello::parse_facets(&json(LISTS), &json(LABELS), &json(MEMBERS));

    let labels = facets.get(FACET_LABELS);
    assert_eq!(labels.len(), 3);
    assert_eq!(labels[0].id, BUG);
    assert_eq!(labels[0].label, "bug");
    // A nameless label shows its colour rather than an empty row.
    assert_eq!(labels[2].label, "purple");

    let members = facets.get(FACET_MEMBERS);
    assert_eq!(members[0].label, "Ada Lovelace");
    // An account with no full name falls back to its username.
    assert_eq!(members[2].label, "anonymous");

    assert_eq!(facets.get(FACET_LISTS).len(), 4);
}

#[test]
fn a_card_becomes_a_neutral_item() {
    let cards = json(CARDS);
    let card = &cards.as_array().expect("an array")[0];
    let item = trello::parse_card(card).expect("a card parses");

    assert_eq!(item.key.as_deref(), Some("#41"));
    assert_eq!(item.url, "https://trello.com/c/Ab12Cd34");
    assert_eq!(item.lane, RemoteLaneId::new(TO_DO));
    assert_eq!(item.title, "Pane resize corrupts the alternate screen");
    // The description is markdown, carried exactly as Trello gave it.
    assert!(item.body.contains("**old**"));
    assert_eq!(item.labels, vec!["bug".to_string()]);
    // `dateLastActivity` is the only version token Trello has.
    assert_eq!(item.revision.as_str(), "2026-09-18T09:14:02.517Z");
    assert_eq!(
        item.updated_at.to_rfc3339(),
        "2026-09-18T09:14:02.517+00:00"
    );
    // Trello has no type, no priority and no hierarchy, and nothing is invented for them.
    assert_eq!(item.kind_hint, None);
    assert_eq!(item.priority_hint, None);
    assert_eq!(item.parent, None);
}

#[test]
fn an_empty_filter_offers_the_whole_board_and_never_the_archive() {
    let items = trello::parse_cards(&json(CARDS), &binding());
    assert_eq!(items.len(), 3, "three open cards, and not the archived one");
    assert!(
        !items
            .iter()
            .any(|item| item.title.contains("nobody should see"))
    );
}

/// Trello has no query language, so every one of these runs in this client after the fetch. That
/// is what `ProviderCaps::server_query` being off means.
#[test]
fn every_filter_is_applied_client_side() {
    let mut narrowed = binding();
    narrowed
        .filter
        .set(FIELD_LISTS, vec![TO_DO.to_string(), DOING.to_string()]);
    let items = trello::parse_cards(&json(CARDS), &narrowed);
    assert_eq!(items.len(), 2);

    let mut by_label = binding();
    by_label.filter.set(FIELD_LABELS, vec![BUG.to_string()]);
    let items = trello::parse_cards(&json(CARDS), &by_label);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key.as_deref(), Some("#41"));

    let mut by_member = binding();
    by_member.filter.set(FIELD_MEMBERS, vec![ADA.to_string()]);
    let items = trello::parse_cards(&json(CARDS), &by_member);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key.as_deref(), Some("#41"));

    // Two filters narrow together, and a pair that agrees about nothing answers nothing.
    let mut both = binding();
    both.filter.set(FIELD_LISTS, vec![DOING.to_string()]);
    both.filter.set(FIELD_LABELS, vec![BUG.to_string()]);
    assert!(trello::parse_cards(&json(CARDS), &both).is_empty());
}

/// A card carries member *ids*; a task's `assigned_to` is what a person is called.
#[test]
fn assignees_are_named_from_the_members_facet_and_an_unknown_id_is_dropped() {
    let cards = json(CARDS);
    let cards = cards.as_array().expect("an array");
    let members = trello::parse_members(&json(MEMBERS));

    let mut item = trello::parse_card(&cards[0]).expect("a card");
    trello::name_assignees(&mut item, &cards[0], &members);
    assert_eq!(item.assignees, vec!["Ada Lovelace".to_string()]);

    // The second card names a member the board does not have; the id is dropped rather than shown.
    let mut item = trello::parse_card(&cards[1]).expect("a card");
    trello::name_assignees(&mut item, &cards[1], &members);
    assert_eq!(item.assignees, vec!["Grace Hopper".to_string()]);
}

/// The author is carried for display and never written onto a task's comment — `D121` says the
/// host stamps that from the path the line arrived on.
#[test]
fn only_comment_actions_are_comments() {
    let comments = trello::parse_comments(&json(ACTIONS));
    assert_eq!(comments.len(), 2, "the updateCard action is not a comment");
    assert_eq!(comments[0].author, "Ada Lovelace");
    assert_eq!(comments[0].text, "Reproduced on a 120-column pane.");
    // An account with no full name is shown by its username.
    assert_eq!(comments[1].author, "anonymous");
}

/// `R10`: Ubiq has one ordered checklist and Trello has many, so the first is the only
/// unambiguous choice and the second is left alone rather than merged.
#[test]
fn the_first_checklist_is_the_one_and_its_items_come_back_in_order() {
    let items = trello::parse_checklists(&json(CHECKLISTS));
    let texts: Vec<&str> = items.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(texts, ["Reproduce it", "Write the test", "Ship it"]);
    assert!(items[0].done);
    assert!(!items[1].done);
    assert!(
        !texts.contains(&"Left alone"),
        "the second checklist is not merged"
    );
    assert!(trello::parse_checklists(&serde_json::json!([])).is_empty());
}

/// Trello writes labels by id and the neutral model carries names, so the board's own list joins
/// them — and a name the board does not have is dropped rather than created.
#[test]
fn a_label_name_becomes_an_id_and_an_unknown_name_is_not_invented() {
    let labels = trello::parse_labels(&json(LABELS));
    let ids = trello::label_ids(
        &["bug".to_string(), "nothing-like-this".to_string()],
        &labels,
    );
    assert_eq!(ids, vec![BUG.to_string()]);
}

/// The default map is seeded by *name* because a freshly bound board has no ids to match against,
/// and is id-keyed from then on. The other four Ubiq lanes start unbound.
#[test]
fn a_fresh_trello_binding_maps_to_do_doing_and_done_and_nothing_else() {
    let mut binding = binding();
    seed_lanes(&mut binding, &trello::parse_lists(&json(LISTS)));

    assert_eq!(binding.lanes.len(), 3);
    assert_eq!(
        binding.status_of(&RemoteLaneId::new(TO_DO)),
        Some(Status::Backlog)
    );
    assert_eq!(
        binding.status_of(&RemoteLaneId::new(DOING)),
        Some(Status::InProgress)
    );
    assert_eq!(
        binding.status_of(&RemoteLaneId::new("6a1b2c3d4e5f60007000a103")),
        Some(Status::Done)
    );
    // "Icebox" is not one of the three names everybody uses, so it is left for the user.
    assert_eq!(
        binding.status_of(&RemoteLaneId::new("6a1b2c3d4e5f60007000a104")),
        None
    );
    for status in [
        Status::Ready,
        Status::Blocked,
        Status::InReview,
        Status::Abandoned,
    ] {
        assert_eq!(binding.lane_of(status), None, "{status:?} starts unbound");
    }
}

/// A binding written by the Trello client round-trips through `tasksrc.toml` unchanged, which is
/// what makes the setup surface's job saving a struct rather than a migration.
#[test]
fn a_seeded_binding_round_trips_through_the_sidecar() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("tasksrc.toml");
    let mut binding = binding();
    seed_lanes(&mut binding, &trello::parse_lists(&json(LISTS)));
    binding.filter.set(FIELD_LABELS, vec![BUG.to_string()]);

    let file = TasksrcFile {
        bindings: vec![binding.clone()],
        ..TasksrcFile::empty()
    };
    ubiq_host::tasksrc::store::save(&path, &file).expect("it writes");
    let loaded = ubiq_host::tasksrc::store::load(&path);
    assert_eq!(loaded.only_binding(), Some(&binding));
}

/// The secret is a **pair**, which no other connector provider's is.
#[test]
fn a_trello_credential_is_an_api_key_and_a_token() {
    let (key, token) = trello::split_secret("0123456789abcdef:fedcba9876543210").expect("a pair");
    assert_eq!(
        trello::auth_header(key, token),
        "OAuth oauth_consumer_key=\"0123456789abcdef\", oauth_token=\"fedcba9876543210\""
    );
    // Half a credential is a refusal, not a request made anyway.
    assert!(trello::split_secret("0123456789abcdef").is_err());
}

/// Trello is the base's seventh connector provider and the connections screen still offers six.
#[test]
fn trello_is_a_provider_the_connections_screen_does_not_offer() {
    assert!(!ProviderId::Trello.pickable());
    assert_eq!(ProviderId::all().len(), 6);
    assert!(ProviderId::every().contains(&ProviderId::Trello));
}

/// What the setup surface renders: four pickers, because Trello has nothing to type into.
#[test]
fn the_declared_schema_is_what_the_base_renders() {
    let registry = Registry::with_defaults();
    let trello = registry.get(trello::ID).expect("Trello is registered");
    let info = trello.info();

    assert_eq!(info.id, "trello");
    assert_eq!(info.config_schema.len(), 4);

    let board = info.config_schema[0];
    assert_eq!(board.kind, ConfigFieldKind::Choice("boards"));
    assert!(board.required);
    // Picking from a list already proves the id exists, so there is nothing for a Test to add.
    assert!(!board.testable);

    for field in &info.config_schema[1..] {
        assert!(matches!(field.kind, ConfigFieldKind::MultiChoice(_)));
        assert!(!field.required, "an unnarrowed filter is the whole board");
        assert!(field.testable);
    }

    // The four things Trello cannot do, stated rather than discovered at the first failed write.
    let caps = trello.caps();
    assert!(!caps.conditional_write);
    assert!(!caps.server_query);
    assert!(!caps.hierarchy);
    assert!(caps.write_lane && caps.checklist_read && caps.comments_read);
}
