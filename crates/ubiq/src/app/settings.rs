use super::*;
use crate::state::new_agent::{NewAgentForm, Purpose};

/// What a kept agent home is called when the choice is made before a name is typed. A home with
/// no name is not a state the host can act on, so the choice never stores one.
const DEFAULT_AGENT_HOME: &str = "default";

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
            accent: theme::accent_id(),
            density: theme::density(),
            // Two of the three text bases. The third is the content family's, which is the
            // project's and travels in `ViewPrefs`.
            chrome_font_size: Some(theme::text_scale().chrome),
            conversation_font_size: Some(theme::text_scale().conversation),
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
    ///
    /// One field on it is host-mutated and travels here only as an echo: `ai_providers`, which the
    /// host discards whatever this sends, because a provider record and the key filed under its id
    /// go together and only the host can write the key. Providers change through the four provider
    /// messages instead — see the assistance section below.
    fn remember_host_settings(&mut self) {
        let mut host = self.workbench.settings.host.clone();
        host.schema = HOST_SETTINGS_SCHEMA;
        self.bus.send(Message::SetSettings {
            layer: SettingsLayer::Host,
            value: serde_json::to_string(&host).unwrap_or_default(),
        });
    }

    /// Save or update one remote host's name, address, scheme and trust flag, keyed by its
    /// stable id — minted here when the dial that proved it reachable was the entry's first.
    /// Called the moment a dial succeeds, so a host is durable the first time it is ever reached,
    /// with no separate "save" step for the user to remember. Answers with the id, which is what
    /// the keychain token and the reconnect loop reference. See
    /// [`ubiq_proto::settings::HostSettings::remote_hosts`] for why this never carries the token,
    /// and why this rides `SetSettings` whole rather than a dedicated message the way
    /// `oauth_apps` does.
    pub(super) fn save_remote_host(
        &mut self,
        save_id: String,
        name: String,
        address: String,
        scheme: RemoteScheme,
        trust_insecure: bool,
        cx: &mut Context<Self>,
    ) -> String {
        let hosts = &mut self.workbench.settings.host.remote_hosts;
        // The id is stable; the address is not — an edit moves the entry, and a legacy entry
        // with no id yet is matched by the address it was first reached at, then given one.
        let id = if save_id.is_empty() {
            if let Some(existing) = hosts.iter_mut().find(|host| host.address == address) {
                existing.name = name;
                existing.scheme = scheme;
                existing.trust_insecure = trust_insecure;
                if existing.id.is_empty() {
                    existing.id = ubiq_proto::ids::HostSaveId::generate().to_string();
                }
                existing.id.clone()
            } else {
                let id = ubiq_proto::ids::HostSaveId::generate().to_string();
                hosts.push(SavedRemoteHost {
                    id: id.clone(),
                    name,
                    address,
                    scheme,
                    trust_insecure,
                });
                id
            }
        } else if let Some(existing) = hosts.iter_mut().find(|host| host.id == save_id) {
            existing.name = name;
            existing.address = address;
            existing.scheme = scheme;
            existing.trust_insecure = trust_insecure;
            existing.id.clone()
        } else {
            hosts.push(SavedRemoteHost {
                id: save_id.clone(),
                name,
                address,
                scheme,
                trust_insecure,
            });
            save_id
        };
        self.remember_host_settings();
        cx.notify();
        id
    }

    /// Rename a saved host. The id, address, scheme and keychain token are untouched — a name is
    /// a label, and live connections keep the one they were dialled under regardless.
    pub fn rename_remote_host(&mut self, save_id: String, name: String, cx: &mut Context<Self>) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(host) = self
            .workbench
            .settings
            .host
            .remote_hosts
            .iter_mut()
            .find(|host| host.id == save_id)
        {
            host.name = name;
            self.remember_host_settings();
        }
        cx.notify();
    }

    /// Drop a saved host record, its keychain token and its reconnect loop. A live connection
    /// under that entry, if this window still has one, is untouched; forgetting is about what
    /// the *next* window sees on open, not about the one open right now.
    pub fn forget_remote_host(&mut self, save_id: String, cx: &mut Context<Self>) {
        let mut forgotten: Vec<SavedRemoteHost> = Vec::new();
        self.workbench
            .settings
            .host
            .remote_hosts
            .retain(|host| {
                let keep = host.id != save_id && !(save_id.is_empty() && host.address == save_id);
                if !keep {
                    forgotten.push(host.clone());
                }
                keep
            });
        for host in &forgotten {
            let key = host_secrets::key_for(&host.id, &host.address);
            host_secrets::delete_token(&key);
            self.workbench.settings.failed_hosts.remove(&host.address);
            self.workbench.settings.reconnects.remove(&key);
            self.workbench.remote_manager.tests.remove(&key);
        }
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
            HostEntry::Saved { id, .. } => {
                let saved = self
                    .workbench
                    .settings
                    .host
                    .remote_hosts
                    .iter()
                    .find(|host| host.id == id)
                    .cloned();
                if let Some(saved) = saved {
                    self.reconnect_saved_host(
                        saved.id,
                        saved.name,
                        saved.address,
                        saved.scheme,
                        saved.trust_insecure,
                        window,
                        cx,
                    );
                }
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
        if nav == SettingsSection::Assist {
            // Asked on arrival rather than at startup, for the reason the connections are: a
            // provider added in another window has to show up here, and the host is the only half
            // that knows which of them has a key.
            self.bus.send(Message::GetAiProviders);
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

    /// Show or hide the cached-token ring in a conversation footer.
    pub fn toggle_cache_ring(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.ui.show_cache_ring = !self.workbench.settings.ui.show_cache_ring;
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

    /// Whether Ubiq reads a conversation's opening exchange and names it. Host-owned, like the
    /// toggle above: the host is what does the reading.
    pub fn toggle_auto_name_conversations(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.host.auto_name_conversations =
            !self.workbench.settings.host.auto_name_conversations;
        self.remember_host_settings();
        cx.notify();
    }

    /// Which backend, if any, writes a suggestion. Host-owned, so this writes the Host layer
    /// like the toggle above.
    ///
    /// The answer to "can assistance run here" changes with the provider — `Off` is itself a
    /// reason it cannot — so the host is asked again rather than the interface deciding what its
    /// own write must have meant.
    pub fn set_assist_provider(&mut self, provider: AssistProvider, cx: &mut Context<Self>) {
        self.workbench.settings.host.assist = provider;
        self.remember_host_settings();
        self.bus.send(Message::GetAssist);
        cx.notify();
    }

    /// Which `$HOME` a confined agent runs with. Host-owned, like the toggle above.
    ///
    /// Picking the kept home seeds a name where there is none, so the choice can never be stored
    /// as a home with no name — the field below it then renames it in place.
    pub fn set_agent_home(&mut self, home: AgentHome, cx: &mut Context<Self>) {
        let home = match home {
            AgentHome::Named(name) if name.trim().is_empty() => {
                AgentHome::Named(DEFAULT_AGENT_HOME.to_string())
            }
            other => other,
        };
        self.workbench.settings.host.agent_home = home;
        self.remember_host_settings();
        cx.notify();
    }

    /// Rename the kept home. Ignored unless a kept home is what is selected — the field is only
    /// drawn then, and an empty name falls back to the default rather than being stored.
    pub fn set_agent_home_name(&mut self, name: String, cx: &mut Context<Self>) {
        if !matches!(self.workbench.settings.host.agent_home, AgentHome::Named(_)) {
            return;
        }
        let name = match name.trim() {
            "" => DEFAULT_AGENT_HOME.to_string(),
            trimmed => trimmed.to_string(),
        };
        self.workbench.settings.host.agent_home = AgentHome::Named(name);
        self.remember_host_settings();
        cx.notify();
    }

    /// Add the path in the grants field, read-only. Read-write is a second gesture on the chip,
    /// so the safer half of the choice is never the one made by accident.
    pub fn add_extra_grant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.grant_path_input.read(cx).value().trim().to_string();
        if path.is_empty() {
            return;
        }
        let grants = &mut self.workbench.settings.host.extra_grants;
        if !grants.iter().any(|grant| grant.path == path) {
            grants.push(Grant { path, write: false });
            self.remember_host_settings();
        }
        self.grant_path_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();
    }

    pub fn remove_extra_grant(&mut self, index: usize, cx: &mut Context<Self>) {
        let grants = &mut self.workbench.settings.host.extra_grants;
        if index >= grants.len() {
            return;
        }
        grants.remove(index);
        self.remember_host_settings();
        cx.notify();
    }

    /// Flip one grant between read-only and read-write.
    pub fn toggle_grant_write(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(grant) = self.workbench.settings.host.extra_grants.get_mut(index) else {
            return;
        };
        grant.write = !grant.write;
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

    /// The host-owned fields hold what the host has stored, on the Git fields' rule: mirrored while
    /// the dialog is up and never while the field is being typed into.
    pub(super) fn sync_settings_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workbench.settings.open {
            return;
        }
        let home = match &self.workbench.settings.host.agent_home {
            AgentHome::Named(name) => name.clone(),
            _ => String::new(),
        };
        for (field, wanted) in [
            (
                self.search_excludes_input.clone(),
                self.workbench.settings.host.search_excludes.join(", "),
            ),
            (
                self.search_fallbacks_input.clone(),
                self.workbench.settings.host.search_fallbacks.join(", "),
            ),
            (self.agent_home_input.clone(), home),
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
            command_open: false,
            command_check: None,
        });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Pick which harness the login is for. Re-picking before it starts is free.
    ///
    /// Seeds the custom-command field with whatever override this machine already holds for that
    /// harness, and shows the field unasked when there is one — an override the user cannot see
    /// is an override they will not think to blame.
    pub fn pick_login_harness(
        &mut self,
        agent_type: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved = self
            .workbench
            .settings
            .host
            .agent_commands
            .get(&agent_type)
            .cloned();
        let default = self
            .workbench
            .agent_types
            .iter()
            .find(|it| it.id == agent_type)
            .map(|it| it.command.clone())
            .unwrap_or_else(|| agent_type.clone());

        let Some(login) = &mut self.workbench.settings.login else {
            return;
        };
        let LoginStep::Choosing { agent_type: chosen } = &mut login.step else {
            return;
        };
        *chosen = Some(agent_type);
        login.command_open = saved.is_some();
        login.command_check = None;

        self.login_command_input.update(cx, |state, cx| {
            state.set_placeholder(default, window, cx);
            state.set_value(saved.unwrap_or_default(), window, cx);
        });
        cx.notify();
    }

    /// Which harness the picker is on, while it is still on the picker.
    fn login_harness(&self) -> Option<String> {
        match &self.workbench.settings.login.as_ref()?.step {
            LoginStep::Choosing { agent_type } => agent_type.clone(),
            _ => None,
        }
    }

    /// Show or hide the custom-command field. Closing is what commits it, which is also how an
    /// emptied field removes the override.
    pub fn toggle_login_command(&mut self, cx: &mut Context<Self>) {
        let Some(login) = &mut self.workbench.settings.login else {
            return;
        };
        login.command_open = !login.command_open;
        if !login.command_open {
            self.commit_login_command(cx);
        }
        cx.notify();
    }

    /// Ask the host whether what is typed actually runs. Nothing is saved by asking — the answer
    /// lands in `command_check` and is drawn under the field.
    pub fn check_login_command(&mut self, cx: &mut Context<Self>) {
        let Some(agent_type) = self.login_harness() else {
            return;
        };
        let command = self.login_command_input.read(cx).value().trim().to_string();
        if let Some(login) = &mut self.workbench.settings.login {
            login.command_check = None;
        }
        self.bus.send(Message::CheckAgentCommand {
            agent_type,
            command,
        });
        cx.notify();
    }

    /// Write the typed command into `agent_commands` for the picked harness — or drop the entry
    /// when the field is empty — and tell the host. `ListAgentTypes` goes straight back out
    /// because whether a harness is available is a fact about the command that just changed.
    fn commit_login_command(&mut self, cx: &mut Context<Self>) {
        let Some(agent_type) = self.login_harness() else {
            return;
        };
        let command = self.login_command_input.read(cx).value().trim().to_string();
        let now = (!command.is_empty()).then(|| command.clone());
        let commands = &mut self.workbench.settings.host.agent_commands;
        let before = match &now {
            Some(command) => commands.insert(agent_type, command.clone()),
            None => commands.remove(&agent_type),
        };
        if before == now {
            return;
        }
        self.remember_host_settings();
        self.bus.send(Message::ListAgentTypes);
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
        // The command field is committed here too: signing in is the other moment its content
        // stops being a draft, and the flow about to start is what will run it.
        self.commit_login_command(cx);
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

    /// Stop the running harness and wait for the outcome, keeping the modal up.
    ///
    /// The difference from [`close_harness_login`](Self::close_harness_login) is the whole point:
    /// a harness whose login is its ordinary interactive screen — grok's is — never exits once
    /// the browser flow is done, so the only thing that ends it is the user saying so. Discarding
    /// the modal at that moment threw away the `HarnessLoginCaptured` on its way back, and a
    /// sign-in that had in fact worked read as nothing happening at all. The state stays, the
    /// pane goes, and `login_ended` draws the `Done` step the host's answer describes.
    /// A probe is not one of them: it never reaches `finish_login`, so no outcome is coming and
    /// waiting for one would hang the modal. Its shell closes the flow outright.
    pub fn finish_harness_login(&mut self, cx: &mut Context<Self>) {
        let Some(login) = &self.workbench.settings.login else {
            return;
        };
        if login.probe {
            return self.close_harness_login(cx);
        }
        if let LoginStep::Running { pane } = login.step {
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
            command_open: false,
            command_check: None,
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
            command_open: false,
            command_check: None,
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
    /// It is the New agent form with [`Purpose::Profile`]: the same questions, so the same rows,
    /// and every pick below the name is answered by that form's own mutators. The two typed
    /// fields are seeded here, the way the rename dialog seeds its own.
    pub fn open_profile_form(
        &mut self,
        profile: Option<ProfileInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = match &profile {
            Some(profile) => NewAgentForm::from_profile(profile, Purpose::Profile),
            None => NewAgentForm::new(Purpose::Profile),
        };
        let id = profile.as_ref().map(|it| it.id.clone()).unwrap_or_default();
        let prompt = form.prompt.clone();
        self.profile_id_input
            .update(cx, |state, cx| state.set_value(&id, window, cx));
        self.set_new_agent_prompt(&prompt, window, cx);
        self.workbench.settings.profile_form = Some(form);
        self.workbench.settings.error = None;
        // What the harness offers is what the model and level rows are drawn from, so an edit
        // opens asking for it rather than showing an empty list until something is repicked.
        self.probe_new_agent_catalogue(cx);
        cx.notify();
    }

    pub fn close_profile_form(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.profile_form = None;
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

    // ── Assistance providers ────────────────────────────────────────
    //
    // Providers are the one part of the host record the interface does not write through
    // `SetSettings`: the host discards whatever an interface sends for `ai_providers`, because a
    // record and the key filed under its id go together and only the host can write the key. So
    // every change here is one of the four provider messages, and every list on screen is what
    // `AiProviders` last said.

    /// Raise the provider form — empty for one being added, seeded for one being edited.
    ///
    /// The key box is always left empty, because a stored key is never read back: the row says
    /// whether there is one and nothing shows it. An edit also asks for the provider's model list
    /// so the two pickers have something to offer — `refresh: false`, so a cached list costs no
    /// network and a picker is openable without one. There is usually one to serve: the host
    /// lists a provider's models the moment it is added.
    pub fn open_ai_form(
        &mut self,
        id: Option<AiProviderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let held = id.and_then(|id| self.workbench.settings.ai_provider(id).cloned());
        let kind = held
            .as_ref()
            .map_or(AiProviderKind::OpenAiCompatible, |info| info.provider.kind);
        let name = held
            .as_ref()
            .map_or(String::new(), |info| info.provider.name.clone());
        let base_url = held
            .as_ref()
            .and_then(|info| info.provider.base_url.clone())
            .unwrap_or_default();
        let fast_model = held
            .as_ref()
            .map_or(String::new(), |info| info.provider.fast_model.clone());
        let smart_model = held
            .and_then(|info| info.provider.smart_model.clone())
            .unwrap_or_default();
        self.set_ai_form_fields(&name, &base_url, &fast_model, &smart_model, window, cx);
        self.workbench.settings.ai_form = Some(AiProviderForm {
            id,
            kind,
            open: None,
        });
        self.workbench.settings.error = None;
        if let Some(provider_id) = id {
            self.bus.send(Message::ListAiModels {
                provider_id,
                refresh: false,
            });
        }
        cx.notify();
    }

    /// Pick the wire protocol a provider speaks. Nothing follows it: the endpoint is the user's,
    /// and which models exist is the provider's own answer rather than a property of the kind.
    pub fn pick_ai_kind(&mut self, kind: AiProviderKind, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.settings.ai_form {
            form.kind = kind;
        }
        cx.notify();
    }

    /// Open one model picker's list, closing whichever was down: exactly one is ever open, the
    /// rule every picker in this window follows.
    pub fn toggle_ai_model_picker(&mut self, role: ModelRole, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.settings.ai_form {
            form.open = if form.open == Some(role) {
                None
            } else {
                Some(role)
            };
        }
        cx.notify();
    }

    /// Write a picked model into the box it belongs to, and close the list it came from.
    ///
    /// The box is the store and the picker fills it, so a picked id and a typed one are the same
    /// thing by the time the form is saved. `None` is the smart picker's first row — "use the fast
    /// model" — and empties the box rather than storing a sentinel, because an empty box is
    /// already what "no smart model" means to `save_ai_form`.
    pub fn pick_ai_model(
        &mut self,
        role: ModelRole,
        model: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = match role {
            ModelRole::Fast => self.ai_fast_model_input.clone(),
            ModelRole::Smart => self.ai_smart_model_input.clone(),
        };
        let value = model.unwrap_or_default();
        input.update(cx, |state, cx| state.set_value(&value, window, cx));
        if let Some(form) = &mut self.workbench.settings.ai_form {
            form.open = None;
        }
        cx.notify();
    }

    /// Ask the provider itself for its models. The only place `refresh: true` is sent, because it
    /// is the only control a user pressed for it — every other ask takes the host's cache.
    pub fn refresh_ai_models(&mut self, cx: &mut Context<Self>) {
        let Some(provider_id) = self
            .workbench
            .settings
            .ai_form
            .as_ref()
            .and_then(|form| form.id)
        else {
            return;
        };
        self.bus.send(Message::ListAiModels {
            provider_id,
            refresh: true,
        });
        cx.notify();
    }

    /// Send the form. The host mints the id for an add, files the key under it, answers with the
    /// whole list and then lists that provider's models unasked — or refuses, which reads as the
    /// banner in this section. The modal closes optimistically, the way the rename dialog does.
    ///
    /// An add carries no model, because the form asks for none: there is nothing to pick from
    /// until the record exists. The two model boxes are empty in that case and stay that way on
    /// the wire, which the host accepts as an unfinished record rather than a bad one.
    pub fn save_ai_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.ai_name_input.read(cx).value().trim().to_string();
        let base_url = self.ai_base_url_input.read(cx).value().trim().to_string();
        // Trimmed, because a key pasted with a newline around it is not a different key.
        let key = self.ai_key_input.read(cx).value().trim().to_string();
        // Typed or picked, a model is read from its box. An empty fast box is the model-less
        // record the host allows; an empty smart box is `None`, which means the fast model
        // answers those subjects too.
        let fast_model = self.ai_fast_model_input.read(cx).value().trim().to_string();
        let smart_model = self
            .ai_smart_model_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(form) = self.workbench.settings.ai_form.take() else {
            return;
        };
        // The two refusals the Save button is dimmed for, enforced here too: a click on a dimmed
        // button is still a click. A key is required only for an add — leaving it blank on an
        // edit is how the stored one is kept. A model is required by neither: the host writes a
        // record with none, and an id that no list would have offered is typed into the box.
        if name.is_empty() || (form.id.is_none() && key.is_empty()) {
            self.workbench.settings.ai_form = Some(form);
            return;
        }
        let draft = AiProviderDraft {
            kind: form.kind,
            name,
            base_url: (!base_url.is_empty()).then_some(base_url),
            fast_model,
            smart_model: (!smart_model.is_empty()).then_some(smart_model),
        };
        let typed_a_key = !key.is_empty();
        match form.id {
            // `key: None` leaves the stored key alone. The interface is never sent a key, so it
            // cannot send one back — a blank box is the only way to say "keep it".
            Some(provider_id) => self.bus.send(Message::UpdateAiProvider {
                provider_id,
                draft,
                key: typed_a_key.then(|| Secret::new(key)),
            }),
            None => self.bus.send(Message::AddAiProvider {
                draft,
                key: Secret::new(key),
            }),
        }
        self.set_ai_form_fields("", "", "", "", window, cx);
        cx.notify();
    }

    pub fn close_ai_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.settings.ai_form = None;
        self.set_ai_form_fields("", "", "", "", window, cx);
        cx.notify();
    }

    /// Seed the form's typed fields. The key box and the two picker filters are always emptied: a
    /// key is never read back, and a filter left over from the last form would hide the list.
    fn set_ai_form_fields(
        &mut self,
        name: &str,
        base_url: &str,
        fast_model: &str,
        smart_model: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (input, value) in [
            (&self.ai_name_input, name),
            (&self.ai_base_url_input, base_url),
            (&self.ai_key_input, ""),
            (&self.ai_fast_model_input, fast_model),
            (&self.ai_smart_model_input, smart_model),
            (&self.ai_fast_search, ""),
            (&self.ai_smart_search, ""),
        ] {
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }
    }

    /// Raise the removal question. It is a danger confirm because the key in the OS keychain goes
    /// with the record — there is nothing left behind to reattach a re-added provider to.
    pub fn open_remove_ai_provider(&mut self, provider_id: AiProviderId, cx: &mut Context<Self>) {
        self.workbench.settings.ai_remove = Some(provider_id);
        self.workbench.settings.error = None;
        cx.notify();
    }

    pub fn confirm_remove_ai_provider(&mut self, cx: &mut Context<Self>) {
        let Some(provider_id) = self.workbench.settings.ai_remove.take() else {
            return;
        };
        self.bus.send(Message::ForgetAiProvider { provider_id });
        cx.notify();
    }

    pub fn close_remove_ai_provider(&mut self, cx: &mut Context<Self>) {
        self.workbench.settings.ai_remove = None;
        cx.notify();
    }

    /// Raise the test modal over one provider's row. Nothing is sent yet — the run is a button
    /// inside it, because a modal that calls a model as it opens calls one nobody asked for.
    pub fn open_ai_test(&mut self, provider_id: AiProviderId, cx: &mut Context<Self>) {
        let Some((name, has_smart)) =
            self.workbench
                .settings
                .ai_provider(provider_id)
                .map(|info| {
                    (
                        info.provider.name.clone(),
                        info.provider.smart_model.is_some(),
                    )
                })
        else {
            return;
        };
        self.workbench.settings.ai_test = Some(AiTest {
            provider_id,
            name,
            has_smart,
            role: ModelRole::Fast,
            suggest_id: None,
            answer: String::new(),
            done: false,
            error: None,
        });
        self.workbench.settings.error = None;
        cx.notify();
    }

    /// Which of the provider's two models the test exercises. Smart is refused where none is
    /// configured, so the dead pill cannot be talked into naming a model that is not there.
    pub fn pick_ai_test_role(&mut self, role: ModelRole, cx: &mut Context<Self>) {
        if let Some(test) = &mut self.workbench.settings.ai_test
            && (role == ModelRole::Fast || test.has_smart)
        {
            test.role = role;
        }
        cx.notify();
    }

    /// Mint an id, keep it, and ask.
    ///
    /// The subject carries the provider and the role and no prompt at all: the prompt is the
    /// host's for this subject as for every other, which is what keeps one off the bus tape. The
    /// answer streams in as `SuggestChunk`, and `receive_assist` puts it here.
    pub fn run_ai_test(&mut self, cx: &mut Context<Self>) {
        let Some((provider_id, role, in_flight)) = self
            .workbench
            .settings
            .ai_test
            .as_ref()
            .map(|test| (test.provider_id, test.role, test.suggest_id))
        else {
            return;
        };
        // A rerun gives up on the run before it. Best effort, like every cancel: an answer to the
        // abandoned id may still arrive, and is discarded by the id it names.
        if let Some(suggest_id) = in_flight {
            self.bus.send(Message::CancelSuggest { suggest_id });
        }
        let suggest_id = SuggestId::generate();
        self.suggest = Some(suggest_id);
        if let Some(test) = &mut self.workbench.settings.ai_test {
            test.suggest_id = Some(suggest_id);
            test.answer.clear();
            test.done = false;
            test.error = None;
        }
        self.bus.send(Message::Suggest {
            suggest_id,
            subject: SuggestSubject::ProviderCheck { provider_id, role },
        });
        cx.notify();
    }

    /// Leave the test, giving up on a run still in flight.
    pub fn close_ai_test(&mut self, cx: &mut Context<Self>) {
        let Some(test) = self.workbench.settings.ai_test.take() else {
            return;
        };
        if let Some(suggest_id) = test.suggest_id {
            self.bus.send(Message::CancelSuggest { suggest_id });
            self.suggest = None;
        }
        cx.notify();
    }

    // ── Explorer ────────────────────────────────────────────────────
}
