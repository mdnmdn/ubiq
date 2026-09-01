//! `AppState`: everything the window knows, and the root of its element tree.
//!
//! It owns the window's own chrome — which rail mode is active, which panels are open, what the
//! chat is showing — and one [`OpenProject`] for every project it holds. No process handle and no
//! pseudo-terminal reaches this far: a pane is an ID, a title, and an emulator reading one end of
//! the bus.
//!
//! **A project's state lives as long as the window holds the project.** Its panes, its tree and
//! its open files are looked up rather than rebuilt, so switching between two projects kills
//! nothing and asks the host for nothing — and switching back finds the terminals still running.
//!
//! Every mutator ends in `cx.notify()`. One that forgets is a panel that stops updating.

use std::collections::HashMap;
use std::time::Duration;

use crate::state::agents::{AgentId, AgentsState, Bucket, InspectorTab, Selection};
use crate::state::{
    ChatState, EditorPaneState, ExplorerState, FileLanguage, LogState, MenuId, RailMode, Toggle,
    WindowRegistry, WorkbenchState, prefs, sample,
};
use crate::theme::{self, ThemeId};
use crate::ui;
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Global, IntoElement, PathPromptOptions, Render,
    ScrollHandle, Subscription, UniformListScrollHandle, Window, WindowBounds, WindowId,
    WindowOptions, point, prelude::*, px, size,
};
use gpui_component::input::{EditorState, InputEvent, InputState, TabSize, TextareaState};
use gpui_component::resizable::ResizableState;
use gpui_terminal::TerminalView;
use ubiq_proto::bus::{self, Client};
use ubiq_proto::files::{FileContents, FileError};
use ubiq_proto::ids::{PaneId, ProjectId, SessionId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::{ProjectSnapshot, Scope};

/// How much of a file the interface asks for. The host has a ceiling of its own and this never
/// widens it; what it does is keep a buffer the user cannot read to the end of off the bus.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// One level of a folder is what an expand asks for. A deeper walk exists for revealing a path,
/// which nothing does yet.
const EXPAND_DEPTH: u8 = 1;

gpui::actions!(ubiq, [SaveFile]);

/// The process-wide switchboard, so every window reaches the same host.
///
/// It is a global for the same reason [`WindowRegistry`] is: `open_project_window` is reached from
/// the project menu as well as from the binary, with no hub in hand. The binary installs it before
/// the first window, and nothing else may.
pub struct BusHub(bus::Hub);

impl Global for BusHub {}

impl BusHub {
    /// Hand the interface its switchboard. Called once, by the binary, before any window exists.
    pub fn install(hub: bus::Hub, cx: &mut App) {
        cx.set_global(Self(hub));
    }

    fn read(cx: &App) -> &bus::Hub {
        &cx.global::<Self>().0
    }
}

/// Single agent harness pane state.
#[derive(Clone)]
pub struct PaneState {
    pub id: PaneId,
    pub harness: String,
    pub rows: u16,
    pub cols: u16,
    pub title: String,
    /// Whether the harness behind the pane is still running. An exited pane keeps its last screen.
    pub running: bool,
}

/// What the window keeps for one pane's terminal: the emulator, and the end of the bus its output
/// arrives on. The sender is dropped when the harness exits, which is how the emulator learns the
/// stream is over.
struct PaneTerminal {
    view: Entity<TerminalView>,
    output: Option<flume::Sender<Vec<u8>>>,
}

/// What the dock's body draws. Which pane, when it is a pane, is the active project's focused one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockTab {
    /// The focused pane's terminal.
    Pane,
    /// The log console, which is a tab in the dock rather than a panel of its own.
    Logs,
}

/// Who is waiting for the keyboard. Focus needs a window, which arrives with the next frame.
enum PendingFocus {
    Pane(PaneId),
    /// The console. A dock showing logs must not leave the keyboard in a terminal nobody can see.
    Logs,
}

/// Layout mode for pane arrangement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutMode {
    /// Single pane fills the dock.
    Single,
    /// Side-by-side vertical split.
    Vsplit,
    /// Top-bottom horizontal split.
    Hsplit,
    /// Grid layout (future).
    Grid,
}

/// What one window holds for one project it has open.
///
/// A window can hold several, and switching between them is a lookup: nothing is rebuilt, nothing
/// is re-read from the host, and the project's terminals keep running behind the one on screen.
pub struct OpenProject {
    /// The panes running in this project, in the order the dock's tabs show them.
    panes: Vec<PaneState>,
    /// Which of them holds the keyboard while this project is the one on screen.
    focused_pane: Option<PaneId>,
    /// Whether this project has ever been given a pane in this window, so becoming active twice
    /// does not spawn twice and a project that had its last pane closed is left alone.
    seeded: bool,
    pub explorer: ExplorerState,
    pub editor: EditorPaneState,
    /// The furniture this project was last left in, kept current so a background project is never
    /// written down wearing the active one's.
    prefs: prefs::ViewPrefs,
    /// Whether the file set has been restored yet — from a parked blob or from the host, first one
    /// wins. Without it a restart's answer would reopen tabs the user has since closed.
    restored: bool,
    /// Folders a blob said were open and that are still out of reach, because a deep folder cannot
    /// be opened before its parents have been listed.
    wanted: Vec<String>,
}

impl OpenProject {
    /// A project this window has just taken, in the furniture it was last left in.
    fn new(prefs: prefs::ViewPrefs) -> Self {
        Self {
            panes: Vec::new(),
            focused_pane: None,
            seeded: false,
            explorer: ExplorerState::empty(),
            editor: EditorPaneState::empty(),
            prefs,
            restored: false,
            wanted: Vec::new(),
        }
    }
}

/// A read the host answered, waiting for the frame that can turn it into a buffer.
///
/// A buffer needs a window and a message does not come with one, so contents queue here exactly as
/// panel sizes and focus already do.
struct FileArrival {
    project: ProjectId,
    path: String,
    contents: FileContents,
}

pub struct AppState {
    /// Which window this state belongs to. It is the key into the process-wide
    /// [`WindowRegistry`], and the only thing that tells two windows apart.
    window_id: WindowId,
    /// One entry per project this window holds. The registry says which of them is on screen.
    projects: HashMap<ProjectId, OpenProject>,
    /// Which project the window was last pointed at, so a switch is noticed exactly once.
    active_seen: Option<ProjectId>,
    /// What a project left behind when it was closed here, so reopening it in the same session
    /// restores its furniture with no round trip and no debounce to race.
    parked: HashMap<ProjectId, prefs::ViewPrefs>,
    /// How the dock arranges its panes.
    layout_mode: LayoutMode,

    /// The window's session — the grouping every workspace it spawns belongs to.
    session: SessionId,
    /// This window's connection to the one host. Nothing else reaches it, and dropping this is
    /// how the host learns the window has gone.
    bus: Client,
    /// One emulator per pane, keyed the way every message is.
    terminals: HashMap<PaneId, PaneTerminal>,
    /// Geometry an emulator measured for itself, on its way back into `PaneState`.
    geometry: flume::Sender<(PaneId, u16, u16)>,
    /// What the dock's tab strip has selected.
    dock_tab: DockTab,
    /// Whoever the keyboard is owed to, once there is a window to give it with.
    pending_focus: Option<PendingFocus>,

    pub workbench: WorkbenchState,
    pub chat: ChatState,
    /// The agents screen: the orchestration graph, what is selected in it, and its tasks.
    pub agents: AgentsState,
    /// What the log console is showing. The records themselves belong to the process-wide sink.
    pub logs: LogState,

    /// Whether this window should take a project from the first catalogue that arrives. True only
    /// for the window the binary opened.
    adopt_on_list: bool,
    /// Whether an `AddProject` this window asked for is still outstanding, so the project it
    /// answers with is opened here rather than merely appearing in every picker.
    adding: bool,
    /// Panel sizes read back from the host, waiting for the frame that can apply them.
    pending_sizes: Option<prefs::ViewPrefs>,
    /// Contents the host sent that still need a window to become buffers. Drained in `render`.
    pending_files: Vec<FileArrival>,
    /// The two resizable groups' own state, owned here rather than left implicit, because a size
    /// that cannot be read back cannot be remembered.
    pub columns: Entity<ResizableState>,
    pub centre: Entity<ResizableState>,

    /// The component library's own state entities. Each open file owns its buffer, so none of
    /// them is the editor's.
    pub chat_input: Entity<TextareaState>,
    /// The inspector's composer on the agents screen. A field of its own rather than the chat's,
    /// because the two are two conversations and a shared draft would leak between them.
    pub agent_input: Entity<TextareaState>,
    pub file_filter: Entity<InputState>,
    /// The titlebar's command field: shortcuts and search, in the middle of the window.
    pub command_input: Entity<InputState>,
    /// The project menu's own search field.
    pub project_search: Entity<InputState>,
    /// The field a picker row becomes while a project is being renamed.
    pub rename_input: Entity<InputState>,
    pub chat_scroll: ScrollHandle,
    pub log_scroll: UniformListScrollHandle,
    /// The console's own focus, so selecting its tab takes the keyboard off the pane behind it.
    log_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl AppState {
    /// Build a window pointed at one project, named by `label`. A second window is the same view
    /// registered under its own letter — see [`open_project_window`], which is the only caller.
    ///
    /// The project is optional: on a first run the catalogue is empty, and a window with nothing
    /// open still has "Add a project…" to offer.
    pub fn for_project(
        project: Option<ProjectId>,
        label: char,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The window takes the project from whatever window held it, so opening one somewhere can
        // leave another showing nothing. That window stays: Ubiq never closes one on the user's
        // behalf.
        let window_id = window.window_handle().window_id();
        cx.global_mut::<WindowRegistry>()
            .register(window_id, label, project);

        let chat_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Reply, or describe the next change\u{2026}")
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });

        let agent_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe a task for this agent\u{2026}")
                .auto_grow(1, 6)
                .submit_on_enter(true)
        });

        let file_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Go to file\u{2026}"));

        let command_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search files, or run a command\u{2026}")
        });

        let project_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a project\u{2026}"));

        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));

        // Owned rather than the implicit keyed state, so a dragged size can be read back and
        // written down.
        let columns = cx.new(|_| ResizableState::default());
        let centre = cx.new(|_| ResizableState::default());

        let mut subscriptions = Vec::new();

        // Every window draws from the same registry, so a project moved in one window redraws the
        // picker in all of them. Another window taking a project is also a change this window has
        // to act on, and it learns about it the same way it learns about its own.
        subscriptions.push(cx.observe_global::<WindowRegistry>(|this, cx| this.sync_projects(cx)));

        subscriptions.push(cx.subscribe_in(
            &chat_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.chat.draft = input.read(cx).value().to_string();
                    cx.notify();
                }
                // The textarea submits on a bare Enter; Shift+Enter still inserts a newline and
                // must not send.
                InputEvent::PressEnter { shift: false, .. } => this.send_chat(window, cx),
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &agent_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.agents.draft = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::PressEnter { shift: false, .. } => this.send_to_agent(window, cx),
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &file_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.workbench.file_filter = input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &project_search,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.workbench.project_filter = input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        ));

        // This window's connection to the host, which is process-wide and already running. The
        // window never starts one: two hosts would race the catalogue and disagree about what
        // exists.
        let bus = BusHub::read(cx).connect();

        let from_host = bus.from_host().clone();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            while let Ok(message) = from_host.recv_async().await {
                if this
                    .update(cx, |this, cx| this.receive(message, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        // The log sink nudges the window when a record arrives. A nudge carries nothing: the
        // console reads the ring itself, so a burst is coalesced into one redraw and a window
        // with the console shut is not woken at all.
        let nudges = ubiq_proto::log::logs().subscribe();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            while nudges.recv_async().await.is_ok() {
                while nudges.try_recv().is_ok() {}
                let shown = this.update(cx, |this, cx| {
                    if this.workbench.show_bottom && this.dock_tab == DockTab::Logs {
                        cx.notify();
                    }
                });
                if shown.is_err() {
                    break;
                }
                // A record emitted while drawing would otherwise redraw the frame that emitted
                // it, forever.
                let settle = cx.background_executor().timer(Duration::from_millis(120));
                settle.await;
            }
        })
        .detach();

        // An emulator measures its own geometry from the bounds it is given; this is how the
        // measurement gets back into the pane it belongs to.
        let (geometry, measurements) = flume::unbounded::<(PaneId, u16, u16)>();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            while let Ok((pane_id, cols, rows)) = measurements.recv_async().await {
                if this
                    .update(cx, |this, cx| this.resize_pane(pane_id, cols, rows, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let mut this = Self {
            window_id,
            projects: HashMap::new(),
            active_seen: None,
            parked: HashMap::new(),
            layout_mode: LayoutMode::Single,
            session: SessionId::generate(),
            bus,
            terminals: HashMap::new(),
            geometry,
            dock_tab: DockTab::Pane,
            pending_focus: None,
            workbench: WorkbenchState::default(),
            chat: sample::chat(),
            agents: sample::agents(),
            logs: LogState::default(),
            adopt_on_list: false,
            adding: false,
            pending_sizes: None,
            pending_files: Vec::new(),
            columns,
            centre,
            chat_input,
            agent_input,
            file_filter,
            command_input,
            project_search,
            rename_input,
            chat_scroll: ScrollHandle::new(),
            log_scroll: UniformListScrollHandle::new(),
            log_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };

        // Nothing about a project is known until the host says so: the interface reads no disk.
        this.bus.send(Message::ListProjects);
        this.bus.send(Message::GetPreferences {
            scope: Scope::Interface,
        });

        // Whatever the registry says this window holds, it now holds — including the pane a
        // project gets when it is first entered. A window opening on nothing spawns nothing.
        this.sync_projects(cx);
        this
    }

    // ── Which projects this window holds ────────────────────────────

    /// Reconcile what the window holds with what the registry says it holds.
    ///
    /// Idempotent, and driven by the registry rather than by each call site, because another
    /// window taking a project is a change this window learns about the same way it learns about
    /// its own.
    fn sync_projects(&mut self, cx: &mut Context<Self>) {
        let (held, active) = match WindowRegistry::read(cx).slot(self.window_id) {
            Some(slot) => (slot.projects.clone(), slot.active_project()),
            None => (Vec::new(), None),
        };

        let gone: Vec<ProjectId> = self
            .projects
            .keys()
            .copied()
            .filter(|id| !held.contains(id))
            .collect();
        for id in gone {
            self.drop_project(id, cx);
        }

        for id in held {
            if self.projects.contains_key(&id) {
                continue;
            }
            // A project closed here earlier in the session left its furniture behind, which is
            // better than the host's copy: it cannot be older, and it costs no round trip.
            let parked = self.parked.remove(&id);
            let restore = parked.clone();
            self.projects
                .insert(id, OpenProject::new(parked.unwrap_or_default()));
            // The tree is the host's, and a project shows nothing until it answers. One level:
            // what is inside a folder is asked for when the folder is opened.
            self.bus.send(Message::ProjectTree {
                project_id: id,
                rel_path: String::new(),
                depth: EXPAND_DEPTH,
            });
            // Where this project's furniture was left across a restart. The answer arrives as
            // `Preferences`, and is ignored if the parked blob got there first.
            self.bus.send(Message::GetPreferences {
                scope: Scope::Project(id),
            });
            if let Some(view) = restore {
                self.restore_files(id, &view, cx);
            }
        }

        if self.active_seen != active {
            self.active_seen = active;
            if let Some(id) = active {
                self.enter_project(id, cx);
            }
        }
        cx.notify();
    }

    /// Everything a project takes with it when it leaves this window: its panes are killed, its
    /// emulators dropped, and what it looked like is written down and parked.
    ///
    /// The panes have to go. No other window can adopt an emulator, and a pane runs in the
    /// project's folder — a harness left behind would be running somewhere nobody is looking.
    fn drop_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        self.remember(project, cx);
        let Some(open) = self.projects.remove(&project) else {
            return;
        };
        self.parked.insert(project, open.prefs);

        for pane in open.panes {
            self.bus.send(Message::CloseWorkspace { pane_id: pane.id });
            self.terminals.remove(&pane.id);
        }
        if self.active_seen == Some(project) {
            self.active_seen = None;
        }
        cx.notify();
    }

    /// A project has become the one this window is pointed at: its furniture reaches the window,
    /// the keyboard goes to its focused pane, and the first time round it is given one.
    fn enter_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let view = open.prefs.clone();
        let focused = open.focused_pane;
        let seeded = open.seeded;
        open.seeded = true;

        // A background project keeps its own furniture, so entering one is where it reaches the
        // window rather than the other way round.
        self.workbench.rail_mode = view.rail_mode;
        self.workbench.show_left = view.show_left;
        self.workbench.show_bottom = view.show_bottom;
        self.workbench.show_right = view.show_right;
        self.pending_sizes = Some(view);

        if self.dock_tab == DockTab::Pane {
            self.pending_focus = focused.map(PendingFocus::Pane);
        }
        if let Some(pane_id) = focused {
            self.bus.send(Message::Focus { pane_id });
        }

        // A window opens a project on one pane, running the session's default agent type. The
        // pane itself arrives with `WorkspaceSpawned`, because only the coordinator knows what
        // started.
        if !seeded {
            self.spawn_pane(None, Vec::new(), cx);
        }
        cx.notify();
    }

    /// What the window holds for the project it is pointed at, if it is pointed at one.
    pub fn open_project(&self, cx: &App) -> Option<&OpenProject> {
        self.projects.get(&self.project(cx)?)
    }

    pub fn open_project_mut(&mut self, cx: &App) -> Option<&mut OpenProject> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id)
    }

    /// The tree the explorer draws, which belongs to the project it is showing.
    pub fn explorer(&self, cx: &App) -> Option<&ExplorerState> {
        self.open_project(cx).map(|open| &open.explorer)
    }

    /// The files open in the project on screen.
    pub fn editor(&self, cx: &App) -> Option<&EditorPaneState> {
        self.open_project(cx).map(|open| &open.editor)
    }

    /// Which project a pane belongs to. A pane is only ever in one, so the first answer is the
    /// answer.
    fn project_of_pane(&self, pane_id: PaneId) -> Option<ProjectId> {
        self.projects
            .iter()
            .find(|(_, open)| open.panes.iter().any(|pane| pane.id == pane_id))
            .map(|(id, _)| *id)
    }

    // ── Panes ───────────────────────────────────────────────────────

    /// The panes the dock draws: the active project's, and none at all without one.
    pub fn panes(&self, cx: &App) -> &[PaneState] {
        self.open_project(cx)
            .map(|open| open.panes.as_slice())
            .unwrap_or(&[])
    }

    pub fn focused_pane(&self, cx: &App) -> Option<&PaneState> {
        let open = self.open_project(cx)?;
        let id = open.focused_pane?;
        open.panes.iter().find(|pane| pane.id == id)
    }

    pub fn focused_pane_index(&self, cx: &App) -> usize {
        self.open_project(cx)
            .and_then(|open| {
                let id = open.focused_pane?;
                open.panes.iter().position(|pane| pane.id == id)
            })
            .unwrap_or(0)
    }

    pub fn layout_mode(&self) -> LayoutMode {
        self.layout_mode
    }

    /// Which tab the dock draws. The console is a tab like a pane, so the dock is the one place
    /// that decides between them.
    ///
    /// With no project there are no panes, so the console is the only thing the dock can be
    /// showing whatever the window last selected.
    pub fn dock_tab(&self, cx: &App) -> DockTab {
        match self.project(cx) {
            Some(_) => self.dock_tab,
            None => DockTab::Logs,
        }
    }

    /// The handle the console's body tracks, so it can hold the keyboard while it is shown.
    pub fn log_focus(&self) -> &FocusHandle {
        &self.log_focus
    }

    /// The emulator a pane is drawn by, for the one module that draws it.
    pub fn terminal(&self, pane_id: PaneId) -> Option<&Entity<TerminalView>> {
        self.terminals.get(&pane_id).map(|pane| &pane.view)
    }

    /// Ask for a workspace. The pane appears when the coordinator answers, so a harness that fails
    /// to start leaves no empty tab behind.
    ///
    /// A pane runs in a project's folder, so a window holding no project asks for nothing: there is
    /// no directory a harness could be started in that the user chose.
    pub fn spawn_pane(
        &mut self,
        agent_type: Option<String>,
        args: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.bus.send(Message::SpawnWorkspace {
            session_id: self.session,
            project_id,
            rel_path: None,
            agent_type,
            args,
        });
    }

    pub fn close_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        self.bus.send(Message::CloseWorkspace { pane_id });
        self.terminals.remove(&pane_id);

        let showing = self.project(cx);
        let Some(project) = self.project_of_pane(pane_id) else {
            cx.notify();
            return;
        };
        // The keyboard only moves for the project on screen: a pane closed in a background
        // project must not take focus off the terminal the user is typing into.
        let on_screen = showing == Some(project) && self.dock_tab == DockTab::Pane;

        let mut next = None;
        if let Some(open) = self.projects.get_mut(&project) {
            open.panes.retain(|pane| pane.id != pane_id);
            if open.focused_pane == Some(pane_id) {
                next = open.panes.first().map(|pane| pane.id);
                open.focused_pane = next;
            }
        }
        // Closing a pane from the strip while the console is the tab shown must not hand the
        // keyboard to a terminal that is off screen.
        if on_screen && let Some(pane_id) = next {
            self.pending_focus = Some(PendingFocus::Pane(pane_id));
        }
        cx.notify();
    }

    pub fn resize_pane(&mut self, pane_id: PaneId, cols: u16, rows: u16, cx: &mut Context<Self>) {
        // A background project's panes are still measured — an emulator that is not drawn keeps
        // the geometry it was last given — so the search is across every project the window holds.
        for open in self.projects.values_mut() {
            if let Some(pane) = open.panes.iter_mut().find(|pane| pane.id == pane_id) {
                if pane.cols == cols && pane.rows == rows {
                    return;
                }
                pane.cols = cols;
                pane.rows = rows;
                cx.notify();
                return;
            }
        }
    }

    /// Give a pane the keyboard. Only the project on screen has panes the user can click, so a
    /// pane belonging to any other is not focusable.
    pub fn focus_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let held = self
            .projects
            .get(&project)
            .is_some_and(|open| open.panes.iter().any(|pane| pane.id == pane_id));
        if !held {
            return;
        }
        if let Some(open) = self.projects.get_mut(&project) {
            open.focused_pane = Some(pane_id);
        }
        self.dock_tab = DockTab::Pane;
        self.pending_focus = Some(PendingFocus::Pane(pane_id));
        self.bus.send(Message::Focus { pane_id });
        cx.notify();
    }

    /// Draw the console in the dock, opening the dock if it is shut. The keyboard comes with it:
    /// a pane the user cannot see must not keep receiving keystrokes.
    pub fn show_logs(&mut self, cx: &mut Context<Self>) {
        self.dock_tab = DockTab::Logs;
        self.workbench.show_bottom = true;
        self.pending_focus = Some(PendingFocus::Logs);
        cx.notify();
    }

    /// The dock's tab strip, in one place: the panes in order, and the console last.
    pub fn select_dock_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.panes(cx).get(index).map(|pane| pane.id) {
            Some(pane_id) => self.focus_pane(pane_id, cx),
            None => self.show_logs(cx),
        }
    }

    /// Everything the coordinator says, in the order it said it.
    fn receive(&mut self, message: Message, cx: &mut Context<Self>) {
        match message {
            Message::WorkspaceSpawned { workspace } => self.open_pane(workspace, cx),

            // Output is handed straight to the pane's emulator. Output for a pane that has gone is
            // dropped: nothing is left to draw it.
            Message::TerminalOutput { pane_id, bytes } => {
                if let Some(terminal) = self.terminals.get(&pane_id)
                    && let Some(output) = &terminal.output
                {
                    let _ = output.send(bytes);
                }
            }

            // An exited harness leaves its pane, showing its last screen. Closing the output
            // stream is what tells the emulator to stop reading.
            Message::PaneExited { pane_id, code } => {
                self.pane_stopped(pane_id);
                if let Some(terminal) = self.terminals.get_mut(&pane_id) {
                    terminal.output = None;
                }
                tracing::info!("pane {pane_id} exited with {code}");
                cx.notify();
            }

            Message::PaneError { pane_id, error } => {
                self.pane_stopped(pane_id);
                tracing::error!("pane {pane_id}: {error}");
                cx.notify();
            }

            // ── the project family ──────────────────────────────────
            // Every window is sent the same snapshots, so each replaces by id and the projection
            // is idempotent by construction.
            Message::ProjectList { projects } => {
                cx.global_mut::<WindowRegistry>().replace_all(projects);
                self.adopt_if_owed(cx);
                // A catalogue that no longer names a project this window held takes it away, so
                // what the window holds is reconciled before anything is drawn from it.
                self.sync_projects(cx);
            }

            Message::ProjectAdded { project } => {
                let id = project.record.id;
                cx.global_mut::<WindowRegistry>().apply(project);
                // Whoever asked for it is the window that opens it.
                if self.adding {
                    self.adding = false;
                    self.take_project(id, cx);
                }
                cx.notify();
            }

            Message::ProjectChanged { project } => {
                cx.global_mut::<WindowRegistry>().apply(project);
                cx.notify();
            }

            Message::ProjectForgotten { project_id } => {
                cx.global_mut::<WindowRegistry>().forget(project_id);
                self.sync_projects(cx);
            }

            Message::ProjectError { project_id, error } => {
                tracing::error!("project {project_id:?}: {error}");
                self.workbench.project_error = Some(error);
                self.adding = false;
                cx.notify();
            }

            Message::Preferences { scope, value } => self.apply_preferences(scope, value, cx),

            // ── the file family ─────────────────────────────────────
            // Every answer names its project and its path, so one that arrives after the user has
            // switched projects lands where it belongs rather than on screen.
            Message::ProjectTreeListing {
                project_id,
                rel_path,
                listings,
            } => {
                let Some(open) = self.projects.get_mut(&project_id) else {
                    return;
                };
                open.explorer.set_loading(&rel_path, false);
                for listing in listings {
                    open.explorer.merge(listing);
                }
                // A listing can put a remembered folder within reach, which is what makes
                // restoring a deep one terminate: each answer either resolves one or drops it.
                self.reach_wanted(project_id, cx);
                cx.notify();
            }

            Message::ProjectFileContents {
                project_id,
                rel_path,
                contents,
            } => {
                self.pending_files.push(FileArrival {
                    project: project_id,
                    path: rel_path,
                    contents,
                });
                cx.notify();
            }

            Message::ProjectFileWritten {
                project_id,
                rel_path,
                version,
            } => {
                // What the buffer holds now, not what was written: anything typed while the save
                // was in flight is still unsaved, and the tab has to keep saying so.
                let current = self
                    .projects
                    .get(&project_id)
                    .and_then(|open| open.editor.open.iter().find(|f| f.path == rel_path))
                    .and_then(|file| file.buffer())
                    .map(|buffer| buffer.read(cx).value().to_string())
                    .unwrap_or_default();

                if let Some(open) = self.projects.get_mut(&project_id)
                    && let Some(file) = open.editor.find_mut(&rel_path)
                {
                    file.saved(version, &current);
                }
                cx.notify();
            }

            Message::ProjectFileError {
                project_id,
                rel_path,
                error,
            } => self.file_failed(project_id, rel_path, error, cx),

            // What the host is. The status bar says so when the root is not the usual one.
            Message::HostInfo {
                config_root,
                is_default,
            } => {
                self.workbench.config_root = Some(config_root);
                self.workbench.config_root_is_default = is_default;
                cx.notify();
            }

            // The rest are the window's own words, coming back the wrong way.
            other => tracing::warn!("the window was sent a message only it may send: {other:?}"),
        }
    }

    /// One path in one project failed.
    ///
    /// A tab waiting for bytes says why instead of sitting empty; a folder waiting for a listing
    /// stops spinning. A folder or a file that has gone is the cue to look at the project's own
    /// health again — the worker that answered does not know the catalogue and cannot say.
    fn file_failed(
        &mut self,
        project: ProjectId,
        rel_path: String,
        error: FileError,
        cx: &mut Context<Self>,
    ) {
        let reason = describe(&error);
        tracing::warn!("{project} {rel_path}: {reason}");

        if let Some(open) = self.projects.get_mut(&project) {
            open.explorer.set_loading(&rel_path, false);
            open.wanted.retain(|wanted| wanted != &rel_path);
            if let Some(file) = open.editor.find_mut(&rel_path) {
                match file.is_loading() {
                    // The read never landed, so the tab has nothing but the reason.
                    true => file.set_failed(reason.clone()),
                    // A write failed against a buffer the user still has: it is untouched, and
                    // still dirty.
                    false => file.save_failed(reason.clone()),
                }
            }
        }

        if matches!(error, FileError::Missing | FileError::Denied(_)) {
            self.bus.send(Message::RefreshProject {
                project_id: project,
            });
        }
        cx.notify();
    }

    /// A pane's harness has stopped, wherever the pane is. An exited pane keeps its last screen,
    /// so the only thing that changes is what the tab's dot reports.
    fn pane_stopped(&mut self, pane_id: PaneId) {
        for open in self.projects.values_mut() {
            if let Some(pane) = open.panes.iter_mut().find(|pane| pane.id == pane_id) {
                pane.running = false;
                return;
            }
        }
    }

    /// Draw a workspace the coordinator started: a tab, and an emulator on the pane's stream.
    ///
    /// The workspace names its project, which is what makes an answer that arrives after the user
    /// has switched projects land in the right place rather than on screen.
    fn open_pane(&mut self, workspace: WorkspaceInfo, cx: &mut Context<Self>) {
        let pane_id = workspace.id;
        let project = workspace.project_id;
        let title = agent_title(&workspace.agent_type);

        // A pane for a project this window no longer holds has nowhere to be drawn, and a harness
        // nobody can see is a leak: it is closed rather than kept.
        if !self.projects.contains_key(&project) {
            tracing::info!("pane {pane_id} arrived for a project this window no longer holds");
            self.bus.send(Message::CloseWorkspace { pane_id });
            return;
        }
        let showing = self.project(cx) == Some(project);

        let (output, reader) = bus::pane_output();
        let writer = self.bus.input(pane_id);
        let config = ui::terminal::config(workspace.cols, workspace.rows);

        let to_host = self.bus.sender();
        let geometry = self.geometry.clone();
        let view = cx.new(|cx| {
            TerminalView::new(writer, reader, config, cx).with_resize_callback(move |cols, rows| {
                let (cols, rows) = (cols as u16, rows as u16);
                to_host.send(Message::TerminalResize {
                    pane_id,
                    cols,
                    rows,
                });
                let _ = geometry.send((pane_id, cols, rows));
            })
        });

        if let Some(open) = self.projects.get_mut(&project) {
            open.panes.push(PaneState {
                id: pane_id,
                harness: workspace.agent_type,
                rows: workspace.rows,
                cols: workspace.cols,
                title,
                running: workspace.running,
            });
            // A pane in a background project becomes that project's focused one only if it had
            // none: the keyboard belongs to whatever is on screen.
            if showing || open.focused_pane.is_none() {
                open.focused_pane = Some(pane_id);
            }
        }
        self.terminals.insert(
            pane_id,
            PaneTerminal {
                view,
                output: Some(output),
            },
        );

        if showing {
            self.dock_tab = DockTab::Pane;
            self.pending_focus = Some(PendingFocus::Pane(pane_id));
            self.bus.send(Message::Focus { pane_id });
        }
        cx.notify();
    }

    /// Give the keyboard to whoever asked for it. Focus needs a window, so it waits for one.
    fn take_focus(&mut self, window: &mut Window, cx: &mut App) {
        match self.pending_focus.take() {
            Some(PendingFocus::Pane(pane_id)) => {
                if let Some(terminal) = self.terminals.get(&pane_id) {
                    terminal
                        .view
                        .read(cx)
                        .focus_handle()
                        .clone()
                        .focus(window, cx);
                }
            }
            Some(PendingFocus::Logs) => self.log_focus.clone().focus(window, cx),
            None => {}
        }
    }

    // ── Workbench chrome ────────────────────────────────────────────

    pub fn set_rail_mode(&mut self, mode: RailMode, cx: &mut Context<Self>) {
        self.workbench.rail_mode = mode;
        self.workbench.open_menu = None;
        self.remember_view(cx);
        cx.notify();
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let next = self.workbench.theme_id.toggled();
        self.workbench.theme_id = next;
        theme::set_mode(next, cx);
        // The palette belongs to the interface, not to any one project.
        self.remember_interface();
        // The emulator holds its own copy of the palette, so the switch has to reach it.
        for terminal in self.terminals.values() {
            terminal.view.update(cx, |view, cx| {
                let (cols, rows) = view.dimensions();
                view.update_config(ui::terminal::config(cols as u16, rows as u16), cx);
            });
        }
        cx.notify();
    }

    pub fn open_menu(&mut self, menu: MenuId, cx: &mut Context<Self>) {
        self.workbench.open_menu = Some(menu);
        cx.notify();
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.pending_close = None;
        cx.notify();
    }

    // ── The log console ────────────────────────────────────────

    pub fn pick_log_subsystem(&mut self, index: usize, cx: &mut Context<Self>) {
        self.logs.pick_subsystem(index);
        self.close_menu(cx);
    }

    pub fn pick_log_level(&mut self, index: usize, cx: &mut Context<Self>) {
        self.logs.pick_level(index);
        self.close_menu(cx);
    }

    pub fn toggle_log_follow(&mut self, cx: &mut Context<Self>) {
        self.logs.follow = !self.logs.follow;
        cx.notify();
    }

    /// Empty the sink. The ring is the whole process's, so this clears every window's console.
    pub fn clear_logs(&mut self, cx: &mut Context<Self>) {
        ubiq_proto::log::logs().clear();
        cx.notify();
    }

    // ── Projects ────────────────────────────────────────────────────

    /// This window's letter — `A`, `B`, `C`… — as the picker prints it beside every project the
    /// window holds.
    pub fn window_label(&self, cx: &App) -> char {
        WindowRegistry::read(cx)
            .slot(self.window_id)
            .map(|slot| slot.label)
            .unwrap_or('?')
    }

    /// The project this window is pointed at, if it has one. A window holds nothing only while the
    /// catalogue is empty, or in the frame before the host has answered.
    pub fn project(&self, cx: &App) -> Option<ProjectId> {
        WindowRegistry::read(cx)
            .slot(self.window_id)
            .and_then(|slot| slot.active_project())
    }

    /// Everything known about the project this window is pointed at.
    pub fn project_snapshot<'a>(&self, cx: &'a App) -> Option<&'a ProjectSnapshot> {
        let id = self.project(cx)?;
        WindowRegistry::read(cx).project(id)
    }

    pub fn project_name(&self, cx: &App) -> String {
        self.project_snapshot(cx)
            .map(|p| p.record.name.clone())
            .unwrap_or_else(|| "No project".to_string())
    }

    /// The colour the whole window is identified by.
    ///
    /// One place decides what a window with no project looks like, rather than four call sites
    /// each falling back to swatch zero and claiming to be a project that is not there.
    pub fn project_tint(&self, cx: &App) -> gpui::Rgba {
        match self.project_snapshot(cx) {
            Some(project) => theme::project_colour(project.record.colour),
            None => theme::border(),
        }
    }

    /// The swatch the interface would give a new project: the one fewest projects are using, so
    /// the palette spreads before it repeats.
    pub fn next_colour(&self, cx: &App) -> usize {
        let registry = WindowRegistry::read(cx);
        let count = theme::project_colour_count();
        let mut used = vec![0usize; count];
        for project in registry.all() {
            used[project.record.colour % count] += 1;
        }
        used.iter()
            .enumerate()
            .min_by_key(|(index, taken)| (**taken, *index))
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    /// The picker's three groups: open here, open in another window, only remembered.
    pub fn project_groups(&self, cx: &App) -> crate::state::ProjectGroups {
        WindowRegistry::read(cx).groups(self.window_id, &self.workbench.project_filter)
    }

    /// Point this window at a project it already holds.
    pub fn activate_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let id = self.window_id;
        cx.global_mut::<WindowRegistry>().activate(id, project);
        self.bus.send(Message::OpenedProject {
            project_id: project,
        });
        // The window now points somewhere else: its tree, its tabs and its terminals are the new
        // project's from here.
        self.sync_projects(cx);
        self.close_menu(cx);
    }

    /// Open a project in this window, taking it from whichever window held it. This is both "open
    /// one from history" and the move-here action on a project open elsewhere: a project is open in
    /// one window at a time, so the two are the same operation.
    pub fn take_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let id = self.window_id;
        // A project the catalogue does not hold is not opened, and the host is not told that a
        // window pointed at one.
        if !cx.global_mut::<WindowRegistry>().open_in(id, project) {
            self.close_menu(cx);
            return;
        }
        // The host decides what opening a project means, and stamps it.
        self.bus.send(Message::OpenedProject {
            project_id: project,
        });
        // Everything the project brings with it — its furniture, its pane, its tree — is the
        // reconciliation's, including the `GetPreferences` that asks where it was left.
        self.sync_projects(cx);
        self.close_menu(cx);
    }

    /// Close a project in this window. One with terminals still running asks first: the menu row
    /// turns into a confirmation rather than taking the click. Closing the last one leaves the
    /// window open on nothing, with the picker to offer.
    pub fn close_project(&mut self, project: ProjectId, force: bool, cx: &mut Context<Self>) {
        // This window's own count, not the catalogue's: closing a project here kills the panes
        // *this* window is running in it, and says so about those.
        let panes = self
            .projects
            .get(&project)
            .map_or(0, |open| open.panes.len());
        if panes > 0 && !force {
            self.workbench.pending_close = Some(project);
            cx.notify();
            return;
        }

        self.workbench.pending_close = None;
        let id = self.window_id;
        cx.global_mut::<WindowRegistry>().close(id, project);
        self.sync_projects(cx);
    }

    pub fn cancel_close(&mut self, cx: &mut Context<Self>) {
        self.workbench.pending_close = None;
        cx.notify();
    }

    // ── Asking the host ─────────────────────────────────────────────

    /// Rename or recolour. The host answers, and every window redraws.
    pub fn update_project(
        &mut self,
        project: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name,
            colour,
        });
        self.workbench.row_action = None;
        cx.notify();
    }

    /// Drop the record and everything Ubiq remembers about it. Nothing inside the project's own
    /// folder is touched, which is why the word is "Forget".
    pub fn forget_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        self.bus.send(Message::ForgetProject {
            project_id: project,
        });
        self.workbench.row_action = None;
        cx.notify();
    }

    /// Look at a project's folder again — the action a row marked missing offers.
    pub fn refresh_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        self.bus.send(Message::RefreshProject {
            project_id: project,
        });
        cx.notify();
    }

    /// Re-point a record at a folder that moved, keeping its id, colour and history.
    pub fn locate_project(&mut self, project: ProjectId, path: String, cx: &mut Context<Self>) {
        self.bus.send(Message::LocateProject {
            project_id: project,
            path,
        });
        cx.notify();
    }

    /// Take a folder into the catalogue.
    pub fn add_project(&mut self, path: String, cx: &mut Context<Self>) {
        let colour = self.next_colour(cx);
        self.bus.send(Message::AddProject {
            path,
            name: None,
            colour: Some(colour),
        });
        cx.notify();
    }

    /// Ask the operating system for a folder, then add it or re-point the project being located.
    ///
    /// The dialog is the platform's own — the one users already know how to type a path into, and
    /// the one that reaches their bookmarks and network volumes. It browses the *interface's*
    /// filesystem, which is the host's only while the two share a machine; see `D32`.
    pub fn choose_folder(&mut self, locating: Option<ProjectId>, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(match locating {
                Some(_) => "Locate".into(),
                None => "Add".into(),
            }),
        });

        cx.spawn(async move |this, cx| {
            // Three outcomes, and only one of them is a path: the channel can close with the
            // dialog, the platform can refuse to open one, and the user can cancel.
            let answer = match chosen.await {
                Ok(answer) => answer,
                Err(_) => return,
            };

            this.update(cx, |this, cx| match answer {
                Ok(Some(paths)) => {
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    let path = path.to_string_lossy().into_owned();
                    match locating {
                        Some(project) => this.locate_project(project, path, cx),
                        None => this.add_project(path, cx),
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    this.workbench.project_error =
                        Some(format!("could not open a chooser: {error}"));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Expand one picker row into a rename, a recolour or a Forget confirmation.
    pub fn set_row_action(
        &mut self,
        action: Option<(ProjectId, crate::state::RowAction)>,
        cx: &mut Context<Self>,
    ) {
        self.workbench.row_action = action;
        cx.notify();
    }

    /// Commit whatever the rename field holds. An empty name is ignored rather than applied: a
    /// project with no name is not something the picker can draw.
    pub fn commit_rename(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let name = self.rename_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.set_row_action(None, cx);
            return;
        }
        self.update_project(project, Some(name), None, cx);
    }

    pub fn dismiss_project_error(&mut self, cx: &mut Context<Self>) {
        self.workbench.project_error = None;
        cx.notify();
    }

    // ── Boot, and what is remembered ────────────────────────────────

    /// Take a project on the first catalogue that arrives.
    ///
    /// Only the window the binary opened is owed this. One code path serves both the ordinary boot
    /// and the empty catalogue, and the cost is a single frame of the picker.
    fn adopt_if_owed(&mut self, cx: &mut Context<Self>) {
        if !self.adopt_on_list {
            return;
        }
        self.adopt_on_list = false;

        if self.project(cx).is_some() {
            return;
        }
        // Nothing to adopt is the first run, and the window stays open on the picker.
        if let Some(id) = WindowRegistry::read(cx).most_recent() {
            self.take_project(id, cx);
        }
    }

    /// Apply what the host had stored for a scope.
    fn apply_preferences(&mut self, scope: Scope, value: Option<String>, cx: &mut Context<Self>) {
        let Some(blob) = value else { return };

        match scope {
            Scope::Interface => {
                if let Some(prefs) = prefs::decode::<prefs::InterfacePrefs>(&blob)
                    && prefs.theme != self.workbench.theme_id
                {
                    self.workbench.theme_id = prefs.theme;
                    theme::set_mode(prefs.theme, cx);
                }
            }
            Scope::Project(id) => {
                let Some(view) = prefs::decode::<prefs::ViewPrefs>(&blob) else {
                    return;
                };
                // An answer for a project this window holds without showing still has to reach
                // it: a project's furniture is its own, whether or not anyone is looking at it.
                let showing = self.project(cx) == Some(id);
                let Some(open) = self.projects.get_mut(&id) else {
                    return;
                };
                open.prefs = view.clone();
                let restore = (!open.restored).then(|| view.clone());
                if showing {
                    self.workbench.rail_mode = view.rail_mode;
                    self.workbench.show_left = view.show_left;
                    self.workbench.show_bottom = view.show_bottom;
                    self.workbench.show_right = view.show_right;
                    self.pending_sizes = Some(view);
                }
                // A project closed and reopened in this session restored from the parked blob
                // already, and reopening the tabs the user has since closed would be worse than
                // useless.
                if let Some(view) = restore {
                    self.restore_files(id, &view, cx);
                }
            }
        }
        cx.notify();
    }

    /// Write down what this window looks like now, for the project on screen. Debounced by the
    /// host, so this may be called as freely as a drag fires.
    pub fn remember_view(&mut self, cx: &App) {
        let Some(id) = self.project(cx) else { return };
        self.remember(id, cx);
    }

    /// Write down what one project this window holds was left looking like.
    ///
    /// The furniture is read off the window only for the project on screen. A background project
    /// keeps the furniture its own blob already carries, which is what stops a project being
    /// written down wearing whatever the user happened to be looking at.
    fn remember(&mut self, project: ProjectId, cx: &App) {
        // The file set is the project's own whether or not anyone is looking at it, so it is read
        // back from the project every time.
        if let Some(open) = self.projects.get_mut(&project) {
            open.prefs.open_files = open.editor.paths();
            open.prefs.active_file = open.editor.active_path();
            open.prefs.expanded = open.explorer.expanded();
            open.prefs.selected = open.explorer.selected.clone();
        }

        if self.project(cx) == Some(project) {
            let sizes = self.panel_sizes(cx);
            let (rail_mode, show_left, show_bottom, show_right) = (
                self.workbench.rail_mode,
                self.workbench.show_left,
                self.workbench.show_bottom,
                self.workbench.show_right,
            );
            if let Some(open) = self.projects.get_mut(&project) {
                open.prefs.rail_mode = rail_mode;
                open.prefs.show_left = show_left;
                open.prefs.show_bottom = show_bottom;
                open.prefs.show_right = show_right;
                open.prefs.explorer_width = sizes.0;
                open.prefs.chat_width = sizes.1;
                open.prefs.dock_height = sizes.2;
            }
        }

        let Some(open) = self.projects.get(&project) else {
            return;
        };
        self.bus.send(Message::SetPreferences {
            scope: Scope::Project(project),
            value: prefs::encode(&open.prefs),
        });
    }

    /// The three panel sizes, as they currently stand: explorer, chat, and the dock's height.
    ///
    /// The columns group is explorer, centre, chat; the centre group is editor, dock. A hidden
    /// panel keeps its size, which is what makes a toggle non-destructive.
    fn panel_sizes(&self, cx: &App) -> (Option<f32>, Option<f32>, Option<f32>) {
        let columns = self.columns.read(cx).sizes();
        let centre = self.centre.read(cx).sizes();
        (
            columns.first().map(|size| f32::from(*size)),
            columns.get(2).map(|size| f32::from(*size)),
            centre.get(1).map(|size| f32::from(*size)),
        )
    }

    /// Put back the panel sizes the host handed us.
    ///
    /// On the frame after they arrive, because a resizable group only has its panels once it has
    /// been laid out — before that there is nothing to resize.
    fn apply_pending_sizes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.pending_sizes.take() else {
            return;
        };
        let laid_out = !self.columns.read(cx).sizes().is_empty();
        if !laid_out {
            // Not yet. Keep them for the next frame rather than dropping them on the floor.
            self.pending_sizes = Some(view);
            return;
        }

        // The columns group is explorer, centre, chat; the centre group is editor, dock.
        if let Some(width) = view.explorer_width {
            self.columns
                .update(cx, |state, cx| state.resize_panel(0, px(width), window, cx));
        }
        if let Some(width) = view.chat_width {
            self.columns
                .update(cx, |state, cx| state.resize_panel(2, px(width), window, cx));
        }
        if let Some(height) = view.dock_height
            && !self.centre.read(cx).sizes().is_empty()
        {
            self.centre.update(cx, |state, cx| {
                state.resize_panel(1, px(height), window, cx)
            });
        }
    }

    /// Write down what belongs to the interface as a whole.
    pub fn remember_interface(&mut self) {
        let prefs = prefs::InterfacePrefs {
            schema: prefs::SCHEMA,
            theme: self.workbench.theme_id,
        };
        self.bus.send(Message::SetPreferences {
            scope: Scope::Interface,
            value: prefs::encode(&prefs),
        });
    }

    // ── Explorer ────────────────────────────────────────────────────

    /// Open or shut a folder, asking the host what is inside it the first time.
    ///
    /// Which folders are open is persisted state, so a toggle is written down as well as drawn.
    pub fn toggle_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        // A folder opened for the first time knows nothing about what is inside it, and says so
        // on the row until the host answers.
        if open.explorer.toggle(&path) == Toggle::Listing {
            open.explorer.set_loading(&path, true);
            self.bus.send(Message::ProjectTree {
                project_id: project,
                rel_path: path,
                depth: EXPAND_DEPTH,
            });
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// Select a row, and open it if it is a file.
    ///
    /// The tab appears on the click rather than on the answer: a click with no visible effect
    /// invites a second one, and a read that fails needs somewhere to say so.
    pub fn select_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.explorer.selected = Some(path.clone());

        let fresh = open.editor.index_of(&path).is_none();
        let index = open.editor.open_pending(&path);
        open.editor.active = index;

        if fresh {
            self.bus.send(Message::ReadProjectFile {
                project_id: project,
                rel_path: path,
                max_bytes: Some(MAX_FILE_BYTES),
            });
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// Ask for every file a project's blob said was open, and open the folders it said were.
    ///
    /// The tree is restored a level at a time: a folder cannot be opened before its parent has
    /// been listed, so what is out of reach waits in `wanted` for the next listing.
    fn restore_files(
        &mut self,
        project: ProjectId,
        view: &prefs::ViewPrefs,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        if open.restored {
            return;
        }
        open.restored = true;

        for path in &view.open_files {
            open.editor.open_pending(path);
        }
        if let Some(active) = &view.active_file
            && let Some(at) = open.editor.index_of(active)
        {
            open.editor.active = at;
        }
        open.explorer.selected = view.selected.clone();
        open.wanted = view.expanded.clone();

        let files = open.editor.paths();
        for rel_path in files {
            self.bus.send(Message::ReadProjectFile {
                project_id: project,
                rel_path,
                max_bytes: Some(MAX_FILE_BYTES),
            });
        }
        self.reach_wanted(project, cx);
        cx.notify();
    }

    /// Open whichever remembered folders have become reachable, and ask for what they hold.
    fn reach_wanted(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let mut wanted = std::mem::take(&mut open.wanted);
        let ask = open.explorer.reopen(&mut wanted);
        open.wanted = wanted;
        for rel_path in &ask {
            open.explorer.set_loading(rel_path, true);
        }

        for rel_path in ask {
            self.bus.send(Message::ProjectTree {
                project_id: project,
                rel_path,
                depth: EXPAND_DEPTH,
            });
        }
        cx.notify();
    }

    // ── Editor ──────────────────────────────────────────────────────

    /// Bring a tab forward. Each file owns its buffer, so nothing is copied and nothing is lost.
    pub fn activate_editor_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        if index >= open.editor.open.len() {
            return;
        }
        open.editor.active = index;
        open.editor.pending_tab_close = None;
        if let Some(path) = open.editor.active_path() {
            open.explorer.selected = Some(path);
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// Close a tab. One holding unsaved changes asks first: the × becomes a confirmation rather
    /// than taking the click, on the pattern a project with running terminals already uses.
    ///
    /// Clicking the tab itself is how the question is answered no, because bringing a tab forward
    /// is already the one thing that clears it.
    pub fn close_editor_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.open.get(index) else {
            return;
        };
        let path = file.path.clone();
        if file.dirty() && open.editor.pending_tab_close.as_deref() != Some(path.as_str()) {
            open.editor.pending_tab_close = Some(path);
            cx.notify();
            return;
        }

        open.editor.close(index);
        self.remember(project, cx);
        cx.notify();
    }

    /// The caret's one-based position, as the status bar reports it. Absent when nothing is open,
    /// so the status bar omits the segment rather than reporting a caret in a buffer nobody is
    /// looking at.
    pub fn cursor_line_column(&self, cx: &App) -> Option<(u32, u32)> {
        let buffer = self.editor(cx)?.active_file()?.buffer()?;
        let position = buffer.read(cx).cursor_position();
        Some((position.line + 1, position.character + 1))
    }

    /// Write the active file back.
    ///
    /// Nothing happens with no project, no active file, bytes that never arrived, or a read the
    /// host cut short: writing a prefix back would shorten the file.
    pub fn save_active_file(&mut self, _: &SaveFile, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(file) = open.editor.active_file() else {
            return;
        };
        if !file.savable() {
            return;
        }
        let Some(buffer) = file.buffer() else {
            return;
        };
        let text = buffer.read(cx).value().to_string();
        let (rel_path, expected) = (file.path.clone(), file.version());

        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&rel_path)
        {
            file.mark_saving(text.clone());
        }
        self.bus.send(Message::WriteProjectFile {
            project_id: project,
            rel_path,
            bytes: text.into_bytes(),
            expected,
        });
        cx.notify();
    }

    /// Turn everything that arrived since the last frame into a buffer.
    ///
    /// In `render`, because a buffer needs a window and a message does not come with one.
    fn attach_arrived_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for arrival in std::mem::take(&mut self.pending_files) {
            self.attach_file(arrival, window, cx);
        }
    }

    fn attach_file(&mut self, arrival: FileArrival, window: &mut Window, cx: &mut Context<Self>) {
        let FileArrival {
            project,
            path,
            contents,
        } = arrival;

        // A tab that already holds bytes is never overwritten: there is no reload action, and
        // whatever has been typed into it would go with them.
        let wanted = self
            .projects
            .get(&project)
            .and_then(|open| open.editor.open.iter().find(|file| file.path == path))
            .is_some_and(|file| file.is_loading());
        if !wanted {
            return;
        }

        if contents.is_binary {
            if let Some(open) = self.projects.get_mut(&project)
                && let Some(file) = open.editor.find_mut(&path)
            {
                file.set_binary();
            }
            cx.notify();
            return;
        }

        // Bytes are the host's, and decoding is the interface's: a file that is not valid UTF-8 is
        // still text somebody wants to read.
        let text = String::from_utf8_lossy(&contents.bytes).into_owned();
        let language = FileLanguage::of(&path);
        let buffer = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(ui::editor::highlighter_language(language))
                .line_number(true)
                .folding(true)
                .show_whitespaces(false)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .default_value(text.clone())
        });

        // Dirty is kept current by the buffer's own change event, because comparing every frame
        // costs the file's length times the tabs open times the frame rate.
        let watched = path.clone();
        let change = cx.subscribe_in(
            &buffer,
            window,
            move |this, buffer, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let typed = buffer.read(cx).value().to_string();
                if let Some(open) = this.projects.get_mut(&project)
                    && let Some(file) = open.editor.find_mut(&watched)
                {
                    file.refresh_dirty(&typed);
                }
                cx.notify();
            },
        );

        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&path)
        {
            file.attach(buffer, text, contents.truncated, contents.version, change);
        }
        cx.notify();
    }

    // ── The agents screen ───────────────────────────────────────────

    /// Point the screen at a session or at one agent. Both are selections, and everything else on
    /// the screen — the graph's session, the inspector, the tasks drawer — is a function of this
    /// one field.
    pub fn select_in_graph(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.agents.selection = Some(selection);
        cx.notify();
    }

    pub fn toggle_agent_bucket(&mut self, bucket: Bucket, cx: &mut Context<Self>) {
        self.agents.toggle_bucket(bucket);
        cx.notify();
    }

    pub fn zoom_graph(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.agents.zoom_by(delta);
        cx.notify();
    }

    pub fn reset_graph_zoom(&mut self, cx: &mut Context<Self>) {
        self.agents.zoom = 1.0;
        cx.notify();
    }

    pub fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        self.agents.show_inspector = !self.agents.show_inspector;
        cx.notify();
    }

    pub fn toggle_tasks_drawer(&mut self, cx: &mut Context<Self>) {
        self.agents.tasks_open = !self.agents.tasks_open;
        cx.notify();
    }

    /// Select one agent and put the inspector on its thread — what the `chat` affordance on a card
    /// does, and the one place the screen changes two things at once, because a card asking for a
    /// conversation with the panel shut has asked for nothing.
    pub fn open_agent_chat(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.agents.selection = Some(Selection::Agent(agent));
        self.agents.tab = InspectorTab::Chat;
        self.agents.show_inspector = true;
        cx.notify();
    }

    pub fn select_inspector_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.agents.tab = if index == 0 {
            InspectorTab::Chat
        } else {
            InspectorTab::Tasks
        };
        cx.notify();
    }

    /// Pick a card up. Selecting it too, because what is being moved is what the user is looking
    /// at, and a drag that left the inspector on something else would be reporting on the wrong
    /// agent.
    pub fn start_agent_carry(&mut self, agent: AgentId, grab: (f32, f32), cx: &mut Context<Self>) {
        self.agents.start_carry(agent, grab);
        self.agents.selection = Some(Selection::Agent(agent));
        cx.notify();
    }

    /// Move the carried card, and lay a grain of sand where the pointer passed.
    ///
    /// The trail is skipped when the system asks for reduced motion — it is the only motion on
    /// this screen, and the card still follows the pointer without it.
    pub fn carry_agent_to(&mut self, at: (f32, f32), pointer: (f32, f32), cx: &mut Context<Self>) {
        if cx.reduce_motion() {
            if let Some(carry) = self.agents.carry
                && let Some(agent) = self.agents.agent_mut(carry.agent)
            {
                agent.at = at;
            }
        } else {
            self.agents.carry_to(at, pointer, std::time::Instant::now());
        }
        cx.notify();
    }

    /// Put the carried card down, and move it into whatever container it landed in.
    pub fn end_agent_carry(&mut self, cx: &mut Context<Self>) {
        self.agents.end_carry();
        cx.notify();
    }

    /// What the composer sends, into the selected agent's thread.
    pub fn send_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.agents.send() {
            return;
        }
        let input = self.agent_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Age the drag trail by one frame, and answer whether it still owes the window another.
    ///
    /// A drag that ended outside the graph — on the inspector, or off the window — never reaches
    /// the canvas's drop handler, so a carry with no live drag behind it is put down here. That is
    /// what stops a card sticking to the pointer after the button came up somewhere else.
    fn settle_graph(&mut self, cx: &mut Context<Self>) {
        if self.agents.carry.is_some() && !cx.has_active_drag() {
            self.agents.end_carry();
        }
        self.agents.settle_sand(std::time::Instant::now());
    }

    // ── Chat ────────────────────────────────────────────────────────

    pub fn send_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat.draft.trim().is_empty() {
            return;
        }
        self.chat.send();

        let input = self.chat_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.chat_scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn select_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.chat.chats.len() {
            self.chat.active = index;
            self.chat_scroll.scroll_to_bottom();
            cx.notify();
        }
    }

    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.chat.new_chat();
        cx.notify();
    }

    pub fn toggle_tool(&mut self, message: usize, block: usize, cx: &mut Context<Self>) {
        self.chat.toggle_tool(message, block);
        cx.notify();
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.take_focus(window, cx);
        self.apply_pending_sizes(window, cx);
        self.attach_arrived_files(window, cx);
        self.settle_graph(cx);
        ui::shell::render(self, window, cx)
    }
}

/// What the interface says about a path the host refused.
///
/// Each arm is a different thing for the user to do about it, which is why the contract carries an
/// enum rather than a sentence.
fn describe(error: &FileError) -> String {
    match error {
        FileError::Refused(reason) => format!("refused: {reason}"),
        FileError::Missing => "no longer there".to_string(),
        FileError::WrongKind => "not a file Ubiq can open".to_string(),
        FileError::Denied(reason) => format!("cannot be read: {reason}"),
        FileError::Conflict => "changed on disk since it was read".to_string(),
        FileError::Failed(reason) => reason.clone(),
    }
}

/// The keys the workbench answers to.
///
/// Called once by the binary, which owns the application's own actions and leaves the window's to
/// the window. The context is what keeps the binding from meaning "save" outside a workbench.
pub fn install_key_bindings(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-s", SaveFile, Some("Workbench")),
        gpui::KeyBinding::new("ctrl-s", SaveFile, Some("Workbench")),
    ]);
}

/// What a pane calls itself before its harness says otherwise: the program, without its path.
fn agent_title(agent_type: &str) -> String {
    agent_type
        .rsplit('/')
        .next()
        .unwrap_or(agent_type)
        .to_string()
}

/// Re-exported so `main.rs` can name the palette it boots with.
pub fn boot_theme() -> ThemeId {
    ThemeId::Dark
}

/// Open a window on a project.
///
/// This is the only place a window is created, so `main.rs` and the project menu's "open in a new
/// window" reach the same code. Each window owns its own `AppState`; they share nothing but the
/// palette and the window registry, both of which are process-wide.
///
/// The project comes with it: a project is open in one window at a time, so the new window takes it
/// from whichever window held it, and that window is left showing nothing.
pub fn open_project_window(project: Option<ProjectId>, cx: &mut App) {
    open_window(project, false, cx)
}

/// The first window, which takes a project from the first catalogue that arrives.
///
/// The binary cannot name one: it has not asked the host what exists yet, and the interface may
/// not look for itself.
pub fn open_first_window(cx: &mut App) {
    open_window(None, true, cx)
}

fn open_window(project: Option<ProjectId>, adopt: bool, cx: &mut App) {
    WindowRegistry::install(cx);
    // The letter is allocated before the window exists, because the title carries it. Nothing can
    // register in between — `open_window` builds the `AppState`, which is what registers.
    let label = WindowRegistry::read(cx).next_label();

    // Step successive windows down and across, so a new one does not land exactly on its parent.
    let offset = (cx.windows().len() as f32) * 28.0;
    let mut bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    bounds.origin += point(px(offset), px(offset));

    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some(format!("Ubiq {label} - Agent Harness Multiplexer").into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| {
                let mut state = AppState::for_project(project, label, window, cx);
                state.adopt_on_list = adopt;
                state
            });
            cx.new(|cx| gpui_component::Root::new(view, window, cx).bg(crate::theme::app_bg()))
        },
    );

    tracing::info!("window {label} opened on project {project:?}");

    if let Ok(handle) = opened {
        handle
            .update(cx, |_, window, _| window.activate_window())
            .ok();
    }
}

/// Bring a window to the front. The picker's rows for projects open elsewhere use it: clicking one
/// is how the user moves between windows.
pub fn focus_window(id: WindowId, cx: &mut App) {
    for handle in cx.windows() {
        if handle.window_id() == id {
            handle
                .update(cx, |_, window, _| window.activate_window())
                .ok();
        }
    }
}

/// A window has gone. Its slot goes with it, and everything it held returns to history.
pub fn window_closed(id: WindowId, cx: &mut App) {
    if cx.has_global::<WindowRegistry>() {
        cx.global_mut::<WindowRegistry>().unregister(id);
    }
}
