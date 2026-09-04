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
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::state::agents::{AgentsView, CHAT_SLOT, COMPOSER_SLOTS};
use crate::state::board::{BoardState, Field};
use crate::state::conversation::{Conversation, Run};
use crate::state::diagrams::{self, DiagramAnswer, DiagramImage, DiagramPalette};
use crate::state::dock::Visibility;
use crate::state::editor::{Subject, ViewLayout, from_tab_key, tab_key};
use crate::state::file_picker::{
    Commit, FilePickerState, PickKind, PickerCount, PickerKey, PickerOwner, PickerView, Pressed,
};
use crate::state::git::{GitView, RefSection, Side as GitSide};
use crate::state::orchestration::{GraphView, Held, InspectorTab, Selection};
use crate::state::settings::{
    self as ui_settings, LoginState, LoginStep, MarkdownOpen, SettingsSection,
};
use crate::state::sink::{
    ColourField, ProjectNav, SettingsMenu, SettingsNav, SinkDoc, SinkModal, SinkSection, SinkState,
};
use crate::state::viewport::{Content, Viewport};
use crate::state::work::WorkProjection;
use crate::state::{
    ActiveSearch, ChatState, EditorPaneState, ExplorerAction, ExplorerKey, ExplorerPressed,
    ExplorerState, ExplorerView, FileBody, FileLanguage, HarnessChoice, LogState, MenuId,
    NewPaneRow, OpenFile, PanelKind, PendingNewAgent, ProjectSettings, ProjectSettingsMode,
    RailMode, Region, SearchState, Toggle, WindowRegistry, WorkbenchState, prefs, sample,
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
use ubiq_proto::git::{GitEntry, GitError as GitFailure, RepoOverview};
use ubiq_proto::ids::{PaneId, ProjectId, SearchId, SessionId, StepId, TaskId};
use ubiq_proto::messages::{Message, WorkspaceInfo};
use ubiq_proto::projects::{ProjectSnapshot, Scope};
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, SettingsLayer};
use ubiq_proto::work::{AgentId, Bucket, Priority, Shape, Status};

/// How much of a file the interface asks for. The host has a ceiling of its own and this never
/// widens it; what it does is keep a buffer the user cannot read to the end of off the bus.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// One level of a folder is what an expand asks for. A deeper walk exists for revealing a path,
/// which nothing does yet.
const EXPAND_DEPTH: u8 = 1;

/// How far the background cache walks into folders nobody has opened. The host clamps this; three
/// is as far as one reply goes, and the next unlisted folders are asked for as that reply lands.
const CACHE_DEPTH: u8 = 3;

/// How long after the last keystroke a filter walk starts. Typing a letter must not walk the
/// cache on the frame; waiting this long coalesces a burst into one background walk.
const FILTER_DEBOUNCE: Duration = Duration::from_millis(100);

/// How long after the last zoom a Markdown preview is rebuilt. The rebuild throws away a parsed
/// document, so a held zoom key must not do it once per point.
const REFLOW_DEBOUNCE: Duration = Duration::from_millis(500);

gpui::actions!(ubiq, [OpenSearch, SaveFile, CloseEditor, ZoomIn, ZoomOut]);

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
    /// Whether the harness behind the pane is still running.
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
    pub explorer: ExplorerState,
    pub editor: EditorPaneState,
    /// The host's work for this project, as this window last heard it. Empty rather than absent
    /// until the `ListWork` is answered, so a project whose work has never arrived draws as empty
    /// rather than as a project with no work.
    pub work: WorkProjection,
    /// The agents screen's view of that work: which agents are in which column, and what is typed
    /// at each. Per project, for the reason the graph's view is.
    pub agents: AgentsView,
    /// The live agents running in this project, by the id every conversation message carries. The
    /// transcript outlives the harness, so an entry stays after its agent has ended and goes only
    /// with the project.
    pub conversations: HashMap<AgentId, Conversation>,
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
    /// What version control last said about this project. Absent until the host answers, and
    /// absent after an answer of "not a repository".
    pub git: Option<RepoOverview>,
    /// The working-tree map was cut at the ceiling, so the explorer is drawing a prefix.
    pub git_truncated: bool,
    /// Every path the last working-tree map had something to say about, as the pairs the host
    /// sent. The explorer keeps the projection of these; the Git screen's change lists need the
    /// pair itself, because staged-and-modified is two rows there and one badge in the tree.
    pub git_entries: Vec<GitEntry>,
    /// The Git screen's view of all of it: which sections are open, what is selected, what is
    /// typed in the commit box. Per project, for the reason the graph's view is.
    pub git_view: GitView,
}

impl OpenProject {
    /// A project this window has just taken, in the furniture it was last left in.
    fn new(prefs: prefs::ViewPrefs) -> Self {
        Self {
            panes: Vec::new(),
            focused_pane: None,
            explorer: ExplorerState::empty(),
            editor: EditorPaneState::empty(),
            work: WorkProjection::empty(),
            agents: AgentsView::default(),
            conversations: HashMap::new(),
            graph: GraphView::default(),
            board: BoardState::default(),
            prefs,
            restored: false,
            wanted: Vec::new(),
            git: None,
            git_truncated: false,
            git_entries: Vec::new(),
            git_view: GitView::new(sample::git_refs(), sample::git_history()),
        }
    }
}

/// Write what a conversation derives onto the agent record the rest of the window reads.
///
/// The badge, the ring and the token count are readings of the stream the window already holds, so
/// they are folded onto the record rather than asked for: the sidebar, the graph and the column
/// header keep their one source, and a token costs no round trip.
fn refresh_agent_record(open: &mut OpenProject, id: AgentId) {
    let Some(conversation) = open.conversations.get(&id) else {
        return;
    };
    let activity = conversation.activity();
    let context_pct = conversation.context_pct();
    let tokens = conversation.tokens() as f32;
    let model = conversation.model.clone();
    let title = conversation.title.clone();

    let Some(record) = open.work.agent_mut(id) else {
        return;
    };
    record.activity = activity;
    record.tokens = tokens;
    // A window nobody reported leaves the ring as it was rather than reading zero, which would
    // draw a full context as an empty one.
    if let Some(pct) = context_pct {
        record.context_pct = pct;
    }
    if let Some(model) = model {
        record.model = model;
    }
    // A conversation the harness has not named a title for keeps whatever name it started with —
    // today's harness-label default from registration. Once it names one, that's the record's
    // name from here on: the sidebar row, the column header and the chat panel row all read it.
    if let Some(title) = title {
        record.name = title;
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
    /// The tab key of the file editor the keyboard is owed to, once it has a window and a buffer.
    ///
    /// A file panel becoming the displayed tab is where this is asked for — `activate_file` — but
    /// the focus needs a `Window`, and the buffer may still be arriving, so it waits for the frame
    /// that can do both. See [`Self::take_editor_focus`].
    pending_editor_focus: Option<String>,

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
    /// A rail-mode switch whose mode had no arrangement to restore: which edge regions that mode's
    /// defaults put on screen, for the frame that has a window to force them with.
    pending_regions: Option<(bool, bool, bool)>,
    /// Whether the left, bottom and right regions held a visible panel as of the last layout
    /// change — the edge that tells "its content just left" apart from "it was just opened
    /// empty", so [`Self::toggle_region`] opening a region on purpose is never mistaken for the
    /// auto-hide this drives. See the `DockEvent::LayoutChanged` subscription in [`Self::new`].
    region_had_content: (bool, bool, bool),

    pub workbench: WorkbenchState,
    pub chat: ChatState,
    /// The kitchen sink's own state: which page is open, and what its controls hold. It belongs to
    /// the window rather than to a project, because the sink has no project behind it.
    pub sink: SinkState,
    /// What the log console is showing. The records themselves belong to the process-wide sink.
    pub logs: LogState,
    /// The project search panel's state: query, options, results.
    pub search: SearchState,
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
    /// A file dropped with no project open: its folder is added as `adding` above, and this is the
    /// leaf to select once the project the host answers with is actually open.
    adding_select: Option<String>,
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
    /// The inspector's composer on the orchestration screen. A field of its own rather than the
    /// chat's, because the two are two conversations and a shared draft would leak between them.
    pub agent_input: Entity<TextareaState>,
    /// One composer per slot that hosts a conversation — every column on the agents screen,
    /// plus the chat panel's — [`COMPOSER_SLOTS`] of them.
    ///
    /// A fixed pool rather than one entity per live column: an entity is created with a `Window`
    /// and columns open from handlers that have one, but the *subscription* that mirrors what is
    /// typed has to be held for the window's life — so all of them are built once, before the
    /// first frame, and a column borrows the slot it is given. `AgentsView::drafts` is the other
    /// half, indexed the same way.
    pub column_inputs: Vec<Entity<TextareaState>>,
    pub file_filter: Entity<InputState>,
    /// The Git screen's search over the log, and its commit box. The entities are the window's and
    /// the text in them is the project's, so both are mirrored into the project's `GitView`.
    pub git_search: Entity<InputState>,
    pub git_message: Entity<TextareaState>,
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
    /// The search panel's query field.
    pub search_query: Entity<InputState>,
    /// The settings dialog's two host-owned search lists, each a comma-separated line. They commit
    /// on Enter and on blur rather than on a keystroke: every commit writes a file on the host.
    pub search_excludes_input: Entity<InputState>,
    pub search_fallbacks_input: Entity<InputState>,
    /// The project settings dialog's name field. Also what a picker row used to become while
    /// renaming; that editor now lives in the dialog.
    pub rename_input: Entity<InputState>,
    /// The project settings dialog's description and custom-colour hex. Separate from the sink's
    /// fixtures, because the dialog can be up over the sink's own project page.
    pub project_form_about: Entity<TextareaState>,
    pub project_form_hex: Entity<InputState>,
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
    /// What the login modal names the identity it is about to capture. Its own field
    /// rather than a shared one, for the reason every other pair here is split: two
    /// states drawn at once would be one field in two places.
    pub login_account_input: Entity<InputState>,
    /// What the new-agent naming prompt has typed for the conversation's own name. Its own field
    /// for the same reason `login_account_input` is: a state drawn once, in its own modal.
    pub new_agent_name_input: Entity<InputState>,
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
    /// The explorer's rows, for the same reason.
    pub explorer_scroll: ScrollHandle,
    /// The agents screen's sidebar. Its own handle rather than the explorer's: the two lists are
    /// on screen in different modes and a shared handle would carry one's position into the other.
    pub agents_scroll: ScrollHandle,
    /// Incremented on every filter keystroke so a debounce that lost the race does not start a
    /// walk for a query the user has already left.
    explorer_filter_gen: u64,
    /// Bumped once the point size has settled, and part of every Markdown preview's element id.
    /// The text view caches the height it measured each block at and only reconsiders when its
    /// width changes, so a zoom reflows nothing until the preview is keyed anew.
    pub md_reflow: u64,
    /// The zoom that asked for the last reflow, so a debounce that lost the race does not rebuild
    /// a preview the user has already zoomed past.
    md_reflow_gen: u64,
    pub log_scroll: UniformListScrollHandle,
    /// Which task the panel's fields were last filled from, so a selection change refills them
    /// exactly once. Writing into the component library's state needs a window and a message does
    /// not come with one, which is why this is drained in `render` beside the arrived files.
    form_filled: Option<TaskId>,
    /// A project switch owes the window's own fields the entered project's text.
    refill_fields: bool,
    /// The agents screen's composers owe themselves a placeholder and a draft: the columns changed,
    /// or a column's tab did, and which agent a composer is addressed at is what its placeholder
    /// says. Drained in `render` for the reason `refill_fields` is.
    refill_columns: bool,
    /// The project settings dialog owes its fields the path, name and colour it was opened with.
    /// Drained in `render` for the same reason as `refill_fields`: `set_value` needs a window.
    fill_project_form: bool,
    _subscriptions: Vec<Subscription>,
}

// The screen-sized halves of `AppState`. Each is one `impl AppState` block; the struct,
// its companions and the free window functions stay here.
mod agents;
mod board;
mod boot;
mod chat;
mod editor;
mod explorer;
mod git;
mod graph;
mod panels;
mod picker;
mod projects;
mod settings;
mod shell;
mod sink;
mod wire;

/// A comma-separated line as a list: trimmed, and without the empties a trailing comma leaves.
pub fn comma_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

/// What the interface says about a path the host refused.
///
/// Each arm is a different thing for the user to do about it, which is why the contract carries an
/// enum rather than a sentence.
fn leaf_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

/// Read a guest file's prefix, since there is no host round trip for a path outside every project.
/// Mirrors `ubiq_host::files::contents`: the same stat guard against a FIFO or a device blocking
/// this thread forever, the same truncation ceiling, and the same NUL sniff for binary. There is no
/// version, because there is no project record to keep one consistent against — `OpenFile::savable`
/// is what turns that absence into an unwritable tab rather than a merely unwritten one.
fn read_guest_file(path: &Path) -> Result<FileContents, String> {
    let stat = fs::metadata(path).map_err(|error| error.to_string())?;
    if !stat.is_file() {
        return Err("not a regular file".to_string());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;

    let truncated = bytes.len() as u64 > MAX_FILE_BYTES;
    if truncated {
        bytes.truncate(MAX_FILE_BYTES as usize);
    }
    const SNIFF_BYTES: usize = 8 * 1024;
    let is_binary = bytes[..SNIFF_BYTES.min(bytes.len())].contains(&0);
    let len = fs::metadata(path).map(|m| m.len()).unwrap_or(stat.len());

    Ok(FileContents {
        bytes,
        len,
        truncated,
        is_binary,
        version: None,
    })
}

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
        gpui::KeyBinding::new("cmd-w", CloseEditor, Some("Workbench")),
        gpui::KeyBinding::new("ctrl-w", CloseEditor, Some("Workbench")),
        gpui::KeyBinding::new("cmd-=", ZoomIn, Some("Workbench")),
        gpui::KeyBinding::new("cmd-shift-=", ZoomIn, Some("Workbench")),
        gpui::KeyBinding::new("cmd--", ZoomOut, Some("Workbench")),
        gpui::KeyBinding::new("cmd-shift-f", OpenSearch, Some("Workbench")),
    ]);
    // ⌘⇧F means project search wherever the caret is, including inside a field.
    //
    // The component library binds it to *replace in this file* at the `Input` context, which is
    // deeper in the tree than `Workbench` and so wins every tie the moment any field holds focus —
    // which is most of the time, the search panel's own query bar included. Binding it again at the
    // field's own depth is what takes it back: same predicate, registered later, and this function
    // runs after `gpui_component::init`. It is the device `ui::file_picker::key_bindings` documents.
    //
    // Replace moves to ⌘⌥F rather than losing its key.
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-shift-f", OpenSearch, Some("Input")),
        gpui::KeyBinding::new("cmd-alt-f", gpui_component::input::Replace, Some("Input")),
    ]);
    // The file picker's and the explorer's, which are the field's as well as the surface's and
    // have to be registered after the component library's own — `ui::file_picker::key_bindings`
    // says why.
    cx.bind_keys(crate::ui::file_picker::key_bindings());
    cx.bind_keys(crate::ui::explorer::key_bindings());
    gpui_terminal::install_key_bindings(cx);
}

/// Release the terminal's keyboard: Shift+Escape, Ctrl+Escape, or Cmd+Escape.
/// Bare Escape still reaches the harness.
fn is_terminal_defocus(keystroke: &gpui::Keystroke) -> bool {
    if keystroke.key != "escape" {
        return false;
    }
    let m = &keystroke.modifiers;
    let shift_only = m.shift && !m.control && !m.alt && !m.platform;
    let ctrl_only = m.control && !m.shift && !m.alt && !m.platform;
    let cmd_only = m.platform && !m.control && !m.alt && !m.shift;
    shift_only || ctrl_only || cmd_only
}

/// What a pane calls itself before its harness says otherwise: the program without its path, and a
/// number, because a project with three shells in it needs three different tabs.
///
/// The number is the lowest one no pane of that program is using in that project, so closing
/// `zsh 2` gives the name back to the next one rather than counting upwards for ever. `taken` is
/// the titles already in use.
///
/// Public because it is the one part of a pane's tab that is a rule rather than a redraw, and
/// `crates/ubiq/tests/new_pane.rs` asserts it without a window.
pub fn pane_title(agent_type: &str, taken: &[String]) -> String {
    let base = agent_type.rsplit('/').next().unwrap_or(agent_type);
    (1..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !taken.iter().any(|used| used == name))
        .unwrap_or_else(|| base.to_string())
}

/// The disambiguating number `pane_title` appended, if the title still ends in one.
fn pane_title_number(title: &str) -> Option<&str> {
    title
        .rsplit_once(' ')
        .and_then(|(_, n)| (!n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).then_some(n))
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
    open_window(project, false, Vec::new(), cx)
}

/// The first window, which takes a project from the first catalogue that arrives.
///
/// The binary cannot name one: it has not asked the host what exists yet, and the interface may
/// not look for itself.
pub fn open_first_window(cx: &mut App) {
    open_window(None, true, Vec::new(), cx)
}

/// Hand paths from outside the window — the command line, a Finder open, a dock-icon drop — to a
/// window that can act on them.
///
/// The front-most window gets it; failing that, whichever the registry opened first. With no
/// window at all, one is opened the same as a cold launch, and the paths are delivered into it
/// before it asks the host for anything — `deliver_paths` runs first, so if `adopt_if_owed` later
/// adopts a remembered project from the catalogue, `Message::ProjectAdded` for the delivered path
/// still lands after and takes the window over unconditionally. The delivered path always wins.
pub fn deliver_paths_to_a_window(paths: Vec<PathBuf>, cx: &mut App) {
    if paths.is_empty() {
        return;
    }

    let target = cx
        .active_window()
        .and_then(|handle| OpenWindows::get(cx, handle.window_id()))
        .or_else(|| {
            WindowRegistry::read(cx)
                .windows
                .first()
                .and_then(|slot| OpenWindows::get(cx, slot.id))
        });

    match target {
        Some(view) => {
            view.update(cx, |state, cx| state.deliver_paths(&paths, cx));
        }
        None => open_window(None, true, paths, cx),
    }
}

fn open_window(project: Option<ProjectId>, adopt: bool, paths: Vec<PathBuf>, cx: &mut App) {
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
                if !paths.is_empty() {
                    state.deliver_paths(&paths, cx);
                }
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
    OpenWindows::unregister(id, cx);
}

/// Which window a delivered path can reach, by id.
///
/// [`WindowRegistry`] draws the picker and never needs a live handle back into a window's own
/// state; `deliver_paths_to_a_window` does, so it keeps this small map beside it instead of
/// growing the registry a concern it otherwise has no use for.
#[derive(Default)]
struct OpenWindows(HashMap<WindowId, WeakEntity<AppState>>);

impl Global for OpenWindows {}

impl OpenWindows {
    fn register(id: WindowId, view: WeakEntity<AppState>, cx: &mut App) {
        if !cx.has_global::<Self>() {
            cx.set_global(Self::default());
        }
        cx.global_mut::<Self>().0.insert(id, view);
    }

    fn unregister(id: WindowId, cx: &mut App) {
        if cx.has_global::<Self>() {
            cx.global_mut::<Self>().0.remove(&id);
        }
    }

    fn get(cx: &App, id: WindowId) -> Option<Entity<AppState>> {
        cx.try_global::<Self>()?.0.get(&id)?.upgrade()
    }
}

/// Where a tab key sits in a project's tab order.
///
/// A free function rather than a method on [`EditorPaneState`], which keys its own lookups by path:
/// a path names a file and its diff both, and a panel names exactly one of them.
fn index_of_key(editor: &EditorPaneState, key: &str) -> Option<usize> {
    editor.open.iter().position(|file| file.key() == key)
}

/// Reveal a resolved absolute path in the system's file manager. The path is a file or folder;
/// a file is revealed by opening its parent directory.
fn open_in_system(path: &str) -> std::io::Result<()> {
    let target = std::path::Path::new(path);
    let dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target
            .parent()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| target.to_path_buf())
    };
    let (cmd, arg) = if cfg!(target_os = "macos") {
        ("open", "-R")
    } else if cfg!(target_os = "windows") {
        ("explorer", "/select,")
    } else {
        ("xdg-open", "")
    };
    let mut command = std::process::Command::new(cmd);
    if !arg.is_empty() {
        command.arg(arg);
    }
    command.arg(dir).spawn().map(|_| ())
}

/// Open a URL in the system's default browser.
fn open_url(url: &str) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "windows") {
        // `start` is a cmd.exe builtin, not a standalone executable, and its first quoted arg is
        // taken as the window title rather than the target — the empty title arg keeps `url` in
        // the right position.
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    } else {
        let cmd = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let mut command = std::process::Command::new(cmd);
        command.arg(url);
        command
    };
    command.spawn().map(|_| ())
}
