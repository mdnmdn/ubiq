//! Turns a [`RunSpec`] + a chosen [`Harness`] into a populated ephemeral
//! config dir + a [`Launch`].
//!
//! This module owns directory creation and location. It does NOT launch the
//! harness (that's `run`, a later stage) and does NOT clean up the directory
//! afterwards (the runner owns that lifecycle) — see
//! `_docs/target/architecture.md` §"The provisioner and the custom config
//! folder bridge".

use std::path::PathBuf;

use anyhow::Context;

use crate::harness::{Harness, Launch};
use crate::spec::{ConfigStrategy, RunSpec};
use crate::Result;

/// The result of provisioning: where the config was written and how to launch.
#[derive(Debug, Clone)]
pub struct Provisioned {
    /// The (created, populated) ephemeral config dir.
    pub dir: PathBuf,
    /// How to launch the harness against `dir`.
    pub launch: Launch,
    /// True if `dir` is a throwaway the runner should delete on exit
    /// (`Ephemeral`); false if the user pinned it (`Fixed`).
    pub ephemeral: bool,
}

/// Provision `spec` for `harness` into a fresh (or pinned) config dir.
pub fn provision(harness: &dyn Harness, spec: &RunSpec) -> Result<Provisioned> {
    let (dir, ephemeral) = match &spec.config {
        ConfigStrategy::Fixed(path) => (path.clone(), false),
        ConfigStrategy::Ephemeral => (new_run_dir()?, true),
    };

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config dir {}", dir.display()))?;

    let launch = harness.provision(spec, &dir)?;

    Ok(Provisioned {
        dir,
        launch,
        ephemeral,
    })
}

/// Generate a fresh `<state>/runs/<run-id>/` path for an ephemeral run.
///
/// `<state>` prefers the OS state dir, falling back to the local data dir,
/// under the `agent-manager` project namespace. `<run-id>` is
/// `<unix-millis>-<pid>`, which is unique enough for a single-host tool
/// without pulling in a UUID dependency.
fn new_run_dir() -> Result<PathBuf> {
    let base = directories::ProjectDirs::from("", "", "agent-manager")
        .map(|dirs| {
            dirs.state_dir()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| dirs.data_local_dir().to_path_buf())
        })
        .context("could not determine a state/data directory for this OS")?;

    let run_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        std::process::id()
    );

    Ok(base.join("runs").join(run_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Claude;
    use crate::spec::RunSpec;
    use std::path::PathBuf;

    #[test]
    fn fixed_strategy_uses_the_given_dir_and_is_not_ephemeral() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(temp.path().to_path_buf());

        let claude = Claude::new();
        let provisioned = provision(&claude, &spec).unwrap();

        assert_eq!(provisioned.dir, temp.path());
        assert!(!provisioned.ephemeral);
        assert!(provisioned.dir.exists());
    }

    #[test]
    fn ephemeral_strategy_creates_a_fresh_dir_under_state() {
        let spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        let claude = Claude::new();
        let provisioned = provision(&claude, &spec).unwrap();

        assert!(provisioned.ephemeral);
        assert!(provisioned.dir.exists());
        assert!(provisioned.dir.to_string_lossy().contains("runs"));

        // Cleanup: this test writes to the real state dir since it exercises
        // the ephemeral path; remove what we created.
        let _ = std::fs::remove_dir_all(&provisioned.dir);
    }
}
