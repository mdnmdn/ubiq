//! The ACP agent registry: which ACP-speaking agents exist, and which of them this machine can run.
//!
//! The registry is the public JSON document at [`REGISTRY_URL`]. This module parses it (leniently:
//! unknown fields are ignored, optional ones default), keeps a local copy, and classifies every
//! agent against the user's `PATH` — see [`detect`]. It never installs anything: [`install_command`]
//! only *names* the command, and the embedder runs it.
//!
//! Claude Code and Codex are never offered here. Ubiq drives them through their own native
//! protocols, so [`detect`] drops `claude-acp`, `codex-acp` and anything whose CLI is `claude`
//! or `codex`.
//!
//! The registry is untrusted data: nothing in it is executed by this module. A `cmd` is only ever
//! reduced to a basename and looked up on `PATH`.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Where the registry is published.
pub const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// File name of the local copy inside the cache directory the caller passes.
pub const CACHE_FILE: &str = "acp-registry.json";

// ---- wire types (lenient) ----

/// The registry document.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Registry {
    /// Registry schema version.
    #[serde(default)]
    pub version: String,
    /// The agents.
    #[serde(default)]
    pub agents: Vec<RegistryAgent>,
}

/// One agent in the registry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistryAgent {
    /// Stable id, e.g. `gemini`.
    #[serde(default)]
    pub id: String,
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Registry-pinned version.
    #[serde(default)]
    pub version: String,
    /// One-line description.
    #[serde(default)]
    pub description: String,
    /// Source repository.
    #[serde(default)]
    pub repository: Option<String>,
    /// Home page.
    #[serde(default)]
    pub website: Option<String>,
    /// Authors.
    #[serde(default)]
    pub authors: Vec<String>,
    /// SPDX licence.
    #[serde(default)]
    pub license: Option<String>,
    /// Icon URL.
    #[serde(default)]
    pub icon: Option<String>,
    /// How the agent is obtained.
    #[serde(default)]
    pub distribution: Distribution,
}

/// The ways an agent is distributed; any subset may be present.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Distribution {
    /// An npm package run through `npx`.
    #[serde(default)]
    pub npx: Option<PackageDist>,
    /// A Python package run through `uvx`.
    #[serde(default)]
    pub uvx: Option<PackageDist>,
    /// Prebuilt archives keyed by `{os}-{arch}`, e.g. `darwin-aarch64`.
    #[serde(default)]
    pub binary: HashMap<String, BinaryDist>,
}

/// An `npx` / `uvx` package.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageDist {
    /// Package spec, usually version-pinned.
    #[serde(default)]
    pub package: String,
    /// Arguments that put the program into ACP mode.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment the agent wants.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// A prebuilt binary for one platform.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BinaryDist {
    /// Archive URL.
    #[serde(default)]
    pub archive: Option<String>,
    /// Path of the entry point inside the archive.
    #[serde(default)]
    pub cmd: String,
    /// Arguments that put the program into ACP mode.
    #[serde(default)]
    pub args: Vec<String>,
    /// Archive checksum, when published.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Environment the agent wants.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

// ---- fetch and cache ----

/// Parse a registry document.
pub fn parse(json: &str) -> Result<Registry> {
    serde_json::from_str(json).context("ACP registry: malformed JSON")
}

/// Download the registry, returning the raw body (not yet validated).
#[cfg(feature = "remote")]
fn fetch_raw() -> Result<String> {
    use std::time::Duration;
    // Proxies come from the environment by default in ureq 3.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    agent
        .get(REGISTRY_URL)
        .call()
        .map_err(|e| anyhow::anyhow!("ACP registry: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("ACP registry: {e}"))
}

/// Fetch and parse the registry over the network.
#[cfg(feature = "remote")]
pub fn fetch() -> Result<Registry> {
    parse(&fetch_raw()?)
}

/// Read `<dir>/acp-registry.json`; `None` when absent or unparsable.
pub fn load_cached(dir: &Path) -> Option<Registry> {
    let body = std::fs::read_to_string(dir.join(CACHE_FILE)).ok()?;
    parse(&body).ok()
}

/// Where a [`Refreshed`] registry came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Just fetched, and the cache rewritten.
    Fresh,
    /// The fetch failed; this is the cached copy, and the string is why.
    Cached(String),
}

/// The result of [`refresh`].
#[derive(Debug, Clone)]
pub struct Refreshed {
    /// The registry to use.
    pub registry: Registry,
    /// Whether it is fresh or a fallback.
    pub source: Source,
}

/// Fetch the registry and write it to `<dir>/acp-registry.json` atomically (temp file, then
/// rename). When the fetch fails, fall back to the cached copy and say so in
/// [`Refreshed::source`]; with no cache either, the fetch's error is returned.
#[cfg(feature = "remote")]
pub fn refresh(dir: &Path) -> Result<Refreshed> {
    let fetched = fetch_raw().and_then(|body| parse(&body).map(|r| (body, r)));
    match fetched {
        Ok((body, registry)) => {
            // A cache that cannot be written costs the next offline start, not this answer.
            if let Err(e) = store(dir, &body) {
                tracing::warn!("the ACP registry cache could not be written: {e:#}");
            }
            Ok(Refreshed { registry, source: Source::Fresh })
        }
        Err(e) => match load_cached(dir) {
            Some(registry) => Ok(Refreshed { registry, source: Source::Cached(e.to_string()) }),
            None => Err(e),
        },
    }
}

/// Atomically write the cache file.
#[cfg(feature = "remote")]
fn store(dir: &Path, body: &str) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let tmp = dir.join(format!("{CACHE_FILE}.{}.{nanos}.tmp", std::process::id()));
    std::fs::write(&tmp, body).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, dir.join(CACHE_FILE)).context("replace the ACP registry cache")
}

// ---- detection ----

/// Which tool installs an adapter package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// `npm install -g`.
    Npm,
    /// `uv tool install`.
    Uv,
}

/// What this machine can do about one registry agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpStatus {
    /// Runnable as is.
    Installed {
        /// The executable.
        command: PathBuf,
        /// Arguments that start it in ACP mode.
        args: Vec<String>,
        /// Environment it wants.
        env: Vec<(String, String)>,
    },
    /// The underlying CLI is present, but its ACP entry point is a package that is not, and the
    /// installer for it is.
    NeedsAdapter {
        /// The underlying CLI.
        base_cli: PathBuf,
        /// What installs the package.
        installer: Installer,
        /// The package to install.
        package: String,
    },
    /// As [`AcpStatus::NeedsAdapter`], but the installer is missing too.
    NeedsAdapterNoInstaller {
        /// The underlying CLI.
        base_cli: PathBuf,
        /// What is missing: `"node"` or `"uv"`.
        needs: &'static str,
    },
    /// Nothing usable found.
    NotInstalled,
}

/// One registry agent and its status here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpAgentStatus {
    /// Registry id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Registry version.
    pub version: String,
    /// Description.
    pub description: String,
    /// What can be done about it here.
    pub status: AcpStatus,
}

/// Agents Ubiq drives natively; never offered through the registry.
const NATIVE_IDS: &[&str] = &["claude-acp", "codex-acp"];
const NATIVE_CLIS: &[&str] = &["claude", "codex"];

/// Registry agents that already have a built-in harness of their own, by registry id → harness id.
/// Never offered as a second harness.
const BUILTINS: &[(&str, &str)] = &[
    ("github-copilot-cli", "copilot"),
    ("grok-build", "grok"),
    ("opencode", "opencode"),
];

/// The built-in harness id registry agent `id` already is, if any.
pub fn builtin_of(id: &str) -> Option<&'static str> {
    BUILTINS.iter().find(|(r, _)| *r == id).map(|(_, h)| *h)
}

/// Where registry agents keep their config, login and sessions, relative to the home directory —
/// which the registry does not say. Conservative: only directories these CLIs are known to use.
const CONFIG_HOMES: &[(&str, &[&str])] = &[
    ("gemini", &[".gemini"]),
    ("qwen-code", &[".qwen"]),
    ("factory-droid", &[".factory"]),
    ("auggie", &[".augment"]),
    ("cline", &[".cline"]),
    ("codebuddy-code", &[".codebuddy"]),
    ("qoder", &[".qoder"]),
    ("goose", &[".config/goose", ".local/share/goose", ".local/state/goose"]),
    ("pi-acp", &[".pi"]),
    ("deepagents", &[".deepagents"]),
];

/// The config homes registry agent `id` is known to use, relative to the home directory.
pub fn config_homes(id: &str) -> &'static [&'static str] {
    CONFIG_HOMES
        .iter()
        .find(|(known, _)| *known == id)
        .map_or(&[], |(_, homes)| homes)
}

/// How an npx/uvx agent's executables are named, which the registry does not say.
///
/// `cli` is the program to probe for. `acp_bin` is the separately installed ACP entry point
/// (an adapter package); `None` means the CLI itself serves ACP when run with the registry's args.
struct Known {
    id: &'static str,
    cli: &'static str,
    acp_bin: Option<&'static str>,
}

/// Conservative: only agents whose installed binary name is known.
const KNOWN: &[Known] = &[
    Known { id: "gemini", cli: "gemini", acp_bin: None },
    Known { id: "github-copilot-cli", cli: "copilot", acp_bin: None },
    Known { id: "qwen-code", cli: "qwen", acp_bin: None },
    Known { id: "grok-build", cli: "grok", acp_bin: None },
    Known { id: "cline", cli: "cline", acp_bin: None },
    Known { id: "auggie", cli: "auggie", acp_bin: None },
    Known { id: "codebuddy-code", cli: "codebuddy", acp_bin: None },
    Known { id: "factory-droid", cli: "droid", acp_bin: None },
    Known { id: "qoder", cli: "qodercli", acp_bin: None },
    // The base names here are generic words; only the adapter's own bin ever makes them
    // `Installed`, the base name only gates offering the install.
    Known { id: "pi-acp", cli: "pi", acp_bin: Some("pi-acp") },
    Known { id: "deepagents", cli: "deepagents", acp_bin: Some("deepagents-acp") },
];

/// Executable names never accepted as an agent, whatever the registry says: shells and
/// interpreters (which would run anything the arguments name), and common words an unrelated
/// program is likely to be installed as.
const NEVER_RUN: &[&str] = &[
    "sh", "bash", "zsh", "fish", "dash", "ksh", "csh", "tcsh", "pwsh", "powershell", "cmd",
    "env", "node", "nodejs", "deno", "bun", "npx", "npm", "python", "python2", "python3", "uv",
    "uvx", "pip", "ruby", "perl", "php", "lua", "java", "osascript", "sudo", "kilo", "nova",
    "dirac", "pi",
];

fn never_run(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let stem = lower
        .strip_suffix(".exe")
        .or_else(|| lower.strip_suffix(".cmd"))
        .or_else(|| lower.strip_suffix(".bat"))
        .unwrap_or(&lower);
    NEVER_RUN.contains(&stem) || stem.starts_with("python")
}

/// Environment the registry may not set: what changes how a loader, an interpreter or a lookup
/// behaves rather than configuring the agent.
fn env_allowed(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    !(k.starts_with("LD_")
        || k.starts_with("DYLD_")
        || k.starts_with("PYTHON")
        || k == "NODE_OPTIONS"
        || k == "NODE_PATH"
        || k == "PATH")
}

/// The registry's `{os}-{arch}` key for this build, e.g. `darwin-aarch64`.
pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// Classify every agent in `reg` against the process `PATH`. `overrides` maps an agent id to a
/// user-chosen executable, which wins when it exists.
pub fn detect(reg: &Registry, overrides: &HashMap<String, PathBuf>) -> Vec<AcpAgentStatus> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    detect_in(reg, overrides, &path, &platform_key())
}

/// [`detect`] against an explicit `PATH` and platform key.
pub fn detect_in(
    reg: &Registry,
    overrides: &HashMap<String, PathBuf>,
    path: &OsStr,
    platform: &str,
) -> Vec<AcpAgentStatus> {
    reg.agents
        .iter()
        .filter(|a| !is_native(a))
        .map(|a| AcpAgentStatus {
            id: a.id.clone(),
            name: a.name.clone(),
            version: a.version.clone(),
            description: a.description.clone(),
            status: classify(a, overrides.get(&a.id), path, platform),
        })
        .collect()
}

fn base_name(cmd: &str) -> &str {
    let cmd = cmd.trim_start_matches("./");
    cmd.rsplit(['/', '\\']).next().unwrap_or(cmd)
}

fn is_native(a: &RegistryAgent) -> bool {
    if NATIVE_IDS.contains(&a.id.as_str()) {
        return true;
    }
    let (cli, _) = probe_of(a);
    NATIVE_CLIS.contains(&cli.as_str())
        || a.distribution
            .binary
            .values()
            .any(|b| NATIVE_CLIS.contains(&base_name(&b.cmd)))
}

/// The CLI to probe for and its separate ACP binary. A known id answers from the table. An
/// unknown one is probed by its full id as the ACP executable; an id ending in `-acp` also names
/// its base CLI (the id without the suffix), which only ever gates offering the adapter install.
fn probe_of(a: &RegistryAgent) -> (String, Option<String>) {
    match KNOWN.iter().find(|k| k.id == a.id) {
        Some(k) => (k.cli.to_string(), k.acp_bin.map(str::to_string)),
        None => match a.id.strip_suffix("-acp") {
            Some(base) => (base.to_string(), Some(a.id.clone())),
            None => (a.id.clone(), None),
        },
    }
}

/// Whether `name` — a registry binary's `cmd` reduced to its basename — may be run for agent `a`:
/// it must be the agent's id or its known CLI name, and never a shell or an interpreter.
fn binary_allowed(a: &RegistryAgent, name: &str) -> bool {
    let stem = name.strip_suffix(".exe").unwrap_or(name);
    let known = KNOWN.iter().find(|k| k.id == a.id).map(|k| k.cli);
    !never_run(stem) && (stem == a.id || Some(stem) == known)
}

fn env_pairs(m: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut v: Vec<_> = m
        .iter()
        .filter(|(k, _)| env_allowed(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    v.sort();
    v
}

/// Whether `package` is a plain registry package spec the installer may be handed: npm
/// `[@scope/]name[@version]` or PEP 508 `name[==|>=|<=|~=|!=version]`. No leading `-`, no URL,
/// no path, no `:`.
pub fn valid_package(installer: Installer, package: &str) -> bool {
    if package.is_empty() || package.starts_with('-') || package.contains(':') {
        return false;
    }
    let name_ok = |s: &str| {
        !s.is_empty()
            && !s.starts_with(['-', '.'])
            && s.chars().all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c))
    };
    let version_ok = |s: &str| {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_alphanumeric() || ".-+_^~*!".contains(c))
    };
    match installer {
        Installer::Npm => {
            let (scope, rest) = match package.strip_prefix('@') {
                Some(scoped) => match scoped.split_once('/') {
                    Some((scope, rest)) => (Some(scope), rest),
                    None => return false,
                },
                None => (None, package),
            };
            if scope.is_some_and(|s| !name_ok(s)) {
                return false;
            }
            match rest.split_once('@') {
                Some((name, version)) => name_ok(name) && version_ok(version),
                None => name_ok(rest),
            }
        }
        Installer::Uv => {
            let at = package.find(['=', '>', '<', '~', '!']);
            let (name, spec) = match at {
                Some(i) => package.split_at(i),
                None => (package, ""),
            };
            if !name_ok(name) {
                return false;
            }
            if spec.is_empty() {
                return true;
            }
            ["==", ">=", "<=", "~=", "!="]
                .iter()
                .find_map(|op| spec.strip_prefix(op))
                .is_some_and(version_ok)
        }
    }
}

fn classify(
    a: &RegistryAgent,
    over: Option<&PathBuf>,
    path: &OsStr,
    platform: &str,
) -> AcpStatus {
    let d = &a.distribution;
    let bin = d.binary.get(platform);
    let pkg = d.npx.as_ref().map(|p| (p, Installer::Npm)).or(d.uvx.as_ref().map(|p| (p, Installer::Uv)));

    // Arguments and environment come from the distribution the entry point belongs to.
    let (args, env) = match (bin, pkg) {
        (Some(b), _) => (b.args.clone(), env_pairs(&b.env)),
        (None, Some((p, _))) => (p.args.clone(), env_pairs(&p.env)),
        _ => (Vec::new(), Vec::new()),
    };

    if let Some(p) = over.filter(|p| p.is_file()) {
        return AcpStatus::Installed { command: p.clone(), args, env };
    }

    if let Some(b) = bin
        && binary_allowed(a, base_name(&b.cmd))
        && let Some(found) = which_in(base_name(&b.cmd), path)
    {
        return AcpStatus::Installed { command: found, args: b.args.clone(), env: env_pairs(&b.env) };
    }

    let Some((p, installer)) = pkg else {
        return AcpStatus::NotInstalled;
    };
    let (cli, acp_bin) = probe_of(a);
    let (args, env) = (p.args.clone(), env_pairs(&p.env));
    let probe = |name: &str| (!never_run(name)).then(|| which_in(name, path)).flatten();
    // The base CLI is run only when it serves ACP itself; behind an adapter it only gates
    // offering the install, so a generic name (`pi`) may be looked for there.
    let base = match acp_bin {
        None => probe(&cli),
        Some(_) => which_in(&cli, path),
    };
    match acp_bin {
        None => match base {
            Some(command) => AcpStatus::Installed { command, args, env },
            None => AcpStatus::NotInstalled,
        },
        Some(acp) => {
            if let Some(command) = probe(&acp) {
                return AcpStatus::Installed { command, args, env };
            }
            let Some(base_cli) = base else {
                return AcpStatus::NotInstalled;
            };
            if !valid_package(installer, &p.package) {
                return AcpStatus::NotInstalled;
            }
            let (tool, needs) = match installer {
                Installer::Npm => ("npm", "node"),
                Installer::Uv => ("uv", "uv"),
            };
            if which_in(tool, path).is_some() {
                AcpStatus::NeedsAdapter { base_cli, installer, package: p.package.clone() }
            } else {
                AcpStatus::NeedsAdapterNoInstaller { base_cli, needs }
            }
        }
    }
}

/// The command that installs the adapter of a [`AcpStatus::NeedsAdapter`] agent; `None` for any
/// other status. Not run here.
pub fn install_command(s: &AcpAgentStatus) -> Option<Vec<String>> {
    match &s.status {
        AcpStatus::NeedsAdapter { installer, package, .. }
            if valid_package(*installer, package) =>
        {
            let head: &[&str] = match installer {
                Installer::Npm => &["npm", "install", "-g", "--"],
                Installer::Uv => &["uv", "tool", "install", "--"],
            };
            Some(head.iter().map(|s| s.to_string()).chain([package.clone()]).collect())
        }
        _ => None,
    }
}

// ---- PATH lookup ----

/// Find an executable called `name` in the `PATH`-style list `path`.
fn which_in(name: &str, path: &OsStr) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let exts: Vec<OsString> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"].iter().map(OsString::from).collect()
    } else {
        vec![OsString::new()]
    };
    std::env::split_paths(path).find_map(|dir| {
        exts.iter().find_map(|ext| {
            let mut file = OsString::from(name);
            file.push(ext);
            let p = dir.join(file);
            is_executable(&p).then_some(p)
        })
    })
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "version": "1.0.0",
      "unknown_top": 1,
      "agents": [
        {"id":"gemini","name":"Gemini","version":"1","description":"g","extra":true,
         "distribution":{"npx":{"package":"@google/gemini-cli@1","args":["--acp"]}}},
        {"id":"pi-acp","name":"Pi","version":"1","description":"p",
         "distribution":{"npx":{"package":"pi-acp@1"}}},
        {"id":"fast-agent","name":"Fast","version":"1","description":"f",
         "distribution":{"uvx":{"package":"fast-agent-acp==1","args":["-x"]}}},
        {"id":"goose","name":"Goose","version":"1","description":"go",
         "distribution":{"binary":{
           "darwin-aarch64":{"archive":"x","cmd":"./goose","args":["acp"]},
           "linux-x86_64":{"archive":"y","cmd":"./bin/goose","args":["acp"],"env":{"A":"b"}}}}},
        {"id":"claude-acp","name":"Claude","version":"1","description":"c",
         "distribution":{"npx":{"package":"@x/claude-agent-acp@1"}}},
        {"id":"codex-acp","name":"Codex","version":"1","description":"c",
         "distribution":{"npx":{"package":"@x/codex-acp@1"}}},
        {"id":"sneaky","name":"S","version":"1","description":"s",
         "distribution":{"binary":{"linux-x86_64":{"cmd":"./claude"}}}},
        {"id":"nothing","name":"N","version":"1","description":"n","distribution":{}}
      ],
      "extensions": []
    }"#;

    fn exe(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    fn run(path: &Path, platform: &str) -> HashMap<String, AcpAgentStatus> {
        let reg = parse(FIXTURE).unwrap();
        detect_in(&reg, &HashMap::new(), path.as_os_str(), platform)
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect()
    }

    #[test]
    fn parses_leniently() {
        let r = parse(FIXTURE).unwrap();
        assert_eq!(r.agents.len(), 8);
        assert_eq!(r.agents[3].distribution.binary.len(), 2);
        assert!(r.agents[7].distribution.npx.is_none());
    }

    #[test]
    fn platform_key_shape() {
        let k = platform_key();
        assert!(k.contains('-') && !k.contains("macos"));
    }

    #[test]
    fn native_agents_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let m = run(dir.path(), "linux-x86_64");
        assert!(!m.contains_key("claude-acp"));
        assert!(!m.contains_key("codex-acp"));
        assert!(!m.contains_key("sneaky"));
        assert!(m.contains_key("gemini"));
    }

    #[test]
    fn nothing_on_path_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let m = run(dir.path(), "linux-x86_64");
        assert!(m.values().all(|s| s.status == AcpStatus::NotInstalled));
    }

    #[test]
    fn binary_cmd_picks_platform_and_strips_dot_slash() {
        let dir = tempfile::tempdir().unwrap();
        let goose = exe(dir.path(), "goose");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(
            m["goose"].status,
            AcpStatus::Installed {
                command: goose.clone(),
                args: vec!["acp".into()],
                env: vec![("A".into(), "b".into())],
            }
        );
        // Another platform key with no entry: not runnable from the binary distribution.
        assert_eq!(run(dir.path(), "windows-x86_64")["goose"].status, AcpStatus::NotInstalled);
    }

    #[test]
    fn cli_itself_serves_acp() {
        let dir = tempfile::tempdir().unwrap();
        let g = exe(dir.path(), "gemini");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(
            m["gemini"].status,
            AcpStatus::Installed { command: g, args: vec!["--acp".into()], env: vec![] }
        );
    }

    #[test]
    fn adapter_needed_and_installable() {
        let dir = tempfile::tempdir().unwrap();
        let pi = exe(dir.path(), "pi");
        exe(dir.path(), "npm");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(
            m["pi-acp"].status,
            AcpStatus::NeedsAdapter {
                base_cli: pi,
                installer: Installer::Npm,
                package: "pi-acp@1".into()
            }
        );
        assert_eq!(
            install_command(&m["pi-acp"]).unwrap(),
            vec!["npm", "install", "-g", "--", "pi-acp@1"]
        );
        assert!(install_command(&m["gemini"]).is_none());
    }

    fn one(json_agent: &str, path: &Path) -> AcpStatus {
        let reg = parse(&format!(r#"{{"agents":[{json_agent}]}}"#)).unwrap();
        detect_in(&reg, &HashMap::new(), path.as_os_str(), "linux-x86_64")
            .remove(0)
            .status
    }

    #[test]
    fn a_binary_must_be_named_for_its_agent_and_never_an_interpreter() {
        let dir = tempfile::tempdir().unwrap();
        exe(dir.path(), "bash");
        exe(dir.path(), "other");
        let shell = r#"{"id":"evil","distribution":{"binary":{"linux-x86_64":{"cmd":"/bin/bash","args":["-c","x"]}}}}"#;
        assert_eq!(one(shell, dir.path()), AcpStatus::NotInstalled);
        let renamed = r#"{"id":"evil","distribution":{"binary":{"linux-x86_64":{"cmd":"./other"}}}}"#;
        assert_eq!(one(renamed, dir.path()), AcpStatus::NotInstalled);
        let node = r#"{"id":"node","distribution":{"binary":{"linux-x86_64":{"cmd":"./node"}}}}"#;
        exe(dir.path(), "node");
        assert_eq!(one(node, dir.path()), AcpStatus::NotInstalled);
    }

    #[test]
    fn loader_and_interpreter_env_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let g = exe(dir.path(), "goose");
        let agent = r#"{"id":"goose","distribution":{"binary":{"linux-x86_64":{"cmd":"./goose",
            "env":{"LD_PRELOAD":"x","DYLD_INSERT_LIBRARIES":"x","NODE_OPTIONS":"x","PATH":"x",
                   "PYTHONPATH":"x","GOOSE_MODE":"acp"}}}}}"#;
        assert_eq!(
            one(agent, dir.path()),
            AcpStatus::Installed {
                command: g,
                args: vec![],
                env: vec![("GOOSE_MODE".into(), "acp".into())]
            }
        );
    }

    #[test]
    fn an_unknown_acp_id_needs_its_own_bin_and_the_base_only_offers_the_install() {
        let dir = tempfile::tempdir().unwrap();
        let agent = r#"{"id":"foo-acp","distribution":{"npx":{"package":"foo-acp@1"}}}"#;
        let base = exe(dir.path(), "foo");
        exe(dir.path(), "npm");
        assert_eq!(
            one(agent, dir.path()),
            AcpStatus::NeedsAdapter {
                base_cli: base,
                installer: Installer::Npm,
                package: "foo-acp@1".into()
            }
        );
        let own = exe(dir.path(), "foo-acp");
        assert_eq!(
            one(agent, dir.path()),
            AcpStatus::Installed { command: own, args: vec![], env: vec![] }
        );
        // A generic word is never probed as an agent.
        exe(dir.path(), "kilo");
        let kilo = r#"{"id":"kilo","distribution":{"npx":{"package":"kilo@1"}}}"#;
        assert_eq!(one(kilo, dir.path()), AcpStatus::NotInstalled);
    }

    #[test]
    fn package_specs_are_validated() {
        for ok in ["pi-acp", "pi-acp@1.2.3", "@scope/pkg@^1.0.0-beta.1", "@x/y"] {
            assert!(valid_package(Installer::Npm, ok), "{ok}");
        }
        for bad in ["-g", "--registry=x", "https://x/y.tgz", "git+ssh:x", "../pkg", "a b", "@x", "file:x"] {
            assert!(!valid_package(Installer::Npm, bad), "{bad}");
        }
        for ok in ["fast-agent-acp", "fast-agent-acp==1.0", "x_y.z>=2"] {
            assert!(valid_package(Installer::Uv, ok), "{ok}");
        }
        for bad in ["-e", "x @ https://h/x", "x==", "./x", "x;rm"] {
            assert!(!valid_package(Installer::Uv, bad), "{bad}");
        }
        let s = AcpAgentStatus {
            id: "x".into(),
            name: String::new(),
            version: String::new(),
            description: String::new(),
            status: AcpStatus::NeedsAdapter {
                base_cli: PathBuf::new(),
                installer: Installer::Npm,
                package: "--evil".into(),
            },
        };
        assert!(install_command(&s).is_none());
    }

    #[test]
    fn adapter_without_installer() {
        let dir = tempfile::tempdir().unwrap();
        let pi = exe(dir.path(), "pi");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(
            m["pi-acp"].status,
            AcpStatus::NeedsAdapterNoInstaller { base_cli: pi, needs: "node" }
        );
    }

    #[test]
    fn adapter_installed_wins() {
        let dir = tempfile::tempdir().unwrap();
        exe(dir.path(), "pi");
        let a = exe(dir.path(), "pi-acp");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(
            m["pi-acp"].status,
            AcpStatus::Installed { command: a, args: vec![], env: vec![] }
        );
    }

    #[test]
    fn uvx_fallback_probe_and_install_command() {
        let dir = tempfile::tempdir().unwrap();
        let f = exe(dir.path(), "fast-agent");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(
            m["fast-agent"].status,
            AcpStatus::Installed { command: f, args: vec!["-x".into()], env: vec![] }
        );
    }

    #[test]
    fn override_wins() {
        let dir = tempfile::tempdir().unwrap();
        let custom = exe(dir.path(), "my-gemini");
        let reg = parse(FIXTURE).unwrap();
        let over = HashMap::from([("gemini".to_string(), custom.clone())]);
        let empty = tempfile::tempdir().unwrap();
        let out = detect_in(&reg, &over, empty.path().as_os_str(), "linux-x86_64");
        let g = out.iter().find(|s| s.id == "gemini").unwrap();
        assert_eq!(
            g.status,
            AcpStatus::Installed { command: custom, args: vec!["--acp".into()], env: vec![] }
        );
    }

    #[test]
    fn cache_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_cached(dir.path()).is_none());
        std::fs::write(dir.path().join(CACHE_FILE), FIXTURE).unwrap();
        assert_eq!(load_cached(dir.path()).unwrap().agents.len(), 8);
        std::fs::write(dir.path().join(CACHE_FILE), "not json").unwrap();
        assert!(load_cached(dir.path()).is_none());
    }
}
