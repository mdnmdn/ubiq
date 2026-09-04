use super::*;

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
        shape: Option<Shape>,
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
            shape,
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
        self.update_task(Some(typed), None, None, None, cx);
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
        self.update_task(None, Some(typed), None, None, cx);
    }

    pub fn set_task_priority(&mut self, priority: Priority, cx: &mut Context<Self>) {
        self.update_task(None, None, Some(priority), None, cx);
    }

    pub fn set_task_shape(&mut self, shape: Shape, cx: &mut Context<Self>) {
        self.update_task(None, None, None, Some(shape), cx);
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
        let (title, description) = selected
            .and_then(|id| self.work(cx).and_then(|work| work.task(id)))
            .map(|task| (task.title.clone(), task.description.clone()))
            .unwrap_or_default();

        self.form_filled = selected;
        if let Some(board) = self.board_mut(cx) {
            board.form.title = title.clone();
            board.form.description = description.clone();
            board.form.step_title.clear();
            board.form.new_step.clear();
        }
        for (input, value) in [
            (self.task_title_input.clone(), title),
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

    /// The column under the pointer, which is what a drop would file the card into.
    pub fn drag_task_over(&mut self, status: Status, cx: &mut Context<Self>) {
        if self
            .board_mut(cx)
            .is_some_and(|board| board.carry_over(status))
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
    pub fn drop_task(&mut self, status: Status, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(board) = self.board_mut(cx) else {
            return;
        };
        board.carry_over(status);
        if let Some((task_id, status)) = board.end_carry() {
            board.moving = Some((task_id, status));
            self.bus.send(Message::MoveTask {
                project_id,
                task_id,
                status,
            });
        }
        cx.notify();
    }

    /// Take a task to the orchestration screen: the graph, pointed at whoever is doing it.
    pub fn show_task_in_graph(&mut self, task: TaskId, cx: &mut Context<Self>) {
        let selection = self.work(cx).and_then(|work| {
            let task = work.task(task)?;
            work.now(task)
                .map(|agent| Selection::Agent(agent.id))
                .or_else(|| task.session.map(Selection::Session))
        });
        if let Some(selection) = selection
            && let Some(graph) = self.graph_mut(cx)
        {
            graph.selection = Some(selection);
        }
        self.set_rail_mode(RailMode::Orchestration, cx);
    }

    /// The way from a card to the conversation with the agent holding it: the **agents** screen,
    /// that agent in a column of its own. The conversation is what was asked for, and the columns
    /// are where a conversation is had — the graph is the map, not the transcript.
    pub fn open_task_chat(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.reveal_agent(agent, cx);
        self.set_rail_mode(RailMode::Agents, cx);
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
