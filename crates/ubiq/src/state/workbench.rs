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
use ubiq_proto::messages::{AccountInfo, AgentTypeInfo, ShellInfo};

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
    /// The agents screen's "New agent": which harness — and which identity — a conversation is
    /// started on. Its rows are [`HarnessChoice`], and where it opened is
    /// `WorkbenchState::new_agent_menu`.
    NewAgent,
}

/// One row of the new-pane control's menu, in the order it is drawn.
///
/// The rows are here rather than in the module that paints them because the pick is matched by
/// position: the menu and the action behind it read the same list, so a row that is not offered
/// cannot be picked by an index that has shifted under it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NewPaneRow {
    /// An agent harness, by its index in [`WorkbenchState::agent_types`].
    Agent(usize),
    /// A shell, by its index in [`WorkbenchState::shells`].
    Shell(usize),
    /// The line between what starts something and what does not.
    Separator,
    /// The console, which is revealed rather than started.
    Console,
}

/// One row of the harness menu, in the order it is drawn.
///
/// Offered by every surface that can start a conversation — the agents screen's New agent and
/// the chat panel's New chat — because they start the same thing and a second list would be a
/// second answer to one question.
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
}

/// A harness and identity have been picked (from `harness_choices`); nothing has started yet —
/// the window between picking and typing a name, where leaving costs nothing, the same property
/// `LoginStep::Choosing` has.
pub struct PendingNewAgent {
    /// Index into [`WorkbenchState::agent_types`].
    pub harness: usize,
    /// The account id, which is what crosses the wire.
    pub account: Option<String>,
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
    /// Where the agents screen's "New agent" was clicked. `Some` exactly while `open_menu` is
    /// `MenuId::NewAgent`, and it reads the same [`WorkbenchState::agent_types`] the new-pane menu
    /// does: which harnesses this machine has is one answer, asked once.
    pub new_agent_menu: Option<(f32, f32)>,
    /// A harness and identity have been picked (from `harness_choices`); nothing has started yet
    /// — the window between picking and typing a name, where leaving costs nothing, the same
    /// property `LoginStep::Choosing` has.
    pub naming_agent: Option<PendingNewAgent>,
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
            new_agent_menu: None,
            naming_agent: None,
            shells: Vec::new(),
            agent_types: Vec::new(),
        }
    }
}

impl WorkbenchState {
    /// What the new-pane control's menu offers.
    ///
    /// A window with no project can start no pane — there is no folder to run one in — so it is
    /// offered the console alone rather than agents and shells that would do nothing. Agent
    /// harnesses are offered above the shells, because starting a harness is the common case and
    /// a bare shell is the fallback. Each separator is a row like any other, and there is none
    /// when there is nothing above it to separate — an empty agent list degrades to exactly the
    /// menu a window with no harnesses installed showed before agents existed.
    pub fn new_pane_rows(&self, has_project: bool) -> Vec<NewPaneRow> {
        let mut rows = Vec::new();
        if has_project {
            rows.extend((0..self.agent_types.len()).map(NewPaneRow::Agent));
            if !self.agent_types.is_empty() {
                rows.push(NewPaneRow::Separator);
            }
            rows.extend((0..self.shells.len()).map(NewPaneRow::Shell));
            if !self.shells.is_empty() {
                rows.push(NewPaneRow::Separator);
            }
        }
        rows.push(NewPaneRow::Console);
        rows
    }

    /// What a harness menu offers: every harness installed here, once per identity that can start
    /// it. One list, read by every surface that starts a conversation and by the pick behind it.
    ///
    /// **A harness with accounts is only offered with one.** Which identity a conversation runs
    /// as is fixed the moment it starts and cannot be changed after, so it is a choice worth
    /// making explicitly — and a bare row beside three named ones would be the one whose
    /// identity nobody could name. A harness with no captured login keeps its bare row, because
    /// that is the only way to start it and the library still has an answer for what it runs as.
    ///
    /// Unavailable harnesses keep their row, disabled, so the menu says a tool is missing rather
    /// than silently omitting it — the same rule the flat list followed before identities.
    pub fn harness_choices(&self, accounts: &[AccountInfo]) -> Vec<HarnessChoice> {
        let mut rows = Vec::new();
        for (index, harness) in self.agent_types.iter().enumerate() {
            let mut named = accounts
                .iter()
                .filter(|account| account.logged_in.contains(&harness.id))
                .peekable();
            if named.peek().is_none() {
                rows.push(HarnessChoice::Harness(index));
                continue;
            }
            rows.extend(named.map(|account| HarnessChoice::Pair {
                harness: index,
                account: account.id.clone(),
            }));
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

    fn harness(id: &str, available: bool) -> AgentTypeInfo {
        AgentTypeInfo {
            id: id.to_string(),
            label: id.to_string(),
            available,
        }
    }

    fn account(id: &str, logged_in: &[&str]) -> AccountInfo {
        AccountInfo {
            id: id.to_string(),
            logged_in: logged_in.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn with(agent_types: Vec<AgentTypeInfo>) -> WorkbenchState {
        WorkbenchState {
            agent_types,
            ..Default::default()
        }
    }

    /// With no accounts at all the menu is exactly the flat harness list it was before
    /// identities existed — which is what keeps a machine that has signed nothing in working.
    #[test]
    fn no_accounts_offers_the_bare_harnesses() {
        let state = with(vec![harness("claude-code", true), harness("codex", true)]);

        assert_eq!(
            state.harness_choices(&[]),
            vec![HarnessChoice::Harness(0), HarnessChoice::Harness(1)]
        );
    }

    /// A harness with identities is offered once per identity and never bare: which account a
    /// conversation runs as cannot be changed after it starts, so it is not a choice to leave
    /// implicit.
    #[test]
    fn a_harness_with_accounts_is_offered_once_per_account() {
        let state = with(vec![harness("claude-code", true)]);
        let accounts = [
            account("mdn", &["claude-code"]),
            account("syn", &["claude-code"]),
        ];

        assert_eq!(
            state.harness_choices(&accounts),
            vec![
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
            state.harness_choices(&accounts),
            vec![
                HarnessChoice::Pair {
                    harness: 0,
                    account: "both".to_string()
                },
                HarnessChoice::Pair {
                    harness: 1,
                    account: "both".to_string()
                },
                // No login anywhere, so it keeps the bare row that is the only way to start it.
                HarnessChoice::Harness(2),
            ]
        );
    }

    /// A harness whose binary is missing keeps its row so the menu can say so, disabled. It
    /// must not be omitted — a tool that vanished should read as unavailable, not as absent —
    /// and the row must still be at the position the pick will look for.
    #[test]
    fn an_unavailable_harness_keeps_its_row() {
        let state = with(vec![harness("claude-code", false), harness("codex", true)]);
        let accounts = [account("mdn", &["claude-code"])];

        assert_eq!(
            state.harness_choices(&accounts),
            vec![
                HarnessChoice::Pair {
                    harness: 0,
                    account: "mdn".to_string()
                },
                HarnessChoice::Harness(1),
            ]
        );
    }
}
