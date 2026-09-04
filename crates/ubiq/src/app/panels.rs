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
    /// **Opening the bottom or right region with nothing in it fills it.** The bottom exists to
    /// hold panes and opens onto a fresh one; the right exists to hold the chat and opens onto it.
    /// A region that opens onto a bar of nothing is not what the switch was asked for — except the
    /// left, whose only furniture is the explorer already on screen in every IDE window, so an
    /// empty left is the user having dragged it away on purpose and the switch leaves it be.
    pub fn toggle_region(&mut self, region: Region, window: &mut Window, cx: &mut Context<Self>) {
        self.dock.update(cx, |dock, cx| {
            dock.toggle_dock(dock::placement_of(region), window, cx);
        });
        let now_empty = {
            let placement = dock::placement_of(region);
            let dock = self.dock.read(cx);
            dock.is_dock_open(placement) && dock.is_empty(placement, cx)
        };
        if now_empty {
            match region {
                Region::Bottom => self.spawn_pane(None, Vec::new(), cx),
                Region::Right => self.pending_panels.push(PanelEdit::Open(PanelKind::Chat)),
                Region::Left | Region::Centre => {}
            }
        }
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
    }

    /// Put the panels that arrived on a message into the dock, and take out the ones that left.
    ///
    /// Drained in `render`, which is the same device the pending focus and the arrived files use,
    /// and for the same reason: both halves of a panel's life need a window.
    pub(super) fn settle_panels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // When a search is active, ensure the search panel is in the dock.
        if self.search.active.is_some() {
            let panel = self.panel(PanelKind::Search, cx);
            if !dock::holds(&self.dock.clone(), &panel, cx) {
                self.pending_panels.push(PanelEdit::Open(PanelKind::Search));
            }
        }
        if self.pending_panels.is_empty() {
            return;
        }
        for edit in std::mem::take(&mut self.pending_panels) {
            match edit {
                PanelEdit::Open(kind) => {
                    let home = kind.home();
                    let panel = self.panel(kind, cx);
                    // A saved arrangement is rebuilt before this queue is drained, so a file panel
                    // can already be in the tree by the time the edit that asked for it is read.
                    // Adding it twice would be two tabs on one file.
                    if dock::holds(&self.dock.clone(), &panel, cx) {
                        continue;
                    }
                    dock::add(&self.dock.clone(), &panel, home, window, cx);
                }
                PanelEdit::Close(kind) => {
                    if let Some(panel) = self.panels.remove(&kind) {
                        dock::remove(&self.dock.clone(), &panel, window, cx);
                    }
                }
            }
        }
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

    /// Rebuild a saved arrangement, on the frame after it arrives.
    ///
    /// A layout this build cannot use — a stale version, or one whose panels it has all lost — is
    /// discarded for the arrangement a fresh window opens in, rather than half-applied.
    pub(super) fn settle_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = self.pending_layout.take() else {
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
                // **A terminal panel is never built here.** A saved leaf names a pane, and a pane
                // exists only because the coordinator says so — one this window does not hold is
                // a pane from another session, and the leaf is dropped rather than drawn as a
                // terminal with no stream behind it.
                if kind.pane().is_some() && !panels.contains_key(&kind) && !kept.contains_key(&kind)
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
                dock::default_layout(&dock, &mut build, window, cx);
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
        for (kind, panel) in panels {
            let restorable =
                kind.pane().is_some() || kind.tab_key().is_some() || on_screen.contains(&kind);
            if restorable && !kept.contains_key(&kind) {
                let home = kind.home();
                // On screen before, on screen after: a reveal also brings its region back, which
                // an arrangement that predates the panel has closed.
                if on_screen.contains(&kind) {
                    dock::reveal(&dock, &panel, home, window, cx);
                } else {
                    dock::add(&dock, &panel, home, window, cx);
                }
                kept.insert(kind, panel);
            }
        }
        self.panels = kept;
        // A file panel's payload carries the layout its viewer was left in, which belongs on the
        // file rather than on the panel: the panel only repeats it, the way it repeats visibility.
        if let Some(project) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&project)
        {
            for (key, layout) in layouts {
                if let Some(file) = open.editor.open.iter_mut().find(|file| file.key() == key) {
                    file.set_layout(layout);
                }
            }
        }
        cx.notify();
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
        let on_screen: Vec<PaneId> = self
            .open_project(cx)
            .map(|open| open.panes.iter().map(|pane| pane.id).collect())
            .unwrap_or_default();
        // The tab keys the project on screen holds, and the layout each of them is in. Read once:
        // every file panel asks the same two questions of it.
        let files: HashMap<String, ViewLayout> = self
            .editor(cx)
            .map(|editor| {
                editor
                    .open
                    .iter()
                    .map(|file| (file.key(), file.layout))
                    .collect()
            })
            .unwrap_or_default();

        let mut changed = false;
        for (kind, panel) in &self.panels {
            let key = kind.tab_key();
            let at = Visibility {
                is_ide,
                has_project,
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
