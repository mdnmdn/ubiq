//! Which shells this machine actually has, and how one is started.
//!
//! The interface may not look for itself — a program on disk is a local fact, and no path crosses
//! into UI code — so the list is made here and answered over the bus as
//! [`Message::ShellList`](ubiq_proto::messages::Message::ShellList).
//!
//! It is a fixed set of known names checked for existence, not a launcher for anything on the
//! machine: a menu of three or four shells is a bounded surface, and an open one would be a
//! configuration feature nobody asked for.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ubiq_proto::messages::ShellInfo;

/// The shells offered, in the order the menu shows them.
#[cfg(unix)]
const CANDIDATES: &[&str] = &["zsh", "bash", "fish", "sh"];
#[cfg(windows)]
const CANDIDATES: &[&str] = &["pwsh.exe", "powershell.exe", "cmd.exe"];

/// Where a shell is looked for besides `PATH`.
///
/// Ubiq started from Finder or a desktop launcher inherits a thin `PATH` — the very reason a pane's
/// shell has to source the user's own login files — so the usual homes are checked as well rather
/// than trusting the environment the application happens to have.
#[cfg(unix)]
const EXTRA_DIRS: &[&str] = &[
    "/bin",
    "/usr/bin",
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "/run/current-system/sw/bin",
];
#[cfg(windows)]
const EXTRA_DIRS: &[&str] = &[];

/// What a session starts when it is not told what to start: the user's own shell.
#[cfg(unix)]
pub fn default_program() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

/// What a session starts when it is not told what to start. Windows has no `$SHELL`, so the
/// command processor the system names is the default, as it is for every other terminal there.
#[cfg(windows)]
pub fn default_program() -> String {
    std::env::var("COMSPEC")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "cmd.exe".to_string())
}

/// Every shell this machine has, in menu order, with the default one marked.
///
/// A candidate that is not installed is left out — the menu offers what can actually be started.
/// The default is always in the list, even when it is a shell this module has never heard of: what
/// the new-pane control already starts has to be something the menu can name.
pub fn available() -> Vec<ShellInfo> {
    let default = default_program();
    let default_name = basename(&default);

    let mut shells: Vec<ShellInfo> = Vec::new();
    if !CANDIDATES.iter().any(|name| *name == default_name) {
        shells.push(ShellInfo {
            label: label_of(&default_name),
            program: default.clone(),
            is_default: true,
        });
    }
    for name in CANDIDATES {
        let is_default = *name == default_name;
        // The default's row is the default's own program, not whichever copy of that name the
        // probe found first: the row and a bare click on "+" have to start the same thing.
        let program = if is_default {
            Some(PathBuf::from(&default))
        } else {
            locate(name)
        };
        if let Some(program) = program {
            shells.push(ShellInfo {
                label: label_of(name),
                program: program.to_string_lossy().into_owned(),
                is_default,
            });
        }
    }
    shells
}

/// Whether `program` is a shell, and so should be started the way a terminal application starts
/// one — see [`crate::pty::spawn`], which is what the answer changes.
pub fn is_shell(program: &str) -> bool {
    let name = basename(program);
    CANDIDATES.iter().any(|candidate| *candidate == name) || name == basename(&default_program())
}

/// A shell's row label: its own name, with the extension Windows spells it with dropped.
fn label_of(name: &str) -> String {
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

/// The program's file name, which is how a shell is recognised — `$SHELL` is a path and a
/// candidate is a name.
fn basename(program: &str) -> String {
    Path::new(program)
        .file_name()
        .unwrap_or_else(|| OsStr::new(program))
        .to_string_lossy()
        .into_owned()
}

/// The `PATH` the user's own login shell has, which is not the one a desktop-launched Ubiq
/// inherits. Asked of the shell once per process: it costs a subprocess, and a machine does not
/// grow a new toolchain directory while a window is open.
#[cfg(unix)]
fn login_path() -> &'static [PathBuf] {
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let shell = default_program();
        // `-lic` rather than `-c`: the directories a toolchain installer adds are written into the
        // login and interactive files, not into a non-interactive shell's environment.
        let Ok(out) = std::process::Command::new(&shell)
            .args(["-lic", "printf %s \"$PATH\""])
            .output()
        else {
            return Vec::new();
        };
        let path = String::from_utf8_lossy(&out.stdout);
        std::env::split_paths(path.trim()).collect()
    })
}

/// Windows has no login shell to ask, so there is nothing beyond `PATH` and the usual homes.
#[cfg(windows)]
fn login_path() -> &'static [PathBuf] {
    &[]
}

/// Where a named program is on this machine: `PATH` first, then the login shell's own `PATH`, then
/// the usual homes.
///
/// The login shell's `PATH` is what finds a harness installed under the user's home — `~/.local/bin`,
/// a node prefix — which the environment a desktop launcher hands Ubiq does not name at all.
pub(crate) fn locate(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    let from_path = path
        .as_deref()
        .map(std::env::split_paths)
        .into_iter()
        .flatten();
    from_path
        .chain(login_path().iter().cloned())
        .chain(EXTRA_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shell_is_always_offered_and_marked() {
        let shells = available();
        let default: Vec<_> = shells.iter().filter(|shell| shell.is_default).collect();
        assert_eq!(
            default.len(),
            1,
            "exactly one row is the default: {shells:?}"
        );
        assert_eq!(default[0].program, default_program());
    }

    #[test]
    fn every_offered_shell_is_a_shell() {
        for shell in available() {
            assert!(
                is_shell(&shell.program),
                "{} is not recognised",
                shell.program
            );
            assert!(!shell.label.contains('/'), "a label is a name, not a path");
        }
    }

    /// What the login shell's `PATH` holds is this machine's business, so the assertion is the
    /// invariant instead: the probe still finds a shell, and the answer is computed once.
    #[test]
    #[cfg(unix)]
    fn the_login_path_is_asked_for_once_and_locate_still_finds_a_shell() {
        assert!(locate("sh").is_some(), "every unix machine has sh");
        assert!(
            std::ptr::eq(login_path(), login_path()),
            "the login shell is asked once per process"
        );
    }

    #[test]
    fn a_program_that_is_not_a_shell_is_not_started_as_one() {
        assert!(!is_shell("/usr/local/bin/claude"));
        assert!(!is_shell("codex"));
    }
}
