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

    /// Keeps the rail-initials field at three characters as it is typed, rather than only refusing
    /// a fourth at Save — a field that silently accepted a longer string until then would look
    /// like it had taken it.
    pub(super) fn clamp_project_initials_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.project_initials_input.read(cx).value().to_string();
        let clamped: String = text.chars().take(3).collect();
        if clamped != text {
            let input = self.project_initials_input.clone();
            input.update(cx, |input, cx| input.set_value(&clamped, window, cx));
        }
    }

    // The picker page's controls. Each sets one field of the request the next dialog is raised
    // with; none of them touches a picker that is already up, because the ask a dialog was opened
    // under is the ask it is answering.

    /// What this window would take with it if it closed: one line per project it holds that has
    /// unsaved files, terminals still running, or agents still running, in name order.
    ///
    /// An untitled buffer nobody has typed into does not count — its baseline is the empty bytes
    /// it opened with, so `OpenFile::dirty` already answers no for it. An unloaded or ended
    /// conversation does not count either — its harness is already gone.
    pub fn unsaved_summary(&self, cx: &App) -> Vec<String> {
        let registry = WindowRegistry::read(cx);
        let mut rows: Vec<String> = self
            .projects
            .keys()
            .filter_map(|project| {
                let holds = self.project_holds(*project, cx);
                let name = registry
                    .project(*project)
                    .map_or_else(|| project.to_string(), |entry| entry.record.name.clone());
                Some(format!("{name} — {}", holds.sentence()?))
            })
            .collect();
        rows.sort();
        rows
    }

    /// What one project this window holds would take with it: unsaved files, terminals still
    /// running, and agents still running. An untitled buffer nobody has typed into is not one of
    /// them — its baseline is the empty bytes it opened with, so `OpenFile::dirty` already answers
    /// no for it. An unloaded or ended conversation is not one of them either — its harness is
    /// already gone.
    ///
    /// An ephemeral clone always holds something, whatever is open in it: closing it forgets the
    /// project and the folder goes with it, which is the one close in this window that cannot be
    /// undone by reopening.
    pub fn project_holds(&self, project: ProjectId, cx: &App) -> Holds {
        let ephemeral = self.project_is_ephemeral(project, cx);
        let Some(open) = self.projects.get(&project) else {
            return Holds {
                ephemeral,
                ..Holds::default()
            };
        };
        Holds {
            files: open.editor.open.iter().filter(|file| file.dirty()).count(),
            panes: open.panes.len(),
            agents: open
                .conversations
                .values()
                .filter(|conversation| conversation.running())
                .count(),
            ephemeral,
        }
    }

    /// The window's close, asked once. `false` holds the window open while the question is up;
    /// the yes comes back through `confirm_file_dialog`, which sets `closing` and closes again.
    pub(super) fn ask_before_closing(&mut self, quitting: bool, cx: &mut Context<Self>) -> bool {
        if self.closing || self.unsaved_summary(cx).is_empty() {
            return true;
        }
        self.workbench.file_dialog = Some(FileDialog::CloseWindow { quitting });
        cx.notify();
        false
    }

    /// Launch or adopt the drone a project names, on the way to opening it.
    ///
    /// **On this side of the bus, never the coordinator's.** `D116` keeps the coordinator from
    /// learning a drone exists at all — attaching one is adding a host to this window, which is
    /// the interface's business and nothing the catalogue can do on its own.
    ///
    /// A host already attached for the origin's profile and folder is adopted: it is already
    /// serving that root, and a second `ssh` to it would start a rival drone for the same folder.
    pub(super) fn ensure_drone(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let Some(origin) = WindowRegistry::read(cx)
            .project(project)
            .and_then(|snapshot| snapshot.record.runs_on.clone())
        else {
            return;
        };
        if self.drone_attached(&origin) {
            return;
        }
        self.dial_project_drone(project, origin, cx);
    }

    /// Activate the Nth project the rail's badges show, `slot` 1-based — `cmd-1`..`cmd-9`.
    /// [`crate::ui::rail::visible_project_order`] says what "Nth" means: the same order and the
    /// same shortage rule the badges themselves draw with, so the digit always lands on the
    /// project its badge shows. A digit with no badge behind it — the badge switch is off, the
    /// window holds one project, or the digit is past the last one — does nothing.
    pub fn activate_project_slot(&mut self, slot: u8, window: &Window, cx: &mut Context<Self>) {
        let order = crate::ui::rail::visible_project_order(self, window, cx);
        if let Some(project) = order.get(usize::from(slot).saturating_sub(1)).copied() {
            self.activate_project(project, cx);
        }
    }

    /// Point this window at a project it already holds.
    pub fn activate_project(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let id = self.window_id;
        self.ensure_drone(project, cx);
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
        // window pointed at one. Asked before the hand-off below rather than after, because
        // `open_in` answers the same question and a project taken out of a window it could not
        // then be put into would be a move that lost it.
        let registry = WindowRegistry::read(cx);
        if registry.project(project).is_none() || registry.slot(id).is_none() {
            self.close_menu(cx);
            return;
        }
        // Taking a project from another window moves what is running in it — the panes and the
        // conversations — rather than closing them. `open_in` below only moves the *row*; this is
        // the window that held it letting go of the live state, which has to happen before the
        // registry changes hands so the losing window's own reconcile finds nothing to drop.
        let handed = WindowRegistry::read(cx)
            .holder(project)
            .map(|slot| slot.id)
            .filter(|holder| *holder != id)
            .and_then(|holder| OpenWindows::get(cx, holder))
            .and_then(|view| view.update(cx, |state, cx| state.hand_off_project(project, cx)));

        if !cx.global_mut::<WindowRegistry>().open_in(id, project) {
            self.close_menu(cx);
            return;
        }
        if let Some(handed) = handed {
            self.adopt_project(project, handed, cx);
        }
        // Before the reconciliation below, so a project that runs on a drone has its `ssh` on the
        // way while the tree and the tabs are being built.
        self.ensure_drone(project, cx);
        // The host decides what opening a project means, and stamps it.
        self.bus.send(Message::OpenedProject {
            project_id: project,
        });
        // Everything the project brings with it — its furniture, its pane, its tree — is the
        // reconciliation's, including the `GetPreferences` that asks where it was left.
        self.sync_projects(cx);
        self.close_menu(cx);
    }

    /// Close a project in this window. One with terminals still running, agents still running, or
    /// unsaved files asks first, in the same modal the window's own close raises — one project's
    /// worth of it. Closing the last one leaves the window open on nothing, with the picker to
    /// offer.
    pub fn close_project(&mut self, project: ProjectId, force: bool, cx: &mut Context<Self>) {
        // This window's own count, not the catalogue's: closing a project here kills the panes
        // *this* window is running in it, unloads the agents it is talking to, and drops what was
        // typed into its buffers, and says so about those.
        if self.project_holds(project, cx).anything() && !force {
            // The menu the click came from goes first: the question is a modal over the window
            // now, and a menu left open behind it is a menu the answer would have to dodge.
            self.close_menu(cx);
            self.workbench.file_dialog = Some(FileDialog::CloseProject { project });
            cx.notify();
            return;
        }

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
            index: None,
            mission_term: None,
            tools: None,
            managed_repos: None,
            lanes: None,
            runs_on: None,
        });
        self.workbench.row_action = None;
        cx.notify();
    }

    /// This project's indexing level, or a return to the application-wide default.
    ///
    /// Sent on the click rather than held until the dialog is dismissed: it is a switch, and the
    /// host acts on it at once — building or dropping an index while the dialog is still open is
    /// what makes the setting feel like it did something.
    pub fn set_project_index(
        &mut self,
        project: ProjectId,
        index: ubiq_proto::projects::IndexChange,
        cx: &mut Context<Self>,
    ) {
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name: None,
            colour: None,
            custom_colour: None,
            search_excludes: None,
            index: Some(index),
            mission_term: None,
            tools: None,
            managed_repos: None,
            lanes: None,
            runs_on: None,
        });
        cx.notify();
    }

    /// This project's own word for a mission, or a return to the application-wide default.
    ///
    /// The same immediacy `set_project_index` gives the index pill: the host acts on it at once,
    /// and the snapshot every window redraws from is updated here rather than waiting for the
    /// host's echo, so the board's mission cards relabel the moment the choice is made.
    pub fn set_project_mission_term(
        &mut self,
        project: ProjectId,
        change: ubiq_proto::projects::MissionTermChange,
        cx: &mut Context<Self>,
    ) {
        let Some(mut snapshot) = WindowRegistry::read(cx).project(project).cloned() else {
            return;
        };
        snapshot.record.mission_term = change.clone().resolve();
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name: None,
            colour: None,
            custom_colour: None,
            search_excludes: None,
            index: None,
            mission_term: Some(change),
            tools: None,
            managed_repos: None,
            lanes: None,
            runs_on: None,
        });
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        cx.notify();
    }

    /// Reveal the project settings dialog's own mission-term field, seeded with whatever word is
    /// held today — the override if there is one, else the application-wide default — and pin the
    /// project to it at once. `mission_term_row`'s "Custom" pill calls this; typing into the field
    /// afterwards goes through `set_project_mission_term_from_field`.
    pub fn begin_project_mission_term_override(
        &mut self,
        project: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = WindowRegistry::read(cx)
            .project(project)
            .map(|s| s.record.clone())
        else {
            return;
        };
        let seed = record
            .mission_term
            .unwrap_or_else(|| self.workbench.settings.host.mission_term.clone());
        let input = self.project_mission_term_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(&seed, window, cx);
            state.focus(window, cx);
        });
        self.set_project_mission_term(
            project,
            ubiq_proto::projects::MissionTermChange::Set(seed),
            cx,
        );
    }

    /// Rename the project's own mission term. Ignored unless the override is what is showing —
    /// the field is only drawn then — and an empty word falls back to the application-wide
    /// default rather than being stored, on `set_agent_home_name`'s rule.
    pub fn set_project_mission_term_from_field(&mut self, term: String, cx: &mut Context<Self>) {
        let Some(project) = self.editing_project() else {
            return;
        };
        let is_override = WindowRegistry::read(cx)
            .project(project)
            .is_some_and(|s| s.record.mission_term.is_some());
        if !is_override {
            return;
        }
        let change = match term.trim() {
            "" => ubiq_proto::projects::MissionTermChange::Inherit,
            trimmed => ubiq_proto::projects::MissionTermChange::Set(trimmed.to_string()),
        };
        self.set_project_mission_term(project, change, cx);
    }

    /// Where this project's folder lives: on a drone, or here.
    ///
    /// Sent on the Remote panel's Save rather than held to the dialog's, and the snapshot is
    /// applied at once, the way `set_project_search_excludes` applies its own — pinning a project
    /// is what the next open acts on, and a row that still reads "runs here" after a save reads
    /// as the save having been dropped.
    ///
    /// Sending it is the whole of the change. Nothing dials here: a drone is launched by
    /// [`Self::ensure_drone`] when the project is opened, which is the moment there is anything
    /// for one to serve.
    pub fn set_project_runs_on(
        &mut self,
        project: ProjectId,
        change: ubiq_proto::projects::DroneChange,
        cx: &mut Context<Self>,
    ) {
        let Some(mut snapshot) = WindowRegistry::read(cx).project(project).cloned() else {
            return;
        };
        snapshot.record.runs_on = change.clone().resolve();
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name: None,
            colour: None,
            custom_colour: None,
            search_excludes: None,
            index: None,
            mission_term: None,
            tools: None,
            managed_repos: None,
            lanes: None,
            runs_on: Some(change),
        });
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        cx.notify();
    }

    /// Write a project's whole search-excludes list, sending it at once and updating the snapshot
    /// every window redraws from — the same immediacy `set_project_index` gives the index pill,
    /// rather than waiting on the host's echo. `pick_explorer_action`'s exclude/add-back toggle and
    /// the project settings dialog both go through here, so there is one place that sends the
    /// message and applies the answer.
    pub fn set_project_search_excludes(
        &mut self,
        project: ProjectId,
        excludes: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut snapshot) = WindowRegistry::read(cx).project(project).cloned() else {
            return;
        };
        snapshot.record.search_excludes = excludes.clone();
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name: None,
            colour: None,
            custom_colour: None,
            search_excludes: Some(excludes),
            index: None,
            mission_term: None,
            tools: None,
            managed_repos: None,
            lanes: None,
            runs_on: None,
        });
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        cx.notify();
    }

    /// Write a project's whole task-board lane list, sending it at once and updating the snapshot
    /// every window redraws from — the same immediacy `set_project_search_excludes` gives the
    /// excludes. The board behind the dialog is redrawn from the snapshot, so a lane hidden here
    /// leaves the board the moment it is ticked rather than at the host's echo.
    ///
    /// Preferences that say nothing are dropped before the list is sent: a lane the user turned on
    /// and off again is a lane never configured, and a catalogue that wrote a row per lane per
    /// project would be a file of defaults.
    pub fn set_project_lanes(
        &mut self,
        project: ProjectId,
        lanes: Vec<ubiq_proto::projects::LanePref>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut snapshot) = WindowRegistry::read(cx).project(project).cloned() else {
            return;
        };
        let lanes: Vec<_> = lanes.into_iter().filter(|pref| !pref.is_plain()).collect();
        snapshot.record.lanes = lanes.clone();
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name: None,
            colour: None,
            custom_colour: None,
            search_excludes: None,
            index: None,
            mission_term: None,
            tools: None,
            managed_repos: None,
            lanes: Some(lanes),
            runs_on: None,
        });
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        cx.notify();
    }

    /// Write a project's whole runnable-tools list, sending it at once and updating the
    /// snapshot every window redraws from — the same immediacy `set_project_search_excludes`
    /// gives the excludes, rather than waiting on the host's echo. The project tools panel
    /// goes through here, so there is one place that sends the message and applies the answer.
    pub fn set_project_tools(
        &mut self,
        project: ProjectId,
        tools: Vec<ubiq_proto::tools::ToolDef>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut snapshot) = WindowRegistry::read(cx).project(project).cloned() else {
            return;
        };
        snapshot.record.tools = tools.clone();
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name: None,
            colour: None,
            custom_colour: None,
            search_excludes: None,
            index: None,
            mission_term: None,
            tools: Some(tools),
            managed_repos: None,
            lanes: None,
            runs_on: None,
        });
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        cx.notify();
    }

    /// Write a project's whole managed-repositories list, sending it at once and updating the
    /// snapshot every window redraws from — the same immediacy `set_project_search_excludes`
    /// gives the excludes, rather than waiting on the host's echo. A managed repository starts
    /// colouring the explorer and showing on the Git screen as soon as the next working-tree walk
    /// answers, which is why this does not wait on a dialog's Save.
    pub fn set_project_managed_repos(
        &mut self,
        project: ProjectId,
        repos: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut snapshot) = WindowRegistry::read(cx).project(project).cloned() else {
            return;
        };
        snapshot.record.managed_repos = repos.clone();
        self.bus.send(Message::UpdateProject {
            project_id: project,
            name: None,
            colour: None,
            custom_colour: None,
            search_excludes: None,
            index: None,
            mission_term: None,
            tools: None,
            managed_repos: Some(repos),
            lanes: None,
            runs_on: None,
        });
        cx.global_mut::<WindowRegistry>().apply(snapshot);
        cx.notify();
    }

    /// Flip one repository between managed and ignored — the settings dialog's tick box.
    pub fn toggle_project_managed_repo(
        &mut self,
        project: ProjectId,
        rel_path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(mut repos) = WindowRegistry::read(cx)
            .project(project)
            .map(|snap| snap.record.managed_repos.clone())
        else {
            return;
        };
        if !repos.iter().any(|p| p == &rel_path) {
            repos.push(rel_path);
        } else {
            repos.retain(|p| p != &rel_path);
        }
        self.set_project_managed_repos(project, repos, cx);
    }

    /// Drop one pattern from a project's search excludes — the settings dialog's remove control.
    pub fn remove_project_search_exclude(
        &mut self,
        project: ProjectId,
        pattern: String,
        cx: &mut Context<Self>,
    ) {
        let Some(mut excludes) = WindowRegistry::read(cx)
            .project(project)
            .map(|snap| snap.record.search_excludes.clone())
        else {
            return;
        };
        excludes.retain(|p| p != &pattern);
        self.set_project_search_excludes(project, excludes, cx);
    }

    /// The project the settings dialog is editing, if it is open on one that already exists. Only
    /// such a project has a record to override — the same gate `ui::sink::project::form_project`
    /// draws for the index row.
    pub(super) fn editing_project(&self) -> Option<ProjectId> {
        match self.workbench.project_settings.as_ref().map(|s| &s.mode) {
            Some(ProjectSettingsMode::Edit { project }) => Some(*project),
            _ => None,
        }
    }

    /// Add whatever is typed into the settings dialog's exclude field as a new pattern, then clear
    /// the field. Empty text and a pattern already in the list are both ignored, silently: an
    /// empty pattern excludes nothing and a duplicate would only reorder the list.
    pub fn add_project_search_exclude_from_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.editing_project() else {
            return;
        };
        let pattern = self
            .project_exclude_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if pattern.is_empty() {
            return;
        }
        let Some(mut excludes) = WindowRegistry::read(cx)
            .project(project)
            .map(|snap| snap.record.search_excludes.clone())
        else {
            return;
        };
        if excludes.iter().any(|p| p == &pattern) {
            return;
        }
        excludes.push(pattern);
        self.set_project_search_excludes(project, excludes, cx);
        let field = self.project_exclude_input.clone();
        field.update(cx, |state, cx| state.set_value("", window, cx));
    }

    /// Raise the native folder chooser and append the chosen folder, relative to the project's own
    /// root, to its search excludes. A folder outside the project — or the root itself — has no
    /// relative path worth writing, and is silently ignored rather than refused with a dialog.
    pub fn choose_project_search_exclude(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.editing_project() else {
            return;
        };
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Exclude".into()),
        });

        cx.spawn(async move |this, cx| {
            let answer = match chosen.await {
                Ok(answer) => answer,
                Err(_) => return,
            };
            this.update(cx, |this, cx| {
                let Ok(Some(paths)) = answer else {
                    return;
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let Some(root) = WindowRegistry::read(cx)
                    .project(project)
                    .map(|snap| snap.record.path.clone())
                else {
                    return;
                };
                let Some(rel) = path
                    .strip_prefix(&root)
                    .ok()
                    .map(|rel| rel.to_string_lossy().into_owned())
                    .filter(|rel| !rel.is_empty())
                else {
                    return;
                };
                let Some(mut excludes) = WindowRegistry::read(cx)
                    .project(project)
                    .map(|snap| snap.record.search_excludes.clone())
                else {
                    return;
                };
                if excludes.iter().any(|p| p == &rel) {
                    return;
                }
                excludes.push(rel);
                this.set_project_search_excludes(project, excludes, cx);
            })
            .ok();
        })
        .detach();
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
    ///
    /// `storage` is where the project keeps its own data, and the host takes it only here: it is
    /// chosen when the project is created and never changed afterwards.
    #[allow(clippy::too_many_arguments)]
    pub fn add_project(
        &mut self,
        path: String,
        name: Option<String>,
        colour: Option<usize>,
        custom: Option<u32>,
        temporary: bool,
        storage: StorageMode,
        cx: &mut Context<Self>,
    ) {
        self.adding = true;
        self.bus.send(Message::AddProject {
            path,
            name,
            colour,
            custom_colour: custom,
            temporary,
            storage,
        });
        cx.notify();
    }

    /// Choose where a project being created will keep its data. Nothing is sent: the answer rides
    /// on the `AddProject` the panel's Create commits.
    pub fn set_create_storage(&mut self, storage: StorageMode, cx: &mut Context<Self>) {
        self.create_storage = storage;
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
        // Every creation starts from the default, so a folder the user once made project-managed
        // does not silently decide for the next one.
        self.create_storage = StorageMode::default();
        self.workbench.open_menu = None;
        self.workbench.settings.open = false;
        self.workbench.project_settings = Some(ProjectSettings {
            mode: ProjectSettingsMode::Create { path },
            colour: ColourField {
                swatch: colour,
                ..ColourField::default()
            },
            // A folder not in the catalogue yet has no origin to seed from, and the Remote nav is
            // not enabled over it either.
            drone: DroneField::default(),
            nav: ProjectNav::General,
            // A folder with no record has no definitions of its own either.
            definitions_use_global: true,
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
            drone: DroneField::from_origin(snapshot.record.runs_on.as_ref()),
            nav: ProjectNav::General,
            // Read off the definitions rather than stored: a project that has written none of its
            // own is a project on the globals.
            definitions_use_global: !self
                .workbench
                .settings
                .project_definitions
                .iter()
                .any(|it| it.project == Some(project)),
        });
        // The KB nav's sources and the explorer's are the same configuration, so whichever asks
        // first is the one that lands: a dialog opened before the mode was ever visited must not
        // draw a blank Documents page for want of an ask.
        if !self
            .projects
            .get(&project)
            .is_some_and(|open| open.kb.loaded)
        {
            self.ask_kb_sources(project);
        }
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
        let initials = self.project_initials_input.read(cx).value().to_string();
        match settings.mode {
            ProjectSettingsMode::Create { path } => {
                // The record does not exist until `AddProject` answers, so an override typed
                // during creation has nowhere to be sent yet — the same reason `fill_project_form`
                // never seeds this field for Create.
                let storage = self.create_storage;
                self.add_project(path, Some(name), Some(colour), custom, false, storage, cx);
            }
            ProjectSettingsMode::Edit { project } => {
                // Nothing marks this as a promotion: the host treats an `UpdateProject` on a
                // temporary record as the project's entry into the real catalogue.
                self.update_project(project, Some(name), Some(colour), custom, cx);
                self.bus.send(Message::SetProjectInitials {
                    project_id: project,
                    initials,
                });
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
        // Create has no record yet to carry an override — only Edit answers with one.
        let initials = match &settings.mode {
            ProjectSettingsMode::Create { .. } => String::new(),
            ProjectSettingsMode::Edit { project } => WindowRegistry::read(cx)
                .project(*project)
                .map(|entry| entry.record.initials.clone())
                .unwrap_or_default(),
        };
        let path = match &settings.mode {
            ProjectSettingsMode::Create { path } => path.clone(),
            ProjectSettingsMode::Edit { project } => WindowRegistry::read(cx)
                .project(*project)
                .map(|entry| entry.record.path.clone())
                .unwrap_or_default(),
        };
        // The far machine's folder, which is the record's and never this machine's — not
        // abbreviated, because a `~` here would name a home directory on the wrong side.
        let remote_root = match &settings.mode {
            ProjectSettingsMode::Create { .. } => String::new(),
            ProjectSettingsMode::Edit { project } => WindowRegistry::read(cx)
                .project(*project)
                .and_then(|entry| entry.record.runs_on.as_ref())
                .map(|origin| origin.root.clone())
                .unwrap_or_default(),
        };
        // Create has no record yet to carry an override, on `initials`'s rule.
        let mission_term = match &settings.mode {
            ProjectSettingsMode::Create { .. } => String::new(),
            ProjectSettingsMode::Edit { project } => WindowRegistry::read(cx)
                .project(*project)
                .and_then(|entry| entry.record.mission_term.clone())
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
        let initials_input = self.project_initials_input.clone();
        initials_input.update(cx, |input, cx| input.set_value(&initials, window, cx));
        let mission_term_input = self.project_mission_term_input.clone();
        mission_term_input.update(cx, |input, cx| input.set_value(&mission_term, window, cx));
        let path_input = self.project_path_input.clone();
        path_input.update(cx, |input, cx| {
            input.set_value(
                crate::ui::sink::project::home_abbreviated(&path),
                window,
                cx,
            );
        });
        let remote_root_input = self.project_remote_root_input.clone();
        remote_root_input.update(cx, |input, cx| input.set_value(&remote_root, window, cx));
        self.sync_project_form_hex(window, cx);
    }

    pub fn dismiss_project_error(&mut self, cx: &mut Context<Self>) {
        self.workbench.project_error = None;
        cx.notify();
    }

    /// Open the "All projects" modal — the picker's History group hands off here once it has
    /// more than it can show. The picker's own menu closes with it: the modal draws over the
    /// window root, not over the row that raised it, so a menu left open behind it would be a
    /// stale trigger with nothing above it any more.
    pub fn open_all_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        self.workbench.all_projects = Some(crate::state::AllProjectsState::default());
        let field = self.all_projects_search.clone();
        field.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn close_all_projects(&mut self, cx: &mut Context<Self>) {
        self.workbench.all_projects = None;
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
                if let Some((prefs, upgraded)) =
                    prefs::decode_upgraded::<prefs::InterfacePrefs>(&blob)
                {
                    self.workbench.interface_rest = prefs.rest;
                    self.workbench.last_start = prefs.last_start;
                    self.workbench.size_presets = prefs.size_presets;
                    // The whole size axis, the content family's included — one setting for all of
                    // Ubiq, so nothing here is per project any more (`D151`).
                    theme::set_metrics(theme::Metrics {
                        ui_scale: prefs.ui_scale,
                        text_ratio: prefs.text_ratio,
                        content_trim: prefs.content_trim,
                        chrome_trim: prefs.chrome_trim,
                        conversation_trim: prefs.conversation_trim,
                    });
                    // An upgraded blob is still owed the content size, which used to live in each
                    // project's `view.toml`. The first project answer to arrive supplies it.
                    self.content_trim_pending = upgraded;
                    self.workbench.theme_id = prefs.theme;
                    // Before the palette is put on: `theme::resolve` reads the custom list, so a
                    // blob naming a fork has to have handed that list over first — and a slug
                    // naming a theme this config root no longer holds falls back here rather than
                    // painting the default under a dead name (`D152`).
                    self.adopt_custom_themes(prefs.custom_themes);
                    // The palette, the accent *and* the scale reach the component library here:
                    // its `font_size` is the window's rem size (`D153`), so a blob carrying a
                    // scale that never reached it would leave every spacing where it was.
                    theme::set_theme(self.workbench.theme_id, prefs.accent, cx);
                    self.redress_terminals(cx);
                }
            }
            Scope::Project(id) => {
                let Some(view) = prefs::decode::<prefs::ViewPrefs>(&blob) else {
                    return;
                };
                // The one awkward step of the `4 → 5` migration: the content size was written
                // once per project and is becoming a single interface value, so the upgrade has
                // to pick one. It takes the most recently opened project's — the first answer to
                // arrive after the upgraded interface blob — and leaves the per-project field
                // parsed and ignored. A user who had genuinely different sizes per project loses
                // that distinction, which is the point of the correction.
                if self.content_trim_pending {
                    self.content_trim_pending = false;
                    if let Some(size) = view.content_font_size {
                        theme::set_trim(theme::Family::Content, size / theme::TEXT_BASE);
                        self.remember_interface();
                        self.redress_terminals(cx);
                    }
                }
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
                    // The mode's own arrangement, or that mode's defaults when it has none — the
                    // same answer `enter_project` and a mode switch give. A start is the case
                    // that needs it: the window opened on the IDE's default tree before this
                    // answer arrived, so a project left in Git would otherwise wear the IDE's
                    // regions and none of Git's panels. Its refs and changes are put in their
                    // home regions, and `settle_mode` opens the two edges onto them.
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
                    if view.rail_mode == RailMode::Git && saved.layout.is_none() {
                        self.queue_git_furniture();
                    }
                    // The project's own config just moved the mode the window reads its context
                    // from — the same edge a mode switch or a project entry gives follow mode.
                    self.sync_help_follow(cx);
                }
                // A project closed and reopened in this session restored from the parked blob
                // already, and reopening the tabs the user has since closed would be worse than
                // useless.
                if let Some(view) = restore {
                    self.restore_chats(id, &view);
                    self.restore_files(id, &view, cx);
                }
            }
        }
        cx.notify();
    }

    /// Reopen the chat tabs a project's blob remembered, and ask the host for the conversations
    /// behind them.
    ///
    /// Called once per project, on the same first-restore branch the file set uses, and it
    /// replaces the single empty tab [`OpenProject::new`] seeds — the blob had not arrived yet
    /// when that ran.
    ///
    /// The revive is a **re-attach**: `source == agent_id`, so the harness resumes its own session
    /// in the run directory it kept, and the answer arrives as `ConversationStarted` like any
    /// other start. Everything in the blob was persistent when it was written — [`Self::remember`]
    /// writes nothing else down — so every id here is one the host still has a row for.
    fn restore_chats(&mut self, project: ProjectId, view: &prefs::ViewPrefs) {
        if view.chats.is_empty() {
            return;
        }
        let Some(open) = self.projects.get_mut(&project) else {
            return;
        };
        open.chats = seeded_chats(&view.chats);
        let agents: Vec<AgentId> = open.chats.iter().filter_map(|tab| tab.attached).collect();
        for agent in agents {
            self.bus.send(Message::ReviveConversation {
                source: agent,
                agent_id: agent,
                project_id: project,
                session_id: self.session,
            });
        }
        self.sync_chat_panels(project);
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
        //
        // An untitled tab names nothing on disk, so its key is not a path `open_files` could ever
        // reread — it goes to `scratch` instead, by name and by the buffer's own text. Everything
        // else here keeps today's rule unchanged: a real file's unsaved edits are not carried past
        // a close, only reread from what is on disk.
        if let Some(open) = self.projects.get_mut(&project) {
            let mut open_files = Vec::new();
            let mut scratch = Vec::new();
            let mut pinned_files = Vec::new();
            for file in &open.editor.open {
                if file.untitled {
                    // An untitled image capture has no buffer — `OpenFile::buffer` answers `None`
                    // for it — and is dropped rather than remembered: there is no text to carry.
                    if let Some(buffer) = file.buffer() {
                        scratch.push(prefs::Scratch {
                            name: file.path.clone(),
                            text: buffer.read(cx).value().to_string(),
                        });
                    }
                    continue;
                }
                if file.pinned {
                    pinned_files.push(file.key());
                }
                open_files.push(file.key());
            }
            open.prefs.open_files = open_files;
            open.prefs.scratch = scratch;
            open.prefs.pinned_files = pinned_files;
            open.prefs.active_file = open.editor.active_file().map(|file| file.key());
            // The chat tabs, as what each was attached to. A tab attached to nothing leaves
            // nothing to bring back, so it is skipped rather than written as a hole.
            //
            // **Only a persistent conversation is written down**, because only a persistent one
            // survives: the host keeps the rows marked persistent and the boot sweep has already
            // deleted the rest of the run directories, so remembering an ordinary conversation
            // would revive a tab attached to an agent that no longer exists and draw an empty
            // panel. A tab whose conversation was not kept is written as nothing, exactly as a
            // tab attached to nothing is.
            //
            // The persistent mark lives on the host's work snapshot and nowhere else, so this can
            // only be answered once that snapshot has arrived: a project is opened by asking for
            // its preferences and its work in two independent round trips, and the blob's own
            // tabs are re-attached the moment the first lands. **With no work reported yet, what
            // the blob already says is left alone** rather than overwritten with nothing — the
            // same guard `Self::settle_persistent_chat` makes, for the same reason. Reading an
            // empty projection as "nothing was persistent" would throw away a persistent tab the
            // previous blob was right about.
            if !open.work.agents.is_empty() {
                let mut chats = Vec::new();
                for tab in &open.chats {
                    if let Some(agent) = tab.attached
                        && open
                            .work
                            .agent(agent)
                            .is_some_and(|record| record.persistent)
                    {
                        chats.push(agent.to_string());
                    }
                }
                open.prefs.chats = chats;
            }
            open.prefs.expanded = open.explorer.expanded();
            open.prefs.selected = open.explorer.selected.clone();
            open.prefs.file_filter = self.workbench.file_filter.clone();
            open.prefs.board_shut = open.board.shut.clone();
            open.prefs.board_popup = open.board.popup;
            open.prefs.teams_hide_done = open.teams.hide_done;
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

/// What a project would take with it if it closed. Counted on demand — a per-frame count is
/// cheaper than a flag every edit has to remember to set, and it cannot go stale.
#[derive(Default, Clone, Copy)]
pub struct Holds {
    pub files: usize,
    pub panes: usize,
    /// Conversations whose harness is actually up — `Conversation::running`. An unloaded or
    /// ended transcript is not one of these.
    pub agents: usize,
    /// Whether the project is a throwaway clone — see `crate::state::clone::is_ephemeral`. Not a
    /// count, and it outranks the other three in the sentence: what is lost is the whole folder.
    pub ephemeral: bool,
}

impl Holds {
    pub fn anything(self) -> bool {
        self.files > 0 || self.panes > 0 || self.agents > 0 || self.ephemeral
    }

    /// "3 unsaved files, 4 terminals and 2 agents", or `None` when there is nothing to say.
    pub fn sentence(self) -> Option<String> {
        if self.ephemeral {
            return Some("This clone will be discarded".to_string());
        }
        let mut parts = Vec::new();
        if self.files > 0 {
            parts.push(format!("{} unsaved file{}", self.files, plural(self.files)));
        }
        if self.panes > 0 {
            parts.push(format!("{} terminal{}", self.panes, plural(self.panes)));
        }
        if self.agents > 0 {
            parts.push(format!("{} agent{}", self.agents, plural(self.agents)));
        }
        join_parts(&parts)
    }
}

fn join_parts(parts: &[String]) -> Option<String> {
    match parts {
        [] => None,
        [one] => Some(one.clone()),
        [a, b] => Some(format!("{a} and {b}")),
        [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
