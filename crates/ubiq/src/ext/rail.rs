//! The rail container: one spec per destination the activity rail offers (`D184`).
//!
//! `X16`'s recipe, applied a second time. The closed enum that used to be matched for an icon, a
//! `UiId`, a destination, a centre screen, a default layout, a default pair of regions and two
//! tables of furniture is gone, and each of those matches is a field on [`RailModeSpec`]. What is
//! left of it is a `Copy` newtype over a [`SlotId`] — [`crate::state::RailMode`] — which is what
//! keeps it usable in `AppState`, in a `HashMap` key and in an element id without churn.
//!
//! **The two things this container has that the settings one does not** are
//! [`Availability`] — a mode may be always offered, offered once a project opts into it, or
//! offered only while a predicate says so — and a **serialized id**. A rail mode is a key in
//! `ViewPrefs::modes`, so the id on disk is this module's, the decoder maps the ten old variant
//! names onto it, and an id no registration answers for is **kept, not dropped**: dropping it
//! would silently lose a person's saved arrangement for a mode a second edition contributes
//! (`D184`).
//!
//! The order is resolved **once** and cached for the process, never walked in a draw path.

use std::collections::HashMap;
use std::sync::OnceLock;

use gpui::{AnyElement, App, Context, Entity, Window};
use gpui_component::Icon;
use gpui_component::dock::DockArea;
use ubiq_proto::ids::ProjectId;

use super::{Registry, SlotId, Slotted, ids};
use crate::app::AppState;
use crate::state::RailMode;
use crate::state::dock::{PanelKind, Region};
use crate::state::nav::View;
use crate::state::ui_id::UiId;
use crate::ui::dock::WorkbenchPanel;

/// The rail's groups, in the order they are drawn, each with the heading the rail prints over it.
///
/// `D177`: ordering is by declared group and never by integer, and a heading is the group's own
/// business rather than a string repeated at each item.
pub const GROUPS: &[(SlotId, &str)] = &[(ids::RAIL_APP, "APP"), (ids::RAIL_PROJECT, "PROJECT")];

/// The group ids alone, which is what [`Registry::resolve`] is ordered against.
fn group_order() -> Vec<SlotId> {
    GROUPS.iter().map(|(id, _)| *id).collect()
}

/// The heading drawn over a group, or the id itself for a group nothing named.
pub fn group_label(group: SlotId) -> &'static str {
    GROUPS
        .iter()
        .find(|(id, _)| *id == group)
        .map(|(_, label)| *label)
        .unwrap_or(group.0)
}

/// When a mode is offered at all.
///
/// The rail's conditional half, and the one thing the settings container has no equivalent of: a
/// contributed mode is frequently about a feature that is only switched on for some projects, and
/// the choice between "the user may hide it" and "it is not there unless something says so" is
/// the contributor's rather than the container's.
#[derive(Clone, Copy)]
pub enum Availability {
    /// Offered wherever its group is drawn; the project may hide it from the rail. Every one of
    /// the base's own ten.
    Always,
    /// Not offered until the project opts into it — `ViewPrefs::opted_in_modes`. An allow-list
    /// rather than [`Always`](Availability::Always)'s deny-list, which is the whole difference:
    /// a project that has never heard of the mode does not draw it.
    OptIn,
    /// Offered while the predicate says so, and not listed at all otherwise. The predicate is
    /// handed the widest context the container has — the whole of `AppState` and the app — rather
    /// than pre-extracted arguments, so a contribution is never limited by a signature the base
    /// chose (the rule `D180`'s dry-run produced).
    ///
    /// **It is answered from interface state.** A predicate that needs something only the host
    /// knows has to have had it sent first, through the message set like everything else.
    When(fn(&AppState, &App) -> bool),
}

/// One rail mode.
///
/// Every field is one match that used to be written over a closed enum (`X16` step 3): a value
/// where the arms were constants, an `fn` pointer where they were expressions.
pub struct RailModeSpec {
    pub id: SlotId,
    /// One of [`GROUPS`]. Asserted by [`register`], which is the only way in.
    pub group: SlotId,
    pub label: &'static str,
    /// The one-line note the empty page prints, and the rail's tooltip for Control.
    pub note: &'static str,
    /// The mode's own short name — what `rail.<slug>` binds a help page to. Written down rather
    /// than derived, because the derivation used to be the variant's `Debug` name and a help page
    /// bound to the old spelling must not come unbound by this conversion.
    pub slug: &'static str,
    /// An `fn` rather than a value: `Icon` owns a `StyleRefinement` and an `Svg`, neither of which
    /// belongs in a process-wide resolved list.
    pub icon: fn() -> Icon,
    /// The stable name of this mode's rail button, for in-place help and anything else that keys
    /// data to a place on the screen.
    pub ui_id: UiId,
    pub availability: Availability,
    /// Whether a pane started here has an edge region to land in that the user is looking at. A
    /// mode that is about the application rather than about a project's folder answers `false`,
    /// and a runner started from one moves the window to the IDE first.
    pub has_pane_region: bool,
    /// Whether the centre draws the "no project" page when the window has no folder open.
    pub needs_project: bool,
    /// Which side regions a **never-arranged** visit to this mode opens. An entry in
    /// `ViewPrefs::modes` carries its own three flags and this is not consulted for it.
    pub opens_left: bool,
    pub opens_right: bool,
    /// The centre region's screen. `None` is a stated gap — the empty page says the mode is not
    /// built — rather than an error state.
    pub centre: Option<Centre>,
    /// The arrangement a window opens this mode in when nothing is saved for it. `None` takes the
    /// IDE's, which is what every mode without an opinion has always taken.
    pub default_layout: Option<DefaultLayout>,
    /// The `ubiq://` destination this mode is drawing, for a bookmark, a recent or a link.
    /// `None` is a mode that is never a place — the sink is the base's one.
    pub destination: Option<Destination>,
    /// The panels a first visit to this mode puts in its home regions, queued as pending edits.
    /// A visit with a saved arrangement never asks: the blob is the answer.
    pub furniture: Option<fn(&mut AppState)>,
    /// The panel this mode calls home in one of its side regions, read only when the user opens
    /// an edge onto nothing (`D156`).
    pub side_furniture: Option<fn(Region) -> Option<PanelKind>>,
    /// Run when the window arrives in this mode — the asks that are only worth making because the
    /// screen is now up. `D180`'s `on_show`, one container along.
    pub on_enter: Option<fn(&mut AppState, &mut Context<AppState>)>,
}

/// The screen a mode draws in the centre region.
pub type Centre = fn(&AppState, &mut Window, &mut Context<AppState>) -> AnyElement;

/// The `ubiq://` destination a mode is pointed at.
///
/// The `&mut ProjectId` is not decoration: a Teams link names the project of what it points at,
/// which under the window span is not the project on screen, so resolving the destination is also
/// what settles which project the link belongs to.
pub type Destination = fn(&AppState, &mut ProjectId, &App) -> Option<View>;

/// The signature `dock::default_layout` hands a mode.
///
/// `&mut dyn FnMut` rather than a generic, because a spec field is an `fn` pointer and an `fn`
/// pointer cannot be generic.
pub type DefaultLayout = fn(
    &Entity<DockArea>,
    &mut dyn FnMut(PanelKind, &mut App) -> Option<Entity<WorkbenchPanel>>,
    &mut Window,
    &mut App,
);

impl Slotted for RailModeSpec {
    fn id(&self) -> SlotId {
        self.id
    }
}

/// Registers a mode under the group it declares.
///
/// The one way in, base or second edition: it is what keeps the registry's ordering group and the
/// spec's own `group` field the same fact rather than two that can disagree — the redundancy
/// `D180` found in its own first draft.
///
/// # Panics
/// If the spec's group is not one the rail declares.
pub fn register(reg: &mut Registry<RailModeSpec>, spec: RailModeSpec) {
    assert!(
        GROUPS.iter().any(|(id, _)| *id == spec.group),
        "ext::rail: mode `{}` is in group `{}`, which the rail does not declare",
        spec.id,
        spec.group
    );
    reg.insert(spec.group, spec);
}

/// The base's own ten modes.
///
/// A second edition receives this already seeded on `Contributions`, which is what lets it
/// relabel, reorder and remove the base's own rather than only append to them (`D177`).
pub fn base_registry() -> Registry<RailModeSpec> {
    let mut reg = Registry::new();
    crate::ui::rail::modes(&mut reg);
    // The kitchen sink's own demo mode (`X11`, invariant 9, M4) — a genuine contribution, not a
    // conversion of a former enum variant, registered exactly the way a second edition registers
    // its own. See `crate::ui::sink::ext_demo`.
    crate::ui::sink::ext_demo::rail_mode(&mut reg);
    reg
}

/// The resolved draw order, computed once for the process.
struct Resolved {
    order: Vec<&'static RailModeSpec>,
    by_id: HashMap<SlotId, &'static RailModeSpec>,
}

static RESOLVED: OnceLock<Resolved> = OnceLock::new();

/// Installs the composed registry, before the first window.
///
/// The base binary never calls this: nothing installed resolves [`base_registry`] instead, which
/// is what keeps a base behaviour from depending on something having been registered.
///
/// # Panics
/// If the order has already been resolved — either a second `install`, or an `install` after
/// something drew.
pub fn install(reg: Registry<RailModeSpec>) {
    if RESOLVED.set(resolve(reg)).is_err() {
        panic!("ext::rail: the rail container was already resolved before install");
    }
}

fn resolved() -> &'static Resolved {
    RESOLVED.get_or_init(|| resolve(base_registry()))
}

fn resolve(reg: Registry<RailModeSpec>) -> Resolved {
    // Leaked deliberately and exactly once: the draw order is process-wide and immortal, and a
    // `&'static` spec is what lets the cached order be handed out without a clone per frame.
    let reg: &'static mut Registry<RailModeSpec> = Box::leak(Box::new(reg));
    let order = Registry::resolve(reg, &group_order());
    let by_id = order.iter().map(|spec| (spec.id, *spec)).collect();
    Resolved { order, by_id }
}

/// Every mode, in the order the rail draws them.
pub fn modes() -> &'static [&'static RailModeSpec] {
    &resolved().order
}

/// One mode by id, or `None` for an id nothing is registered under — a mode a second edition
/// removed, or one a saved blob names that this build has never heard of.
pub fn spec(id: SlotId) -> Option<&'static RailModeSpec> {
    resolved().by_id.get(&id).copied()
}

/// The label a mode prints, or the id itself if nothing is registered under it.
pub fn label(id: SlotId) -> &'static str {
    spec(id).map(|s| s.label).unwrap_or(id.0)
}

/// The modes in one group, in drawn order.
pub fn modes_in(group: SlotId) -> impl Iterator<Item = RailMode> {
    modes()
        .iter()
        .filter(move |spec| spec.group == group)
        .map(|spec| RailMode(spec.id))
}

// ── The serialized form (`X10`, `D184`) ─────────────────────────────

/// The ten names `RailMode` used to serialize under, mapped onto the ids that replaced them.
///
/// A blob written at schema 5 or below carries these; a blob at schema 6 carries the ids. Keeping
/// the table rather than migrating the file once is what lets a config root be shared with an
/// older build without either side losing the other's arrangements.
const LEGACY_NAMES: &[(&str, SlotId)] = &[
    ("Control", ids::RAIL_CONTROL),
    ("Ide", ids::RAIL_IDE),
    ("Git", ids::RAIL_GIT),
    ("Agents", ids::RAIL_AGENTS),
    ("Teams", ids::RAIL_TEAMS),
    ("TeamsAll", ids::RAIL_TEAMS_ALL),
    ("TeamsOld", ids::RAIL_TEAMS_OLD),
    ("Kb", ids::RAIL_KB),
    ("Tasks", ids::RAIL_TASKS),
    ("Sink", ids::RAIL_SINK),
];

/// Read one key back.
///
/// **Nothing is ever dropped.** An id no registration answers for is interned and kept as it was
/// found, so a build without a second edition's registrations still writes that edition's saved
/// arrangements back out untouched (`D184`). The one transformation is the legacy table above.
pub fn decode_id(text: &str) -> SlotId {
    LEGACY_NAMES
        .iter()
        .find(|(name, _)| *name == text)
        .map(|(_, id)| *id)
        .unwrap_or_else(|| SlotId::intern(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legacy_table_covers_every_one_of_the_ten_original_modes() {
        // Every id the legacy table names must still resolve — that is the direction a schema-5
        // blob depends on. The reverse does not hold since M4: `modes()` also carries the kitchen
        // sink's own demo contribution (`X11`), which never had an old enum-variant name to decode
        // from and is not meant to. Asserting the reverse over `modes()` would fail the moment any
        // future contribution joins the container, which is the whole point of the container.
        for (name, id) in LEGACY_NAMES {
            assert!(
                spec(*id).is_some(),
                "legacy name `{name}` maps to `{id}`, which nothing is registered under"
            );
        }
    }

    #[test]
    fn an_unregistered_id_is_kept_rather_than_dropped() {
        let id = decode_id("studio.rail.portfolio");
        assert_eq!(id.0, "studio.rail.portfolio");
        assert!(spec(id).is_none(), "and nothing is registered under it");
        // Interned, so two reads of the same blob are the same `SlotId`.
        assert_eq!(decode_id("studio.rail.portfolio"), id);
    }

    #[test]
    fn an_old_variant_name_decodes_to_the_id_that_replaced_it() {
        assert_eq!(decode_id("TeamsOld"), ids::RAIL_TEAMS_OLD);
        assert_eq!(decode_id("Ide"), ids::RAIL_IDE);
    }
}
