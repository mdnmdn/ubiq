//! Spawns and supervises the harness child process.
//!
//! Owns the process lifecycle for Phase-1 passthrough: allocate a PTY, spawn
//! the provisioned [`Launch`] into it, forward the local tty
//! ([`crate::io::passthrough`]), resize the PTY on `SIGWINCH`, wait for the
//! child, and propagate its exit code. Also owns cleanup of the ephemeral
//! config dir the provisioner created (see `_docs/io-modes.md`
//! §passthrough and `_docs/cli.md` §"Exit codes & passthrough
//! fidelity").
//!
//! Spawning is split out into [`spawn_in_pty`] so it can be exercised in
//! tests without a controlling terminal (no real tty is needed to open a
//! PTY, spawn a child into it, and wait for its exit status).

use std::io::IsTerminal;
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::harness::Launch;
use crate::provision::Provisioned;
use crate::Result;

/// Spawn `launch` in a fresh PTY with working dir `cwd` and initial size
/// `(rows, cols)`. Returns the child process handle and the PTY master.
///
/// `pub` (rather than crate-private) so integration tests — which compile as
/// a separate crate — can drive it directly against a fake harness without a
/// controlling terminal.
pub fn spawn_in_pty(
    launch: &Launch,
    cwd: &Path,
    size: (u16, u16),
) -> Result<(Box<dyn Child + Send + Sync>, Box<dyn MasterPty + Send>)> {
    let (rows, cols) = size;
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new(&launch.program);
    cmd.args(&launch.args);
    cmd.cwd(cwd);
    apply_env(&mut cmd, launch);

    let child = pair.slave.spawn_command(cmd)?;
    // Close the parent's copy of the slave fd/handle now that the child owns
    // its end; otherwise the master's reader never sees EOF after the child
    // exits (the slave would still have an open reference).
    drop(pair.slave);

    Ok((child, pair.master))
}

/// Apply `launch`'s env hygiene to `cmd`: `CommandBuilder::new` seeds `cmd`
/// with the current process's environment already, so removing
/// `launch.env_remove` and then setting `launch.env` yields exactly
/// "inherit the parent env, minus `env_remove`, plus `env`".
fn apply_env(cmd: &mut CommandBuilder, launch: &Launch) {
    for var in &launch.env_remove {
        cmd.env_remove(var);
    }
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }
}

/// Run `provisioned`'s launch to completion in a PTY, forwarding the local
/// tty, resizing on `SIGWINCH`, and propagating the child's exit code.
/// Cleans up the ephemeral config dir afterwards unless `keep_config` or the
/// dir was pinned (`!provisioned.ephemeral`). Returns the child's exit code.
pub fn run(provisioned: &Provisioned, cwd: &Path, keep_config: bool) -> Result<i32> {
    let (child, master) = spawn_in_pty(&provisioned.launch, cwd, terminal_size())?;
    let code = run_with(child, master, cwd)?;

    cleanup(provisioned, keep_config);

    Ok(code)
}

/// The tty-forwarding + wait part of [`run`], factored out so it can be
/// unit-tested against an already-spawned child/master (e.g. from
/// [`spawn_in_pty`]) without duplicating the pump/resize/wait wiring.
fn run_with(
    mut child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    _cwd: &Path,
) -> Result<i32> {
    let pty_reader = master.try_clone_reader()?;
    let pty_writer = master.take_writer()?;

    // The resize watcher needs to reach back into the master after pumping
    // has already taken its reader/writer, so it's shared behind a mutex
    // (MasterPty isn't Sync on its own, only the Box is Send).
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(master));

    let raw_guard = crate::io::pump(pty_reader, pty_writer);
    spawn_resize_watcher(Arc::clone(&master));

    let status = child.wait()?;
    let code = status.exit_code() as i32;

    // Restore the local terminal's cooked mode as soon as the child is
    // done, before we do any cleanup work.
    drop(raw_guard);

    Ok(code)
}

/// Local terminal size as `(rows, cols)` for the PTY's initial size; a sane
/// default when stdout isn't attached to a terminal (headless / piped /
/// under `cargo test`). Note `crossterm::terminal::size()` returns
/// `(cols, rows)` — [`PtySize`] wants `rows, cols`, so callers must swap.
fn terminal_size() -> (u16, u16) {
    if std::io::stdout().is_terminal()
        && let Ok((cols, rows)) = crossterm::terminal::size()
    {
        return (rows, cols);
    }
    (24, 80)
}

/// Spawn a detached background thread that resizes `master` to match the
/// local terminal on each `SIGWINCH`. Harmless when not attached to a tty:
/// no real terminal means no `SIGWINCH` ever arrives, so the thread just
/// blocks forever on the signal iterator and dies with the process.
fn spawn_resize_watcher(master: Arc<Mutex<Box<dyn MasterPty + Send>>>) {
    std::thread::spawn(move || {
        let mut signals =
            match signal_hook::iterator::Signals::new([signal_hook::consts::SIGWINCH]) {
                Ok(s) => s,
                Err(_) => return,
            };
        for _ in signals.forever() {
            if !std::io::stdout().is_terminal() {
                continue;
            }
            let Ok((cols, rows)) = crossterm::terminal::size() else {
                continue;
            };
            if let Ok(master) = master.lock() {
                let _ = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
        }
    });
}

/// Best-effort removal of the ephemeral config dir, unless it was pinned
/// (`!ephemeral`) or the caller asked to keep it (`keep_config`). Errors are
/// swallowed: cleanup is a courtesy, not something worth failing the run
/// over after the child has already produced its result.
fn cleanup(provisioned: &Provisioned, keep_config: bool) {
    if provisioned.ephemeral && !keep_config {
        let _ = std::fs::remove_dir_all(&provisioned.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `env_remove` strips a variable that's already part of the base
    /// environment `CommandBuilder::new` captures from the current process,
    /// and `env` injects a new one — exercised directly against the
    /// `CommandBuilder` (via its safe `get_env` accessor) rather than by
    /// mutating the test process's real environment, since
    /// `std::env::set_var` is `unsafe` and this crate forbids unsafe code.
    /// `PATH` stands in for "a var inherited from the parent" since it's
    /// reliably present without us having to set anything.
    #[test]
    fn apply_env_removes_and_injects() {
        assert!(
            std::env::var_os("PATH").is_some(),
            "test precondition: PATH must be set in the test process's env"
        );

        let launch = Launch {
            program: "true".to_string(),
            args: vec![],
            env: vec![("CLAUDE_CONFIG_DIR".to_string(), "/tmp/x".to_string())],
            env_remove: vec!["PATH".to_string()],
        };

        let mut cmd = CommandBuilder::new(&launch.program);
        apply_env(&mut cmd, &launch);

        assert_eq!(cmd.get_env("PATH"), None, "PATH should have been removed");
        assert_eq!(
            cmd.get_env("CLAUDE_CONFIG_DIR"),
            Some(std::ffi::OsStr::new("/tmp/x")),
            "CLAUDE_CONFIG_DIR should have been injected"
        );
    }
}
