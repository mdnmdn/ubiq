//! What one project's missions look like *to this window* — the view state both mission surfaces
//! read.
//!
//! The record itself is [`ubiq_proto::mission::MissionRecord`], carried across the bus and held by
//! the project (`app::OpenProject::missions`); nothing here copies any of it. This is the other
//! half: which tab the full view is on, where its WBS is zoomed and what is selected in it, and
//! which work states the tasks are filtered to.
//!
//! **One view state, two shapes.** The full view is a modal or a document tab depending on where
//! it was opened from (§6.2), and moving between the two must lose nothing — so the selection
//! lives here rather than on either surface. The side panel reads the same thing, which is what
//! makes a segment of its progress bar able to open the full view already filtered.
//!
//! Per project, the way the board's and the graph's views are: a window holding three projects
//! holds three of these.

use std::collections::HashMap;

use ubiq_proto::ids::{SpawnId, TaskId};
use ubiq_proto::mission::JournalEntry;

use crate::state::work::WorkState;

/// The MCP servers a mission's **coordinator** is launched with (M13, §6.1).
///
/// Named once, here, because two surfaces compose the same launch — the new-mission dialog's
/// assistant and the panel's *Spawn ▾* — and two lists are two chances for a coordinator to reach
/// the mission surface through one of them and not the other.
pub const COORDINATOR_MCPS: [&str; 4] =
    ["ubiq-mission", "ubiq-plan", "manage-ubiq-tasks", "ubiq-ask"];

/// The MCP servers a mission's **worker** is launched with. The narrower pair: a worker uses the
/// mission and the task it was given, and it never plans or moves the phase.
pub const WORKER_MCPS: [&str; 3] = ["use-mission", "use-task", "ubiq-ask"];

/// The opening prompt a spawned worker starts on — M13's briefing template.
///
/// Composed here rather than at the launch for `mission_briefing`'s reason: what a worker is told
/// is a fact about the mission design, and a test can read it without a window.
///
/// **The requester's own prompt is the body of it.** The frame says where the agent is and what
/// it is; it never rewrites what it was asked to do.
pub fn worker_briefing(
    mission: TaskId,
    kind: &str,
    task: Option<TaskId>,
    reason: &str,
    prompt: &str,
) -> String {
    let mut text = format!("You are a {kind} on mission {mission}.");
    if let Some(task) = task {
        text.push_str(&format!(" You have been given task {task}."));
    }
    let reason = reason.trim();
    if !reason.is_empty() {
        text.push_str(&format!(" You were asked for because: {reason}"));
    }
    text.push_str(
        "\n\nRead the mission with `use-mission` and the task with `use-task`. Report what you \
         do with `report_progress`, and ask the user through `ubiq-ask` rather than guessing at \
         anything that matters.",
    );
    let prompt = prompt.trim();
    if !prompt.is_empty() {
        text.push_str("\n\n");
        text.push_str(prompt);
    }
    text
}

/// What the user changed a pending spawn request to before allowing it (M13).
///
/// **It is the window's, not the record's.** The request on the record is what the *agent* asked
/// for and never changes; this is what the user is about to launch instead, held only until the
/// row is answered — which is why `SpawnOutcome::Launched` has to carry the kind at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpawnPick {
    /// The kind to launch, replacing the request's.
    pub kind: String,
    /// The profile to resolve it through, for the `custom` kind.
    pub profile: Option<String>,
}

/// One mission's journal, as this window has read it back (M12).
///
/// **Newest first**, the order `Message::Journal` sends and the order both surfaces draw — the
/// panel's *Latest* takes the first three and the *Activity* tab takes the lot. `more` is whether
/// anything older exists, so a view knows whether to offer another step back.
#[derive(Clone, Debug, Default)]
pub struct MissionJournal {
    pub entries: Vec<JournalEntry>,
    pub more: bool,
    /// Whether a page has been asked for. Stops the two surfaces asking again on every frame.
    pub asked: bool,
}

impl MissionJournal {
    /// Fold one page in, newest first, dropping anything already held — a second `Journal` for the
    /// same page is the answer to a second ask, not new history.
    pub fn page(&mut self, entries: Vec<JournalEntry>, more: bool) {
        for entry in entries {
            if !self.entries.iter().any(|held| held.seq == entry.seq) {
                self.entries.push(entry);
            }
        }
        self.entries
            .sort_by_key(|entry| std::cmp::Reverse(entry.seq));
        self.more = more;
    }

    /// One line, just written. Newest first, so it goes on the front.
    pub fn appended(&mut self, entry: JournalEntry) {
        if self.entries.iter().any(|held| held.seq == entry.seq) {
            return;
        }
        self.entries.insert(0, entry);
    }

    /// The cursor another page back — the oldest sequence held.
    pub fn oldest(&self) -> Option<u64> {
        self.entries.last().map(|entry| entry.seq)
    }
}

/// One row of the side panel's *Spawn ▾* (§6.1), matched by position the way every menu in this
/// window is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionSpawnRow {
    /// A coordinator — the one role a mission has at most one of.
    Coordinator,
    /// One of [`ubiq_proto::mission::MissionRecord::agent_kinds`], by name.
    Kind(String),
    /// The New agent form, pre-filled with this mission — every answer, rather than a kind's.
    Any,
    /// An agent already running in this window, adopted onto the roster.
    Attach,
}

/// Which stage the *Spawn ▾* menu is on: what to spawn, or which running agent to adopt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpawnStage {
    #[default]
    Kinds,
    Attach,
}

/// The *Spawn ▾* menu, while it is down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MissionSpawnMenu {
    pub task: TaskId,
    pub at: (f32, f32),
    pub stage: SpawnStage,
}

/// Everything one mission launch is composed from, beyond the project it runs in.
///
/// A struct rather than eight arguments: the composition is shared by the spawn policy, the
/// panel's *Spawn ▾* and the coordinator button, and a positional list this long is a list where
/// two `Option<String>`s eventually swap places.
#[derive(Clone, Debug, Default)]
pub struct MissionLaunch {
    /// The kind to resolve, by name.
    pub kind: String,
    /// A profile named outright, for the `custom` case and for a kind the table has no row for.
    pub profile: Option<String>,
    /// Whether this is the coordinator — which is the whole of the difference between the two MCP
    /// sets, and nothing else.
    pub coordinator: bool,
    /// The agent that asked for this one, for a launch answering a relayed request.
    pub spawned_by: Option<ubiq_proto::work::AgentId>,
    /// The opening turn.
    pub briefing: String,
    /// The task to assign it to once it exists, if any.
    pub assign_to: Option<TaskId>,
}

/// One row of a kind-picking menu: one of the mission's own kinds, or a profile named outright.
///
/// Both in one list because they answer one question — *what should this be* — and a menu that
/// split them into two controls would make the `custom` case read as a different kind of answer
/// than it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KindPick {
    Kind(String),
    Profile(String),
}

impl KindPick {
    pub fn label(&self) -> String {
        match self {
            KindPick::Kind(name) => name.clone(),
            KindPick::Profile(id) => format!("profile \u{00b7} {id}"),
        }
    }
}

/// What a kind-picking menu is about to answer for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindTarget {
    /// A pending spawn row: change what will be launched before allowing it.
    Pending(SpawnId),
    /// One row of the agent-kinds table: which profile it resolves to.
    Row(usize),
    /// The table's `+`: a new kind, from a profile.
    Add,
}

/// Which part of the full view is on screen (§6.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MissionTab {
    #[default]
    Overview,
    Wbs,
    Tasks,
    Agents,
    Docs,
    Activity,
    Settings,
}

impl MissionTab {
    pub fn label(self) -> &'static str {
        match self {
            MissionTab::Overview => "Overview",
            MissionTab::Wbs => "WBS",
            MissionTab::Tasks => "Tasks",
            MissionTab::Agents => "Agents",
            MissionTab::Docs => "Plan & docs",
            MissionTab::Activity => "Activity",
            MissionTab::Settings => "Settings",
        }
    }

    pub const fn all() -> [MissionTab; 7] {
        [
            MissionTab::Overview,
            MissionTab::Wbs,
            MissionTab::Tasks,
            MissionTab::Agents,
            MissionTab::Docs,
            MissionTab::Activity,
            MissionTab::Settings,
        ]
    }
}

/// One row of a mission side panel's `⋯` (§6.1).
///
/// **Every row is drawn, whether or not there is anything behind it.** A menu that grows rows as
/// stages land teaches the user a different shape each release; one that says what a mission can
/// be done to, with the rows that have no message yet dead, says the same thing once. What is
/// live today is [`Self::enabled`]'s answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionMenuRow {
    /// Move the mission to `Completed` — a `SetPhase`, the user's own move (M5).
    Complete,
    /// Move it to `Abandoned`. The same message, from any phase: abandoning is terminal from
    /// anywhere and never asks.
    Abandon,
    /// Stop every agent on it: each roster member's harness is unloaded, and nothing is ended —
    /// the transcripts and the run directories stay, so every one of them resumes.
    PauseAll,
    /// Show the anchor on the tasks board.
    OpenOnBoard,
    /// Show the mission's agents on the orchestration graph. The graph has no mission fence yet.
    OpenOnTeams,
    /// Switch between manual and auto (M22) — a `SetMissionField`, which the Settings tab (S6)
    /// owns along with every other field write.
    ExecutionMode,
}

impl MissionMenuRow {
    pub fn all() -> [MissionMenuRow; 6] {
        [
            MissionMenuRow::Complete,
            MissionMenuRow::Abandon,
            MissionMenuRow::PauseAll,
            MissionMenuRow::OpenOnBoard,
            MissionMenuRow::OpenOnTeams,
            MissionMenuRow::ExecutionMode,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            MissionMenuRow::Complete => "Complete",
            MissionMenuRow::Abandon => "Abandon",
            MissionMenuRow::PauseAll => "Pause all agents",
            MissionMenuRow::OpenOnBoard => "Open on board",
            MissionMenuRow::OpenOnTeams => "Open on Teams",
            MissionMenuRow::ExecutionMode => "Execution mode",
        }
    }

    /// Whether this build has anything behind the row. The three that answer `false` are drawn
    /// dead, with [`Self::note`] saying why on hover.
    pub fn enabled(self) -> bool {
        matches!(
            self,
            MissionMenuRow::Complete
                | MissionMenuRow::Abandon
                | MissionMenuRow::OpenOnBoard
                | MissionMenuRow::PauseAll
        )
    }

    /// What a dead row says on hover — the one thing its label has no room for.
    pub fn note(self) -> Option<&'static str> {
        match self {
            MissionMenuRow::Complete | MissionMenuRow::Abandon => None,
            MissionMenuRow::PauseAll => {
                Some("Unloads every member's harness. Nothing is ended, and every one resumes.")
            }
            MissionMenuRow::OpenOnTeams => Some("The graph has no mission fence yet."),
            MissionMenuRow::ExecutionMode => {
                Some("Switched on the full view's Settings tab, with the rest of them.")
            }
            MissionMenuRow::OpenOnBoard => None,
        }
    }
}

/// How one project's missions are being looked at.
///
/// The side panel uses [`Self::state_filter`] to say which segment of its progress bar was
/// clicked, and reads nothing else; the full view reads the lot — which tab it is on, which shape
/// and zoom the WBS is in, and which node of it is selected. The filter is one narrowing across
/// both tabs that honour it, which is why a segment's click survives moving between them.
#[derive(Clone, Debug)]
pub struct MissionView {
    /// Which tab the full view is on. Shared by the modal and the document tab, so `Open as tab`
    /// keeps the reader where they were.
    pub tab: MissionTab,
    /// How far the WBS graph is zoomed, `1.0` being life size. The board's and the graph's own
    /// convention.
    pub zoom: f32,
    /// The task selected in the WBS, whose detail is drawn beside it.
    pub selected: Option<ubiq_proto::ids::TaskId>,
    /// Whether the WBS is showing the table rather than the graph. The same data either way — the
    /// toggle is which shape of it is easier to read, not which half is shown.
    pub wbs_table: bool,
    /// Whether the longest prerequisite chain is highlighted on the WBS.
    pub wbs_critical: bool,
    /// The work state the Tasks tab and the WBS are filtered to, or nothing for all of them —
    /// what clicking a count or a progress segment sets (M26).
    pub state_filter: Option<WorkState>,
    /// What the user changed a pending spawn request to before allowing it (M13), by request.
    /// Dropped when the row is answered; a request the record no longer holds leaves a dead entry
    /// nobody reads, which is cheaper than watching for one.
    pub spawn_picks: HashMap<SpawnId, SpawnPick>,
}

impl Default for MissionView {
    fn default() -> Self {
        Self {
            tab: MissionTab::default(),
            zoom: 1.0,
            selected: None,
            wbs_table: false,
            wbs_critical: false,
            state_filter: None,
            spawn_picks: HashMap::new(),
        }
    }
}

impl MissionView {
    /// Show one work state and nothing else, or clear the filter when it is already the one on —
    /// a count is a toggle, the way the board's label chips are.
    pub fn toggle_state_filter(&mut self, state: WorkState) {
        self.state_filter = match self.state_filter {
            Some(held) if held == state => None,
            _ => Some(state),
        };
    }
}
