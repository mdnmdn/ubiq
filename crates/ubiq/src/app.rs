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

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::state::agents::{GraphView, Held, InspectorTab, Selection};
use crate::state::board::{BoardState, Field};
use crate::state::diagrams::{self, DiagramAnswer, DiagramImage, DiagramPalette};
use crate::state::dock::Visibility;
use crate::state::editor::{Subject, ViewLayout, from_tab_key, tab_key};
use crate::state::file_picker::{
    Commit, FilePickerState, PickKind, PickerCount, PickerKey, PickerOwner, PickerView, Pressed,
};
use crate::state::sink::{
    ProjectNav, SettingsMenu, SettingsNav, SinkDoc, SinkModal, SinkSection, SinkState,
};
use crate::state::viewport::{Content, Viewport};
use crate::state::work::WorkProjection;
use crate::state::{
    ChatState, EditorPaneState, ExplorerState, FileLanguage, LogState, MenuId, OpenFile, PanelKind,
    RailMode, Region, Toggle, WindowRegistry, WorkbenchState, prefs, sample,
};
use crate::theme::{self, ThemeId};
use crate::ui;
use crate::ui::dock::{self as dock, WorkbenchPanel};
use gpui::{
    App, Bounds, Context, Entity, Focusable, Global, Image, ImageFormat, IntoElement,
    PathPromptOptions, Pixels, Render, ScrollHandle, Subscription, UniformListScrollHandle,
    WeakEntity, Window, WindowBounds, WindowId, WindowOptions, point, prelude::*, px, size,
};
use gpui_component::dock::{DockArea, DockEvent, PanelId};
use gpui_component::input::{EditorState, InputEvent, InputState, TabSize, TextareaState};
use gpui_terminal::TerminalView;
use ubiq_proto::bus::{self, Client};
use ubiq_proto::files::{DiffBase, FileContents, FileError};
use ubiq_proto::ids::{PaneId, ProjectId, SessionId, StepId, TaskId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::{ProjectSnapshot, Scope};
use ubiq_proto::work::{AgentId, Bucket, Priority, Shape, Status};

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

/// A drawn diagram, ready to go on screen.
///
/// The picture is built when the render lands rather than on every frame: `Image` identifies
/// itself by hashing its bytes, and a diagram rebuilt each frame would hash and copy them each
/// frame.
#[derive(Clone)]
pub struct DiagramPicture {
    pub image: Arc<Image>,
    /// The picture's own size, read out of the SVG's viewBox. Drawing at it is what keeps a diagram
    /// sharp instead of stretched to whatever box it landed in.
    pub width: f32,
    pub height: f32,
}

/// Where one diagram has got to.
///
/// A source that would not draw is remembered as `Failed` rather than forgotten. Nothing is cached
/// for it — no picture was made — but the window has to stop asking, and the viewer has to have
/// something to say beside the source.
#[derive(Clone)]
pub enum DiagramEntry {
    Pending,
    Ready(DiagramPicture),
    Failed(String),
}

/// The picture a rendered diagram becomes. Always SVG: that is the only thing merman emits.
fn diagram_picture(image: DiagramImage) -> DiagramPicture {
    DiagramPicture {
        width: image.width,
        height: image.height,
        image: Arc::new(Image::from_bytes(ImageFormat::Svg, image.bytes)),
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

/// An edit to the dock that is waiting for a window.
///
/// A panel is added and removed through the dock, which needs a `Window`, and a message does not
/// come with one — so these queue exactly as the pending focus and the arrived files already do,
/// and are drained in `render`.
enum PanelEdit {
    Open(PanelKind),
    Close(PanelKind),
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
    /// The host's work for this project, as this window last heard it. Empty rather than absent
    /// until the `ListWork` is answered, so a project whose work has never arrived draws as empty
    /// rather than as a project with no work.
    pub work: WorkProjection,
    /// The graph's view of that work: what is selected in it, which states it is showing, and where
    /// its cards sit. Per project, because a selection and an arrangement are about one project's
    /// agents and switching away must not lose either.
    pub graph: GraphView,
    /// The board's view of the same work: what is filtered, which task is open, which columns and
    /// cards are shut.
    pub board: BoardState,
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
            work: WorkProjection::empty(),
            graph: GraphView::default(),
            board: BoardState::default(),
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
    /// This state, weakly. The dock it owns holds panels that read it back, and a strong handle
    /// either way would be a cycle the window never gets out of.
    this: WeakEntity<Self>,
    /// One entry per project this window holds. The registry says which of them is on screen.
    projects: HashMap<ProjectId, OpenProject>,
    /// Which project the window was last pointed at, so a switch is noticed exactly once.
    active_seen: Option<ProjectId>,
    /// What a project left behind when it was closed here, so reopening it in the same session
    /// restores its furniture with no round trip and no debounce to race.
    parked: HashMap<ProjectId, prefs::ViewPrefs>,

    /// The window's session — the grouping every workspace it spawns belongs to.
    session: SessionId,
    /// This window's connection to the one host. Nothing else reaches it, and dropping this is
    /// how the host learns the window has gone.
    bus: Client,
    /// One emulator per pane, keyed the way every message is.
    terminals: HashMap<PaneId, PaneTerminal>,
    /// Geometry an emulator measured for itself, on its way back into `PaneState`.
    geometry: flume::Sender<(PaneId, u16, u16)>,
    /// Whoever the keyboard is owed to, once there is a window to give it with.
    pending_focus: Option<PaneId>,

    /// The window's whole arrangement: a tree of tabbed groups the user rearranges by dragging.
    /// Every area of the workbench is a panel in it, and nothing outside it decides where anything
    /// sits.
    dock: Entity<DockArea>,
    /// One panel per kind, so a panel is looked up rather than rebuilt. A terminal's key carries
    /// its pane id, which is what makes "the panel for this pane" a map read.
    panels: HashMap<PanelKind, Entity<WorkbenchPanel>>,
    /// Panels waiting for the frame that can put them in the dock or take them out of it.
    pending_panels: Vec<PanelEdit>,
    /// A saved arrangement waiting for the same frame. Restoring one needs a window, and it
    /// arrives from the host on a message.
    pending_layout: Option<serde_json::Value>,

    pub workbench: WorkbenchState,
    pub chat: ChatState,
    /// The kitchen sink's own state: which page is open, and what its controls hold. It belongs to
    /// the window rather than to a project, because the sink has no project behind it.
    pub sink: SinkState,
    /// What the log console is showing. The records themselves belong to the process-wide sink.
    pub logs: LogState,
    /// The file picker, when one is up. It belongs to the window rather than to the screen that
    /// raised it — exactly one may be up, whichever screen asked — and the request it carries says
    /// who is owed the answer.
    pub file_picker: Option<FilePickerState>,

    /// Whether this window should take a project from the first catalogue that arrives. True only
    /// for the window the binary opened.
    adopt_on_list: bool,
    /// Whether an `AddProject` this window asked for is still outstanding, so the project it
    /// answers with is opened here rather than merely appearing in every picker.
    adding: bool,
    /// Contents the host sent that still need a window to become buffers. Drained in `render`.
    pending_files: Vec<FileArrival>,

    /// Every diagram this window has drawn, by content key — the cache's memory tier. **Behind a
    /// cell because a viewer meets it mid-frame**: the element tree is built from `&AppState`, and
    /// a diagram is found to be missing exactly there.
    diagrams: RefCell<HashMap<String, DiagramEntry>>,
    /// The diagrams a frame turned out to need: the source and the palette. A render cannot be
    /// started from inside one — `AppState` is mid-update, and the work belongs on another thread
    /// anyway — so they queue here and are handed to the background executor once the frame is
    /// built.
    diagram_asks: RefCell<Vec<(String, DiagramPalette)>>,

    /// The camera on each diagram and scene this window is showing, keyed by the tab (or the
    /// sink document) that holds it. Behind a cell because a viewer meets it mid-frame the same
    /// way it meets the diagram cache: the element tree is built from `&AppState`.
    viewports: RefCell<HashMap<String, Viewport>>,
    /// The picture the pointer is dragging, and where the last move was. Separate from the camera
    /// so a drag that is interrupted leaves the pan where the last move put it.
    viewport_drag: RefCell<Option<(String, gpui::Point<gpui::Pixels>)>>,

    /// The component library's own state entities. Each open file owns its buffer, so none of
    /// them is the editor's.
    pub chat_input: Entity<TextareaState>,
    /// The inspector's composer on the agents screen. A field of its own rather than the chat's,
    /// because the two are two conversations and a shared draft would leak between them.
    pub agent_input: Entity<TextareaState>,
    pub file_filter: Entity<InputState>,
    /// The file picker's own field. Separate from the explorer's because the two are up at once
    /// and one state drawn twice is one field in two places.
    pub picker_filter: Entity<InputState>,
    /// The board's one field: what filters the cards, and what names the next one.
    pub task_filter: Entity<InputState>,
    /// The task panel's four fields. They belong to the window because there is one of each per
    /// window, and what is typed into them belongs to the project — see `BoardState::form`.
    pub task_title_input: Entity<InputState>,
    pub task_description_input: Entity<TextareaState>,
    pub step_title_input: Entity<InputState>,
    pub new_step_input: Entity<InputState>,
    /// The titlebar's command field: shortcuts and search, in the middle of the window.
    pub command_input: Entity<InputState>,
    /// The project menu's own search field.
    pub project_search: Entity<InputState>,
    /// The field a picker row becomes while a project is being renamed.
    pub rename_input: Entity<InputState>,
    /// One buffer per kitchen-sink fixture, by the document's key. The sink's documents are the
    /// window's own rather than a project's files — nothing reads them from disk and nothing writes
    /// them back — so their buffers sit here beside the window's other component-library state
    /// instead of on an `OpenFile`.
    sink_buffers: HashMap<&'static str, Entity<EditorState>>,
    /// The style reference's two fields, and the one its form modal carries. Three rather than one,
    /// because the modal can be raised while the fields page is on screen and one state drawn twice
    /// is one field in two places.
    pub sink_input: Entity<InputState>,
    pub sink_textarea: Entity<TextareaState>,
    pub sink_modal_input: Entity<InputState>,
    /// The settings pages' fields. Separate from the style reference's, because a fixture's
    /// value is the thing being looked at and one state drawn on two pages is one field in two
    /// places if both were ever on screen at once — they are not, but the split matches every
    /// other pair of fields in the window.
    pub sink_search: Entity<InputState>,
    pub sink_harness_name: Entity<InputState>,
    pub sink_harness_exec: Entity<InputState>,
    pub sink_harness_prompt: Entity<TextareaState>,
    pub sink_harness_env: Entity<InputState>,
    pub sink_project_name: Entity<InputState>,
    pub sink_project_about: Entity<TextareaState>,
    pub sink_project_hex: Entity<InputState>,
    pub chat_scroll: ScrollHandle,
    /// The file picker's rows. It is what a keyboard cursor moved past the last drawn row is
    /// brought back into view with.
    pub picker_scroll: ScrollHandle,
    pub log_scroll: UniformListScrollHandle,
    /// Which task the panel's fields were last filled from, so a selection change refills them
    /// exactly once. Writing into the component library's state needs a window and a message does
    /// not come with one, which is why this is drained in `render` beside the arrived files.
    form_filled: Option<TaskId>,
    /// A project switch owes the window's own fields the entered project's text.
    refill_fields: bool,
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

        let picker_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by name or path\u{2026}"));

        let task_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter tasks\u{2026}"));

        let task_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Task title"));

        // The one field in the window that must not submit on Enter: a newline is a paragraph
        // break in Markdown, so Save is a button rather than a key.
        let task_description_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe the task in Markdown\u{2026}")
                .auto_grow(3, 14)
        });

        // One field for renaming whichever sub-task is open, because only one ever is.
        let step_title_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Rename this sub-task"));

        let new_step_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add a sub-task\u{2026}"));

        let command_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Search files, or run a command\u{2026}")
        });

        let project_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Find a project\u{2026}"));

        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));

        // The kitchen sink's fixtures become buffers here, where there is a window to build one
        // with. They are constants, so this is the whole of their lifecycle: nothing arrives late,
        // nothing is saved, and a change event would have nothing to compare against.
        let sink_buffers: HashMap<&'static str, Entity<EditorState>> = crate::state::sink::docs()
            .iter()
            .map(|doc| {
                let buffer = cx.new(|cx| {
                    EditorState::new(window, cx)
                        .language(ui::editor::highlighter_language(doc.language()))
                        .line_number(true)
                        .folding(true)
                        .show_whitespaces(false)
                        .tab_size(TabSize {
                            tab_size: 2,
                            ..Default::default()
                        })
                        .default_value(doc.source)
                });
                (doc.key, buffer)
            })
            .collect();

        let sink_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("a field, with nothing behind it")
                .default_value("crates/ubiq/src/theme.rs")
        });

        let sink_textarea = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("a textarea, with nothing behind it\u{2026}")
                .auto_grow(2, 6)
        });

        let sink_modal_input = cx.new(|cx| InputState::new(window, cx).placeholder("Session name"));

        let sink_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search settings\u{2026}"));
        let sink_harness_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Display name")
                .default_value("Claude Code — work")
        });
        let sink_harness_exec = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/absolute/path")
                .default_value("/opt/homebrew/bin/claude")
        });
        let sink_harness_prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("House rules, not a task\u{2026}")
                .auto_grow(3, 8)
                .default_value(
                    "You work in a Tauri + React codebase. Prefer small diffs, keep the existing \
                     file conventions, and never touch src-tauri without saying so first.",
                )
        });
        let sink_harness_env = cx.new(|cx| InputState::new(window, cx).placeholder("KEY=value"));
        let sink_project_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Project name")
                .default_value(crate::state::sink::PROJECT_NAME)
        });
        let sink_project_about = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Two lines about this codebase\u{2026}")
                .auto_grow(3, 6)
                .default_value(crate::state::sink::PROJECT_ABOUT)
        });
        let sink_project_hex = {
            let swatch = theme::project_colour(crate::state::sink::PROJECT_COLOUR);
            let hex = crate::state::sink::hex_string(crate::state::sink::rgb_from_channels(
                swatch.r, swatch.g, swatch.b,
            ));
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("#RRGGBB")
                    .default_value(hex)
            })
        };

        // The window's arrangement. It is built before anything is put in it, because a panel is
        // an entity that has to be handed somewhere the moment it exists.
        let dock = cx.new(|cx| {
            DockArea::new("ubiq-workbench", Some(dock::LAYOUT_VERSION), window, cx)
                .with_renderer(crate::ui::dock::skin::Skin::new())
        });

        // Every panel reads this window's state, and `cx.entity()` is that handle before the state
        // it names exists — the slot is reserved for the duration of the constructor.
        let app = cx.weak_entity();
        let mut panels: HashMap<PanelKind, Entity<WorkbenchPanel>> = HashMap::new();
        {
            let mut build = |kind: PanelKind, cx: &mut App| {
                panels
                    .entry(kind.clone())
                    .or_insert_with(|| WorkbenchPanel::new(kind, app.clone(), cx))
                    .clone()
            };
            dock::default_layout(&dock, &mut build, window, cx);
        }

        let mut subscriptions = Vec::new();

        // The dock is where the arrangement lives, so it is the dock that says it changed. Every
        // edit fires this — a drag, a split, a divider let go of — and the host debounces the
        // write, so it may fire as freely as it likes. The placement policy is applied first: a
        // panel that landed somewhere its kind forbids is put back before the layout is written
        // down, so what is remembered is what is on screen.
        subscriptions.push(cx.subscribe_in(
            &dock,
            window,
            |this, _, event: &DockEvent, window, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.enforce_placement(window, cx);
                    this.remember_view(cx);
                    cx.notify();
                }
            },
        ));

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
                    let draft = input.read(cx).value().to_string();
                    // A window with no project has no composer on screen to have typed into, so
                    // there is nothing to mirror it onto.
                    if let Some(graph) = this.graph_mut(cx) {
                        graph.draft = draft;
                    }
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
            &picker_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = input.read(cx).value().to_string();
                    if let Some(picker) = this.file_picker.as_mut() {
                        picker.set_filter(filter);
                    }
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &task_filter,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.filter = filter;
                    }
                    cx.notify();
                }
            },
        ));

        // The panel's four fields mirror into the project's own form and commit on Enter or on the
        // control beside them — never on losing focus, which is the project picker's rename rule
        // and for its reason: a blur fires before the click that caused it, so a field that
        // committed on blur could not be cancelled by the button next to it. A commit is an act
        // rather than a keystroke, which is also what keeps the host's writes un-debounced.
        subscriptions.push(cx.subscribe_in(
            &task_title_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.title = text;
                    }
                }
                InputEvent::PressEnter { shift: false, .. } => this.commit_task_title(window, cx),
                _ => {}
            },
        ));

        // No `PressEnter` arm, and no `Blur` arm: Enter is a newline here, and a blur would commit
        // on the very click that asks for the preview.
        subscriptions.push(cx.subscribe_in(
            &task_description_input,
            window,
            |this, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.description = text;
                    }
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &step_title_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.step_title = text;
                    }
                }
                InputEvent::PressEnter { shift: false, .. } => this.commit_step_title(window, cx),
                _ => {}
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &new_step_input,
            window,
            |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    if let Some(board) = this.board_mut(cx) {
                        board.form.new_step = text;
                    }
                }
                InputEvent::PressEnter { shift: false, .. } => this.add_task_step(window, cx),
                _ => {}
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

        subscriptions.push(cx.subscribe_in(
            &sink_project_hex,
            window,
            |this, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_sink_project_hex(cx);
                }
            },
        ));

        // A field's underline is drawn by the parent, so a focus change has to redraw the window
        // rather than only the library widget.
        for handle in [
            sink_input.read(cx).focus_handle(cx),
            sink_textarea.read(cx).focus_handle(cx),
            sink_modal_input.read(cx).focus_handle(cx),
            sink_search.read(cx).focus_handle(cx),
            sink_harness_name.read(cx).focus_handle(cx),
            sink_harness_exec.read(cx).focus_handle(cx),
            sink_harness_prompt.read(cx).focus_handle(cx),
            sink_harness_env.read(cx).focus_handle(cx),
            sink_project_name.read(cx).focus_handle(cx),
            sink_project_about.read(cx).focus_handle(cx),
            sink_project_hex.read(cx).focus_handle(cx),
        ] {
            subscriptions.push(cx.on_focus(&handle, window, |_, _, cx| cx.notify()));
            subscriptions.push(cx.on_focus_out(&handle, window, |_, _, _, cx| cx.notify()));
        }

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
                // The console is a panel of its own now, so there is no tab to test: a window
                // whose console is a background tab redraws it and lays it out anyway, and one
                // whose bottom region is closed draws nothing for it.
                let shown = this.update(cx, |_, cx| cx.notify());
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
            this: cx.weak_entity(),
            projects: HashMap::new(),
            active_seen: None,
            parked: HashMap::new(),
            session: SessionId::generate(),
            bus,
            terminals: HashMap::new(),
            geometry,
            pending_focus: None,
            dock,
            panels,
            pending_panels: Vec::new(),
            pending_layout: None,
            workbench: WorkbenchState::default(),
            chat: sample::chat(),
            sink: SinkState::default(),
            file_picker: None,
            logs: LogState::default(),
            adopt_on_list: false,
            adding: false,
            pending_files: Vec::new(),
            diagrams: RefCell::new(HashMap::new()),
            diagram_asks: RefCell::new(Vec::new()),
            viewports: RefCell::new(HashMap::new()),
            viewport_drag: RefCell::new(None),
            chat_input,
            agent_input,
            file_filter,
            picker_filter,
            task_filter,
            task_title_input,
            task_description_input,
            step_title_input,
            new_step_input,
            command_input,
            project_search,
            rename_input,
            sink_buffers,
            sink_input,
            sink_textarea,
            sink_modal_input,
            sink_search,
            sink_harness_name,
            sink_harness_exec,
            sink_harness_prompt,
            sink_harness_env,
            sink_project_name,
            sink_project_about,
            sink_project_hex,
            chat_scroll: ScrollHandle::new(),
            picker_scroll: ScrollHandle::new(),
            log_scroll: UniformListScrollHandle::new(),
            form_filled: None,
            refill_fields: false,
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
            // The work is the host's as well, and the graph and the board draw nothing until it
            // answers. Once per newly held project: the reply is the whole of it.
            self.bus.send(Message::ListWork { project_id: id });
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
            // The pane's panel goes with it. It is queued rather than taken out here, because a
            // panel leaves the dock through a `Window` and this is reached from a message.
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::Terminal(pane.id)));
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
        // window rather than the other way round. The window's own fields are part of that: the
        // entities are the window's, but the text in them is about the project on screen.
        self.form_filled = None;
        self.refill_fields = true;
        self.workbench.rail_mode = view.rail_mode;
        self.pending_layout = view.layout.clone();
        self.sync_file_panels(project);

        self.pending_focus = focused;
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

    /// The work the two screens over it draw, which belongs to the project on screen.
    pub fn work(&self, cx: &App) -> Option<&WorkProjection> {
        self.open_project(cx).map(|open| &open.work)
    }

    /// The graph's view of that work.
    pub fn graph(&self, cx: &App) -> Option<&GraphView> {
        self.open_project(cx).map(|open| &open.graph)
    }

    /// The board's view of the same work.
    pub fn board(&self, cx: &App) -> Option<&BoardState> {
        self.open_project(cx).map(|open| &open.board)
    }

    pub fn work_mut(&mut self, cx: &App) -> Option<&mut WorkProjection> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.work)
    }

    pub fn graph_mut(&mut self, cx: &App) -> Option<&mut GraphView> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.graph)
    }

    pub fn board_mut(&mut self, cx: &App) -> Option<&mut BoardState> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.board)
    }

    /// The graph and the work behind it, together.
    ///
    /// A drag reads the records while it writes the arrangement, and the two live in the same
    /// [`OpenProject`] — so the pair is handed out once rather than borrowed twice, which nothing
    /// would let a caller do.
    fn graph_over_work(&mut self, cx: &App) -> Option<(&mut GraphView, &WorkProjection)> {
        let id = self.project(cx)?;
        let open = self.projects.get_mut(&id)?;
        Some((&mut open.graph, &open.work))
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

    /// One pane of whichever project holds it, named rather than found through focus. Every panel
    /// draws its own pane, so the lookup is across every project the window holds.
    pub fn pane(&self, pane_id: PaneId) -> Option<&PaneState> {
        self.projects
            .values()
            .find_map(|open| open.panes.iter().find(|pane| pane.id == pane_id))
    }

    /// Whether a pane belongs to the project this window is pointed at. A pane of any other keeps
    /// running and keeps its scrollback; its panel is hidden rather than closed, which is what
    /// keeps its place in the arrangement.
    pub fn pane_is_on_screen(&self, pane_id: PaneId, cx: &App) -> bool {
        self.open_project(cx)
            .is_some_and(|open| open.panes.iter().any(|pane| pane.id == pane_id))
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

    /// End a pane: the harness is killed, the emulator dropped, and the panel taken out of the
    /// dock.
    ///
    /// Idempotent, because the dock reaches it too — a panel whose tab was closed asks for this
    /// once it is sure it was closed rather than displaced. A pane the window has already let go
    /// of is not closed twice.
    pub fn close_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(project) = self.project_of_pane(pane_id) else {
            return;
        };
        self.bus.send(Message::CloseWorkspace { pane_id });
        self.terminals.remove(&pane_id);

        let showing = self.project(cx);
        // The keyboard only moves for the project on screen: a pane closed in a background
        // project must not take focus off the terminal the user is typing into.
        let on_screen = showing == Some(project);
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::Terminal(pane_id)));

        let mut next = None;
        if let Some(open) = self.projects.get_mut(&project) {
            open.panes.retain(|pane| pane.id != pane_id);
            if open.focused_pane == Some(pane_id) {
                next = open.panes.first().map(|pane| pane.id);
                open.focused_pane = next;
            }
        }
        // Closing a pane while a panel that is not a terminal holds the keyboard must not hand it
        // to a terminal that is off screen.
        if on_screen && let Some(pane_id) = next {
            self.pending_focus = Some(pane_id);
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
        let already = self
            .projects
            .get(&project)
            .is_some_and(|open| open.focused_pane == Some(pane_id));
        if let Some(open) = self.projects.get_mut(&project) {
            open.focused_pane = Some(pane_id);
        }
        // `Focus` is sent on the transition and no other. The dock calls this every time a
        // terminal panel becomes the displayed tab of its group, which includes the tab it was
        // already showing.
        if !already {
            self.bus.send(Message::Focus { pane_id });
        }
        cx.notify();
    }

    /// The keyboard has gone to a panel that is not a terminal, so no pane holds it.
    ///
    /// This is today's console rule stated for every non-terminal panel rather than for one. The
    /// project keeps which pane was last focused, so coming back to a terminal panel is where the
    /// pane gets the keyboard again.
    pub fn blur_panes(&mut self, cx: &mut Context<Self>) {
        self.pending_focus = None;
        cx.notify();
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

            Message::ProjectFileDiffed {
                project_id,
                rel_path,
                diff,
            } => {
                // The base is echoed because the interface may have switched since it asked, and
                // it is the base that says which of a file's tabs this belongs in.
                let key = tab_key(&rel_path, Subject::Diff(diff.base));
                if let Some(open) = self.projects.get_mut(&project_id)
                    && let Some(file) = open.editor.open.iter_mut().find(|file| file.key() == key)
                {
                    file.attach_diff(diff);
                }
                cx.notify();
            }

            Message::ProjectFileError {
                project_id,
                rel_path,
                error,
            } => self.file_failed(project_id, rel_path, error, cx),

            // ── the work family ─────────────────────────────────────
            // Every arm is guarded on the project still being held, because an answer can arrive
            // after the window has stopped holding it — the file family's rule, for the file
            // family's reason. The work belongs to the project, so an answer for one nobody here
            // has open has nowhere to be drawn.
            //
            // Anything the host confirms clears the last refusal: a sentence about a change that
            // did not happen is stale the moment one does.
            Message::WorkList {
                project_id,
                sessions,
                agents,
                tasks,
            } => {
                self.workbench.work_error = None;
                let Some(open) = self.projects.get_mut(&project_id) else {
                    return;
                };
                open.work.replace_all(sessions, agents, tasks);
                open.graph.relayout(&open.work);
                // Pointing the screen at the first agent was the fixture constructor's job. It
                // belongs to whoever first learns there is one to point at, and only then: a
                // second `ListWork` must not move a selection the user has since made.
                if open.graph.selection.is_none() {
                    open.graph.selection = open.work.agents.first().map(|a| Selection::Agent(a.id));
                }
                cx.notify();
            }

            Message::TaskCreated { project_id, task } => {
                self.workbench.work_error = None;
                let Some(open) = self.projects.get_mut(&project_id) else {
                    return;
                };
                let id = task.id;
                open.work.apply_task(task);
                open.graph
                    .layout
                    .place_new(&open.work.agents, &open.work.tasks);
                // The task that arrives is the one to select, because the interface could not know
                // the id it was going to be given — the same mechanism `AppState::adding` uses to
                // open the project an `AddProject` answers with.
                if open.board.awaiting_new {
                    open.board.awaiting_new = false;
                    open.board.select(id);
                }
                cx.notify();
            }

            Message::TaskChanged { project_id, task } => {
                self.workbench.work_error = None;
                let Some(open) = self.projects.get_mut(&project_id) else {
                    return;
                };
                // The mark goes on whatever column the answer reports, the old one included: a
                // refusal that left the card where it was must not leave it saying it is still on
                // its way.
                if open.board.is_moving(task.id) {
                    open.board.moving = None;
                }
                let selected = open.board.selected == Some(task.id);
                let editing = open.board.editing.is_some();
                open.work.apply_task(task);
                open.graph
                    .layout
                    .place_new(&open.work.agents, &open.work.tasks);
                // Refill the panel from what the host actually stored — it trims a title, and a
                // field showing what was typed rather than what was kept would be a small lie. Not
                // while a field is open: the user's text wins until they commit or discard it.
                if selected && !editing {
                    self.form_filled = None;
                }
                cx.notify();
            }

            Message::TaskDeleted {
                project_id,
                task_id,
            } => {
                self.workbench.work_error = None;
                let Some(open) = self.projects.get_mut(&project_id) else {
                    return;
                };
                open.work.forget_task(task_id);
                // A panel pointed at a task that has gone reports on nothing, and a mark for a
                // move that can never be answered would never come off.
                if open.board.selected == Some(task_id) {
                    open.board.selected = None;
                    // A field open on a task that has gone has nowhere to commit to.
                    open.board.stop_editing();
                    open.board.confirm_delete = false;
                }
                if open.board.is_moving(task_id) {
                    open.board.moving = None;
                }
                cx.notify();
            }

            Message::AgentChanged { project_id, agent } => {
                self.workbench.work_error = None;
                let Some(open) = self.projects.get_mut(&project_id) else {
                    return;
                };
                open.work.apply_agent(*agent);
                open.graph
                    .layout
                    .place_new(&open.work.agents, &open.work.tasks);
                cx.notify();
            }

            Message::WorkError {
                project_id,
                task_id,
                error,
            } => {
                tracing::error!("work {project_id} {task_id:?}: {error}");
                self.workbench.work_error = Some(error);
                // A refusal ends whatever asked for it, so the panel goes back to reporting the
                // task the host still holds rather than sitting in a field that will not commit.
                if let Some(board) = self.board_mut(cx) {
                    board.stop_editing();
                    board.moving = None;
                    board.awaiting_new = false;
                    board.confirm_delete = false;
                }
                self.form_filled = None;
                cx.notify();
            }

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

        // The pane's panel joins the region terminals live in. It is queued rather than added
        // here: a panel reaches the dock through a `Window`, and a message does not come with one.
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::Terminal(pane_id)));

        if showing {
            self.pending_focus = Some(pane_id);
            self.bus.send(Message::Focus { pane_id });
        }
        cx.notify();
    }

    /// Give the keyboard to whoever asked for it. Focus needs a window, so it waits for one.
    ///
    /// Only a pane is ever owed it here. Every other panel takes the keyboard from the dock, which
    /// focuses whatever panel it has just displayed.
    fn take_focus(&mut self, window: &mut Window, cx: &mut App) {
        let Some(pane_id) = self.pending_focus.take() else {
            return;
        };
        if let Some(terminal) = self.terminals.get(&pane_id) {
            terminal
                .view
                .read(cx)
                .focus_handle()
                .clone()
                .focus(window, cx);
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
        self.sink.settings.menu = None;
        cx.notify();
    }

    // ── The kitchen sink ────────────────────────────────────────────
    //
    // The application's own test bench. Every mutator here ends in `cx.notify()` like every other
    // one, and none of them means anything: the sink is where a control is looked at, so what it
    // holds is a value and never a claim about a project, a pane or a task.

    pub fn set_sink_section(&mut self, section: SinkSection, cx: &mut Context<Self>) {
        self.sink.section = section;
        cx.notify();
    }

    /// Put one fixture into one of its viewer's layouts. A viewer with no preview keeps its source,
    /// which is [`SinkState::set_layout`]'s rule rather than this method's.
    pub fn set_sink_layout(
        &mut self,
        doc: &'static SinkDoc,
        layout: ViewLayout,
        cx: &mut Context<Self>,
    ) {
        self.sink.set_layout(doc, layout);
        cx.notify();
    }

    /// The buffer one fixture is edited in. The document is a constant and the buffer is what has
    /// been typed into it, which is what lets the preview follow the source half of a split.
    pub fn sink_buffer(&self, key: &str) -> Option<&Entity<EditorState>> {
        self.sink_buffers.get(key)
    }

    pub fn toggle_sink_facet(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(facet) = self.sink.facets.get_mut(index) {
            *facet = !*facet;
        }
        cx.notify();
    }

    pub fn set_sink_choice(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.choice = index;
        cx.notify();
    }

    pub fn nudge_sink(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.nudge(delta);
        cx.notify();
    }

    pub fn toggle_sink_disclosure(&mut self, cx: &mut Context<Self>) {
        self.sink.disclosed = !self.sink.disclosed;
        cx.notify();
    }

    /// The style reference's demo menu. It closes on the pick like every other menu in the window,
    /// because that is the behaviour being demonstrated.
    pub fn pick_sink_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.picked = index;
        self.workbench.open_menu = None;
        cx.notify();
    }

    pub fn open_sink_modal(&mut self, modal: SinkModal, cx: &mut Context<Self>) {
        // A modal takes the window's attention, so it closes whatever menu was down: two things
        // claiming an outside click is how a dismissal races itself.
        self.workbench.open_menu = None;
        self.sink.modal = Some(modal);
        cx.notify();
    }

    pub fn close_sink_modal(&mut self, cx: &mut Context<Self>) {
        self.sink.modal = None;
        cx.notify();
    }

    pub fn set_sink_settings_nav(&mut self, nav: SettingsNav, cx: &mut Context<Self>) {
        self.sink.settings.nav = nav;
        cx.notify();
    }

    pub fn set_sink_settings_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.theme = index;
        cx.notify();
    }

    pub fn toggle_sink_accent_follows(&mut self, cx: &mut Context<Self>) {
        self.sink.settings.accent_follows = !self.sink.settings.accent_follows;
        cx.notify();
    }

    pub fn set_sink_settings_density(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.density = index;
        cx.notify();
    }

    pub fn nudge_sink_font(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_font(delta);
        cx.notify();
    }

    pub fn toggle_sink_reduce_motion(&mut self, cx: &mut Context<Self>) {
        self.sink.settings.reduce_motion = !self.sink.settings.reduce_motion;
        cx.notify();
    }

    pub fn set_sink_permission(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.permission = index;
        cx.notify();
    }

    pub fn nudge_sink_agents(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_agents(delta);
        cx.notify();
    }

    pub fn nudge_sink_warn(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_warn(delta);
        cx.notify();
    }

    pub fn toggle_sink_retry(&mut self, cx: &mut Context<Self>) {
        self.sink.settings.retry = !self.sink.settings.retry;
        cx.notify();
    }

    pub fn nudge_sink_idle(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_idle(delta);
        cx.notify();
    }

    pub fn toggle_sink_harness(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.toggle_harness(index);
        cx.notify();
    }

    pub fn toggle_sink_harness_open(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.toggle_open(index);
        cx.notify();
    }

    pub fn open_sink_settings_menu(&mut self, which: SettingsMenu, cx: &mut Context<Self>) {
        self.workbench.open_menu = Some(MenuId::SinkSettings);
        self.sink.settings.menu = Some(which);
        cx.notify();
    }

    pub fn pick_sink_settings_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.sink.settings.menu {
            Some(SettingsMenu::Auth) => self.sink.settings.auth = index,
            Some(SettingsMenu::Model) => self.sink.settings.model = index,
            Some(SettingsMenu::Thinking) => self.sink.settings.thinking = index,
            Some(SettingsMenu::Mode) => self.sink.settings.mode = index,
            None => {}
        }
        self.workbench.open_menu = None;
        self.sink.settings.menu = None;
        cx.notify();
    }

    pub fn add_sink_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pair = self.sink_harness_env.read(cx).value().to_string();
        self.sink.settings.add_env(pair);
        let input = self.sink_harness_env.clone();
        input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn remove_sink_env(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.remove_env(index);
        cx.notify();
    }

    pub fn set_sink_project_nav(&mut self, nav: ProjectNav, cx: &mut Context<Self>) {
        self.sink.project.nav = nav;
        cx.notify();
    }

    pub fn set_sink_project_colour(
        &mut self,
        colour: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sink.project.set_swatch(colour);
        self.sync_sink_project_hex(window, cx);
        cx.notify();
    }

    pub fn toggle_sink_colour_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = !self.sink.project.picker_open;
        if open {
            let rgb = self.sink_project_rgb();
            let (hue, sat, val) = crate::state::sink::rgb_to_hsv(rgb);
            self.sink.project.hue = hue;
            self.sink.project.sat = sat;
            self.sink.project.val = val;
        }
        self.sink.project.picker_open = open;
        self.sync_sink_project_hex(window, cx);
        cx.notify();
    }

    pub fn set_sink_project_hsv(
        &mut self,
        hue: f32,
        sat: f32,
        val: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sink.project.set_hsv(hue, sat, val);
        self.sync_sink_project_hex(window, cx);
        cx.notify();
    }

    fn apply_sink_project_hex(&mut self, cx: &mut Context<Self>) {
        let text = self.sink_project_hex.read(cx).value();
        let Some(rgb) = crate::state::sink::parse_hex(text.as_ref()) else {
            return;
        };
        if self.sink.project.custom == Some(rgb) {
            return;
        }
        if self.sink.project.custom.is_none() && rgb == self.sink_project_swatch_rgb() {
            return;
        }
        self.sink.project.set_rgb(rgb);
        cx.notify();
    }

    fn sync_sink_project_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = crate::state::sink::hex_string(self.sink_project_rgb());
        let input = self.sink_project_hex.clone();
        input.update(cx, |input, cx| input.set_value(&hex, window, cx));
    }

    fn sink_project_rgb(&self) -> u32 {
        self.sink
            .project
            .custom
            .unwrap_or_else(|| self.sink_project_swatch_rgb())
    }

    fn sink_project_swatch_rgb(&self) -> u32 {
        let colour = theme::project_colour(self.sink.project.colour);
        crate::state::sink::rgb_from_channels(colour.r, colour.g, colour.b)
    }

    pub fn reset_sink_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.project.reset();
        let name = self.sink_project_name.clone();
        let about = self.sink_project_about.clone();
        name.update(cx, |input, cx| {
            input.set_value(crate::state::sink::PROJECT_NAME, window, cx)
        });
        about.update(cx, |input, cx| {
            input.set_value(crate::state::sink::PROJECT_ABOUT, window, cx)
        });
        self.sync_sink_project_hex(window, cx);
        cx.notify();
    }

    // The picker page's controls. Each sets one field of the request the next dialog is raised
    // with; none of them touches a picker that is already up, because the ask a dialog was opened
    // under is the ask it is answering.

    pub fn set_sink_pick_kind(&mut self, kind: PickKind, cx: &mut Context<Self>) {
        self.sink.picker.kind = kind;
        cx.notify();
    }

    pub fn set_sink_pick_count(&mut self, count: PickerCount, cx: &mut Context<Self>) {
        self.sink.picker.count = count;
        cx.notify();
    }

    pub fn set_sink_pick_commit(&mut self, commit: Commit, cx: &mut Context<Self>) {
        self.sink.picker.commit = commit;
        cx.notify();
    }

    pub fn set_sink_pick_modal(&mut self, modal: bool, cx: &mut Context<Self>) {
        self.sink.picker.modal = modal;
        cx.notify();
    }

    pub fn set_sink_pick_view(&mut self, view: PickerView, cx: &mut Context<Self>) {
        self.sink.picker.view = view;
        cx.notify();
    }

    pub fn set_sink_pick_root(&mut self, root: usize, cx: &mut Context<Self>) {
        self.sink.picker.root = root;
        cx.notify();
    }

    pub fn set_sink_pick_pattern(&mut self, pattern: usize, cx: &mut Context<Self>) {
        self.sink.picker.pattern = pattern;
        cx.notify();
    }

    /// Raise a picker over the sink's fixture tree, in the shape the page's controls describe.
    ///
    /// The previous answer goes with it: a readout left standing over a dialog that is being asked
    /// again reads as this dialog's answer, which it is not.
    pub fn raise_sink_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.picker.result = None;
        self.sink.picker.dismissed = false;
        let request = self.sink.picker.request();
        let view = self.sink.picker.view;
        self.open_file_picker(request, crate::state::sink::picker_tree(), view, window, cx);
    }

    // ── The file picker ─────────────────────────────────────────
    //
    // One dialog, raised by whichever screen needs a path and answered back to whoever asked. The
    // window holds it because exactly one may be up, and because the field above its rows is one
    // of the window's fields like every other.

    /// Raise a picker over `forest`, in the arrangement it should open in.
    ///
    /// The field is emptied first: a filter left over from the last dialog would hide rows the new
    /// one was raised to show.
    pub fn open_file_picker(
        &mut self,
        request: crate::state::file_picker::PickerRequest,
        forest: Vec<crate::state::file_picker::PickerNode>,
        view: PickerView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A dialog takes the window's attention, so it closes whatever menu was down: two things
        // claiming an outside click is how a dismissal races itself.
        self.workbench.open_menu = None;
        self.picker_filter
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.file_picker = Some(FilePickerState::open(request, forest, view));
        // The field takes the keyboard, because the first thing a picker is for is typing a name
        // into it. Every other key the dialog answers to is bound against the field as well as
        // against the dialog, so the arrows still drive the rows — see `ui::file_picker`.
        let field = self.picker_filter.read(cx).focus_handle(cx);
        window.focus(&field, cx);
        cx.notify();
    }

    /// A key the dialog answered, or did not.
    ///
    /// Answering `false` is what hands the key back: `left` and `right` mean nothing in the flat
    /// list, and the caller propagates so the filter field gets its caret keys back.
    pub fn press_picker_key(&mut self, key: PickerKey, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.file_picker.as_mut() else {
            return false;
        };
        let pressed = picker.press(key);
        let at = picker.cursor_index();

        match pressed {
            Pressed::Ignored => false,
            Pressed::Moved => {
                // Follow the cursor: an arrow past the last drawn row has to bring it into view.
                if let Some(at) = at {
                    self.picker_scroll.scroll_to_item(at);
                }
                cx.notify();
                true
            }
            Pressed::Commit => {
                self.commit_file_picker(cx);
                true
            }
            Pressed::Dismiss => {
                self.cancel_file_picker(cx);
                true
            }
        }
    }

    pub fn set_picker_view(&mut self, view: PickerView, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.set_view(view);
        }
        cx.notify();
    }

    pub fn toggle_picker_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.toggle_folder(&path);
        }
        cx.notify();
    }

    /// What a click on a row does, which the picker itself decides: a folder that cannot be picked
    /// opens, and a pick that was asked to be final closes the dialog on the spot.
    pub fn click_picker_row(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.as_mut() else {
            return;
        };
        if picker.click(&path) {
            self.commit_file_picker(cx);
            return;
        }
        cx.notify();
    }

    /// Hand what was chosen to whoever asked for it, and take the dialog down.
    pub fn commit_file_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        let picked = picker.picked().to_vec();
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = Some(picked);
                self.sink.picker.dismissed = false;
            }
        }
        cx.notify();
    }

    /// Take the dialog down with nothing chosen. Dismissed is not the same answer as an empty one,
    /// so whoever asked is told which it was.
    pub fn cancel_file_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = None;
                self.sink.picker.dismissed = true;
            }
        }
        cx.notify();
    }

    pub fn start_picker_resize(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.start_drag(at);
        }
        cx.notify();
    }

    /// Follow a corner drag. The window is what the dialog has to fit inside, so its size comes in
    /// with the pointer.
    pub fn drag_picker_resize(&mut self, at: (f32, f32), window: &Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        if let Some(picker) = self.file_picker.as_mut()
            && picker.drag_to(at, (f32::from(viewport.width), f32::from(viewport.height)))
        {
            cx.notify();
        }
    }

    pub fn end_picker_resize(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut()
            && picker.is_resizing()
        {
            picker.end_drag();
            cx.notify();
        }
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
                    self.pending_layout = view.layout.clone();
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
        // back from the project every time. Tab keys rather than paths, because a file and its diff
        // are two tabs and a path names both.
        if let Some(open) = self.projects.get_mut(&project) {
            open.prefs.open_files = open.editor.open.iter().map(|file| file.key()).collect();
            open.prefs.active_file = open.editor.active_file().map(|file| file.key());
            open.prefs.expanded = open.explorer.expanded();
            open.prefs.selected = open.explorer.selected.clone();
        }

        if self.project(cx) == Some(project) {
            // The whole arrangement in one blob — the tree, the axes, the sizes, and which tab is
            // displayed. The three region flags are written beside it for a build that has the
            // blob and cannot read it; the blob is what a restore uses.
            let layout = self.layout_blob(cx);
            let (left, bottom, right) = self.regions_open(cx);
            let rail_mode = self.workbench.rail_mode;
            if let Some(open) = self.projects.get_mut(&project) {
                open.prefs.rail_mode = rail_mode;
                open.prefs.show_left = left;
                open.prefs.show_bottom = bottom;
                open.prefs.show_right = right;
                open.prefs.layout = layout;
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

    // ── The dock ────────────────────────────────────────────────────

    /// The window's arrangement, for the one module that draws it.
    pub fn dock(&self) -> &Entity<DockArea> {
        &self.dock
    }

    /// Which of the three edge regions are on screen, for the titlebar's switches. The dock is
    /// asked rather than a flag beside it, so a region the user emptied reads as closed here too.
    pub fn regions_open(&self, cx: &App) -> (bool, bool, bool) {
        let dock = self.dock.read(cx);
        (
            dock.is_dock_open(dock::placement_of(Region::Left)),
            dock.is_dock_open(dock::placement_of(Region::Bottom)),
            dock.is_dock_open(dock::placement_of(Region::Right)),
        )
    }

    /// Put a region away, or bring it back. The dock remembers the size either way, which is what
    /// makes a toggle non-destructive.
    pub fn toggle_region(&mut self, region: Region, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |dock, cx| {
            dock.toggle_dock(dock::placement_of(region), window, cx);
        });
        cx.notify();
    }

    /// The panel for one kind, built the first time it is asked for.
    fn panel(&mut self, kind: PanelKind, cx: &mut App) -> Entity<WorkbenchPanel> {
        if let Some(panel) = self.panels.get(&kind) {
            return panel.clone();
        }
        let panel = WorkbenchPanel::new(kind.clone(), self.this.clone(), cx);
        self.panels.insert(kind, panel.clone());
        panel
    }

    /// Make the dock's file panels the files of one project.
    ///
    /// **The open files are a project's, and the panels are the window's**, so the two have to be
    /// squared whenever the window changes which project it is pointed at. A saved arrangement
    /// usually carries the incoming project's own file panels and these edits are then no-ops;
    /// a project that has never been written down has none, and this is what gives it them.
    fn sync_file_panels(&mut self, project: ProjectId) {
        let wanted: Vec<String> = self
            .projects
            .get(&project)
            .map(|open| open.editor.open.iter().map(|file| file.key()).collect())
            .unwrap_or_default();

        for kind in self.panels.keys() {
            if let Some(key) = kind.tab_key()
                && !wanted.iter().any(|open| open == key)
            {
                self.pending_panels.push(PanelEdit::Close(kind.clone()));
            }
        }
        for key in wanted {
            let kind = PanelKind::File(key);
            if !self.panels.contains_key(&kind) {
                self.pending_panels.push(PanelEdit::Open(kind));
            }
        }
    }

    /// Put the panels that arrived on a message into the dock, and take out the ones that left.
    ///
    /// Drained in `render`, which is the same device the pending focus and the arrived files use,
    /// and for the same reason: both halves of a panel's life need a window.
    fn settle_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_panels.is_empty() {
            return;
        }
        for edit in std::mem::take(&mut self.pending_panels) {
            match edit {
                PanelEdit::Open(kind) => {
                    let home = kind.home();
                    let panel = self.panel(kind, cx);
                    // A saved arrangement is rebuilt before this queue is drained, so a file panel
                    // can already be in the tree by the time the edit that asked for it is read.
                    // Adding it twice would be two tabs on one file.
                    if dock::holds(&self.dock.clone(), &panel, cx) {
                        continue;
                    }
                    dock::add(&self.dock.clone(), &panel, home, window, cx);
                }
                PanelEdit::Close(kind) => {
                    if let Some(panel) = self.panels.remove(&kind) {
                        dock::remove(&self.dock.clone(), &panel, window, cx);
                    }
                }
            }
        }
    }

    /// Rebuild a saved arrangement, on the frame after it arrives.
    ///
    /// A layout this build cannot use — a stale version, or one whose panels it has all lost — is
    /// discarded for the arrangement a fresh window opens in, rather than half-applied.
    fn settle_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = self.pending_layout.take() else {
            return;
        };
        let dock = self.dock.clone();
        let panels = std::mem::take(&mut self.panels);
        let app = self.this.clone();
        let mut kept = HashMap::new();
        let mut layouts: Vec<(String, ViewLayout)> = Vec::new();
        {
            let mut build = |kind: PanelKind, cx: &mut App| {
                kept.entry(kind.clone())
                    .or_insert_with(|| {
                        panels
                            .get(&kind)
                            .cloned()
                            .unwrap_or_else(|| WorkbenchPanel::new(kind, app.clone(), cx))
                    })
                    .clone()
            };
            if !dock::restore(&dock, &saved, &mut build, &mut layouts, window, cx) {
                dock::default_layout(&dock, &mut build, window, cx);
            }
        }
        // A terminal's panel is never in a saved layout — harnesses do not persist — so the ones
        // this window still holds are put back beside whatever was restored.
        for (kind, panel) in panels {
            if kind.pane().is_some() && !kept.contains_key(&kind) {
                let home = kind.home();
                dock::add(&dock, &panel, home, window, cx);
                kept.insert(kind, panel);
            }
        }
        self.panels = kept;
        // A file panel's payload carries the layout its viewer was left in, which belongs on the
        // file rather than on the panel: the panel only repeats it, the way it repeats visibility.
        if let Some(project) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&project)
        {
            for (key, layout) in layouts {
                if let Some(file) = open.editor.open.iter_mut().find(|file| file.key() == key) {
                    file.set_layout(layout);
                }
            }
        }
        cx.notify();
    }

    /// Tell every panel whether it is drawn.
    ///
    /// **The window pushes this rather than the panel reading it back.** The dock asks a panel
    /// whether it is visible while it is reconciling a tree — which happens from inside this
    /// window's own update, when a region is toggled, a panel is added, or the arrangement is
    /// written down — and a panel reading `AppState` there would be reading an entity that is
    /// already leased. So the answer is kept current here, where the facts are, and the panel only
    /// repeats it.
    fn settle_visibility(&mut self, cx: &mut Context<Self>) {
        let is_ide = self.workbench.is_ide();
        let has_project = self.project(cx).is_some();
        let on_screen: Vec<PaneId> = self
            .open_project(cx)
            .map(|open| open.panes.iter().map(|pane| pane.id).collect())
            .unwrap_or_default();
        // The tab keys the project on screen holds, and the layout each of them is in. Read once:
        // every file panel asks the same two questions of it.
        let files: HashMap<String, ViewLayout> = self
            .editor(cx)
            .map(|editor| {
                editor
                    .open
                    .iter()
                    .map(|file| (file.key(), file.layout))
                    .collect()
            })
            .unwrap_or_default();

        let mut changed = false;
        for (kind, panel) in &self.panels {
            let key = kind.tab_key();
            let at = Visibility {
                is_ide,
                has_project,
                pane_on_screen: kind.pane().is_some_and(|id| on_screen.contains(&id)),
                file_open: key.is_some_and(|key| files.contains_key(key)),
                any_file_open: !files.is_empty(),
            };
            let drawn = kind.is_drawn(at);
            let layout = key
                .and_then(|key| files.get(key).copied())
                .unwrap_or_default();
            changed |= panel.update(cx, |panel, _| {
                let visible = panel.set_visible(drawn);
                panel.set_layout(layout) || visible
            });
        }
        if changed {
            cx.notify();
        }
    }

    /// Put back any panel the user dropped somewhere its kind forbids.
    fn enforce_placement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kinds: HashMap<PanelId, PanelKind> = self
            .panels
            .iter()
            .map(|(kind, panel)| (PanelId::from(panel.entity_id()), kind.clone()))
            .collect();
        let dock = self.dock.clone();
        dock::enforce_placement(&dock, &|id| kinds.get(&id).cloned(), window, cx);
    }

    /// The arrangement as it stands, for the blob the host keeps.
    fn layout_blob(&self, cx: &App) -> Option<serde_json::Value> {
        serde_json::to_value(self.dock.read(cx).dump(cx)).ok()
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
            // The tab and its panel open together: each open file is its own panel, so a tab with
            // none is a file with nowhere to be drawn.
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(tab_key(
                    &path,
                    Subject::File,
                ))));
            self.bus.send(Message::ReadProjectFile {
                project_id: project,
                rel_path: path,
                max_bytes: Some(MAX_FILE_BYTES),
            });
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// Open a tab on a file's change against a version-control base, and ask the host for it.
    ///
    /// A diff is not the file, so it is a tab of its own beside it rather than something the file's
    /// tab switches into: opening a comparison never takes over what is being read or edited.
    pub fn open_diff(&mut self, path: String, base: DiffBase, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let key = tab_key(&path, Subject::Diff(base));
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };

        let fresh = index_of_key(&open.editor, &key).is_none();
        if fresh {
            open.editor
                .open
                .push(OpenFile::pending_on(&path, Subject::Diff(base)));
        }
        open.editor.active = index_of_key(&open.editor, &key).unwrap_or(0);

        if fresh {
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(key)));
            self.bus.send(Message::DiffProjectFile {
                project_id: project,
                rel_path: path,
                base,
            });
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// One open tab of the project on screen, by its key. What a file panel draws and what its tab
    /// reports are both this.
    pub fn file(&self, key: &str, cx: &App) -> Option<&OpenFile> {
        self.editor(cx)?.open.iter().find(|file| file.key() == key)
    }

    /// Put one file's viewer into one of its layouts.
    ///
    /// The panel repeats the fact rather than owning it — `settle_visibility` pushes it every
    /// frame — but it is pushed here too, because the arrangement can be written down before the
    /// next frame runs and a panel one frame behind would write down the layout the file was in
    /// before the click.
    pub fn set_view_layout(&mut self, key: &str, layout: ViewLayout, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        file.set_layout(layout);

        let panel = self.panels.get(&PanelKind::File(key.to_string())).cloned();
        if let Some(panel) = panel {
            panel.update(cx, |panel, _| panel.set_layout(layout));
        }
        self.remember_view(cx);
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

        // Tab keys, so a remembered diff reopens as a diff rather than as the file it was taken
        // from. A key that is already open is not opened twice.
        for key in &view.open_files {
            if index_of_key(&open.editor, key).is_some() {
                continue;
            }
            let (path, subject) = from_tab_key(key);
            open.editor.open.push(OpenFile::pending_on(&path, subject));
        }
        if let Some(active) = &view.active_file
            && let Some(at) = index_of_key(&open.editor, active)
        {
            open.editor.active = at;
        }
        open.explorer.selected = view.selected.clone();
        open.wanted = view.expanded.clone();

        // Each tab is a panel. A saved arrangement usually carries them and the queued edits are
        // then no-ops, but one that was discarded — a stale version, an unreadable blob — must
        // still leave the files somewhere to be drawn.
        let tabs: Vec<(String, String, Subject)> = open
            .editor
            .open
            .iter()
            .map(|file| (file.key(), file.path.clone(), file.subject))
            .collect();
        for (key, rel_path, subject) in tabs {
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(key)));
            match subject {
                Subject::File => self.bus.send(Message::ReadProjectFile {
                    project_id: project,
                    rel_path,
                    max_bytes: Some(MAX_FILE_BYTES),
                }),
                Subject::Diff(base) => self.bus.send(Message::DiffProjectFile {
                    project_id: project,
                    rel_path,
                    base,
                }),
            }
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
        let key = file.key();
        if file.dirty() && open.editor.pending_tab_close.as_deref() != Some(key.as_str()) {
            open.editor.pending_tab_close = Some(key);
            cx.notify();
            return;
        }

        open.editor.close(index);
        // The tab and its panel go together, and the panel leaves through a `Window` this does not
        // have — so it queues, like every other edit to the dock.
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::File(key)));
        self.remember(project, cx);
        cx.notify();
    }

    /// A file panel became the displayed tab of its group, so its file is the active one.
    ///
    /// The dock is where that is decided and the editor learns it from here, which is the same
    /// direction focus already travels.
    pub fn activate_file(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            return;
        };
        open.editor.active = at;
        open.editor.pending_tab_close = None;
        if let Some(path) = open.editor.active_file().map(|file| file.path.clone()) {
            open.explorer.selected = Some(path);
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// A file panel left the dock for good, so its tab closes with it.
    ///
    /// A tab holding unsaved changes asks first, which through the dock means **coming back**: the
    /// panel is put in its home group again with its label turned into the question, and a second
    /// close takes it. Bringing it forward is still how the question is answered no.
    pub fn closed_file_panel(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            // The tab went first — `close_editor_tab` — and this is the panel following it out.
            self.panels.remove(&PanelKind::File(key.to_string()));
            return;
        };

        if open.editor.open[at].dirty() && open.editor.pending_tab_close.as_deref() != Some(key) {
            open.editor.pending_tab_close = Some(key.to_string());
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(key.to_string())));
            cx.notify();
            return;
        }

        open.editor.close(at);
        self.panels.remove(&PanelKind::File(key.to_string()));
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
    //
    // Every handler here is guarded on the window holding a project: the screen is a view of one
    // project's work, and a window with none open has nothing for it to act on.

    /// Point the screen at a session or at one agent. Both are selections, and everything else on
    /// the screen — the graph's session, the inspector, the tasks drawer — is a function of this
    /// one field.
    pub fn select_in_graph(&mut self, selection: Selection, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.selection = Some(selection);
        }
        cx.notify();
    }

    /// Draw one session's agents, or every session's. It does not move the selection: what the
    /// inspector and the drawer report on is a separate question from what the canvas draws.
    pub fn show_graph_session(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.show_session(session);
        }
        cx.notify();
    }

    /// Put every filter on the agents screen back. The one control for "show me all of it".
    pub fn clear_graph_filters(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.clear_filters();
        }
        cx.notify();
    }

    pub fn toggle_agent_bucket(&mut self, bucket: Bucket, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.toggle_bucket(bucket);
        }
        cx.notify();
    }

    pub fn zoom_graph(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.zoom_by(delta);
        }
        cx.notify();
    }

    pub fn reset_graph_zoom(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.zoom = 1.0;
        }
        cx.notify();
    }

    /// Throw the arrangement away and lay the graph out again from what the agents and tasks say.
    ///
    /// Every hand-placed card is lost, which is the point: it is the way back from a canvas the
    /// user has pulled apart, and there is nothing else on the screen that undoes a drag. The full
    /// `relayout` rather than `place_new`, which is the one that leaves placed cards alone.
    pub fn tidy_graph(&mut self, cx: &mut Context<Self>) {
        if let Some((graph, work)) = self.graph_over_work(cx) {
            graph.relayout(work);
        }
        cx.notify();
    }

    pub fn toggle_inspector(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.show_inspector = !graph.show_inspector;
        }
        cx.notify();
    }

    pub fn toggle_tasks_drawer(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.tasks_open = !graph.tasks_open;
        }
        cx.notify();
    }

    /// Select one agent and put the inspector on its thread — what the `chat` affordance on a card
    /// does, and the one place the screen changes two things at once, because a card asking for a
    /// conversation with the panel shut has asked for nothing.
    pub fn open_agent_chat(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.selection = Some(Selection::Agent(agent));
            graph.tab = InspectorTab::Chat;
            graph.show_inspector = true;
        }
        cx.notify();
    }

    pub fn select_inspector_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.tab = if index == 0 {
                InspectorTab::Chat
            } else {
                InspectorTab::Tasks
            };
        }
        cx.notify();
    }

    /// Pick a card or a container up.
    ///
    /// A card selects itself on the way up, because what is being moved is what the user is
    /// looking at, and a drag that left the inspector on something else would be reporting on the
    /// wrong agent. A container does not: dragging a box to make room is not a claim about what
    /// the user wants to read.
    pub fn start_graph_carry(&mut self, held: Held, grab: (f32, f32), cx: &mut Context<Self>) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.start_carry(held, grab);
            if let Held::Agent(agent) = held {
                graph.selection = Some(Selection::Agent(agent));
            }
        }
        cx.notify();
    }

    /// Move whatever is being carried, and lay a grain of sand where the pointer passed.
    ///
    /// The trail is skipped when the system asks for reduced motion — it is the only motion on
    /// this screen, and what is held still follows the pointer without it.
    pub fn move_graph_carry(
        &mut self,
        at: (f32, f32),
        pointer: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let trail = (!cx.reduce_motion()).then_some(pointer);
        if let Some((graph, work)) = self.graph_over_work(cx) {
            graph.carry_to(work, at, trail, std::time::Instant::now());
        }
        cx.notify();
    }

    /// Put it down, and ask for the card to be moved into whatever container it landed in.
    ///
    /// **Position is the interface's own fact, membership is the host's.** The drop writes the
    /// card's new offset and nothing else; which task it serves is written down, so the answer is
    /// an `AssignAgent` and the card only changes hands when the host says it has.
    pub fn end_graph_carry(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let landed = self
            .graph_over_work(cx)
            .and_then(|(graph, work)| graph.end_carry(work));
        if let Some((agent_id, task_id)) = landed {
            self.bus.send(Message::AssignAgent {
                project_id,
                agent_id,
                task_id: Some(task_id),
            });
        }
        cx.notify();
    }

    /// What the composer sends, to the selected agent.
    ///
    /// Nothing is appended here. The line lands in the thread when the host answers with the agent
    /// carrying it — an interface that writes its own message into a transcript is inventing half
    /// of a conversation.
    pub fn send_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(graph) = self.graph(cx) else {
            return;
        };
        let text = graph.draft.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(Selection::Agent(agent_id)) = graph.selection else {
            return;
        };
        self.bus.send(Message::SendToAgent {
            project_id,
            agent_id,
            text,
        });
        if let Some(graph) = self.graph_mut(cx) {
            graph.draft.clear();
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
        let stranded = self
            .graph(cx)
            .is_some_and(|graph| graph.carry.is_some() && !cx.has_active_drag());
        if stranded {
            self.end_graph_carry(cx);
        }
        if let Some(graph) = self.graph_mut(cx) {
            graph.settle_sand(std::time::Instant::now());
        }
    }

    // ── The tasks board ─────────────────────────────────────────────

    // ── the task panel's own edits ──────────────────────────────────
    // Every one of these asks and waits. The panel goes on reporting the task the host last
    // confirmed, so a refusal leaves nothing to unwind — which is the same reason a pane is drawn
    // when the coordinator answers rather than when the interface asked.

    /// Open one of the panel's fields.
    pub fn begin_task_edit(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        // A step's field starts from what the step says now, because it is one field shared by
        // however many steps the task has.
        if let Field::Step(step_id) = field {
            let title = self
                .open_task_form(cx)
                .and_then(|(_, task_id, _)| self.work(cx)?.task(task_id))
                .and_then(|task| task.step(step_id))
                .map(|step| step.title.clone())
                .unwrap_or_default();
            if let Some(board) = self.board_mut(cx) {
                board.form.step_title = title.clone();
            }
            let input = self.step_title_input.clone();
            input.update(cx, |state, cx| state.set_value(&title, window, cx));
        }
        if let Some(board) = self.board_mut(cx) {
            board.edit(field);
        }
        // The field takes the keyboard from the click that opened it, so one click starts typing.
        match field {
            Field::Title => {
                let input = self.task_title_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::Description => {
                let input = self.task_description_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::Step(_) => {
                let input = self.step_title_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::NewStep => {
                let input = self.new_step_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
        }
        cx.notify();
    }

    /// Put the open field away and keep the task as the host last reported it.
    pub fn cancel_task_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        // Refill from the record, so what was typed and thrown away is gone rather than waiting to
        // be committed by the next click.
        self.form_filled = None;
        self.fill_task_form(window, cx);
        cx.notify();
    }

    pub fn toggle_description_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.preview = !board.preview;
        }
        cx.notify();
    }

    /// The project, the open task and the panel's form, or nothing if there is no task open.
    fn open_task_form(&self, cx: &App) -> Option<(ProjectId, TaskId, &BoardState)> {
        let project = self.project(cx)?;
        let board = self.board(cx)?;
        Some((project, board.selected?, board))
    }

    /// Send one `UpdateTask`, and put the field away.
    ///
    /// A value equal to the one the host already holds sends nothing: the message set is for acts,
    /// and re-asserting a title is not one.
    fn update_task(
        &mut self,
        title: Option<String>,
        description: Option<String>,
        priority: Option<Priority>,
        shape: Option<Shape>,
        cx: &mut Context<Self>,
    ) {
        let Some((project_id, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        self.bus.send(Message::UpdateTask {
            project_id,
            task_id,
            title,
            description,
            priority,
            shape,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        cx.notify();
    }

    pub fn commit_task_title(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        if !board.is_editing(Field::Title) {
            return;
        }
        let typed = board.form.title.trim().to_string();
        // An empty title is a slip rather than an intention, so it is refused here and never sent —
        // the same posture as Send reading as disabled on an empty draft.
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .is_some_and(|task| task.title == typed);
        if typed.is_empty() || unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        self.update_task(Some(typed), None, None, None, cx);
    }

    /// A description, unlike a title, may be emptied: clearing one is a thing to mean.
    pub fn commit_task_description(&mut self, cx: &mut Context<Self>) {
        let Some((_, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let typed = board.form.description.clone();
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .is_some_and(|task| task.description == typed);
        if unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        self.update_task(None, Some(typed), None, None, cx);
    }

    pub fn set_task_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        self.update_task(None, None, Some(priority), None, cx);
    }

    pub fn set_task_shape(&mut self, shape: Shape, cx: &mut Context<Self>) {
        self.update_task(None, None, None, Some(shape), cx);
    }

    /// Hand the open task to a session, or take it back. `None` is a task nobody has started.
    pub fn set_task_session(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        let Some((project_id, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        self.close_menu(cx);
        self.bus.send(Message::AssignTask {
            project_id,
            task_id,
            session,
        });
        cx.notify();
    }

    /// Add a sub-task and keep the field, so several can be typed in a row.
    pub fn add_task_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project_id, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let title = board.form.new_step.trim().to_string();
        if title.is_empty() {
            return;
        }
        self.bus.send(Message::AddStep {
            project_id,
            task_id,
            title,
        });
        if let Some(board) = self.board_mut(cx) {
            board.form.new_step.clear();
        }
        let input = self.new_step_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub fn commit_step_title(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((project_id, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let Some(Field::Step(step_id)) = board.editing else {
            return;
        };
        let title = board.form.step_title.trim().to_string();
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .and_then(|task| task.step(step_id))
            .is_some_and(|step| step.title == title);
        if title.is_empty() || unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        self.bus.send(Message::RenameStep {
            project_id,
            task_id,
            step_id,
            title,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        cx.notify();
    }

    /// Drop a sub-task. No confirmation: the two-click question is for what cannot be retyped, and
    /// a sub-task's title is one line.
    pub fn remove_task_step(&mut self, step_id: StepId, cx: &mut Context<Self>) {
        let Some((project_id, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        self.bus.send(Message::RemoveStep {
            project_id,
            task_id,
            step_id,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        cx.notify();
    }

    /// Delete the open task. The first click asks; only the second sends.
    ///
    /// A task is the one thing on this panel that cannot be retyped, which is why it takes the
    /// question the picker's Forget takes and a sub-task's × does not.
    pub fn delete_task(&mut self, cx: &mut Context<Self>) {
        let Some((project_id, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        if !board.confirm_delete {
            if let Some(board) = self.board_mut(cx) {
                board.confirm_delete = true;
            }
            cx.notify();
            return;
        }
        self.bus.send(Message::DeleteTask {
            project_id,
            task_id,
        });
        if let Some(board) = self.board_mut(cx) {
            board.confirm_delete = false;
        }
        cx.notify();
    }

    /// Withdraw the delete question, which any other click on the panel does.
    pub fn withdraw_task_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            if !board.confirm_delete {
                return;
            }
            board.confirm_delete = false;
        }
        cx.notify();
    }

    /// Fill the panel's fields from the task that is open, once per selection.
    ///
    /// Drained in `render` rather than done where the selection changes, because `set_value` needs a
    /// window and three of the callers have none: a message that arrives, a project switch, and the
    /// board's own jump to the graph. The guard is what stops it writing over what the user is
    /// typing on every frame.
    fn fill_task_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.refill_fields {
            self.refill_fields = false;
            let (filter, draft) = self
                .open_project(cx)
                .map(|open| (open.board.filter.clone(), open.graph.draft.clone()))
                .unwrap_or_default();
            let task_filter = self.task_filter.clone();
            task_filter.update(cx, |state, cx| state.set_value(&filter, window, cx));
            let agent_input = self.agent_input.clone();
            agent_input.update(cx, |state, cx| state.set_value(&draft, window, cx));
        }

        let Some(board) = self.board(cx) else {
            return;
        };
        if !board.needs_fill(self.form_filled) {
            return;
        }
        let selected = board.selected;
        let (title, description) = selected
            .and_then(|id| self.work(cx).and_then(|work| work.task(id)))
            .map(|task| (task.title.clone(), task.description.clone()))
            .unwrap_or_default();

        self.form_filled = selected;
        if let Some(board) = self.board_mut(cx) {
            board.form.title = title.clone();
            board.form.description = description.clone();
            board.form.step_title.clear();
            board.form.new_step.clear();
        }
        for (input, value) in [
            (self.task_title_input.clone(), title),
            (self.step_title_input.clone(), String::new()),
            (self.new_step_input.clone(), String::new()),
        ] {
            input.update(cx, |state, cx| state.set_value(&value, window, cx));
        }
        let description_input = self.task_description_input.clone();
        description_input.update(cx, |state, cx| state.set_value(&description, window, cx));
    }

    /// Point the panel at a task. Picking a card always opens the panel, because a selection
    /// nothing reports on is not a selection.
    pub fn select_task(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.select(task);
        }
        cx.notify();
    }

    pub fn close_task_detail(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.show_detail = false;
        }
        cx.notify();
    }

    /// Which session the board is showing. `None` is every session, including the tasks that
    /// belong to none.
    pub fn pick_board_session(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.session = session;
        }
        cx.notify();
    }

    /// Ask for a task in the backlog, named by whatever is in the filter field.
    ///
    /// One field finds work and names it: what you typed to look for a card is what you meant to
    /// call it when there was none. The field is cleared, so the board is not left filtered down to
    /// the one card that was just made.
    ///
    /// It cannot select what it asked for, because the id is the host's to mint. `awaiting_new` is
    /// what selects the task that arrives — the same mechanism `AppState::adding` uses to open the
    /// project an `AddProject` answers with.
    pub fn new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(board) = self.board(cx) else {
            return;
        };
        let typed = board.filter.trim().to_string();
        let title = if typed.is_empty() {
            "New task".to_string()
        } else {
            typed
        };
        let session = board.session;
        self.bus.send(Message::CreateTask {
            project_id,
            title,
            session,
        });
        if let Some(board) = self.board_mut(cx) {
            board.filter.clear();
            board.awaiting_new = true;
        }
        let input = self.task_filter.clone();
        input.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub fn toggle_board_column(&mut self, status: Status, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.toggle_column(status);
        }
        cx.notify();
    }

    pub fn toggle_task_fold(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.toggle_fold(task);
        }
        cx.notify();
    }

    /// Tick or untick one sub-task.
    ///
    /// A toggle rather than a target state: what unticking lands on is a rule about the work, and
    /// the work is the host's. The checkbox changes when the task comes back.
    pub fn toggle_task_step(&mut self, task_id: TaskId, step_id: StepId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.bus.send(Message::ToggleStep {
            project_id,
            task_id,
            step_id,
        });
        cx.notify();
    }

    /// Pick a card up. It selects itself on the way, for the reason a dragged agent card does:
    /// what is being moved is what the user is looking at.
    pub fn start_task_carry(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.start_carry(task);
            board.select(task);
        }
        cx.notify();
    }

    /// The column under the pointer, which is what a drop would file the card into.
    pub fn drag_task_over(&mut self, status: Status, cx: &mut Context<Self>) {
        if self
            .board_mut(cx)
            .is_some_and(|board| board.carry_over(status))
        {
            cx.notify();
        }
    }

    /// Put it down. Unlike the graph's canvas, the column *is* the drop target: a card is filed
    /// somewhere rather than placed anywhere, so where it landed is what took the drop.
    ///
    /// Which column a task is in is written down, so the drop asks rather than moves. The card
    /// says it is waiting until the answer comes back, which is what keeps a slow host from
    /// reading as a drag that failed.
    pub fn drop_task(&mut self, status: Status, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(board) = self.board_mut(cx) else {
            return;
        };
        board.carry_over(status);
        if let Some((task_id, status)) = board.end_carry() {
            board.moving = Some((task_id, status));
            self.bus.send(Message::MoveTask {
                project_id,
                task_id,
                status,
            });
        }
        cx.notify();
    }

    /// Take a task to the agents screen: the graph, pointed at whoever is doing it.
    pub fn show_task_in_graph(&mut self, task: TaskId, cx: &mut Context<Self>) {
        let selection = self.work(cx).and_then(|work| {
            let task = work.task(task)?;
            work.now(task)
                .map(|agent| Selection::Agent(agent.id))
                .or_else(|| task.session.map(Selection::Session))
        });
        if let Some(selection) = selection
            && let Some(graph) = self.graph_mut(cx)
        {
            graph.selection = Some(selection);
        }
        self.set_rail_mode(RailMode::Agents, cx);
    }

    /// The way from a card to the conversation with the agent holding it: the agents screen, that
    /// agent selected, the inspector on its thread.
    pub fn open_task_chat(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.open_agent_chat(agent, cx);
        self.set_rail_mode(RailMode::Agents, cx);
    }

    /// A drag that ended anywhere but a column never reaches a drop handler, so a carry with no
    /// live drag behind it is put down here — and the card stays in the column it came from.
    fn settle_board(&mut self, cx: &mut Context<Self>) {
        let stranded = self
            .board(cx)
            .is_some_and(|board| board.carry.is_some() && !cx.has_active_drag());
        if stranded && let Some(board) = self.board_mut(cx) {
            board.carry = None;
        }
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

/// The diagram cache, and the queue that fills it.
///
/// Two tiers. The memory one is [`AppState::diagrams`], which every frame reads; the disk one is
/// [`crate::state::diagrams::Disk`], in the project's workarea, which survives a restart. Between
/// them and the renderer is the background executor: **a diagram is never drawn on the frame
/// thread**, because layout is superlinear and a large graph takes seconds.
impl AppState {
    /// What has been drawn for a source, queueing a render if nothing has been.
    ///
    /// Takes `&self` and answers a clone because it is called while the frame is being built: a
    /// viewer is a pure function of bytes and reaches no mutable window. The render is queued
    /// rather than started, and [`AppState::drain_diagram_asks`] starts it once the frame is built.
    pub fn diagram(&self, source: &str) -> DiagramEntry {
        let palette = self.diagram_palette();
        let key = diagrams::key(source, palette);

        let mut cache = self.diagrams.borrow_mut();
        if let Some(entry) = cache.get(&key) {
            return entry.clone();
        }
        cache.insert(key, DiagramEntry::Pending);
        self.diagram_asks
            .borrow_mut()
            .push((source.to_string(), palette));
        DiagramEntry::Pending
    }

    /// Which palette a diagram drawn now is drawn for. The renderer bakes its colours in, so this
    /// is part of what is asked for and part of what it is filed under.
    fn diagram_palette(&self) -> DiagramPalette {
        match self.workbench.theme_id {
            ThemeId::Dark => DiagramPalette::Dark,
            ThemeId::Light => DiagramPalette::Light,
        }
    }

    /// Draw every diagram the frame turned out to need, off the frame thread.
    ///
    /// In `render` for the reason `attach_arrived_files` is: the work belongs to the frame and
    /// cannot be done from inside it. Each render goes to the background executor and comes back
    /// as an entity update — **the window keeps drawing and keeps taking keystrokes while it
    /// runs**, and the viewer shows a pending state until it lands.
    fn drain_diagram_asks(&mut self, cx: &mut Context<Self>) {
        let asks = std::mem::take(&mut *self.diagram_asks.borrow_mut());
        if asks.is_empty() {
            return;
        }

        // The disk tier belongs to the project on screen. A window with no project yet renders
        // with the memory tier alone rather than not rendering.
        let dir = self
            .project_snapshot(cx)
            .map(|project| diagrams::cache_dir(&project.workarea));

        for (source, palette) in asks {
            let dir = dir.clone();
            let drawing =
                cx.background_spawn(async move { diagrams::resolve(&source, palette, dir) });
            cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
                let answer = drawing.await;
                // A window closed while its diagram was being drawn is not an error.
                let _ = this.update(cx, |this, cx| this.diagram_drawn(answer, cx));
            })
            .detach();
        }
    }

    /// One background render, landed.
    ///
    /// Keyed by the content key the render was filed under, so an answer finds its entry however
    /// long it took and whatever the window has done since — including a theme switch, which asks
    /// for a different key and leaves this one to be found again on the way back.
    fn diagram_drawn(&mut self, answer: DiagramAnswer, cx: &mut Context<Self>) {
        let entry = match answer.result {
            Ok(image) => DiagramEntry::Ready(diagram_picture(image)),
            Err(reason) => DiagramEntry::Failed(reason),
        };
        self.diagrams.borrow_mut().insert(answer.key, entry);
        cx.notify();
    }

    /// The camera on one picture, or the fitted default if the user has not touched it.
    pub fn viewport(&self, key: &str) -> Viewport {
        self.viewports
            .borrow()
            .get(key)
            .copied()
            .unwrap_or_default()
    }

    /// Remember the picture's own rectangle so a later wheel or drag can pin the fit.
    pub fn touch_viewport(&self, key: &str, content: Content) {
        self.viewports
            .borrow_mut()
            .entry(key.to_string())
            .or_default()
            .set_content(content);
    }

    /// Remember the panel a picture was just laid out in. Returns whether it went from unmeasured
    /// to measured, which is the one change that owes the window another frame — a resize already
    /// asked for one.
    pub fn note_viewport_panel(&self, key: &str, bounds: Bounds<Pixels>) -> bool {
        self.viewports
            .borrow_mut()
            .entry(key.to_string())
            .or_default()
            .set_panel(
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
            )
    }

    pub fn zoom_viewport(
        &mut self,
        key: &str,
        factor: f32,
        cursor: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some(vp) = self.viewports.borrow_mut().get_mut(key) {
            vp.zoom_at(factor, f32::from(cursor.x), f32::from(cursor.y));
        }
        cx.notify();
    }

    pub fn start_viewport_drag(
        &mut self,
        key: &str,
        at: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        *self.viewport_drag.borrow_mut() = Some((key.to_string(), at));
        cx.notify();
    }

    pub fn drag_viewport(&mut self, key: &str, at: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let last = {
            let drag = self.viewport_drag.borrow();
            match drag.as_ref() {
                Some((held, last)) if held == key => Some(*last),
                _ => None,
            }
        };
        let Some(last) = last else {
            return;
        };
        let dx = f32::from(at.x - last.x);
        let dy = f32::from(at.y - last.y);
        if let Some(vp) = self.viewports.borrow_mut().get_mut(key) {
            vp.pan_by(dx, dy);
        }
        *self.viewport_drag.borrow_mut() = Some((key.to_string(), at));
        cx.notify();
    }

    pub fn end_viewport_drag(&mut self, cx: &mut Context<Self>) {
        if self.viewport_drag.borrow_mut().take().is_some() {
            cx.notify();
        }
    }

    pub fn reset_viewport(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(vp) = self.viewports.borrow_mut().get_mut(key) {
            vp.reset();
        }
        cx.notify();
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The dock is settled first: a panel that arrived on a message has to be in the tree
        // before the frame that focuses it, and a restored arrangement before either. Visibility
        // leads, because installing a layout asks every panel for it.
        self.settle_visibility(cx);
        self.settle_layout(window, cx);
        self.settle_panels(window, cx);
        self.take_focus(window, cx);
        self.attach_arrived_files(window, cx);
        self.fill_task_form(window, cx);
        self.settle_graph(cx);
        self.settle_board(cx);
        // Made anonymous straight away so the frame stops borrowing the window: the queue below
        // is drained on the same `&mut self` the tree was built from.
        let tree = ui::shell::render(self, window, cx).into_any_element();
        // Diagrams a viewer found it needed while the tree was being built. Started here, where
        // the update the frame was built inside is done with `AppState` — never from inside one,
        // and never on this thread.
        self.drain_diagram_asks(cx);
        tree
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
    // The file picker's, which are the field's as well as the dialog's and have to be registered
    // after the component library's own — `ui::file_picker::key_bindings` says why.
    cx.bind_keys(crate::ui::file_picker::key_bindings());
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

/// Where a tab key sits in a project's tab order.
///
/// A free function rather than a method on [`EditorPaneState`], which keys its own lookups by path:
/// a path names a file and its diff both, and a panel names exactly one of them.
fn index_of_key(editor: &EditorPaneState, key: &str) -> Option<usize> {
    editor.open.iter().position(|file| file.key() == key)
}
