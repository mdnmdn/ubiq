use super::*;

impl AppState {
    /// Point the screen at a session, at one agent, or at one of that agent's delegates. All three
    /// are selections, and everything else on the screen — the graph's session, the inspector, the
    /// tasks drawer — is a function of this one field.
    ///
    /// **Selecting a delegate points its parent's transcript at it.** Which subagent is being read
    /// is the conversation's own field, shared by every surface showing it ([`Self::
    /// view_conversation_agent`]), so the selection sets it rather than keeping a second answer —
    /// a canvas saying "this delegate" over a thread showing another would be two readings of one
    /// question.
    pub fn select_in_teams(&mut self, selection: TeamsSelection, cx: &mut Context<Self>) {
        let thread = match &selection {
            TeamsSelection::Session(_) => None,
            TeamsSelection::Agent(id) => Some((*id, None)),
            TeamsSelection::Subagent { agent, subagent } => Some((*agent, Some(subagent.clone()))),
        };
        if let Some(graph) = self.teams_mut(cx) {
            graph.selection = Some(selection);
        }
        if let Some((agent, subagent)) = thread {
            self.view_conversation_agent(agent, subagent, cx);
        }
        cx.notify();
    }

    /// Draw one session's agents, or every session's. It does not move the selection: what the
    /// inspector and the drawer report on is a separate question from what the canvas draws.
    pub fn show_teams_session(&mut self, session: Option<SessionId>, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.show_session(session);
        }
        cx.notify();
    }

    /// Put every filter on the orchestration screen back. The one control for "show me all of it".
    pub fn clear_teams_filters(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.clear_filters();
        }
        cx.notify();
    }

    pub fn toggle_teams_bucket(&mut self, bucket: Bucket, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.toggle_bucket(bucket);
        }
        cx.notify();
    }

    pub fn zoom_teams(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.zoom_by(delta);
        }
        cx.notify();
    }

    pub fn reset_teams_zoom(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.zoom = 1.0;
        }
        cx.notify();
    }

    /// Throw the arrangement away and lay the graph out again from what the agents and tasks say.
    ///
    /// Every hand-placed card is lost, which is the point: it is the way back from a canvas the
    /// user has pulled apart, and there is nothing else on the screen that undoes a drag. The full
    /// `relayout` rather than `place_new`, which is the one that leaves placed cards alone.
    pub fn tidy_teams(&mut self, cx: &mut Context<Self>) {
        if let Some((graph, work)) = self.teams_over_work(cx) {
            graph.relayout(&work);
        }
        cx.notify();
    }

    /// Pick an arrangement off the toolbar's dropdown, by its row in `Algo::ALL`.
    pub fn set_teams_layout(&mut self, index: usize, cx: &mut Context<Self>) {
        self.close_menu(cx);
        let Some(algo) = Algo::ALL.get(index).copied() else {
            return;
        };
        if let Some((graph, work)) = self.teams_over_work(cx) {
            graph.set_algo(algo, &work);
        }
        cx.notify();
    }

    pub fn toggle_teams_inspector(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.show_inspector = !graph.show_inspector;
        }
        cx.notify();
    }

    pub fn toggle_teams_tasks_drawer(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.tasks_open = !graph.tasks_open;
        }
        cx.notify();
    }

    /// Select one agent and put the inspector on its thread — what the `chat` affordance on a card
    /// does, and the one place the screen changes two things at once, because a card asking for a
    /// conversation with the panel shut has asked for nothing.
    pub fn open_teams_chat(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.tab = TeamsInspectorTab::Chat;
            graph.show_inspector = true;
        }
        self.select_in_teams(TeamsSelection::Agent(agent), cx);
    }

    /// The same, for one box in a card's ring: select that delegate, and open the inspector on its
    /// parent's thread with the delegate's own turns showing.
    ///
    /// **A delegate has no conversation of its own.** It is a stamp on the lines of the one its
    /// parent is holding, so what opens here is that conversation — the shared view, on the
    /// subagent it was pointed at.
    pub fn open_teams_subagent(
        &mut self,
        agent: AgentId,
        subagent: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.tab = TeamsInspectorTab::Chat;
            graph.show_inspector = true;
        }
        self.select_in_teams(TeamsSelection::Subagent { agent, subagent }, cx);
    }

    pub fn select_teams_inspector_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.tab = if index == 0 {
                TeamsInspectorTab::Chat
            } else {
                TeamsInspectorTab::Tasks
            };
        }
        cx.notify();
    }

    /// Pick a card or a container up.
    ///
    /// A card selects itself on the way up, because what is being moved is what the user is
    /// looking at, and a drag that left the inspector on something else would be reporting on the
    /// wrong agent. A container does not: dragging a box to make room is not a claim about what
    /// the user wants to read.
    pub fn start_teams_carry(&mut self, held: TeamsHeld, grab: (f32, f32), cx: &mut Context<Self>) {
        // What picking a thing up says about what the reader is looking at, decided before the
        // carry takes the value: a card selects itself, a delegate selects itself inside its
        // parent, and a container claims nothing.
        // A delegate picked up is a delegate the reader is now on, which is the same thing a click
        // on it says — and it says it through the one call that points the shared transcript at
        // it. Read before the carry takes the value.
        let delegate = match &held {
            TeamsHeld::Subagent { agent, subagent } => Some(TeamsSelection::Subagent {
                agent: *agent,
                subagent: subagent.clone(),
            }),
            _ => None,
        };
        if let Some(graph) = self.teams_mut(cx) {
            let card = match &held {
                TeamsHeld::Agent(agent) => Some(*agent),
                _ => None,
            };
            graph.start_carry(held, grab);
            // A card already in focus through one of its delegates stays that way: picking a card
            // up is not a claim that the reader is done with the delegate's turns.
            if let Some(agent) = card
                && graph.agent_in_focus() != Some(agent)
            {
                graph.selection = Some(TeamsSelection::Agent(agent));
            }
        }
        if let Some(selection) = delegate {
            self.select_in_teams(selection, cx);
        }
        cx.notify();
    }

    /// Move whatever is being carried, and lay a grain of sand where the pointer passed.
    ///
    /// The trail is skipped when the system asks for reduced motion — it is the only motion on
    /// this screen, and what is held still follows the pointer without it.
    pub fn move_teams_carry(
        &mut self,
        at: (f32, f32),
        pointer: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let trail = (!cx.reduce_motion()).then_some(pointer);
        if let Some((graph, work)) = self.teams_over_work(cx) {
            graph.carry_to(&work, at, trail, std::time::Instant::now());
        }
        cx.notify();
    }

    /// Put it down, and ask for the card to be moved into whatever container it landed in.
    ///
    /// **Position is the interface's own fact, membership is the host's.** The drop writes the
    /// card's new offset and nothing else; which task it serves is written down, so the answer is
    /// an `AssignAgent` and the card only changes hands when the host says it has.
    pub fn end_teams_carry(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let landed = self
            .teams_over_work(cx)
            .and_then(|(graph, work)| graph.end_carry(&work));
        if let Some((agent_id, task_id)) = landed {
            self.bus.send(Message::AssignAgent {
                project_id,
                agent_id,
                task_id: Some(task_id),
            });
        }
        cx.notify();
    }

    /// Age the drag trail by one frame, and answer whether it still owes the window another.
    ///
    /// A drag that ended outside the graph — on the inspector, or off the window — never reaches
    /// the canvas's drop handler, so a carry with no live drag behind it is put down here. That is
    /// what stops a card sticking to the pointer after the button came up somewhere else.
    pub(super) fn settle_teams(&mut self, cx: &mut Context<Self>) {
        // Which delegates each card wears a ring for. Nothing on the wire says a subagent
        // exists — it is a stamp on the lines of a transcript — so the ids are read off the
        // conversations here, once, and the geometry readers in `state::teams` work from the list
        // rather than from the transcript. Written only when it changed, so a settled canvas does
        // not touch the state every frame.
        let counts: std::collections::HashMap<AgentId, Vec<String>> = self
            .open_project(cx)
            .map(|open| {
                open.conversations
                    .iter()
                    .map(|(id, conversation)| {
                        let subs: Vec<String> = conversation
                            .subagents()
                            .into_iter()
                            .map(|tab| tab.id)
                            .collect();
                        (*id, subs)
                    })
                    .filter(|(_, subs)| !subs.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(graph) = self.teams_mut(cx)
            && graph.rings != counts
        {
            graph.rings = counts;
        }

        let stranded = self
            .teams(cx)
            .is_some_and(|graph| graph.carry.is_some() && !cx.has_active_drag());
        if stranded {
            self.end_teams_carry(cx);
        }
        if let Some(graph) = self.teams_mut(cx) {
            graph.settle_sand(std::time::Instant::now());
        }
    }

    // ── The tasks board ─────────────────────────────────────────────

    // ── the task panel's own edits ──────────────────────────────────
    // Every one of these asks and waits. The panel goes on reporting the task the host last
    // confirmed, so a refusal leaves nothing to unwind — which is the same reason a pane is drawn
    // when the coordinator answers rather than when the interface asked.
}
