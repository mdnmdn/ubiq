use super::*;

impl AppState {
    /// The arrangement as it stands, for the blob the host keeps.
    pub(super) fn layout_blob(&self, cx: &App) -> Option<serde_json::Value> {
        serde_json::to_value(self.dock.read(cx).dump(cx)).ok()
    }

    /// Write down what belongs to the interface as a whole.
    pub fn remember_interface(&mut self) {
        let prefs = prefs::InterfacePrefs {
            schema: prefs::SCHEMA,
            theme: self.workbench.theme_id,
            last_start: self.workbench.last_start.clone(),
            // Whatever the blob carried that this build does not name, put back as it was found.
            rest: self.workbench.interface_rest.clone(),
        };
        self.bus.send(Message::SetPreferences {
            scope: Scope::Interface,
            value: prefs::encode(&prefs),
        });
    }

    /// Write down how the interface behaves. Immediate: a toggle is one event, not a drag.
    pub(super) fn remember_settings(&mut self) {
        let mut ui = self.workbench.settings.ui.clone();
        ui.schema = ui_settings::SCHEMA;
        self.bus.send(Message::SetSettings {
            layer: SettingsLayer::Ui,
            value: ui_settings::encode(&ui),
        });
    }

    /// Write down how the host behaves. The blob is the host's own schema — this half only ever
    /// stamps the version it was built against and hands the rest across, unparsed on the way out
    /// the same way it is unparsed on the way back until the host answers.
    fn remember_host_settings(&mut self) {
        let mut host = self.workbench.settings.host.clone();
        host.schema = HOST_SETTINGS_SCHEMA;
        self.bus.send(Message::SetSettings {
            layer: SettingsLayer::Host,
            value: serde_json::to_string(&host).unwrap_or_default(),
        });
    }

    /// Save or update one remote host's name and address, keyed by address. Called the moment a
    /// dial to that address succeeds — whether it was typed fresh into the titlebar modal or
    /// started as a reconnect from the Hosts section — so a host is durable the first time it is
    /// ever reached, with no separate "save" step for the user to remember. See
    /// [`ubiq_proto::settings::HostSettings::remote_hosts`] for why this never carries the token,
    /// and why this rides `SetSettings` whole rather than a dedicated message the way
    /// `oauth_apps` does.
    pub(super) fn save_remote_host(
        &mut self,
        name: String,
        address: String,
        cx: &mut Context<Self>,
    ) {
        let hosts = &mut self.workbench.settings.host.remote_hosts;
        match hosts.iter_mut().find(|host| host.address == address) {
            Some(existing) => existing.name = name,
            None => hosts.push(SavedRemoteHost { name, address }),
        }
        self.remember_host_settings();
        cx.notify();
    }

    /// Drop a saved host record. Only ever the record — a live connection under that address, if
    /// this window still has one, is untouched; forgetting is about what the *next* window sees
    /// on open, not about the one open right now.
    pub fn forget_remote_host(&mut self, address: String, cx: &mut Context<Self>) {
        self.workbench
            .settings
            .host
            .remote_hosts
            .retain(|host| host.address != address);
        self.workbench.settings.failed_hosts.remove(&address);
        self.remember_host_settings();
        cx.notify();
    }

    /// Toggle the Hosts section's dropdown.
    pub fn toggle_host_picker(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.host_picker_open = !self.workbench.settings.host_picker_open;
        cx.notify();
    }

    /// Pick one row from the Hosts section's dropdown.
    ///
    /// `Local` and an already-attached remote just move `Bus::active` — the default destination
    /// for a message naming neither a pane nor a project, and nothing else; no pane or project
    /// moves with it, on `Bus::set_active`'s own contract. A saved-but-unattached row instead
    /// raises the connect modal on that address, reusing [`Self::reconnect_saved_host`] rather
    /// than dialling here directly.
    pub fn pick_host_menu_entry(
        &mut self,
        entry: super::HostEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::HostEntry;
        self.workbench.settings.host_picker_open = false;
        match entry {
            HostEntry::Local => self.bus.set_active(HostRef::Local),
            HostEntry::Remote { host, .. } => self.bus.set_active(HostRef::Remote(host)),
            HostEntry::Saved { name, address } => {
                self.reconnect_saved_host(name, address, window, cx);
                return;
            }
        }
        cx.notify();
    }

    pub(super) fn apply_settings(
        &mut self,
        layer: SettingsLayer,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match layer {
            SettingsLayer::Ui => {
                let Some(blob) = value else { return };
                if let Some(ui) = ui_settings::decode(&blob) {
                    self.workbench.settings.ui = ui;
                    cx.notify();
                }
            }
            SettingsLayer::Host => {
                // Absent means nothing was ever written: the default already showing is correct,
                // and there is nothing to decode.
                let Some(blob) = value else { return };
                match serde_json::from_str::<HostSettings>(&blob) {
                    Ok(host) => {
                        self.workbench.settings.host = host;
                        self.file_pending_secret();
                        cx.notify();
                    }
                    Err(error) => {
                        tracing::debug!("discarding unreadable host settings: {error}");
                    }
                }
            }
        }
    }

    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        let open = !self.workbench.settings.open;
        self.workbench.settings.open = open;
        if open {
            self.workbench.project_settings = None;
            self.workbench.open_menu = None;
            // Asked on every open rather than once at startup, for the same reason the
            // harness list is: an account logged in from elsewhere should appear without a
            // restart, and the answer is cheap.
            self.bus.send(Message::ListAccounts);
            self.bus.send(Message::ListProfiles);
            self.bus.send(Message::ListAgentTypes);
            // Same reasoning, for the other half of the identities: a connection made in
            // another window should be here without a restart.
            self.bus.send(Message::ListConnections);
        }
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.open = false;
        cx.notify();
    }

    pub fn set_settings_nav(&mut self, nav: SettingsSection, cx: &mut Context<Self>) {
        self.workbench.settings.nav = nav;
        if nav == SettingsSection::Connectors {
            // Asked on arrival for the same reason the shortcut is: a flow that finished in
            // another window has to show up, and the answer is a list the host already holds.
            self.bus.send(Message::ListConnections);
        }
        if nav == SettingsSection::CommandLine {
            // Asked on arrival for the same reason the accounts are: the shortcut can be moved,
            // deleted or left behind by another build while the window is open, and the answer
            // costs a directory listing.
            self.ask_cli_shortcut(CliShortcutAction::Query);
        }
        cx.notify();
    }

    /// Ask after, write or delete the `ubiq` command. All three answer the same way, so one
    /// sender serves the section and `apply_cli_shortcut` is the only thing that draws.
    pub fn ask_cli_shortcut(&mut self, action: CliShortcutAction) {
        self.bus.send(Message::CliShortcut { action });
    }

    pub(super) fn apply_cli_shortcut(&mut self, cli: CliShortcut, cx: &mut Context<Self>) {
        self.workbench.settings.cli = Some(cli);
        cx.notify();
    }

    pub fn toggle_explorer_preview(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.ui.explorer_preview = !self.workbench.settings.ui.explorer_preview;
        self.remember_settings();
        cx.notify();
    }

    /// Show or hide the window-capture control. Off removes the keystroke with it — a
    /// switch that left the shortcut live would not do what its label says.
    pub fn toggle_capture(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.ui.capture_enabled = !self.workbench.settings.ui.capture_enabled;
        self.remember_settings();
        cx.notify();
    }

    /// Turn modal editing on or off. Reached from two places — the settings checkbox and the
    /// status bar's chip — so that the readout and the switch can never disagree.
    pub fn toggle_rail_projects(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.ui.rail_projects = !self.workbench.settings.ui.rail_projects;
        self.remember_settings();
        cx.notify();
    }

    pub fn toggle_vim_mode(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.ui.vim_mode = !self.workbench.settings.ui.vim_mode;
        // Leaving a half-typed command or a Normal-mode caret behind would mean the next time vim
        // is switched on it starts somewhere the user never put it.
        self.vim = VimState::default();
        self.remember_settings();
        cx.notify();
    }

    pub fn set_markdown_open(&mut self, choice: MarkdownOpen, cx: &mut Context<Self>) {
        self.workbench.settings.ui.markdown_open = choice;
        self.remember_settings();
        cx.notify();
    }

    /// Flip the deny-by-default policy an agent spawns under. Host-owned, so this writes the
    /// Host layer rather than the Ui one.
    pub fn toggle_isolate_agents(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.host.isolate_agents = !self.workbench.settings.host.isolate_agents;
        self.remember_host_settings();
        cx.notify();
    }

    /// Where a clone lands by default, and where an ephemeral one lands. Host-owned, so both
    /// write the Host layer. An empty string means "the host's own default" and is stored as
    /// `None` — the interface names no path of its own, so it has no default to write instead.
    pub fn set_clone_root(&mut self, path: String, ephemeral: bool, cx: &mut Context<Self>) {
        let value = (!path.trim().is_empty()).then(|| path.trim().to_string());
        let host = &mut self.workbench.settings.host;
        match ephemeral {
            true => host.ephemeral_root = value,
            false => host.projects_root = value,
        }
        self.remember_host_settings();
        cx.notify();
    }

    /// Ask the operating system which folder that is. The same chooser, and the same three
    /// outcomes, `choose_folder` handles.
    pub fn choose_clone_root(&mut self, ephemeral: bool, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });

        cx.spawn(async move |this, cx| {
            let answer = match chosen.await {
                Ok(answer) => answer,
                Err(_) => return,
            };
            this.update(cx, |this, cx| match answer {
                Ok(Some(paths)) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.set_clone_root(path.to_string_lossy().into_owned(), ephemeral, cx);
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    this.workbench.settings.error =
                        Some(format!("could not open a chooser: {error}"));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The globs every project search skips. Host-owned, and committed rather than typed.
    pub fn set_search_excludes(&mut self, globs: Vec<String>, cx: &mut Context<Self>) {
        if self.workbench.settings.host.search_excludes == globs {
            return;
        }
        self.workbench.settings.host.search_excludes = globs;
        self.remember_host_settings();
        cx.notify();
    }

    /// How much of a project Ubiq indexes, for every project that does not override it.
    ///
    /// The host acts on the change for every project that is open — turning indexing off and
    /// watching nothing happen would read as a setting that does not work.
    pub fn set_index_level(
        &mut self,
        level: ubiq_proto::projects::IndexLevel,
        cx: &mut Context<Self>,
    ) {
        if self.workbench.settings.host.index_level == level {
            return;
        }
        self.workbench.settings.host.index_level = level;
        self.remember_host_settings();
        cx.notify();
    }

    /// The external tools a search may fall back to, in the order they are tried.
    pub fn set_search_fallbacks(&mut self, tools: Vec<String>, cx: &mut Context<Self>) {
        if self.workbench.settings.host.search_fallbacks == tools {
            return;
        }
        self.workbench.settings.host.search_fallbacks = tools;
        self.remember_host_settings();
        cx.notify();
    }

    /// The two search fields hold what the host has stored, on the Git fields' rule: mirrored while
    /// the dialog is up and never while the field is being typed into.
    pub(super) fn sync_search_settings_fields(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workbench.settings.open {
            return;
        }
        for (field, wanted) in [
            (
                self.search_excludes_input.clone(),
                self.workbench.settings.host.search_excludes.join(", "),
            ),
            (
                self.search_fallbacks_input.clone(),
                self.workbench.settings.host.search_fallbacks.join(", "),
            ),
        ] {
            if !field.read(cx).focus_handle(cx).is_focused(window)
                && field.read(cx).value() != wanted.as_str()
            {
                field.update(cx, |state, cx| state.set_value(&wanted, window, cx));
            }
        }
    }

    // ── Harness logins ──────────────────────────────────────────────

    /// Raise the login modal, on the step where nothing has happened yet.
    ///
    /// The name field is emptied here rather than left holding the last identity typed: two
    /// logins in a row are two different accounts far more often than they are the same one,
    /// and a prefilled name is how the second one silently overwrites the first.
    pub fn open_harness_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.login_account_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.workbench.settings.login = Some(LoginState {
            account: String::new(),
            step: LoginStep::Choosing { agent_type: None },
            links: Vec::new(),
            probe: false,
        });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Pick which harness the login is for. Re-picking before it starts is free.
    pub fn pick_login_harness(&mut self, agent_type: String, cx: &mut Context<Self>) {
        if let Some(login) = &mut self.workbench.settings.login
            && let LoginStep::Choosing { agent_type: chosen } = &mut login.step
        {
            *chosen = Some(agent_type);
            cx.notify();
        }
    }

    /// Start the harness's own login flow. The host answers with the pane it runs in.
    pub fn start_harness_login(&mut self, cx: &mut Context<Self>) {
        self.begin_harness_login(false, cx);
    }

    /// Open a plain shell under this login's own sandbox instead of running the harness — the
    /// same harness and identity the `Sign in` button would use, but a diagnostic rather than a
    /// way to sign in: it never captures a credential, and the host never treats its exit as a
    /// login outcome. See `Message::BeginHarnessLogin`'s `probe` field.
    pub fn start_harness_shell(&mut self, cx: &mut Context<Self>) {
        self.begin_harness_login(true, cx);
    }

    /// The shared core of [`Self::start_harness_login`] and [`Self::start_harness_shell`]: same
    /// harness and account selection, same `Starting` transition, and the same message —
    /// `probe` is the only thing that differs.
    ///
    /// The account name is read here rather than tracked per keystroke: it is only needed at
    /// the moment the flow starts, and a field the interface mirrors into its own state is a
    /// second copy that can disagree with the one on screen.
    fn begin_harness_login(&mut self, probe: bool, cx: &mut Context<Self>) {
        let account = self.login_account_input.read(cx).value().trim().to_string();
        let Some(login) = &mut self.workbench.settings.login else {
            return;
        };
        let LoginStep::Choosing {
            agent_type: Some(agent_type),
        } = &login.step
        else {
            return;
        };
        // Both are required and the button is disabled without them, so this is the
        // belt-and-braces case rather than a path the user can reach.
        if account.is_empty() {
            return;
        }

        let agent_type = agent_type.clone();
        login.account = account.clone();
        login.probe = probe;
        login.step = LoginStep::Starting {
            agent_type: agent_type.clone(),
        };
        self.bus.send(Message::BeginHarnessLogin {
            agent_type,
            account,
            probe,
        });
        cx.notify();
    }

    /// Abandon a running login, or dismiss a finished one.
    ///
    /// Closing the pane is what abandons it, and that is safe by construction: a login that
    /// did not write a credential captured nothing, and the host says so rather than
    /// recording a half-made account.
    pub fn close_harness_login(&mut self, cx: &mut Context<Self>) {
        if let Some(login) = self.workbench.settings.login.take()
            && let LoginStep::Running { pane } = login.step
        {
            self.close_login_pane(pane, cx);
        }
        cx.notify();
    }

    /// The login is running: adopt its pane so the modal can draw it.
    ///
    /// A login pane belongs to no project, so it joins no project's pane list and gets no
    /// dock panel — the modal is the only thing that renders it, which is also what keeps
    /// one emulator from being drawn in two places.
    pub(super) fn login_started(
        &mut self,
        pane_id: PaneId,
        agent_type: String,
        account: String,
        cols: u16,
        rows: u16,
        cx: &mut Context<Self>,
    ) {
        // A login whose modal has already gone has nobody to draw it, and a harness nobody
        // can see is a leak — the same rule `open_pane` applies to an orphaned pane.
        if self.workbench.settings.login.is_none() {
            tracing::info!("login pane {pane_id} arrived with no modal to draw it");
            self.bus.send(Message::CloseWorkspace { pane_id });
            return;
        }

        // `Starting`'s own `LoginState` already carries whether this is a probe; read it before
        // it is replaced rather than defaulting to false, or a probe's running step would draw
        // as an ordinary sign-in.
        let probe = self
            .workbench
            .settings
            .login
            .as_ref()
            .is_some_and(|login| login.probe);

        self.open_terminal(pane_id, cols, rows, theme::TERMINAL_FONT_SIZE, cx);
        self.workbench.settings.login = Some(LoginState {
            account,
            step: LoginStep::Running { pane: pane_id },
            links: Vec::new(),
            probe,
        });
        self.pending_focus = Some(pane_id);
        self.bus.send(Message::Focus { pane_id });
        tracing::info!("login for {agent_type} running in pane {pane_id}");
        cx.notify();
    }

    /// The login ended. Show what came of it, and stop drawing its pane.
    pub(super) fn login_ended(&mut self, captured: bool, message: String, cx: &mut Context<Self>) {
        // The outcome arrives whether or not the modal is still up — a login the user walked
        // away from still finished — so a captured account is recorded either way and only
        // the display is conditional.
        if let Some(pane) = self.login_pane() {
            self.close_login_pane(pane, cx);
        }
        if let Some(login) = &mut self.workbench.settings.login {
            login.step = LoginStep::Done { captured, message };
            login.links.clear();
        }
        cx.notify();
    }

    /// A URL the running login's own output printed. Pushed only while a login is running in
    /// exactly this pane — a link for a pane that is not the login's own, or one arriving with
    /// no login up at all, is ignored rather than misfiled onto the wrong flow.
    ///
    /// Capped and deduplicated here as well as on the host: state must not grow from a
    /// misbehaving one.
    pub(super) fn login_link(&mut self, pane_id: PaneId, url: String, cx: &mut Context<Self>) {
        let Some(login) = &mut self.workbench.settings.login else {
            return;
        };
        if !matches!(login.step, LoginStep::Running { pane } if pane == pane_id) {
            return;
        }
        if login.links.contains(&url) || login.links.len() >= MAX_LOGIN_LINKS {
            return;
        }
        login.links.push(url);
        cx.notify();
    }

    /// The pane a login is running in, when one is.
    pub fn login_pane(&self) -> Option<PaneId> {
        match &self.workbench.settings.login {
            Some(login) => match login.step {
                LoginStep::Running { pane } => Some(pane),
                _ => None,
            },
            None => None,
        }
    }

    /// Stop drawing a login's pane and tell the host to end it.
    fn close_login_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        self.terminals.remove(&pane);
        if self.pending_focus == Some(pane) {
            self.pending_focus = None;
        }
        self.bus.send(Message::CloseWorkspace { pane_id: pane });
        cx.notify();
    }

    /// Ask whether an account's credential for a harness is still good. No modal: the status
    /// line updates in place when `HarnessLoginStatus` answers.
    pub fn check_harness_login(
        &mut self,
        agent_type: String,
        account: String,
        cx: &mut Context<Self>,
    ) {
        self.workbench.settings.error = None;
        self.bus.send(Message::CheckHarnessLogin {
            agent_type,
            account,
        });
        cx.notify();
    }

    /// Re-authenticate a harness that already has a name: an ordinary login, skipping the
    /// picker because both the harness and the identity are already known.
    ///
    /// `login` is set before the send, on the same reasoning `login_started` requires it —
    /// an answer with no modal to draw it closes the pane instead. The harness may well say
    /// it is already logged in; that is its own output for the user to read, not something
    /// this method pre-empts.
    pub fn reauthenticate_harness(
        &mut self,
        agent_type: String,
        account: String,
        cx: &mut Context<Self>,
    ) {
        self.workbench.settings.error = None;
        self.workbench.settings.login = Some(LoginState {
            account: account.clone(),
            step: LoginStep::Starting {
                agent_type: agent_type.clone(),
            },
            links: Vec::new(),
            probe: false,
        });
        self.bus.send(Message::BeginHarnessLogin {
            agent_type,
            account,
            probe: false,
        });
        cx.notify();
    }

    /// Raise the rename dialog, seeded with the account's current id.
    pub fn open_rename_account(
        &mut self,
        account: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.account_rename_input
            .update(cx, |state, cx| state.set_value(&account, window, cx));
        self.workbench.settings.dialog = Some(AccountDialog::Rename { account });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Send the rename. The dialog closes optimistically; a refusal comes back as
    /// `AccountError` and reads as the banner in the harnesses section, and `Accounts`
    /// redraws the list on success — this method mutates no account itself.
    pub fn confirm_rename_account(&mut self, cx: &mut Context<Self>) {
        let new_account = self
            .account_rename_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(AccountDialog::Rename { account }) = self.workbench.settings.dialog.take() else {
            return;
        };
        if new_account.is_empty() {
            return;
        }
        self.bus.send(Message::RenameAccount {
            account,
            new_account,
        });
        cx.notify();
    }

    /// Raise the delete confirmation, over one account.
    pub fn open_delete_account(&mut self, account: String, cx: &mut Context<Self>) {
        self.workbench.settings.dialog = Some(AccountDialog::Delete { account });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Delete the account and every harness login inside it.
    pub fn confirm_delete_account(&mut self, cx: &mut Context<Self>) {
        let Some(AccountDialog::Delete { account }) = self.workbench.settings.dialog.take() else {
            return;
        };
        self.bus.send(Message::DeleteAccount { account });
        cx.notify();
    }

    /// Raise the sign-out confirmation, over one harness on one account.
    pub fn open_sign_out(&mut self, agent_type: String, account: String, cx: &mut Context<Self>) {
        self.workbench.settings.dialog = Some(AccountDialog::SignOut {
            agent_type,
            account,
        });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Sign one harness out, leaving the account and its other harnesses alone.
    pub fn confirm_sign_out(&mut self, cx: &mut Context<Self>) {
        let Some(AccountDialog::SignOut {
            agent_type,
            account,
        }) = self.workbench.settings.dialog.take()
        else {
            return;
        };
        self.bus.send(Message::DeleteHarnessLogin {
            agent_type,
            account,
        });
        cx.notify();
    }

    /// Dismiss whichever account dialog is up, with nothing sent.
    pub fn close_account_dialog(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.dialog = None;
        cx.notify();
    }

    /// Dismiss the last refusal the host reported for an account action.
    pub fn dismiss_account_error(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.error = None;
        cx.notify();
    }

    // ── Profiles ────────────────────────────────────────────────────

    /// Raise the profile form. `profile` is `None` for a new setup, or the one being edited —
    /// the id is what the host overwrites by, so editing keeps it and typing a new one saves a
    /// second profile rather than renaming the first.
    ///
    /// The two typed fields are seeded here, the way the rename dialog seeds its own: they are
    /// read back only at save time, so nothing mirrors them per keystroke.
    pub fn open_profile_form(
        &mut self,
        profile: Option<ProfileInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = profile.unwrap_or(ProfileInfo {
            id: String::new(),
            agent_type: String::new(),
            account: None,
            model: None,
            mode: None,
        });
        self.profile_id_input
            .update(cx, |state, cx| state.set_value(&profile.id, window, cx));
        self.profile_model_input.update(cx, |state, cx| {
            state.set_value(profile.model.as_deref().unwrap_or(""), window, cx)
        });
        self.workbench.settings.profile_form = Some(profile);
        self.workbench.settings.error = None;
        cx.notify();
    }

    pub fn close_profile_form(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.profile_form = None;
        cx.notify();
    }

    /// Pick which harness the setup is for. The mode goes with it: the choices come from the
    /// harness's own list, so one kept across a switch would name a mode the new harness has
    /// never heard of.
    pub fn pick_profile_harness(&mut self, agent_type: String, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.settings.profile_form
            && form.agent_type != agent_type
        {
            form.agent_type = agent_type;
            form.mode = None;
            // The account may not be signed in to the new harness either.
            form.account = None;
            cx.notify();
        }
    }

    /// Pick the identity, or clear it by picking the one already chosen.
    pub fn pick_profile_account(&mut self, account: Option<String>, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.settings.profile_form {
            form.account = account;
            cx.notify();
        }
    }

    pub fn pick_profile_mode(&mut self, mode: Option<String>, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.settings.profile_form {
            form.mode = mode;
            cx.notify();
        }
    }

    /// Write the setup down. The host answers with `Profiles`, or with `AccountError` when the
    /// id is not a name it can file — profiles are stored beside accounts and fail the same way.
    pub fn save_profile(&mut self, cx: &mut Context<Self>) {
        let id = self.profile_id_input.read(cx).value().trim().to_string();
        let model = self.profile_model_input.read(cx).value().trim().to_string();
        let Some(form) = self.workbench.settings.profile_form.take() else {
            return;
        };
        // Both are required and the button is disabled without them: belt to its braces.
        if id.is_empty() || form.agent_type.is_empty() {
            self.workbench.settings.profile_form = Some(form);
            return;
        }
        self.bus.send(Message::SaveProfile {
            profile: ProfileInfo {
                id,
                model: (!model.is_empty()).then_some(model),
                ..form
            },
        });
        cx.notify();
    }

    // ── Connectors ──────────────────────────────────────────────────

    /// Raise the connect modal on its picker, with all three fields empty.
    ///
    /// The id is minted here and not before: it is what every stage of this flow is matched
    /// against, so a second flow can never be answered by the first one's messages.
    pub fn open_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_connect_inputs(window, cx);
        self.workbench.settings.error = None;
        self.workbench.settings.connect = Some(ConnectState {
            connect_id: ConnectId::generate(),
            label: String::new(),
            provider: ProviderId::Github,
            instance: None,
            app: ConnectApp::Instance,
            app_open: false,
            step: ConnectStep::Choosing {
                provider: None,
                auth: None,
            },
        });
        cx.notify();
    }

    /// Pick a provider. The flow choice goes with it: which flows exist is a property of the
    /// provider and the instance, so a kind picked for the last provider may not be on offer.
    pub fn pick_connect_provider(&mut self, provider: ProviderId, cx: &mut Context<Self>) {
        let app = self.first_connect_app(provider);
        if let Some(connect) = &mut self.workbench.settings.connect {
            connect.provider = provider;
            connect.app = app;
            connect.app_open = false;
            connect.step = ConnectStep::Choosing {
                provider: Some(provider),
                auth: None,
            };
        }
        cx.notify();
    }

    /// What the application picker opens on for a provider: this build's own application where
    /// there is one, otherwise the first registration, otherwise a typed URL.
    ///
    /// Whether the build ships one is the host's answer and never a guess — an interface that
    /// assumed a default would open a browser at an authorization URL with no client id in it.
    pub fn first_connect_app(&self, provider: ProviderId) -> ConnectApp {
        if self.workbench.settings.bundled.contains(&provider) {
            return ConnectApp::Bundled;
        }
        match self
            .workbench
            .settings
            .host
            .oauth_apps
            .iter()
            .find(|app| app.provider == provider)
        {
            Some(app) => ConnectApp::Registration(app.id),
            None => ConnectApp::Instance,
        }
    }

    /// Which application this flow is to authenticate as. Picking one sets where the connection
    /// lives as well, since a registration already knows its own instance.
    pub fn pick_connect_app(&mut self, app: ConnectApp, cx: &mut Context<Self>) {
        if let Some(connect) = &mut self.workbench.settings.connect {
            connect.app = app;
            connect.app_open = false;
        }
        cx.notify();
    }

    pub fn toggle_connect_app_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(connect) = &mut self.workbench.settings.connect {
            connect.app_open = !connect.app_open;
        }
        cx.notify();
    }

    pub fn pick_connect_auth(&mut self, auth: AuthKind, cx: &mut Context<Self>) {
        if let Some(connect) = &mut self.workbench.settings.connect
            && let ConnectStep::Choosing { provider, .. } = connect.step
        {
            connect.step = ConnectStep::Choosing {
                provider,
                auth: Some(auth),
            };
        }
        cx.notify();
    }

    /// Send `BeginConnect`. The three fields are read here rather than mirrored per keystroke,
    /// the reasoning `begin_harness_login` sets out.
    pub fn start_connect(&mut self, cx: &mut Context<Self>) {
        let label = self.login_account_input.read(cx).value().trim().to_string();
        let instance = self
            .connect_instance_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let client_id = self
            .connect_client_id_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        // A registration answers both questions at once: where the identity lives, and which
        // application the flow authenticates as. Read before the borrow below, since it reads the
        // registration list the same state holds.
        let chosen = self
            .workbench
            .settings
            .connect
            .as_ref()
            .map(|connect| connect.app);
        let registration = match chosen {
            Some(ConnectApp::Registration(id)) => self
                .workbench
                .settings
                .host
                .oauth_apps
                .iter()
                .find(|app| app.id == id)
                .cloned(),
            _ => None,
        };
        let Some(connect) = &mut self.workbench.settings.connect else {
            return;
        };
        let ConnectStep::Choosing {
            provider: Some(provider),
            auth: Some(auth),
        } = connect.step
        else {
            return;
        };
        if label.is_empty() {
            return;
        }
        let (instance, client_id, oauth_app) = match &registration {
            Some(app) => (
                app.origin.clone(),
                Some(app.client_id.clone()),
                Some(app.id),
            ),
            None => (
                (!instance.is_empty()).then_some(instance),
                (!client_id.is_empty()).then_some(client_id),
                None,
            ),
        };
        connect.label = label.clone();
        connect.provider = provider;
        connect.instance = instance.clone();
        connect.step = ConnectStep::Starting;
        self.bus.send(Message::BeginConnect {
            connect_id: connect.connect_id,
            provider,
            instance,
            label,
            auth,
            client_id,
            oauth_app,
        });
        cx.notify();
    }

    /// Answer a `NeedSecret` stage with what was pasted. The field is cleared straight away:
    /// the value is on the bus, and there is no reason for it to stay on screen.
    pub fn submit_connect_secret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.connect_secret_input.read(cx).value().to_string();
        let Some(connect) = &self.workbench.settings.connect else {
            return;
        };
        if value.trim().is_empty() {
            return;
        }
        self.bus.send(Message::SubmitConnectSecret {
            connect_id: connect.connect_id,
            secret: Secret::new(value),
        });
        self.connect_secret_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    /// Abandon a flow. The certificate question goes with it — it belongs to this flow and
    /// answering it after the flow is gone would resume nothing.
    pub fn cancel_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(connect) = self.workbench.settings.connect.take() {
            self.bus.send(Message::CancelConnect {
                connect_id: connect.connect_id,
            });
        }
        self.workbench.settings.cert = None;
        self.clear_connect_inputs(window, cx);
        cx.notify();
    }

    /// Back to the picker after a failure, with the fields as they were left: a wrong instance
    /// URL is corrected by editing it, not by typing the whole form again.
    pub fn retry_connect(&mut self, cx: &mut Context<Self>) {
        if let Some(connect) = &mut self.workbench.settings.connect {
            connect.step = ConnectStep::Choosing {
                provider: Some(connect.provider),
                auth: None,
            };
        }
        cx.notify();
    }

    fn clear_connect_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            // The label field is the login modal's — the two modals occlude the settings page
            // that raises them, so neither can be on screen while the other is.
            &self.login_account_input,
            &self.connect_instance_input,
            &self.connect_client_id_input,
            &self.connect_secret_input,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    /// Ask a connection's provider whether its token still works. `probe: true` is a live
    /// handshake, which is why this is a button and not something a render does.
    pub fn check_connection(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.workbench.settings.error = None;
        self.bus.send(Message::CheckConnection {
            connection,
            probe: true,
        });
        cx.notify();
    }

    /// Raise the rename dialog, seeded with the connection's current label.
    ///
    /// The account dialog is dropped rather than left underneath: both draw
    /// `account_rename_input`, and one field in two places is one field too many.
    pub fn open_rename_connection(
        &mut self,
        connection: ConnectionId,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.account_rename_input
            .update(cx, |state, cx| state.set_value(&label, window, cx));
        self.workbench.settings.dialog = None;
        self.workbench.settings.connector = Some(ConnectorDialog::Rename { connection, label });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Send the rename. The dialog closes optimistically; a refusal comes back as
    /// `ConnectorError` and reads as the banner in this section.
    pub fn confirm_rename_connection(&mut self, cx: &mut Context<Self>) {
        let label = self
            .account_rename_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(ConnectorDialog::Rename { connection, .. }) =
            self.workbench.settings.connector.take()
        else {
            return;
        };
        if label.is_empty() {
            return;
        }
        self.bus
            .send(Message::RenameConnection { connection, label });
        cx.notify();
    }

    pub fn open_disconnect(
        &mut self,
        connection: ConnectionId,
        label: String,
        cx: &mut Context<Self>,
    ) {
        self.workbench.settings.connector = Some(ConnectorDialog::Disconnect { connection, label });
        self.workbench.settings.error = None;
        cx.notify();
    }

    pub fn confirm_disconnect(&mut self, cx: &mut Context<Self>) {
        let Some(ConnectorDialog::Disconnect { connection, .. }) =
            self.workbench.settings.connector.take()
        else {
            return;
        };
        self.bus.send(Message::DeleteConnection { connection });
        cx.notify();
    }

    /// Raise the forget-certificate question. `uses` is counted by the caller, which is where
    /// the connection list is already in hand.
    pub fn open_forget_cert(&mut self, origin: String, uses: usize, cx: &mut Context<Self>) {
        self.workbench.settings.connector = Some(ConnectorDialog::ForgetCert { origin, uses });
        self.workbench.settings.error = None;
        cx.notify();
    }

    pub fn confirm_forget_cert(&mut self, cx: &mut Context<Self>) {
        let Some(ConnectorDialog::ForgetCert { origin, .. }) =
            self.workbench.settings.connector.take()
        else {
            return;
        };
        self.bus.send(Message::ForgetCertificate { origin });
        cx.notify();
    }

    /// Vouch for the certificate the host offered, and resume the flow.
    ///
    /// The fingerprint sent back is the one out of the held prompt and never a recomputed
    /// one — that is what makes the confirmation the user's answer rather than a formality.
    pub fn trust_certificate(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.workbench.settings.cert.take() else {
            return;
        };
        self.bus.send(Message::TrustCertificate {
            connect_id: prompt.connect_id,
            origin: prompt.origin,
            sha256: prompt.cert.sha256,
        });
        cx.notify();
    }

    /// Decline the certificate. The flow stays stopped until it times out or is cancelled —
    /// nothing is sent, because there is no "no" on the wire and inventing one would be a lie
    /// about what the host was told.
    pub fn cancel_certificate(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.cert = None;
        cx.notify();
    }

    /// Raise the application-registration form — empty for a new one, seeded for an existing one.
    ///
    /// The four fields are the connect modal's: the two modals occlude each other, so neither can
    /// be on screen while the other is. The secret box is always left empty, because a stored
    /// secret is never read back — the row says whether there is one and nothing shows it.
    pub fn open_app_form(
        &mut self,
        id: Option<OauthAppId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let held = id.and_then(|id| {
            self.workbench
                .settings
                .host
                .oauth_apps
                .iter()
                .find(|app| app.id == id)
                .cloned()
        });
        let provider = held.as_ref().map_or(ProviderId::Github, |app| app.provider);
        let url = match &held {
            Some(app) => app
                .origin
                .clone()
                .or_else(|| provider.cloud_url().map(str::to_string))
                .unwrap_or_default(),
            None => provider.cloud_url().unwrap_or_default().to_string(),
        };
        let name = held.as_ref().map_or(String::new(), |app| app.name.clone());
        let client_id = held.map_or(String::new(), |app| app.client_id);
        self.set_app_form_fields(&name, &url, &client_id, window, cx);
        self.workbench.settings.connect = None;
        self.workbench.settings.connector = None;
        self.workbench.settings.app_form = Some(AppForm {
            id,
            provider,
            open: false,
        });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Pick the provider a registration is at. The URL follows it, so the field is never left
    /// naming a different provider's cloud — unless the user has already typed their own install,
    /// which is theirs and is kept.
    pub fn pick_app_provider(
        &mut self,
        provider: ProviderId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let typed = self.connect_instance_input.read(cx).value().to_string();
        let was_a_cloud = typed.trim().is_empty()
            || ProviderId::all()
                .iter()
                .any(|other| other.cloud_url() == Some(typed.trim()));
        if was_a_cloud {
            let url = provider.cloud_url().unwrap_or_default().to_string();
            self.connect_instance_input
                .update(cx, |state, cx| state.set_value(&url, window, cx));
        }
        if let Some(form) = &mut self.workbench.settings.app_form {
            form.provider = provider;
            form.open = false;
        }
        cx.notify();
    }

    pub fn toggle_app_provider_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.settings.app_form {
            form.open = !form.open;
        }
        cx.notify();
    }

    /// Send the form. The host mints the id, validates the name and the client id, and answers
    /// with the settings blob every window draws from — or with a refusal, which reads as the
    /// banner in this section.
    pub fn save_app_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.login_account_input.read(cx).value().trim().to_string();
        let url = self
            .connect_instance_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let client_id = self
            .connect_client_id_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let secret = self.connect_secret_input.read(cx).value().to_string();
        let Some(form) = self.workbench.settings.app_form.take() else {
            return;
        };
        if name.is_empty() || client_id.is_empty() {
            self.workbench.settings.app_form = Some(form);
            return;
        }
        // A URL equal to the provider's own cloud is that cloud, and stores nothing: `origin` is
        // what a settings match and a certificate pin are keyed by, and "the cloud" is `None`.
        let origin = match form.provider.cloud_url() {
            Some(cloud) if cloud == url => None,
            _ => origin(&url),
        };
        self.bus.send(Message::SaveOauthApp {
            id: form.id,
            provider: form.provider,
            name: name.clone(),
            origin,
            client_id: client_id.clone(),
        });
        match form.id {
            // An existing registration already has an id to file a secret under.
            Some(app) => self.send_app_secret(app, &secret),
            // A new one does not, so the secret waits for the record to come back with one.
            None if !secret.trim().is_empty() => {
                self.workbench.settings.pending_secret = Some(PendingSecret {
                    name,
                    client_id,
                    secret,
                });
            }
            None => {}
        }
        self.clear_connect_inputs(window, cx);
        cx.notify();
    }

    pub fn close_app_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.settings.app_form = None;
        self.clear_connect_inputs(window, cx);
        cx.notify();
    }

    pub fn open_delete_app(&mut self, app: OauthAppId, name: String, cx: &mut Context<Self>) {
        self.workbench.settings.connector = Some(ConnectorDialog::DeleteApp { app, name });
        self.workbench.settings.error = None;
        cx.notify();
    }

    pub fn confirm_delete_app(&mut self, cx: &mut Context<Self>) {
        let Some(ConnectorDialog::DeleteApp { app, .. }) = self.workbench.settings.connector.take()
        else {
            return;
        };
        self.bus.send(Message::DeleteOauthApp { id: app });
        cx.notify();
    }

    /// Set or clear one registration's client secret. Blank clears, because a field the user
    /// emptied is a secret they meant to be rid of.
    fn send_app_secret(&mut self, app: OauthAppId, secret: &str) {
        if secret.trim().is_empty() {
            self.bus.send(Message::ClearAppSecret { app });
        } else {
            self.bus.send(Message::SetAppSecret {
                app,
                secret: Secret::new(secret.to_string()),
            });
        }
    }

    /// File a secret that was typed for a registration the host had not minted an id for yet.
    ///
    /// Matched on the two fields the user typed rather than on position, since another window may
    /// have written a registration of its own in between. A secret that matches nothing is dropped
    /// rather than kept waiting: the save it belonged to was refused.
    fn file_pending_secret(&mut self) {
        let Some(pending) = self.workbench.settings.pending_secret.take() else {
            return;
        };
        let Some(app) = self
            .workbench
            .settings
            .host
            .oauth_apps
            .iter()
            .find(|app| pending.matches(app))
            .map(|app| app.id)
        else {
            return;
        };
        self.send_app_secret(app, &pending.secret);
    }

    fn set_app_form_fields(
        &mut self,
        name: &str,
        url: &str,
        client_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, value) in [
            (&self.login_account_input, name),
            (&self.connect_instance_input, url),
            (&self.connect_client_id_input, client_id),
            (&self.connect_secret_input, ""),
        ] {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    pub fn close_connector_dialog(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.connector = None;
        cx.notify();
    }

    // ── Explorer ────────────────────────────────────────────────────
}
