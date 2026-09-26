//! Trello, the first provider.
//!
//! Chosen because it is free, personal, ubiquitous, and **weak in exactly the places a good
//! abstraction must survive**: no query language, no work-item type, no priority, no conditional
//! write, no hierarchy. A layer that fits Trello fits a richer tracker by relaxing, which is the
//! direction that works.
//!
//! The module is split so the network and the meaning are separable. Every `parse_*` function
//! takes a [`Value`] the API already answered and returns typed items; every `get`/`post`/`put`
//! wrapper does one round trip and nothing else. That is what lets the whole of what this file
//! *knows* be tested against recorded fixtures — see `crates/ubiq-host/tests/tasksrc_trello.rs` —
//! rather than against somebody's live board, which is the same as not testing it.
//!
//! Three facts shape everything here:
//!
//! - **No query language.** `GET /1/boards/{id}/cards` returns the board's open cards and that is
//!   the whole of what can be asked. Every filter is applied here, after the fetch, which is why
//!   [`ProviderCaps::server_query`] is off.
//! - **No `If-Match` on a card.** `dateLastActivity` is the only version token, and comparing it
//!   is a check in this client rather than a precondition at Trello, so
//!   [`ProviderCaps::conditional_write`] is off and `R8`'s window is stated rather than hidden.
//! - **A paired secret.** Trello authenticates with an API key *and* a token, sent as
//!   `Authorization: OAuth oauth_consumer_key="…", oauth_token="…"`. The connector family's
//!   secret store holds one string per connection, so the pair is stored joined by a colon and
//!   split here — the one place both halves are needed at once (`D183`).

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;
use ubiq_proto::tasksrc::{
    ConfigField, ConfigFieldKind, FacetValue, Facets, ProviderCaps, RemoteCheckId, RemoteCheckItem,
    RemoteComment, RemoteContainer, RemoteContainerId, RemoteDraft, RemoteItem, RemoteItemId,
    RemoteLane, RemoteLaneId, RemotePatch, Revision,
};

use super::{Binding, Identity, TaskProvider};
use crate::connectors::http;
use crate::connectors::tls;

/// The provider's stable key, which is what a binding names.
pub const ID: &str = "trello";

/// Where every request goes. Trello has one cloud and nothing to self-host, so this is not built
/// from an instance.
const API: &str = "https://api.trello.com";

// ── the declared configuration ───────────────────────────────────────

/// The board. A `Choice`, and **not** testable: picking from the list already proves the id
/// exists, and a Test would be a round trip that can only agree with the one that filled the list.
pub const FIELD_BOARD: &str = "board";
/// Which lists are offered. Empty is every list, never none.
pub const FIELD_LISTS: &str = "lists";
pub const FIELD_LABELS: &str = "labels";
pub const FIELD_MEMBERS: &str = "members";

/// The facet names the `Choice` and `MultiChoice` fields draw from.
pub const FACET_BOARDS: &str = "boards";
pub const FACET_LISTS: &str = "lists";
pub const FACET_LABELS: &str = "labels";
pub const FACET_MEMBERS: &str = "members";

/// What the setup surface renders. Four fields, all of them pickers: Trello has nothing to type
/// into, which is exactly why it is the provider that tests the schema (`R6`).
const SCHEMA: &[ConfigField] = &[
    ConfigField {
        key: FIELD_BOARD,
        label: "Board",
        kind: ConfigFieldKind::Choice(FACET_BOARDS),
        required: true,
        testable: false,
    },
    ConfigField {
        key: FIELD_LISTS,
        label: "Lists",
        kind: ConfigFieldKind::MultiChoice(FACET_LISTS),
        required: false,
        testable: true,
    },
    ConfigField {
        key: FIELD_LABELS,
        label: "Labels",
        kind: ConfigFieldKind::MultiChoice(FACET_LABELS),
        required: false,
        testable: true,
    },
    ConfigField {
        key: FIELD_MEMBERS,
        label: "Members",
        kind: ConfigFieldKind::MultiChoice(FACET_MEMBERS),
        required: false,
        testable: true,
    },
];

/// What Trello can do, and the four things it cannot.
const CAPS: ProviderCaps = ProviderCaps {
    write_lane: true,
    write_title: true,
    write_body: true,
    write_labels: true,
    // **Off, and it used to be on** (`D188`). `update` deliberately never pushes `assignees`: a
    // task's `assigned_to` is free text and a Trello member is an account, so writing one from the
    // other would invent a membership. A flag claiming a write the one method that could make it
    // refuses to make is the exact failure `T-237` names — a capability that means "sometimes" —
    // and outbound is where it bites, because M8 gates the patch on this flag. Under-declaring is
    // the safe direction: the interface greys the row with its reason beside it, and a local
    // assignee change is filed as drift that settles nowhere rather than silently dropped.
    write_assignee: false,
    comments_read: true,
    comments_write: true,
    checklist_read: true,
    checklist_write: true,
    // `dateLastActivity` is a token this client compares, never a precondition Trello honours.
    conditional_write: false,
    // `GET /1/boards/{id}/cards` is the whole query surface. Everything else is client-side.
    server_query: false,
    // Trello has no parent/child relation between cards.
    hierarchy: false,
};

/// The Trello client. Holds nothing: the identity arrives per call and the binding says which
/// board, so one instance serves every project.
#[derive(Debug, Default, Clone, Copy)]
pub struct Trello;

impl Trello {
    pub fn new() -> Self {
        Self
    }
}

// ── the secret, which is a pair ──────────────────────────────────────

/// What joins the API key to the token in the one secret slot a connection has.
pub const PAIR_SEPARATOR: char = ':';

/// Split the stored secret into the API key and the token.
///
/// A colon, because neither half ever contains one — a Trello API key is 32 hex characters and a
/// token 64 — and because it is what the paste prompt asks the user for. A secret without one is a
/// refusal rather than a request made with half a credential.
pub fn split_secret(secret: &str) -> Result<(&str, &str)> {
    let (key, token) = secret
        .trim()
        .split_once(PAIR_SEPARATOR)
        .ok_or_else(|| anyhow!("a Trello credential is an API key and a token, as key:token"))?;
    if key.trim().is_empty() || token.trim().is_empty() {
        bail!("a Trello credential needs both an API key and a token");
    }
    Ok((key.trim(), token.trim()))
}

/// The `Authorization` header Trello wants. Not a bearer: the key identifies the application and
/// the token the user, and Trello refuses a request carrying only one of them.
pub fn auth_header(key: &str, token: &str) -> String {
    format!("OAuth oauth_consumer_key=\"{key}\", oauth_token=\"{token}\"")
}

/// The pair out of an identity, or a failure naming what is missing.
fn credential(who: &Identity) -> Result<(String, String)> {
    let secret = who
        .token
        .as_deref()
        .context("this Trello connection has no credential filed")?;
    let (key, token) = split_secret(secret)?;
    Ok((key.to_string(), token.to_string()))
}

// ── the network, which is four lines per verb ────────────────────────

/// One GET that answers JSON.
fn get(who: &Identity, path: &str) -> Result<Value> {
    request(who, "GET", path, &[])
}

/// One request with form parameters, for the three verbs that change something.
fn request(who: &Identity, method: &str, path: &str, fields: &[(&str, &str)]) -> Result<Value> {
    let (key, token) = credential(who)?;
    let seen = tls::seen();
    let url = format!("{API}{path}");
    let call = http::agent(who.pin.as_deref(), &seen)
        .request(method, &url)
        .set("Accept", "application/json")
        .set("Authorization", &auth_header(&key, &token));
    let response = if fields.is_empty() {
        call.call()
    } else {
        call.send_form(fields)
    };
    match response {
        Ok(response) => response
            .into_json()
            .with_context(|| format!("Trello answered {method} {path} with something not JSON")),
        Err(error) => Err(anyhow!("{:?}", http::map_error(error, &seen)))
            .with_context(|| format!("Trello refused {method} {path}")),
    }
}

// ── the parsing, which is everything worth testing ───────────────────

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

/// Trello stamps every date as RFC 3339 in UTC. A card whose date will not parse is not dropped —
/// the epoch is a value that sorts last and shows as obviously wrong, which is better than losing
/// the card.
fn when(value: &Value, key: &str) -> DateTime<Utc> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map_or_else(DateTime::<Utc>::default, |stamped| {
            stamped.with_timezone(&Utc)
        })
}

/// The boards an identity can see, out of `GET /1/members/me/boards`.
///
/// A closed board is skipped: it is not something a project's tasks should be bound to, and
/// Trello returns them alongside the open ones.
pub fn parse_boards(value: &Value) -> Vec<RemoteContainer> {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|board| board.get("closed").and_then(Value::as_bool) != Some(true))
        .filter_map(|board| {
            Some(RemoteContainer {
                id: RemoteContainerId::new(text(board, "id")?),
                name: text(board, "name").unwrap_or_default(),
                url: text(board, "shortUrl")
                    .or_else(|| text(board, "url"))
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// A board's lists, out of `GET /1/boards/{id}/lists`, in the board's own order.
///
/// An archived list is skipped for [`parse_boards`]'s reason: it is not a column anything should
/// be mapped onto.
pub fn parse_lists(value: &Value) -> Vec<RemoteLane> {
    let mut lanes: Vec<RemoteLane> = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|list| list.get("closed").and_then(Value::as_bool) != Some(true))
        .filter_map(|list| {
            Some(RemoteLane {
                id: RemoteLaneId::new(text(list, "id")?),
                name: text(list, "name").unwrap_or_default(),
                // Trello's `pos` is a float it rebalances; only the order it implies is read.
                position: list
                    .get("pos")
                    .and_then(Value::as_f64)
                    .map(|pos| pos as i64),
            })
        })
        .collect();
    lanes.sort_by_key(|lane| lane.position.unwrap_or(i64::MAX));
    lanes
}

/// A board's labels as candidates.
///
/// **The id is the swatch and the label is the name**, which is the case that forces a facet value
/// to carry both: a Trello label may have no name at all, and then the colour is the only thing to
/// show. A colour is never carried onto a task — `crates/ubiq/src/theme.rs` keeps its monopoly.
pub fn parse_labels(value: &Value) -> Vec<FacetValue> {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|label| {
            let id = text(label, "id")?;
            let name = text(label, "name").filter(|name| !name.is_empty());
            let colour = text(label, "color").unwrap_or_else(|| "no colour".to_string());
            Some(FacetValue::new(id, name.unwrap_or(colour)))
        })
        .collect()
}

/// A board's members as candidates. The full name is what a card's `assigned_to` becomes, so it is
/// what the picker shows; the username is the fallback for an account that set no name.
pub fn parse_members(value: &Value) -> Vec<FacetValue> {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|member| {
            let id = text(member, "id")?;
            let name = text(member, "fullName")
                .filter(|name| !name.is_empty())
                .or_else(|| text(member, "username"))
                .unwrap_or_else(|| id.clone());
            Some(FacetValue::new(id, name))
        })
        .collect()
}

/// The three candidate lists in one [`Facets`], as `facets()` answers.
pub fn parse_facets(lists: &Value, labels: &Value, members: &Value) -> Facets {
    let mut facets = Facets::default();
    facets.insert(
        FACET_LISTS,
        parse_lists(lists)
            .into_iter()
            .map(|lane| FacetValue::new(lane.id.to_string(), lane.name))
            .collect(),
    );
    facets.insert(FACET_LABELS, parse_labels(labels));
    facets.insert(FACET_MEMBERS, parse_members(members));
    facets
}

/// One card, out of anything that returns one.
///
/// `revision` is `dateLastActivity`: the only version token Trello has, it moves on any action,
/// and this client compares it rather than sending it (`R8`).
pub fn parse_card(card: &Value) -> Option<RemoteItem> {
    let id = text(card, "id")?;
    let labels = card
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| text(label, "name").filter(|name| !name.is_empty()))
                .collect()
        })
        .unwrap_or_default();
    let revision = text(card, "dateLastActivity").unwrap_or_default();
    Some(RemoteItem {
        id: RemoteItemId::new(id),
        // `#{idShort}` is what a person says out loud about a card; the long id is not.
        key: card
            .get("idShort")
            .and_then(Value::as_i64)
            .map(|short| format!("#{short}")),
        url: text(card, "shortUrl")
            .or_else(|| text(card, "url"))
            .unwrap_or_default(),
        lane: RemoteLaneId::new(text(card, "idList").unwrap_or_default()),
        title: text(card, "name").unwrap_or_default(),
        // Trello's `desc` is markdown, and nothing here parses it.
        body: text(card, "desc").unwrap_or_default(),
        labels,
        // Filled by `fetch`/`query` from the members facet: a card carries member *ids*, and a
        // task's `assigned_to` is free text, so the two are joined where the names are in hand.
        assignees: Vec::new(),
        // Trello has no type field and no priority field. The kind map is label → `Kind`, applied
        // above this by the binding, so nothing is hinted here.
        kind_hint: None,
        priority_hint: None,
        parent: None,
        checklist: Vec::new(),
        comments: Vec::new(),
        revision: Revision::new(revision),
        updated_at: when(card, "dateLastActivity"),
    })
}

/// The board's cards, filtered.
///
/// **Every filter is applied here**, because Trello has no query language — that is what
/// [`ProviderCaps::server_query`] being off means, and it is honest about the cost: a board is a
/// fine thing to filter client-side and a hundred-thousand-item tracker is not.
///
/// An empty filter field is **no constraint**, never "match nothing": a binding nobody narrowed
/// offers the whole board.
pub fn parse_cards(value: &Value, binding: &Binding) -> Vec<RemoteItem> {
    let lists = binding.filter.many(FIELD_LISTS);
    let labels = binding.filter.many(FIELD_LABELS);
    let members = binding.filter.many(FIELD_MEMBERS);

    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        // An archived card is not on the board, so it is not in the answer.
        .filter(|card| card.get("closed").and_then(Value::as_bool) != Some(true))
        .filter(|card| {
            lists.is_empty() || text(card, "idList").is_some_and(|list| lists.contains(&list))
        })
        .filter(|card| labels.is_empty() || has_any_id(card, "idLabels", labels, "labels"))
        .filter(|card| members.is_empty() || has_any_id(card, "idMembers", members, "members"))
        .filter_map(parse_card)
        .collect()
}

/// Whether a card carries any of the wanted ids.
///
/// Trello answers a card's labels two ways depending on what was asked for — a bare `idLabels`
/// array, or a `labels` array of objects — and a filter that only understood one of them would
/// silently match nothing against the other. Both are read.
fn has_any_id(card: &Value, id_key: &str, wanted: &[String], object_key: &str) -> bool {
    if let Some(ids) = card.get(id_key).and_then(Value::as_array)
        && ids
            .iter()
            .filter_map(Value::as_str)
            .any(|held| wanted.iter().any(|want| want == held))
    {
        return true;
    }
    card.get(object_key)
        .and_then(Value::as_array)
        .is_some_and(|objects| {
            objects
                .iter()
                .filter_map(|object| object.get("id").and_then(Value::as_str))
                .any(|held| wanted.iter().any(|want| want == held))
        })
}

/// Fill a card's assignees from the members facet, by id.
///
/// A member id the facet does not name is dropped rather than carried: the id means nothing to a
/// reader, and a task's `assigned_to` is what a person is called.
pub fn name_assignees(item: &mut RemoteItem, card: &Value, members: &[FacetValue]) {
    item.assignees = card
        .get("idMembers")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .filter_map(|id| members.iter().find(|member| member.id == id))
                .map(|member| member.label.clone())
                .collect()
        })
        .unwrap_or_default();
}

/// A card's comments, out of `GET /1/cards/{id}/actions?filter=commentCard`.
///
/// The author is carried for display and **never written onto a task's comment**: a task's comment
/// author is stamped by the host from the path the line arrived on (`D121`).
pub fn parse_comments(value: &Value) -> Vec<RemoteComment> {
    value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|action| text(action, "type").as_deref() == Some("commentCard"))
        .filter_map(|action| {
            // An account that set no full name answers `""` rather than omitting the key, so the
            // fallback has to test for empty and not merely for absent.
            let author = action
                .get("memberCreator")
                .and_then(|member| {
                    text(member, "fullName")
                        .filter(|name| !name.is_empty())
                        .or_else(|| text(member, "username"))
                })
                .unwrap_or_default();
            Some(RemoteComment {
                id: RemoteItemId::new(text(action, "id")?),
                author,
                text: action
                    .get("data")
                    .and_then(|data| text(data, "text"))
                    .unwrap_or_default(),
                created_at: when(action, "date"),
            })
        })
        .collect()
}

/// **The card's first checklist**, out of `GET /1/cards/{id}/checklists`.
///
/// Ubiq has one ordered checklist and Trello has many; the first by position is the only
/// unambiguous choice, and a second checklist on the card is left alone rather than merged or
/// deleted (`R10`). A card with no checklist answers an empty list, which is not a claim that it
/// has none — `ProviderCaps::checklist_read` is.
pub fn parse_checklists(value: &Value) -> Vec<RemoteCheckItem> {
    let lists = value.as_array().map(Vec::as_slice).unwrap_or_default();
    let Some(first) = lists
        .iter()
        .min_by(|a, b| position(a).total_cmp(&position(b)))
    else {
        return Vec::new();
    };
    let mut items: Vec<(f64, RemoteCheckItem)> = first
        .get("checkItems")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(|item| {
            Some((
                position(item),
                RemoteCheckItem {
                    id: RemoteCheckId::new(text(item, "id")?),
                    text: text(item, "name").unwrap_or_default(),
                    done: text(item, "state").as_deref() == Some("complete"),
                },
            ))
        })
        .collect();
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    items.into_iter().map(|(_, item)| item).collect()
}

fn position(value: &Value) -> f64 {
    value
        .get("pos")
        .and_then(Value::as_f64)
        .unwrap_or(f64::INFINITY)
}

/// The label ids a patch's label *names* mean on this board.
///
/// Trello writes labels by id, and the neutral model carries names — so the board's own label list
/// is what joins them. A name the board does not have is dropped rather than created: creating a
/// label on somebody's shared board is not something a sync pass should do unasked.
pub fn label_ids(names: &[String], board_labels: &[FacetValue]) -> Vec<String> {
    names
        .iter()
        .filter_map(|name| {
            board_labels
                .iter()
                .find(|label| label.label == *name)
                .map(|label| label.id.clone())
        })
        .collect()
}

// ── the trait ────────────────────────────────────────────────────────

impl TaskProvider for Trello {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        "Trello"
    }

    fn caps(&self) -> ProviderCaps {
        CAPS
    }

    /// Trello's own connector family. The connections screen does not offer one — a Trello
    /// identity is made through this layer's own setup surface (`R3`) — but once made it is an
    /// ordinary [`ProviderId::Trello`] connection, and that is what the binding's connection
    /// picker offers.
    fn connector(&self) -> Option<ubiq_proto::connectors::ProviderId> {
        Some(ubiq_proto::connectors::ProviderId::Trello)
    }

    fn config_schema(&self) -> &'static [ConfigField] {
        SCHEMA
    }

    fn containers(&self, who: &Identity) -> Result<Vec<RemoteContainer>> {
        let body = get(
            who,
            "/1/members/me/boards?filter=open&fields=id,name,url,shortUrl,closed",
        )?;
        Ok(parse_boards(&body))
    }

    fn lanes(&self, who: &Identity, binding: &Binding) -> Result<Vec<RemoteLane>> {
        let body = get(who, &format!("/1/boards/{}/lists", binding.container))?;
        Ok(parse_lists(&body))
    }

    fn facets(&self, who: &Identity, binding: &Binding) -> Result<Facets> {
        let board = &binding.container;
        let lists = get(who, &format!("/1/boards/{board}/lists"))?;
        let labels = get(who, &format!("/1/boards/{board}/labels"))?;
        let members = get(who, &format!("/1/boards/{board}/members"))?;
        Ok(parse_facets(&lists, &labels, &members))
    }

    fn query(&self, who: &Identity, binding: &Binding) -> Result<Vec<RemoteItem>> {
        let body = get(who, &format!("/1/boards/{}/cards", binding.container))?;
        let members = get(who, &format!("/1/boards/{}/members", binding.container))
            .map(|value| parse_members(&value))
            .unwrap_or_default();
        let cards = body.as_array().cloned().unwrap_or_default();
        let mut items = parse_cards(&body, binding);
        for item in &mut items {
            if let Some(card) = cards
                .iter()
                .find(|card| text(card, "id").as_deref() == Some(item.id.as_str()))
            {
                name_assignees(item, card, &members);
            }
        }
        Ok(items)
    }

    fn fetch(
        &self,
        who: &Identity,
        _binding: &Binding,
        ids: &[RemoteItemId],
    ) -> Result<Vec<RemoteItem>> {
        // One call per card. `GET /1/batch?urls=` takes ten at a time and is the lever when the
        // rate limit bites; the budget is spent per *linked* card, not per board card, so this is
        // the honest shape until a board with hundreds of links exists to measure.
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            let body = get(who, &format!("/1/cards/{id}"))?;
            if let Some(item) = parse_card(&body) {
                items.push(item);
            }
        }
        Ok(items)
    }

    fn create(&self, who: &Identity, binding: &Binding, draft: &RemoteDraft) -> Result<RemoteItem> {
        if draft.lane.as_str().is_empty() {
            bail!("a Trello card needs a list to land in");
        }
        let body = request(
            who,
            "POST",
            "/1/cards",
            &[
                ("idList", draft.lane.as_str()),
                ("name", draft.title.as_str()),
                ("desc", draft.body.as_str()),
            ],
        )?;
        let mut item = parse_card(&body).context("Trello answered a card that has no id")?;
        if !draft.labels.is_empty() {
            let labels = self.board_labels(who, binding)?;
            let ids = label_ids(&draft.labels, &labels).join(",");
            if !ids.is_empty() {
                let body = request(
                    who,
                    "PUT",
                    &format!("/1/cards/{}", item.id),
                    &[("idLabels", ids.as_str())],
                )?;
                item = parse_card(&body).unwrap_or(item);
            }
        }
        Ok(item)
    }

    fn update(
        &self,
        who: &Identity,
        binding: &Binding,
        id: &RemoteItemId,
        patch: &RemotePatch,
        expect: &Revision,
    ) -> Result<RemoteItem> {
        if patch.is_empty() {
            return self
                .fetch(who, binding, std::slice::from_ref(id))?
                .pop()
                .context("Trello no longer has this card");
        }
        // Trello honours no precondition, so the window is closed as far as a round trip can close
        // it: read immediately before writing and refuse when the token moved (`R8`). A write
        // inside that window still wins silently, and the settings surface says so.
        let current = get(who, &format!("/1/cards/{id}"))?;
        let seen = Revision::new(text(&current, "dateLastActivity").unwrap_or_default());
        if seen != *expect {
            bail!("this card changed at Trello since it was last read");
        }

        let labels = if patch.labels.is_some() {
            self.board_labels(who, binding)?
        } else {
            Vec::new()
        };
        let joined = patch
            .labels
            .as_ref()
            .map(|names| label_ids(names, &labels).join(","));

        let mut fields: Vec<(&str, &str)> = Vec::new();
        if let Some(lane) = &patch.lane {
            fields.push(("idList", lane.as_str()));
        }
        if let Some(title) = &patch.title {
            fields.push(("name", title.as_str()));
        }
        if let Some(body) = &patch.body {
            fields.push(("desc", body.as_str()));
        }
        if let Some(joined) = &joined {
            fields.push(("idLabels", joined.as_str()));
        }
        // `assignees` is deliberately not pushed: a task's `assigned_to` is free text and a Trello
        // member is an account, so writing one from the other would invent a membership.
        let body = request(who, "PUT", &format!("/1/cards/{id}"), &fields)?;
        parse_card(&body).context("Trello answered a card that has no id")
    }

    fn comment(
        &self,
        who: &Identity,
        _binding: &Binding,
        id: &RemoteItemId,
        text: &str,
    ) -> Result<RemoteComment> {
        let body = request(
            who,
            "POST",
            &format!("/1/cards/{id}/actions/comments"),
            &[("text", text)],
        )?;
        parse_comments(&Value::Array(vec![body]))
            .pop()
            .context("Trello answered a comment it will not describe")
    }

    fn set_check(
        &self,
        who: &Identity,
        _binding: &Binding,
        id: &RemoteItemId,
        item: &RemoteCheckId,
        done: bool,
    ) -> Result<()> {
        request(
            who,
            "PUT",
            &format!("/1/cards/{id}/checkItem/{item}"),
            &[("state", if done { "complete" } else { "incomplete" })],
        )?;
        Ok(())
    }
}

impl Trello {
    /// The bound board's labels, which is what joins a label *name* to the id Trello writes by.
    fn board_labels(&self, who: &Identity, binding: &Binding) -> Result<Vec<FacetValue>> {
        let body = get(who, &format!("/1/boards/{}/labels", binding.container))?;
        Ok(parse_labels(&body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_credential_is_a_pair_or_it_is_refused() {
        assert_eq!(split_secret("abc:def").expect("a pair"), ("abc", "def"));
        assert_eq!(split_secret(" abc : def ").expect("a pair"), ("abc", "def"));
        assert!(split_secret("just-a-token").is_err());
        assert!(split_secret(":def").is_err());
        assert!(split_secret("abc:").is_err());
    }

    #[test]
    fn the_header_names_both_halves() {
        assert_eq!(
            auth_header("k", "t"),
            "OAuth oauth_consumer_key=\"k\", oauth_token=\"t\""
        );
    }
}
