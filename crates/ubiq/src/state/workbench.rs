//! The shell's own state: the projects it can switch between, which rail mode is active, which
//! panels are open, and which of the window's single-open-at-a-time menus is down.

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
}

/// A project the window can be pointed at.
///
/// `colour` indexes the theme's project swatches; it is the project's identity everywhere it
/// appears. `terminals` is how many terminals it has open, which is what makes closing it a
/// question rather than a click.
#[derive(Clone, Debug)]
pub struct Project {
    pub name: String,
    pub path: String,
    pub colour: usize,
    pub open: bool,
    pub terminals: usize,
    /// Relative time, as the recent list shows it.
    pub when: String,
}

pub struct WorkbenchState {
    pub rail_mode: RailMode,
    pub show_left: bool,
    pub show_bottom: bool,
    pub show_right: bool,
    pub theme_id: ThemeId,
    pub open_menu: Option<MenuId>,

    pub projects: Vec<Project>,
    pub project: usize,
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
    pub fn project(&self) -> &Project {
        &self.projects[self.project]
    }

    pub fn project_name(&self) -> &str {
        &self.project().name
    }

    /// The swatch index the whole window is identified by.
    pub fn project_colour(&self) -> usize {
        self.project().colour
    }

    pub fn branch_name(&self) -> &str {
        &self.branches[self.branch]
    }

    /// Whether the explorer, the editor, the dock and the chat are on screen at all. They are all
    /// IDE furniture and leave together.
    pub fn is_ide(&self) -> bool {
        self.rail_mode == RailMode::Ide
    }

    /// The projects currently open, and the ones only remembered, each with its own index. The
    /// filter matches on name and path, so a path fragment finds a project too.
    pub fn filtered(&self, open: bool) -> Vec<(usize, &Project)> {
        let needle = self.project_filter.trim().to_lowercase();
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.open == open)
            .filter(|(_, p)| {
                needle.is_empty()
                    || p.name.to_lowercase().contains(&needle)
                    || p.path.to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// Close a project. Answers whether it went, so the caller knows a confirmation is pending.
    pub fn close_project(&mut self, index: usize, force: bool) -> bool {
        let Some(project) = self.projects.get(index) else {
            return false;
        };
        if project.terminals > 0 && !force {
            self.pending_close = Some(index);
            return false;
        }

        self.projects[index].open = false;
        self.projects[index].terminals = 0;
        self.pending_close = None;

        // The window always points at an open project; fall back to the first one left.
        if self.project == index
            && let Some(next) = self.projects.iter().position(|p| p.open)
        {
            self.project = next;
        }
        true
    }
}
