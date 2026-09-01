//! The tasks board's own state: which column a task is drawn in is the task's, so what is left
//! here is what the *view* of it is — what was typed in the filter, which session is being looked
//! at, which task is open, which columns and cards are shut, and what the pointer is carrying.
//!
//! The tasks themselves are the host's, and live in [`super::work`] beside the agents doing them,
//! because a board and a graph are two views of one set of work rather than two sets. Everything
//! this module answers is a function of that projection and the fields below; nothing here draws
//! and nothing here names a colour.

use ubiq_proto::ids::{SessionId, StepId, TaskId};
use ubiq_proto::work::{Status, TaskRecord};

use super::work::WorkProjection;

/// Which one of a task's fields is open for editing.
///
/// One at a time, like the project picker's rows: the panel is a report first, and a panel where
/// every field is a text box has stopped reporting. A step is named by its id rather than its place
/// in the list, so a step removed while another is being renamed cannot move the edit onto it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Title,
    Description,
    Step(StepId),
    /// The field at the foot of the list, which names the next sub-task rather than an existing one.
    NewStep,
}

/// What is typed into the panel's fields, mirrored out of the component library's own state.
///
/// It belongs to the project, not to the window: the entities behind these strings are the
/// window's, because there is one of each per window, but what was typed into them is about the
/// task that is open, and so is the board's own filter.
///
/// An uncommitted field does not survive leaving the project, because entering one refills the
/// panel from the record — a draft is a keystroke away from being retyped, and a field showing what
/// was typed in another project would be worse than an empty one.
#[derive(Default)]
pub struct TaskForm {
    pub title: String,
    pub description: String,
    pub step_title: String,
    pub new_step: String,
}

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
    /// A drop the host has not answered yet, so the card can say it is waiting rather than look
    /// like a drag that failed.
    pub moving: Option<(TaskId, Status)>,
    /// A `New task` the host has not answered yet. The task that arrives is the one to select,
    /// because the interface could not know the id it was going to be given.
    pub awaiting_new: bool,
    /// Which field of the open task is being edited, if any.
    pub editing: Option<Field>,
    /// Whether the description is showing as markdown while it is being written. Inside edit mode
    /// rather than instead of it, so Save is still there and the draft is not lost.
    pub preview: bool,
    /// Whether Delete has been asked once. A task is the one thing on this panel that cannot be
    /// retyped, so it takes a second, explicit click — the picker's Forget, for the picker's reason.
    pub confirm_delete: bool,
    pub form: TaskForm,
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
            moving: None,
            awaiting_new: false,
            editing: None,
            preview: false,
            confirm_delete: false,
            form: TaskForm::default(),
        }
    }
}

impl BoardState {
    /// Whether one task passes the filter field and the session pills.
    ///
    /// The text is matched against what the card actually prints — its title and the session it
    /// names — so a user filtering on `cold-start` finds the card that says `cold-start`.
    pub fn matches(&self, work: &WorkProjection, task: &TaskRecord) -> bool {
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
            .and_then(|id| work.session(id))
            .is_some_and(|s| s.name.to_lowercase().contains(&needle))
    }

    /// The cards one column draws, in the order the tasks were defined.
    pub fn column<'a>(&self, work: &'a WorkProjection, status: Status) -> Vec<&'a TaskRecord> {
        work.tasks
            .iter()
            .filter(|task| task.status == status && self.matches(work, task))
            .collect()
    }

    /// What the status bar counts: how many cards are in each column, after the filters.
    pub fn counts(&self, work: &WorkProjection) -> Vec<(Status, usize)> {
        Status::all()
            .into_iter()
            .map(|status| (status, self.column(work, status).len()))
            .collect()
    }

    /// Sub-tasks done and sub-tasks in total, over the cards on screen.
    pub fn steps(&self, work: &WorkProjection) -> (usize, usize) {
        work.tasks
            .iter()
            .filter(|task| self.matches(work, task))
            .fold((0, 0), |(done, total), task| {
                (done + task.done(), total + task.steps.len())
            })
    }

    /// How many of the cards on screen nobody can finish without the user.
    pub fn blocked(&self, work: &WorkProjection) -> usize {
        work.tasks
            .iter()
            .filter(|task| self.matches(work, task) && task.blocked())
            .count()
    }

    /// The task the detail panel is about, when there is one and it is open.
    pub fn open_task<'a>(&self, work: &'a WorkProjection) -> Option<&'a TaskRecord> {
        if !self.show_detail {
            return None;
        }
        work.task(self.selected?)
    }

    /// Whether a card is a drop the host has not answered yet. The card says so rather than
    /// sitting in its old column looking like a drag that failed.
    pub fn is_moving(&self, task: TaskId) -> bool {
        self.moving.is_some_and(|(id, _)| id == task)
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
        // A field open on the card that was selected before was about *that* task, so picking
        // another one discards it rather than committing it somewhere it was never typed.
        self.editing = None;
        self.preview = false;
        self.confirm_delete = false;
    }

    /// Whether one field is the one being edited.
    pub fn is_editing(&self, field: Field) -> bool {
        self.editing == Some(field)
    }

    /// Open one field, and close whichever was open. Entering the description starts on the writing
    /// side: the reader was already looking at the rendered version, and clicking it asked to
    /// change that.
    pub fn edit(&mut self, field: Field) {
        self.editing = Some(field);
        self.preview = false;
        self.confirm_delete = false;
    }

    /// Put every field away. What was typed is left in `form`, because the next thing that fills it
    /// is a selection change and that is what discards a draft.
    pub fn stop_editing(&mut self) {
        self.editing = None;
        self.preview = false;
    }

    /// Whether the panel's fields still describe the task that is open.
    ///
    /// A pure predicate rather than the refill itself, because writing into the component library's
    /// state needs a window and this has to be testable without one.
    pub fn needs_fill(&self, filled: Option<TaskId>) -> bool {
        filled != self.selected
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
