use super::*;

use crate::state::navigator::{self, NavAction, NavRow, NavigatorState, kept_recents};

impl AppState {
    /// The place the window is drawing, if it is drawing one.
    ///
    /// A screen pointed at nothing — the graph with no selection, the board with no task — is not
    /// a place and answers `None`, and neither is the sink: it is the test bench and has no
    /// project behind it.
    pub fn current_destination(&self, cx: &App) -> Option<Destination> {
        let project = self.project(cx)?;
        let view = match self.workbench.rail_mode {
            RailMode::Control => View::Control,
            RailMode::Kb => View::Kb,
            RailMode::Git => View::Git,
            RailMode::Ide => View::Ide {
                key: self.editor(cx)?.active_file()?.key(),
            },
            RailMode::Orchestration => {
                let graph = self.graph(cx)?;
                View::Graph {
                    selection: graph.selection?,
                    tab: graph.tab,
                }
            }
            RailMode::Agents => {
                let agents = self.agents(cx)?;
                View::Agents {
                    agent: agents.columns.get(agents.focus)?.active_agent()?,
                }
            }
            RailMode::Tasks => View::Tasks {
                task: self.board(cx)?.selected?,
            },
            RailMode::Sink => return None,
        };
        let locus = self.where_locus(&view, cx);
        Some(Destination {
            project,
            view,
            locus,
        })
    }

    /// Go somewhere. The one call every link, card, bookmark and history press ends in.
    ///
    /// A project another window holds is not taken from it: that window arrives instead and comes
    /// to the front, so nothing moves between windows and this one's place is untouched.
    pub fn navigate(&mut self, dest: Destination, cx: &mut Context<Self>) {
        if self.project(cx) != Some(dest.project) {
            let holder = WindowRegistry::read(cx)
                .holder(dest.project)
                .map(|slot| slot.id)
                .filter(|id| *id != self.window_id);
            if let Some(id) = holder {
                if let Some(view) = OpenWindows::get(cx, id) {
                    view.update(cx, |state, cx| state.navigate(dest, cx));
                    focus_window(id, cx);
                }
                return;
            }
            self.activate_project(dest.project, cx);
        }

        if let Some(mode) = rail_of(&dest.view) {
            self.set_rail_mode(mode, cx);
        }
        match &dest.view {
            View::Control | View::Kb | View::Git => {}
            View::Ide { key } => self.reveal_ide(key, dest.locus.as_ref(), cx),
            View::Explorer { path } => self.reveal_explorer(path.clone(), cx),
            View::Terminal { pane } => {
                self.pending_panels
                    .push(PanelEdit::Reveal(PanelKind::Terminal(*pane)));
                self.focus_pane(*pane, cx);
            }
            View::Logs => self.pending_panels.push(PanelEdit::Reveal(PanelKind::Logs)),
            View::Chat { chat } => self
                .pending_panels
                .push(PanelEdit::Reveal(PanelKind::Chat(*chat))),
            View::Graph { selection, tab } => {
                self.reveal_graph(*selection, *tab, dest.locus.as_ref(), cx)
            }
            View::Agents { agent } => self.reveal_agent(*agent, cx),
            View::Tasks { task } => self.select_task(*task, cx),
        }
        // The one arrival point, so the one place recents are kept.
        if dest.persistable()
            && let Some(open) = self.projects.get_mut(&dest.project)
        {
            navigator::remember(&mut open.prefs.recents, &dest);
            self.store_prefs(dest.project);
        }
        cx.notify();
    }

    /// Put a place on the clipboard as its `ubiq://` text — the one form that survives a
    /// document, a chat message and tomorrow.
    pub fn copy_link(&self, dest: &Destination, cx: &mut App) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(dest.to_string()));
    }

    /// Remember where the window is drawing, once a frame.
    ///
    /// One push site rather than one per departure: a rail switch, a file open and a project
    /// change all change the drawn destination and push, while a caret move, a pan and a zoom
    /// change only the locus and update the entry in place. **No `cx.notify()`** — this runs
    /// inside `Render` and a notify from there spins the frame.
    pub(super) fn settle_nav(&mut self, cx: &mut Context<Self>) {
        let Some(dest) = self.current_destination(cx) else {
            return;
        };
        // A back or forward press is a return to an entry, not an arrival at a new place: the
        // spot it landed on is the one the user left, so it replaces what was recorded.
        if std::mem::take(&mut self.nav_settling) {
            if let Some(current) = self.nav.current_mut() {
                current.locus = dest.locus;
            }
            return;
        }
        self.nav.record(dest);
    }

    pub fn back(&mut self, _: &NavBack, _: &mut Window, cx: &mut Context<Self>) {
        let fate = self.fate_fn(cx);
        if let Some(dest) = self.nav.back(fate.as_ref()) {
            self.nav_settling = true;
            self.navigate(dest, cx);
        }
    }

    pub fn forward(&mut self, _: &NavForward, _: &mut Window, cx: &mut Context<Self>) {
        let fate = self.fate_fn(cx);
        if let Some(dest) = self.nav.forward(fate.as_ref()) {
            self.nav_settling = true;
            self.navigate(dest, cx);
        }
    }

    /// What history should make of each remembered place, as this window sees the registry now.
    ///
    /// Boxed and owning its two lists: the walk it is handed to needs `&mut self`, so it may not
    /// hold a borrow of either the registry or the window.
    fn fate_fn(&self, cx: &App) -> Box<dyn Fn(&Destination) -> Fate> {
        let registry = WindowRegistry::read(cx);
        let known: Vec<ProjectId> = registry.all().map(|snapshot| snapshot.record.id).collect();
        let elsewhere: Vec<ProjectId> = registry
            .windows
            .iter()
            .filter(|slot| slot.id != self.window_id)
            .flat_map(|slot| slot.projects.iter().copied())
            .collect();
        Box::new(move |dest: &Destination| {
            if !known.contains(&dest.project) {
                Fate::Gone
            } else if elsewhere.contains(&dest.project) {
                Fate::Elsewhere
            } else {
                Fate::Here
            }
        })
    }

    /// Where in the drawn screen the user is. Only the two screens that have an inside answer.
    fn where_locus(&self, view: &View, cx: &App) -> Option<Locus> {
        match view {
            View::Ide { .. } => self
                .cursor_line_column(cx)
                .map(|(line, _)| Locus::Line { line }),
            View::Graph { .. } => {
                let offset = self.graph_scroll.offset();
                Some(Locus::Viewport {
                    x: f32::from(offset.x),
                    y: f32::from(offset.y),
                    scale: self.graph(cx)?.zoom,
                })
            }
            _ => None,
        }
    }

    /// Bring a file forward and put the caret where the destination asked.
    ///
    /// A file with no buffer yet has nothing to put a caret in, so the spot is stashed and the
    /// arriving bytes take it, through the same `set_restore` a reload's caret uses.
    fn reveal_ide(&mut self, key: &str, locus: Option<&Locus>, cx: &mut Context<Self>) {
        let held = self
            .editor(cx)
            .is_some_and(|editor| editor.index_of_key(key).is_some());
        if held {
            self.activate_file(key, cx);
        } else {
            self.select_file(from_tab_key(key).0, cx);
        }
        let Some(locus) = locus else {
            return;
        };
        let buffer = self
            .editor(cx)
            .and_then(|editor| editor.open.get(editor.index_of_key(key)?))
            .and_then(|file| file.buffer())
            .cloned();
        let Some(buffer) = buffer else {
            self.pending_goto = Some((key.to_string(), locus.clone()));
            return;
        };
        let text = buffer.read(cx).value().to_string();
        // `set_selected_range` scrolls the caret in, which is what every vim motion relies on too.
        if let Some(range) = range_for(&text, locus) {
            buffer.update(cx, |state, cx| state.set_selected_range(range, cx));
        }
    }

    /// Point the tree at a path: the folders above it open, the cursor moves and the row scrolls
    /// into view — all of which [`Self::reveal_active_file`] already does for the selection.
    fn reveal_explorer(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.explorer.selected = Some(path);
        }
        self.reveal_active_file(cx);
    }

    /// Point the graph at a selection, on the inspector tab the destination named.
    fn reveal_graph(
        &mut self,
        selection: Selection,
        tab: InspectorTab,
        locus: Option<&Locus>,
        cx: &mut Context<Self>,
    ) {
        if let Some(graph) = self.graph_mut(cx) {
            graph.selection = Some(selection);
            graph.tab = tab;
            if let Some(Locus::Viewport { scale, .. }) = locus {
                graph.zoom = *scale;
            }
        }
        if let Some(Locus::Viewport { x, y, .. }) = locus {
            self.graph_scroll.set_offset(point(px(*x), px(*y)));
        }
        cx.notify();
    }
}

/// Which rail mode a view is drawn in. `None` for the three that are panels rather than screens:
/// they are revealed where they already sit, whatever mode is up.
pub fn rail_of(view: &View) -> Option<RailMode> {
    Some(match view {
        View::Control => RailMode::Control,
        View::Kb => RailMode::Kb,
        View::Git => RailMode::Git,
        View::Ide { .. } | View::Explorer { .. } => RailMode::Ide,
        View::Graph { .. } => RailMode::Orchestration,
        View::Agents { .. } => RailMode::Agents,
        View::Tasks { .. } => RailMode::Tasks,
        View::Terminal { .. } | View::Logs | View::Chat { .. } => return None,
    })
}

impl AppState {
    /// Open or shut the explorer's bookmarks section. Furniture: nothing is written down.
    pub fn toggle_bookmarks_section(&mut self, cx: &mut Context<Self>) {
        self.workbench.bookmarks_open = !self.workbench.bookmarks_open;
        cx.notify();
    }

    /// The places this project has written down.
    pub fn bookmarks(&self, cx: &App) -> &[Bookmark] {
        self.project(cx)
            .and_then(|project| self.projects.get(&project))
            .map(|open| open.prefs.bookmarks.as_slice())
            .unwrap_or_default()
    }

    /// How many bookmarks one file holds, for the count its tab wears.
    pub fn bookmark_count(&self, key: &str, cx: &App) -> usize {
        self.bookmarks(cx)
            .iter()
            .filter(|mark| matches!(&mark.dest.view, View::Ide { key: held } if held == key))
            .count()
    }

    /// Write down where the user is, or take it away if it is already written down.
    ///
    /// A place that will not survive a restart is not offered one — see
    /// [`Destination::persistable`] — and a line in an open file takes its own text with it, so
    /// the mark can find the line again after the file has been edited above it.
    pub fn toggle_bookmark(&mut self, _: &ToggleBookmark, _: &mut Window, cx: &mut Context<Self>) {
        let Some(dest) = self.current_destination(cx) else {
            return;
        };
        if !dest.persistable() {
            return;
        }
        let anchor = self.anchor_text(&dest, cx);
        let mark = Bookmark {
            name: dest.label(),
            dest: dest.clone(),
            note: String::new(),
            anchor,
            adrift: false,
        };
        if let Some(open) = self.projects.get_mut(&dest.project) {
            toggle_mark(&mut open.prefs.bookmarks, mark);
        }
        self.store_prefs(dest.project);
        if let View::Ide { key } = &dest.view {
            self.mark_bookmarks(&key.clone(), cx);
        }
        cx.notify();
    }

    /// The bookmarked line's own text, where the place is a line in a file whose bytes are here.
    fn anchor_text(&self, dest: &Destination, cx: &App) -> Option<String> {
        let View::Ide { key } = &dest.view else {
            return None;
        };
        let line = dest.line()?;
        let buffer = self.file(key, cx)?.buffer()?;
        let text = buffer.read(cx).value();
        let found: String = text
            .lines()
            .nth(line.checked_sub(1)? as usize)?
            .trim()
            .chars()
            .take(crate::state::nav::ANCHOR_CHARS)
            .collect();
        (!found.is_empty()).then_some(found)
    }

    /// Find one file's bookmarks in the bytes as they now read, and light the lines they landed on.
    ///
    /// A line found somewhere else is re-stamped and stored; **a line that cannot be found writes
    /// nothing** and is left marked adrift, because a bookmark quietly pointing at the wrong line
    /// is worse than one that says it has lost its place.
    pub(super) fn mark_bookmarks(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(buffer) = self.file(key, cx).and_then(|file| file.buffer()).cloned() else {
            return;
        };
        let text = buffer.read(cx).value().to_string();

        let mut lit: Vec<u32> = Vec::new();
        let mut moved = false;
        if let Some(open) = self.projects.get_mut(&project) {
            for mark in &mut open.prefs.bookmarks {
                let View::Ide { key: held } = &mark.dest.view else {
                    continue;
                };
                if held != key {
                    continue;
                }
                let Some(line) = mark.dest.line() else {
                    continue;
                };
                let found = match &mark.anchor {
                    Some(anchor) => resolve_anchor(&text, line, anchor),
                    // Nothing to look for: the number is all the bookmark ever had.
                    None => Anchored::Exact(line),
                };
                mark.adrift = matches!(found, Anchored::Adrift(_));
                if let Anchored::Moved(to) = found {
                    restamp(&mut mark.dest.locus, to);
                    moved = true;
                }
                if !mark.adrift {
                    lit.push(found.line());
                }
            }
        }
        if moved {
            self.store_prefs(project);
        }

        let marks: Vec<TextDecoration> = lit
            .into_iter()
            .map(|line| {
                TextDecoration::new(
                    crate::state::nav::line_range(&text, line, line),
                    gpui::HighlightStyle {
                        background_color: Some(crate::theme::accent_soft().into()),
                        ..Default::default()
                    },
                )
            })
            .collect();
        // One collection per buffer, kept: the library hands a collection out once and it lives
        // as long as the buffer it was made from. A reload is a new buffer and so a new one.
        let id = buffer.entity_id();
        let collection = match self.bookmark_marks.get(&id) {
            Some(collection) => collection.clone(),
            None => {
                let collection = buffer.update(cx, |state, cx| {
                    state.create_decorations_collection(Vec::new(), cx)
                });
                self.bookmark_marks.insert(id, collection.clone());
                collection
            }
        };
        collection.set(marks, cx);
    }
}

/// Move a locus onto a line, a span keeping its length.
fn restamp(locus: &mut Option<Locus>, line: u32) {
    match locus {
        Some(Locus::Line { line: at }) => *at = line,
        Some(Locus::Span { from, to }) => {
            let span = to.saturating_sub(*from);
            *from = line;
            *to = line + span;
        }
        _ => {}
    }
}

impl AppState {
    /// Raise the navigator over the titlebar's field, with the caret in it. Whatever is already
    /// typed there is the query it opens on.
    pub fn open_navigator(
        &mut self,
        _: &OpenNavigator,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.command_input.read(cx).value().to_string();
        self.navigator = Some(NavigatorState { query, cursor: 0 });
        let field = self.command_input.read(cx).focus_handle(cx);
        window.focus(&field, cx);
        cx.notify();
    }

    /// Take the list away. **The text is left alone**: closing the list is not undoing the typing,
    /// and the field is the project search again the moment the navigator is shut.
    pub fn close_navigator(&mut self, cx: &mut Context<Self>) {
        self.navigator = None;
        cx.notify();
    }

    /// The field changed under an open navigator: new answers, and the cursor back to the top.
    pub(super) fn retype_navigator(&mut self, query: String, cx: &mut Context<Self>) {
        let Some(nav) = &mut self.navigator else {
            return;
        };
        nav.query = query;
        nav.cursor = 0;
        cx.notify();
    }

    /// Walk the list. It is flat, so there is nothing but up and down.
    pub fn move_navigator(&mut self, down: bool, cx: &mut Context<Self>) {
        let last = self.navigator_rows(cx).len().saturating_sub(1);
        if let Some(nav) = &mut self.navigator {
            nav.cursor = match down {
                true => (nav.cursor + 1).min(last),
                false => nav.cursor.saturating_sub(1),
            };
        }
        cx.notify();
    }

    /// Act on one row. A row that names no place — a link to a project the catalogue does not
    /// hold — says so and does nothing, so the list stays up rather than closing on a dead press.
    pub fn press_navigator(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // An action is checked first: a row that carries one never carries a destination too, and
        // the navigator only knows how to navigate.
        if let Some(action) = self
            .navigator_rows(cx)
            .get(index)
            .and_then(|row| row.action.clone())
        {
            self.navigator = None;
            self.command_input
                .clone()
                .update(cx, |state, cx| state.set_value("", window, cx));
            match action {
                NavAction::Clone(url) => self.open_clone(Some(url), window, cx),
            }
            return;
        }
        let Some(dest) = self
            .navigator_rows(cx)
            .get(index)
            .and_then(|row| row.dest.clone())
        else {
            return;
        };
        self.navigator = None;
        self.command_input
            .clone()
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.navigate(dest, cx);
    }

    /// What the navigator is offering, as the window's own lists read now.
    ///
    /// **Files are the explorer's own answer**, filtered on the frame over what the host has
    /// already named — with the panel's three-character floor, and deepening as the host's
    /// background walk lands, exactly as the panel does. Only the tree that is loaded can match,
    /// which is the honest answer for a folder nobody has opened.
    pub fn navigator_rows(&self, cx: &App) -> Vec<NavRow> {
        let (Some(nav), Some(project), Some(open)) =
            (&self.navigator, self.project(cx), self.open_project(cx))
        else {
            return Vec::new();
        };

        let recents = kept_recents(&open.prefs.recents);
        let files: Vec<(String, String, bool)> =
            match nav.query.trim().chars().count() < crate::app::MIN_QUERY {
                true => Vec::new(),
                false => open
                    .explorer
                    .rows(&nav.query)
                    .into_iter()
                    // The project's own row carries the empty path and is not a place to go.
                    .filter(|row| !row.path.is_empty())
                    .map(|row| (row.name, row.path, row.is_dir))
                    .collect(),
            };
        let tasks: Vec<(TaskId, String)> = open
            .work
            .tasks
            .iter()
            .map(|task| (task.id, task.title.clone()))
            .collect();
        let agents: Vec<(AgentId, String, String)> = open
            .work
            .agents
            .iter()
            .map(|agent| (agent.id, agent.name.clone(), agent.role.clone()))
            .collect();

        let registry = WindowRegistry::read(cx);
        let name_of = |id: ProjectId| {
            registry
                .project(id)
                .map(|snapshot| snapshot.record.name.clone())
        };
        navigator::rows(
            &nav.query,
            project,
            &recents,
            &open.prefs.bookmarks,
            &files,
            &tasks,
            &agents,
            &name_of,
        )
    }
}
