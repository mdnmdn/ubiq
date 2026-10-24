//! The ACP agent registry, as the host uses it: checking it against this machine, installing an
//! agent (a downloaded binary, or its adapter package), and turning the agents the user added into
//! harnesses.
//!
//! The library owns the registry and what an entry means ([`agent_manager::acp_registry`]); this
//! module owns what only the embedder knows — where the cache lives (`<config root>/cache/`),
//! where downloaded binaries live (`<config root>/acp-agents/`), which `PATH` to look on
//! ([`crate::shells::user_path`]) and which agents the user added
//! ([`ubiq_proto::settings::HostSettings::acp_agents`]).
//!
//! The registry shown is always [`acp_registry::load`]: the cache, else the bundled snapshot, so
//! an offline first run still has a list. A fetch happens only when the cache is older than the
//! TTL ([`refresh_if_stale`]), never on the coordinator. [`check`], [`install`] and the refresh
//! block — a shell spawn, a download, a package install — and are only ever called from a thread
//! of their own. [`install_harnesses`] reads one local file and stats a handful of paths, and runs
//! on the coordinator.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use agent_manager::acp_registry::{self, AcpAgentStatus, AcpStatus, Registry, RegistryAgent};
use agent_manager::harness::{RegistryHarness, set_registry_harnesses};
use ubiq_proto::bus::Voice;
use ubiq_proto::messages::{AcpRegistryAgent, AcpRegistryStatus, Message};

/// Where the registry's local copy lives.
fn cache_dir(root: &Path) -> PathBuf {
    root.join("cache")
}

/// Where the binaries Ubiq downloaded live.
fn installs_dir(root: &Path) -> PathBuf {
    root.join(acp_registry::INSTALLS_DIR)
}

/// A command override names the executable a registry agent is found at: the first word of the
/// user's command line, resolved. The rest of the line is applied at spawn, as for any harness.
fn overrides(commands: &BTreeMap<String, String>) -> HashMap<String, PathBuf> {
    commands
        .iter()
        .filter_map(|(id, command)| {
            let first = crate::agent::split_command(command).into_iter().next()?;
            let path = PathBuf::from(crate::agent::resolve_bare(&first));
            path.is_file().then(|| (id.clone(), path))
        })
        .collect()
}

fn detect(
    root: &Path,
    registry: &Registry,
    commands: &BTreeMap<String, String>,
    path: &OsStr,
) -> Vec<AcpAgentStatus> {
    acp_registry::detect_in(
        registry,
        &overrides(commands),
        Some(&installs_dir(root)),
        path,
        &acp_registry::platform_key(),
    )
}

/// Offer every added registry agent this machine can run as a harness — installed, downloaded, or
/// runnable through `npx` / `uvx` — from the local registry; no network call, so it is what a
/// restart does. An added agent the registry does not list, or that is not runnable, is simply not
/// offered.
///
/// Called on the coordinator only, with the settings as they stand, so two writers cannot race.
/// When the user's real `PATH` is not known yet it is asked for in the background, and `voice`
/// then says [`Message::ListAgentTypes`] so the coordinator runs this again and tells every
/// window — an agent the thin boot `PATH` missed still appears.
pub(crate) fn install_harnesses(
    root: &Path,
    added: &[String],
    commands: &BTreeMap<String, String>,
    voice: &Voice,
) {
    if !added.is_empty() && crate::shells::user_path_if_known().is_none() {
        let voice = voice.clone();
        let _ = std::thread::Builder::new()
            .name("user-path".to_string())
            .spawn(move || {
                crate::shells::user_path();
                voice.say(Message::ListAgentTypes);
            });
    }
    let harnesses = if added.is_empty() {
        Vec::new()
    } else {
        let (mut registry, _) = acp_registry::load(&cache_dir(root));
        registry.agents.retain(|agent| added.contains(&agent.id));
        let path = crate::shells::known_path();
        detect(root, &registry, commands, &path)
            .iter()
            .filter_map(RegistryHarness::from_status)
            .collect()
    };
    set_registry_harnesses(harnesses);
}

/// Refresh the registry cache when it is missing or older than [`acp_registry::TTL`]. Blocking.
/// `true` when a fetched copy landed, so the caller re-offers and re-reports.
pub(crate) fn refresh_if_stale(root: &Path) -> Refresh {
    let dir = cache_dir(root);
    if !acp_registry::is_stale(&dir, acp_registry::TTL) {
        return Refresh::NotDue;
    }
    match acp_registry::refresh(&dir) {
        // A fetched copy that could not be written leaves the cache stale: it did not land.
        Ok(refreshed)
            if matches!(refreshed.source, acp_registry::Source::Fresh)
                && !acp_registry::is_stale(&dir, acp_registry::TTL) =>
        {
            Refresh::Landed
        }
        Ok(_) => Refresh::Failed,
        Err(error) => {
            tracing::info!("the ACP registry could not be refreshed: {error:#}");
            Refresh::Failed
        }
    }
}

/// What [`refresh_if_stale`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refresh {
    /// The cache is young enough; nothing was fetched.
    NotDue,
    /// A fetched copy was stored; the caller re-offers and re-reports.
    Landed,
    /// The fetch, or the write of its result, failed; the cached or bundled copy stays.
    Failed,
}

/// Agents with an install running, so a second request for the same one is refused.
static INSTALLING: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Takes an id out of [`INSTALLING`] when dropped.
struct InstallGuard(String);

impl Drop for InstallGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = INSTALLING.lock() {
            set.remove(&self.0);
        }
    }
}

/// [`refresh_if_stale`] on a thread of its own, at boot; `voice` says [`Message::ListAgentTypes`]
/// when a fetched copy landed, so the added agents are re-offered from it.
pub(crate) fn refresh_in_background(root: &Path, voice: &Voice) {
    let (root, voice) = (root.to_path_buf(), voice.clone());
    let _ = std::thread::Builder::new()
        .name("acp-registry-refresh".to_string())
        .spawn(move || {
            if refresh_if_stale(&root) == Refresh::Landed {
                voice.say(Message::ListAgentTypes);
            }
        });
}

/// Classify the local registry against the user's real `PATH`. It never fetches; the note says
/// which copy is shown. Blocking on the `PATH` probe.
pub(crate) fn check(root: &Path, commands: &BTreeMap<String, String>) -> Message {
    check_in(root, commands, crate::shells::user_path())
}

/// [`check`] after a refresh that did not land: the same list, with a note that says so instead
/// of "refreshing".
pub(crate) fn check_refresh_failed(root: &Path, commands: &BTreeMap<String, String>) -> Message {
    let mut message = check_in(root, commands, crate::shells::user_path());
    if let Message::AcpRegistryChecked { note, .. } = &mut message {
        let kept = match acp_registry::load(&cache_dir(root)).1 {
            acp_registry::Origin::Bundled => "the bundled registry",
            _ => "the cached registry",
        };
        *note = Some(format!("Showing {kept} \u{2014} refresh failed."));
    }
    message
}

fn check_in(root: &Path, commands: &BTreeMap<String, String>, path: &OsStr) -> Message {
    let (registry, origin) = acp_registry::load(&cache_dir(root));
    let note = match origin {
        acp_registry::Origin::Fresh => {
            "Showing the registry fetched within the last 24 hours.".to_string()
        }
        acp_registry::Origin::Cached { age } => format!(
            "Showing the cached registry, {} hours old \u{2014} refreshing.",
            age.as_secs() / 3600
        ),
        acp_registry::Origin::Bundled => {
            "Showing the registry bundled with Ubiq \u{2014} no fetched copy yet.".to_string()
        }
    };
    report(root, &registry, commands, path, Some(note))
}

/// Whether the registry publishes a checksum for `id`'s archive on this platform.
fn published_sha(registry: &Registry, id: &str) -> bool {
    registry
        .agents
        .iter()
        .find(|agent| agent.id == id)
        .and_then(|agent| agent.distribution.binary.get(&acp_registry::platform_key()))
        .is_some_and(|binary| binary.sha256.is_some())
}

fn report(
    root: &Path,
    registry: &Registry,
    commands: &BTreeMap<String, String>,
    path: &OsStr,
    note: Option<String>,
) -> Message {
    let agents = detect(root, registry, commands, path)
        .into_iter()
        .map(|agent| {
            let (status, detail) = match &agent.status {
                AcpStatus::Installed { command, .. } => (
                    AcpRegistryStatus::Installed,
                    command.to_string_lossy().into_owned(),
                ),
                AcpStatus::NeedsAdapter { .. } => (
                    AcpRegistryStatus::NeedsAdapter,
                    acp_registry::install_command(&agent)
                        .map(|words| words.join(" "))
                        .unwrap_or_default(),
                ),
                AcpStatus::NeedsAdapterNoInstaller { needs, .. } => {
                    (AcpRegistryStatus::NeedsInstaller, (*needs).to_string())
                }
                AcpStatus::Downloadable { url, .. } => {
                    (AcpRegistryStatus::Downloadable, url.clone())
                }
                AcpStatus::ViaRunner { package, .. } => {
                    (AcpRegistryStatus::ViaRunner, package.clone())
                }
                AcpStatus::NotInstalled => (AcpRegistryStatus::NotInstalled, String::new()),
            };
            AcpRegistryAgent {
                // For an update, whether the registry's current archive publishes a checksum.
                verified: match &agent.status {
                    AcpStatus::Downloadable { sha256, .. } => sha256.is_some(),
                    AcpStatus::Installed {
                        install_root: Some(_),
                        ..
                    } => published_sha(registry, &agent.id),
                    _ => false,
                },
                update: agent.update.clone(),
                runner: match &agent.status {
                    AcpStatus::ViaRunner { installer, .. } => Some(
                        match installer {
                            acp_registry::Installer::Npm => "npx",
                            acp_registry::Installer::Uv => "uvx",
                        }
                        .to_string(),
                    ),
                    _ => None,
                },
                builtin: agent_manager::harness::builtin_for(&agent.id),
                id: agent.id,
                name: agent.name,
                version: agent.version,
                description: agent.description,
                status,
                detail,
            }
        })
        .collect();
    Message::AcpRegistryChecked { agents, note }
}

/// Install registry agent `id`: download its binary when that is what the registry offers, else
/// install its adapter package with the installer the registry names, under the user's real
/// `PATH`. Then report the outcome and a fresh classification. Blocking.
pub(crate) fn install(
    root: &Path,
    id: &str,
    allow_unverified: bool,
    commands: &BTreeMap<String, String>,
) -> Vec<Message> {
    let fresh = INSTALLING
        .lock()
        .map(|mut set| set.insert(id.to_string()))
        .unwrap_or(false);
    if !fresh {
        return vec![Message::AcpAdapterInstalled {
            id: id.to_string(),
            ok: false,
            detail: format!("an install of {id} is already running"),
        }];
    }
    let _guard = InstallGuard(id.to_string());
    install_with(
        root,
        id,
        allow_unverified,
        commands,
        crate::shells::user_path(),
        acp_registry::install_binary,
    )
}

/// [`install`] with the `PATH` and the binary download injected, so a test needs no network.
fn install_with(
    root: &Path,
    id: &str,
    allow_unverified: bool,
    commands: &BTreeMap<String, String>,
    path: &OsStr,
    download: impl Fn(&Path, &RegistryAgent, &str) -> anyhow::Result<PathBuf>,
) -> Vec<Message> {
    let (ok, detail) = run_install(root, id, allow_unverified, commands, path, download);
    vec![
        Message::AcpAdapterInstalled {
            id: id.to_string(),
            ok,
            detail,
        },
        check_in(root, commands, path),
    ]
}

fn run_install(
    root: &Path,
    id: &str,
    allow_unverified: bool,
    commands: &BTreeMap<String, String>,
    path: &OsStr,
    download: impl Fn(&Path, &RegistryAgent, &str) -> anyhow::Result<PathBuf>,
) -> (bool, String) {
    let (registry, _) = acp_registry::load(&cache_dir(root));
    let statuses = detect(root, &registry, commands, path);
    let (Some(status), Some(agent)) = (
        statuses.iter().find(|agent| agent.id == id),
        registry.agents.iter().find(|agent| agent.id == id),
    ) else {
        return (false, format!("{id} is not in the ACP registry"));
    };
    // A first download, or an update of a binary Ubiq itself downloaded earlier.
    let downloads = match &status.status {
        AcpStatus::Downloadable { sha256, .. } => Some(sha256.is_some()),
        AcpStatus::Installed {
            install_root: Some(_),
            ..
        } if status.update.is_some() => Some(published_sha(&registry, id)),
        _ => None,
    };
    if let Some(verified) = downloads {
        if !verified && !allow_unverified {
            return (
                false,
                format!(
                    "{id}'s archive has no published checksum, and the download was not confirmed"
                ),
            );
        }
        tracing::info!(agent = id, "downloading an ACP agent binary");
        return match download(&installs_dir(root), agent, &acp_registry::platform_key()) {
            Ok(command) => (true, command.to_string_lossy().into_owned()),
            Err(error) => (false, format!("{error:#}")),
        };
    }
    let Some(words) = acp_registry::install_command(status) else {
        return (false, format!("{id} has nothing to install"));
    };
    let Some((tool, args)) = words.split_first() else {
        return (false, "empty install command".to_string());
    };
    let program = crate::agent::resolve_bare(tool);
    let mut cmd = std::process::Command::new(&program);
    cmd.args(args)
        .env("PATH", path)
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    tracing::info!(agent = id, command = %words.join(" "), "installing an ACP adapter");
    run_with_timeout(&mut cmd, tool, INSTALL_BUDGET)
}

/// How long an adapter install may run before it is killed.
const INSTALL_BUDGET: std::time::Duration = std::time::Duration::from_secs(300);

/// Run `cmd` to completion or `budget`, whichever comes first; both pipes are drained on threads
/// of their own so a chatty installer cannot wedge on a full pipe.
fn run_with_timeout(
    cmd: &mut std::process::Command,
    tool: &str,
    budget: std::time::Duration,
) -> (bool, String) {
    use std::io::Read as _;
    use std::process::Stdio;

    let mut child = match cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(error) => return (false, format!("{tool} could not start: {error}")),
    };
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = std::time::Instant::now() + budget;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // On a timeout the readers are left behind rather than joined: a grandchild the kill did not
    // reach may still hold the pipes open.
    let Some(status) = status else {
        return (
            false,
            format!(
                "{tool} did not finish within {} minutes and was stopped",
                budget.as_secs() / 60
            ),
        );
    };
    let text = format!(
        "{}{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    (status.success(), tail(&text, 6))
}

/// The last `lines` non-empty lines of `text`.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_last_non_empty_lines() {
        assert_eq!(tail("a\n\nb\nc\n", 2), "b\nc");
        assert_eq!(tail("", 3), "");
    }

    /// A cached registry with one downloadable agent whose archive is a raw executable, and
    /// `InstallAcpAdapter` for it: `install_binary`'s download is replaced by `install_archive`
    /// over a local fixture (the real download is https-only, so `file://` is not an option).
    #[test]
    fn installing_a_downloadable_agent_makes_it_installed() {
        let root = tempfile::tempdir().unwrap();
        let platform = acp_registry::platform_key();
        let registry = format!(
            r#"{{"version":"1","agents":[{{"id":"fixture-agent","name":"Fixture","version":"1.2.3",
            "distribution":{{"binary":{{"{platform}":{{
            "archive":"https://example.invalid/fixture-agent","cmd":"./fixture-agent","args":["acp"]}}}}}}}}]}}"#
        );
        std::fs::create_dir_all(cache_dir(root.path())).unwrap();
        std::fs::write(cache_dir(root.path()).join(acp_registry::CACHE_FILE), registry).unwrap();
        let fixture = root.path().join("fixture-download");
        std::fs::write(&fixture, "#!/bin/sh\nexit 0\n").unwrap();

        let commands = BTreeMap::new();
        let before = check_in(root.path(), &commands, OsStr::new(""));
        let Message::AcpRegistryChecked { agents, .. } = before else { panic!("not a check") };
        assert_eq!(agents[0].status, AcpRegistryStatus::Downloadable);
        assert!(!agents[0].verified);

        let replies = install_with(root.path(), "fixture-agent", true, &commands, OsStr::new(""), |installs, agent, platform| {
            acp_registry::install_archive(installs, agent, platform, &fixture)
        });
        let [Message::AcpAdapterInstalled { ok, detail, .. }, Message::AcpRegistryChecked { agents, .. }] =
            &replies[..]
        else {
            panic!("unexpected replies: {replies:?}")
        };
        assert!(*ok, "{detail}");
        assert_eq!(agents[0].status, AcpRegistryStatus::Installed);
        assert!(agents[0].detail.contains("1.2.3"));
    }
}
