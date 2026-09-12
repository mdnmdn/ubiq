//! The shell's own state: which rail mode is active, and which of the window's
//! single-open-at-a-time menus is down.
//!
//! Where the panels are is not here. The window's arrangement is the dock's own — see
//! [`super::dock`] for what a panel is and where it may sit — and asking the dock is the only way
//! to know whether a region is on screen, because the user can empty one by dragging.
//!
//! Which projects exist and which window holds which is not here — that is process-wide, and lives
//! in [`super::windows`]. What stays is what belongs to this window alone: what was typed into the
//! picker's and the explorer's search fields, and which project's close is waiting on an answer.
//!
//! Nothing about version control is here. The branch, the ahead and behind counts and the
//! working-tree totals were invented, and a fact nobody can answer for is not drawn at all.

use gpui::SharedString;
use ubiq_proto::ids::ProjectId;
use ubiq_proto::mcp::McpInfo;
use ubiq_proto::messages::{AccountInfo, AgentTypeInfo, ProfileInfo, ShellInfo};
use ubiq_proto::tools::ListedTool;
use ubiq_proto::work::AgentId;

use crate::state::PanelKind;
use crate::state::clone::CloneState;
use crate::state::remote::RemoteConnectState;
use crate::state::remote_hosts::RemoteManagerState;
use crate::state::settings::SettingsState;
use crate::state::sink::{ColourField, ProjectNav};
use crate::theme::ThemeId;

/// The left rail's destinations. `Control`, `Ide`, `Git`, `Agents`, `Orchestration`, `Tasks` and
/// `Sink` are built; the rest render an empty page.
///
/// `Agents` and `Orchestration` are two screens over the same records, and the split is the point.
/// `Agents` is where the user *talks to* the agents — parallel columns, one conversation each.
/// `Orchestration` is where the user *arranges* them — the graph of who spawned whom and which
/// task each card serves.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum RailMode {
    Control,
    Ide,
    Git,
    Agents,
    Orchestration,
    Kb,
    Tasks,
    /// The kitchen sink: the application's own test bench. The one mode with no project behind it
    /// at all — see [`super::sink`].
    Sink,
}

impl RailMode {
    /// Whether this mode is the IDE. The one mode the left rail's side panels belong to.
    pub fn is_ide(self) -> bool {
        self == RailMode::Ide
    }

    pub fn label(self) -> &'static str {
        match self {
            RailMode::Control => "Control",
            RailMode::Ide => "IDE",
            RailMode::Git => "Git",
            RailMode::Agents => "Agents",
            RailMode::Orchestration => "Teams",
            RailMode::Kb => "KB",
            RailMode::Tasks => "Tasks",
            RailMode::Sink => "Sink",
        }
    }

    /// The one-line note the empty page shows for a mode that is not built yet. `Control` keeps
    /// one because the rail's tooltip prints it too, and it now says what the screen actually
    /// draws rather than what it was going to.
    pub fn note(self) -> &'static str {
        match self {
            RailMode::Control => "What this Ubiq is doing, and what its agents have spent.",
            RailMode::Ide => "",
            RailMode::Git => "What version control knows about this project.",
            RailMode::Agents => "The agents running in this project, one column each.",
            RailMode::Orchestration => "How the agents are arranged, and which task each serves.",
            RailMode::Kb => "Notes and documents the agents can read.",
            RailMode::Tasks => "Work queued for the agents in this session.",
            RailMode::Sink => "The application's own test bench.",
        }
    }

    /// Every mode, in the order the rail draws them.
    pub fn every() -> impl Iterator<Item = RailMode> {
        Self::groups()
            .iter()
            .flat_map(|(_, modes)| modes.iter().copied())
    }

    /// The rail groups, in the order they are drawn.
    pub fn groups() -> &'static [(&'static str, &'static [RailMode])] {
        &[
            ("APP", &[RailMode::Control, RailMode::Sink]),
            (
                "PROJECT",
                &[
                    RailMode::Ide,
                    RailMode::Git,
                    RailMode::Agents,
                    RailMode::Orchestration,
                    RailMode::Kb,
                    RailMode::Tasks,
                ],
            ),
        ]
    }
}

/// What one picker row has expanded into. Only one row at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowAction {
    ConfirmForget,
}

/// Why the project settings dialog is up.
#[derive(Clone, Debug)]
pub enum ProjectSettingsMode {
    /// A folder has been chosen and is not in the catalogue yet.
    Create { path: String },
    /// The project this window is showing.
    Edit { project: ProjectId },
}

/// The project settings dialog, when it is up over the workbench.
///
/// Name, description and hex live in the window's input entities and are filled on the next
/// frame — `set_value` needs a window, and the folder chooser does not come with one.
pub struct ProjectSettings {
    pub mode: ProjectSettingsMode,
    pub colour: ColourField,
    /// The dialog's own nav, starting on General. The sink page keeps its separate
    /// [`crate::state::sink`] nav — a dialog left on Tools must not reopen the sink there.
    pub nav: ProjectNav,
}

/// The "All projects" modal, while it is up.
///
/// A picker row is one line, and the picker itself only ever shows nine of history before it
/// hands off — this is where the rest of the catalogue is read, searched and acted on. Its own
/// filter rather than a share of `project_filter`: the two fields are on screen at different
/// times, but a modal raised over the picker must not go blank because the row that opened it
/// had typed something into the other one.
#[derive(Default)]
pub struct AllProjectsState {
    pub filter: String,
}

/// Every menu in the window. Exactly one may be open, so the shell keeps a single `Option`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuId {
    Project,
    LogSubsystem,
    LogLevel,
    /// The task panel's session picker. Priority and shape are pill rows rather than menus, because
    /// three fixed values read better as the report and the control at once.
    TaskSession,
    /// The task panel's label `+`: which of the project's labels to put on, or a name that does
    /// not exist yet and the swatch to give it. A menu rather than a pill row, because unlike a
    /// kind the list is as long as the project's own vocabulary and it grows.
    TaskLabels,
    /// One agents-screen column's `+`: which benched agent to group into it. It carries the
    /// column, because a row of columns each has one and only one may be open.
    AgentBench(usize),
    /// The clone modal's two dropdowns: whose repositories are listed, and which branch the
    /// clone checks out. Two ids rather than one carrying a discriminant, because they are two
    /// controls and only one menu in the window is open at a time anyway.
    CloneConnection,
    CloneBranch,
    /// The style reference's demo dropdown. It picks nothing: the sink is where a control is
    /// looked at, and one menu in the window has to be openable with no project behind it.
    SinkPicker,
    /// The A2UI page's example picker: which surface the preview draws.
    SinkA2ui,
    /// A dropdown on the settings page. Which one is `SinkState::settings.menu`.
    SinkSettings,
    /// The explorer's right-click menu. Which row (or the empty panel) is on `ExplorerState::menu`.
    Explorer,
    /// The status bar's text-size dropdown. It offers the whole point range the chrome admits.
    FontSize,
    /// A tab's right-click menu — a file, a terminal or a chat tab. Which panel it opened on, and
    /// where, is `WorkbenchState::tab_menu`.
    Tab,
    /// The new-pane control's chevron menu: which shell a pane runs, and the console. Where it
    /// opened is `WorkbenchState::new_pane_menu`.
    NewPane,
    /// The titlebar's overflow chevron: the rarely-used commands moved off the strip to make room
    /// for it — remote connect, web export, window capture and settings. Where it opened is
    /// `WorkbenchState::overflow_menu`.
    Overflow,
    /// The `+` menu every surface that hosts a conversation raises: *New agent*, which opens the
    /// form, and *Attach existing agent*, which lists what is already running. Where it opened,
    /// which surface asked and which of its two stages is drawn is
    /// `WorkbenchState::new_agent_menu`.
    NewAgent,
    /// One conversation's three-dots lifecycle menu (Stop, Unload, Resume, Delete), by the agent
    /// it belongs to — several conversations can be on screen at once, each with its own. Where
    /// it opened is `WorkbenchState::conversation_menu`.
    ConversationLifecycle(AgentId),
    /// The orchestration toolbar's arrangement dropdown: which of `layout::Algo` the graph lays
    /// itself out in. No position of its own — one menu in the window is open at a time, and this
    /// one hangs off its own trigger.
    GraphLayout,
}

/// One row of the new-pane control's menu, in the order it is drawn.
///
/// The rows are here rather than in the module that paints them because the pick is matched by
/// position: the menu and the action behind it read the same list, so a row that is not offered
/// cannot be picked by an index that has shifted under it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewPaneRow {
    /// The heading over the detached group, drawn only when it has at least one row under it.
    /// Disabled and unclickable — the same decoration `HarnessChoice::Label` is — but still a row,
    /// because the pick that follows it is an index into this very list.
    DetachedHeading,
    /// A pane no panel currently draws, by its index into the detached list the caller passed
    /// `new_pane_rows` — that list lives on `AppState`, not here, so unlike `Agent` this index
    /// resolves against the caller's own copy rather than a field of `WorkbenchState`.
    Detached(usize),
    /// An agent harness, by its index in [`WorkbenchState::agent_types`].
    Agent(usize),
    /// A shell, by its index in [`WorkbenchState::shells`].
    Shell(usize),
    /// A runnable tool, by its index in [`WorkbenchState::tools`].
    Tool(usize),
    /// The line between what starts something and what does not.
    Separator,
    /// The console, which is revealed rather than started.
    Console,
}

/// One row of the titlebar's overflow menu, in the order it is drawn.
///
/// Here rather than in the module that paints it, for the same reason [`NewPaneRow`] is: the pick
/// is matched by position, so a row that is not offered must not shift the index of the one after
/// it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverflowRow {
    RemoteConnect,
    WebExport,
    CaptureWindow,
    Settings,
}

/// One row of the harness menu, in the order it is drawn.
///
/// Here rather than in the module that paints it for the same reason [`NewPaneRow`] is: the
/// pick is matched by position, so the menu and the action behind it must read one list.
///
/// A flat list rather than a submenu, because the kit has no submenu and the pick is an index —
/// the same reason `NewPaneRow` flattens shells and harnesses into one sequence.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HarnessChoice {
    /// A harness with no identity to choose from, by its index in
    /// [`WorkbenchState::agent_types`]. What it runs as is then the library's answer — a
    /// profile, or the user's own home.
    Harness(usize),
    /// A harness and the identity to run it as: the pair the interface calls a harness.
    Pair {
        /// Index into [`WorkbenchState::agent_types`].
        harness: usize,
        /// The account id, which is what crosses the wire.
        account: String,
    },
    /// A saved setup, by its index in [`crate::state::settings::SettingsState::profiles`]. It
    /// names its own harness, identity, model and mode — everything the start needs.
    Profile(usize),
    /// A heading or a hairline: drawn, never picked. It holds an index because a menu's rows and
    /// the actions behind them are matched by position, which is what keeps `on_pick(index)`
    /// honest once the list has groups.
    Label(SharedString),
    Separator,
}

/// The question a file gesture is asking, while one is up. One at a time, the rule
/// `AccountDialog` already follows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialog {
    /// Naming something new inside `parent`. Empty `parent` is the project's root. `ext` is the
    /// extension the typed name is forced to carry — set from the menu action that raised this, so
    /// a drawing made as `New Excalidraw` cannot become a text file by deleting the suffix.
    New {
        parent: String,
        dir: bool,
        ext: Option<String>,
    },
    /// Renaming `path`, seeded with its leaf name.
    Rename { path: String },
    /// A tab's own name, typed over whatever it is currently showing — a terminal's pane title or
    /// a chat's agent name. `current` is what the field is seeded with, since neither is a fact
    /// `kind` alone can answer without a window in hand.
    RenameTab { kind: PanelKind, current: String },
    /// Removing `path`. `trash` is false when Shift was held, and the wording and the button say
    /// which one it is rather than leaving the user to know.
    Remove {
        path: String,
        dir: bool,
        trash: bool,
    },
    /// A drag that would move `path` into the folder `into`. Only ever raised for a folder.
    Move { path: String, into: String },
    /// An untitled buffer asking where to be saved. `key` is its tab key.
    SaveAs { key: String },
    /// The image Text tool's click, waiting on its string. `key` is its tab key; the point
    /// travels on the capture itself, set by the click that raised this.
    ImageText { key: String },
    /// A file tab holding unsaved changes, asked before its buffer is dropped. `key` is its tab
    /// key.
    DiscardChanges { key: String },
    /// ⌘N while the clipboard holds an image: paste it as an untitled picture, or open the
    /// text buffer the keystroke has always meant. Carries no bytes — they are re-read on the
    /// answer, so a per-frame clone never carries them.
    PasteImage,
    /// The window's close, asked while any project it holds has unsaved files, running
    /// terminals or running agents. What each of them holds is counted when the dialog is drawn.
    /// `quitting` is the same question asked for the whole application — ⌘Q — which takes every
    /// window with it.
    CloseWindow { quitting: bool },
    /// One project's close, asked for the same reasons and answered in the same modal — the close
    /// in the project menu takes the window's unsaved files, running terminals and running agents
    /// just as seriously, it only has one project to say it about.
    CloseProject { project: ProjectId },
}

pub struct WorkbenchState {
    /// Where the host writes everything down, and whether that is the usual place. The status bar
    /// says so when it is not, because a config root you cannot see is a foot-gun.
    pub config_root: Option<String>,
    pub config_root_is_default: bool,

    pub rail_mode: RailMode,
    pub theme_id: ThemeId,
    /// The interface-scope preference keys this build does not know, carried between the blob it
    /// read and the blob it writes. `remember_interface` builds a fresh `InterfacePrefs`, so
    /// without somewhere to keep them they would be dropped on the first write — see
    /// `state/prefs.rs`'s `rest`.
    pub interface_rest: std::collections::BTreeMap<String, serde_json::Value>,
    /// What the last conversation was started on. Read back from the interface's preferences and
    /// written whenever one starts, so an empty chat tab opens on the last thing that worked
    /// rather than on whatever happens to be first in the list.
    pub last_start: Option<crate::state::prefs::LastStart>,
    pub open_menu: Option<MenuId>,

    /// What was typed into the project menu's search field.
    pub project_filter: String,
    /// A row expanded into a Forget confirmation.
    pub row_action: Option<(ProjectId, RowAction)>,
    /// Project settings, raised over the window to create a project or edit the one on screen.
    pub project_settings: Option<ProjectSettings>,
    /// The clone modal, while it is up. Beside `project_settings` rather than a mode of it: a
    /// clone has no project yet, and the two questions — "which repository" and "what is this
    /// project called here" — are asked in different places.
    pub clone_project: Option<CloneState>,
    /// The "All projects" modal, while it is up. Raised from the picker's History group when it
    /// hides more than it shows — beside `clone_project` for the same reason: a question raised
    /// over the window, answered from its own state rather than the picker's.
    pub all_projects: Option<AllProjectsState>,
    /// The New agent modal, while it is up. Beside `clone_project` because it is the same kind of
    /// thing: a question raised over the window, answered once, and carrying its own pickers'
    /// open state because a modal is redrawn from state on every frame.
    pub new_agent: Option<crate::state::new_agent::NewAgentForm>,
    /// What a start still has to say to a harness, by the conversation it was started for.
    ///
    /// A start composes a preamble — the subagent ceiling as a directive, and the opening prompt
    /// the form was given — and **does not send it**: a turn sent before the user has said
    /// anything opens the transcript on words the user never wrote. It is held here instead, and
    /// the composer's send path takes it and puts it in front of the first thing the user sends,
    /// so the harness reads it and the transcript does not show it. See
    /// `crate::state::new_agent::fold_preamble`.
    ///
    /// One entry per conversation, taken on first use and never re-added: it is a preamble, not a
    /// standing prefix.
    pub agent_preambles: std::collections::HashMap<AgentId, String>,
    /// The "Connect to a remote host" modal, while it is up. Beside `clone_project` for the same
    /// reason: raised from the titlebar rather than from settings, and answering a question that
    /// has nothing to do with any project on screen.
    pub remote_connect: Option<RemoteConnectState>,
    /// The "Remote hosts" manager panel, while it is up. The connections it lists live on the
    /// bus and in the settings record — this is only the panel's own test outcomes.
    pub remote_manager: RemoteManagerState,
    /// Application settings, raised from the titlebar's gear. Interface-wide, so it opens with
    /// no project.
    pub settings: SettingsState,
    /// The last thing the host refused to do, shown at the top of the picker until dismissed.
    pub project_error: Option<String>,
    /// The last thing the host refused to do to the work, drawn at the top of the task panel by
    /// `ui::board::form::refusal`. Its own field rather than `project_error`, because that one is
    /// drawn at the top of the project picker and a task that would not move is not a fact about
    /// the catalogue — it has to be said where the user is looking. Cleared by the next thing the
    /// host confirms.
    pub work_error: Option<String>,

    /// What was typed into the explorer's "Go to file…" field. It belongs to the window rather than
    /// to a tree, because one field filters whichever project is on screen.
    pub file_filter: String,
    /// The tab whose right-click menu is open, and where the click went down. The menu is one at a
    /// time, so this is a single `Option` like `open_menu`; the panel names the tab, the point
    /// anchors the `context_menu` over the window.
    pub tab_menu: Option<(PanelKind, (f32, f32))>,
    /// The file question that is up, if one is. One at a time, and drawn from the window's root so
    /// that both the explorer and the editor's save-as reach it.
    pub file_dialog: Option<FileDialog>,
    /// Until when a folder move skips its confirmation, from the dialog's checkbox. In memory and
    /// per window: ten minutes is not a preference, and there is nothing to migrate.
    pub move_unasked_until: Option<std::time::Instant>,
    /// Where the new-pane menu's chevron was clicked, which is what anchors the menu over the
    /// window. `Some` exactly while `open_menu` is `MenuId::NewPane`.
    pub new_pane_menu: Option<(f32, f32)>,
    /// Where the titlebar's overflow chevron was clicked, which is what anchors the menu over the
    /// window. `Some` exactly while `open_menu` is `MenuId::Overflow`.
    pub overflow_menu: Option<(f32, f32)>,
    /// The `+` menu, while it is down. `Some` exactly while `open_menu` is `MenuId::NewAgent`.
    pub new_agent_menu: Option<NewAgentMenu>,
    /// Where a conversation's three-dots menu was clicked. `Some` exactly while `open_menu` is
    /// `MenuId::ConversationLifecycle(_)` — the agent it belongs to is carried on that `MenuId`
    /// itself rather than duplicated here.
    pub conversation_menu: Option<(f32, f32)>,
    /// The conversation Delete asked to confirm — destructive and irreversible, so it is not fired
    /// on the click. `None` when no confirm is up.
    pub confirm_end_conversation: Option<AgentId>,
    /// The shells the host says this machine has, in the order the menu offers them. Empty until
    /// the host answers — a window asks as it attaches and again every time the menu opens, so a
    /// shell installed since is offered without a restart.
    pub shells: Vec<ShellInfo>,
    /// The agent harnesses the host says can be started here, in the order the menu offers them
    /// above the shells. Empty until the host answers — asked alongside [`Message::ListShells`]
    /// for the same reason: a harness installed since is offered without a restart.
    ///
    /// [`Message::ListShells`]: ubiq_proto::messages::Message::ListShells
    pub agent_types: Vec<AgentTypeInfo>,
    /// The MCP servers this build offers to inject into a harness, from
    /// [`Message::Mcps`]. Empty until the host answers, and every form that offers them draws
    /// nothing rather than inventing a row.
    ///
    /// **One list for the window, not one per form.** What Ubiq can inject is a property of the
    /// build the host is running, not of the harness, the account or the setup being filled in —
    /// it does not change while the window is open, and a copy on each form would be the same
    /// answer stored twice and refreshed at two different moments.
    ///
    /// [`Message::Mcps`]: ubiq_proto::messages::Message::Mcps
    pub mcps: Vec<McpInfo>,
    /// The runnable tools the host lists: the machine-wide rows first, then the current
    /// project's. The menu offers the applicable ones below the shells. Empty until the host
    /// answers — asked with the shells and harnesses every time the menu opens, so a tool
    /// added in the settings is offered without a restart.
    pub tools: Vec<ListedTool>,
    /// Whether the explorer's bookmarks section is open. Furniture, so it is not written down.
    pub bookmarks_open: bool,
}

/// Which surface raised the `+` menu, and so where whatever it produces lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewAgentSurface {
    /// The agents screen: a pick opens a column.
    Agents,
    /// The IDE's chat strip: a pick opens a chat tab, whichever row it was. The strip's `+` means
    /// "add a view" and keeps meaning it — the menu only answers what the view is looking at.
    Chat,
    /// The kitchen sink's bench, which reads one conversation at a time.
    Sink,
}

/// The `+` menu while it is down: where it opened, who asked, and which stage is drawn.
///
/// Two stages rather than one flat list, because the two rows ask different kinds of question —
/// *New agent* raises a form, *Attach existing agent* opens a list that can run to every
/// conversation in the project, and a list that long under a row that is not it reads as the menu
/// having only one real answer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NewAgentMenu {
    pub at: (f32, f32),
    pub surface: NewAgentSurface,
    /// Whether the second stage — the list of conversations — is what is drawn.
    pub attach: bool,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            config_root: None,
            config_root_is_default: true,
            rail_mode: RailMode::Ide,
            theme_id: ThemeId::DARK,
            interface_rest: Default::default(),
            last_start: None,
            open_menu: None,
            project_filter: String::new(),
            row_action: None,
            project_settings: None,
            clone_project: None,
            all_projects: None,
            new_agent: None,
            agent_preambles: Default::default(),
            remote_connect: None,
            remote_manager: RemoteManagerState::default(),
            settings: SettingsState::default(),
            project_error: None,
            work_error: None,
            file_filter: String::new(),
            tab_menu: None,
            file_dialog: None,
            move_unasked_until: None,
            new_pane_menu: None,
            overflow_menu: None,
            new_agent_menu: None,
            conversation_menu: None,
            confirm_end_conversation: None,
            shells: Vec::new(),
            agent_types: Vec::new(),
            mcps: Vec::new(),
            tools: Vec::new(),
            bookmarks_open: false,
        }
    }
}

impl WorkbenchState {
    /// What the new-pane control's menu offers.
    ///
    /// A window with no project can start no pane — there is no folder to run one in — so it is
    /// offered the console alone rather than anything that would do nothing. Detached panes come
    /// first, above the agents group: reattaching one is picking up work already running, which
    /// reads before starting something new. Agent harnesses are offered above the shells, because
    /// starting a harness is the common case and a bare shell is the fallback; runnable tools come
    /// below the shells, because they are the specific case. Only applicable tools are rows — a
    /// macOS-only row on Windows is not offered. Each separator is a row like any other, and there
    /// is none when there is nothing above it to separate — an empty agent list degrades to
    /// exactly the menu a window with no harnesses installed showed before agents existed, and no
    /// detached pane degrades to exactly the menu before detaching existed.
    ///
    /// `detached_count` is the length of the caller's own detached-pane list — that list lives on
    /// `AppState`, which `state/` holds no reference to, so it is threaded in the same way
    /// `has_project` already is rather than read from a field here.
    pub fn new_pane_rows(&self, has_project: bool, detached_count: usize) -> Vec<NewPaneRow> {
        let mut rows = Vec::new();
        if detached_count > 0 {
            rows.push(NewPaneRow::DetachedHeading);
            rows.extend((0..detached_count).map(NewPaneRow::Detached));
            rows.push(NewPaneRow::Separator);
        }
        if has_project {
            rows.extend((0..self.agent_types.len()).map(NewPaneRow::Agent));
            if !self.agent_types.is_empty() {
                rows.push(NewPaneRow::Separator);
            }
            rows.extend((0..self.shells.len()).map(NewPaneRow::Shell));
            if !self.shells.is_empty() {
                rows.push(NewPaneRow::Separator);
            }
            let tools: Vec<usize> = self
                .tools
                .iter()
                .enumerate()
                .filter(|(_, tool)| tool.applicable)
                .map(|(index, _)| index)
                .collect();
            rows.extend(tools.into_iter().map(NewPaneRow::Tool));
            if self.tools.iter().any(|tool| tool.applicable) {
                rows.push(NewPaneRow::Separator);
            }
        }
        rows.push(NewPaneRow::Console);
        rows
    }

    /// What the titlebar's overflow menu offers.
    ///
    /// `has_project` and `capture_offered` are asked of the caller rather than read from `self`
    /// for the reason `new_pane_rows` takes `has_project`: this is plain data, and neither the
    /// project nor the capture backend is a fact `WorkbenchState` itself can answer.
    pub fn overflow_rows(&self, has_project: bool, capture_offered: bool) -> Vec<OverflowRow> {
        let mut rows = vec![OverflowRow::RemoteConnect];
        if has_project {
            rows.push(OverflowRow::WebExport);
        }
        if has_project && capture_offered {
            rows.push(OverflowRow::CaptureWindow);
        }
        rows.push(OverflowRow::Settings);
        rows
    }

    /// What a harness menu offers: one row per identity signed into a harness, and one per saved
    /// setup — grouped so both are legible, read by every surface that offers a list of them and
    /// by the pick behind it.
    ///
    /// **A bare harness is no longer a row.** Starting one with nothing else answered is what the
    /// New agent form is for, and it asks the identity, the model, the level and the mode in the
    /// same breath; a row that started a harness on whatever the library happened to resolve was
    /// the same launch with every question skipped. So the `Default` group — every
    /// [`HarnessChoice::Harness`] row, its heading and its separator — is gone, and what is left
    /// is the two groups that name something the user set up.
    ///
    /// Unavailable harnesses are still what a row draws disabled over, so a list says a tool is
    /// missing rather than silently omitting it.
    ///
    /// **A harness that cannot converse is omitted entirely**, unavailable ones notwithstanding:
    /// the two absences say different things. "Not installed" is worth drawing disabled, because
    /// installing it is the fix; "has no structured bridge" is not something the reader can act
    /// on, and the harness is not missing — it still runs perfectly well in a pane, which is where
    /// [`Self::new_pane_rows`] keeps offering it. Indices stay indices into `agent_types`, so the
    /// two menus read one list.
    pub fn harness_choices(
        &self,
        accounts: &[AccountInfo],
        profiles: &[ProfileInfo],
    ) -> Vec<HarnessChoice> {
        let conversable: Vec<usize> = self
            .agent_types
            .iter()
            .enumerate()
            .filter(|(_, harness)| harness.chat)
            .map(|(index, _)| index)
            .collect();

        let pairs: Vec<HarnessChoice> = conversable
            .iter()
            .map(|&index| (index, &self.agent_types[index]))
            .flat_map(|(index, harness)| {
                accounts
                    .iter()
                    .filter(move |account| account.logged_in.contains(&harness.id))
                    .map(move |account| HarnessChoice::Pair {
                        harness: index,
                        account: account.id.clone(),
                    })
            })
            .collect();

        let mut rows: Vec<HarnessChoice> = Vec::new();
        if !pairs.is_empty() {
            rows.push(HarnessChoice::Label("Configured".into()));
            rows.extend(pairs);
        }
        if !profiles.is_empty() {
            if !rows.is_empty() {
                rows.push(HarnessChoice::Separator);
            }
            rows.push(HarnessChoice::Label("Defined".into()));
            rows.extend((0..profiles.len()).map(HarnessChoice::Profile));
        }
        rows
    }

    /// Whether the explorer and the chat are on screen at all. They are IDE furniture and leave
    /// together — every other panel outlives a rail-mode switch, and the centre panel is what the
    /// mode actually selects between.
    pub fn is_ide(&self) -> bool {
        self.rail_mode.is_ide()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::quota::QuotaSource;

    fn harness(id: &str, available: bool) -> AgentTypeInfo {
        AgentTypeInfo {
            id: id.to_string(),
            label: id.to_string(),
            command: id.to_string(),
            available,
            chat: true,
            modes: Vec::new(),
            unattended_mode: None,
            keeps_sessions: true,
            quota: QuotaSource::default(),
        }
    }

    fn account(id: &str, logged_in: &[&str]) -> AccountInfo {
        AccountInfo {
            id: id.to_string(),
            logged_in: logged_in.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn profile(id: &str, agent_type: &str) -> ProfileInfo {
        ProfileInfo {
            id: id.to_string(),
            agent_type: agent_type.to_string(),
            account: None,
            model: None,
            mode: None,
            thinking: None,
            max_subagents: None,
            prompt: None,
            mcps: Vec::new(),
        }
    }

    fn with(agent_types: Vec<AgentTypeInfo>) -> WorkbenchState {
        WorkbenchState {
            agent_types,
            ..Default::default()
        }
    }

    /// With nothing signed in and nothing saved there is no list at all. A bare harness is not a
    /// row any more — the New agent form is what starts one — so a machine that has configured
    /// nothing has nothing to offer here rather than a group of unanswered launches.
    #[test]
    fn nothing_configured_offers_nothing() {
        let state = with(vec![harness("claude-code", true), harness("codex", true)]);

        assert_eq!(state.harness_choices(&[], &[]), Vec::new());
    }

    /// Signing in is what puts rows on the list: one `Configured` group, its heading a decoration
    /// at a position the pick must skip, and no `Default` group over it.
    #[test]
    fn accounts_are_the_configured_group() {
        let state = with(vec![harness("claude-code", true)]);
        let accounts = [
            account("mdn", &["claude-code"]),
            account("syn", &["claude-code"]),
        ];

        assert_eq!(
            state.harness_choices(&accounts, &[]),
            vec![
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "mdn".to_string()
                },
                HarnessChoice::Pair {
                    harness: 0,
                    account: "syn".to_string()
                },
            ]
        );
    }

    /// An account is only offered for the harnesses it actually has a login for. One account
    /// serving two harnesses is normal, and an account that serves neither offers nothing.
    #[test]
    fn an_account_is_only_offered_where_it_is_signed_in() {
        let state = with(vec![
            harness("claude-code", true),
            harness("codex", true),
            harness("copilot", true),
        ]);
        let accounts = [
            account("both", &["claude-code", "codex"]),
            account("byenv", &[]),
        ];

        assert_eq!(
            state.harness_choices(&accounts, &[]),
            vec![
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "both".to_string()
                },
                HarnessChoice::Pair {
                    harness: 1,
                    account: "both".to_string()
                },
            ]
        );
    }

    /// A saved setup adds a second, "Defined" group — and it appears with no account signed in at
    /// all, since a profile carries its own identity. `Configured` stays absent in that case:
    /// an empty heading is worse than none, which is the rule both groups follow.
    #[test]
    fn profiles_add_a_defined_group_of_their_own() {
        let state = with(vec![harness("codex", true)]);
        let profiles = [profile("reviewer", "codex"), profile("writer", "codex")];

        assert_eq!(
            state.harness_choices(&[], &profiles),
            vec![
                HarnessChoice::Label("Defined".into()),
                HarnessChoice::Profile(0),
                HarnessChoice::Profile(1),
            ]
        );
    }

    /// The whole point of matching by position: once the decorations are counted in, a `Pair`'s
    /// index in the full list still names the same `(harness, account)` the row shows.
    #[test]
    fn a_pairs_index_in_the_full_list_still_resolves_to_it() {
        let state = with(vec![harness("claude-code", true), harness("codex", true)]);
        let accounts = [account("mdn", &["codex"])];

        let rows = state.harness_choices(&accounts, &[]);
        assert_eq!(
            rows[1],
            HarnessChoice::Pair {
                harness: 1,
                account: "mdn".to_string()
            }
        );
    }

    /// A harness with no structured bridge is not offered as a conversation — it would be started
    /// and then never say anything. It is not *missing*, though: it keeps its place in
    /// `agent_types` and its row in the new-pane menu, where a terminal is exactly what it wants.
    #[test]
    fn a_harness_that_cannot_converse_is_not_offered_as_one() {
        let mut grok = harness("grok", true);
        grok.chat = false;
        let state = with(vec![
            harness("claude-code", true),
            grok,
            harness("codex", true),
        ]);
        let accounts = [account("mdn", &["claude-code", "grok", "codex"])];

        assert_eq!(
            state.harness_choices(&accounts, &[]),
            vec![
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "mdn".to_string()
                },
                HarnessChoice::Pair {
                    harness: 2,
                    account: "mdn".to_string()
                },
            ],
            "the indices are still positions in `agent_types`, gap and all"
        );
        assert_eq!(
            state
                .new_pane_rows(true, 0)
                .iter()
                .filter(|row| matches!(row, NewPaneRow::Agent(_)))
                .count(),
            3,
            "a pane can still run it"
        );
    }

    /// A logged-in identity for a harness that cannot converse adds no row either: the pair would
    /// name a conversation that cannot happen.
    #[test]
    fn an_account_on_a_non_chat_harness_adds_no_pair() {
        let mut grok = harness("grok", true);
        grok.chat = false;
        let state = with(vec![grok]);
        let accounts = [account("mdn", &["grok"])];

        assert_eq!(state.harness_choices(&accounts, &[]), Vec::new());
    }
}
