use super::*;

use crate::state::Layer;
use ubiq_proto::work::WorkAgent;

impl AppState {
    /// Reconcile what the window holds with what the registry says it holds.
    ///
    /// Idempotent, and driven by the registry rather than by each call site, because another
    /// window taking a project is a change this window learns about the same way it learns about
    /// its own.
    pub(super) fn sync_projects(&mut self, cx: &mut Context<Self>) {
        let (held, active) = match WindowRegistry::read(cx).slot(self.window_id) {
            Some(slot) => (slot.projects.clone(), slot.active_project()),
            None => (Vec::new(), None),
        };

        let gone: Vec<ProjectId> = self
            .projects
            .keys()
            .copied()
            .filter(|id| !held.contains(id))
            .collect();
        for id in gone {
            self.drop_project(id, cx);
        }

        for id in held {
            if self.projects.contains_key(&id) {
                continue;
            }
            // A project closed here earlier in the session left its furniture behind, which is
            // better than the host's copy: it cannot be older, and it costs no round trip.
            let parked = self.parked.remove(&id);
            let restore = parked.clone();
            self.projects
                .insert(id, OpenProject::new(parked.unwrap_or_default()));
            // The dotfile switch is the window's rather than the project's, so a tree opening now
            // is told what the setting already says.
            let hidden = self.workbench.settings.ui.explorer_hidden;
            // The Teams graph opens in the arrangement the settings name, the same way a fresh
            // explorer opens on the dotfile switch above — set once here rather than read every
            // relayout, because a session the user has since repicked an arrangement for must not
            // snap back to the default under it.
            let teams_algo = self.workbench.settings.ui.teams_algo;
            if let Some(open) = self.projects.get_mut(&id) {
                open.explorer.set_show_hidden(hidden);
                open.teams.algo = teams_algo;
            }
            // The tree is the host's, and a project shows nothing until it answers. One level:
            // what is inside a folder is asked for when the folder is opened.
            self.bus.send(Message::ProjectTree {
                project_id: id,
                rel_path: String::new(),
                depth: EXPAND_DEPTH,
            });
            // Where this project's furniture was left across a restart. The answer arrives as
            // `Preferences`, and is ignored if the parked blob got there first.
            self.bus.send(Message::GetPreferences {
                scope: Scope::Project(id),
            });
            // The work is the host's as well, and the three screens over it draw nothing until it
            // answers. Once per newly held project: the reply is the whole of it, and it is the
            // frame the agents screen lays its columns out on.
            self.bus.send(Message::ListWork { project_id: id });
            // The overview is cheap and lands first; the working-tree walk follows on the same
            // worker, behind it, so the branch name is not stuck waiting for badges.
            self.bus.send(Message::ProjectGit { project_id: id });
            self.bus.send(Message::RefreshProjectGit {
                project_id: id,
                full: true,
            });
            self.bus.send(Message::ProjectGitRefs {
                project_id: id,
                with_tracking: true,
            });
            self.bus.send(Message::ProjectGitLog {
                project_id: id,
                cursor: None,
                count: 100,
                rel_path: None,
                first_parent: false,
                rev: None,
            });
            // `log_inflight` is what tells this reply apart from a stale one still in flight —
            // see `GitView::log_inflight` and `receive_git`.
            if let Some(open) = self.projects.get_mut(&id) {
                open.git_view.log_inflight = Some(None);
            }
            if let Some(view) = restore {
                self.restore_files(id, &view, cx);
            }
        }

        // The tree's first row is the project's name, and the registry is where the window has it
        // in hand. Set on every reconcile rather than in `ExplorerState::empty`, which has no name
        // to give it — and so a rename reaches the row as well.
        for (id, open) in self.projects.iter_mut() {
            if let Some(snapshot) = WindowRegistry::read(cx).project(*id) {
                open.explorer.root_name = snapshot.record.name.clone();
            }
        }

        if self.active_seen != active {
            // The project on screen is leaving it, so its arrangement is written down here rather
            // than trusted to whatever the dock last happened to emit — what comes back when the
            // user returns is what they were looking at.
            if let Some(previous) = self.active_seen.filter(|id| self.projects.contains_key(id)) {
                self.remember(previous, cx);
            }
            self.active_seen = active;
            if let Some(id) = active {
                self.enter_project(id, cx);
            }
        }
        cx.notify();
    }

    /// Everything a project takes with it when it leaves this window: its panes are killed, its
    /// running agents unloaded, its emulators dropped, and what it looked like is written down and
    /// parked.
    ///
    /// The panes have to go. No other window can adopt an emulator, and a pane runs in the
    /// project's folder — a harness left behind would be running somewhere nobody is looking.
    /// The agents have to go for the same reason: a conversation's harness is this project's, and
    /// unloading it is what stops the process without deleting a conversation the user marked to
    /// keep. The window's own close is what parks or deletes what is left.
    fn drop_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        self.remember(project, cx);
        let Some(open) = self.projects.remove(&project) else {
            return;
        };
        self.parked.insert(project, open.prefs);

        for pane in open.panes {
            self.bus.send(Message::CloseWorkspace { pane_id: pane.id });
            self.terminals.remove(&pane.id);
            // The pane's panel goes with it. It is queued rather than taken out here, because a
            // panel leaves the dock through a `Window` and this is reached from a message.
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::Terminal(pane.id)));
        }
        for (agent_id, conversation) in &open.conversations {
            if conversation.running() {
                self.bus.send(Message::UnloadConversation {
                    agent_id: *agent_id,
                });
            }
        }
        if self.active_seen == Some(project) {
            self.active_seen = None;
        }
        // The project's conversations left with it, and a dialog standing on one of their asks
        // would be an empty modal holding `Layer::Ask` over an otherwise usable window.
        self.settle_ask_dialog(cx);
        cx.notify();
    }

    /// Everything a project takes with it when it **moves** to another window: nothing is killed.
    ///
    /// The difference from [`Self::drop_project`] is the whole of the move. A project closed here
    /// is going nowhere, so its harnesses have nobody left to watch them and are ended; a project
    /// taken by another window is still on screen, in that window, and ending its harnesses would
    /// throw away the user's running work for a gesture that only changed which window draws it.
    /// So this lets go rather than closes: no `CloseWorkspace`, no `UnloadConversation`, and the
    /// whole [`OpenProject`] — panes, conversations, tree, editor, furniture — is handed over
    /// intact for the taking window to install.
    ///
    /// What does *not* travel is anything tied to this window: the emulators, the panels drawing
    /// them, and the pane routing in this window's bus. The taking window builds its own, which is
    /// why the panes a panel was drawing are named in the answer — a pane the user had detached
    /// (`D103`) stays detached on the other side.
    pub(super) fn hand_off_project(
        &mut self,
        project: ProjectId,
        cx: &mut Context<Self>,
    ) -> Option<HandedOffProject> {
        // Before the project leaves, so what it is carrying is what the user was last looking at.
        self.remember(project, cx);
        let open = self.projects.remove(&project)?;

        let mut shown = Vec::new();
        for pane in &open.panes {
            let kind = PanelKind::Terminal(pane.id);
            if self.panels.contains_key(&kind) {
                shown.push(pane.id);
            }
            self.terminals.remove(&pane.id);
            // This window is not the pane's route any more; the taking window notes it as its own.
            self.bus.forget_pane(pane.id);
            self.tab_names.remove(&kind);
            self.pinned_tabs.remove(&kind);
            // Queued rather than taken out here: a panel leaves the dock through a `Window`.
            self.pending_panels.push(PanelEdit::Close(kind));
        }
        // The chat tabs go with it as well. `sync_chat_panels` takes them out on the way into the
        // next project, but a window left holding nothing enters none — and a panel drawing a
        // conversation another window is now talking to is worse than one drawing a dead harness.
        for tab in &open.chats {
            let kind = PanelKind::Chat(tab.id);
            self.tab_names.remove(&kind);
            self.pinned_tabs.remove(&kind);
            self.pending_panels.push(PanelEdit::Close(kind));
        }
        if self
            .pending_focus
            .is_some_and(|id| open.panes.iter().any(|pane| pane.id == id))
        {
            self.pending_focus = None;
        }
        if self.active_seen == Some(project) {
            self.active_seen = None;
        }
        // The conversations travel with the project, so an ask dialog over one of them stops
        // naming anything this window holds.
        self.settle_ask_dialog(cx);
        cx.notify();
        Some(HandedOffProject { open, shown })
    }

    /// Install a project another window just handed over, running harnesses and all.
    ///
    /// The state arrives whole, so nothing is re-read from the host — no tree, no work, no Git,
    /// no preferences. What this window has to build is what was left behind: an emulator per
    /// pane on *this* window's bus, a panel over each one that was on screen, and the pane routing
    /// that tells the bus which host those panes belong to.
    ///
    /// The emulator is a fresh one. The old window's `TerminalView` was built on that window's
    /// keystroke writer, its resize sender and its state handle, so moving the entity would leave
    /// every one of those pointing at a window that no longer owns the pane — and the host would
    /// drop them, because [`Message::AdoptProject`] has just made this window the owner. The
    /// harness is untouched and redraws into the new screen; the scrollback it had already written
    /// does not come back, which is the one thing a move costs.
    pub(super) fn adopt_project(
        &mut self,
        project: ProjectId,
        handed: HandedOffProject,
        cx: &mut Context<Self>,
    ) {
        let HandedOffProject { open, shown } = handed;
        // Before the first keystroke or resize can reach the host from the emulators below: until
        // the host has re-homed the panes, everything this window says about them is somebody
        // else's pane and is dropped.
        self.bus.send(Message::AdoptProject {
            project_id: project,
        });

        let host = self.bus.host_of_project(project);
        let term_font = theme::content_base();
        let panes: Vec<(PaneId, u16, u16)> = open
            .panes
            .iter()
            .map(|pane| (pane.id, pane.cols, pane.rows))
            .collect();
        self.projects.insert(project, open);

        for (pane_id, cols, rows) in panes {
            self.bus.note_pane(pane_id, host);
            self.open_terminal(pane_id, cols, rows, term_font, cx);
            if shown.contains(&pane_id) {
                self.pending_panels
                    .push(PanelEdit::Open(PanelKind::Terminal(pane_id)));
            }
        }
        cx.notify();
    }

    /// A project has become the one this window is pointed at: its furniture reaches the window,
    /// the keyboard goes to its focused pane, and the first time round it is given one.
    fn enter_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let view = open.prefs.clone();
        let focused = open.focused_pane;

        // A background project keeps its own furniture, so entering one is where it reaches the
        // window rather than the other way round. The window's own fields are part of that: the
        // entities are the window's, but the text in them is about the project on screen.
        self.form_filled = None;
        self.refill_fields = true;
        // The composers are the window's and the drafts are the project's, so the entering
        // project's text has to be written into them.
        self.refill_columns = true;
        self.workbench.rail_mode = view.rail_mode;
        // Entering a project restores its mode without going through `set_rail_mode`, so a project
        // left in Control would come back to a screen of em dashes until the user touched a tab.
        // Asking here is what makes the restored screen say something.
        self.poll_stats(cx);
        // The arrangement is the mode's own: whichever mode this project was left in is the one
        // whose window comes back. A mode this project never arranged opens on that mode's
        // defaults, regions and all — the same answer a mode switch gives — because otherwise a
        // project that has never been arranged inherits the regions of the one that was on screen,
        // and the next thing the dock says writes them down as its own.
        let saved = view
            .modes
            .get(&view.rail_mode)
            .cloned()
            .unwrap_or_else(|| prefs::ModeLayout::default_for(view.rail_mode));
        self.pending_layout = saved.layout.clone();
        self.pending_regions = saved.layout.is_none().then_some((
            saved.show_left,
            saved.show_bottom,
            saved.show_right,
        ));
        if view.rail_mode == RailMode::Git && saved.layout.is_none() {
            self.queue_git_furniture();
        }
        if view.rail_mode == RailMode::Kb && saved.layout.is_none() {
            self.queue_kb_furniture();
        }
        if saved.layout.is_none() {
            self.queue_mode_furniture(view.rail_mode);
        }
        self.reset_furniture = true;
        // The search results go with the panel the sweep above takes out: they are one project's
        // hits, and the next project is not the one they are about. A mode switch inside one
        // project keeps them, which is why this is here and not in `sweep_furniture`.
        self.search.reset();
        self.sync_file_panels(project);
        self.sync_chat_panels(project);
        // The one exception to a project opening with the right region closed: a persistent
        // agent's tab, shown the moment the work that names it is in hand. If it has not arrived
        // yet, the `WorkList` answer settles this instead.
        self.settle_persistent_chat(project, cx);
        // The field is the window's and the query in it is the project's, so a switch brings back
        // whatever this project was left filtering by rather than carrying the last one's over.
        // `sync_file_filter_field` writes it into the field on the next frame, which is where a
        // window is on hand.
        self.workbench.file_filter = view.file_filter.clone();
        self.explorer_filter_gen = self.explorer_filter_gen.wrapping_add(1);
        self.spawn_explorer_filter(view.file_filter.clone(), cx);

        self.pending_focus = focused;
        if let Some(pane_id) = focused {
            self.bus.send(Message::Focus { pane_id });
        }
        // Entering a project moves the reader's context as much as a mode switch does, so follow
        // mode swaps the help page here too — see `Self::sync_help_follow`.
        self.sync_help_follow(cx);
        cx.notify();
    }

    /// What the window holds for the project it is pointed at, if it is pointed at one.
    pub fn open_project(&self, cx: &App) -> Option<&OpenProject> {
        self.projects.get(&self.project(cx)?)
    }

    pub fn open_project_mut(&mut self, cx: &App) -> Option<&mut OpenProject> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id)
    }

    /// What the window holds for one named project, which is not necessarily the one on screen: a
    /// dialog raised from the projects list edits a project this window may hold without showing.
    /// `None` for a project it does not hold at all, which has no walk of its own to read.
    pub fn held_project(&self, project: ProjectId) -> Option<&OpenProject> {
        self.projects.get(&project)
    }

    /// The tree the explorer draws, which belongs to the project it is showing.
    pub fn explorer(&self, cx: &App) -> Option<&ExplorerState> {
        self.open_project(cx).map(|open| &open.explorer)
    }

    /// The knowledge base the project on screen holds: its sources and the document open over
    /// them.
    pub fn kb(&self, cx: &App) -> Option<&KbState> {
        self.open_project(cx).map(|open| &open.kb)
    }

    pub fn kb_mut(&mut self, cx: &App) -> Option<&mut KbState> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.kb)
    }

    /// The files open in the project on screen.
    pub fn editor(&self, cx: &App) -> Option<&EditorPaneState> {
        self.open_project(cx).map(|open| &open.editor)
    }

    /// The work the two screens over it draw, which belongs to the project on screen.
    pub fn work(&self, cx: &App) -> Option<&WorkProjection> {
        self.open_project(cx).map(|open| &open.work)
    }

    /// The agents screen's view of that work: the columns and the bench.
    pub fn agents(&self, cx: &App) -> Option<&AgentsView> {
        self.open_project(cx).map(|open| &open.agents)
    }

    /// One live agent's conversation, if the project on screen is running it.
    pub fn conversation(&self, id: AgentId, cx: &App) -> Option<&Conversation> {
        self.open_project(cx)
            .and_then(|open| open.conversations.get(&id))
    }

    /// The graph's view of that work.
    pub fn graph(&self, cx: &App) -> Option<&GraphView> {
        self.open_project(cx).map(|open| &open.graph)
    }

    /// Which projects the Teams screen is about: every project the window holds under
    /// [`TeamsSpan::Window`], and the active one alone under [`TeamsSpan::Project`].
    ///
    /// **The one answer**, so the projection, the owner map and every write the screen makes
    /// cannot disagree about what is on the canvas.
    pub fn teams_projects(&self, cx: &App) -> Vec<ProjectId> {
        match self.teams_span() {
            TeamsSpan::Project => self.project(cx).into_iter().collect(),
            TeamsSpan::Window => self.window_projects(cx),
        }
    }

    /// Every project this window both holds and has built state for, whatever span is up.
    ///
    /// The order is the registry's — picker order, which never moves — so a relayout puts the
    /// same project's cards in the same region twice running. A window with no slot in the
    /// registry sorts its own map instead: `HashMap` order permutes on rehash, and an order that
    /// moved between two reads would move every card on the canvas with it. `ProjectId` is a
    /// ULID, so sorting it is the order the projects were created in.
    ///
    /// **Not [`Self::teams_projects`]**, which answers for the span that is up: this is the
    /// window span's own list, which the layout the window span keeps has to be fed from even
    /// while the project span is what is on screen.
    pub fn window_projects(&self, cx: &App) -> Vec<ProjectId> {
        match WindowRegistry::read(cx).slot(self.window_id) {
            Some(slot) => slot
                .projects
                .iter()
                .copied()
                // The registry can name a project this window has not built an `OpenProject`
                // for yet — `sync_projects` runs on a later frame — and a card cannot be drawn
                // for work that is not here.
                .filter(|id| self.projects.contains_key(id))
                .collect(),
            None => {
                let mut ids: Vec<ProjectId> = self.projects.keys().copied().collect();
                ids.sort();
                ids
            }
        }
    }

    /// The merged projection over named projects, and who owns each card. `None` when none of them
    /// is held, which is what makes every Teams accessor answer `None` for a window with no
    /// project — the screen draws nothing rather than an empty graph to explain.
    ///
    /// The owner map is handed back rather than dropped because `settle_teams` writes it into
    /// [`Self::teams_owner`] from the same pass that builds the projection.
    pub(super) fn teams_merged(
        &self,
        projects: &[ProjectId],
    ) -> Option<(WorkProjection, HashMap<AgentId, ProjectId>)> {
        let held: Vec<(ProjectId, &WorkProjection, &[AgentId])> = projects
            .iter()
            .filter_map(|id| {
                self.projects
                    .get(id)
                    .map(|open| (*id, &open.work, open.agents.live.as_slice()))
            })
            .collect();
        if held.is_empty() {
            return None;
        }
        Some(window_work(&held))
    }

    /// Which project one agent belongs to — the question every *write* the Teams screen makes has
    /// to answer, because a merged projection has lost it.
    ///
    /// The span's answer first: the owner map under [`TeamsSpan::Window`], the active project
    /// under [`TeamsSpan::Project`]. Both are guesses about a map that is rebuilt a frame behind
    /// the projection, so the answer is only taken when the project it names actually holds the
    /// agent's conversation; otherwise the held projects are searched for the one that does. An
    /// agent nobody holds a conversation with — a card in the frame between an agent ending and
    /// the canvas hearing about it — falls back to the span's guess, which is what the callers
    /// that send against a project did before there was a second span.
    pub fn project_of_agent(&self, agent: AgentId, cx: &App) -> Option<ProjectId> {
        let guess = match self.teams_span() {
            TeamsSpan::Project => self.project(cx),
            TeamsSpan::Window => self.teams_owner.get(&agent).copied(),
        };
        let holds = |id: &ProjectId| {
            self.projects
                .get(id)
                .is_some_and(|open| open.conversations.contains_key(&agent))
        };
        if let Some(id) = guess.filter(&holds) {
            return Some(id);
        }
        self.projects.keys().copied().find(holds).or(guess)
    }

    /// Which project one session belongs to — [`Self::project_of_agent`]'s sibling, for the one
    /// selection on this screen that names no agent.
    ///
    /// A session is minted inside a project and never leaves it, so the first agent under it
    /// answers for it, exactly as the session pills resolve the chip they wear. Read off the held
    /// projects in the window's own order rather than off the merged projection: building one to
    /// answer a single question would clone every card on the canvas. A session no held project
    /// claims — the frame between the last agent under it ending and the canvas hearing — falls
    /// back to the active project, which is what a link built from it said before there was a
    /// second span.
    pub fn project_of_session(&self, session: SessionId, cx: &App) -> Option<ProjectId> {
        match self.teams_span() {
            TeamsSpan::Project => self.project(cx),
            TeamsSpan::Window => self
                .window_projects(cx)
                .into_iter()
                .find(|id| {
                    self.projects.get(id).is_some_and(|open| {
                        open.work
                            .agents
                            .iter()
                            .any(|agent| agent.session == session)
                    })
                })
                .or_else(|| self.project(cx)),
        }
    }

    /// The Teams screen's conversation lookup: [`Self::conversation`] over the project that owns
    /// the agent rather than over the one on screen. The active project's answer under
    /// [`TeamsSpan::Project`], and the only one that can find a foreign card's thread under
    /// [`TeamsSpan::Window`].
    pub fn teams_conversation(&self, agent: AgentId, cx: &App) -> Option<&Conversation> {
        let project = self.project_of_agent(agent, cx)?;
        self.held_project(project)?.conversations.get(&agent)
    }

    /// The host's record for one agent, read from the project that owns it rather than from the
    /// one on screen — [`Self::work`]'s sibling, on the same rule as [`Self::teams_conversation`].
    ///
    /// What the lifecycle menu's three state rows are read through. Reading the wrong project's
    /// projection answers `None`, which a toggle cannot tell from "off": the row would send
    /// *enable* every time and the flag could never be turned back off.
    pub fn teams_agent(&self, agent: AgentId, cx: &App) -> Option<&WorkAgent> {
        let project = self.project_of_agent(agent, cx)?;
        self.held_project(project)?.work.agent(agent)
    }

    /// The Teams screen's own view of that work, independent of `graph`. **The span decides which
    /// one**: the active project's under [`TeamsSpan::Project`], the window's own under
    /// [`TeamsSpan::Window`], which is a second arrangement over a different set of cards.
    pub fn teams(&self, cx: &App) -> Option<&TeamsView> {
        match self.teams_span() {
            TeamsSpan::Project => self.open_project(cx).map(|open| &open.teams),
            TeamsSpan::Window => {
                (!self.teams_projects(cx).is_empty()).then_some(&self.teams_window)
            }
        }
    }

    /// The work the Teams mode draws: the host's projection narrowed to the agents this window
    /// holds a conversation with — see [`crate::state::teams::live_work`]. **The span decides how
    /// wide that is**: the active project alone, or every project the window holds concatenated by
    /// [`crate::state::teams::window_work`].
    ///
    /// Owned rather than borrowed, because it is a narrowing of what the project holds rather than
    /// a field of it. Every reader on that screen asks for this instead of [`Self::work`];
    /// `TeamsOld` keeps asking for the whole projection.
    pub fn teams_work(&self, cx: &App) -> Option<WorkProjection> {
        match self.teams_span() {
            TeamsSpan::Project => self
                .open_project(cx)
                .map(|open| live_work(&open.work, &open.agents.live)),
            TeamsSpan::Window => self
                .teams_merged(&self.teams_projects(cx))
                .map(|(work, _)| work),
        }
    }

    /// The board's view of the same work.
    pub fn board(&self, cx: &App) -> Option<&BoardState> {
        self.open_project(cx).map(|open| &open.board)
    }

    pub fn work_mut(&mut self, cx: &App) -> Option<&mut WorkProjection> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.work)
    }

    pub fn agents_mut(&mut self, cx: &App) -> Option<&mut AgentsView> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.agents)
    }

    pub fn graph_mut(&mut self, cx: &App) -> Option<&mut GraphView> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.graph)
    }

    /// The same view [`Self::teams`] reads, to write: the span decides which one.
    pub fn teams_mut(&mut self, cx: &App) -> Option<&mut TeamsView> {
        match self.teams_span() {
            TeamsSpan::Project => {
                let id = self.project(cx)?;
                self.projects.get_mut(&id).map(|open| &mut open.teams)
            }
            TeamsSpan::Window => (!self.teams_projects(cx).is_empty())
                .then_some(())
                .map(|()| &mut self.teams_window),
        }
    }

    // ── the Git screen ──────────────────────────────────────────────

    pub fn board_mut(&mut self, cx: &App) -> Option<&mut BoardState> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.board)
    }

    /// The graph and the work behind it, together.
    ///
    /// A drag reads the records while it writes the arrangement, and the two live in the same
    /// [`OpenProject`] — so the pair is handed out once rather than borrowed twice, which nothing
    /// would let a caller do.
    pub(super) fn graph_over_work(
        &mut self,
        cx: &App,
    ) -> Option<(&mut GraphView, &WorkProjection)> {
        let id = self.project(cx)?;
        let open = self.projects.get_mut(&id)?;
        Some((&mut open.graph, &open.work))
    }

    /// The Teams view and the work behind it, together — the same pairing `graph_over_work` gives
    /// `TeamsOld`, kept for the reason that one is: a drag reads the records while it writes the
    /// arrangement, and the two live in the same [`OpenProject`].
    ///
    /// The work is the narrowed one, owned — a drag on this canvas has to measure the same
    /// containers the canvas drew, and those are [`Self::teams_work`]'s. The span decides which
    /// view and how wide the projection is, exactly as it does for the readers above; the owned
    /// projection is built *before* the view is borrowed mutably, which is the only order the
    /// borrow checker allows when both come out of `self`.
    pub(super) fn teams_over_work(&mut self, cx: &App) -> Option<(&mut TeamsView, WorkProjection)> {
        match self.teams_span() {
            TeamsSpan::Project => {
                let id = self.project(cx)?;
                let open = self.projects.get(&id)?;
                let work = live_work(&open.work, &open.agents.live);
                let open = self.projects.get_mut(&id)?;
                Some((&mut open.teams, work))
            }
            TeamsSpan::Window => {
                let (work, _) = self.teams_merged(&self.teams_projects(cx))?;
                Some((&mut self.teams_window, work))
            }
        }
    }

    /// Which project a pane belongs to. A pane is only ever in one, so the first answer is the
    /// answer.
    pub(super) fn project_of_pane(&self, pane_id: PaneId) -> Option<ProjectId> {
        self.projects
            .iter()
            .find(|(_, open)| open.panes.iter().any(|pane| pane.id == pane_id))
            .map(|(id, _)| *id)
    }

    // ── Panes ───────────────────────────────────────────────────────

    /// Activate the Nth rail mode enabled for the current project, `slot` 1-based —
    /// `ctrl-1`..`ctrl-9`. The order is [`RailMode::project_modes`]'s, so `ctrl-1` is IDE, the
    /// first mode of that group, then the rest in rail order. `Control` and `Sink` are not in
    /// that group, so no digit reaches them. A digit past the last enabled mode is a no-op.
    pub fn activate_rail_mode_slot(&mut self, slot: u8, cx: &mut Context<Self>) {
        let mode = RailMode::project_modes()
            .filter(|mode| self.mode_enabled(*mode, cx))
            .nth(usize::from(slot).saturating_sub(1));
        if let Some(mode) = mode {
            self.set_rail_mode(mode, cx);
        }
    }

    /// Move the window to another rail mode, because the user asked for it — the rail, the mode
    /// menu, a `ctrl-`digit.
    pub fn set_rail_mode(&mut self, mode: RailMode, cx: &mut Context<Self>) {
        self.enter_rail_mode(mode, true, cx);
    }

    /// [`Self::set_rail_mode`], told whether the user is the one who asked.
    ///
    /// **Only a switch the user made sweeps the window's furniture** (`D156`). The window moves
    /// itself in two places — a pane started from a mode with no pane region, and the mode the
    /// window is in being hidden — and taking the log console off screen because a runner needed
    /// the IDE is the same class of unasked-for rearrangement the sweep exists to stop.
    pub(super) fn enter_rail_mode(
        &mut self,
        mode: RailMode,
        by_user: bool,
        cx: &mut Context<Self>,
    ) {
        if mode == self.workbench.rail_mode {
            return;
        }
        // The window is leaving one mode and entering another, so the outgoing mode's arrangement
        // is written down first — the rest of this function is about the incoming one.
        self.remember_view(cx);
        self.workbench.rail_mode = mode;
        self.workbench.open_menu = None;
        // Search, the log and — unless it is following the reader — help are the window's
        // furniture and not any one mode's, so they go before the incoming arrangement lands
        // (`D156`). A mode that had one of them open named it in its own blob, and the restore is
        // the only thing that brings it back.
        //
        // **With no project there is no blob and so no restore**: `remember_view` writes nothing
        // for a projectless window, and a console or a help page swept here would be gone for
        // good, with the rail still drawing every mode. Nothing that cannot be put back is taken
        // away, so the sweep waits for a project.
        if by_user && self.project(cx).is_some() {
            self.reset_furniture = true;
        }

        if let Some(project) = self.project(cx)
            && let Some(open) = self.projects.get(&project)
        {
            let saved = open
                .prefs
                .modes
                .get(&mode)
                .cloned()
                .unwrap_or_else(|| prefs::ModeLayout::default_for(mode));
            // Read before the furniture and the ask below touch `self` mutably: `open` borrows
            // `self.projects` and cannot outlive the first `&mut self` call.
            let kb_loaded = open.kb.loaded;
            // A saved arrangement restores whole, regions included. A mode never arranged has no
            // blob, so its defaults are forced directly: regions open or shut on the frame, the
            // tree left as the other mode had it.
            self.pending_layout = saved.layout.clone();
            self.pending_regions = saved.layout.is_none().then_some((
                saved.show_left,
                saved.show_bottom,
                saved.show_right,
            ));
            // Git's refs and changes are not in the IDE tree. A first visit has no blob, so they
            // have to be put in their home regions or the opened edges would be empty.
            if mode == RailMode::Git && saved.layout.is_none() {
                self.queue_git_furniture();
            }
            // The KB explorer is the same kind of furniture, and the configuration behind it is
            // asked for here rather than on every frame: the first visit to the mode is when a
            // blank explorer needs an answer, not every redraw of it.
            if saved.layout.is_none() {
                self.queue_mode_furniture(mode);
            }
            if mode == RailMode::Kb {
                if saved.layout.is_none() {
                    self.queue_kb_furniture();
                }
                if !kb_loaded {
                    self.ask_kb_sources(project);
                }
            }
            // Which mode the window is in is settled now, and is written down now rather than
            // waiting for the arrangement to change: two modes that arrange nothing between them
            // would otherwise leave the window reopening in the one it left. The arrangement
            // itself is not written here — this mode's has not been restored yet.
            if let Some(open) = self.projects.get_mut(&project) {
                open.prefs.rail_mode = mode;
            }
            self.store_prefs(project);
        }
        // Control is the one screen that has to ask for what it draws, and it only asks while it
        // is up. Starting the loop here is what makes entering the mode the thing that starts it.
        if mode == RailMode::Control {
            self.poll_stats(cx);
        }
        self.sync_help_follow(cx);
        cx.notify();
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.set_palette(self.workbench.theme_id.counterpart(), cx);
    }

    /// Wear another palette outright — the Appearance section picks a family, the titlebar's
    /// toggle picks the counterpart, and both land here. The accent and the density are axes of
    /// their own and do not move.
    pub fn set_palette(&mut self, id: theme::ThemeId, cx: &mut Context<Self>) {
        self.workbench.theme_id = id;
        theme::set_mode(id, cx);
        // The palette belongs to the interface, not to any one project.
        self.remember_interface();
        self.redress_terminals(cx);
        cx.notify();
    }

    /// Dress the palette in another accent, or in the palette's own seed with `None`. The accent
    /// is an axis of its own: the ground does not move.
    pub fn set_accent(&mut self, accent: Option<theme::AccentId>, cx: &mut Context<Self>) {
        theme::set_theme(self.workbench.theme_id, accent, cx);
        self.remember_interface();
        self.redress_terminals(cx);
        cx.notify();
    }

    /// Move the **UI scale** — the axis every dimension in the window follows, the window's rem
    /// size included (`D151`, `D153`). The palette does not move.
    ///
    /// The component library has to be re-dressed, not only re-painted: its `font_size` *is* the
    /// rem size, and every `p_3` and `gap_2` in the tree expands to `rems(...)`.
    /// `TERMINAL_PADDING` follows the scale too, so every open emulator has to be re-dressed with
    /// the new inset — and that is also how the harness learns: the emulator re-measures its cell
    /// grid from its bounds minus the padding on the next paint and fires the resize callback that
    /// sends `TerminalResize`, the same path a window resize takes. A pane redrawn at a new inset
    /// without that is the classic corruption bug. That walk is debounced, because a slider drag
    /// must not send the harness two hundred resizes — see [`Self::settle_metrics`].
    pub fn set_ui_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        if (scale - theme::ui_scale()).abs() < f32::EPSILON {
            return;
        }
        theme::set_ui_scale(scale);
        theme::redress(cx);
        self.settle_metrics(cx);
        cx.notify();
    }

    /// Move the **text ratio** — type only, on top of the UI scale. The furniture stays put, so
    /// nothing outside the type scale has to be told; the emulators still do, because a pane's
    /// cell grid is measured from its point size.
    pub fn set_text_ratio(&mut self, ratio: f32, cx: &mut Context<Self>) {
        theme::set_text_ratio(ratio);
        self.settle_metrics(cx);
        cx.notify();
    }

    /// Nudge one family against the other two. The chrome's reflows the window, the
    /// conversation's re-measures the transcript's row heights on the next frame, and the
    /// content's is what `cmd-=` moves — see [`Self::nudge_content_trim`].
    pub fn set_trim(&mut self, family: theme::Family, trim: f32, cx: &mut Context<Self>) {
        theme::set_trim(family, trim);
        self.settle_metrics(cx);
        cx.notify();
    }

    /// Every open emulator holds its own copy of the palette, so a colour switch has to reach it —
    /// the same walk a size change does.
    ///
    /// **Every project's panes, not the active project's.** The content size is one setting for
    /// all of Ubiq now, so a pane in a project this window holds without showing is drawn at the
    /// same size as the one on screen.
    pub(super) fn redress_terminals(&mut self, cx: &mut Context<Self>) {
        let font = theme::content_base();
        let views: Vec<_> = self.terminals.values().map(|t| t.view.clone()).collect();
        for view in views {
            view.update(cx, |view, cx| {
                let (cols, rows) = view.dimensions();
                view.update_config(ui::terminal::config(cols as u16, rows as u16, font), cx);
            });
        }
    }

    /// Let a size change settle, then pay for it once.
    ///
    /// Re-dressing the emulators rebuilds every config, re-measures every cell grid and emits a
    /// `TerminalResize` per pane; writing the blob is a message to the host. Neither belongs on a
    /// slider's every frame, so both wait out [`REFLOW_DEBOUNCE`] behind a generation token — the
    /// device [`Self::schedule_markdown_reflow`] already uses, and the Markdown reflow rides along
    /// on the same settle.
    fn settle_metrics(&mut self, cx: &mut Context<Self>) {
        self.metrics_gen = self.metrics_gen.wrapping_add(1);
        let token = self.metrics_gen;
        self.schedule_markdown_reflow(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFLOW_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if token == this.metrics_gen {
                    this.redress_terminals(cx);
                    this.remember_interface();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn open_menu(&mut self, menu: MenuId, cx: &mut Context<Self>) {
        if menu != MenuId::Explorer {
            self.drop_explorer_menu(cx);
        }
        // Exactly one menu is open at a time, so opening one ends whatever the last one was
        // showing. The preview is cleared here as well as in `close_menu` because its own state
        // claims the invariant "`Some` exactly while `MenuId::AttachmentPreview` is open": a
        // panel left behind by a menu opened straight over it makes the next Escape close
        // something invisible. `open_attachment_preview` sets its own after this call.
        self.workbench.attachment_preview = None;
        self.workbench.open_menu = Some(menu);
        cx.notify();
    }

    /// The topmost overlay the window is painting, if any.
    ///
    /// One arm per rung of [`Layer`], whose order *is* `ui::shell`'s paint order — so the answer
    /// is `max`, and adding an overlay is a rung there and a pair here. See `state/overlay.rs`
    /// for why an outside click needs this at all.
    pub fn top_layer(&self) -> Option<Layer> {
        let w = &self.workbench;
        let s = &w.settings;
        // A picker a form keeps open on its own state: painted above every modal, so it is the
        // top rung whatever raised it — the same four `cancel_dialog` peels before anything else.
        let dropdown = w.kb_source.as_ref().is_some_and(|form| form.open.is_some())
            || self
                .new_agent_form()
                .is_some_and(|form| form.open.is_some())
            || w.new_mission.as_ref().is_some_and(|form| form.open)
            || s.ai_form.as_ref().is_some_and(|form| form.open.is_some())
            || s.app_form.as_ref().is_some_and(|form| form.open)
            || self.notifications.muting.is_some();

        [
            (Layer::ProjectSettings, w.project_settings.is_some()),
            (Layer::KbSource, w.kb_source.is_some()),
            (Layer::Settings, s.open),
            (Layer::Login, s.login.is_some()),
            (Layer::NewAgent, w.new_agent.is_some()),
            (
                Layer::NewAgentNaming,
                w.new_agent.as_ref().is_some_and(|form| form.naming),
            ),
            (Layer::NewMission, w.new_mission.is_some()),
            (Layer::ProfileForm, s.profile_form.is_some()),
            (Layer::AccountDialog, s.dialog.is_some()),
            (Layer::Connect, s.connect.is_some()),
            (Layer::AppForm, s.app_form.is_some()),
            (Layer::Connector, s.connector.is_some()),
            (Layer::Cert, s.cert.is_some()),
            (Layer::AiForm, s.ai_form.is_some()),
            (Layer::AiTest, s.ai_test.is_some()),
            (Layer::AiRemove, s.ai_remove.is_some()),
            (Layer::SshForm, s.ssh_form.is_some()),
            (Layer::SshRemove, s.ssh_remove.is_some()),
            (Layer::DroneStop, s.drone_stop.is_some()),
            (Layer::Clone, w.clone_project.is_some()),
            (Layer::Feedback, w.feedback.is_some()),
            (Layer::Ask, w.ask.is_some()),
            (Layer::AllProjects, w.all_projects.is_some()),
            (Layer::FileDialog, w.file_dialog.is_some()),
            (Layer::SizeNaming, w.size_prompt.is_some()),
            (Layer::ThemeEditor, w.theme_editor.is_some()),
            (Layer::ThemeNaming, w.theme_prompt.is_some()),
            (Layer::ClosePane, w.confirm_close_pane.is_some()),
            (Layer::EndConversation, w.confirm_end_conversation.is_some()),
            (Layer::FilePicker, self.file_picker.is_some()),
            (
                Layer::Menu,
                w.open_menu.is_some() || w.new_agent_menu.is_some(),
            ),
            (Layer::RemoteManager, w.remote_manager.open),
            (Layer::RemoteConnect, w.remote_connect.is_some()),
            (Layer::Notifications, self.notifications.open),
            // Only the dialog takes a rung: a document open inside a markdown tab's annotation
            // layout is not an overlay, and Escape over it belongs to whatever is.
            (
                Layer::Plan,
                w.plan.as_ref().is_some_and(|doc| doc.is_modal()),
            ),
            (Layer::Dropdown, dropdown),
            (Layer::HelpTarget, w.help_target.is_some()),
        ]
        .into_iter()
        .filter_map(|(layer, up)| up.then_some(layer))
        .max()
    }

    /// Whether something is painted above `layer` — in which case an outside click is that
    /// layer's, not this one's, and this one stays up.
    ///
    /// The rule a stacked overlay's `on_mouse_down_out` consults: capture-phase dismissal fires
    /// on a panel's own bounds, so without this every layer below the one clicked in dismisses
    /// itself. One gesture peels one layer, which is what Escape does here too.
    pub fn covered(&self, layer: Layer) -> bool {
        self.top_layer().is_some_and(|top| top > layer)
    }

    /// Escape: take away the topmost thing that is up, and nothing else.
    ///
    /// **One handler, in paint order, top first.** Every overlay in the window is raised from
    /// `WorkbenchState` and painted by `ui::shell` in a fixed order, so which one Escape means is
    /// not a question each surface answers for itself — it is this list, read from the end. That
    /// is what makes the key behave the same over a confirm in settings as over the file dialog,
    /// and it is why `ui::kit::overlay` binds nothing: a modal there is a function returning an
    /// element, with no focus of its own for a key to arrive at.
    ///
    /// **Escape peels one layer.** A dropdown open over a modal closes first and the modal stays,
    /// because dropping a half-filled form because a menu was down loses everything typed into it.
    /// The clone modal used to be the only surface that knew this; now every one of them does.
    ///
    /// Handed back when nothing is up: a bare Escape then belongs to the explorer, the terminal
    /// and the search panel, and swallowing it here would take it from all three. The surfaces
    /// that bind Escape at their own depth — the file picker, the navigator, the explorer's
    /// filter — still win, because a deeper binding fires before this one ever runs.
    pub fn cancel_dialog(&mut self, _: &DialogCancel, window: &mut Window, cx: &mut Context<Self>) {
        // In-place help is `Layer::HelpTarget`, the top rung in the window, so it is peeled before
        // anything at all — including a menu. The mode is deliberately able to cover a dialog and
        // point at its controls, and the price of that is that Escape means "stop pointing" while
        // it is up rather than "close what I was doing".
        if self.workbench.help_target.is_some() {
            self.close_help_target(cx);
            return;
        }
        // A menu is drawn over whatever raised it, so it is peeled before anything else.
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
            return;
        }
        // A picker whose list is down over a form is the same case, and `open_menu` does not know
        // about it: a picker inside a modal keeps its open state on the form, because the modal is
        // redrawn from state on every frame. Peeled here, or Escape would take the half-filled
        // form the list is sitting on.
        if let Some(role) = self
            .workbench
            .settings
            .ai_form
            .as_ref()
            .and_then(|form| form.open)
        {
            self.toggle_ai_model_picker(role, cx);
            return;
        }
        if self
            .workbench
            .settings
            .app_form
            .as_ref()
            .is_some_and(|form| form.open)
        {
            self.toggle_app_provider_picker(cx);
            return;
        }
        // The same case again for the start form's own pickers, which keep their open state on
        // the form for the same reason: Escape takes the list down before the form under it.
        if let Some(list) = self.new_agent_form().and_then(|form| form.open) {
            self.toggle_new_agent_list(list, window, cx);
            return;
        }
        // The new-mission dialog's own assistant picker, on the same terms.
        if self
            .workbench
            .new_mission
            .as_ref()
            .is_some_and(|form| form.open)
        {
            self.toggle_new_mission_assistant_list(cx);
            return;
        }
        // And again for the "Add source" form, whose pickers keep their open state on the form for
        // exactly the same reason.
        if let Some(list) = self.workbench.kb_source.as_ref().and_then(|form| form.open) {
            self.toggle_kb_source_list(list, window, cx);
            return;
        }
        // The bell's list is painted last of the window's overlays, so it is peeled first — and
        // the mute picker inside it before the list it is drawn in.
        if self.notifications.muting.is_some() {
            self.cancel_mute(cx);
            return;
        }
        if self.notifications.open {
            self.close_notifications(cx);
            return;
        }
        let settings = &self.workbench.settings;
        if self.workbench.remote_connect.is_some() {
            self.cancel_remote_connect(window, cx);
        } else if self.workbench.remote_manager.renaming.is_some() {
            self.cancel_rename_remote_host(cx);
        } else if self.workbench.remote_manager.open {
            self.close_remote_manager(cx);
        } else if self.workbench.confirm_end_conversation.is_some() {
            // The two destructive closes, in reverse paint order: `ui::shell` draws the pane's
            // question and then the conversation's, so Escape peels the conversation's first.
            // Escape is the answer a destructive confirm should be easiest of all to give, which
            // is why both sit this high.
            self.dismiss_end_conversation_confirm(cx);
        } else if self.workbench.confirm_close_pane.is_some() {
            self.dismiss_close_pane_confirm(cx);
        } else if self.workbench.theme_prompt.is_some() {
            // The theme's two surfaces, in reverse paint order: the name prompt is drawn over the
            // editor — and is raised *by* it — so Escape takes the prompt and leaves the editor.
            self.close_theme_prompt(cx);
        } else if self.workbench.theme_editor.is_some() {
            self.close_theme_editor(cx);
        } else if self.workbench.size_prompt.is_some() {
            // Above the file question in paint order, so Escape takes the name prompt first. The
            // popover that raised it is a menu and was already peeled at the top of this function.
            self.close_size_prompt(cx);
        } else if matches!(self.workbench.file_dialog, Some(FileDialog::PasteImage)) {
            // Escape takes the text file: the keystroke's own meaning.
            self.decline_paste_image(cx);
        } else if self.workbench.file_dialog.is_some() {
            self.close_file_dialog(cx);
        } else if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| doc.is_modal())
        {
            // Below the file question in paint order: the plan modal's own Export raises one
            // over it, so Escape takes that first and leaves the plan open underneath. Only the
            // dialog: a document open inside a markdown tab is not an overlay and Escape there
            // belongs to whatever is.
            self.close_plan(cx);
        } else if self.workbench.ask.is_some() {
            // Escape puts an agent's question away and sends nothing — what was filled in stays on
            // the ask's own record, and the transcript entry reopens it. Dismissing is not
            // answering, so the harness is still parked and the entry is still there to say so.
            self.close_ask(window, cx);
        } else if self.workbench.feedback.is_some() {
            // Before the clone modal, in reverse paint order: `ui::shell` draws feedback after it,
            // and the balloon is reachable from the titlebar with anything already up.
            self.close_feedback(window, cx);
        } else if self.workbench.clone_project.is_some() {
            self.close_clone(cx);
        } else if self.workbench.all_projects.is_some() {
            self.close_all_projects(cx);
        } else if settings.ssh_remove.is_some() {
            // The two SSH layers, in reverse paint order — `ui::shell` draws them last of the
            // settings page's modals, so Escape takes them first.
            self.close_remove_ssh_profile(cx);
        } else if settings.ssh_form.is_some() {
            self.close_ssh_form(window, cx);
        } else if settings.ai_remove.is_some() {
            // The three provider layers, in reverse paint order — `ui::shell` draws the form,
            // then the test, then the removal question, so Escape takes them the other way up.
            self.close_remove_ai_provider(cx);
        } else if settings.ai_test.is_some() {
            self.close_ai_test(cx);
        } else if settings.ai_form.is_some() {
            self.close_ai_form(window, cx);
        } else if settings.cert.is_some() {
            self.cancel_certificate(cx);
        } else if settings.connector.is_some() {
            self.close_connector_dialog(cx);
        } else if settings.app_form.is_some() {
            self.close_app_form(window, cx);
        } else if settings.connect.is_some() {
            self.cancel_connect(window, cx);
        } else if settings.dialog.is_some() {
            self.close_account_dialog(cx);
        } else if self.workbench.new_agent.is_some() {
            // Painted over the settings page and everything it raises, so it is peeled first of
            // the forms.
            self.close_new_agent(cx);
        } else if self.workbench.new_mission.is_some() {
            self.close_new_mission(cx);
        } else if settings.profile_form.is_some() {
            self.close_profile_form(cx);
        } else if settings.login.is_some() {
            // Only reached while the login is *not* running: a running one draws a live terminal
            // that takes the keyboard, and a bare Escape belongs to the harness inside it.
            self.close_harness_login(cx);
        } else if settings.open {
            self.close_settings(cx);
        } else if self.workbench.kb_source.is_some() {
            // Painted over the project settings page that raised it, so it is peeled before that
            // page — dropping the page and leaving the question over nothing is the one order
            // this pair must not have.
            self.close_kb_source_form(cx);
        } else if self.workbench.project_settings.is_some() {
            self.close_project_settings(cx);
        } else if self.conversation_info.is_some() {
            // Still raised from a dock panel rather than the window root — it reads the live
            // conversation — so it stays under everything above it.
            self.dismiss_conversation_info(cx);
        } else if self
            .board(cx)
            .is_some_and(|board| board.popup && board.draft)
        {
            // The new-task draft form, raised as a modal the same way — closing it is exactly what
            // the modal's × already does.
            self.cancel_new_task(window, cx);
        } else if self
            .board(cx)
            .is_some_and(|board| board.popup && board.show_detail && board.selected.is_some())
        {
            // The tasks board's own popup, raised from its dock panel the same way — closing it
            // is exactly what the side panel's × already does.
            self.close_task_detail(cx);
        } else if self.sink.modal.is_some() {
            self.close_sink_modal(cx);
        } else {
            cx.propagate();
        }
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.tab_menu = None;
        self.workbench.new_pane_menu = None;
        self.workbench.overflow_menu = None;
        self.workbench.new_project_menu = None;
        self.workbench.run_tool_menu = None;
        self.workbench.conversation_menu = None;
        self.workbench.attachment_preview = None;
        self.sink.settings.menu = None;
        self.drop_explorer_menu(cx);
        self.drop_kb_menu(cx);
        cx.notify();
    }

    /// The explorer menu's own outside click, carrying the menu it was drawn for. A dismiss for a
    /// menu that has already been replaced — the right-click on a second row fires the first
    /// menu's handler too — does nothing.
    pub fn dismiss_explorer_menu(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.explorer.close_menu(epoch);
            if open.explorer.menu.is_none() && self.workbench.open_menu == Some(MenuId::Explorer) {
                self.workbench.open_menu = None;
            }
        }
        cx.notify();
    }

    // ── The kitchen sink ────────────────────────────────────────────
    //
    // The application's own test bench. Every mutator here ends in `cx.notify()` like every other
    // one, and none of them means anything: the sink is where a control is looked at, so what it
    // holds is a value and never a claim about a project, a pane or a task.

    /// This window's letter — `A`, `B`, `C`… — as the picker prints it beside every project the
    /// window holds.
    pub fn window_label(&self, cx: &App) -> char {
        WindowRegistry::read(cx)
            .slot(self.window_id)
            .map(|slot| slot.label)
            .unwrap_or('?')
    }

    /// Whether a window's letter is worth printing: a letter tells one window from another, so a
    /// lone window's is noise. The chrome asks this rather than counting windows itself.
    pub fn several_windows(&self, cx: &App) -> bool {
        WindowRegistry::read(cx).window_count() > 1
    }

    /// The project this window is pointed at, if it has one. A window holds nothing only while the
    /// catalogue is empty, or in the frame before the host has answered.
    pub fn project(&self, cx: &App) -> Option<ProjectId> {
        WindowRegistry::read(cx)
            .slot(self.window_id)
            .and_then(|slot| slot.active_project())
    }

    /// What this window holds, straight from the registry — unlike `project_groups`, unfiltered
    /// by the picker's search box.
    pub fn window_slot<'a>(&self, cx: &'a App) -> Option<&'a crate::state::WindowSlot> {
        WindowRegistry::read(cx).slot(self.window_id)
    }

    /// Everything known about the project this window is pointed at.
    pub fn project_snapshot<'a>(&self, cx: &'a App) -> Option<&'a ProjectSnapshot> {
        let id = self.project(cx)?;
        WindowRegistry::read(cx).project(id)
    }

    pub fn project_name(&self, cx: &App) -> String {
        self.project_snapshot(cx)
            .map(|p| p.record.name.clone())
            .unwrap_or_else(|| "No project".to_string())
    }

    /// The word this project's board uses for a mission — its own override, or the application
    /// wide default. See [`crate::state::work::mission_term`].
    pub fn mission_term(&self, cx: &App) -> String {
        let project_override = self
            .project_snapshot(cx)
            .and_then(|p| p.record.mission_term.as_deref());
        let default = &self.workbench.settings.host.mission_term;
        crate::state::work::mission_term(project_override, default)
    }

    /// Rebuild the Markdown previews once the zoom stops moving. See [`AppState::md_reflow`].
    fn schedule_markdown_reflow(&mut self, cx: &mut Context<Self>) {
        self.md_reflow_gen = self.md_reflow_gen.wrapping_add(1);
        let token = self.md_reflow_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFLOW_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if token == this.md_reflow_gen {
                    this.md_reflow = this.md_reflow.wrapping_add(1);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// `cmd-=` and `cmd--`: nudge the content family's trim up or down.
    ///
    /// A step of the trim rather than a whole point, because the trim is a ratio over
    /// [`theme::TEXT_BASE`] and a point is not a fixed fraction of it once the UI scale has moved.
    /// It dresses the editor, the viewer, the terminal panes, the explorer tree and search results
    /// together, in every project — the zoom is the person's, not the folder's (`D151`).
    pub fn nudge_content_trim(&mut self, direction: i8, cx: &mut Context<Self>) {
        let next = theme::content_trim() + direction as f32 * CONTENT_TRIM_STEP;
        self.set_trim(theme::Family::Content, next, cx);
    }

    /// Whether the active project's file editors soft-wrap long lines. `None` is the editor's own
    /// default, which is to wrap.
    pub fn editor_wrap(&self, cx: &App) -> Option<bool> {
        let id = self.project(cx)?;
        self.projects
            .get(&id)
            .and_then(|open| open.prefs.editor_wrap)
    }

    /// Flip whether the active project's file editors wrap, and bring every already-open buffer in
    /// line with the new preference rather than waiting for a reopen.
    pub fn toggle_editor_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.project(cx) else {
            return;
        };
        let next = {
            let Some(open) = self.projects.get_mut(&id) else {
                return;
            };
            let next = !open.prefs.editor_wrap.unwrap_or(true);
            open.prefs.editor_wrap = Some(next);
            next
        };
        let buffers: Vec<_> = self
            .projects
            .get(&id)
            .into_iter()
            .flat_map(|open| open.editor.open.iter())
            .filter_map(|file| match &file.body {
                FileBody::Text { state, .. } => Some(state.clone()),
                _ => None,
            })
            .collect();
        for state in buffers {
            state.update(cx, |state, cx| state.set_soft_wrap(next, window, cx));
        }
        self.remember(id, cx);
        cx.notify();
    }

    /// Whether a rail mode is on screen for the active project. Unknown project: everything is,
    /// since there is nothing to have hidden it.
    pub fn mode_enabled(&self, mode: RailMode, cx: &App) -> bool {
        let Some(id) = self.project(cx) else {
            return true;
        };
        self.projects
            .get(&id)
            .is_none_or(|open| !open.prefs.hidden_modes.contains(&mode))
    }

    /// Show or hide one rail mode for the active project. The last visible mode cannot be hidden,
    /// and hiding the mode the window is in moves it to the first one still visible.
    pub fn toggle_mode(&mut self, mode: RailMode, cx: &mut Context<Self>) {
        let Some(id) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&id) else {
            return;
        };
        match open.prefs.hidden_modes.iter().position(|m| *m == mode) {
            Some(at) => {
                open.prefs.hidden_modes.remove(at);
            }
            None => {
                if open.prefs.hidden_modes.len() + 1 >= RailMode::every().count() {
                    return;
                }
                open.prefs.hidden_modes.push(mode);
            }
        }
        if self.workbench.rail_mode == mode
            && let Some(next) = RailMode::every().find(|m| self.mode_enabled(*m, cx))
        {
            // Hiding the mode the window is in moves the window; the user asked for the mode to
            // go, not for the console to go with it, so this sweeps nothing (`D156`).
            self.enter_rail_mode(next, false, cx);
        }
        self.remember(id, cx);
        cx.notify();
    }

    /// The colour the whole window is identified by.
    ///
    /// One place decides what a window with no project looks like, rather than four call sites
    /// each falling back to swatch zero and claiming to be a project that is not there.
    pub fn project_tint(&self, cx: &App) -> gpui::Rgba {
        match self.project_snapshot(cx) {
            Some(project) => theme::project_tint(
                project.record.temporary,
                project.record.colour,
                project.record.custom_colour,
            ),
            None => theme::border(),
        }
    }

    /// The swatch the interface would give a new project: the one fewest projects are using, so
    /// the palette spreads before it repeats.
    pub fn next_colour(&self, cx: &App) -> usize {
        let registry = WindowRegistry::read(cx);
        let count = theme::project_colour_count();
        let mut used = vec![0usize; count];
        for project in registry.all() {
            used[project.record.colour % count] += 1;
        }
        used.iter()
            .enumerate()
            .min_by_key(|(index, taken)| (**taken, *index))
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    /// The picker's three groups: open here, open in another window, only remembered.
    pub fn project_groups(&self, cx: &App) -> crate::state::ProjectGroups {
        WindowRegistry::read(cx).groups(self.window_id, &self.workbench.project_filter)
    }

    /// Bring the console on screen: its region back if it was put away, and its tab to the front.
    pub fn reveal_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panel(PanelKind::Logs, cx);
        dock::reveal(
            &self.dock.clone(),
            &panel,
            PanelKind::Logs.home(),
            window,
            cx,
        );
        cx.notify();
    }

    /// Bring the search panel on screen: its region back if it was put away, and its tab to the
    /// front.
    pub fn reveal_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panel(PanelKind::Search, cx);
        dock::reveal(
            &self.dock.clone(),
            &panel,
            PanelKind::Search.home(),
            window,
            cx,
        );
        // The query field takes the keyboard, because typing a query is the only thing revealing
        // the panel is for. A panel that opens with the caret left where it was reads as nothing
        // having happened, which is how the gesture looked while the binding was being lost.
        let field = self.search.query.read(cx).focus_handle(cx);
        window.focus(&field, cx);
        cx.notify();
    }

    /// Bring the outline panel on screen, and parse the buffer for it if it was put away while
    /// the file changed underneath it.
    pub fn reveal_outline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panel(PanelKind::Outline, cx);
        dock::reveal(
            &self.dock.clone(),
            &panel,
            PanelKind::Outline.home(),
            window,
            cx,
        );
        self.settle_outline(cx);
        cx.notify();
    }

    /// ⌘P: bring the explorer out if it is put away, and put the caret in its filter.
    ///
    /// The same shape as [`Self::reveal_search`], for the same reason: the gesture is "go to a
    /// file", and a panel that opens without the keyboard in the field reads as nothing having
    /// happened.
    pub fn reveal_explorer_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panel(PanelKind::Explorer, cx);
        dock::reveal(
            &self.dock.clone(),
            &panel,
            PanelKind::Explorer.home(),
            window,
            cx,
        );
        self.focus_explorer_filter(window, cx);
    }

    /// Serve the active project over the local web-export server and open it in the browser.
    pub fn open_web_export(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.project_snapshot(cx) else {
            return;
        };
        let project_id = snapshot.record.id.to_string();
        let project_name = snapshot.record.name.clone();
        let root = std::path::PathBuf::from(&snapshot.record.path);
        match crate::web_export::ensure_started_and_registered(&project_id, &project_name, &root) {
            Ok(url) => {
                let _ = open_url(&url);
            }
            Err(err) => {
                tracing::error!("web export failed to start: {err}");
            }
        }
        cx.notify();
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The dock is settled first: a panel that arrived on a message has to be in the tree
        // before the frame that focuses it, and a restored arrangement before either. Visibility
        // leads, because installing a layout asks every panel for it.
        self.settle_visibility(cx);
        self.settle_mode(window, cx);
        self.settle_layout(window, cx);
        self.settle_panels(window, cx);
        self.take_focus(window, cx);
        self.attach_arrived_files(window, cx);
        self.attach_kb_docs(window, cx);
        // Which document the annotation surface is pointed at follows the editor's active tab, so
        // it is settled before the buffer that document is read into.
        self.settle_annotation_document(cx);
        // The annotated document's buffer is seeded and its decorations painted here for the
        // reason `attach_arrived_files` is: both need a `Window`, and the host's answer arrives
        // without one.
        self.settle_plan_editor(window, cx);
        // A web panel's edits go into the buffer `attach_arrived_files` made, so they follow it,
        // and the sessions and browsers they arrive through are settled first.
        self.settle_web_panels(window, cx);
        self.apply_web_documents(window, cx);
        // The keyboard a file panel asked for waits for its buffer, which `attach_arrived_files`
        // may have just delivered in this same frame — so the editor is focused after it, not
        // before.
        self.take_editor_focus(window, cx);
        self.fill_task_form(window, cx);
        self.fill_columns(window, cx);
        self.fill_project_form(window, cx);
        self.fill_ask_fields(window, cx);
        // Kept in step with the sources while the dialog holding them is up; a settings row for a
        // source with no field yet would have nothing to type into. No `cx.notify()` —
        // `settle_nav`'s discipline, run from the same place.
        self.ensure_kb_inputs(window, cx);
        self.settle_graph(cx);
        self.settle_teams(cx);
        self.settle_board(cx);
        // Where the window is drawing, remembered once the screens above have settled on it.
        self.settle_nav(cx);
        self.settle_tab_drag(cx);
        // Last of the dock's settles: a runner's region is opened after the mode's own regions
        // have been forced and after anything emptied has been hidden, so nothing overrules it.
        self.settle_pane_region(window, cx);
        // The filter field is one per window; the project on screen's filter is the window's habit
        // from the frame after the project swings in. Cheap when nothing changed, and it never
        // fights a query being typed.
        self.sync_file_filter_field(window, cx);
        self.sync_git_fields(window, cx);
        self.sync_settings_fields(window, cx);
        // Made anonymous straight away so the frame stops borrowing the window: the queue below
        // is drained on the same `&mut self` the tree was built from.
        let tree = ui::shell::render(self, window, cx).into_any_element();
        // Diagrams a viewer found it needed while the tree was being built. Started here, where
        // the update the frame was built inside is done with `AppState` — never from inside one,
        // and never on this thread.
        self.drain_diagram_asks(cx);
        self.drain_exported_asks(cx);
        tree
    }
}

/// One project on its way from one window to another, with everything running in it still running.
///
/// Produced by [`AppState::hand_off_project`] and consumed by [`AppState::adopt_project`], and by
/// nothing else: it exists only for the instant between the two, which is why it carries the state
/// rather than a copy of it.
pub(super) struct HandedOffProject {
    open: OpenProject,
    /// The panes a panel was drawing when the project left. A pane that is not here is one the
    /// user had detached — still running, nothing showing it — and it arrives detached.
    shown: Vec<PaneId>,
}
