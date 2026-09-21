use super::*;

impl AppState {
    /// The window's arrangement, for the one module that draws it.
    pub fn dock(&self) -> &Entity<DockArea> {
        &self.dock
    }

    /// Which of the three edge regions are on screen, for the titlebar's switches. The dock is
    /// asked rather than a flag beside it, so a region the user emptied reads as closed here too.
    pub fn regions_open(&self, cx: &App) -> (bool, bool, bool) {
        let dock = self.dock.read(cx);
        (
            dock.is_dock_open(dock::placement_of(Region::Left)),
            dock.is_dock_open(dock::placement_of(Region::Bottom)),
            dock.is_dock_open(dock::placement_of(Region::Right)),
        )
    }

    /// Whether the arrangement already holds a panel of one kind, wherever it sits and whether or
    /// not the region it sits in is on screen. A kind this window has never built a panel for is
    /// not in the tree by definition.
    pub(super) fn holds_kind(&self, kind: &PanelKind, cx: &mut App) -> bool {
        self.panels
            .get(kind)
            .cloned()
            .is_some_and(|panel| dock::holds(&self.dock.clone(), &panel, cx))
    }

    /// The name the user has typed over a tab's own, if any. `WorkbenchPanel::tab` reads this to
    /// override the label it would otherwise compute.
    pub fn tab_name(&self, kind: &PanelKind) -> Option<SharedString> {
        self.tab_names.get(kind).cloned()
    }

    /// Whether a tab is pinned — protected from close. A file's lives on the file itself, since
    /// it is the one pin that is written down; a terminal or a chat tab's is `pinned_tabs`.
    pub fn tab_pinned(&self, kind: &PanelKind, cx: &App) -> bool {
        match kind {
            PanelKind::File(key) => self.file(key, cx).is_some_and(|file| file.pinned),
            PanelKind::Kb(key) => self.kb_doc(key, cx).is_some_and(|doc| doc.pinned),
            _ => self.pinned_tabs.contains(kind),
        }
    }

    /// Whether a tab's right-click menu offers Restart: a terminal whose pane was started by a
    /// configured tool, whose command has since ended.
    ///
    /// A pane whose command is still running has nothing to restart — Close is the row that ends
    /// it — and a shell or a harness pane has no tool to run again. Only a tool with "wait on
    /// exit" is ever both stopped and still on screen, which is exactly the case this answers to.
    pub fn tab_restartable(&self, kind: &PanelKind, _cx: &App) -> bool {
        match kind {
            PanelKind::Terminal(pane_id) => self
                .pane(*pane_id)
                .is_some_and(|pane| pane.tool.is_some() && !pane.running),
            _ => false,
        }
    }

    /// Flip whether a tab is pinned, the rename menu's Pin/Unpin row.
    pub fn toggle_tab_pin(&mut self, kind: PanelKind, cx: &mut Context<Self>) {
        match &kind {
            PanelKind::File(key) => {
                let Some(project) = self.project(cx) else {
                    return;
                };
                let Some(open) = self.projects.get_mut(&project) else {
                    return;
                };
                let Some(file) = open.editor.find_key_mut(key) else {
                    return;
                };
                file.pinned = !file.pinned;
                self.remember(project, cx);
            }
            PanelKind::Kb(key) => {
                let Some(project) = self.project(cx) else {
                    return;
                };
                let Some(open) = self.projects.get_mut(&project) else {
                    return;
                };
                let Some(doc) = open.kb.doc_mut(key) else {
                    return;
                };
                doc.pinned = !doc.pinned;
            }
            _ => {
                if !self.pinned_tabs.remove(&kind) {
                    self.pinned_tabs.insert(kind);
                }
            }
        }
        cx.notify();
    }

    /// Whether a tab group is one of the pane region's.
    ///
    /// The new-pane control has to stay on the strip of a region the user has emptied, and the
    /// skin that draws it is handed a node and knows nothing about placement — so the window,
    /// which holds the dock, answers.
    pub fn is_pane_region(&self, node: gpui_component::dock::NodeId, cx: &App) -> bool {
        self.dock
            .read(cx)
            .layout(dock::placement_of(Region::Bottom))
            .is_some_and(|tree| tree.node_ids().contains(&node))
    }

    /// Put a region away, or bring it back. The dock remembers the size either way, which is what
    /// makes a toggle non-destructive.
    ///
    /// **Opening a region with nothing in it fills it**, and this is the one place that happens:
    /// a click on the region's switch is the user asking to see something there, where a mode or
    /// project switch is not (`D156`). The bottom exists to hold panes and opens onto a fresh one.
    /// The right in IDE opens onto a chat tab; in Git onto the changes panel; in Tasks onto the
    /// task. The left opens onto that mode's explorer — the IDE's, Git's refs, the knowledge
    /// base's, the agents list — and a mode with no left-hand furniture is left be.
    pub fn toggle_region(&mut self, region: Region, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |dock, cx| {
            dock.toggle_dock(dock::placement_of(region), window, cx);
        });
        let placement = dock::placement_of(region);
        let now_open = self.dock.read(cx).is_dock_open(placement);
        let now_empty = {
            let dock = self.dock.read(cx);
            now_open && dock.is_empty(placement, cx)
        };
        if now_empty {
            let mode = self.workbench.rail_mode;
            // What a side region opened onto nothing fills with is the mode's own furniture where
            // it has some — and a fresh chat tab where the side is the one the conversation calls
            // home. A side that is neither is the user having dragged its panel away on purpose,
            // and the switch leaves it be.
            let furniture = mode_side_furniture(mode, region);
            // Whether that furniture is already in the tree, somewhere the user dragged it to.
            // Read before the match, because there is nothing to put here in that case and the
            // toggle has to be taken back rather than answered.
            let elsewhere = furniture
                .clone()
                .is_some_and(|kind| self.holds_kind(&kind, cx));
            match region {
                Region::Bottom => self.spawn_pane(None, Vec::new(), AgentPicks::default(), cx),
                Region::Centre => {}
                // The mode's furniture is on the other edge: `settle_panels` skips a kind the tree
                // already holds, so asking for it would leave an empty bar the dock writes down as
                // the user's own choice. There is nothing this side can show, so the switch does
                // nothing. A mode with no furniture for the side promised nothing and keeps the
                // edge the user asked for — see [`mode_side_furniture`].
                _ if elsewhere => {
                    self.dock.update(cx, |dock, cx| {
                        dock.toggle_dock(placement, window, cx);
                    });
                }
                _ => match furniture {
                    Some(kind) => self.pending_panels.push(PanelEdit::Open(kind)),
                    // The one place the window has to decide *which* chat tab an empty region
                    // opens onto: a fresh one, attached to nothing.
                    None if region == PanelKind::chat_home(mode) => {
                        if let Some(id) = self.open_chat_tab(cx) {
                            self.pending_panels
                                .push(PanelEdit::Open(PanelKind::Chat(id)));
                        }
                    }
                    None => {}
                },
            }
        }
        cx.notify();
    }

    /// The titlebar's "New terminal" shortcut: bring the bottom region on screen if the user had
    /// put it away, then start a fresh pane in it.
    ///
    /// `toggle_region` already starts a pane on its own when opening the bottom onto nothing — the
    /// case where the region was closed with no panels left in it — so that case is left to it
    /// rather than spawning a second pane on top. Every other case — already open, or reopened onto
    /// panes still in it — has no pane of `toggle_region`'s own coming, so this spawns the one the
    /// shortcut promised.
    pub fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placement = dock::placement_of(Region::Bottom);
        let (was_open, was_empty) = {
            let dock = self.dock.read(cx);
            (dock.is_dock_open(placement), dock.is_empty(placement, cx))
        };
        if !was_open {
            self.toggle_region(Region::Bottom, window, cx);
        }
        if was_open || !was_empty {
            self.spawn_pane(None, Vec::new(), AgentPicks::default(), cx);
        }
    }

    /// Make sure a pane about to be started has somewhere on screen to land: the IDE if the mode
    /// the window is in has no pane region at all, and the bottom region opened if it is shut.
    ///
    /// **Not `toggle_region`.** That fills an empty bottom with a fresh shell, which is exactly
    /// the wrong thing here — the pane the caller is starting is what fills it, and a shell
    /// beside it is a pane nobody asked for. So the dock is opened directly. A region opened
    /// empty is not closed again by `hide_emptied_regions`: that only fires on a region that
    /// went from holding something to holding nothing.
    /// It is **asked for rather than done here**, because everything that could shut the region
    /// again runs later in the same frame: a mode switch forces the incoming mode's own regions
    /// in `settle_mode`, and a pane closed on the way out empties the region, which
    /// `hide_emptied_regions` closes. So the ask is queued and [`Self::settle_pane_region`]
    /// answers it at the end of the frame, when nothing is left to overrule it.
    pub fn reveal_pane_region(&mut self, cx: &mut Context<Self>) {
        if !self.workbench.rail_mode.has_pane_region() {
            // The window moving itself, not the user moving it: the furniture sweep stays out of
            // it, or starting a pane from Control would take the console with it (`D156`).
            self.enter_rail_mode(RailMode::Ide, false, cx);
        }
        self.pending_pane_region = true;
        cx.notify();
    }

    /// Answer a queued [`Self::reveal_pane_region`]: open the bottom region if it is still shut.
    pub(super) fn settle_pane_region(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.pending_pane_region) {
            return;
        }
        let placement = dock::placement_of(Region::Bottom);
        if self.dock.read(cx).is_dock_open(placement) {
            return;
        }
        self.dock.update(cx, |dock, cx| {
            dock.toggle_dock(placement, window, cx);
        });
        cx.notify();
    }

    /// Close a region the user just emptied — by closing its last panel or dragging it elsewhere —
    /// rather than leaving a bar with nothing in it on screen.
    ///
    /// Runs after every dock layout change, so it has to tell that apart from a region a caller
    /// just opened empty on purpose: [`Self::region_had_content`] is the edge, ticked here, that
    /// only fires once a region goes from holding something to holding nothing.
    pub(super) fn hide_emptied_regions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut close = Vec::new();
        {
            let dock = self.dock.read(cx);
            let (had_left, had_bottom, had_right) = self.region_had_content;
            let mut check = |region: Region, had: bool| {
                let placement = dock::placement_of(region);
                let open = dock.is_dock_open(placement);
                let empty = dock.is_empty(placement, cx);
                if open && empty && had {
                    close.push(region);
                }
                open && !empty
            };
            self.region_had_content = (
                check(Region::Left, had_left),
                check(Region::Bottom, had_bottom),
                check(Region::Right, had_right),
            );
        }
        if close.is_empty() {
            return;
        }
        self.dock.update(cx, |dock, cx| {
            for region in close {
                dock.toggle_dock(dock::placement_of(region), window, cx);
            }
        });
    }

    /// Collapse a region a whole arrangement was just installed over, if it landed empty.
    ///
    /// [`Self::hide_emptied_regions`] only fires on the edge — a region that *went* from holding
    /// something to holding nothing — because a caller may open one empty on purpose and fill it a
    /// frame later. A restore is not that: the arrangement it installed is final, so an open region
    /// with nothing in it is one the project on screen has no use for. Arming the edge is the whole
    /// of it: the next pass reads the region as having just been emptied and puts it away.
    ///
    /// **A region a queued panel is about to land in is not empty, only a step early.**
    /// `settle_panels` drains that queue later in the same frame, so the edge is left unarmed for
    /// those regions — otherwise entering Git, whose refs and changes are queued by
    /// [`Self::queue_git_furniture`] rather than named by a blob, opens its two side regions and
    /// then puts them away before their panels arrive.
    pub(super) fn collapse_empty_regions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let incoming = self.incoming_regions();
        self.region_had_content = (
            !incoming.contains(&Region::Left),
            !incoming.contains(&Region::Bottom),
            !incoming.contains(&Region::Right),
        );
        self.hide_emptied_regions(window, cx);
    }

    /// The regions the queued panel edits are about to put something in.
    fn incoming_regions(&self) -> Vec<Region> {
        self.pending_panels
            .iter()
            .filter_map(|edit| match edit {
                PanelEdit::Open(kind) | PanelEdit::Reveal(kind) => {
                    Some(kind.home_in(self.workbench.rail_mode))
                }
                PanelEdit::Close(_) => None,
            })
            .collect()
    }

    /// Queue the Git screen's own panels into their home regions.
    ///
    /// A mode never arranged keeps the tree the last mode had, so the refs and the changes have
    /// to be asked for or the opened left and right regions would show nothing of this screen.
    /// `settle_panels` skips a kind the tree already holds.
    pub(super) fn queue_git_furniture(&mut self) {
        for kind in [
            PanelKind::GitRefs,
            PanelKind::GitChanges,
            PanelKind::GitHistory,
        ] {
            self.pending_panels.push(PanelEdit::Open(kind));
        }
        // The commit list is the centre of the Git screen.
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::GitHistory));
    }

    /// Queue the KB screen's own panel into its home region — the left-edge explorer, on
    /// [`Self::queue_git_furniture`]'s reasoning: a mode never arranged keeps the tree the last
    /// mode had, and the explorer is not named by a blob until one exists.
    pub(super) fn queue_kb_furniture(&mut self) {
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::KbExplorer));
    }

    /// The same for the three other screens with a side panel of their own: the IDE's file
    /// explorer on the left, the board's task on the right, and the agents list on the left. One
    /// kind each, for [`Self::queue_kb_furniture`]'s reason — a first visit to the mode has no
    /// blob naming it.
    ///
    /// **The IDE's explorer is the one that is not optional.** It is never closable and it is how
    /// the files are reached, so a visit to the IDE that no blob arranges asks for it rather than
    /// leaving the left region empty. Asking is all it is: `settle_panels` skips a kind the tree
    /// already holds, so an explorer the user has dragged to another region stays where it was put.
    pub(super) fn queue_mode_furniture(&mut self, mode: RailMode) {
        let kind = match mode {
            RailMode::Ide => PanelKind::Explorer,
            RailMode::Tasks => PanelKind::Task,
            RailMode::Agents => PanelKind::AgentsExplorer,
            _ => return,
        };
        self.pending_panels.push(PanelEdit::Open(kind));
    }

    /// The panel for one kind, built the first time it is asked for.
    pub(super) fn panel(&mut self, kind: PanelKind, cx: &mut App) -> Entity<WorkbenchPanel> {
        if let Some(panel) = self.panels.get(&kind) {
            return panel.clone();
        }
        let panel = WorkbenchPanel::new(kind.clone(), self.this.clone(), cx);
        self.panels.insert(kind, panel.clone());
        panel
    }

    /// Make the dock's file panels the files of one project.
    ///
    /// **The open files are a project's, and the panels are the window's**, so the two have to be
    /// squared whenever the window changes which project it is pointed at. A saved arrangement
    /// usually carries the incoming project's own file panels and these edits are then no-ops;
    /// a project that has never been written down has none, and this is what gives it them.
    pub(super) fn sync_file_panels(&mut self, project: ProjectId) {
        let wanted: Vec<String> = self
            .projects
            .get(&project)
            .map(|open| open.editor.open.iter().map(|file| file.key()).collect())
            .unwrap_or_default();

        for kind in self.panels.keys() {
            if let Some(key) = kind.tab_key()
                && !wanted.iter().any(|open| open == key)
            {
                self.pending_panels.push(PanelEdit::Close(kind.clone()));
            }
        }
        for key in wanted {
            let kind = PanelKind::File(key);
            if !self.panels.contains_key(&kind) {
                self.pending_panels.push(PanelEdit::Open(kind));
            }
        }

        // The knowledge base's documents are the same bargain one mode along: the documents are
        // a project's and the panels are the window's, so a switch has to square them or the
        // documents of the project just left would keep their tabs.
        let docs: Vec<String> = self
            .projects
            .get(&project)
            .map(|open| open.kb.docs.iter().map(|doc| doc.key()).collect())
            .unwrap_or_default();
        for kind in self.panels.keys() {
            if let Some(key) = kind.kb_key()
                && !docs.iter().any(|open| open == key)
            {
                self.pending_panels.push(PanelEdit::Close(kind.clone()));
            }
        }
        for key in docs {
            let kind = PanelKind::Kb(key);
            if !self.panels.contains_key(&kind) {
                self.pending_panels.push(PanelEdit::Open(kind));
            }
        }
    }

    /// Make the dock's chat panels the tabs of one project, the same way [`Self::sync_file_panels`]
    /// squares the file panels.
    ///
    /// **This is a chat tab's real population**, not `dock::restore` or `dock::default_layout` —
    /// both of those only ever draw scaffolding or reattach an id this window already held, since
    /// a chat id is minted fresh every process and never the host's to confirm. Called whenever a
    /// project is entered — including the first time, right after [`OpenProject::new`] has seeded
    /// its one default tab — and again at the end of [`Self::settle_layout`], so a restore that
    /// dropped an unknown id or fell back to scaffolding is squared with the truth immediately
    /// rather than left a frame behind.
    pub(super) fn sync_chat_panels(&mut self, project: ProjectId) {
        let wanted: Vec<ChatId> = self
            .projects
            .get(&project)
            .map(|open| open.chats.iter().map(|tab| tab.id).collect())
            .unwrap_or_default();

        for kind in self.panels.keys() {
            if let Some(id) = kind.chat_id()
                && !wanted.contains(&id)
            {
                self.pending_panels.push(PanelEdit::Close(kind.clone()));
            }
        }
        for id in wanted {
            let kind = PanelKind::Chat(id);
            if !self.panels.contains_key(&kind) {
                self.pending_panels.push(PanelEdit::Open(kind));
            }
        }
    }

    /// Put the panels that arrived on a message into the dock, and take out the ones that left.
    ///
    /// Drained in `render`, which is the same device the pending focus and the arrived files use,
    /// and for the same reason: both halves of a panel's life need a window.
    ///
    /// **Nothing is added here on the strength of a state flag.** This used to put the search panel
    /// back whenever `search.active` was set, which made an unfinished search — one the window
    /// switched project or mode away from — re-add the panel on the next frame IDE was on screen,
    /// over whatever arrangement the user had left. A search's *only* way into the dock is
    /// [`Self::reveal_search`], which the titlebar's icon, the ⌘F binding and
    /// [`Self::search_for`] all go through and which adds the panel if the tree does not hold it —
    /// so the guarantee the flag was standing in for is already the gesture's, and the panel's
    /// presence is the arrangement's business alone.
    pub(super) fn settle_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The outline follows the tab on screen, and a tab switch does not pass through here on a
        // message — it is noticed in the frame that draws the new one.
        self.settle_outline(cx);
        if self.pending_panels.is_empty() {
            return;
        }
        for edit in std::mem::take(&mut self.pending_panels) {
            match edit {
                PanelEdit::Open(kind) => {
                    // An unattached chat tab is only worth a panel in a region that is asking to
                    // be filled: a project is seeded with one empty tab (`seeded_chats`), and
                    // adding it here is what used to leave an empty agent panel in the right
                    // region and open the region onto it. A tab with an agent behind it is not
                    // idle and passes — which is how a persistent agent's claim gets its panel
                    // without a `Reveal` and without the region moving
                    // (`AppState::settle_persistent_chat`). Only `Open` is filtered at all: a
                    // `Reveal` is the user asking for a chat by name.
                    if self.is_idle_chat(&kind, cx) {
                        continue;
                    }
                    let home = kind.home_in(self.workbench.rail_mode);
                    let panel = self.panel(kind, cx);
                    // A saved arrangement is rebuilt before this queue is drained, so a file panel
                    // can already be in the tree by the time the edit that asked for it is read.
                    // Adding it twice would be two tabs on one file.
                    if dock::holds(&self.dock.clone(), &panel, cx) {
                        continue;
                    }
                    dock::add(&self.dock.clone(), &panel, home, window, cx);
                }
                PanelEdit::Reveal(kind) => {
                    let home = kind.home_in(self.workbench.rail_mode);
                    let panel = self.panel(kind, cx);
                    dock::reveal(&self.dock.clone(), &panel, home, window, cx);
                }
                PanelEdit::Close(kind) => {
                    if let Some(panel) = self.panels.remove(&kind) {
                        dock::remove(&self.dock.clone(), &panel, window, cx);
                    }
                }
            }
        }
    }

    /// Whether a panel kind is a chat tab attached to nothing, bound for a right region that is
    /// not asking for one — anything but open and empty.
    ///
    /// A tab with no conversation behind it is the empty agent panel, and the only region worth
    /// putting one in is one the user has just opened onto nothing: that is
    /// [`Self::toggle_region`]'s gesture, and the tab is the answer to it. **A region already
    /// holding something is not asking** (`D156`): [`Self::sync_chat_panels`] queues an `Open` for
    /// every tab the project has whenever a project or a mode settles, and letting the seeded tab
    /// through wherever the right region happened to be open is what put a chat beside Git's
    /// changes panel that nobody opened there and then wrote it into Git's blob.
    ///
    /// A tab with an agent behind it is not idle and is worth a panel wherever it lands, and a
    /// `Reveal` — every gesture that makes or picks a chat — does not come through here at all.
    fn is_idle_chat(&self, kind: &PanelKind, cx: &App) -> bool {
        let Some(id) = kind.chat_id() else {
            return false;
        };
        let attached = self
            .open_project(cx)
            .and_then(|open| open.chats.iter().find(|tab| tab.id == id))
            .is_some_and(|tab| tab.attached.is_some());
        if attached {
            return false;
        }
        let placement = dock::placement_of(PanelKind::chat_home(self.workbench.rail_mode));
        let dock = self.dock.read(cx);
        !(dock.is_dock_open(placement) && dock.is_empty(placement, cx))
    }

    /// Put a rail mode's regions where that mode wants them, on the frame after the switch.
    ///
    /// A mode with a saved arrangement is restored whole by [`Self::settle_layout`] and never
    /// reaches here. A mode with none got its defaults forced instead: the region a mode's default
    /// says is off screen is put away, a region it says is on screen is brought back. Idempotent —
    /// a region already where the mode wants it is not toggled.
    pub(super) fn settle_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((left, bottom, right)) = self.pending_regions.take() else {
            return;
        };
        let dock = self.dock.clone();
        dock.update(cx, |dock, cx| {
            for (region, want_open) in [
                (Region::Left, left),
                (Region::Bottom, bottom),
                (Region::Right, right),
            ] {
                let placement = dock::placement_of(region);
                if want_open != dock.is_dock_open(placement) {
                    dock.toggle_dock(placement, window, cx);
                }
            }
        });
    }

    /// Take the window's own furniture out of the tree, on a switch of project or of mode.
    ///
    /// **Search and the log are asked for, never inherited** (`D156`): both are opened by a
    /// gesture, for one project on one screen, and a panel that followed the switch would be a
    /// panel the next thing the dock says writes down as the target's own. Help is the one that
    /// travels, and only in follow mode — the panel whose whole contract is to keep up with where
    /// the reader is standing. A mode that had any of the three open when it was left named them
    /// in its own blob, and the restore below puts them back; that is the only way they come.
    ///
    /// **Follow mode's help is the one panel a switch puts back on screen**, and it is the one
    /// exception (`D156`). Leaving it in the tree is not enough: the arrangement landing over it
    /// may have the region it sits in shut, and a leftover is added rather than revealed — so help
    /// would be in the tree, off screen, and written into the incoming mode's blob that way. A
    /// panel whose whole contract is to keep up with the reader is a user gesture deferred, so it
    /// is revealed. Queued rather than done here, because [`Self::settle_panels`] drains the queue
    /// after the restore and after the regions it settled on have been forced back.
    fn sweep_furniture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut sweep = vec![PanelKind::Search, PanelKind::Logs];
        if self.help.follow {
            // Only a panel that was on screen travels. Help the reader never opened, or closed,
            // is not brought up by a mode switch.
            if self.holds_kind(&PanelKind::Help, cx) {
                self.pending_panels.push(PanelEdit::Reveal(PanelKind::Help));
            }
        } else {
            sweep.push(PanelKind::Help);
        }
        for kind in sweep {
            if let Some(panel) = self.panels.remove(&kind) {
                dock::remove(&self.dock.clone(), &panel, window, cx);
            }
        }
    }

    /// Rebuild a saved arrangement, on the frame after it arrives.
    ///
    /// A layout this build cannot use — a stale version, or one whose panels it has all lost — is
    /// discarded for the arrangement a fresh window opens in, rather than half-applied.
    pub(super) fn settle_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The window furniture goes before anything is installed over it — see `reset_furniture`.
        let reset = std::mem::take(&mut self.reset_furniture);
        if reset {
            self.sweep_furniture(window, cx);
        }
        let Some(saved) = self.pending_layout.take() else {
            // A project with nothing written down keeps the tree it arrived on, so the region the
            // furniture just left has to be put away rather than left as an empty bar.
            if reset {
                self.collapse_empty_regions(window, cx);
                self.note_settled(cx);
            }
            return;
        };
        let dock = self.dock.clone();
        let panels = std::mem::take(&mut self.panels);
        // Which of them are on screen *now*, read before the restore rearranges the tree. A blob
        // written before a panel was revealed cannot name it, and dropping it is the panel
        // vanishing a frame after the user asked for it.
        let on_screen: Vec<PanelKind> = panels
            .iter()
            .filter(|(_, panel)| dock::holds(&dock, panel, cx))
            .map(|(kind, _)| kind.clone())
            .collect();
        let app = self.this.clone();
        let mut kept = HashMap::new();
        let mut layouts: Vec<(String, ViewLayout)> = Vec::new();
        {
            let mut build = |kind: PanelKind, cx: &mut App| {
                // **A terminal or a chat panel is never built here unless it is already known.**
                // A saved leaf names a pane or a chat id, and neither is this window's to trust on
                // its say-so alone: a pane exists because the coordinator says so, and a chat id
                // is minted fresh every process — so one this window did not already hold, this
                // run or the last, is dropped rather than rebuilt from a stale name. A chat tab's
                // *real* population is `Self::sync_chat_panels`, driven by `OpenProject::chats`,
                // not this restore.
                if (kind.pane().is_some() || kind.chat_id().is_some())
                    && !panels.contains_key(&kind)
                    && !kept.contains_key(&kind)
                {
                    return None;
                }
                Some(
                    kept.entry(kind.clone())
                        .or_insert_with(|| {
                            panels
                                .get(&kind)
                                .cloned()
                                .unwrap_or_else(|| WorkbenchPanel::new(kind, app.clone(), cx))
                        })
                        .clone(),
                )
            };
            if !dock::restore(&dock, &saved, &mut build, &mut layouts, window, cx) {
                dock::default_layout(&dock, &mut build, window, cx, self.workbench.rail_mode);
            }
        }
        // A panel the window holds that the restored arrangement does not name is put back in its
        // home region rather than dropped: a pane spawned in another mode, a file opened while a
        // different arrangement was on screen, or anything at all when the blob was discarded and
        // the default arrangement was installed over it. Losing one would leave a live pane or an
        // open tab with nothing drawing it.
        //
        // A panel that was on screen when this ran keeps its place for the same reason one frame
        // later: the blob predates it. A panel the user closed is not on screen and does not come
        // back — closing is what took it out of the tree.
        //
        // **A leftover is added, never revealed** (`D156`). A reveal opens the region it lands in,
        // and the arrangement just installed is the one the user left this mode or this project
        // in: a switch that opened an edge for a panel the blob does not name would be the window
        // rearranging itself, and the dock's own `LayoutChanged` would then write that
        // rearrangement down as the user's. Which regions are on screen is read here and forced
        // back below, because adding to a region no blob named makes one, and a made one is open.
        let open_before = self.regions_open(cx);
        for (kind, panel) in panels {
            if kept.contains_key(&kind) {
                continue;
            }
            // **A chat tab is placed by the user, never by a switch** (`D156`) — the same rule
            // search and the log answer to, one kind along. A chat is drawn in every mode about a
            // project, so a tab open in the IDE is a leftover everywhere; adding it would put a
            // conversation beside Git's changes that nobody opened there, and the dock's own
            // `LayoutChanged` would write it into Git's blob as the user's own.
            //
            // The **panel is kept and only the placement dropped**: the entity is the one thing a
            // rebuild cannot make again — a chat id is minted fresh every process, so a tree this
            // window no longer holds the panel for is a tab the mode's own blob can never bring
            // back — and `Self::sync_chat_panels` reads the same map and would queue an `Open`
            // for a kind missing from it, putting the tab back in by the other door.
            if kind.chat_id().is_some() {
                kept.insert(kind, panel);
                continue;
            }
            // A mode's own panels belong to that mode's blob. Putting one back here would leave a
            // panel this mode hides in this mode's tree.
            let restorable = !kind.is_mode_owned()
                && (kind.pane().is_some() || kind.tab_key().is_some() || on_screen.contains(&kind));
            if restorable {
                let home = kind.home_in(self.workbench.rail_mode);
                dock::add(&dock, &panel, home, window, cx);
                kept.insert(kind, panel);
            }
        }
        self.panels = kept;
        // The regions the restore settled on, put back where an add made one.
        let (left, bottom, right) = open_before;
        self.dock.update(cx, |dock, cx| {
            for (region, open) in [
                (Region::Left, left),
                (Region::Bottom, bottom),
                (Region::Right, right),
            ] {
                let placement = dock::placement_of(region);
                if !open && dock.is_dock_open(placement) {
                    dock.toggle_dock(placement, window, cx);
                }
            }
        });
        // A file panel's payload carries the layout its viewer was left in, which belongs on the
        // file rather than on the panel: the panel only repeats it, the way it repeats visibility.
        if let Some(project) = self.project(cx) {
            if let Some(open) = self.projects.get_mut(&project) {
                for (key, layout) in layouts {
                    if let Some(file) = open.editor.open.iter_mut().find(|file| file.key() == key) {
                        file.set_layout(layout);
                    }
                }
            }
            // A chat leaf this rebuild dropped for naming an id the window did not already hold
            // (or a fallback default's own throwaway scaffold, when the restore failed outright)
            // is cleaned up the same way `Self::enter_project` gets the first one in: by squaring
            // the tree with `OpenProject::chats`, which is the tab's actual source of truth.
            self.sync_chat_panels(project);
        }
        self.collapse_empty_regions(window, cx);
        self.note_settled(cx);
        cx.notify();
    }

    /// Remember the arrangement this settle ended on, so the `LayoutChanged` it is about to fire
    /// is not written back over the mode's blob as the user's own choice (`D156`).
    ///
    /// **A settle rearranges the window without being asked to**: it installs a blob, puts
    /// leftovers back, and collapses a region that landed empty. The dock says so the way it says
    /// anything, and the subscription that hears it writes the arrangement down — so a mode whose
    /// blob names a region the settle then found empty has that region rewritten as shut, which is
    /// the write-back this card exists to stop. The blob is left saying what the user left it
    /// saying instead, and is repaired the next time they arrange anything.
    ///
    /// Comparing the arrangement is the whole of the mechanism, and it has to be: the dock's event
    /// is delivered after this frame's update has finished, so a flag set across the settle is
    /// already cleared by the time anything could read it. An arrangement still exactly the one
    /// the settle installed is the settle's own echo; anything else is a real edit.
    fn note_settled(&mut self, cx: &App) {
        self.settled_layout = Some(self.current_layout(cx));
    }

    /// The arrangement on screen, shaped as [`AppState::remember`] would write it down.
    pub(super) fn current_layout(&self, cx: &App) -> prefs::ModeLayout {
        let (show_left, show_bottom, show_right) = self.regions_open(cx);
        prefs::ModeLayout {
            show_left,
            show_bottom,
            show_right,
            layout: self.layout_blob(cx),
        }
    }

    /// Whether the arrangement is still exactly the one the last settle installed — see
    /// [`Self::note_settled`].
    pub(super) fn is_settled_echo(&self, cx: &App) -> bool {
        self.settled_layout
            .as_ref()
            .is_some_and(|settled| *settled == self.current_layout(cx))
    }

    /// Tell every panel whether it is drawn.
    ///
    /// **The window pushes this rather than the panel reading it back.** The dock asks a panel
    /// whether it is visible while it is reconciling a tree — which happens from inside this
    /// window's own update, when a region is toggled, a panel is added, or the arrangement is
    /// written down — and a panel reading `AppState` there would be reading an entity that is
    /// already leased. So the answer is kept current here, where the facts are, and the panel only
    /// repeats it.
    pub(super) fn settle_visibility(&mut self, cx: &mut Context<Self>) {
        let is_ide = self.workbench.is_ide();
        let has_project = self.project(cx).is_some();
        let rail_mode = if is_ide {
            None
        } else {
            Some(self.workbench.rail_mode)
        };
        let on_screen: Vec<PaneId> = self
            .open_project(cx)
            .map(|open| open.panes.iter().map(|pane| pane.id).collect())
            .unwrap_or_default();
        // The tab keys the project on screen holds, and the layout each of them is in. Read
        // once: every file panel asks the same two questions of it. The knowledge base's open
        // documents are in the same map — they are the same `OpenFile`, their keys are in a key
        // space of their own, and `is_drawn` is where the two are told apart.
        let mut files: HashMap<String, ViewLayout> = self
            .editor(cx)
            .map(|editor| {
                editor
                    .open
                    .iter()
                    .map(|file| (file.key(), file.layout))
                    .collect()
            })
            .unwrap_or_default();
        let docs: Vec<(String, ViewLayout)> = self
            .open_project(cx)
            .map(|open| {
                open.kb
                    .docs
                    .iter()
                    .map(|doc| (doc.key(), doc.layout))
                    .collect()
            })
            .unwrap_or_default();
        files.extend(docs);

        let mut changed = false;
        for (kind, panel) in &self.panels {
            let key = kind.tab_key().or_else(|| kind.kb_key());
            let at = Visibility {
                is_ide,
                has_project,
                rail_mode,
                pane_on_screen: kind.pane().is_some_and(|id| on_screen.contains(&id)),
                file_open: key.is_some_and(|key| files.contains_key(key)),
                any_file_open: !files.is_empty(),
            };
            let drawn = kind.is_drawn(at);
            let layout = key
                .and_then(|key| files.get(key).copied())
                .unwrap_or_default();
            changed |= panel.update(cx, |panel, _| {
                let visible = panel.set_visible(drawn);
                panel.set_layout(layout) || visible
            });
        }
        if changed {
            cx.notify();
        }
    }

    /// Put back any panel the user dropped somewhere its kind forbids.
    pub(super) fn enforce_placement(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kinds: HashMap<PanelId, PanelKind> = self
            .panels
            .iter()
            .map(|(kind, panel)| (PanelId::from(panel.entity_id()), kind.clone()))
            .collect();
        let dock = self.dock.clone();
        dock::enforce_placement(&dock, &|id| kinds.get(&id).cloned(), window, cx);
    }
}

/// The panel a mode calls home in one of its side regions, if it has one there.
///
/// Read by [`AppState::toggle_region`] alone, and only for the region the user has just opened
/// onto nothing: **opening an edge is a gesture and deserves an answer, a switch is not** (`D156`).
/// A region a restore left open and empty is closed rather than filled — see
/// [`AppState::collapse_empty_regions`].
///
/// Teams and Tasks have no left-hand furniture, and Teams' right is the inspector its own screen
/// draws inline rather than a dockable panel, so neither names a kind here.
fn mode_side_furniture(mode: RailMode, region: Region) -> Option<PanelKind> {
    match (mode, region) {
        // The explorer is IDE furniture that is never closed (`PanelKind::closable`), so an IDE
        // left opened onto nothing is one the user dragged it out of. Where it went is preserved:
        // an explorer still in the tree is left where it was put and the switch is taken back
        // instead — [`AppState::toggle_region`] holds that half of the rule, because a panel that
        // cannot be put here is the caller's problem, not this table's.
        (RailMode::Ide, Region::Left) => Some(PanelKind::Explorer),
        (RailMode::Git, Region::Left) => Some(PanelKind::GitRefs),
        (RailMode::Git, Region::Right) => Some(PanelKind::GitChanges),
        (RailMode::Kb, Region::Left) => Some(PanelKind::KbExplorer),
        (RailMode::Tasks, Region::Right) => Some(PanelKind::Task),
        (RailMode::Agents, Region::Left) => Some(PanelKind::AgentsExplorer),
        _ => None,
    }
}
