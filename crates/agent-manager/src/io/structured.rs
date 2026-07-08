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

use crate::harness::{Harness, Launch};
use crate::io::IoBridge;
use crate::provision::Provisioned;
use crate::Result;

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
