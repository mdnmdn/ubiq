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
    /// Who the task is assigned to, by whatever name they go by.
    AssignedTo,
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
    pub assigned_to: String,
    pub step_title: String,
    pub new_step: String,
    pub new_comment: String,
    /// What is being typed into the field that names a label, which is a name and not yet a label:
    /// the colour is chosen when it is added, and until then there is nothing to put on the task.
    pub new_label: String,
}

/// The rest of a new task, waiting for the id the host is about to mint.
///
/// A `CreateTask` carries a title and a session and nothing else, so a card written with a
/// description is finished in a second message once there is a task to send it to.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PendingTask {
    /// Sent as an `UpdateTask` the moment the task exists. Empty means there was none.
    pub description: String,
    /// Whether the title the card was created with is a stand-in cut from the description, and so
    /// whether to ask for a written one. The user typed no title, and a first line is a placeholder
    /// rather than a name.
    pub name_it: bool,
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
    /// The columns held open against the project's "shut this lane when it is empty" setting.
    ///
    /// The counterpart of `shut`, and needed for the same reason it is: a lane that shuts itself
    /// when empty is already a strip, so the click on that strip has nothing in `shut` to remove
    /// and the lane could never be looked into. This is what that click writes instead. Both lists
    /// are the *window's* view of the board and are never written down — the setting is the
    /// project's, and reopening the project is what returns to it.
    pub opened: Vec<Status>,
    /// The cards folded to their title.
    pub folded: Vec<TaskId>,
    pub carry: Option<Carry>,
    /// A drop the host has not answered yet, so the card can say it is waiting rather than look
    /// like a drag that failed.
    pub moving: Option<(TaskId, Status)>,
    /// A `New task` the host has not answered yet. The task that arrives is the one to select,
    /// because the interface could not know the id it was going to be given.
    pub awaiting_new: bool,
    /// A new task being written, before anything has been sent.
    ///
    /// `New task` opens one of these instead of creating a row: a card is made once it has a name
    /// or a description, and never by the click that opened the form — pressing the button, looking
    /// away and pressing it again used to leave two empty cards behind, and a board fills with them
    /// faster than anyone deletes them.
    /// It carries nothing of its own: what is being typed is in `form`, and which session the card
    /// will belong to is the board's.
    pub draft: bool,
    /// What still has to reach the host once the task it belongs to has an id, which is the whole
    /// of a draft that a `CreateTask` could not carry.
    pub pending: Option<PendingTask>,
    /// Which field of the open task is being edited, if any.
    pub editing: Option<Field>,
    /// Whether the description is showing as markdown while it is being written. Inside edit mode
    /// rather than instead of it, so Save is still there and the draft is not lost.
    pub preview: bool,
    /// Whether Delete has been asked once. A task is the one thing on this panel that cannot be
    /// retyped, so it takes a second, explicit click — the picker's Forget, for the picker's reason.
    pub confirm_delete: bool,
    /// Whether the open task draws as a centred modal instead of the side panel. A project's own
    /// choice of *shape* rather than a second copy of the task: both draw from `selected` and
    /// `editing`, so the toggle only changes where the report and its controls are painted.
    pub popup: bool,
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
            opened: Vec::new(),
            folded: Vec::new(),
            carry: None,
            moving: None,
            awaiting_new: false,
            draft: false,
            pending: None,
            editing: None,
            preview: false,
            confirm_delete: false,
            popup: false,
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
    ///
    /// Nothing while a new task is being written: the panel has one slot, and a draft is what is
    /// in it until it is created or given up.
    pub fn open_task<'a>(&self, work: &'a WorkProjection) -> Option<&'a TaskRecord> {
        if !self.show_detail || self.draft {
            return None;
        }
        work.task(self.selected?)
    }

    /// Start writing a new task, or leave the one already being written alone.
    ///
    /// Answers whether the fields need clearing, which is the same question as whether this opened
    /// a draft rather than returning to one — pressing `New task` twice must not throw away what
    /// was typed between the two presses, and must not make a second card either.
    pub fn start_draft(&mut self) -> bool {
        if self.draft {
            return false;
        }
        self.draft = true;
        self.editing = None;
        self.preview = false;
        true
    }

    /// Whether a draft has enough to be a card: a title, or a description, or both.
    ///
    /// The whole of the rule — nothing is sent for a draft that is only a click.
    pub fn draft_ready(&self) -> bool {
        !self.form.title.trim().is_empty() || !self.form.description.trim().is_empty()
    }

    /// Put the draft away, whether it was created or given up.
    pub fn stop_draft(&mut self) {
        self.draft = false;
        self.preview = false;
    }

    /// Whether a card is a drop the host has not answered yet. The card says so rather than
    /// sitting in its old column looking like a drag that failed.
    pub fn is_moving(&self, task: TaskId) -> bool {
        self.moving.is_some_and(|(id, _)| id == task)
    }

    pub fn is_shut(&self, status: Status) -> bool {
        self.shut.contains(&status)
    }

    /// Whether the user has asked for this column against a setting that would shut it.
    pub fn is_held_open(&self, status: Status) -> bool {
        self.opened.contains(&status)
    }

    /// Put a column in one of the two states, saying which rather than flipping.
    ///
    /// The caller is the one that can see the column: whether it is on screen as a strip is the
    /// project's setting and the column's count as well as `shut`, and only the app layer holds
    /// all three. Setting one list always clears the other — the two are answers to one question.
    pub fn set_column(&mut self, status: Status, shut: bool) {
        self.shut.retain(|s| *s != status);
        self.opened.retain(|s| *s != status);
        match shut {
            true => self.shut.push(status),
            false => self.opened.push(status),
        }
    }

    /// Swap the open task between the side panel and a centred modal. Neither `selected` nor
    /// `editing` moves: the shape changes, not what is open or what is being typed.
    pub fn toggle_popup(&mut self) {
        self.popup = !self.popup;
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
    /// A draft never needs filling: there is no record behind it, and a refill would wipe what is
    /// being typed on the next frame.
    pub fn needs_fill(&self, filled: Option<TaskId>) -> bool {
        !self.draft && filled != self.selected
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

    /// The pointer entered this column. If it was already here, the gap a card claimed is left
    /// alone — the column only means the end of itself when the pointer first arrives, or when
    /// the space under the cards claims it. Without that, every drag-move on the column would
    /// overwrite the card's `before` with `None` and every drop would land at the bottom.
    pub fn carry_over_column(&mut self, status: Status) -> bool {
        let Some(carry) = self.carry.as_mut() else {
            return false;
        };
        if carry.over == Some(status) {
            return false;
        }
        carry.over = Some(status);
        carry.before = None;
        true
    }

    /// Put it down, and answer the task, the column it landed in and the card it landed in front
    /// of — `None` being the end of that column.
    pub fn end_carry(&mut self) -> Option<(TaskId, Status, Option<TaskId>)> {
        let carry = self.carry.take()?;
        Some((carry.task, carry.over?, carry.before))
    }
}
