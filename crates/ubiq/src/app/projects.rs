use super::*;

impl AppState {
    pub(super) fn apply_project_form_hex(&mut self, cx: &mut Context<Self>) {
        let text = self.project_form_hex.read(cx).value();
        let Some(rgb) = crate::state::sink::parse_hex(text.as_ref()) else {
            return;
        };
        let Some(settings) = self.workbench.project_settings.as_mut() else {
            return;
        };
        if settings.colour.apply_hex(rgb) {
            cx.notify();
        }
    }

    pub(super) fn sync_project_form_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = self.workbench.project_settings.as_ref() else {
            return;
        };
        let hex = crate::state::sink::hex_string(settings.colour.rgb());
        let input = self.project_form_hex.clone();
        input.update(cx, |input, cx| input.set_value(&hex, window, cx));
    }

    // The picker page's controls. Each sets one field of the request the next dialog is raised
    // with; none of them touches a picker that is already up, because the ask a dialog was opened
    // under is the ask it is answering.

    /// Point this window at a project it already holds.
    pub fn activate_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let id = self.window_id;
        cx.global_mut::<WindowRegistry>().activate(id, project);
        self.bus.send(Message::OpenedProject {
            project_id: project,
        });
        // The window now points somewhere else: its tree, its tabs and its terminals are the new
        // project's from here.
        self.sync_projects(cx);
        self.close_menu(cx);
    }

    /// Open a project in this window, taking it from whichever window held it. This is both "open
    /// one from history" and the move-here action on a project open elsewhere: a project is open in
    /// one window at a time, so the two are the same operation.
    pub fn take_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let id = self.window_id;
        // A project the catalogue does not hold is not opened, and the host is not told that a
        // window pointed at one.
        if !cx.global_mut::<WindowRegistry>().open_in(id, project) {
            self.close_menu(cx);
            return;
        }
        // The host decides what opening a project means, and stamps it.
        self.bus.send(Message::OpenedProject {
            project_id: project,
        });
        // Everything the project brings with it — its furniture, its pane, its tree — is the
        // reconciliation's, including the `GetPreferences` that asks where it was left.
        self.sync_projects(cx);
        self.close_menu(cx);
    }

    /// Close a project in this window. One with terminals still running asks first: the menu row
    /// turns into a confirmation rather than taking the click. Closing the last one leaves the
    /// window open on nothing, with the picker to offer.
    pub fn close_project(&mut self, project: ProjectId, force: bool, cx: &mut Context<Self>) {
        // This window's own count, not the catalogue's: closing a project here kills the panes
        // *this* window is running in it, and says so about those.
        let panes = self
            .projects
            .get(&project)
            .map_or(0, |open| open.panes.len());
        if panes > 0 && !force {
            self.workbench.pending_close = Some(project);
            cx.notify();
            return;
        }

        self.workbench.pending_close = None;
        // A search over a project this window no longer holds has nowhere to draw: it goes with
        // the project's other state, and the worker is told so it stops walking.
        if let Some(active) = self
            .search
            .active
            .as_ref()
            .filter(|a| a.project_id == project)
        {
            self.bus.send(Message::CancelSearch {
                project_id: active.project_id,
                search_id: active.search_id,
            });
            self.search.reset();
        }
        let id = self.window_id;
        // Read before the close, while the registry still answers for this project: `close` only
        // drops it from this window's slot, but the snapshot is the same one either side of that.
        let temporary = WindowRegistry::read(cx)
            .project(project)
            .is_some_and(|p| p.record.temporary);
        cx.global_mut::<WindowRegistry>().close(id, project);
        self.sync_projects(cx);
        // Project messages are broadcast, so another window may still hold this project — a
        // temporary one is only forgotten once nothing points at it any more, or that window's
        // project would close out from under it.
        if temporary && WindowRegistry::read(cx).holder(project).is_none() {
            self.bus.send(Message::ForgetProject {
                project_id: project,
            });
        }
    }

    pub fn cancel_close(&mut self, cx: &mut Context<Self>) {
        self.workbench.pending_close = None;
        cx.notify();
    }

    // ── Asking the host ─────────────────────────────────────────────

    /// Rename or recolour. The host answers, and every window redraws.
    pub fn update_project(
        &mut self,
        project: ProjectId,
        name: Option<String>,
        colour: Option<usize>,
        custom: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name,
            colour,
            custom_colour: custom,
            search_excludes: None,
            no_local_index: None,
        });
        self.workbench.row_action = None;
        cx.notify();
    }

    /// Drop the record and everything Ubiq remembers about it. Nothing inside the project's own
    /// folder is touched, which is why the word is "Forget".
    pub fn forget_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        self.bus.send(Message::ForgetProject {
            project_id: project,
        });
        self.workbench.row_action = None;
        cx.notify();
    }

    /// Look at a project's folder again — the action a row marked missing offers.
    pub fn refresh_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        self.bus.send(Message::RefreshProject {
            project_id: project,
        });
        cx.notify();
    }

    /// Re-point a record at a folder that moved, keeping its id, colour and history.
    pub fn locate_project(&mut self, project: ProjectId, path: String, cx: &mut Context<Self>) {
        self.bus.send(Message::LocateProject {
            project_id: project,
            path,
        });
        cx.notify();
    }

    /// Take a folder into the catalogue. This window opens whatever the host answers with.
    ///
    /// `temporary` is a folder dropped in rather than chosen: it opens the same way, but the host
    /// never writes it to the catalogue unless the user later keeps it from the titlebar.
    pub fn add_project(
        &mut self,
        path: String,
        name: Option<String>,
        colour: Option<usize>,
        custom: Option<u32>,
        temporary: bool,
        cx: &mut Context<Self>,
    ) {
        self.adding = true;
        self.bus.send(Message::AddProject {
            path,
            name,
            colour,
            custom_colour: custom,
            temporary,
        });
        cx.notify();
    }

    /// Ask the operating system for a folder, then add it or re-point the project being located.
    ///
    /// The dialog is the platform's own — the one users already know how to type a path into, and
    /// the one that reaches their bookmarks and network volumes. It browses the *interface's*
    /// filesystem, which is the host's only while the two share a machine; see `D32`.
    pub fn choose_folder(&mut self, locating: Option<ProjectId>, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(match locating {
                Some(_) => "Locate".into(),
                None => "Add".into(),
            }),
        });

        cx.spawn(async move |this, cx| {
            // Three outcomes, and only one of them is a path: the channel can close with the
            // dialog, the platform can refuse to open one, and the user can cancel.
            let answer = match chosen.await {
                Ok(answer) => answer,
                Err(_) => return,
            };

            this.update(cx, |this, cx| match answer {
                Ok(Some(paths)) => {
                    let Some(path) = paths.into_iter().next() else {
                        return;
                    };
                    let path = path.to_string_lossy().into_owned();
                    match locating {
                        Some(project) => this.locate_project(project, path, cx),
                        None => this.open_create_project(path, cx),
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    this.workbench.project_error =
                        Some(format!("could not open a chooser: {error}"));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Expand one picker row into a Forget confirmation.
    pub fn set_row_action(
        &mut self,
        action: Option<(ProjectId, crate::state::RowAction)>,
        cx: &mut Context<Self>,
    ) {
        self.workbench.row_action = action;
        cx.notify();
    }

    /// Name and colour a folder before it enters the catalogue. The fields are filled on the next
    /// frame, because `set_value` needs a window and the chooser does not come with one.
    pub fn open_create_project(&mut self, path: String, cx: &mut Context<Self>) {
        let colour = self.next_colour(cx);
        self.workbench.open_menu = None;
        self.workbench.settings.open = false;
        self.workbench.project_settings = Some(ProjectSettings {
            mode: ProjectSettingsMode::Create { path },
            colour: ColourField {
                swatch: colour,
                ..ColourField::default()
            },
        });
        self.fill_project_form = true;
        cx.notify();
    }

    /// Open project settings for the project this window is showing. Path stays as it is.
    pub fn open_edit_project(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.project_snapshot(cx) else {
            return;
        };
        let project = snapshot.record.id;
        // A temporary project was never let choose its own colour — it drew the grey instead — so
        // this seeds a real one rather than carrying that grey's index into the form.
        let colour = if snapshot.record.temporary {
            self.next_colour(cx)
        } else {
            snapshot.record.colour
        };
        // A temporary project's grey was never a real colour either, so its custom colour, if any,
        // does not carry over — the same reasoning that seeds `colour` from `next_colour` above.
        let custom = if snapshot.record.temporary {
            None
        } else {
            snapshot.record.custom_colour
        };
        self.workbench.open_menu = None;
        self.workbench.settings.open = false;
        self.workbench.project_settings = Some(ProjectSettings {
            mode: ProjectSettingsMode::Edit { project },
            colour: ColourField {
                swatch: colour,
                custom,
                ..ColourField::default()
            },
        });
        self.fill_project_form = true;
        cx.notify();
    }

    pub fn close_project_settings(&mut self, cx: &mut Context<Self>) {
        self.workbench.project_settings = None;
        cx.notify();
    }

    /// Create the project, or write the name and colour back. An empty name is ignored rather than
    /// applied: a project with no name is not something the picker can draw.
    pub fn commit_project_settings(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.workbench.project_settings.take() else {
            return;
        };
        let name = self.rename_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.workbench.project_settings = Some(settings);
            return;
        }
        let colour = settings.colour.swatch;
        let custom = settings.colour.custom;
        match settings.mode {
            ProjectSettingsMode::Create { path } => {
                self.add_project(path, Some(name), Some(colour), custom, false, cx);
            }
            ProjectSettingsMode::Edit { project } => {
                // Nothing marks this as a promotion: the host treats an `UpdateProject` on a
                // temporary record as the project's entry into the real catalogue.
                self.update_project(project, Some(name), Some(colour), custom, cx);
            }
        }
    }

    pub(super) fn fill_project_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fill_project_form {
            return;
        }
        self.fill_project_form = false;
        let Some(settings) = self.workbench.project_settings.as_ref() else {
            return;
        };
        let name = match &settings.mode {
            ProjectSettingsMode::Create { path } => leaf_name(path).to_string(),
            ProjectSettingsMode::Edit { project } => WindowRegistry::read(cx)
                .project(*project)
                .map(|entry| entry.record.name.clone())
                .unwrap_or_default(),
        };
        if let Some(settings) = self.workbench.project_settings.as_mut() {
            settings.colour.seed_hsv();
        }
        let name_input = self.rename_input.clone();
        name_input.update(cx, |input, cx| {
            input.set_value(&name, window, cx);
            input.focus(window, cx);
        });
        let about = self.project_form_about.clone();
        about.update(cx, |input, cx| input.set_value("", window, cx));
        self.sync_project_form_hex(window, cx);
    }

    pub fn dismiss_project_error(&mut self, cx: &mut Context<Self>) {
        self.workbench.project_error = None;
        cx.notify();
    }

    // ── Boot, and what is remembered ────────────────────────────────

    /// Take a project on the first catalogue that arrives.
    ///
    /// Only the window the binary opened is owed this. One code path serves both the ordinary boot
    /// and the empty catalogue, and the cost is a single frame of the picker.
    pub(super) fn adopt_if_owed(&mut self, cx: &mut Context<Self>) {
        if !self.adopt_on_list {
            return;
        }
        self.adopt_on_list = false;

        if self.project(cx).is_some() {
            return;
        }
        // Nothing to adopt is the first run, and the window stays open on the picker.
        if let Some(id) = WindowRegistry::read(cx).most_recent() {
            self.take_project(id, cx);
        }
    }

    /// Apply what the host had stored for a scope.
    pub(super) fn apply_preferences(
        &mut self,
        scope: Scope,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(blob) = value else { return };

        match scope {
            Scope::Interface => {
                if let Some(prefs) = prefs::decode::<prefs::InterfacePrefs>(&blob)
                    && prefs.theme != self.workbench.theme_id
                {
                    self.workbench.theme_id = prefs.theme;
                    theme::set_mode(prefs.theme, cx);
                }
            }
            Scope::Project(id) => {
                let Some(view) = prefs::decode::<prefs::ViewPrefs>(&blob) else {
                    return;
                };
                // An answer for a project this window holds without showing still has to reach
                // it: a project's furniture is its own, whether or not anyone is looking at it.
                let showing = self.project(cx) == Some(id);
                let Some(open) = self.projects.get_mut(&id) else {
                    return;
                };
                open.prefs = view.clone();
                let restore = (!open.restored).then(|| view.clone());
                if showing {
                    self.workbench.rail_mode = view.rail_mode;
                    self.workbench.file_filter = view.file_filter.clone();
                    self.pending_layout = view
                        .modes
                        .get(&view.rail_mode)
                        .and_then(|mode| mode.layout.clone());
                }
                // A project closed and reopened in this session restored from the parked blob
                // already, and reopening the tabs the user has since closed would be worse than
                // useless.
                if let Some(view) = restore {
                    self.restore_files(id, &view, cx);
                }
            }
        }
        cx.notify();
    }

    /// Write down what this window looks like now, for the project on screen. Debounced by the
    /// host, so this may be called as freely as a drag fires.
    pub fn remember_view(&mut self, cx: &App) {
        let Some(id) = self.project(cx) else { return };
        self.remember(id, cx);
    }

    /// Write down what one project this window holds was left looking like.
    ///
    /// The furniture is read off the window only for the project on screen. A background project
    /// keeps the furniture its own blob already carries, which is what stops a project being
    /// written down wearing whatever the user happened to be looking at.
    pub(super) fn remember(&mut self, project: ProjectId, cx: &App) {
        // The file set is the project's own whether or not anyone is looking at it, so it is read
        // back from the project every time. Tab keys rather than paths, because a file and its diff
        // are two tabs and a path names both.
        if let Some(open) = self.projects.get_mut(&project) {
            open.prefs.open_files = open.editor.open.iter().map(|file| file.key()).collect();
            open.prefs.active_file = open.editor.active_file().map(|file| file.key());
            open.prefs.expanded = open.explorer.expanded();
            open.prefs.selected = open.explorer.selected.clone();
            open.prefs.file_filter = self.workbench.file_filter.clone();
        }

        if self.project(cx) == Some(project) {
            // The current mode's whole arrangement in one blob — the tree, the axes, the sizes,
            // and which tab is displayed. The three region flags are written beside it for a mode
            // that has the flags and cannot read the blob; the blob is what a restore uses. Every
            // other mode keeps the entry it was last written in, which is what stops the IDE's
            // side panels coming back undone because the sink was used in between.
            let rail_mode = self.workbench.rail_mode;
            let layout = self.layout_blob(cx);
            let (left, bottom, right) = self.regions_open(cx);
            if let Some(open) = self.projects.get_mut(&project) {
                open.prefs.rail_mode = rail_mode;
                open.prefs.modes.insert(
                    rail_mode,
                    prefs::ModeLayout {
                        show_left: left,
                        show_bottom: bottom,
                        show_right: right,
                        layout,
                    },
                );
            }
        }

        self.store_prefs(project);
    }

    /// Send a project's stored view to the host as it stands, reading nothing off the window.
    ///
    /// [`Self::remember`] is how the arrangement on screen gets into it; this is for the fields
    /// that are settled without looking at the dock — the rail mode a switch has just chosen,
    /// whose own arrangement has not been restored yet and must not be written down as if it had.
    pub(super) fn store_prefs(&self, project: ProjectId) {
        let Some(open) = self.projects.get(&project) else {
            return;
        };
        self.bus.send(Message::SetPreferences {
            scope: Scope::Project(project),
            value: prefs::encode(&open.prefs),
        });
    }

    // ── The dock ────────────────────────────────────────────────────
}
