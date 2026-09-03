//! The window's arrangement: a dock of movable panels, and the adapter that makes one.
//!
//! Every area of the workbench is a **panel** in a tree the user rearranges by dragging — tabs
//! come from the middle of a group, rows and columns from its edges. The tree, the drag, the drop
//! indicators and the serialisation are the component library's; what Ubiq owns is which panels
//! exist, where each may sit, and how they are drawn ([`skin`]).
//!
//! **A panel is a view, and `AppState` is the only owner of state.** The library requires a panel
//! to be an entity that renders, focuses and emits, which is the half of `D17` this reverses. The
//! half that stays is the one that mattered: [`WorkbenchPanel`] holds a weak `AppState` handle and
//! a [`PanelKind`], and its render delegates straight to the area functions that exist. Adding a
//! panel is an arm of a `match`, not a new owner of state.
//!
//! The three facts beside them are not an exception to that. They are what the library asks a panel
//! *outside* a render — whether it is drawn, whether it is in the tree, and what a file panel
//! writes into the saved arrangement — and each is pushed to the panel rather than read back,
//! because the dock asks them while the window is mid-update and reading the window there is a
//! panic rather than a stale answer.
//!
//! **In IDE mode each open file is a panel.** Its tab therefore belongs to the group it sits in,
//! which is what lets a file be dragged beside another, and the centre panel steps aside for as
//! long as one is open. The tab key travels in the panel's payload rather than in its name, because
//! a name is a `&'static str` and is the same for every file.
//!
//! **Moving a panel does not rebuild it.** A dragged tab is re-parented by id, so a dragged
//! terminal is the same `TerminalView`, on the same stream, under the same harness — and it is
//! laid out in its new rectangle, measures itself, and posts `TerminalResize`. *A move is a
//! resize*, and nothing here has to arrange that.

pub mod skin;

use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement as _, Pixels, Render, Rgba, SharedString, WeakEntity, Window, div,
    px,
};
use gpui_component::dock::{
    BasePanel, BasePanelView, DockArea, DockLayout, DockPlacement, InsertTarget, PaneRef, Panel,
    PanelEvent, PanelId, PanelInfo, PanelState, TabGroup, panel_handle,
};

use ubiq_proto::ids::PaneId;

use crate::app::AppState;
use crate::state::RailMode;
use crate::state::dock::{PanelKind, Region};
use crate::state::editor::ViewLayout;
use crate::theme;
use crate::ui::{
    agents, board, chat, editor, empty, explorer, git, logs, orchestration, rail, search, sink,
    terminal,
};

/// The version a saved layout is written under. It travels with the preferences schema, because
/// the layout blob is one field of the same document.
pub const LAYOUT_VERSION: usize = crate::state::prefs::SCHEMA as usize;

/// Ubiq's region as the library's placement, and back. The two enums are the same four values;
/// keeping our own is what stops `state/` naming a widget library.
pub fn placement_of(region: Region) -> DockPlacement {
    match region {
        Region::Centre => DockPlacement::Center,
        Region::Left => DockPlacement::Left,
        Region::Right => DockPlacement::Right,
        Region::Bottom => DockPlacement::Bottom,
    }
}

pub fn region_of(placement: DockPlacement) -> Region {
    match placement {
        DockPlacement::Center => Region::Centre,
        DockPlacement::Left => Region::Left,
        DockPlacement::Right => Region::Right,
        DockPlacement::Bottom => Region::Bottom,
    }
}

/// One movable panel: what it is, and the window it reads.
///
/// It holds no state of its own beyond the kind that identifies it — a pane id, a tab key, or
/// nothing at all.
/// Everything it draws comes from `AppState`, which is why a panel can be dragged, tabbed and
/// rebuilt from a saved layout without anything being lost.
pub struct WorkbenchPanel {
    kind: PanelKind,
    /// The window this panel is a view of, weakly: the state owns the dock, the dock owns this,
    /// and a strong handle back would be a cycle the window never gets out of.
    app: WeakEntity<AppState>,
    /// Where the keyboard rests for a panel that is not a terminal. A terminal answers with its
    /// emulator's handle instead, which is what puts keystrokes on the harness.
    focus_handle: FocusHandle,
    /// Whether the panel is drawn at all — **pushed by the window, never read back from it.**
    ///
    /// The dock asks this question while it is reconciling a tree, and it reconciles from inside
    /// the window's own update: a toggled region, an added panel, a layout written down. Reading
    /// `AppState` there would be reading an entity that is already leased, which is a panic rather
    /// than a stale answer. So the one fact the dock needs of a panel out of band is the one fact
    /// the window keeps current, in `AppState::settle_visibility`.
    visible: bool,
    /// Whether the panel is in the dock's tree. What tells a terminal panel the user closed —
    /// which kills its harness, and a file panel's which closes its tab — from one displaced by a
    /// whole arrangement being installed over it, which must not.
    attached: bool,
    /// Which of its viewer's layouts a file panel's file is in — **pushed by the window** for the
    /// same reason `visible` is, and for one more: [`BasePanel::dump`] is reached from inside the
    /// window's own update, so the payload a panel writes has to be a fact it already holds.
    layout: ViewLayout,
}

impl WorkbenchPanel {
    pub fn new(kind: PanelKind, app: WeakEntity<AppState>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            kind,
            app,
            focus_handle: cx.focus_handle(),
            visible: true,
            attached: false,
            layout: ViewLayout::default(),
        })
    }

    /// Tell the panel whether it is drawn. Answers whether that changed, so the window only
    /// redraws when it did.
    pub fn set_visible(&mut self, visible: bool) -> bool {
        let changed = self.visible != visible;
        self.visible = visible;
        changed
    }

    pub fn kind(&self) -> &PanelKind {
        &self.kind
    }

    /// Whether the panel is in the dock's tree. What stops a panel already restored from a saved
    /// arrangement being added to it a second time.
    pub fn attached(&self) -> bool {
        self.attached
    }

    /// Tell a file panel which layout its file is in, for the payload it writes down. Answers
    /// whether that changed, like [`WorkbenchPanel::set_visible`].
    pub fn set_layout(&mut self, layout: ViewLayout) -> bool {
        let changed = self.layout != layout;
        self.layout = layout;
        changed
    }

    /// The handle a dock hands a panel, carrying its title across the renderer seam.
    pub fn handle(panel: &Entity<Self>) -> Arc<dyn BasePanelView> {
        panel_handle(panel.clone())
    }

    /// What the tab says and how it is drawn.
    pub fn tab(&self, cx: &App) -> TabInfo {
        let Some(app) = self.app.upgrade() else {
            return TabInfo {
                label: SharedString::from(self.kind.name()),
                ..TabInfo::default()
            };
        };
        let app = app.read(cx);
        match &self.kind {
            PanelKind::Terminal(pane_id) => {
                let pane = app.pane(*pane_id);
                let title = pane
                    .map(|pane| pane.title.clone())
                    .unwrap_or_else(|| "pane".to_string());
                let dot = match pane.map(|pane| pane.running) {
                    Some(true) => theme::success(),
                    _ => theme::text_faint(),
                };
                TabInfo {
                    label: SharedString::from(title),
                    dot_colour: Some(dot),
                    ..TabInfo::default()
                }
            }
            PanelKind::Logs => {
                let dot = ubiq_proto::log::logs()
                    .loudest()
                    .filter(|level| *level >= ubiq_proto::log::LogLevel::Warn)
                    .map(logs::level_colour);
                TabInfo {
                    label: "Logs".into(),
                    dot_colour: dot,
                    ..TabInfo::default()
                }
            }
            PanelKind::Search => TabInfo {
                label: "Search".into(),
                ..TabInfo::default()
            },
            PanelKind::Explorer => TabInfo {
                label: "Explorer".into(),
                ..TabInfo::default()
            },
            PanelKind::Chat => TabInfo {
                label: "Chat".into(),
                ..TabInfo::default()
            },
            PanelKind::Centre => TabInfo {
                label: centre_title(app.workbench.rail_mode).into(),
                ..TabInfo::default()
            },
            // A file's tab is the file's own report — its name, what it is looking at, whether it
            // is dirty and whether its close is a question — which is `ui/editor.rs`'s to say.
            PanelKind::File(key) => match app.file(key, cx) {
                Some(file) => {
                    let asking = app
                        .editor(cx)
                        .and_then(|editor| editor.pending_tab_close.clone())
                        == Some(key.clone());
                    let explorer = app.explorer(cx);
                    TabInfo {
                        label: editor::label(file, asking),
                        title_colour: explorer
                            .map(|explorer| editor::git_colour(file, explorer))
                            .unwrap_or_else(theme::text_muted),
                        dot_colour: did_save_or_dirty(file).then(|| editor::dirty_colour(file)),
                        temporary: file.temporary,
                    }
                }
                // The tab of a file this window no longer holds. It is hidden rather than drawn,
                // so this is the name it keeps its slot under.
                None => TabInfo {
                    label: SharedString::from(crate::state::editor::from_tab_key(key).0),
                    ..TabInfo::default()
                },
            },
        }
    }
}

/// The dirty/save indicator appears only while the file has something to say — a dot on a
/// clean, idle tab would be furniture, not information.
fn did_save_or_dirty(file: &crate::state::OpenFile) -> bool {
    file.dirty()
        || matches!(
            (&file.save, &file.body),
            (
                crate::state::SaveState::Failed(_) | crate::state::SaveState::Saving(_),
                _
            ) | (
                _,
                crate::state::FileBody::Failed(_) | crate::state::FileBody::Loading
            )
        )
}

/// What the centre's tab is called. It follows the rail mode, because the centre is what the rail
/// selects between.
fn centre_title(mode: RailMode) -> &'static str {
    match mode {
        RailMode::Ide => "Editor",
        RailMode::Sink => "Kitchen sink",
        other => other.label(),
    }
}

/// Everything a tab's skin needs to draw it: the label, the title's colour, whether a status dot
/// sits at the row's right edge and what it says, and whether the tab is a temporary preview.
#[derive(Clone)]
pub struct TabInfo {
    pub label: SharedString,
    pub title_colour: Rgba,
    pub dot_colour: Option<Rgba>,
    pub temporary: bool,
}

impl Default for TabInfo {
    fn default() -> Self {
        Self {
            label: SharedString::default(),
            title_colour: theme::text(),
            dot_colour: None,
            temporary: false,
        }
    }
}

impl BasePanel for WorkbenchPanel {
    fn panel_name(&self) -> &'static str {
        self.kind.name()
    }

    /// A panel that has nothing to show is hidden rather than removed: it keeps its place in the
    /// tree and its tab slot, and comes back where it was left. What decides it is
    /// [`crate::state::dock::PanelKind`]'s own rule, applied by the window — see the field.
    fn visible(&self, _: &App) -> bool {
        self.visible
    }

    fn closable(&self, _: &App) -> bool {
        self.kind.closable()
    }

    /// The dock is where focus is decided, and `AppState` learns it from here rather than the other
    /// way round. A terminal panel becoming the displayed tab is what sends `Focus`; any other
    /// panel becoming it clears the focused pane, so no pane receives keystrokes.
    /// **The answer waits a turn, for the reason [`BasePanel::on_removed`] does.**
    ///
    /// This arrives while the dock is reconciling, which is *inside this panel's own update*. What
    /// the window does about it ends in writing the arrangement down, and writing it down reads
    /// every panel in the tree — including this one, which is still leased. Doing it here is
    /// therefore not a race but a certainty, so it is deferred to after the lease has ended.
    fn set_active(&mut self, active: bool, _window: &mut Window, cx: &mut Context<Self>) {
        if !active {
            return;
        }
        let kind = self.kind.clone();
        let app = self.app.clone();
        cx.defer(move |cx| {
            _ = app.update(cx, |app, cx| match kind.pane() {
                Some(pane_id) => app.focus_pane(pane_id, cx),
                None => {
                    app.blur_panes(cx);
                    // A file panel becoming the displayed tab is what makes its file the active
                    // one: the dock is where that is decided, and the editor learns it from here.
                    if let Some(key) = kind.tab_key() {
                        app.activate_file(key, cx);
                    }
                }
            });
        });
    }

    fn on_added_to(&mut self, _: WeakEntity<TabGroup>, _: &mut Window, _: &mut Context<Self>) {
        self.attached = true;
    }

    /// **Closing a terminal panel closes its pane, closing a file panel closes its tab, and being
    /// displaced does neither.**
    ///
    /// The library reports both the same way: a panel is told it left the dock whether the user
    /// closed its tab or a whole arrangement was installed over it. The two have to be told apart,
    /// because one of them kills a harness. So the answer waits a turn — a displaced panel is put
    /// back in the same edit, and hears [`BasePanel::on_added_to`] again before this runs.
    ///
    /// Waiting is also what makes it safe: this arrives while the dock is reconciling, which is
    /// inside the window's own update, and ending a pane is the window's to do.
    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.attached = false;
        let kind = self.kind.clone();
        if kind.pane().is_none() && kind.tab_key().is_none() {
            return;
        }
        let app = self.app.clone();
        let panel = cx.weak_entity();
        cx.defer(move |cx| {
            if panel
                .read_with(cx, |panel, _| panel.attached)
                .unwrap_or(true)
            {
                return;
            }
            _ = app.update(cx, |app, cx| match &kind {
                PanelKind::Terminal(pane_id) => app.close_pane(*pane_id, cx),
                PanelKind::File(key) => app.closed_file_panel(key, cx),
                _ => {}
            });
        });
    }

    /// What a saved layout carries for this panel: **what it is looking at, never what it drew.**
    ///
    /// Two panels write more than their name, because their name is the same for every one of
    /// them. A file writes its tab key and the layout its viewer was left in; a terminal writes
    /// its pane's id, which is what lets the arrangement put a pane back where the user left it
    /// when the window still holds it. Never a parsed scene, a computed diff or a rendered
    /// diagram: those are functions of bytes the host will send again.
    fn dump(&self, _: &App) -> PanelState {
        let mut state = PanelState::new(self.kind.name());
        if let Some(key) = self.kind.tab_key() {
            state.info = PanelInfo::panel(file_payload(key, self.layout));
        }
        if let Some(pane_id) = self.kind.pane() {
            state.info = PanelInfo::panel(pane_payload(pane_id));
        }
        state
    }
}

impl Panel for WorkbenchPanel {
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let TabInfo { label, .. } = self.tab(cx);
        div().child(label)
    }

    /// The dock's own padding would inset the coloured left edge every surface is identified by,
    /// which `D18` says has to sit on the boundary. Every panel here draws its own frame.
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for WorkbenchPanel {}

impl Focusable for WorkbenchPanel {
    /// A terminal panel's keyboard *is* its emulator's, so focusing the panel puts keystrokes on
    /// the harness with nothing in between. Every other panel answers with its own handle, which
    /// is how "when the focused panel is not a terminal, no pane holds the keyboard" is true by
    /// construction rather than by a rule someone has to remember.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if let Some(pane_id) = self.kind.pane()
            && let Some(app) = self.app.upgrade()
            && let Some(terminal) = app.read(cx).terminal(pane_id)
        {
            return terminal.read(cx).focus_handle().clone();
        }
        self.focus_handle.clone()
    }
}

impl Render for WorkbenchPanel {
    /// **The panel renders by updating `AppState` from inside its own render**, which is sound
    /// because a child view's render runs in the layout pass, after the parent's has returned: the
    /// state is not leased at this point. That is what lets every area module keep the
    /// `fn(&AppState, &mut Context<AppState>)` signature it already has — the adapter is a
    /// `match`, and nothing under `ui/` was rewritten to become a panel.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self.kind.clone();
        self.app
            .update(cx, |app, cx| body(&kind, app, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// One panel's body, drawn by the area module that already owned it.
///
/// Every arm is a call into `ui/` with the signature those modules already have, which is what
/// makes the adapter a `match` rather than a rewrite.
fn body(
    kind: &PanelKind,
    app: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> AnyElement {
    match kind {
        PanelKind::Terminal(pane_id) => terminal::pane(app, *pane_id, cx),
        PanelKind::Logs => logs::render(app, cx),
        PanelKind::Search => search::render(app, window, cx),
        PanelKind::Explorer => explorer::render(app, window, cx),
        PanelKind::Chat => chat::render(app, window, cx).into_any_element(),
        PanelKind::Centre => centre(app, window, cx),
        PanelKind::File(key) => editor::render_file(app, key, cx),
    }
}

/// The centre region's one panel. Which screen it is is the rail's answer, which is what the rail
/// has always decided; what has changed is that the screen is now a panel like any other.
fn centre(app: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> AnyElement {
    let wb = &app.workbench;
    let has_project = app.project(cx).is_some();

    match wb.rail_mode {
        RailMode::Ide if has_project => editor::render(app, cx),
        RailMode::Git if has_project => git::render(app, window, cx).into_any_element(),
        RailMode::Agents if has_project => agents::render(app, window, cx).into_any_element(),
        RailMode::Orchestration if has_project => {
            orchestration::render(app, window, cx).into_any_element()
        }
        RailMode::Tasks if has_project => board::render(app, window, cx).into_any_element(),
        // The two modes that are about the application rather than a project answer whether or
        // not one is open: the sink draws its own fixtures, and Control says what it will hold.
        RailMode::Sink => sink::render(app, window, cx),
        RailMode::Control => not_built(RailMode::Control),
        // Every other mode is a project's, as the rail says by putting them under `PROJECT`. With
        // none open there is no work to draw, so they say what the editor says.
        _ if !has_project => empty::no_project(cx),
        mode => not_built(mode),
    }
}

/// A rail mode with no screen behind it. A stated gap, not an error state.
fn not_built(mode: RailMode) -> AnyElement {
    empty::empty_page(
        mode.label(),
        mode.note(),
        rail::mode_icon(mode),
        Some(empty::not_built()),
    )
    .into_any_element()
}

// ── Building the arrangement ────────────────────────────────────────

/// The arrangement a window opens in, and the one a discarded layout falls back to: the explorer
/// on the left border, the chat on the right, the centre above an empty bottom region.
///
/// **Nothing is in the bottom region and it opens closed.** A pane exists because the coordinator
/// says it does, so its panel arrives with `WorkspaceSpawned`; the console is opened from the
/// new-pane control's menu. What the region gives a fresh window is its size and its strip, so
/// there is somewhere for the first of either to land.
pub fn default_layout(
    dock: &Entity<DockArea>,
    panel: &mut impl FnMut(PanelKind, &mut App) -> Option<Entity<WorkbenchPanel>>,
    window: &mut Window,
    cx: &mut App,
) {
    // Only a terminal is ever refused, and none of these is one.
    let (Some(explorer), Some(chat), Some(centre)) = (
        panel(PanelKind::Explorer, cx),
        panel(PanelKind::Chat, cx),
        panel(PanelKind::Centre, cx),
    ) else {
        return;
    };
    let explorer = WorkbenchPanel::handle(&explorer);
    let chat = WorkbenchPanel::handle(&chat);
    let centre = WorkbenchPanel::handle(&centre);

    dock.update(cx, |dock, cx| {
        dock.set_center(DockLayout::tabs().panel_view(centre, cx), window, cx);
        install(
            dock,
            Region::Left,
            DockLayout::tabs().panel_view(explorer, cx),
            px(theme::EXPLORER_WIDTH),
            window,
            cx,
        );
        install(
            dock,
            Region::Right,
            DockLayout::tabs().panel_view(chat, cx),
            px(theme::CHAT_WIDTH),
            window,
            cx,
        );
        install(
            dock,
            Region::Bottom,
            DockLayout::tabs(),
            px(theme::DOCK_HEIGHT),
            window,
            cx,
        );
        // A region with nothing in it is a strip of nothing, so it starts put away. It is opened
        // by the titlebar's switch — which starts a pane in it — or by the first pane or console
        // that asks for it.
        if dock.is_dock_open(placement_of(Region::Bottom)) {
            dock.toggle_dock(placement_of(Region::Bottom), window, cx);
        }
    });
}

/// One edge region, at the size the theme calls its default. Every region collapses, because a
/// region the user cannot put away is a frame again.
fn install(
    dock: &mut DockArea,
    region: Region,
    layout: DockLayout,
    size: Pixels,
    window: &mut Window,
    cx: &mut Context<DockArea>,
) {
    let placement = placement_of(region);
    dock.set_dock(placement, layout, window, cx);
    dock.set_dock_size(placement, size, window, cx);
    dock.set_dock_collapsible(placement, true, window, cx);
}

/// Whether the dock's tree already holds a panel. What stops a file panel restored from a saved
/// arrangement being added a second time by the queued edit that also asked for it.
pub fn holds(dock: &Entity<DockArea>, panel: &Entity<WorkbenchPanel>, cx: &mut App) -> bool {
    let id = PanelId::from(panel.entity_id());
    dock.update(cx, |dock, _| {
        [Region::Centre, Region::Left, Region::Right, Region::Bottom]
            .into_iter()
            .any(|region| {
                dock.layout(placement_of(region))
                    .is_some_and(|tree| tree.panels().any(|panel| panel == id))
            })
    })
}

/// Put a panel in a region, joining the group that is already there.
pub fn add(
    dock: &Entity<DockArea>,
    panel: &Entity<WorkbenchPanel>,
    region: Region,
    window: &mut Window,
    cx: &mut App,
) {
    let handle = WorkbenchPanel::handle(panel);
    dock.update(cx, |dock, cx| {
        dock.add_panel_view(handle, placement_of(region), None, window, cx);
    });
}

/// Bring a panel that is already in the arrangement back on screen.
///
/// The console is not closable — it keeps its place in the tree — so "show me the console" is a
/// reveal rather than an add: whichever region holds it is brought back if it was put away, and its
/// tab is made the one its group displays. A panel the tree does not hold is put in its home region
/// first, which is what makes this safe to call for a panel that has never been installed.
pub fn reveal(
    dock: &Entity<DockArea>,
    panel: &Entity<WorkbenchPanel>,
    home: Region,
    window: &mut Window,
    cx: &mut App,
) {
    if !holds(dock, panel, cx) {
        add(dock, panel, home, window, cx);
    }
    let id = PanelId::from(panel.entity_id());
    dock.update(cx, |dock, cx| {
        let found = [Region::Bottom, Region::Left, Region::Right, Region::Centre]
            .into_iter()
            .find_map(|region| {
                dock.layout(placement_of(region))
                    .and_then(|tree| tree.find_panel_node(id))
                    .map(|node| (region, node))
            });
        let Some((region, node)) = found else { return };
        // The centre is not a region that can be put away, so only an edge is ever brought back.
        let placement = placement_of(region);
        if region != Region::Centre && !dock.is_dock_open(placement) {
            dock.toggle_dock(placement, window, cx);
        }
        dock.move_panel(
            id,
            InsertTarget::Tabs {
                node,
                ix: None,
                activate: true,
            },
            window,
            cx,
        );
    });
}

/// Take a panel out of the dock for good. Only a terminal and a file ever leave — every other panel
/// is hidden instead, which is what keeps its place in the tree.
pub fn remove(
    dock: &Entity<DockArea>,
    panel: &Entity<WorkbenchPanel>,
    window: &mut Window,
    cx: &mut App,
) {
    dock.update(cx, |dock, cx| {
        dock.remove_panel(panel.clone(), window, cx);
    });
}

// ── The placement policy ────────────────────────────────────────────

/// Put back any panel that has landed in a region its class forbids.
///
/// The library's drop is region-blind — a group offers a drop or it does not — so the policy is
/// applied where the arrangement is known rather than where the pointer is: an Edge panel dropped
/// in the centre is moved back to its home border on the same edit, and the drop reads as refused.
/// Answered as a `bool` so a caller knows whether the layout it was about to write down is the one
/// still on screen.
pub fn enforce_placement(
    dock: &Entity<DockArea>,
    kind_of: &impl Fn(PanelId) -> Option<PanelKind>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let mut strays: Vec<(PanelId, Region)> = Vec::new();

    dock.update(cx, |dock, _| {
        for region in [Region::Centre, Region::Left, Region::Right, Region::Bottom] {
            let Some(tree) = dock.layout(placement_of(region)) else {
                continue;
            };
            for panel in tree.panels() {
                let Some(kind) = kind_of(panel) else { continue };
                if !kind.class().allows(region) {
                    strays.push((panel, kind.home()));
                }
            }
        }
    });

    if strays.is_empty() {
        return false;
    }

    for (panel, home) in strays {
        let target = dock.update(cx, |dock, _| first_group(dock, home));
        let Some(node) = target else { continue };
        dock.update(cx, |dock, cx| {
            dock.move_panel(
                panel,
                InsertTarget::Tabs {
                    node,
                    ix: None,
                    activate: true,
                },
                window,
                cx,
            );
        });
    }
    true
}

/// The first tab group in a region, which is where a panel put back in it goes.
fn first_group(dock: &DockArea, region: Region) -> Option<gpui_component::dock::NodeId> {
    let tree = dock.layout(placement_of(region))?;
    let mut found = None;
    tree.root().walk(&mut |node| {
        if found.is_none() && matches!(node.kind(), PaneRef::Tabs { .. }) {
            found = Some(node.id());
        }
    });
    found
}

// ── Persistence ─────────────────────────────────────────────────────

/// Rebuild a saved arrangement, or answer `false` for one this build cannot use.
///
/// Two rules from the proposal are enforced here rather than described. **A stale version is
/// discarded whole** for the default arrangement, because a half-applied layout is worse than none.
/// And **layout persists; harnesses do not** — a saved terminal leaf is dropped and the tree
/// normalises around the gap, so a window reopens with its side panels, its console and its open
/// files where they were, and no terminals. Restoring those is `Q1`.
pub fn restore(
    dock: &Entity<DockArea>,
    saved: &serde_json::Value,
    panel: &mut impl FnMut(PanelKind, &mut App) -> Option<Entity<WorkbenchPanel>>,
    layouts: &mut Vec<(String, ViewLayout)>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Ok(state) = serde_json::from_value::<gpui_component::dock::DockAreaState>(saved.clone())
    else {
        tracing::debug!("discarding an unreadable dock layout");
        return false;
    };
    if state.version != Some(LAYOUT_VERSION) {
        tracing::debug!(
            "discarding a dock layout written for version {:?}",
            state.version
        );
        return false;
    }

    let centre = rebuild(&state.center, panel, layouts, cx);
    let mut regions: Vec<(Region, DockLayout, Pixels, bool)> = Vec::new();
    for (saved, region) in [
        (&state.left_dock, Region::Left),
        (&state.right_dock, Region::Right),
        (&state.bottom_dock, Region::Bottom),
    ] {
        let Some(saved) = saved else { continue };
        // A region whose every panel was dropped is installed empty rather than left out. Leaving
        // it out kept whatever the mode before it had in that region on screen, under the incoming
        // mode's rail — and an empty region is a legal arrangement here: the pane region starts
        // that way, and its strip is where a pane is opened from.
        let layout = rebuild(saved.panel(), panel, layouts, cx).unwrap_or_else(DockLayout::tabs);
        regions.push((region, layout, saved.size(), saved.open()));
    }

    // A layout that rebuilt no centre is one whose every centre panel this build has lost. The
    // default is the honest answer, rather than a window with nothing in the middle.
    let Some(centre) = centre else { return false };

    dock.update(cx, |dock, cx| {
        dock.set_center(centre, window, cx);
        for (region, layout, size, open) in regions {
            install(dock, region, layout, size, window, cx);
            // The blob says whether the region was on screen. A region is forced to match — left
            // shut by the mode we sat in meanwhile, it is reopened here, and one that was shut is
            // shut again — because coming back to a mode must restore the whole arrangement, not
            // just the panels inside a region that already happened to be open.
            if open != dock.is_dock_open(placement_of(region)) {
                dock.toggle_dock(placement_of(region), window, cx);
            }
        }
    });
    true
}

/// One saved node as a layout this build can install.
///
/// Ubiq rebuilds the tree itself rather than through the library's global panel registry, because
/// a registered builder is handed the dock area and never the window's `AppState` — and every panel
/// here is a view of exactly one window's state. Nothing is lost by it: the names are Ubiq's own,
/// so nothing else can be in the file.
///
/// `None` means the node rebuilt to nothing — every panel under it was dropped — and the caller
/// leaves the slot out rather than installing an empty container.
fn rebuild(
    state: &PanelState,
    panel: &mut impl FnMut(PanelKind, &mut App) -> Option<Entity<WorkbenchPanel>>,
    layouts: &mut Vec<(String, ViewLayout)>,
    cx: &mut App,
) -> Option<DockLayout> {
    match &state.info {
        PanelInfo::Stack { sizes, axis } => {
            let mut layout = match axis {
                0 => DockLayout::h_split(),
                _ => DockLayout::v_split(),
            };
            let mut any = false;
            for (ix, child) in state.children.iter().enumerate() {
                let Some(child) = rebuild(child, panel, layouts, cx) else {
                    continue;
                };
                any = true;
                layout = layout.child(child, sizes.get(ix).copied());
            }
            any.then_some(layout)
        }
        PanelInfo::Tabs { active_index } => {
            let mut layout = DockLayout::tabs();
            let mut count = 0;
            for child in &state.children {
                // A leaf this build cannot rebuild — a terminal, always — is dropped.
                let Some(kind) = leaf(child, layouts) else {
                    continue;
                };
                // A panel the window will not supply — a terminal whose pane has gone — is dropped
                // the same way an unreadable leaf is.
                let Some(built) = panel(kind, cx) else {
                    continue;
                };
                layout = layout.panel_view(WorkbenchPanel::handle(&built), cx);
                count += 1;
            }
            (count > 0).then(|| layout.active_index((*active_index).min(count - 1)))
        }
        // A bare leaf where a container belongs, and a tiles canvas Ubiq never builds. Both are
        // read as a group of one so a hand-edited file cannot lose a panel.
        PanelInfo::Panel(_) | PanelInfo::Tiles { .. } => {
            let kind = leaf(state, layouts)?;
            let built = panel(kind, cx)?;
            Some(DockLayout::tabs().panel_view(WorkbenchPanel::handle(&built), cx))
        }
    }
}

/// One saved leaf as the kind it names, collecting what its payload carried on the way.
///
/// Every panel but a file is its name and nothing else. A file's name is the same for all of them,
/// so it is the payload beside it that says which tab it is — and the layout it was left in, which
/// is handed back for the caller to put on the file rather than applied here: this function knows
/// nothing about open files.
fn leaf(state: &PanelState, layouts: &mut Vec<(String, ViewLayout)>) -> Option<PanelKind> {
    if state.panel_name == PanelKind::TERMINAL {
        let PanelInfo::Panel(payload) = &state.info else {
            // A terminal panel from a build that wrote no payload names no pane.
            return None;
        };
        return pane_from_payload(payload);
    }
    if state.panel_name != PanelKind::File(String::new()).name() {
        return PanelKind::from_name(&state.panel_name);
    }
    let PanelInfo::Panel(payload) = &state.info else {
        // A file panel with no payload names no tab, so there is nothing to rebuild it as.
        return None;
    };
    let (kind, layout) = file_from_payload(payload)?;
    if let Some(key) = kind.tab_key() {
        layouts.push((key.to_string(), layout));
    }
    Some(kind)
}

/// What a file panel writes into the dock's saved layout: **what it is looking at, not what it
/// drew.** The tab key, because the panel's name is the same for every file, and the layout the
/// viewer was left in, because that is the one thing a viewer keeps.
pub fn file_payload(key: &str, layout: ViewLayout) -> serde_json::Value {
    serde_json::json!({ "key": key, "layout": layout })
}

/// What a terminal panel writes into the dock's saved layout: the pane it draws.
///
/// **Layout persists and harnesses do not**, so this is not a promise that the pane comes back — a
/// leaf naming a pane the window no longer holds is dropped. It is what keeps a pane's *place*
/// across the two things that rebuild the tree under it: a rail-mode switch and a project switch.
pub fn pane_payload(pane_id: PaneId) -> serde_json::Value {
    serde_json::json!({ "pane": pane_id.to_string() })
}

/// The same payload read back, or nothing for a payload that names no pane.
pub fn pane_from_payload(payload: &serde_json::Value) -> Option<PanelKind> {
    let pane_id = payload.get("pane")?.as_str()?.parse::<PaneId>().ok()?;
    Some(PanelKind::Terminal(pane_id))
}

/// The same payload read back. A payload with no key names no tab and rebuilds to nothing; one
/// with no layout — or a layout this build does not know — opens in the viewer's default, because
/// which layout a document was left in is not worth losing the document over.
pub fn file_from_payload(payload: &serde_json::Value) -> Option<(PanelKind, ViewLayout)> {
    let key = payload.get("key")?.as_str()?.to_string();
    let layout = payload
        .get("layout")
        .and_then(|value| serde_json::from_value::<ViewLayout>(value.clone()).ok())
        .unwrap_or_default();
    Some((PanelKind::File(key), layout))
}
