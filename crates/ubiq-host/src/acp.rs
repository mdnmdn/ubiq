//! The ACP agent registry, as the host uses it: checking it against this machine, installing an
//! agent's adapter package, and turning the agents the user added into harnesses.
//!
//! The library owns the registry and what an entry means ([`agent_manager::acp_registry`]); this
//! module owns the three things only the embedder knows — where the cache lives (`<config
//! root>/cache/`), which `PATH` to look on ([`crate::shells::user_path`]) and which agents the user
//! added ([`ubiq_proto::settings::HostSettings::acp_agents`]).
//!
//! [`check`] and [`install`] block — a network call, a shell spawn, a package install — and are
//! only ever called from a thread of their own. [`install_harnesses`] reads one cached file and
//! stats a handful of paths, and runs on the coordinator.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use agent_manager::acp_registry::{self, AcpAgentStatus, AcpStatus, Registry};
use agent_manager::harness::{RegistryHarness, set_registry_harnesses};
use ubiq_proto::bus::Voice;
use ubiq_proto::messages::{AcpRegistryAgent, AcpRegistryStatus, Message};

/// Where the registry's local copy lives.
fn cache_dir(root: &Path) -> PathBuf {
    root.join("cache")
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
    registry: &Registry,
    commands: &BTreeMap<String, String>,
    path: &OsStr,
) -> Vec<AcpAgentStatus> {
    acp_registry::detect_in(
        registry,
        &overrides(commands),
        path,
        &acp_registry::platform_key(),
    )
}

/// Offer every added registry agent this machine can run as a harness, from the cached registry —
/// no network call, so it is what a restart does. An added agent the cache does not list, or that
/// is no longer installed, is simply not offered.
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
    let registry = (!added.is_empty())
        .then(|| acp_registry::load_cached(&cache_dir(root)))
        .flatten();
    let harnesses = registry
        .map(|mut registry| {
            registry.agents.retain(|agent| added.contains(&agent.id));
            let path = crate::shells::known_path();
            detect(&registry, commands, &path)
                .iter()
                .filter_map(RegistryHarness::from_status)
                .collect()
        })
        .unwrap_or_default();
    set_registry_harnesses(harnesses);
}

/// Refresh the registry into the cache and classify it against the user's real `PATH`. Blocking.
/// The caller asks the coordinator to re-offer the added agents afterwards.
pub(crate) fn check(root: &Path, commands: &BTreeMap<String, String>) -> Message {
    let path = crate::shells::user_path();
    let (registry, note) = match acp_registry::refresh(&cache_dir(root)) {
        Ok(refreshed) => (
            refreshed.registry,
            match refreshed.source {
                acp_registry::Source::Fresh => None,
                acp_registry::Source::Cached(why) => Some(format!(
                    "Showing the cached registry \u{2014} the fetch failed: {why}"
                )),
            },
        ),
        Err(error) => {
            return Message::AcpRegistryChecked {
                agents: Vec::new(),
                note: Some(format!("The ACP registry could not be fetched: {error:#}")),
            };
        }
    };
    report(&registry, commands, path, note)
}

/// [`check`] against the cached registry only — what follows an install, which changed this
/// machine and not the registry.
fn recheck(root: &Path, commands: &BTreeMap<String, String>) -> Message {
    match acp_registry::load_cached(&cache_dir(root)) {
        Some(registry) => report(&registry, commands, crate::shells::user_path(), None),
        None => Message::AcpRegistryChecked {
            agents: Vec::new(),
            note: Some("No cached ACP registry \u{2014} check again.".to_string()),
        },
    }
}

fn report(
    registry: &Registry,
    commands: &BTreeMap<String, String>,
    path: &OsStr,
    note: Option<String>,
) -> Message {
    let agents = detect(registry, commands, path)
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
                AcpStatus::NotInstalled => (AcpRegistryStatus::NotInstalled, String::new()),
            };
            AcpRegistryAgent {
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

/// Install registry agent `id`'s adapter package with the installer the registry names, under the
/// user's real `PATH`, then report the outcome and a fresh classification. Blocking.
pub(crate) fn install(root: &Path, id: &str, commands: &BTreeMap<String, String>) -> Vec<Message> {
    let (ok, detail) = run_install(root, id, commands);
    vec![
        Message::AcpAdapterInstalled {
            id: id.to_string(),
            ok,
            detail,
        },
        recheck(root, commands),
    ]
}

fn run_install(root: &Path, id: &str, commands: &BTreeMap<String, String>) -> (bool, String) {
    let Some(registry) = acp_registry::load_cached(&cache_dir(root)) else {
        return (
            false,
            "no cached ACP registry \u{2014} check first".to_string(),
        );
    };
    let path = crate::shells::user_path();
    let Some(words) = detect(&registry, commands, path)
        .iter()
        .find(|agent| agent.id == id)
        .and_then(acp_registry::install_command)
    else {
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
}
