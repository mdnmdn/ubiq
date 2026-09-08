//! The machine's development environment: the variables and the directories a confined agent
//! needs because *this* machine keeps its toolchains somewhere the shipped policy cannot guess.
//!
//! Two facts make this a file of its own rather than another field in
//! [`ubiq_proto::settings::HostSettings`]:
//!
//! - **It is machine state, not preference.** A relocated `CARGO_HOME`, an SDK under
//!   `~/works`, a `pyenv` outside `~/.pyenv` — none of it belongs in a settings record that
//!   syncs or that a user edits per project, and all of it is what the policy has to know.
//! - **It is what the environment Ubiq was launched from may not carry.** The isolate stage
//!   grants a relocated toolchain root by reading its variable (`CARGO_HOME`, `PYENV_ROOT`,
//!   `GOROOT`, …), and a GUI launch has none of them: no login shell ever ran. A run then
//!   holds `cargo` in its `PATH` and is denied the moment it is executed. This file is where
//!   those answers live so they are the same however Ubiq was started.
//!
//! `<config root>/environment.toml`, read at startup, absent by default. The planned
//! policy UI edits this one file, which is why it holds both halves of the answer — the
//! variables and the grants — rather than scattering them.
//!
//! ```toml
//! [env]
//! CARGO_HOME = "/Users/me/works/rust/sdk/cargo"
//!
//! [[grants]]
//! path = "~/works/flutter/sdk"
//! write = false
//! ```

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ubiq_proto::settings::Grant;

/// The file under the config root, named here because the future settings UI writes it too.
pub const FILE: &str = "environment.toml";

/// What this machine has to say about where its toolchains are.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// Variables every run gets, unless the harness itself set the same name — the harness's
    /// own `CLAUDE_CONFIG_DIR` and its siblings are never shadowed from here.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Directories a run may reach beyond the ones the variables above already imply.
    #[serde(default)]
    pub grants: Vec<Grant>,
}

impl Environment {
    /// Read `<root>/environment.toml`.
    ///
    /// A missing file is the empty environment, and so is an unreadable or malformed one: a
    /// machine description nobody can parse must never be the reason a pane refuses to open.
    /// It is logged, because a policy silently ignored is the bug that looks like a denial.
    pub fn load(root: &Path) -> Self {
        let path = root.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                tracing::warn!("reading {}: {error}", path.display());
                return Self::default();
            }
        };
        match toml::from_str(&text) {
            Ok(environment) => environment,
            Err(error) => {
                tracing::warn!("{} is not a valid environment: {error}", path.display());
                Self::default()
            }
        }
    }

    /// What a variable resolves to for a run: this file first, then the process Ubiq itself
    /// was started with.
    ///
    /// The file wins because it is the deliberate answer and the process environment is
    /// whatever happened to launch the app — the Finder's, on macOS, where none of these are
    /// set at all.
    pub fn lookup(&self, name: &str) -> Option<OsString> {
        match self.env.get(name) {
            Some(value) => Some(OsString::from(value)),
            None => std::env::var_os(name),
        }
    }

    /// Every absolute directory on the run's `PATH`.
    ///
    /// Granted read-only, and that is the default this file exists to fix: a tool the user
    /// installed outside the layers' expectations is on `PATH` and nothing else can name it,
    /// so it reports as `operation not permitted` rather than as anything a person can act
    /// on. Reaching a tool is not the same as reaching what it writes — a build still needs
    /// its cache root, which is what the toolchain variables above grant.
    pub fn path_dirs(&self) -> Vec<PathBuf> {
        let Some(path) = self.lookup("PATH") else {
            return Vec::new();
        };
        let mut dirs = Vec::new();
        for dir in std::env::split_paths(&path) {
            // A relative entry resolves against the run's own cwd, which the policy already
            // grants, and a `~` one is a shell-ism no grant can expand.
            if dir.is_absolute() && !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_the_empty_environment() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(Environment::load(root.path()), Environment::default());
    }

    #[test]
    fn the_file_outranks_the_process_environment() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(FILE),
            "[env]\nCARGO_HOME = \"/opt/cargo\"\n\n[[grants]]\npath = \"~/sdk\"\nwrite = true\n",
        )
        .unwrap();
        let environment = Environment::load(root.path());
        assert_eq!(
            environment.lookup("CARGO_HOME"),
            Some(OsString::from("/opt/cargo"))
        );
        assert_eq!(environment.grants[0].path, "~/sdk");
        assert!(environment.grants[0].write);
        // Not named by the file, so the process answers.
        assert_eq!(environment.lookup("PATH"), std::env::var_os("PATH"));
    }

    #[test]
    fn a_malformed_file_denies_nothing_and_grants_nothing() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(FILE), "this is not toml {{{").unwrap();
        assert_eq!(Environment::load(root.path()), Environment::default());
    }

    #[test]
    fn path_dirs_are_absolute_and_unique() {
        let environment = Environment {
            env: BTreeMap::from([("PATH".to_string(), "/usr/bin:rel:/usr/bin:/bin".to_string())]),
            grants: Vec::new(),
        };
        assert_eq!(
            environment.path_dirs(),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }
}
