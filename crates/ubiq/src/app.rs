//! `AppState`: everything the window knows, and the root of its element tree.
//!
//! It owns the panes, the focused pane and the layout mode, plus the workbench's own state — which
//! rail mode is active, which panels are open, what the explorer, the editor and the chat are
//! showing. No process handle and no pseudo-terminal reaches this far: a pane is an ID, a title,
//! and an emulator reading one end of the bus.
//!
//! Every mutator ends in `cx.notify()`. One that forgets is a panel that stops updating.

use std::collections::HashMap;
use std::time::Duration;

use crate::state::{
    ChatState, EditorPaneState, ExplorerState, LogState, MenuId, RailMode, WindowRegistry,
    WorkbenchState, prefs, sample,
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
use ubiq_proto::ids::{PaneId, ProjectId, SessionId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::{ProjectSnapshot, Scope};

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

/// What the dock's body draws. Which pane, when it is a pane, is [`AppState::focused_pane`].
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

pub struct AppState {
    /// Which window this state belongs to. It is the key into the process-wide
    /// [`WindowRegistry`], and the only thing that tells two windows apart.
    window_id: WindowId,
    /// Active panes, in the order the dock's tabs show them.
    panes: Vec<PaneState>,
    /// Currently focused pane.
    focused_pane: Option<PaneId>,
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
    pub explorer: ExplorerState,
    pub editor: EditorPaneState,
    pub chat: ChatState,
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
    /// The two resizable groups' own state, owned here rather than left implicit, because a size
    /// that cannot be read back cannot be remembered.
    pub columns: Entity<ResizableState>,
    pub centre: Entity<ResizableState>,

    /// The component library's own state entities.
    pub editor_state: Entity<EditorState>,
    pub chat_input: Entity<TextareaState>,
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
        // leave another with nothing to show.
        let window_id = window.window_handle().window_id();
        let emptied = cx
            .global_mut::<WindowRegistry>()
            .register(window_id, label, project);
        close_windows(emptied, cx);

        let editor = sample::editor();
        let source = editor
            .active_file()
            .map(|f| f.source.clone())
            .unwrap_or_default();
        let language = editor
            .active_file()
            .map(|f| ui::editor::highlighter_language(f.language))
            .unwrap_or(gpui_component::highlighter::Language::Plain);

        let editor_state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .folding(true)
                .show_whitespaces(false)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .default_value(source)
        });

        let chat_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Reply, or describe the next change\u{2026}")
                .auto_grow(1, 8)
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
        // picker in all of them.
        subscriptions.push(cx.observe_global::<WindowRegistry>(|_, cx| cx.notify()));

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
            &file_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.explorer.filter = input.read(cx).value().to_string();
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
            panes: Vec::new(),
            focused_pane: None,
            layout_mode: LayoutMode::Single,
            session: SessionId::generate(),
            bus,
            terminals: HashMap::new(),
            geometry,
            dock_tab: DockTab::Pane,
            pending_focus: None,
            workbench: sample::workbench(),
            explorer: sample::explorer(),
            editor,
            chat: sample::chat(),
            logs: LogState::default(),
            adopt_on_list: false,
            adding: false,
            pending_sizes: None,
            columns,
            centre,
            editor_state,
            chat_input,
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
        if let Some(project) = project {
            this.bus.send(Message::GetPreferences {
                scope: Scope::Project(project),
            });
        }

        // A window opens on one pane, running the session's default agent type. The pane itself
        // arrives with `WorkspaceSpawned`, because only the coordinator knows what started.
        this.spawn_pane(None, Vec::new(), cx);
        this
    }

    // ── Panes ───────────────────────────────────────────────────────

    pub fn panes(&self) -> &[PaneState] {
        &self.panes
    }

    pub fn focused_pane(&self) -> Option<&PaneState> {
        self.focused_pane
            .and_then(|id| self.panes.iter().find(|p| p.id == id))
    }

    pub fn focused_pane_index(&self) -> usize {
        self.focused_pane
            .and_then(|id| self.panes.iter().position(|p| p.id == id))
            .unwrap_or(0)
    }

    pub fn layout_mode(&self) -> LayoutMode {
        self.layout_mode
    }

    /// Which tab the dock draws. The console is a tab like a pane, so the dock is the one place
    /// that decides between them.
    pub fn dock_tab(&self) -> DockTab {
        self.dock_tab
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
    pub fn spawn_pane(
        &mut self,
        agent_type: Option<String>,
        args: Vec<String>,
        _cx: &mut Context<Self>,
    ) {
        let project_id = self.project(_cx);
        self.bus.send(Message::SpawnWorkspace {
            session_id: self.session,
            project_id,
            agent_type,
            args,
            folder: None,
        });
    }

    pub fn close_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        self.bus.send(Message::CloseWorkspace { pane_id });
        self.panes.retain(|p| p.id != pane_id);
        self.terminals.remove(&pane_id);
        if self.focused_pane == Some(pane_id) {
            let next = self.panes.first().map(|p| p.id);
            self.focused_pane = next;
            // Closing a pane from the strip while the console is the tab shown must not hand the
            // keyboard to a terminal that is off screen.
            if self.dock_tab == DockTab::Pane {
                self.pending_focus = next.map(PendingFocus::Pane);
            }
        }
        cx.notify();
    }

    pub fn resize_pane(&mut self, pane_id: PaneId, cols: u16, rows: u16, cx: &mut Context<Self>) {
        if let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) {
            if pane.cols == cols && pane.rows == rows {
                return;
            }
            pane.cols = cols;
            pane.rows = rows;
            cx.notify();
        }
    }

    pub fn focus_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        if self.panes.iter().any(|p| p.id == pane_id) {
            self.focused_pane = Some(pane_id);
            self.dock_tab = DockTab::Pane;
            self.pending_focus = Some(PendingFocus::Pane(pane_id));
            self.bus.send(Message::Focus { pane_id });
            cx.notify();
        }
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
        match self.panes.get(index).map(|pane| pane.id) {
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
                if let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) {
                    pane.running = false;
                }
                if let Some(terminal) = self.terminals.get_mut(&pane_id) {
                    terminal.output = None;
                }
                tracing::info!("pane {pane_id} exited with {code}");
                cx.notify();
            }

            Message::PaneError { pane_id, error } => {
                if let Some(pane) = self.panes.iter_mut().find(|p| p.id == pane_id) {
                    pane.running = false;
                }
                tracing::error!("pane {pane_id}: {error}");
                cx.notify();
            }

            // ── the project family ──────────────────────────────────
            // Every window is sent the same snapshots, so each replaces by id and the projection
            // is idempotent by construction.
            Message::ProjectList { projects } => {
                cx.global_mut::<WindowRegistry>().replace_all(projects);
                self.adopt_if_owed(cx);
                cx.notify();
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
                let emptied = cx.global_mut::<WindowRegistry>().forget(project_id);
                close_windows(emptied, cx);
                cx.notify();
            }

            Message::ProjectError { project_id, error } => {
                tracing::error!("project {project_id:?}: {error}");
                self.workbench.project_error = Some(error);
                self.adding = false;
                cx.notify();
            }

            Message::Preferences { scope, value } => self.apply_preferences(scope, value, cx),

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

    /// Draw a workspace the coordinator started: a tab, and an emulator on the pane's stream.
    fn open_pane(&mut self, workspace: WorkspaceInfo, cx: &mut Context<Self>) {
        let pane_id = workspace.id;
        let title = agent_title(&workspace.agent_type);

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

        self.panes.push(PaneState {
            id: pane_id,
            harness: workspace.agent_type,
            rows: workspace.rows,
            cols: workspace.cols,
            title,
            running: workspace.running,
        });
        self.terminals.insert(
            pane_id,
            PaneTerminal {
                view,
                output: Some(output),
            },
        );

        self.focused_pane = Some(pane_id);
        self.dock_tab = DockTab::Pane;
        self.pending_focus = Some(PendingFocus::Pane(pane_id));
        self.bus.send(Message::Focus { pane_id });
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
        self.close_menu(cx);
    }

    /// Open a project in this window, taking it from whichever window held it. This is both "open
    /// one from history" and the move-here action on a project open elsewhere: a project is open in
    /// one window at a time, so the two are the same operation.
    pub fn take_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let id = self.window_id;
        let emptied = cx.global_mut::<WindowRegistry>().open_in(id, project);
        close_windows(emptied, cx);
        // The host decides what opening a project means, and stamps it.
        self.bus.send(Message::OpenedProject {
            project_id: project,
        });
        // Where this project's furniture was left. The answer arrives as `Preferences`.
        self.bus.send(Message::GetPreferences {
            scope: Scope::Project(project),
        });
        self.close_menu(cx);
    }

    /// Close a project in this window. One with terminals still running asks first: the menu row
    /// turns into a confirmation rather than taking the click. Closing the last one closes the
    /// window, because a window with nothing open has nothing to show.
    pub fn close_project(&mut self, project: ProjectId, force: bool, cx: &mut Context<Self>) {
        let panes = WindowRegistry::read(cx)
            .project(project)
            .map_or(0, |p| p.open_panes);
        if panes > 0 && !force {
            self.workbench.pending_close = Some(project);
            cx.notify();
            return;
        }

        self.workbench.pending_close = None;
        let id = self.window_id;
        let emptied = cx.global_mut::<WindowRegistry>().close(id, project);
        close_windows(emptied, cx);
        cx.notify();
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
                // The answer may arrive after the window has moved on to another project.
                if self.project(cx) != Some(id) {
                    return;
                }
                let Some(view) = prefs::decode::<prefs::ViewPrefs>(&blob) else {
                    return;
                };
                self.workbench.rail_mode = view.rail_mode;
                self.workbench.show_left = view.show_left;
                self.workbench.show_bottom = view.show_bottom;
                self.workbench.show_right = view.show_right;
                self.pending_sizes = Some(view);
            }
        }
        cx.notify();
    }

    /// Write down what this window looks like now. Debounced by the host, so this may be called as
    /// freely as a drag fires.
    pub fn remember_view(&mut self, cx: &App) {
        let Some(id) = self.project(cx) else { return };
        let sizes = self.panel_sizes(cx);
        let view = prefs::ViewPrefs {
            schema: prefs::SCHEMA,
            rail_mode: self.workbench.rail_mode,
            show_left: self.workbench.show_left,
            show_bottom: self.workbench.show_bottom,
            show_right: self.workbench.show_right,
            explorer_width: sizes.0,
            chat_width: sizes.1,
            dock_height: sizes.2,
        };
        self.bus.send(Message::SetPreferences {
            scope: Scope::Project(id),
            value: prefs::encode(&view),
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

    pub fn toggle_folder(&mut self, path: String, cx: &mut Context<Self>) {
        self.explorer.toggle(&path);
        cx.notify();
    }

    pub fn select_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.explorer.selected = Some(path.clone());
        if let Some(index) = self.editor.open.iter().position(|f| f.path == path) {
            self.activate_editor_tab(index, window, cx);
        } else {
            cx.notify();
        }
    }

    // ── Editor ──────────────────────────────────────────────────────

    /// Switch tabs, writing the outgoing buffer back first so an edit survives the move.
    pub fn activate_editor_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.editor.open.len() || index == self.editor.active {
            return;
        }

        let current = self.editor_state.read(cx).value().to_string();
        if let Some(file) = self.editor.active_file_mut() {
            file.dirty = file.dirty || file.source != current;
            file.source = current;
        }

        self.editor.active = index;
        let Some(file) = self.editor.active_file() else {
            return;
        };
        let (source, language) = (file.source.clone(), file.language);
        self.explorer.selected = Some(file.path.clone());

        self.editor_state.update(cx, |state, cx| {
            state.set_highlighter(ui::editor::highlighter_language(language), cx);
            state.set_value(source, window, cx);
        });
        cx.notify();
    }

    pub fn close_editor_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_active = index == self.editor.active;
        self.editor.close(index);
        if was_active && let Some(file) = self.editor.active_file() {
            let (source, language) = (file.source.clone(), file.language);
            self.editor_state.update(cx, |state, cx| {
                state.set_highlighter(ui::editor::highlighter_language(language), cx);
                state.set_value(source, window, cx);
            });
        }
        cx.notify();
    }

    /// The caret's one-based position, as the status bar reports it.
    pub fn cursor_line_column(&self, cx: &App) -> (u32, u32) {
        let position = self.editor_state.read(cx).cursor_position();
        (position.line + 1, position.character + 1)
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
        ui::shell::render(self, window, cx)
    }
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
/// from whichever window held it, and that window closes if it held nothing else.
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

/// Close the windows the registry has just emptied.
///
/// Deferred, because the caller is usually inside one of these windows' own event handlers, and a
/// window may not be updated while it is being updated.
fn close_windows(ids: Vec<WindowId>, cx: &mut App) {
    if ids.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for handle in cx.windows() {
            if ids.contains(&handle.window_id()) {
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            }
        }
    });
}

/// A window has gone. Its slot goes with it, and everything it held returns to history.
pub fn window_closed(id: WindowId, cx: &mut App) {
    if cx.has_global::<WindowRegistry>() {
        cx.global_mut::<WindowRegistry>().unregister(id);
    }
}
