//! Harnesses built at runtime from the ACP agent registry ([`crate::acp_registry`]).
//!
//! One generic [`Harness`] impl, [`RegistryHarness`], stands for any registry agent this machine
//! can run: it launches the resolved executable — or `npx` / `uvx` with the pinned package — with
//! the registry's ACP arguments and drives it through the generic [`crate::io::AcpBridge`]. A plain
//! terminal runs the agent's own interface ([`RegistryHarness::tui`]), never the ACP program, and
//! is refused when there is none. The embedder decides which agents are offered and
//! installs them with [`set_registry_harnesses`]; from then on [`super::all`] and
//! [`super::resolve`] answer them like any built-in harness.
//!
//! The registry knows nothing about a harness's config home, accounts, skills or hooks, so none of
//! those reach a registry run: it inherits the user's own environment and config, and MCP servers
//! are whatever ACP's `session/new` carries.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::Result;
use crate::acp_registry::{AcpAgentStatus, AcpStatus, Installer};
use crate::spec::{HarnessId, IoModes, RunSpec};

use super::{Harness, IoSupport, Launch, ModelInfo};

/// A registry agent this machine can run, resolved to an executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryHarness {
    /// The registry id, used as the harness id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Registry version, used as the catalogue cache key instead of spawning `--version`.
    pub version: String,
    /// The executable's file name — what a conversation is named after; the id for a runner.
    pub command: String,
    /// The resolved executable: the agent, or `npx` / `uvx` for a runner.
    pub program: PathBuf,
    /// Arguments before the ACP ones: `--yes -- <package>` for `npx`, `<package>` for `uvx`.
    pub prefix: Vec<String>,
    /// Arguments that start it in ACP mode.
    pub args: Vec<String>,
    /// Environment the registry says it wants.
    pub env: Vec<(String, String)>,
    /// What a plain terminal runs; `None` refuses one — an ACP adapter has no interface.
    pub tui: Option<PathBuf>,
    /// The version directory a downloaded binary was installed into; read-only to a confined run.
    pub install_root: Option<PathBuf>,
    /// The runner, when the agent is run through `npx` / `uvx` rather than installed.
    pub runner: Option<Installer>,
}

impl RegistryHarness {
    /// The harness for an [`AcpStatus::Installed`] or [`AcpStatus::ViaRunner`] agent; `None` for
    /// any other status, and for an agent that is already a built-in harness
    /// ([`crate::acp_registry::builtin_of`]) or whose id a built-in one already answers to.
    pub fn from_status(status: &AcpAgentStatus) -> Option<Self> {
        if builtin_for(&status.id).is_some() {
            return None;
        }
        let base = |command: String, program: &PathBuf, args: &Vec<String>, env: &Vec<_>| Self {
            id: status.id.clone(),
            name: status.name.clone(),
            version: status.version.clone(),
            command,
            program: program.clone(),
            prefix: Vec::new(),
            args: args.clone(),
            env: env.clone(),
            tui: None,
            install_root: None,
            runner: None,
        };
        match &status.status {
            AcpStatus::Installed { command, args, env, tui, install_root } => {
                let file = command
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| status.id.clone());
                Some(Self {
                    tui: tui.clone(),
                    install_root: install_root.clone(),
                    ..base(file, command, args, env)
                })
            }
            AcpStatus::ViaRunner { runner, installer, package, args, env } => {
                let prefix = match installer {
                    Installer::Npm => vec!["--yes".to_string(), "--".to_string(), package.clone()],
                    Installer::Uv => vec![package.clone()],
                };
                Some(Self {
                    prefix,
                    runner: Some(*installer),
                    ..base(status.id.clone(), runner, args, env)
                })
            }
            _ => None,
        }
    }

    /// A structured run launches the ACP program; a plain terminal launches [`Self::tui`], and
    /// is refused without one.
    fn launch(&self, spec: &RunSpec) -> Result<Launch> {
        let (program, mut args, env) = if spec.io == IoModes::Structured {
            let args = self.prefix.iter().chain(&self.args).cloned().collect();
            (&self.program, args, self.env.clone())
        } else {
            let Some(tui) = &self.tui else {
                anyhow::bail!("{} has no terminal interface: it runs over ACP only", self.name);
            };
            (tui, Vec::new(), Vec::new())
        };
        args.extend(spec.passthrough_args.iter().cloned());
        Ok(Launch {
            program: program.to_string_lossy().into_owned(),
            args,
            env,
            env_remove: Vec::new(),
            env_clear: false,
        })
    }
}

/// The directories an executable is installed under, to be read by a confined run. The
/// executable is resolved through symlinks first. Always its own directory; beyond that, a
/// prefix only where the resolved path says what it is: the outermost `node_modules` it sits in
/// (an npm package and its dependencies), or a `uv/tools/<name>` tool environment plus uv's
/// `python` dir its interpreter links into. Never anything shallower than two levels below the
/// root, the home directory or a direct child of it (`~/.cargo`, `~/.local`), `/usr/local` or
/// `/opt/homebrew`.
fn install_prefixes(program: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |dir: &Path| {
        let deep = dir.components().count() >= 3;
        let homeish = home.is_some_and(|h| dir == h || dir.parent() == Some(h));
        let shared = [Path::new("/usr/local"), Path::new("/opt/homebrew")].contains(&dir);
        if deep && !homeish && !shared && !out.iter().any(|it| it == dir) {
            out.push(dir.to_path_buf());
        }
    };
    let resolved = std::fs::canonicalize(program).unwrap_or_else(|_| program.to_path_buf());
    if let Some(dir) = resolved.parent() {
        push(dir);
    }
    if let Some(modules) = resolved
        .ancestors()
        .filter(|a| a.file_name().is_some_and(|n| n == "node_modules"))
        .last()
    {
        push(modules);
    }
    // `<…>/uv/tools/<name>/…`: the tool's environment, and uv's managed Pythons beside it.
    let parts: Vec<&std::ffi::OsStr> = resolved.iter().collect();
    if let Some(i) = parts
        .windows(2)
        .position(|w| w[0] == "uv" && w[1] == "tools")
        && let Some(name) = parts.get(i + 2)
    {
        let uv: PathBuf = parts[..=i].iter().collect();
        push(&uv.join("tools").join(name));
        push(&uv.join("python"));
    }
    out
}

/// The built-in harness registry agent `id` already is: the registry's mapping
/// ([`crate::acp_registry::builtin_of`]), else a built-in whose id or alias is `id`.
pub fn builtin_for(id: &str) -> Option<String> {
    crate::acp_registry::builtin_of(id)
        .map(str::to_string)
        .or_else(|| {
            super::builtins()
                .into_iter()
                .find(|h| h.id() == id || h.aliases().contains(&id))
                .map(|h| h.id())
        })
}

static INSTALLED: RwLock<Vec<RegistryHarness>> = RwLock::new(Vec::new());

/// Replace the registry harnesses [`super::all`] offers after the built-in ones.
pub fn set_registry_harnesses(harnesses: Vec<RegistryHarness>) {
    *INSTALLED.write().unwrap_or_else(|e| e.into_inner()) = harnesses;
}

/// The registry harnesses currently offered.
pub fn registry_harnesses() -> Vec<RegistryHarness> {
    INSTALLED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

impl Harness for RegistryHarness {
    fn id(&self) -> HarnessId {
        self.id.clone()
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn command(&self) -> &str {
        &self.command
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn io_support(&self) -> IoSupport {
        IoSupport {
            passthrough: self.tui.is_some(),
            structured: true,
            multi_turn: true,
            acp: true,
            steer: false,
            quota: crate::quota::QuotaSource::None,
        }
    }

    /// Read-write: the agent's known config homes ([`crate::acp_registry::config_homes`]) and
    /// nothing guessed — a guess from a command name could reach `~/.ssh`. Read-only: where the
    /// executable and its interface are installed ([`install_prefixes`]) and the version
    /// directory of a downloaded binary. A runner's package cache (`~/.npm`, `~/.cache/uv`) is
    /// read-write too: it fetches the package there on every launch.
    fn run_grants(&self) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf());
        let mut ro = install_prefixes(&self.program, home.as_deref());
        if let Some(tui) = &self.tui {
            for dir in install_prefixes(tui, home.as_deref()) {
                if !ro.contains(&dir) {
                    ro.push(dir);
                }
            }
        }
        if let Some(root) = &self.install_root
            && !ro.contains(root)
        {
            ro.push(root.clone());
        }
        let Some(home) = home else {
            return (Vec::new(), ro);
        };
        let mut rw: Vec<PathBuf> = crate::acp_registry::config_homes(&self.id)
            .iter()
            .map(|rel| home.join(rel))
            .collect();
        match self.runner {
            Some(Installer::Npm) => rw.push(home.join(".npm")),
            Some(Installer::Uv) => rw.push(home.join(".cache/uv")),
            None => {}
        }
        (rw, ro)
    }

    /// Nothing to write: the registry describes no config home, so the run uses the user's own.
    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        let launch = self.launch(spec)?;
        std::fs::create_dir_all(dir)?;
        Ok(launch)
    }

    /// The models come over ACP once a session is open; there is nothing to probe beforehand.
    fn discover_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    fn version(&self) -> Result<String> {
        Ok(self.version.clone())
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        Ok(Box::new(crate::io::AcpBridge::new(
            child,
            cwd,
            provisioned.resume.as_deref(),
            provisioned.model.as_deref(),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(id: &str, s: AcpStatus) -> AcpAgentStatus {
        AcpAgentStatus {
            id: id.into(),
            name: "N".into(),
            version: "1".into(),
            description: String::new(),
            status: s,
            update: None,
        }
    }

    fn installed() -> AcpStatus {
        AcpStatus::Installed {
            command: PathBuf::from("/opt/bin/gemini"),
            args: vec!["--acp".into()],
            env: vec![],
            tui: Some(PathBuf::from("/opt/bin/gemini")),
            install_root: None,
        }
    }

    fn adapter() -> AcpStatus {
        AcpStatus::Installed {
            command: PathBuf::from("/opt/x/vibe-acp"),
            args: vec![],
            env: vec![("K".into(), "v".into())],
            tui: None,
            install_root: Some(PathBuf::from("/cfg/acp-agents/mistral-vibe/3.0.1")),
        }
    }

    #[test]
    fn a_plain_terminal_runs_the_tui_and_is_refused_without_one() {
        let mut spec = RunSpec::new("mistral-vibe".to_string(), PathBuf::from("."));
        spec.io = IoModes::Passthrough;
        let h = RegistryHarness::from_status(&status("mistral-vibe", adapter())).unwrap();
        assert!(!h.io_support().passthrough);
        assert!(h.launch(&spec).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert!(h.provision(&spec, &dir.path().join("run")).is_err());
        spec.io = IoModes::Structured;
        assert_eq!(h.launch(&spec).unwrap().program, "/opt/x/vibe-acp");
        let (_, ro) = h.run_grants();
        assert!(ro.contains(&PathBuf::from("/cfg/acp-agents/mistral-vibe/3.0.1")));

        let mut with_tui = adapter();
        if let AcpStatus::Installed { tui, .. } = &mut with_tui {
            *tui = Some(PathBuf::from("/opt/x/vibe"));
        }
        let h = RegistryHarness::from_status(&status("mistral-vibe", with_tui)).unwrap();
        spec.io = IoModes::Passthrough;
        let l = h.launch(&spec).unwrap();
        assert_eq!(l.program, "/opt/x/vibe");
        assert!(l.args.is_empty() && l.env.is_empty());
    }

    #[test]
    fn a_runner_launches_the_pinned_package() {
        let s = |installer, package: &str| AcpStatus::ViaRunner {
            runner: PathBuf::from("/usr/bin/runner"),
            installer,
            package: package.into(),
            args: vec!["--acp".into()],
            env: vec![],
        };
        let mut spec = RunSpec::new("autohand".to_string(), PathBuf::from("."));
        spec.io = IoModes::Structured;
        let h = RegistryHarness::from_status(&status("autohand", s(Installer::Npm, "a@1.0.0"))).unwrap();
        let l = h.launch(&spec).unwrap();
        assert_eq!(l.program, "/usr/bin/runner");
        assert_eq!(l.args, vec!["--yes", "--", "a@1.0.0", "--acp"]);
        assert_eq!(h.command, "autohand");
        spec.io = IoModes::Passthrough;
        assert!(h.launch(&spec).is_err());
        let h = RegistryHarness::from_status(&status("m", s(Installer::Uv, "m==1"))).unwrap();
        spec.io = IoModes::Structured;
        assert_eq!(h.launch(&spec).unwrap().args, vec!["m==1", "--acp"]);
        if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
            assert!(h.run_grants().0.contains(&home.join(".cache/uv")));
        }
    }

    #[test]
    fn installed_becomes_a_harness_and_builtins_do_not() {
        let h = RegistryHarness::from_status(&status("gemini", installed())).unwrap();
        assert_eq!(h.command, "gemini");
        assert!(RegistryHarness::from_status(&status("github-copilot-cli", installed())).is_none());
        assert!(RegistryHarness::from_status(&status("opencode", installed())).is_none());
        assert!(RegistryHarness::from_status(&status("x", AcpStatus::NotInstalled)).is_none());
    }

    #[test]
    fn structured_launch_carries_the_acp_args() {
        let h = RegistryHarness::from_status(&status("gemini", installed())).unwrap();
        let mut spec = RunSpec::new("gemini".to_string(), PathBuf::from("."));
        spec.io = IoModes::Structured;
        let l = h.launch(&spec).unwrap();
        assert_eq!(l.program, "/opt/bin/gemini");
        assert_eq!(l.args, vec!["--acp"]);
        spec.io = IoModes::Passthrough;
        assert!(h.launch(&spec).unwrap().args.is_empty());
    }

    #[test]
    fn install_prefixes_reach_the_package_and_stay_narrow() {
        let p = install_prefixes(
            Path::new("/opt/homebrew/lib/node_modules/pi-acp/dist/index.js"),
            None,
        );
        assert_eq!(
            p,
            vec![
                PathBuf::from("/opt/homebrew/lib/node_modules/pi-acp/dist"),
                PathBuf::from("/opt/homebrew/lib/node_modules"),
            ]
        );
        let p = install_prefixes(
            Path::new("/home/u/.local/share/uv/tools/fast/bin/fast"),
            Some(Path::new("/home/u")),
        );
        assert_eq!(
            p,
            vec![
                PathBuf::from("/home/u/.local/share/uv/tools/fast/bin"),
                PathBuf::from("/home/u/.local/share/uv/tools/fast"),
                PathBuf::from("/home/u/.local/share/uv/python"),
            ]
        );
        // A plain binary gets its own directory only, and never a shared or home-level one.
        assert!(install_prefixes(Path::new("/usr/bin"), None).is_empty());
        assert!(install_prefixes(Path::new("/usr/local/bin/x/y"), None)
            .iter()
            .all(|d| d != Path::new("/usr/local")));
        assert!(
            install_prefixes(Path::new("/home/u/.cargo/goose"), Some(Path::new("/home/u")))
                .is_empty()
        );
    }
}
