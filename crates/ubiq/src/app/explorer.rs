use super::*;

impl AppState {
    /// Open or shut a folder, asking the host what is inside it the first time.
    ///
    /// Which folders are open is persisted state, so a toggle is written down as well as drawn.
    pub fn toggle_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.explorer.set_cursor(&path);
        // A folder opened for the first time knows nothing about what is inside it, and says so
        // on the row until the host answers.
        if open.explorer.toggle(&path) == Toggle::Listing {
            open.explorer.set_loading(&path, true);
            self.bus.send(Message::ProjectTree {
                project_id: project,
                rel_path: path.clone(),
                depth: EXPAND_DEPTH,
            });
        }
        self.remember(project, cx);
        cx.notify();
    }

    /// Fill the explorer's file cache in the background.
    ///
    /// The tree stays shut: a listing is cached so a filter can match, it does not expand.
    /// Skip-set folders are never asked about, which is how `node_modules` stays one row.
    /// Coalesce keystrokes, then walk the cache off the frame.
    ///
    /// An empty field is applied immediately: that walk is only open folders and must feel
    /// instant. Anything else waits [`FILTER_DEBOUNCE`] so a burst of letters is one job, and the
    /// walk itself runs on the background executor so the window keeps taking keystrokes.
    pub(super) fn schedule_explorer_filter(&mut self, draft: String, cx: &mut Context<Self>) {
        if short_query(&draft) {
            self.explorer_filter_gen = self.explorer_filter_gen.wrapping_add(1);
            self.workbench.file_filter.clear();
            if let Some(open) = self.open_project_mut(cx) {
                open.explorer.clear_filter();
                open.explorer.reanchor("");
            }
            cx.notify();
            return;
        }

        self.explorer_filter_gen = self.explorer_filter_gen.wrapping_add(1);
        let token = self.explorer_filter_gen;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FILTER_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if token != this.explorer_filter_gen {
                    return;
                }
                let text = this.file_filter.read(cx).value().to_string();
                this.spawn_explorer_filter(text, cx);
            });
        })
        .detach();
    }

    pub(super) fn spawn_explorer_filter(&mut self, text: String, cx: &mut Context<Self>) {
        if short_query(&text) {
            self.workbench.file_filter.clear();
            if let Some(open) = self.open_project_mut(cx) {
                open.explorer.clear_filter();
                open.explorer.reanchor("");
            }
            cx.notify();
            return;
        }

        let Some(open) = self.open_project_mut(cx) else {
            return;
        };
        // The snap is an `Arc` of the tree, not a copy of it. The walk runs on the background
        // executor — a separate thread — so the frame that started the job can keep taking keys.
        let snap = open.explorer.filter_snap();
        let job = open.explorer.begin_filter();
        let view = snap.view;
        let needle = text.clone();
        let drawing =
            cx.background_spawn(async move { ExplorerState::rows_from_snap(snap, &needle) });
        cx.spawn(async move |this, cx| {
            let rows = drawing.await;
            let _ = this.update(cx, |this, cx| {
                this.explorer_filter_ready(job, text, view, rows, cx);
            });
        })
        .detach();
    }

    fn explorer_filter_ready(
        &mut self,
        job: u64,
        text: String,
        view: ExplorerView,
        rows: Vec<crate::state::Row>,
        cx: &mut Context<Self>,
    ) {
        let (at, asking) = {
            let Some(open) = self.open_project_mut(cx) else {
                return;
            };
            // A folder the filter matched is an answer with its contents, so one the host has
            // never listed is asked for here. The reply re-runs the walk, which is what fills the
            // rows under it.
            let asking = open.explorer.unlisted_hits(&rows);
            open.explorer.begin_cache(&asking);
            if !open.explorer.apply_hits(job, text.clone(), view, rows) {
                return;
            }
            (open.explorer.cursor_index(&text), asking)
        };
        if let Some(project) = self.project(cx) {
            for rel_path in asking {
                self.bus.send(Message::ProjectTree {
                    project_id: project,
                    rel_path,
                    depth: EXPAND_DEPTH,
                });
            }
        }
        self.workbench.file_filter = text;
        if let Some(at) = at {
            self.explorer_scroll.scroll_to_item(at);
        }
        cx.notify();
    }

    pub(super) fn fill_explorer_cache(&mut self, project: ProjectId) {
        let asking = {
            let Some(open) = self.projects.get_mut(&project) else {
                return;
            };
            let asking = open.explorer.unlisted_for_cache();
            open.explorer.begin_cache(&asking);
            asking
        };
        for path in asking {
            self.bus.send(Message::ProjectTree {
                project_id: project,
                rel_path: path,
                depth: CACHE_DEPTH,
            });
        }
    }

    fn ask_listing(&mut self, project: ProjectId, path: String, cx: &mut Context<Self>) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.explorer.set_loading(&path, true);
        self.bus.send(Message::ProjectTree {
            project_id: project,
            rel_path: path,
            depth: EXPAND_DEPTH,
        });
        self.remember(project, cx);
        cx.notify();
    }

    pub fn set_explorer_view(&mut self, view: ExplorerView, cx: &mut Context<Self>) {
        let filter = self.workbench.file_filter.clone();
        if let Some(open) = self.open_project_mut(cx) {
            open.explorer.set_view(view, &filter);
        }
        if !filter.trim().is_empty() {
            self.spawn_explorer_filter(filter, cx);
        }
        cx.notify();
    }

    pub fn collapse_explorer(&mut self, cx: &mut Context<Self>) {
        let filter = self.workbench.file_filter.clone();
        if let Some(open) = self.open_project_mut(cx) {
            open.explorer.collapse_all();
            open.explorer.reanchor(&filter);
        }
        if let Some(project) = self.project(cx) {
            self.remember(project, cx);
        }
        cx.notify();
    }

    /// A key the explorer answered, or did not.
    ///
    /// Answering `false` is what hands the key back: `left` and `right` mean nothing in the flat
    /// list, and the caller propagates so the filter field gets its caret keys back.
    pub fn press_explorer_key(
        &mut self,
        key: ExplorerKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let filter = self.workbench.file_filter.clone();
        let (pressed, at) = {
            let Some(open) = self.open_project_mut(cx) else {
                return false;
            };
            let pressed = open.explorer.press(key, &filter);
            let at = open.explorer.cursor_index(&filter);
            (pressed, at)
        };

        match pressed {
            ExplorerPressed::Ignored => false,
            ExplorerPressed::Moved => {
                if let Some(at) = at {
                    self.explorer_scroll.scroll_to_item(at);
                }
                cx.notify();
                true
            }
            ExplorerPressed::Open { path } => {
                if matches!(key, ExplorerKey::ShiftEnter)
                    || !self.workbench.settings.ui.explorer_preview
                {
                    self.select_file(path, cx);
                } else {
                    self.select_file_temporary(path, cx);
                }
                true
            }
            ExplorerPressed::Listing { path } => {
                let Some(project) = self.project(cx) else {
                    return true;
                };
                self.ask_listing(project, path, cx);
                true
            }
            ExplorerPressed::Dismissed => {
                cx.notify();
                true
            }
            ExplorerPressed::ClearFilter => {
                self.explorer_filter_gen = self.explorer_filter_gen.wrapping_add(1);
                self.file_filter.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.workbench.file_filter.clear();
                if let Some(open) = self.open_project_mut(cx) {
                    open.explorer.clear_filter();
                    open.explorer.reanchor("");
                }
                cx.notify();
                true
            }
            // The same question the menu's Delete row raises, and the same modifier on it: the
            // keyboard is another way to ask, not a second set of rules.
            ExplorerPressed::Remove { path, is_dir } => {
                let trash = !window.modifiers().shift;
                self.workbench.file_dialog = Some(FileDialog::Remove {
                    path,
                    dir: is_dir,
                    trash,
                });
                cx.notify();
                true
            }
        }
    }

    pub fn click_explorer_row(
        &mut self,
        path: String,
        permanent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pressed = {
            let Some(open) = self.open_project_mut(cx) else {
                return;
            };
            open.explorer.click(&path)
        };
        match pressed {
            ExplorerPressed::Open { path } => {
                if permanent || !self.workbench.settings.ui.explorer_preview {
                    self.select_file(path, cx);
                } else {
                    // A preview leaves the keyboard on the tree: the click was a step through the
                    // tree, not a decision to start editing. A permanent open — double-click,
                    // Shift, ⌘ — is the decision, and that one takes the keyboard with it.
                    //
                    // A file that is **already open** is neither: there is no preview to make, the
                    // click only picks which of the open tabs to look at, and that one takes the
                    // keyboard the way clicking its tab would.
                    let previewing = self
                        .open_project(cx)
                        .is_some_and(|open| open.editor.index_of(&path).is_none());
                    self.select_file_temporary(path, cx);
                    if previewing {
                        self.pending_editor_focus = None;
                        self.focus_explorer_tree(window, cx);
                    }
                }
            }
            ExplorerPressed::Listing { path } => {
                let Some(project) = self.project(cx) else {
                    return;
                };
                self.focus_explorer_tree(window, cx);
                self.ask_listing(project, path, cx);
            }
            ExplorerPressed::Moved => {
                self.focus_explorer_tree(window, cx);
                cx.notify();
            }
            ExplorerPressed::Ignored
            | ExplorerPressed::Dismissed
            | ExplorerPressed::ClearFilter
            | ExplorerPressed::Remove { .. } => {}
        }
    }

    pub fn focus_explorer_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.file_filter.read(cx).focus_handle(cx);
        window.focus(&field, cx);
        cx.notify();
    }

    /// Put the keyboard on the tree itself. The tree's own keys — the arrows, Enter, and the one
    /// that removes a row — are live only from here, which is what stops a Backspace meant for the
    /// filter's text from reaching a file.
    pub fn focus_explorer_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.explorer_focus.clone(), cx);
        cx.notify();
    }

    /// Cycle the follow button: off, once, locked. Both live states reveal straight away.
    pub fn cycle_explorer_follow(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.open_project_mut(cx) else {
            return;
        };
        open.explorer.follow = open.explorer.follow.next();
        let reveal = open.explorer.follow != Follow::Off;
        if reveal {
            self.reveal_active_file(cx);
        }
        cx.notify();
    }

    /// The active editor changed: reveal it if the tree is locked to it, and otherwise do nothing.
    pub(super) fn follow_active_file(&mut self, cx: &mut Context<Self>) {
        if self.explorer(cx).map(|tree| tree.follow) != Some(Follow::Locked) {
            return;
        }
        self.reveal_active_file(cx);
    }

    /// Put the tree on the active editor's file: open the folders above it, move the keyboard
    /// there, and scroll it into view.
    ///
    /// The folders are opened through the same `wanted` list a restore uses, so a folder the host
    /// has not listed is asked for and opened when the answer lands rather than skipped.
    pub fn reveal_active_file(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(path) = open.explorer.selected.clone() else {
            return;
        };
        let mut at = path.as_str();
        while let Some(cut) = at.rfind('/') {
            at = &at[..cut];
            open.wanted.push(at.to_string());
        }
        open.explorer.set_cursor(&path);
        self.reach_wanted(project, cx);
        self.scroll_explorer_to_cursor(cx);
    }

    /// Bring the row the keyboard is on into view. Called again as listings land, because a folder
    /// opened by a reveal changes where the row is.
    pub(super) fn scroll_explorer_to_cursor(&mut self, cx: &mut Context<Self>) {
        let filter = self.workbench.file_filter.clone();
        let at = self
            .open_project(cx)
            .and_then(|open| open.explorer.cursor_index(&filter));
        if let Some(at) = at {
            self.explorer_scroll.scroll_to_item(at);
        }
        cx.notify();
    }

    /// Decorate the filter field with the filter the project on screen was left filtering by.
    ///
    /// The field is one per window; the filter it should show is the active project's, which
    /// changes when the window swings to another project. The `SetPreferences` answer that makes
    /// that true arrives on the bus loop, which has no window to write a field with, so this runs
    /// where a window is on hand — the dock's render — and only when the field is not being typed
    /// into, so a half-typed query is never stomped by a restore.
    pub fn sync_file_filter_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .file_filter
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        if focused {
            return;
        }
        let wanted = self.workbench.file_filter.clone();
        let shown = self.file_filter.read(cx).value().to_string();
        if shown != wanted {
            let field = self.file_filter.clone();
            field.update(cx, |state, cx| state.set_value(wanted, window, cx));
        }
    }

    pub fn open_explorer_menu(
        &mut self,
        path: Option<String>,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        self.workbench.open_menu = Some(MenuId::Explorer);
        if let Some(open) = self.open_project_mut(cx) {
            open.explorer.open_menu(path.as_deref(), at.0, at.1);
        }
        cx.notify();
    }

    pub(super) fn drop_explorer_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.explorer.menu = None;
        }
    }

    pub fn pick_explorer_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = {
            let Some(open) = self.open_project_mut(cx) else {
                return;
            };
            let Some(menu) = open.explorer.menu.clone() else {
                return;
            };
            let Some(entry) = menu.entries().get(index).copied() else {
                return;
            };
            open.explorer.menu = None;
            self.workbench.open_menu = None;
            (entry, menu.path)
        };

        let (entry, path) = picked;
        if !entry.enabled {
            cx.notify();
            return;
        }

        match entry.action {
            ExplorerAction::Open => {
                if let Some(path) = path {
                    self.select_file(path, cx);
                }
            }
            ExplorerAction::OpenDiff => {
                if let Some(path) = path {
                    self.open_diff(path, DiffBase::Head, cx);
                }
            }
            ExplorerAction::CopyPath => {
                if let Some(path) = path {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(path));
                }
                cx.notify();
            }
            ExplorerAction::CopyLink => {
                if let Some(path) = path
                    && let Some(project) = self.project(cx)
                {
                    let dest = Destination::new(project, View::Explorer { path });
                    self.copy_link(&dest, cx);
                }
                cx.notify();
            }
            ExplorerAction::CopyFullPath => {
                if let Some(rel) = path
                    && let Some(snap) = self.project_snapshot(cx)
                {
                    let full = std::path::Path::new(&snap.record.path)
                        .join(&rel)
                        .to_string_lossy()
                        .to_string();
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(full));
                }
                cx.notify();
            }
            ExplorerAction::OpenInSystem => {
                if let Some(rel) = path
                    && let Some(snap) = self.project_snapshot(cx)
                {
                    let full = std::path::Path::new(&snap.record.path)
                        .join(&rel)
                        .to_string_lossy()
                        .to_string();
                    let _ = open_in_system(&full);
                }
                cx.notify();
            }
            ExplorerAction::OpenInWeb => {
                if let Some(rel) = path
                    && let Some(snap) = self.project_snapshot(cx)
                {
                    let project_id = snap.record.id.to_string();
                    let project_name = snap.record.name.clone();
                    let root = std::path::PathBuf::from(&snap.record.path);
                    match crate::web_export::ensure_started_and_registered(
                        &project_id,
                        &project_name,
                        &root,
                    ) {
                        Ok(base) => {
                            let full = format!("{base}{}", rel.trim_start_matches('/'));
                            let _ = open_url(&full);
                        }
                        Err(err) => {
                            tracing::error!("web export failed to start: {err}");
                        }
                    }
                }
                cx.notify();
            }
            ExplorerAction::Refresh => {
                let Some(project) = self.project(cx) else {
                    cx.notify();
                    return;
                };
                if let Some(open) = self.projects.get_mut(&project) {
                    if let Some(rel) = &path {
                        open.explorer.set_loading(rel, true);
                        self.bus.send(Message::ProjectTree {
                            project_id: project,
                            rel_path: rel.clone(),
                            depth: EXPAND_DEPTH,
                        });
                    } else if open.explorer.is_listed() {
                        self.bus.send(Message::ProjectTree {
                            project_id: project,
                            rel_path: String::new(),
                            depth: EXPAND_DEPTH,
                        });
                    }
                }
                self.remember(project, cx);
                cx.notify();
            }
            ExplorerAction::CollapseAll => self.collapse_explorer(cx),
            ExplorerAction::NewFile | ExplorerAction::NewFolder => {
                let dir = entry.action == ExplorerAction::NewFolder;
                let parent = self
                    .explorer(cx)
                    .map(|explorer| explorer.target_dir(path.as_deref().unwrap_or_default()))
                    .unwrap_or_default();
                self.open_file_dialog(FileDialog::New { parent, dir }, "", window, cx);
            }
            ExplorerAction::Rename => {
                if let Some(path) = path {
                    let leaf = leaf_of(&path).to_string();
                    self.open_file_dialog(FileDialog::Rename { path }, &leaf, window, cx);
                }
            }
            ExplorerAction::Delete => {
                if let Some(path) = path {
                    let dir = self
                        .explorer(cx)
                        .is_some_and(|explorer| explorer.target_dir(&path) == path);
                    // Shift is read off the window rather than off the click, because the menu
                    // row is picked long after the right-click that opened it.
                    let trash = !window.modifiers().shift;
                    self.workbench.file_dialog = Some(FileDialog::Remove { path, dir, trash });
                    cx.notify();
                }
            }
            ExplorerAction::Copy => {
                if let Some(open) = self.open_project_mut(cx) {
                    open.explorer.copied = path;
                }
                cx.notify();
            }
            ExplorerAction::Paste => {
                let target = self
                    .explorer(cx)
                    .map(|explorer| explorer.target_dir(path.as_deref().unwrap_or_default()))
                    .unwrap_or_default();
                let source = self.explorer(cx).and_then(|e| e.copied.clone());
                if let Some(source) = source {
                    self.copy_path_into(source, target, cx);
                }
            }
            // Never picked: a separator is drawn disabled and the guard above has already
            // returned. The arm is here because the set is closed.
            ExplorerAction::Separator => {}
            ExplorerAction::Duplicate => {
                if let Some(path) = path {
                    let parent = parent_dir(&path);
                    self.copy_path_into(path, parent, cx);
                }
            }
        }
    }

    /// Copy one path into a folder, under a name that folder does not already hold.
    ///
    /// The free name is worked out here rather than by the host so that Duplicate — where the
    /// collision is certain — does not need a refusal to discover it.
    fn copy_path_into(&mut self, source: String, target: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let name = open.explorer.free_name(&target, leaf_of(&source));
        self.bus.send(Message::EditProjectPath {
            project_id: project,
            rel_path: source,
            to: Some(child_path(&target, &name)),
            op: PathOp::Copy,
        });
        cx.notify();
    }

    /// Put a file question up, with the field seeded and holding the keyboard.
    fn open_file_dialog(
        &mut self,
        dialog: FileDialog,
        seed: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench.file_dialog = Some(dialog);
        let field = self.file_name.clone();
        field.update(cx, |state, cx| state.set_value(seed, window, cx));
        let handle = self.file_name.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Enter on the dialog that is up. Bound at the window, so it works whether the keyboard is
    /// in the field or on the modal itself.
    ///
    /// With no dialog up the key is handed back: Enter belongs to every field and every list on
    /// screen, and swallowing it here would take it from all of them.
    pub fn confirm_dialog(
        &mut self,
        _: &DialogConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workbench.file_dialog.is_none() {
            cx.propagate();
            return;
        }
        self.confirm_file_dialog(window, cx);
    }

    /// Escape on the dialog that is up, handed back the same way when there is none — a bare
    /// Escape is the explorer's, the terminal's and the search panel's.
    pub fn cancel_dialog(&mut self, _: &DialogCancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.file_dialog.is_none() {
            cx.propagate();
            return;
        }
        self.close_file_dialog(cx);
    }

    pub fn close_file_dialog(&mut self, cx: &mut Context<Self>) {
        self.workbench.file_dialog = None;
        cx.notify();
    }

    /// A row dropped on a folder. Refused when it changes nothing (already there), when the folder
    /// is the row itself, and when the folder is *under* the row — dragging a folder into its own
    /// child. The host refuses that last one too; refusing it here is what keeps the gesture from
    /// raising a confirmation for a move that could never happen.
    pub fn drop_path_on(&mut self, path: String, into: String, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.explorer.drop_onto = None;
        }
        if !can_move_into(&path, &into) {
            cx.notify();
            return;
        }
        let is_dir = self
            .explorer(cx)
            .is_some_and(|explorer| explorer.target_dir(&path) == path);
        let unasked = self
            .workbench
            .move_unasked_until
            .is_some_and(|until| until > std::time::Instant::now());
        if is_dir && !unasked {
            self.workbench.file_dialog = Some(FileDialog::Move { path, into });
            cx.notify();
            return;
        }
        self.send_move(path, into, cx);
    }

    /// Note which folder a drag is over, so the row under the pointer can say it would take the
    /// drop. The only answer the user gets before letting go.
    pub fn drag_path_over(&mut self, into: Option<String>, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx)
            && open.explorer.drop_onto != into
        {
            open.explorer.drop_onto = into;
            cx.notify();
        }
    }

    fn send_move(&mut self, path: String, into: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let to = child_path(&into, leaf_of(&path));
        self.bus.send(Message::EditProjectPath {
            project_id: project,
            rel_path: path,
            to: Some(to),
            op: PathOp::Move,
        });
        cx.notify();
    }

    /// Answer the file question that is up, with whatever was typed into `file_name`.
    ///
    /// One method for all five, because each is the same two steps — one message, then the dialog
    /// goes away — and the branch is which op the message carries.
    pub fn confirm_file_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.workbench.file_dialog.clone() else {
            return;
        };
        // The two questions that are not about a path in a project are answered first: one drops a
        // buffer, the other takes the window with it.
        match dialog {
            FileDialog::DiscardChanges { key } => {
                self.close_file_dialog(cx);
                self.force_close_tab(&key, cx);
                return;
            }
            FileDialog::CloseWindow { quitting } => {
                self.close_file_dialog(cx);
                if quitting {
                    cx.quit();
                    return;
                }
                self.closing = true;
                window.remove_window();
                return;
            }
            _ => {}
        }
        let Some(project) = self.project(cx) else {
            self.close_file_dialog(cx);
            return;
        };
        let typed = self.file_name.read(cx).value().trim().to_string();

        match dialog {
            FileDialog::New { parent, dir } => {
                if typed.is_empty() {
                    return;
                }
                self.bus.send(Message::EditProjectPath {
                    project_id: project,
                    rel_path: child_path(&parent, &typed),
                    to: None,
                    op: PathOp::Create { dir },
                });
            }
            FileDialog::Rename { path } => {
                if typed.is_empty() || typed == leaf_of(&path) {
                    return;
                }
                self.bus.send(Message::EditProjectPath {
                    project_id: project,
                    rel_path: path.clone(),
                    to: Some(child_path(&parent_dir(&path), &typed)),
                    op: PathOp::Move,
                });
            }
            FileDialog::Remove { path, trash, .. } => {
                self.bus.send(Message::EditProjectPath {
                    project_id: project,
                    rel_path: path,
                    to: None,
                    op: if trash { PathOp::Trash } else { PathOp::Delete },
                });
            }
            FileDialog::Move { path, into } => {
                self.send_move(path, into, cx);
            }
            FileDialog::SaveAs { key } => {
                if typed.is_empty() {
                    return;
                }
                self.save_untitled_as(&key, typed, cx);
            }
            // Answered above, before the project was looked up.
            FileDialog::DiscardChanges { .. } | FileDialog::CloseWindow { .. } => {}
        }
        self.close_file_dialog(cx);
    }

    /// Stop asking about a folder move for the next ten minutes. The dialog's checkbox, which is
    /// window state and in memory: nothing to persist and nothing to migrate.
    pub fn toggle_move_unasked(&mut self, cx: &mut Context<Self>) {
        self.workbench.move_unasked_until = match self.workbench.move_unasked_until {
            Some(_) => None,
            None => Some(std::time::Instant::now() + MOVE_UNASKED),
        };
        cx.notify();
    }

    /// Select a row, and open it if it is a file.
    ///
    /// The tab appears on the click rather than on the answer: a click with no visible effect
    /// invites a second one, and a read that fails needs somewhere to say so.
    pub fn select_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let markdown_open = self.workbench.settings.ui.markdown_open.layout();
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.explorer.selected = Some(path.clone());
        self.pending_editor_focus = Some(tab_key(&path, Subject::File));

        let fresh = open.editor.index_of(&path).is_none();
        // A permanent open (double-click, Shift+click, menu) also promotes a preview that is
        // already showing the path: the user asked for it to stick, so it stops being replaceable.
        open.editor.promote(&path);
        let index = open.editor.open_pending(&path, markdown_open);
        open.editor.active = index;

        if fresh {
            // The tab and its panel open together: each open file is its own panel, so a tab with
            // none is a file with nowhere to be drawn.
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(tab_key(
                    &path,
                    Subject::File,
                ))));
            self.bus.send(Message::ReadProjectFile {
                project_id: project,
                rel_path: path,
                max_bytes: Some(MAX_FILE_BYTES),
            });
        } else {
            // A file already open is a panel the dock already holds, and adding it again is a
            // no-op — so the dock is asked to bring it forward instead. Which tab a group displays
            // is the dock's to say, and the highlight, the body, the keyboard and the strip
            // scrolling the tab into view all follow from it.
            self.pending_panels
                .push(PanelEdit::Reveal(PanelKind::File(tab_key(
                    &path,
                    Subject::File,
                ))));
        }
        self.follow_active_file(cx);
        self.remember(project, cx);
        cx.notify();
    }

    /// Open a file permanently (as opposed to temp preview). Double-click, Shift+click, or
    /// Shift+Enter all reach here.
    pub fn double_click_explorer_row(&mut self, path: String, cx: &mut Context<Self>) {
        self.select_file(path, cx);
    }

    /// Open a file as a temporary preview tab. A single temp tab is kept at a time: opening
    /// another temp closes the previous one. Editing the file promotes it to permanent.
    pub fn select_file_temporary(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let markdown_open = self.workbench.settings.ui.markdown_open.layout();
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.explorer.selected = Some(path.clone());
        self.pending_editor_focus = Some(tab_key(&path, Subject::File));

        let fresh = open.editor.index_of(&path).is_none();
        let (index, closed) = open.editor.open_temporary(&path, markdown_open);
        open.editor.active = index;

        if let Some(closed) = closed {
            self.pending_panels
                .push(PanelEdit::Close(PanelKind::File(closed)));
        }

        if fresh {
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(tab_key(
                    &path,
                    Subject::File,
                ))));
            self.bus.send(Message::ReadProjectFile {
                project_id: project,
                rel_path: path,
                max_bytes: Some(MAX_FILE_BYTES),
            });
        } else {
            self.pending_panels
                .push(PanelEdit::Reveal(PanelKind::File(tab_key(
                    &path,
                    Subject::File,
                ))));
        }
        self.follow_active_file(cx);
        self.remember(project, cx);
        cx.notify();
    }

    /// Open a file from outside every project this window holds — dropped in, not opened from the
    /// tree. It is read-only and hosted by the active project so it has somewhere to live among
    /// the panels, but the bus is never asked: there is no project to answer for a path outside
    /// its own.
    ///
    /// The tab key is the absolute path itself: `tab_key` is `subject.tag() + path` and
    /// `Subject::File`'s tag is empty, so a project-relative path (which never starts with `/`)
    /// cannot collide with it.
    pub fn open_guest_file(&mut self, abs: &Path, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let path = abs.to_string_lossy().into_owned();
        let markdown_open = self.workbench.settings.ui.markdown_open.layout();
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };

        let fresh = open.editor.index_of(&path).is_none();
        let index = open.editor.open_pending(&path, markdown_open);
        open.editor.open[index].guest = true;
        open.editor.active = index;
        self.pending_editor_focus = Some(tab_key(&path, Subject::File));

        if !fresh {
            cx.notify();
            return;
        }
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::File(tab_key(
                &path,
                Subject::File,
            ))));
        cx.notify();

        // The tab above is what `attach_file` requires before it will fill anything in — it finds
        // a tab by project and path and only fills one that `is_loading()` — so the read has to
        // happen after the tab exists, not before it.
        match read_guest_file(abs) {
            Ok(contents) => self.pending_files.push(FileArrival {
                project,
                path,
                contents,
            }),
            Err(reason) => {
                if let Some(open) = self.projects.get_mut(&project)
                    && let Some(file) = open.editor.find_mut(&path)
                {
                    file.set_failed(reason);
                }
            }
        }
    }

    /// A drop from outside the app: a folder becomes a project (temporary, until kept from the
    /// titlebar), a file under a project this window holds opens there, a file with a project open
    /// but outside all of them opens as a read-only guest, and a file with none open takes its
    /// folder in as a project and waits to select the file once the host answers.
    pub fn deliver_paths(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        for path in paths {
            if path.is_dir() {
                self.add_project(
                    path.to_string_lossy().into_owned(),
                    None,
                    None,
                    None,
                    true,
                    cx,
                );
                continue;
            }

            // Every project *this window* holds, not the whole catalogue: a file under a project
            // open in another window is exactly as much a stranger here as one under no project.
            let mut roots: Vec<(ProjectId, String)> = {
                let registry = WindowRegistry::read(cx);
                registry
                    .slot(self.window_id)
                    .into_iter()
                    .flat_map(|slot| slot.projects.iter().copied())
                    .filter_map(|id| registry.project(id).map(|s| (id, s.record.path.clone())))
                    .collect()
            };
            // Longest root first, so a project nested inside another one wins the match.
            roots.sort_by_key(|(_, root)| std::cmp::Reverse(root.len()));
            let hit = roots.into_iter().find_map(|(id, root)| {
                path.strip_prefix(&root)
                    .ok()
                    .map(|rel| (id, rel.to_string_lossy().into_owned()))
            });

            if let Some((id, rel)) = hit {
                // The match can be a project this window holds but is not showing; a drop opens
                // it, the same as clicking its row would.
                if self.project(cx) != Some(id) {
                    self.activate_project(id, cx);
                }
                self.select_file(rel, cx);
                continue;
            }

            if self.project(cx).is_some() {
                self.open_guest_file(path, cx);
                continue;
            }

            // No project open at all: the drop's folder becomes one, and the file is what
            // `Message::ProjectAdded` selects once that folder is actually open.
            let Some(parent) = path.parent() else {
                continue;
            };
            self.adding_select = Some(path.to_string_lossy().into_owned());
            self.add_project(
                parent.to_string_lossy().into_owned(),
                None,
                None,
                None,
                true,
                cx,
            );
        }
    }

    ///
    /// A diff is not the file, so it is a tab of its own beside it rather than something the file's
    /// tab switches into: opening a comparison never takes over what is being read or edited.
    pub fn open_diff(&mut self, path: String, base: DiffBase, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let key = tab_key(&path, Subject::Diff(base));
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };

        let fresh = index_of_key(&open.editor, &key).is_none();
        if fresh {
            open.editor
                .open
                .push(OpenFile::pending_on(&path, Subject::Diff(base)));
        }
        open.editor.active = index_of_key(&open.editor, &key).unwrap_or(0);

        if fresh {
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(key)));
            self.bus.send(Message::DiffProjectFile {
                project_id: project,
                rel_path: path,
                base,
            });
        }
        self.remember(project, cx);
        cx.notify();
    }
}

/// The last segment of a project-relative path.
pub(super) fn leaf_of(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((_, leaf)) => leaf,
        None => path,
    }
}

/// The folder holding a path. Empty is the project's root, which is what every path the interface
/// builds is relative to.
pub(super) fn parent_dir(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

/// `leaf` inside `parent`, with no leading slash when the parent is the root.
pub(super) fn child_path(parent: &str, leaf: &str) -> String {
    match parent.is_empty() {
        true => leaf.to_string(),
        false => format!("{parent}/{leaf}"),
    }
}

/// Whether moving `path` into the folder `into` is a move at all.
///
/// Three refusals: it is already there, the folder is the row itself, and the folder is under the
/// row — a folder dragged into its own child. The host refuses the last one too, and refusing it
/// here is what stops the gesture asking about a move that could never happen.
pub(super) fn can_move_into(path: &str, into: &str) -> bool {
    path != into && parent_dir(path) != into && !into.starts_with(&format!("{path}/"))
}

/// The shortest query worth walking the cache for. One or two letters match nearly every file in a
/// project, which costs a full walk to answer with a screen the user has to narrow anyway — so
/// under this the field is treated as empty and the tree is left alone.
pub const MIN_QUERY: usize = 3;

/// Whether a query is too short to search with, an empty field included.
fn short_query(text: &str) -> bool {
    text.trim().chars().count() < MIN_QUERY
}
