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
use ubiq_proto::messages::{AccountInfo, AgentTypeInfo, ShellInfo};
use ubiq_proto::work::AgentId;

use crate::state::settings::SettingsState;
use crate::state::sink::ColourField;
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
            RailMode::Orchestration => "Teams",
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
    /// One conversation's three-dots lifecycle menu (Stop, Unload, Resume, Delete), by the agent
    /// it belongs to — several conversations can be on screen at once, each with its own. Where
    /// it opened is `WorkbenchState::conversation_menu`.
    ConversationLifecycle(AgentId),
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
    /// Naming something new inside `parent`. Empty `parent` is the project's root.
    New { parent: String, dir: bool },
    /// Renaming `path`, seeded with its leaf name.
    Rename { path: String },
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
    /// A file tab holding unsaved changes, asked before its buffer is dropped. `key` is its tab
    /// key.
    DiscardChanges { key: String },
    /// The window's close, asked while any project it holds has unsaved files or running
    /// terminals. What each of them holds is counted when the dialog is drawn. `quitting` is the
    /// same question asked for the whole application — ⌘Q — which takes every window with it.
    CloseWindow { quitting: bool },
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
    /// The file question that is up, if one is. One at a time, and drawn from the window's root so
    /// that both the explorer and the editor's save-as reach it.
    pub file_dialog: Option<FileDialog>,
    /// Until when a folder move skips its confirmation, from the dialog's checkbox. In memory and
    /// per window: ten minutes is not a preference, and there is nothing to migrate.
    pub move_unasked_until: Option<std::time::Instant>,
    /// Where the new-pane menu's chevron was clicked, which is what anchors the menu over the
    /// window. `Some` exactly while `open_menu` is `MenuId::NewPane`.
    pub new_pane_menu: Option<(f32, f32)>,
    /// Where the agents screen's "New agent" was clicked. `Some` exactly while `open_menu` is
    /// `MenuId::NewAgent`, and it reads the same [`WorkbenchState::agent_types`] the new-pane menu
    /// does: which harnesses this machine has is one answer, asked once.
    pub new_agent_menu: Option<(f32, f32)>,
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
    /// Whether the explorer's bookmarks section is open. Furniture, so it is not written down.
    pub bookmarks_open: bool,
}

impl Default for WorkbenchState {
    fn default() -> Self {
        Self {
            config_root: None,
            config_root_is_default: true,
            rail_mode: RailMode::Ide,
            theme_id: ThemeId::Dark,
            interface_rest: Default::default(),
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
            file_dialog: None,
            move_unasked_until: None,
            new_pane_menu: None,
            new_agent_menu: None,
            conversation_menu: None,
            confirm_end_conversation: None,
            shells: Vec::new(),
            agent_types: Vec::new(),
            bookmarks_open: false,
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

    /// What a harness menu offers: every installed harness bare, plus one row per identity signed
    /// into one — grouped so both are legible, read by every surface that starts a conversation
    /// and by the pick behind it.
    ///
    /// **Signing in must not take away the zero-config start.** A harness with no captured login
    /// is the only "Default" row it can be, and that stays true once accounts exist: which
    /// identity a conversation runs as is fixed the moment it starts, so naming one is a choice
    /// worth making explicitly, not a default a login should have taken away silently. So every
    /// available harness keeps its bare `Default` row regardless of what accounts exist, and each
    /// logged-in identity adds its own row in a second, `Configured` group below a separator —
    /// omitted entirely, heading included, when nothing is signed in: a lone empty heading is
    /// worse than none.
    ///
    /// Unavailable harnesses keep their row in `Default`, disabled, so the menu says a tool is
    /// missing rather than silently omitting it — the same rule the flat list followed before
    /// identities.
    pub fn harness_choices(&self, accounts: &[AccountInfo]) -> Vec<HarnessChoice> {
        let defaults = (0..self.agent_types.len()).map(HarnessChoice::Harness);

        let pairs: Vec<HarnessChoice> = self
            .agent_types
            .iter()
            .enumerate()
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

        if pairs.is_empty() {
            return defaults.collect();
        }

        let mut rows: Vec<HarnessChoice> = vec![HarnessChoice::Label("Default".into())];
        rows.extend(defaults);
        rows.push(HarnessChoice::Separator);
        rows.push(HarnessChoice::Label("Configured".into()));
        rows.extend(pairs);
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

    /// Signing in adds a second, "Configured" group; it never removes the "Default" row a
    /// harness with no identity chosen still needs — that is the library's own zero-config path,
    /// and losing it on first login was accidental. The separator sits between every `Harness`
    /// and every `Pair`, and both headings are decorations at the positions the pick must skip.
    #[test]
    fn accounts_add_a_configured_group_without_removing_the_default_row() {
        let state = with(vec![harness("claude-code", true)]);
        let accounts = [
            account("mdn", &["claude-code"]),
            account("syn", &["claude-code"]),
        ];

        assert_eq!(
            state.harness_choices(&accounts),
            vec![
                HarnessChoice::Label("Default".into()),
                HarnessChoice::Harness(0),
                HarnessChoice::Separator,
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
    /// serving two harnesses is normal, and an account that serves neither offers nothing — but
    /// every harness, signed into or not, still gets its `Default` row.
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
                HarnessChoice::Label("Default".into()),
                HarnessChoice::Harness(0),
                HarnessChoice::Harness(1),
                HarnessChoice::Harness(2),
                HarnessChoice::Separator,
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
                HarnessChoice::Label("Default".into()),
                HarnessChoice::Harness(0),
                HarnessChoice::Harness(1),
                HarnessChoice::Separator,
                HarnessChoice::Label("Configured".into()),
                HarnessChoice::Pair {
                    harness: 0,
                    account: "mdn".to_string()
                },
            ]
        );
    }

    /// The whole point of matching by position: once the decorations are counted in, a `Pair`'s
    /// index in the full list still names the same `(harness, account)` the row shows.
    #[test]
    fn a_pairs_index_in_the_full_list_still_resolves_to_it() {
        let state = with(vec![harness("claude-code", true), harness("codex", true)]);
        let accounts = [account("mdn", &["codex"])];

        let rows = state.harness_choices(&accounts);
        assert_eq!(
            rows[5],
            HarnessChoice::Pair {
                harness: 1,
                account: "mdn".to_string()
            }
        );
    }
}
