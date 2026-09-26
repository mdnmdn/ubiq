//! A card on a board somewhere, as one type.
//!
//! The task-source layer binds a project's board to a board at an external tracker — a Trello
//! board, a work-item query, a column of issues. This module is the **neutral half** of that: the
//! shape every provider answers in, and nothing any one tracker is proud of. A provider that needs
//! the layer to learn what a Trello list is has been written wrong.
//!
//! Three rules hold the whole module together:
//!
//! - **Every mapping is a string keyed onto a closed Ubiq set**, and the provider supplies the
//!   candidate strings by listing them. That is what lets a Trello list, a work-item state and an
//!   issue column all be "the lane" without anything here knowing which it is holding.
//! - **Capabilities, not conditionals.** [`ProviderCaps`] is a flag set the provider answers with,
//!   and the interface greys out what the bound provider cannot do rather than failing when it is
//!   asked. Nothing here branches on a provider's name.
//! - **Configuration is a declared schema, not a drawn view.** [`ConfigField`] is what a provider
//!   says it needs; the base renders it. A provider's own UI never reaches the interface crate.
//!
//! Nothing here holds credential material. A binding names a
//! [`ConnectionId`](crate::ids::ConnectionId) and the token behind it stays in the connector
//! family's secret store, which is the same discipline every other consumer of a connection
//! follows.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{BindingId, ConnectionId, TaskId};
use crate::work::{Kind, Priority, Status};

/// Declare one provider-scoped id kind.
///
/// These are `String`-backed rather than ULIDs, which is the difference from
/// [`crate::ids`]: the id is the **provider's**, minted by a tracker Ubiq does not run, and its
/// shape is that tracker's business — a 24-character Mongo id, a decimal work-item number, a
/// `gid://` URI. Nothing here parses one, and the newtypes exist for the same reason the ULID ones
/// do: a lane id and a card id are both strings, and only the compiler stops one being passed
/// where the other belongs.
///
/// Every kind gets the same surface, so none of them grows its own.
macro_rules! remote_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// Opaque and provider-scoped: two providers may mint the same string and mean different
        /// things, so an id is only ever compared against another from the same binding.
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wrap what the provider said. There is no validation, because there is no shape to
            /// validate against.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// The string underneath, for the URL a request is built into.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

remote_id! {
    /// One item at the remote — a Trello card, a work item, an issue.
    RemoteItemId
}

remote_id! {
    /// One lane at the remote: the dimension a provider says corresponds to a board column. A
    /// Trello list, a work-item state, a project column. Which dimension that is, is the
    /// provider's choice and it makes it in `lanes()`.
    RemoteLaneId
}

remote_id! {
    /// One container of items — a Trello board, a project, a repository. What a binding is bound
    /// *to*, and the scope every other id in a binding is meaningful inside.
    RemoteContainerId
}

remote_id! {
    /// One entry of an item's checklist. Distinct from a [`crate::ids::StepId`] on purpose: the
    /// two are different ids for the same line of text, and the link table holds the pair.
    RemoteCheckId
}

/// What the last successful read saw, as the provider expresses it.
///
/// Opaque because providers disagree about what a version even is: an ETag, a row version, a
/// last-activity timestamp. The layer stores it and hands it back as a precondition where
/// [`ProviderCaps::conditional_write`] says one is honoured, and compares it against a fresh read
/// where it is not. It is never ordered and never parsed — equal or not equal is the only question
/// asked of it.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(String);

impl Revision {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One container an identity can reach — a board, a project, a repository.
///
/// What the setup surface lists so the user picks rather than types an id. Nothing beyond the name
/// and the URL, because a picker needs no more and a provider that returns more would be inviting
/// the layer to display something only it understands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteContainer {
    pub id: RemoteContainerId,
    /// What the container is called at the remote. Display only — the map is keyed by id, so a
    /// rename at the remote costs nothing.
    pub name: String,
    /// Where a user goes to look at it. Carried so the settings surface can offer the link that
    /// proves the right board was picked.
    pub url: String,
}

/// One lane of a bound container — the candidate strings the lane map is built out of.
///
/// The lane map is keyed by [`RemoteLaneId`] rather than by [`Self::name`], because an id is
/// stable across the rename a name is not: a list renamed at the remote keeps its mapping instead
/// of falling out of the map and parking every task in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteLane {
    pub id: RemoteLaneId,
    /// What the lane is called at the remote, which is the only thing a user recognises when
    /// building the map.
    pub name: String,
    /// The lane's place in the remote's own ordering, where it has one. The setup surface lists
    /// lanes in this order so the map reads the way the board does; `None` is a provider whose
    /// lanes are a set rather than a sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
}

/// One entry of an item's checklist, as the remote holds it.
///
/// Matched against a task's steps **by text, not by position**: the two id spaces are unrelated
/// and a step inserted above another would otherwise retick the wrong line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCheckItem {
    pub id: RemoteCheckId,
    pub text: String,
    pub done: bool,
}

/// One comment on an item.
///
/// [`Self::author`] is what the remote said, carried for display and **never written onto a task's
/// comment**: a task's comment author is stamped by the host from the path the line arrived on
/// (`D121`), and a name from a tracker is not one of those paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteComment {
    /// The remote's own id for the comment. A [`RemoteItemId`] because providers overwhelmingly
    /// make a comment an addressable item of its own, and a second id kind for it would buy
    /// nothing the layer reads.
    pub id: RemoteItemId,
    pub author: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

/// One item at the remote, as every provider answers.
///
/// The type carries what every tracker has. A field a single tracker is proud of belongs in one of
/// the `_hint` strings or nowhere: the layer's job is to be the same shape for all of them, and a
/// field only Trello fills is a branch on a provider's name wearing a struct member's clothes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteItem {
    pub id: RemoteItemId,
    /// What a human says out loud — `#42`, `4711`, `UBQ-123`. `None` is a tracker with no such
    /// notion. Written onto a task's `key`, which is free text and the user's to clear, so this is
    /// never the sync identity — the link table is (`R11`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Where the item lives, for the chip on the card. Display only, exactly as a task's `link`
    /// is: nothing here parses a URL or fetches one.
    pub url: String,
    /// Which lane the item is in. Mapped onto a [`crate::work::Status`] through the binding's lane
    /// map; a lane the map does not name parks the task where it is rather than moving it (`R9`).
    pub lane: RemoteLaneId,
    pub title: String,
    /// The body, as markdown, exactly as the provider gives it. **Never parsed here** — the host
    /// has no markdown opinion about a task's description and gains none by fetching one.
    pub body: String,
    /// The remote's labels, by name. Names rather than ids because a label is matched against a
    /// task's own labels, which are names, and because a colour swatch is the interface's alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Display names, not ids: a task's `assigned_to` is free text on both sides, so a member id
    /// resolved here would be a second identity notion with nowhere to land.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<String>,
    /// The provider's own word for the item's type, unmapped. The kind map turns this into a
    /// [`crate::work::Kind`]; keeping it unmapped here is what lets one tracker's "Bug" and
    /// another's "defect" both be mappable without the layer holding a synonym table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_hint: Option<String>,
    /// The provider's own word for how much the item matters, unmapped, for [`Self::kind_hint`]'s
    /// reason. `None` is the usual case: most trackers have no priority field at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority_hint: Option<String>,
    /// The item this one hangs under, where the provider has a hierarchy at all. Only read when
    /// [`ProviderCaps::hierarchy`] is set, and depth is capped at one on Ubiq's side whatever the
    /// remote allows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<RemoteItemId>,
    /// The checklist, where the provider has one and [`ProviderCaps::checklist_read`] says it was
    /// fetched. Empty otherwise — an empty list is *not* a claim that the remote has none, which
    /// is why the cap exists rather than a sentinel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checklist: Vec<RemoteCheckItem>,
    /// The comments, on [`Self::checklist`]'s terms: fetched only where
    /// [`ProviderCaps::comments_read`] is set, and empty is not a claim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<RemoteComment>,
    /// What this read saw, to be handed back as a precondition or compared against the next read.
    pub revision: Revision,
    /// When the remote last saw the item change, by the remote's clock. Never compared against a
    /// local clock to decide anything — the two are not the same clock — only shown, and used to
    /// order a listing.
    pub updated_at: DateTime<Utc>,
}

/// A new item, on its way to the remote.
///
/// Separate from [`RemoteItem`] because the ids, the revision and the timestamps are the remote's
/// to mint and a draft carrying empty ones would be a type that lies about what it knows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDraft {
    /// Where it lands. Required: every tracker puts a new item somewhere, and the binding's lane
    /// map is what chooses.
    pub lane: RemoteLaneId,
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<String>,
}

impl Default for RemoteLaneId {
    /// The empty lane, which is what a [`RemoteDraft`] holds before the lane map has chosen one.
    /// Never sent: a provider refuses a draft whose lane is empty rather than guessing a default,
    /// because guessing puts somebody's card in the wrong column of a shared board.
    fn default() -> Self {
        Self::new(String::new())
    }
}

/// A change to an item: one `Option` per writable field.
///
/// **Absent means leave alone, present means set** — `SetTaskField`'s own discipline, for the
/// reason it exists there. A patch that spelled a no-op as an equal value would push every
/// unchanged field on every pass and make the remote's activity feed useless.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<RemoteLaneId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The whole label set, where it is set at all. A list rather than an add/remove pair because
    /// no provider agrees on how to express a removal, and a whole set is the one form every
    /// tracker's API can be driven to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,
}

impl RemotePatch {
    /// Whether this patch would change anything. A pass skips an empty one rather than making a
    /// round trip that asks the remote to do nothing.
    pub fn is_empty(&self) -> bool {
        self.lane.is_none()
            && self.title.is_none()
            && self.body.is_none()
            && self.labels.is_none()
            && self.assignees.is_none()
    }
}

/// What a provider can do, as it answers for itself.
///
/// The interface greys out what the bound provider cannot do rather than failing when it is asked,
/// and that is the whole mechanism keeping provider specifics out of the core. Every flag is a
/// statement about the *provider*, never about one binding's permissions: a token too weak to
/// write is a failed request, not a cleared flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCaps {
    /// Whether an item can be moved between lanes.
    pub write_lane: bool,
    pub write_title: bool,
    pub write_body: bool,
    pub write_labels: bool,
    pub write_assignee: bool,
    /// Whether [`RemoteItem::comments`] is filled at all. Absent comments on a provider without
    /// this are absent because nothing looked, not because there are none.
    pub comments_read: bool,
    pub comments_write: bool,
    /// Whether [`RemoteItem::checklist`] is filled, on [`Self::comments_read`]'s terms.
    pub checklist_read: bool,
    pub checklist_write: bool,
    /// Whether a write honours [`Revision`] as a precondition. Off means the layer re-reads
    /// immediately before writing and compares — which narrows the window to a round trip and does
    /// not close it, and the settings surface says so rather than pretending otherwise (`R8`).
    pub conditional_write: bool,
    /// Whether the filter is applied by the remote. Off means every filter is applied client-side
    /// after a full fetch, which is fine for a board and wrong for a tracker with a hundred
    /// thousand items.
    pub server_query: bool,
    /// Whether [`RemoteItem::parent`] means anything here.
    pub hierarchy: bool,
}

/// What a provider's configuration field is made of.
///
/// `Choice` and `MultiChoice` name a facet rather than carrying candidates, because the candidates
/// depend on which container is bound and are fetched — a schema is a `&'static` fact about the
/// provider, and a fetched list is not.
///
/// **It travels both ways, and the inbound half interns** (`D187`). The schema was first written
/// `Serialize`-only, on the reading that it only ever travels outward. That reading was wrong, and
/// M7 is where it broke: [`ProviderInfo`] is the payload of `TaskProviders`, and
/// [`crate::messages::Message`] derives `Deserialize` for the whole enum, so a schema that cannot
/// be read back cannot reach the interface at all — which is the one place it has to reach, since
/// a second edition's provider compiles into the *host* half and the renderer lives in the
/// interface half. The fix keeps one type rather than growing an owned twin: [`intern`] turns a
/// deserialised `String` into a `&'static str`, so the field stays `Copy` and a provider's own
/// `config_schema()` stays a plain `const`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldKind {
    /// A line of text. A tracker's query language is this plus a Test.
    Text,
    /// Material. Never written into the binding file: a secret field's value goes to the connector
    /// family's secret store like every other credential, and the binding holds a reference.
    Secret,
    Bool,
    /// Candidates come from `TaskProvider::facets()`, keyed by this facet name (e.g. `"labels"`,
    /// `"members"`, `"lists"`).
    Choice(&'static str),
    MultiChoice(&'static str),
}

/// One field a provider declares it needs, for the base to render.
///
/// A declared schema rather than a view the provider draws, which is the single decision that
/// keeps every provider's UI out of the interface crate (`R6`). The cost is honest and bounded: a
/// provider whose configuration does not fit cannot ship until the schema grows, and that shows up
/// as a missing [`ConfigFieldKind`] rather than as a leak.
///
/// `Copy`, because every field of it is a `&'static str` or a `bool` — a schema is a compiled-in
/// fact about a provider and never something built at run time. See [`ConfigFieldKind`] for how
/// the pair survives a round trip without giving that up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ConfigField {
    /// What the binding's config map keys this value under. Stable: renaming it orphans whatever
    /// the user already configured.
    pub key: &'static str,
    /// What the field is called on screen.
    pub label: &'static str,
    pub kind: ConfigFieldKind,
    /// Whether a binding without this field is incomplete. The setup surface refuses to save one
    /// that is.
    pub required: bool,
    /// Whether this field's value can be checked with a Test round trip before saving —
    /// `TestTaskSource` on the wire (R7). A query field carries this; a board picker does not,
    /// because picking already proves the id exists.
    pub testable: bool,
}

/// Turn a string that arrived over the wire into a `&'static str`, once per distinct string.
///
/// A schema is a closed, tiny, compiled-in fact — four or five fields per provider, a handful of
/// providers — and the interface asks for it when a settings page opens. Interning is what makes
/// "leak it" bounded rather than a slow leak per redraw: the same `"wiql"` deserialised a hundred
/// times is one allocation. The alternative, an owned twin of [`ConfigField`] that only the
/// interface names, is the two-types-for-one-fact shape `R6` exists to avoid.
fn intern(text: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static POOL: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(HashSet::new()));
    let mut pool = pool.lock().unwrap_or_else(|held| held.into_inner());
    if let Some(found) = pool.get(text) {
        return found;
    }
    let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
    pool.insert(leaked);
    leaked
}

/// The owned shape [`ConfigFieldKind`] arrives in, before [`intern`] gives its facet name back its
/// `'static` lifetime. Private: nothing outside this module ever holds one.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConfigFieldKindWire {
    Text,
    Secret,
    Bool,
    Choice(String),
    MultiChoice(String),
}

impl<'de> Deserialize<'de> for ConfigFieldKind {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        Ok(match ConfigFieldKindWire::deserialize(de)? {
            ConfigFieldKindWire::Text => ConfigFieldKind::Text,
            ConfigFieldKindWire::Secret => ConfigFieldKind::Secret,
            ConfigFieldKindWire::Bool => ConfigFieldKind::Bool,
            ConfigFieldKindWire::Choice(facet) => ConfigFieldKind::Choice(intern(&facet)),
            ConfigFieldKindWire::MultiChoice(facet) => ConfigFieldKind::MultiChoice(intern(&facet)),
        })
    }
}

/// [`ConfigField`]'s owned twin, on [`ConfigFieldKindWire`]'s terms.
#[derive(Deserialize)]
struct ConfigFieldWire {
    key: String,
    label: String,
    kind: ConfigFieldKind,
    required: bool,
    testable: bool,
}

impl<'de> Deserialize<'de> for ConfigField {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let wire = ConfigFieldWire::deserialize(de)?;
        Ok(ConfigField {
            key: intern(&wire.key),
            label: intern(&wire.label),
            kind: wire.kind,
            required: wire.required,
            testable: wire.testable,
        })
    }
}

/// One candidate a [`ConfigFieldKind::Choice`] or [`ConfigFieldKind::MultiChoice`] offers.
///
/// Both halves are needed and neither substitutes for the other: the id is what the binding
/// stores, so a rename at the remote costs nothing, and the label is the only thing the user
/// recognises. A Trello label is the case that forces it — its id is a swatch key and its name is
/// what is written on the card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetValue {
    pub id: String,
    pub label: String,
}

impl FacetValue {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// The candidate lists a bound container offers, keyed by facet name.
///
/// One call rather than one per facet, because every provider answering them fetches the same
/// container description to do it and three calls would be three round trips against the same rate
/// limit. A facet nobody declared a field for is simply not looked up.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Facets {
    #[serde(flatten)]
    pub values: BTreeMap<String, Vec<FacetValue>>,
}

impl Facets {
    /// The candidates for one facet. An empty slice for a facet the provider did not fill, which
    /// is a picker with nothing in it rather than an error — a board with no labels is ordinary.
    pub fn get(&self, facet: &str) -> &[FacetValue] {
        self.values.get(facet).map_or(&[], Vec::as_slice)
    }

    pub fn insert(&mut self, facet: impl Into<String>, values: Vec<FacetValue>) {
        self.values.insert(facet.into(), values);
    }
}

/// One provider, as the setup surface is told about it.
///
/// The schema is owned rather than `&'static [ConfigField]` because this crosses the wire, and a
/// borrowed payload is exactly what the contract crate refuses. The provider's own
/// `config_schema()` stays `&'static`; this is its copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProviderInfo {
    /// The provider's stable key — `"trello"`. What a binding names, so it survives a change of
    /// [`Self::label`].
    pub id: &'static str,
    pub label: &'static str,
    pub caps: ProviderCaps,
    pub config_schema: Vec<ConfigField>,
    /// Which connector family this provider's identities come from, so the setup surface can
    /// offer the connections that could actually authenticate it (`D189`).
    ///
    /// **`None` is not "any connection"** — it is *this build holds no connector family that can
    /// authenticate this provider*, and the setup surface says exactly that rather than offering a
    /// list where every entry is wrong. ClickUp is the live example: the provider is complete, and
    /// [`crate::connectors::ProviderId`] has no variant for it, so it is registered, listed and
    /// honestly unconnectable until one exists.
    #[serde(default)]
    pub connector: Option<crate::connectors::ProviderId>,
}

/// [`ProviderInfo`]'s owned twin, on [`ConfigFieldWire`]'s terms.
#[derive(Deserialize)]
struct ProviderInfoWire {
    id: String,
    label: String,
    caps: ProviderCaps,
    config_schema: Vec<ConfigField>,
    #[serde(default)]
    connector: Option<crate::connectors::ProviderId>,
}

impl<'de> Deserialize<'de> for ProviderInfo {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let wire = ProviderInfoWire::deserialize(de)?;
        Ok(ProviderInfo {
            id: intern(&wire.id),
            label: intern(&wire.label),
            caps: wire.caps,
            config_schema: wire.config_schema,
            connector: wire.connector,
        })
    }
}

/// Which way work moves across a binding.
///
/// `Pull` is the default and stays the default: a first binding that silently started writing to
/// somebody's shared board is the failure mode that loses trust in the whole feature (`R13`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Pull,
    TwoWay,
}

/// Who wins when **both sides changed** — the only case it is consulted in.
///
/// A switch rather than a merge. Every other combination has nothing to decide: one side changed,
/// so that side's value is the answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    /// The local edit wins, and the remote's value is kept in a comment.
    UbiqWins,
    /// The remote's value wins, and the local one is kept in a comment. The default, because it is
    /// the one that never silently overwrites somebody else's board.
    #[default]
    RemoteWins,
}

/// The values a binding holds for the provider's declared configuration schema.
///
/// Keyed by [`ConfigField::key`], and a list rather than a string so a `MultiChoice` needs no
/// second shape: a `Text`, a `Bool` or a `Choice` holds exactly one element, a `MultiChoice` holds
/// as many as were picked, and an absent key is a field nobody filled. That is also what makes the
/// whole thing round-trip through TOML with no custom serialisation.
///
/// The filter *is* this: `R6` makes a provider's configuration and its filter one declared schema,
/// so there is no second struct for "which items the binding is about".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    #[serde(flatten)]
    pub fields: BTreeMap<String, Vec<String>>,
}

impl Filter {
    /// The single value of a `Text`, `Bool` or `Choice` field. `None` for a field nobody filled.
    pub fn one(&self, key: &str) -> Option<&str> {
        self.fields.get(key)?.first().map(String::as_str)
    }

    /// Every value of a `MultiChoice` field. Empty means **no constraint**, never "match nothing":
    /// a filter a user has not narrowed offers the whole container.
    pub fn many(&self, key: &str) -> &[String] {
        self.fields.get(key).map_or(&[], Vec::as_slice)
    }

    /// A `Bool` field, absent reading as false.
    pub fn flag(&self, key: &str) -> bool {
        self.one(key) == Some("true")
    }

    pub fn set(&mut self, key: impl Into<String>, values: Vec<String>) {
        self.fields.insert(key.into(), values);
    }
}

/// The default poll interval: five minutes.
pub const POLL_DEFAULT: u64 = 300;

/// The floor under a poll interval, in seconds. A user who asks for less gets this, because a pass
/// spends a rate-limit budget per linked card and a board polled every ten seconds exhausts it
/// (`R14`).
pub const POLL_FLOOR: u64 = 60;

/// One project's binding to a board somewhere else.
///
/// **Everything provider-specific is a string the provider itself supplied.** The lane, kind and
/// priority maps are keyed by ids out of `lanes()` and `facets()`, and [`Self::filter`] is keyed
/// by the provider's own `config_schema()`. That is what lets this type be the same for Trello, a
/// work-item tracker and an issue column without knowing which it is holding (`R5`, `R6`).
///
/// **Here rather than in the host** (`D187`): the settings surface edits one and `SetTaskSource`
/// carries it back, so it is contract vocabulary. The host re-exports it, and the file it is
/// written to is still the host's alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// Named from the first row even though there is one per project, so a second board is a
    /// setup-surface change and not a file migration (`R4`).
    pub id: BindingId,
    /// Which provider answers for it — `TaskProvider::id()`, not a `ProviderId`. The two are
    /// different questions: this says who fetches, the connection says who authenticates.
    pub provider: String,
    /// The identity the calls are made as. A reference: nothing here is material.
    pub connection: ConnectionId,
    /// The board, project or repository this is bound to. Filled from the `Choice` field the
    /// provider declares for it, and kept here rather than only in [`Self::filter`] because every
    /// other id in the binding is only meaningful inside it.
    pub container: RemoteContainerId,
    /// Remote lane id → the column a task in it sits in. Keyed by **id**, so a lane renamed at the
    /// remote keeps its mapping; a lane the map does not name parks the task where it is (`R9`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lanes: BTreeMap<String, Status>,
    /// A provider's own word for a type → a [`Kind`]. Empty by default: most trackers have no type
    /// field, and a guessed map is worse than none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kinds: BTreeMap<String, Kind>,
    /// The same, for [`Priority`]. Empty by default for the same reason, and emptier: a tracker
    /// with a native priority field is the exception.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub priorities: BTreeMap<String, Priority>,
    /// What the provider's declared configuration schema was filled with — which is also what the
    /// filter is. See [`Filter`].
    #[serde(default)]
    pub filter: Filter,
    #[serde(default)]
    pub direction: Direction,
    #[serde(default)]
    pub authority: Authority,
    /// Seconds between passes, as asked for. Read through [`Self::poll_seconds`], which applies
    /// the floor — the stored value is the user's and is never quietly rewritten.
    #[serde(default = "default_poll")]
    pub poll: u64,
    /// Whether every item the filter matches becomes a task without being picked. Off, because
    /// import is explicit and the filter governs what is *offered* rather than what is created
    /// (`R12`).
    #[serde(default)]
    pub auto_import: bool,
}

fn default_poll() -> u64 {
    POLL_DEFAULT
}

impl Binding {
    /// A fresh binding: bound, mapped by nothing, pulling only.
    pub fn new(
        provider: impl Into<String>,
        connection: ConnectionId,
        container: RemoteContainerId,
    ) -> Self {
        Self {
            id: BindingId::generate(),
            provider: provider.into(),
            connection,
            container,
            lanes: BTreeMap::new(),
            kinds: BTreeMap::new(),
            priorities: BTreeMap::new(),
            filter: Filter::default(),
            direction: Direction::Pull,
            authority: Authority::default(),
            poll: POLL_DEFAULT,
            auto_import: false,
        }
    }

    /// The interval a pass actually runs at, with [`POLL_FLOOR`] applied.
    pub fn poll_seconds(&self) -> u64 {
        self.poll.max(POLL_FLOOR)
    }

    /// Which column an item in this lane belongs in, or `None` for a lane nobody mapped — which
    /// parks the task where it is rather than moving it.
    pub fn status_of(&self, lane: &RemoteLaneId) -> Option<Status> {
        self.lanes.get(lane.as_str()).copied()
    }

    /// The lane a task in this column is pushed to. The first mapping that names the column, in
    /// the map's own order, because the lane map is many-to-one and the reverse is a choice: two
    /// Trello lists both mapped to `Done` are both "done", and a push has to pick one.
    pub fn lane_of(&self, status: Status) -> Option<RemoteLaneId> {
        self.lanes
            .iter()
            .find(|(_, mapped)| **mapped == status)
            .map(|(lane, _)| RemoteLaneId::new(lane.clone()))
    }

    /// The [`Kind`] this item maps to, or `None` for one nothing names.
    ///
    /// **The provider's own word first, then the item's labels.** A tracker with a type field says
    /// it in [`RemoteItem::kind_hint`]; a tracker without one — Trello — has no type at all, and
    /// the convention there is a label, so the same map serves both keyed by whichever string
    /// arrived. `None` is a kind nobody mapped, and an unmapped kind is left alone rather than
    /// guessed, for `R9`'s reason.
    pub fn kind_of(&self, item: &RemoteItem) -> Option<Kind> {
        if let Some(hint) = item.kind_hint.as_deref()
            && let Some(kind) = self.kinds.get(hint)
        {
            return Some(*kind);
        }
        item.labels
            .iter()
            .find_map(|label| self.kinds.get(label))
            .copied()
    }

    /// The [`Priority`] this item maps to, on [`Self::kind_of`]'s own terms and for its reasons.
    /// `None` is the usual case: a tracker with a native priority field is the exception.
    pub fn priority_of(&self, item: &RemoteItem) -> Option<Priority> {
        if let Some(hint) = item.priority_hint.as_deref()
            && let Some(priority) = self.priorities.get(hint)
        {
            return Some(*priority);
        }
        item.labels
            .iter()
            .find_map(|label| self.priorities.get(label))
            .copied()
    }

    /// Whether this binding may write at all.
    pub fn writes(&self) -> bool {
        self.direction == Direction::TwoWay
    }
}

/// Where one linked task has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    /// The two sides agree, as of [`TaskLink::synced_at`].
    #[default]
    Linked,
    /// The item's remote lane is one the binding's lane map does not name, so the task was left
    /// exactly where it is. **This is the badge**: the card says its lane is unbound rather than
    /// moving to a column nobody chose. The settled Trello map binds three lanes and leaves four
    /// unbound on purpose, and a guess would move somebody's card silently (`R9`).
    Parked,
    /// Both sides changed the same field. The authority switch settled it and the loser's value is
    /// in a comment; the card says so until somebody looks.
    Drifted,
    /// A write was refused because somebody wrote between the fetch and the push. Never retried by
    /// overwriting — a human decides.
    Conflict,
    /// The item left the filter, or was deleted at the remote. The task keeps its content and its
    /// history and loses its link (`R9`); nothing deletes anything.
    Unlinked,
}

impl LinkState {
    /// What the card's dot says out loud. One sentence, because a badge with no explanation is a
    /// coloured dot somebody has to ask about.
    pub fn note(self) -> &'static str {
        match self {
            LinkState::Linked => "In step with the remote item.",
            LinkState::Parked => {
                "Parked: the remote lane is not in this binding's lane map, so the task was left \
                 where it is rather than moved to a column nobody chose."
            }
            LinkState::Drifted => {
                "Drifted: both sides changed the same field. The authority switch settled it and \
                 the loser's value is in a comment."
            }
            LinkState::Conflict => {
                "Conflict: a write was refused because somebody wrote between the fetch and the \
                 push. Nothing was overwritten."
            }
            LinkState::Unlinked => {
                "Unlinked: the item left the filter or is gone from the remote. The task keeps \
                 everything it had and lost only its link."
            }
        }
    }

    /// What the badge reads. Short, because it sits on a card beside the title.
    pub fn label(self) -> &'static str {
        match self {
            LinkState::Linked => "Synced",
            LinkState::Parked => "Parked",
            LinkState::Drifted => "Drifted",
            LinkState::Conflict => "Conflict",
            LinkState::Unlinked => "Unlinked",
        }
    }

    /// Whether this state is worth drawing a badge for at all. `Linked` is the ordinary case and
    /// says nothing a person needs told; the other four are all somebody's business.
    pub fn is_notable(self) -> bool {
        self != LinkState::Linked
    }
}

/// One row of the link table: the identity of a synced task, and what the last pass saw.
///
/// **This is the sync identity**, not the task's `key` or `link`. Those are the user's fields,
/// free text and clearable, and a user who clears `link` has not unlinked the task (`R11`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLink {
    /// Which binding this row belongs to. Present from the first row, for [`Binding::id`]'s
    /// reason.
    pub binding: BindingId,
    pub task: TaskId,
    pub item: RemoteItemId,
    /// Kept beside the item even though the binding names one, because an item that moved between
    /// containers is a fact the next pass has to notice rather than assume away.
    pub container: RemoteContainerId,
    /// What the last successful read saw. The precondition for the next write where the provider
    /// honours one, and the comparison where it does not.
    pub revision: Revision,
    /// **Per field**, not per record: the dirty test asks which *field* changed on which side, and
    /// a whole-record hash can only answer "something did". Keyed by the field's own name —
    /// `title`, `body`, `lane`, `labels`, `assignees`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hash: BTreeMap<String, String>,
    /// The same digests taken over the **task**, at the same moment (`D188`).
    ///
    /// Two maps rather than one, because the dirty test is two questions and not one: [`Self::hash`]
    /// answers *did the remote move since the last pass*, this answers *did we*. Comparing a task
    /// against [`Self::hash`] cannot work — a lane map is many-to-one, so two lists that both mean
    /// `Done` would read as a change every pass — and normalising one side onto the other would
    /// make every mapping decision a hashing decision. Each side is hashed in its own vocabulary
    /// and compared only against its own previous value.
    ///
    /// Empty on a row written before outbound existed, which reads as *the local side has not
    /// changed* and is the safe answer: the pass pulls, exactly as `D185` did.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub local: BTreeMap<String, String>,
    /// The fields whose two sides differ right now, and which way the binding's switch would
    /// settle each. **This is the drift overview's whole content** — it rides the link row rather
    /// than a message of its own, because a row is already broadcast on every change and already
    /// persisted, and drift is a fact about one task and not about a query.
    ///
    /// Empty for a task in step, which is almost all of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<FieldDrift>,
    pub synced_at: DateTime<Utc>,
    #[serde(default)]
    pub state: LinkState,
}

/// Which way one field is settled — the two things a person can ask for, and the two a pass can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftSide {
    /// The local value wins and is written to the remote item.
    Push,
    /// The remote value wins and is written to the task.
    Pull,
}

impl DriftSide {
    pub fn label(self) -> &'static str {
        match self {
            DriftSide::Push => "Push",
            DriftSide::Pull => "Pull",
        }
    }
}

/// One field of one task whose two sides disagree.
///
/// Carries **both values, as text**, because the drift overview's job is to let somebody read the
/// two and choose — a row that said only "the title differs" would send them to the browser to
/// find out what it differs *to*, which is the question the overview exists to answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDrift {
    /// The field's own name, as the per-field hash keys it: `title`, `body`, `lane`, `labels`,
    /// `assignees`, `kind`, `priority`, `key`, `url`.
    pub field: String,
    /// What the task says, rendered for reading and never parsed back.
    pub local: String,
    /// What the remote item says, on [`Self::local`]'s terms.
    pub remote: String,
    /// Which way the binding's authority switch settles this field — and **`None` where the
    /// provider cannot take the write at all**, which is a standing divergence rather than a
    /// decision anybody gets to make. A capability a provider does not have is not attempted, and
    /// this is where it is said out loud rather than dropped (`D188`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settles: Option<DriftSide>,
    /// Why, in one sentence. Drawn beside the row.
    pub note: String,
}

/// What one project's binding is doing, as the board's status item reads it.
///
/// Distinct from [`LinkState`], which is one task's: this is the binding's, and the two answer
/// different questions — "is the board in step" against "is this card".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// No binding on this project. The status item draws nothing at all.
    #[default]
    Unbound,
    /// Bound and between passes.
    Idle,
    /// A pass is running.
    Running,
    /// The last pass failed. The sentence is on the message, not here.
    Failed,
}

/// What one pass, or one force, is asked to cover.
///
/// The same message serves the board's force button and a card's, which is why this exists rather
/// than two messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncScope {
    All,
    Task(TaskId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_opaque_and_round_trips_as_a_bare_string() {
        let id = RemoteItemId::new("5f2a7b1c9d0e");
        let json = serde_json::to_string(&id).expect("an id serialises");
        assert_eq!(json, "\"5f2a7b1c9d0e\"");
        assert_eq!(
            serde_json::from_str::<RemoteItemId>(&json).expect("and back"),
            id
        );
    }

    #[test]
    fn an_empty_patch_asks_the_remote_for_nothing() {
        assert!(RemotePatch::default().is_empty());
        assert!(
            !RemotePatch {
                title: Some("moved".into()),
                ..RemotePatch::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn a_facet_nobody_filled_is_an_empty_picker_rather_than_an_error() {
        let mut facets = Facets::default();
        facets.insert("labels", vec![FacetValue::new("red", "blocked")]);
        assert_eq!(facets.get("labels").len(), 1);
        assert!(facets.get("members").is_empty());
    }

    #[test]
    fn caps_default_to_a_provider_that_can_do_nothing() {
        // The default has to be the closed one: a provider that forgot to answer must grey
        // everything out rather than have the interface offer a write it cannot make.
        let caps = ProviderCaps::default();
        assert!(!caps.write_lane);
        assert!(!caps.conditional_write);
        assert!(!caps.server_query);
    }

    #[test]
    fn a_declared_schema_survives_the_wire_and_comes_back_static() {
        // The whole of `R6` depends on this: a provider compiled into the *host* half declares a
        // schema, and the renderer that draws it lives in the *interface* half. If the schema
        // cannot be read back, a second edition's provider can never be drawn.
        let info = ProviderInfo {
            id: "ado",
            label: "Azure DevOps",
            caps: ProviderCaps {
                server_query: true,
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
                    key: "wiql",
                    label: "WIQL condition",
                    kind: ConfigFieldKind::Text,
                    required: false,
                    testable: true,
                },
            ],
            connector: Some(crate::connectors::ProviderId::AzureDevops),
        };
        let json = serde_json::to_string(&info).expect("a schema serialises");
        let back: ProviderInfo = serde_json::from_str(&json).expect("and back");
        assert_eq!(back, info);
        assert!(back.config_schema[1].testable);
        // `D189`: the connector family crosses with the schema. Without it the interface cannot
        // offer the connections that could authenticate a second edition's provider.
        assert_eq!(
            back.connector,
            Some(crate::connectors::ProviderId::AzureDevops)
        );
    }

    #[test]
    fn interning_hands_back_the_same_pointer_for_the_same_string() {
        // What makes "leak it" bounded rather than a leak per redraw.
        let first = intern("iterations");
        let second = intern(&String::from("iterations"));
        assert!(std::ptr::eq(first, second));
    }

    #[test]
    fn only_a_linked_task_says_nothing() {
        assert!(!LinkState::Linked.is_notable());
        for state in [
            LinkState::Parked,
            LinkState::Drifted,
            LinkState::Conflict,
            LinkState::Unlinked,
        ] {
            assert!(state.is_notable());
            assert!(!state.label().is_empty());
        }
    }
}
