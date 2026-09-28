//! The bar-menu container: one spec per block of rows a top-bar chevron offers (`D177`, `T-267`).
//!
//! `X16`'s recipe, applied a third time. The three closed enums that used to say what the
//! titlebar's menus offer — `NewPaneRow`, `NewProjectRow`, `OverflowRow` — are gone, and with
//! them the pick-by-position `match` each had on `AppState`. What replaced them is
//! [`crate::ui::menus::MenuEntry`]: a row that carries its own wording, its own glyph, its own
//! predicate **and** its own action.
//!
//! **A block, not a row, is the registered thing.** A menu's rows are not a fixed list — the
//! shells are the machine's, the tools are the project's, the hidden agents are whatever is
//! running — so a spec contributes an `fn` that builds rows rather than one row's fields. The
//! base registers exactly five blocks, one per menu, each the flat list in `crate::ui::menus`
//! that a reader reorders by moving a line.
//!
//! **The groups are the menus.** A bar menu has no internal groups: within one, the order is
//! registration order, which is the base's block first and a contributed one after it unless the
//! contributor reorders or removes.
//!
//! A second edition adds a row by registering a block of its own; it drops or rewrites one of the
//! base's by removing that block and registering a replacement — `Registry`'s documented "replace
//! is remove plus insert under a new id". Every base builder in `crate::ui::menus` is `pub`, so a
//! replacement that wants the base's rows minus one calls it and filters what comes back.
//!
//! The order is resolved **once** and cached for the process, never walked in a draw path.

use std::sync::OnceLock;

use gpui::App;

use super::{Registry, SlotId, Slotted, ids};
use crate::app::AppState;
use crate::ui::menus::MenuEntry;

/// Every bar menu, which is this container's set of groups.
pub const GROUPS: &[SlotId] = &[
    ids::MENU_NEW_PROJECT,
    ids::MENU_RUN_TOOL,
    ids::MENU_NEW_PANE,
    ids::MENU_OVERFLOW,
    ids::MENU_HIDDEN_AGENTS,
];

/// One block of rows in one bar menu.
pub struct MenuBlockSpec {
    pub id: SlotId,
    /// Which menu the block belongs to: one of [`GROUPS`]. Asserted by [`register`], the only way
    /// in.
    pub group: SlotId,
    /// The rows this block contributes, built fresh every time the menu opens — a menu is a
    /// question about the moment it was opened in, not a table.
    ///
    /// An `fn` pointer rather than a closure, for the reason every other spec field is one: the
    /// resolved order is process-wide and immortal.
    pub rows: fn(&AppState, &App) -> Vec<MenuEntry>,
}

impl Slotted for MenuBlockSpec {
    fn id(&self) -> SlotId {
        self.id
    }
}

/// Registers a block under the menu it declares.
///
/// # Panics
/// If the spec's group is not a menu this container declares.
pub fn register(reg: &mut Registry<MenuBlockSpec>, spec: MenuBlockSpec) {
    assert!(
        GROUPS.contains(&spec.group),
        "ext::menu: block `{}` is in group `{}`, which is not a bar menu",
        spec.id,
        spec.group
    );
    reg.insert(spec.group, spec);
}

/// The base's own five blocks.
///
/// A second edition receives this already seeded, which is what lets it relabel, reorder and
/// remove the base's own rather than only append to them (`D177`).
pub fn base_registry() -> Registry<MenuBlockSpec> {
    let mut reg = Registry::new();
    crate::ui::menus::blocks(&mut reg);
    reg
}

static RESOLVED: OnceLock<Vec<&'static MenuBlockSpec>> = OnceLock::new();

/// Installs the composed registry, before the first window.
///
/// The base binary never calls this: nothing installed resolves [`base_registry`] instead, which
/// is what keeps a base behaviour from depending on something having been registered.
///
/// # Panics
/// If the order has already been resolved — either a second `install`, or an `install` after
/// something drew.
pub fn install(reg: Registry<MenuBlockSpec>) {
    if RESOLVED.set(resolve(reg)).is_err() {
        panic!("ext::menu: the bar-menu container was already resolved before install");
    }
}

fn resolve(reg: Registry<MenuBlockSpec>) -> Vec<&'static MenuBlockSpec> {
    // Leaked deliberately and exactly once, the way the rail's is: the draw order is process-wide
    // and immortal.
    let reg: &'static mut Registry<MenuBlockSpec> = Box::leak(Box::new(reg));
    Registry::resolve(reg, GROUPS)
}

fn resolved() -> &'static [&'static MenuBlockSpec] {
    RESOLVED.get_or_init(|| resolve(base_registry()))
}

/// The blocks of one menu, in drawn order.
pub fn blocks_in(menu: SlotId) -> impl Iterator<Item = &'static MenuBlockSpec> {
    resolved()
        .iter()
        .copied()
        .filter(move |spec| spec.group == menu)
}

/// Every row one menu offers, its blocks concatenated in order.
pub fn rows(menu: SlotId, app: &AppState, cx: &App) -> Vec<MenuEntry> {
    blocks_in(menu)
        .flat_map(|spec| (spec.rows)(app, cx))
        .collect()
}
