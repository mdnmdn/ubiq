//! Which shells this machine actually has, and how one is started.
//!
//! The interface may not look for itself — a program on disk is a local fact, and no path crosses
//! into UI code — so the list is made here and answered over the bus as
//! [`Message::ShellList`](ubiq_proto::messages::Message::ShellList).
//!
//! It is a fixed set of known names checked for existence, not a launcher for anything on the
//! machine: a menu of three or four shells is a bounded surface, and an open one would be a
//! configuration feature nobody asked for.

use std::ffi::{OsStr, OsString};
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

/// The `PATH` this process should really be running with: what it already has, then the login
/// shell's own `PATH`, then the usual homes ([`EXTRA_DIRS`]) — de-duplicated, order preserved.
///
/// Pure and side-effect-free so it can be tested without a login shell; [`repair_path`] is the
/// only caller that feeds it the real environment and writes the result back.
fn effective_path(current: &OsStr, login: &[PathBuf], extra: &[&str]) -> OsString {
    let mut seen = std::collections::HashSet::new();
    let dirs: Vec<PathBuf> = std::env::split_paths(current)
        .chain(login.iter().cloned())
        .chain(extra.iter().map(PathBuf::from))
        .filter(|dir| seen.insert(dir.clone()))
        .collect();
    std::env::join_paths(dirs).unwrap_or_else(|_| current.to_os_string())
}

/// Repair this process's own `PATH` from the login shell, once, at startup.
///
/// A desktop launcher (Finder, the dock) hands Ubiq a thin `PATH` — see the module doc — and
/// nothing before this point has done anything about it, so every bare-name spawn inside
/// `agent-manager` (`Command::new("claude")` and its siblings: `discover_models`,
/// `discover_thinking`, `version()`, every harness launch) fails under exactly that launch even
/// though the same binaries are found fine from a terminal.
///
/// # Safety / soundness
/// Must be called exactly once, before any other thread is spawned in this process. Mutating the
/// environment ([`std::env::set_var`] is `unsafe` since Rust 2024) races any concurrent reader —
/// the only window in which it is sound is before a second thread exists to read it. The sole
/// call site is the very start of `ubiq_app::run`, ahead of the coordinator thread, ahead of the
/// GPUI event loop, ahead of anything else this process spawns.
pub fn repair_path() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let composed = effective_path(&current, login_path(), EXTRA_DIRS);
    if composed == current {
        return;
    }
    // SAFETY: see above — this runs once, at process startup, before any other thread exists.
    unsafe { std::env::set_var("PATH", &composed) };
    tracing::debug!(path = %composed.to_string_lossy(), "repaired process PATH");
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

    #[cfg(unix)]
    fn path(dirs: &[&str]) -> OsString {
        std::env::join_paths(dirs.iter().map(PathBuf::from)).unwrap()
    }

    #[cfg(unix)]
    fn paths(dirs: &[&str]) -> Vec<PathBuf> {
        dirs.iter().map(PathBuf::from).collect()
    }

    /// A directory the login shell adds that `PATH` doesn't have yet is appended, after what was
    /// already there.
    #[test]
    #[cfg(unix)]
    fn effective_path_appends_new_login_directories_after_the_existing_ones() {
        let current = path(&["/usr/bin", "/bin"]);
        let login = paths(&["/usr/bin", "/opt/homebrew/bin"]);
        let composed = effective_path(&current, &login, &[]);
        assert_eq!(
            std::env::split_paths(&composed).collect::<Vec<_>>(),
            paths(&["/usr/bin", "/bin", "/opt/homebrew/bin"])
        );
    }

    /// A login shell that names nothing new, and no extra dirs either, leaves `PATH` untouched —
    /// not merely equal, but the very same value, so [`repair_path`] can skip the write.
    #[test]
    #[cfg(unix)]
    fn effective_path_is_a_no_op_when_the_login_shell_adds_nothing_new() {
        let current = path(&["/usr/bin", "/bin"]);
        let login = paths(&["/bin", "/usr/bin"]);
        let composed = effective_path(&current, &login, &[]);
        assert_eq!(composed, current);
    }

    /// Existing entries always come first, whatever order the login shell and the extra homes
    /// name their own duplicates in.
    #[test]
    #[cfg(unix)]
    fn effective_path_keeps_existing_entries_first_and_drops_duplicates() {
        let current = path(&["/opt/homebrew/bin", "/bin"]);
        let login = paths(&["/bin", "/opt/homebrew/bin", "/usr/local/bin"]);
        let composed = effective_path(
            &current,
            &login,
            &["/usr/local/bin", "/run/current-system/sw/bin"],
        );
        assert_eq!(
            std::env::split_paths(&composed).collect::<Vec<_>>(),
            paths(&[
                "/opt/homebrew/bin",
                "/bin",
                "/usr/local/bin",
                "/run/current-system/sw/bin",
            ])
        );
    }
}
