//! The runner-kind container: how the run picker draws a tool the host *discovered*.
//!
//! A discovered tool carries the id of the runner source that found it
//! (`ubiq_proto::tools::ToolOrigin::Discovered::runner`), and that id is all the interface knows
//! about it. One spec per kind says what to call it and which icon marks its rows. The base seeds
//! `make`, `just` and `mise`; a second edition whose host contributes a further runner source
//! registers the matching kind here, and a runner nothing registered a kind for is drawn with a
//! generic icon rather than dropped.
//!
//! A tool the user wrote is not a runner kind: the picker marks it with `UbiqIcon::RunnerUbiq`
//! itself.
//!
//! The order is resolved **once** and cached for the process, never walked in a draw path.

use std::sync::OnceLock;

use gpui_component::{Icon, IconName};

use super::{Registry, SlotId, Slotted, ids};
use crate::ui::kit::UbiqIcon;

/// The one group a kind is registered in. A kind has nothing to be ordered against.
pub const GROUPS: &[SlotId] = &[ids::RUNNER_KIND];

/// One kind of runner.
pub struct RunnerKindSpec {
    pub id: SlotId,
    /// The runner source's id, as `ToolOrigin::Discovered::runner` spells it.
    pub runner: &'static str,
    /// What the picker's tooltip calls the kind.
    pub label: &'static str,
    /// An `fn` rather than a value, for the rail's reason: `Icon` is not for a process-wide list.
    pub icon: fn() -> Icon,
}

impl Slotted for RunnerKindSpec {
    fn id(&self) -> SlotId {
        self.id
    }
}

/// Registers a kind. The one way in, base or second edition.
pub fn register(reg: &mut Registry<RunnerKindSpec>, spec: RunnerKindSpec) {
    reg.insert(ids::RUNNER_KIND, spec);
}

/// The base's own three kinds. A second edition receives this already seeded on `Contributions`,
/// which is what lets it relabel, reorder and remove them (`D177`).
pub fn base_registry() -> Registry<RunnerKindSpec> {
    let mut reg = Registry::new();
    for (id, runner, label, icon) in [
        (
            ids::RUNNER_MAKE,
            "make",
            "Make target",
            (|| UbiqIcon::RunnerMake.into()) as fn() -> Icon,
        ),
        (ids::RUNNER_JUST, "just", "Just recipe", || {
            UbiqIcon::RunnerJust.into()
        }),
        (ids::RUNNER_MISE, "mise", "Mise task", || {
            UbiqIcon::RunnerMise.into()
        }),
    ] {
        register(
            &mut reg,
            RunnerKindSpec {
                id,
                runner,
                label,
                icon,
            },
        );
    }
    reg
}

static RESOLVED: OnceLock<Vec<&'static RunnerKindSpec>> = OnceLock::new();

/// Installs the composed registry, before the first window. The base binary never calls this:
/// nothing installed resolves [`base_registry`] instead.
///
/// # Panics
/// If the order has already been resolved.
pub fn install(reg: Registry<RunnerKindSpec>) {
    if RESOLVED.set(resolve(reg)).is_err() {
        panic!("ext::runner: the runner-kind container was already resolved before install");
    }
}

fn resolve(reg: Registry<RunnerKindSpec>) -> Vec<&'static RunnerKindSpec> {
    // Leaked deliberately and exactly once, the way the rail's and the menus' are.
    let reg: &'static mut Registry<RunnerKindSpec> = Box::leak(Box::new(reg));
    Registry::resolve(reg, GROUPS)
}

fn resolved() -> &'static [&'static RunnerKindSpec] {
    RESOLVED.get_or_init(|| resolve(base_registry()))
}

/// The kind registered for a runner source's id, if any.
pub fn kind_of(runner: &str) -> Option<&'static RunnerKindSpec> {
    resolved()
        .iter()
        .copied()
        .find(|kind| kind.runner == runner)
}

/// The icon that marks a discovered tool's runner: its kind's, or the generic one for a runner no
/// kind was registered for.
pub fn icon_of(runner: &str) -> Icon {
    match kind_of(runner) {
        Some(kind) => (kind.icon)(),
        None => IconName::SquareTerminal.into(),
    }
}
