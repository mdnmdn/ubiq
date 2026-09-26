//! The task-source layer: what a provider must answer, what a project's board is bound to, and
//! the registry a boot fills.
//!
//! The neutral half — [`RemoteItem`] and the rest of the vocabulary — is
//! [`ubiq_proto::tasksrc`], because both halves of Ubiq eventually name it. What lives here is the
//! half only the host has any business holding: the trait, the registry, and the file a binding
//! is written to.
//!
//! **[`Binding`], [`Filter`], [`Direction`], [`Authority`], [`LinkState`] and [`TaskLink`] were
//! defined here and are now the contract crate's** (`D187`), re-exported below so every existing
//! path still reads. The move is what M7's settings surface forced and nothing more: the surface
//! edits a binding and `SetTaskSource` carries it back, so a binding is vocabulary both halves
//! name. The file it is written to is still `store`'s alone.
//!
//! - [`TaskProvider`]: the one seam. Blocking, on a thread of its own, because every call is
//!   already off the coordinator's thread and an async runtime would be a second scheduler for
//!   nobody — the same reasoning that picked `ureq` for the connector family.
//! - [`Binding`]: one project's configuration. Provider-agnostic by construction: every map in it
//!   is keyed by a string the *provider* supplied, so nothing here learns what a Trello list is.
//! - [`Registry`]: a plain list of providers, filled at boot. Not
//!   [`crates/ubiq/src/ext`](../../../ubiq/src/ext)'s `Registry<T>`, which is the interface's
//!   extension machinery and a different thing entirely — the host has no dependency on it and
//!   gains nothing from one.
//! - `sync`: the inbound pass and the thread that runs it — the third holder of
//!   `crate::work::Handle` (`D120`), and where the two rules of the *layer* live: an unmapped
//!   lane parks its task, and an item that leaves the filter loses its link and nothing else.
//!   Behind `harness` **and** `listener`, because it needs a board to write and an identity to
//!   call as.
//! - `store`: `tasksrc.toml`, one per project.
//! - `trello`: the first provider, behind `listener` because it makes HTTP requests.
//!
//! **Nothing here holds credential material.** A [`Binding`] names a [`ConnectionId`] and the
//! token stays in the connector family's secret store; [`Identity`] is what a resolved one looks
//! like for the length of one call, and it is never written down.

use anyhow::Result;
use ubiq_proto::connectors::ProviderId;
use ubiq_proto::tasksrc::{
    Facets, ProviderCaps, ProviderInfo, RemoteCheckId, RemoteComment, RemoteContainer, RemoteDraft,
    RemoteItem, RemoteItemId, RemoteLane, RemotePatch, Revision,
};

/// The binding vocabulary, which the contract crate owns since `D187`. Re-exported rather than
/// aliased so `tasksrc::Binding` — the path the worker, the store and the tests already use —
/// stays the one name for the one type.
pub use ubiq_proto::tasksrc::{
    Authority, Binding, Direction, DriftSide, FieldDrift, Filter, LinkState, POLL_DEFAULT,
    POLL_FLOOR, SyncScope, SyncState, TaskLink,
};

/// The conflict table and the local side's hashes. Pure — no board, no thread, no network — so
/// the rule can be read and tested on its own, which is the only form a rule of this shape is
/// checkable in. Gated with [`sync`] because it hashes a [`crate::work`] task record, which is
/// what `harness` supplies; nothing in it makes a request.
#[cfg(all(feature = "harness", feature = "listener"))]
pub mod outbound;
/// The host's arms for the §3.6 wire family (`D188`) — what M7 drew a page against. Behind the
/// same two features as [`sync`], because every arm either runs a pass or resolves an identity.
#[cfg(all(feature = "harness", feature = "listener"))]
pub mod service;
pub mod store;
/// The pass needs both halves of what a lean build may leave out: [`crate::work`] to write a task
/// (`harness`) and [`crate::connectors`] to resolve the identity a provider is called as
/// (`listener`). A build with neither has a binding it can read and nothing to sync it with, which
/// is the honest shape — the model, the trait and the sidecar above stay unconditional so a lean
/// embedder can still hold one.
#[cfg(all(feature = "harness", feature = "listener"))]
pub mod sync;
#[cfg(feature = "listener")]
pub mod trello;

/// A connection, resolved into what one provider call needs.
///
/// The same five facts `repos::list::Identity` carries, and deliberately a second type rather than
/// a shared one: that one is behind the `git` feature, and a lean build that syncs a board but
/// compiles no version control must not have to take `git2` with it. Lifting both into the
/// connector family is `G363`.
///
/// Held for the length of a call and never written to a file.
#[derive(Clone, Debug)]
pub struct Identity {
    pub provider: ProviderId,
    /// `None` is the provider's own cloud. Trello has only that.
    pub instance: Option<String>,
    /// The provider's own name for the account, for a listing that scopes to it.
    pub account: String,
    /// What was filed under this connection. `None` is a connection whose secret is gone from the
    /// store, which is a call that fails rather than one made anonymously.
    pub token: Option<String>,
    /// The certificate vouched for at this instance's origin, if one was.
    pub pin: Option<String>,
}

/// What a provider must answer.
///
/// Blocking, `Send + Sync`, and every call takes the resolved [`Identity`] beside the
/// [`Binding`] — a binding names a connection and holds no material, so the token reaches a call
/// without ever reaching the file the binding is written to.
pub trait TaskProvider: Send + Sync {
    /// The provider's stable key — `"trello"`. What a [`Binding::provider`] names.
    fn id(&self) -> &'static str;

    /// What it is called on screen.
    fn label(&self) -> &'static str;

    /// What it can do. The interface greys out the rest rather than failing when it is asked.
    fn caps(&self) -> ProviderCaps;

    /// Which connector family the connections this provider is called as belong to (`D189`).
    ///
    /// The setup surface offers exactly the held connections whose `provider` matches, which is
    /// how a binding is bound to a real connection instead of a minted placeholder. The default is
    /// `None`, and `None` is a claim rather than a shrug: **no connector family in this build can
    /// authenticate this provider**, so the surface offers nothing and says why. A provider that
    /// leaves the default in place while a matching [`ProviderId`] exists is a bug, not a
    /// configuration.
    fn connector(&self) -> Option<ProviderId> {
        None
    }

    /// What it needs configuring with, for the base to render. `&'static` because a schema is a
    /// compiled-in fact about the provider, never something built per binding.
    fn config_schema(&self) -> &'static [ubiq_proto::tasksrc::ConfigField];

    /// The containers this identity can reach — the boards a picker offers. Takes no binding,
    /// because this is what runs *before* there is one.
    fn containers(&self, who: &Identity) -> Result<Vec<RemoteContainer>>;

    /// The bound container's lanes, which are the candidate strings the lane map is built from.
    fn lanes(&self, who: &Identity, binding: &Binding) -> Result<Vec<RemoteLane>>;

    /// Every candidate list the declared `Choice`/`MultiChoice` fields draw from, in one call —
    /// they all come off the same container description, and three calls would be three round
    /// trips against one rate limit.
    fn facets(&self, who: &Identity, binding: &Binding) -> Result<Facets>;

    /// Everything under the binding's filter. Where [`ProviderCaps::server_query`] is off, the
    /// provider fetches the container and applies the filter itself — the layer never sees an
    /// unfiltered list.
    fn query(&self, who: &Identity, binding: &Binding) -> Result<Vec<RemoteItem>>;

    /// Named items, whatever the filter says. This is what a pass over the *linked* set uses, so
    /// an item the user narrowed the filter past still syncs until it is unlinked deliberately.
    fn fetch(
        &self,
        who: &Identity,
        binding: &Binding,
        ids: &[RemoteItemId],
    ) -> Result<Vec<RemoteItem>>;

    fn create(&self, who: &Identity, binding: &Binding, draft: &RemoteDraft) -> Result<RemoteItem>;

    /// Apply a patch. `expect` is the revision the caller believes is current: a provider with
    /// [`ProviderCaps::conditional_write`] sends it as a precondition, and one without re-reads
    /// and compares, which narrows the window to a round trip rather than closing it (`R8`).
    fn update(
        &self,
        who: &Identity,
        binding: &Binding,
        id: &RemoteItemId,
        patch: &RemotePatch,
        expect: &Revision,
    ) -> Result<RemoteItem>;

    fn comment(
        &self,
        who: &Identity,
        binding: &Binding,
        id: &RemoteItemId,
        text: &str,
    ) -> Result<RemoteComment>;

    fn set_check(
        &self,
        who: &Identity,
        binding: &Binding,
        id: &RemoteItemId,
        item: &RemoteCheckId,
        done: bool,
    ) -> Result<()>;

    /// Everything the setup surface is told about this provider, assembled from the three
    /// questions above. Never overridden.
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.id(),
            label: self.label(),
            caps: self.caps(),
            config_schema: self.config_schema().to_vec(),
            connector: self.connector(),
        }
    }
}

/// The providers this build has, filled once at boot.
///
/// A plain list. The registry exists so a provider shipping from outside this repository is one
/// entry point rather than a fork — and it has a base-side user from its first commit, which is
/// what a seam is supposed to have.
///
/// **This is the value `ubiq_app::Contributions::task_providers` carries** (`D189`), and it obeys
/// the same two rules every other container on that struct does: it arrives **seeded** with the
/// base's own providers, so a second edition can substitute or remove one rather than only append
/// to them; and [`Registry::register`] is the only way in. It is deliberately *not* a
/// `ubiq::ext::Registry<Spec>` — a host service is resolved by id and never drawn, so it needs no
/// group, no label, no icon and no predicate, and giving it them to make the two look alike would
/// be the framework mistake the pattern's own §4 names.
#[derive(Default)]
pub struct Registry {
    providers: Vec<Box<dyn TaskProvider>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// What this build ships. Trello, and only where the HTTP it needs is compiled in.
    pub fn with_defaults() -> Self {
        #[allow(unused_mut)]
        let mut registry = Self::new();
        #[cfg(feature = "listener")]
        registry.register(Box::new(trello::Trello::new()));
        registry
    }

    /// Add one. A second registration under an id already held **replaces** it, so an embedder can
    /// substitute a provider rather than having to prevent the default being registered.
    pub fn register(&mut self, provider: Box<dyn TaskProvider>) {
        let id = provider.id();
        self.providers.retain(|held| held.id() != id);
        self.providers.push(provider);
    }

    /// Drop one by id, and say whether there was one. What lets a second edition **remove** a
    /// seeded base provider rather than only add to the list — the same operation the UI
    /// containers give a contribution, on a registry that has no spec to relabel.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.providers.len();
        self.providers.retain(|held| held.id() != id);
        before != self.providers.len()
    }

    /// The provider a binding names, or `None` for a binding written by a build that had one this
    /// one does not — which parks the binding rather than failing the project.
    pub fn get(&self, id: &str) -> Option<&dyn TaskProvider> {
        self.providers
            .iter()
            .find(|held| held.id() == id)
            .map(AsRef::as_ref)
    }

    /// What `ListTaskProviders` answers with.
    pub fn infos(&self) -> Vec<ProviderInfo> {
        self.providers.iter().map(|held| held.info()).collect()
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Named here rather than leaned on through `super::*`: `D187` moved the binding vocabulary
    // into the contract crate, and these four never travelled with it — the module re-exports the
    // binding types and not the ids and the closed sets the tests build one out of.
    use ubiq_proto::ids::ConnectionId;
    use ubiq_proto::tasksrc::{RemoteContainerId, RemoteLaneId};
    use ubiq_proto::work::Status;

    fn binding() -> Binding {
        Binding::new(
            "trello",
            ConnectionId::generate(),
            RemoteContainerId::new("board1"),
        )
    }

    #[test]
    fn a_fresh_binding_pulls_and_writes_nothing() {
        let binding = binding();
        assert_eq!(binding.direction, Direction::Pull);
        assert!(!binding.writes());
        assert!(!binding.auto_import);
        assert_eq!(binding.poll_seconds(), POLL_DEFAULT);
    }

    #[test]
    fn the_poll_floor_is_applied_on_read_and_never_written_back() {
        let mut binding = binding();
        binding.poll = 5;
        assert_eq!(binding.poll_seconds(), POLL_FLOOR);
        // What the user asked for is still what is stored: the floor is this build's rule, and a
        // later build with a lower one must not find the value rewritten.
        assert_eq!(binding.poll, 5);
    }

    #[test]
    fn an_unmapped_lane_parks_rather_than_guessing() {
        let mut binding = binding();
        binding.lanes.insert("list-a".into(), Status::InProgress);
        assert_eq!(
            binding.status_of(&RemoteLaneId::new("list-a")),
            Some(Status::InProgress)
        );
        assert_eq!(binding.status_of(&RemoteLaneId::new("list-z")), None);
        assert_eq!(
            binding.lane_of(Status::InProgress),
            Some(RemoteLaneId::new("list-a"))
        );
        assert_eq!(binding.lane_of(Status::Blocked), None);
    }

    #[test]
    fn an_unfilled_filter_field_is_no_constraint() {
        let mut filter = Filter::default();
        filter.set("lists", vec!["list-a".into(), "list-b".into()]);
        assert_eq!(filter.many("lists").len(), 2);
        assert!(filter.many("labels").is_empty());
        assert_eq!(filter.one("query"), None);
        assert!(!filter.flag("archived"));
    }

    #[test]
    fn a_registry_answers_by_id() {
        let registry = Registry::with_defaults();
        #[cfg(feature = "listener")]
        {
            assert!(registry.get("trello").is_some());
            assert_eq!(registry.len(), 1);
            // The registry is what `ListTaskProviders` reads.
            assert_eq!(registry.infos().len(), 1);
            // `D189`: what a binding's connection picker offers for this provider is the family
            // it declares here, and the base's own provider declares one.
            assert_eq!(registry.infos()[0].connector, Some(ProviderId::Trello));
        }
        assert!(registry.get("nothing-ships-this").is_none());
    }
}
