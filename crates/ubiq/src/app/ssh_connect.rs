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

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::Deserialize;
use ubiq_proto::bus::{self, Client};
use ubiq_proto::carrier::{self, DroneIdentity, HandshakeError};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::settings::{DronePreset, SshAuth, SshProfile};

use super::remote_connect::{ConnectFailure, spawn_pump};

/// The drone binary as named on the far machine, when nothing more specific is known.
///
/// A bare name, resolved by the remote `PATH` — what every reach of a drone used before phase 9's
/// deployer existed, and still the fallback [`remote_command`] runs when [`SshDialParams::drone_path`]
/// is `None`. A saved `RemoteCarrier::Ssh`'s own `drone_path` — typed by hand, or written back by
/// [`ensure_drone`] after it uploads one — is what a caller prefers over this instead.
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

/// What a drone dial needs: the profile to reach it with, the folder it should serve, and how
/// long it should live once this session lets go of it.
#[derive(Clone, Debug)]
pub struct SshDialParams {
    pub profile: SshProfile,
    /// The folder on the far machine, as the user typed it. Empty is the drone's own default.
    pub root: String,
    /// Attached, session-lingering, or managed — see [`DronePreset`] and [`remote_command`].
    pub preset: DronePreset,
    /// The id the drone should announce [`Self::root`] under, for a dial a project's `runs_on`
    /// started. Passing it is what makes the drone's catalogue answer name the row the local one
    /// already holds — one row in the list whose host attribution moves, not two (`D116`).
    /// `None` for a dial the user typed into the connect modal, which has no project behind it.
    pub root_id: Option<ProjectId>,
    /// A `--linger` in seconds overriding the preset's own, as a project's [`DroneOrigin`] may
    /// carry. `None` takes the preset's default.
    ///
    /// [`DroneOrigin`]: ubiq_proto::projects::DroneOrigin
    pub linger_secs: Option<u64>,
    /// Where the drone binary sits on the far machine, when it is not on the remote `PATH` —
    /// [`RemoteCarrier::Ssh`]'s own field, threaded through unchanged. `None` runs
    /// [`DRONE_COMMAND`] and leaves it to the remote `PATH`.
    ///
    /// [`RemoteCarrier::Ssh`]: ubiq_proto::settings::RemoteCarrier::Ssh
    pub drone_path: Option<String>,
    /// Where the askpass helper reads the profile's secret from — Ubiq's config root, as the
    /// local host announced it in its `HostInfo` greeting. `None` lets the helper resolve its
    /// own, which is right whenever this window is attached to a host that was started without
    /// `--config-root`.
    pub config_root: Option<String>,
}

/// What a successful dial hands back: the `Client` the bus registers, what the drone said about
/// itself, and the remote path it was actually launched at.
///
/// `drone_path` is the field a caller saves. `None` is the remote `PATH`'s own [`DRONE_COMMAND`];
/// `Some` is either the path the caller already had or the one [`ensure_drone`] has just
/// uploaded — and saving that second case is what makes a deploy happen once rather than on every
/// connect to the same machine.
pub struct SshDial {
    pub client: Client,
    pub identity: DroneIdentity,
    pub drone_path: Option<String>,
}

/// Dial a drone over `ssh`, deploying one first only when the far machine turns out not to have
/// one.
///
/// **The remote `PATH` is tried first; the deployer is the fallback.** A drone installed by hand
/// on the far `PATH` is all phase 5 ever had, and it still works: this dials exactly as it did,
/// and reaches [`ensure_drone`] only once the far shell has answered that there is nothing named
/// [`DRONE_COMMAND`] to run. Deploying eagerly would invert that — a manifest with no entry for
/// the probed triple, as this tree's empty one has for every triple, turns every working connect
/// into a refusal naming `just drone-build` — so the order is the decision here, not a tuning.
///
/// The re-dial runs once, and only when [`SshDialParams::drone_path`] was `None`: an explicit path
/// is the caller's own answer to this same question and is never second-guessed. Any other failure
/// comes back unchanged, and so does the deploy's own — a deploy that finds no built binary
/// reports *its* sentence rather than the "not found" that provoked it, because that is the one
/// that says what to do next.
///
/// Blocking from end to end, deploy included.
pub fn dial_ssh(params: &SshDialParams) -> Result<SshDial, ConnectFailure> {
    dial_ssh_reporting(params, &mut |_step| {})
}

/// [`dial_ssh`], reporting the deployer's steps through `report` when it reaches one — the same
/// callback [`ensure_drone_reporting`] takes, so a modal can say which step a slow first connect
/// is on. A dial that never deploys never calls it.
pub fn dial_ssh_reporting(
    params: &SshDialParams,
    report: &mut dyn FnMut(DeployStep),
) -> Result<SshDial, ConnectFailure> {
    let given = params.drone_path.clone();
    match dial_once(params, given.as_deref()) {
        Ok((client, identity)) => Ok(SshDial {
            client,
            identity,
            drone_path: given,
        }),
        Err(refusal) => {
            if !deploy_after(given.as_deref(), refusal.drone_missing) {
                return Err(refusal.failure);
            }
            // No cached path to check: the one thing known here is that the bare command is not on
            // the far `PATH`, so the deploy starts at its probe.
            let deployed = ensure_drone_reporting(
                &params.profile,
                params.config_root.as_deref(),
                None,
                report,
            )
            .map_err(|deploy_failed| deploy_refusal(deploy_failed, refusal.failure))?;
            let (client, identity) = dial_once(params, Some(&deployed)).map_err(|it| it.failure)?;
            Ok(SshDial {
                client,
                identity,
                drone_path: Some(deployed),
            })
        }
    }
}

/// Whether a failed first attempt is answered by deploying a drone and dialling again.
///
/// Two conditions, both necessary. `given` being `Some` means the caller already named where the
/// binary is — a saved path or one typed by hand — and a dial that fails against a named path
/// fails; replacing it would throw away the user's own answer. `drone_missing` being false means
/// the far side ran something, so there is nothing an upload would fix.
fn deploy_after(given: Option<&str>, drone_missing: bool) -> bool {
    given.is_none() && drone_missing
}

/// Which sentence a dial that fell back to the deployer reports when the deploy itself failed:
/// the deploy's own, with the "not found" that provoked it kept in the log.
///
/// The deploy's sentence is the one that says what to do next — a triple with no entry in the
/// manifest names itself and `just drone-build` — where the original says only that the far
/// `PATH` has no drone, which is exactly the thing the deploy was already answering.
fn deploy_refusal(deployed: ConnectFailure, provoked_by: ConnectFailure) -> ConnectFailure {
    tracing::warn!("deploying a drone failed after {provoked_by}: {deployed}");
    deployed
}

/// A failed dial, plus whether the far side's answer was that it has no drone — the one failure
/// [`dial_ssh_reporting`] retries rather than reports.
struct DialFailure {
    failure: ConnectFailure,
    drone_missing: bool,
}

impl DialFailure {
    /// A failure with nothing to retry: everything that goes wrong before the far side has run
    /// anything — an untypable host, an `ssh` that would not start, a pipe that is not there.
    fn plain(failure: ConnectFailure) -> Self {
        Self {
            failure,
            drone_missing: false,
        }
    }
}

/// One `ssh`, one handshake. On success the child becomes the session; on failure it is killed and
/// reaped, so a refused handshake leaves no `ssh` behind.
fn dial_once(
    params: &SshDialParams,
    drone_path: Option<&str>,
) -> Result<(Client, DroneIdentity), DialFailure> {
    let command = remote_command(
        &params.root,
        params.preset,
        params.root_id,
        params.linger_secs,
        drone_path,
    );
    let mut child = spawn_ssh(params, &command).map_err(DialFailure::plain)?;

    // Taken before anything is read: the pump owns these for the life of the session, and the
    // handshake below runs on the same two halves the pump will inherit.
    let mut stdout = child.stdout.take().ok_or_else(|| {
        DialFailure::plain(ConnectFailure::Io(
            "ssh gave no standard output".to_string(),
        ))
    })?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        DialFailure::plain(ConnectFailure::Io("ssh gave no standard input".to_string()))
    })?;
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
            let code = status.and_then(|status| status.code());
            return Err(DialFailure {
                drone_missing: drone_missing(&error, code),
                failure: handshake_failure(error, status, &noise.take()),
            });
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

/// Validate the folder and hand off to [`spawn_ssh_command`], which builds the argv and starts
/// `ssh` with all three streams piped.
fn spawn_ssh(params: &SshDialParams, remote_command: &str) -> Result<Child, ConnectFailure> {
    // A profile's fields reach `argv`, and `argv` is not a shell — but a control character has no
    // business in a folder path either. Refused before the process exists, on
    // `remote_connect::is_typable`'s reasoning; the host and port half of this check lives in
    // `spawn_ssh_command`, shared with the Drones section below, which has no folder to check.
    if params.root.chars().any(|c| c.is_control()) {
        return Err(ConnectFailure::Untypable("folder"));
    }
    spawn_ssh_command(
        &params.profile,
        params.config_root.as_deref(),
        remote_command,
    )
}

/// Build the argv shared by every ssh reach of a target — a dial, a Drones-panel refresh, a
/// stop — and start it with all three streams piped.
///
/// The remote command is a parameter rather than always [`remote_command`]'s launch line: what
/// differs between a fresh dial and the Drones settings section listing or stopping what already
/// runs there is only this one string, over the identical argv otherwise.
fn spawn_ssh_command(
    profile: &SshProfile,
    config_root: Option<&str>,
    remote_command: &str,
) -> Result<Child, ConnectFailure> {
    // A value beginning with `-` would be read by `ssh` as an option of its own, and a control
    // character has no business in a hostname.
    let target = ssh_target(profile);
    if target.is_empty() || target.starts_with('-') || target.chars().any(|c| c.is_control()) {
        return Err(ConnectFailure::Untypable("host"));
    }

    let mut command = Command::new("ssh");
    apply_connection_options(&mut command, profile, ConnectionKind::Ssh);
    // No pseudo-terminal, ever: the far end's standard output is a length-prefixed binary stream,
    // and a tty between the two would rewrite it.
    command.arg("-T").arg(target);
    command.arg(remote_command);

    apply_askpass(&mut command, profile, config_root);

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    put_in_own_session(&mut command);

    command
        .spawn()
        .map_err(|error| ConnectFailure::Unreachable(format!("ssh could not be started: {error}")))
}

/// What `ssh` is pointed at: the alias for a `ConfigAlias` profile, the host otherwise. The user
/// rides as `-l` rather than as `user@host`, so a name with an `@` in it is not ambiguous.
fn ssh_target(profile: &SshProfile) -> String {
    profile.host.trim().to_string()
}

/// Which program [`apply_connection_options`] is building argv for. `ssh` and `scp` read
/// identical `-o` options but spell the port and the user differently — `-p`/`-l` against
/// `-P`/`-o User=`, `scp` having no `-l` of its own.
#[derive(Clone, Copy)]
enum ConnectionKind {
    Ssh,
    Scp,
}

/// The profile fields `ssh` and `scp` read the same way: port, user, identity file, connect
/// timeout and batch mode — everything [`SshAuth::ConfigAlias`] defers to `~/.ssh/config` instead,
/// on [`spawn_ssh_command`]'s original reasoning. Factored out so [`ensure_drone`]'s `scp` upload
/// builds its argv from the same profile fields a dial already does, rather than a second reading
/// of them drifting from the first.
fn apply_connection_options(command: &mut Command, profile: &SshProfile, kind: ConnectionKind) {
    if matches!(profile.auth, SshAuth::ConfigAlias) {
        return;
    }
    match kind {
        ConnectionKind::Ssh => {
            command.arg("-p").arg(profile.port.to_string());
            if !profile.user.trim().is_empty() {
                command.arg("-l").arg(profile.user.trim());
            }
        }
        ConnectionKind::Scp => {
            command.arg("-P").arg(profile.port.to_string());
            if !profile.user.trim().is_empty() {
                command
                    .arg("-o")
                    .arg(format!("User={}", profile.user.trim()));
            }
        }
    }
    if let SshAuth::KeyFile { path, .. } = &profile.auth
        && !path.trim().is_empty()
    {
        command.arg("-i").arg(path.trim());
    }
    command
        .arg("-o")
        .arg(format!("ConnectTimeout={CONNECT_TIMEOUT_SECS}"));
    // Batch mode where there is nothing to answer a prompt with: an agent key or an unencrypted
    // key file either works or does not, and failing at once beats blocking on a prompt no
    // terminal will ever see. Where a secret *is* filed, batch mode would refuse the askpass
    // helper that supplies it, so it is left off.
    if !profile.auth.has_secret() {
        command.arg("-o").arg("BatchMode=yes");
    }
}

/// A session of its own, so an interrupt aimed at the terminal Ubiq was launched from does not
/// take the child with it. `SSH_ASKPASS_REQUIRE=force` is what actually keeps the password off a
/// terminal; this only keeps the child out of the window's job control.
fn put_in_own_session(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// The command the far machine runs. One string, because that is what `ssh` hands its login
/// shell — so the root is single-quoted for that shell, which is the only quoting this needs.
///
/// [`DronePreset::Attached`] is unchanged from before phase 7: `--stdio`, no socket, dies with
/// this `ssh`. The other two presets both detach and both reattach the same way — `--listen`
/// binds a socket, or, when one for these roots is already listening, *adopts* it and exits at
/// once instead of serving anything — so the whole line is "adopt or start, then attach", one
/// shell `&&`. That single line is the whole of phase 7's adoption story on this side: a second
/// launch against the same roots and linger never starts a rival drone, it reattaches to the one
/// already there. `--linger` is repeated on the `--attach` half because a detached drone's linger
/// is reasserted on every attach (phase 6) — the value that survives is whichever launch attaches
/// last, which for this line is always this one.
///
/// `root_id` rides every `--root` the line carries, including the `--listen` half: the id is what
/// the launching invocation seeds its catalogue with, so putting it only on the `--attach` half
/// would have an adopted drone announce the folder under an id of its own minting and the user
/// would see the project twice (`D116`).
///
/// `drone_path` names the far machine's binary in place of the bare [`DRONE_COMMAND`] whenever a
/// saved profile or a fresh [`ensure_drone`] deploy has one — shell-quoted like the root, since a
/// hand-typed or uploaded path may carry a space.
fn remote_command(
    root: &str,
    preset: DronePreset,
    root_id: Option<ProjectId>,
    linger_secs: Option<u64>,
    drone_path: Option<&str>,
) -> String {
    let root = root.trim();
    let drone = drone_path
        .map(shell_quote)
        .unwrap_or_else(|| DRONE_COMMAND.to_string());
    let id_arg = root_id
        .map(|id| format!(" --root-id {id}"))
        .unwrap_or_default();
    match preset {
        DronePreset::Attached => {
            if root.is_empty() {
                format!("{drone} --stdio")
            } else {
                format!("{drone} --stdio --root {}{id_arg}", shell_quote(root))
            }
        }
        DronePreset::Session | DronePreset::Managed => {
            let linger = linger_arg(preset, linger_secs);
            let root_arg = shell_quote(root);
            format!(
                "{drone} --listen --root {root_arg}{id_arg} --linger {linger} \
                 >/dev/null && exec {drone} --attach --root {root_arg}{id_arg} \
                 --linger {linger}"
            )
        }
    }
}

/// The `--linger` value a preset launches with, or the override a project's `DroneOrigin` carries.
/// `Attached` has no socket and so no linger at all; this is only ever asked of the other two.
fn linger_arg(preset: DronePreset, linger_secs: Option<u64>) -> String {
    if let Some(secs) = linger_secs {
        return format!("{secs}s");
    }
    match preset {
        DronePreset::Attached => "0".to_string(),
        DronePreset::Session => "600s".to_string(),
        DronePreset::Managed => "never".to_string(),
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
fn apply_askpass(command: &mut Command, profile: &SshProfile, config_root: Option<&str>) {
    if !profile.auth.has_secret() {
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
    command.env(ASKPASS_PROFILE_VAR, profile.id.to_string());
    if let Some(root) = config_root {
        command.env(ASKPASS_ROOT_VAR, root);
    }
}

/// The environment variable that puts the binary into askpass mode, and names which profile's
/// secret is wanted. Read by `ubiq_app::askpass`; a reference, never material.
pub const ASKPASS_PROFILE_VAR: &str = "UBIQ_ASKPASS_PROFILE";

/// Which config root the helper opens its secret store under. Absent, the helper resolves its own
/// the way any other launch does.
pub const ASKPASS_ROOT_VAR: &str = "UBIQ_ASKPASS_ROOT";

// ── The deployer ──────────────────────────────────────────────────────────────

/// Which of [`ensure_drone`]'s steps is running, for a caller on the GPUI thread that wants to
/// say more than "connecting" while the real work sits on the background executor a deploy always
/// runs on — a probe, an upload and a `chmod` are not instant, and a modal with no word for any of
/// them looks hung rather than busy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployStep {
    /// Asking the cached remote path whether it is still this build's drone.
    CheckingCache,
    /// `ssh <target> uname -sm`.
    Probing,
    /// Looking up and hash-verifying the local cross-built binary for the probed triple.
    Resolving,
    /// `scp`-ing the verified binary to the far machine's cache directory.
    Uploading,
}

/// Make sure a drone binary is on the far machine and return the path to launch it at.
///
/// **Skip everything** when `cached_remote_path` still answers `--version` with this build's own
/// drone version — the whole point of caching the path is never repeating a probe, a resolve and
/// an upload once one has already landed. Otherwise: probe the triple, resolve and verify a
/// locally cross-built binary for it from the hash-pinned manifest (`G108` — this proves the
/// bytes are what `just drone-manifest` last hashed, not who built them), `scp` it to a throwaway
/// path under the remote user's cache directory, and `chmod +x` it there.
///
/// A triple [`ubiq_proto::drone::entry`] has no build for is a refusal naming the triple and
/// `just drone-build` — an empty manifest, as this sandbox's is, means that is the answer every
/// time, and it has to read as an instruction rather than a mystery.
pub fn ensure_drone(
    profile: &SshProfile,
    config_root: Option<&str>,
    cached_remote_path: Option<&str>,
) -> Result<String, ConnectFailure> {
    ensure_drone_reporting(profile, config_root, cached_remote_path, &mut |_step| {})
}

/// [`ensure_drone`], reporting which of its four steps is running through `report` as it goes —
/// a callback rather than a channel because every step already blocks in turn on this same
/// thread, so there is nothing to race and nothing worth a second thread's queue.
pub fn ensure_drone_reporting(
    profile: &SshProfile,
    config_root: Option<&str>,
    cached_remote_path: Option<&str>,
    report: &mut dyn FnMut(DeployStep),
) -> Result<String, ConnectFailure> {
    if let Some(path) = cached_remote_path {
        report(DeployStep::CheckingCache);
        if cached_drone_still_matches(profile, config_root, path) {
            return Ok(path.to_string());
        }
    }

    report(DeployStep::Probing);
    let uname = run_ssh_command(profile, config_root, "uname -sm")?;
    let uname = uname.trim();
    let triple = ubiq_proto::drone::triple_for(uname).ok_or_else(|| {
        ConnectFailure::Refused(format!(
            "'{uname}' is not a machine this build knows a drone triple for"
        ))
    })?;

    report(DeployStep::Resolving);
    let local_path = resolve_local_binary(triple)?;

    report(DeployStep::Uploading);
    let version = ubiq_proto::drone::manifest::VERSION;
    let remote_dir = remote_cache_dir(version);
    let remote_path = remote_cache_path(version);
    run_ssh_command(
        profile,
        config_root,
        &format!("mkdir -p {}", shell_quote(&remote_dir)),
    )?;
    upload(profile, config_root, &local_path, &remote_path)?;
    run_ssh_command(
        profile,
        config_root,
        &format!("chmod +x {}", shell_quote(&remote_path)),
    )?;
    Ok(remote_path)
}

/// Whether a cached remote path is still this build's own drone: `ssh <target> <path> --version`,
/// compared against [`ubiq_proto::drone::manifest::VERSION`]. Any failure — the path is gone, the
/// binary refuses `--version`, the machine is unreachable — answers `false` rather than
/// propagating: a stale or missing cache is not a connect failure, it is exactly the case the
/// three steps after this one exist to repair.
fn cached_drone_still_matches(profile: &SshProfile, config_root: Option<&str>, path: &str) -> bool {
    let command = format!("{} --version", shell_quote(path));
    match run_ssh_command(profile, config_root, &command) {
        Ok(output) => output.trim() == ubiq_proto::drone::manifest::VERSION,
        Err(_) => false,
    }
}

/// The manifest entry for `triple`, with its local binary hash-verified against the pin before
/// anything is done with it.
///
/// The path a manifest entry names is repo-root-relative (`target/<triple>/release/ubiq-drone`) —
/// where `just drone-build` leaves it, in the source checkout `just drone-manifest` hashed it
/// from. There is no packaging pipeline yet (`G108`'s other half): once Ubiq ships as a bundled
/// app rather than run from a checkout, resolving this against an embedded resource instead of
/// `repo_root()` is that pipeline's job, not this one's.
fn resolve_local_binary(triple: &str) -> Result<std::path::PathBuf, ConnectFailure> {
    let entry = ubiq_proto::drone::entry(triple).ok_or_else(|| {
        ConnectFailure::Refused(format!(
            "no drone built for {triple} — run `just drone-build {triple}` and \
             `just drone-manifest`, then reconnect"
        ))
    })?;
    let path = repo_root().join(entry.path);
    ubiq_proto::drone::verify(&path, entry).map_err(|error| {
        ConnectFailure::Io(format!("drone binary for {triple} failed its pin: {error}"))
    })?;
    Ok(path)
}

/// This checkout's root, from the crate's own compile-time location (`crates/ubiq/..`) — see
/// [`resolve_local_binary`] for why that is the only answer available before Ubiq has a packaging
/// pipeline of its own.
fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// Where an uploaded drone lands under the remote user's own cache directory, keyed by version so
/// two Ubiq builds talking to the same machine never collide — `scp` and `ssh <target> mkdir -p`
/// both read `~` as that user's home, so nothing here has to resolve it first.
fn remote_cache_dir(version: &str) -> String {
    format!("~/.cache/ubiq/drone/{version}")
}

/// [`remote_cache_dir`], plus the binary's own name.
fn remote_cache_path(version: &str) -> String {
    format!("{}/ubiq-drone", remote_cache_dir(version))
}

/// `scp` a verified local binary to `remote_path` on `profile`'s target, and wait for it to
/// finish.
fn upload(
    profile: &SshProfile,
    config_root: Option<&str>,
    local_path: &Path,
    remote_path: &str,
) -> Result<(), ConnectFailure> {
    let mut command = scp_upload_command(profile, config_root, local_path, remote_path)?;
    let mut child = command.spawn().map_err(|error| {
        ConnectFailure::Unreachable(format!("scp could not be started: {error}"))
    })?;
    let noise = drain_stderr(child.stderr.take());
    let status = child
        .wait()
        .map_err(|error| ConnectFailure::Io(error.to_string()))?;
    if status.success() {
        return Ok(());
    }
    let said = noise.take();
    let said = said.trim();
    Err(if said.is_empty() {
        ConnectFailure::Refused(format!("scp exited {}", status.code().unwrap_or(-1)))
    } else {
        ConnectFailure::Refused(said.to_string())
    })
}

/// Build the `scp` argv for one upload: the same profile fields [`spawn_ssh_command`] reads
/// through [`apply_connection_options`], over `scp`'s own flag spelling, plus the same askpass
/// environment a dial gets. `ConfigAlias` suppresses all of it, on the same terms it always does —
/// the file the user pointed at governs the port, the user and the identity, not this argv.
fn scp_upload_command(
    profile: &SshProfile,
    config_root: Option<&str>,
    local_path: &Path,
    remote_path: &str,
) -> Result<Command, ConnectFailure> {
    let target = ssh_target(profile);
    if target.is_empty() || target.starts_with('-') || target.chars().any(|c| c.is_control()) {
        return Err(ConnectFailure::Untypable("host"));
    }

    let mut command = Command::new("scp");
    apply_connection_options(&mut command, profile, ConnectionKind::Scp);
    command.arg(local_path);
    command.arg(format!("{target}:{remote_path}"));

    apply_askpass(&mut command, profile, config_root);

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    put_in_own_session(&mut command);
    Ok(command)
}

// ── The Drones settings section ──────────────────────────────────────────────

/// One drone's own state, as `ubiq-drone --list` and `--status` report it.
///
/// The mirror of the JSON object `crates/ubiq-drone` (built in parallel, phase 7) prints — this
/// crate defines its own copy rather than sharing one, because `crates/ubiq` does not depend on
/// that binary's lib and the wire here is a line of JSON on a pipe, not a `Message`. `linger` is
/// left as the drone's own string (`"0"`, `"600s"`, `"never"`) rather than parsed into
/// [`DronePreset`] — a managed drone launched by a build that adds a fourth shape must still show
/// its actual linger rather than being forced into one of these three.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct DroneState {
    pub version: String,
    pub pid: u32,
    pub triple: String,
    pub started_at: u64,
    pub roots: Vec<String>,
    pub panes: usize,
    pub linger: String,
    pub socket: String,
}

/// List the drones running on `profile`'s target: `ubiq-drone --list` over `ssh`, parsed as JSON.
///
/// Blocking, like every other reach of `ssh` in this module — the Drones settings section runs
/// this inside `cx.background_spawn`, the same discipline [`AppState::try_connect_drone`] uses
/// for a dial rather than a fresh function of its own.
pub fn list_drones(
    profile: &SshProfile,
    config_root: Option<&str>,
) -> Result<Vec<DroneState>, ConnectFailure> {
    let output = run_ssh_command(profile, config_root, "ubiq-drone --list")?;
    serde_json::from_str(output.trim())
        .map_err(|error| ConnectFailure::Refused(format!("the drone list did not parse: {error}")))
}

/// Stop one drone at `socket`: `ubiq-drone --stop <socket>` over `ssh`.
///
/// This kills every pane the drone holds, which is why the row that calls it sits behind the
/// same danger-confirm pattern `ssh_remove` uses — a drone is never hot-swapped under live panes,
/// and stopping one is the one action here that is not reversible by a retry.
pub fn stop_drone(
    profile: &SshProfile,
    config_root: Option<&str>,
    socket: &str,
) -> Result<(), ConnectFailure> {
    let command = format!("ubiq-drone --stop {}", shell_quote(socket));
    run_ssh_command(profile, config_root, &command)?;
    Ok(())
}

/// Run one short-lived remote command over `ssh` and return its standard output.
///
/// Shares [`spawn_ssh_command`]'s argv and askpass handling with a dial; what differs is that
/// this waits for the child to exit rather than handing its pipes to a pump, because a `--list`
/// or a `--stop` is a command with an end, not a session.
///
/// A non-zero exit becomes an `Err`: every caller of this function names a command that either
/// works or names why it did not, and [`ConnectFailure::Refused`] is where that sentence goes.
/// [`check_host`] cannot use this — a check's whole point is telling "no drone here" apart from
/// "unreachable", and both currently arrive as the same `Err` variant — so it calls
/// [`run_ssh_command_capturing`] beneath this and reads the exit status itself instead. Nothing
/// here changes: [`list_drones`] and [`stop_drone`] still see a plain `Result<String, _>`.
fn run_ssh_command(
    profile: &SshProfile,
    config_root: Option<&str>,
    command: &str,
) -> Result<String, ConnectFailure> {
    let (output, status, stderr) = run_ssh_command_capturing(profile, config_root, command)?;
    if status.success() {
        return Ok(output);
    }
    let said = stderr.trim();
    Err(if said.is_empty() {
        ConnectFailure::Refused(format!("the drone exited {}", status.code().unwrap_or(-1)))
    } else {
        ConnectFailure::Refused(said.to_string())
    })
}

/// [`run_ssh_command`]'s body, minus the exit-status opinion: the command's standard output, its
/// exit status and whatever it said on standard error, for a caller that has to tell exit codes
/// apart rather than treat every non-zero one as a refusal.
fn run_ssh_command_capturing(
    profile: &SshProfile,
    config_root: Option<&str>,
    command: &str,
) -> Result<(String, std::process::ExitStatus, String), ConnectFailure> {
    let mut child = spawn_ssh_command(profile, config_root, command)?;
    // Nothing is written to it and nothing on the far end reads from it, but an open pipe is an
    // open pipe: closing this end promptly is what the target's own `read` would need to see EOF
    // if it ever tried.
    drop(child.stdin.take());
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ConnectFailure::Io("ssh gave no standard output".to_string()))?;
    let noise = drain_stderr(child.stderr.take());
    let watchdog = Watchdog::arm(&mut child);
    let mut output = String::new();
    let read = stdout.read_to_string(&mut output);
    watchdog.disarm();
    let status = child.wait();
    read.map_err(|error| ConnectFailure::Io(error.to_string()))?;
    let status = status.map_err(|error| ConnectFailure::Io(error.to_string()))?;
    Ok((output, status, noise.take()))
}

/// What a far machine says about itself when a check asks: whether `ssh` itself got through, and
/// what `ubiq-drone` answered when it was run there.
#[derive(Clone, Debug, PartialEq)]
pub struct HostCheck {
    /// Always `true`: a check that could not reach or authenticate against the profile returns
    /// `Err(ConnectFailure)` instead of this struct at all. Kept as a field rather than folded
    /// away because a caller reading a `HostCheck` should not have to know that convention to
    /// answer "did ssh work" — the answer is right here.
    pub reachable: bool,
    /// The drone's own facts, when one is on the remote `PATH`. `None` means `ssh` worked and the
    /// remote command still failed — most often the shell's own 127, "command not found" — which
    /// is a fact about the host, not a failure of this check.
    pub drone: Option<DroneProbe>,
    /// Wall-clock time for the whole round trip, `ssh` startup included — a number worth showing
    /// next to "reachable" precisely because it is not part of the drone's own answer.
    pub elapsed_ms: u64,
}

/// A drone's version and target triple, as [`ubiq_drone::probe_line`] and `--version` print them.
#[derive(Clone, Debug, PartialEq)]
pub struct DroneProbe {
    pub version: String,
    pub triple: String,
}

/// Probe whether `profile`'s target is reachable over `ssh` and whether a drone is installed
/// there, without deploying or dialling one.
///
/// **`--version && --probe` in one remote command, not `uname -sm`.** [`ensure_drone`]'s own probe
/// step shells `uname -sm` because it only needs a triple to pick a binary to upload; this needs
/// to answer a different question — is *this build's own* `ubiq-drone` already on the remote
/// `PATH`, and which one — and `uname` cannot say that. `ubiq_drone::probe_line()` prints the
/// machine's facts but carries no version of its own, so it is chained after `--version` with a
/// shell `&&`, on [`remote_command`]'s own pattern for joining two commands in one `ssh`: one
/// round trip answers reachable, installed and version together, rather than three.
///
/// Three outcomes, not two: `Err(ConnectFailure)` is `ssh` itself failing — unreachable, refused,
/// not authenticated. `Ok(HostCheck { drone: None, .. })` is `ssh` succeeding and the remote shell
/// failing to run `ubiq-drone` at all (typically exit 127) — the host is fine, nothing is
/// installed there. `Ok(HostCheck { drone: Some(..), .. })` is both working. Distinguishing the
/// last two is exactly what [`run_ssh_command`] cannot do — it turns every non-zero exit into the
/// same `Err(ConnectFailure::Refused)` — so this calls [`run_ssh_command_capturing`] instead and
/// reads the exit status itself, rather than changing what [`list_drones`] and [`stop_drone`]
/// already rely on.
pub fn check_host(
    profile: &SshProfile,
    config_root: Option<&str>,
) -> Result<HostCheck, ConnectFailure> {
    let started = std::time::Instant::now();
    let (output, status, _stderr) =
        run_ssh_command_capturing(profile, config_root, &check_command())?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let drone = if status.success() {
        parse_probe_output(&output)
    } else {
        None
    };
    Ok(HostCheck {
        reachable: true,
        drone,
        elapsed_ms,
    })
}

/// The remote command [`check_host`] runs: this build's own `--version`, then
/// [`ubiq_drone::probe_line`]'s facts, chained so both come back from one `ssh`.
fn check_command() -> String {
    format!("{DRONE_COMMAND} --version && {DRONE_COMMAND} --probe")
}

/// Parse [`check_command`]'s two lines — `ubiq-drone <version>` then `probe_line()`'s
/// `os=… arch=… triplet=…` — into a [`DroneProbe`]. Anything short of both lines parsing cleanly
/// answers `None` rather than panicking: a truncated read, an unexpected banner from a login
/// shell, or a `ubiq-drone` build old enough to print something else are all "no drone this check
/// can vouch for", not a crash.
fn parse_probe_output(output: &str) -> Option<DroneProbe> {
    let mut lines = output.lines();
    let version = lines
        .next()?
        .strip_prefix("ubiq-drone ")
        .map(str::trim)
        .filter(|version| !version.is_empty())?
        .to_string();
    let triple = lines
        .next()?
        .split_whitespace()
        .find_map(|token| token.strip_prefix("triplet="))
        .filter(|triple| !triple.is_empty())?
        .to_string();
    Some(DroneProbe { version, triple })
}

/// Turn a failed handshake, the child's exit status and whatever it said on standard error into
/// one of the vocabulary the modal already draws.
///
/// A refusal is the drone's own sentence, verbatim: it was written to be read in a modal, and
/// rewording it here would lose the two schema numbers it names.
/// The exit code a POSIX shell reserves for a command it could not find — what the far side
/// answers when [`DRONE_COMMAND`] is not on its `PATH`.
const NOT_FOUND_EXIT: i32 = 127;

/// Whether a failed handshake is the far machine having no drone to run — the one failure
/// [`dial_ssh_reporting`] answers by deploying one.
///
/// The exit code alone decides it. 127 is the shell's own reserved answer, it is already the
/// branch [`handshake_failure`] reads as "the far command failed" rather than as `ssh`'s own 255,
/// and it holds where the sentence does not: the shell's wording is localised, quoted differently
/// per shell, and absent entirely when stderr was lost. A [`HandshakeError::Refused`] is excluded
/// because a drone that refused is a drone that ran — deploying over it would replace a working
/// binary on a version disagreement.
fn drone_missing(error: &HandshakeError, code: Option<i32>) -> bool {
    !matches!(error, HandshakeError::Refused(_)) && code == Some(NOT_FOUND_EXIT)
}

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
        assert_eq!(
            remote_command("", DronePreset::Attached, None, None, None),
            "ubiq-drone --stdio"
        );
        assert_eq!(
            remote_command("/srv/my work", DronePreset::Attached, None, None, None),
            "ubiq-drone --stdio --root '/srv/my work'"
        );
        assert_eq!(
            remote_command("/srv/it's", DronePreset::Attached, None, None, None),
            "ubiq-drone --stdio --root '/srv/it'\\''s'"
        );
    }

    /// A dial a project's `runs_on` started names the id the local catalogue already holds, on
    /// every `--root` the line carries — which is what keeps the project one row and not two.
    #[test]
    fn a_projects_dial_names_the_local_id_on_every_root() {
        let id = ProjectId::generate();
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Attached, Some(id), None, None),
            format!("ubiq-drone --stdio --root '/srv/proj' --root-id {id}")
        );
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Managed, Some(id), Some(90), None),
            format!(
                "ubiq-drone --listen --root '/srv/proj' --root-id {id} --linger 90s >/dev/null \
                 && exec ubiq-drone --attach --root '/srv/proj' --root-id {id} --linger 90s"
            )
        );
    }

    /// The three presets' exact remote command strings — `Session` and `Managed` both adopt or
    /// start, then attach, which is phase 7's whole adoption story on this side.
    #[test]
    fn the_three_presets_build_the_three_documented_commands() {
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Attached, None, None, None),
            "ubiq-drone --stdio --root '/srv/proj'"
        );
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Session, None, None, None),
            "ubiq-drone --listen --root '/srv/proj' --linger 600s >/dev/null && \
             exec ubiq-drone --attach --root '/srv/proj' --linger 600s"
        );
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Managed, None, None, None),
            "ubiq-drone --listen --root '/srv/proj' --linger never >/dev/null && \
             exec ubiq-drone --attach --root '/srv/proj' --linger never"
        );
    }

    /// An explicit `drone_path` — a saved setting, or what [`ensure_drone`] just uploaded — runs
    /// in place of the bare [`DRONE_COMMAND`] across all three presets, quoted the same way a
    /// root is.
    #[test]
    fn remote_command_honours_an_explicit_drone_path_across_every_preset() {
        let path = Some("/opt/my drone/ubiq-drone");
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Attached, None, None, path),
            "'/opt/my drone/ubiq-drone' --stdio --root '/srv/proj'"
        );
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Session, None, None, path),
            "'/opt/my drone/ubiq-drone' --listen --root '/srv/proj' --linger 600s >/dev/null && \
             exec '/opt/my drone/ubiq-drone' --attach --root '/srv/proj' --linger 600s"
        );
        assert_eq!(
            remote_command("/srv/proj", DronePreset::Managed, None, None, path),
            "'/opt/my drone/ubiq-drone' --listen --root '/srv/proj' --linger never >/dev/null && \
             exec '/opt/my drone/ubiq-drone' --attach --root '/srv/proj' --linger never"
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

    /// `--list`'s contract, verbatim: this is the shape the parallel `ubiq-drone` work is building
    /// to, and `DroneState` has to parse it with no field renamed and nothing extra required.
    #[test]
    fn a_drone_state_parses_the_lists_json_shape() {
        let json = r#"{"version":"0.4.0","pid":4242,"triple":"x86_64-unknown-linux-gnu",
            "started_at":1700000000,"roots":["/srv/proj"],"panes":2,"linger":"never",
            "socket":"/run/user/1000/ubiq-drone/ab12.sock"}"#;
        let state: DroneState = serde_json::from_str(json).expect("valid drone state json");
        assert_eq!(state.version, "0.4.0");
        assert_eq!(state.pid, 4242);
        assert_eq!(state.triple, "x86_64-unknown-linux-gnu");
        assert_eq!(state.started_at, 1700000000);
        assert_eq!(state.roots, vec!["/srv/proj".to_string()]);
        assert_eq!(state.panes, 2);
        assert_eq!(state.linger, "never");
        assert_eq!(state.socket, "/run/user/1000/ubiq-drone/ab12.sock");
    }

    /// `--list` prints an array — the shape `list_drones` feeds straight to `serde_json`.
    #[test]
    fn a_drone_list_parses_as_an_array_of_states() {
        let json = r#"[
            {"version":"0.4.0","pid":1,"triple":"x86_64-unknown-linux-gnu","started_at":1,
             "roots":["/a"],"panes":0,"linger":"0","socket":"/a.sock"},
            {"version":"0.4.0","pid":2,"triple":"aarch64-apple-darwin","started_at":2,
             "roots":["/b"],"panes":3,"linger":"600s","socket":"/b.sock"}
        ]"#;
        let states: Vec<DroneState> = serde_json::from_str(json).expect("valid drone list json");
        assert_eq!(states.len(), 2);
        assert_eq!(states[0].pid, 1);
        assert_eq!(states[1].linger, "600s");
    }

    // ── The deployer ──────────────────────────────────────────────────────────

    /// The signal a fallback deploy keys off: the shell's own 127, and nothing else. `ssh`'s 255,
    /// an ordinary non-zero exit and a drone that answered the handshake all mean a drone ran —
    /// or that the machine was never reached — and neither is repaired by an upload.
    #[test]
    fn only_the_shells_not_found_code_reads_as_a_missing_drone() {
        let wire = || {
            HandshakeError::Wire(ubiq_proto::wire::WireError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "eof",
            )))
        };
        assert!(drone_missing(&wire(), Some(127)));
        assert!(!drone_missing(&wire(), Some(255)));
        assert!(!drone_missing(&wire(), Some(1)));
        assert!(!drone_missing(&wire(), None));
        // A drone that refused ran: replacing it over a version disagreement would overwrite a
        // working binary with one that disagrees just as much.
        assert!(!drone_missing(
            &HandshakeError::Refused("This Ubiq speaks 1; the drone speaks 8.".to_string()),
            Some(127)
        ));
    }

    /// The fallback's own rule: deploy only for a missing drone, and only when nothing named where
    /// the binary is. An explicit path is the caller's answer and is dialled or reported, never
    /// deployed over.
    #[test]
    fn a_deploy_follows_a_missing_drone_and_only_an_unnamed_path() {
        assert!(deploy_after(None, true));
        assert!(!deploy_after(None, false));
        assert!(!deploy_after(Some("/opt/ubiq-drone"), true));
        assert!(!deploy_after(Some("/opt/ubiq-drone"), false));
    }

    /// A deploy that fails reports its own sentence. The "not found" that provoked it says only
    /// what the deploy was already answering, and would bury the instruction.
    #[test]
    fn a_failed_deploy_reports_its_own_sentence() {
        let reported = deploy_refusal(
            ConnectFailure::Refused(
                "no drone built for x86_64-unknown-linux-gnu — run `just drone-build …`"
                    .to_string(),
            ),
            ConnectFailure::Refused("the drone exited 127: ubiq-drone: not found".to_string()),
        );
        match reported {
            ConnectFailure::Refused(reason) => assert!(reason.contains("just drone-build")),
            other => panic!("expected the deploy's own refusal, got {other:?}"),
        }
    }

    /// The remote cache a deploy writes to and reads back from — keyed by version, under the
    /// remote user's own cache directory, exactly as the module doc names it.
    #[test]
    fn the_remote_cache_path_is_versioned_under_the_users_cache_dir() {
        assert_eq!(remote_cache_dir("0.4.0"), "~/.cache/ubiq/drone/0.4.0");
        assert_eq!(
            remote_cache_path("0.4.0"),
            "~/.cache/ubiq/drone/0.4.0/ubiq-drone"
        );
    }

    /// `ensure_drone`'s middle step, in isolation: this sandbox's manifest is empty (no cross
    /// toolchain), so every known triple refuses, and the refusal has to read as an instruction —
    /// naming the triple and `just drone-build` — rather than a bare "not found".
    #[test]
    fn resolving_an_unbuilt_triple_names_it_and_drone_build() {
        let error = resolve_local_binary("x86_64-unknown-linux-gnu")
            .expect_err("this sandbox's manifest has no entries");
        match error {
            ConnectFailure::Refused(reason) => {
                assert!(reason.contains("x86_64-unknown-linux-gnu"));
                assert!(reason.contains("just drone-build"));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// A triple `uname -sm` cannot be mapped at all — an architecture or an OS this build has no
    /// entry shape for — is the same refusal family the manifest-entry case is, just earlier.
    #[test]
    fn an_unmapped_uname_answer_is_a_refusal_too() {
        assert_eq!(ubiq_proto::drone::triple_for("FreeBSD x86_64"), None);
    }

    /// The `scp` argv reads the same fields a dial does — port, user, identity, timeout, batch
    /// mode — and the askpass environment on top of them, for a profile with a secret filed.
    #[test]
    fn the_scp_argv_carries_the_profiles_connection_fields() {
        let profile = profile(SshAuth::KeyFile {
            path: "/home/ubiq/.ssh/id_ed25519".to_string(),
            has_passphrase: true,
        });
        let command = scp_upload_command(
            &profile,
            Some("/config/root"),
            Path::new("/local/ubiq-drone"),
            "~/.cache/ubiq/drone/0.4.0/ubiq-drone",
        )
        .expect("a typable profile builds an scp command");

        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(command.get_program(), "scp");
        assert!(args.contains(&"-P".to_string()));
        assert!(args.contains(&"2222".to_string()));
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"User=ubiq".to_string()));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"/home/ubiq/.ssh/id_ed25519".to_string()));
        assert!(args.contains(&"/local/ubiq-drone".to_string()));
        assert!(args.contains(&"build.example:~/.cache/ubiq/drone/0.4.0/ubiq-drone".to_string()));
        // A secret is filed, so batch mode must be left off — it would refuse the askpass helper.
        assert!(!args.contains(&"BatchMode=yes".to_string()));

        let envs: Vec<_> = command.get_envs().collect();
        assert!(
            envs.iter()
                .any(|(key, _)| *key == std::ffi::OsStr::new(ASKPASS_PROFILE_VAR))
        );
        assert!(envs.iter().any(|(key, value)| {
            *key == std::ffi::OsStr::new(ASKPASS_ROOT_VAR)
                && *value == Some(std::ffi::OsStr::new("/config/root"))
        }));
    }

    // ── The host check ───────────────────────────────────────────────────────

    /// The remote command a check runs: this build's own `--version` chained with
    /// [`ubiq_drone::probe_line`]'s facts, so one `ssh` answers both.
    #[test]
    fn the_check_command_chains_version_and_probe() {
        assert_eq!(
            check_command(),
            "ubiq-drone --version && ubiq-drone --probe"
        );
    }

    /// A well-formed two-line answer parses into the version and triple a caller wants.
    #[test]
    fn a_well_formed_probe_answer_parses_into_a_drone_probe() {
        let output = "ubiq-drone 0.4.1\nos=linux arch=aarch64 triplet=aarch64-linux-gnu\n";
        assert_eq!(
            parse_probe_output(output),
            Some(DroneProbe {
                version: "0.4.1".to_string(),
                triple: "aarch64-linux-gnu".to_string(),
            })
        );
    }

    /// A malformed or empty answer — a missing line, a banner with no version, a probe line with
    /// no `triplet=` field — parses to `None` rather than panicking.
    #[test]
    fn a_malformed_probe_answer_parses_to_none() {
        assert_eq!(parse_probe_output(""), None);
        assert_eq!(parse_probe_output("ubiq-drone 0.4.1\n"), None);
        assert_eq!(
            parse_probe_output("ubiq-drone 0.4.1\nos=linux arch=aarch64\n"),
            None
        );
        assert_eq!(
            parse_probe_output("not ubiq-drone at all\nos=linux arch=aarch64 triplet=x\n"),
            None
        );
    }

    /// `ConfigAlias` defers wholly to `~/.ssh/config`: no port, user, identity or batch-mode flag
    /// may appear, for `scp` on the same terms `ssh` already holds to.
    #[test]
    fn config_alias_suppresses_every_scp_option() {
        let command = scp_upload_command(
            &profile(SshAuth::ConfigAlias),
            None,
            Path::new("/local/ubiq-drone"),
            "~/.cache/ubiq/drone/0.4.0/ubiq-drone",
        )
        .expect("a typable profile builds an scp command");

        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(!args.iter().any(|arg| arg == "-P"));
        assert!(!args.iter().any(|arg| arg == "-i"));
        assert!(!args.iter().any(|arg| arg.starts_with("User=")));
        assert!(!args.iter().any(|arg| arg.starts_with("ConnectTimeout=")));
        assert!(!args.iter().any(|arg| arg == "BatchMode=yes"));
        assert_eq!(
            args.last(),
            Some(&"build.example:~/.cache/ubiq/drone/0.4.0/ubiq-drone".to_string())
        );
    }
}
