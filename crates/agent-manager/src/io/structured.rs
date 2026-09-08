//! The shared plumbing every structured (non-passthrough) bridge is built
//! on: spawning a harness's process with piped stdio instead of a PTY.
//!
//! This is **core** (always compiled): it only needs `std::process`, so it
//! builds under `--no-default-features` same as [`super::model`]. The PTY
//! runner in [`crate::run`] stays `pty`-gated and unrelated — passthrough
//! wires a real tty, structured bridges wire pipes and speak a protocol over
//! them.

use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::Result;
use crate::harness::{Harness, Launch};
use crate::io::IoBridge;
use crate::provision::Provisioned;

/// Spawn `launch` with piped stdin/stdout, cwd `cwd`.
///
/// Applies `launch.env_remove` (via [`Command::env_remove`]) and then
/// `launch.env` (via [`Command::env`]) on top of the inherited environment —
/// the same "inherit, minus `env_remove`, plus `env`" hygiene [`crate::run`]
/// applies for the PTY path, just via `std::process::Command` instead of
/// `portable_pty::CommandBuilder`. stderr is inherited (shown on `am`'s own
/// stderr) rather than piped, so harness diagnostics aren't silently
/// swallowed while a structured bridge is still shaking out in early steps.
///
/// This is the shared entry point every structured bridge uses to start its
/// underlying process; concrete bridges (C2/C3/C4) wrap the returned
/// [`Child`]'s stdin/stdout in their own protocol framing.
pub fn spawn_piped(launch: &Launch, cwd: &Path) -> Result<Child> {
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args);
    cmd.current_dir(cwd);

    // A confined launch carries the whole environment its policy computed, so
    // inheriting this process's would put back what the sandbox took out.
    if launch.env_clear {
        cmd.env_clear();
    }
    for var in &launch.env_remove {
        cmd.env_remove(var);
    }
    for (k, v) in &launch.env {
        cmd.env(k, v);
    }

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::inherit());

    let child = cmd.spawn()?;
    Ok(child)
}

/// The [`crate::io::AgentKill`] for a process [`spawn_piped`] started: it holds the pid
/// and nothing else, so it is `Send + Sync` and detachable from the bridge
/// that owns the [`Child`].
///
/// **The pid cannot go stale while the bridge is alive.** A bridge keeps its
/// `Child` unreaped until it is dropped, and an unreaped child keeps its pid —
/// a zombie on Unix, an open handle on Windows — so there is no window in
/// which this could name a process the operating system has since given to
/// somebody else.
///
/// The kill goes through the platform's own tool rather than a signal call:
/// `std` has no kill-by-pid, `libc::kill` is `unsafe` and this crate forbids
/// unsafe code, and one bounded process is cheaper than two platform
/// dependencies in a module that must keep building under
/// `--no-default-features`.
pub struct ProcessKill {
    pid: u32,
}

impl ProcessKill {
    /// A killer for `child`, taken while the bridge still owns it.
    pub fn new(child: &Child) -> Self {
        Self { pid: child.id() }
    }
}

impl crate::io::AgentKill for ProcessKill {
    fn kill(&self) -> Result<()> {
        let pid = self.pid.to_string();
        #[cfg(unix)]
        let mut cmd = {
            let mut cmd = Command::new("kill");
            cmd.args(["-KILL", pid.as_str()]);
            cmd
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut cmd = Command::new("taskkill");
            cmd.args(["/F", "/T", "/PID", pid.as_str()]);
            cmd
        };
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        // A non-zero status is "no such process" — it has already exited, which
        // is the state the caller asked for.
        let _ = cmd.status()?;
        tracing::debug!(pid = self.pid, "harness process killed");
        Ok(())
    }
}

/// Build the structured-I/O bridge for `harness`'s already-provisioned run.
///
/// A thin convenience wrapper — the real work is
/// [`Harness::structured_bridge`], which each harness implements (or, until
/// its bridge lands, inherits the trait's default "unsupported" error from).
pub fn run_structured(
    harness: &dyn Harness,
    provisioned: &Provisioned,
    cwd: &Path,
) -> Result<Box<dyn IoBridge>> {
    harness.structured_bridge(provisioned, cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn launch(program: &str, args: &[&str]) -> Launch {
        Launch {
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
            env_remove: Vec::new(),
            env_clear: false,
        }
    }

    #[test]
    fn spawn_piped_runs_a_trivial_command_to_success() {
        let cwd = std::env::current_dir().unwrap();
        let mut child = spawn_piped(&launch("true", &[]), &cwd).unwrap();
        let status = child.wait().unwrap();
        assert!(status.success());
    }

    #[test]
    fn spawn_piped_wires_piped_stdio() {
        let cwd = std::env::current_dir().unwrap();
        let l = launch("/bin/sh", &["-c", "cat"]);
        let mut child = spawn_piped(&l, &cwd).unwrap();

        // stdin/stdout must actually be pipes: write a line in, read it back.
        child
            .stdin
            .as_mut()
            .expect("stdin should be piped")
            .write_all(b"hello\n")
            .unwrap();
        drop(child.stdin.take());

        let mut out = String::new();
        child
            .stdout
            .as_mut()
            .expect("stdout should be piped")
            .read_to_string(&mut out)
            .unwrap();
        let status = child.wait().unwrap();

        assert_eq!(out, "hello\n");
        assert!(status.success());
    }

    /// `env_remove` strips a variable inherited from the parent; `env`
    /// injects a new one. Verified with `/usr/bin/printenv` rather than a
    /// shell: shells like bash fall back to a *default* `PATH` when the
    /// variable is entirely unset (POSIX startup behavior), which would mask
    /// `env_remove` actually working — `printenv` has no such fallback, it
    /// just reports what's in its own process env.
    ///
    /// PATH stands in for "a var inherited from the parent" since it's
    /// reliably present without mutating the test process's real
    /// environment (`std::env::set_var` is `unsafe` as of edition 2024, and
    /// this crate forbids unsafe code).
    /// The killer ends a process that would otherwise outlive the test, and
    /// the wait that follows is what a bridge's own teardown does — so this
    /// covers the whole of the abort path's contract: kill, then reap.
    #[test]
    #[cfg(unix)]
    fn a_process_kill_ends_the_child_it_names() {
        use crate::io::AgentKill;

        let cwd = std::env::current_dir().unwrap();
        let mut child = spawn_piped(&launch("/bin/sh", &["-c", "sleep 30"]), &cwd).unwrap();
        let killer = ProcessKill::new(&child);

        killer.kill().unwrap();

        let status = child.wait().unwrap();
        assert!(
            !status.success(),
            "a killed process does not exit successfully; status was {status:?}"
        );
    }

    #[test]
    fn spawn_piped_applies_env_remove() {
        assert!(
            std::env::var_os("PATH").is_some(),
            "test precondition: PATH must be set in the test process's env"
        );

        let cwd = std::env::current_dir().unwrap();
        let mut l = launch("/usr/bin/printenv", &["PATH"]);
        l.env_remove = vec!["PATH".to_string()];

        let mut child = spawn_piped(&l, &cwd).unwrap();
        drop(child.stdin.take());
        let mut out = String::new();
        child
            .stdout
            .as_mut()
            .expect("stdout should be piped")
            .read_to_string(&mut out)
            .unwrap();
        let status = child.wait().unwrap();

        // printenv exits non-zero and prints nothing when the named var
        // isn't in its environment.
        assert!(!status.success());
        assert_eq!(out, "");
    }

    #[test]
    fn spawn_piped_applies_env() {
        let cwd = std::env::current_dir().unwrap();
        let mut l = launch("/usr/bin/printenv", &["AM_STRUCTURED_TEST"]);
        l.env = vec![("AM_STRUCTURED_TEST".to_string(), "injected".to_string())];

        let mut child = spawn_piped(&l, &cwd).unwrap();
        drop(child.stdin.take());
        let mut out = String::new();
        child
            .stdout
            .as_mut()
            .expect("stdout should be piped")
            .read_to_string(&mut out)
            .unwrap();
        let status = child.wait().unwrap();

        assert!(status.success());
        assert_eq!(out, "injected\n");
    }
}
