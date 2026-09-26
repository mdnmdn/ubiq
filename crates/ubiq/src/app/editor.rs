use super::*;

impl AppState {
    /// One open tab of the project on screen, by its key. What a file panel draws and what its tab
    /// reports are both this.
    pub fn file(&self, key: &str, cx: &App) -> Option<&OpenFile> {
        self.editor(cx)?.open.iter().find(|file| file.key() == key)
    }

    /// Put one file's viewer into one of its layouts.
    ///
    /// **This is a per-document override and it is never written down** (T-202): it lives on the
    /// `OpenFile` and dies with the tab. What persists is the *default* a document opens in, which
    /// is the `markdown_open` setting and is changed in settings, not here.
    ///
    /// The panel repeats the fact rather than owning it — `settle_visibility` pushes it every
    /// frame — but it is pushed here too, so the click redraws in the new layout on this frame
    /// rather than the next.
    pub fn set_view_layout(&mut self, key: &str, layout: ViewLayout, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        // A knowledge-base document is the same `OpenFile` under a key of its own, held in the
        // other half of the project — so the lookup falls through to it and the panel it pushes
        // the settled layout onto is its own kind.
        let (file, kind) = match open.editor.find_key_mut(key) {
            Some(file) => (file, PanelKind::File(key.to_string())),
            None => match open.kb.doc_mut(key) {
                Some(doc) => (doc, PanelKind::Kb(key.to_string())),
                None => return,
            },
        };
        file.set_layout(layout);
        // What the file *took*, not what was asked: a viewer refuses a layout it does not offer,
        // and a panel told the asked-for one would write it into the arrangement anyway.
        let settled = file.layout;

        let panel = self.panels.get(&kind).cloned();
        if let Some(panel) = panel {
            panel.update(cx, |panel, _| panel.set_layout(settled));
        }
        // No `remember_view` here: since T-202 nothing about this is in the saved arrangement, so
        // there is nothing for a toggle to write down.
        cx.notify();
    }

    /// The annotated document open for this tab, if the one document the window holds is this
    /// file's and is the tab's rather than the dialog's.
    ///
    /// The handle is rebuilt from the open document's own project id rather than from the window's
    /// current one, so a tab left over from a project switch cannot match by path alone.
    pub fn annotation_document(
        &self,
        file: &OpenFile,
    ) -> Option<&crate::state::document::DocumentEditor> {
        let doc = self.workbench.plan.as_ref()?;
        if doc.is_modal() || !file.annotatable() {
            return None;
        }
        (doc.doc == crate::state::plan::file_document(doc.project_id(), file.path.clone()))
            .then_some(doc)
    }

    /// Whether a markdown tab's file is annotated — what the header's annotation badge and the
    /// source-mode warning both read.
    ///
    /// **Three answers, most exact first.** A document actually open in the annotation surface
    /// answers from its own loaded threads. One that is not — which a tab showing raw source
    /// always is, since that is exactly what closes it (`close_file_document`, T-124) — answers
    /// from `AppState::annotation_hints` instead: the real count the host last stated for this
    /// path, in *this* window, kept after the surface closed rather than only while it was open.
    /// **Sidecar presence is not this answer, and cannot be** (T-183): the sidecar's block index
    /// has to be written whether or not anything is annotated — a second call re-matching a
    /// `BlockId` against nothing on disk would re-mint every id and refuse the very first
    /// annotation ever made — so a file's sidecar existing has never told the truth about whether
    /// it carries a thread. A path this window has not asked the host about at all — no tab has
    /// opened it in the annotation surface this session — falls through to the explorer's own
    /// presence check, which is a hint nobody can confirm and is better a guess than nothing: it
    /// costs no round trip and is the one case an over-read cannot yet have been recorded to
    /// correct.
    ///
    /// A folder nobody has listed yet answers `false`: the badge is a hint, and a hint nobody can
    /// confirm is better absent than guessed.
    pub fn has_annotations(&self, file: &OpenFile, cx: &App) -> bool {
        if !file.annotatable() {
            return false;
        }
        if let Some(doc) = self.annotation_document(file)
            && matches!(
                doc.annotations,
                crate::state::document::AnnotationsBody::Loaded { .. }
            )
        {
            return !doc.annotations.annotations().is_empty();
        }
        if let Some(known) = self
            .open_project(cx)
            .and_then(|open| open.annotation_hints.get(&file.path))
        {
            return *known;
        }
        self.explorer(cx)
            .map(|explorer| explorer.presence(&file.annotation_sidecar()))
            .is_some_and(|presence| presence == crate::state::explorer::Presence::Live)
    }

    /// Which layout a markdown file opens in: the `markdown_open` setting, **unless the file is
    /// already annotated**, in which case it opens in `Preview` (T-124).
    ///
    /// A source buffer is where an edit can orphan a thread, so a document carrying threads is not
    /// the one to drop somebody into the raw text of by default. Read from the explorer's tree —
    /// see [`Self::has_annotations`] — so it costs nothing and needs no round trip; a file whose
    /// folder has not been listed simply takes the setting, which is the honest answer when
    /// nothing is known.
    pub fn markdown_open(&self, path: &str, cx: &App) -> ViewLayout {
        let setting = self.workbench.settings.ui.markdown_open.layout();
        if setting == ViewLayout::Preview
            || crate::state::editor::ViewerKind::of(path) != ViewerKind::Markdown
        {
            return setting;
        }
        let annotated = self
            .explorer(cx)
            .map(|explorer| explorer.presence(&format!("{path}.annotation.json")))
            .is_some_and(|presence| presence == crate::state::explorer::Presence::Live);
        if annotated {
            ViewLayout::Preview
        } else {
            setting
        }
    }

    /// Draw one open tab with a different viewer than its extension asked for.
    ///
    /// The new kind may offer none of the layouts the old one did, so the layout is re-settled
    /// here rather than left to fail silently: a layout the new kind does not offer is replaced by
    /// the first one it does, and a kind with no toggle at all keeps whatever is there, which is
    /// what `has_preview` already makes harmless. The settled value goes to the panel by the same
    /// path [`Self::set_view_layout`] uses, and for the same reason.
    ///
    /// The forced kind also settles `language` — [`ViewerKind::forced_language`] — and pushes it
    /// onto the buffer's own highlighter. `language` alone would leave the source unhighlighted:
    /// the buffer's `EditorState` is built once, at `attach_file`, with the language baked in as a
    /// tree-sitter grammar, so a fact changed only on `OpenFile` is a fact the already-open buffer
    /// never sees. `set_highlighter` is `gpui-component`'s own hook for exactly this — a language
    /// picked after the buffer exists.
    ///
    /// **Nothing here is written down** — not into `ViewPrefs`, not into the dock's payload. A
    /// viewer the user forced on for one sitting is a way of looking at the file now, not a fact
    /// about the file, so a tab closed and reopened starts from `ViewerKind::of` again. For the
    /// same reason `OpenFile::retarget` re-derives the viewer on a rename or a save-as: the tab
    /// is then pointed at a different path, and the new path's own extension is the better answer.
    pub fn set_viewer_kind(&mut self, key: &str, viewer: ViewerKind, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        file.viewer = viewer;
        if let Some(language) = viewer.forced_language() {
            file.language = language;
        }
        if !viewer.offers(file.layout)
            && let Some(first) = viewer.layouts().first().copied()
        {
            file.layout = first;
        }
        let settled = file.layout;
        let language = file.language;
        let buffer = file.buffer().cloned();

        if let Some(buffer) = buffer {
            buffer.update(cx, |state, cx| {
                state.set_highlighter(ui::editor::highlighter_language(language), cx);
            });
        }

        let panel = self.panels.get(&PanelKind::File(key.to_string())).cloned();
        if let Some(panel) = panel {
            panel.update(cx, |panel, _| panel.set_layout(settled));
        }
        cx.notify();
    }

    /// Open the status bar's viewer picker, with its filter empty.
    ///
    /// Every searchable picker shares one buffer, so it is cleared on the way in — otherwise this
    /// menu opens holding whatever was typed into the last one.
    pub fn open_viewer_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker_search = self.picker_search.clone();
        picker_search.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.open_menu(MenuId::ViewerKind, cx);
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

    /// Set the Markdown preview's text-column width preset. Global to the window, not per file
    /// type (proposal §4.1's per-file-type memory is a later card) — and persisted through
    /// `UiSettings.md_width` (T-118), the gap this card closed.
    pub fn set_md_width(&mut self, width: crate::theme::MdWidth, cx: &mut Context<Self>) {
        self.workbench.settings.ui.md_width = width;
        self.remember_settings();
        cx.notify();
    }

    /// Set the Markdown preview's density mode. Same scope and persistence note as
    /// [`AppState::set_md_width`].
    pub fn set_md_density(&mut self, density: crate::theme::MdDensity, cx: &mut Context<Self>) {
        self.workbench.settings.ui.md_density = density;
        self.remember_settings();
        cx.notify();
    }

    /// Raise the zoom modal on one picture (T-185) — a corner button on a scaled-down image or
    /// diagram, wherever a markdown preview draws one.
    pub fn open_image_zoom(
        &mut self,
        target: crate::state::zoom::ImageZoom,
        cx: &mut Context<Self>,
    ) {
        self.workbench.image_zoom = Some(target);
        cx.notify();
    }

    /// Put the zoom modal away. The camera it was showing is left in the viewport cache under its
    /// own key exactly as any other panel's is — reopening the same picture resumes where the
    /// reader left it, on the same terms a diagram panel does.
    pub fn close_image_zoom(&mut self, cx: &mut Context<Self>) {
        self.workbench.image_zoom = None;
        cx.notify();
    }

    /// The zoom modal's own Copy button: the picture's bytes, straight to the platform clipboard.
    pub fn copy_zoomed_image(&mut self, cx: &mut Context<Self>) {
        if let Some(zoom) = &self.workbench.image_zoom {
            cx.write_to_clipboard(gpui::ClipboardItem::new_image(&zoom.image));
        }
    }

    /// The reading-options popover's character-size slider (T-188), one tab's own in-memory
    /// setting — see [`crate::state::editor::MdReading`].
    pub fn set_md_char_scale(&mut self, key: &str, scale: f32, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        file.md_reading.char_scale = scale.clamp(
            crate::state::editor::MD_CHAR_SCALE_MIN,
            crate::state::editor::MD_CHAR_SCALE_MAX,
        );
        cx.notify();
    }

    /// The reading-options popover's text-colour picker (T-188), on the same per-tab terms as
    /// [`Self::set_md_char_scale`].
    pub fn set_md_text_shade(
        &mut self,
        key: &str,
        shade: crate::state::editor::TextShade,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        file.md_reading.text_shade = shade;
        cx.notify();
    }

    /// The reading-options popover's "make default, system-wide" button (T-188): this tab's own
    /// character size and text-colour shade, written into `UiSettings` as what a newly opened
    /// document starts from — through the same `SetSettings { layer: Ui }` every other UI
    /// preference already persists by, since that layer is exactly the interface-owned, opaque-to
    /// the-host blob this is. `MdReading` itself stays in memory, per tab; only these two numbers
    /// cross to the host.
    ///
    /// **Per-project is not this method.** A per-project default needs a settings record scoped
    /// to a project, and no message in `ubiq-proto` carries one today — seeing this through would
    /// mean inventing one, which this card stops short of. See the doc comment on
    /// `ui::viewer::md_options` for what such a message would have to carry.
    pub fn make_md_reading_default(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(reading) = self.file(key, cx).map(|file| file.md_reading) else {
            return;
        };
        self.workbench.settings.ui.md_char_scale_default = reading.char_scale;
        self.workbench.settings.ui.md_text_shade_default = reading.text_shade;
        self.remember_settings();
        cx.notify();
    }

    /// The markdown viewer header's heading navigator (T-124). Opened rather than toggled, the
    /// window's one rule for every anchored panel.
    pub fn open_md_navigator(&mut self, cx: &mut Context<Self>) {
        self.open_menu(MenuId::MdNavigator, cx);
    }

    /// A heading picked in that navigator: scroll the tab's own preview to it, and close the list
    /// the way any dropdown row does.
    ///
    /// Proportional, for [`Self::select_md_minimap_mark`]'s own reason — the standard viewer draws
    /// one `TextView`, so a heading has no measured position of its own to jump to.
    pub fn select_md_nav_heading(&mut self, key: &str, index: usize, cx: &mut Context<Self>) {
        self.close_menu(cx);
        self.select_md_minimap_mark(key, index, cx);
    }

    /// A mark picked on the standard viewer's markdown minimap (T-118): scroll that tab's own
    /// document by the same proportion down as the heading sits at.
    ///
    /// **Proportional, not pixel-exact** — the standard viewer draws one `TextView` rather than a
    /// block per heading (`ui/plan.rs`'s own minimap has real block bounds to jump to; this one
    /// does not), so the mark's `fraction` from `markdown::heading_marks` is reused directly as
    /// the scroll position's own fraction of `ScrollHandle::max_offset`.
    pub fn select_md_minimap_mark(&mut self, key: &str, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        let FileBody::Text { state, .. } = &file.body else {
            return;
        };
        let source = state.read(cx).value().to_string();
        let Some(mark) = crate::ui::viewer::markdown::heading_marks(key, &source)
            .get(index)
            .map(|m| m.fraction)
        else {
            return;
        };
        let max_offset = file.md_scroll.max_offset();
        file.md_scroll
            .set_offset(gpui::point(px(0.), -max_offset.y * mark));
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
            let mut file = OpenFile::pending_on(&path, subject);
            file.pinned = view.pinned_files.iter().any(|pinned| pinned == key);
            open.editor.open.push(file);
        }

        // Untitled buffers have no path to reread, so they are replayed through the same
        // machinery ⌘N uses: a name and bytes handed straight to the arrival queue, no host round
        // trip. A name already open is not opened twice.
        let scratch: Vec<prefs::Scratch> = view
            .scratch
            .iter()
            .filter(|s| open.editor.index_of(&s.name).is_none())
            .cloned()
            .collect();
        for scratch in scratch {
            let len = scratch.text.len() as u64;
            self.push_untitled(
                project,
                scratch.name,
                FileContents {
                    bytes: scratch.text.into_bytes(),
                    len,
                    truncated: false,
                    is_binary: false,
                    version: None,
                },
                cx,
            );
        }

        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        // `push_untitled` above moved `active` to whichever scratch tab it opened last; the saved
        // active file — real or scratch — is what belongs in front.
        if let Some(active) = &view.active_file
            && let Some(at) = index_of_key(&open.editor, active)
        {
            open.editor.active = at;
        }
        open.explorer.selected = view.selected.clone();
        open.wanted = view.expanded.clone();
        open.board.shut = view.board_shut.clone();
        open.board.popup = view.board_popup;
        open.teams.hide_done = view.teams_hide_done;

        // Each tab is a panel. A saved arrangement usually carries them and the queued edits are
        // then no-ops, but one that was discarded — a stale version, an unreadable blob — must
        // still leave the files somewhere to be drawn. Untitled tabs are excluded: `push_untitled`
        // above already queued their panel and their bytes, and a `ReadProjectFile` here would ask
        // the host for a path that names nothing on disk.
        let tabs: Vec<(String, String, Subject)> = open
            .editor
            .open
            .iter()
            .filter(|file| !file.untitled)
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
                    old: None,
                    new: None,
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
        if let Some(path) = open.editor.active_path() {
            open.explorer.selected = Some(path);
        }
        self.follow_active_file(cx);
        self.remember(project, cx);
        cx.notify();
    }

    /// Close a tab. One holding unsaved changes asks first, in the window's one file dialog —
    /// which is where Enter and Escape already answer.
    pub fn close_editor_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(file) = open.editor.open.get(index) else {
            return;
        };
        let key = file.key();
        if file.dirty() {
            self.workbench.file_dialog = Some(FileDialog::DiscardChanges { key });
            cx.notify();
            return;
        }
        self.force_close_tab(&key, cx);
    }

    /// Drop a tab and its panel, unsaved edit and all. The one path that discards a buffer: every
    /// close that could lose work asks first and lands here on a yes.
    pub(super) fn force_close_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            return;
        };
        open.editor.close(at);
        // A web panel is a session over this buffer, so it goes with the buffer. The browser
        // window may still be on screen; nothing it posts is read from here on.
        self.close_web_session(key);
        // The tab and its panel go together, and the panel leaves through a `Window` this does not
        // have — so it queues, like every other edit to the dock.
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::File(key.to_string())));
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
        if let Some(path) = open.editor.active_file().map(|file| file.path.clone()) {
            open.explorer.selected = Some(path);
        }
        self.follow_active_file(cx);
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
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            // The tab went first — `close_editor_tab` — and this is the panel following it out.
            self.panels.remove(&PanelKind::File(key.to_string()));
            return;
        };

        if open.editor.open[at].dirty() {
            self.workbench.file_dialog = Some(FileDialog::DiscardChanges {
                key: key.to_string(),
            });
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::File(key.to_string())));
            cx.notify();
            return;
        }

        if let Some(open) = self.projects.get_mut(&project) {
            open.editor.close(at);
        }
        self.close_web_session(key);
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
    /// index still to come). Dirty ones are only asked for, by [`Self::close_editor_tab`]. A
    /// pinned tab is never in the set at all — every bulk close routes through here, so this is
    /// the one place that has to know a pin blocks a close.
    pub(super) fn close_editor_tabs_filtered(
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
                    let file = &open.editor.open[ix];
                    !file.pinned && keep(ix, &file.key())
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

    /// Send one text tab's write, `WriteHostFile` for a guest tab (an absolute path, no project
    /// to name) and `WriteProjectFile` otherwise — the branch `OpenFile::guest`'s own doc points
    /// at. `savable()` already required a version before either save path called this, so `guest`
    /// with no `expected` is unreached; skipping rather than sending a message the host would
    /// refuse anyway costs nothing and needs no `unwrap`.
    fn send_file_write(
        &self,
        project: ProjectId,
        guest: bool,
        path: String,
        bytes: Vec<u8>,
        expected: Option<FileVersion>,
    ) {
        if guest {
            let Some(expected) = expected else {
                return;
            };
            self.bus.send(Message::WriteHostFile {
                path,
                bytes,
                expected,
            });
        } else {
            self.bus.send(Message::WriteProjectFile {
                project_id: project,
                rel_path: path,
                bytes,
                expected,
                overwrite: false,
            });
        }
    }

    /// Write the file behind one tab — not just the active one — back, so a context menu can save
    /// the tab it was opened on. The save is `save_active_file`'s, with the file named by its tab
    /// key instead of by whatever tab happens to be on screen.
    pub fn save_file(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
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
        // An untitled buffer names nothing on disk, so the save is a question first.
        if file.untitled {
            let key = file.key();
            self.ask_save_as(key, window, cx);
            return;
        }
        // A buffer that cannot be written says why, rather than swallowing the keystroke.
        if let Some(refusal) = file.save_refusal() {
            let (key, refusal) = (file.key(), refusal.to_string());
            self.refuse_save(key, refusal, cx);
            return;
        }
        // A capture writes its flatten, not text.
        if file.image_edit().is_some() {
            self.save_image_file(project, key, cx);
            return;
        }
        let Some(buffer) = file.buffer() else {
            return;
        };
        let text = buffer.read(cx).value().to_string();
        let (path, expected, guest) = (file.path.clone(), file.version(), file.guest);
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&path)
        {
            file.mark_saving(text.clone());
        }
        self.send_file_write(project, guest, path, text.into_bytes(), expected);
        cx.notify();
    }

    /// Open the right-click menu on a tab, anchored where the button went down.
    ///
    /// The dock's tab bar draws the tab whose kind this is and hands the click across the
    /// renderer seam; the menu itself is painted here, over the window, so it is a fact about
    /// `AppState` rather than about the dock.
    pub fn open_tab_menu(&mut self, kind: PanelKind, at: (f32, f32), cx: &mut Context<Self>) {
        if self.workbench.open_menu != Some(MenuId::Explorer) && self.workbench.open_menu.is_some()
        {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::Tab);
        self.workbench.tab_menu = Some((kind, at));
        cx.notify();
    }

    /// Act on one row of the open tab menu, by the row's index.
    ///
    /// The row at that index is read off `ui::tab_menu::rows(&kind, pinned)` — the same call the
    /// frame drew from — rather than a hardcoded position, because Pin/Unpin and the suppressed
    /// Close row mean two tabs of the same kind can offer different-length menus; a literal index
    /// table would have to know that shape twice.
    pub fn pick_tab_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((kind, _)) = self.workbench.tab_menu.clone() else {
            return;
        };
        self.workbench.open_menu = None;
        self.workbench.tab_menu = None;
        let pinned = self.tab_pinned(&kind, cx);
        let restartable = self.tab_restartable(&kind, cx);
        let Some(&row) = ui::tab_menu::rows(&kind, pinned, restartable).get(index) else {
            return;
        };
        match &kind {
            PanelKind::File(key) => {
                let key = key.clone();
                match row {
                    "Close" => self.close_editor_tab_at_key(&key, cx),
                    "Close Others" => self.close_editor_tabs_except(&key, cx),
                    "Close Left" => self.close_editor_tabs_left(&key, cx),
                    "Close Right" => self.close_editor_tabs_right(&key, cx),
                    "Close All" => self.close_all_editor_tabs(cx),
                    "Copy Full Path" => self.copy_full_path_for_tab(&key, cx),
                    "Copy link" => self.copy_link_for_tab(&key, cx),
                    "Open in Finder" => self.open_in_finder_for_tab(&key, cx),
                    "Save" => self.save_file(&key, window, cx),
                    "Word Wrap" => self.toggle_editor_wrap(window, cx),
                    "Pin" | "Unpin" => self.toggle_tab_pin(kind, cx),
                    _ => {}
                }
            }
            PanelKind::Kb(key) => {
                let key = key.clone();
                match row {
                    "Close" => self.close_kb_doc(&key, cx),
                    "Save" => self.save_kb_doc(&key, cx),
                    "Pin" | "Unpin" => self.toggle_tab_pin(kind, cx),
                    _ => {}
                }
            }
            // Terminal and chat tabs share the whole menu: name it, put it away, end it, or pin
            // it against the putting away. See `ui::tab_menu` for why Hide and Close are two rows
            // rather than one, and why pinning suppresses only the first.
            //
            // **Neither close goes through `ui::dock::close_panel`.** This runs inside an
            // `AppState` update, and that function takes a second lease on the same entity to
            // reach the dock — a circular lease, which panics rather than closing anything. The
            // state path queues a `PanelEdit` instead, which is what every other close in the
            // window already does.
            PanelKind::Terminal(pane_id) => match row {
                "Rename…" => self.open_rename_tab(kind, window, cx),
                "Restart" => self.restart_pane_tool(*pane_id, cx),
                "Hide" => self.detach_pane(*pane_id, cx),
                "Close" => self.ask_close_pane(*pane_id, cx),
                "Pin" | "Unpin" => self.toggle_tab_pin(kind, cx),
                _ => {}
            },
            PanelKind::Chat(id) => match row {
                "Rename…" => self.open_rename_tab(kind, window, cx),
                "Hide" => self.close_chat_tab(*id, cx),
                // The conversation's own Delete, raised from the tab that is looking at it. The
                // tab stays up: the confirm is painted *inside* the conversation panel, so hiding
                // first would ask a question nothing draws. A tab attached to nothing has no
                // conversation to end, and its Close is the hide.
                "Close" => match self.chat_tab_agent(*id, cx) {
                    Some(agent_id) => {
                        self.workbench.confirm_end_conversation = Some(agent_id);
                    }
                    None => self.close_chat_tab(*id, cx),
                },
                "Pin" | "Unpin" => self.toggle_tab_pin(kind, cx),
                _ => {}
            },
            _ => {}
        }
        cx.notify();
    }

    /// Raise the rename dialog on a terminal or a chat tab, seeded with its current label.
    fn open_rename_tab(&mut self, kind: PanelKind, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.tab_current_name(&kind, cx);
        self.open_file_dialog(
            FileDialog::RenameTab {
                kind,
                current: current.clone(),
            },
            &current,
            window,
            cx,
        );
    }

    /// What a terminal or a chat tab is called right now, typed override included — the same
    /// label `ui::dock::WorkbenchPanel::tab` would compute before it is truncated for the strip.
    fn tab_current_name(&self, kind: &PanelKind, cx: &App) -> String {
        if let Some(name) = self.tab_name(kind) {
            return name.to_string();
        }
        match kind {
            PanelKind::Terminal(pane_id) => self
                .pane(*pane_id)
                .map(|pane| pane.title.clone())
                .unwrap_or_else(|| "pane".to_string()),
            PanelKind::Chat(id) => {
                let attached = self
                    .open_project(cx)
                    .and_then(|open| open.chats.iter().find(|tab| tab.id == *id))
                    .and_then(|tab| tab.attached);
                attached
                    .and_then(|agent| self.work(cx)?.agent(agent))
                    .map(|agent| agent.name.clone())
                    .unwrap_or_else(|| "New chat".to_string())
            }
            _ => String::new(),
        }
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

    /// Copy the tab's `ubiq://` link: the file itself, no line — a tab is not a spot in one.
    fn copy_link_for_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let dest = Destination {
            project,
            view: View::Ide {
                key: key.to_string(),
            },
            locus: None,
        };
        self.copy_link(&dest, cx);
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

    /// Open the titlebar's run chevron, anchored where it was clicked: every runnable tool this
    /// project offers.
    ///
    /// The list is asked for again here for [`Self::open_new_pane_menu`]'s reason: a tool added
    /// in the settings since the window opened is offered without a restart. Whatever is already
    /// known is what this frame draws; the answer replaces it.
    pub fn open_run_tool_menu(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::RunTool);
        self.workbench.run_tool_menu = Some(at);
        self.bus.send(Message::ListTools {
            project_id: self.project(cx),
        });
        cx.notify();
    }

    /// Act on one row of the open run menu, by the row's index — the tool at that place in
    /// `WorkbenchState::run_tool_rows`, which is the same list the menu drew from.
    pub fn pick_run_tool_menu(
        &mut self,
        index: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench.open_menu = None;
        self.workbench.run_tool_menu = None;
        if let Some(&at) = self.workbench.run_tool_rows().get(index) {
            self.run_tool_at(at, cx);
        }
        cx.notify();
    }

    /// Dismiss the run menu — an outside click, or a pick already taken it.
    pub fn dismiss_run_tool_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.run_tool_menu = None;
        cx.notify();
    }

    /// Run the tool at one index of `WorkbenchState::tools`, if it is one this host can run.
    ///
    /// The one place a listed row becomes a `RunTool`: the play button and the menu both come
    /// through here, so neither can start a tool the other would refuse.
    pub fn run_tool_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(listed) = self.workbench.tools.get(index) else {
            return;
        };
        if !listed.applicable {
            return;
        }
        let (scope, id) = (listed.scope.clone(), listed.tool.id);
        self.run_tool(scope, id, cx);
    }

    /// The titlebar's play triangle: run the first tool this project offers. Nothing happens when
    /// it offers none — the chevron beside it is what says so.
    pub fn run_first_tool(&mut self, cx: &mut Context<Self>) {
        if let Some(&first) = self.workbench.run_tool_rows().first() {
            self.run_tool_at(first, cx);
        }
    }

    /// Act on one row of the open new-pane menu, by the row's index.
    ///
    /// A detached row brings a still-running pane's panel back, the same list
    /// `ui::new_pane_menu::overlay` read to draw it. A shell row starts a pane running that shell
    /// — the same call the "+" makes, with a program on it. Past the last one is the separator,
    /// which is a row and does nothing, and then the console, which is revealed rather than
    /// started. A runnable tool is not a row here: it is the titlebar's run control's, through
    /// [`Self::pick_run_tool_menu`].
    pub fn pick_new_pane_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench.open_menu = None;
        self.workbench.new_pane_menu = None;
        let has_project = self.project(cx).is_some();
        let detached = self.detached_panes(cx);
        match self
            .workbench
            .new_pane_rows(has_project, detached.len())
            .get(index)
        {
            Some(NewPaneRow::Detached(at)) => {
                if let Some(&pane_id) = detached.get(*at) {
                    self.reattach_pane(pane_id, cx);
                }
            }
            Some(NewPaneRow::DetachedHeading) => {}
            Some(NewPaneRow::Shell(shell)) => {
                let Some(program) = self
                    .workbench
                    .shells
                    .get(*shell)
                    .map(|shell| shell.program.clone())
                else {
                    return;
                };
                self.spawn_pane(Some(program), Vec::new(), AgentPicks::default(), cx);
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

    /// Open the titlebar's overflow chevron menu, anchored where the button went down.
    ///
    /// What it offers — remote connect, web export, window capture, settings — used to be four
    /// separate controls on the strip; folding them behind a chevron is the same trade
    /// [`Self::open_new_pane_menu`] makes for the dock's own.
    pub fn open_overflow_menu(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::Overflow);
        self.workbench.overflow_menu = Some(at);
        cx.notify();
    }

    /// Act on one row of the open overflow menu, by the row's index.
    pub fn pick_overflow_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench.open_menu = None;
        self.workbench.overflow_menu = None;
        let has_project = self.project(cx).is_some();
        let capture_offered = self.capture_offered(cx);
        match self
            .workbench
            .overflow_rows(has_project, capture_offered)
            .get(index)
        {
            Some(OverflowRow::RemoteConnect) => self.open_remote_connect(window, cx),
            Some(OverflowRow::WebExport) => self.open_web_export(window, cx),
            Some(OverflowRow::CaptureWindow) => self.capture_window(&CaptureWindow, window, cx),
            Some(OverflowRow::Help) => self.reveal_help(window, cx),
            Some(OverflowRow::PointAtSomething) => self.open_help_target(cx),
            Some(OverflowRow::Settings) => self.toggle_settings(cx),
            None => {}
        }
        cx.notify();
    }

    /// Dismiss the overflow menu — an outside click, or a pick already taken it.
    pub fn dismiss_overflow_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.overflow_menu = None;
        cx.notify();
    }

    /// Open the titlebar's new-project chevron menu, anchored where it was clicked.
    ///
    /// The same three rows the project picker's foot offers — add, clone, remote — reached
    /// without opening the picker first. The `+` beside it runs the first of the three directly.
    pub fn open_new_project_menu(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::NewProject);
        self.workbench.new_project_menu = Some(at);
        cx.notify();
    }

    /// Act on one row of the open new-project menu, by the row's index — the same action its
    /// namesake row runs in the project picker (`ui::project_menu`'s `add_row`, `clone_row` and
    /// `remote_row`).
    pub fn pick_new_project_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench.open_menu = None;
        self.workbench.new_project_menu = None;
        match self.workbench.new_project_rows().get(index) {
            Some(NewProjectRow::AddProject) => self.choose_folder(None, cx),
            Some(NewProjectRow::CloneProject) => self.open_clone(None, window, cx),
            Some(NewProjectRow::RemoteProject) => match self.preferred_remote_host() {
                Some((host, label)) => self.open_remote_project_picker(host, label, window, cx),
                None => self.open_remote_connect(window, cx),
            },
            None => {}
        }
        cx.notify();
    }

    /// Dismiss the new-project menu — an outside click, or a pick already taken it.
    pub fn dismiss_new_project_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.new_project_menu = None;
        cx.notify();
    }

    /// Open the search panel and bring it into focus.
    pub fn open_search(&mut self, _: &OpenSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_search(window, cx);
    }

    /// ⇧⌘O: bring the outline panel out if it is put away.
    pub fn open_outline(&mut self, _: &OpenOutline, window: &mut Window, cx: &mut Context<Self>) {
        self.reveal_outline(window, cx);
    }

    /// Rebuild the outline once the typing stops.
    ///
    /// The parse is a second one over the same bytes — see `ui::outline::extract` — so it is worth
    /// neither doing per keystroke nor doing at all while the panel is put away. The generation
    /// token is `md_reflow_gen`'s device: a debounce that lost the race installs nothing.
    pub(super) fn schedule_outline(&mut self, cx: &mut Context<Self>) {
        if !self.outline_panel_open(cx) {
            return;
        }
        self.outline_gen = self.outline_gen.wrapping_add(1);
        let token = self.outline_gen;
        let Some((key, text, ext)) = self.active_buffer_text(cx) else {
            self.outline = Vec::new();
            self.outline_key = String::new();
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OUTLINE_DEBOUNCE).await;
            let defs = cx
                .background_spawn(async move { ui::outline::extract(&text, &ext) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if token == this.outline_gen {
                    this.outline = defs;
                    this.outline_key = key;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The outline the panel draws, when the tab it was parsed from is no longer the tab on screen.
    ///
    /// A tab switch is not a keystroke, so it is parsed on the spot rather than debounced: waiting
    /// would draw the outgoing file's definitions over the incoming file's text.
    pub(super) fn settle_outline(&mut self, cx: &mut Context<Self>) {
        if !self.outline_panel_open(cx) {
            return;
        }
        match self.active_buffer_text(cx) {
            Some((key, _, _)) if key == self.outline_key => {}
            Some((key, text, ext)) => {
                self.outline = ui::outline::extract(&text, &ext);
                self.outline_key = key;
                cx.notify();
            }
            None => {
                if !self.outline.is_empty() {
                    self.outline = Vec::new();
                    self.outline_key = String::new();
                    cx.notify();
                }
            }
        }
    }

    /// Whether the outline panel is in the dock at all. Nothing is parsed for a panel nobody has
    /// asked for.
    fn outline_panel_open(&mut self, cx: &mut Context<Self>) -> bool {
        // Read rather than `panel()`: that one builds the entity if it is missing, and a window
        // whose user never opens the outline should not carry one.
        let Some(panel) = self.panels.get(&PanelKind::Outline).cloned() else {
            return false;
        };
        dock::holds(&self.dock.clone(), &panel, cx)
    }

    /// The active tab's key, its text and its extension — everything an outline is made of.
    fn active_buffer_text(&self, cx: &App) -> Option<(String, String, String)> {
        let file = self.editor(cx)?.active_file()?;
        let text = file.buffer()?.read(cx).value().to_string();
        Some((
            tab_key(&file.path, Subject::File),
            text,
            ui::outline::extension(&file.path),
        ))
    }

    /// Dismiss the tab's menu — an outside click, or a pick already taken it.
    pub fn dismiss_tab_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.open_menu = None;
        self.workbench.tab_menu = None;
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

    /// Close the active file's tab, taking an unsaved edit with it — vim's `:qa`.
    ///
    /// `close_editor_tab` asks before it drops a dirty buffer; `:q!` is the answer given in
    /// advance, so it goes straight to the one path that discards a buffer.
    pub fn discard_active_editor(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(key) = open.editor.active_file().map(|file| file.key()) else {
            return;
        };
        self.force_close_tab(&key, cx);
    }

    /// Write the active file back.
    ///
    /// Nothing happens with no project and no active file. Every other way this can decline —
    /// bytes that never arrived, a read the host cut short, a guest file — says so in a modal
    /// instead, because a ⌘S that quietly does nothing is indistinguishable from a save.
    pub fn save_active_file(&mut self, _: &SaveFile, window: &mut Window, cx: &mut Context<Self>) {
        // The plan dialog is raised over the window and holds the focus while it is up, so ⌘S
        // means *that* document rather than whatever tab is behind it. One key, one meaning: the
        // surface is the editor, not a dialog with a Save button.
        //
        // **A document open inside a markdown tab is not that case**: the key is over a file tab,
        // and it means the file. The annotation surface writes through its own section edits,
        // which save as they are confirmed, so nothing there is waiting on a ⌘S.
        if self
            .workbench
            .plan
            .as_ref()
            .is_some_and(|doc| doc.is_modal())
        {
            self.save_document(window, cx);
            return;
        }
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(file) = open.editor.active_file() else {
            return;
        };
        // An untitled buffer names nothing on disk, so the save is a question first.
        if file.untitled {
            let key = file.key();
            self.ask_save_as(key, window, cx);
            return;
        }
        // A buffer that cannot be written says why, rather than swallowing the keystroke.
        if let Some(refusal) = file.save_refusal() {
            let (key, refusal) = (file.key(), refusal.to_string());
            self.refuse_save(key, refusal, cx);
            return;
        }
        // A capture writes its flatten, not text.
        if file.image_edit().is_some() {
            let key = file.key();
            self.save_image_file(project, &key, cx);
            return;
        }
        let Some(buffer) = file.buffer() else {
            return;
        };
        let text = buffer.read(cx).value().to_string();
        let (path, expected, guest) = (file.path.clone(), file.version(), file.guest);

        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&path)
        {
            file.mark_saving(text.clone());
        }
        self.send_file_write(project, guest, path, text.into_bytes(), expected);
        cx.notify();
    }

    /// Open an empty buffer with no path yet.
    ///
    /// The tab and its panel open together, the same two steps `select_file` takes; the buffer
    /// itself is built by the arrival machinery, because a buffer needs a window and this does
    /// not have one to hand.
    pub fn new_untitled_file(&mut self, _: &NewFile, _window: &mut Window, cx: &mut Context<Self>) {
        // An image on the clipboard turns the keystroke into a question: paste it as an
        // untitled picture, or open the text buffer the key has always meant. Anything else —
        // text, nothing, a format this build does not draw — is the text path unchanged.
        if clipboard::clipboard_image(cx).is_some() {
            self.workbench.file_dialog = Some(FileDialog::PasteImage);
            cx.notify();
            return;
        }
        self.open_untitled_text(cx);
    }

    /// Paste the clipboard's image as an untitled picture. With text or nothing there the key
    /// is handed back: something deeper may still want it.
    pub fn paste_clipboard_image(
        &mut self,
        _: &PasteClipboardImage,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bytes) = clipboard::clipboard_image(cx) else {
            cx.propagate();
            return;
        };
        self.open_untitled_image(bytes, cx);
    }

    /// The ⌘N clipboard question answered "text": the keystroke's own meaning, from the modal's
    /// row, its dismissal, and its Escape alike.
    pub fn decline_paste_image(&mut self, cx: &mut Context<Self>) {
        self.close_file_dialog(cx);
        self.open_untitled_text(cx);
    }

    /// The buffer ⌘N has always opened, and the answer the clipboard question falls back to.
    pub(super) fn open_untitled_text(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let path = {
            let Some(open) = self.projects.get_mut(&project) else {
                return;
            };
            // Numbered past whatever is already open, so two of them are told apart.
            (1..)
                .map(|n| format!("untitled-{n}"))
                .find(|path| open.editor.index_of(path).is_none())
                .expect("the numbering grows without bound")
        };
        self.push_untitled(
            project,
            path,
            FileContents {
                bytes: Vec::new(),
                len: 0,
                truncated: false,
                is_binary: false,
                version: None,
            },
            cx,
        );
    }

    /// An untitled picture from the clipboard — and, when it lands, from capture: named with
    /// its extension, because `image::format` and `ViewerKind::of` read nothing else.
    pub(super) fn open_untitled_image(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let path = {
            let Some(open) = self.projects.get_mut(&project) else {
                return;
            };
            clipboard::next_capture_name(&open.editor)
        };
        let len = bytes.len() as u64;
        self.push_untitled(
            project,
            path,
            FileContents {
                bytes,
                len,
                truncated: false,
                is_binary: false,
                version: None,
            },
            cx,
        );
    }

    /// Open a buffer with no path yet.
    ///
    /// The tab and its panel open together, the same two steps `select_file` takes; the buffer
    /// itself is built by the arrival machinery, because a buffer needs a window and this does
    /// not have one to hand.
    fn push_untitled(
        &mut self,
        project: ProjectId,
        path: String,
        contents: FileContents,
        cx: &mut Context<Self>,
    ) {
        let markdown_open = self.workbench.settings.ui.markdown_open.layout();
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.editor
            .open
            .push(OpenFile::untitled(&path, markdown_open));
        open.editor.active = open.editor.open.len() - 1;
        self.pending_editor_focus = Some(tab_key(&path, Subject::File));
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::File(tab_key(
                &path,
                Subject::File,
            ))));
        // No read is coming, so the bytes are handed over as if one had arrived — an image tab
        // draws them, a text tab builds its buffer from them.
        self.pending_files.push(FileArrival {
            project,
            path,
            contents,
        });
        cx.notify();
    }

    fn ask_save_as(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        // Seeded with the tab's own name: a capture suggests `capture-1.png` rather than
        // asking to be typed from nothing, and a text buffer suggests its `untitled-n`.
        let seed = self
            .project(cx)
            .and_then(|project| self.projects.get(&project))
            .and_then(|open| {
                open.editor
                    .index_of_key(&key)
                    .map(|at| open.editor.open[at].name.clone())
            })
            .unwrap_or_default();
        self.open_file_dialog(FileDialog::SaveAs { key }, &seed, window, cx);
    }

    /// Write the capture behind one tab back: its flatten over `WriteProjectFile`, with the
    /// version discipline any other write carries. Nothing new crosses the bus.
    fn save_image_file(&mut self, project: ProjectId, key: &str, cx: &mut Context<Self>) {
        // Every image now carries a scene, so a save on one nobody annotated would re-encode the
        // file over itself for nothing. An unannotated picture writes nothing.
        if self
            .projects
            .get(&project)
            .and_then(|open| open.editor.open.iter().find(|file| file.key() == key))
            .is_some_and(|file| !file.dirty())
        {
            return;
        }
        let flat = self
            .projects
            .get(&project)
            .and_then(|open| open.editor.open.iter().find(|file| file.key() == key))
            .and_then(|file| {
                file.image_edit().and_then(|edit| {
                    edit.flatten()
                        .map(|png| (file.path.clone(), edit.version, file.guest, png))
                })
            });
        let Some((path, expected, guest, png)) = flat else {
            if let Some(open) = self.projects.get_mut(&project)
                && let Some(file) = open.editor.find_key_mut(key)
            {
                file.save_failed("the picture did not flatten".to_string());
            }
            cx.notify();
            return;
        };
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&path)
        {
            file.mark_saving(String::new());
        }
        self.send_file_write(project, guest, path, png, expected);
        cx.notify();
    }

    /// Give an untitled buffer a path and write it there.
    ///
    /// The tab is retitled on the click rather than on the answer — the same bet `select_file`
    /// makes — so a refusal lands on `ProjectFileError` for the path the user chose and the
    /// message on the tab reads correctly. `expected: None` is what already means *create, and
    /// refuse if anything is there*, so there is no new host code behind this at all.
    pub(super) fn save_untitled_as(&mut self, key: &str, path: String, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            return;
        };
        let (bytes, saving) = save_payload(&open.editor.open[at], cx);
        let Some(bytes) = bytes else {
            if let Some(open) = self.projects.get_mut(&project)
                && let Some(file) = open.editor.find_key_mut(key)
            {
                file.save_failed("the picture did not flatten".to_string());
            }
            cx.notify();
            return;
        };
        self.retarget_editor_tab(project, key, &path, cx);
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&path)
        {
            file.mark_saving(saving);
        }
        self.bus.send(Message::WriteProjectFile {
            project_id: project,
            rel_path: path,
            bytes,
            expected: None,
            overwrite: false,
        });
        cx.notify();
    }

    /// Write over the file a save landed on, now that the user has been asked and said yes.
    ///
    /// The same bytes the refused write carried, sent again with `overwrite` — which is the host's
    /// name for *the interface asked*. The tab already points at the path, so nothing is
    /// retargeted here; only the flag is different.
    pub(super) fn overwrite_file(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        let Some(at) = index_of_key(&open.editor, key) else {
            return;
        };
        let path = open.editor.open[at].path.clone();
        let (bytes, saving) = save_payload(&open.editor.open[at], cx);
        let Some(bytes) = bytes else {
            let name = open.editor.open[at].name.clone();
            self.refuse_save(key.to_string(), format!("{name} has no bytes to write"), cx);
            return;
        };
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_key_mut(key)
        {
            file.mark_saving(saving);
        }
        self.bus.send(Message::WriteProjectFile {
            project_id: project,
            rel_path: path,
            bytes,
            expected: None,
            overwrite: true,
        });
        cx.notify();
    }

    /// Say a save did not happen, and why.
    ///
    /// The buffer is untouched and stays dirty — losing the edits is the one outcome that is never
    /// acceptable — and the modal is what the user gets instead of a keystroke that did nothing
    /// and said nothing.
    pub(super) fn refuse_save(&mut self, key: String, reason: String, cx: &mut Context<Self>) {
        if let Some(project) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_key_mut(&key)
        {
            file.save_failed(reason.clone());
        }
        self.workbench.file_dialog = Some(FileDialog::SaveFailed { key, reason });
        cx.notify();
    }

    /// Point one open tab at another path, keeping its buffer.
    ///
    /// The panel is keyed by the tab key, and the key is the path — so the panel that drew the old
    /// key is taken out and one for the new key put in. The buffer lives on the tab rather than in
    /// the panel, so nothing typed is lost on the way.
    pub(super) fn retarget_editor_tab(
        &mut self,
        project: ProjectId,
        key: &str,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        let Some(file) = open.editor.find_key_mut(key) else {
            return;
        };
        file.retarget(path);
        let fresh = file.key();
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::File(key.to_string())));
        self.pending_panels
            .push(PanelEdit::Open(PanelKind::File(fresh.clone())));
        self.pending_editor_focus = Some(fresh);
        self.remember(project, cx);
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
        // it is not text and is not nothing, and the decoder wants what the host read. An
        // untitled picture was never on disk, so it opens dirty and closing it asks — as an
        // editable capture when the bytes carry a picture, as read-only bytes when they do not.
        if draws_bytes {
            if let Some(open) = self.projects.get_mut(&project)
                && let Some(file) = open.editor.find_mut(&path)
            {
                if file.untitled {
                    file.set_image_untitled(contents.bytes);
                } else {
                    file.set_image(contents.bytes, contents.version);
                }
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
                // The outline is one debounce behind the buffer, and is not rebuilt at all while
                // its panel is put away.
                this.schedule_outline(cx);
                cx.notify();
            },
        );

        // A destination that named a spot in this file arrived before its bytes did. It outranks
        // a reload's saved caret: the reload is where the tab was, the goto is where the user
        // asked to be.
        let goto = self
            .pending_goto
            .take_if(|(key, _)| *key == tab_key(&path, Subject::File))
            .and_then(|(_, locus)| range_for(&text, &locus));

        if let Some(open) = self.projects.get_mut(&project)
            && let Some(file) = open.editor.find_mut(&path)
        {
            // A reload's cursor and scroll were saved off the buffer this one replaces; put them
            // back now so a background tab's refresh lands where the user left it rather than at
            // the top. Nothing to restore for a tab that was never reloaded.
            let restore = file.take_restore();
            file.attach(
                buffer.clone(),
                text,
                contents.truncated,
                contents.version,
                change,
            );
            if let Some((selection, scroll)) = restore {
                buffer.update(cx, |state, cx| {
                    state.set_selected_range(selection, cx);
                    state.set_scroll_offset(scroll, cx);
                });
            }
            // The goto lands last: a restore's `set_scroll_offset` would otherwise put the view
            // back where the tab was, leaving the caret on the asked-for line off screen — the
            // reason a search hit used to need a second click to be seen.
            if let Some(range) = goto {
                buffer.update(cx, |state, cx| state.set_selected_range(range, cx));
            }
        }
        // The bytes are here, so what this file's bookmarks now point at can be answered.
        self.mark_bookmarks(&tab_key(&path, Subject::File), cx);
        cx.notify();
    }

    // ── The agents screen ───────────────────────────────────────────
    //
    // The arrangement is the interface's own: not one of these sends a message. What crosses the
    // bus from this screen is the one thing that is not about arrangement — what the user typed at
    // an agent, which `steer_column` sends. Every handler is guarded on the window holding a
    // project, because the screen is a view of one project's work.
}

/// The bytes one tab writes, and the in-flight text its acknowledgement rebases against.
///
/// A body is not always a string: text sends its buffer, an unedited capture its raw bytes, an
/// annotated one its flatten. PNG only, in every image case. The in-flight text travels with a
/// text save so the acknowledgement rebases against what was actually written; an image save
/// carries none, and its `saved` just clears dirty.
fn save_payload(file: &OpenFile, cx: &App) -> (Option<Vec<u8>>, String) {
    match &file.body {
        FileBody::Bytes(raw) => (Some(raw.clone()), String::new()),
        FileBody::ImageEdit(edit) => (edit.flatten(), String::new()),
        FileBody::Text { .. } => match file
            .buffer()
            .map(|buffer| buffer.read(cx).value().to_string())
        {
            Some(text) => (Some(text.clone().into_bytes()), text),
            None => (None, String::new()),
        },
        _ => (None, String::new()),
    }
}
