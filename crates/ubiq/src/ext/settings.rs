//! The settings container: one spec, two instances — the application overlay and the project
//! dialog (`D180`).
//!
//! `X16`'s recipe, applied: the two closed enums that used to be matched for a label, an icon, a
//! body, a count, an on-arrival refresh and an enabled test are gone, and each of those matches is
//! a field on [`SettingsSectionSpec`]. What is left of them is a `Copy` newtype over a
//! [`SlotId`] — [`crate::state::settings::SettingsSection`] and
//! [`crate::state::sink::ProjectNav`] — which is what keeps them usable in `AppState` and in an
//! element id without churn.
//!
//! **The container owns layout and scroll; a section owns content** (`D180`). Both bodies already
//! scroll, so `render` returns a plain column. A section's own inner list that can grow long is
//! `flex_none` with a `max_h` and a scroller of its own: a box inside a scroller never resolves a
//! height to measure, so a second unbounded list hugs its content and grows the page instead.
//!
//! The order is resolved **once** and cached for the process, never walked in a draw path.

use std::collections::HashMap;
use std::sync::OnceLock;

use gpui::{AnyElement, App, Context, Window};
use gpui_component::Icon;
use ubiq_proto::ids::ProjectId;

use super::{Registry, SlotId, Slotted, ids};
use crate::app::AppState;
use crate::ui::sink::project::Form;

/// Which of the two containers a section belongs to. A field rather than a second type: the two
/// navs are the same shape, drawn twice, and one spec is what stops them drifting apart again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsContainer {
    /// The application settings overlay.
    App,
    /// The project settings dialog — the live one and the sink's fixture.
    Project,
}

impl SettingsContainer {
    /// The groups this container declares, in drawn order (`D177`: ordering is by declared group,
    /// never by integer).
    pub fn groups(self) -> &'static [SlotId] {
        match self {
            SettingsContainer::App => &[
                ids::SETTINGS_APP_INTERFACE,
                ids::SETTINGS_APP_AGENTS,
                ids::SETTINGS_APP_CONNECTIVITY,
                ids::SETTINGS_APP_SYSTEM,
            ],
            SettingsContainer::Project => {
                &[ids::SETTINGS_PROJECT_CORE, ids::SETTINGS_PROJECT_CONTENT]
            }
        }
    }

    /// The container's own id.
    pub fn id(self) -> SlotId {
        match self {
            SettingsContainer::App => ids::SETTINGS_APP,
            SettingsContainer::Project => ids::SETTINGS_PROJECT,
        }
    }
}

/// When a section is offered at all.
///
/// The project dialog is create-or-edit rather than a page: a folder not yet in the catalogue has
/// no record to attach tools, a task board, a drone origin, agent definitions or a knowledge base
/// to. The sink's fixture answers to everything, because it is a fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionGate {
    /// Offered whenever its container is open. Every application section, and the project
    /// dialog's General.
    Always,
    /// Offered once the project has a record behind it.
    WithRecord,
    /// Fixture copy only: drawn in the sink's page, never offered by the live dialog.
    SinkOnly,
}

impl SectionGate {
    /// Whether the nav row is clickable, and whether [`AppState::set_sink_project_nav`] will move
    /// to it. One answer, so the drawn state and the accepted state cannot disagree.
    pub fn enabled(self, form: Form, has_record: bool) -> bool {
        match (self, form) {
            (_, Form::Sink) => true,
            (SectionGate::Always, _) => true,
            (SectionGate::WithRecord, _) => has_record,
            (SectionGate::SinkOnly, _) => false,
        }
    }
}

/// What a section is drawn against.
///
/// One struct rather than two, because one struct is what stops the two containers becoming two
/// traits again. The application overlay fills in `Form::Live` and no project: it has no fixture
/// twin, so neither field says anything there.
pub struct SectionCtx<'a> {
    pub app: &'a AppState,
    /// Which copy of the project dialog is being drawn.
    pub form: Form,
    /// The project the live dialog is editing, when there is one.
    pub project: Option<ProjectId>,
}

/// One settings section, in either container.
///
/// Every field is one match that used to be written over a closed enum (`X16` step 3): a value
/// where the arms were constants, an `fn` pointer where they were expressions.
pub struct SettingsSectionSpec {
    pub id: SlotId,
    pub container: SettingsContainer,
    /// One of [`SettingsContainer::groups`]. Asserted at resolve time — an item outside its
    /// container's declared list is a misconfiguration, not data to degrade gracefully on.
    pub group: SlotId,
    pub label: &'static str,
    /// An `fn` rather than a value: `Icon` owns a `StyleRefinement` and an `Svg`, neither of which
    /// belongs in the process-wide resolved list.
    pub icon: fn() -> Icon,
    pub gate: SectionGate,
    /// The count the nav prints beside the row. `None` prints none.
    pub count: Option<fn(&SectionCtx<'_>, &App) -> Option<usize>>,
    /// Run when the nav arrives at this section — the asks that are only worth making because the
    /// page is now on screen.
    pub on_show: Option<fn(&mut AppState, &mut Context<AppState>)>,
    /// The body. A plain column: the container owns the scroll.
    pub render: fn(&SectionCtx<'_>, &Window, &mut Context<AppState>) -> AnyElement,
}

impl Slotted for SettingsSectionSpec {
    fn id(&self) -> SlotId {
        self.id
    }
}

/// Registers a section under the group it declares.
///
/// The one way in, base or second edition: it is what keeps the registry's ordering group and the
/// spec's own `group` field the same fact rather than two that can disagree.
///
/// # Panics
/// If the spec's group is not one its container declares — the container's own list is what every
/// item is ordered against, and an item outside it is a misconfiguration, not data to degrade
/// gracefully on.
pub fn register(reg: &mut Registry<SettingsSectionSpec>, spec: SettingsSectionSpec) {
    assert!(
        spec.container.groups().contains(&spec.group),
        "ext::settings: section `{}` is in group `{}`, which its container `{}` does not declare",
        spec.id,
        spec.group,
        spec.container.id()
    );
    reg.insert(spec.group, spec);
}

/// The base's own 23 sections — 15 in the overlay, 8 in the project dialog.
///
/// A second edition receives this already seeded on `Contributions`, which is what lets it
/// relabel, reorder and remove the base's own rather than only append to them (`D177`).
pub fn base_registry() -> Registry<SettingsSectionSpec> {
    let mut reg = Registry::new();
    crate::ui::settings::sections(&mut reg);
    crate::ui::sink::project::sections(&mut reg);
    // The kitchen sink's own demo section (`X11`, invariant 9, M4) — a genuine contribution, not a
    // conversion of a former enum variant, registered exactly the way a second edition registers
    // its own. See `crate::ui::sink::ext_demo`.
    crate::ui::sink::ext_demo::settings_section(&mut reg);
    // Task sync (`D187`) — the second genuine contribution to this container, registered exactly
    // the way the demo above is and with nothing in this module or in `ui::sink::project` touched
    // to add it. That is what makes the container's claim testable rather than asserted.
    crate::ui::tasksrc::settings_section(&mut reg);
    reg
}

/// The resolved draw order, computed once for the process.
struct Resolved {
    app: Vec<&'static SettingsSectionSpec>,
    project: Vec<&'static SettingsSectionSpec>,
    by_id: HashMap<SlotId, &'static SettingsSectionSpec>,
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
pub fn install(reg: Registry<SettingsSectionSpec>) {
    if RESOLVED.set(resolve(reg)).is_err() {
        panic!("ext::settings: the settings container was already resolved before install");
    }
}

fn resolved() -> &'static Resolved {
    RESOLVED.get_or_init(|| resolve(base_registry()))
}

fn resolve(reg: Registry<SettingsSectionSpec>) -> Resolved {
    // Leaked deliberately and exactly once: the draw order is process-wide and immortal, and a
    // `&'static` spec is what lets the cached order be handed out without a clone per frame.
    let reg: &'static mut Registry<SettingsSectionSpec> = Box::leak(Box::new(reg));

    // One resolve over both containers' groups, App's first, then partitioned: `Registry::resolve`
    // takes the whole declared order, and two calls would each need their own registry.
    let mut groups: Vec<SlotId> = SettingsContainer::App.groups().to_vec();
    groups.extend_from_slice(SettingsContainer::Project.groups());
    let ordered = Registry::resolve(reg, &groups);

    let mut app = Vec::new();
    let mut project = Vec::new();
    let mut by_id = HashMap::new();
    for spec in ordered {
        by_id.insert(spec.id, spec);
        match spec.container {
            SettingsContainer::App => app.push(spec),
            SettingsContainer::Project => project.push(spec),
        }
    }
    Resolved {
        app,
        project,
        by_id,
    }
}

/// The application overlay's nav, in drawn order.
pub fn app_sections() -> &'static [&'static SettingsSectionSpec] {
    &resolved().app
}

/// The project dialog's nav, in drawn order.
pub fn project_sections() -> &'static [&'static SettingsSectionSpec] {
    &resolved().project
}

/// One section by id, in either container.
pub fn spec(id: SlotId) -> Option<&'static SettingsSectionSpec> {
    resolved().by_id.get(&id).copied()
}

/// The label a section prints, or the id itself if nothing is registered under it — a contribution
/// removed by a second edition can still be the nav the window last stood on.
pub fn label(id: SlotId) -> &'static str {
    spec(id).map(|s| s.label).unwrap_or(id.0)
}
