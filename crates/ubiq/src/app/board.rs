use super::*;

use crate::state::board::PendingTask;
use ubiq_proto::messages::TaskField;
use ubiq_proto::work::{Complexity, Kind, Label};

impl AppState {
    /// Open one of the panel's fields.
    pub fn begin_task_edit(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        // A step's field starts from what the step says now, because it is one field shared by
        // however many steps the task has.
        if let Field::Step(step_id) = field {
            let title = self
                .open_task_form(cx)
                .and_then(|(_, task_id, _)| self.work(cx)?.task(task_id))
                .and_then(|task| task.step(step_id))
                .map(|step| step.title.clone())
                .unwrap_or_default();
            if let Some(board) = self.board_mut(cx) {
                board.form.step_title = title.clone();
            }
            let input = self.step_title_input.clone();
            input.update(cx, |state, cx| state.set_value(&title, window, cx));
        }
        if let Some(board) = self.board_mut(cx) {
            board.edit(field);
        }
        // The field takes the keyboard from the click that opened it, so one click starts typing.
        match field {
            Field::Title => {
                let input = self.task_title_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::Description => {
                let input = self.task_description_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::Step(_) => {
                let input = self.step_title_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::NewStep => {
                let input = self.new_step_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::Key => {
                let input = self.task_key_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::Link => {
                let input = self.task_link_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
            Field::AssignedTo => {
                let input = self.task_assigned_input.clone();
                input.update(cx, |state, cx| state.focus(window, cx));
            }
        }
        cx.notify();
    }

    /// Put the open field away and keep the task as the host last reported it.
    pub fn cancel_task_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        // Refill from the record, so what was typed and thrown away is gone rather than waiting to
        // be committed by the next click.
        self.form_filled = None;
        self.fill_task_form(window, cx);
        cx.notify();
    }

    pub fn toggle_description_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.preview = !board.preview;
        }
        cx.notify();
    }

    /// The project, the open task and the panel's form, or nothing if there is no task open.
    fn open_task_form(&self, cx: &App) -> Option<(ProjectId, TaskId, &BoardState)> {
        let project = self.project(cx)?;
        let board = self.board(cx)?;
        Some((project, board.selected?, board))
    }

    /// Send one `UpdateTask`, and put the field away.
    ///
    /// A value equal to the one the host already holds sends nothing: the message set is for acts,
    /// and re-asserting a title is not one.
    fn update_task(
        &mut self,
        title: Option<String>,
        description: Option<String>,
        priority: Option<Priority>,
        cx: &mut Context<Self>,
    ) {
        let Some((project_id, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        self.bus.send(Message::UpdateTask {
            project_id,
            task_id,
            title,
            description,
            priority,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        cx.notify();
    }

    /// Send one `SetTaskField`, and put the field away.
    ///
    /// The optional half of a task goes through here rather than through [`Self::update_task`],
    /// because absent and cleared are different answers and only a variant of its own can say
    /// which: every arm's `None` is the user rubbing the fact out.
    fn set_task_field(&mut self, field: TaskField, cx: &mut Context<Self>) {
        let Some((project_id, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        self.bus.send(Message::SetTaskField {
            project_id,
            task_id,
            field,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        cx.notify();
    }

    pub fn commit_task_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Enter in the title of a new task is what creates it: the draft has no record to commit
        // one field to, and its two fields are sent together or not at all.
        if self.board(cx).is_some_and(|board| board.draft) {
            self.create_task(window, cx);
            return;
        }
        let Some((_, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        if !board.is_editing(Field::Title) {
            return;
        }
        let typed = board.form.title.trim().to_string();
        // An empty title is a slip rather than an intention, so it is refused here and never sent —
        // the same posture as Send reading as disabled on an empty draft.
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .is_some_and(|task| task.title == typed);
        if typed.is_empty() || unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        self.update_task(Some(typed), None, None, cx);
    }

    /// A description, unlike a title, may be emptied: clearing one is a thing to mean.
    pub fn commit_task_description(&mut self, cx: &mut Context<Self>) {
        let Some((_, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let typed = board.form.description.clone();
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .is_some_and(|task| task.description == typed);
        if unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        self.update_task(None, Some(typed), None, cx);
    }

    /// The user's own id for the task. Like a description and unlike a title, it may be emptied:
    /// a key rubbed out is a task that never had one to begin with.
    pub fn commit_task_key(&mut self, cx: &mut Context<Self>) {
        let Some((_, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let typed = board.form.key.trim().to_string();
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .is_some_and(|task| task.key.as_deref().unwrap_or_default() == typed);
        if unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        let key = (!typed.is_empty()).then_some(typed);
        self.set_task_field(TaskField::Key(key), cx);
    }

    /// The URL the task stands for, emptied the same way. Nothing here parses or fetches it — it is
    /// a string the host stores, and the glyph beside it is the interface reading the host's own
    /// answer about which tracker it belongs to.
    pub fn commit_task_link(&mut self, cx: &mut Context<Self>) {
        let Some((_, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let typed = board.form.link.trim().to_string();
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .is_some_and(|task| task.link.as_deref().unwrap_or_default() == typed);
        if unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        let link = (!typed.is_empty()).then_some(typed);
        self.set_task_field(TaskField::Link(link), cx);
    }

    /// Who the task is assigned to, emptied the same way as a key or a link.
    pub fn commit_task_assigned(&mut self, cx: &mut Context<Self>) {
        let Some((_, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let typed = board.form.assigned_to.trim().to_string();
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .is_some_and(|task| task.assigned_to.as_deref().unwrap_or_default() == typed);
        if unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        let assigned_to = (!typed.is_empty()).then_some(typed);
        self.set_task_field(TaskField::AssignedTo(assigned_to), cx);
    }

    pub fn set_task_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        self.update_task(None, None, Some(priority), cx);
    }

    /// How the agents on the task are arranged, or nobody has said. `None` is the pick that takes
    /// the claim back, which is not the same as picking `Direct`.
    pub fn set_task_shape(&mut self, shape: Option<Shape>, cx: &mut Context<Self>) {
        self.set_task_field(TaskField::Shape(shape), cx);
    }

    /// What kind of work it is, or nobody has said.
    pub fn set_task_kind(&mut self, kind: Option<Kind>, cx: &mut Context<Self>) {
        self.set_task_field(TaskField::Kind(kind), cx);
    }

    /// How complex the task is, or nobody has said.
    pub fn set_task_complexity(&mut self, complexity: Option<Complexity>, cx: &mut Context<Self>) {
        self.set_task_field(TaskField::Complexity(complexity), cx);
    }

    /// The card's own swatch, or none so the edge reads the pulse again.
    pub fn set_task_colour(&mut self, colour: Option<usize>, cx: &mut Context<Self>) {
        self.set_task_field(TaskField::Colour(colour), cx);
    }

    /// Put one label on the open task, keeping the ones it already carries.
    ///
    /// The whole list is sent rather than the one that was added, because a label list is short and
    /// is edited as a set. A name already on the task is a no-op: two pills reading the same word
    /// are not two labels.
    pub fn add_task_label(&mut self, name: String, colour: usize, cx: &mut Context<Self>) {
        let Some((_, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some(mut labels) = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .map(|task| task.labels.clone())
        else {
            return;
        };
        if labels.iter().any(|label| label.name == name) {
            return;
        }
        labels.push(Label::new(name, colour));
        if let Some(board) = self.board_mut(cx) {
            board.form.new_label.clear();
        }
        self.set_task_field(TaskField::Labels(labels), cx);
    }

    /// Take one label off, by the name the pill prints.
    pub fn remove_task_label(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some((_, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        let Some(labels) = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .map(|task| task.labels.clone())
        else {
            return;
        };
        let kept: Vec<Label> = labels
            .iter()
            .filter(|label| label.name != name)
            .cloned()
            .collect();
        // A name the task does not carry asks for nothing: the message set is for acts.
        if kept.len() == labels.len() {
            return;
        }
        self.set_task_field(TaskField::Labels(kept), cx);
    }

    /// Hand the open task to a session, or take it back. `None` is a task nobody has started.
    pub fn set_task_session(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        let Some((project_id, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        self.close_menu(cx);
        self.bus.send(Message::AssignTask {
            project_id,
            task_id,
            session,
        });
        cx.notify();
    }

    /// Add a sub-task and keep the field, so several can be typed in a row.
    pub fn add_task_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project_id, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let title = board.form.new_step.trim().to_string();
        if title.is_empty() {
            return;
        }
        self.bus.send(Message::AddStep {
            project_id,
            task_id,
            title,
        });
        if let Some(board) = self.board_mut(cx) {
            board.form.new_step.clear();
        }
        let input = self.new_step_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Leave a comment on the open task. The host stamps the author as the user.
    pub fn add_task_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((project_id, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let text = board.form.new_comment.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.bus.send(Message::AddComment {
            project_id,
            task_id,
            text,
        });
        if let Some(board) = self.board_mut(cx) {
            board.form.new_comment.clear();
        }
        let input = self.new_comment_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub fn commit_step_title(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((project_id, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        let Some(Field::Step(step_id)) = board.editing else {
            return;
        };
        let title = board.form.step_title.trim().to_string();
        let unchanged = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .and_then(|task| task.step(step_id))
            .is_some_and(|step| step.title == title);
        if title.is_empty() || unchanged {
            if let Some(board) = self.board_mut(cx) {
                board.stop_editing();
            }
            cx.notify();
            return;
        }
        self.bus.send(Message::RenameStep {
            project_id,
            task_id,
            step_id,
            title,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        cx.notify();
    }

    /// Drop a sub-task. No confirmation: the two-click question is for what cannot be retyped, and
    /// a sub-task's title is one line.
    pub fn remove_task_step(&mut self, step_id: StepId, cx: &mut Context<Self>) {
        let Some((project_id, task_id, _)) = self.open_task_form(cx) else {
            return;
        };
        self.bus.send(Message::RemoveStep {
            project_id,
            task_id,
            step_id,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_editing();
        }
        cx.notify();
    }

    /// Delete the open task. The first click asks; only the second sends.
    ///
    /// A task is the one thing on this panel that cannot be retyped, which is why it takes the
    /// question the picker's Forget takes and a sub-task's × does not.
    pub fn delete_task(&mut self, cx: &mut Context<Self>) {
        let Some((project_id, task_id, board)) = self.open_task_form(cx) else {
            return;
        };
        if !board.confirm_delete {
            if let Some(board) = self.board_mut(cx) {
                board.confirm_delete = true;
            }
            cx.notify();
            return;
        }
        self.bus.send(Message::DeleteTask {
            project_id,
            task_id,
        });
        if let Some(board) = self.board_mut(cx) {
            board.confirm_delete = false;
        }
        cx.notify();
    }

    /// Withdraw the delete question, which any other click on the panel does.
    pub fn withdraw_task_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            if !board.confirm_delete {
                return;
            }
            board.confirm_delete = false;
        }
        cx.notify();
    }

    /// Fill the panel's fields from the task that is open, once per selection.
    ///
    /// Drained in `render` rather than done where the selection changes, because `set_value` needs a
    /// window and three of the callers have none: a message that arrives, a project switch, and the
    /// board's own jump to the graph. The guard is what stops it writing over what the user is
    /// typing on every frame.
    pub(super) fn fill_task_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.refill_fields {
            self.refill_fields = false;
            let (filter, draft) = self
                .open_project(cx)
                .map(|open| (open.board.filter.clone(), open.graph.draft.clone()))
                .unwrap_or_default();
            let task_filter = self.task_filter.clone();
            task_filter.update(cx, |state, cx| state.set_value(&filter, window, cx));
            let agent_input = self.agent_input.clone();
            agent_input.update(cx, |state, cx| state.set_value(&draft, window, cx));
        }

        let Some(board) = self.board(cx) else {
            return;
        };
        if !board.needs_fill(self.form_filled) {
            return;
        }
        let selected = board.selected;
        let (title, description, key, link, assigned_to) = selected
            .and_then(|id| self.work(cx).and_then(|work| work.task(id)))
            .map(|task| {
                (
                    task.title.clone(),
                    task.description.clone(),
                    task.key.clone().unwrap_or_default(),
                    task.link.clone().unwrap_or_default(),
                    task.assigned_to.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default();

        self.form_filled = selected;
        if let Some(board) = self.board_mut(cx) {
            board.form.title = title.clone();
            board.form.description = description.clone();
            board.form.key = key.clone();
            board.form.link = link.clone();
            board.form.assigned_to = assigned_to.clone();
            board.form.step_title.clear();
            board.form.new_step.clear();
            board.form.new_comment.clear();
            board.form.new_label.clear();
        }
        for (input, value) in [
            (self.task_title_input.clone(), title),
            (self.task_key_input.clone(), key),
            (self.task_link_input.clone(), link),
            (self.task_assigned_input.clone(), assigned_to),
            (self.task_label_input.clone(), String::new()),
            (self.step_title_input.clone(), String::new()),
            (self.new_step_input.clone(), String::new()),
            (self.new_comment_input.clone(), String::new()),
        ] {
            input.update(cx, |state, cx| state.set_value(&value, window, cx));
        }
        let description_input = self.task_description_input.clone();
        description_input.update(cx, |state, cx| state.set_value(&description, window, cx));
    }

    /// Point the panel at a task. Picking a card always opens the panel, because a selection
    /// nothing reports on is not a selection.
    pub fn select_task(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.select(task);
        }
        cx.notify();
    }

    pub fn close_task_detail(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.show_detail = false;
        }
        cx.notify();
    }

    /// Which session the board is showing. `None` is every session, including the tasks that
    /// belong to none.
    pub fn pick_board_session(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.session = session;
        }
        cx.notify();
    }

    /// Light one label's pill or put it out. Lighting a second narrows further: a card has to carry
    /// every label that is lit.
    pub fn toggle_board_label(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.toggle_label(name);
        }
        cx.notify();
    }

    /// Show everything: the text, the session pill and every label pill, all put back at once.
    ///
    /// The filter field is one of them, so the input is emptied with the state behind it — a board
    /// showing everything under a field that still says `cache` would be two answers to one
    /// question.
    pub fn clear_board_filters(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.clear_filters();
        }
        // Writing into the field needs a window, so the refill is left for `fill_task_form` to
        // drain in `render` — the same route a project switch takes.
        self.refill_fields = true;
        cx.notify();
    }

    /// Open the form for a new task, seeded by whatever is in the filter field.
    ///
    /// One field finds work and names it: what you typed to look for a card is what you meant to
    /// call it when there was none. The field is cleared, so the board is not left filtered down to
    /// the one card that is about to be made.
    ///
    /// **Nothing is sent from here.** A card is created by [`Self::create_task`], once it has a
    /// name or a description — pressing this, looking away and pressing it again leaves one empty
    /// form rather than two empty cards. Pressing it while a draft is open returns to that draft
    /// with what was typed still in it.
    pub fn new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(board) = self.board(cx) else {
            return;
        };
        let typed = board.filter.trim().to_string();
        let opened = self.board_mut(cx).is_some_and(|board| board.start_draft());
        if !opened {
            // Already writing one: bring the keyboard back to it and leave the draft alone.
            let input = self.task_title_input.clone();
            input.update(cx, |state, cx| state.focus(window, cx));
            cx.notify();
            return;
        }
        if let Some(board) = self.board_mut(cx) {
            board.filter.clear();
            board.form.title = typed.clone();
            board.form.description.clear();
        }
        for (input, value) in [
            (self.task_filter.clone(), String::new()),
            (self.task_title_input.clone(), typed),
        ] {
            input.update(cx, |state, cx| state.set_value(&value, window, cx));
        }
        let description = self.task_description_input.clone();
        description.update(cx, |state, cx| state.set_value("", window, cx));
        let title = self.task_title_input.clone();
        title.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Send the draft. The first and only save of a new task.
    ///
    /// Refused unless the draft has a title or a description, which is the rule this whole form
    /// exists for. A `CreateTask` carries neither a description nor a generated name, so what is
    /// left of the draft is parked in `board.pending` and finished in [`Self::settle_new_task`]
    /// once the host has answered with an id.
    ///
    /// A draft with a description and no title is created under the description's first line and
    /// then asks the host's own assistance for a written one. The stand-in matters: a model that is
    /// switched off, unreachable or slow leaves a card named after what the user actually wrote,
    /// never an empty one.
    pub fn create_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(board) = self.board(cx) else {
            return;
        };
        if !board.draft || !board.draft_ready() {
            return;
        }
        let typed = board.form.title.trim().to_string();
        let description = board.form.description.clone();
        let session = board.session;
        let name_it = typed.is_empty();
        let title = if name_it {
            stand_in_title(&description)
        } else {
            typed
        };

        self.bus.send(Message::CreateTask {
            project_id,
            title,
            session,
        });
        if let Some(board) = self.board_mut(cx) {
            board.stop_draft();
            board.awaiting_new = true;
            board.pending = Some(PendingTask {
                description,
                name_it,
            });
        }
        // The panel now reports the task that is about to arrive, so the fields go back to being
        // the record's — `form_filled` is what makes `fill_task_form` do it.
        self.form_filled = None;
        self.fill_task_form(window, cx);
        cx.notify();
    }

    /// Give the draft up. Nothing was sent, so there is nothing to unwind.
    pub fn cancel_new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.stop_draft();
        }
        self.form_filled = None;
        self.fill_task_form(window, cx);
        cx.notify();
    }

    /// Finish a task the host has just created: its description, and a written title if it was
    /// made under a stand-in.
    ///
    /// Called from the `TaskCreated` arm, because the id is the host's to mint and neither message
    /// could be sent before it existed.
    pub(super) fn settle_new_task(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        pending: PendingTask,
    ) {
        if !pending.description.trim().is_empty() {
            self.bus.send(Message::UpdateTask {
                project_id,
                task_id,
                title: None,
                description: Some(pending.description),
                priority: None,
            });
        }
        if !pending.name_it {
            return;
        }
        // The same assistance that names a conversation, asked through the same pair of messages.
        // One suggestion is in flight at a time, so an earlier one is given up rather than raced.
        if let Some(suggest_id) = self.suggest.take() {
            self.bus.send(Message::CancelSuggest { suggest_id });
        }
        let suggest_id = SuggestId::generate();
        self.suggest = Some(suggest_id);
        self.task_naming = Some((suggest_id, project_id, task_id));
        self.bus.send(Message::Suggest {
            suggest_id,
            subject: SuggestSubject::TaskTitle {
                project_id,
                task_id,
            },
        });
    }

    /// Put a suggested title on the task that asked for it.
    ///
    /// Sanitised first: the prompt asks for plain text, and this is the net under the prompt — a
    /// `**` or a rocket would otherwise be permanent, because a title is written to the record.
    /// An answer with nothing left in it leaves the stand-in alone.
    pub(super) fn settle_task_title(&mut self, suggest_id: SuggestId, text: &str) -> bool {
        let Some((asked, project_id, task_id)) = self.task_naming else {
            return false;
        };
        if asked != suggest_id {
            return false;
        }
        self.task_naming = None;
        let title = text
            .lines()
            .map(ubiq_proto::assist::plain_text)
            .find(|line| !line.is_empty());
        if let Some(title) = title {
            self.bus.send(Message::UpdateTask {
                project_id,
                task_id,
                title: Some(title),
                description: None,
                priority: None,
            });
        }
        true
    }

    pub fn toggle_board_column(&mut self, status: Status, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.toggle_column(status);
        }
        cx.notify();
    }

    /// Swap the open task between the side panel and a centred modal.
    pub fn toggle_board_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.toggle_popup();
        }
        cx.notify();
    }

    pub fn toggle_task_fold(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.toggle_fold(task);
        }
        cx.notify();
    }

    /// Tick or untick one sub-task.
    ///
    /// A toggle rather than a target state: what unticking lands on is a rule about the work, and
    /// the work is the host's. The checkbox changes when the task comes back.
    pub fn toggle_task_step(&mut self, task_id: TaskId, step_id: StepId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.bus.send(Message::ToggleStep {
            project_id,
            task_id,
            step_id,
        });
        cx.notify();
    }

    /// Pick a card up. It selects itself on the way, for the reason a dragged agent card does:
    /// what is being moved is what the user is looking at.
    pub fn start_task_carry(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.start_carry(task);
            board.select(task);
        }
        cx.notify();
    }

    /// The column under the pointer and the card the pointer is above, which together are what a
    /// drop would file the card into and where. `before` is `None` past the last card, which is the
    /// end of the column.
    pub fn drag_task_over(
        &mut self,
        status: Status,
        before: Option<TaskId>,
        cx: &mut Context<Self>,
    ) {
        if self
            .board_mut(cx)
            .is_some_and(|board| board.carry_over(status, before))
        {
            cx.notify();
        }
    }

    /// The pointer is inside a column but not over a card that has claimed a gap. Only the column
    /// changes, so a drag across cards inside one column keeps the gap the last card named.
    pub fn drag_task_column(&mut self, status: Status, cx: &mut Context<Self>) {
        if self
            .board_mut(cx)
            .is_some_and(|board| board.carry_over_column(status))
        {
            cx.notify();
        }
    }

    /// Put it down. Unlike the graph's canvas, the column *is* the drop target: a card is filed
    /// somewhere rather than placed anywhere, so where it landed is what took the drop.
    ///
    /// Which column a task is in is written down, so the drop asks rather than moves. The card
    /// says it is waiting until the answer comes back, which is what keeps a slow host from
    /// reading as a drag that failed. The projection is spliced the same way so a reorder is
    /// visible without waiting for the host to confirm the order it already holds.
    pub fn drop_task(&mut self, status: Status, before: Option<TaskId>, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let landed = {
            let Some(board) = self.board_mut(cx) else {
                return;
            };
            board.carry_over(status, before);
            let landed = board.end_carry();
            if let Some((task_id, status, _)) = landed {
                board.moving = Some((task_id, status));
            }
            landed
        };
        if let Some((task_id, status, before)) = landed {
            if let Some(work) = self.work_mut(cx) {
                work.place(task_id, status, before);
            }
            self.bus.send(Message::MoveTask {
                project_id,
                task_id,
                status,
                before,
            });
        }
        cx.notify();
    }

    /// The way from a card to the conversation with the agent holding it: the **agents** screen,
    /// that agent in a column of its own. The conversation is what was asked for, and the columns
    /// are where a conversation is had — the graph is the map, not the transcript.
    pub fn open_task_chat(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        self.navigate(Destination::new(project, View::Agents { agent }), cx);
    }

    /// A drag that ended anywhere but a column never reaches a drop handler, so a carry with no
    /// live drag behind it is put down here — and the card stays in the column it came from.
    pub(super) fn settle_board(&mut self, cx: &mut Context<Self>) {
        let stranded = self
            .board(cx)
            .is_some_and(|board| board.carry.is_some() && !cx.has_active_drag());
        if stranded && let Some(board) = self.board_mut(cx) {
            board.carry = None;
        }
    }

    // ── Chat ────────────────────────────────────────────────────────
}

/// How long a title cut from a description may be before it is trimmed to a word boundary.
const STAND_IN_TITLE: usize = 60;

/// A title for a card whose writer gave it none: the first line of its description, cut short.
///
/// What the card is called until assistance answers with something written, and what it stays
/// called if assistance is off or never answers. A first line is a poor title and an honest one —
/// it is what the user typed — which is the point: no card is ever created called nothing.
pub(super) fn stand_in_title(description: &str) -> String {
    let first = description
        .lines()
        .map(|line| ubiq_proto::assist::plain_text(line))
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    if first.chars().count() <= STAND_IN_TITLE {
        return first;
    }
    // On a word boundary, so the stand-in reads as a phrase rather than as a cut string. The ellipsis
    // says it is not the whole of what was written.
    let cut: String = first.chars().take(STAND_IN_TITLE).collect();
    let kept = match cut.rsplit_once(' ') {
        Some((head, _)) if !head.is_empty() => head.to_string(),
        _ => cut,
    };
    format!("{}…", kept.trim_end_matches([',', '.', ';', ':']))
}

#[cfg(test)]
mod tests {
    use super::stand_in_title;

    #[test]
    fn a_card_written_as_a_description_is_never_created_called_nothing() {
        assert_eq!(stand_in_title("Fix the loader"), "Fix the loader");
        // The first line, not the whole of it: a description is prose and a title is a line.
        assert_eq!(
            stand_in_title("\n\nFix the loader\nit runs twice on a cold start"),
            "Fix the loader"
        );
        // Markdown a user typed is not part of the name.
        assert_eq!(stand_in_title("## Fix the **loader**"), "Fix the loader");
        // A description with nothing in it cannot reach here, and answers nothing if it does.
        assert_eq!(stand_in_title("   \n\n"), "");
    }

    #[test]
    fn a_long_first_line_is_cut_on_a_word() {
        let title = stand_in_title(
            "the project picker should remember which folder the last add came from, forever",
        );
        assert!(title.ends_with('…'), "{title}");
        assert!(title.chars().count() <= 61, "{title}");
        assert!(!title.contains("forever"));
        // Cut between words, never through one.
        assert!(title.trim_end_matches('…').ends_with("came"), "{title}");
    }
}
