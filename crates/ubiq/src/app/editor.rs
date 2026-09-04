use super::*;

impl AppState {
    /// One open tab of the project on screen, by its key. What a file panel draws and what its tab
    /// reports are both this.
    pub fn file(&self, key: &str, cx: &App) -> Option<&OpenFile> {
        self.editor(cx)?.open.iter().find(|file| file.key() == key)
    }

    /// Put one file's viewer into one of its layouts.
    ///
    /// The panel repeats the fact rather than owning it — `settle_visibility` pushes it every
    /// frame — but it is pushed here too, because the arrangement can be written down before the
    /// next frame runs and a panel one frame behind would write down the layout the file was in
    /// before the click.
    pub fn set_view_layout(&mut self, key: &str, layout: ViewLayout, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        file.set_layout(layout);

        let panel = self.panels.get(&PanelKind::File(key.to_string())).cloned();
        if let Some(panel) = panel {
            panel.update(cx, |panel, _| panel.set_layout(layout));
        }
        self.remember_view(cx);
        cx.notify();
    }

    /// Toggle whether the YAML frontmatter disclosure is open for the given tab.
    pub fn toggle_frontmatter(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        file.toggle_frontmatter();
        cx.notify();
    }

    /// Ask for every file a project's blob said was open, and open the folders it said were.
    ///
    /// The tree is restored a level at a time: a folder cannot be opened before its parent has
    /// been listed, so what is out of reach waits in `wanted` for the next listing.
    pub(super) fn restore_files(
        &mut self,
        project: ProjectId,
        view: &prefs::ViewPrefs,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        if open.restored {
            return;
        }
        open.restored = true;

        // Tab keys, so a remembered diff reopens as a diff rather than as the file it was taken
        // from. A key that is already open is not opened twice.
        for key in &view.open_files {
            if index_of_key(&open.editor, key).is_some() {
                continue;
            }
            let (path, subject) = from_tab_key(key);
            open.editor.open.push(OpenFile::pending_on(&path, subject));
        }
        if let Some(active) = &view.active_file
            && let Some(at) = index_of_key(&open.editor, active)
        {
            open.editor.active = at;
        }
        open.explorer.selected = view.selected.clone();
        open.wanted = view.expanded.clone();

        // Each tab is a panel. A saved arrangement usually carries them and the queued edits are
        // then no-ops, but one that was discarded — a stale version, an unreadable blob — must
        // still leave the files somewhere to be drawn.
        let tabs: Vec<(String, String, Subject)> = open
            .editor
            .open
            .iter()
            .map(|file| (file.key(), file.path.clone(), file.subject))
            .collect();
        for (key, rel_path, subject) in tabs {
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(key)));
            match subject {
                Subject::File => self.bus.send(Message::ReadProjectFile {
                    project_id: project,
                    rel_path,
                    max_bytes: Some(MAX_FILE_BYTES),
                }),
                Subject::Diff(base) => self.bus.send(Message::DiffProjectFile {
                    project_id: project,
                    rel_path,
                    base,
                }),
            }
        }
        self.reach_wanted(project, cx);
        cx.notify();
    }

    /// Open whichever remembered folders have become reachable, and ask for what they hold.
    pub(super) fn reach_wanted(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let mut wanted = std::mem::take(&mut open.wanted);
        let ask = open.explorer.reopen(&mut wanted);
        open.wanted = wanted;
        for rel_path in &ask {
            open.explorer.set_loading(rel_path, true);
        }

        for rel_path in ask {
            self.bus.send(Message::ProjectTree {
                project_id: project,
                rel_path,
                depth: EXPAND_DEPTH,
            });
        }
        cx.notify();
    }

    // ── Editor ──────────────────────────────────────────────────────

    /// Bring a tab forward. Each file owns its buffer, so nothing is copied and nothing is lost.
    pub fn activate_editor_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        if index >= open.editor.open.len() {
            return;
        }
        open.editor.active = index;
        open.editor.pending_tab_close = None;
        if let Some(path) = open.editor.active_path() {
            open.explorer.selected = Some(path);
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// Close a tab. One holding unsaved changes asks first: the × becomes a confirmation rather
    /// than taking the click, on the pattern a project with running terminals already uses.
    ///
    /// Clicking the tab itself is how the question is answered no, because bringing a tab forward
    /// is already the one thing that clears it.
    pub fn close_editor_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.open.get(index) else {
            return;
        };
        let key = file.key();
        if file.dirty() && open.editor.pending_tab_close.as_deref() != Some(key.as_str()) {
            open.editor.pending_tab_close = Some(key);
            cx.notify();
            return;
        }

        open.editor.close(index);
        // The tab and its panel go together, and the panel leaves through a `Window` this does not
        // have — so it queues, like every other edit to the dock.
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::File(key)));
        self.remember(project, cx);
        cx.notify();
    }

    /// A file panel became the displayed tab of its group, so its file is the active one.
    ///
    /// The dock is where that is decided and the editor learns it from here, which is the same
    /// direction focus already travels.
    pub fn activate_file(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            return;
        };
        open.editor.active = at;
        open.editor.pending_tab_close = None;
        if let Some(path) = open.editor.active_file().map(|file| file.path.clone()) {
            open.explorer.selected = Some(path);
        }
        // A file panel becoming the displayed tab asks for the keyboard, which the frame grants
        // once it has a window and the file has a buffer — see [`Self::take_editor_focus`].
        self.pending_editor_focus = Some(key.to_string());
        self.remember(project, cx);
        cx.notify();
    }

    /// A file panel left the dock for good, so its tab closes with it.
    ///
    /// A tab holding unsaved changes asks first, which through the dock means **coming back**: the
    /// panel is put in its home group again with its label turned into the question, and a second
    /// close takes it. Bringing it forward is still how the question is answered no.
    pub fn closed_file_panel(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            // The tab went first — `close_editor_tab` — and this is the panel following it out.
            self.panels.remove(&PanelKind::File(key.to_string()));
            return;
        };

        if open.editor.open[at].dirty() && open.editor.pending_tab_close.as_deref() != Some(key) {
            open.editor.pending_tab_close = Some(key.to_string());
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(key.to_string())));
            cx.notify();
            return;
        }

        open.editor.close(at);
        self.panels.remove(&PanelKind::File(key.to_string()));
        self.remember(project, cx);
        cx.notify();
    }

    /// Close every open file tab except `key`, in `EditorPaneState::open` order. A dirty tab is
    /// asked for rather than closed — see [`Self::close_editor_tab`] for the ask — so a burst of
    /// "close others" never eats unsaved work.
    pub fn close_editor_tabs_except(&mut self, key: &str, cx: &mut Context<Self>) {
        self.close_editor_tabs_filtered(|_, open_key| open_key != key, cx);
    }

    /// Close the file tabs to the left of `key` in `EditorPaneState::open` order.
    pub fn close_editor_tabs_left(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(at) = self.editor_index_of_key(key, cx) else {
            return;
        };
        self.close_editor_tabs_in_range(0..at, cx);
    }

    /// Close the file tabs to the right of `key` in `EditorPaneState::open` order.
    pub fn close_editor_tabs_right(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(at) = self.editor_index_of_key(key, cx) else {
            return;
        };
        self.close_editor_tabs_in_range(at + 1..usize::MAX, cx);
    }

    /// Close every open file tab.
    pub fn close_all_editor_tabs(&mut self, cx: &mut Context<Self>) {
        self.close_editor_tabs_in_range(0..usize::MAX, cx);
    }

    /// The index of a file tab in the active project's `EditorPaneState::open`, or `None`.
    fn editor_index_of_key(&self, key: &str, cx: &App) -> Option<usize> {
        let project = self.project(cx)?;
        let open = self.projects.get(&project)?;
        index_of_key(&open.editor, key)
    }

    /// Close the tabs the filter names, from the highest index down (so removal never shifts an
    /// index still to come). Dirty ones are only asked for, by [`Self::close_editor_tab`].
    fn close_editor_tabs_filtered(
        &mut self,
        keep: impl Fn(usize, &str) -> bool + Copy,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let indices = {
            let Some(open) = self.projects.get(&project) else {
                return;
            };
            (0..open.editor.open.len())
                .filter(|&ix| {
                    let key = open.editor.open[ix].key();
                    keep(ix, &key)
                })
                .collect::<Vec<_>>()
        };
        for ix in indices.into_iter().rev() {
            self.close_editor_tab(ix, cx);
        }
    }

    /// Close the tabs whose indices fall in `range`, high-to-low, each asked if dirty.
    fn close_editor_tabs_in_range(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) {
        self.close_editor_tabs_filtered(|ix, _| range.contains(&ix), cx);
    }

    /// Write the file behind one tab — not just the active one — back, so a context menu can save
    /// the tab it was opened on. The save is `save_active_file`'s, with the file named by its tab
    /// key instead of by whatever tab happens to be on screen.
    pub fn save_file(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            return;
        };
        let Some(file) = open.editor.open.get(at) else {
            return;
        };
        if !file.savable() {
            return;
        }
        let Some(buffer) = file.buffer() else {
            return;
        };
        let text = buffer.read(cx).value().to_string();
        let (rel_path, expected) = (file.path.clone(), file.version());
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&rel_path)
        {
            file.mark_saving(text.clone());
        }
        self.bus.send(Message::WriteProjectFile {
            project_id: project,
            rel_path,
            bytes: text.into_bytes(),
            expected,
        });
        cx.notify();
    }

    /// Open the right-click menu on a file tab, anchored where the button went down.
    ///
    /// The dock's tab bar draws the tab whose key this is and hands the click across the renderer
    /// seam; the menu itself is painted here, over the window, so it is a fact about `AppState`
    /// rather than about the dock.
    pub fn open_file_tab_menu(&mut self, key: &str, at: (f32, f32), cx: &mut Context<Self>) {
        if self.workbench.open_menu != Some(MenuId::Explorer) && self.workbench.open_menu.is_some()
        {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::FileTab);
        self.workbench.file_tab_menu = Some((key.to_string(), at));
        cx.notify();
    }

    /// Act on one row of the open file-tab menu, by the row's index.
    pub fn pick_file_tab_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((key, _)) = self.workbench.file_tab_menu.clone() else {
            return;
        };
        self.workbench.open_menu = None;
        self.workbench.file_tab_menu = None;
        match index {
            0 => {
                self.close_editor_tab_at_key(&key, cx);
            }
            1 => self.close_editor_tabs_except(&key, cx),
            2 => self.close_editor_tabs_left(&key, cx),
            3 => self.close_editor_tabs_right(&key, cx),
            4 => self.close_all_editor_tabs(cx),
            5 => self.copy_full_path_for_tab(&key, cx),
            6 => self.open_in_finder_for_tab(&key, cx),
            7 => self.save_file(&key, cx),
            8 => self.toggle_editor_wrap(window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Copy the file a tab names, resolved against the project root, to the clipboard.
    fn copy_full_path_for_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        let (rel, _) = from_tab_key(key);
        if let Some(snap) = self.project_snapshot(cx) {
            // A guest tab's key is already an absolute path, and `Path::join` with an absolute
            // argument replaces the base rather than concatenating with it — so this resolves a
            // guest file correctly without a special case here.
            let full = std::path::Path::new(&snap.record.path)
                .join(&rel)
                .to_string_lossy()
                .to_string();
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(full));
        }
    }

    /// Reveal the file a tab names in the system file manager.
    fn open_in_finder_for_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        let (rel, _) = from_tab_key(key);
        if let Some(snap) = self.project_snapshot(cx) {
            // See `copy_full_path_for_tab`: `join` with an absolute `rel` replaces the base, so a
            // guest file's absolute key resolves to itself here too.
            let full = std::path::Path::new(&snap.record.path)
                .join(&rel)
                .to_string_lossy()
                .to_string();
            let _ = open_in_system(&full);
        }
    }

    /// Close the one tab named by a key, the way the close button does: a dirty tab is asked for
    /// rather than silently closed.
    fn close_editor_tab_at_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(at) = self.editor_index_of_key(key, cx) else {
            return;
        };
        self.close_editor_tab(at, cx);
    }

    /// Open the new-pane control's chevron menu, anchored where the button went down.
    ///
    /// The list is asked for again here rather than trusted from attach: a shell installed since
    /// the window opened is offered, and the probe is a handful of `stat` calls on the host's own
    /// thread. Whatever is already known is what this frame draws; the answer replaces it.
    pub fn open_new_pane_menu(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::NewPane);
        self.workbench.new_pane_menu = Some(at);
        self.bus.send(Message::ListShells);
        self.bus.send(Message::ListAgentTypes);
        cx.notify();
    }

    /// Act on one row of the open new-pane menu, by the row's index.
    ///
    /// An agent row starts a pane running that harness, and a shell row starts one running that
    /// shell — the same call the "+" makes, with a program on it. A harness the host could not
    /// find is drawn disabled and takes no click, so picking it here does nothing rather than
    /// asking for a spawn that would fail. Past the last shell is the separator, which is a row
    /// and does nothing, and then the console, which is revealed rather than started.
    pub fn pick_new_pane_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench.open_menu = None;
        self.workbench.new_pane_menu = None;
        let has_project = self.project(cx).is_some();
        match self.workbench.new_pane_rows(has_project).get(index) {
            Some(NewPaneRow::Agent(agent)) => {
                let Some(agent) = self.workbench.agent_types.get(*agent) else {
                    return;
                };
                if !agent.available {
                    return;
                }
                self.spawn_pane(Some(agent.id.clone()), Vec::new(), cx);
            }
            Some(NewPaneRow::Shell(shell)) => {
                let Some(program) = self
                    .workbench
                    .shells
                    .get(*shell)
                    .map(|shell| shell.program.clone())
                else {
                    return;
                };
                self.spawn_pane(Some(program), Vec::new(), cx);
            }
            Some(NewPaneRow::Console) => self.reveal_console(window, cx),
            Some(NewPaneRow::Separator) | None => {}
        }
        cx.notify();
    }

    /// Dismiss the new-pane menu — an outside click, or a pick already taken it.
    pub fn dismiss_new_pane_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.new_pane_menu = None;
        cx.notify();
    }

    /// Open the search panel and bring it into focus.
    pub fn open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_search(window, cx);
    }

    /// Dismiss the file tab's menu — an outside click, or a pick already taken it.
    pub fn dismiss_file_tab_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.file_tab_menu = None;
        cx.notify();
    }

    /// The caret's one-based position, as the status bar reports it. Absent when nothing is open,
    /// so the status bar omits the segment rather than reporting a caret in a buffer nobody is
    /// looking at.
    pub fn cursor_line_column(&self, cx: &App) -> Option<(u32, u32)> {
        let buffer = self.editor(cx)?.active_file()?.buffer()?;
        let position = buffer.read(cx).cursor_position();
        Some((position.line + 1, position.character + 1))
    }

    /// Close the active file's tab — the keyboard equivalent of clicking its ×.
    pub fn close_active_editor(
        &mut self,
        _: &CloseEditor,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        if open.editor.active_file().is_none() {
            return;
        }
        self.close_editor_tab(open.editor.active, cx);
    }

    /// Write the active file back.
    ///
    /// Nothing happens with no project, no active file, bytes that never arrived, or a read the
    /// host cut short: writing a prefix back would shorten the file.
    pub fn save_active_file(&mut self, _: &SaveFile, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(file) = open.editor.active_file() else {
            return;
        };
        if !file.savable() {
            return;
        }
        let Some(buffer) = file.buffer() else {
            return;
        };
        let text = buffer.read(cx).value().to_string();
        let (rel_path, expected) = (file.path.clone(), file.version());

        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&rel_path)
        {
            file.mark_saving(text.clone());
        }
        self.bus.send(Message::WriteProjectFile {
            project_id: project,
            rel_path,
            bytes: text.into_bytes(),
            expected,
        });
        cx.notify();
    }

    /// Turn everything that arrived since the last frame into a buffer.
    ///
    /// In `render`, because a buffer needs a window and a message does not come with one.
    pub(super) fn attach_arrived_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for arrival in std::mem::take(&mut self.pending_files) {
            self.attach_file(arrival, window, cx);
        }
    }

    fn attach_file(&mut self, arrival: FileArrival, window: &mut Window, cx: &mut Context<Self>) {
        let FileArrival {
            project,
            path,
            contents,
        } = arrival;

        // A tab that already holds bytes is never overwritten: there is no reload action, and
        // whatever has been typed into it would go with them.
        let draws_bytes = self
            .projects
            .get(&project)
            .and_then(|open| open.editor.open.iter().find(|file| file.path == path))
            .is_some_and(|file| file.is_loading() && file.draws_bytes());
        if !self
            .projects
            .get(&project)
            .and_then(|open| open.editor.open.iter().find(|file| file.path == path))
            .is_some_and(|file| file.is_loading())
        {
            return;
        }

        // A file whose viewer is a decoder — an image — is handed its own bytes, not a buffer:
        // it is not text and is not nothing, and the decoder wants what the host read.
        if draws_bytes {
            if let Some(open) = self.projects.get_mut(&project)
                && let Some(file) = open.editor.find_mut(&path)
            {
                file.set_bytes(contents.bytes);
            }
            cx.notify();
            return;
        }

        if contents.is_binary {
            if let Some(open) = self.projects.get_mut(&project)
                && let Some(file) = open.editor.find_mut(&path)
            {
                file.set_binary();
            }
            cx.notify();
            return;
        }

        // Bytes are the host's, and decoding is the interface's: a file that is not valid UTF-8 is
        // still text somebody wants to read.
        let text = String::from_utf8_lossy(&contents.bytes).into_owned();
        let language = FileLanguage::of(&path);
        // The project's wrap is a preference of the project rather than of a file, so a buffer
        // opens the way the project was left rather than the editor's own default.
        let wrap = self
            .projects
            .get(&project)
            .and_then(|open| open.prefs.editor_wrap)
            .unwrap_or(true);
        let buffer = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(ui::editor::highlighter_language(language))
                .line_number(true)
                .folding(true)
                .show_whitespaces(false)
                .soft_wrap(wrap)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .default_value(text.clone())
        });

        // Dirty is kept current by the buffer's own change event, because comparing every frame
        // costs the file's length times the tabs open times the frame rate.
        let watched = path.clone();
        let change = cx.subscribe_in(
            &buffer,
            window,
            move |this, buffer, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let typed = buffer.read(cx).value().to_string();
                if let Some(open) = this.projects.get_mut(&project) {
                    let key = tab_key(&watched, Subject::File);
                    let promoted = open
                        .editor
                        .find_mut(&watched)
                        .is_some_and(|file| file.refresh_dirty(&typed));
                    // The first edit promotes the preview; as it does, the pane must forget it was
                    // the replaceable preview, or opening another file would close the promoted tab.
                    if promoted {
                        open.editor.promote_key(&key);
                    }
                }
                cx.notify();
            },
        );

        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&path)
        {
            file.attach(buffer, text, contents.truncated, contents.version, change);
        }
        cx.notify();
    }

    // ── The agents screen ───────────────────────────────────────────
    //
    // The arrangement is the interface's own: not one of these sends a message. What crosses the
    // bus from this screen is the one thing that is not about arrangement — what the user typed at
    // an agent, which `steer_column` sends. Every handler is guarded on the window holding a
    // project, because the screen is a view of one project's work.
}
