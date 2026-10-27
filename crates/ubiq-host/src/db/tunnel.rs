//! An SSH tunnel in front of a database connection: the system `ssh` with a local port forward,
//! started where the connection is dialled and killed when the connection goes.
//!
//! The argv is [`ubiq_proto::ssh::tunnel_command`] — the same reading of an [`SshProfile`] the
//! interface's drone dial uses. `ubiq-db` never hears of it: [`route`] rewrites the config to
//! `127.0.0.1:<local port>` and the driver dials that.
//!
//! **Lifecycle.** A [`Tunnel`] is owned by whatever owns the driver connection — a session worker,
//! or a Test's thread — and dropping it kills and reaps the `ssh`. A session that ends (tab closed,
//! `DbDisconnect`, window gone, connection edited) drops its worker and with it the tunnel.
//!
//! **Quit and crash.** A worker is not joined when the application quits, and `ssh -N` runs in a
//! process group of its own with no stdio tied to Ubiq, so it would outlive the process. Every
//! live child is therefore also in a process-wide registry, and [`kill_all_tunnels`] — called by
//! the binary on quit and on a main-thread panic — ends them all. A `SIGKILL` still leaks them.

use std::collections::HashMap;
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ubiq_db::conn::{ConnectionConfig, DbKind, SslMode};
use ubiq_proto::db::{DbFailure, DbFailureKind};
use ubiq_proto::ids::SshProfileId;
use ubiq_proto::settings::SshProfile;

/// How a session finds the profile a connection names: the host's settings, read at dial time so
/// an edited profile is what the next connect uses.
pub type SshLookup = Arc<dyn Fn(SshProfileId) -> Option<SshProfile> + Send + Sync>;

/// A lookup that knows no profile — a test's, or a host without settings.
pub fn no_profiles() -> SshLookup {
    Arc::new(|_| None)
}

/// How long the forward may take to come up — connect, authenticate, bind — before it is killed.
/// Above `ssh`'s own `ConnectTimeout`, which covers only the TCP connect.
const READY_TIMEOUT: Duration = Duration::from_secs(20);

/// How much of `ssh`'s standard error a failure sentence keeps.
const MAX_STDERR: usize = 4 * 1024;

/// Every live tunnel's `ssh`, by key, so a quit can end them all.
static LIVE: LazyLock<Mutex<HashMap<u64, Child>>> = LazyLock::new(Default::default);
static NEXT: AtomicU64 = AtomicU64::new(0);

fn live() -> MutexGuard<'static, HashMap<u64, Child>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

fn end(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Kill every tunnel this process started. For the quit and crash paths; a session whose tunnel
/// is gone finds its connection lost and dials again.
pub fn kill_all_tunnels() {
    let all: Vec<Child> = live().drain().map(|(_, child)| child).collect();
    all.into_iter().for_each(end);
}

/// A running forward, its `ssh` held in [`LIVE`]. Dropping it ends it.
pub struct Tunnel {
    key: u64,
    pub local_port: u16,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let child = live().remove(&self.key);
        if let Some(child) = child {
            end(child);
        }
    }
}

/// Put `config` behind the tunnel `ssh` names, if it names one: open it, and point the config at
/// its local end. A file database, or no profile, dials as it is.
pub fn route(
    config: &mut ConnectionConfig,
    ssh: Option<SshProfileId>,
    lookup: &SshLookup,
    config_root: &Path,
) -> Result<Option<Tunnel>, DbFailure> {
    let Some(id) = ssh else {
        return Ok(None);
    };
    if config.kind == DbKind::Sqlite {
        return Ok(None);
    }
    // The driver would check the certificate against `127.0.0.1`, which it never names.
    if config.ssl == SslMode::VerifyFull {
        return Err(DbFailure::new(
            DbFailureKind::Config,
            "certificate host verification is not supported through an SSH tunnel; use Require",
        ));
    }
    let profile = lookup(id).ok_or_else(|| {
        DbFailure::new(
            DbFailureKind::Config,
            "the SSH profile this connection tunnels through no longer exists",
        )
    })?;
    let port = config.effective_port().ok_or_else(|| {
        DbFailure::new(
            DbFailureKind::Config,
            "a named instance needs an explicit port to be reached through an SSH tunnel",
        )
    })?;
    let tunnel = open(&profile, config_root, config.effective_host(), port)?;
    config.host = Some(Ipv4Addr::LOCALHOST.to_string());
    config.port = Some(tunnel.local_port);
    config.instance = None;
    Ok(Some(tunnel))
}

/// Start `ssh -N -L` on a free loopback port and wait until the port answers, `ssh` exits, or
/// [`READY_TIMEOUT`] passes.
fn open(
    profile: &SshProfile,
    config_root: &Path,
    remote_host: &str,
    remote_port: u16,
) -> Result<Tunnel, DbFailure> {
    let local_port = free_port()?;
    let root = config_root.to_string_lossy();
    let root = Some(root.as_ref()).filter(|r| !r.is_empty());
    let mut command =
        ubiq_proto::ssh::tunnel_command(profile, root, local_port, remote_host, remote_port)
            .ok_or_else(|| {
                DbFailure::new(
                    DbFailureKind::Config,
                    "the SSH profile's host or the database host cannot be tunnelled to",
                )
            })?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        DbFailure::new(
            DbFailureKind::Connect,
            format!("ssh could not be started: {error}"),
        )
    })?;
    let noise = drain(child.stderr.take());
    // Registered at once, so a quit during the wait below ends it too; every failure return
    // drops `tunnel`, which kills it.
    let key = NEXT.fetch_add(1, Ordering::Relaxed);
    live().insert(key, child);
    let tunnel = Tunnel { key, local_port };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port));
    let started = Instant::now();
    loop {
        let exited = live().get_mut(&key).map(|child| child.try_wait());
        match exited {
            None => return Err(refused("the SSH tunnel was closed".into(), &noise)),
            Some(Ok(Some(status))) => {
                return Err(refused(format!("ssh exited ({status})"), &noise));
            }
            Some(_) => {}
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            return Ok(tunnel);
        }
        if started.elapsed() > READY_TIMEOUT {
            return Err(refused(
                "the SSH tunnel did not come up in time".into(),
                &noise,
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A loopback port nothing holds right now. `ssh` binds it a moment later; a race lost to another
/// process is `ExitOnForwardFailure`'s exit, reported like any other.
fn free_port() -> Result<u16, DbFailure> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| {
            DbFailure::new(
                DbFailureKind::Connect,
                format!("no local port for the SSH tunnel: {error}"),
            )
        })
}

/// Read `ssh`'s standard error for the life of the child — a pipe nobody reads would block it —
/// keeping the first [`MAX_STDERR`] bytes for a failure sentence.
fn drain(stderr: Option<std::process::ChildStderr>) -> Arc<Mutex<String>> {
    let kept = Arc::new(Mutex::new(String::new()));
    if let Some(mut stderr) = stderr {
        let sink = kept.clone();
        let spawned = std::thread::Builder::new()
            .name("ubiq-db-tunnel".into())
            .spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = stderr.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut kept = sink.lock().unwrap_or_else(|e| e.into_inner());
                    if kept.len() < MAX_STDERR {
                        kept.push_str(&String::from_utf8_lossy(&buf[..n]));
                    }
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("an SSH tunnel's error output has no reader: {error}");
        }
    }
    kept
}

fn refused(what: String, noise: &Arc<Mutex<String>>) -> DbFailure {
    // The drain may still be reading the last bytes of an `ssh` that has just exited.
    std::thread::sleep(Duration::from_millis(50));
    let said = noise
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .trim()
        .to_string();
    let message = match said.is_empty() {
        true => what,
        false => format!("{what}: {said}"),
    };
    DbFailure::new(DbFailureKind::Connect, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_profile_and_a_file_database_dial_as_they_are() {
        let lookup = no_profiles();
        let mut server = ConnectionConfig::new(DbKind::Postgres);
        server.host = Some("db.internal".into());
        let before = server.clone();
        assert!(
            route(&mut server, None, &lookup, Path::new(""))
                .unwrap()
                .is_none()
        );
        assert_eq!(server, before);
        let mut file = ConnectionConfig::new(DbKind::Sqlite);
        let named = Some(SshProfileId::generate());
        assert!(
            route(&mut file, named, &lookup, Path::new(""))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn verify_full_through_a_tunnel_is_refused() {
        let mut server = ConnectionConfig::new(DbKind::Postgres);
        server.ssl = SslMode::VerifyFull;
        let failure = route(
            &mut server,
            Some(SshProfileId::generate()),
            &no_profiles(),
            Path::new(""),
        )
        .err()
        .expect("refused");
        assert_eq!(failure.kind, DbFailureKind::Config);
        assert!(
            failure.message.contains("use Require"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn a_profile_that_is_gone_is_a_config_failure() {
        let mut server = ConnectionConfig::new(DbKind::MySql);
        let failure = route(
            &mut server,
            Some(SshProfileId::generate()),
            &no_profiles(),
            Path::new(""),
        )
        .err()
        .expect("refused");
        assert_eq!(failure.kind, DbFailureKind::Config);
    }
}
