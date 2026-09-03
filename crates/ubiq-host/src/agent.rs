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

use std::path::{Path, PathBuf};

use agent_manager::account::{AccountStore, FsAccountStore};
use agent_manager::harness::{self, Launch};
use agent_manager::io::IoBridge;
use agent_manager::isolate::{self, Confined, IsolateOptions};
use agent_manager::profile::FsProfileStore;
use agent_manager::provision;
use agent_manager::registry::FsRegistry;
use agent_manager::resolve;
use agent_manager::settings::Settings;
use agent_manager::spec::{ConfigStrategy, IoModes, Isolation};
use anyhow::{Context, Result, anyhow, bail};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::{AccountInfo, AgentTypeInfo};
use ubiq_proto::work::AgentId;

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
        }
    }

    /// Whether a run is confined unless something says otherwise.
    pub fn isolate(&self) -> bool {
        self.isolate
    }

    /// Follow the host settings, which are what the user last chose.
    pub fn set_isolate(&mut self, isolate: bool) {
        self.isolate = isolate;
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
            .map(|harness| AgentTypeInfo {
                id: harness.id(),
                label: harness.display_name().to_string(),
                available: crate::shells::locate(harness.command()).is_some(),
            })
            .collect()
    }

    /// Whether `id` names an agent type, so a spawn refuses a name the user can
    /// see rather than failing as a process that would not start.
    pub fn is_agent_type(&self, id: &str) -> bool {
        harness::resolve(id).is_some()
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

    /// Rewrite `launch`'s program to an absolute path, when `shells::locate` can find it.
    ///
    /// A harness's own `Launch` names its program bare (`"claude"`), because
    /// `crates/agent-manager` reads no process environment — `isolate.rs` says so of itself.
    /// Ubiq is the embedder that may, so it is done here, once, before a bare name reaches
    /// either a pty spawn or isol8's `confine_executable`: both resolve a bare name against
    /// only the thin `PATH` a desktop launch inherits, which is exactly the gap `locate`
    /// closes by also asking the login shell. Left bare when `locate` finds nothing, so a
    /// genuinely missing binary still fails with the library's own "not found" error rather
    /// than a swallowed one here.
    fn resolve_program(launch: &mut Launch) {
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
    pub fn begin_login(&self, agent_type: &str, account: &str) -> Result<PendingLogin> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        let home = self
            .account_store()
            .login_home(account)
            .with_context(|| format!("preparing a home for account '{account}'"))?;
        let mut plan = harness
            .login(&home)
            .with_context(|| format!("asking {agent_type} how it logs in"))?;
        Self::resolve_program(&mut plan.launch);

        // The credential's timestamp before the login runs. A harness that exits cleanly
        // without refreshing its credential has not logged anyone in, and this is the only
        // thing that tells the two apart.
        let captured_before = Self::credential_mtime(&home, &plan.credential_files);

        let confined = isolate::login_confined(
            &home,
            &plan,
            None,
            &IsolateOptions::new(self.root.join("isol8")),
        )
        .with_context(|| format!("resolving the policy a {agent_type} login runs under"))?;
        let launch = isolate::confined_launch(&confined)
            .with_context(|| format!("preparing a confined {agent_type} login"))?;

        Ok(PendingLogin {
            account: account.to_string(),
            agent_type: agent_type.to_string(),
            home,
            files: plan.credential_files,
            captured_before,
            launch,
        })
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
            None,
        )
    }

    /// Compose a run for `agent` and drive it over structured I/O, answering
    /// the bridge its events come out of.
    ///
    /// The conversation face of the same thing [`compose`](Self::compose)
    /// builds. Two differences, both forced rather than chosen:
    ///
    /// - The run is **never confined**, whatever the setting says. A bridge
    ///   spawns its own child with pipes on its descriptors, and the sandbox
    ///   needs those descriptors to hand it a policy; the library refuses the
    ///   combination outright, and producing it quietly here would be worse.
    /// - The id is the agent's rather than a pane's, because a conversation
    ///   has no pane. It is the day `WorkspaceId` and `PaneId` come apart.
    pub fn converse(
        &self,
        agent: AgentId,
        agent_type: &str,
        cwd: &Path,
        account: Option<String>,
    ) -> Result<(Composed, Box<dyn IoBridge>)> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        let composed = self.compose_run(
            &agent.to_string(),
            agent_type,
            cwd,
            Vec::new(),
            IoModes::Structured,
            account,
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

        let bridge = harness
            .structured_bridge(&composed.provisioned, cwd)
            .with_context(|| format!("starting a {agent_type} conversation"))?;
        Ok((composed, bridge))
    }

    fn compose_run(
        &self,
        key: &str,
        agent_type: &str,
        cwd: &Path,
        args: Vec<String>,
        io: IoModes,
        account: Option<String>,
    ) -> Result<Composed> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;

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
            // mean: an identity chosen when the conversation started outranks the profile's.
            account,
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

        // The three answers that are Ubiq's rather than the library's: which directory this
        // run's configuration lives in, which face it wears, and whether it is confined.
        // The last one replaces whatever a profile asked for, because a conversation is
        // never confined (see `converse`) and the toggle belongs to Ubiq's own settings.
        let structured = io == IoModes::Structured;
        spec.config = ConfigStrategy::Fixed(self.run_dir_for(key));
        spec.io = io;
        spec.isolation = if self.isolate && !structured {
            Isolation::Sandboxed(String::new())
        } else {
            Isolation::None
        };

        let templates = harness::FsTemplateStore::new(self.root.join("harness-templates"));
        let mut provisioned = provision::provision(harness.as_ref(), &spec, &templates)
            .with_context(|| format!("composing a {agent_type} run"))?;
        Self::resolve_program(&mut provisioned.launch);

        let confined = isolate::plan(
            &provisioned.launch,
            &spec,
            &provisioned.dir,
            &IsolateOptions::new(self.root.join("isol8")),
        )
        .with_context(|| format!("resolving the policy for a {agent_type} run"))?;

        Ok(Composed {
            launch: provisioned.launch.clone(),
            confined,
            dir: provisioned.dir.clone(),
            provisioned,
            spec_account: spec.account.as_ref().map(|a| a.id.clone()),
        })
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

    /// Remove what a pane's run left behind. Best effort: a directory that
    /// cannot be deleted is a stale directory, not a reason to fail a close the
    /// user already saw happen.
    pub fn retire(&self, pane: PaneId) {
        let _ = std::fs::remove_dir_all(self.run_dir(pane));
    }

    /// Delete every run directory left by a previous process.
    ///
    /// A run directory outlives its pane only when Ubiq did not get to close
    /// it — a crash, a kill. Sweeping at startup is what keeps that from
    /// accumulating, and it is safe because no pane from a previous process is
    /// still running.
    pub fn sweep(&self) {
        let runs = self.root.join("runs");
        let Ok(entries) = std::fs::read_dir(&runs) else {
            return;
        };
        for entry in entries.flatten() {
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
        let _ = std::fs::remove_dir_all(self.agent_dir(agent));
    }

    /// One directory per run, named by whatever owns it — a pane or an agent.
    /// Both ids are ULIDs, so neither can be mistaken for the other's.
    fn run_dir_for(&self, key: &str) -> PathBuf {
        self.root.join("runs").join(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
