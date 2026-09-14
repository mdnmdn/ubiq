//! Reach a drone over `ssh`: spawn the system client, shake hands on its stdio, and hand the two
//! halves to the same pump a socket dial uses.
//!
//! **Ubiq shells to the system `ssh` and never parses its output for structure.** What comes back
//! is an exit code and bytes — the bytes go verbatim into a failure sentence, and nothing here
//! reads them for fields, states or prompts. That is what separates this from `D43`: the thing
//! being driven is a program with a stable exit contract, not a program with a screen.
//!
//! **The process handle stays here.** `architecture.md` names the `TcpStream` in
//! `remote_connect.rs` as the sanctioned exception to "no descriptor in the interface"; the child
//! `ssh` is the same exception on the same terms. Nothing below this module's boundary — no state
//! type, no drawing module — ever sees a `Child`, a pipe or a path. What crosses out is a
//! [`Client`] and a [`ConnectFailure`].
//!
//! **Never on the GPUI thread.** Spawning, the handshake read and the stderr drain are all
//! blocking; `AppState::try_connect_remote` runs this inside `cx.background_spawn` exactly as it
//! runs a socket dial.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use ubiq_proto::bus::{self, Client};
use ubiq_proto::carrier::{self, DroneIdentity, HandshakeError};
use ubiq_proto::settings::{SshAuth, SshProfile};

use super::remote_connect::{ConnectFailure, spawn_pump};

/// The drone binary as named on the far machine.
///
/// A bare name rather than a path, resolved by the remote `PATH`: phase 9 is what deploys a drone
/// to a known location and caches where it put it, and until there is a deployer there is nothing
/// to point a path setting at. A user whose drone is elsewhere puts it on their `PATH`, which is
/// the same answer `ssh <host> <command>` gives for every other command.
const DRONE_COMMAND: &str = "ubiq-drone";

/// How long `ssh` may spend on its own connection before giving up. Mirrors
/// `remote_connect::CONNECT_TIMEOUT` — a wrong address must not hang the modal for whatever the
/// platform's own ceiling is.
const CONNECT_TIMEOUT_SECS: u64 = 6;

/// How long the whole launch — connect, authenticate, run the drone, exchange the handshake — may
/// take before the child is killed and the modal is told.
///
/// Generous, because everything slow inside it is legitimate: a far machine, a key agent, a drone
/// binary being paged in. What it exists to stop is the case with no timer of its own at all — an
/// `ssh` sitting on a prompt nothing will ever answer, with a pipe the reader below blocks on
/// forever.
const HANDSHAKE_TIMEOUT_SECS: u64 = 45;

/// How much of the child's standard error is kept for a failure sentence. Enough for OpenSSH's
/// own diagnosis and a shell's "command not found"; short enough that a chatty `-v` cannot make
/// this process buffer without limit.
const MAX_STDERR: usize = 4 * 1024;

/// What a drone dial needs: the profile to reach it with, and the folder it should serve.
#[derive(Clone, Debug)]
pub struct SshDialParams {
    pub profile: SshProfile,
    /// The folder on the far machine, as the user typed it. Empty is the drone's own default.
    pub root: String,
    /// Where the askpass helper reads the profile's secret from — Ubiq's config root, as the
    /// local host announced it in its `HostInfo` greeting. `None` lets the helper resolve its
    /// own, which is right whenever this window is attached to a host that was started without
    /// `--config-root`.
    pub config_root: Option<String>,
}

/// Dial a drone over `ssh` and, on success, hand back the `Client` the bus registers plus what
/// the drone said about itself.
///
/// Blocking from end to end. Every failure kills the child and reaps it, so a refused handshake
/// leaves no `ssh` behind.
pub fn dial_ssh(params: &SshDialParams) -> Result<(Client, DroneIdentity), ConnectFailure> {
    let mut child = spawn_ssh(params)?;

    // Taken before anything is read: the pump owns these for the life of the session, and the
    // handshake below runs on the same two halves the pump will inherit.
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ConnectFailure::Io("ssh gave no standard output".to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ConnectFailure::Io("ssh gave no standard input".to_string()))?;
    let stderr = child.stderr.take();

    // Drained from the moment the child exists: a pipe nobody reads fills and blocks `ssh` itself,
    // and what it says is the whole of what a failure has to report.
    let noise = drain_stderr(stderr);

    // The child has no timer of its own for the case where it neither connects nor exits — a
    // host-key prompt with no terminal to ask at, say. This kills it, which ends the read below.
    let watchdog = Watchdog::arm(&mut child);

    let identity = carrier::welcome(&mut stdout, &mut stdin, env!("CARGO_PKG_VERSION"));
    watchdog.disarm();

    let identity = match identity {
        Ok(identity) => identity,
        Err(error) => {
            let _ = child.kill();
            let status = child.wait().ok();
            return Err(handshake_failure(error, status, &noise.take()));
        }
    };

    // From here the child is the session. Its stderr keeps draining into the same buffer, which
    // nothing reads again — the thread ends when `ssh` closes the pipe, which is how a session
    // that dies later is noticed by the pump rather than by this function.
    let (client, detached) = bus::detached();
    spawn_pump(
        stdout,
        stdin,
        // A dead `ssh` that failed the write has already closed the stdin the reader sits on, so
        // the `Closer` a socket needs is not load-bearing here (the ledger's §2 `NoCloser`
        // reasoning). Killing is still what this does, for the other direction: when the *window*
        // ends the session, the far drone has to go with it rather than linger against an `ssh`
        // nobody is reading.
        Box::new(move || {
            let _ = child.kill();
            let _ = child.wait();
        }),
        detached,
    );
    Ok((client, identity))
}

/// Build the argv and start `ssh` with all three streams piped.
fn spawn_ssh(params: &SshDialParams) -> Result<Child, ConnectFailure> {
    let profile = &params.profile;
    // A profile's fields reach `argv`, and `argv` is not a shell — but a value beginning with `-`
    // would be read by `ssh` as an option of its own, and a control character has no business in
    // a hostname. Refused before the process exists, on `remote_connect::is_typable`'s reasoning.
    let target = ssh_target(profile);
    if target.is_empty() || target.starts_with('-') || target.chars().any(|c| c.is_control()) {
        return Err(ConnectFailure::Untypable("host"));
    }
    if params.root.chars().any(|c| c.is_control()) {
        return Err(ConnectFailure::Untypable("folder"));
    }

    let mut command = Command::new("ssh");
    // `ConfigAlias` defers wholly to `~/.ssh/config`: a port, a user or a batch-mode option
    // written here would override exactly the file the user chose this variant to be governed by.
    if !matches!(profile.auth, SshAuth::ConfigAlias) {
        command.arg("-p").arg(profile.port.to_string());
        if !profile.user.trim().is_empty() {
            command.arg("-l").arg(profile.user.trim());
        }
        if let SshAuth::KeyFile { path, .. } = &profile.auth
            && !path.trim().is_empty()
        {
            command.arg("-i").arg(path.trim());
        }
        command
            .arg("-o")
            .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"));
        // Batch mode where there is nothing to answer a prompt with: an agent key or an
        // unencrypted key file either works or does not, and failing at once beats blocking on a
        // prompt no terminal will ever see. Where a secret *is* filed, batch mode would refuse the
        // askpass helper that supplies it, so it is left off.
        if !profile.auth.has_secret() {
            command.arg("-o").arg("BatchMode=yes");
        }
    }
    // No pseudo-terminal, ever: the far end's standard output is a length-prefixed binary stream,
    // and a tty between the two would rewrite it.
    command.arg("-T").arg(target);
    command.arg(remote_command(&params.root));

    apply_askpass(&mut command, params);

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A session of its own, so an interrupt aimed at the terminal Ubiq was launched from does not
    // take the drone with it. `SSH_ASKPASS_REQUIRE=force` above is what actually keeps the
    // password off a terminal; this only keeps the child out of the window's job control.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    command
        .spawn()
        .map_err(|error| ConnectFailure::Unreachable(format!("ssh could not be started: {error}")))
}

/// What `ssh` is pointed at: the alias for a `ConfigAlias` profile, the host otherwise. The user
/// rides as `-l` rather than as `user@host`, so a name with an `@` in it is not ambiguous.
fn ssh_target(profile: &SshProfile) -> String {
    profile.host.trim().to_string()
}

/// The command the far machine runs. One string, because that is what `ssh` hands its login
/// shell — so the root is single-quoted for that shell, which is the only quoting this needs.
fn remote_command(root: &str) -> String {
    let root = root.trim();
    if root.is_empty() {
        format!("{DRONE_COMMAND} --stdio")
    } else {
        format!("{DRONE_COMMAND} --stdio --root {}", shell_quote(root))
    }
}

/// One argument, quoted for a POSIX login shell. Single quotes take everything literally, and the
/// one character they cannot carry is closed, escaped and reopened.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Point `ssh` at Ubiq's own binary for the password prompt, when the profile has a secret filed.
///
/// **The material is never in `argv`, never in a file, and never on the bus.** What crosses here
/// is the profile *id* — a reference, exactly as it is on the settings record — in the child's
/// environment. The helper, which is this same executable in the mode
/// `ubiq_app::askpass` answers, opens the host's secret store under the config root and writes the
/// material on its own standard output, which is where OpenSSH reads an askpass answer from. So
/// the material's whole path is keychain → helper stdout → `ssh`: it never enters this process at
/// all, which is a stronger statement than keeping it out of `argv`.
///
/// **Why the helper and not a message.** `crates/ubiq` cannot read the host's keychain, so the
/// alternative was a new message carrying the secret back to the interface in a `Secret`. That
/// would be legal under `D65` and still wrong: it would make the interface hold material for the
/// first time, to hand it to a child process it could have let fetch its own. `D124`'s split — the
/// list is the interface's, the material is the host's — survives untouched this way, and no
/// message was added.
fn apply_askpass(command: &mut Command, params: &SshDialParams) {
    if !params.profile.auth.has_secret() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        // Without a path to this binary there is no helper to point at. `ssh` then prompts, finds
        // no terminal, and fails with its own sentence — which is the right failure to show.
        tracing::warn!("no path to this executable: ssh will have no askpass helper");
        return;
    };
    command.env("SSH_ASKPASS", exe);
    // OpenSSH 8.4 and later: use the helper whatever the terminal and `DISPLAY` say. Older
    // clients consult `DISPLAY` instead, which is why one is set below when there is none.
    command.env("SSH_ASKPASS_REQUIRE", "force");
    if std::env::var_os("DISPLAY").is_none() {
        command.env("DISPLAY", ":0");
    }
    command.env(ASKPASS_PROFILE_VAR, params.profile.id.to_string());
    if let Some(root) = &params.config_root {
        command.env(ASKPASS_ROOT_VAR, root);
    }
}

/// The environment variable that puts the binary into askpass mode, and names which profile's
/// secret is wanted. Read by `ubiq_app::askpass`; a reference, never material.
pub const ASKPASS_PROFILE_VAR: &str = "UBIQ_ASKPASS_PROFILE";

/// Which config root the helper opens its secret store under. Absent, the helper resolves its own
/// the way any other launch does.
pub const ASKPASS_ROOT_VAR: &str = "UBIQ_ASKPASS_ROOT";

/// Turn a failed handshake, the child's exit status and whatever it said on standard error into
/// one of the vocabulary the modal already draws.
///
/// A refusal is the drone's own sentence, verbatim: it was written to be read in a modal, and
/// rewording it here would lose the two schema numbers it names.
fn handshake_failure(
    error: HandshakeError,
    status: Option<std::process::ExitStatus>,
    stderr: &str,
) -> ConnectFailure {
    if let HandshakeError::Refused(reason) = error {
        return ConnectFailure::Refused(reason);
    }
    let code = status.and_then(|status| status.code());
    let said = stderr.trim();
    match code {
        // `ssh`'s own failure class — unreachable, refused, or an authentication that did not
        // work. Its sentence is the useful one, so it is what the modal shows.
        Some(255) | None if !said.is_empty() => ConnectFailure::Unreachable(said.to_string()),
        Some(255) | None => ConnectFailure::Unreachable(format!("ssh failed: {error}")),
        // `ssh` connected and the far side ran something: a non-zero exit is the *drone* failing,
        // most often by not being on the remote `PATH` at all.
        Some(code) if code != 0 && !said.is_empty() => {
            ConnectFailure::Refused(format!("the drone exited {code}: {said}"))
        }
        Some(code) if code != 0 => ConnectFailure::Refused(format!("the drone exited {code}")),
        _ if !said.is_empty() => ConnectFailure::Refused(said.to_string()),
        _ => ConnectFailure::Refused(error.to_string()),
    }
}

/// Read the child's standard error into a capped buffer on a thread of its own, logging each line
/// as it arrives. The buffer is what a failure quotes; the log is what a session that dies an hour
/// later leaves behind.
fn drain_stderr(stderr: Option<std::process::ChildStderr>) -> Noise {
    let kept = Arc::new(Mutex::new(String::new()));
    let Some(stderr) = stderr else {
        return Noise(kept);
    };
    let into = kept.clone();
    thread::Builder::new()
        .name("ubiq-ssh-stderr".to_string())
        .spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                tracing::debug!("ssh: {line}");
                let mut held = into.lock().unwrap_or_else(|held| held.into_inner());
                if held.len() < MAX_STDERR {
                    if !held.is_empty() {
                        held.push('\n');
                    }
                    held.push_str(&line);
                }
            }
        })
        .expect("the ssh stderr thread");
    Noise(kept)
}

/// What the child said on standard error, so far.
struct Noise(Arc<Mutex<String>>);

impl Noise {
    fn take(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(|held| held.into_inner())
            .clone()
    }
}

/// Kills the child if the handshake has not finished in time.
///
/// A thread rather than a read deadline because a pipe has no timeout to set: the only way to
/// unblock a read on one is to close the far end, and killing the child does exactly that.
struct Watchdog {
    done: Arc<Mutex<bool>>,
}

impl Watchdog {
    fn arm(child: &mut Child) -> Self {
        let done = Arc::new(Mutex::new(false));
        let watching = done.clone();
        // A handle that can kill without owning: `Child::kill` needs `&mut`, so what crosses to
        // the thread is the raw id and the platform's own way of ending it.
        let killer = Killer::of(child);
        thread::Builder::new()
            .name("ubiq-ssh-watchdog".to_string())
            .spawn(move || {
                let deadline = std::time::Instant::now()
                    + std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS);
                while std::time::Instant::now() < deadline {
                    if *watching.lock().unwrap_or_else(|held| held.into_inner()) {
                        return;
                    }
                    thread::sleep(std::time::Duration::from_millis(200));
                }
                if !*watching.lock().unwrap_or_else(|held| held.into_inner()) {
                    tracing::warn!("ssh did not finish the handshake in time: ending it");
                    killer.kill();
                }
            })
            .expect("the ssh watchdog thread");
        Self { done }
    }

    fn disarm(&self) {
        *self.done.lock().unwrap_or_else(|held| held.into_inner()) = true;
    }
}

/// Ending a child from a thread that does not own it.
struct Killer {
    pid: u32,
}

impl Killer {
    fn of(child: &Child) -> Self {
        Self { pid: child.id() }
    }

    #[cfg(unix)]
    fn kill(&self) {
        // No `libc` in this crate's graph, and one flag does not earn one: `kill` is on every
        // machine that has `ssh`.
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(self.pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    #[cfg(windows)]
    fn kill(&self) {
        let _ = Command::new("taskkill")
            .args(["/PID", &self.pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::ids::SshProfileId;

    fn profile(auth: SshAuth) -> SshProfile {
        SshProfile {
            id: SshProfileId::generate(),
            name: "build box".to_string(),
            host: "build.example".to_string(),
            port: 2222,
            user: "ubiq".to_string(),
            auth,
        }
    }

    /// The remote command is one string a login shell parses, so a folder with a space or a quote
    /// in it has to survive that parse intact.
    #[test]
    fn the_remote_command_quotes_the_root_for_a_shell() {
        assert_eq!(remote_command(""), "ubiq-drone --stdio");
        assert_eq!(
            remote_command("/srv/my work"),
            "ubiq-drone --stdio --root '/srv/my work'"
        );
        assert_eq!(
            remote_command("/srv/it's"),
            "ubiq-drone --stdio --root '/srv/it'\\''s'"
        );
    }

    /// A config alias is governed by `~/.ssh/config` alone, so nothing else may be its target.
    #[test]
    fn the_target_is_the_host_or_the_alias() {
        assert_eq!(ssh_target(&profile(SshAuth::Agent)), "build.example");
        assert_eq!(ssh_target(&profile(SshAuth::ConfigAlias)), "build.example");
    }

    /// The drone's refusal sentence names both schema numbers and is written for a modal. It
    /// reaches one unchanged.
    #[test]
    fn a_refusal_carries_the_drones_sentence_verbatim() {
        let failure = handshake_failure(
            HandshakeError::Refused("This Ubiq speaks 1; the drone speaks 8.".to_string()),
            None,
            "noise nobody needs",
        );
        match failure {
            ConnectFailure::Refused(reason) => {
                assert_eq!(reason, "This Ubiq speaks 1; the drone speaks 8.");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// `ssh`'s own exit code is 255, and its sentence is the one worth showing; anything else
    /// non-zero is the far command failing, which reads differently.
    #[test]
    fn an_exit_code_decides_which_half_failed() {
        let wire = || {
            HandshakeError::Wire(ubiq_proto::wire::WireError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "eof",
            )))
        };
        let unreachable = handshake_failure(
            wire(),
            Some(exit_status(255)),
            "ssh: connect to host build.example port 2222: Connection refused",
        );
        assert!(
            matches!(unreachable, ConnectFailure::Unreachable(said) if said.contains("refused"))
        );

        let refused = handshake_failure(wire(), Some(exit_status(127)), "ubiq-drone: not found");
        assert!(matches!(refused, ConnectFailure::Refused(said) if said.contains("not found")));
    }

    #[cfg(unix)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }
}
