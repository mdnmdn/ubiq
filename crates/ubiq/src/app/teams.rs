use super::*;

impl AppState {
    /// Point the screen at a session, at one agent, or at one of that agent's delegates. All three
    /// are selections, and everything else on the screen — the graph's session and the tasks
    /// drawer — is a function of this one field.
    ///
    /// **Selecting a delegate points its parent's transcript at it.** Which subagent is being read
    /// is the conversation's own field, shared by every surface showing it ([`Self::
    /// view_conversation_agent`]), so the selection sets it rather than keeping a second answer —
    /// a canvas saying "this delegate" over a thread showing another would be two readings of one
    /// question.
    ///
    /// **Selecting a block opens it in the right dock.** There is no inline detail area on this
    /// canvas any more — a card is a map pin, not a page of its own — so picking an agent or a
    /// delegate is also [`Self::open_teams_agent_panel`]'s cue to put that agent's conversation in
    /// front of the reader. A session has no workspace behind it and opens nothing.
    pub fn select_in_teams(&mut self, selection: TeamsSelection, cx: &mut Context<Self>) {
        let thread = match &selection {
            TeamsSelection::Session(_) | TeamsSelection::Mission(_) => None,
            TeamsSelection::Agent(id) => Some((*id, None)),
            TeamsSelection::Subagent { agent, subagent } => Some((*agent, Some(subagent.clone()))),
        };
        // A mission is not a workspace, so it opens no conversation — what its fence handle puts
        // in the right dock is the mission's own panel (M15).
        let mission = match &selection {
            TeamsSelection::Mission(task) => Some(*task),
            _ => None,
        };
        if let Some(graph) = self.teams_mut(cx) {
            graph.selection = Some(selection);
        }
        if let Some((agent, subagent)) = thread {
            self.view_conversation_agent(agent, subagent, cx);
            self.open_teams_agent_panel(agent, cx);
        }
        if let Some(task) = mission {
            self.open_mission_panel(task, cx);
        }
        cx.notify();
    }

    /// Put one agent's conversation in front of the reader, in the right dock.
    ///
    /// **Reuses the first chat tab the project already holds**, the way a fresh project's
    /// persistent agent claims one ([`Self::settle_persistent_chat`]) — a canvas full of agents
    /// opens one panel and re-aims it rather than growing a tab per card clicked. A project with no
    /// tab yet is given one. Either way the panel is revealed, which is what brings the region back
    /// if it was closed and what focuses it if another panel was in front.
    ///
    /// **The tab belongs to the project on screen, whoever owns the agent** (`T-149`). A chat tab
    /// is a *view*, not the workspace — so the one thing that has to be the active project's is
    /// the tab, because the dock's chat leaves are exactly that project's tabs
    /// ([`Self::sync_chat_panels`]): a tab minted into a project the window is not pointed at gets
    /// no panel at all, and the one it already had is closed on the next settle. Under
    /// [`crate::state::TeamsSpan::Window`] the attachment may therefore name an agent the tab's own
    /// project does not hold, and every reader behind it resolves the agent's project for itself —
    /// [`Self::teams_conversation`], [`Self::teams_agent`], [`Self::project_of_agent`]. Under the
    /// project span nothing changes: the two answers are the same project.
    pub fn open_teams_agent_panel(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(id) = reuse_or_mint_chat(open) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(tab) = open.chats.iter_mut().find(|tab| tab.id == id)
        {
            tab.attached = Some(agent);
        }
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::Chat(id)));
        cx.notify();
    }

    /// The right dock's `+`: put a chat panel in front of the reader with nothing attached yet,
    /// so its own header offers exactly the choice starting an agent from a card skips — *New
    /// agent* or *attach existing* — through the chevron [`crate::ui::chat::sidebar::header`]
    /// already draws on every chat tab. Copies the IDE and KB dock headers' own `+`, which raises
    /// the same choice on a tab their panel always has one of; the Teams screen has none until a
    /// card is clicked, which is the gap this closes.
    ///
    /// **Shares [`Self::open_teams_agent_panel`]'s reuse-or-mint step** rather than duplicating it
    /// — this is the same panel, only revealed with no agent to point it at yet, so a tab already
    /// open (attached or not) is reused rather than growing a second one.
    pub fn open_teams_new_agent_panel(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(id) = reuse_or_mint_chat(open) else {
            return;
        };
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::Chat(id)));
        cx.notify();
    }

    /// Tick one session's filter on or off. It does not move the selection: what the right dock
    /// and the drawer report on is a separate question from what the canvas draws.
    pub fn toggle_teams_session(&mut self, session: SessionId, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.toggle_session(session);
        }
        cx.notify();
    }

    /// Tick one mission's filter on or off. Like the session row's, it leaves the selection alone.
    pub fn toggle_teams_mission(&mut self, mission: TaskId, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.toggle_mission(mission);
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
    ///
    /// **The pick is also the new default.** `Algo::ALL` is a canvas control, not a settings-page
    /// one, but the last arrangement the user actually chose is the one the next canvas should
    /// open in — this project's on a restart, and any other project's the moment it opens
    /// ([`Self::sync_projects`] and the window-span catch-up in [`Self::apply_settings`] both seed
    /// from `ui.teams_algo`) — so a pick here writes the same field [`Self::set_teams_default_algo`]
    /// does, through the one path an app-wide UI preference is written down.
    pub fn set_teams_layout(&mut self, index: usize, cx: &mut Context<Self>) {
        self.close_menu(cx);
        let Some(algo) = Algo::ALL.get(index).copied() else {
            return;
        };
        if let Some((graph, work)) = self.teams_over_work(cx) {
            graph.set_algo(algo, &work);
        }
        self.workbench.settings.ui.teams_algo = algo;
        self.remember_settings();
        cx.notify();
    }

    pub fn toggle_teams_tasks_drawer(&mut self, cx: &mut Context<Self>) {
        if let Some(graph) = self.teams_mut(cx) {
            graph.tasks_open = !graph.tasks_open;
        }
        cx.notify();
    }

    /// The same as [`Self::select_in_teams`], for one box in a card's ring: select that delegate,
    /// which opens its parent's conversation in the right dock with the delegate's own turns
    /// showing.
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
        self.select_in_teams(TeamsSelection::Subagent { agent, subagent }, cx);
    }

    /// Pick a card or a container up.
    ///
    /// A card selects itself on the way up, because what is being moved is what the user is
    /// looking at, and a drag that left the right dock's panel pointed at something else would be
    /// reporting on the wrong agent. A container does not: dragging a box to make room is not a
    /// claim about what the user wants to read.
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

    /// Every mission the canvas has a fence for, against everybody on it (M11).
    ///
    /// Membership is both halves of the rule, unioned: the agents holding the mission's anchor
    /// task or one of its children, which the projection answers, **and** the live roster
    /// entries, which only the record carries — `Work::assign_agent` clears a `WorkAgent::parent`
    /// on every reassignment, so the spawn link cannot be recomputed after the fact.
    ///
    /// Read across every project the span is about, so a fence on `All Teams` is drawn for a
    /// mission the project on screen has never heard of. A mission whose anchor task the
    /// projection does not carry has no fence: there is nothing on the canvas for it to enclose
    /// and no container to merge into.
    fn teams_missions(&self, cx: &App) -> HashMap<TaskId, Vec<AgentId>> {
        let mut missions: HashMap<TaskId, Vec<AgentId>> = HashMap::new();
        for open in self
            .teams_projects(cx)
            .iter()
            .filter_map(|id| self.projects.get(id))
        {
            for record in open.missions.values() {
                let anchor = record.task_id;
                if open.work.task(anchor).is_none() {
                    continue;
                }
                let children: Vec<TaskId> =
                    open.work.children_of(anchor).map(|task| task.id).collect();
                let mut members: Vec<AgentId> = open
                    .work
                    .agents
                    .iter()
                    .filter(|agent| {
                        agent
                            .task
                            .is_some_and(|held| held == anchor || children.contains(&held))
                    })
                    .map(|agent| agent.id)
                    .collect();
                for entry in &record.roster {
                    if entry.left_at.is_none() && !members.contains(&entry.agent) {
                        members.push(entry.agent);
                    }
                }
                missions.insert(anchor, members);
            }
        }
        missions
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
    /// A drag that ended outside the graph — on the tasks drawer, the right dock, or off the
    /// window — never reaches the canvas's drop handler, so a carry with no live drag behind it is
    /// put down here. That is what stops a card sticking to the pointer after the button came up
    /// somewhere else.
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
        // Which missions the canvas fences, and who is on each. Read here for the rings' reason:
        // the roster is the project's, the geometry is `state::teams`'s, and that module does not
        // know what a project or a `MissionRecord` is.
        let missions = self.teams_missions(cx);
        if let Some(graph) = self.teams_mut(cx)
            && graph.missions != missions
        {
            graph.missions = missions;
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

/// The chat tab a reveal points at: the project's first, or a freshly minted one where it holds
/// none yet. Shared by [`AppState::open_teams_agent_panel`] and
/// [`AppState::open_teams_new_agent_panel`] — both are "put a chat panel in front of the reader",
/// and the only thing that differs between them is whether an agent is attached to it afterwards.
fn reuse_or_mint_chat(open: &mut OpenProject) -> Option<ChatId> {
    match open.chats.first() {
        Some(tab) => Some(tab.id),
        None => {
            let slot = free_chat_slot(&open.chats)?;
            let id = ChatId::generate();
            open.chats.push(ChatTab {
                id,
                slot,
                attached: None,
                picker_open: false,
            });
            Some(id)
        }
    }
}
