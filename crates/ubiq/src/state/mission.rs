//! What one project's missions look like *to this window* — the view state both mission surfaces
//! read.
//!
//! The mission itself is its parent task plus the host's derived
//! [`ubiq_proto::mission::MissionSnapshot`], carried across the bus and held by the project
//! (`app::OpenProject::missions`); nothing here copies any of it. This is the other half: which
//! tab the full view is on, where its WBS is zoomed and what is selected in it, which work states
//! the tasks are filtered to, and how the History tab is narrowed.
//!
//! **One view state, two shapes.** The full view is a modal or a document tab depending on where
//! it was opened from, and moving between the two must lose nothing — so the selection lives here
//! rather than on either surface. The side panel reads the same thing, which is what makes a
//! segment of its progress bar able to open the full view already filtered.
//!
//! Per project, the way the board's and the graph's views are: a window holding three projects
//! holds three of these.

use std::collections::{HashMap, HashSet};

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{HistoryEntry, HistoryKind, Phase};
use ubiq_proto::work::AgentId;

use crate::state::work::WorkState;

/// Which part of the full view is on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MissionTab {
    #[default]
    Overview,
    Tasks,
    PlanDocs,
    History,
    Settings,
}

impl MissionTab {
    pub fn label(self) -> &'static str {
        match self {
            MissionTab::Overview => "Overview",
            MissionTab::Tasks => "Tasks",
            MissionTab::PlanDocs => "Plan & docs",
            MissionTab::History => "History",
            MissionTab::Settings => "Settings",
        }
    }

    pub const fn all() -> [MissionTab; 5] {
        [
            MissionTab::Overview,
            MissionTab::Tasks,
            MissionTab::PlanDocs,
            MissionTab::History,
            MissionTab::Settings,
        ]
    }
}

/// One row of a mission side panel's `⋯`, matched by position the way every menu in this window
/// is. Every row is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissionMenuRow {
    /// *Pause* while running, *Resume* while paused — one row, whose label follows the phase.
    PauseResume,
    /// The planner revises the plan and the tasks that have not started; asks for a note first.
    Replan,
    /// Show the parent on the tasks board.
    OpenOnBoard,
    /// The same as *Mark reviewed*, from any phase.
    Complete,
    /// Stop the mission's workers, abandon its open tasks and the parent.
    Abandon,
}

impl MissionMenuRow {
    pub fn all() -> [MissionMenuRow; 5] {
        [
            MissionMenuRow::PauseResume,
            MissionMenuRow::Replan,
            MissionMenuRow::OpenOnBoard,
            MissionMenuRow::Complete,
            MissionMenuRow::Abandon,
        ]
    }

    pub fn label(self, phase: Phase) -> &'static str {
        match self {
            MissionMenuRow::PauseResume => match phase {
                Phase::Paused => "Resume",
                _ => "Pause",
            },
            MissionMenuRow::Replan => "Replan\u{2026}",
            MissionMenuRow::OpenOnBoard => "Open on board",
            MissionMenuRow::Complete => "Complete",
            MissionMenuRow::Abandon => "Abandon",
        }
    }

    /// Whether the row means anything in this phase. The host refuses the rest; drawing them dead
    /// says why before the click rather than after it.
    pub fn enabled(self, phase: Phase) -> bool {
        match self {
            MissionMenuRow::PauseResume => matches!(phase, Phase::Running | Phase::Paused),
            MissionMenuRow::Replan => matches!(phase, Phase::Running | Phase::Paused | Phase::Review),
            MissionMenuRow::OpenOnBoard => true,
            MissionMenuRow::Complete | MissionMenuRow::Abandon => {
                !matches!(phase, Phase::Done | Phase::Abandoned)
            }
        }
    }

    pub fn note(self, phase: Phase) -> Option<&'static str> {
        match self {
            MissionMenuRow::PauseResume if !self.enabled(phase) => {
                Some("Only a running or paused mission can be paused.")
            }
            MissionMenuRow::PauseResume => {
                Some("No new starts while paused; running workers finish.")
            }
            MissionMenuRow::Replan if !self.enabled(phase) => {
                Some("Replanning applies once the mission is running.")
            }
            MissionMenuRow::Replan => Some("The planner revises the plan and the unstarted tasks."),
            _ => None,
        }
    }
}

/// One of the side panel's own sections — the ones drawn with a fold, each of which opens and
/// shuts on its own rather than as one switch over the whole panel.
///
/// **Not the header, the stepper or the progress bar.** Those are the panel's chrome, and the
/// guidance composer at the foot is not a section either: what folds is the content between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MissionSection {
    NeedsYou,
    Running,
    Next,
    Documents,
}

impl MissionSection {
    /// A bare id fragment for this section's element id — never shown, an `eid2` component.
    pub fn label(self) -> &'static str {
        match self {
            MissionSection::NeedsYou => "needs-you",
            MissionSection::Running => "running",
            MissionSection::Next => "next",
            MissionSection::Documents => "documents",
        }
    }
}

/// What the note dialog is for. One dialog, one textarea, one Send — the five places a mission
/// asks the user for words before it acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    /// *Reject…* on a task awaiting approval: the feedback goes to its worker.
    Reject(TaskId),
    /// *Fail…* on a human task: the feedback goes to the task it reviews.
    Fail(TaskId),
    /// *Replan…*: what the planner is to change.
    Replan(TaskId),
    /// *Redo breakdown…*: what to change in the draft.
    Redo(TaskId),
    /// *Reject…* of the mission itself in Review: replan with this as the note.
    RejectMission(TaskId),
}

impl NoteKind {
    pub fn title(self) -> &'static str {
        match self {
            NoteKind::Reject(_) => "Reject with feedback",
            NoteKind::Fail(_) => "Fail with feedback",
            NoteKind::Replan(_) => "Replan",
            NoteKind::Redo(_) => "Redo breakdown",
            NoteKind::RejectMission(_) => "Reject the mission",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            NoteKind::Reject(_) => {
                "Saved as a comment on the task and sent to its worker. With no worker alive the \
                 task shows Retry and the feedback goes with it."
            }
            NoteKind::Fail(_) => {
                "Saved as a comment and sent to the task this one reviews, which returns to in \
                 progress."
            }
            NoteKind::Replan(_) => {
                "The planner revises the plan and the tasks that have not started, then the \
                 mission goes back through Breakdown."
            }
            NoteKind::Redo(_) => "A note to the planner, which rewrites the draft.",
            NoteKind::RejectMission(_) => {
                "Replans with this as the note: Breakdown, then Running again."
            }
        }
    }

    pub fn send_label(self) -> &'static str {
        "Send"
    }
}

/// The note dialog while it is up. The text itself lives in the window's own textarea.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissionNote {
    pub kind: NoteKind,
}

/// How the History tab is narrowed. Three independent filters; `None` is all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryFilter {
    pub kind: Option<HistoryKind>,
    pub task: Option<TaskId>,
    pub agent: Option<AgentId>,
}

impl HistoryFilter {
    pub fn admits(&self, entry: &HistoryEntry) -> bool {
        self.kind.is_none_or(|kind| kind == entry.kind)
            && self.task.is_none_or(|task| entry.task == Some(task))
            && self.agent.is_none_or(|agent| entry.agent == Some(agent))
    }
}

/// A short word for a history kind — the filter pill and the row's tag.
pub fn history_kind_label(kind: HistoryKind) -> &'static str {
    match kind {
        HistoryKind::Phase => "phase",
        HistoryKind::Plan => "plan",
        HistoryKind::Agent => "agent",
        HistoryKind::Review => "review",
        HistoryKind::Feedback => "feedback",
        HistoryKind::Guidance => "guidance",
        HistoryKind::Status => "status",
        HistoryKind::Comment => "comment",
    }
}

pub const HISTORY_KINDS: [HistoryKind; 8] = [
    HistoryKind::Phase,
    HistoryKind::Plan,
    HistoryKind::Agent,
    HistoryKind::Review,
    HistoryKind::Feedback,
    HistoryKind::Guidance,
    HistoryKind::Status,
    HistoryKind::Comment,
];

/// How one project's missions are being looked at.
///
/// The side panel uses [`Self::state_filter`] to say which segment of its progress bar was
/// clicked; the full view reads the lot — which tab it is on, which shape and zoom the WBS is in,
/// and which node of it is selected. The filter is one narrowing across both tabs that honour it,
/// which is why a segment's click survives moving between them.
#[derive(Clone, Debug)]
pub struct MissionView {
    /// Which tab the full view is on. Shared by the modal and the document tab, so `Open as tab`
    /// keeps the reader where they were.
    pub tab: MissionTab,
    /// How far the WBS graph is zoomed, `1.0` being life size.
    pub zoom: f32,
    /// The task selected in the WBS, whose detail is drawn beside it.
    pub selected: Option<TaskId>,
    /// Whether the WBS is showing the table rather than the graph.
    pub wbs_table: bool,
    /// Whether the longest prerequisite chain is highlighted on the WBS.
    pub wbs_critical: bool,
    /// The work state the Tasks tab is filtered to, or nothing for all of them.
    pub state_filter: Option<WorkState>,
    /// Which of a mission's side-panel sections are shut, by the mission's own parent task. A
    /// mission absent from the map has every section open: UI-local persistence, not a fact the
    /// host has an opinion about.
    pub shut_sections: HashMap<TaskId, HashSet<MissionSection>>,
    /// The History tab's narrowing.
    pub history: HistoryFilter,
    /// The guidance composer's *also send to running workers* box.
    pub guidance_to_running: bool,
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
            shut_sections: HashMap::new(),
            history: HistoryFilter::default(),
            guidance_to_running: false,
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

    /// Whether one mission's section is open — every section starts open, which is why this reads
    /// the absence of a shut mark rather than the presence of an open one.
    pub fn section_open(&self, task_id: TaskId, section: MissionSection) -> bool {
        !self
            .shut_sections
            .get(&task_id)
            .is_some_and(|shut| shut.contains(&section))
    }

    /// Open a shut section or shut an open one, remembered against this mission's own parent task.
    pub fn toggle_section(&mut self, task_id: TaskId, section: MissionSection) {
        let shut = self.shut_sections.entry(task_id).or_default();
        if !shut.remove(&section) {
            shut.insert(section);
        }
    }
}
