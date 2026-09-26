use super::*;

impl AppState {
    /// The Git screen's view of the project on screen. Absent with no project, which is what keeps
    /// the screen from drawing an empty repository.
    pub fn git_view(&self, cx: &App) -> Option<&GitView> {
        self.open_project(cx).map(|open| &open.git_view)
    }

    pub fn git_view_mut(&mut self, cx: &App) -> Option<&mut GitView> {
        let id = self.project(cx)?;
        self.projects.get_mut(&id).map(|open| &mut open.git_view)
    }

    /// Every path the last working-tree map had something to say about. **Absent is "nothing has
    /// been read"** and empty is "nothing has changed", which are two different screens.
    pub fn git_entries(&self, cx: &App) -> Option<&[GitEntry]> {
        let open = self.open_project(cx)?;
        open.git.as_ref().map(|_| open.git_entries.as_slice())
    }

    /// Ask for the repository again: the overview, the working tree, the refs and the first page
    /// of history. What the readout in the status bar does, from the screen that is about it.
    pub fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        self.bus.send(Message::ProjectGit { project_id });
        self.bus.send(Message::RefreshProjectGit {
            project_id,
            full: true,
        });
        self.bus.send(Message::ProjectGitRefs {
            project_id,
            with_tracking: true,
        });
        if let Some(git) = self.git_view_mut(cx) {
            // A fresh first page, not a page appended to whatever the last project on screen left
            // behind. `log_inflight` is what tells `receive_git` this request's reply apart from
            // a stale one still in flight from before.
            git.log_cursor = None;
            git.log_inflight = Some(None);
            git.pending = Some(GitPending::Refresh);
        }
        self.send_git_log(None, cx);
        cx.notify();
    }

    /// Stage one path in the project's repository.
    pub fn stage_git_path(&mut self, path: &str, cx: &mut Context<Self>) {
        self.write_git(
            GitWriteOp::Stage {
                rel_path: path.to_string(),
            },
            false,
            cx,
        );
    }

    /// Take one path out of the index.
    pub fn unstage_git_path(&mut self, path: &str, cx: &mut Context<Self>) {
        self.write_git(
            GitWriteOp::Unstage {
                rel_path: path.to_string(),
            },
            false,
            cx,
        );
    }

    /// Put every project-relative change in the index.
    pub fn stage_all_git(&mut self, cx: &mut Context<Self>) {
        self.write_git(GitWriteOp::StageAll, false, cx);
    }

    /// Restore the index to HEAD for every path in the project's scope.
    pub fn unstage_all_git(&mut self, cx: &mut Context<Self>) {
        self.write_git(GitWriteOp::UnstageAll, false, cx);
    }

    /// Commit what is staged, with the message in the commit box. A blank message is a no-op.
    pub fn commit_git(&mut self, cx: &mut Context<Self>) {
        let message = self.git_message.read(cx).value().trim().to_string();
        if message.is_empty() {
            return;
        }
        let amend = self.git_view(cx).is_some_and(|git| git.amend);
        self.write_git(GitWriteOp::Commit { message, amend }, true, cx);
    }

    pub fn fetch_all_git(&mut self, cx: &mut Context<Self>) {
        self.write_git(GitWriteOp::FetchAll, true, cx);
    }

    pub fn pull_git(&mut self, cx: &mut Context<Self>) {
        self.write_git(GitWriteOp::Pull, true, cx);
    }

    pub fn push_git(&mut self, cx: &mut Context<Self>) {
        self.write_git(GitWriteOp::Push, true, cx);
    }

    fn write_git(&mut self, op: GitWriteOp, refresh_history: bool, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        if let Some(git) = self.git_view_mut(cx) {
            git.pending = Some(match &op {
                GitWriteOp::Stage { .. } => GitPending::Stage,
                GitWriteOp::Unstage { .. } => GitPending::Unstage,
                GitWriteOp::StageAll => GitPending::StageAll,
                GitWriteOp::UnstageAll => GitPending::UnstageAll,
                GitWriteOp::Commit { .. } => GitPending::Commit,
                GitWriteOp::FetchAll => GitPending::FetchAll,
                GitWriteOp::Pull => GitPending::Pull,
                GitWriteOp::Push => GitPending::Push,
            });
        }
        self.bus.send(Message::WriteProjectGit { project_id, op });
        if refresh_history {
            self.bus.send(Message::ProjectGitRefs {
                project_id,
                with_tracking: true,
            });
            if let Some(git) = self.git_view_mut(cx) {
                git.log_cursor = None;
                git.log_inflight = Some(None);
            }
            self.send_git_log(None, cx);
        }
        cx.notify();
    }

    /// Ask for the next page of history — what a "load more" trigger at the bottom of the list
    /// sends. A no-op with a page already in flight or nothing left to page in, on the same
    /// `log_inflight`/`log_done` bookkeeping `refresh_git` and `receive_git` already keep.
    pub fn load_more_git_log(&mut self, cx: &mut Context<Self>) {
        let Some(git) = self.git_view(cx) else {
            return;
        };
        if git.log_done || git.log_inflight.is_some() {
            return;
        }
        let cursor = git.log_cursor.clone();
        self.send_git_log(cursor, cx);
        cx.notify();
    }

    /// Ask for a page of history, walking `GitView::branch_filter` when one is set.
    fn send_git_log(&mut self, cursor: Option<String>, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let rev = self.git_view(cx).and_then(|git| git.branch_filter.clone());
        if let Some(git) = self.git_view_mut(cx) {
            if cursor.is_none() {
                git.log_cursor = None;
            }
            git.log_inflight = Some(cursor.clone());
        }
        self.bus.send(Message::ProjectGitLog {
            project_id,
            cursor,
            count: 100,
            rel_path: None,
            first_parent: false,
            rev,
        });
    }

    pub fn toggle_git_section(&mut self, section: RefSection, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.toggle_section(section);
        }
        cx.notify();
    }

    pub fn toggle_git_change_section(&mut self, section: ChangeSection, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.toggle_change_section(section);
        }
        cx.notify();
    }

    pub fn select_git_ref(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.selected_ref = Some(index);
        }
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::GitHistory));
        cx.notify();
    }

    /// Bring the history to the named ref's commit, scrolling it into view. If that commit is not
    /// in the loaded walk, the history is asked again from that ref so the tip is on screen.
    pub fn jump_to_git_ref(&mut self, index: usize, cx: &mut Context<Self>) {
        self.select_git_ref(index, cx);
        if let Some(slot) = self.git_view(cx).and_then(|git| {
            let commit = git.commit_index_for_ref(index)?;
            git.visible_commits()
                .iter()
                .position(|(held, _)| *held == commit)
        }) {
            if let Some(git) = self.git_view_mut(cx) {
                git.selected_commit = git.commit_index_for_ref(index);
            }
            self.git_scroll
                .scroll_to_item(slot, gpui::ScrollStrategy::Top);
            cx.notify();
            return;
        }
        let name = self
            .git_view(cx)
            .and_then(|git| git.refs.get(index).map(|row| row.name.clone()));
        if let Some(name) = name {
            self.set_git_branch_filter(Some(name), cx);
        }
    }

    pub fn toggle_git_folder(&mut self, section: RefSection, path: String, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.toggle_folder(section, &path);
        }
        cx.notify();
    }

    pub fn set_git_branch_filter(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.branch_filter = name;
        }
        self.send_git_log(None, cx);
        self.close_menu(cx);
        cx.notify();
    }

    /// Point the screen at a commit, or at the working tree. `None` is the uncommitted row, which
    /// is a selection like any other rather than nothing selected. Any comparison in progress
    /// drops: a plain click asks a new question, not a third end to the old one.
    pub fn select_git_commit(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.selected_commit = index;
            git.compare_commit = None;
            git.range_from = None;
            git.range_to = None;
            git.range_files.clear();
            git.range_inflight = false;
        }
        cx.notify();
    }

    /// Cmd/ctrl-click on a commit: the second end of a range comparison, or dropping the one
    /// already set. A pair asks the host for the paths between them; breaking the pair clears
    /// what it asked without asking again.
    pub fn toggle_git_compare(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let request = {
            let Some(git) = self.git_view_mut(cx) else {
                return;
            };
            if git.toggle_compare(index) {
                let Some((from, to)) = git.compare_ids() else {
                    return;
                };
                git.range_from = Some(from.clone());
                git.range_to = Some(to.clone());
                git.range_inflight = true;
                git.range_files.clear();
                git.selected_path = None;
                git.diff = None;
                Some((from, to))
            } else {
                git.range_from = None;
                git.range_to = None;
                git.range_files.clear();
                git.range_inflight = false;
                None
            }
        };
        if let Some((from, to)) = request {
            self.bus.send(Message::ProjectGitChanged {
                project_id,
                from: Some(from),
                to,
            });
        }
        cx.notify();
    }

    /// Raise a Git screen context menu at the pointer: a changed path, a commit, or a ref.
    pub fn open_git_menu(&mut self, kind: GitMenuKind, at: (f32, f32), cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.open_menu(kind, at.0, at.1);
        }
        cx.notify();
    }

    pub fn dismiss_git_menu(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.close_menu(epoch);
        }
        cx.notify();
    }

    /// Answer a pick from the Git screen's context menu. What each row does today: stage,
    /// unstage, and the copies — the rest are drawn disabled until the host has an operation
    /// behind them.
    pub fn pick_git_menu_action(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.git_view(cx).and_then(|git| git.menu.clone()) else {
            return;
        };
        let (stageable, unstageable) = match &menu.kind {
            GitMenuKind::Change { path, .. } => self
                .git_entries(cx)
                .unwrap_or(&[])
                .iter()
                .find(|entry| &entry.rel_path == path)
                .map(|entry| (can_stage(entry), can_unstage(entry)))
                .unwrap_or((false, false)),
            _ => (false, false),
        };
        let Some(entry) = menu.entries(stageable, unstageable).get(index).copied() else {
            return;
        };
        let epoch = menu.epoch;
        if !entry.enabled {
            self.dismiss_git_menu(epoch, cx);
            return;
        }

        match (&menu.kind, entry.action) {
            (GitMenuKind::Change { path, .. }, GitAction::Stage) => {
                self.stage_git_path(path, cx);
            }
            (GitMenuKind::Change { path, .. }, GitAction::Unstage) => {
                self.unstage_git_path(path, cx);
            }
            (GitMenuKind::Change { path, .. }, GitAction::CopyPath) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(path.clone()));
            }
            (GitMenuKind::Commit { index }, GitAction::CopySha) => {
                if let Some(id) = self
                    .git_view(cx)
                    .and_then(|git| git.commits.get(*index))
                    .map(|commit| commit.id.clone())
                {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(id));
                }
            }
            (GitMenuKind::Ref { index }, GitAction::CopyName) => {
                if let Some(name) = self
                    .git_view(cx)
                    .and_then(|git| git.refs.get(*index))
                    .map(|row| row.name.clone())
                {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(name));
                }
            }
            _ => {}
        }
        self.dismiss_git_menu(epoch, cx);
    }

    pub fn toggle_git_mine(&mut self, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.mine_only = !git.mine_only;
        }
        cx.notify();
    }

    pub fn clear_git_filters(&mut self, cx: &mut Context<Self>) {
        let had_branch = self
            .git_view(cx)
            .is_some_and(|git| git.branch_filter.is_some());
        if let Some(git) = self.git_view_mut(cx) {
            git.clear_filters();
        }
        if had_branch {
            self.send_git_log(None, cx);
        }
        cx.notify();
    }

    // ── Project search ──────────────────────────────────────────────

    /// Run what is in the query field over the project's files.
    ///
    /// The search id is minted here and held on `SearchState::active`: every reply is discarded
    /// unless it names it, which is what makes superseding a search a cancel and a replacement
    /// rather than two answers interleaving.
    pub fn run_project_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let text = self.search.query.read(cx).value().trim().to_string();
        // An empty query is not a search and not an error — nothing is sent and what is drawn
        // stays as it was.
        if text.is_empty() {
            return;
        }

        if let Some(active) = self.search.active.as_ref() {
            self.bus.send(Message::CancelSearch {
                project_id: active.project_id,
                search_id: active.search_id,
            });
        }

        let query = ubiq_proto::search::Query {
            text,
            case_sensitive: self.search.case_sensitive,
            whole_word: self.search.whole_word,
            regex: self.search.regex,
        };
        self.search.reset();
        let search_id = SearchId::generate();
        self.search.active = Some(ActiveSearch {
            search_id,
            project_id,
        });
        self.bus.send(Message::SearchProject {
            project_id,
            search_id,
            query,
            scope: ubiq_proto::search::Scope::Files,
            filter: ubiq_proto::search::Filter::default(),
        });
        cx.notify();
    }

    /// The titlebar's command field, on Enter: switch to the IDE, hand the typed text to the
    /// search panel's own query field, reveal it and run the search — then clear the field it
    /// came from, the way a command palette clears once its command has fired. A no-op with
    /// nothing open, on the same guard `run_project_search` already applies.
    pub fn submit_header_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.command_input.read(cx).value().trim().to_string();
        self.search_for(text, window, cx);
    }

    /// The same search, for a term the field no longer holds — the navigator's own search row
    /// carries it, and pressing that row is what runs this.
    pub fn search_for(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.project(cx).is_none() {
            return;
        }
        if text.is_empty() {
            return;
        }
        self.set_rail_mode(RailMode::IDE, cx);
        let query = self.search.query.clone();
        query.update(cx, |state, cx| state.set_value(&text, window, cx));
        self.reveal_search(window, cx);
        self.run_project_search(window, cx);
        let command_input = self.command_input.clone();
        command_input.update(cx, |state, cx| state.set_value("", window, cx));
    }

    /// The three query options. A flip applies to the next search, never re-running this one:
    /// re-searching under the pointer would throw away the list the user is working through.
    pub fn toggle_search_case(&mut self, cx: &mut Context<Self>) {
        self.search.case_sensitive = !self.search.case_sensitive;
        cx.notify();
    }

    pub fn toggle_search_whole_word(&mut self, cx: &mut Context<Self>) {
        self.search.whole_word = !self.search.whole_word;
        cx.notify();
    }

    pub fn toggle_search_regex(&mut self, cx: &mut Context<Self>) {
        self.search.regex = !self.search.regex;
        cx.notify();
    }

    /// Point the diff pane at a changed path, and ask the host for the comparison. Nothing is sent
    /// when the selection did not move: a second click on the row already being compared is not a
    /// second question.
    pub fn select_git_path(&mut self, side: GitSide, path: &str, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(git) = self.git_view_mut(cx) else {
            return;
        };
        let moved = git.select_path(side, path);
        let base = git.base;
        let (old, new) = if base == DiffBase::Commits {
            (git.range_from.clone(), git.range_to.clone())
        } else {
            (None, None)
        };
        if moved {
            self.bus.send(Message::DiffProjectFile {
                project_id,
                rel_path: path.to_string(),
                base,
                old,
                new,
            });
            self.pending_panels
                .push(PanelEdit::Reveal(PanelKind::GitDiff));
        }
        cx.notify();
    }

    pub fn set_git_split(&mut self, split: bool, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.split = split;
        }
        cx.notify();
    }

    pub fn toggle_git_diff_pane(&mut self, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.diff_open = !git.diff_open;
        }
        cx.notify();
    }

    pub fn toggle_git_amend(&mut self, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.amend = !git.amend;
        }
        cx.notify();
    }

    /// The Git screen's four fields hold the project on screen's text, on the explorer filter's
    /// rule: mirrored from the frame after a project swings in, and never while it is being typed
    /// into.
    pub fn sync_git_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((search, message, ref_search, change_search)) = self.git_view(cx).map(|git| {
            (
                git.search.clone(),
                git.message.clone(),
                git.ref_search.clone(),
                git.change_search.clone(),
            )
        }) else {
            return;
        };

        if !self.git_search.read(cx).focus_handle(cx).is_focused(window)
            && self.git_search.read(cx).value() != search.as_str()
        {
            let field = self.git_search.clone();
            field.update(cx, |state, cx| state.set_value(search, window, cx));
        }

        if !self
            .git_ref_query
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
            && self.git_ref_query.read(cx).value() != ref_search.as_str()
        {
            let field = self.git_ref_query.clone();
            field.update(cx, |state, cx| state.set_value(ref_search, window, cx));
        }

        if !self
            .git_change_query
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
            && self.git_change_query.read(cx).value() != change_search.as_str()
        {
            let field = self.git_change_query.clone();
            field.update(cx, |state, cx| state.set_value(change_search, window, cx));
        }

        // The commit box is the one field that is also mirrored while it is focused, and only
        // to empty it: a commit clears the draft it was written from, and the caret is
        // usually still in the box when its reply lands.
        if (message.is_empty()
            || !self
                .git_message
                .read(cx)
                .focus_handle(cx)
                .is_focused(window))
            && self.git_message.read(cx).value() != message.as_str()
        {
            let field = self.git_message.clone();
            field.update(cx, |state, cx| state.set_value(message, window, cx));
        }
    }
}
