//! Runner discovery: the targets a project's own task-runner files offer, served as tools.
//!
//! A `Makefile`, a `justfile` and a `mise.toml` each list things worth running. This module turns
//! them into [`ToolDef`]s the run menu can offer without the user writing a row for each — and
//! without Ubiq storing one: a discovered tool is read again on every scan, its id is
//! [`ToolId::derived`] from the runner and the target so it is the same id each time, and nothing
//! here touches `ProjectRecord::tools` or any store.
//!
//! - [`RunnerSource`]: one runner. `discover` is handed a project's folder and answers its targets.
//!   It runs **off the coordinator's thread** and may spawn a CLI through [`capture`], which bounds
//!   the wait; a missing CLI is an empty list, not an error.
//! - [`Registry`]: a plain list of sources, filled at boot — the shape of
//!   [`crate::tasksrc::Registry`], for the same reasons. It is the value
//!   `ubiq_app::Contributions::runner_sources` carries: **seeded** with the base's three and
//!   changed only by `register` (substitutes by id) and `remove`.
//!
//! The coordinator owns the cache and the scan scheduling (`Coordinator::scan_runners`); this
//! module owns what a scan finds.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use ubiq_proto::ids::ToolId;
use ubiq_proto::tools::ToolDef;

mod just;
mod make;
mod mise;

pub use just::Just;
pub use make::Make;
pub use mise::Mise;

/// How long a runner CLI may take before its answer is given up on. A scan is on a worker thread,
/// so this bounds how stale a project's list can be after a hung `mise`, not a stalled pane.
pub const CLI_TIMEOUT: Duration = Duration::from_secs(2);

/// One thing a runner can run: the program, and what to hand it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerTarget {
    /// What the tab says and the menu lists — the target as the runner's file names it.
    pub name: String,
    /// The program, bare: `just`, `make`, `mise`. Resolved on the login shell's `PATH` at run time,
    /// the way every tool row's is.
    pub command: String,
    /// What follows it: `build`, `run test`.
    pub args: String,
}

impl RunnerTarget {
    /// The tool this target is. One run at a time and the pane kept open afterwards, because a
    /// target is usually a build or a server whose output is the point; no platform is named.
    pub fn into_tool(self, runner: &str) -> ToolDef {
        ToolDef {
            id: ToolId::derived(&format!("{runner}:{}", self.name)),
            name: self.name,
            command: self.command,
            args: self.args,
            env: BTreeMap::new(),
            platforms: Vec::new(),
            wait_on_exit: true,
            wait_on_error: false,
            single_instance: true,
            starting_folder: None,
        }
    }
}

/// One task runner, resolved by [`RunnerSource::id`].
pub trait RunnerSource: Send + Sync {
    /// Stable id — `make`, `just`, `mise` — which is also the `runner` a listed row carries and the
    /// prefix of its derived [`ToolId`]. A second registration under it replaces this one.
    fn id(&self) -> &str;

    /// The targets `root` offers, in the order the runner's own file gives them. Empty when the
    /// folder has no file for this runner or the runner's CLI is not installed.
    ///
    /// Blocking, and called from a worker thread only.
    fn discover(&self, root: &Path) -> Vec<RunnerTarget>;
}

/// The runner sources this build has, filled once at boot.
///
/// **This is the value `ubiq_app::Contributions::runner_sources` carries.** It obeys the rule every
/// container on that struct does: it arrives seeded with the base's own sources so a second edition
/// can substitute or remove one rather than only append; [`Registry::register`] is the only way
/// in. Like [`crate::tasksrc::Registry`] it is a host service resolved by id and never drawn, so it
/// is not a `ubiq::ext::Registry<Spec>`.
#[derive(Default)]
pub struct Registry {
    sources: Vec<Box<dyn RunnerSource>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// What this build ships: `just`, `make` and `mise`.
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(Just));
        registry.register(Box::new(Make));
        registry.register(Box::new(Mise));
        registry
    }

    /// Add one. A second registration under an id already held **replaces** it.
    pub fn register(&mut self, source: Box<dyn RunnerSource>) {
        let id = source.id().to_string();
        self.sources.retain(|held| held.id() != id);
        self.sources.push(source);
    }

    /// Drop one by id, and say whether there was one.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.sources.len();
        self.sources.retain(|held| held.id() != id);
        before != self.sources.len()
    }

    /// The ids held, in registration order.
    pub fn ids(&self) -> Vec<&str> {
        self.sources.iter().map(|held| held.id()).collect()
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Every source's targets for `root`, as `(runner id, tool)` in registration order. A target
    /// two sources both claim under one id appears once. Blocking — a worker thread only.
    pub fn discover(&self, root: &Path) -> Vec<(String, ToolDef)> {
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for source in &self.sources {
            for target in source.discover(root) {
                let tool = target.into_tool(source.id());
                if seen.insert(tool.id) {
                    found.push((source.id().to_string(), tool));
                }
            }
        }
        found
    }
}

/// The first of `names` that is a file in `root`, so a source can ask "is there a runner file here"
/// with the spellings the runner accepts.
pub(crate) fn first_file(root: &Path, names: &[&str]) -> Option<std::path::PathBuf> {
    names
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
}

/// Run `program` in `dir` and return its standard output, or `None` when it is not installed, does
/// not finish within [`CLI_TIMEOUT`], or exits unsuccessfully. Every `None` is logged at debug: a
/// machine without `just` is the ordinary case, not a fault.
///
/// The program is resolved on the login shell's `PATH` like a tool row's, since a desktop-launched
/// Ubiq has a thin one.
pub(crate) fn capture(program: &str, args: &[&str], dir: &Path) -> Option<String> {
    let Some(path) = crate::shells::locate(program) else {
        tracing::debug!("runner: {program} is not installed");
        return None;
    };
    let mut cmd = Command::new(path);
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!("runner: {program} did not start: {error}");
            return None;
        }
    };
    let mut stdout = child.stdout.take()?;
    // A reader of its own, so a child that fills the pipe cannot block the wait below.
    let (tx, rx) = flume::bounded(1);
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        let _ = tx.send(out);
    });
    match rx.recv_timeout(CLI_TIMEOUT) {
        Ok(out) => match child.wait() {
            Ok(status) if status.success() => Some(out),
            Ok(status) => {
                tracing::debug!("runner: {program} {args:?} exited with {status}");
                None
            }
            Err(_) => None,
        },
        Err(_) => {
            tracing::debug!("runner: {program} {args:?} timed out");
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str, &'static [&'static str]);

    impl RunnerSource for Fixed {
        fn id(&self) -> &str {
            self.0
        }
        fn discover(&self, _: &Path) -> Vec<RunnerTarget> {
            self.1
                .iter()
                .map(|name| RunnerTarget {
                    name: (*name).to_string(),
                    command: self.0.to_string(),
                    args: (*name).to_string(),
                })
                .collect()
        }
    }

    #[test]
    fn the_defaults_are_the_three_base_runners() {
        assert_eq!(Registry::with_defaults().ids(), ["just", "make", "mise"]);
    }

    #[test]
    fn register_substitutes_by_id_and_remove_takes_one_away() {
        let mut registry = Registry::with_defaults();
        registry.register(Box::new(Fixed("make", &["a"])));
        assert_eq!(registry.len(), 3);
        assert_eq!(registry.discover(Path::new("."))[0].1.name, "a");
        assert!(registry.remove("just"));
        assert!(!registry.remove("just"));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn a_target_becomes_a_single_instance_tool_with_a_stable_id() {
        let target = RunnerTarget {
            name: "build".to_string(),
            command: "just".to_string(),
            args: "build".to_string(),
        };
        let tool = target.clone().into_tool("just");
        assert_eq!(tool.id, ToolId::derived("just:build"));
        assert_eq!(tool.id, target.into_tool("just").id);
        assert!(tool.single_instance && tool.wait_on_exit && tool.platforms.is_empty());
    }

    #[test]
    fn a_target_repeated_under_one_runner_is_listed_once() {
        let mut registry = Registry::new();
        registry.register(Box::new(Fixed("just", &["a", "b", "a"])));
        let names: Vec<_> = registry
            .discover(Path::new("."))
            .into_iter()
            .map(|(_, tool)| tool.name)
            .collect();
        assert_eq!(names, ["a", "b"]);
    }
}
