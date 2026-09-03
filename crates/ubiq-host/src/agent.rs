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

use agent_manager::harness::{self, Launch};
use agent_manager::io::IoBridge;
use agent_manager::isolate::{self, Confined, IsolateOptions};
use agent_manager::provision;
use agent_manager::spec::{ConfigStrategy, IoModes, Isolation, RunSpec};
use anyhow::{Context, Result, anyhow};
use ubiq_proto::ids::PaneId;
use ubiq_proto::messages::AgentTypeInfo;
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
        self.compose_run(
            &pane.to_string(),
            agent_type,
            cwd,
            args,
            IoModes::Passthrough,
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
    ) -> Result<(Composed, Box<dyn IoBridge>)> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;
        let composed = self.compose_run(
            &agent.to_string(),
            agent_type,
            cwd,
            Vec::new(),
            IoModes::Structured,
        )?;
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
    ) -> Result<Composed> {
        let harness = harness::resolve(agent_type)
            .ok_or_else(|| anyhow!("unknown agent type '{agent_type}'"))?;

        let structured = io == IoModes::Structured;
        let mut spec = RunSpec::new(harness.id(), cwd.to_path_buf());
        spec.config = ConfigStrategy::Fixed(self.run_dir_for(key));
        spec.passthrough_args = args;
        spec.io = io;
        if self.isolate && !structured {
            spec.isolation = Isolation::Sandboxed(String::new());
        }

        let templates = harness::FsTemplateStore::new(self.root.join("harness-templates"));
        let provisioned = provision::provision(harness.as_ref(), &spec, &templates)
            .with_context(|| format!("composing a {agent_type} run"))?;

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
        })
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
