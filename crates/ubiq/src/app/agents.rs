use super::*;

impl AppState {
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
    /// The chat panel's own selection for [`CHAT_SLOT`]; otherwise the active tab of whichever
    /// column owns the slot. The one place this is answered, so the Enter-key path and the
    /// click-based Send/Enqueue button resolve the same agent for the same slot on every surface
    /// that hosts a composer.
    pub fn agent_for_slot(&self, slot: usize, cx: &App) -> Option<AgentId> {
        if slot == CHAT_SLOT {
            return self.chat.selected;
        }
        self.agents(cx)?
            .columns
            .iter()
            .find(|column| column.slot == slot)
            .and_then(|column| column.active_agent())
    }

    /// What one composer sends, to the agent its slot is addressed at.
    ///
    /// Nothing is appended here, for the reason [`Self::send_to_agent`] appends nothing: the line
    /// lands in the thread when the host answers with the agent carrying it.
    pub fn steer_column(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(agent_id) = self.agent_for_slot(slot, cx) else {
            return;
        };
        // A column showing a live agent sends or queues, whichever the turn in flight calls for;
        // one showing a mock keeps the path it had. Both are on screen at once, and the
        // difference is whether a conversation exists.
        if self.conversation(agent_id, cx).is_some() {
            self.send_or_enqueue(agent_id, slot, window, cx);
            return;
        }
        let Some(agents) = self.agents(cx) else {
            return;
        };
        let text = agents.draft(slot).trim().to_string();
        if text.is_empty() {
            return;
        }
        self.bus.send(Message::SendToAgent {
            project_id,
            agent_id,
            text,
        });
        self.clear_composer(slot, window, cx);
        cx.notify();
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
        let text = input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        if !self
            .conversation(agent_id, cx)
            .is_some_and(|conversation| conversation.accepts_input)
        {
            return;
        }
        self.bus.send(Message::PromptAgent { agent_id, text });
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
            .conversation(agent_id, cx)
            .is_some_and(|conversation| conversation.run == Run::Working);
        if !working {
            self.prompt_agent(agent_id, slot, window, cx);
            return;
        }
        let Some(input) = self.column_inputs.get(slot) else {
            return;
        };
        let text = input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(id) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.enqueue(text);
        }
        self.clear_composer(slot, window, cx);
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
        let Some(id) = self.project(cx) else {
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
        if let Some(id) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.remove_queued(queued_id);
        }
        cx.notify();
    }

    /// Interrupt the turn in flight.
    pub fn cancel_turn(&mut self, agent_id: AgentId, cx: &mut Context<Self>) {
        self.bus.send(Message::CancelTurn { agent_id });
        cx.notify();
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

    /// Pick a row of the lifecycle menu, in the order it draws them: 0 Stop, 1 Unload, 2 Resume,
    /// 3 Delete. Delete does not act here — it raises a confirm instead, being the one
    /// destructive, irreversible verb of the four.
    pub fn pick_conversation_menu(
        &mut self,
        agent_id: AgentId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_conversation_menu(cx);
        match index {
            0 => self.cancel_turn(agent_id, cx),
            1 => self.unload_agent(agent_id, cx),
            2 => self.resume_agent(agent_id, cx),
            3 => {
                self.workbench.confirm_end_conversation = Some(agent_id);
                cx.notify();
            }
            _ => {}
        }
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
        self.bus.send(Message::AnswerPermission {
            agent_id,
            request_id,
            option_id,
        });
        // The dialog goes as the answer does: leaving it up would offer a second answer to a
        // question already settled.
        if let Some(id) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.pending = None;
        }
        cx.notify();
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
        if let Some(id) = self.project(cx)
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
        if let Some(id) = self.project(cx)
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

    /// Dismiss whichever pre-launch config picker is open, without picking — an outside click.
    pub fn dismiss_agent_config_menu(
        &mut self,
        agent_id: AgentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project(cx)
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
        let Some(id) = self.project(cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent_id)
        {
            conversation.toggle_tool(&call_id);
            cx.notify();
        }
    }

    /// Empty one composer and put the keyboard back in it.
    fn clear_composer(&mut self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(agents) = self.agents_mut(cx) {
            agents.clear_draft(slot);
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

    /// Open the agents screen's "New agent" menu, anchored where it was clicked.
    ///
    /// The list is asked for again here for the reason [`Self::open_new_pane_menu`] asks: a
    /// harness installed since the window opened is offered without a restart.
    pub fn open_new_agent_menu(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::NewAgent);
        self.workbench.new_agent_menu = Some(at);
        self.bus.send(Message::ListAgentTypes);
        // Both halves of what the menu offers are asked for on every open, so a harness
        // installed or an account signed in since the window opened is offered without a
        // restart. The harness list already worked this way.
        self.bus.send(Message::ListAccounts);
        cx.notify();
    }

    /// Pick the harness — and the identity — at that row of the menu, and start the conversation
    /// at once: naming is the host's, from the harness's command, so there is nothing left to ask
    /// the user before [`Message::StartConversation`] goes out.
    ///
    /// A harness the host could not find is drawn disabled and takes no click, so picking it does
    /// nothing rather than asking for a start that would fail.
    pub fn pick_new_agent_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.new_agent_menu = None;
        // The same list the menu drew, so an index cannot mean one row on screen and another
        // here — the rule every position-matched menu in the window follows.
        let rows = self
            .workbench
            .harness_choices(&self.workbench.settings.accounts);
        let (harness, account) = match rows.get(index) {
            Some(HarnessChoice::Harness(harness)) => (*harness, None),
            Some(HarnessChoice::Pair { harness, account }) => (*harness, Some(account.clone())),
            // A heading or a hairline is drawn, never picked — a click cannot land on one today
            // since both are disabled, but this is what stops a future reorder turning into a
            // wrong launch.
            Some(HarnessChoice::Label(_)) | Some(HarnessChoice::Separator) | None => return,
        };
        let Some(agent) = self.workbench.agent_types.get(harness) else {
            return;
        };
        if !agent.available {
            return;
        }
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let agent_id = AgentId::generate();
        self.bus.send(Message::StartConversation {
            agent_id,
            project_id,
            session_id: self.session,
            rel_path: None,
            agent_type: agent.id.clone(),
            account,
        });
        cx.notify();
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
