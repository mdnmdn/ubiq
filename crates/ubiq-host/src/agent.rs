//! Agent types: which harnesses can run here, and what one run is composed of.
//!
//! This module is the whole of Ubiq's knowledge about agents, and it is
//! deliberately thin: the list comes from [`agent_manager::harness::all`], the
//! launch comes from the library's provisioner, and the policy that confines it
//! comes from the library's isolate stage. What is Ubiq's own is everything the
//! library has no opinion about — which pane owns a run, where that run's
//! throwaway configuration lives under Ubiq's config root, and when it is
//! deleted.
//!
//! Nothing here names a harness binary, a configuration path or a launch flag.
//! A path literal in this file is the clearest possible sign the boundary in
//! `_docs/tech/agent-manager.md` has been crossed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agent_manager::Validity;
use agent_manager::account::{AccountStore, FsAccountStore, login_validity};
use agent_manager::config::{McpServer, McpTransport};
use agent_manager::harness::{self, Launch, ModelInfo};
use agent_manager::io::IoBridge;
use agent_manager::isolate::{self, Confined, IsolateOptions};
use agent_manager::profile::{FsProfileStore, Profile, ProfileDefaults, ProfileStore};
use agent_manager::provision;
use agent_manager::registry::FsRegistry;
use agent_manager::resolve;
use agent_manager::session;
use agent_manager::settings::Settings;
use agent_manager::source::Source;
use agent_manager::spec::{ConfigStrategy, IoModes, Isolation, McpRef, Policy};
use anyhow::{Context, Result, anyhow, bail};
use ubiq_proto::conversation::ConfigChoice;
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::{AccountInfo, AgentTypeInfo, LoginStatus, ProfileInfo};
use ubiq_proto::settings::{AgentHome, Grant};
use ubiq_proto::work::AgentId;

use crate::environment::Environment;

/// The agent types this machine can run, and the composer behind them.
///
/// Held by the coordinator, one per process, because everything it owns —
/// where run directories go, whether an agent is confined — is a property of
/// the host rather than of a window.
pub struct Agents {
    /// Ubiq's own config root. Run directories and isolation state hang off it,
    /// so a relocated root relocates an agent's state with it.
    root: PathBuf,
    /// Whether a run is confined unless something says otherwise.
    isolate: bool,
    /// Which `$HOME` a confined run gets.
    home: AgentHome,
    /// Directories a confined run may reach beyond what its policy grants.
    extra_grants: Vec<Grant>,
    /// Where this machine keeps its toolchains, from `environment.toml`. Separate from the
    /// settings above because it describes the machine rather than a preference — see
    /// [`crate::environment`].
    environment: Environment,
    /// Custom command line overrides, harness id → what to run instead of the library's own
    /// bare program, as the user set them in Settings.
    commands: BTreeMap<String, String>,
    /// Where the host's MCP listener is bound — `http://127.0.0.1:<port>` — or `None` when the
    /// port would not bind. A run composed without it simply gets no injected servers: an agent
    /// that cannot ask Ubiq what project it is in is a smaller failure than one that will not
    /// start.
    mcp_base: Option<String>,
    /// Who that listener will answer for. Held here rather than only by the coordinator because
    /// **retiring a run is what forgets an agent**, and every path that ends a run — a launch that
    /// would not compose, a window closing, `EndConversation`, the startup sweep — already goes
    /// through [`retire`](Self::retire), [`retire_agent`](Self::retire_agent) or
    /// [`park_agent`](Self::park_agent). Hooking the coordinator's arms one at a time would mean
    /// finding them all again the next time one is added.
    mcp_agents: crate::mcp::Registry,
}

/// A run composed and ready to spawn: what to exec, where its configuration
/// was written, and the policy it runs under.
///
/// The launch and the policy are private on purpose. A caller asks
/// [`exec`](Self::exec) what to start and gets the harness under its policy
/// whenever there is one, so a confined run cannot be spawned unconfined by
/// reaching for the wrong field.
pub struct Composed {
    /// The program, arguments and environment the harness wants.
    launch: Launch,
    /// The policy confining the run, when it is confined.
    confined: Option<Confined>,
    /// The configuration directory this run was provisioned into, which the
    /// pane owns and [`Agents::retire`] removes.
    pub dir: PathBuf,
    /// What the library provisioned, kept because a structured bridge is
    /// built from it rather than from the launch alone.
    provisioned: provision::Provisioned,
    /// The id of the account this run resolved to, when a profile named one.
    spec_account: Option<String>,
}

/// Everything a caller chooses about one run, over and above the harness, the folder and the
/// face it wears.
///
/// One struct rather than seven parameters because [`Agents::converse`] and
/// [`Agents::compose`] hand the identical set through to the same place, and a pane's answer to
/// all of it is `Default::default()` — the library resolves what nothing named.
///
/// Every field is a *pick*, and a pick outranks the profile inside the library's `resolve`. `None`
/// everywhere is the zero-config start: whatever the harness, its profile and its own defaults
/// say.
#[derive(Default)]
pub struct ConverseOptions {
    /// The identity this run answers as, when one was named.
    pub account: Option<String>,
    /// The harness-native model id, never interpreted here.
    pub model: Option<String>,
    /// The harness-native reasoning-effort level.
    pub thinking: Option<String>,
    /// One of the harness's own `modes()` ids.
    pub mode: Option<String>,
    /// The saved setup the picks above sit on top of.
    pub profile: Option<String>,
    /// The first turn's text, for a **one-shot** harness whose prompt is argv rather than a frame
    /// on a pipe. Left `None` for a multi-turn harness, which is prompted over its bridge after
    /// it is running — putting a prompt here for one of those would run its first turn twice.
    pub prompt: Option<String>,
    /// The harness's own session id, to continue a conversation whose last process has exited.
    /// Which flag that becomes is the library's answer, never Ubiq's.
    pub resume: Option<String>,
    /// The MCP servers Ubiq should inject into this run, by their slug in
    /// [`ubiq_proto::mcp::McpInfo::name`]. Each becomes an inline http server pointed at this
    /// host's own listener; a name this build does not offer is dropped with a warning rather
    /// than failing the run, the same rule a stale reference lives by everywhere else.
    ///
    /// The picks only. What the profile saved is read from the profile inside `compose_run`,
    /// because that is where the profile is known — see the note there.
    pub mcps: Vec<String>,
}

/// A login that has been prepared and not yet finished: what to run, and what finishing it
/// would mean.
///
/// Held by the coordinator against the pane it opened, because the answer to "did this log
/// anyone in" is only available once that pane's process has exited.
pub struct PendingLogin {
    /// The account this login is for.
    pub account: String,
    /// The harness being logged in, for the message that reports the outcome.
    pub agent_type: String,
    /// The account's capture home: the login's `$HOME`, and where its credential lands.
    home: PathBuf,
    /// The credential files the harness said it would write, relative to `home`. The first
    /// is required; the rest are metadata.
    files: Vec<PathBuf>,
    /// When the required credential was last written before the login ran, so a harness
    /// that exits without refreshing it cannot pass for a success.
    captured_before: Option<std::time::SystemTime>,
    /// The confined launch to spawn under a pseudo-terminal.
    launch: Launch,
    /// A plain shell running under this login's policy, standing in for the harness — asked
    /// for empirically inspecting the sandbox rather than signing anyone in. A probe pane's
    /// exit must never be read as a login outcome; that is `coordinator::Coordinator::
    /// login_gone`'s branch on this flag.
    pub probe: bool,
}

impl PendingLogin {
    /// What to spawn. A login is an ordinary process to Ubiq — the policy that makes it
    /// capturable is already rendered into this launch.
    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// Where the login runs, which is also the only directory it may write.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// A fixture `PendingLogin`, for `coordinator`'s own tests of `login_gone` — which needs one
    /// parked in `Coordinator::logins` without going through `begin_login`'s real isolation
    /// stack (that needs a harness binary this crate's tests must not depend on having
    /// installed).
    #[cfg(test)]
    pub(crate) fn for_test(
        account: impl Into<String>,
        agent_type: impl Into<String>,
        home: PathBuf,
        files: Vec<PathBuf>,
        captured_before: Option<std::time::SystemTime>,
        probe: bool,
    ) -> Self {
        Self {
            account: account.into(),
            agent_type: agent_type.into(),
            home,
            files,
            captured_before,
            launch: Launch::default(),
            probe,
        }
    }
}

impl Composed {
    /// What to actually start: the harness under its policy when the run is
    /// confined, the harness itself when it is not.
    ///
    /// Resolving the policy is what materializes the run's home, so this is
    /// called once, at the spawn, rather than when the run was composed.
    pub fn exec(&self) -> Result<Launch> {
        match &self.confined {
            Some(confined) => isolate::confined_launch(confined)
                .context("preparing the policy this agent runs under"),
            None => Ok(self.launch.clone()),
        }
    }

    /// Whether this run is confined, for the log line that says so.
    pub fn is_confined(&self) -> bool {
        self.confined.is_some()
    }

    /// The account this run resolved to, when a profile named one. An id, never a
    /// credential — which is the whole of what Ubiq is allowed to know about it.
    pub fn account(&self) -> Option<&str> {
        self.spec_account.as_deref()
    }
}

impl Agents {
    /// The registry over Ubiq's config `root`, confining runs when `isolate`.
    ///
    /// Whether this process can confine anything at all is a property of the
    /// process, so it is reported here, once, rather than as an error on every
    /// pane the user tries to open.
    pub fn new(root: impl Into<PathBuf>, isolate: bool) -> Self {
        if isolate && let Err(error) = isolate::ensure_can_confine() {
            tracing::warn!(
                "agents cannot be confined in this process: {error}. \
                 Turn isolation off in settings to start them unconfined."
            );
        }

        Self {
            root: root.into(),
            isolate,
            home: AgentHome::default(),
            extra_grants: Vec::new(),
            environment: Environment::default(),
            commands: BTreeMap::new(),
            mcp_base: None,
            mcp_agents: crate::mcp::Registry::new(),
        }
    }

    /// Point runs at the host's MCP listener: the base URL it is bound on, and the table it
    /// resolves an agent from.
    ///
    /// Set once at startup, before the first run is composed, for the same reason the listener is
    /// bound before this is built — a URL handed to a harness is written into its configuration
    /// and never revisited, so there is no second chance to tell a run where the port is.
    pub fn set_mcp(&mut self, base_url: String, agents: crate::mcp::Registry) {
        self.mcp_base = Some(base_url);
        self.mcp_agents = agents;
    }

    /// The table the listener answers from, for the coordinator to write an agent's facts into
    /// when it launches one.
    pub fn mcp_agents(&self) -> crate::mcp::Registry {
        self.mcp_agents.clone()
    }

    /// Whether a run is confined unless something says otherwise.
    pub fn isolate(&self) -> bool {
        self.isolate
    }

    /// Take the home and the extra grants from the host settings.
    ///
    /// Read at the next spawn rather than kept by a live run: a policy is rendered once, and a
    /// setting changed mid-run cannot reach the process it already confined.
    pub fn set_policy(&mut self, home: AgentHome, extra_grants: Vec<Grant>) {
        self.home = home;
        self.extra_grants = extra_grants;
    }

    /// Take this machine's own environment, read from `environment.toml` at startup.
    ///
    /// Read at the next spawn, like the settings above: a policy is rendered once.
    pub fn set_environment(&mut self, environment: Environment) {
        self.environment = environment;
    }

    /// Follow the host settings, which are what the user last chose.
    pub fn set_isolate(&mut self, isolate: bool) {
        self.isolate = isolate;
    }

    /// Take the per-harness command overrides from the host settings, mirroring
    /// [`set_policy`](Self::set_policy): read at the next spawn, never applied to a run already
    /// under way.
    pub fn set_commands(&mut self, commands: BTreeMap<String, String>) {
        self.commands = commands;
    }

    /// Every agent type the library knows, in the order a menu offers them,
    /// each marked with whether its binary is actually on this machine.
    ///
    /// The probe is done per request rather than cached, for the same reason
    /// the shell list is: a harness installed after the window was opened
    /// should be offered without a restart.
    pub fn types(&self) -> Vec<AgentTypeInfo> {
        harness::all()
            .into_iter()
            .map(|harness| {
                let id = harness.id();
                // An override makes a harness available even when its own binary is not on
                // this machine's `PATH` — that is the whole point of setting one.
                let available = self.commands.contains_key(&id)
                    || crate::shells::locate(harness.command()).is_some();
                AgentTypeInfo {
                    id,
                    label: harness.display_name().to_string(),
                    command: harness.command().to_string(),
                    available,
                    chat: harness.io_support().structured,
                    modes: harness
                        .modes()
                        .into_iter()
                        .map(|mode| ConfigChoice {
                            value: mode.id,
                            name: mode.label,
                            description: mode.description,
                            group: None,
                        })
                        .collect(),
                    unattended_mode: harness.unattended_mode().map(str::to_string),
                    // The same question `Self::forkable` answers, asked of the harness rather
                    // than of a running agent: does its own session store land in the run
                    // directory Ubiq owns, so keeping or copying that directory means anything.
                    keeps_sessions: !harness.config_anchor().levers.is_empty(),
                }
            })
            .collect()
    }

    /// Whether `id` names an agent type, so a spawn refuses a name the user can
    /// see rather than failing as a process that would not start.
    pub fn is_agent_type(&self, id: &str) -> bool {
        harness::resolve(id).is_some()
    }

    /// Whether `agent_type` can be *conversed* with at all — the library has a structured bridge
    /// for it. False for a harness that only knows how to draw a screen, which a pane can still
    /// do; there is simply nothing on the other end of a pipe to map onto `ConvUpdate`s.
    ///
    /// Asked before anything is spawned, because it is a fact of the harness rather than of a
    /// process. An unknown id is not conversable either — refusing is the honest answer, and
    /// [`Self::is_agent_type`] is what tells the two refusals apart.
    pub fn converses(&self, agent_type: &str) -> bool {
        harness::resolve(agent_type).is_some_and(|harness| harness.io_support().structured)
    }

    /// Whether one process of `agent_type` survives its own first answer.
    ///
    /// False for a **one-shot** harness — Copilot, opencode — which takes its prompt in argv,
    /// answers once and exits. That is not a broken harness: it converses one turn per process,
    /// and the caller continues it by launching again with [`ConverseOptions::resume`] set to the
    /// id its last run reported. `false` for anything that does not converse at all, where the
    /// question does not arise.
    pub fn multi_turn(&self, agent_type: &str) -> bool {
        harness::resolve(agent_type).is_some_and(|harness| harness.io_support().multi_turn)
    }

    /// The binary `agent_type` launches as — `claude`, `codex` — which is what a conversation is
    /// named after. Ubiq never spells this out; the library answers it.
    pub fn command_of(&self, agent_type: &str) -> Option<String> {
        harness::resolve(agent_type).map(|h| h.command().to_string())
    }

    /// The models `agent_type` will answer for, before anything is spawned.
    ///
    /// `discover_models` takes no account and no directory: it spawns its own probe rather than
    /// asking a running harness, which is what makes P3's ordering possible — the picker is
    /// filled *instead of* starting the agent, not after. What the probe returns is per harness
    /// rather than per identity for the same reason (see `_docs/wip/agent-setup.md`, open
    /// question 6).
    pub fn discover_models(&self, agent_type: &str) -> Result<Vec<ModelInfo>> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        harness
            .discover_models()
            .with_context(|| format!("discovering {agent_type}'s models"))
    }

    /// The account store, over Ubiq's own root.
    ///
    /// Built per call rather than held, because it is a path wrapper and holding it would
    /// mean a login captured by another process stayed invisible until a restart.
    fn account_store(&self) -> FsAccountStore {
        FsAccountStore::new(self.root.join("accounts"))
    }

    /// Every account Ubiq knows, each with the harnesses it can actually log in.
    ///
    /// Which harnesses an account serves is *derived*, not recorded: an account is a home,
    /// and a harness is logged in there when the files its own `login_seed` names are
    /// present. So one account can serve several harnesses without saying so anywhere, and
    /// a capture that half-failed reports the harness it did not cover.
    pub fn accounts(&self) -> Result<Vec<AccountInfo>> {
        let store = self.account_store();
        let harnesses = harness::all();

        store
            .accounts()
            .context("reading the accounts Ubiq knows")?
            .into_iter()
            .map(|account| {
                let logged_in = match &account.home {
                    Some(home) => harnesses
                        .iter()
                        .filter(|harness| Self::has_capture(harness.as_ref(), home))
                        .map(|harness| harness.id())
                        .collect(),
                    // An account that references an environment variable or a helper
                    // instead of a captured home has no files to look for.
                    None => Vec::new(),
                };
                Ok(AccountInfo {
                    id: account.id,
                    logged_in,
                })
            })
            .collect()
    }

    /// The profile store, over Ubiq's own root. Built per call, for the same reason
    /// [`account_store`](Self::account_store) is.
    fn profile_store(&self) -> FsProfileStore {
        FsProfileStore::new(self.root.join("profiles"))
    }

    /// Every profile Ubiq knows: a saved setup, flattened to the four references the
    /// interface shows. A profile with no harness pin is skipped — the interface offers
    /// profiles per harness row, and one that names none belongs to no row.
    pub fn profiles(&self) -> Result<Vec<ProfileInfo>> {
        Ok(self
            .profile_store()
            .profiles()
            .context("reading the profiles Ubiq knows")?
            .into_iter()
            .filter_map(|profile| {
                Some(ProfileInfo {
                    id: profile.id,
                    agent_type: profile.harness?,
                    account: profile.account,
                    model: profile.defaults.model,
                    mode: profile.mode,
                    thinking: profile.defaults.thinking,
                    max_subagents: profile.max_subagents,
                    prompt: profile.defaults.prompt,
                    // "This profile mentioned nothing" and "this profile picked none" are the
                    // same row in a checklist, so the interface is told the empty list for both.
                    // The distinction still matters on disk — see [`save_profile`](Self::save_profile).
                    mcps: profile.defaults.mcps.unwrap_or_default(),
                })
            })
            .collect())
    }

    /// What a profile saved under `defaults.mcps`, or `None` when it mentioned none — the
    /// distinction the library's own merge turns on, kept rather than flattened to a list.
    ///
    /// The named profile's own row, not its inheritance chain: Ubiq writes no parent, and
    /// flattening one here would be this module holding a second answer to a question
    /// `resolve` already answers.
    fn profile_mcps(&self, id: &str) -> Option<Vec<String>> {
        self.profile_store()
            .profile(id)
            .ok()
            .flatten()
            .and_then(|profile| profile.defaults.mcps)
    }

    /// Write a profile, creating it when its id names none. Overwrites in place: a saved
    /// setup is edited, not versioned.
    ///
    /// An empty pick is written as `None` rather than as an empty list, because
    /// [`ProfileDefaults`] draws a real distinction between them: `None` is "this profile did not
    /// mention MCP servers", which lets a parent profile's — or the library's own — answer stand,
    /// and `Some([])` is "this profile says none", which overrides one. A checklist with nothing
    /// ticked is the first of those. Nothing in Ubiq's own interface can express the second, and
    /// inventing it here would mean every profile saved through this window silently overriding a
    /// default it was never shown.
    pub fn save_profile(&self, profile: ProfileInfo) -> Result<()> {
        let record = Profile {
            id: profile.id,
            harness: Some(profile.agent_type),
            account: profile.account,
            defaults: ProfileDefaults {
                model: profile.model,
                thinking: profile.thinking,
                prompt: profile.prompt,
                mcps: (!profile.mcps.is_empty()).then_some(profile.mcps),
                ..Default::default()
            },
            mode: profile.mode,
            max_subagents: profile.max_subagents,
            ..Default::default()
        };
        self.profile_store()
            .save(&record)
            .with_context(|| format!("saving profile '{}'", record.id))?;
        Ok(())
    }

    /// Whether `account` has a usable, current credential for `agent_type`, as the credential
    /// itself claims. An unknown harness or an account with no home for it is
    /// [`LoginStatus::Missing`], not an error — a check that finds nothing is an answer.
    pub fn check_login(&self, agent_type: &str, account: &str, now_ms: i64) -> LoginStatus {
        let Some(harness) = harness::resolve(agent_type) else {
            return LoginStatus::Missing;
        };
        let home = self
            .account_store()
            .account(account)
            .ok()
            .flatten()
            .and_then(|account| account.home);
        let Some(home) = home else {
            return LoginStatus::Missing;
        };
        match login_validity(harness.as_ref(), &home, now_ms) {
            Validity::Empty => LoginStatus::Missing,
            Validity::Valid {
                expires_at_ms: Some(expires_at_ms),
            } => LoginStatus::Valid { expires_at_ms },
            Validity::Valid {
                expires_at_ms: None,
            }
            | Validity::Unknown => LoginStatus::Unknown,
            Validity::Expired { expires_at_ms } => LoginStatus::Expired { expires_at_ms },
        }
    }

    /// Rename an account, and every harness's login inside it. The store validates the new
    /// name; this only forwards what it decides.
    pub fn rename_account(&self, from: &str, to: &str) -> Result<()> {
        self.account_store().rename_account(from, to)
    }

    /// Delete an account and every harness login inside it.
    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.account_store().delete_account(id)
    }

    /// Sign one harness out of `account`, leaving the account and its other harnesses'
    /// logins alone. The harness names its own credential files — the store never learns a
    /// harness's layout, and Ubiq never hard-codes one.
    pub fn delete_harness_login(&self, agent_type: &str, account: &str) -> Result<()> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        let files: Vec<PathBuf> = harness
            .config_anchor()
            .login_seed
            .iter()
            .map(|seed| seed.src.clone())
            .collect();
        self.account_store().sign_out(account, &files)
    }

    /// Rewrite `launch`'s program to what `agent_type` should actually run.
    ///
    /// A harness's own `Launch` names its program bare (`"claude"`), because
    /// `crates/agent-manager` reads no process environment — `isolate.rs` says so of itself.
    /// Ubiq is the embedder that may, so it is done here, once, before a bare name reaches
    /// either a pty spawn or isol8's `confine_executable`: both resolve a bare name against
    /// only the thin `PATH` a desktop launch inherits, which is exactly the gap `locate`
    /// closes by also asking the login shell. Left bare when `locate` finds nothing, so a
    /// genuinely missing binary still fails with the library's own "not found" error rather
    /// than a swallowed one here.
    ///
    /// A configured override replaces the program outright: `agent_type`'s command line is
    /// split into words, the first resolved exactly as a bare harness name is, the rest
    /// prepended to `launch.args` — so `mise exec -- opencode` runs the harness's own
    /// arguments through `mise` rather than losing them.
    fn resolve_program(&self, agent_type: &str, launch: &mut Launch) {
        if let Some(command) = self.commands.get(agent_type) {
            let mut words = split_command(command);
            if !words.is_empty() {
                launch.program = resolve_bare(&words.remove(0));
                words.append(&mut launch.args);
                launch.args = words;
                return;
            }
        }
        if let Some(path) = crate::shells::locate(&launch.program) {
            launch.program = path.to_string_lossy().into_owned();
        }
    }

    /// Whether `home` holds what makes `harness` logged in. A harness that names no login
    /// files cannot be answered this way, so it does not count as captured.
    fn has_capture(harness: &dyn harness::Harness, home: &Path) -> bool {
        let seed = harness.config_anchor().login_seed;
        !seed.is_empty() && seed.iter().any(|file| home.join(&file.src).exists())
    }

    /// What an interactive login for `account` into `agent_type` has to run, and what
    /// finishing it means.
    ///
    /// The launch is confined, and that is not the usual reason. A harness asked to log in
    /// with a merely *unreachable* keychain reports an error rather than writing the
    /// plaintext credential a capture needs, so the policy denies the keychain instead —
    /// see [`agent_manager::isolate::login_confined`]. Ubiq names none of that: it asks the
    /// library for the policy and spawns what comes back, exactly as it does for a pane.
    /// `probe` runs a plain shell under the login's policy instead of the harness — see
    /// [`Self::shell_probe_launch`] for why the policy is unaffected by the swap.
    /// This machine's own variables, added to a launch the harness has already described.
    /// Never over a name the harness itself set: a confined launch's `env` is the whole
    /// environment, and `CLAUDE_CONFIG_DIR` and its siblings are what pin a run — or a
    /// login's capture home — to the directory it was composed for.
    fn add_machine_env(&self, launch: &mut Launch) {
        for (key, value) in &self.environment.env {
            if !launch.env.iter().any(|(name, _)| name == key) {
                launch.env.push((key.clone(), value.clone()));
            }
        }
    }

    /// The grants this machine adds to any policy, a run's and a login's alike.
    ///
    /// A login used to get none of these, and on a machine whose node, python or homebrew
    /// tree sits where no shipped layer expects it that is a harness which cannot start: the
    /// pane opened, printed nothing, and the sign-in never began.
    ///
    /// Toolchain roots come before the user's own grants, so an explicit one is the last word
    /// on a path the environment also named. The lookup is this machine's file first and the
    /// process second: Ubiq started from the Finder has none of these set, and a toolchain
    /// root nobody named is a root nothing grants.
    ///
    /// Every directory on `PATH` is granted read-only. A tool installed where no layer expects
    /// it is otherwise unreachable — `operation not permitted: cargo`, with the binary sitting
    /// in the run's own `PATH`. Reading a tool is not writing its cache: what a build writes to
    /// is granted by the toolchain roots, not by this.
    fn isolate_options(&self) -> IsolateOptions {
        let mut options = IsolateOptions::new(self.root.join("isol8"));
        options.grant_toolchains(|name| self.environment.lookup(name));
        options.extra_ro.extend(self.environment.path_dirs());
        for grant in self.environment.grants.iter().chain(&self.extra_grants) {
            let path = expand_home(&grant.path);
            if grant.write {
                options.extra_rw.push(path);
            } else {
                options.extra_ro.push(path);
            }
        }
        options
    }

    pub fn begin_login(
        &self,
        agent_type: &str,
        account: &str,
        probe: bool,
    ) -> Result<PendingLogin> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        let home = self
            .account_store()
            .login_home(account)
            .with_context(|| format!("preparing a home for account '{account}'"))?;
        let mut plan = harness
            .login(&home)
            .with_context(|| format!("asking {agent_type} how it logs in"))?;
        self.resolve_program(agent_type, &mut plan.launch);
        // Before the policy is planned, so a grant can be read off these — the same order
        // `compose` keeps.
        self.add_machine_env(&mut plan.launch);

        // The credential's timestamp before the login runs. A harness that exits cleanly
        // without refreshing its credential has not logged anyone in, and this is the only
        // thing that tells the two apart. A probe never reaches `finish_login`, but it costs
        // nothing to compute unconditionally rather than branch this too.
        let captured_before = Self::credential_mtime(&home, &plan.credential_files);

        // The policy is rendered from `plan` — the harness's own program — before anything
        // about `probe` is looked at, so a probe inspects exactly the sandbox a real login
        // would run under, not a policy computed for a shell.
        let confined = isolate::login_confined(&home, &plan, None, &self.isolate_options())
            .with_context(|| format!("resolving the policy a {agent_type} login runs under"))?;
        let mut launch = isolate::confined_launch(&confined)
            .with_context(|| format!("preparing a confined {agent_type} login"))?;
        if probe {
            launch = Self::shell_probe_launch(launch);
        }

        Ok(PendingLogin {
            account: account.to_string(),
            agent_type: agent_type.to_string(),
            home,
            files: plan.credential_files,
            captured_before,
            launch,
            probe,
        })
    }

    /// Replace a confined login's argv with an interactive shell, keeping everything about the
    /// policy it runs under untouched.
    ///
    /// `isolate::confined_launch` renders macOS's `Launch` as `sandbox-exec -p <policy>
    /// <harness argv...>` — the policy text is the second argument, after the `-p` flag — so
    /// only the harness argv past that pair is replaced; `program`, `-p`, the policy string,
    /// `env` and `env_clear` all stay exactly as `confined_launch` produced them. The shell
    /// runs `-i` so the user gets a prompt rather than a one-shot command.
    fn shell_probe_launch(mut launch: Launch) -> Launch {
        debug_assert_eq!(
            launch.args.first().map(String::as_str),
            Some("-p"),
            "confined_launch's argv shape changed out from under the probe swap"
        );
        launch.args.truncate(2);
        launch.args.push(crate::shells::default_program());
        launch.args.push("-i".to_string());
        launch
    }

    /// Record a finished login, or say why it captured nothing.
    ///
    /// Three outcomes, and only the first is a login: the required credential appeared and
    /// is newer than it was; it is there but untouched, so the harness exited without
    /// logging anyone in; or it is absent, so the flow was abandoned. The middle case is
    /// why the timestamp is taken before the launch — without it, a stale credential left
    /// by an earlier attempt would read as a fresh success.
    pub fn finish_login(&self, pending: &PendingLogin) -> Result<()> {
        let Some(required) = pending.files.first() else {
            bail!(
                "{} names no credential file, so a login cannot be captured",
                pending.agent_type
            );
        };
        let path = pending.home.join(required);
        if !path.exists() {
            bail!("the login wrote no credential, so nothing was captured");
        }
        if Self::credential_mtime(&pending.home, &pending.files) <= pending.captured_before {
            bail!("the login left its credential untouched, so nobody was logged in");
        }

        self.account_store()
            .capture_login(&pending.account, &pending.home, &pending.files)
            .with_context(|| format!("recording account '{}'", pending.account))
    }

    /// When the credential a login is meant to write was last written, or `None` when it is
    /// not there at all — which is what an account being logged in for the first time looks
    /// like.
    fn credential_mtime(home: &Path, files: &[PathBuf]) -> Option<std::time::SystemTime> {
        let required = files.first()?;
        std::fs::metadata(home.join(required))
            .and_then(|meta| meta.modified())
            .ok()
    }

    /// Compose the run for `pane`: provision the harness's configuration into a
    /// directory named by that pane, and resolve the policy it runs under.
    ///
    /// The directory is `Fixed` rather than the library's own ephemeral choice,
    /// because a pane's run belongs to the pane: it is named by it, it is found
    /// again after a crash, and [`retire`](Self::retire) deletes it when the
    /// pane closes.
    pub fn compose(
        &self,
        pane: PaneId,
        agent_type: &str,
        cwd: &Path,
        args: Vec<String>,
    ) -> Result<Composed> {
        // A pane names no identity yet: the picker that offers one is the conversation's, so a
        // terminal harness resolves whatever the library does.
        self.compose_run(
            &pane.to_string(),
            agent_type,
            cwd,
            args,
            IoModes::Passthrough,
            ConverseOptions::default(),
        )
    }

    /// Compose a run for `agent` and drive it over structured I/O, answering
    /// the bridge its events come out of.
    ///
    /// The conversation face of the same thing [`compose`](Self::compose)
    /// builds. One difference, forced rather than chosen: the id is the
    /// agent's rather than a pane's, because a conversation has no pane. It is
    /// the day `WorkspaceId` and `PaneId` come apart.
    ///
    /// Confinement applies here exactly as it does to a pane. The policy is
    /// rendered into an argv (`sandbox-exec -p <policy> -- <harness>`) which
    /// the bridge spawns with pipes of its own, so nothing about the sandbox
    /// needs to own the descriptors.
    pub fn converse(
        &self,
        agent: AgentId,
        agent_type: &str,
        cwd: &Path,
        options: ConverseOptions,
    ) -> Result<(Composed, Box<dyn IoBridge>)> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        if !harness.io_support().structured {
            bail!(
                "{} has no structured bridge, so it cannot hold a conversation — run it in a \
                 terminal pane instead",
                harness.display_name()
            );
        }
        let mut composed = self.compose_run(
            &agent.to_string(),
            agent_type,
            cwd,
            Vec::new(),
            IoModes::Structured,
            options,
        )?;
        // A harness with no credential in its run directory reports itself logged out, from
        // inside the transcript, where it reads as the agent talking rather than as a setup
        // problem. Saying it here is what makes that actionable.
        if !Self::has_login(harness.as_ref(), &composed.dir) {
            match composed.account() {
                // A profile named an account and its login still did not land, so the
                // account itself is the thing that is not logged in.
                Some(account) => tracing::warn!(
                    harness = %agent_type,
                    account = %account,
                    "no credential reached this run: the account named for this harness has no \
                     captured login to seed from. Log it in to write one."
                ),
                // Nothing named an account, so the run fell back to the user's own home and
                // found nothing there either.
                None => tracing::warn!(
                    harness = %agent_type,
                    "no credential reached this run: no account was named, and the harness \
                     found nothing to seed from in the user's own home. A login kept in the \
                     operating system's keychain is not a file, so there is nothing to copy."
                ),
            }
        }

        // What the bridge spawns is what a pane would spawn: the harness under its policy when
        // the run is confined, the harness itself when it is not. Resolving it here rather than
        // at composition is what materializes the run's home.
        composed.provisioned.launch = composed.exec()?;

        let bridge = harness
            .structured_bridge(&composed.provisioned, cwd)
            .with_context(|| format!("starting a {agent_type} conversation"))?;
        Ok((composed, bridge))
    }

    /// A profile id as a single path segment isol8 will accept for a managed home: no
    /// separator, no `..`, nothing empty.
    ///
    /// The library's CLI keeps its own copy of this. Sharing one would mean exporting a name
    /// across a feature gate the `cli` module sits behind, for four lines.
    fn home_id(profile: &str) -> String {
        let cleaned: String = profile
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        if cleaned.is_empty() {
            "default".to_string()
        } else {
            cleaned
        }
    }

    fn compose_run(
        &self,
        key: &str,
        agent_type: &str,
        cwd: &Path,
        args: Vec<String>,
        io: IoModes,
        options: ConverseOptions,
    ) -> Result<Composed> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;

        // Which of Ubiq's own MCP servers this run gets, and — the reason this is here rather
        // than left to the library — which names still belong to `resolve`.
        //
        // A profile's `defaults.mcps` is read by `resolve` as a list of ids in the **on-disk
        // catalog**, and an id it cannot find there is a `bail!` that fails the whole run. Ubiq's
        // built-ins are in no catalog: they are this process, on a port it bound at startup. So
        // the profile's list is split here — the names this build answers are lifted out and
        // injected below, and only the rest is handed back down as `flags.mcps`, which outranks
        // the profile in the library's own merge. The alternative was to make `resolve` lenient
        // about an unknown catalog id, which would turn a typo in any embedder's profile into a
        // server that silently is not there; a run must still fail loudly for a name nobody
        // answers.
        let mut injected: Vec<String> = Vec::new();
        for name in &options.mcps {
            if !crate::mcp::knows(name) {
                tracing::warn!(
                    mcp = %name,
                    harness = %agent_type,
                    "this build offers no MCP server by that name, so nothing was injected for it"
                );
            } else if !injected.contains(name) {
                injected.push(name.clone());
            }
        }
        let catalog_mcps = options
            .profile
            .as_deref()
            .and_then(|profile| self.profile_mcps(profile))
            .map(|saved| {
                saved
                    .into_iter()
                    .filter(|name| {
                        if crate::mcp::knows(name) {
                            if !injected.contains(name) {
                                injected.push(name.clone());
                            }
                            false
                        } else {
                            true
                        }
                    })
                    .collect::<Vec<String>>()
            });

        // What a run is composed *of* — its account, its model, the profile that names
        // them — is the library's question, and `resolve` is the one place that answers it.
        // Ubiq builds no `RunSpec` of its own beyond the three fields below, so an account
        // named in a profile reaches a pane without this module learning what an account is.
        //
        // The library's own settings file is deliberately not read: Ubiq's settings are the
        // settings surface, and a second file answering the same question is a second
        // answer. That leaves `resolve`'s precedence as flags, then the profile.
        let flags = resolve::RunFlags {
            harness: harness.id(),
            cwd: cwd.to_path_buf(),
            passthrough_args: args,
            // Highest precedence in `resolve`, which is what "the user picked this one" has to
            // mean: an identity — or a model, a thinking level, a mode — chosen when the
            // conversation started outranks the profile's.
            account: options.account,
            model: options.model,
            thinking: options.thinking,
            permission_mode: options.mode,
            // The saved setup the picks sit on top of. `None` is a bare start, and the
            // library then falls back to whatever profile it resolves on its own.
            profile: options.profile,
            // Whatever the profile named that Ubiq does not answer itself, passed back as a flag
            // so the built-ins never reach the catalog lookup. `None` when the profile mentioned
            // no servers at all, which leaves the library's own precedence untouched.
            mcps: catalog_mcps,
            // The two fields a one-shot harness converses through: its prompt is argv, and the
            // only thing joining one turn to the next is the harness's own session id. `resolve`
            // lands them on `spec.initial.prompt` and `spec.resume`, and each harness's
            // provisioner spells out its own flag for them — Ubiq names neither.
            prompt: options.prompt,
            resume: options.resume,
            ..Default::default()
        };
        let mut spec = resolve::resolve(
            &flags,
            &Settings::default(),
            &FsRegistry::new(self.root.join("catalog")),
            &FsAccountStore::new(self.root.join("accounts")),
            &FsProfileStore::new(self.root.join("profiles")),
        )
        .with_context(|| format!("composing a {agent_type} run"))?;

        // The five answers that are Ubiq's rather than the library's: which directory this
        // run's configuration lives in, which face it wears, whether it is confined, — when
        // it is — that it asks nothing, and which of Ubiq's own MCP servers it can reach.
        // The isolation replaces whatever a profile asked for,
        // because the toggle belongs to Ubiq's own settings and applies to both faces alike.
        let structured = io == IoModes::Structured;
        spec.config = ConfigStrategy::Fixed(self.run_dir_for(key));
        spec.io = io;
        spec.isolation = if self.isolate {
            Isolation::Sandboxed(String::new())
        } else {
            Isolation::None
        };
        // A confined run is contained by the sandbox, not by the prompts, so it launches with
        // permissions bypassed — otherwise every step stops on an ask the sandbox already
        // answered. Which mode that *is* stays the harness's own word (`unattended_mode`);
        // Ubiq names none. An explicit pick for this run outranks it: the profile's mode does
        // not, being a default like the isolation toggle it sits under.
        if self.isolate
            && flags.permission_mode.is_none()
            && let Some(mode) = harness.unattended_mode()
        {
            spec.policy
                .get_or_insert_with(Policy::default)
                .permission_mode = Some(mode.to_string());
        }

        // Ubiq's own servers, inline rather than from the catalog: there is no file behind them
        // to name, only a port this process bound at startup. The run's own key rides in the
        // path, which is the whole of how the listener knows which agent is calling — nothing
        // else about the request identifies it (`crate::mcp`).
        match &self.mcp_base {
            Some(base) => {
                for name in &injected {
                    spec.mcps.push(McpRef::Inline(McpServer {
                        id: name.clone(),
                        transport: McpTransport::Http,
                        command: None,
                        args: Vec::new(),
                        env: BTreeMap::new(),
                        url: Some(format!("{base}/mcps/{key}/{name}")),
                        headers: BTreeMap::new(),
                    }));
                }
            }
            // A listener that would not bind was already reported once, at startup. Saying it
            // again per run would bury it; saying nothing would leave a missing tool unexplained.
            None if !injected.is_empty() => tracing::warn!(
                harness = %agent_type,
                "no MCP listener is bound in this process, so none of this run's servers were injected"
            ),
            None => {}
        }

        let templates = harness::FsTemplateStore::new(self.root.join("harness-templates"));
        let mut provisioned = provision::provision(harness.as_ref(), &spec, &templates)
            .with_context(|| format!("composing a {agent_type} run"))?;
        self.resolve_program(agent_type, &mut provisioned.launch);
        // This machine's own variables, before the policy is planned so a grant can be read
        // off them. Never over a name the harness itself set: a confined launch's `env` is the
        // whole environment, and `CLAUDE_CONFIG_DIR` and its siblings are what pin a run to its
        // throwaway configuration.
        self.add_machine_env(&mut provisioned.launch);

        // Every confined run keeps the real home unless the user said otherwise. A home of its
        // own was once the answer to "a second run of the same profile should find its caches,
        // its indexes and its logins where it left them" — but the real home is already where
        // those are, and a replaced one aims every toolchain layer's `~/.cargo`-shaped grant at
        // an empty directory, so the agent could not build. What stays per-run is the
        // configuration, which `CLAUDE_CONFIG_DIR` and its siblings pin to the run dir.
        //
        // The home and the extra grants are the two answers Ubiq holds rather than the library:
        // both are settings a person set on this machine, and the library has no way to ask.
        let mut options = self.isolate_options();
        options.home = home_mode(&self.home);

        let confined = isolate::plan(&provisioned.launch, &spec, &provisioned.dir, &options)
            .with_context(|| format!("resolving the policy for a {agent_type} run"))?;

        // The session record, written now rather than at teardown: a run that
        // crashes is exactly the one whose record is worth keeping, and
        // `archive` needs a meta to find it by. Best effort, like the library's
        // own CLI recorder — a sessions root that cannot be written must never
        // fail a spawn.
        let mut meta = session::SessionMeta::new(
            spec.harness.clone(),
            cwd.to_path_buf(),
            std::iter::once(provisioned.launch.program.clone())
                .chain(provisioned.launch.args.iter().cloned())
                .collect(),
            spec.account.as_ref().map(|a| a.id.clone()),
            if structured {
                "structured".to_string()
            } else {
                "passthrough".to_string()
            },
            provisioned.dir.clone(),
        );
        // The pane's (or agent's) own id, not the library's `<millis>-<pid>`:
        // it is what a teardown has in hand, and the harness's own session id
        // never reaches this process.
        meta.id = key.to_string();
        // Where the login came from, so the teardown can write a refreshed one
        // back. It is recorded here rather than kept in memory because nothing
        // holds the `Composed` that long: a pane's run is torn down by
        // `retire`, which has an id and this record and nothing else.
        meta.login_home = match &provisioned.login_origin {
            Some(Source::Dir(home)) => Some(home.clone()),
            _ => None,
        };
        let _ = session::save(&self.sessions_dir(), &meta);

        Ok(Composed {
            launch: provisioned.launch.clone(),
            confined,
            dir: provisioned.dir.clone(),
            provisioned,
            spec_account: spec.account.as_ref().map(|a| a.id.clone()),
        })
    }

    /// Ubiq's own session store: one directory per run, under Ubiq's config
    /// root. Always passed explicitly, never resolved through
    /// `session::sessions_root` — `AM_SESSIONS` must not be able to redirect a
    /// user's Ubiq transcripts into the library's store.
    fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Copy the harness's own record of the conversation out of a run
    /// directory, and put a login the run refreshed back where it came from,
    /// when that run ends — whether or not the directory is then deleted.
    ///
    /// Which files those are is the harness's answer, not Ubiq's — a path
    /// literal here would be the boundary this module's header names. Entirely
    /// best effort: this runs on teardown paths, and no session that cannot be
    /// archived is a reason to fail a close. A run with no meta is a plain
    /// shell pane, which is the common case rather than an error.
    ///
    /// The harvest belongs here rather than in [`retire`](Self::retire),
    /// [`sweep`](Self::sweep) and [`park_agent`](Self::park_agent) separately
    /// because it is the one thing all three do when a run ends, and the
    /// session record is where the origin was written down — none of them
    /// holds the run's `Provisioned` any more. A run whose login was not
    /// seeded from a directory records no origin, and the harness's own
    /// account of its live login (a keychain) is what finds it again.
    ///
    /// It has to run on the *parking* path too, where the directory is kept.
    /// An OAuth refresh rotates the token and **revokes** the one it was
    /// seeded from, so a run that harvested nothing leaves the account home
    /// holding a dead credential — and every other agent on that account
    /// starts logged out. Keeping the directory does not keep the origin
    /// valid; only the write-back does.
    ///
    /// ponytail: harvesting at teardown means a token the harness rotated
    /// mid-run is lost if Ubiq is killed. Upgrade path is a watcher on the
    /// credential file, writing back as it changes.
    fn archive(&self, key: &str) {
        let sessions = self.sessions_dir();
        let Ok(mut meta) = session::load(&sessions, key) else {
            return;
        };
        let Some(harness) = harness::resolve(&meta.harness) else {
            return;
        };

        if let Some(origin) = meta
            .login_home
            .clone()
            .map(Source::Dir)
            .or_else(|| harness.ambient_login())
        {
            let _ = harness::harvest_login(harness.as_ref(), &self.run_dir_for(key), &origin);
        }

        let dest = sessions.join(key).join("harness");
        for src in harness.transcripts(&self.run_dir_for(key)) {
            let Some(name) = src.file_name() else {
                continue;
            };
            let _ = std::fs::create_dir_all(&dest);
            let _ = std::fs::copy(&src, dest.join(name));
        }

        meta.finished_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        let _ = session::save(&sessions, &meta);
    }

    /// Whether anything that makes a session logged in landed in `dir`.
    ///
    /// The library seeds a harness's own login files into the run it composes — from the account a
    /// profile named, or failing that from the user's real home. It cannot seed what is not a file,
    /// so a login held in the operating system's keychain leaves nothing behind and the run starts
    /// unauthenticated. A harness that declares no login files at all is not answerable this way,
    /// so it counts as fine.
    fn has_login(harness: &dyn harness::Harness, dir: &Path) -> bool {
        let seed = harness.config_anchor().login_seed;
        seed.is_empty() || seed.iter().any(|file| dir.join(&file.dst).exists())
    }

    /// Write down the harness's own id for a conversation, while it is still
    /// running.
    ///
    /// Nothing else writes this field before teardown, and a `kill -9` is
    /// exactly the case that has to survive: a resume needs the harness's id,
    /// and a process that never got to tear down never recorded one. So this
    /// is called as soon as the harness names its session, not at the end.
    ///
    /// Best effort like the rest of this module — a sessions root that cannot
    /// be written must never fail anything the user asked for.
    pub fn remember_session(&self, agent: AgentId, id: &str) {
        let sessions = self.sessions_dir();
        let key = agent.to_string();
        let Ok(mut meta) = session::load(&sessions, &key) else {
            return;
        };
        meta.harness_session_id = Some(id.to_string());
        let _ = session::save(&sessions, &meta);
    }

    /// Delete every login file the library seeded into a run directory that is
    /// being kept.
    ///
    /// All of them, not only the ones marked credential:
    /// `provision::seed_zero_config_login` early-returns the moment *any*
    /// `login_seed` destination already exists, on the reasoning that a login
    /// already materialized (an account home, a profile overlay) wins over the
    /// zero-config fallback. So one leftover file — for Claude Code that is
    /// the non-credential `.claude.json` — is enough to make the next launch
    /// skip seeding entirely: the credential is never refreshed, and the
    /// `login_origin` it would have returned comes back `None`, which also
    /// disables the next [`archive`](Self::archive) harvest.
    ///
    /// Which files those are stays the harness's answer, read through the same
    /// access path as [`has_login`](Self::has_login).
    pub fn scrub_login(&self, key: &str) {
        let Ok(meta) = session::load(&self.sessions_dir(), key) else {
            return;
        };
        let Some(harness) = harness::resolve(&meta.harness) else {
            return;
        };
        let dir = self.run_dir_for(key);
        for file in harness.config_anchor().login_seed {
            let _ = std::fs::remove_file(dir.join(&file.dst));
        }
    }

    /// End an agent's run but keep its directory: the process is gone, the
    /// conversation may come back.
    ///
    /// The run directory *is* the harness's session store — Claude's
    /// `projects/<slug>/*.jsonl`, Codex's rollouts, opencode's data dir — so
    /// keeping it is the whole of what lets a resume find the conversation
    /// again. What must not be kept is the login: it is seeded fresh at every
    /// launch, and a stale copy left here would suppress that seeding (see
    /// [`scrub_login`](Self::scrub_login)).
    pub fn park_agent(&self, agent: AgentId) {
        let key = agent.to_string();
        self.archive(&key);
        self.scrub_login(&key);
        // Its process has gone, so the MCP listener has nobody left to answer for at that
        // address. A parked conversation registers again when it is revived.
        self.mcp_agents.forget(&key);
    }

    /// Copy one agent's run directory onto another's, so the second resumes
    /// from the first's conversation instead of starting empty.
    ///
    /// The login files are excluded — they are seeded again at launch, and a
    /// copy of them would be the stale login `scrub_login` exists to avoid.
    /// So is `sessions/`: [`compose_run`](Self::compose_run) writes the fork's
    /// own meta, and the source's would name the wrong run.
    ///
    /// Refused for a harness with no config lever. That is grok, which writes
    /// its sessions to the real `~/.grok/sessions/` whatever `HOME` says (its
    /// own module header records the observation), so the copy would isolate
    /// nothing and the two agents would append to one store.
    ///
    /// ponytail: a synchronous copy on the caller's thread. The caller only
    /// offers a fork on an idle conversation, because copying a store
    /// mid-append tears the last record — move this to a thread if a large
    /// directory is ever seen stalling the loop.
    pub fn fork_run(&self, from: AgentId, to: AgentId) -> Result<()> {
        let key = from.to_string();
        let meta = session::load(&self.sessions_dir(), &key)
            .with_context(|| format!("no session record for the agent being forked ({key})"))?;
        let harness = harness::resolve(&meta.harness)
            .ok_or_else(|| anyhow!("unknown harness '{}'", meta.harness))?;
        let anchor = harness.config_anchor();
        if anchor.levers.is_empty() {
            bail!(
                "{} keeps its conversations outside the run directory, so a fork would share one store",
                meta.harness
            );
        }
        let skip: Vec<PathBuf> = anchor.login_seed.iter().map(|f| f.dst.clone()).collect();
        copy_tree(
            &self.run_dir_for(&key),
            &self.run_dir_for(&to.to_string()),
            Path::new(""),
            &skip,
        )
    }

    /// Whether a harness keeps its conversation somewhere a fork can copy —
    /// that is, whether it declares a relocating config lever at all. The UI
    /// asks so it can disable the control with a reason rather than offer a
    /// fork that would not isolate anything.
    pub fn forkable(&self, agent_type: &str) -> bool {
        harness::resolve(agent_type).is_some_and(|h| !h.config_anchor().levers.is_empty())
    }

    /// Whether the conversation that owns a run directory is meant to outlive
    /// its process, read from Ubiq's own row rather than the library's meta:
    /// persistence is Ubiq's vocabulary, and `SessionMeta` belongs to
    /// `agent-manager`.
    ///
    /// False on any error, including the common one — a pane has no row at
    /// all, so a terminal's directory is swept exactly as it always was.
    ///
    /// Read through [`crate::conversation_record`] rather than by parsing the
    /// file here: the flag decides three separate things — whether a close
    /// parks or deletes, whether the boot sweep passes a directory over, and
    /// whether the collector may take a record — and three readers of one
    /// field is how they drift apart.
    fn is_persistent(&self, key: &str) -> bool {
        crate::conversation_record::is_persistent_at(&self.sessions_dir(), key)
    }

    /// Remove what a pane's run left behind. Best effort: a directory that
    /// cannot be deleted is a stale directory, not a reason to fail a close the
    /// user already saw happen.
    pub fn retire(&self, pane: PaneId) {
        self.archive(&pane.to_string());
        self.mcp_agents.forget(&pane.to_string());
        let _ = std::fs::remove_dir_all(self.run_dir(pane));
    }

    /// Delete every run directory left by a previous process, except the ones
    /// a persistent conversation still needs.
    ///
    /// A run directory outlives its pane only when Ubiq did not get to close
    /// it — a crash, a kill. Sweeping at startup is what keeps that from
    /// accumulating, and it is safe because no pane from a previous process is
    /// still running.
    ///
    /// A persistent conversation's directory is parked instead of deleted: it
    /// holds the harness's session store, which is the only reason the
    /// conversation can be resumed at all. It is still parked rather than
    /// simply left alone — the crash is precisely the case where the account
    /// home is holding a token the run already rotated away.
    pub fn sweep(&self) {
        let runs = self.root.join("runs");
        let Ok(entries) = std::fs::read_dir(&runs) else {
            return;
        };
        for entry in entries.flatten() {
            // A crashed run is where the record matters most, and its meta was
            // written when the run was composed, so this works verbatim here.
            let key = entry.file_name().to_string_lossy().into_owned();
            self.archive(&key);
            if self.is_persistent(&key) {
                self.scrub_login(&key);
                continue;
            }
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }

    /// Where a pane's run is provisioned. One directory per pane, named by it.
    fn run_dir(&self, pane: PaneId) -> PathBuf {
        self.run_dir_for(&pane.to_string())
    }

    /// Where an agent's conversation is provisioned.
    pub fn agent_dir(&self, agent: AgentId) -> PathBuf {
        self.run_dir_for(&agent.to_string())
    }

    /// Remove what an agent's conversation left behind.
    pub fn retire_agent(&self, agent: AgentId) {
        self.archive(&agent.to_string());
        self.mcp_agents.forget(&agent.to_string());
        let _ = std::fs::remove_dir_all(self.agent_dir(agent));
    }

    /// One directory per run, named by whatever owns it — a pane or an agent.
    /// Both ids are ULIDs, so neither can be mistaken for the other's.
    fn run_dir_for(&self, key: &str) -> PathBuf {
        self.root.join("runs").join(key)
    }
}

/// Copy `src` onto `dst`, recursively, skipping the run-dir-relative paths in
/// `skip` and anything under `sessions/`. `rel` is where in the tree the
/// recursion currently is, which is what makes a nested skip entry (grok's
/// `.grok/auth.json`) match.
///
/// Here rather than from a crate because std has no recursive copy and this is
/// the only caller — [`Agents::fork_run`].
fn copy_tree(src: &Path, dst: &Path, rel: &Path, skip: &[PathBuf]) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let here = rel.join(entry.file_name());
        if here == Path::new("sessions") || skip.contains(&here) {
            continue;
        }
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to, &here, skip)?;
        } else {
            std::fs::copy(entry.path(), &to)
                .with_context(|| format!("copying to {}", to.display()))?;
        }
    }
    Ok(())
}

/// The library's home mode for a setting.
///
/// A named home is keyed by [`Agents::home_id`] so a name a person typed cannot become a path
/// isol8 refuses — a separator or a `..` in it would otherwise fail at the spawn.
fn home_mode(home: &AgentHome) -> isolate::HomeMode {
    match home {
        AgentHome::Inherit => isolate::HomeMode::Inherit,
        AgentHome::Ephemeral => isolate::HomeMode::Ephemeral,
        AgentHome::Named(name) => isolate::HomeMode::Managed(Agents::home_id(name)),
    }
}

/// A `~`-prefixed grant against the real home.
///
/// The library grants absolute paths, and a person types `~/.cargo`. Expanding here rather than
/// passing the token through keeps the grant correct under a *replaced* home too, where isol8
/// would resolve `~` to the replacement and grant the wrong directory.
fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => isolate::real_home().join(rest),
        None if path == "~" => isolate::real_home(),
        _ => PathBuf::from(path),
    }
}

/// Split a command line into words: whitespace-separated, with `"` and `'` grouping a run of
/// words into one. No backslash escaping — a Windows path (`C:\tools\claude.exe`) must reach the
/// far side with its backslashes untouched, not eaten as escapes.
///
/// Shared with tool runs, which split a command textbox and a parameters textbox by the same
/// rule a harness override is split by.
pub(crate) fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in command.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                in_word = true;
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(current);
    }
    words
}

/// Resolve one word of a command override to what should actually be exec'd: unchanged when it
/// already names a path (it contains a `/` or a `\`), otherwise looked up on the login shell's
/// `PATH` exactly as a harness's own bare program is — so `claudex` resolves the same way
/// `claude` does.
///
/// Shared with tool runs, so a bare `cargo` in a tool row finds the same binary a shell would.
pub(crate) fn resolve_bare(word: &str) -> String {
    if word.contains('/') || word.contains('\\') {
        return word.to_string();
    }
    crate::shells::locate(word)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| word.to_string())
}

/// Try `command` as a way to start a harness: split it, resolve a bare program, and run it with
/// `--version`. Used only for [`Message::CheckAgentCommand`][cac] — the user is typing the answer
/// and wants to know before saving it, not after a pane fails to spawn.
///
/// `ok` is "it ran and exited successfully"; the detail is one short line either way: the
/// version output's first line, or why it did not run.
///
/// [cac]: ubiq_proto::messages::Message::CheckAgentCommand
pub(crate) fn check_command(command: &str) -> (bool, String) {
    let mut words = split_command(command);
    if words.is_empty() {
        return (false, "empty command".to_string());
    }
    let program = resolve_bare(&words.remove(0));
    words.push("--version".to_string());

    let mut child = match std::process::Command::new(&program)
        .args(&words)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return (false, "not found on PATH".to_string()),
    };

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return command_outcome(child, status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return (false, "timed out".to_string());
            }
        }
    }
}

/// What a finished check command found: its first line of output, on either stream, and whether
/// it counts as having worked.
fn command_outcome(
    mut child: std::process::Child,
    status: std::process::ExitStatus,
) -> (bool, String) {
    use std::io::Read;

    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    if out.trim().is_empty()
        && let Some(mut stderr) = child.stderr.take()
    {
        let _ = stderr.read_to_string(&mut out);
    }
    let line = out.lines().next().unwrap_or("").trim().to_string();

    if status.success() {
        (true, line)
    } else {
        (false, format!("exited with status {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two questions the coordinator asks before it spawns anything, and the answer they share
    /// today. Both are the library's facts, read here rather than restated — a list of harness
    /// names in this crate is the bug `G95` was.
    ///
    /// `converses` decides whether a conversation may start at all; `multi_turn` decides whether
    /// a turn is written to a running process or becomes the next launch's argv. Every structured
    /// harness answers both `true` today: Copilot and opencode used to be conversable but one-shot,
    /// until ACP (`copilot --acp`, `opencode acp`) made them multi-turn like Claude Code, codex and
    /// Grok.
    #[test]
    fn converses_and_multi_turn_read_the_librarys_three_answers() {
        let agents = Agents::new(std::env::temp_dir().join("ubiq-test-agents"), false);

        // copilot and opencode now speak ACP like claude-code, codex and grok, and ACP is
        // inherently multi-turn — so all five converse and all five take a second turn.
        for id in ["claude-code", "codex", "copilot", "opencode", "grok"] {
            assert!(agents.converses(id), "{id} should converse");
            assert!(agents.multi_turn(id), "{id} should take a second turn");
        }
    }

    /// An id the library does not know converses no more than one it knows cannot. Refusing is
    /// the honest answer to both; `is_agent_type` is what tells the two refusals apart.
    #[test]
    fn an_unknown_harness_neither_converses_nor_takes_a_second_turn() {
        let agents = Agents::new(std::env::temp_dir().join("ubiq-test-agents"), false);

        assert!(!agents.converses("not-a-harness"));
        assert!(!agents.multi_turn("not-a-harness"));
        assert!(!agents.is_agent_type("not-a-harness"));
    }

    /// A bare name is one word, and a Windows path's backslashes are not escapes: both must
    /// come out exactly as typed for `resolve_bare` to tell "look this up on PATH" from
    /// "this already names a path".
    #[test]
    fn split_command_keeps_backslashes_and_splits_on_whitespace() {
        assert_eq!(split_command("claudex"), vec!["claudex"]);
        assert_eq!(split_command("/opt/bin/claude"), vec!["/opt/bin/claude"]);
        assert_eq!(
            split_command(r"C:\tools\claude.exe"),
            vec![r"C:\tools\claude.exe"]
        );
        assert_eq!(
            split_command("mise exec -- opencode"),
            vec!["mise", "exec", "--", "opencode"]
        );
    }

    /// The test that protects the whole point of the probe feature: swapping in a shell must
    /// change nothing about the policy a real login would render, and the harness's own argv is
    /// the only thing that differs.
    ///
    /// Built on a `Launch` shaped exactly as `isolate::confined_launch` renders one on macOS
    /// (`sandbox-exec -p <policy> <harness argv...>` — read from its source rather than assumed),
    /// instead of calling the real `login_confined`/`confined_launch` pair: this process is
    /// itself running under a sandbox while these tests execute, and a sandbox cannot nest —
    /// `isolate::ensure_can_confine` refuses exactly that, which is what the real pair would hit
    /// here regardless of platform.
    #[test]
    fn probe_launch_keeps_the_harness_s_policy_and_only_swaps_the_argv_after_it() {
        let harness_launch = Launch {
            program: "/usr/bin/sandbox-exec".to_string(),
            args: vec![
                "-p".to_string(),
                "(version 1)(deny default)(allow file-read* (subpath \"/usr\"))".to_string(),
                "/opt/harness/bin/claude".to_string(),
                "auth".to_string(),
                "login".to_string(),
            ],
            env: vec![("HOME".to_string(), "/tmp/capture-home".to_string())],
            env_remove: Vec::new(),
            env_clear: true,
        };

        let probe_launch = Agents::shell_probe_launch(harness_launch.clone());

        assert_eq!(
            probe_launch.program, harness_launch.program,
            "the probe must still run through sandbox-exec"
        );
        assert_eq!(probe_launch.args[0], "-p");
        assert_eq!(
            probe_launch.args[1], harness_launch.args[1],
            "a probe must run under the exact policy a real login would get"
        );
        assert_eq!(
            probe_launch.args[2..].to_vec(),
            vec![crate::shells::default_program(), "-i".to_string()],
            "everything after the policy must be the shell, run interactively"
        );
        assert_ne!(
            probe_launch.args[2..],
            harness_launch.args[2..],
            "the argv after the policy is the only thing that may differ"
        );
        assert_eq!(probe_launch.env, harness_launch.env);
        assert_eq!(probe_launch.env_clear, harness_launch.env_clear);
    }

    /// Every agent type the library knows is offered, with an id the spawn path
    /// accepts — a list the menu could show but the coordinator would refuse is
    /// the failure this guards.
    #[test]
    fn every_offered_type_is_a_type_the_spawn_path_accepts() {
        let agents = Agents::new(PathBuf::from("/nowhere"), false);
        let types = agents.types();

        assert!(!types.is_empty(), "the library knows some harnesses");
        for offered in &types {
            assert!(
                agents.is_agent_type(&offered.id),
                "offered {} but the spawn path would refuse it",
                offered.id
            );
            assert!(!offered.label.is_empty());
        }
    }

    /// A name no harness answers to is refused, so an unknown agent type is an
    /// error about a choice rather than a failed process.
    #[test]
    fn an_unknown_name_is_not_an_agent_type() {
        let agents = Agents::new(PathBuf::from("/nowhere"), false);
        assert!(!agents.is_agent_type("not-a-harness"));
    }

    /// A run directory is named by the pane that owns it and sits under Ubiq's
    /// own root — which is what lets a close delete exactly one run's state.
    #[test]
    fn a_run_directory_is_named_by_its_pane() {
        let agents = Agents::new(PathBuf::from("/tmp/ubiq-root"), false);
        let pane = PaneId::generate();

        let dir = agents.run_dir(pane);

        assert!(dir.starts_with("/tmp/ubiq-root/runs"));
        assert!(dir.ends_with(pane.to_string()));
    }

    /// Composing a run writes the harness's configuration into that directory
    /// and answers with the launch, so a pane has something to spawn.
    #[test]
    fn composing_provisions_into_the_pane_s_own_directory() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let pane = PaneId::generate();

        let composed = agents
            .compose(pane, "claude-code", cwd.path(), Vec::new())
            .expect("composing a claude-code run");

        assert_eq!(composed.dir, agents.run_dir(pane));
        assert!(composed.dir.exists());
        assert!(!composed.exec().unwrap().program.is_empty());
        assert!(!composed.is_confined(), "isolation was off");

        agents.retire(pane);
        assert!(!composed.dir.exists());
    }

    /// G92: a conversation is confined by the same setting a pane is. The argv the policy
    /// renders into is `confined_launch`'s job and is asserted where it can actually be
    /// rendered (`agent-manager`'s `tests/confined_bridge.rs`) — a sandbox cannot nest, and
    /// these tests may themselves be running inside one. What is Ubiq's own answer, and so
    /// what is asserted here, is that the structured face plans a policy at all.
    #[test]
    fn a_structured_run_is_confined_when_isolation_is_on() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), true);

        let composed = agents
            .compose_run(
                "agent-1",
                "claude-code",
                cwd.path(),
                Vec::new(),
                IoModes::Structured,
                ConverseOptions::default(),
            )
            .expect("composing a structured claude-code run");

        assert!(
            composed.is_confined(),
            "a conversation follows the isolation setting, same as a pane"
        );
    }

    /// P6: every confined run keeps the real home, whether its conversation named a profile
    /// or not. A home of its own would aim each toolchain layer's `~/.cargo`-shaped grant at
    /// an empty directory, so an agent could hold `cargo` in its `PATH` and still not build;
    /// what stays per-run is the configuration dir, not the home. The rendered policy's home
    /// path is the proof, and rendering one spawns nothing.
    #[test]
    fn every_confined_run_keeps_the_real_home() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        given_an_account(root.path(), "work.setup", "work");
        let agents = Agents::new(root.path(), true);

        let compose = |profile: Option<String>| {
            agents
                .compose_run(
                    "agent-1",
                    "claude-code",
                    cwd.path(),
                    Vec::new(),
                    IoModes::Structured,
                    ConverseOptions {
                        profile,
                        ..Default::default()
                    },
                )
                .expect("composing a structured claude-code run")
        };

        let real_home = isolate::real_home();

        for profile in [Some("work.setup".to_string()), None] {
            let composed = compose(profile.clone());
            let home = isolate::describe(composed.confined.as_ref().unwrap())
                .unwrap()
                .home_path;
            assert_eq!(
                home,
                real_home,
                "a {} run keeps the real home, not {}",
                if profile.is_some() {
                    "defined"
                } else {
                    "an ad-hoc"
                },
                home.display()
            );
        }
    }

    /// Write a profile naming an account, and the account's own captured-login home,
    /// under a Ubiq config root. This is the fixture `am account login` produces.
    fn given_an_account(root: &Path, profile: &str, account: &str) {
        let home = root.join("accounts").join(account);
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/.credentials.json"),
            b"{\"from\":\"account\"}",
        )
        .unwrap();
        std::fs::write(
            root.join("accounts").join(format!("{account}.toml")),
            format!("id = \"{account}\"\nhome = \"{}\"\n", home.display()),
        )
        .unwrap();

        let dir = root.join("profiles").join(profile);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("profile.toml"),
            format!("harness = \"claude-code\"\naccount = \"{account}\"\n"),
        )
        .unwrap();
    }

    /// The point of the whole package: a profile named `default` names an account, and the
    /// account's captured credential is what the run is composed with — not whatever
    /// happens to be in the user's own home. The byte comparison is the proof, because a
    /// zero-config seed from `$HOME` would also leave a file at that path.
    #[test]
    fn a_profile_s_account_is_what_composes_the_run() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        given_an_account(root.path(), "default", "work");

        let agents = Agents::new(root.path(), false);
        let pane = PaneId::generate();
        let composed = agents
            .compose(pane, "claude-code", cwd.path(), Vec::new())
            .expect("composing a claude-code run against the default profile");

        assert_eq!(composed.account(), Some("work"));
        assert_eq!(
            std::fs::read(composed.dir.join(".credentials.json")).unwrap(),
            b"{\"from\":\"account\"}",
            "the account's own credential is what reached the run"
        );

        agents.retire(pane);
    }

    /// An account id nothing answers to fails the compose rather than starting a run that
    /// would report itself logged out from inside its own transcript.
    #[test]
    fn an_unknown_account_refuses_the_run() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let dir = root.path().join("profiles").join("default");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("profile.toml"),
            "harness = \"claude-code\"\naccount = \"nope\"\n",
        )
        .unwrap();

        let agents = Agents::new(root.path(), false);
        let Err(error) = agents.compose(PaneId::generate(), "claude-code", cwd.path(), Vec::new())
        else {
            panic!("an unknown account should refuse the run");
        };

        assert!(
            format!("{error:#}").contains("nope"),
            "the refusal names the account that is missing: {error:#}"
        );
    }

    /// No profile at all resolves exactly as it did before this seam existed, which is what
    /// keeps a machine that has never configured an account working.
    #[test]
    fn no_profile_composes_a_run_with_no_account() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let pane = PaneId::generate();

        let composed = agents
            .compose(pane, "claude-code", cwd.path(), Vec::new())
            .expect("composing with no profiles root at all");

        assert_eq!(composed.account(), None);
        agents.retire(pane);
    }

    /// A login prepared against `account`, as `begin_login` would leave it — without needing
    /// the harness's binary, which a test machine may not have.
    fn a_pending_login(agents: &Agents, account: &str) -> PendingLogin {
        let home = agents
            .account_store()
            .login_home(account)
            .expect("a capture home");
        PendingLogin {
            account: account.to_string(),
            agent_type: "claude-code".to_string(),
            files: vec![PathBuf::from(".claude/.credentials.json")],
            captured_before: Agents::credential_mtime(
                &home,
                &[PathBuf::from(".claude/.credentials.json")],
            ),
            home,
            launch: Launch::default(),
            probe: false,
        }
    }

    /// Write a credential under a login's home, as the harness's own flow would.
    fn the_login_writes_a_credential(pending: &PendingLogin) {
        let path = pending.home.join(".claude/.credentials.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{\"token\":\"fresh\"}").unwrap();
    }

    /// The happy path: the harness wrote its credential, so the account exists and names the
    /// home the credential is in. That reference is the whole of what is recorded — the
    /// credential itself is never copied anywhere.
    #[test]
    fn a_login_that_wrote_a_credential_becomes_an_account() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let pending = a_pending_login(&agents, "work");

        the_login_writes_a_credential(&pending);
        agents
            .finish_login(&pending)
            .expect("the login is captured");

        let accounts = agents.accounts().expect("listing accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "work");
        assert!(
            accounts[0].logged_in.contains(&"claude-code".to_string()),
            "the harness whose credential landed is reported logged in: {accounts:?}"
        );
    }

    /// Abort: the pane was closed before the flow finished, so no credential exists. Not an
    /// error in Ubiq, but emphatically not an account either — which is what makes pressing
    /// abort and starting again safe.
    #[test]
    fn an_abandoned_login_captures_nothing() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let pending = a_pending_login(&agents, "work");

        assert!(agents.finish_login(&pending).is_err());
        assert!(
            agents.accounts().expect("listing accounts").is_empty(),
            "an abandoned login must leave no account behind"
        );
    }

    /// The case the timestamp exists for: a credential is already there from an earlier
    /// attempt, and the harness exits cleanly without touching it. Without the before-shot
    /// that stale file would read as a fresh success and the account would claim a login
    /// nobody performed.
    #[test]
    fn a_login_that_left_a_stale_credential_captures_nothing() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);

        // An earlier attempt's credential, in place *before* this login is prepared.
        let early = a_pending_login(&agents, "work");
        the_login_writes_a_credential(&early);

        let pending = a_pending_login(&agents, "work");
        assert!(
            pending.captured_before.is_some(),
            "the before-shot must see the credential that is already there"
        );

        let error = agents
            .finish_login(&pending)
            .expect_err("an untouched credential is not a login");
        assert!(
            format!("{error:#}").contains("untouched"),
            "the reason says the credential was not refreshed: {error:#}"
        );
    }

    /// An account referencing an environment variable has no captured home, so no harness is
    /// reported logged in — rather than every harness being claimed because a home is absent.
    #[test]
    fn an_account_with_no_captured_home_reports_no_logins() {
        let root = tempfile::TempDir::new().unwrap();
        let accounts_root = root.path().join("accounts");
        std::fs::create_dir_all(&accounts_root).unwrap();
        std::fs::write(
            accounts_root.join("byenv.toml"),
            "id = \"byenv\"\napi_key_env = \"ANTHROPIC_API_KEY\"\n",
        )
        .unwrap();

        let accounts = Agents::new(root.path(), false)
            .accounts()
            .expect("listing accounts");

        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "byenv");
        assert!(accounts[0].logged_in.is_empty());
    }

    /// The sweep clears what a previous process left in the runs directory, and
    /// tolerates there being nothing there at all.
    #[test]
    fn the_sweep_clears_stale_runs_and_tolerates_none() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);

        agents.sweep();

        let stale = root.path().join("runs").join("01ABCDEF");
        std::fs::create_dir_all(stale.join("inside")).unwrap();
        agents.sweep();

        assert!(!stale.exists());
    }

    /// A run directory as a crashed or parked process leaves one: the session
    /// record the storage paths read, every login file the library would have
    /// seeded, and one file standing in for the harness's own session store.
    ///
    /// `login_home` is always set to a directory of its own, so `archive`'s
    /// harvest writes back there rather than reaching for the machine's real
    /// ambient login (Claude Code's keychain) from a test.
    fn given_a_run(agents: &Agents, key: &str, harness_id: &str) -> PathBuf {
        let dir = agents.run_dir_for(key);
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        std::fs::write(dir.join("projects").join("store.jsonl"), "a turn").unwrap();

        let harness = harness::resolve(harness_id).unwrap();
        for file in harness.config_anchor().login_seed {
            let dst = dir.join(&file.dst);
            std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
            std::fs::write(&dst, "a login").unwrap();
        }

        let origin = agents.root.join("origins").join(key);
        std::fs::create_dir_all(&origin).unwrap();
        let mut meta = session::SessionMeta::new(
            harness_id.to_string(),
            PathBuf::from("/tmp"),
            vec!["true".to_string()],
            None,
            "structured".to_string(),
            dir.clone(),
        );
        meta.id = key.to_string();
        meta.login_home = Some(origin);
        session::save(&agents.sessions_dir(), &meta).unwrap();
        dir
    }

    /// Whether a Claude Code run directory still holds anything that makes it
    /// logged in, asked the same way the composer asks.
    fn logged_in(dir: &Path) -> bool {
        Agents::has_login(harness::resolve("claude-code").unwrap().as_ref(), dir)
    }

    /// Every seed destination goes, not only the credential: leaving Claude's
    /// non-credential `.claude.json` behind would make the next launch's
    /// `seed_zero_config_login` decide a login was already there.
    #[test]
    fn scrubbing_a_login_removes_every_seed_destination() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let dir = given_a_run(&agents, "agent-1", "claude-code");

        let seed = harness::resolve("claude-code")
            .unwrap()
            .config_anchor()
            .login_seed;
        assert!(
            seed.iter().any(|f| f.credential) && seed.iter().any(|f| !f.credential),
            "this test is only meaningful with both kinds present"
        );

        agents.scrub_login("agent-1");

        for file in seed {
            assert!(
                !dir.join(&file.dst).exists(),
                "{} survived",
                file.dst.display()
            );
        }
        assert!(dir.join("projects").join("store.jsonl").exists());
    }

    /// Parking keeps the harness's session store — that is the whole point —
    /// and takes the login with it.
    #[test]
    fn parking_keeps_the_store_and_drops_the_login() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let agent = AgentId::generate();
        let dir = given_a_run(&agents, &agent.to_string(), "claude-code");

        agents.park_agent(agent);

        assert!(dir.join("projects").join("store.jsonl").exists());
        assert!(!logged_in(&dir));
    }

    /// Retiring is unchanged: the directory goes.
    #[test]
    fn retiring_an_agent_still_removes_the_directory() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let agent = AgentId::generate();
        let dir = given_a_run(&agents, &agent.to_string(), "claude-code");

        agents.retire_agent(agent);

        assert!(!dir.exists());
    }

    /// A fork copies the store, so the second agent resumes the first's
    /// conversation, and leaves the login behind to be seeded fresh.
    #[test]
    fn forking_copies_the_store_and_not_the_login() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let from = AgentId::generate();
        let to = AgentId::generate();
        given_a_run(&agents, &from.to_string(), "claude-code");

        agents
            .fork_run(from, to)
            .expect("forking a claude-code run");

        let forked = agents.agent_dir(to);
        assert_eq!(
            std::fs::read_to_string(forked.join("projects").join("store.jsonl")).unwrap(),
            "a turn"
        );
        assert!(!logged_in(&forked));
    }

    /// grok has no config lever, so its sessions land in the real home
    /// whatever the run directory holds — a fork there would share one store,
    /// and is refused rather than quietly doing nothing.
    #[test]
    fn forking_a_harness_with_no_lever_is_refused() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let from = AgentId::generate();
        given_a_run(&agents, &from.to_string(), "grok");

        assert!(!agents.forkable("grok"));
        assert!(agents.forkable("claude-code"));
        assert!(agents.fork_run(from, AgentId::generate()).is_err());
    }

    /// The sweep spares a persistent conversation's directory — that is where
    /// its harness's session store lives — but still parks it, because a crash
    /// is exactly when the origin is left holding a rotated-away token.
    #[test]
    fn the_sweep_parks_a_persistent_run_and_deletes_the_rest() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let kept = given_a_run(&agents, "agent-kept", "claude-code");
        let gone = given_a_run(&agents, "agent-gone", "claude-code");
        std::fs::write(
            agents
                .sessions_dir()
                .join("agent-kept")
                .join("conversation.json"),
            r#"{"persistent": true}"#,
        )
        .unwrap();

        agents.sweep();

        assert!(kept.join("projects").join("store.jsonl").exists());
        assert!(!logged_in(&kept));
        assert!(!gone.exists());
    }
}
