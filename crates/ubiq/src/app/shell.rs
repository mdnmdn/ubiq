use super::*;

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
    /// emulators dropped, and what it looked like is written down and parked.
    ///
    /// The panes have to go. No other window can adopt an emulator, and a pane runs in the
    /// project's folder — a harness left behind would be running somewhere nobody is looking.
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
        if self.active_seen == Some(project) {
            self.active_seen = None;
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
        self.sync_file_panels(project);
        self.sync_chat_panels(project);
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

    /// The tree the explorer draws, which belongs to the project it is showing.
    pub fn explorer(&self, cx: &App) -> Option<&ExplorerState> {
        self.open_project(cx).map(|open| &open.explorer)
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

    /// Which project a pane belongs to. A pane is only ever in one, so the first answer is the
    /// answer.
    pub(super) fn project_of_pane(&self, pane_id: PaneId) -> Option<ProjectId> {
        self.projects
            .iter()
            .find(|(_, open)| open.panes.iter().any(|pane| pane.id == pane_id))
            .map(|(id, _)| *id)
    }

    // ── Panes ───────────────────────────────────────────────────────

    pub fn set_rail_mode(&mut self, mode: RailMode, cx: &mut Context<Self>) {
        if mode == self.workbench.rail_mode {
            return;
        }
        // The window is leaving one mode and entering another, so the outgoing mode's arrangement
        // is written down first — the rest of this function is about the incoming one.
        self.remember_view(cx);
        self.workbench.rail_mode = mode;
        self.workbench.open_menu = None;

        if let Some(project) = self.project(cx)
            && let Some(open) = self.projects.get(&project)
        {
            let saved = open
                .prefs
                .modes
                .get(&mode)
                .cloned()
                .unwrap_or_else(|| prefs::ModeLayout::default_for(mode));
            // A saved arrangement restores whole, regions included. A mode never arranged has no
            // blob, so its defaults are forced directly: regions open or shut on the frame, the
            // tree left as the other mode had it.
            self.pending_layout = saved.layout.clone();
            self.pending_regions = saved.layout.is_none().then_some((
                saved.show_left,
                saved.show_bottom,
                saved.show_right,
            ));
            // Which mode the window is in is settled now, and is written down now rather than
            // waiting for the arrangement to change: two modes that arrange nothing between them
            // would otherwise leave the window reopening in the one it left. The arrangement
            // itself is not written here — this mode's has not been restored yet.
            if let Some(open) = self.projects.get_mut(&project) {
                open.prefs.rail_mode = mode;
            }
            self.store_prefs(project);
        }
        cx.notify();
    }

    pub fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let next = self.workbench.theme_id.toggled();
        self.workbench.theme_id = next;
        theme::set_mode(next, cx);
        // The palette belongs to the interface, not to any one project.
        self.remember_interface();
        // The emulator holds its own copy of the palette, so the switch has to reach it.
        let font = self.ui_font_size_or_default(cx);
        for terminal in self.terminals.values() {
            terminal.view.update(cx, |view, cx| {
                let (cols, rows) = view.dimensions();
                view.update_config(ui::terminal::config(cols as u16, rows as u16, font), cx);
            });
        }
        cx.notify();
    }

    pub fn open_menu(&mut self, menu: MenuId, cx: &mut Context<Self>) {
        if menu != MenuId::Explorer {
            self.drop_explorer_menu(cx);
        }
        self.workbench.open_menu = Some(menu);
        cx.notify();
    }

    pub fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.pending_close = None;
        self.workbench.file_tab_menu = None;
        self.workbench.new_pane_menu = None;
        self.workbench.conversation_menu = None;
        self.sink.settings.menu = None;
        self.drop_explorer_menu(cx);
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

    /// The point size the active project's text is drawn at — editors, terminal panes and the
    /// explorer tree together — or `None` for the interface default. `None` stays `None`, so each
    /// surface falls back to its own default rather than a value coalesced upstream.
    pub fn ui_font_size(&self, cx: &App) -> Option<f32> {
        let id = self.project(cx)?;
        self.projects
            .get(&id)
            .and_then(|open| open.prefs.ui_font_size)
    }

    /// The active project's text size as a live value, or the interface default when the project
    /// has not chosen one. This is the value the chrome mutates, so `None` is not allowed through to
    /// it.
    pub fn ui_font_size_or_default(&self, cx: &App) -> f32 {
        self.ui_font_size(cx).unwrap_or(theme::EDITOR_FONT_SIZE)
    }

    /// Set the active project's text size outright — the status bar's dropdown hands a choice in,
    /// rather than a nudge — within the range the chrome admits, and write it down as the
    /// project's own. Emulators already open are dressed to match, since a zoom has to reach a
    /// pane that is on screen, not wait for a restart.
    pub fn set_ui_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let Some(id) = self.project(cx) else {
            return;
        };
        let size = size.clamp(theme::EDITOR_FONT_MIN, theme::EDITOR_FONT_MAX);
        if let Some(open) = self.projects.get_mut(&id) {
            open.prefs.ui_font_size = Some(size);
        }
        let panes: Vec<_> = self
            .projects
            .get(&id)
            .into_iter()
            .flat_map(|open| open.panes.iter())
            .filter_map(|pane| self.terminals.get(&pane.id).map(|t| &t.view))
            .cloned()
            .collect();
        for view in panes {
            view.update(cx, |view, cx| {
                let (cols, rows) = view.dimensions();
                view.update_config(ui::terminal::config(cols as u16, rows as u16, size), cx);
            });
        }
        self.schedule_markdown_reflow(cx);
        self.remember(id, cx);
        cx.notify();
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

    /// Nudge the active project's text size up or down by one point, within the range the chrome
    /// admits, and write the result down as the project's own. A zoom is a preference of the
    /// project, so it travels with a project and survives a restart, and it dresses the editor,
    /// the terminal panes and the explorer tree together.
    pub fn nudge_ui_font_size(&mut self, direction: i8, cx: &mut Context<Self>) {
        let current = self.ui_font_size_or_default(cx);
        self.set_ui_font_size(
            (current + direction as f32).clamp(theme::EDITOR_FONT_MIN, theme::EDITOR_FONT_MAX),
            cx,
        );
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
        // The keyboard a file panel asked for waits for its buffer, which `attach_arrived_files`
        // may have just delivered in this same frame — so the editor is focused after it, not
        // before.
        self.take_editor_focus(window, cx);
        self.fill_task_form(window, cx);
        self.fill_columns(window, cx);
        self.fill_project_form(window, cx);
        self.settle_graph(cx);
        self.settle_board(cx);
        self.settle_tab_drag(cx);
        // The filter field is one per window; the project on screen's filter is the window's habit
        // from the frame after the project swings in. Cheap when nothing changed, and it never
        // fights a query being typed.
        self.sync_file_filter_field(window, cx);
        self.sync_git_fields(window, cx);
        self.sync_search_settings_fields(window, cx);
        // Made anonymous straight away so the frame stops borrowing the window: the queue below
        // is drained on the same `&mut self` the tree was built from.
        let tree = ui::shell::render(self, window, cx).into_any_element();
        // Diagrams a viewer found it needed while the tree was being built. Started here, where
        // the update the frame was built inside is done with `AppState` — never from inside one,
        // and never on this thread.
        self.drain_diagram_asks(cx);
        tree
    }
}
