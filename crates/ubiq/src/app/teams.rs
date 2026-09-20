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

    /// Stop drawing the delegates that have finished, or draw them again.
    ///
    /// **It does not relayout**, exactly as the bucket pills do not: a filter narrows what is on
    /// the canvas, and throwing every hand-placed card away is `Tidy`'s job and nobody else's. The
    /// rings the arrangement measures against are refreshed on the next frame by `settle_teams`,
    /// so the next relayout — a `Tidy`, an arrangement picked, or the next thing the host says —
    /// packs the cards against the shorter rings.
    pub fn toggle_teams_hide_done(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.toggle_hide_done();
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
        // Which containers the card in the air may be filed into, decided here because
        // `state::teams` does not know what a project is and is the better for it. Read before the
        // view is borrowed to write, which is the only order the borrow checker allows.
        let eligible = self.teams_carry_tasks(cx);
        if let Some((graph, work)) = self.teams_over_work(cx) {
            graph.carry_to(
                &work,
                at,
                trail,
                std::time::Instant::now(),
                eligible.as_ref(),
            );
        }
        cx.notify();
    }

    /// The tasks the card currently in the air may be dropped into: its own project's, and no
    /// other's. `None` is every task on the canvas, which is the project span's answer — one
    /// project's canvas draws one project's tasks and the question does not arise.
    ///
    /// **A hand-over is within a project.** Under the window span the merged projection's tasks
    /// are every held project's, so without this every project's container is a live target for
    /// every card: the drop would send an `AssignAgent` naming project A, A's agent and B's task,
    /// which A's host has never heard of and would refuse — after the canvas had already drawn the
    /// move. A card whose project cannot be named lands in no container at all, which is the safe
    /// half of the same rule.
    fn teams_carry_tasks(&self, cx: &App) -> Option<HashSet<TaskId>> {
        if self.teams_span() == TeamsSpan::Project {
            return None;
        }
        let TeamsHeld::Agent(agent) = &self.teams(cx)?.carry.as_ref()?.held else {
            return None;
        };
        let tasks = self
            .project_of_agent(*agent, cx)
            .and_then(|project| self.held_project(project))
            .map(|open| open.work.tasks.iter().map(|task| task.id).collect())
            .unwrap_or_default();
        Some(tasks)
    }

    /// Put it down, and ask for the card to be moved into whatever container it landed in.
    ///
    /// **Position is the interface's own fact, membership is the host's.** The drop writes the
    /// card's new offset and nothing else; which task it serves is written down, so the answer is
    /// an `AssignAgent` and the card only changes hands when the host says it has.
    pub fn end_teams_carry(&mut self, cx: &mut Context<Self>) {
        let landed = self
            .teams_over_work(cx)
            .and_then(|(graph, work)| graph.end_carry(&work));
        if let Some((agent_id, task_id)) = landed {
            // The project is the *dropped card's*, not the window's. Under the window span the
            // canvas draws every open project's agents, and both ids in this message belong to
            // whichever project the card came from — filing it against the active project would
            // name a task that project's host has never heard of, and move somebody else's agent
            // onto work it cannot serve. Under the project span it is the same answer as
            // `self.project(cx)`.
            let Some(project_id) = self.project_of_agent(agent_id, cx) else {
                return;
            };
            self.bus.send(Message::AssignAgent {
                project_id,
                agent_id,
                task_id: Some(task_id),
            });
        }
        cx.notify();
    }

    /// Lay the window span's own view out over the merged projection, after a wire arm has laid
    /// the arriving project's view out over its own.
    ///
    /// **The window span has a second view over a second set of cards**, and nothing in those arms
    /// reaches it: each holds one project's [`OpenProject`] and writes the [`TeamsView`] inside
    /// it. Left out, `teams_window` keeps the empty `Layout` it was built with, every card answers
    /// the same default offset, and switching the span draws one pile in the corner with each new
    /// arrival landing under the last.
    ///
    /// Done whatever span is up, because the view is the window's and not the screen's: a span
    /// switched to after the work arrived must find the cards already placed. `relayout` where the
    /// per-project call is a relayout, `absorb_new` where it is one, so the two spans keep the
    /// same rule about which hand-placed cards survive. Nothing to do for a window holding no
    /// project — [`Self::teams_merged`] answers `None` and there is no canvas.
    pub(super) fn settle_window_layout(&mut self, relayout: bool, cx: &App) {
        let projects = self.window_projects(cx);
        let Some((work, _)) = self.teams_merged(&projects) else {
            return;
        };
        if relayout {
            self.teams_window.relayout(&work);
        } else {
            self.teams_window.absorb_new(&work);
        }
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
        //
        // Every delegate the transcripts named, before the filter: which of them a card *draws*
        // is `TeamsView::drawn_delegates`'s answer and only its, so the room the arrangement
        // leaves under a card and the boxes the canvas puts there cannot disagree.
        //
        // Read from every project the span is about, not from the one on screen: under the window
        // span a card whose rings came from `open_project` would be a card with no delegates on
        // it, for no reason the reader can see.
        let projects = self.teams_projects(cx);
        let named: std::collections::HashMap<
            AgentId,
            Vec<crate::state::conversation::SubagentTab>,
        > = projects
            .iter()
            .filter_map(|id| self.projects.get(id))
            .flat_map(|open| {
                open.conversations
                    .iter()
                    .map(|(id, conversation)| (*id, conversation.subagents()))
            })
            .collect();
        // Whose agent each card is, from the same pass that builds the projection the canvas
        // measures — the one thing merging the projections throws away, and the one thing every
        // write on this screen needs back. Write-if-changed, like the rings below it.
        // Only under the window span: the project span answers "whose agent" with the project on
        // screen and never reads the map, and building it would clone a whole projection every
        // frame for a question nobody asks.
        let owner = match self.teams_span() {
            TeamsSpan::Project => Default::default(),
            TeamsSpan::Window => self
                .teams_merged(&projects)
                .map(|(_, owner)| owner)
                .unwrap_or_default(),
        };
        if self.teams_owner != owner {
            self.teams_owner = owner;
        }
        if let Some(graph) = self.teams_mut(cx) {
            let counts: std::collections::HashMap<AgentId, Vec<String>> = named
                .into_iter()
                .map(|(id, tabs)| {
                    let subs: Vec<String> = graph
                        .drawn_delegates(tabs)
                        .into_iter()
                        .map(|tab| tab.id)
                        .collect();
                    (id, subs)
                })
                .filter(|(_, subs)| !subs.is_empty())
                .collect();
            if graph.rings != counts {
                graph.rings = counts;
            }
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
