use super::*;

impl AppState {
    /// The panes the dock draws: the active project's, and none at all without one.
    pub fn panes(&self, cx: &App) -> &[PaneState] {
        self.open_project(cx)
            .map(|open| open.panes.as_slice())
            .unwrap_or(&[])
    }

    pub fn focused_pane(&self, cx: &App) -> Option<&PaneState> {
        let open = self.open_project(cx)?;
        let id = open.focused_pane?;
        open.panes.iter().find(|pane| pane.id == id)
    }

    /// One pane of whichever project holds it, named rather than found through focus. Every panel
    /// draws its own pane, so the lookup is across every project the window holds.
    pub fn pane(&self, pane_id: PaneId) -> Option<&PaneState> {
        self.projects
            .values()
            .find_map(|open| open.panes.iter().find(|pane| pane.id == pane_id))
    }

    /// Whether a pane belongs to the project this window is pointed at. A pane of any other keeps
    /// running and keeps its scrollback; its panel is hidden rather than closed, which is what
    /// keeps its place in the arrangement.
    pub fn pane_is_on_screen(&self, pane_id: PaneId, cx: &App) -> bool {
        self.open_project(cx)
            .is_some_and(|open| open.panes.iter().any(|pane| pane.id == pane_id))
    }

    /// The emulator a pane is drawn by, for the one module that draws it.
    pub fn terminal(&self, pane_id: PaneId) -> Option<&Entity<TerminalView>> {
        self.terminals.get(&pane_id).map(|pane| &pane.view)
    }

    /// Ask for a workspace. The pane appears when the coordinator answers, so a harness that fails
    /// to start leaves no empty tab behind.
    ///
    /// A pane runs in a project's folder, so a window holding no project asks for nothing: there is
    /// no directory a harness could be started in that the user chose.
    ///
    /// `picks` is what a terminal harness resolves the identity, the model and the rest against —
    /// the pseudo-terminal has no composer to fold a start's answers into, so they travel with the
    /// spawn instead. A shell ignores them: it has no account, no profile and no modes to be picked.
    pub fn spawn_pane(
        &mut self,
        agent_type: Option<String>,
        args: Vec<String>,
        picks: AgentPicks,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.bus.send(Message::SpawnWorkspace {
            session_id: self.session,
            project_id,
            rel_path: None,
            agent_type,
            args,
            picks,
        });
    }

    /// Run a configured tool in this project's folder. The pane appears when the coordinator
    /// answers, so a tool that fails to start leaves no empty tab behind — the same standing
    /// `spawn_pane` gives a harness.
    pub fn run_tool(&mut self, scope: Scope, id: ToolId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.bus.send(Message::RunTool {
            session_id: self.session,
            project_id,
            scope,
            id,
        });
    }

    /// Whether a panel currently draws this pane. `panels` is the live registry, so this is what
    /// tells a running pane the user can see from one that is only still running.
    pub fn pane_has_panel(&self, pane_id: PaneId) -> bool {
        self.panels.contains_key(&PanelKind::Terminal(pane_id))
    }

    /// Every pane the open project holds that no panel draws — still running, nothing showing them.
    ///
    /// Computed, not stored, the way the agents screen's bench is: a detach only takes the panel
    /// away, so the difference between `panes` and `panels` *is* the list, and no flag can fall out
    /// of step with it.
    pub fn detached_panes(&self, cx: &App) -> Vec<PaneId> {
        self.panes(cx)
            .iter()
            .map(|pane| pane.id)
            .filter(|id| !self.pane_has_panel(*id))
            .collect()
    }

    /// The project's first pane a panel still draws, skipping `except`.
    ///
    /// Where the keyboard goes when the pane holding it stops being drawn. `except` is for the
    /// pane whose own panel is only *queued* to leave: `pending_panels` settles a frame later, so
    /// its entry is still in `panels` and [`Self::pane_has_panel`] cannot answer for it yet.
    fn next_focus_pane(&self, project: ProjectId, except: Option<PaneId>) -> Option<PaneId> {
        self.projects
            .get(&project)?
            .panes
            .iter()
            .map(|pane| pane.id)
            .find(|id| Some(*id) != except && self.pane_has_panel(*id))
    }

    /// Take a pane's panel off the screen and leave everything else alone: the harness keeps
    /// running, the emulator keeps taking its bytes, and the pane stays in its project.
    ///
    /// This is what closing a tab means. Nothing is sent to the host and nothing is forgotten —
    /// `terminals` still holds the live screen, `bus` still knows which host owns the pane so
    /// keystrokes keep their route, and the name and the pin the user gave the tab are still
    /// theirs. [`Self::reattach_pane`] puts a panel back over a session that never stopped;
    /// [`Self::close_pane`] is the one that kills.
    ///
    /// Idempotent, because the dock reaches it too — a panel whose tab was closed asks for this
    /// once it is sure it was closed rather than displaced. A pane no panel draws is not detached
    /// twice.
    pub fn detach_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(project) = self.project_of_pane(pane_id) else {
            return;
        };
        if !self.pane_has_panel(pane_id) {
            return;
        }
        // The keyboard only moves for the project on screen: a tab closed in a background project
        // must not take focus off the terminal the user is typing into.
        let on_screen = self.project(cx) == Some(project);
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::Terminal(pane_id)));

        let mut next = None;
        if self.projects.get(&project).map(|open| open.focused_pane) == Some(Some(pane_id)) {
            next = self.next_focus_pane(project, Some(pane_id));
            if let Some(open) = self.projects.get_mut(&project) {
                open.focused_pane = next;
            }
        }
        if on_screen && let Some(pane_id) = next {
            self.pending_focus = Some(pane_id);
        }
        cx.notify();
    }

    /// Draw a detached pane again: the panel comes back over the emulator that never stopped, so
    /// the screen is the one the harness has been writing to all along.
    ///
    /// Deliberately not `open_terminal`, which inserts a fresh emulator unconditionally and would
    /// throw that screen away. Refused for a pane this window does not hold, and for one a panel
    /// already draws — there is nothing to bring back.
    pub fn reattach_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        if self.pane(pane_id).is_none() || self.pane_has_panel(pane_id) {
            return;
        }
        // `Reveal` rather than `Open`: the region terminals live in may have been put away since,
        // and the tab has to end up the displayed one of its group.
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::Terminal(pane_id)));
        // The same pair `focus_pane` sends for an ordinary pane: the project's focus moves, and the
        // host hears it on the transition.
        if let Some(project) = self.project_of_pane(pane_id)
            && let Some(open) = self.projects.get_mut(&project)
            && open.focused_pane != Some(pane_id)
        {
            open.focused_pane = Some(pane_id);
            self.bus.send(Message::Focus { pane_id });
        }
        self.pending_focus = Some(pane_id);
        cx.notify();
    }

    /// End a pane: the harness is killed, the emulator dropped, and the panel taken out of the
    /// dock.
    ///
    /// Idempotent, because more than one caller reaches it — a harness that exited, a pin menu's
    /// Close, a host that went away. A pane the window has already let go of is not closed twice.
    /// The dock's own tab close does not come here: it detaches.
    pub fn close_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(project) = self.project_of_pane(pane_id) else {
            return;
        };
        self.bus.send(Message::CloseWorkspace { pane_id });
        self.terminals.remove(&pane_id);
        // After the send above, which still needed to know which host owned it.
        self.bus.forget_pane(pane_id);
        self.tab_names.remove(&PanelKind::Terminal(pane_id));
        self.pinned_tabs.remove(&PanelKind::Terminal(pane_id));

        let showing = self.project(cx);
        // The keyboard only moves for the project on screen: a pane closed in a background
        // project must not take focus off the terminal the user is typing into.
        let on_screen = showing == Some(project);
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::Terminal(pane_id)));

        let mut refocus = false;
        if let Some(open) = self.projects.get_mut(&project) {
            open.panes.retain(|pane| pane.id != pane_id);
            refocus = open.focused_pane == Some(pane_id);
        }
        // Not simply the first pane: a detached pane is still in `panes` with nothing on screen,
        // and the keyboard must never go to one of those.
        let mut next = None;
        if refocus {
            next = self.next_focus_pane(project, None);
            if let Some(open) = self.projects.get_mut(&project) {
                open.focused_pane = next;
            }
        }
        // Closing a pane while a panel that is not a terminal holds the keyboard must not hand it
        // to a terminal that is off screen.
        if on_screen && let Some(pane_id) = next {
            self.pending_focus = Some(pane_id);
        }
        cx.notify();
    }

    /// A harness renamed itself over its own stream (`ESC ] 0 ; title BEL`). The dedup number
    /// `pane_title` gave the tab is not the harness's to spend, so it survives the rename.
    fn pane_title_reported(&mut self, pane_id: PaneId, title: String, cx: &mut Context<Self>) {
        for open in self.projects.values_mut() {
            if let Some(pane) = open.panes.iter_mut().find(|pane| pane.id == pane_id) {
                pane.title = match pane_title_number(&pane.title) {
                    Some(n) => format!("{title} {n}"),
                    None => title,
                };
                cx.notify();
                return;
            }
        }
    }

    pub fn resize_pane(&mut self, pane_id: PaneId, cols: u16, rows: u16, cx: &mut Context<Self>) {
        // A background project's panes are still measured — an emulator that is not drawn keeps
        // the geometry it was last given — so the search is across every project the window holds.
        for open in self.projects.values_mut() {
            if let Some(pane) = open.panes.iter_mut().find(|pane| pane.id == pane_id) {
                if pane.cols == cols && pane.rows == rows {
                    return;
                }
                pane.cols = cols;
                pane.rows = rows;
                cx.notify();
                return;
            }
        }
    }

    /// Give a pane the keyboard. Only the project on screen has panes the user can click, so a
    /// pane belonging to any other is not focusable.
    pub fn focus_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let held = self
            .projects
            .get(&project)
            .is_some_and(|open| open.panes.iter().any(|pane| pane.id == pane_id));
        if !held {
            return;
        }
        let already = self
            .projects
            .get(&project)
            .is_some_and(|open| open.focused_pane == Some(pane_id));
        if let Some(open) = self.projects.get_mut(&project) {
            open.focused_pane = Some(pane_id);
        }
        // `Focus` is sent on the transition and no other. The dock calls this every time a
        // terminal panel becomes the displayed tab of its group, which includes the tab it was
        // already showing.
        if !already {
            self.bus.send(Message::Focus { pane_id });
        }
        cx.notify();
    }

    /// The keyboard has gone to a panel that is not a terminal, so no pane holds it.
    ///
    /// This is today's console rule stated for every non-terminal panel rather than for one. The
    /// project keeps which pane was last focused, so coming back to a terminal panel is where the
    /// pane gets the keyboard again.
    pub fn blur_panes(&mut self, cx: &mut Context<Self>) {
        self.pending_focus = None;
        // The keyboard left any editor too, so an editor focus asked for earlier and not yet
        // granted is withdrawn: whatever panel took the keyboard owns it now.
        self.pending_editor_focus = None;
        cx.notify();
    }

    /// One router task: drain a connection's inbound stream and hand each arrival to [`Self::receive`],
    /// tagged with the [`HostRef`] it came from before that method ever sees it.
    ///
    /// Shared by the loop `boot.rs` runs once over `bus.connections()` at construction, and by
    /// `remote_connect.rs`'s success path for a connection minted after boot — the same shape
    /// either way, so it lives once here rather than twice. Ends when the channel disconnects: for
    /// the local host that never happens before the window itself does, but for a remote it means
    /// the pump threads gave up (the socket failed, or the connection was dropped on purpose), so
    /// this also removes it from `Bus` and says so where every window already reads its log —
    /// `tracing::warn!` reaches the console panel through `ubiq_proto::log`.
    pub(super) fn route_host(
        host: HostRef,
        from_host: flume::Receiver<Message>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Ok(message) = from_host.recv_async().await {
                if this
                    .update(cx, |this, cx| this.receive(host, message, cx))
                    .is_err()
                {
                    return;
                }
            }
            if let HostRef::Remote(id) = host {
                // The socket failed rather than the user asking for this. Panes close the same
                // way a manual disconnect closes them, and a connection with a saved entry
                // behind it starts the reconnect loop — see `remote_socket_lost`.
                let _ = this.update(cx, |this, cx| {
                    this.remote_socket_lost(id, cx);
                });
            }
        })
        .detach();
    }

    /// Let a remote host go: close every pane it was running, forget its projects, and drop the
    /// connection — which is what tells the far side, over `FromClient::Gone`, that this window is
    /// no longer attached.
    ///
    /// One method for both ways a host is lost: the socket ending under [`Self::route_host`], and
    /// the Hosts section's Disconnect button. Answers with the label it was attached under, which
    /// is all either caller needs afterwards and is gone from the `Bus` by then.
    ///
    /// Panes are closed *before* anything is forgotten about them — `close_pane` sends a
    /// `CloseWorkspace` that has to resolve to this host to be dropped rather than misdelivered,
    /// and forgets the pane itself as it goes. See [`Bus::drop_remote`].
    pub fn disconnect_host(&mut self, id: HostId, cx: &mut Context<Self>) -> String {
        let saved = self.bus.remote(id).map(|remote| {
            (
                remote.save_id.clone(),
                remote.address.clone(),
                remote.label.clone(),
            )
        });
        let label = saved
            .as_ref()
            .map(|(_, _, label)| label.clone())
            .unwrap_or_default();
        for pane_id in self.bus.drop_remote(id) {
            self.close_pane(pane_id, cx);
            // `close_pane` gives up on a pane whose project this window does not hold open, so it
            // has not necessarily forgotten it. Nothing may stay recorded under a host that is
            // gone.
            self.bus.forget_pane(pane_id);
        }
        // Asked for, not dropped: the reconnect loop stops with it rather than dialling back a
        // host the user just let go of.
        if let Some((save_id, address, _)) = saved {
            let key = crate::app::host_secrets::key_for(&save_id, &address);
            self.workbench.settings.reconnects.remove(&key);
        }
        cx.notify();
        label
    }

    /// The saved address a live connection's label belongs to.
    ///
    /// A remote is labelled with the saved host's name when it was reconnected from the Hosts
    /// section and with the bare address when it was dialled fresh, so both are tried — the
    /// address is what `failed_hosts` and every other saved-host lookup is keyed by.
    pub(super) fn address_of_host(&self, label: &str) -> Option<String> {
        self.workbench
            .settings
            .host
            .remote_hosts
            .iter()
            .find(|host| host.name == label || host.address == label)
            .map(|host| host.address.clone())
    }

    /// Everything the coordinator says, in the order it said it.
    ///
    /// `host` names which of the window's connections `message` arrived on — routed here by
    /// [`Self::route_host`], one task per connection. It is threaded through every family so a
    /// handler can record which host a pane or project belongs to as it first hears of one, and,
    /// from a later phase on, keep two hosts' projections apart instead of merging a remote's
    /// answer into the local one's state.
    ///
    /// The families are disjoint, so each helper answers with the message back when it is
    /// none of its own and the next one is offered it.
    pub(super) fn receive(&mut self, host: HostRef, message: Message, cx: &mut Context<Self>) {
        let Some(message) = self.receive_pane(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_project(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_file(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_git(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_work(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_conversation(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_session(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_account(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_search(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_assist(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_repo(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_host_browse(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_notifications(host, message, cx) else {
            return;
        };
        let Some(message) = self.receive_web_assets(message, cx) else {
            return;
        };
        // The rest are the window's own words, coming back the wrong way.
        tracing::warn!("the window was sent a message only it may send: {message:?}");
    }

    /// The pane family.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_pane(
        &mut self,
        host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::WorkspaceSpawned { workspace } => {
                // Recorded before `open_pane` draws anything, so a message about this pane that
                // arrives on the very next poll — a resize, an exit — already finds it owned.
                self.bus.note_pane(workspace.id, host);
                self.open_pane(workspace, cx);
            }

            // Output is handed straight to the pane's emulator. Output for a pane that has gone is
            // dropped: nothing is left to draw it.
            Message::TerminalOutput { pane_id, bytes } => {
                if let Some(terminal) = self.terminals.get(&pane_id)
                    && let Some(output) = &terminal.output
                {
                    let _ = output.send(bytes);
                }
            }

            // The harness ended: close the tab. `close_pane` sends `CloseWorkspace` so the host
            // drops the pseudo-terminal, and queues the panel out of the dock.
            Message::PaneExited { pane_id, code } => {
                let project = self.projects.iter().find_map(|(id, open)| {
                    open.panes
                        .iter()
                        .any(|pane| pane.id == pane_id)
                        .then_some(*id)
                });
                tracing::info!("pane {pane_id} exited with {code}");
                // A login pane belongs to no project, so `close_pane` would return early and
                // the host would never be told the pane is over — and being told is what makes
                // it look for the credential. Ending it here is the whole of the successful
                // path: a harness that finishes its own sign-in exits by itself.
                //
                // A probe never writes a credential, so the host's `login_gone` sends nothing
                // back for it — no `HarnessLoginCaptured`/`HarnessLoginFailed` will ever arrive
                // to drive `login_ended`. This exit is the only signal a probe gets, so it is
                // read here, locally, instead of waiting on an answer that is never coming.
                if self.login_pane() == Some(pane_id) {
                    let probe = self
                        .workbench
                        .settings
                        .login
                        .as_ref()
                        .is_some_and(|login| login.probe);
                    if probe {
                        self.login_ended(false, "The shell exited.".to_string(), cx);
                    } else {
                        self.bus.send(Message::CloseWorkspace { pane_id });
                    }
                    return None;
                }
                if let Some(project_id) = project {
                    self.bus.send(Message::RefreshProjectGit {
                        project_id,
                        full: true,
                    });
                }
                // A tool run with "wait on exit" stays readable: the command is over but its
                // output is what the pane was opened for, so the tab keeps it until it is
                // closed. The dot reports the stop; closing still goes through `close_pane`.
                if self.pane_wait_on_exit(pane_id) {
                    self.pane_stopped(pane_id);
                    cx.notify();
                } else {
                    self.close_pane(pane_id, cx);
                }
            }

            Message::PaneError { pane_id, error } => {
                self.pane_stopped(pane_id);
                tracing::error!("pane {pane_id}: {error}");
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The project family.
    /// Every window is sent the same snapshots, so each replaces by id and the projection
    /// is idempotent by construction.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_project(
        &mut self,
        host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::ProjectList { projects } => {
                // Every project this catalogue answer names belongs to whichever host sent it —
                // recorded before `sync_projects` reads the registry, so a project this window
                // adopts as it reconciles already resolves to the right host.
                // Read before `cx.global_mut` borrows the app: every project some *other* host
                // reported, which this answer knows nothing about and must not take away.
                let keep = self.bus.projects_not_on(host);
                for project in &projects {
                    self.bus.note_project(project.record.id, host);
                }
                cx.global_mut::<WindowRegistry>()
                    .replace_all_except(projects, &keep);
                self.adopt_if_owed(cx);
                // A catalogue that no longer names a project this window held takes it away, so
                // what the window holds is reconciled before anything is drawn from it.
                self.sync_projects(cx);
            }

            Message::ProjectAdded { project } => {
                let id = project.record.id;
                let root = project.record.path.clone();
                self.bus.note_project(id, host);
                cx.global_mut::<WindowRegistry>().apply(project);
                // There is no clone-success message: a finished clone is a registered project, so
                // this is where the modal learns it worked and gets out of the way.
                if self.clone_in_flight() {
                    self.workbench.clone_project = None;
                    self.take_project(id, cx);
                    cx.notify();
                    return None;
                }
                // Whoever asked for it is the window that opens it.
                if self.adding {
                    self.adding = false;
                    self.take_project(id, cx);
                    // A file dropped with no project open named this folder's leaf as what to
                    // show once the project it became was actually open.
                    if let Some(dropped) = self.adding_select.take()
                        && let Ok(rel) = Path::new(&dropped).strip_prefix(&root)
                    {
                        self.select_file(rel.to_string_lossy().into_owned(), cx);
                    }
                }
                cx.notify();
            }

            Message::ProjectChanged { project } => {
                self.bus.note_project(project.record.id, host);
                cx.global_mut::<WindowRegistry>().apply(project);
                cx.notify();
            }

            Message::ProjectForgotten { project_id } => {
                self.bus.forget_project(project_id);
                cx.global_mut::<WindowRegistry>().forget(project_id);
                self.sync_projects(cx);
            }

            Message::ProjectError { project_id, error } => {
                tracing::error!("project {project_id:?}: {error}");
                self.workbench.project_error = Some(error);
                self.adding = false;
                self.workbench.open_menu = Some(MenuId::Project);
                cx.notify();
            }

            Message::Preferences { scope, value } => self.apply_preferences(scope, value, cx),

            Message::CliShortcutState {
                installed,
                stale,
                target,
                candidates,
                error,
            } => self.apply_cli_shortcut(
                CliShortcut {
                    installed,
                    stale,
                    target,
                    candidates,
                    error,
                },
                cx,
            ),

            Message::Settings { layer, value } => self.apply_settings(layer, value, cx),
            Message::SettingsError { layer, error } => {
                tracing::error!("settings {layer:?}: {error}");
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The file family.
    /// Every answer names its project and its path, so one that arrives after the user has
    /// switched projects lands where it belongs rather than on screen.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_file(
        &mut self,
        // Every file message already names its project, and today every project is Local; a
        // later phase that keeps two hosts' files apart reads this instead of re-deriving it.
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::ProjectTreeListing {
                project_id,
                rel_path,
                listings,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                open.explorer.set_loading(&rel_path, false);
                let filter = self.workbench.file_filter.clone();
                for listing in listings {
                    open.explorer.merge(listing);
                }
                open.explorer.reanchor(&filter);
                // A listing can put a remembered folder within reach, which is what makes
                // restoring a deep one terminate: each answer either resolves one or drops it.
                self.reach_wanted(project_id, cx);
                // A folder opened by a reveal moves the row that was being revealed, so a tree
                // that is following brings it back into view as each answer lands.
                if self
                    .explorer(cx)
                    .is_some_and(|tree| tree.follow != Follow::Off)
                {
                    self.scroll_explorer_to_cursor(cx);
                }
                // The cache fills in the background from project open: each reply names more
                // folders, and those are asked about next, until the skip set is all that remains.
                self.fill_explorer_cache(project_id);
                if !self.workbench.file_filter.trim().is_empty() {
                    let text = self.workbench.file_filter.clone();
                    self.spawn_explorer_filter(text, cx);
                }
                cx.notify();
            }

            Message::ProjectFileContents {
                project_id,
                rel_path,
                contents,
            } => {
                self.pending_files.push(FileArrival {
                    project: project_id,
                    path: rel_path,
                    contents,
                });
                cx.notify();
            }

            Message::ProjectFileWritten {
                project_id,
                rel_path,
                version,
            } => {
                // What the buffer holds now, not what was written: anything typed while the save
                // was in flight is still unsaved, and the tab has to keep saying so.
                let current = self
                    .projects
                    .get(&project_id)
                    .and_then(|open| open.editor.open.iter().find(|f| f.path == rel_path))
                    .and_then(|file| file.buffer())
                    .map(|buffer| buffer.read(cx).value().to_string())
                    .unwrap_or_default();

                if let Some(open) = self.projects.get_mut(&project_id) {
                    if let Some(file) = open.editor.find_mut(&rel_path) {
                        file.saved(version, &current);
                    }
                    // The watcher will echo this write back as a `ProjectFilesChanged` shortly;
                    // that arrival is not a change to react to.
                    open.just_saved.insert(rel_path);
                }
                self.bus.send(Message::RefreshProjectGit {
                    project_id,
                    full: true,
                });
                cx.notify();
            }

            Message::ProjectFileDiffed {
                project_id,
                rel_path,
                diff,
            } => {
                // The base is echoed because the interface may have switched since it asked, and
                // it is the base that says which of a file's tabs this belongs in.
                let key = tab_key(&rel_path, Subject::Diff(diff.base));
                if let Some(open) = self.projects.get_mut(&project_id) {
                    // The Git screen's pane asked for the same comparison a diff tab would, so the
                    // one reply serves whichever of the two is waiting for this path.
                    if open.git_view.path() == Some(rel_path.as_str())
                        && open.git_view.base == diff.base
                    {
                        open.git_view.diff = Some(diff.clone());
                    }
                    if let Some(file) = open.editor.open.iter_mut().find(|file| file.key() == key) {
                        file.attach_diff(diff);
                    }
                }
                cx.notify();
            }

            Message::ProjectPathEdited {
                project_id,
                rel_path,
                to,
                op,
            } => self.path_edited(project_id, rel_path, to, op, cx),

            Message::ProjectFileError {
                project_id,
                rel_path,
                error,
            } => self.file_failed(project_id, rel_path, error, cx),

            Message::ProjectFilesChanged {
                project_id,
                changed,
                truncated,
                repository,
            } => {
                if repository {
                    self.bus.send(Message::RefreshProjectGit {
                        project_id,
                        full: true,
                    });
                }
                let open = self.projects.get(&project_id)?;
                // Only folders the tree already holds are re-asked: a listing for one it does not
                // know is thrown away by `merge` anyway. A burst too large to name its paths says
                // to re-list the root instead.
                let mut dirs: Vec<String> = Vec::new();
                if truncated {
                    if open.explorer.is_listed() {
                        dirs.push(String::new());
                    }
                } else {
                    for path in &changed {
                        let dir = match path.rsplit_once('/') {
                            Some((head, _)) => head.to_string(),
                            None => String::new(),
                        };
                        if !dirs.contains(&dir) && open.explorer.is_folder_listed(&dir) {
                            dirs.push(dir);
                        }
                    }
                }
                // A clean background tab shows what is on disk, so it is read again. A dirty one is
                // left exactly as it is: what has been typed into it is not on disk anywhere. The
                // tab on screen is left alone too — rebuilding its buffer reruns the highlighter,
                // which is a visible flash for a file the user is looking at right now; later this
                // is where "the file changed, keep mine or reload" will hook in instead of silence.
                // A path this window just wrote is the watcher echoing our own save, not a change
                // to react to at all.
                // Whatever is reread keeps its cursor and scroll — captured off the buffer here,
                // before `reload` drops it, and handed back once the fresh bytes attach — because a
                // reread is not a fact the user asked to see from the top; the file they were
                // looking at just changed under them, in place.
                let active_key = open.editor.active_file().map(|file| file.key());
                type Restore = (std::ops::Range<usize>, gpui::Point<Pixels>);
                let reload: Vec<(String, Option<Restore>)> = changed
                    .iter()
                    .filter_map(|path| {
                        if open.just_saved.contains(path) {
                            return None;
                        }
                        let file = open
                            .editor
                            .index_of(path)
                            .map(|index| &open.editor.open[index])?;
                        if file.dirty() || file.is_loading() || Some(file.key()) == active_key {
                            return None;
                        }
                        let restore = file.buffer().map(|buffer| {
                            let state = buffer.read(cx);
                            (state.selected_range(), state.scroll_offset())
                        });
                        Some((path.clone(), restore))
                    })
                    .collect();

                if let Some(open) = self.projects.get_mut(&project_id) {
                    // The echo this arrival might be has now arrived either way.
                    for path in &changed {
                        open.just_saved.remove(path);
                    }
                    for dir in &dirs {
                        open.explorer.set_loading(dir, true);
                    }
                    for (path, restore) in &reload {
                        if let Some(file) = open.editor.find_mut(path) {
                            if let Some((selection, scroll)) = restore {
                                file.set_restore(selection.clone(), *scroll);
                            }
                            file.reload();
                        }
                    }
                }
                for dir in dirs {
                    self.bus.send(Message::ProjectTree {
                        project_id,
                        rel_path: dir,
                        depth: EXPAND_DEPTH,
                    });
                }
                for (path, _) in reload {
                    self.bus.send(Message::ReadProjectFile {
                        project_id,
                        rel_path: path,
                        max_bytes: Some(MAX_FILE_BYTES),
                    });
                }
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The git family.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_git(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::GitOverview {
                project_id,
                overview,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                match overview {
                    None => {
                        open.git = None;
                        open.git_truncated = false;
                        open.explorer.clear_git();
                        open.git_entries.clear();
                        open.git_view.settle(&open.git_entries);
                    }
                    Some(next) => {
                        if let Some(held) = &open.git
                            && next.generation < held.generation
                        {
                            return None;
                        }
                        let counts = next
                            .counts
                            .or_else(|| open.git.as_ref().and_then(|g| g.counts));
                        let mut next = next;
                        if next.counts.is_none() {
                            next.counts = counts;
                        }
                        let submodules = next.submodules.clone();
                        open.git = Some(next);
                        // The Submodules section is built from the overview, not the refs reply,
                        // so a refresh that changes it would otherwise lag one refs answer behind.
                        // Submodule rows always sort last (`RefSection::all()`'s own order), so
                        // dropping and re-appending them leaves every other row's index alone.
                        if !open.git_view.refs.is_empty() {
                            open.git_view
                                .refs
                                .retain(|row| row.section != RefSection::Submodules);
                            open.git_view.refs.extend(submodule_rows(&submodules));
                            if open
                                .git_view
                                .selected_ref
                                .is_some_and(|i| i >= open.git_view.refs.len())
                            {
                                open.git_view.selected_ref = None;
                            }
                        }
                    }
                }
                cx.notify();
            }

            Message::GitWorkingTree {
                project_id,
                generation,
                entries,
                rollups,
                repos,
                truncated,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                if !open
                    .explorer
                    .apply_git(generation, &entries, &rollups, &repos)
                {
                    return None;
                }
                open.git_truncated = truncated;
                // The Git screen's lists are the pairs themselves, so the map is kept whole beside
                // the projection the tree got. A selection whose path has gone clean goes with it.
                open.git_entries = entries;
                open.git_view.settle(&open.git_entries);
                cx.notify();
            }

            Message::GitError { project_id, error } => {
                tracing::error!("git {project_id}: {error}");
                let open = self.projects.get_mut(&project_id)?;
                match error {
                    GitFailure::Corrupt | GitFailure::NotFound => {
                        open.git = None;
                        open.git_truncated = false;
                        open.explorer.clear_git();
                        open.git_entries.clear();
                        open.git_view.settle(&open.git_entries);
                    }
                    GitFailure::Interrupted => {}
                    GitFailure::Denied | GitFailure::Failed(_) => {}
                }
                cx.notify();
            }

            Message::GitRefs { project_id, refs } => {
                let open = self.projects.get_mut(&project_id)?;
                let submodules = open
                    .git
                    .as_ref()
                    .map(|overview| overview.submodules.as_slice())
                    .unwrap_or(&[]);
                open.git_view.refs = ref_rows(&refs, submodules);
                open.git_view.selected_ref = open.git_view.refs.iter().position(|row| row.current);
                cx.notify();
            }

            Message::GitLogPage {
                project_id,
                cursor,
                commits,
                next_cursor,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                // Staleness rule: this reply is answered only if its echoed `cursor` matches the
                // request the view is currently waiting on (`log_inflight`). Two requests can
                // share the same cursor value — most commonly two first-page requests, both
                // `None` — so the *value* alone cannot tell a stale reply from the current one;
                // `log_inflight` is overwritten on every send, so it always names the most
                // recently sent request, and every other reply is discarded here rather than
                // replacing or appending.
                if open.git_view.log_inflight != Some(cursor.clone()) {
                    return None;
                }
                open.git_view.log_inflight = None;
                let rows = commit_rows(&commits);
                if cursor.is_none() {
                    open.git_view.set_commits(rows);
                } else {
                    open.git_view.extend_commits(rows);
                }
                open.git_view.log_done = next_cursor.is_none();
                open.git_view.log_cursor = next_cursor;
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The work family.
    /// Every arm is guarded on the project still being held, because an answer can arrive
    /// after the window has stopped holding it — the file family's rule, for the file
    /// family's reason. The work belongs to the project, so an answer for one nobody here
    /// has open has nowhere to be drawn.
    ///
    /// Anything the host confirms clears the last refusal: a sentence about a change that
    /// did not happen is stale the moment one does.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_work(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::WorkList {
                project_id,
                sessions,
                agents,
                tasks,
            } => {
                self.workbench.work_error = None;
                let open = self.projects.get_mut(&project_id)?;
                open.work.replace_all(sessions, agents, tasks);
                open.graph.relayout(&open.work);
                // Pointing the screen at the first agent was the fixture constructor's job. It
                // belongs to whoever first learns there is one to point at, and only then: a
                // second `ListWork` must not move a selection the user has since made.
                if open.graph.selection.is_none() {
                    open.graph.selection = open.work.agents.first().map(|a| Selection::Agent(a.id));
                }
                // The agents screen lays its columns out the first time it hears there is work,
                // and only prunes after that: an arrangement the user has changed is not something
                // a re-sent list may undo.
                if open.agents.arranged {
                    open.agents.prune(&open.work);
                } else {
                    open.agents.arrange(&open.work);
                }
                self.refill_columns = true;
                self.settle_persistent_chat(project_id, cx);
                cx.notify();
            }

            Message::TaskCreated { project_id, task } => {
                self.workbench.work_error = None;
                let open = self.projects.get_mut(&project_id)?;
                let id = task.id;
                open.work.apply_task(task);
                open.graph.absorb_new(&open.work);
                // The task that arrives is the one to select, because the interface could not know
                // the id it was going to be given — the same mechanism `AppState::adding` uses to
                // open the project an `AddProject` answers with.
                if open.board.awaiting_new {
                    open.board.awaiting_new = false;
                    open.board.select(id);
                }
                cx.notify();
            }

            Message::TaskChanged { project_id, task } => {
                self.workbench.work_error = None;
                let open = self.projects.get_mut(&project_id)?;
                // The mark goes on whatever column the answer reports, the old one included: a
                // refusal that left the card where it was must not leave it saying it is still on
                // its way.
                if open.board.is_moving(task.id) {
                    open.board.moving = None;
                }
                let selected = open.board.selected == Some(task.id);
                let editing = open.board.editing.is_some();
                open.work.apply_task(task);
                open.graph.absorb_new(&open.work);
                // Refill the panel from what the host actually stored — it trims a title, and a
                // field showing what was typed rather than what was kept would be a small lie. Not
                // while a field is open: the user's text wins until they commit or discard it.
                if selected && !editing {
                    self.form_filled = None;
                }
                cx.notify();
            }

            Message::TaskDeleted {
                project_id,
                task_id,
            } => {
                self.workbench.work_error = None;
                let open = self.projects.get_mut(&project_id)?;
                open.work.forget_task(task_id);
                // A panel pointed at a task that has gone reports on nothing, and a mark for a
                // move that can never be answered would never come off.
                if open.board.selected == Some(task_id) {
                    open.board.selected = None;
                    // A field open on a task that has gone has nowhere to commit to.
                    open.board.stop_editing();
                    open.board.confirm_delete = false;
                }
                if open.board.is_moving(task_id) {
                    open.board.moving = None;
                }
                cx.notify();
            }

            Message::AgentChanged { project_id, agent } => {
                self.workbench.work_error = None;
                let open = self.projects.get_mut(&project_id)?;
                open.work.apply_agent(*agent);
                open.graph.absorb_new(&open.work);
                // An arriving agent is not put in a column: the arrangement is the user's, and the
                // sidebar lists it on the bench with one click to bring it on. What a change *can*
                // do is take a column's tab away, if the agent behind it has gone.
                if open.agents.prune(&open.work) {
                    self.refill_columns = true;
                }
                cx.notify();
            }

            Message::WorkError {
                project_id,
                task_id,
                error,
            } => {
                tracing::error!("work {project_id} {task_id:?}: {error}");
                self.workbench.work_error = Some(error);
                // A refusal ends whatever asked for it, so the panel goes back to reporting the
                // task the host still holds rather than sitting in a field that will not commit.
                if let Some(board) = self.board_mut(cx) {
                    board.stop_editing();
                    board.moving = None;
                    board.awaiting_new = false;
                    board.confirm_delete = false;
                }
                self.form_filled = None;
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The conversation family.
    ///
    /// A live agent joins the work as any other agent does, so the sidebar and the graph
    /// find it with no change of their own. What is different is that its record is then
    /// kept current from the stream rather than from a reply: what the transcript already
    /// says is what the badge, the ring and the token count are read off, because a round
    /// trip per token would be a round trip per token.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_conversation(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::ConversationStarted {
                project_id,
                agent,
                session,
                accepts_input,
            } => {
                self.workbench.work_error = None;
                let open = self.projects.get_mut(&project_id)?;
                let id = agent.id;
                let harness = agent.harness.clone();
                let account = agent.account.clone();
                // The session first: the sidebar lists agents under one, so an agent applied
                // before its heading exists is an agent drawn nowhere.
                open.work.apply_session(session);
                open.work.apply_agent(*agent);
                open.graph.absorb_new(&open.work);
                let conversation = open
                    .conversations
                    .entry(id)
                    .or_insert_with(|| Conversation::new(id, harness, account));
                conversation.accepts_input = accepts_input;
                // The agents screen draws only what this window can talk to, and this is the one
                // place a conversation comes into being — see `AgentsView::live`.
                open.agents.live = open.conversations.keys().copied().collect();
                open.agents.prune(&open.work);
                // Where it lands is whoever asked. A `+` or a chat header that raised the New
                // agent form aimed the start at itself — see `AppState::aim_start` — and anything
                // else comes on the agents screen's field rather than onto the bench: unlike an
                // agent that merely changed, this one was asked for.
                match (
                    self.pending_chat_attach.take(),
                    std::mem::take(&mut self.pending_chat_open),
                    std::mem::take(&mut self.sink.messages.pending_attach),
                ) {
                    (Some(chat), _, _) => self.attach_chat(chat, Some(id), cx),
                    // The chat strip's `+`: **this** is where the tab comes into being, so a form
                    // that was dismissed instead left no empty tab behind.
                    (None, true, _) => {
                        if let Some(chat) = self.open_chat_tab_now(cx) {
                            self.attach_chat(chat, Some(id), cx);
                        }
                    }
                    (None, false, true) => self.sink.messages.agent = Some(id),
                    (None, false, false) => {
                        if let Some(open) = self.projects.get_mut(&project_id) {
                            open.agents.reveal(id);
                        }
                    }
                }
                self.refill_columns = true;
                cx.notify();
            }

            Message::ConversationUpdate {
                agent_id,
                seq,
                update,
                ..
            } => {
                // The per-token deltas, which are the ones that arrive at the harness's own rate.
                // Everything else — a title, a usage report, a permission ask, a turn ending —
                // changes what the sidebar and the lifecycle glyph say and always draws.
                let streaming = matches!(
                    update.as_ref(),
                    ubiq_proto::conversation::ConvUpdate::AgentChunk { .. }
                        | ubiq_proto::conversation::ConvUpdate::ThoughtChunk { .. }
                );
                // What the bell cares about, read off the update before `apply` consumes it:
                // a permission ask (any conversation, a delegate's own included — the ask
                // still blocks a turn somebody has to answer) and a turn ending, narrowed to
                // the main agent below since a delegate's turns are folded into the
                // transcript rather than answered with their own `TurnEnded`.
                let wants_permission = matches!(
                    update.as_ref(),
                    ubiq_proto::conversation::ConvUpdate::PermissionRequest { .. }
                );
                let turn_ended = matches!(
                    update.as_ref(),
                    ubiq_proto::conversation::ConvUpdate::TurnEnded { .. }
                );
                // Anything that is not a chunk draws whatever is on screen; a chunk draws where
                // the style reference's bench is reading this conversation. That is the third
                // surface hosting one, and it picks across every project — so it is asked before
                // the project holding this one is borrowed.
                let elsewhere = !streaming || self.sink_agent() == Some(agent_id);
                let open = self
                    .projects
                    .values_mut()
                    .find(|open| open.conversations.contains_key(&agent_id))?;
                let conversation = open.conversations.get_mut(&agent_id)?;
                if !conversation.is_next(seq) {
                    tracing::warn!(
                        "conversation {agent_id}: update {seq} does not follow the last one \
                             applied, so something was lost between them; applying it anyway"
                    );
                }
                conversation.apply(seq, *update);
                // A turn that just ended may have prompts typed while it was running, waiting
                // behind it — the front of the queue goes out now, the same way it would have if
                // the box had been empty when it was typed.
                let next_prompt = (conversation.run == Run::Idle)
                    .then(|| conversation.dequeue_front())
                    .flatten();
                refresh_agent_record(open, agent_id);
                // Whether anything on screen is drawing this conversation: the tab a column has
                // up, or a chat tab attached to it. A delegate nobody is looking at still folds
                // its stream into the record — it just stops driving frames while it does.
                let shown = (0..open.agents.columns.len())
                    .any(|column| open.agents.active_agent(column) == Some(agent_id))
                    || open.chats.iter().any(|tab| tab.attached == Some(agent_id));
                let on_screen = elsewhere || shown;
                // Read before `open`'s borrow ends below — the bell, per `G198`. A permission ask
                // is worth the interruption whoever it is for, a delegate included: it still
                // blocks a turn somebody has to answer. A turn ending is the *main* agent's alone
                // — a delegate's turns fold into the transcript rather than closing with their
                // own `TurnEnded`, so `parent` is what tells the two apart — and it is the bell
                // for the conversation nobody has on screen.
                let agent_name = open.work.agent(agent_id).map(|a| a.name.clone());
                let is_delegate = open
                    .work
                    .agent(agent_id)
                    .is_some_and(|a| a.parent.is_some());
                if on_screen {
                    self.draw_conversation(agent_id, streaming, cx);
                }
                // A permission ask is raised **whether or not the conversation is on screen**,
                // and it is the one notification that also leaves the window. Every other bell
                // here reports something that already happened, so a surface already drawing it
                // has said it; an ask is a question that blocks the turn until somebody answers,
                // and the surface drawing it may be behind another window, on another screen, or
                // scrolled away from the prompt. `shown` says a pane exists, never that the user
                // is looking at it. A mute rule on `Agents · permission` is how somebody who does
                // not want this turns it off.
                if wants_permission {
                    let mut request = NotificationRequest::warning(
                        Family::Agents,
                        "Wants permission to continue.",
                    )
                    .with_category("permission")
                    .with_link(UbiqLink::Agent(agent_id))
                    .with_os();
                    if let Some(name) = agent_name.clone() {
                        request = request.with_actor(name);
                    }
                    self.raise_notification(request);
                }
                if !shown && turn_ended && !is_delegate {
                    let mut request =
                        NotificationRequest::info(Family::Agents, "The turn finished.")
                            .with_category("turn")
                            .with_link(UbiqLink::Agent(agent_id));
                    if let Some(name) = agent_name {
                        request = request.with_actor(name);
                    }
                    self.raise_notification(request);
                }
                if let Some(queued) = next_prompt {
                    self.send_prompt(agent_id, queued.text);
                }
            }

            // The harness has gone; the transcript has not. The record stops moving, and the
            // conversation is kept so what was said is still readable.
            Message::ConversationEnded {
                agent_id,
                stop_reason,
            } => {
                let open = self
                    .projects
                    .values_mut()
                    .find(|open| open.conversations.contains_key(&agent_id))?;
                let conversation = open.conversations.get_mut(&agent_id)?;
                conversation.ended(stop_reason);
                refresh_agent_record(open, agent_id);
                cx.notify();
            }

            // The harness is gone, but unlike `ConversationEnded` the conversation itself is back
            // to its pre-launch state: the pickers return, and the next prompt — or a resume —
            // starts a new process.
            Message::ConversationUnloaded { agent_id } => {
                let open = self
                    .projects
                    .values_mut()
                    .find(|open| open.conversations.contains_key(&agent_id))?;
                let conversation = open.conversations.get_mut(&agent_id)?;
                conversation.unloaded();
                refresh_agent_record(open, agent_id);
                cx.notify();
            }

            // Delete answered: unlike `ConversationEnded`, which keeps the transcript because the
            // harness merely stopped, there is nothing left to draw here. The conversation goes,
            // the agent record goes with it, and **every chat tab that was looking at it is
            // closed** — a detached tab would be an empty panel the user never asked for, left
            // where a conversation used to be, and the delete is the one gesture that says there
            // is nothing to look at. An empty tab is what a fresh `+` produces on request; it is
            // not what a delete should leave behind.
            //
            // Closing goes through `close_chat_tab_in` rather than dropping the `ChatTab` row
            // here: the dock panel has to leave the tree too, and the composer slot has to be
            // cleared before it is handed on.
            Message::ConversationDeleted { agent_id } => {
                let (project, open) = self
                    .projects
                    .iter_mut()
                    .find(|(_, open)| open.conversations.contains_key(&agent_id))?;
                let project = *project;
                open.conversations.remove(&agent_id);
                // The row goes from the work projection, which is what the agents columns, the
                // computed bench and a chat tab's attach list all read — so one removal takes it
                // off every surface that could still offer it.
                open.work.remove_agent(agent_id);
                let watching: Vec<ChatId> = open
                    .chats
                    .iter()
                    .filter(|tab| tab.attached == Some(agent_id))
                    .map(|tab| tab.id)
                    .collect();
                open.agents.live = open.conversations.keys().copied().collect();
                open.agents.prune(&open.work);
                open.graph.absorb_new(&open.work);
                for tab in watching {
                    self.close_chat_tab_in(project, tab, cx);
                }
                self.refill_columns = true;
                cx.notify();
            }

            // A start that failed before a conversation existed still has to say so, which is why
            // the sentence goes to the workbench as well as onto the transcript: the screen the
            // user is looking at is the agents screen either way.
            Message::ConversationError { agent_id, error } => {
                tracing::error!("conversation {agent_id}: {error}");
                if let Some(conversation) = self
                    .projects
                    .values_mut()
                    .find_map(|open| open.conversations.get_mut(&agent_id))
                {
                    conversation.error = Some(error.clone());
                }
                self.workbench.work_error = Some(error);
                cx.notify();
            }

            // Ubiq read the opening exchange and named the conversation. Not a `ConvUpdate` and
            // carrying no `seq`: the naming is this side's own reading rather than something the
            // harness said, so it never counts against the gap check.
            Message::ConversationNamed {
                agent_id,
                title,
                summary,
            } => {
                let open = self
                    .projects
                    .values_mut()
                    .find(|open| open.conversations.contains_key(&agent_id))?;
                let conversation = open.conversations.get_mut(&agent_id)?;
                conversation.name(title, summary);
                refresh_agent_record(open, agent_id);
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// Draw the delta just folded into a conversation — at most once a frame while it is
    /// streaming.
    ///
    /// The frame itself is the coalescing window: the first delta of a burst asks for one, and
    /// every delta after it is already in the record that frame draws, so it asks for nothing.
    /// `ui::conversation`'s transcript clears the flag as it draws, which is what starts the next
    /// cycle — see [`Conversation::notify_due`]. Anything that is not a chunk draws immediately:
    /// those are the updates the sidebar, the lifecycle glyph and the permission strip read.
    fn draw_conversation(&mut self, agent_id: AgentId, streaming: bool, cx: &mut Context<Self>) {
        let due = !streaming
            || self
                .projects
                .values()
                .find_map(|open| open.conversations.get(&agent_id))
                .is_some_and(Conversation::notify_due);
        if due {
            cx.notify();
        }
    }

    /// The notification family: the bell's whole state, and each arrival as it is filed.
    ///
    /// Both are broadcast to every window, so nothing here is a reply to a request this window
    /// made — it redraws from what the host says, and never edits the state it was sent.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_notifications(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            // The whole state, replacing what was held: the list is capped and the host is the
            // one that keeps it, so half an old state beside half a new one is nobody's bell.
            Message::NotificationsState { state } => {
                self.notifications.wire = *state;
                cx.notify();
            }

            // One arrival, put at the front because the list is newest first. The host has
            // already applied the rules, so `muted` is its verdict and not a question: a muted
            // one only moves the badge, and a loud one is what the bell flashes for.
            Message::NotificationRaised { notification } => {
                let (level, link, muted) = (
                    notification.level,
                    notification.link.clone(),
                    notification.muted,
                );
                self.notifications.wire.items.insert(0, *notification);
                self.notifications.wire.items.truncate(HISTORY_CAP);
                if muted {
                    cx.notify();
                } else {
                    self.flash_bell(level, link, cx);
                }
            }

            other => return Some(other),
        }
        None
    }

    /// The session family: what the host is, and what can be started on it.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_session(
        &mut self,
        host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            // What the host is. The status bar says so when the root is not the usual one.
            //
            // Only the local host's answer is taken. Every host greets a new client with this
            // unsolicited, so a remote's would otherwise arrive on attach and repaint the config
            // root — and, worse, re-ask for the shells, harnesses, accounts and profiles, whose
            // answers replace those lists whole. The menus name what can be started on *this*
            // machine; a remote's belong to a per-host set of them that does not exist yet.
            Message::HostInfo {
                ref config_root,
                is_default,
                ..
            } => {
                if host != HostRef::Local {
                    self.bus.note_remote_info(host, &message);
                    return None;
                }
                self.workbench.config_root = Some(config_root.clone());
                self.workbench.config_root_is_default = is_default;
                // The new-pane menu offers what this machine has, and only the host can say what
                // that is. Asked on attach so the first menu is not empty, and again on every
                // open — see `open_new_pane_menu`.
                self.bus.send(Message::ListShells);
                self.bus.send(Message::ListAgentTypes);
                // The identities half of the same question: the New agent menu offers a harness
                // per account, so an empty account list would offer the harness alone and start
                // it as nobody in particular.
                self.bus.send(Message::ListAccounts);
                // And the setups built on top of them, for the same reason: the menu offers a
                // row per profile.
                self.bus.send(Message::ListProfiles);
                // And the bell, so a window that has just attached draws the badge the other
                // windows are already drawing rather than an empty one until something arrives.
                self.bus.send(Message::ListNotifications);
                cx.notify();
            }

            // What can be started here. Replaced whole rather than merged: the host's answer is
            // the list, and a shell that has been uninstalled has to leave the menu.
            Message::ShellList { shells } => {
                self.workbench.shells = shells;
                cx.notify();
            }

            // One reading of the host, in answer to one `ListStats`. Replaced whole rather than
            // merged: every field is a sample taken at the same moment, and half of an old
            // reading beside half of a new one is a picture of no moment at all.
            Message::Stats { stats } => {
                if host != HostRef::Local {
                    self.bus.note_remote_stats(host, &stats);
                } else {
                    self.stats.host = Some(stats);
                }
                cx.notify();
            }

            // Which agent harnesses can be started here. Replaced whole, same as the shell list:
            // a harness that has been uninstalled has to leave the menu, or read as unavailable.
            Message::AgentTypes { agent_types } => {
                self.workbench.agent_types = agent_types;
                cx.notify();
            }

            // The runnable tools for the new-pane menu: the machine-wide rows, then the
            // project's. Replaced whole, same as the shell list — a tool deleted in the
            // settings has to leave the menu.
            Message::ToolsListed { system, project } => {
                self.workbench.tools = system.into_iter().chain(project).collect();
                cx.notify();
            }

            // A tool run was refused before a pane existed: nothing was announced, so there is
            // no tab to close and no dot to dim. A log line, the way a refused spawn's
            // `PaneError` is one.
            Message::ToolError { project_id, error } => {
                tracing::error!("tool for project {project_id:?} was not run: {error}");
            }

            // What a harness offers, for whichever start form asked. Dropped unless it matches
            // the form's harness *and* identity as they stand now: a probe is slow exactly once
            // and a slow answer for a harness the user has since changed away from would
            // overwrite the fresh one it arrived behind.
            Message::HarnessCatalogue {
                agent_type,
                account,
                models,
                last_model,
                last_thinking,
            } => {
                let profile_model = self
                    .new_agent_form()
                    .and_then(|form| form.model.clone())
                    .filter(|it| !it.is_empty());
                let profile_thinking = self
                    .new_agent_form()
                    .and_then(|form| form.thinking.clone())
                    .filter(|it| !it.is_empty());
                if let Some(form) = self.new_agent_form_mut()
                    && form.agent_type == agent_type
                    && form.account == account
                {
                    // Preselection, in order of how much it knows: what the form was opened
                    // holding, then what this harness was last launched with, then the harness's
                    // own default.
                    let model = profile_model
                        .or_else(|| (!last_model.is_empty()).then(|| last_model.clone()))
                        .or_else(|| models.iter().find(|it| it.default).map(|it| it.id.clone()))
                        .filter(|id| models.iter().any(|it| it.id == *id));
                    let thinking = profile_thinking
                        .or_else(|| (!last_thinking.is_empty()).then(|| last_thinking.clone()))
                        .or_else(|| {
                            model.as_ref().and_then(|id| {
                                models
                                    .iter()
                                    .find(|it| it.id == *id)
                                    .and_then(|it| it.default_level.clone())
                            })
                        });
                    form.models = models;
                    form.probing = false;
                    form.model = model;
                    form.thinking = thinking;
                }
                cx.notify();
            }

            // What the host made of a typed command. Kept on the login modal, and only while it
            // is still on the harness that was asked about — an answer about the old pick would
            // read as an answer about the new one.
            Message::AgentCommandChecked {
                agent_type,
                ok,
                detail,
            } => {
                if let Some(login) = &mut self.workbench.settings.login
                    && let LoginStep::Choosing {
                        agent_type: Some(chosen),
                    } = &login.step
                    && *chosen == agent_type
                {
                    login.command_check = Some((ok, detail));
                }
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The account family.
    /// Replaced whole for the same reason as the two lists above: the host's answer is
    /// the set of identities, and one deleted elsewhere has to leave the screen.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_account(
        &mut self,
        host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::Accounts { accounts } => {
                // Prune whatever `statuses` and `dialog` named that this answer no longer
                // carries, so a renamed or deleted account cannot leak an entry forever.
                self.workbench
                    .settings
                    .statuses
                    .retain(|(agent_type, account), _| {
                        accounts.iter().any(|info| {
                            info.id == *account && info.logged_in.iter().any(|id| id == agent_type)
                        })
                    });
                // The quota maps are keyed the same way and go stale the same way, so they are
                // pruned against the same answer.
                self.workbench
                    .settings
                    .quotas
                    .retain(|(agent_type, account), _| {
                        accounts.iter().any(|info| {
                            info.id == *account && info.logged_in.iter().any(|id| id == agent_type)
                        })
                    });
                self.workbench
                    .settings
                    .quota_errors
                    .retain(|(agent_type, account), _| {
                        accounts.iter().any(|info| {
                            info.id == *account && info.logged_in.iter().any(|id| id == agent_type)
                        })
                    });
                self.workbench.settings.accounts = accounts;
                // The accounts page is what the answer was asked for: it arrives after the page
                // is already open, so this is where the readouts are filled rather than in the
                // open handler, which had no list to walk yet. Cached answers only — a fresh
                // read is what the refresh control is for.
                if self.workbench.settings.open
                    && self.workbench.settings.nav == SettingsSection::Harnesses
                {
                    self.ask_quotas();
                }
                cx.notify();
            }
            // How much of one login's plan is left, in answer to one `QueryQuota`. The two
            // fields are exclusive and each replaces its own entry: a failed refresh writes the
            // sentence and leaves the last good reading on screen beside it, because a reading
            // that was true ten minutes ago is still worth more than an empty panel.
            Message::QuotaRead {
                account,
                harness,
                snapshot,
                error,
            } => {
                let key = (harness, account);
                match snapshot {
                    Some(snapshot) => {
                        self.workbench.settings.quotas.insert(key.clone(), snapshot);
                        self.workbench.settings.quota_errors.remove(&key);
                    }
                    None => {
                        if let Some(error) = error {
                            self.workbench.settings.quota_errors.insert(key, error);
                        }
                    }
                }
                cx.notify();
            }
            // The same fact, said without being asked — a running agent pushed a window, or the
            // host's poll refreshed one. Broadcast to every window, so this is how a surface
            // showing that account keeps up without polling the host itself.
            Message::QuotaChanged {
                account,
                harness,
                snapshot,
            } => {
                let key = (harness, account);
                self.workbench.settings.quotas.insert(key.clone(), snapshot);
                self.workbench.settings.quota_errors.remove(&key);
                cx.notify();
            }
            // The saved setups, replaced whole for the reason the accounts are: the host's
            // answer is the list. It also closes the form, since a `Profiles` right after a
            // `SaveProfile` is what says the write landed.
            Message::Profiles { profiles } => {
                self.workbench.settings.profiles = profiles;
                self.workbench.settings.profile_form = None;
                cx.notify();
            }
            // What this build can inject into a harness. Replaced whole, the same way the harness
            // list is: the host's answer *is* the catalogue, and a server this build no longer
            // ships has to leave the checklist.
            Message::Mcps { servers } => {
                self.workbench.mcps = servers;
                cx.notify();
            }
            Message::HarnessLoginStarted {
                pane_id,
                agent_type,
                account,
                cols,
                rows,
            } => {
                // A login pane belongs to no project, so this is the only place it is ever
                // recorded as belonging to a host at all.
                self.bus.note_pane(pane_id, host);
                self.login_started(pane_id, agent_type, account, cols, rows, cx);
            }
            Message::HarnessLoginCaptured {
                agent_type,
                account,
            } => {
                self.login_ended(true, format!("{account} is signed in to {agent_type}."), cx);
            }
            Message::HarnessLoginFailed {
                agent_type,
                account,
                error,
            } => {
                tracing::info!("login for {account} on {agent_type} captured nothing: {error}");
                self.login_ended(false, error, cx);
            }
            Message::HarnessLoginLink { pane_id, url } => {
                self.login_link(pane_id, url, cx);
            }
            Message::HarnessLoginStatus {
                agent_type,
                account,
                status,
            } => {
                self.workbench
                    .settings
                    .statuses
                    .insert((agent_type, account), status);
                cx.notify();
            }
            Message::AccountError { error } => {
                self.workbench.settings.error = Some(error);
                cx.notify();
            }

            // The connector family rides here rather than in a family of its own: it answers the
            // same section's questions and writes the same error field.
            //
            // Every flow message is matched against the id the modal holds and discarded
            // otherwise — the search family's discipline, and the only thing that stops a stage
            // still in flight from reopening a modal the user has closed.
            Message::Connections {
                connections,
                bundled,
            } => {
                self.workbench.settings.bundled = bundled;
                self.workbench.settings.connection_status = connections
                    .iter()
                    .map(|info| (info.connection.id, info.status.clone()))
                    .collect();
                self.workbench.settings.host.connections = connections
                    .into_iter()
                    .map(|info| info.connection)
                    .collect();
                cx.notify();
            }
            Message::ConnectPending { connect_id, stage } => {
                if let Some(connect) = &mut self.workbench.settings.connect
                    && connect.connect_id == connect_id
                {
                    connect.step = match stage {
                        ConnectStage::Opening => ConnectStep::Opening,
                        ConnectStage::DeviceCode {
                            user_code,
                            verification_url,
                            expires_in,
                        } => ConnectStep::DeviceCode {
                            user_code,
                            verification_url,
                            expires_in,
                        },
                        ConnectStage::AwaitingCallback { port, url } => {
                            ConnectStep::AwaitingCallback { port, url }
                        }
                        ConnectStage::Exchanging => ConnectStep::Exchanging,
                        ConnectStage::NeedSecret { prompt } => ConnectStep::NeedSecret { prompt },
                        ConnectStage::AwaitingCertificate => ConnectStep::AwaitingCertificate,
                    };
                    cx.notify();
                }
            }
            Message::ConnectCaptured {
                connect_id,
                connection,
            } => {
                // The record is written whether or not the modal is still up — a flow the user
                // walked away from still finished — and only the modal is conditional.
                let connections = &mut self.workbench.settings.host.connections;
                let id = connection.connection.id;
                connections.retain(|existing| existing.id != id);
                connections.push(connection.connection);
                self.workbench
                    .settings
                    .connection_status
                    .insert(id, connection.status);
                if self
                    .workbench
                    .settings
                    .connect
                    .as_ref()
                    .is_some_and(|connect| connect.connect_id == connect_id)
                {
                    self.workbench.settings.connect = None;
                    self.workbench.settings.cert = None;
                }
                cx.notify();
            }
            Message::ConnectFailed { connect_id, error } => {
                if let Some(connect) = &mut self.workbench.settings.connect
                    && connect.connect_id == connect_id
                {
                    // The fields are left alone: "Try again" is a retry, not a re-type.
                    connect.step = ConnectStep::Failed { error };
                    self.workbench.settings.cert = None;
                    cx.notify();
                }
            }
            Message::ConfirmCertificate {
                connect_id,
                origin,
                cert,
            } => {
                if self
                    .workbench
                    .settings
                    .connect
                    .as_ref()
                    .is_some_and(|connect| connect.connect_id == connect_id)
                {
                    self.workbench.settings.cert = Some(CertPrompt {
                        connect_id,
                        origin,
                        cert,
                    });
                    cx.notify();
                }
            }
            Message::ConnectionStatus { connection, status } => {
                // Patched in place: the answer is about one connection, and replacing the list
                // would throw away everything the host has not just re-sent.
                self.workbench
                    .settings
                    .connection_status
                    .insert(connection, status);
                cx.notify();
            }
            Message::ConnectorError { error } => {
                self.workbench.settings.error = Some(error);
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The search family.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_search(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::SearchMatches {
                project_id,
                search_id,
                batch,
            } => {
                let dominated = self
                    .search
                    .active
                    .as_ref()
                    .is_some_and(|a| a.search_id == search_id && a.project_id == project_id);
                if !dominated {
                    return None;
                }
                if let ubiq_proto::search::Batch::Files(file_hits) = batch {
                    for hit in file_hits {
                        self.search.total_hits += hit.lines.len();
                        if let Some(existing) = self
                            .search
                            .results
                            .iter_mut()
                            .find(|r| r.rel_path == hit.rel_path)
                        {
                            existing.hits.extend(hit.lines);
                            existing.truncated |= hit.truncated;
                        } else {
                            self.search.truncated |= hit.truncated;
                            self.search.results.push(crate::state::search::FileResult {
                                rel_path: hit.rel_path,
                                hits: hit.lines,
                                truncated: hit.truncated,
                            });
                        }
                    }
                }
                cx.notify();
            }

            Message::SearchProgress {
                project_id,
                search_id,
                files_seen,
            } => {
                let dominated = self
                    .search
                    .active
                    .as_ref()
                    .is_some_and(|a| a.search_id == search_id && a.project_id == project_id);
                if !dominated {
                    return None;
                }
                self.search.files_seen = files_seen;
                cx.notify();
            }

            Message::SearchFinished {
                project_id,
                search_id,
                searched: _,
                truncated,
            } => {
                let dominated = self
                    .search
                    .active
                    .as_ref()
                    .is_some_and(|a| a.search_id == search_id && a.project_id == project_id);
                if !dominated {
                    return None;
                }
                self.search.truncated |= truncated;
                self.search.finished = true;
                self.search.active = None;
                cx.notify();
            }

            Message::SearchError {
                project_id,
                search_id,
                error,
            } => {
                let dominated = self
                    .search
                    .active
                    .as_ref()
                    .is_some_and(|a| a.search_id == search_id && a.project_id == project_id);
                if !dominated {
                    return None;
                }
                self.search.error = Some(error);
                self.search.finished = true;
                self.search.active = None;
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// The assist family: whether assistance can run, what one asked-for suggestion came back
    /// with, and what the host holds for the configured API providers.
    ///
    /// Nothing here names a pane, so the family is routed by its variants alone. `Assist` is the
    /// host's standing answer and is simply kept — a window that has not been told yet draws
    /// "checking" rather than "unavailable". A suggestion, whole or in chunks, is matched against
    /// the id the interface is waiting on and discarded otherwise, the search family's discipline
    /// and for its reason: an answer to a request nobody is waiting for has nowhere to be put.
    ///
    /// The provider list arrives here too, and is kept beside the host record rather than in it:
    /// `ai_providers` on the settings blob is host-mutated, and `has_key` is an answer about a
    /// record rather than part of one.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_assist(
        &mut self,
        _host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            Message::Assist {
                available,
                reason,
                detail,
                limits,
            } => {
                self.workbench.settings.assist = Some(AssistInfo {
                    available,
                    reason,
                    detail,
                    limits,
                });
                cx.notify();
            }

            Message::Suggestion { suggest_id, text } => {
                if self.suggest != Some(suggest_id) {
                    return None;
                }
                tracing::debug!("suggestion for {suggest_id}: {} bytes", text.len());
                self.suggest = None;
                // The whole answer replaces whatever the chunks drew. They concatenate to exactly
                // this text, so a backend that streamed is not redrawn and one that did not gets
                // its answer here.
                if let Some(test) = &mut self.workbench.settings.ai_test
                    && test.suggest_id == Some(suggest_id)
                {
                    test.answer = text;
                    test.done = true;
                    test.suggest_id = None;
                }
                cx.notify();
            }

            // Part of an answer, as it arrives. Filtered against the id in flight exactly as the
            // whole answer is, then appended — a first token on screen instead of a spinner.
            Message::SuggestChunk { suggest_id, text } => {
                if self.suggest != Some(suggest_id) {
                    return None;
                }
                if let Some(test) = &mut self.workbench.settings.ai_test
                    && test.suggest_id == Some(suggest_id)
                {
                    test.answer.push_str(&text);
                }
                cx.notify();
            }

            Message::SuggestError { suggest_id, error } => {
                if self.suggest != Some(suggest_id) {
                    return None;
                }
                tracing::warn!("suggestion {suggest_id} failed: {error}");
                self.suggest = None;
                // Chunks already drawn are not a partial answer: they go with the failure.
                if let Some(test) = &mut self.workbench.settings.ai_test
                    && test.suggest_id == Some(suggest_id)
                {
                    test.answer.clear();
                    test.error = Some(error);
                    test.done = true;
                    test.suggest_id = None;
                }
                cx.notify();
            }

            Message::AiProviders { providers } => {
                self.workbench.settings.ai_providers = providers;
                cx.notify();
            }

            // Kept per provider, because the form's two pickers read the list for the provider
            // they are editing and nothing else. `refreshed` is only worth a line in the log: the
            // note under the pickers reads the list's own timestamp.
            Message::AiModels { list, refreshed } => {
                tracing::debug!(
                    "{} models for {} ({})",
                    list.models.len(),
                    list.provider_id,
                    if refreshed { "listed" } else { "cached" }
                );
                self.workbench
                    .settings
                    .ai_models
                    .insert(list.provider_id, list);
                cx.notify();
            }

            // One line for the assistance section's banner. Nothing was changed on the way to
            // failing, so there is no list to put back and no dialog to reopen.
            Message::AiProviderError { provider_id, error } => {
                tracing::warn!("provider {provider_id:?} refused: {error}");
                self.workbench.settings.error = Some(error);
                cx.notify();
            }

            other => return Some(other),
        }
        None
    }

    /// One path in one project was created, moved, copied or removed.
    ///
    /// The request is echoed whole, because the answer arrives long after the click and what to do
    /// with it is the gesture's: a created file is opened, a moved one takes its tabs with it, a
    /// removed one closes them. The tabs are settled before anything is re-listed, so a tab on its
    /// way out is not re-read on the way past.
    fn path_edited(
        &mut self,
        project_id: ProjectId,
        rel_path: String,
        to: Option<String>,
        op: PathOp,
        cx: &mut Context<Self>,
    ) {
        let below = format!("{rel_path}/");
        let under = |path: &str| path == rel_path || path.starts_with(&below);

        match op {
            // A tab follows the file it is looking at: the bytes did not move, only the name did,
            // and re-reading would risk whatever has been typed since.
            PathOp::Move => {
                if let Some(to) = &to {
                    let moved: Vec<(String, String)> = self
                        .projects
                        .get(&project_id)
                        .map(|open| {
                            open.editor
                                .open
                                .iter()
                                .filter(|file| !file.guest && under(&file.path))
                                .map(|file| {
                                    (file.key(), format!("{to}{}", &file.path[rel_path.len()..]))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    for (key, path) in moved {
                        self.retarget_editor_tab(project_id, &key, &path, cx);
                    }
                }
            }
            // Nothing is left to draw, and a dirty tab is asked about rather than dropped — which
            // is `close_editor_tab`'s rule, kept here too.
            PathOp::Trash | PathOp::Delete => {
                if self.project(cx) == Some(project_id) {
                    let doomed: Vec<usize> = self
                        .projects
                        .get(&project_id)
                        .map(|open| {
                            (0..open.editor.open.len())
                                .filter(|&ix| under(&open.editor.open[ix].path))
                                .collect()
                        })
                        .unwrap_or_default();
                    self.close_editor_tabs_filtered(|ix, _| doomed.contains(&ix), cx);
                }
            }
            PathOp::Create { .. } | PathOp::Copy => {}
        }

        // What a Copy remembered is only worth remembering while it is still there.
        if let Some(open) = self.projects.get_mut(&project_id)
            && let Some(copied) = open.explorer.copied.as_deref()
            && matches!(op, PathOp::Move | PathOp::Trash | PathOp::Delete)
            && under(copied)
        {
            open.explorer.copied = None;
        }

        // The watch would get here on its own; asking now is what makes the gesture feel finished.
        // Only a folder the tree already holds, the same guard the watch's own handler uses: a
        // listing for one it does not know is thrown away by `merge` anyway.
        let mut dirs: Vec<String> = vec![explorer::parent_dir(&rel_path)];
        if let Some(to) = &to {
            let other = explorer::parent_dir(to);
            if !dirs.contains(&other) {
                dirs.push(other);
            }
        }
        for dir in dirs {
            let listed = self
                .projects
                .get(&project_id)
                .is_some_and(|open| open.explorer.is_folder_listed(&dir));
            if !listed {
                continue;
            }
            if let Some(open) = self.projects.get_mut(&project_id) {
                open.explorer.set_loading(&dir, true);
            }
            self.bus.send(Message::ProjectTree {
                project_id,
                rel_path: dir,
                depth: EXPAND_DEPTH,
            });
        }

        // A new file opens where the user made it. A new folder has nothing to open.
        if matches!(op, PathOp::Create { dir: false }) && self.project(cx) == Some(project_id) {
            self.select_file(rel_path, cx);
        }

        // An edit changes the working tree, the same as a save does.
        self.bus.send(Message::RefreshProjectGit {
            project_id,
            full: true,
        });
        cx.notify();
    }

    /// One path in one project failed.
    ///
    /// A tab waiting for bytes says why instead of sitting empty; a folder waiting for a listing
    /// stops spinning. A folder or a file that has gone is the cue to look at the project's own
    /// health again — the worker that answered does not know the catalogue and cannot say.
    fn file_failed(
        &mut self,
        project: ProjectId,
        rel_path: String,
        error: FileError,
        cx: &mut Context<Self>,
    ) {
        let reason = describe(&error);
        tracing::warn!("{project} {rel_path}: {reason}");

        if let Some(open) = self.projects.get_mut(&project) {
            open.explorer.set_loading(&rel_path, false);
            open.wanted.retain(|wanted| wanted != &rel_path);
            if let Some(file) = open.editor.find_mut(&rel_path) {
                match file.is_loading() {
                    // The read never landed, so the tab has nothing but the reason.
                    true => file.set_failed(reason.clone()),
                    // A write failed against a buffer the user still has: it is untouched, and
                    // still dirty.
                    false => file.save_failed(reason.clone()),
                }
            }
        }

        if matches!(error, FileError::Missing | FileError::Denied(_)) {
            self.bus.send(Message::RefreshProject {
                project_id: project,
            });
        }
        cx.notify();
    }

    /// A pane's harness has stopped, wherever the pane is: the tab's dot reports it.
    fn pane_stopped(&mut self, pane_id: PaneId) {
        for open in self.projects.values_mut() {
            if let Some(pane) = open.panes.iter_mut().find(|pane| pane.id == pane_id) {
                pane.running = false;
                return;
            }
        }
    }

    /// Whether the pane runs a tool with "wait on exit": the exit closes the process, not the tab.
    fn pane_wait_on_exit(&self, pane_id: PaneId) -> bool {
        self.projects
            .values()
            .flat_map(|open| open.panes.iter())
            .any(|pane| pane.id == pane_id && pane.wait_on_exit)
    }

    /// Draw a workspace the coordinator started: a tab, and an emulator on the pane's stream.
    ///
    /// The workspace names its project, which is what makes an answer that arrives after the user
    /// has switched projects land in the right place rather than on screen.
    /// Build the emulator for `pane_id` and register it, without deciding where it is drawn.
    ///
    /// This is everything a pane *is* on the interface's side: a byte stream in, a byte
    /// stream out, and a geometry callback that tells the host what the emulator measured.
    /// Where it appears is a separate question, which is why this is separate from
    /// [`open_pane`](Self::open_pane) — a login runs in a pane that belongs to no project and
    /// is drawn in a modal rather than the dock, and it needs all of this and none of that.
    pub(super) fn open_terminal(
        &mut self,
        pane_id: PaneId,
        cols: u16,
        rows: u16,
        font_size: f32,
        cx: &mut Context<Self>,
    ) {
        let (output, reader) = bus::pane_output();
        let writer = self.bus.input(pane_id);
        let config = ui::terminal::config(cols, rows, font_size);

        let to_host = self.bus.sender();
        let geometry = self.geometry.clone();
        let app = cx.entity().downgrade();
        let app_title = app.clone();
        let view = cx.new(|cx| {
            TerminalView::new(writer, reader, config, cx)
                .with_resize_callback(move |cols, rows| {
                    let (cols, rows) = (cols as u16, rows as u16);
                    to_host.send(Message::TerminalResize {
                        pane_id,
                        cols,
                        rows,
                    });
                    let _ = geometry.send((pane_id, cols, rows));
                })
                .with_key_handler(move |event, window, cx| {
                    if !is_terminal_defocus(&event.keystroke) {
                        return false;
                    }
                    window.blur(cx);
                    let _ = app.update(cx, |app, cx| app.blur_panes(cx));
                    true
                })
                .with_title_callback(move |_window, cx, title| {
                    let title = title.to_string();
                    let _ =
                        app_title.update(cx, |app, cx| app.pane_title_reported(pane_id, title, cx));
                })
        });

        self.terminals.insert(
            pane_id,
            PaneTerminal {
                view,
                output: Some(output),
            },
        );
    }

    fn open_pane(&mut self, workspace: WorkspaceInfo, cx: &mut Context<Self>) {
        let pane_id = workspace.id;
        let project = workspace.project_id;
        let taken: Vec<String> = self
            .projects
            .get(&project)
            .map(|open| open.panes.iter().map(|pane| pane.title.clone()).collect())
            .unwrap_or_default();
        let title = pane_title(&workspace.agent_type, &taken);

        // A pane for a project this window no longer holds has nowhere to be drawn, and a harness
        // nobody can see is a leak: it is closed rather than kept.
        if !self.projects.contains_key(&project) {
            tracing::info!("pane {pane_id} arrived for a project this window no longer holds");
            self.bus.send(Message::CloseWorkspace { pane_id });
            return;
        }
        let showing = self.project(cx) == Some(project);

        let term_font = self
            .projects
            .get(&project)
            .and_then(|open| open.prefs.content_font_size)
            .unwrap_or(theme::TERMINAL_FONT_SIZE);
        self.open_terminal(pane_id, workspace.cols, workspace.rows, term_font, cx);

        if let Some(open) = self.projects.get_mut(&project) {
            open.panes.push(PaneState {
                id: pane_id,
                harness: workspace.agent_type,
                rows: workspace.rows,
                cols: workspace.cols,
                title,
                running: workspace.running,
                wait_on_exit: workspace.wait_on_exit,
            });
            // A pane in a background project becomes that project's focused one only if it had
            // none: the keyboard belongs to whatever is on screen.
            if showing || open.focused_pane.is_none() {
                open.focused_pane = Some(pane_id);
            }
        }
        // The pane's panel joins the region terminals live in. It is queued rather than added
        // here: a panel reaches the dock through a `Window`, and a message does not come with one.
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::Terminal(pane_id)));

        if showing {
            self.pending_focus = Some(pane_id);
            self.bus.send(Message::Focus { pane_id });
        }
        cx.notify();
    }

    /// Give the keyboard to whoever asked for it. Focus needs a window, so it waits for one.
    ///
    /// Only a pane is ever owed it here. Every other panel takes the keyboard from the dock, which
    /// focuses whatever panel it has just displayed.
    pub(super) fn take_focus(&mut self, window: &mut Window, cx: &mut App) {
        let Some(pane_id) = self.pending_focus.take() else {
            return;
        };
        self.pending_editor_focus = None;
        if let Some(terminal) = self.terminals.get(&pane_id) {
            terminal
                .view
                .read(cx)
                .focus_handle()
                .clone()
                .focus(window, cx);
        }
    }

    /// Give the keyboard to the editor the last active file panel asked for.
    ///
    /// Focus needs a window, so this waits for the frame like [`Self::take_focus`] does — and it
    /// needs the file's buffer, which may still be arriving, so a file whose editor has no bytes
    /// yet keeps its turn until it does.
    pub(super) fn take_editor_focus(&mut self, window: &mut Window, cx: &mut App) {
        let Some(key) = self.pending_editor_focus.clone() else {
            return;
        };
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.active_file() else {
            return;
        };
        if file.key() != key {
            return;
        }
        // Nothing in the tab body to focus — Markdown/Mermaid showing only their preview,
        // Excalidraw's scene — hands the keyboard to the workbench root instead. Focusing the
        // buffer's `InputState` here anyway would leave the window's focus on a node this frame
        // never painted; GPUI's key dispatch then falls back to the window's own root, which
        // carries none of the app's key contexts, and the whole "Workbench" context — including
        // `CloseEditor` — goes unreachable until something else takes focus.
        if !file.viewer.shows_buffer(file.layout) {
            self.pending_editor_focus = None;
            self.workbench_focus.focus(window, cx);
            return;
        }
        let editor = match &file.body {
            crate::state::FileBody::Text { state, .. } => state.clone(),
            _ => return,
        };
        self.pending_editor_focus = None;
        editor.read(cx).focus_handle(cx).focus(window, cx);
    }

    // ── Workbench chrome ────────────────────────────────────────────
}
