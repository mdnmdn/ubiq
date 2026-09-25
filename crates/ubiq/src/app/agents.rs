use super::*;
use crate::state::ConvBlock;
use crate::state::conversation::ActivityPanel;

/// How often [`AppState::watch_for_dump_path`] looks to see whether the host has named the file,
/// and how many times before it stops looking. Four seconds all told: long enough for a round trip
/// to a host on the other end of a socket, short enough that nothing is still waiting by the time
/// the user has forgotten they clicked.
const DUMP_PATH_POLL: std::time::Duration = std::time::Duration::from_millis(100);
const DUMP_PATH_TRIES: usize = 40;

impl AppState {
    /// What a reader is shown as this agent's name — the dock tab, the column's own header and
    /// its tab strip, every surface that used to print [`WorkAgent::name`] directly.
    ///
    /// **The definition it was started from outranks the harness-label default, until something real
    /// replaces both.** `WorkAgent::summary` is `None` until the harness (or a user rename) names
    /// the conversation for itself — `refresh_agent_record`'s own signal — so that is the gate:
    /// a fresh agent nothing has named yet shows the definition it was picked from rather than the
    /// bare harness label every unnamed conversation used to wear, and a named one shows what it
    /// was actually named, same as before. No definition remembered (a bare harness start, or a
    /// window reloaded since) falls through to `WorkAgent::name` unchanged.
    pub fn agent_title(&self, agent: &ubiq_proto::work::WorkAgent) -> SharedString {
        if agent.summary.is_none()
            && let Some(definition) = self.workbench.agent_started_definition.get(&agent.id)
        {
            return SharedString::from(definition.clone());
        }
        if agent.name.is_empty() {
            SharedString::from(agent.harness.clone())
        } else {
            SharedString::from(agent.name.clone())
        }
    }

    /// Bring an agent to the front: the tab of whatever column holds it, or a column of its own.
    /// The one thing a click in the sidebar does.
    pub fn reveal_agent(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.reveal(agent);
        }
        self.refill_columns = true;
        cx.notify();
    }

    /// Add an agent to one column's strip. What a column's `+` does — grouping it with whatever is
    /// already there rather than widening the row.
    pub fn group_agent_into(&mut self, column: usize, agent: AgentId, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.open_in(column, agent);
        }
        self.refill_columns = true;
        cx.notify();
    }

    // ── a column's `+` menu ───────────────────────────────────────────

    /// Open one column's `+` menu, with a fresh search field — the same "clear on open" rule
    /// every menu sharing `picker_search` follows, so it never opens holding what was typed into
    /// a different one.
    pub fn open_agent_bench_menu(
        &mut self,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::AgentBench(column));
        let picker_search = self.picker_search.clone();
        picker_search.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Pick a row of a column's `+` menu: the same [`AgentsView::bench_rows`] list the menu drew,
    /// filtered by whatever is typed into the search field, so an index here names the row the
    /// user actually saw. A pick landing on a `Label`, a `Separator`, or an agent already on
    /// screen elsewhere — drawn disabled — does nothing.
    pub fn pick_agent_bench_menu(
        &mut self,
        column: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.picker_search.read(cx).value().to_string();
        let picked = match (self.agents(cx), self.work(cx)) {
            (Some(agents), Some(work)) => {
                match agents.bench_rows(column, work, &query).get(index) {
                    Some(BenchRow::Agent {
                        id,
                        disabled: false,
                    }) => Some(*id),
                    _ => None,
                }
            }
            _ => None,
        };
        self.close_menu(cx);
        self.clear_picker_search(window, cx);
        if let Some(id) = picked {
            self.group_agent_into(column, id, cx);
            // The column's own slot, read after the move rather than assumed: `column` is a
            // position in the row, not a slot, and grouping never renumbers the row for a pick
            // off the bench (nothing was displaced), but reading it back is what
            // `restore_composer_draft` addresses.
            let slot = self
                .agents(cx)
                .and_then(|agents| agents.columns.get(column))
                .map(|col| col.slot);
            if let Some(slot) = slot {
                self.restore_composer_draft(id, slot, window, cx);
            }
        }
    }

    pub fn dismiss_agent_bench_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        self.clear_picker_search(window, cx);
    }

    /// Take an agent off the field. **The agent keeps running** — a column tab is a view onto a
    /// conversation, not the harness's screen — so this benches it and the sidebar still lists it.
    pub fn bench_agent(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.bench(agent);
        }
        self.refill_columns = true;
        cx.notify();
    }

    pub fn select_column_tab(&mut self, column: usize, tab: usize, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.select_tab(column, tab);
        }
        // Which agent a composer is addressed at is what its placeholder says, so a tab change
        // owes it one.
        self.refill_columns = true;
        cx.notify();
    }

    pub fn focus_agent_column(&mut self, column: usize, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.focus_column(column);
        }
        cx.notify();
    }

    pub fn toggle_agents_session(&mut self, session: SessionId, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.toggle_session(session);
        }
        cx.notify();
    }

    /// The Missions section's own fold: one row expanded to its roster (M14).
    pub fn toggle_mission_row(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.toggle_mission_row(task_id);
        }
        cx.notify();
    }

    /// The Missions section's own toggle: whether `Completed` and `Abandoned` missions are drawn
    /// too.
    pub fn toggle_show_closed_missions(&mut self, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.toggle_show_closed_missions();
        }
        cx.notify();
    }

    /// A tab has been picked up. Nothing moves yet: what the drop lands on is what decides whether
    /// it groups or splits.
    pub fn start_tab_drag(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.dragging = Some(agent);
        }
        cx.notify();
    }

    /// A tab dropped on a column joins it.
    pub fn drop_tab_on(&mut self, column: usize, cx: &mut Context<Self>) {
        let Some(agent) = self.agents(cx).and_then(|agents| agents.dragging) else {
            return;
        };
        if let Some(agents) = self.agents_mut(cx) {
            agents.dragging = None;
            agents.open_in(column, agent);
        }
        self.refill_columns = true;
        cx.notify();
    }

    /// A tab dropped past the last column opens one of its own.
    pub fn drop_tab_at_end(&mut self, cx: &mut Context<Self>) {
        let Some(agent) = self.agents(cx).and_then(|agents| agents.dragging) else {
            return;
        };
        if let Some(agents) = self.agents_mut(cx) {
            agents.dragging = None;
            let at = agents.columns.len();
            agents.split_off(agent, at);
        }
        self.refill_columns = true;
        cx.notify();
    }

    /// A drag that ended anywhere but a column never reaches a drop handler, so a tab left in the
    /// air is put down here — and it stays in the column it came from.
    pub(super) fn settle_tab_drag(&mut self, cx: &mut Context<Self>) {
        let stranded = self
            .agents(cx)
            .is_some_and(|agents| agents.dragging.is_some() && !cx.has_active_drag());
        if stranded && let Some(agents) = self.agents_mut(cx) {
            agents.dragging = None;
        }
    }

    /// Which agent slot `slot` is currently addressed at.
    ///
    /// A chat tab's own attachment for a slot in the upper range; otherwise the active tab of
    /// whichever column owns a slot in the lower one. The one place this is answered, so the
    /// Enter-key path and the click-based Send/Enqueue button resolve the same agent for the same
    /// slot on every surface that hosts a composer.
    pub fn agent_for_slot(&self, slot: usize, cx: &App) -> Option<AgentId> {
        // The sink's bench addresses whatever conversation it is reading, which is the page's
        // own pick rather than anything the arrangement holds.
        if slot == crate::state::agents::SINK_SLOT {
            return self.sink_agent();
        }
        if slot >= COLUMNS_MAX {
            return self
                .open_project(cx)?
                .chats
                .iter()
                .find(|tab| tab.slot == slot)
                .and_then(|tab| tab.attached);
        }
        self.agents(cx)?
            .columns
            .iter()
            .find(|column| column.slot == slot)
            .and_then(|column| column.active_agent())
    }

    /// Fold what is typed into the agent's own conversation, beside its attachments and its
    /// queue — [`Conversation::draft`] — rather than only the composer slot's own mirror.
    ///
    /// A slot is a place in the window's furniture; an agent's unsent draft is a fact about the
    /// conversation, so it has to survive whatever happens to the slot that was drawing it — a
    /// chat tab hidden and reopened, a column an arrangement change moved the agent out of. This
    /// is the write side of that; [`Self::restore_composer_draft`] is the read side.
    pub(super) fn remember_conversation_draft(
        &mut self,
        agent_id: AgentId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project_of_agent(agent_id, cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&project_id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.draft = text;
        }
    }

    /// Put an agent's own unsent draft back into the composer now addressing it, so reattaching to
    /// a conversation — the chat panel's *Attach running*, most of all — picks up where the last
    /// surface that had it open left off. Only when the field is empty, the same guard
    /// [`Self::recall_last_message`] uses: a composer already being written into is never
    /// overwritten by a fact from elsewhere.
    pub(super) fn restore_composer_draft(
        &mut self,
        agent_id: AgentId,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.column_inputs.get(slot).cloned() else {
            return;
        };
        if !input.read(cx).value().is_empty() {
            return;
        }
        let Some(draft) = self
            .teams_conversation(agent_id, cx)
            .map(|conversation| conversation.draft.clone())
            .filter(|draft| !draft.is_empty())
        else {
            return;
        };
        input.update(cx, |state, cx| {
            state.set_value(&draft, window, cx);
        });
        if let Some(agents) = self.agents_mut(cx) {
            agents.set_draft(slot, draft);
        }
    }

    /// What one composer sends, to the agent its slot is addressed at.
    ///
    /// Nothing is appended here, for the reason [`Self::send_to_agent`] appends nothing: the line
    /// lands in the thread when the host answers with the agent carrying it.
    pub fn steer_column(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent_id) = self.agent_for_slot(slot, cx) else {
            return;
        };
        // Every column holds a live agent (T-143): `AgentsView::live` is exactly
        // `open.conversations.keys()`, and a tab whose agent drops out of it is pruned, so this
        // always sends or queues, never `Message::SendToAgent` — that path is for a screen with
        // agents that have no conversation, which a column can no longer be.
        self.send_or_enqueue(agent_id, slot, window, cx);
    }

    /// Take a turn to a live agent from one of the window's pooled composers.
    ///
    /// Nothing is appended: the line is drawn when the harness echoes it back as a `UserChunk`,
    /// which is what it actually received.
    pub fn prompt_agent(
        &mut self,
        agent_id: AgentId,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self.column_inputs.get(slot) else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        let Some(conversation) = self.teams_conversation(agent_id, cx) else {
            return;
        };
        if !conversation.accepts_input {
            return;
        }
        // Attachments are composed into the one prompt text as `@path` mentions — nothing new
        // crosses the bus for them. Which is also why a turn with attachments and nothing typed
        // is still something to send.
        let text = conversation.compose_prompt(&typed);
        if text.is_empty() {
            return;
        }
        self.send_prompt(agent_id, text);
        self.clear_attachments(agent_id, cx);
        self.clear_composer(slot, window, cx);
        cx.notify();
    }

    /// Send this composer's draft, or hold it for later — the one function both the composer's
    /// button and its Enter-key path call, so the two ways to send a live agent's conversation
    /// agree on what "send" means while a turn is running.
    ///
    /// Not running: the same as [`Self::prompt_agent`]. Running with something typed: the draft
    /// is queued on the conversation instead of sent, and the composer is cleared the same way a
    /// send clears it. Running with nothing typed: nothing to send or hold, so nothing happens —
    /// Stop is a separate control, not reached through this one.
    pub fn send_or_enqueue(
        &mut self,
        agent_id: AgentId,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let working = self
            .teams_conversation(agent_id, cx)
            .is_some_and(|conversation| conversation.run == Run::Working);
        if !working {
            self.prompt_agent(agent_id, slot, window, cx);
            return;
        }
        let Some(input) = self.column_inputs.get(slot) else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        // A queued prompt is one string, so its attachments are composed into it here rather than
        // held per entry: a queue row that carried its own tag list would need its own tag row,
        // its own removes and its own colouring, which is a second composer. The paths are in the
        // text the row previews, and an edit brings them back into the field as text.
        let Some(text) = self
            .teams_conversation(agent_id, cx)
            .map(|conversation| conversation.compose_prompt(&typed))
            .filter(|text| !text.is_empty())
        else {
            return;
        };
        // The queue is the conversation's own, so it is written where the conversation lives.
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.enqueue(text);
            conversation.clear_attached();
        }
        self.clear_composer(slot, window, cx);
        cx.notify();
    }

    /// Take every attachment off the composer and hang it on the turn that is leaving — what a
    /// prompt leaving consumes, the same moment the draft is cleared.
    ///
    /// The tags do not simply go away: the paths are inside the `PromptAgent` text as `@path`
    /// mentions and the harness echoes that text back, so the transcript has to be told which of
    /// them were files the user attached. See `Conversation::expect_attachments`.
    fn clear_attachments(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.expect_attachments();
        }
    }

    /// Put the last thing said to this composer's agent back into it, and say whether it did.
    ///
    /// Only when the field is empty: a composer with a draft in it is being written, and a key
    /// that overwrites what is typed is a key that loses work. The transcript is the history —
    /// nothing is kept beside it, because the turns the harness echoed back are what was actually
    /// sent.
    pub fn recall_last_message(
        &mut self,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(input) = self.column_inputs.get(slot).cloned() else {
            return false;
        };
        if !input.read(cx).value().is_empty() {
            return false;
        }
        let Some(text) = self
            .agent_for_slot(slot, cx)
            .and_then(|agent_id| self.teams_conversation(agent_id, cx))
            .and_then(|conversation| {
                conversation
                    .blocks
                    .iter()
                    .rev()
                    .find_map(|block| match block {
                        ConvBlock::User { text, .. } => Some(text.clone()),
                        _ => None,
                    })
            })
        else {
            return false;
        };
        input.update(cx, |state, cx| state.set_value(text, window, cx));
        cx.notify();
        true
    }

    // ── how tall a composer is ────────────────────────────────────────

    /// Note where a drag on a composer's top edge began, and how tall it was then. Everything
    /// after it is measured from here.
    ///
    /// A field nobody has resized has no height of its own to read — it is however tall what is
    /// typed makes it, and the figure the library computes for that is not reachable from here. So
    /// the drag starts from an estimate of what is on screen: the lines in the draft, inside the
    /// same bounds the field grows between. Estimating is what keeps the first pixel of the drag
    /// from jumping — beginning at the ceiling would take a one-line field to six rows the moment
    /// the pointer moved.
    pub fn start_composer_resize(&mut self, slot: usize, at: f32, cx: &mut Context<Self>) {
        let rows = match self.composer_rows.get(slot).copied().flatten() {
            Some(rows) => rows,
            None => self
                .column_inputs
                .get(slot)
                .map(|input| input.read(cx).value().lines().count().max(1))
                .unwrap_or(1)
                .clamp(COMPOSER_ROWS_MIN, COMPOSER_ROWS_MAX_DEFAULT),
        };
        self.composer_drag = Some((slot, at, rows));
        cx.notify();
    }

    /// Follow the pointer. The top edge is what is being dragged, so up is taller — the pointer
    /// rising by a row's height is one more row of writing space.
    pub fn drag_composer_resize(&mut self, at: f32, cx: &mut Context<Self>) {
        let Some((slot, from, rows)) = self.composer_drag else {
            return;
        };
        let grown = (from - at) / COMPOSER_ROW_HEIGHT;
        let rows = (rows as f32 + grown)
            .round()
            .clamp(COMPOSER_ROWS_MIN as f32, COMPOSER_ROWS_MAX as f32) as usize;
        self.set_composer_rows(slot, Some(rows), cx);
    }

    /// Let the edge go. The height it was dragged to stays; only the drag ends.
    pub fn end_composer_resize(&mut self, cx: &mut Context<Self>) {
        if self.composer_drag.take().is_some() {
            cx.notify();
        }
    }

    /// Put one composer at a height. `None` hands it back to the pool's own behaviour — one row,
    /// growing with what is typed — which is what a double-click on the edge asks for.
    ///
    /// A resized field is that tall empty or full: `min` and `max` are the same number, because a
    /// space asked for that collapsed the moment it was emptied would not be a space.
    pub fn set_composer_rows(&mut self, slot: usize, rows: Option<usize>, cx: &mut Context<Self>) {
        let Some(current) = self.composer_rows.get_mut(slot) else {
            return;
        };
        if *current == rows {
            return;
        }
        *current = rows;
        let Some(input) = self.column_inputs.get(slot).cloned() else {
            return;
        };
        let (min, max) = match rows {
            Some(rows) => (rows, rows),
            None => (1, COMPOSER_ROWS_MAX_DEFAULT),
        };
        input.update(cx, |state, cx| state.set_auto_grow(min, max, cx));
        cx.notify();
    }

    /// Take a queued prompt back out and load it into the composer — a queue row's edit control.
    pub fn edit_queued_message(
        &mut self,
        agent_id: AgentId,
        slot: usize,
        queued_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A queue row belongs to whichever agent it lists, not to the window's active project: the
        // Teams canvas draws every open project's queue under the window span, and editing one is a
        // write into the project that holds it.
        let Some(id) = self.project_of_agent(agent_id, cx) else {
            return;
        };
        let Some(text) = self
            .projects
            .get_mut(&id)
            .and_then(|open| open.conversations.get_mut(&agent_id))
            .and_then(|conversation| conversation.remove_queued(queued_id))
        else {
            return;
        };
        if let Some(agents) = self.agents_mut(cx) {
            agents.set_draft(slot, text.clone());
        }
        let Some(input) = self.column_inputs.get(slot).cloned() else {
            return;
        };
        input.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Drop a queued prompt outright — a queue row's delete control.
    pub fn delete_queued_message(
        &mut self,
        agent_id: AgentId,
        queued_id: u64,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.remove_queued(queued_id);
        }
        cx.notify();
    }

    /// Send a queued prompt down the wire right now — a queue row's Send ASAP control.
    ///
    /// The queue's normal flush waits for [`Run::Idle`], the turn to end. This is the row's way
    /// of not waiting: the entry comes back out and goes straight to [`Self::send_prompt`], the
    /// same wire call a live turn uses. Nothing is cancelled — the turn in flight keeps running,
    /// and the harness picks the prompt up at its next step.
    pub fn send_queued_message_now(
        &mut self,
        agent_id: AgentId,
        queued_id: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.project_of_agent(agent_id, cx) else {
            return;
        };
        let Some(text) = self
            .projects
            .get_mut(&id)
            .and_then(|open| open.conversations.get_mut(&agent_id))
            .and_then(|conversation| conversation.remove_queued(queued_id))
        else {
            return;
        };
        self.send_prompt(agent_id, text);
        cx.notify();
    }

    /// Interrupt the turn in flight.
    ///
    /// Every prompt still up goes with it: the host answers each outstanding request as cancelled
    /// on its way to the cancel, so a prompt left on screen would offer an answer to a question
    /// already closed.
    pub fn cancel_turn(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        self.bus.send(Message::CancelTurn { agent_id });
        // The pending list cleared here is the agent's own project's, not the window's: the wire
        // answer settles the turn regardless, but the local clear has to land on the record the
        // agent actually lives in, or a foreign card under the window span keeps its stale pending.
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.pending.clear();
        }
        cx.notify();
    }

    /// Go ahead with what the conversation being read is asking about — ⌘⌥Y.
    pub fn allow_permission(
        &mut self,
        _: &AllowPermission,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(agent_id) = self.read_conversation(cx) {
            self.answer_oldest_permission(agent_id, true, cx);
        }
    }

    /// Refuse it — ⌘⌥N.
    pub fn reject_permission(
        &mut self,
        _: &RejectPermission,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(agent_id) = self.read_conversation(cx) {
            self.answer_oldest_permission(agent_id, false, cx);
        }
    }

    /// End a conversation outright — unlike [`Self::cancel_turn`], this closes it rather than
    /// interrupting the turn in flight.
    pub fn end_conversation(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        self.bus.send(Message::EndConversation { agent_id });
        cx.notify();
    }

    /// Kill the harness, keeping the conversation, its transcript and its run directory —
    /// unlike [`Self::end_conversation`], which takes all three with it.
    pub fn unload_agent(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        self.bus.send(Message::UnloadConversation { agent_id });
        cx.notify();
    }

    /// Kill the harness's process now, keeping the conversation, its transcript and its run
    /// directory — so [`Self::resume_agent`] brings it back.
    ///
    /// What is left when Stop has nothing to interrupt with. [`Self::cancel_turn`] asks the
    /// harness to end the turn and [`Self::unload_agent`] asks it to shut down; a harness that has
    /// stopped answering answers neither, and this is the verb that does not ask.
    pub fn abort_agent(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        self.bus.send(Message::AbortConversation { agent_id });
        cx.notify();
    }

    /// Start an unloaded conversation's harness again, under the same `agent_id`, with no prompt.
    pub fn resume_agent(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        self.bus.send(Message::ResumeConversation { agent_id });
        cx.notify();
    }

    // ── the conversation's three-dots lifecycle menu ─────────────────

    /// Open a conversation's lifecycle menu (Stop, Unload, Resume, Delete), anchored where the
    /// three dots were clicked.
    pub fn open_conversation_menu(
        &mut self,
        agent_id: AgentId,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::ConversationLifecycle(agent_id));
        self.workbench.conversation_menu = Some(at);
        cx.notify();
    }

    /// Pick a row of the lifecycle menu, in the order it draws them:
    ///
    /// ```text
    /// 0 Stop   1 Abort   2 Unload   3 Resume
    /// 4 ─────
    /// 5 Info   6 Fork   7 persistence   8 accept-all   9 dump
    /// 10 ─────
    /// 11 Hide   12 Close
    /// ```
    ///
    /// The order is [`crate::ui::conversation::lifecycle_menu_rows`]'s and nothing else's — the
    /// rows are dispatched by position, so the two are read together or not at all. **The two
    /// separators are dead indices here**, and deliberately so: a hairline occupies a row in the
    /// list it is drawn from, so it has to occupy one in the match that answers it.
    ///
    /// Close does not act here — it raises a confirm instead, being the one destructive,
    /// irreversible verb on the menu. Hide, beside it, ends nothing.
    pub fn pick_conversation_menu(
        &mut self,
        agent_id: AgentId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_conversation_menu(cx);
        match index {
            0 => self.cancel_turn(agent_id, cx),
            1 => self.abort_agent(agent_id, cx),
            2 => self.unload_agent(agent_id, cx),
            3 => self.resume_agent(agent_id, cx),
            // 4 is the hairline above the tools.
            5 => self.open_conversation_info(agent_id, cx),
            6 => self.fork_conversation(agent_id, cx),
            7 => self.toggle_conversation_persistent(agent_id, cx),
            8 => self.toggle_conversation_accept_all(agent_id, cx),
            9 => self.toggle_conversation_debug_dump(agent_id, cx),
            // 10 is the hairline above the closing pair.
            11 => self.hide_conversation_view(agent_id, cx),
            12 => {
                self.workbench.confirm_end_conversation = Some(agent_id);
                cx.notify();
            }
            _ => {}
        }
    }

    /// Put this conversation's view away. **Nothing ends.** The chat tab attached to the agent is
    /// closed and the conversation goes on being the host's — it keeps its harness, keeps taking
    /// turns, and the sidebar goes on listing it, one click from being looked at again.
    ///
    /// This is the window acting on its own arrangement, so it is the arrangement that is read:
    /// the tab is found by what it is attached to, and a conversation no tab is showing has
    /// nothing to hide, which is why the menu draws the row dead in that case.
    pub fn hide_conversation_view(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        let tab = self.open_project(cx).and_then(|open| {
            open.chats
                .iter()
                .find(|tab| tab.attached == Some(agent_id))
                .map(|tab| tab.id)
        });
        if let Some(tab) = tab {
            self.close_chat_tab(tab, cx);
        }
    }

    // ── the conversation's Info panel ────────────────────────────────

    /// Raise the Info panel over one conversation: what it is running as, what it has spent, and
    /// the three directories it lives in.
    ///
    /// Reads nothing and asks nothing. Everything it draws is already in the window — the work
    /// record the host broadcasts and the conversation's own state — so opening it is a flag, and
    /// a conversation that has not launched yet simply has fewer answers to give.
    pub fn open_conversation_info(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        self.conversation_info = Some(agent_id);
        self.conversation_info_capabilities = false;
        cx.notify();
    }

    pub fn dismiss_conversation_info(&mut self, cx: &mut Context<Self>) {
        self.conversation_info = None;
        self.conversation_info_capabilities = false;
        cx.notify();
    }

    /// Show or hide the Info panel's ACP capabilities section.
    ///
    /// The one thing in that panel that asks the host anything, and expanding it is what puts the
    /// question: the record is a harness fact the host already holds, so the ask is a map lookup
    /// there. Asked on every expansion rather than once for the life of the window, for the reason
    /// [`crate::state::settings::SettingsState::acp_asked`] is cleared on a visit — a conversation
    /// started since the last look is exactly what turns "none has yet" into a record.
    ///
    /// `harness` is the display label the work record carries —
    /// [`crate::state::workbench::WorkbenchState::agent_type_by_label`] is what turns it back into
    /// the harness the question is about.
    pub fn toggle_conversation_info_capabilities(
        &mut self,
        harness: String,
        cx: &mut Context<Self>,
    ) {
        self.conversation_info_capabilities = !self.conversation_info_capabilities;
        if self.conversation_info_capabilities
            && let Some(id) = self
                .workbench
                .agent_type_by_label(&harness)
                .map(|info| info.id.clone())
        {
            self.workbench.settings.acp_asked.remove(&id);
            self.ask_acp_capabilities(id);
        }
        cx.notify();
    }

    /// Show one of a conversation's directories in the desktop's own file manager.
    ///
    /// The window is handed the path by the host and hands it straight to the platform: it does
    /// not read it, list it, or learn anything about what a harness keeps in it. A path that will
    /// not open is left to the log — there is nothing the reader could do about it here that
    /// opening their own file manager would not do better.
    pub fn reveal_conversation_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if let Err(error) = open_in_system(&path) {
            tracing::warn!(%path, "revealing a conversation directory failed: {error}");
        }
        cx.notify();
    }

    /// Mark a conversation as one to keep, or stop keeping it. Nothing is drawn optimistically:
    /// the host owns the record, and the glyph moves when the work snapshot says it did — a mark
    /// that flips on the click and back on the refusal is worse than one that waits.
    pub fn toggle_conversation_persistent(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        // The three-dots menu reads the work record through the agent's own span, since a Teams
        // card selected under the window span may be reporting on a project other than the active
        // one.
        let persistent = self
            .teams_agent(agent_id, cx)
            .is_some_and(|agent| agent.persistent);
        self.bus.send(Message::SetConversationPersistent {
            agent_id,
            persistent: !persistent,
        });
        cx.notify();
    }

    /// Answer every permission request on this conversation with allow, without showing any of
    /// them — or stop doing so. The host does the answering; nothing about it is decided here.
    ///
    /// Nothing is drawn optimistically, for [`Self::toggle_conversation_persistent`]'s reason: the
    /// host owns the record, and the row's wording turns over when the work snapshot says it did.
    /// A label that flips on the click and back on the refusal would be worse than one that waits
    /// — and worse here than there, because what it would be wrong about is whether the next
    /// question gets asked at all.
    pub fn toggle_conversation_accept_all(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        let accept_all = self
            .teams_agent(agent_id, cx)
            .is_some_and(|agent| agent.accept_all);
        self.bus.send(Message::SetConversationAcceptAll {
            agent_id,
            accept_all: !accept_all,
        });
        cx.notify();
    }

    /// Start writing this conversation's traffic to a file, or stop. The window asks for the dump
    /// and is told afterwards where it went: the host chooses the path and reports it back on
    /// `WorkAgent::debug_dump`, which is both what the menu row's wording reads and what its
    /// tooltip says.
    ///
    /// So the message carries a `bool` and the record carries a path, and nothing is drawn
    /// optimistically — for [`Self::toggle_conversation_persistent`]'s reason, and because there
    /// is no path to draw until the host has picked one.
    ///
    /// **Both edges put the path on the clipboard**, because both are the moment the user is
    /// thinking about the file. They reach it from opposite directions. Stopping has the path in
    /// hand: it is on the record now and will not be in a moment, so it is copied here. Starting
    /// has nothing yet — the file is the host's to name — so [`Self::dump_copy_pending`] records
    /// who asked and [`Self::watch_for_dump_path`] copies the answer when it lands.
    ///
    /// No toast follows it. The window's notification list is the host's — every row in it is a
    /// broadcast from the other half — and raising a purely local one would mean either a message
    /// for something no other window cares about or a second, window-only list beside the one that
    /// is there. The menu row's tooltip already says the path, which is the same fact in the place
    /// the click just happened.
    pub fn toggle_conversation_debug_dump(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        let dumping = self
            .teams_agent(agent_id, cx)
            .and_then(|agent| agent.debug_dump.clone());
        self.bus.send(Message::SetConversationDebugDump {
            agent_id,
            debug_dump: dumping.is_none(),
        });
        match dumping {
            Some(path) => {
                // A capture being turned off is no longer one anybody is waiting on a name for.
                self.dump_copy_pending = None;
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(path));
            }
            None => {
                self.dump_copy_pending = Some(agent_id);
                self.watch_for_dump_path(agent_id, cx);
            }
        }
        cx.notify();
    }

    /// Wait for the host to name the file it just started writing, and put it on the clipboard.
    ///
    /// **A poll rather than a hook on the arriving record.** The answer reaches the window as an
    /// ordinary work update folded in with every other, and there is one place that does the
    /// folding for every screen; hanging a clipboard write off it would make one surface's menu a
    /// concern of the wire's. Whoever asked waits here instead, reading the record they already
    /// have, which is also what makes the wait give up on its own: a host that never answers
    /// leaves nothing behind but a cleared field.
    fn watch_for_dump_path(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            for _ in 0..DUMP_PATH_TRIES {
                cx.background_executor().timer(DUMP_PATH_POLL).await;
                let settled = this.update(cx, |this, cx| {
                    // Somebody else's answer, or the same user turning the capture off again
                    // before it was named: either way this wait is over and copies nothing.
                    if this.dump_copy_pending != Some(agent_id) {
                        return true;
                    }
                    let Some(path) = this
                        .teams_agent(agent_id, cx)
                        .and_then(|agent| agent.debug_dump.clone())
                    else {
                        return false;
                    };
                    this.dump_copy_pending = None;
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(path));
                    cx.notify();
                    true
                });
                if !matches!(settled, Ok(false)) {
                    return;
                }
            }
            let _ = this.update(cx, |this, _| {
                if this.dump_copy_pending == Some(agent_id) {
                    this.dump_copy_pending = None;
                }
            });
        })
        .detach();
    }

    /// Fork a conversation: a second agent, launched from a copy of this one's run directory, so
    /// the two share every turn up to here and diverge from the next one on.
    ///
    /// The new agent's id is minted here, the way [`Self::start_new_agent`] mints one — the window
    /// names the agent it asked for, and the host answers under that name. The source is untouched.
    pub fn fork_conversation(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        // The forked agent's own project, not the window's: this files a `ReviveConversation`
        // against a project, and under the window span the source may belong to any open one — the
        // fork has to land beside the agent it copies, not wherever the window is pointed.
        let Some(project_id) = self.project_of_agent(agent_id, cx) else {
            return;
        };
        self.bus.send(Message::ReviveConversation {
            source: agent_id,
            agent_id: AgentId::generate(),
            project_id,
            session_id: self.session,
        });
        cx.notify();
    }

    pub fn dismiss_conversation_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.conversation_menu = None;
        cx.notify();
    }

    /// Delete's confirm answered yes.
    pub fn confirm_end_conversation(&mut self, cx: &mut Context<Self>) {
        if let Some(agent_id) = self.workbench.confirm_end_conversation.take() {
            self.end_conversation(agent_id, cx);
        }
        cx.notify();
    }

    pub fn dismiss_end_conversation_confirm(&mut self, cx: &mut Context<Self>) {
        self.workbench.confirm_end_conversation = None;
        cx.notify();
    }

    /// Bench every agent on screen — the "Close all" control on the agents screen.
    ///
    /// Closing one tab benches the agent behind it rather than ending it — see `state::agents`'s
    /// module doc — and this is that same thing for every tab in every column at once, not
    /// `end_conversation`: closing a column does not kill what was running in it, and "close all"
    /// is not the exception. A screen with nothing on screen is a correct no-op.
    pub fn close_all_conversations(&mut self, cx: &mut Context<Self>) {
        let Some(agents) = self.agents(cx) else {
            return;
        };
        let ids: Vec<AgentId> = agents
            .columns
            .iter()
            .flat_map(|column| column.tabs.iter().copied())
            .collect();
        for id in ids {
            self.bench_agent(id, cx);
        }
    }

    /// Answer a permission the agent is waiting on, naming one of the options it offered.
    pub fn answer_permission(
        &mut self,
        agent_id: AgentId,
        request_id: String,
        option_id: String,
        cx: &mut Context<Self>,
    ) {
        // This prompt goes as its answer does — and only this one: a second request outstanding
        // under another id is still waiting, and clearing it here would strand the turn on a
        // question nobody can answer any more.
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.answered(&request_id);
        }
        self.bus.send(Message::AnswerPermission {
            agent_id,
            request_id,
            option_id,
        });
        cx.notify();
    }

    /// Answer the oldest request this conversation is waiting on, going ahead or refusing — what
    /// ⌘⌥Y and ⌘⌥N do. The option is the first the harness offered of that reading; a harness that
    /// offered none of it is left alone rather than answered with the other.
    pub fn answer_oldest_permission(
        &mut self,
        agent_id: AgentId,
        allow: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(conversation) = open.conversations.get(&agent_id) else {
            return;
        };
        let Some(pending) = conversation.oldest_pending() else {
            return;
        };
        let Some(option) = pending.option_for(allow) else {
            return;
        };
        let (request_id, option_id) = (pending.request_id.clone(), option.option_id.clone());
        self.answer_permission(agent_id, request_id, option_id, cx);
    }

    /// The conversation the keyboard means: the active tab of the agents screen's focused column.
    fn read_conversation(&self, cx: &App) -> Option<AgentId> {
        let agents = self.agents(cx)?;
        agents.columns.get(agents.focus)?.active_agent()
    }

    /// Pick a value for one launch-time config option before this conversation's harness has
    /// launched — the send is the same `SetAgentConfig` a live conversation would use, but here
    /// nothing is running yet to answer it, so the pick is also kept locally for the picker to
    /// highlight.
    pub fn pick_agent_config(
        &mut self,
        agent_id: AgentId,
        config_id: String,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // This UI-only state — the picker's choice, its open flag, a panel, a tool's disclosure —
        // lives on the conversation record in the agent's own project, so the toggle lands on the
        // record the screen is actually showing.
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.chosen.insert(config_id.clone(), value.clone());
            // Picking a value always means its dropdown should close, so this is the one place
            // that does it rather than leaving it to every caller.
            conversation.open_config = None;
        }
        self.bus.send(Message::SetAgentConfig {
            agent_id,
            config_id,
            value,
        });
        self.clear_picker_search(window, cx);
        cx.notify();
    }

    /// Open or shut one pre-launch config picker for one conversation. `open_config` rather than
    /// the window's single `open_menu`: several pending conversations can each have a picker open
    /// at once, and this is that per-conversation flag's own one-at-a-time rule.
    pub fn toggle_agent_config_menu(
        &mut self,
        agent_id: AgentId,
        config_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.open_config =
                if conversation.open_config.as_deref() == Some(config_id.as_str()) {
                    None
                } else {
                    Some(config_id)
                };
        }
        // A fresh search on every open: the field belongs to whichever picker is down, not to
        // the one that was down last.
        let picker_search = self.picker_search.clone();
        picker_search.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Switch which agent's transcript one conversation is showing — `None` being its own turns,
    /// `Some(id)` a spawned subagent's instance id. Per conversation rather than per window, like
    /// `open_config` beside it: several conversations are on screen at once and each is read
    /// independently.
    ///
    /// The project is the agent's rather than the window's — [`Self::project_of_agent`] — because
    /// the Teams canvas selects cards from every open project under the window span, and pointing
    /// a foreign card's transcript at a delegate is a write into the project that holds it. For
    /// every other caller, and for the project span, that is the project on screen.
    pub fn view_conversation_agent(
        &mut self,
        agent_id: AgentId,
        subagent: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.viewing = subagent;
        }
        cx.notify();
    }

    /// Go to the prompt a "needs you" strip named: switch to whoever raised it, and bring the
    /// call it authorises into view.
    ///
    /// **One reading, two answers.** Which delegate and which block both come from
    /// `Conversation::pending_route`, so the transcript the reader lands in is the transcript the
    /// prompt is drawn in. A request whose call this transcript never saw is drawn as the
    /// self-contained prompt at the end, and that is where this sends them — the tail.
    ///
    /// `slot` is the surface asking, because the scroll belongs to the surface: the same
    /// conversation may be open in a column and a chat tab, and only the one that was clicked
    /// moves.
    pub fn reveal_permission(
        &mut self,
        agent_id: AgentId,
        slot: usize,
        request_id: String,
        cx: &mut Context<Self>,
    ) {
        let route = self
            .project_of_agent(agent_id, cx)
            .and_then(|id| self.projects.get(&id))
            .and_then(|open| open.conversations.get(&agent_id))
            .and_then(|conversation| {
                let pending = conversation
                    .pending
                    .iter()
                    .find(|held| held.request_id == request_id)?;
                Some(conversation.pending_route(pending))
            });
        let Some((who, block)) = route else {
            return;
        };
        if let Some(scroll) = self.transcript_scrolls.get(slot) {
            match block {
                Some(block) => scroll.request(block),
                None => scroll.to_tail(),
            }
        }
        self.view_conversation_agent(agent_id, who, cx);
    }

    /// Put one surface's transcript back on its tail — the jump button over a transcript the
    /// reader has scrolled up in.
    pub fn scroll_transcript_to_tail(&mut self, slot: usize, cx: &mut Context<Self>) {
        if let Some(scroll) = self.transcript_scrolls.get(slot) {
            scroll.to_tail();
        }
        cx.notify();
    }

    /// Open or close one of the conversation's activity panels — the
    /// spawned-subagent list, the todo list. Collapsed is the resting state: a
    /// conversation's delegates or plans are worth a line, not a permanent
    /// list, so this is the one thing that opens one, picking a row from it
    /// closes it again, and asking for a second while one is open is a switch,
    /// not a second panel.
    pub fn toggle_conversation_panel(
        &mut self,
        agent_id: AgentId,
        panel: ActivityPanel,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.panel_open = (conversation.panel_open != Some(panel)).then_some(panel);
        }
        cx.notify();
    }

    /// Close whichever activity panel is open — a row picked in one, an Escape
    /// elsewhere. `None` is the resting state, so this is a no-op there.
    pub fn close_conversation_panel(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.panel_open = None;
        }
        cx.notify();
    }

    /// Dismiss whichever pre-launch config picker is open, without picking — an outside click.
    pub fn dismiss_agent_config_menu(
        &mut self,
        agent_id: AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project_of_agent(agent_id, cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.open_config = None;
        }
        self.clear_picker_search(window, cx);
        cx.notify();
    }

    /// Empty the one buffer every searchable `kit::Picker` shares, so the next one to open does
    /// not inherit what was typed into a different menu.
    fn clear_picker_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker_search = self.picker_search.clone();
        picker_search.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
    }

    /// Open or shut one tool block's detail.
    pub fn toggle_conversation_tool(
        &mut self,
        agent_id: AgentId,
        call_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.project_of_agent(agent_id, cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.toggle_tool(&call_id);
            cx.notify();
        }
    }

    /// Open or shut one run of consecutive thinking blocks, by their positions in the transcript.
    /// A run is one box on screen, so the disclosure on it moves every block it stands for — a
    /// run of one included.
    pub fn toggle_conversation_thought_group(
        &mut self,
        agent_id: AgentId,
        blocks: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.project_of_agent(agent_id, cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.toggle_thought_group(&blocks);
            cx.notify();
        }
    }

    /// Open or shut one collapsed run of same-kind tool calls, named by its first call's id.
    pub fn toggle_conversation_tool_group(
        &mut self,
        agent_id: AgentId,
        key: String,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.project_of_agent(agent_id, cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.toggle_group(&key);
            cx.notify();
        }
    }

    /// Empty one composer and put the keyboard back in it.
    fn clear_composer(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.clear_draft(slot);
        }
        // The slot's mirror is cleared above; the conversation's own copy — what survives a
        // reattach — goes with it, a sent or queued turn leaving nothing behind to restore.
        if let Some(agent_id) = self.agent_for_slot(slot, cx) {
            self.remember_conversation_draft(agent_id, String::new(), cx);
        }
        let Some(input) = self.column_inputs.get(slot).cloned() else {
            return;
        };
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
    }

    // ── starting a live agent ───────────────────────────────────────

    /// Open the `+` menu, anchored where it was clicked, on its first stage.
    ///
    /// **One menu, three surfaces.** The agents screen, the IDE's chat strip and the sink's bench
    /// all ask the same two questions — start something, or look at something already running —
    /// so they raise one menu and it carries who asked. What a pick then *does* differs, and
    /// [`Self::pick_new_agent_menu`] is where that is answered.
    ///
    /// The lists it leads to are asked for again here, for the reason
    /// [`Self::open_new_pane_menu`] asks: a harness installed, an account signed in or a definition
    /// written since the window opened is offered without a restart.
    pub fn open_new_agent_menu(
        &mut self,
        at: (f32, f32),
        surface: NewAgentSurface,
        cx: &mut Context<Self>,
    ) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::NewAgent);
        self.workbench.new_agent_menu = Some(NewAgentMenu {
            at,
            surface,
            stage: NewAgentStage::Menu,
        });
        self.bus.send(Message::ListAgentTypes);
        self.bus.send(Message::ListAccounts);
        self.bus.send(Message::ListAgentDefinitions);
        cx.notify();
    }

    /// Raise the New agent form directly, skipping the `+` menu's first stage.
    ///
    /// The titlebar's shortcut is the menu's row 0 with the stop left out: [`Self::aim_start`]
    /// says where the conversation lands once it does, the same call [`Self::pick_new_agent_menu`]
    /// makes for that row, and the surface is picked the same way — the chat strip in the IDE and
    /// in Tasks (`T-109`: the board's own `+ New agent` reaches this, and a start it raises has to
    /// land in the right dock beside the task it was asked from, the same as `assign_task_to_agent`
    /// already lands one), an agents-screen column everywhere else.
    pub fn open_new_agent_direct(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let surface = if matches!(self.workbench.rail_mode, RailMode::Ide | RailMode::Tasks) {
            NewAgentSurface::Chat
        } else {
            NewAgentSurface::Agents
        };
        self.aim_start(surface, cx);
        self.open_new_agent(window, cx);
    }

    /// What the `+` menu's second stage offers: every conversation in this project that no other
    /// panel of the asking surface already shows.
    ///
    /// The one builder every attach list in the window goes through, so "already taken" means the
    /// same thing here as it does in the chat header's own control.
    ///
    /// The stage draws a filter field — it is a `kit::Picker`, the window's one searchable-list
    /// mechanism — so what was typed narrows the rows here, once, for both the frame that draws
    /// them and the click that resolves against them. The **first** stage's two fixed rows read
    /// this too, to say whether there is anything to attach at all, and must not be narrowed by
    /// whatever some other picker was last used to search: the query counts only while the second
    /// stage is the one on screen.
    pub fn attach_rows(&self, surface: NewAgentSurface, cx: &App) -> AttachChoices {
        let query = match self.workbench.new_agent_menu {
            Some(menu) if menu.stage == NewAgentStage::Attach => {
                self.picker_search.read(cx).value().to_string()
            }
            _ => String::new(),
        };
        let agents = self
            .work(cx)
            .map(|work| work.agents.as_slice())
            .unwrap_or(&[]);
        let live = self
            .agents(cx)
            .map(|view| view.live.as_slice())
            .unwrap_or(&[]);
        let (shown, mine): (Vec<AgentId>, Option<AgentId>) = match surface {
            NewAgentSurface::Agents => (
                self.agents(cx)
                    .map(|view| {
                        view.columns
                            .iter()
                            .flat_map(|column| column.tabs.iter().copied())
                            .collect()
                    })
                    .unwrap_or_default(),
                None,
            ),
            NewAgentSurface::Chat => (
                self.open_project(cx)
                    .map(|open| open.chats.iter().filter_map(|tab| tab.attached).collect())
                    .unwrap_or_default(),
                None,
            ),
            NewAgentSurface::Sink => (Vec::new(), self.sink_agent()),
        };
        attach_choices(agents, live, &shown, mine, &query)
    }

    /// One row of the `+` menu, clicked.
    ///
    /// The first stage's two rows are fixed, so their indices are; the second stage's are
    /// [`Self::attach_rows`], read again here exactly as it was drawn — the rule every
    /// position-matched menu in this window follows.
    pub fn pick_new_agent_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.workbench.new_agent_menu else {
            return;
        };
        match menu.stage {
            NewAgentStage::Menu => {
                match index {
                    // *New agent*: the form answers everything, and where the conversation lands
                    // is written down now so the answer can find its way there when it arrives.
                    0 => {
                        self.workbench.open_menu = None;
                        self.workbench.new_agent_menu = None;
                        self.aim_start(menu.surface, cx);
                        self.open_new_agent(window, cx);
                    }
                    // *Attach existing agent*: the same menu, second stage. Nothing else may be
                    // open at once, so the menu stays where it is rather than reopening somewhere
                    // new.
                    //
                    // The stage is a searchable picker, so the filter it shares with every other
                    // one is cleared and focused on the way in — the rule `open_picker_menu`
                    // says once.
                    1 => {
                        self.workbench.new_agent_menu = Some(NewAgentMenu {
                            stage: NewAgentStage::Attach,
                            ..menu
                        });
                        let search = self.picker_search.clone();
                        search.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                            state.focus(window, cx);
                        });
                        cx.notify();
                    }
                    // Row 2 is the separator ahead of the mission rows (§6.3), on
                    // `NewAgentSurface::Chat` only — `new_agent_menu` draws neither for any other
                    // surface, so no other surface's click can land here.
                    3 if menu.surface == NewAgentSurface::Chat => {
                        self.workbench.open_menu = None;
                        self.workbench.new_agent_menu = None;
                        self.open_new_mission(window, cx);
                    }
                    4 if menu.surface == NewAgentSurface::Chat => {
                        self.workbench.new_agent_menu = Some(NewAgentMenu {
                            stage: NewAgentStage::Missions,
                            ..menu
                        });
                        let search = self.picker_search.clone();
                        search.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                            state.focus(window, cx);
                        });
                        cx.notify();
                    }
                    _ => {}
                }
                return;
            }
            NewAgentStage::Missions => {
                let query = self.picker_search.read(cx).value().trim().to_lowercase();
                let picked = self.open_missions(&query, cx).get(index).map(|r| r.id);
                self.workbench.open_menu = None;
                self.workbench.new_agent_menu = None;
                let Some(task_id) = picked else {
                    cx.notify();
                    return;
                };
                self.open_mission_panel(task_id, cx);
                return;
            }
            NewAgentStage::Attach => {}
        }

        let rows = self.attach_rows(menu.surface, cx);
        let picked = rows
            .items
            .get(index)
            .filter(|_| !rows.disabled.contains(&index))
            .map(|(agent, _)| *agent);
        self.workbench.open_menu = None;
        self.workbench.new_agent_menu = None;
        let Some(agent) = picked else {
            cx.notify();
            return;
        };
        self.show_conversation(menu.surface, agent, cx);
    }

    /// Write down where the conversation a start is about to produce should land, so
    /// `Message::ConversationStarted` can put it there. Nothing to write for the agents screen:
    /// a conversation with no other claim on it opens a column, which is what that screen does
    /// with every arrival anyway.
    ///
    /// **Nothing is created here.** A view raised as the form goes up is a view left behind when
    /// the form is dismissed — an empty column or an empty tab nobody asked for — so the aim is
    /// only written down, and the panel is minted when the conversation lands. See
    /// [`Self::clear_aim`], which is what a dismissal calls.
    pub(super) fn aim_start(&mut self, surface: NewAgentSurface, _cx: &mut Context<Self>) {
        self.clear_aim();
        match surface {
            NewAgentSurface::Agents => {}
            NewAgentSurface::Chat => self.pending_chat_open = true,
            NewAgentSurface::Sink => self.sink.messages.pending_attach = true,
        }
    }

    /// Forget where a start was aimed. A form that never started anything leaves no claim on the
    /// next conversation from anywhere else.
    pub(super) fn clear_aim(&mut self) {
        self.pending_chat_attach = None;
        self.pending_chat_open = false;
        self.sink.messages.pending_attach = false;
        self.new_agent_project = None;
    }

    /// Show a conversation on the surface that asked for it.
    fn show_conversation(
        &mut self,
        surface: NewAgentSurface,
        agent: AgentId,
        cx: &mut Context<Self>,
    ) {
        match surface {
            NewAgentSurface::Agents => self.reveal_agent(agent, cx),
            NewAgentSurface::Chat => {
                if let Some(id) = self.open_chat_tab_now(cx) {
                    self.attach_chat(id, Some(agent), cx);
                }
            }
            NewAgentSurface::Sink => self.set_sink_conversation(agent, cx),
        }
    }

    /// Write down what a conversation was just started on, so the next form opens offering
    /// it. Interface scope: which harnesses this machine has is a fact about the machine, not
    /// about the project that happened to use one.
    pub(super) fn remember_harness_choice(
        &mut self,
        agent_type: &str,
        account: Option<&str>,
        definition: Option<&str>,
        mode: Option<&str>,
        max_subagents: Option<u8>,
        _cx: &mut Context<Self>,
    ) {
        let last = crate::state::prefs::LastStart {
            agent_type: agent_type.to_string(),
            account: account.map(str::to_string),
            definition: definition.map(str::to_string),
            mode: mode.map(str::to_string),
            max_subagents,
        };
        if self.workbench.last_start.as_ref() == Some(&last) {
            return;
        }
        self.workbench.last_start = Some(last);
        self.remember_interface();
    }

    pub fn dismiss_new_agent_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.new_agent_menu = None;
        cx.notify();
    }

    /// Give every column's composer its placeholder and its draft.
    ///
    /// Drained in `render` rather than done where the columns change, because `set_placeholder` and
    /// `set_value` both need a window and three of the callers have none: a message that arrives, a
    /// project switch, and a jump from another screen. The flag is what stops it writing over what
    /// the user is typing on every frame.
    pub(super) fn fill_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.refill_columns {
            return;
        }
        self.refill_columns = false;

        // A slot no column holds is left alone: it is off screen, and the next column to be given
        // it is what fills it.
        let filled: Vec<(usize, String, String)> = self
            .agents(cx)
            .map(|agents| {
                agents
                    .columns
                    .iter()
                    .map(|column| {
                        let name = column
                            .active_agent()
                            .and_then(|id| self.work(cx).and_then(|work| work.agent(id)))
                            .map(|agent| agent.name.clone())
                            .unwrap_or_else(|| "this agent".to_string());
                        (
                            column.slot,
                            format!("Ask {name}\u{2026}"),
                            agents.draft(column.slot).to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        for (slot, placeholder, draft) in filled {
            let Some(input) = self.column_inputs.get(slot).cloned() else {
                continue;
            };
            input.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx);
                if state.value() != draft.as_str() {
                    state.set_value(&draft, window, cx);
                }
            });
        }
    }

    // ── The orchestration screen ────────────────────────────────────
    //
    // Every handler here is guarded on the window holding a project: the screen is a view of one
    // project's work, and a window with none open has nothing for it to act on.
}
