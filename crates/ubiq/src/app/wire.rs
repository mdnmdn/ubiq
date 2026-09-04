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
    pub fn spawn_pane(
        &mut self,
        agent_type: Option<String>,
        args: Vec<String>,
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
        });
    }

    /// End a pane: the harness is killed, the emulator dropped, and the panel taken out of the
    /// dock.
    ///
    /// Idempotent, because the dock reaches it too — a panel whose tab was closed asks for this
    /// once it is sure it was closed rather than displaced. A pane the window has already let go
    /// of is not closed twice.
    pub fn close_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(project) = self.project_of_pane(pane_id) else {
            return;
        };
        self.bus.send(Message::CloseWorkspace { pane_id });
        self.terminals.remove(&pane_id);

        let showing = self.project(cx);
        // The keyboard only moves for the project on screen: a pane closed in a background
        // project must not take focus off the terminal the user is typing into.
        let on_screen = showing == Some(project);
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::Terminal(pane_id)));

        let mut next = None;
        if let Some(open) = self.projects.get_mut(&project) {
            open.panes.retain(|pane| pane.id != pane_id);
            if open.focused_pane == Some(pane_id) {
                next = open.panes.first().map(|pane| pane.id);
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

    /// Everything the coordinator says, in the order it said it.
    ///
    /// The families are disjoint, so each helper answers with the message back when it is
    /// none of its own and the next one is offered it.
    pub(super) fn receive(&mut self, message: Message, cx: &mut Context<Self>) {
        let Some(message) = self.receive_pane(message, cx) else {
            return;
        };
        let Some(message) = self.receive_project(message, cx) else {
            return;
        };
        let Some(message) = self.receive_file(message, cx) else {
            return;
        };
        let Some(message) = self.receive_git(message, cx) else {
            return;
        };
        let Some(message) = self.receive_work(message, cx) else {
            return;
        };
        let Some(message) = self.receive_conversation(message, cx) else {
            return;
        };
        let Some(message) = self.receive_session(message, cx) else {
            return;
        };
        let Some(message) = self.receive_account(message, cx) else {
            return;
        };
        let Some(message) = self.receive_search(message, cx) else {
            return;
        };
        // The rest are the window's own words, coming back the wrong way.
        tracing::warn!("the window was sent a message only it may send: {message:?}");
    }

    /// The pane family.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_pane(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
        match message {
            Message::WorkspaceSpawned { workspace } => self.open_pane(workspace, cx),

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
                if self.login_pane() == Some(pane_id) {
                    self.bus.send(Message::CloseWorkspace { pane_id });
                    return None;
                }
                if let Some(project_id) = project {
                    self.bus.send(Message::RefreshProjectGit {
                        project_id,
                        full: true,
                    });
                }
                self.close_pane(pane_id, cx);
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
    fn receive_project(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
        match message {
            Message::ProjectList { projects } => {
                cx.global_mut::<WindowRegistry>().replace_all(projects);
                self.adopt_if_owed(cx);
                // A catalogue that no longer names a project this window held takes it away, so
                // what the window holds is reconciled before anything is drawn from it.
                self.sync_projects(cx);
            }

            Message::ProjectAdded { project } => {
                let id = project.record.id;
                let root = project.record.path.clone();
                cx.global_mut::<WindowRegistry>().apply(project);
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
                cx.global_mut::<WindowRegistry>().apply(project);
                cx.notify();
            }

            Message::ProjectForgotten { project_id } => {
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
    fn receive_file(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
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
    fn receive_git(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
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
                        open.git = Some(next);
                    }
                }
                cx.notify();
            }

            Message::GitWorkingTree {
                project_id,
                generation,
                entries,
                rollups,
                truncated,
            } => {
                let open = self.projects.get_mut(&project_id)?;
                if !open.explorer.apply_git(generation, &entries, &rollups) {
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
    fn receive_work(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
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
                cx.notify();
            }

            Message::TaskCreated { project_id, task } => {
                self.workbench.work_error = None;
                let open = self.projects.get_mut(&project_id)?;
                let id = task.id;
                open.work.apply_task(task);
                open.graph
                    .layout
                    .place_new(&open.work.agents, &open.work.tasks);
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
                open.graph
                    .layout
                    .place_new(&open.work.agents, &open.work.tasks);
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
                open.graph
                    .layout
                    .place_new(&open.work.agents, &open.work.tasks);
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
                open.graph
                    .layout
                    .place_new(&open.work.agents, &open.work.tasks);
                let conversation = open
                    .conversations
                    .entry(id)
                    .or_insert_with(|| Conversation::new(id, harness, account));
                conversation.accepts_input = accepts_input;
                open.agents.prune(&open.work);
                // Unlike an agent that merely changed, this one was asked for: the user pressed
                // New agent a moment ago, so it comes on the field rather than onto the bench.
                open.agents.reveal(id);
                // And the chat panel shows it, for the same reason: whichever surface asked, what
                // was asked for is the conversation the user now wants to be looking at.
                self.chat.selected = Some(id);
                self.refill_columns = true;
                cx.notify();
            }

            Message::ConversationUpdate {
                agent_id,
                seq,
                update,
            } => {
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
                cx.notify();
                if let Some(queued) = next_prompt {
                    self.bus.send(Message::PromptAgent {
                        agent_id,
                        text: queued.text,
                    });
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

            other => return Some(other),
        }
        None
    }

    /// The session family: what the host is, and what can be started on it.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_session(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
        match message {
            // What the host is. The status bar says so when the root is not the usual one.
            Message::HostInfo {
                config_root,
                is_default,
            } => {
                self.workbench.config_root = Some(config_root);
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
                cx.notify();
            }

            // What can be started here. Replaced whole rather than merged: the host's answer is
            // the list, and a shell that has been uninstalled has to leave the menu.
            Message::ShellList { shells } => {
                self.workbench.shells = shells;
                cx.notify();
            }

            // Which agent harnesses can be started here. Replaced whole, same as the shell list:
            // a harness that has been uninstalled has to leave the menu, or read as unavailable.
            Message::AgentTypes { agent_types } => {
                self.workbench.agent_types = agent_types;
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
    fn receive_account(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
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
                self.workbench.settings.accounts = accounts;
                cx.notify();
            }
            Message::HarnessLoginStarted {
                pane_id,
                agent_type,
                account,
                cols,
                rows,
            } => {
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

            other => return Some(other),
        }
        None
    }

    /// The search family.
    ///
    /// Answers with the message when it belongs to another family.
    fn receive_search(&mut self, message: Message, cx: &mut Context<Self>) -> Option<Message> {
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
            .and_then(|open| open.prefs.ui_font_size)
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
        let editor = match &file.body {
            crate::state::FileBody::Text { state, .. } => state.clone(),
            _ => return,
        };
        self.pending_editor_focus = None;
        editor.read(cx).focus_handle(cx).focus(window, cx);
    }

    // ── Workbench chrome ────────────────────────────────────────────
}
