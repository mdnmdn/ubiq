//! The shell's own state: which rail mode is active, which panels are open, and which of the
//! window's single-open-at-a-time menus is down.
//!
//! Which projects exist and which window holds which is not here — that is process-wide, and lives
//! in [`super::windows`]. What stays is what belongs to this window alone: what was typed into the
//! picker's search field, and which project's close is waiting on an answer.

use crate::theme::ThemeId;

/// The left rail's destinations. Only `Ide` is built; the rest render an empty page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RailMode {
    Control,
    Ide,
    Agents,
    Kb,
    Tasks,
}

impl RailMode {
    pub fn label(self) -> &'static str {
        match self {
            RailMode::Control => "Control",
            RailMode::Ide => "IDE",
            RailMode::Agents => "Agents",
            RailMode::Kb => "KB",
            RailMode::Tasks => "Tasks",
        }
    }

    /// The one-line note the empty page shows for a mode that is not built yet.
    pub fn note(self) -> &'static str {
        match self {
            RailMode::Control => "Sessions, workspaces and the agents running in them.",
            RailMode::Ide => "",
            RailMode::Agents => "Agent types, accounts, skills and MCP servers.",
            RailMode::Kb => "Notes and documents the agents can read.",
            RailMode::Tasks => "Work queued for the agents in this session.",
        }
    }

    /// The rail groups, in the order they are drawn.
    pub fn groups() -> &'static [(&'static str, &'static [RailMode])] {
        &[
            ("APP", &[RailMode::Control]),
            (
                "PROJECT",
                &[
                    RailMode::Ide,
                    RailMode::Agents,
                    RailMode::Kb,
                    RailMode::Tasks,
                ],
            ),
        ]
    }
}

/// Every menu in the window. Exactly one may be open, so the shell keeps a single `Option`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuId {
    Project,
    Branch,
    Harness,
    Model,
    Thinking,
    Mode,
    LogSubsystem,
    LogLevel,
}

pub struct WorkbenchState {
    pub rail_mode: RailMode,
    pub show_left: bool,
    pub show_bottom: bool,
    pub show_right: bool,
    pub theme_id: ThemeId,
    pub open_menu: Option<MenuId>,

    /// What was typed into the project menu's search field.
    pub project_filter: String,
    /// A project whose close is waiting on an answer, because it still has terminals open.
    pub pending_close: Option<usize>,

    pub branches: Vec<String>,
    pub branch: usize,
    /// Working-tree summary, as the status bar reports it.
    pub ahead: usize,
    pub behind: usize,
    pub modified: usize,
    pub untracked: usize,
    pub conflicts: usize,
}

impl WorkbenchState {
    pub fn branch_name(&self) -> &str {
        &self.branches[self.branch]
    }

    /// Whether the explorer, the editor, the dock and the chat are on screen at all. They are all
    /// IDE furniture and leave together.
    pub fn is_ide(&self) -> bool {
        self.rail_mode == RailMode::Ide
    }
}
