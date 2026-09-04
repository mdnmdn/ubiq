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
            // Whatever the blob carried that this build does not name, put back as it was found.
            rest: self.workbench.interface_rest.clone(),
        };
        self.bus.send(Message::SetPreferences {
            scope: Scope::Interface,
            value: prefs::encode(&prefs),
        });
    }

    /// Write down how the interface behaves. Immediate: a toggle is one event, not a drag.
    fn remember_settings(&mut self) {
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
            self.bus.send(Message::ListAgentTypes);
        }
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.open = false;
        cx.notify();
    }

    pub fn set_settings_nav(&mut self, nav: SettingsSection, cx: &mut Context<Self>) {
        self.workbench.settings.nav = nav;
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

    /// The globs every project search skips. Host-owned, and committed rather than typed.
    pub fn set_search_excludes(&mut self, globs: Vec<String>, cx: &mut Context<Self>) {
        if self.workbench.settings.host.search_excludes == globs {
            return;
        }
        self.workbench.settings.host.search_excludes = globs;
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
    ///
    /// The account name is read here rather than tracked per keystroke: it is only needed at
    /// the moment the flow starts, and a field the interface mirrors into its own state is a
    /// second copy that can disagree with the one on screen.
    pub fn start_harness_login(&mut self, cx: &mut Context<Self>) {
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
        login.step = LoginStep::Starting {
            agent_type: agent_type.clone(),
        };
        self.bus.send(Message::BeginHarnessLogin {
            agent_type,
            account,
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

        self.open_terminal(pane_id, cols, rows, theme::TERMINAL_FONT_SIZE, cx);
        self.workbench.settings.login = Some(LoginState {
            account,
            step: LoginStep::Running { pane: pane_id },
            links: Vec::new(),
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
        });
        self.bus.send(Message::BeginHarnessLogin {
            agent_type,
            account,
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

    // ── Explorer ────────────────────────────────────────────────────
}
