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

/// What a needle has to start with to be read as one field rather than as free text.
const KEY_PREFIX: &str = "key:";
const LABEL_PREFIX: char = '#';

use super::work::WorkProjection;

/// Which one of a task's fields is open for editing.
///
/// One at a time, like the project picker's rows: the panel is a report first, and a panel where
/// every field is a text box has stopped reporting. A step is named by its id rather than its place
/// in the list, so a step removed while another is being renamed cannot move the edit onto it.
///
/// Only what is *typed* is here. A kind and a label are picked from a list of what there is, and a
/// pick commits the moment it is made — there is no half-made one to hold open, which is why
/// neither has a field of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Title,
    Description,
    /// The user's own id for the task — what their tracker calls it, not the ULID.
    Key,
    /// The URL to the issue the task stands for.
    Link,
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
    pub key: String,
    pub link: String,
    pub step_title: String,
    pub new_step: String,
    /// What is being typed into the field that names a label, which is a name and not yet a label:
    /// the colour is chosen when it is added, and until then there is nothing to put on the task.
    pub new_label: String,
}

/// A task under the pointer, the column a drop would put it in, and where in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Carry {
    pub task: TaskId,
    pub over: Option<Status>,
    /// The card a drop would land immediately in front of; `None` is the end of the column.
    ///
    /// An id rather than an index, because the board filters: an index into what the user can see
    /// is wrong by however many hidden cards of that status sit above it. An id names the same card
    /// whatever else the filters left out.
    pub before: Option<TaskId>,
}

pub struct BoardState {
    /// What was typed into the filter field. It is also what names a new task: one field to find
    /// work and to add it, which is why the board needs no second box.
    pub filter: String,
    /// Which session the board is showing. `None` is every session, and is what "all sessions"
    /// means — including the tasks that belong to none.
    pub session: Option<SessionId>,
    /// The labels being narrowed to, by name. Empty is every label, the way no session pill is
    /// every session — a row with nothing lit is not filtering.
    ///
    /// Names rather than [`ubiq_proto::work::Label`]s, because the colour is the task's: two cards
    /// carrying `flaky` in two colours are carrying one label, and the pill filters on what the
    /// user reads.
    pub labels: Vec<String>,
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
            labels: Vec::new(),
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
    /// Whether one task passes the filter field, the session pill and the label pills.
    ///
    /// The three narrow together rather than in turn: a task has to be in the shown session, carry
    /// **every** label that is lit, and answer the text. Labels AND rather than OR because lighting
    /// a second pill is asking a narrower question, and a row that widened as it was clicked would
    /// be a filter nobody could aim.
    ///
    /// The text is read as one field when it says which:
    ///
    /// * `key:foo` matches the task's key and nothing else — a tracker id is a thing to look up
    ///   exactly, and `UBQ-4` in a description is not the card called `UBQ-4`.
    /// * `#foo` matches label names and nothing else.
    /// * anything else is the whole card: its title, its notes, its key, what kind of work it is,
    ///   its labels and the session doing it.
    ///
    /// Both prefixes are case-insensitive substrings, as the plain form is, and a prefix with
    /// nothing after it narrows nothing — a user halfway through typing `key:` is not asking for an
    /// empty board.
    pub fn matches(&self, work: &WorkProjection, task: &TaskRecord) -> bool {
        if let Some(session) = self.session
            && task.session != Some(session)
        {
            return false;
        }
        if !self
            .labels
            .iter()
            .all(|name| task.labels.iter().any(|label| label.name == *name))
        {
            return false;
        }
        let needle = self.filter.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        if let Some(rest) = needle.strip_prefix(KEY_PREFIX) {
            let rest = rest.trim();
            return rest.is_empty()
                || task
                    .key
                    .as_deref()
                    .is_some_and(|key| key.to_lowercase().contains(rest));
        }
        if let Some(rest) = needle.strip_prefix(LABEL_PREFIX) {
            let rest = rest.trim();
            return rest.is_empty() || Self::has_label(task, rest);
        }
        if task.title.to_lowercase().contains(&needle)
            || task.description.to_lowercase().contains(&needle)
            || task
                .key
                .as_deref()
                .is_some_and(|key| key.to_lowercase().contains(&needle))
            || task
                .kind
                .is_some_and(|kind| kind.label().contains(needle.as_str()))
            || Self::has_label(task, &needle)
        {
            return true;
        }
        task.session
            .and_then(|id| work.session(id))
            .is_some_and(|s| s.name.to_lowercase().contains(&needle))
    }

    /// Whether any of a task's labels reads as the needle, which is already lowercased.
    fn has_label(task: &TaskRecord, needle: &str) -> bool {
        task.labels
            .iter()
            .any(|label| label.name.to_lowercase().contains(needle))
    }

    /// Whether one label's pill is lit.
    pub fn is_label_on(&self, name: &str) -> bool {
        self.labels.iter().any(|held| held == name)
    }

    /// Light one label's pill or put it out. Any of them may be the last: with none lit the row is
    /// not filtering, which is the way back from having turned them all on.
    pub fn toggle_label(&mut self, name: &str) {
        if let Some(ix) = self.labels.iter().position(|held| held == name) {
            self.labels.remove(ix);
        } else {
            self.labels.push(name.to_string());
        }
    }

    /// Whether anything is being hidden, so the control that clears the filters can say whether it
    /// has anything to do.
    pub fn filtering(&self) -> bool {
        !self.filter.trim().is_empty() || self.session.is_some() || !self.labels.is_empty()
    }

    /// Put every filter back, which is the toolbar's one control for "show everything".
    ///
    /// The filter field is also what names a new task, so clearing it here is clearing a draft
    /// title — which is what the user asked for by asking to see everything.
    pub fn clear_filters(&mut self) {
        self.filter.clear();
        self.session = None;
        self.labels.clear();
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
        self.carry = Some(Carry {
            task,
            over: None,
            before: None,
        });
    }

    /// Which column the pointer is over, and which card it is above. Answers whether **either**
    /// changed, so a drag past several cards inside one column redraws once per card boundary
    /// rather than once per pixel.
    pub fn carry_over(&mut self, status: Status, before: Option<TaskId>) -> bool {
        let Some(carry) = self.carry.as_mut() else {
            return false;
        };
        if carry.over == Some(status) && carry.before == before {
            return false;
        }
        carry.over = Some(status);
        carry.before = before;
        true
    }

    /// Put it down, and answer the task, the column it landed in and the card it landed in front
    /// of — `None` being the end of that column.
    pub fn end_carry(&mut self) -> Option<(TaskId, Status, Option<TaskId>)> {
        let carry = self.carry.take()?;
        Some((carry.task, carry.over?, carry.before))
    }
}
