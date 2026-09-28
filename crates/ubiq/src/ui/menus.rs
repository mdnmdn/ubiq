//! The top bar's menus, as data.
//!
//! Every chevron on the titlebar opens the same shape: a flat column of rows, each a label, a
//! glyph and one thing it does. Those rows used to be written three times over — an enum in
//! `state/workbench.rs` for the order, a `match` in the drawing module for the wording, and a
//! second `match` on `AppState` for the action, tied to the first two by *position*. This module
//! is the one list instead: a [`MenuEntry`] carries its own wording **and** its own action, so
//! reordering a menu is moving a line, and removing a row is commenting one out.
//!
//! One walker draws them all — [`overlay`] — and `AppState::pick_*_menu` resolves its index
//! against [`flatten`] of the very same list, so a row and the action behind it can no longer
//! drift apart.
//!
//! ## Extending a menu from outside this crate
//!
//! A bar menu is an extension container like the rail and the settings nav (`D177`, `D178`):
//! [`crate::ext::menu`] holds one [`MenuBlockSpec`](crate::ext::menu::MenuBlockSpec) per block of
//! rows, the base registers exactly one block per menu — the functions below — and a second
//! edition inserts, reorders or removes blocks on the `Registry` it is handed in
//! `ubiq_app::Contributions`. No hook, no second mechanism: the same four operations every other
//! container has.
//!
//! Every builder here is `pub`, so a block that wants the base's rows minus one writes
//! `ubiq::ui::menus::overflow(app, cx)` and filters what comes back.

use std::rc::Rc;

use gpui::{AnyElement, App, Context, ElementId, Entity, IntoElement, SharedString, Window};
use gpui::{point, px};
use gpui_component::{Icon, IconName};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::AgentPicks;

use crate::app::AppState;
use crate::ext::{SlotId, ids};
use crate::ui::kit;

/// What one row does when it is picked. The same shape `ui::handler` bridges, so an entry's
/// action is an ordinary method call on the root view.
pub type MenuAction = Rc<dyn Fn(&mut AppState, &mut Window, &mut Context<AppState>)>;

/// One row of a bar menu: what it says, what it looks like, whether it is offered, and what it
/// does.
///
/// A row with no action is drawn and does nothing — a heading, a hairline, or the sentence a menu
/// that opened onto nothing shows in place of being empty.
#[derive(Clone)]
pub struct MenuEntry {
    pub label: SharedString,
    /// A glyph before the label, from either icon set. Never *instead* of the label.
    pub icon: Option<Icon>,
    /// What the label has no room for, shown on hover.
    pub tooltip: Option<SharedString>,
    /// Drawn faint and unclickable. A heading is this, and so is an action the host cannot answer
    /// yet.
    pub enabled: bool,
    /// `false` drops the row before anything is drawn, so it occupies no index at all — the
    /// predicate form of commenting the line out.
    pub visible: bool,
    /// A hairline instead of a row. Still an entry, because a menu is a list of positions.
    pub separator: bool,
    pub action: Option<MenuAction>,
    /// Rows that belong under this one. The kit draws no nested panel, so [`flatten`] lays a
    /// submenu out flat: this entry heads it and its children follow, closed by a hairline.
    /// Nothing in the base groups its rows that way yet; it is here so a contributed block can
    /// without a second render path.
    pub submenu: Vec<MenuEntry>,
}

impl MenuEntry {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            tooltip: None,
            enabled: true,
            visible: true,
            separator: false,
            action: None,
            submenu: Vec::new(),
        }
    }

    /// The line between two groups of rows.
    pub fn separator() -> Self {
        Self {
            separator: true,
            enabled: false,
            ..Self::new("")
        }
    }

    /// A row that names a group, or says why a menu is empty: drawn faint, never picked.
    pub fn heading(label: impl Into<SharedString>) -> Self {
        Self::new(label).disabled()
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// The enabled predicate, for a caller with a condition rather than a fact.
    pub fn enabled_when(mut self, yes: bool) -> Self {
        self.enabled = yes;
        self
    }

    /// The visibility predicate. A row that answers `false` is not in the menu at all, and takes
    /// no index with it.
    pub fn visible_when(mut self, yes: bool) -> Self {
        self.visible = yes;
        self
    }

    /// What picking this row does.
    pub fn on(
        mut self,
        f: impl Fn(&mut AppState, &mut Window, &mut Context<AppState>) + 'static,
    ) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    pub fn submenu(mut self, rows: Vec<MenuEntry>) -> Self {
        self.submenu = rows;
        self
    }
}

// ── the walker ─────────────────────────────────────────────────────

/// The rows a definition really has: invisible entries dropped, a submenu laid flat under its
/// parent. The index of a row here is the index a pick carries, and it is what
/// `AppState::pick_*_menu` resolves against.
pub fn flatten(entries: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut out = Vec::with_capacity(entries.len());
    for mut entry in entries.into_iter().filter(|entry| entry.visible) {
        let children = std::mem::take(&mut entry.submenu);
        out.push(entry);
        if !children.is_empty() {
            out.extend(flatten(children));
            out.push(MenuEntry::separator());
        }
    }
    out
}

/// Turn a definition into the anchored menu the window paints, each row carrying its own action.
///
/// The single render walker: every bar menu's overlay is one call to it.
pub fn overlay(
    view: &Entity<AppState>,
    id: impl Into<ElementId>,
    at: (f32, f32),
    entries: Vec<MenuEntry>,
    dismiss: impl Fn(&mut AppState, &mut Window, &mut Context<AppState>) + 'static,
) -> AnyElement {
    let entries = flatten(entries);
    let items: Vec<kit::ContextItem> = entries.iter().map(item).collect();
    let actions: Rc<Vec<Option<MenuAction>>> = Rc::new(
        entries
            .into_iter()
            .map(|entry| entry.enabled.then_some(entry.action).flatten())
            .collect(),
    );
    let dismiss = Rc::new(dismiss);

    kit::context_menu(
        id,
        point(px(at.0), px(at.1)),
        items,
        {
            let view = view.clone();
            let dismiss = dismiss.clone();
            move |index, window, cx| {
                let action = actions.get(index).cloned().flatten();
                view.update(cx, |this, cx| {
                    // The menu closes first, whatever the row does: a row that raises a modal
                    // must not leave the menu it was picked from standing behind it.
                    dismiss(this, window, cx);
                    if let Some(action) = action {
                        action(this, window, cx);
                    }
                    cx.notify();
                });
            }
        },
        {
            let view = view.clone();
            move |window, cx| {
                view.update(cx, |this, cx| {
                    dismiss(this, window, cx);
                    cx.notify();
                });
            }
        },
    )
    .into_any_element()
}

/// One entry as the kit's row. A row with nothing behind it is drawn disabled whatever it claims:
/// a clickable row that does nothing is worse than a faint one that says so.
fn item(entry: &MenuEntry) -> kit::ContextItem {
    if entry.separator {
        return kit::ContextItem::separator();
    }
    let mut item = kit::ContextItem::new(entry.label.clone());
    if !entry.enabled || entry.action.is_none() {
        item = item.disabled();
    }
    if let Some(icon) = entry.icon.clone() {
        item = item.icon(icon);
    }
    if let Some(tooltip) = entry.tooltip.clone() {
        item = item.tooltip(tooltip);
    }
    item
}

/// Every row one bar menu offers, base blocks and contributed ones together, in the container's
/// resolved order.
///
/// The one entry point for both the overlay that draws a menu and the pick that acts on it.
pub fn entries(menu: SlotId, app: &AppState, cx: &App) -> Vec<MenuEntry> {
    flatten(crate::ext::menu::rows(menu, app, cx))
}

// ── the base's own blocks ──────────────────────────────────────────

/// The titlebar's new-project chevron: the same three ways into a project the picker's foot
/// offers. Reused, not restated — every action here is the one `ui::project_menu`'s own row runs.
pub fn new_project(app: &AppState, _cx: &App) -> Vec<MenuEntry> {
    // The picker's own wording for the third row, so the two never say something different about
    // the same action.
    let remote = match app.preferred_remote_host() {
        Some((_, label)) => format!("Open a project on {label}\u{2026}"),
        None => "Open remote project\u{2026}".to_string(),
    };
    vec![
        MenuEntry::new("Add a project\u{2026}")
            .icon(IconName::Plus)
            .on(|this, _, cx| this.choose_folder(None, cx)),
        MenuEntry::new("Clone a project\u{2026}")
            .icon(IconName::Globe)
            .on(|this, window, cx| this.open_clone(None, window, cx)),
        MenuEntry::new(remote)
            .icon(IconName::Network)
            .on(|this, window, cx| match this.preferred_remote_host() {
                Some((host, label)) => this.open_remote_project_picker(host, label, window, cx),
                None => this.open_remote_connect(window, cx),
            }),
    ]
}

/// What the run menu says when this host has no tool it could run here. Drawn disabled: a menu
/// that opened onto nothing at all would read as a control that did not work.
pub const NO_TOOLS_ROW: &str = "No tools for this project";

/// The titlebar's run chevron: every runnable tool this project offers, machine-wide rows and the
/// project's own together, in the order the host listed them.
///
/// `WorkbenchState::run_tool_rows` is still what says which tools are applicable — that is the
/// host's own stamp, and the play triangle beside the chevron reads the same list.
pub fn run_tool(app: &AppState, _cx: &App) -> Vec<MenuEntry> {
    let rows = app.workbench.run_tool_rows();
    if rows.is_empty() {
        return vec![MenuEntry::heading(NO_TOOLS_ROW)];
    }
    rows.iter()
        .map(|&at| {
            let name = app.workbench.tools[at].tool.name.clone();
            MenuEntry::new(SharedString::from(name))
                .icon(IconName::Play)
                .on(move |this, _, cx| this.run_tool_at(at, cx))
        })
        .collect()
}

/// The trailing row of the new-pane menu, which is not a shell: it brings the console back on
/// screen.
pub const CONSOLE_ROW: &str = "Logs";

/// The new-pane chevron: any pane still running with no panel drawing it, then every shell the
/// host found on this machine, then the console.
///
/// A window with no project can start no pane — there is no folder to run one in — so it is
/// offered the console alone. Each separator is a row like any other, and there is none where
/// there is nothing above it to separate.
///
/// **No harness is a row here**: starting an agent is the New agent form's job, which asks the
/// identity, the model, the level and the mode. **No runnable tool either** — those are the
/// titlebar's own play control's, through [`run_tool`].
pub fn new_pane(app: &AppState, cx: &App) -> Vec<MenuEntry> {
    let has_project = app.project(cx).is_some();
    let detached = app.detached_panes(cx);
    let mut entries = Vec::new();

    // Detached panes lead: picking work back up reads before starting something new.
    if !detached.is_empty() {
        entries.push(MenuEntry::heading("Detached"));
        entries.extend(detached.iter().map(|&id| detached_entry(app, id)));
        entries.push(MenuEntry::separator());
    }
    if has_project && !app.workbench.shells.is_empty() {
        entries.extend(app.workbench.shells.iter().map(|shell| {
            // The default is marked rather than reordered: the row a bare click on "+" already
            // runs has to be findable, and the host's order is the one shown.
            let label = if shell.is_default {
                format!("{} (default)", shell.label)
            } else {
                shell.label.clone()
            };
            let program = shell.program.clone();
            MenuEntry::new(SharedString::from(label)).on(move |this, _, cx| {
                this.spawn_pane(Some(program.clone()), Vec::new(), AgentPicks::default(), cx);
            })
        }));
        // The console is not a pane, and the line says so: everything above it starts something.
        entries.push(MenuEntry::separator());
    }
    entries
        .push(MenuEntry::new(CONSOLE_ROW).on(|this, window, cx| this.reveal_console(window, cx)));
    entries
}

/// What the hidden-agents menu says when nothing is hidden. Drawn disabled, for
/// [`NO_TOOLS_ROW`]'s reason.
pub const NO_HIDDEN_AGENTS_ROW: &str = "No hidden agents";

/// The chevron beside New agent: every agent pane a tab's `Hide` took off the screen, with its
/// harness still running behind it (`T-266`).
///
/// **Hidden is `AppState::detached_panes` narrowed to agents** — the panes whose harness is one
/// the host offers as an agent type. A shell hidden the same way is the new-pane menu's Detached
/// group, not this one; there is no second concept and no new message behind it, because a
/// detach is already the difference between the panes a project holds and the panels drawing
/// them. Picking a row **reattaches** the live pane: the panel comes back over the emulator that
/// never stopped, and nothing is respawned.
pub fn hidden_agents(app: &AppState, cx: &App) -> Vec<MenuEntry> {
    let hidden: Vec<PaneId> = app
        .detached_panes(cx)
        .into_iter()
        .filter(|&id| {
            app.pane(id)
                .is_some_and(|pane| is_agent(app, &pane.harness))
        })
        .collect();

    if hidden.is_empty() {
        return vec![MenuEntry::heading(NO_HIDDEN_AGENTS_ROW)];
    }
    hidden
        .into_iter()
        .map(|id| detached_entry(app, id).icon(IconName::Bot))
        .collect()
}

/// Whether a pane's harness is one the host offers as an agent, rather than a shell or a tool.
fn is_agent(app: &AppState, harness: &str) -> bool {
    app.workbench
        .agent_types
        .iter()
        .any(|agent| agent.id == harness)
}

/// One still-running pane with no panel over it, named the way both menus that list one name it:
/// the harness as the host labels it, then the tab's own title.
fn detached_entry(app: &AppState, id: PaneId) -> MenuEntry {
    let Some(pane) = app.pane(id) else {
        // A pane that went away between the list and the row. Drawn empty and dead rather than
        // dropped, so no other row moves under the pointer mid-frame.
        return MenuEntry::heading("");
    };
    let harness = app
        .workbench
        .agent_types
        .iter()
        .find(|agent| agent.id == pane.harness)
        .map(|agent| agent.label.clone())
        .unwrap_or_else(|| pane.harness.clone());
    MenuEntry::new(SharedString::from(format!(
        "{harness} \u{00b7} {}",
        pane.title
    )))
    .on(move |this, _, cx| this.reattach_pane(id, cx))
}

/// The titlebar's overflow chevron: the commands reached occasionally rather than every session,
/// moved off the strip to make room for the ones that are not.
pub fn overflow(app: &AppState, cx: &App) -> Vec<MenuEntry> {
    let has_project = app.project(cx).is_some();
    let capture_offered = app.capture_offered(cx);
    vec![
        MenuEntry::new("Connect to a remote host")
            .icon(kit::UbiqIcon::HostRemote)
            .on(|this, window, cx| this.open_remote_connect(window, cx)),
        MenuEntry::new("Explore the project in browser")
            .icon(kit::UbiqIcon::TitlebarBrowser)
            .visible_when(has_project)
            .on(|this, window, cx| this.open_web_export(window, cx)),
        MenuEntry::new("Capture this window")
            .icon(kit::UbiqIcon::CaptureWindow)
            .visible_when(has_project && capture_offered)
            .on(|this, window, cx| this.capture_window(&crate::app::CaptureWindow, window, cx)),
        // Always offered, project or not: help is about the application, and a window with no
        // folder open is one of the places a reader most wants it.
        MenuEntry::new("Help")
            .icon(kit::UbiqIcon::TitlebarHelp)
            .on(|this, window, cx| this.reveal_help(window, cx)),
        // Under Help, because it is the same question the other way round: Help opens the page
        // for where you are standing, this one waits for you to point at something. The same
        // glyph, because the registry has no pointer icon — `G294`'s neighbour rather than this
        // change's business.
        MenuEntry::new("Point at something\u{2026}")
            .icon(kit::UbiqIcon::TitlebarHelp)
            .on(|this, _, cx| this.open_help_target(cx)),
        MenuEntry::new("Settings")
            .icon(kit::UbiqIcon::TitlebarSettings)
            .on(|this, _, cx| this.toggle_settings(cx)),
    ]
}

/// The base's five blocks, one per menu, registered the way a second edition registers its own.
pub fn blocks(reg: &mut crate::ext::Registry<crate::ext::menu::MenuBlockSpec>) {
    use crate::ext::menu::{MenuBlockSpec, register};

    for (id, group, rows) in [
        (
            ids::MENU_NEW_PROJECT_BASE,
            ids::MENU_NEW_PROJECT,
            new_project as fn(&AppState, &App) -> Vec<MenuEntry>,
        ),
        (ids::MENU_RUN_TOOL_BASE, ids::MENU_RUN_TOOL, run_tool),
        (ids::MENU_NEW_PANE_BASE, ids::MENU_NEW_PANE, new_pane),
        (ids::MENU_OVERFLOW_BASE, ids::MENU_OVERFLOW, overflow),
        (
            ids::MENU_HIDDEN_AGENTS_BASE,
            ids::MENU_HIDDEN_AGENTS,
            hidden_agents,
        ),
    ] {
        register(reg, MenuBlockSpec { id, group, rows });
    }
}
