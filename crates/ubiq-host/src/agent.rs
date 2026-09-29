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
use agent_manager::account::{Account, AccountStore, FsAccountStore};
use agent_manager::config::{McpServer, McpTransport};
use agent_manager::harness::{self, Launch, ModelInfo};
use agent_manager::home::{HomeStore, login_validity};
use agent_manager::io::IoBridge;
use agent_manager::isolate::{self, Confined, IsolateOptions};
use agent_manager::profile::{
    FsProfileStore, Profile as DefinitionRecord, ProfileDefaults, ProfileStore, ScopedProfileStore,
};
use agent_manager::provision;
use agent_manager::registry::FsRegistry;
use agent_manager::resolve;
use agent_manager::session;
use agent_manager::settings::Settings;
use agent_manager::spec::{ConfigStrategy, IoModes, Isolation, McpRef, Policy};
use anyhow::{Context, Result, anyhow, bail};
use ubiq_proto::conversation::ConfigChoice;
use ubiq_proto::ids::{PaneId, ProjectId};
use ubiq_proto::messages::{AccountInfo, AgentDefinition, AgentTypeInfo, LoginStatus};
use ubiq_proto::settings::{AgentHome, Grant};
use ubiq_proto::work::AgentId;

use crate::environment::Environment;
use crate::mcp::catalogue;

/// Where agent definitions live, under the config root and under a project's own folder.
pub const DEFINITIONS_DIR: &str = "agent-definitions";

/// What that directory was called while an agent definition was still called a profile
/// (`D174`). A tree written by an older build is renamed in place the first time the new name
/// is asked for, so nobody's saved setups are orphaned by the rename.
const LEGACY_DEFINITIONS_DIR: &str = "profiles";

/// The definitions directory under `base`, migrating the old name onto the new one if that is
/// what is there. A rename that fails is reported and the old directory used, because reading
/// the user's definitions from where they are beats refusing to read them at all.
fn definitions_dir(base: &Path) -> PathBuf {
    let dir = base.join(DEFINITIONS_DIR);
    let legacy = base.join(LEGACY_DEFINITIONS_DIR);
    if !dir.exists() && legacy.is_dir() {
        if let Err(error) = std::fs::rename(&legacy, &dir) {
            tracing::warn!(
                "the profiles under {} could not be renamed to {}: {error}",
                legacy.display(),
                dir.display()
            );
            return legacy;
        }
        tracing::info!(
            "profiles under {} are now agent definitions",
            base.display()
        );
    }
    dir
}

/// Whether a save may go ahead: a **new** definition needs a harness to name, an existing one
/// does not.
///
/// A machine with no harness has nothing a definition could run, so creating one is refused in
/// the host rather than left to a hidden button — every surface that writes a definition writes
/// it through here. Editing is not refused: a machine that has lost its harness must still be
/// able to repair what it already wrote.
fn may_write_definition(exists: bool, has_harness: bool) -> Result<()> {
    if !exists && !has_harness {
        bail!(
            "no harness is configured on this machine \u{2014} install one, or set its command \
             in settings, before writing an agent definition"
        );
    }
    Ok(())
}

/// Merge back the MCP servers this definition's role flags imply, keeping what it already names
/// and its order. The rule the flags exist for: a coordinator without the coordinator's servers
/// is not a coordinator, whatever a checklist was left saying.
fn apply_role_mcps(definition: &mut AgentDefinition) {
    for name in catalogue::role_mcps(definition.mission_coordinator, definition.mission_worker) {
        if !definition.mcps.contains(&name) {
            definition.mcps.push(name);
        }
    }
}

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
    /// The id of the account this run resolved to, when a definition named one.
    spec_account: Option<String>,
    /// A stale catalog reference (an mcp id, a skill id, an account id, a hook id) that the
    /// definition or the run's flags named and `resolve` could not find — dropped from the spec
    /// rather than failing this run. See [`agent_manager::spec::RunSpec::problems`]. Reported by
    /// the caller as a dismissable notification once the run has actually started; empty in the
    /// overwhelmingly common case.
    pub problems: Vec<String>,
}

/// Everything a caller chooses about one run, over and above the harness, the folder and the
/// face it wears.
///
/// One struct rather than seven parameters because [`Agents::converse`] and
/// [`Agents::compose`] hand the identical set through to the same place, and a pane's answer to
/// all of it is `Default::default()` — the library resolves what nothing named.
///
/// Every field is a *pick*, and a pick outranks the definition inside the library's `resolve`. `None`
/// everywhere is the zero-config start: whatever the harness, its definition and its own defaults
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
    pub definition: Option<String>,
    /// The project this run belongs to, when it belongs to one. Not a pick: it is what decides
    /// **which definitions exist** for this run — that project's own are resolvable here and
    /// nowhere else, and they shadow a global definition of the same name. `None` (a login pane, a
    /// run outside any project) resolves against the global root alone, which is what every run
    /// did before project scoping.
    pub project: Option<ProjectId>,
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
    /// The picks only. What the definition saved is read from the definition inside `compose_run`,
    /// because that is where the definition is known — see the note there.
    pub mcps: Vec<String>,
}

/// A sign-in into an account's config home (`D194`) that has been prepared and not yet
/// finished: what to run, and which account it is for.
///
/// Held by the coordinator against the pane it opened, because the answer to "did this sign
/// anyone in" is how that pane's process exits.
pub struct PendingLogin {
    /// The harness being signed in, for the message that reports the outcome.
    pub agent_type: String,
    /// The account whose config home this login signs in. One home per `(account, harness)`,
    /// so this — not a definition — is what the sign-in is keyed by.
    pub account: String,
    /// That home: where the login runs and where the harness keeps it.
    home: PathBuf,
    /// The launch to spawn under a pseudo-terminal, confined when runs are.
    launch: Launch,
    /// The login process's exit code, written by the pane's reaper before the window hears the
    /// pane ended — so it is there by the time the pane is closed and the outcome is read.
    exit: std::sync::Arc<std::sync::OnceLock<i32>>,
}

impl PendingLogin {
    /// What to spawn.
    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// Where the login runs.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where the pane's reaper notes the login's exit code — see `pty::reap_noting`.
    pub fn exit_slot(&self) -> std::sync::Arc<std::sync::OnceLock<i32>> {
        self.exit.clone()
    }

    /// How the login process exited, once it has. `None` is a login closed while still
    /// running, which is an abandoned one.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit.get().copied()
    }

    /// A fixture sign-in into `account`'s home that exited with `code` (`None`: still
    /// running when its pane closed), for `coordinator`'s own tests of `login_gone`.
    #[cfg(test)]
    pub(crate) fn for_home_test(
        agent_type: impl Into<String>,
        account: impl Into<String>,
        code: Option<i32>,
    ) -> Self {
        let pending = Self {
            agent_type: agent_type.into(),
            account: account.into(),
            home: PathBuf::new(),
            launch: Launch::default(),
            exit: Default::default(),
        };
        if let Some(code) = code {
            let _ = pending.exit.set(code);
        }
        pending
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

    /// The account this run resolved to, when a definition named one. An id, never a
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
                    acp: harness.io_support().acp,
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
                    quota: quota_source(harness.io_support().quota),
                    shares_home: harness.shares_home(),
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
    /// mean an account written by another process stayed invisible until a restart.
    fn account_store(&self) -> FsAccountStore {
        FsAccountStore::new(self.root.join("accounts"))
    }

    /// The harness config homes, keyed by account (`D194`), under Ubiq's own root.
    ///
    /// Built per call for the reason [`account_store`](Self::account_store) is: a home is a
    /// directory another process may have signed in or signed out since this one started, and a
    /// held store would answer from the world as it was.
    pub(crate) fn home_store(&self) -> HomeStore {
        HomeStore::new(self.root.join("harness-homes"))
    }

    /// Whether `agent_type` states a limit at all, and by what route. `None` for an id that names
    /// no harness, which is the same refusal [`Self::is_agent_type`] tells apart.
    ///
    /// Asked before anything is spawned and before anything is probed, so a harness whose
    /// provider publishes nothing costs no network call.
    pub fn quota_source(&self, agent_type: &str) -> Option<ubiq_proto::quota::QuotaSource> {
        harness::resolve(agent_type).map(|harness| quota_source(harness.io_support().quota))
    }

    /// How much of `account`'s plan is left under `agent_type`, asked of the provider now.
    ///
    /// A blocking network call: everything that asks does so from a thread of its own, which is
    /// why the work is [`quota_of`] and this is the thin `&self` face of it.
    pub fn quota(
        &self,
        account: &str,
        agent_type: &str,
    ) -> Result<ubiq_proto::quota::QuotaSnapshot> {
        quota_of(&self.root, account, agent_type)
    }

    /// Every account Ubiq knows, each with the harnesses it is actually signed in to.
    ///
    /// Which harnesses an account serves is *derived*, not recorded (`D194`): an account keys a
    /// home per harness, and a harness is signed in when the login files it names itself are
    /// present there. So one account serves several harnesses without saying so anywhere, and a
    /// sign-in that never finished reports the harness it did not cover.
    ///
    /// References only: the harness ids, never a path and never a token — the whole of what the
    /// interface is allowed to learn about a login.
    pub fn accounts(&self) -> Result<Vec<AccountInfo>> {
        let homes = self.home_store();
        let harnesses = harness::all();
        Ok(self
            .account_store()
            .accounts()
            .context("reading the accounts Ubiq knows")?
            .into_iter()
            .map(|account| AccountInfo {
                logged_in: harnesses
                    .iter()
                    .filter(|harness| homes.signed_in(harness.as_ref(), &account.id))
                    .map(|harness| harness.id())
                    .collect(),
                id: account.id,
            })
            .collect())
    }

    /// Write down that a sign-in into `account`'s home for `agent_type` finished: the marker in
    /// the home, then the account record.
    ///
    /// **The marker is what makes the account read as signed in**, because the credential itself
    /// is wherever the harness decided to keep it — on macOS, for Claude Code, the Keychain and
    /// not the home. Without it `accounts()` reports an account signed in to nothing, and every
    /// picker that offers a harness-and-identity pair has nothing to offer (`HomeStore::signed_in`).
    pub fn record_sign_in(&self, agent_type: &str, account: &str) -> Result<()> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        self.home_store()
            .mark_signed_in(account, &harness.id())
            .with_context(|| format!("recording the {agent_type} sign-in of '{account}'"))?;
        self.record_account(account)
    }

    /// Write a bare record for `account` if nothing on disk answers to that name yet.
    ///
    /// A clean sign-in is the one thing that makes an account (`D194`): the home it signed in is
    /// keyed by a name that until now was only typed into a dialog, and a name with no record is
    /// a name no definition can pick from a list. The record holds nothing but the id —
    /// credential *references* are the user's to add afterwards, and credential material never.
    /// An account that is already there is left exactly as it is.
    pub fn record_account(&self, account: &str) -> Result<()> {
        let store = self.account_store();
        if store
            .account(account)
            .with_context(|| format!("looking for account '{account}'"))?
            .is_some()
        {
            return Ok(());
        }
        store
            .save(&Account {
                id: account.to_string(),
                ..Default::default()
            })
            .with_context(|| format!("recording account '{account}'"))?;
        Ok(())
    }

    /// Whether `account`'s home for `agent_type` holds a usable, current login, as the login
    /// itself claims. An unknown harness, an unusable account name or a home with nothing in it
    /// is [`LoginStatus::Missing`], not an error — a check that finds nothing is an answer.
    ///
    /// **A home a sign-in finished in reads `Unknown`, never `Missing`.** On macOS Claude Code
    /// keeps its login in the Keychain rather than in the home, so a home that is signed in can
    /// hold no readable credential at all; calling that "missing" would tell the user they are
    /// signed out of an account that works. `HomeStore::marked_signed_in` is the fact that
    /// outranks the empty reading, and `Unknown` is the honest word for it: signed in, with an
    /// expiry this cannot see.
    ///
    /// The read is in place, in the home the harness owns: nothing is copied out and nothing is
    /// written (`G380`'s sibling, `G382`).
    pub fn check_login(&self, agent_type: &str, account: &str, now_ms: i64) -> LoginStatus {
        let Some(harness) = harness::resolve(agent_type) else {
            return LoginStatus::Missing;
        };
        let homes = self.home_store();
        let Some(home) = homes.home(account, &harness.id()) else {
            return LoginStatus::Missing;
        };
        match login_validity(harness.as_ref(), &home, now_ms) {
            Validity::Empty if homes.marked_signed_in(account, &harness.id()) => {
                LoginStatus::Unknown
            }
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

    /// Sign one harness out of `account`: its home goes, and with it the login the harness kept
    /// there. The account and its other harnesses' homes are untouched — that is what makes this
    /// different from [`delete_account`](Self::delete_account).
    ///
    /// Ubiq names no credential file here: removing the home removes whatever the harness put in
    /// it, which is the only way to sign out without learning a harness's layout.
    pub fn delete_harness_login(&self, agent_type: &str, account: &str) -> Result<()> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        self.home_store()
            .forget(account, &harness.id())
            .with_context(|| format!("signing '{account}' out of {agent_type}"))
    }

    /// The global definition store, over Ubiq's own root. Built per call, for the same reason
    /// [`account_store`](Self::account_store) is.
    fn definition_store(&self) -> FsProfileStore {
        FsProfileStore::new(definitions_dir(&self.root))
    }

    /// One project's own definition store, under the directory that project already owns.
    ///
    /// A project definition **is** its location (decision 8): the scope cannot contradict the
    /// record, and `Projects::forget` removing `<root>/projects/<id>` takes these with it
    /// without knowing they are there — the same way it already takes the plans and the index.
    fn project_definition_store(&self, project: ProjectId) -> FsProfileStore {
        FsProfileStore::new(
            crate::store::project_dir::ProjectData::under_config(&self.root, project).definitions(),
        )
    }

    /// Every global definition Ubiq knows: a saved setup, flattened to the four references the
    /// interface shows. A definition with no harness pin is skipped — the interface offers
    /// definitions per harness row, and one that names none belongs to no row.
    pub fn definitions(&self) -> Result<Vec<AgentDefinition>> {
        let store = self.definition_store();
        Ok(Self::infos(&store, None).context("reading the definitions Ubiq knows")?)
    }

    /// One project's own definitions, each carrying the project it is scoped to. Empty for a
    /// project that has none, which is every project until one is written there.
    pub fn project_definitions(&self, project: ProjectId) -> Result<Vec<AgentDefinition>> {
        let store = self.project_definition_store(project);
        Self::infos(&store, Some(project))
            .with_context(|| format!("reading the definitions of project {project}"))
    }

    /// One store's definitions as the interface is told them, stamped with the scope they were
    /// found in. The one place a library `Profile` record becomes an [`AgentDefinition`], so the
    /// two scopes cannot drift into two answers.
    fn infos(store: &FsProfileStore, project: Option<ProjectId>) -> Result<Vec<AgentDefinition>> {
        Ok(store
            .profiles()?
            .into_iter()
            .filter_map(|record| {
                Some(AgentDefinition {
                    id: record.id,
                    description: record.description,
                    agent_type: record.harness?,
                    account: record.account,
                    model: record.defaults.model,
                    mode: record.mode,
                    thinking: record.defaults.thinking,
                    max_subagents: record.max_subagents,
                    prompt: record.defaults.prompt,
                    // "This definition mentioned nothing" and "this definition picked none" are the
                    // same row in a checklist, so the interface is told the empty list for both.
                    // The distinction still matters on disk — see [`save_definition`](Self::save_definition).
                    mcps: record.defaults.mcps.unwrap_or_default(),
                    skills: record.defaults.skills.unwrap_or_default(),
                    mission_assistant: record.mission_assistant,
                    mission_coordinator: record.mission_coordinator.unwrap_or(false),
                    mission_worker: record.mission_worker.unwrap_or(false),
                    disabled: record.disabled.unwrap_or(false),
                    project,
                })
            })
            .collect())
    }

    /// One definition's description, scoped the same way [`Self::definition_mcps`] is: a
    /// project's own definition of `id` answers before the global one.
    ///
    /// A standalone function rather than a method, and taking just the config root rather than a
    /// whole `&Agents`, because this is what lets a caller with no `Agents` at all — the mission
    /// MCP listener's own thread, which the coordinator's `Agents` never leaves (`D120`'s own
    /// reasoning: reading it must not need the coordinator's thread) — resolve a definition's
    /// description. The read is a fresh `FsProfileStore` lookup, exactly as [`Self::definitions`]
    /// itself is, so there is no coordinator state to share, only a path.
    pub fn definition_description(
        root: &Path,
        id: &str,
        project: Option<ProjectId>,
    ) -> Option<String> {
        let global = FsProfileStore::new(definitions_dir(root));
        let scoped = project.map(|project| {
            FsProfileStore::new(
                crate::store::project_dir::ProjectData::under_config(root, project).definitions(),
            )
        });
        ScopedProfileStore::new(&global, scoped.as_ref().map(|it| it as &dyn ProfileStore))
            .profile(id)
            .ok()
            .flatten()
            .and_then(|record| record.description)
    }

    /// What a definition saved under `defaults.mcps`, or `None` when it mentioned none — the
    /// distinction the library's own merge turns on, kept rather than flattened to a list.
    ///
    /// Resolved through the run's own scope: inside a project, that project's definition of this
    /// name answers before the global one, exactly as it will when the run is composed.
    ///
    /// The named definition's own row, not its inheritance chain: Ubiq writes no parent, and
    /// flattening one here would be this module holding a second answer to a question
    /// `resolve` already answers.
    fn definition_mcps(&self, id: &str, project: Option<ProjectId>) -> Option<Vec<String>> {
        let global = self.definition_store();
        let scoped = project.map(|project| self.project_definition_store(project));
        ScopedProfileStore::new(&global, scoped.as_ref().map(|it| it as &dyn ProfileStore))
            .profile(id)
            .ok()
            .flatten()
            .and_then(|record| record.defaults.mcps)
    }

    /// Write a definition, creating it when its id names none. Overwrites in place: a saved
    /// setup is edited, not versioned.
    ///
    /// An empty pick is written as `None` rather than as an empty list, because
    /// [`ProfileDefaults`] draws a real distinction between them: `None` is "this definition did not
    /// mention MCP servers", which lets a parent definition's — or the library's own — answer stand,
    /// and `Some([])` is "this definition says none", which overrides one. A checklist with nothing
    /// ticked is the first of those. Nothing in Ubiq's own interface can express the second, and
    /// inventing it here would mean every definition saved through this window silently overriding a
    /// default it was never shown.
    /// Which root it lands in is [`AgentDefinition::project`]: the global one when absent, that
    /// project's own when present. The two are separate namespaces, so saving `review` into a
    /// project never overwrites the global `review` — it shadows it, inside that project.
    ///
    /// Two rules are enforced here rather than in a screen, so they hold however the definition
    /// was edited:
    ///
    /// - **A role flag re-asserts its MCP servers.** `mission_coordinator` and `mission_worker`
    ///   each imply a set ([`role_mcps`]), and the set is merged back in on every save — a user
    ///   who unticks one of them by hand gets it back with the flag still on, because the flag is
    ///   what the role means and a half-equipped coordinator is a broken one.
    /// - **A new definition needs a harness.** With no harness available on this machine there is
    ///   nothing a definition could name, so creating one is refused rather than left to a hidden
    ///   button. Editing an existing one is not refused: a machine that lost its harness must
    ///   still be able to repair what it already wrote.
    pub fn save_definition(&self, mut definition: AgentDefinition) -> Result<()> {
        let store = match definition.project {
            Some(project) => self.project_definition_store(project),
            None => self.definition_store(),
        };
        let exists = store.profile(&definition.id).unwrap_or(None).is_some();
        may_write_definition(exists, self.has_available_harness())?;
        apply_role_mcps(&mut definition);
        let record = DefinitionRecord {
            id: definition.id,
            description: definition.description,
            harness: Some(definition.agent_type),
            account: definition.account,
            defaults: ProfileDefaults {
                model: definition.model,
                thinking: definition.thinking,
                prompt: definition.prompt,
                mcps: (!definition.mcps.is_empty()).then_some(definition.mcps),
                skills: (!definition.skills.is_empty()).then_some(definition.skills),
                ..Default::default()
            },
            mode: definition.mode,
            max_subagents: definition.max_subagents,
            mission_assistant: definition.mission_assistant,
            mission_coordinator: definition.mission_coordinator.then_some(true),
            mission_worker: definition.mission_worker.then_some(true),
            disabled: definition.disabled.then_some(true),
            ..Default::default()
        };
        store
            .save(&record)
            .with_context(|| format!("saving agent definition '{}'", record.id))?;
        Ok(())
    }

    /// Copy a definition under a new name, inside the scope it already lives in.
    ///
    /// The copy is of the *record*, not of what the interface was shown, so a field no screen
    /// draws is carried across too. Refused when the source is not there or the name is taken:
    /// a clone that overwrites is a delete nobody asked for.
    pub fn clone_definition(
        &self,
        id: &str,
        new_id: &str,
        project: Option<ProjectId>,
    ) -> Result<()> {
        let store = match project {
            Some(project) => self.project_definition_store(project),
            None => self.definition_store(),
        };
        let Some(mut record) = store
            .profile(id)
            .with_context(|| format!("reading agent definition '{id}'"))?
        else {
            bail!("there is no agent definition called '{id}'");
        };
        if store.profile(new_id).unwrap_or(None).is_some() {
            bail!("an agent definition called '{new_id}' already exists");
        }
        record.id = new_id.to_string();
        store
            .save(&record)
            .with_context(|| format!("saving agent definition '{new_id}'"))?;
        Ok(())
    }

    /// Whether this machine has a harness a definition could name at all — its binary found, or
    /// its command overridden in settings.
    pub fn has_available_harness(&self) -> bool {
        self.types().iter().any(|it| it.available)
    }

    /// Write the three definitions a fresh install starts with, if and only if the global root
    /// holds none and there is a harness to pin them to.
    ///
    /// The three are the roles Ubiq's own MCP servers are split along — a mission's coordinator,
    /// a mission's worker, and a helper that reads Ubiq's documentation — so a new install has a
    /// working agent for each without anyone assembling a server list by hand. Only ever a
    /// seeding: once one definition exists, the user's list is theirs, and deleting all three
    /// back to nothing is not an invitation to write them again on the next start.
    pub fn ensure_default_definitions(&self) -> Result<bool> {
        let store = self.definition_store();
        if !store.profiles().unwrap_or_default().is_empty() {
            return Ok(false);
        }
        let Some(harness) = self
            .types()
            .into_iter()
            .find(|it| it.available)
            .map(|it| it.id)
        else {
            return Ok(false);
        };
        for (id, description, coordinator, worker, mcps) in [
            (
                "Coordinator",
                "Runs a mission end to end: writes the plan, breaks it into tasks and hands \
                 them to workers, then tracks progress to completion. Carries ubiq-mission \
                 (mission lifecycle), ubiq-plan (the plan document), manage-ubiq-tasks (create, \
                 assign and update tasks) and ubiq-kb (the shared knowledge base).",
                true,
                false,
                Vec::new(),
            ),
            (
                "Ubiq helper",
                "Answers questions about Ubiq itself — its features, screens and settings — by \
                 reading Ubiq's own documentation. Carries ubiq-help.",
                false,
                false,
                vec![catalogue::UBIQ_HELP.to_string()],
            ),
            (
                "Worker",
                "Works one task at a time inside a mission: reads the mission and the task it \
                 was assigned, reports progress, and looks up project facts as needed. Carries \
                 use-mission (read the mission), use-task (read and update the assigned task), \
                 project-info (facts about the project) and ubiq-kb (the shared knowledge base).",
                false,
                true,
                Vec::new(),
            ),
        ] {
            self.save_definition(AgentDefinition {
                id: id.to_string(),
                description: Some(description.to_string()),
                agent_type: harness.clone(),
                account: None,
                model: None,
                mode: None,
                thinking: None,
                max_subagents: None,
                prompt: None,
                mcps,
                skills: Vec::new(),
                mission_assistant: None,
                mission_coordinator: coordinator,
                mission_worker: worker,
                disabled: false,
                project: None,
            })
            .with_context(|| format!("writing the default agent definition '{id}'"))?;
        }
        Ok(true)
    }

    /// Rename an account, and move every harness home keyed by it (`D194`), so the logins stay
    /// with the identity they were made for. The account store validates the new name; this only
    /// forwards what it decides.
    ///
    /// The homes move **first**. A record renamed over homes that did not move is an account
    /// whose logins have all silently vanished, and there is no second name to find them under;
    /// a home moved under a record that then failed to rename is reported here, with both names
    /// in the sentence, and is one `rename` back.
    pub fn rename_account(&self, from: &str, to: &str) -> Result<()> {
        let homes = self.home_store();
        homes
            .rename(from, to)
            .with_context(|| format!("moving the harness homes of '{from}' to '{to}'"))?;
        if let Err(error) = self.account_store().rename_account(from, to) {
            // Put the homes back, so the account and its logins still agree under one name.
            let _ = homes.rename(to, from);
            return Err(error);
        }
        Ok(())
    }

    /// Delete an account, and with it every harness home keyed by it — the logins go where the
    /// identity goes (`D194`).
    ///
    /// The record goes first: a record that will not delete leaves the homes where the account
    /// that still names them can reach them, and homes that will not delete leave no record
    /// pointing at them for the user to worry about.
    pub fn delete_account(&self, id: &str) -> Result<()> {
        self.account_store().delete_account(id)?;
        self.home_store()
            .delete(id)
            .with_context(|| format!("removing the harness homes of '{id}'"))
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

    /// This machine's own variables, added to a launch the harness has already described.
    /// Never over a name the harness itself set: a confined launch's `env` is the whole
    /// environment, and `CLAUDE_CONFIG_DIR` and its siblings are what pin a run — or a
    /// sign-in — to the directory it was composed for.
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

    /// What signing an account's config home in has to run (`D194`): the harness's own login,
    /// straight into the one home every run as that account reads — the library's `login_home`,
    /// after `prepare_home` has made the home ready. Nothing is captured or read back; the
    /// harness keeps the login there and refreshes it, and whether it worked is how the process
    /// exits.
    ///
    /// The account need not exist yet. It is the sign-in that makes one: a clean exit is what
    /// writes the record ([`record_account`](Self::record_account)), so the name typed into the
    /// dialog becomes an identity a definition can pick only once there is a login behind it.
    ///
    /// Confined exactly when a run would be, and under the policy a run from this home gets —
    /// granted the home **read-write**, which is the whole point: the login has to land where a
    /// run of this machine looks for it, and on macOS that is also the Keychain item the harness
    /// keys to the home.
    pub fn begin_home_login(&self, agent_type: &str, account: &str) -> Result<PendingLogin> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        if !harness.shares_home() {
            bail!(
                "{} does not run from a config home of its own, so there is nothing to sign in",
                harness.display_name()
            );
        }
        if account.trim().is_empty() {
            bail!("a sign-in needs an account name: the home is keyed by it");
        }
        let home = self
            .home_store()
            .home(account, &harness.id())
            .ok_or_else(|| anyhow!("'{account}' is not a name a config home can be keyed by"))?;
        let templates = harness::FsTemplateStore::new(self.root.join("harness-templates"));
        provision::prepare_home(harness.as_ref(), &home, &templates)
            .with_context(|| format!("preparing the {agent_type} home of '{account}'"))?;
        let mut launch = harness
            .login_home(&home)
            .with_context(|| format!("asking {agent_type} how it signs in"))?;
        self.resolve_program(agent_type, &mut launch);
        self.add_machine_env(&mut launch);

        if self.isolate {
            let mut spec = agent_manager::spec::RunSpec::new(harness.id(), home.clone());
            spec.isolation = Isolation::Sandboxed(String::new());
            // A run from this home, not a per-run dir: its Keychain item is reachable.
            spec.config = ConfigStrategy::Home {
                home: home.clone(),
                scratch: home.clone(),
            };
            // Read-write on the home, exactly as an ordinary run from it gets — a login that
            // cannot write the home signs nobody in.
            let mut options = self.isolate_options();
            options.grant_config_home(harness.as_ref(), &spec.config);
            let confined = isolate::plan(&launch, &spec, &home, &options).with_context(|| {
                format!("resolving the policy a {agent_type} sign-in runs under")
            })?;
            if let Some(confined) = confined {
                launch = isolate::confined_launch(&confined)
                    .with_context(|| format!("preparing a confined {agent_type} sign-in"))?;
            }
        }

        Ok(PendingLogin {
            agent_type: agent_type.to_string(),
            account: account.to_string(),
            home,
            launch,
            exit: Default::default(),
        })
    }

    /// Compose the run for `pane`: provision the harness's configuration into a
    /// directory named by that pane, and resolve the policy it runs under.
    ///
    /// The directory is Ubiq's rather than the library's own ephemeral choice,
    /// because a pane's run belongs to the pane: it is named by it, it is found
    /// again after a crash, and [`retire`](Self::retire) deletes it when the
    /// pane closes. It is the whole run's configuration, or — for a harness
    /// that runs from a shared home — the scratch beside that home (`run_config`).
    pub fn compose(
        &self,
        pane: PaneId,
        agent_type: &str,
        cwd: &Path,
        args: Vec<String>,
        options: ConverseOptions,
    ) -> Result<Composed> {
        // A pane's run takes the same picks a conversation's does — the new-pane menu and a
        // shell row still hand it `ConverseOptions::default()`, but a pane started with an
        // account, definition, model or MCP list resolves exactly what was named.
        self.compose_run(
            &pane.to_string(),
            agent_type,
            cwd,
            args,
            IoModes::Passthrough,
            options,
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
        // What the bridge spawns is what a pane would spawn: the harness under its policy when
        // the run is confined, the harness itself when it is not. Resolving it here rather than
        // at composition is what materializes the run's home.
        composed.provisioned.launch = composed.exec()?;

        let bridge = harness
            .structured_bridge(&composed.provisioned, cwd)
            .with_context(|| format!("starting a {agent_type} conversation"))?;
        Ok((composed, bridge))
    }

    /// A definition id as a single path segment isol8 will accept for a managed home: no
    /// separator, no `..`, nothing empty.
    ///
    /// The library's CLI keeps its own copy of this. Sharing one would mean exporting a name
    /// across a feature gate the `cli` module sits behind, for four lines.
    fn home_id(definition: &str) -> String {
        let cleaned: String = definition
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
        // A definition's `defaults.mcps` is read by `resolve` as a list of ids in the **on-disk
        // catalog**, and an id not found there is dropped to `RunSpec::problems`. Ubiq's
        // built-ins are in no catalog: they are this process, on a port it bound at startup. So
        // the definition's list is split here — the names this build answers are lifted out and
        // injected below, and only the rest is handed back down as `flags.mcps`, which outranks
        // the definition in the library's own merge. A `--mcp-as-skill` flag naming an id outside
        // the run's injected set and `--safe` naming a missing preset still hard-fail: both are
        // flag misuse, not a stale saved reference.
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
            .definition
            .as_deref()
            .and_then(|definition| self.definition_mcps(definition, options.project))
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

        // What a run is composed *of* — its account, its model, the definition that names
        // them — is the library's question, and `resolve` is the one place that answers it.
        // Ubiq builds no `RunSpec` of its own beyond the three fields below, so an account
        // named in a definition reaches a pane without this module learning what an account is.
        //
        // The library's own settings file is deliberately not read: Ubiq's settings are the
        // settings surface, and a second file answering the same question is a second
        // answer. That leaves `resolve`'s precedence as flags, then the definition.
        let flags = resolve::RunFlags {
            harness: harness.id(),
            cwd: cwd.to_path_buf(),
            passthrough_args: args,
            // Highest precedence in `resolve`, which is what "the user picked this one" has to
            // mean: an identity — or a model, a thinking level, a mode — chosen when the
            // conversation started outranks the definition's.
            account: options.account,
            model: options.model,
            thinking: options.thinking,
            permission_mode: options.mode,
            // The saved setup the picks sit on top of. `None` is a bare start, and the
            // library then falls back to whatever definition it resolves on its own.
            profile: options.definition,
            // Whatever the definition named that Ubiq does not answer itself, passed back as a flag
            // so the built-ins never reach the catalog lookup. `None` when the definition mentioned
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
        // Two definition stores, read as one: this project's own over the global root. Which of the
        // two a name resolves in is the whole of what "project-scoped" means — the record says
        // nothing about a project, its location does. `None` for a run belonging to no project
        // is the global root alone, which is every run's answer before and after this.
        //
        // The library resolves the `extends` chain through the same pair, so a project definition
        // may specialise a global one, and a global definition naming a project's is refused there
        // rather than here (`agent_manager::profile::resolve_chain`).
        let global_definitions = self.definition_store();
        let project_definitions = options
            .project
            .map(|project| self.project_definition_store(project));
        let definitions = ScopedProfileStore::new(
            &global_definitions,
            project_definitions
                .as_ref()
                .map(|it| it as &dyn ProfileStore),
        );
        let mut spec = resolve::resolve(
            &flags,
            &Settings::default(),
            &FsRegistry::new(self.root.join("catalog")),
            &FsAccountStore::new(self.root.join("accounts")),
            &definitions,
        )
        .with_context(|| format!("composing a {agent_type} run"))?;
        // The account `resolve` actually landed on — the one this run picked, or the definition's,
        // or none — because under `D194` that is what the config home is keyed by. Read off the
        // spec rather than re-derived: an account id nothing answers to has already degraded to
        // `None` here, and a run that resolved no account runs on the user's own config.
        let account = spec.account.as_ref().map(|it| it.id.clone());

        // The five answers that are Ubiq's rather than the library's: which directory this
        // run's configuration lives in, which face it wears, whether it is confined, — when
        // it is — that it asks nothing, and which of Ubiq's own MCP servers it can reach.
        // The isolation replaces whatever a definition asked for,
        // because the toggle belongs to Ubiq's own settings and applies to both faces alike.
        let structured = io == IoModes::Structured;
        spec.config = run_config(
            harness.as_ref(),
            account.as_deref(),
            &self.home_store(),
            self.run_dir_for(key),
        );
        let shared_home = matches!(
            spec.config,
            ConfigStrategy::Home { .. } | ConfigStrategy::Native { .. }
        );
        spec.io = io;
        spec.isolation = if self.isolate {
            Isolation::Sandboxed(String::new())
        } else {
            Isolation::None
        };
        // A confined run is contained by the sandbox, not by the prompts, so it launches with
        // permissions bypassed — otherwise every step stops on an ask the sandbox already
        // answered. Which mode that *is* stays the harness's own word (`unattended_mode`);
        // Ubiq names none. An explicit pick for this run outranks it: the definition's mode does
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
        // own was once the answer to "a second run of the same definition should find its caches,
        // its indexes and its logins where it left them" — but the real home is already where
        // those are, and a replaced one aims every toolchain layer's `~/.cargo`-shaped grant at
        // an empty directory, so the agent could not build. What stays per-run is the
        // configuration, which `CLAUDE_CONFIG_DIR` and its siblings pin to the run dir.
        //
        // The home and the extra grants are the two answers Ubiq holds rather than the library:
        // both are settings a person set on this machine, and the library has no way to ask.
        let mut options = self.isolate_options();
        options.home = home_mode(&self.home);
        // The account's home — or, with none, the harness's own — is where the harness keeps
        // its login and its sessions; the policy grants the run's own scratch dir, not that.
        options.grant_config_home(harness.as_ref(), &spec.config);

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
        // A run from a shared home records which one, so the teardown archives only this run's
        // session from it (`D193`).
        meta.config = shared_home.then(|| spec.config.clone());
        let _ = session::save(&self.sessions_dir(), &meta);

        tracing::info!(
            run = %key,
            dir = %provisioned.dir.display(),
            account = spec.account.as_ref().map(|a| a.id.as_str()).unwrap_or("<none>"),
            harness = %agent_type,
            "compose_run: provisioned a run"
        );

        Ok(Composed {
            launch: provisioned.launch.clone(),
            confined,
            dir: provisioned.dir.clone(),
            provisioned,
            spec_account: spec.account.as_ref().map(|a| a.id.clone()),
            problems: spec.problems.clone(),
        })
    }

    /// Ubiq's own session store: one directory per run, under Ubiq's config
    /// root. Always passed explicitly, never resolved through
    /// `session::sessions_root` — `AM_SESSIONS` must not be able to redirect a
    /// user's Ubiq transcripts into the library's store.
    fn sessions_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Copy the harness's own record of the conversation out of a run directory when that run
    /// ends — whether or not the directory is then deleted.
    ///
    /// Which files those are is the harness's answer, not Ubiq's — a path
    /// literal here would be the boundary this module's header names. Entirely
    /// best effort: this runs on teardown paths, and no session that cannot be
    /// archived is a reason to fail a close. A run with no meta is a plain
    /// shell pane, which is the common case rather than an error.
    fn archive(&self, key: &str) {
        let sessions = self.sessions_dir();
        let Ok(mut meta) = session::load(&sessions, key) else {
            return;
        };
        let Some(harness) = harness::resolve(&meta.harness) else {
            return;
        };

        let dest = sessions.join(key).join("harness");
        // A home the harness owns holds every run's sessions, so only this run's own is taken —
        // the one the harness named, and nothing when it never named one.
        let transcripts = match &meta.config {
            Some(ConfigStrategy::Home { home, .. }) => {
                session_transcripts(harness.as_ref(), Some(home), &meta)
            }
            Some(ConfigStrategy::Native { .. }) => {
                session_transcripts(harness.as_ref(), None, &meta)
            }
            _ => harness.transcripts(&self.run_dir_for(key)),
        };
        for src in transcripts {
            let Some(name) = src.file_name() else {
                continue;
            };
            let _ = std::fs::create_dir_all(&dest);
            // A session's companion directory (its subagents' transcripts) comes whole.
            if src.is_dir() {
                let _ = copy_tree(&src, &dest.join(name), Path::new(""));
            } else {
                let _ = std::fs::copy(&src, dest.join(name));
            }
        }

        meta.finished_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        let _ = session::save(&sessions, &meta);
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

    /// End an agent's run but keep its directory: the process is gone, the
    /// conversation may come back.
    ///
    /// The run directory *is* the harness's session store — Claude's
    /// `projects/<slug>/*.jsonl`, Codex's rollouts, opencode's data dir — so
    /// keeping it is the whole of what lets a resume find the conversation
    /// again.
    pub fn park_agent(&self, agent: AgentId) {
        let key = agent.to_string();
        tracing::info!(
            run = %key,
            dir = %self.agent_dir(agent).display(),
            "park_agent: conversation ended; keeping its run directory"
        );
        self.archive(&key);
        // Its process has gone, so the MCP listener has nobody left to answer for at that
        // address. A parked conversation registers again when it is revived.
        self.mcp_agents.forget(&key);
    }

    /// Copy one agent's run directory onto another's, so the second resumes
    /// from the first's conversation instead of starting empty.
    ///
    /// `sessions/` is excluded: [`compose_run`](Self::compose_run) writes the fork's
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
        copy_tree(
            &self.run_dir_for(&key),
            &self.run_dir_for(&to.to_string()),
            Path::new(""),
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
        let dir = self.run_dir(pane);
        self.archive(&pane.to_string());
        self.mcp_agents.forget(&pane.to_string());
        if std::fs::remove_dir_all(&dir).is_ok() {
            tracing::info!(
                pane = %pane,
                dir = %dir.display(),
                "retire: pane gone; removed its run directory"
            );
        }
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
    /// conversation can be resumed at all.
    pub fn sweep(&self) {
        let runs = self.root.join("runs");
        let Ok(entries) = std::fs::read_dir(&runs) else {
            return;
        };
        let mut removed = 0usize;
        let mut parked = 0usize;
        for entry in entries.flatten() {
            // A crashed run is where the record matters most, and its meta was
            // written when the run was composed, so this works verbatim here.
            let key = entry.file_name().to_string_lossy().into_owned();
            self.archive(&key);
            if self.is_persistent(&key) {
                parked += 1;
                continue;
            }
            if std::fs::remove_dir_all(entry.path()).is_ok() {
                removed += 1;
            }
        }
        tracing::info!(
            removed,
            parked,
            "sweep: startup sweep of leftover run directories from a previous process"
        );
    }

    /// Where a pane's run is provisioned. One directory per pane, named by it.
    fn run_dir(&self, pane: PaneId) -> PathBuf {
        self.run_dir_for(&pane.to_string())
    }

    /// Where an agent's conversation is provisioned.
    pub fn agent_dir(&self, agent: AgentId) -> PathBuf {
        self.run_dir_for(&agent.to_string())
    }

    /// Remove what an agent's conversation left behind: its run directory, and only that. A
    /// run from an account's home (`D194`) leaves the home alone — the run directory is its
    /// scratch, beside the home and never above it.
    pub fn retire_agent(&self, agent: AgentId) {
        let dir = self.agent_dir(agent);
        self.archive(&agent.to_string());
        self.mcp_agents.forget(&agent.to_string());
        if std::fs::remove_dir_all(&dir).is_ok() {
            tracing::info!(
                agent = %agent,
                dir = %dir.display(),
                "retire_agent: conversation ended; removed its run directory"
            );
        }
    }

    /// One directory per run, named by whatever owns it — a pane or an agent.
    /// Both ids are ULIDs, so neither can be mistaken for the other's.
    fn run_dir_for(&self, key: &str) -> PathBuf {
        self.root.join("runs").join(key)
    }
}

/// The files `harness` wrote for this run's one session in a shared config home — `home`, or
/// the user's own when `None`. Empty when the harness never named its session, or cannot name
/// one session's files.
fn session_transcripts(
    harness: &dyn harness::Harness,
    home: Option<&Path>,
    meta: &session::SessionMeta,
) -> Vec<PathBuf> {
    meta.harness_session_id
        .as_deref()
        .and_then(|id| harness.session_transcripts(home, &meta.cwd, id))
        .unwrap_or_default()
}

/// Where a run's configuration lives (`D194`), for a harness that can run from a shared home:
/// the **account's** own home when the run resolved one and its name can key a home, and
/// otherwise the user's own config in place, every per-run file in `scratch` either way —
/// confined or not, since the sandbox is granted that home
/// ([`IsolateOptions::grant_config_home`]). Only a harness that cannot share a home keeps the
/// per-run directory.
///
/// Keyed by account rather than by definition because one login per identity is what a person
/// means by "my work account": two definitions naming the same account share it, concurrently
/// and read-write, and the harness owns the refresh.
fn run_config(
    harness: &dyn harness::Harness,
    account: Option<&str>,
    homes: &HomeStore,
    scratch: PathBuf,
) -> ConfigStrategy {
    if !harness.shares_home() {
        return ConfigStrategy::Fixed(scratch);
    }
    match account.and_then(|account| homes.home(account, &harness.id())) {
        Some(home) => ConfigStrategy::Home { home, scratch },
        None => ConfigStrategy::Native { scratch },
    }
}

/// Copy `src` onto `dst`, recursively, skipping the top-level `sessions/`. `rel` is where in
/// the tree the recursion currently is, so only the top-level one is skipped.
///
/// Here rather than from a crate because std has no recursive copy.
fn copy_tree(src: &Path, dst: &Path, rel: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("creating {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let here = rel.join(entry.file_name());
        if here == Path::new("sessions") {
            continue;
        }
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to, &here)?;
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

    let mut cmd = std::process::Command::new(&program);
    cmd.args(&words)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // The user is only asked for a first line of output, never a screen — a console-subsystem
    // child would otherwise flash a window on Windows, since the app that spawns it has already
    // freed its own console (`ubiq_app::detach_console`).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
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

/// How much of `account`'s plan is left under `agent_type`, read from the provider.
///
/// A free function over the config root rather than a method, for the reason `probe_catalogue`
/// is one: the quota worker has no `&self` to borrow and a probe is a blocking HTTPS call that
/// must not happen on the coordinator's thread.
///
/// The login read is the one the harness keeps for a run of `account` (`D194`, `G380`): that
/// account's own home for this harness, and with none — an empty account, or a name no home can
/// be keyed by — the harness's own default home, which is where a run naming no account runs.
/// Under `D193` this had to hunt for a definition of this harness that named `account`, because
/// the home was the definition's; the account *is* the key now, so the lookup is one call.
///
/// The credential never comes back out: the library reads the token in place, spends it on one
/// request and returns percentages. This stays the one token read `G380` is honest about — it is
/// here because nothing else can say what a plan has left, and it goes as soon as something can.
pub fn quota_of(
    root: &Path,
    account: &str,
    agent_type: &str,
) -> Result<ubiq_proto::quota::QuotaSnapshot> {
    let harness =
        harness::resolve(agent_type).ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
    let home = HomeStore::new(root.join("harness-homes"))
        .home(account, &harness.id())
        .filter(|home| home.is_dir());
    let snapshot = harness
        .quota(account, home.as_deref())
        .with_context(|| format!("asking {agent_type} what '{account}' has left"))?;
    Ok(quota_snapshot(snapshot))
}

/// One ACP agent's `initialize` answer, as the wire spells it.
///
/// Mirrors field for field; see [`quota_source`] for why the mapping is written out rather than
/// derived. `discovered_ms` is left at zero here — the store stamps it, because the store is what
/// knows when the record was written.
pub fn acp_capabilities(
    capabilities: agent_manager::io::AcpCapabilities,
) -> ubiq_proto::acp::AcpCapabilitiesRecord {
    use ubiq_proto::acp as wire;
    wire::AcpCapabilitiesRecord {
        protocol_version: capabilities.protocol_version,
        agent: capabilities
            .agent
            .map(|agent| wire::AcpImplementationRecord {
                name: agent.name,
                title: agent.title,
                version: agent.version,
            }),
        groups: capabilities
            .groups
            .into_iter()
            .map(|group| wire::AcpCapabilityGroupRecord {
                label: group.label,
                entries: group
                    .entries
                    .into_iter()
                    .map(|entry| wire::AcpCapabilityRecord {
                        id: entry.id,
                        label: entry.label,
                        supported: entry.supported,
                        description: entry.description,
                    })
                    .collect(),
            })
            .collect(),
        auth_methods: capabilities
            .auth_methods
            .into_iter()
            .map(|method| wire::AcpAuthMethodRecord {
                id: method.id,
                name: method.name,
                description: method.description,
                default: method.default,
            })
            .collect(),
        discovered_ms: 0,
    }
}

/// The library's quota source, as the wire spells it.
///
/// The two crates do not share a type and never will: `crates/ubiq` does not depend on
/// `agent-manager`, and the host is the only thing that depends on both — the same reason
/// `RateLimitRecord` mirrors the library's `RateLimitWindow`. A `From` impl would have to live
/// in one of the two crates, so the mapping lives here instead.
fn quota_source(source: agent_manager::quota::QuotaSource) -> ubiq_proto::quota::QuotaSource {
    use agent_manager::quota::QuotaSource as Lib;
    use ubiq_proto::quota::QuotaSource as Wire;
    match source {
        Lib::None => Wire::None,
        Lib::Push => Wire::Push,
        Lib::Probe => Wire::Probe,
        Lib::Bridge => Wire::Bridge,
    }
}

/// One snapshot, as the wire spells it. Mirrors field for field; see [`quota_source`] for why
/// the mapping is written out rather than derived.
fn quota_snapshot(
    snapshot: agent_manager::quota::QuotaSnapshot,
) -> ubiq_proto::quota::QuotaSnapshot {
    ubiq_proto::quota::QuotaSnapshot {
        account: snapshot.account,
        harness: snapshot.harness,
        plan: snapshot.plan,
        gauges: snapshot.gauges.into_iter().map(quota_gauge).collect(),
        as_of: snapshot.as_of,
    }
}

/// One gauge, as the wire spells it.
fn quota_gauge(gauge: agent_manager::quota::QuotaGauge) -> ubiq_proto::quota::QuotaGauge {
    ubiq_proto::quota::QuotaGauge {
        label: gauge.label,
        reading: quota_reading(gauge.reading),
        resets_at: gauge.resets_at,
        detail: gauge.detail,
    }
}

/// One reading, as the wire spells it. A variant added to the library must be added here too,
/// which is what the exhaustive match is for.
fn quota_reading(reading: agent_manager::quota::QuotaReading) -> ubiq_proto::quota::QuotaReading {
    use agent_manager::quota::QuotaReading as Lib;
    use ubiq_proto::quota::QuotaReading as Wire;
    match reading {
        Lib::Window { used_pct } => Wire::Window { used_pct },
        Lib::Count { used, limit } => Wire::Count { used, limit },
        Lib::Credit {
            remaining,
            currency,
        } => Wire::Credit {
            remaining,
            currency,
        },
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

    /// A definition written into a project is offered in that project and nowhere else, and a
    /// global-only setup is untouched by the second store existing at all.
    #[test]
    fn a_project_definition_is_listed_in_its_project_and_not_globally() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());
        let project = ProjectId::generate();
        let other = ProjectId::generate();

        let info = |id: &str, project: Option<ProjectId>| AgentDefinition {
            project,
            ..a_definition(id)
        };

        agents.save_definition(info("standard", None)).unwrap();
        agents
            .save_definition(info("reviewer", Some(project)))
            .unwrap();

        let global: Vec<String> = agents
            .definitions()
            .unwrap()
            .into_iter()
            .map(|it| it.id)
            .collect();
        assert_eq!(
            global,
            vec!["standard".to_string()],
            "global list is global"
        );

        let scoped = agents.project_definitions(project).unwrap();
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].id, "reviewer");
        assert_eq!(
            scoped[0].project,
            Some(project),
            "a project definition says which project it is in"
        );
        assert!(
            agents.project_definitions(other).unwrap().is_empty(),
            "another project sees none of it"
        );

        // On disk under the project's own directory, which is what `Projects::forget`
        // removes whole — so forgetting the project takes the definition with it.
        let dir = root
            .path()
            .join("projects")
            .join(project.to_string())
            .join(DEFINITIONS_DIR)
            .join("reviewer");
        assert!(dir.join("profile.toml").is_file());
        std::fs::remove_dir_all(root.path().join("projects").join(project.to_string())).unwrap();
        assert!(agents.project_definitions(project).unwrap().is_empty());
        assert_eq!(
            agents.definitions().unwrap().len(),
            1,
            "the global root stands"
        );
    }

    /// The two stores read as one at composition time: inside a project, that project's definition
    /// of a name answers before the global one; outside it, the global one does. Asserted
    /// through `definition_mcps`, which is the one place `compose_run` asks a definition a question
    /// and needs no harness binary to answer.
    #[test]
    fn a_project_definition_shadows_a_global_one_only_inside_its_project() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());
        let project = ProjectId::generate();
        let other = ProjectId::generate();

        let info = |mcps: &[&str], project: Option<ProjectId>| AgentDefinition {
            mcps: mcps.iter().map(|it| it.to_string()).collect(),
            project,
            ..a_definition("review")
        };

        agents
            .save_definition(info(&["manage-ubiq-tasks"], None))
            .unwrap();
        agents
            .save_definition(info(&["ubiq-plan"], Some(project)))
            .unwrap();

        assert_eq!(
            agents.definition_mcps("review", Some(project)),
            Some(vec!["ubiq-plan".to_string()]),
            "inside the project, the project's definition answers"
        );
        assert_eq!(
            agents.definition_mcps("review", Some(other)),
            Some(vec!["manage-ubiq-tasks".to_string()]),
            "another project sees the global one"
        );
        assert_eq!(
            agents.definition_mcps("review", None),
            Some(vec!["manage-ubiq-tasks".to_string()]),
            "a run in no project sees the global one"
        );
    }

    /// A definition written under the old `profiles/` directory is still the user's definition
    /// after the rename: the directory is moved onto the new name the first time it is asked
    /// for, records and all (`D174`).
    #[test]
    fn a_profiles_directory_written_before_the_rename_is_migrated_in_place() {
        let root = tempfile::TempDir::new().unwrap();
        let legacy = root.path().join("profiles").join("reviewer");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(
            legacy.join("profile.toml"),
            "id = \"reviewer\"\nharness = \"claude-code\"\n",
        )
        .unwrap();

        let agents = Agents::new(root.path(), false);
        let found: Vec<String> = agents
            .definitions()
            .unwrap()
            .into_iter()
            .map(|it| it.id)
            .collect();

        assert_eq!(
            found,
            vec!["reviewer".to_string()],
            "the record came across"
        );
        assert!(
            root.path()
                .join("agent-definitions")
                .join("reviewer")
                .is_dir(),
            "under the new name"
        );
        assert!(
            !root.path().join("profiles").exists(),
            "and the old directory is gone rather than left as a second answer"
        );
    }

    /// The whole point of the two role flags: the servers the role needs come back on save, even
    /// when the definition that arrived had them stripped out by hand.
    #[test]
    fn a_role_flag_re_adds_its_mcp_servers_on_save() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());

        agents
            .save_definition(AgentDefinition {
                mission_coordinator: true,
                mission_worker: true,
                // Ticked off by hand, which is what the flags outrank.
                mcps: vec!["ubiq-help".to_string()],
                ..a_definition("both")
            })
            .unwrap();

        let saved = agents.definitions().unwrap().remove(0);
        assert_eq!(
            saved.mcps.first().map(String::as_str),
            Some("ubiq-help"),
            "what the user picked keeps its place"
        );
        for implied in crate::mcp::catalogue::role_mcps(true, true) {
            assert!(
                saved.mcps.contains(&implied),
                "a coordinator-and-worker definition carries {implied}"
            );
        }
        assert!(saved.mission_coordinator && saved.mission_worker);
    }

    /// `description` round-trips like every other optional field: written, read back verbatim,
    /// and an old record with none stays `None` rather than becoming `Some("")`.
    #[test]
    fn a_description_round_trips_and_an_old_record_still_loads() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());

        agents
            .save_definition(AgentDefinition {
                description: Some("Reviews pull requests against the style guide.".to_string()),
                ..a_definition("reviewer")
            })
            .unwrap();
        agents.save_definition(a_definition("bare")).unwrap();

        let mut saved = agents.definitions().unwrap();
        saved.sort_by(|a, b| a.id.cmp(&b.id));
        let bare = saved.iter().find(|it| it.id == "bare").unwrap();
        let reviewer = saved.iter().find(|it| it.id == "reviewer").unwrap();
        assert_eq!(
            reviewer.description.as_deref(),
            Some("Reviews pull requests against the style guide.")
        );
        assert_eq!(
            bare.description, None,
            "a definition written with no description reads back as None, not Some(\"\")"
        );
    }

    /// `disabled` is a fact the record keeps: written, read back, and not something a save has to
    /// be told twice.
    #[test]
    fn a_disabled_definition_stays_disabled() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());

        agents
            .save_definition(AgentDefinition {
                disabled: true,
                ..a_definition("retired")
            })
            .unwrap();

        assert!(agents.definitions().unwrap()[0].disabled);
    }

    /// A clone is a copy under a new name in the same scope, and it never overwrites: the thing
    /// it would overwrite is the user's own saved setup.
    #[test]
    fn a_clone_copies_the_record_and_refuses_a_name_already_taken() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());
        agents
            .save_definition(AgentDefinition {
                mcps: vec!["ubiq-kb".to_string()],
                mission_worker: true,
                ..a_definition("worker")
            })
            .unwrap();

        agents
            .clone_definition("worker", "worker copy", None)
            .unwrap();

        let mut saved = agents.definitions().unwrap();
        saved.sort_by(|a, b| a.id.cmp(&b.id));
        let copy = saved.iter().find(|it| it.id == "worker copy").unwrap();
        let source = saved.iter().find(|it| it.id == "worker").unwrap();
        assert_eq!(copy.mcps, source.mcps, "the servers came with it");
        assert!(copy.mission_worker, "and so did the role");

        assert!(
            agents.clone_definition("worker", "worker", None).is_err(),
            "a clone never overwrites"
        );
        assert!(
            agents.clone_definition("nobody", "somebody", None).is_err(),
            "and there is nothing to copy from a name that is not there"
        );
    }

    /// The three a fresh install starts with, and only on a fresh install: seeding is what an
    /// empty root gets, never something a later start adds to a list the user has made their own.
    #[test]
    fn the_three_default_definitions_are_written_once_into_an_empty_root() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());

        assert!(
            agents.ensure_default_definitions().unwrap(),
            "a fresh root is seeded"
        );
        let mut names: Vec<String> = agents
            .definitions()
            .unwrap()
            .into_iter()
            .map(|it| it.id)
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "Coordinator".to_string(),
                "Ubiq helper".to_string(),
                "Worker".to_string()
            ]
        );

        let by_id = |id: &str| {
            agents
                .definitions()
                .unwrap()
                .into_iter()
                .find(|it| it.id == id)
                .unwrap()
        };
        assert_eq!(
            by_id("Coordinator").mcps,
            crate::mcp::catalogue::COORDINATOR_MCPS
                .iter()
                .map(|it| it.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            by_id("Worker").mcps,
            crate::mcp::catalogue::WORKER_MCPS
                .iter()
                .map(|it| it.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(by_id("Ubiq helper").mcps, vec!["ubiq-help".to_string()]);

        for id in ["Coordinator", "Worker", "Ubiq helper"] {
            assert!(
                by_id(id).description.is_some_and(|it| !it.is_empty()),
                "{id} carries a real description another agent can read"
            );
        }

        assert!(
            !agents.ensure_default_definitions().unwrap(),
            "a root that holds one already is left alone"
        );
    }

    /// With no harness on the machine there is nothing a definition could name, so writing a new
    /// one is refused — and repairing one that already exists is not.
    #[test]
    fn a_new_definition_needs_a_harness_and_an_existing_one_does_not() {
        assert!(may_write_definition(false, true).is_ok());
        assert!(
            may_write_definition(true, false).is_ok(),
            "an edit still goes through"
        );
        let refused = may_write_definition(false, false).unwrap_err().to_string();
        assert!(
            refused.contains("no harness is configured"),
            "the refusal says why: {refused}"
        );
    }

    /// An `Agents` over `root` that has a harness to pin a definition to, whatever this machine
    /// has installed: a command override is what makes one available (`Agents::types`).
    fn with_a_harness(root: &Path) -> Agents {
        let mut agents = Agents::new(root, false);
        agents.set_commands(BTreeMap::from([(
            "claude-code".to_string(),
            "echo".to_string(),
        )]));
        agents
    }

    /// A bare definition of `id`, for the tests above: the one harness they all pin to, nothing
    /// else mentioned.
    fn a_definition(id: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            description: None,
            agent_type: "claude-code".to_string(),
            account: None,
            model: None,
            mode: None,
            thinking: None,
            max_subagents: None,
            prompt: None,
            mcps: Vec::new(),
            skills: Vec::new(),
            mission_assistant: None,
            mission_coordinator: false,
            mission_worker: false,
            disabled: false,
            project: None,
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
            .compose(
                pane,
                "claude-code",
                cwd.path(),
                Vec::new(),
                ConverseOptions::default(),
            )
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

    /// P6: every confined run keeps the real home, whether its conversation named a definition
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

        let compose = |definition: Option<String>| {
            agents
                .compose_run(
                    "agent-1",
                    "claude-code",
                    cwd.path(),
                    Vec::new(),
                    IoModes::Structured,
                    ConverseOptions {
                        definition,
                        ..Default::default()
                    },
                )
                .expect("composing a structured claude-code run")
        };

        let real_home = isolate::real_home();

        for definition in [Some("work.setup".to_string()), None] {
            let composed = compose(definition.clone());
            let home = isolate::describe(composed.confined.as_ref().unwrap())
                .unwrap()
                .home_path;
            assert_eq!(
                home,
                real_home,
                "a {} run keeps the real home, not {}",
                if definition.is_some() {
                    "defined"
                } else {
                    "an ad-hoc"
                },
                home.display()
            );
        }
    }

    /// Write a definition naming an account, and the account's record, under a Ubiq config root.
    fn given_an_account(root: &Path, definition: &str, account: &str) {
        std::fs::create_dir_all(root.join("accounts")).unwrap();
        std::fs::write(
            root.join("accounts").join(format!("{account}.toml")),
            format!("id = \"{account}\"\n"),
        )
        .unwrap();

        let dir = root.join(DEFINITIONS_DIR).join(definition);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("profile.toml"),
            format!("harness = \"claude-code\"\naccount = \"{account}\"\n"),
        )
        .unwrap();
    }

    /// `D194`: a definition named `default` — the one a bare start resolves — names an account,
    /// and the run goes to **that account's** home, with the run directory as its scratch beside
    /// it. Retiring the run takes the scratch and never the home, which outlives every run.
    #[test]
    fn a_run_goes_to_the_home_of_the_account_it_resolved() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        given_an_account(root.path(), "default", "work");

        let agents = Agents::new(root.path(), false);
        let pane = PaneId::generate();
        let composed = agents
            .compose(
                pane,
                "claude-code",
                cwd.path(),
                Vec::new(),
                ConverseOptions::default(),
            )
            .expect("composing a claude-code run against the default definition");

        let home = agents.home_store().home("work", "claude-code").unwrap();
        assert_eq!(composed.account(), Some("work"));
        assert_eq!(
            composed.dir,
            agents.run_dir(pane),
            "the scratch is the pane's own"
        );
        assert!(home.is_dir(), "the home was prepared");
        let meta = session::load(&agents.sessions_dir(), &pane.to_string()).unwrap();
        assert_eq!(
            meta.config,
            Some(ConfigStrategy::Home {
                home: home.clone(),
                scratch: agents.run_dir(pane),
            })
        );

        agents.retire(pane);
        assert!(!composed.dir.exists());
        assert!(home.is_dir(), "retiring never reaches the home");
    }

    /// A run that resolves no account runs from the user's own config in place, and says so in
    /// its record — nothing of the user's is written, and no home is made for a nameless run.
    #[test]
    fn no_account_runs_on_the_user_s_own_config() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let agent = AgentId::generate();

        let composed = agents
            .compose_run(
                &agent.to_string(),
                "claude-code",
                cwd.path(),
                Vec::new(),
                IoModes::Structured,
                ConverseOptions::default(),
            )
            .expect("composing a claude-code run with no definition");

        assert_eq!(composed.dir, agents.agent_dir(agent));
        let meta = session::load(&agents.sessions_dir(), &agent.to_string()).unwrap();
        assert_eq!(
            meta.config,
            Some(ConfigStrategy::Native {
                scratch: agents.agent_dir(agent),
            })
        );
        agents.retire_agent(agent);
    }

    /// Which strategy a run gets (`D194`): the account's home, the user's own config with no
    /// account or a name no home can be keyed by, and the run directory only for a harness that
    /// cannot share a home — never for a built-in one.
    #[test]
    fn run_config_picks_the_home_native_or_the_run_directory() {
        let root = tempfile::TempDir::new().unwrap();
        let homes = HomeStore::new(root.path().join("harness-homes"));
        let scratch = root.path().join("runs").join("key");
        // Every built-in harness shares a home, so the one that does not is a stand-in.
        struct Unshared;
        impl harness::Harness for Unshared {
            fn id(&self) -> agent_manager::spec::HarnessId {
                "unshared".to_string()
            }
            fn display_name(&self) -> &str {
                "unshared"
            }
            fn command(&self) -> &str {
                "unshared"
            }
            fn aliases(&self) -> &[&str] {
                &[]
            }
            fn io_support(&self) -> harness::IoSupport {
                harness::IoSupport::default()
            }
            fn provision(
                &self,
                _spec: &agent_manager::spec::RunSpec,
                _dir: &Path,
            ) -> Result<Launch> {
                bail!("not launched in this test")
            }
        }
        let claude = harness::resolve("claude-code").unwrap();
        let unshared = Unshared;
        let pick = |harness: &dyn harness::Harness, account: Option<&str>| {
            run_config(harness, account, &homes, scratch.clone())
        };
        let home = ConfigStrategy::Home {
            home: homes.home("work", "claude-code").unwrap(),
            scratch: scratch.clone(),
        };
        let native = ConfigStrategy::Native {
            scratch: scratch.clone(),
        };

        assert_eq!(pick(claude.as_ref(), Some("work")), home);
        assert_eq!(pick(claude.as_ref(), None), native);
        // A name that could escape the homes root keys no home, so the run falls back to the
        // user's own config rather than to a path built out of it.
        assert_eq!(pick(claude.as_ref(), Some("../elsewhere")), native);
        let fixed = ConfigStrategy::Fixed(scratch.clone());
        assert_eq!(pick(&unshared, Some("work")), fixed);
        assert_eq!(pick(&unshared, None), fixed);
        for harness in harness::all() {
            for account in [None, Some("work")] {
                assert!(
                    matches!(
                        pick(harness.as_ref(), account),
                        ConfigStrategy::Home { .. } | ConfigStrategy::Native { .. }
                    ),
                    "{} with account {account:?} took the per-run dir",
                    harness.id()
                );
            }
        }
    }

    /// A confined run takes the same strategy an unconfined one does (`D194`), and its policy is
    /// granted the home it runs from read-write: the account's with one, the harness's own
    /// default config with none.
    #[test]
    fn a_confined_run_is_granted_the_home_it_runs_from() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let mut agents = Agents::new(root.path(), false);
        agents.set_isolate(true);
        let granted = |composed: &Composed, path: &Path| {
            composed
                .confined
                .as_ref()
                .expect("a confined run has a policy")
                .spec
                .add_dirs_rw
                .contains(&path.display().to_string())
        };

        let pane = PaneId::generate();
        let composed = agents
            .compose(
                pane,
                "claude-code",
                cwd.path(),
                Vec::new(),
                ConverseOptions::default(),
            )
            .expect("composing a confined claude-code run with no definition");
        let meta = session::load(&agents.sessions_dir(), &pane.to_string()).unwrap();
        assert_eq!(
            meta.config,
            Some(ConfigStrategy::Native {
                scratch: agents.run_dir(pane),
            })
        );
        let claude = harness::resolve("claude-code").unwrap();
        assert!(!claude.default_homes().is_empty());
        for home in claude.default_homes() {
            assert!(granted(&composed, &home), "{} not granted", home.display());
        }
        agents.retire(pane);

        given_an_account(root.path(), "default", "work");
        let pane = PaneId::generate();
        let composed = agents
            .compose(
                pane,
                "claude-code",
                cwd.path(),
                Vec::new(),
                ConverseOptions::default(),
            )
            .expect("composing a confined claude-code run against the default definition");
        let home = agents.home_store().home("work", "claude-code").unwrap();
        let meta = session::load(&agents.sessions_dir(), &pane.to_string()).unwrap();
        assert_eq!(
            meta.config,
            Some(ConfigStrategy::Home {
                home: home.clone(),
                scratch: agents.run_dir(pane),
            })
        );
        assert!(granted(&composed, &home), "the account's home");
        agents.retire(pane);
    }

    /// Archiving a run from a shared home takes this run's one session out of it — the
    /// transcript and its companion directory — and none of the other runs' beside it.
    #[test]
    fn archiving_a_home_run_takes_only_its_own_session() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let agent = AgentId::generate();
        let key = agent.to_string();
        let dir = given_a_run(&agents, &key, "claude-code");
        let home = root.path().join("home");
        let project = home.join("projects").join("-tmp");
        std::fs::create_dir_all(project.join("mine").join("subagents")).unwrap();
        std::fs::write(project.join("mine.jsonl"), "mine").unwrap();
        std::fs::write(project.join("mine/subagents/a.jsonl"), "sub").unwrap();
        std::fs::write(project.join("theirs.jsonl"), "theirs").unwrap();
        let mut meta = session::load(&agents.sessions_dir(), &key).unwrap();
        meta.config = Some(ConfigStrategy::Home {
            home: home.clone(),
            scratch: dir,
        });
        session::save(&agents.sessions_dir(), &meta).unwrap();

        // No session named yet: nothing is taken.
        agents.archive(&key);
        let archived = agents.sessions_dir().join(&key).join("harness");
        assert!(!archived.exists());

        agents.remember_session(agent, "mine");
        agents.archive(&key);

        let mut names: Vec<String> = std::fs::read_dir(&archived)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["mine".to_string(), "mine.jsonl".to_string()]);
        assert!(archived.join("mine/subagents/a.jsonl").is_file());
        assert!(home.join("projects/-tmp/theirs.jsonl").is_file());
    }

    /// Signing an account in prepares **its** home and runs the harness's own login into it —
    /// nothing captured. The account need not exist first: it is the sign-in that makes one, so
    /// a name with no record is prepared and no record is written until the login exits cleanly.
    /// An unknown harness and a name no home can be keyed by are both refused.
    #[test]
    fn an_account_sign_in_prepares_its_home_and_logs_into_it() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());

        let pending = agents
            .begin_home_login("claude-code", "work")
            .expect("a sign-in into the account's home");

        let home = agents.home_store().home("work", "claude-code").unwrap();
        assert_eq!(pending.home(), home);
        assert!(home.is_dir(), "prepare_home made the home");
        assert_eq!(pending.account, "work");
        assert!(
            pending
                .launch()
                .env
                .iter()
                .any(|(_, value)| value == &home.display().to_string()),
            "the login is pointed at the home"
        );
        assert!(agents.begin_home_login("no-such-harness", "work").is_err());
        for bad in ["", "  ", "..", "a/b"] {
            assert!(
                agents.begin_home_login("claude-code", bad).is_err(),
                "accepted {bad:?} as an account name"
            );
        }
        assert!(
            agents.accounts().unwrap().is_empty(),
            "a sign-in that has not finished records nothing"
        );
    }

    /// A clean sign-in is the one thing that makes an account, and the homes are what say which
    /// harnesses it is signed in to. Signing one harness out shortens that list and leaves the
    /// account — and its other harnesses — alone; deleting the account takes every home with it.
    #[test]
    fn a_recorded_account_lists_the_harnesses_its_homes_hold() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());
        let claude = harness::resolve("claude-code").unwrap();

        agents.record_account("work").unwrap();
        let listed = agents.accounts().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "work");
        assert!(
            listed[0].logged_in.is_empty(),
            "a recorded account with no login yet is signed in nowhere"
        );

        // What the harness itself calls its login, written where a real one would land.
        let home = agents.home_store().home("work", "claude-code").unwrap();
        let login = home.join(claude.login_files().first().expect("a login file"));
        std::fs::create_dir_all(login.parent().unwrap()).unwrap();
        std::fs::write(&login, b"{}").unwrap();
        assert_eq!(agents.accounts().unwrap()[0].logged_in, ["claude-code"]);

        // Recording twice never overwrites what is there.
        agents.record_account("work").unwrap();
        assert_eq!(agents.accounts().unwrap().len(), 1);

        agents.delete_harness_login("claude-code", "work").unwrap();
        let listed = agents.accounts().unwrap();
        assert_eq!(listed.len(), 1, "the account survives its sign-out");
        assert!(listed[0].logged_in.is_empty());
        assert!(!home.exists());
    }

    /// The macOS case, and the one that made a finished sign-in read as nothing: Claude Code
    /// keeps its login in the Keychain, so the home holds no credential file for anything to
    /// find. A recorded sign-in is what the account is listed by, and `Check` says `Unknown` —
    /// signed in, expiry out of reach — rather than `Missing`.
    #[test]
    fn a_sign_in_that_left_no_file_still_lists_and_checks_as_signed_in() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());

        assert!(matches!(
            agents.check_login("claude-code", "work", 0),
            LoginStatus::Missing
        ));

        agents.record_sign_in("claude-code", "work").unwrap();

        let home = agents.home_store().home("work", "claude-code").unwrap();
        assert!(!home.join(".credentials.json").exists(), "nothing captured");
        assert_eq!(agents.accounts().unwrap()[0].logged_in, ["claude-code"]);
        assert!(matches!(
            agents.check_login("claude-code", "work", 0),
            LoginStatus::Unknown
        ));

        agents.delete_harness_login("claude-code", "work").unwrap();
        assert!(agents.accounts().unwrap()[0].logged_in.is_empty());
        assert!(matches!(
            agents.check_login("claude-code", "work", 0),
            LoginStatus::Missing
        ));
    }

    /// A rename moves the homes with the identity, so the logins are still there under the new
    /// name; a delete takes them. `check_login` reads the home the account keys, and a home with
    /// nothing in it is `Missing` rather than an error.
    #[test]
    fn renaming_an_account_moves_its_homes_and_deleting_takes_them() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = with_a_harness(root.path());
        let claude = harness::resolve("claude-code").unwrap();
        let login_file = claude.login_files().first().expect("a login file").clone();

        agents.record_account("work").unwrap();
        let home = agents.home_store().home("work", "claude-code").unwrap();
        std::fs::create_dir_all(home.join(&login_file).parent().unwrap()).unwrap();
        std::fs::write(home.join(&login_file), b"{}").unwrap();

        assert_eq!(
            agents.check_login("claude-code", "nobody", 0),
            LoginStatus::Missing
        );
        assert_eq!(
            agents.check_login("no-such-harness", "work", 0),
            LoginStatus::Missing
        );
        // A login stating no expiry is stored but cannot say whether it still works.
        assert_eq!(
            agents.check_login("claude-code", "work", 0),
            LoginStatus::Unknown
        );

        agents.rename_account("work", "day-job").unwrap();
        let moved = agents.home_store().home("day-job", "claude-code").unwrap();
        assert!(moved.join(&login_file).is_file(), "the login moved too");
        assert!(!home.exists());
        let listed = agents.accounts().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "day-job");
        assert_eq!(listed[0].logged_in, ["claude-code"]);

        agents.delete_account("day-job").unwrap();
        assert!(agents.accounts().unwrap().is_empty());
        assert!(!moved.exists(), "the homes go with the identity");
    }

    /// An account id nothing answers to used to fail the whole compose. It now degrades like
    /// every other stale definition reference (T-73): the run still starts, with no account
    /// resolved, and the dropped id is named on `Composed::problems` for the caller to surface —
    /// a typo in one line of a definition must not be the reason nothing launches.
    #[test]
    fn an_unknown_account_degrades_rather_than_refusing_the_run() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let dir = root.path().join(DEFINITIONS_DIR).join("default");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("profile.toml"),
            "harness = \"claude-code\"\naccount = \"nope\"\n",
        )
        .unwrap();

        let agents = Agents::new(root.path(), false);
        let composed = agents
            .compose(
                PaneId::generate(),
                "claude-code",
                cwd.path(),
                Vec::new(),
                ConverseOptions::default(),
            )
            .expect("an unknown account degrades rather than refusing the run");

        assert!(composed.account().is_none());
        assert_eq!(composed.problems.len(), 1);
        assert!(
            composed.problems[0].contains("nope"),
            "the problem names the account that is missing: {}",
            composed.problems[0]
        );
    }

    /// No definition at all resolves exactly as it did before this seam existed, which is what
    /// keeps a machine that has never configured an account working.
    #[test]
    fn no_definition_composes_a_run_with_no_account() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let pane = PaneId::generate();

        let composed = agents
            .compose(
                pane,
                "claude-code",
                cwd.path(),
                Vec::new(),
                ConverseOptions::default(),
            )
            .expect("composing with no definitions root at all");

        assert_eq!(composed.account(), None);
        agents.retire(pane);
    }

    /// An account referencing an environment variable is listed by its id.
    #[test]
    fn an_account_is_listed_by_its_id() {
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
        assert!(
            accounts[0].logged_in.is_empty(),
            "no home, so no harness is signed in"
        );
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
    /// record the storage paths read, and one file standing in for the harness's
    /// own session store.
    fn given_a_run(agents: &Agents, key: &str, harness_id: &str) -> PathBuf {
        let dir = agents.run_dir_for(key);
        std::fs::create_dir_all(dir.join("projects")).unwrap();
        std::fs::write(dir.join("projects").join("store.jsonl"), "a turn").unwrap();

        let mut meta = session::SessionMeta::new(
            harness_id.to_string(),
            PathBuf::from("/tmp"),
            vec!["true".to_string()],
            None,
            "structured".to_string(),
            dir.clone(),
        );
        meta.id = key.to_string();
        session::save(&agents.sessions_dir(), &meta).unwrap();
        dir
    }

    /// Parking keeps the harness's session store — that is the whole point.
    #[test]
    fn parking_keeps_the_store() {
        let root = tempfile::TempDir::new().unwrap();
        let agents = Agents::new(root.path(), false);
        let agent = AgentId::generate();
        let dir = given_a_run(&agents, &agent.to_string(), "claude-code");

        agents.park_agent(agent);

        assert!(dir.join("projects").join("store.jsonl").exists());
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
    /// conversation.
    #[test]
    fn forking_copies_the_store() {
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
    /// its harness's session store lives.
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
        assert!(!gone.exists());
    }
}
