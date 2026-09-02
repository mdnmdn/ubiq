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

use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::ShellInfo;

use crate::state::settings::SettingsState;
use crate::theme::ThemeId;

/// The left rail's destinations. `Ide`, `Git`, `Agents`, `Orchestration`, `Tasks` and `Sink` are
/// built; the rest render an empty page.
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
            RailMode::Orchestration => "Orchestration",
            RailMode::Kb => "KB",
            RailMode::Tasks => "Tasks",
            RailMode::Sink => "Sink",
        }
    }

    /// The one-line note the empty page shows for a mode that is not built yet.
    pub fn note(self) -> &'static str {
        match self {
            RailMode::Control => "Sessions, workspaces and the agents running in them.",
            RailMode::Ide => "",
            RailMode::Git => "What version control knows about this project.",
            RailMode::Agents => "The agents running in this project, one column each.",
            RailMode::Orchestration => "How the agents are arranged, and which task each serves.",
            RailMode::Kb => "Notes and documents the agents can read.",
            RailMode::Tasks => "Work queued for the agents in this session.",
            RailMode::Sink => "The application's own test bench.",
        }
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
    pub colour: usize,
    pub custom: Option<u32>,
    pub picker_open: bool,
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
}

/// Every menu in the window. Exactly one may be open, so the shell keeps a single `Option`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuId {
    Project,
    Harness,
    Model,
    Thinking,
    Mode,
    LogSubsystem,
    LogLevel,
    /// The task panel's session picker. Priority and shape are pill rows rather than menus, because
    /// three fixed values read better as the report and the control at once.
    TaskSession,
    /// One agents-screen column's `+`: which benched agent to group into it. It carries the
    /// column, because a row of columns each has one and only one may be open.
    AgentBench(usize),
    /// The style reference's demo dropdown. It picks nothing: the sink is where a control is
    /// looked at, and one menu in the window has to be openable with no project behind it.
    SinkPicker,
    /// A dropdown on the settings page. Which one is `SinkState::settings.menu`.
    SinkSettings,
    /// The explorer's right-click menu. Which row (or the empty panel) is on `ExplorerState::menu`.
    Explorer,
    /// The status bar's text-size dropdown. It offers the whole point range the chrome admits.
    FontSize,
    /// The file tab's right-click menu. Which tab it opened on, and where, is
    /// `WorkbenchState::file_tab_menu`.
    FileTab,
    /// The new-pane control's chevron menu: which shell a pane runs, and the console. Where it
    /// opened is `WorkbenchState::new_pane_menu`.
    NewPane,
}

/// One row of the new-pane control's menu, in the order it is drawn.
///
/// The rows are here rather than in the module that paints them because the pick is matched by
/// position: the menu and the action behind it read the same list, so a row that is not offered
/// cannot be picked by an index that has shifted under it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewPaneRow {
    /// A shell, by its index in [`WorkbenchState::shells`].
    Shell(usize),
    /// The line between what starts something and what does not.
    Separator,
    /// The console, which is revealed rather than started.
    Console,
}

pub struct WorkbenchState {
    /// Where the host writes everything down, and whether that is the usual place. The status bar
    /// says so when it is not, because a config root you cannot see is a foot-gun.
    pub config_root: Option<String>,
    pub config_root_is_default: bool,

    pub rail_mode: RailMode,
    pub theme_id: ThemeId,
    pub open_menu: Option<MenuId>,

    /// What was typed into the project menu's search field.
    pub project_filter: String,
    /// A project whose close is waiting on an answer, because it still has terminals open.
    pub pending_close: Option<ProjectId>,
    /// A row expanded into a Forget confirmation.
    pub row_action: Option<(ProjectId, RowAction)>,
    /// Project settings, raised over the window to create a project or edit the one on screen.
    pub project_settings: Option<ProjectSettings>,
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
    /// The file tab whose right-click menu is open, and where the click went down. The menu is one
    /// at a time, so this is a single `Option` like `open_menu`; the tab key names the file, the
    /// point anchors the `context_menu` over the window.
    pub file_tab_menu: Option<(String, (f32, f32))>,
    /// Where the new-pane menu's chevron was clicked, which is what anchors the menu over the
    /// window. `Some` exactly while `open_menu` is `MenuId::NewPane`.
    pub new_pane_menu: Option<(f32, f32)>,
    /// The shells the host says this machine has, in the order the menu offers them. Empty until
    /// the host answers — a window asks as it attaches and again every time the menu opens, so a
    /// shell installed since is offered without a restart.
    pub shells: Vec<ShellInfo>,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            config_root: None,
            config_root_is_default: true,
            rail_mode: RailMode::Ide,
            theme_id: ThemeId::Dark,
            open_menu: None,
            project_filter: String::new(),
            pending_close: None,
            row_action: None,
            project_settings: None,
            settings: SettingsState::default(),
            project_error: None,
            work_error: None,
            file_filter: String::new(),
            file_tab_menu: None,
            new_pane_menu: None,
            shells: Vec::new(),
        }
    }
}

impl WorkbenchState {
    /// What the new-pane control's menu offers.
    ///
    /// A window with no project can start no pane — there is no folder to run one in — so it is
    /// offered the console alone rather than shells that would do nothing. The separator is a row
    /// like any other, and there is none when there is nothing above it to separate.
    pub fn new_pane_rows(&self, has_project: bool) -> Vec<NewPaneRow> {
        let mut rows = Vec::new();
        if has_project {
            rows.extend((0..self.shells.len()).map(NewPaneRow::Shell));
            if !self.shells.is_empty() {
                rows.push(NewPaneRow::Separator);
            }
        }
        rows.push(NewPaneRow::Console);
        rows
    }

    /// Whether the explorer and the chat are on screen at all. They are IDE furniture and leave
    /// together — every other panel outlives a rail-mode switch, and the centre panel is what the
    /// mode actually selects between.
    pub fn is_ide(&self) -> bool {
        self.rail_mode.is_ide()
    }
}
