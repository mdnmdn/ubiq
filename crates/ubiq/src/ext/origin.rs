//! The origin-carrier container: a way to reach a drone-served project's machine besides SSH.
//!
//! A project that runs on a drone carries a [`DroneOrigin`]. The base reaches the machine through
//! a saved SSH profile; a second edition with a carrier of its own registers one spec here, and an
//! origin then names it by [`OriginVia`] — the spec's `kind` and one of its targets — in place of
//! a profile. The base registers none, and an origin naming a kind nothing registered is a row
//! that says so rather than a guess.
//!
//! What a spec answers is exactly what the base cannot: which targets exist (the project dialog's
//! Remote panel lists them), whether an attached drone was reached through one (the remote folder
//! browser's commit pins its project to it), and how to launch or adopt the drone a project names
//! when it is opened. **The project record stays local either way** (`D132`): the carrier only
//! moves the serving.
//!
//! The order is resolved **once** and cached for the process, never walked in a draw path.

use std::sync::OnceLock;

use gpui::{AnyElement, App, Context, Window};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::projects::{DroneOrigin, OriginVia};

use super::{Registry, SlotId, Slotted, ids};
use crate::app::AppState;

/// The one group a carrier is registered in.
pub const GROUPS: &[SlotId] = &[ids::ORIGIN_CARRIER];

/// One target a carrier can reach, as the Remote panel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginTarget {
    /// The carrier's own id for it — what [`OriginVia::target`] holds.
    pub target: String,
    /// What the panel calls it.
    pub label: String,
}

/// One contributed carrier.
pub struct OriginCarrierSpec {
    pub id: SlotId,
    /// The key an origin names this carrier by — [`OriginVia::kind`]. Written into the catalogue,
    /// so it never changes once shipped.
    pub kind: &'static str,
    /// Every target this carrier can reach now, for the Remote panel.
    pub targets: fn(&AppState, &App) -> Vec<OriginTarget>,
    /// Which of this carrier's targets a saved remote host is the drone of, if any — the remote
    /// folder browser asks it of the host it browsed, so the folder picked there becomes a local
    /// project pinned to that target.
    pub claims: fn(&AppState, &str) -> Option<String>,
    /// Launch the drone this project's origin names. Called on an open only when no live
    /// connection is one [`Self::claims`] names as the origin's target: the base adopts that one
    /// itself and asks it to serve the folder (`ServeProject`), as it does for every pinned
    /// project when such a connection attaches. A failure is the carrier's to put on the row,
    /// through `WindowRegistry::mark_unreadable`.
    pub launch: fn(&mut AppState, ProjectId, &DroneOrigin, &mut Context<AppState>),
}

impl Slotted for OriginCarrierSpec {
    fn id(&self) -> SlotId {
        self.id
    }
}

/// Registers a carrier. The one way in.
pub fn register(reg: &mut Registry<OriginCarrierSpec>, spec: OriginCarrierSpec) {
    reg.insert(ids::ORIGIN_CARRIER, spec);
}

/// The base's own carriers: none. SSH is not one — it is the origin's own `profile`.
pub fn base_registry() -> Registry<OriginCarrierSpec> {
    Registry::new()
}

static RESOLVED: OnceLock<Vec<&'static OriginCarrierSpec>> = OnceLock::new();

/// Installs the composed registry, before the first window. The base binary never calls this:
/// nothing installed resolves [`base_registry`] instead.
///
/// # Panics
/// If the order has already been resolved.
pub fn install(reg: Registry<OriginCarrierSpec>) {
    if RESOLVED.set(resolve(reg)).is_err() {
        panic!("ext::origin: the origin-carrier container was already resolved before install");
    }
}

fn resolve(reg: Registry<OriginCarrierSpec>) -> Vec<&'static OriginCarrierSpec> {
    // Leaked deliberately and exactly once, the way every other container's is.
    let reg: &'static mut Registry<OriginCarrierSpec> = Box::leak(Box::new(reg));
    Registry::resolve(reg, GROUPS)
}

/// Every registered carrier, in order.
pub fn carriers() -> &'static [&'static OriginCarrierSpec] {
    RESOLVED.get_or_init(|| resolve(base_registry()))
}

/// The carrier an origin's `via` names, if this build has it.
pub fn carrier(kind: &str) -> Option<&'static OriginCarrierSpec> {
    carriers().iter().copied().find(|spec| spec.kind == kind)
}

/// A carrier's question before it does something to a project that loses work — replacing a
/// drone that holds panes, say. The window draws it as a danger confirm (`Layer::OriginConfirm`);
/// the answer goes back to the carrier through `on_confirm` or `on_cancel`, with the project.
#[derive(Clone)]
pub struct OriginConfirm {
    pub project: ProjectId,
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub on_confirm: fn(&mut AppState, ProjectId, &mut Context<AppState>),
    pub on_cancel: fn(&mut AppState, ProjectId, &mut Context<AppState>),
}

impl AppState {
    /// Put a carrier's question up. A second one replaces the first, whose project is cancelled.
    pub fn ask_origin_confirm(&mut self, confirm: OriginConfirm, cx: &mut Context<Self>) {
        if let Some(old) = self.workbench.origin_confirm.replace(confirm) {
            (old.on_cancel)(self, old.project, cx);
        }
        cx.notify();
    }

    /// The carrier's question answered yes.
    pub fn confirm_origin(&mut self, cx: &mut Context<Self>) {
        if let Some(confirm) = self.workbench.origin_confirm.take() {
            (confirm.on_confirm)(self, confirm.project, cx);
        }
        cx.notify();
    }

    /// The carrier's question cancelled — Cancel, an outside click or Escape.
    pub fn dismiss_origin_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(confirm) = self.workbench.origin_confirm.take() {
            (confirm.on_cancel)(self, confirm.project, cx);
        }
        cx.notify();
    }
}

/// The carrier's question, drawn at the window root while one is up.
pub fn confirm(app: &AppState, window: &Window, cx: &mut Context<AppState>) -> Option<AnyElement> {
    let confirm = app.workbench.origin_confirm.as_ref()?;
    let entity = cx.entity();
    Some(crate::ui::kit::confirm_modal(
        "origin-confirm",
        &confirm.title,
        &confirm.message,
        &confirm.confirm_label,
        true,
        crate::ui::handler(&entity, |this, _, cx| this.confirm_origin(cx)),
        crate::ui::handler(&entity, |this, _, cx| this.dismiss_origin_confirm(cx)),
        window,
    ))
}

/// The contributed carrier and target a saved remote host is the drone of, if any claims it.
pub fn claim(app: &AppState, save_id: &str) -> Option<OriginVia> {
    carriers().iter().find_map(|spec| {
        (spec.claims)(app, save_id).map(|target| OriginVia {
            kind: spec.kind.to_string(),
            target,
        })
    })
}
