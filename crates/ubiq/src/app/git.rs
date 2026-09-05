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
            // behind — `receive_git` tells the two apart by whether a cursor is already held.
            git.log_cursor = None;
        }
        self.bus.send(Message::ProjectGitLog {
            project_id,
            cursor: None,
            count: 100,
            rel_path: None,
            first_parent: false,
        });
        cx.notify();
    }

    pub fn toggle_git_section(&mut self, section: RefSection, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.toggle_section(section);
        }
        cx.notify();
    }

    pub fn select_git_ref(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.selected_ref = Some(index);
        }
        cx.notify();
    }

    /// Point the screen at a commit, or at the working tree. `None` is the uncommitted row, which
    /// is a selection like any other rather than nothing selected.
    pub fn select_git_commit(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.selected_commit = index;
        }
        cx.notify();
    }

    pub fn toggle_git_mine(&mut self, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.mine_only = !git.mine_only;
        }
        cx.notify();
    }

    pub fn clear_git_filters(&mut self, cx: &mut Context<Self>) {
        if let Some(git) = self.git_view_mut(cx) {
            git.clear_filters();
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
        if self.project(cx).is_none() {
            return;
        }
        let text = self.command_input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.set_rail_mode(RailMode::Ide, cx);
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
        if moved {
            self.bus.send(Message::DiffProjectFile {
                project_id,
                rel_path: path.to_string(),
                base,
            });
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

    /// The Git screen's two fields hold the project on screen's text, on the explorer filter's
    /// rule: mirrored from the frame after a project swings in, and never while it is being typed
    /// into.
    pub fn sync_git_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((search, message)) = self
            .git_view(cx)
            .map(|git| (git.search.clone(), git.message.clone()))
        else {
            return;
        };

        if !self.git_search.read(cx).focus_handle(cx).is_focused(window)
            && self.git_search.read(cx).value() != search.as_str()
        {
            let field = self.git_search.clone();
            field.update(cx, |state, cx| state.set_value(search, window, cx));
        }

        if !self
            .git_message
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
            && self.git_message.read(cx).value() != message.as_str()
        {
            let field = self.git_message.clone();
            field.update(cx, |state, cx| state.set_value(message, window, cx));
        }
    }
}
