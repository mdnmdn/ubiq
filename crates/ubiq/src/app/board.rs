use super::*;

use ubiq_proto::messages::TaskField;
use ubiq_proto::work::{Kind, Label};

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

    pub fn commit_task_title(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
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
        let (title, description, key, link) = selected
            .and_then(|id| self.work(cx).and_then(|work| work.task(id)))
            .map(|task| {
                (
                    task.title.clone(),
                    task.description.clone(),
                    task.key.clone().unwrap_or_default(),
                    task.link.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default();

        self.form_filled = selected;
        if let Some(board) = self.board_mut(cx) {
            board.form.title = title.clone();
            board.form.description = description.clone();
            board.form.key = key.clone();
            board.form.link = link.clone();
            board.form.step_title.clear();
            board.form.new_step.clear();
            board.form.new_label.clear();
        }
        for (input, value) in [
            (self.task_title_input.clone(), title),
            (self.task_key_input.clone(), key),
            (self.task_link_input.clone(), link),
            (self.task_label_input.clone(), String::new()),
            (self.step_title_input.clone(), String::new()),
            (self.new_step_input.clone(), String::new()),
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

    /// Ask for a task in the backlog, named by whatever is in the filter field.
    ///
    /// One field finds work and names it: what you typed to look for a card is what you meant to
    /// call it when there was none. The field is cleared, so the board is not left filtered down to
    /// the one card that was just made.
    ///
    /// It cannot select what it asked for, because the id is the host's to mint. `awaiting_new` is
    /// what selects the task that arrives — the same mechanism `AppState::adding` uses to open the
    /// project an `AddProject` answers with.
    pub fn new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(board) = self.board(cx) else {
            return;
        };
        let typed = board.filter.trim().to_string();
        let title = if typed.is_empty() {
            "New task".to_string()
        } else {
            typed
        };
        let session = board.session;
        self.bus.send(Message::CreateTask {
            project_id,
            title,
            session,
        });
        if let Some(board) = self.board_mut(cx) {
            board.filter.clear();
            board.awaiting_new = true;
        }
        let input = self.task_filter.clone();
        input.update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub fn toggle_board_column(&mut self, status: Status, cx: &mut Context<Self>) {
        if let Some(board) = self.board_mut(cx) {
            board.toggle_column(status);
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

    /// Put it down. Unlike the graph's canvas, the column *is* the drop target: a card is filed
    /// somewhere rather than placed anywhere, so where it landed is what took the drop.
    ///
    /// Which column a task is in is written down, so the drop asks rather than moves. The card
    /// says it is waiting until the answer comes back, which is what keeps a slow host from
    /// reading as a drag that failed.
    pub fn drop_task(&mut self, status: Status, before: Option<TaskId>, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(board) = self.board_mut(cx) else {
            return;
        };
        board.carry_over(status, before);
        if let Some((task_id, status, before)) = board.end_carry() {
            board.moving = Some((task_id, status));
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
