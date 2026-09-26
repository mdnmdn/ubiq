//! What the interface knows about a project's binding to a board somewhere else.
//!
//! Everything here arrived across [`ubiq_proto::messages`] and nothing in it is provider-shaped.
//! That is the whole point of `M7`: the settings section, the import dialog, the card badge and
//! the board's status item are drawn once, from a schema the provider **declared** rather than a
//! view it drew (`R6`), and the only thing that varies between Trello, Azure DevOps and ClickUp is
//! the contents of [`TaskSrcState::providers`].
//!
//! **Two rules this module exists to keep honest, and both are enforced by shape rather than by
//! discipline:**
//!
//! - **No per-provider branch.** Nothing here — and nothing in [`crate::ui::tasksrc`] — matches on
//!   [`Binding::provider`]. A provider's configuration reaches the screen as
//!   [`ubiq_proto::tasksrc::ConfigField`]s and its limits reach it as [`ProviderCaps`]; if a
//!   provider ever needed a branch, that would be a hole in `R6` and a finding, not a patch.
//! - **No capability branch in the drawing code either.** The controls a capability can grey out
//!   are a **table** — [`ABILITIES`] and [`CAVEATS`] — which the section iterates. A control that
//!   depends on a capability is a row there; the draw path reads `(row.needs)(caps)` and knows
//!   nothing about which flag it just asked about. That is the same move `X16` makes for a
//!   container's spec fields, applied to a flag set.
//!
//! **Nothing here holds credential material.** A [`ubiq_proto::tasksrc::ConfigFieldKind::Secret`]
//! field is typed into an input whose value goes out on `SetTaskSource` and is filed by the host
//! in the connector family's secret store; the binding on disk holds the reference alone. The
//! interface never reads one back, which is why a secret field draws as empty-with-a-note rather
//! than as a masked value it does not have.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use gpui::Entity;
use gpui_component::input::InputState;
use ubiq_proto::connectors::Connection;
use ubiq_proto::ids::{ProjectId, TaskId, TaskSrcQueryId};
use ubiq_proto::tasksrc::{
    Binding, Facets, ProviderCaps, ProviderInfo, RemoteContainer, RemoteItem, RemoteItemId,
    RemoteLane, SyncState, TaskLink,
};

/// One thing a binding can be configured to do, and the capability that says whether the bound
/// provider can do it at all.
///
/// A table rather than a run of `if caps.write_lane` in the drawing code. The difference is not
/// style: a branch per flag is eleven places to forget one, and the plan's own gate says a
/// capability match in the renderer is the smell that the schema is too narrow. Here the draw path
/// is one loop and the flags are data.
pub struct Ability {
    pub key: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    /// The flag that must be on for this control to do anything. Returning `true` unconditionally
    /// is a control every provider has.
    pub needs: fn(&ProviderCaps) -> bool,
    /// What the row says instead, when the bound provider cannot do it. Drawn faint beside the
    /// greyed control — a disabled thing with no reason given is a thing somebody files a bug
    /// about.
    pub absent: &'static str,
}

/// The write-side controls, in drawn order.
///
/// Each is a thing the *binding* can be told to do and the provider may not be able to. The
/// section draws every row whatever the provider answers, greyed where it cannot: a control that
/// vanished would make two providers' pages different shapes, and the point of the page is that
/// they are not.
pub const ABILITIES: &[Ability] = &[
    Ability {
        key: "write_lane",
        label: "Move cards between lanes",
        note: "A task dragged to another column moves the remote item to the lane the map names.",
        needs: |caps| caps.write_lane,
        absent: "This provider does not accept a lane change.",
    },
    Ability {
        key: "write_title",
        label: "Push the title",
        note: "A renamed task renames the remote item.",
        needs: |caps| caps.write_title,
        absent: "This provider does not accept a title change.",
    },
    Ability {
        key: "write_body",
        label: "Push the description",
        note: "An edited description is written to the remote item's body.",
        needs: |caps| caps.write_body,
        absent: "This provider does not accept a body change.",
    },
    Ability {
        key: "write_labels",
        label: "Push labels",
        note: "The whole label set is written, because no provider agrees how to express a removal.",
        needs: |caps| caps.write_labels,
        absent: "This provider does not accept a label change.",
    },
    Ability {
        key: "write_assignee",
        label: "Push the assignee",
        note: "Written as a display name on both sides.",
        needs: |caps| caps.write_assignee,
        absent: "This provider does not accept an assignee change.",
    },
    Ability {
        key: "comments_write",
        label: "Push comments",
        note: "A comment added here is added to the remote item.",
        needs: |caps| caps.comments_write,
        absent: "This provider does not accept a comment.",
    },
    Ability {
        key: "checklist_write",
        label: "Push the checklist",
        note: "A ticked sub-task ticks the matching check item, matched by text.",
        needs: |caps| caps.checklist_write,
        absent: "This provider has no checklist.",
    },
];

/// A sentence the page owes the reader when a capability is **off**, drawn where the control it
/// qualifies is.
///
/// `R8` asks for exactly one of these in prose — *say so next to the switch; do not pretend the two
/// providers are equally safe* — and once there is one there is a table rather than an `if`.
pub struct Caveat {
    /// Which control the sentence sits under.
    pub at: &'static str,
    /// True when the sentence needs saying. Named for the *gap*, not the flag, so reading the row
    /// says what the reader will see.
    pub when: fn(&ProviderCaps) -> bool,
    pub note: &'static str,
}

/// The gaps worth stating. Both are `R7`/`R8`'s, and both are about a cost the user pays rather
/// than a thing that is broken.
pub const CAVEATS: &[Caveat] = &[
    Caveat {
        at: "authority",
        when: |caps| !caps.conditional_write,
        note: "This provider has no conditional write, so a push re-reads the item immediately \
               before writing and compares. That narrows the window to one round trip; it does \
               not close it. A write inside it wins silently.",
    },
    Caveat {
        at: "filter",
        when: |caps| !caps.server_query,
        note: "This provider applies no filter of its own, so every pass fetches the whole \
               container and filters here. Fine for a board; expensive for a tracker with a \
               hundred thousand items.",
    },
    Caveat {
        at: "interval",
        when: |caps| !caps.server_query,
        note: "Each pass is a full fetch, so a short interval spends the rate limit quickly.",
    },
];

/// Which of the section's dropdowns is open. One menu in the window is open at a time, so this is
/// *which* — the discriminant `MenuId::TaskSrc` carries, on `MenuId::AgentBench`'s precedent.
///
/// `Field`, `Lane`, `Kind` and `Priority` carry a **key**, not a provider: a control is identified
/// by the thing it edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskSrcMenu {
    Provider,
    Connection,
    Container,
    /// One declared [`ubiq_proto::tasksrc::ConfigField`], by its `key`.
    Field(String),
    /// One lane of the bound container, by its remote id.
    Lane(String),
    /// One entry of the kind map, by the provider's own word.
    Kind(String),
    /// One entry of the priority map, by the provider's own word.
    Priority(String),
    Direction,
    Authority,
    /// The import dialog's own filter-shaped picker.
    Import,
}

/// What a Test came back with. `R7`'s whole answer: **a count the filter actually returned**, and
/// enough of what it returned to recognise it.
#[derive(Clone, Debug, PartialEq)]
pub struct TestResult {
    /// Which ask this answered, so an answer to a filter the user has typed past is discarded.
    pub query_id: TaskSrcQueryId,
    pub count: usize,
    /// The first handful of titles. A bare number is a claim; a number with three titles under it
    /// is evidence.
    pub sample: Vec<String>,
    pub at: DateTime<Utc>,
}

/// The import dialog, while it is up.
///
/// **Import is explicit** (`R12`): the filter governs what is *offered* here, never what is
/// created. Nothing appears on the board that somebody did not tick.
#[derive(Clone, Debug, Default)]
pub struct ImportDialog {
    /// The ask in flight, so a listing for a filter since edited is discarded.
    pub query_id: Option<TaskSrcQueryId>,
    pub loading: bool,
    pub items: Vec<RemoteItem>,
    /// Items that already have a link row. Drawn, and not tickable: a second copy of a task that
    /// exists is the failure this list prevents.
    pub linked: Vec<RemoteItemId>,
    pub picked: Vec<RemoteItemId>,
    pub error: Option<String>,
}

impl ImportDialog {
    pub fn is_linked(&self, id: &RemoteItemId) -> bool {
        self.linked.contains(id)
    }

    pub fn is_picked(&self, id: &RemoteItemId) -> bool {
        self.picked.contains(id)
    }
}

/// Everything one window holds about task sync.
///
/// One per window rather than one per project: a window points at one project at a time, and the
/// binding follows it. `project` is what the rest of the fields are about, and a reply naming
/// another project is discarded rather than drawn onto this one.
#[derive(Default)]
pub struct TaskSrcState {
    /// What this build's host can sync against, with each one's caps and declared schema. Asked
    /// for once, when a surface that needs it opens.
    pub providers: Vec<ProviderInfo>,
    /// Which project everything below is about.
    pub project: Option<ProjectId>,
    /// The binding as the settings section has it, which is what `SetTaskSource` sends. `None` is
    /// an unbound project — and an unbound project is the ordinary case, not an error.
    pub draft: Option<Binding>,
    /// The binding as the host last confirmed it. What `draft` is compared against to know whether
    /// there is anything to save.
    pub saved: Option<Binding>,
    pub state: SyncState,
    pub last_sync: Option<DateTime<Utc>>,
    /// The tasks whose two sides differ. Filled by `M8`; drawn here from the first commit so the
    /// status item does not grow a second shape later.
    pub drifted: Vec<TaskId>,
    /// The last failure, as a sentence.
    pub error: Option<String>,
    /// The boards this connection can reach, for the picker that binds one.
    pub containers: Vec<RemoteContainer>,
    /// The bound container's lanes — the candidate strings the lane map is built from.
    pub lanes: Vec<RemoteLane>,
    /// Everything the declared `Choice`/`MultiChoice` fields draw from.
    pub facets: Facets,
    pub test: Option<TestResult>,
    /// A Test in flight, by the id that will answer it.
    pub testing: Option<TaskSrcQueryId>,
    /// Per-task link rows, for the card badge. Keyed by task, because that is what a card has.
    pub links: HashMap<TaskId, TaskLink>,
    pub import: Option<ImportDialog>,
    /// Which dropdown is open.
    pub menu: Option<TaskSrcMenu>,
    /// One text field per declared `Text` or `Secret` field, by the field's key. Kept in step with
    /// the bound provider's schema by `AppState::ensure_tasksrc_inputs`, on the knowledge base's
    /// filter-field precedent — a row for a field with no buffer has nothing to type into.
    pub inputs: HashMap<String, Entity<InputState>>,
    /// The listings in flight, so a stale answer is dropped rather than drawn.
    pub asking: BTreeMap<TaskSrcQueryId, Ask>,
}

/// What one query id was asked for. Only enough to route the answer — the answer itself carries
/// everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Providers,
    Containers,
    Lanes,
    Test,
    Items,
}

impl TaskSrcState {
    /// The provider the draft binding names, as it described itself. `None` is either no binding
    /// or a binding naming a provider this build does not have — which parks the section with a
    /// sentence rather than failing the project.
    pub fn provider(&self) -> Option<&ProviderInfo> {
        let draft = self.draft.as_ref()?;
        self.providers.iter().find(|info| info.id == draft.provider)
    }

    /// What the bound provider can do, or the closed set when nothing is bound. The default is the
    /// provider that can do nothing, which is what greys the write side out rather than offering a
    /// write nothing will make.
    pub fn caps(&self) -> ProviderCaps {
        self.provider()
            .map_or_else(ProviderCaps::default, |info| info.caps)
    }

    /// Whether the draft's connection is one this build actually holds.
    ///
    /// A draft can carry an id nothing matches for two reasons and both are ordinary: a binding
    /// written on another machine, and a provider picked before a connection was chosen. Either
    /// way the surface says so rather than letting a save produce a binding that resolves to no
    /// token on the next pass.
    pub fn connection_held(&self, held: &[Connection]) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| held.iter().any(|one| one.id == draft.connection))
    }

    /// Whether the draft differs from what the host confirmed.
    pub fn dirty(&self) -> bool {
        self.draft != self.saved
    }

    /// Whether every `required` field of the declared schema has a value. The setup surface refuses
    /// to save a binding that is incomplete — the schema says which fields those are, so nothing
    /// here knows one field from another.
    pub fn complete(&self) -> bool {
        let (Some(draft), Some(info)) = (self.draft.as_ref(), self.provider()) else {
            return false;
        };
        if draft.container.as_str().is_empty() {
            return false;
        }
        info.config_schema.iter().all(|field| {
            !field.required
                || draft
                    .filter
                    .fields
                    .get(field.key)
                    .is_some_and(|values| values.iter().any(|value| !value.trim().is_empty()))
        })
    }

    /// The link row for one task, for the card's badge.
    pub fn link(&self, task: TaskId) -> Option<&TaskLink> {
        self.links.get(&task)
    }

    /// Every candidate string any facet offered, deduplicated and in facet order.
    ///
    /// What the kind and priority maps are built from. Provider-agnostic by construction: a facet
    /// is *the provider's own list of its own words* (`R5`), so a tracker with a type field offers
    /// its types and one without offers its labels, and this asks neither which it is.
    pub fn candidates(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        for values in self.facets.values.values() {
            for value in values {
                if !seen.contains(&value.label.as_str()) {
                    seen.push(&value.label);
                }
            }
        }
        seen
    }

    /// Forget everything about the project this window has left, keeping the provider list, which
    /// is a fact about the build rather than about a project.
    pub fn leave_project(&mut self) {
        let providers = std::mem::take(&mut self.providers);
        let inputs = std::mem::take(&mut self.inputs);
        *self = Self {
            providers,
            inputs,
            ..Self::default()
        };
    }
}

/// The held connections that could authenticate one provider, in the order they are held
/// (`D189`).
///
/// **The one place the binding surface looks at a connection's provider**, and it is not a
/// per-provider branch: the family is a field the provider *declared* on its [`ProviderInfo`],
/// exactly as its schema and its capabilities are, so this code names no tracker.
///
/// An empty answer has two meanings, and the section draws a different sentence for each:
/// [`ProviderInfo::connector`] present with nothing matching is *make one in Settings ›
/// Connections*; absent is *this build cannot authenticate this provider at all*, which is what
/// a provider whose connector family the base has no variant for honestly reports.
pub fn connections_for<'a>(info: &ProviderInfo, held: &'a [Connection]) -> Vec<&'a Connection> {
    let Some(family) = info.connector else {
        return Vec::new();
    };
    held.iter().filter(|one| one.provider == family).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::connectors::{AuthKind, ProviderId};
    use ubiq_proto::ids::ConnectionId;
    use ubiq_proto::tasksrc::{ConfigField, ConfigFieldKind, RemoteContainerId};

    fn connection(provider: ProviderId, label: &str) -> Connection {
        Connection {
            id: ConnectionId::generate(),
            provider,
            label: label.into(),
            instance: None,
            auth: AuthKind::Token,
            scopes: Vec::new(),
            account: label.into(),
            client_id: None,
            oauth_app: None,
        }
    }

    #[test]
    fn a_provider_is_offered_its_own_connector_familys_connections_and_no_others() {
        let mut trello = info(ProviderCaps::default(), Vec::new());
        trello.connector = Some(ProviderId::Trello);
        let held = vec![
            connection(ProviderId::Github, "work"),
            connection(ProviderId::Trello, "boards"),
            connection(ProviderId::AzureDevops, "acme"),
        ];
        let offered = connections_for(&trello, &held);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].label, "boards");
    }

    #[test]
    fn a_provider_with_no_connector_family_is_offered_nothing_rather_than_everything() {
        // ClickUp's case, and the reason the field is an `Option` rather than a default: offering
        // the whole list would let a person pick a connection whose token cannot possibly work,
        // and the surface would have lied about it. See `T-236`.
        let orphan = info(ProviderCaps::default(), Vec::new());
        assert!(orphan.connector.is_none());
        let held = vec![
            connection(ProviderId::Github, "work"),
            connection(ProviderId::Trello, "boards"),
        ];
        assert!(connections_for(&orphan, &held).is_empty());
    }

    #[test]
    fn a_draft_naming_a_connection_this_build_does_not_hold_is_not_held() {
        let held = vec![connection(ProviderId::Trello, "boards")];
        let mut state = TaskSrcState::default();
        assert!(!state.connection_held(&held));
        state.draft = Some(Binding::new(
            "test",
            ConnectionId::generate(),
            RemoteContainerId::new("board"),
        ));
        assert!(!state.connection_held(&held));
        state.draft = Some(Binding::new(
            "test",
            held[0].id,
            RemoteContainerId::new("board"),
        ));
        assert!(state.connection_held(&held));
    }

    fn info(caps: ProviderCaps, schema: Vec<ConfigField>) -> ProviderInfo {
        ProviderInfo {
            id: "test",
            label: "Test",
            caps,
            config_schema: schema,
            connector: None,
        }
    }

    #[test]
    fn an_unbound_project_greys_every_write_out() {
        // The closed default is what makes "no binding" safe: the page draws every row and offers
        // none of them, rather than offering a write nothing will make.
        let state = TaskSrcState::default();
        let caps = state.caps();
        assert!(ABILITIES.iter().all(|ability| !(ability.needs)(&caps)));
    }

    #[test]
    fn a_required_field_nobody_filled_blocks_the_save() {
        let mut state = TaskSrcState {
            providers: vec![info(
                ProviderCaps::default(),
                vec![ConfigField {
                    key: "wiql",
                    label: "WIQL condition",
                    kind: ConfigFieldKind::Text,
                    required: true,
                    testable: true,
                }],
            )],
            ..TaskSrcState::default()
        };
        let mut binding = Binding::new(
            "test",
            ConnectionId::generate(),
            RemoteContainerId::new("board"),
        );
        state.draft = Some(binding.clone());
        assert!(!state.complete());

        binding
            .filter
            .set("wiql", vec!["[State] = 'Active'".into()]);
        state.draft = Some(binding);
        assert!(state.complete());
    }

    #[test]
    fn the_caveats_fire_on_the_gap_rather_than_the_flag() {
        // A provider that filters server-side owes no sentence about full fetches; one that does
        // not, owes two. Neither is a branch in the drawing code.
        let closed = ProviderCaps::default();
        assert_eq!(
            CAVEATS.iter().filter(|c| (c.when)(&closed)).count(),
            CAVEATS.len()
        );
        let rich = ProviderCaps {
            conditional_write: true,
            server_query: true,
            ..ProviderCaps::default()
        };
        assert_eq!(CAVEATS.iter().filter(|c| (c.when)(&rich)).count(), 0);
    }
}
