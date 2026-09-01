//! The tasks board's own state: which column a task is drawn in is the task's, so what is left
//! here is what the *view* of it is — what was typed in the filter, which session is being looked
//! at, which task is open, which columns and cards are shut, and what the pointer is carrying.
//!
//! The tasks themselves live in [`super::agents`] beside the agents doing them, because a board
//! and a graph are two views of one set of work rather than two sets. Everything this module
//! answers is a function of that vector and the fields below; nothing here draws and nothing here
//! names a colour.

use super::agents::{AgentsState, SessionId, Status, Task, TaskId};

/// A task under the pointer, and the column a drop would put it in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Carry {
    pub task: TaskId,
    pub over: Option<Status>,
}

pub struct BoardState {
    /// What was typed into the filter field. It is also what names a new task: one field to find
    /// work and to add it, which is why the board needs no second box.
    pub filter: String,
    /// Which session the board is showing. `None` is every session, and is what "all sessions"
    /// means — including the tasks that belong to none.
    pub session: Option<SessionId>,
    pub selected: Option<TaskId>,
    pub show_detail: bool,
    /// The columns shut to a strip. A shut column still counts and still takes a drop.
    pub shut: Vec<Status>,
    /// The cards folded to their title.
    pub folded: Vec<TaskId>,
    pub carry: Option<Carry>,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            filter: String::new(),
            session: None,
            selected: None,
            show_detail: true,
            shut: Vec::new(),
            folded: Vec::new(),
            carry: None,
        }
    }
}

impl BoardState {
    /// Whether one task passes the filter field and the session pills.
    ///
    /// The text is matched against what the card actually prints — its title and the session it
    /// names — so a user filtering on `cold-start` finds the card that says `cold-start`.
    pub fn matches(&self, agents: &AgentsState, task: &Task) -> bool {
        if let Some(session) = self.session
            && task.session != Some(session)
        {
            return false;
        }
        let needle = self.filter.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        if task.title.to_lowercase().contains(&needle) {
            return true;
        }
        task.session
            .and_then(|id| agents.session(id))
            .is_some_and(|s| s.name.to_lowercase().contains(&needle))
    }

    /// The cards one column draws, in the order the tasks were defined.
    pub fn column<'a>(&self, agents: &'a AgentsState, status: Status) -> Vec<&'a Task> {
        agents
            .tasks
            .iter()
            .filter(|task| task.status == status && self.matches(agents, task))
            .collect()
    }

    /// What the status bar counts: how many cards are in each column, after the filters.
    pub fn counts(&self, agents: &AgentsState) -> Vec<(Status, usize)> {
        Status::all()
            .into_iter()
            .map(|status| (status, self.column(agents, status).len()))
            .collect()
    }

    /// Sub-tasks done and sub-tasks in total, over the cards on screen.
    pub fn steps(&self, agents: &AgentsState) -> (usize, usize) {
        agents
            .tasks
            .iter()
            .filter(|task| self.matches(agents, task))
            .fold((0, 0), |(done, total), task| {
                (done + task.done(), total + task.steps.len())
            })
    }

    /// How many of the cards on screen nobody can finish without the user.
    pub fn blocked(&self, agents: &AgentsState) -> usize {
        agents
            .tasks
            .iter()
            .filter(|task| self.matches(agents, task) && task.blocked())
            .count()
    }

    /// The task the detail panel is about, when there is one and it is open.
    pub fn open_task<'a>(&self, agents: &'a AgentsState) -> Option<&'a Task> {
        if !self.show_detail {
            return None;
        }
        agents.task(self.selected?)
    }

    pub fn is_shut(&self, status: Status) -> bool {
        self.shut.contains(&status)
    }

    pub fn toggle_column(&mut self, status: Status) {
        if let Some(ix) = self.shut.iter().position(|s| *s == status) {
            self.shut.remove(ix);
        } else {
            self.shut.push(status);
        }
    }

    pub fn is_folded(&self, task: TaskId) -> bool {
        self.folded.contains(&task)
    }

    pub fn toggle_fold(&mut self, task: TaskId) {
        if let Some(ix) = self.folded.iter().position(|t| *t == task) {
            self.folded.remove(ix);
        } else {
            self.folded.push(task);
        }
    }

    /// Point the panel at a task. Picking a card always opens the panel: a selection nothing
    /// reports on is not a selection.
    pub fn select(&mut self, task: TaskId) {
        self.selected = Some(task);
        self.show_detail = true;
    }

    pub fn start_carry(&mut self, task: TaskId) {
        self.carry = Some(Carry { task, over: None });
    }

    /// Which column the pointer is over. Answers whether that changed, so a drag across one column
    /// does not ask for a frame per pixel.
    pub fn carry_over(&mut self, status: Status) -> bool {
        let Some(carry) = self.carry.as_mut() else {
            return false;
        };
        if carry.over == Some(status) {
            return false;
        }
        carry.over = Some(status);
        true
    }

    /// Put it down, and answer the task and the column it landed in.
    pub fn end_carry(&mut self) -> Option<(TaskId, Status)> {
        let carry = self.carry.take()?;
        Some((carry.task, carry.over?))
    }
}
