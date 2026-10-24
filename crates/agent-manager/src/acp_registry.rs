//! The ACP agent registry: which ACP-speaking agents exist, and which of them this machine can run.
//!
//! The registry is the public JSON document at [`REGISTRY_URL`]. This module parses it (leniently:
//! unknown fields are ignored, optional and `null` ones default), keeps a local copy with a
//! [`TTL`], falls back to the snapshot bundled at build time ([`bundled`]) when there is none, and
//! classifies every agent against the user's `PATH` and Ubiq's own install directory — see
//! [`detect`]. Packages are never installed here: [`install_command`] only *names* the command,
//! and the embedder runs it. A prebuilt binary is the one thing this module installs, through
//! [`install_binary`], into `<install dir>/<id>/<version>/`.
//!
//! Claude Code and Codex are never offered here. Ubiq drives them through their own native
//! protocols, so [`detect`] drops `claude-acp`, `codex-acp` and anything whose CLI is `claude`
//! or `codex`.
//!
//! The registry is untrusted data: nothing in it is executed by this module. A `cmd` is either
//! reduced to a basename and looked up on `PATH`, or resolved as a relative path that must stay
//! inside its own install directory.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer};

/// Where the registry is published.
pub const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// File name of the local copy inside the cache directory the caller passes.
pub const CACHE_FILE: &str = "acp-registry.json";

/// How long a cached copy counts as current ([`is_stale`]).
pub const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The registry as it stood when this crate was built: what a first run with no network shows.
pub const SNAPSHOT: &str = include_str!("acp_registry_snapshot.json");

/// The directory, under the embedder's config root, prebuilt binaries are installed into.
pub const INSTALLS_DIR: &str = "acp-agents";

/// The largest archive [`install_binary`] downloads.
pub const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

// ---- wire types (lenient) ----

/// `null` reads as the default, as a missing field does.
fn nullable<'de, D, T>(d: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// The registry document.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Registry {
    /// Registry schema version.
    #[serde(default, deserialize_with = "nullable")]
    pub version: String,
    /// The agents.
    #[serde(default, deserialize_with = "nullable")]
    pub agents: Vec<RegistryAgent>,
}

/// One agent in the registry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistryAgent {
    /// Stable id, e.g. `gemini`.
    #[serde(default, deserialize_with = "nullable")]
    pub id: String,
    /// Display name.
    #[serde(default, deserialize_with = "nullable")]
    pub name: String,
    /// Registry-pinned version.
    #[serde(default, deserialize_with = "nullable")]
    pub version: String,
    /// One-line description.
    #[serde(default, deserialize_with = "nullable")]
    pub description: String,
    /// Source repository.
    #[serde(default)]
    pub repository: Option<String>,
    /// Home page.
    #[serde(default)]
    pub website: Option<String>,
    /// Authors.
    #[serde(default, deserialize_with = "nullable")]
    pub authors: Vec<String>,
    /// SPDX licence.
    #[serde(default)]
    pub license: Option<String>,
    /// Icon URL.
    #[serde(default)]
    pub icon: Option<String>,
    /// How the agent is obtained.
    #[serde(default, deserialize_with = "nullable")]
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
    #[serde(default, deserialize_with = "nullable")]
    pub binary: HashMap<String, BinaryDist>,
}

/// An `npx` / `uvx` package.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageDist {
    /// Package spec, usually version-pinned.
    #[serde(default, deserialize_with = "nullable")]
    pub package: String,
    /// Arguments that put the program into ACP mode.
    #[serde(default, deserialize_with = "nullable")]
    pub args: Vec<String>,
    /// Environment the agent wants.
    #[serde(default, deserialize_with = "nullable")]
    pub env: HashMap<String, String>,
}

/// A prebuilt binary for one platform.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BinaryDist {
    /// Archive URL.
    #[serde(default)]
    pub archive: Option<String>,
    /// Path of the entry point inside the archive.
    #[serde(default, deserialize_with = "nullable")]
    pub cmd: String,
    /// Arguments that put the program into ACP mode.
    #[serde(default, deserialize_with = "nullable")]
    pub args: Vec<String>,
    /// Archive checksum, when published.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Environment the agent wants.
    #[serde(default, deserialize_with = "nullable")]
    pub env: HashMap<String, String>,
}

// ---- fetch and cache ----

/// Parse a registry document.
pub fn parse(json: &str) -> Result<Registry> {
    serde_json::from_str(json).context("ACP registry: malformed JSON")
}

/// The bundled snapshot ([`SNAPSHOT`]), parsed.
pub fn bundled() -> Registry {
    parse(SNAPSHOT).unwrap_or_default()
}

/// Download the registry, returning the raw body (not yet validated).
#[cfg(feature = "remote")]
fn fetch_raw() -> Result<String> {
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

/// How old the cache file is, by its modification time; `None` when there is none. A time in the
/// future reads as zero.
fn cache_age(dir: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(dir.join(CACHE_FILE)).ok()?.modified().ok()?;
    Some(modified.elapsed().unwrap_or_default())
}

/// Whether the cache in `dir` is missing or older than `ttl` — whether a refresh is due.
pub fn is_stale(dir: &Path, ttl: Duration) -> bool {
    cache_age(dir).is_none_or(|age| age >= ttl)
}

/// Where a [`load`]ed registry came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The cache, younger than [`TTL`].
    Fresh,
    /// The cache, older than [`TTL`]: a refresh is due.
    Cached {
        /// How old it is.
        age: Duration,
    },
    /// No usable cache: the snapshot bundled at build time.
    Bundled,
}

/// The registry to show without touching the network: the cache in `dir` when it parses, else the
/// bundled snapshot. Never fails.
pub fn load(dir: &Path) -> (Registry, Origin) {
    match load_cached(dir) {
        Some(registry) => {
            let age = cache_age(dir).unwrap_or_default();
            let origin = if age < TTL { Origin::Fresh } else { Origin::Cached { age } };
            (registry, origin)
        }
        None => (bundled(), Origin::Bundled),
    }
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
/// [`Refreshed::source`]; with no cache either, the fetch's error is returned (and [`load`] is
/// what still has an answer).
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

/// Which tool installs — or runs — an adapter package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installer {
    /// `npm install -g`; run with `npx`.
    Npm,
    /// `uv tool install`; run with `uvx`.
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
        /// The program a plain terminal runs: `command` itself when the CLI serves ACP by
        /// argument, the base CLI found next to an adapter (`vibe` beside `vibe-acp`), else
        /// `None` — an adapter has no interface of its own.
        tui: Option<PathBuf>,
        /// The version directory under the install dir it was found in, when it was installed
        /// by [`install_binary`]; `None` for one found by override or on `PATH`.
        install_root: Option<PathBuf>,
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
    /// Not found, but this platform has a prebuilt binary at an `https` URL:
    /// [`install_binary`] installs it.
    Downloadable {
        /// The archive.
        url: String,
        /// Its published checksum; `None` is an unverified download.
        sha256: Option<String>,
    },
    /// Not installed, but its version-pinned package runs through `npx` / `uvx`, which is here.
    ViaRunner {
        /// The resolved `npx` / `uvx`.
        runner: PathBuf,
        /// Which of the two it is.
        installer: Installer,
        /// The pinned package spec.
        package: String,
        /// Arguments that start it in ACP mode, after the package.
        args: Vec<String>,
        /// Environment it wants.
        env: Vec<(String, String)>,
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
    /// The registry version, when it is newer than the installed one this machine runs. Known
    /// only for a binary installed by [`install_binary`]; never acted on here.
    pub update: Option<String>,
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
    ("mistral-vibe", &[".vibe"]),
];

/// The config homes registry agent `id` is known to use, relative to the home directory.
pub fn config_homes(id: &str) -> &'static [&'static str] {
    CONFIG_HOMES
        .iter()
        .find(|(known, _)| *known == id)
        .map_or(&[], |(_, homes)| homes)
}

/// How an agent's executables are named, which the registry does not say.
///
/// `cli` is the program to probe for. `acp_bin` is the separate ACP entry point (an adapter
/// package, or a second binary in the same archive); `None` means the CLI itself serves ACP when
/// run with the registry's args.
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
    Known { id: "goose", cli: "goose", acp_bin: None },
    Known { id: "kimi", cli: "kimi", acp_bin: None },
    Known { id: "amp-acp", cli: "amp", acp_bin: Some("amp-acp") },
    Known { id: "mistral-vibe", cli: "vibe", acp_bin: Some("vibe-acp") },
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
/// user-chosen executable, which wins when it exists; `installs` is the install directory
/// ([`INSTALLS_DIR`] under the config root), looked in before `PATH`.
pub fn detect(
    reg: &Registry,
    overrides: &HashMap<String, PathBuf>,
    installs: Option<&Path>,
) -> Vec<AcpAgentStatus> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    detect_in(reg, overrides, installs, &path, &platform_key())
}

/// [`detect`] against an explicit `PATH` and platform key.
///
/// The lookup order for one agent: its override; then, for a binary distribution,
/// `installs/<id>/<version>/<cmd>` (else the newest version present there), then `cmd`'s basename
/// on `PATH`; then the package's known executable on `PATH`. What is not found is offered as
/// [`AcpStatus::Downloadable`], then [`AcpStatus::ViaRunner`], when either applies.
pub fn detect_in(
    reg: &Registry,
    overrides: &HashMap<String, PathBuf>,
    installs: Option<&Path>,
    path: &OsStr,
    platform: &str,
) -> Vec<AcpAgentStatus> {
    reg.agents
        .iter()
        .filter(|a| !is_native(a))
        .map(|a| {
            let (status, update) = classify(a, overrides.get(&a.id), installs, path, platform);
            AcpAgentStatus {
                id: a.id.clone(),
                name: a.name.clone(),
                version: a.version.clone(),
                description: a.description.clone(),
                status,
                update,
            }
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
    let d = &a.distribution;
    NATIVE_CLIS.contains(&cli.as_str())
        || d.binary.values().any(|b| NATIVE_CLIS.contains(&base_name(&b.cmd)))
        || [&d.npx, &d.uvx]
            .into_iter()
            .flatten()
            .any(|p| NATIVE_PACKAGES.contains(&package_name(&p.package).to_ascii_lowercase().as_str()))
}

/// Adapter packages that are Claude Code or Codex, under any scope.
const NATIVE_PACKAGES: &[&str] = &["claude-agent-acp", "claude-code-acp", "codex-acp"];

/// A package spec's bare name: no `@scope/`, no version (`@x`, `==x`, `>=x`, …).
fn package_name(spec: &str) -> &str {
    let unscoped = match spec.strip_prefix('@') {
        Some(scoped) => scoped.split_once('/').map_or(scoped, |(_, rest)| rest),
        None => spec,
    };
    let end = unscoped.find(['@', '=', '>', '<', '~', '!', '[', ' ', ';']).unwrap_or(unscoped.len());
    &unscoped[..end]
}

/// The CLI and its separate ACP binary. A known id answers from the table. For an unknown one —
/// never probed on `PATH`, only used to spot a native agent and an interface beside an installed
/// binary — the id is the ACP executable, and an id ending in `-acp` also names its base CLI (the
/// id without the suffix).
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
/// it must be a [`KNOWN`] agent's CLI name or ACP binary, and never a shell or an interpreter. An
/// unknown id is never matched on `PATH`: whatever program happens to share its `cmd` (`git`) is
/// not the agent.
fn binary_allowed(a: &RegistryAgent, name: &str) -> bool {
    let stem = name.strip_suffix(".exe").unwrap_or(name);
    !never_run(stem)
        && known(a).is_some_and(|k| stem == k.cli || Some(stem) == k.acp_bin)
}

fn known(a: &RegistryAgent) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.id == a.id)
}

/// What the registry may not set for `npx` / `uvx` either: the runner's own configuration, which
/// could fetch the pinned package from another index (`NPM_CONFIG_REGISTRY`, `UV_INDEX_URL`,
/// `PIP_INDEX_URL`).
fn runner_env_allowed(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    env_allowed(key)
        && !(k.starts_with("NPM_CONFIG_") || k.starts_with("UV_") || k.starts_with("PIP_"))
}

fn env_pairs(m: &HashMap<String, String>) -> Vec<(String, String)> {
    env_pairs_with(m, env_allowed)
}

fn env_pairs_with(m: &HashMap<String, String>, allowed: fn(&str) -> bool) -> Vec<(String, String)> {
    let mut v: Vec<_> = m
        .iter()
        .filter(|(k, _)| allowed(k))
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

/// Whether `package` passes [`valid_package`] and names one exact version (`name@1.2.3`,
/// `name==1.2.3`) — what a runner may fetch and run each launch without the code changing
/// underneath.
pub fn pinned_package(installer: Installer, package: &str) -> bool {
    if !valid_package(installer, package) {
        return false;
    }
    let exact = |v: &str| {
        v.starts_with(|c: char| c.is_ascii_digit())
            && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
    };
    match installer {
        Installer::Npm => {
            let rest = match package.strip_prefix('@') {
                Some(scoped) => scoped.split_once('/').map_or("", |(_, r)| r),
                None => package,
            };
            rest.split_once('@').is_some_and(|(_, v)| exact(v))
        }
        Installer::Uv => package.split_once("==").is_some_and(|(_, v)| exact(v)),
    }
}

/// The program a plain terminal runs for `command`: itself when it serves ACP by argument
/// (`direct`), else the agent's base CLI when one sits in the same directory.
fn tui_for(command: &Path, direct: bool, base_cli: &str) -> Option<PathBuf> {
    if direct {
        return Some(command.to_path_buf());
    }
    let own = command.file_stem().and_then(OsStr::to_str);
    if base_cli.is_empty() || never_run(base_cli) || own == Some(base_cli) {
        return None;
    }
    let dir = command.parent()?;
    which_in(base_cli, dir.as_os_str())
}

fn classify(
    a: &RegistryAgent,
    over: Option<&PathBuf>,
    installs: Option<&Path>,
    path: &OsStr,
    platform: &str,
) -> (AcpStatus, Option<String>) {
    let d = &a.distribution;
    let bin = d.binary.get(platform);
    let pkg = d.npx.as_ref().map(|p| (p, Installer::Npm)).or(d.uvx.as_ref().map(|p| (p, Installer::Uv)));
    let (cli, acp_bin) = probe_of(a);

    // Arguments and environment come from the distribution the entry point belongs to.
    let (args, env) = match (bin, pkg) {
        (Some(b), _) => (b.args.clone(), env_pairs(&b.env)),
        (None, Some((p, _))) => (p.args.clone(), env_pairs(&p.env)),
        _ => (Vec::new(), Vec::new()),
    };
    // A CLI serves ACP itself when it is no separate ACP binary and the registry's args switch
    // it into ACP; then it is its own interface too.
    let direct = |args: &[String]| acp_bin.is_none() && !args.is_empty();
    let installed = |command: PathBuf, args: Vec<String>, env, install_root| AcpStatus::Installed {
        tui: tui_for(&command, direct(&args), &cli),
        command,
        args,
        env,
        install_root,
    };

    if let Some(p) = over.filter(|p| p.is_file()) {
        return (installed(p.clone(), args, env, None), None);
    }

    if let Some(b) = bin {
        if let Some(installs) = installs
            && let Some((root, command, version)) = find_installed(installs, a, b)
        {
            let update = version_newer(&a.version, &version).then(|| a.version.clone());
            let status = installed(command, b.args.clone(), env_pairs(&b.env), Some(root));
            return (status, update);
        }
        if binary_allowed(a, base_name(&b.cmd))
            && let Some(found) = which_in(base_name(&b.cmd), path)
        {
            return (installed(found, b.args.clone(), env_pairs(&b.env), None), None);
        }
    }

    let mut fallback = AcpStatus::NotInstalled;
    // Only a known agent's package is looked for on `PATH`; an unknown one is reached through the
    // override, the install dir or a runner.
    if let Some((p, installer)) = pkg
        && known(a).is_some()
    {
        let (args, env) = (p.args.clone(), env_pairs(&p.env));
        let probe = |name: &str| (!never_run(name)).then(|| which_in(name, path)).flatten();
        // The base CLI is run only when it serves ACP itself; behind an adapter it only gates
        // offering the install, so a generic name (`pi`) may be looked for there.
        let base = match acp_bin {
            None => probe(&cli),
            Some(_) => which_in(&cli, path),
        };
        match &acp_bin {
            None => {
                if let Some(command) = base {
                    return (installed(command, args, env, None), None);
                }
            }
            Some(acp) => {
                if let Some(command) = probe(acp) {
                    return (installed(command, args, env, None), None);
                }
                if let Some(base_cli) = base
                    && valid_package(installer, &p.package)
                {
                    let (tool, needs) = match installer {
                        Installer::Npm => ("npm", "node"),
                        Installer::Uv => ("uv", "uv"),
                    };
                    if which_in(tool, path).is_some() {
                        let status = AcpStatus::NeedsAdapter {
                            base_cli,
                            installer,
                            package: p.package.clone(),
                        };
                        return (status, None);
                    }
                    fallback = AcpStatus::NeedsAdapterNoInstaller { base_cli, needs };
                }
            }
        }
    }

    if let Some(b) = bin
        && let Some(url) = b.archive.as_deref().filter(|u| u.starts_with("https://"))
        && rel_cmd(&b.cmd).is_some()
        && !never_run(base_name(&b.cmd))
        && safe_component(&a.id)
        && safe_component(&a.version)
    {
        let status = AcpStatus::Downloadable { url: url.to_string(), sha256: b.sha256.clone() };
        return (status, None);
    }

    if let Some((p, installer)) = pkg
        && pinned_package(installer, &p.package)
    {
        let runner = match installer {
            Installer::Npm => "npx",
            Installer::Uv => "uvx",
        };
        if let Some(runner) = which_in(runner, path) {
            let status = AcpStatus::ViaRunner {
                runner,
                installer,
                package: p.package.clone(),
                args: p.args.clone(),
                env: env_pairs_with(&p.env, runner_env_allowed),
            };
            return (status, None);
        }
    }
    (fallback, None)
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

// ---- the install directory ----

/// Whether `s` may name a directory under the install dir: an id or a version.
fn safe_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "-._+".contains(c))
}

/// A registry `cmd` as a path relative to its install directory: `.` and empty segments dropped,
/// `/` and `\` both separators. `None` for an absolute path, a drive or stream (`:`), a `..`, or
/// nothing at all.
fn rel_cmd(cmd: &str) -> Option<PathBuf> {
    if cmd.starts_with(['/', '\\']) || cmd.contains([':', '\0']) {
        return None;
    }
    let mut out = PathBuf::new();
    for part in cmd.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p => out.push(p),
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// `dir/rel` when it is an executable whose canonical path stays under `dir`'s.
fn contained(dir: &Path, rel: &Path) -> Option<PathBuf> {
    let p = dir.join(rel);
    let canon = std::fs::canonicalize(&p).ok()?;
    let root = std::fs::canonicalize(dir).ok()?;
    (canon.starts_with(&root) && is_executable(&canon)).then_some(p)
}

/// The installed copy of `a`'s binary `b`: the registry's own version, else the newest one
/// present. Returns the version directory, the executable and the version.
fn find_installed(
    installs: &Path,
    a: &RegistryAgent,
    b: &BinaryDist,
) -> Option<(PathBuf, PathBuf, String)> {
    if !safe_component(&a.id) || never_run(base_name(&b.cmd)) {
        return None;
    }
    let rel = rel_cmd(&b.cmd)?;
    let agent_dir = installs.join(&a.id);
    let mut versions: Vec<String> = std::fs::read_dir(&agent_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|v| safe_component(v) && !v.contains(".tmp-") && !v.contains(".old-"))
        .collect();
    // The registry's version first, then newest to oldest.
    versions.sort_by(|x, y| version_cmp(y, x));
    if let Some(i) = versions.iter().position(|v| *v == a.version) {
        let v = versions.remove(i);
        versions.insert(0, v);
    }
    versions.into_iter().find_map(|v| {
        let root = agent_dir.join(&v);
        contained(&root, &rel).map(|command| (root, command, v))
    })
}

/// Compare two version strings segment by segment (`.`, `-`, `+`), numerically where both
/// segments are numbers. A leading `v` is ignored.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> Vec<String> {
        s.trim_start_matches('v').split(['.', '-', '+']).map(str::to_string).collect()
    };
    let (x, y) = (split(a), split(b));
    for (p, q) in x.iter().zip(&y) {
        let o = match (p.parse::<u64>(), q.parse::<u64>()) {
            (Ok(m), Ok(n)) => m.cmp(&n),
            _ => p.cmp(q),
        };
        if o.is_ne() {
            return o;
        }
    }
    x.len().cmp(&y.len())
}

/// Whether version `a` is newer than `b`.
fn version_newer(a: &str, b: &str) -> bool {
    version_cmp(a, b).is_gt()
}

/// How an archive is unpacked, from its URL.
#[cfg(feature = "remote")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Tar,
    Zip,
    Raw,
}

#[cfg(feature = "remote")]
fn archive_kind(url: &str) -> ArchiveKind {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    if path.ends_with(".zip") {
        ArchiveKind::Zip
    } else if [".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tbz", ".tar.xz", ".txz", ".tar"]
        .iter()
        .any(|ext| path.ends_with(ext))
    {
        ArchiveKind::Tar
    } else {
        ArchiveKind::Raw
    }
}

/// What [`install_binary`] installs for `agent` on `platform`, validated: the binary entry, its
/// `https` URL and its `cmd` as a relative path.
#[cfg(feature = "remote")]
fn install_plan<'a>(
    agent: &'a RegistryAgent,
    platform: &str,
) -> Result<(&'a BinaryDist, &'a str, PathBuf)> {
    let b = agent
        .distribution
        .binary
        .get(platform)
        .with_context(|| format!("{} has no binary for {platform}", agent.id))?;
    anyhow::ensure!(safe_component(&agent.id), "unsafe agent id {:?}", agent.id);
    anyhow::ensure!(safe_component(&agent.version), "unsafe version {:?}", agent.version);
    let url = b
        .archive
        .as_deref()
        .filter(|u| u.starts_with("https://"))
        .with_context(|| format!("{} has no https archive for {platform}", agent.id))?;
    let rel = rel_cmd(&b.cmd)
        .with_context(|| format!("{}: cmd {:?} leaves its directory", agent.id, b.cmd))?;
    anyhow::ensure!(!never_run(base_name(&b.cmd)), "{}: refusing to install {:?}", agent.id, b.cmd);
    Ok((b, url, rel))
}

/// Download `agent`'s prebuilt binary for `platform` (at most [`MAX_DOWNLOAD`]), verify its
/// checksum when one is published, unpack it and move it into `installs/<id>/<version>/`
/// atomically. Returns the installed executable. Blocking.
#[cfg(feature = "remote")]
pub fn install_binary(installs: &Path, agent: &RegistryAgent, platform: &str) -> Result<PathBuf> {
    let (_, url, _) = install_plan(agent, platform)?;
    let agent_dir = installs.join(&agent.id);
    std::fs::create_dir_all(&agent_dir)
        .with_context(|| format!("create {}", agent_dir.display()))?;
    let archive = agent_dir.join(format!(".download-{}", unique()));
    let result = download(url, &archive).and_then(|()| install_archive(installs, agent, platform, &archive));
    let _ = std::fs::remove_file(&archive);
    result
}

#[cfg(feature = "remote")]
fn download(url: &str, to: &Path) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(30 * 60)))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| anyhow::anyhow!("download {url}: {e}"))?;
    let mut reader = response.body_mut().with_config().limit(MAX_DOWNLOAD).reader();
    let mut file = std::fs::File::create(to).with_context(|| format!("create {}", to.display()))?;
    std::io::copy(&mut reader, &mut file).with_context(|| format!("download {url}"))?;
    Ok(())
}

/// [`install_binary`] from an archive already on disk: verify, unpack, move into place.
#[cfg(feature = "remote")]
pub fn install_archive(
    installs: &Path,
    agent: &RegistryAgent,
    platform: &str,
    archive: &Path,
) -> Result<PathBuf> {
    let (b, url, rel) = install_plan(agent, platform)?;
    if let Some(expected) = &b.sha256 {
        let actual = sha256_file(archive)?;
        anyhow::ensure!(
            actual.eq_ignore_ascii_case(expected.trim()),
            "{}: checksum mismatch (expected {expected}, got {actual})",
            agent.id
        );
    }
    let agent_dir = installs.join(&agent.id);
    std::fs::create_dir_all(&agent_dir)
        .with_context(|| format!("create {}", agent_dir.display()))?;
    let tag = unique();
    let tmp = agent_dir.join(format!("{}.tmp-{tag}", agent.version));
    std::fs::create_dir_all(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let unpacked = unpack(archive_kind(url), archive, &tmp, &rel).and_then(|()| {
        let command = contained(&tmp, &rel).or_else(|| {
            // An archive may carry the entry point without its executable bit.
            make_executable(&tmp.join(&rel));
            contained(&tmp, &rel)
        });
        command.with_context(|| format!("{}: {} is not in the archive", agent.id, rel.display()))
    });
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let dest = agent_dir.join(&agent.version);
    let old = agent_dir.join(format!("{}.old-{tag}", agent.version));
    // A stale `.old-*` an interrupted install left is never in the way: the name is unique.
    let _ = std::fs::remove_dir_all(&old);
    // Move the current copy aside, then this one in. A concurrent install of the same version
    // can land between the two steps; then the pair is simply tried again.
    let mut attempts = 0;
    let result = loop {
        attempts += 1;
        let had = match std::fs::rename(&dest, &old) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => break Err(e).with_context(|| format!("move aside {}", dest.display())),
        };
        let moved = std::fs::rename(&tmp, &dest);
        // On failure the old copy goes back, unless another install already took its place.
        if had && (moved.is_ok() || std::fs::rename(&old, &dest).is_err()) {
            let _ = std::fs::remove_dir_all(&old);
        }
        match moved {
            Ok(()) => break Ok(dest.join(&rel)),
            Err(e) if attempts >= 8 => {
                break Err(e).with_context(|| format!("move into {}", dest.display()));
            }
            Err(_) => {}
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// A name suffix no other install in any process shares: the pid, then a per-process counter.
#[cfg(feature = "remote")]
fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

#[cfg(feature = "remote")]
fn sha256_file(p: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(p).with_context(|| format!("open {}", p.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).with_context(|| format!("read {}", p.display()))?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Unpack `archive` into the empty `dir`. `tar` handles tarballs everywhere and zips where it is
/// bsdtar (macOS, Windows); Linux unzips with `unzip`. Both refuse members that climb out by
/// default; what lands is checked again through [`contained`]. A raw executable is copied to
/// `dir/rel`.
#[cfg(feature = "remote")]
fn unpack(kind: ArchiveKind, archive: &Path, dir: &Path, rel: &Path) -> Result<()> {
    let mut cmd = match kind {
        ArchiveKind::Raw => {
            let to = dir.join(rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(archive, &to).with_context(|| format!("copy to {}", to.display()))?;
            make_executable(&to);
            return Ok(());
        }
        ArchiveKind::Zip if cfg!(target_os = "linux") => {
            let mut c = std::process::Command::new("unzip");
            // `-o`: never stop at an overwrite prompt.
            c.arg("-q").arg("-o").arg(archive).arg("-d").arg(dir);
            c
        }
        ArchiveKind::Tar | ArchiveKind::Zip => {
            let mut c = std::process::Command::new("tar");
            // GNU tar reads `host:path` as a remote archive; bsdtar has no such mode and rejects
            // the flag that turns it off.
            if gnu_tar() {
                c.arg("--force-local");
            }
            c.arg("-xf").arg(archive).arg("-C").arg(dir);
            c
        }
    };
    let out = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .context("run the archive tool")?;
    anyhow::ensure!(
        out.status.success(),
        "unpack failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(())
}

/// Whether the `tar` on `PATH` is GNU tar, asked once.
#[cfg(feature = "remote")]
fn gnu_tar() -> bool {
    static GNU: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GNU.get_or_init(|| {
        std::process::Command::new("tar")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("GNU tar"))
    })
}

#[cfg(feature = "remote")]
fn make_executable(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::symlink_metadata(p)
            && meta.is_file()
        {
            let mode = meta.permissions().mode() | 0o755;
            let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode));
        }
    }
    #[cfg(not(unix))]
    let _ = p;
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

    fn inst(command: PathBuf, args: &[&str], env: Vec<(String, String)>, tui: Option<PathBuf>) -> AcpStatus {
        AcpStatus::Installed {
            command,
            args: args.iter().map(|s| s.to_string()).collect(),
            env,
            tui,
            install_root: None,
        }
    }

    fn run(path: &Path, platform: &str) -> HashMap<String, AcpAgentStatus> {
        let reg = parse(FIXTURE).unwrap();
        detect_in(&reg, &HashMap::new(), None, path.as_os_str(), platform)
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
        let nulls = parse(r#"{"agents":[{"id":"a","distribution":{"binary":{"linux-x86_64":
            {"cmd":"./a","args":null,"env":null}}}}]}"#)
        .unwrap();
        assert!(nulls.agents[0].distribution.binary["linux-x86_64"].args.is_empty());
    }

    #[test]
    fn the_bundled_snapshot_parses() {
        let r = bundled();
        assert!(r.agents.len() > 10, "{}", r.agents.len());
        assert!(r.agents.iter().any(|a| a.id == "mistral-vibe"));
    }

    #[test]
    fn load_falls_back_to_the_snapshot_and_ages_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_stale(dir.path(), TTL));
        let (r, origin) = load(dir.path());
        assert_eq!(origin, Origin::Bundled);
        assert!(!r.agents.is_empty());
        std::fs::write(dir.path().join(CACHE_FILE), FIXTURE).unwrap();
        assert!(!is_stale(dir.path(), TTL));
        assert!(is_stale(dir.path(), Duration::ZERO));
        let (r, origin) = load(dir.path());
        assert_eq!(origin, Origin::Fresh);
        assert_eq!(r.agents.len(), 8);
        let old = std::time::SystemTime::now() - Duration::from_secs(48 * 3600);
        std::fs::File::options()
            .write(true)
            .open(dir.path().join(CACHE_FILE))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(is_stale(dir.path(), TTL));
        assert!(matches!(load(dir.path()).1, Origin::Cached { age } if age >= TTL));
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
            inst(goose.clone(), &["acp"], vec![("A".into(), "b".into())], Some(goose.clone()))
        );
        // Another platform key with no entry: not runnable from the binary distribution.
        assert_eq!(run(dir.path(), "windows-x86_64")["goose"].status, AcpStatus::NotInstalled);
    }

    #[test]
    fn cli_itself_serves_acp() {
        let dir = tempfile::tempdir().unwrap();
        let g = exe(dir.path(), "gemini");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(m["gemini"].status, inst(g.clone(), &["--acp"], vec![], Some(g)));
    }

    const VIBE: &str = r#"{"id":"mistral-vibe","version":"3.0.1","distribution":{"binary":{
        "linux-x86_64":{"archive":"https://h/vibe-acp-linux-x86_64-3.0.1.tar.gz","cmd":"./vibe-acp",
                        "sha256":"ab"}}}}"#;

    #[test]
    fn vibe_is_found_by_its_acp_bin_and_its_tui_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            one(VIBE, dir.path()),
            AcpStatus::Downloadable {
                url: "https://h/vibe-acp-linux-x86_64-3.0.1.tar.gz".into(),
                sha256: Some("ab".into()),
            }
        );
        let acp = exe(dir.path(), "vibe-acp");
        assert_eq!(one(VIBE, dir.path()), inst(acp.clone(), &[], vec![], None));
        let vibe = exe(dir.path(), "vibe");
        assert_eq!(one(VIBE, dir.path()), inst(acp, &[], vec![], Some(vibe)));
        assert_eq!(config_homes("mistral-vibe"), &[".vibe"]);
    }

    #[test]
    fn a_binary_only_agent_is_downloadable_over_https_only() {
        let dir = tempfile::tempdir().unwrap();
        let agent = r#"{"id":"amp-acp","version":"1","distribution":{"binary":{"linux-x86_64":
            {"archive":"https://h/amp.tar.gz","cmd":"./amp-acp"}}}}"#;
        assert_eq!(
            one(agent, dir.path()),
            AcpStatus::Downloadable { url: "https://h/amp.tar.gz".into(), sha256: None }
        );
        let http = agent.replace("https://", "http://");
        assert_eq!(one(&http, dir.path()), AcpStatus::NotInstalled);
        let climb = agent.replace("./amp-acp", "../amp-acp");
        assert_eq!(one(&climb, dir.path()), AcpStatus::NotInstalled);
    }

    #[test]
    fn a_pinned_package_runs_through_its_runner() {
        let dir = tempfile::tempdir().unwrap();
        let agent = r#"{"id":"autohand","version":"1","distribution":{"npx":
            {"package":"@autohandai/autohand-acp@0.2.1","args":["--acp"],"env":{"X":"1"}}}}"#;
        assert_eq!(one(agent, dir.path()), AcpStatus::NotInstalled);
        let npx = exe(dir.path(), "npx");
        assert_eq!(
            one(agent, dir.path()),
            AcpStatus::ViaRunner {
                runner: npx,
                installer: Installer::Npm,
                package: "@autohandai/autohand-acp@0.2.1".into(),
                args: vec!["--acp".into()],
                env: vec![("X".into(), "1".into())],
            }
        );
        // An unpinned package is never run through a runner.
        let loose = agent.replace("@0.2.1", "@^0.2");
        assert_eq!(one(&loose, dir.path()), AcpStatus::NotInstalled);
        let uvx = exe(dir.path(), "uvx");
        let uv = r#"{"id":"minion","distribution":{"uvx":{"package":"minion-code==0.1.44"}}}"#;
        assert!(matches!(one(uv, dir.path()), AcpStatus::ViaRunner { runner, installer: Installer::Uv, .. } if runner == uvx));
        assert!(!pinned_package(Installer::Uv, "x>=1"));
        assert!(pinned_package(Installer::Npm, "@x/y@1.2.3-beta.1"));
        assert!(!pinned_package(Installer::Npm, "@x/y"));
        assert!(!pinned_package(Installer::Npm, "y@latest"));
    }

    #[test]
    fn a_runner_gets_no_env_that_redirects_its_index() {
        let dir = tempfile::tempdir().unwrap();
        exe(dir.path(), "npx");
        let agent = r#"{"id":"autohand","distribution":{"npx":{"package":"a@1.0.0","env":{
            "NPM_CONFIG_REGISTRY":"x","npm_config_registry":"x","UV_INDEX_URL":"x",
            "PIP_INDEX_URL":"x","pip_extra_index_url":"x","KEEP":"1"}}}}"#;
        let AcpStatus::ViaRunner { env, .. } = one(agent, dir.path()) else { panic!() };
        assert_eq!(env, vec![("KEEP".to_string(), "1".to_string())]);
    }

    #[test]
    fn claude_and_codex_packages_never_come_back_under_another_id() {
        let dir = tempfile::tempdir().unwrap();
        exe(dir.path(), "npx");
        exe(dir.path(), "uvx");
        for pkg in [
            r#"{"npx":{"package":"@zed-industries/claude-code-acp@1.0.0"}}"#,
            r#"{"npx":{"package":"claude-agent-acp@1.0.0"}}"#,
            r#"{"npx":{"package":"@other/codex-acp@2.0.0"}}"#,
            r#"{"uvx":{"package":"codex-acp==1.0"}}"#,
        ] {
            let reg = parse(&format!(r#"{{"agents":[{{"id":"renamed","distribution":{pkg}}}]}}"#)).unwrap();
            assert!(detect_in(&reg, &HashMap::new(), None, dir.path().as_os_str(), "linux-x86_64").is_empty(), "{pkg}");
        }
        assert_eq!(package_name("@s/codex-acp@1"), "codex-acp");
        assert_eq!(package_name("x==1"), "x");
    }

    #[test]
    fn installs_dir_wins_over_path_and_reports_an_update() {
        let installs = tempfile::tempdir().unwrap();
        let path = tempfile::tempdir().unwrap();
        exe(path.path(), "vibe-acp");
        let old = installs.path().join("mistral-vibe/2.9.0");
        std::fs::create_dir_all(&old).unwrap();
        let acp = exe(&old, "vibe-acp");
        let vibe = exe(&old, "vibe");
        // A half-finished install is never picked.
        let tmp = installs.path().join("mistral-vibe/9.0.0.tmp-1");
        std::fs::create_dir_all(&tmp).unwrap();
        exe(&tmp, "vibe-acp");
        let reg = parse(&format!(r#"{{"agents":[{VIBE}]}}"#)).unwrap();
        let s = detect_in(&reg, &HashMap::new(), Some(installs.path()), path.path().as_os_str(), "linux-x86_64")
            .remove(0);
        assert_eq!(
            s.status,
            AcpStatus::Installed {
                command: acp,
                args: vec![],
                env: vec![],
                tui: Some(vibe),
                install_root: Some(old.clone()),
            }
        );
        assert_eq!(s.update.as_deref(), Some("3.0.1"));
        // The registry's own version present: no update.
        let cur = installs.path().join("mistral-vibe/3.0.1");
        std::fs::create_dir_all(&cur).unwrap();
        exe(&cur, "vibe-acp");
        let s = detect_in(&reg, &HashMap::new(), Some(installs.path()), path.path().as_os_str(), "linux-x86_64")
            .remove(0);
        assert!(matches!(&s.status, AcpStatus::Installed { install_root: Some(r), .. } if *r == cur));
        assert_eq!(s.update, None);
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_cmd_may_not_symlink_out_of_its_dir() {
        let installs = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = exe(outside.path(), "vibe-acp");
        let v = installs.path().join("mistral-vibe/3.0.1");
        std::fs::create_dir_all(&v).unwrap();
        std::os::unix::fs::symlink(&target, v.join("vibe-acp")).unwrap();
        let empty = tempfile::tempdir().unwrap();
        let reg = parse(&format!(r#"{{"agents":[{VIBE}]}}"#)).unwrap();
        let s = detect_in(&reg, &HashMap::new(), Some(installs.path()), empty.path().as_os_str(), "linux-x86_64");
        assert!(matches!(s[0].status, AcpStatus::Downloadable { .. }));
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version_newer("3.0.10", "3.0.9"));
        assert!(version_newer("v2", "1.9"));
        assert!(!version_newer("1.0", "1.0"));
        assert!(version_newer("1.0.1", "1.0"));
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
        detect_in(&reg, &HashMap::new(), None, path.as_os_str(), "linux-x86_64")
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
        // Not on PATH, and an interpreter is never offered for download either.
        let dl = r#"{"id":"evil","version":"1","distribution":{"binary":{"linux-x86_64":
            {"archive":"https://h/x.tar.gz","cmd":"./bash"}}}}"#;
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(one(dl, empty.path()), AcpStatus::NotInstalled);
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
            inst(g, &[], vec![("GOOSE_MODE".into(), "acp".into())], None)
        );
    }

    #[test]
    fn an_unknown_id_is_never_found_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let agent = r#"{"id":"foo-acp","distribution":{"npx":{"package":"foo-acp@1"}}}"#;
        exe(dir.path(), "foo");
        exe(dir.path(), "npm");
        exe(dir.path(), "foo-acp");
        assert_eq!(one(agent, dir.path()), AcpStatus::NotInstalled);
        // Its pinned package still runs through the runner.
        exe(dir.path(), "npx");
        assert!(matches!(one(agent, dir.path()), AcpStatus::ViaRunner { .. }));
        // A program that merely shares the registry's `cmd` is not the agent.
        exe(dir.path(), "git");
        let git = r#"{"id":"gitagent","distribution":{"binary":{"linux-x86_64":{"cmd":"./git","args":["acp"]}}}}"#;
        assert_eq!(one(git, dir.path()), AcpStatus::NotInstalled);
        let git_id = r#"{"id":"git","distribution":{"binary":{"linux-x86_64":{"cmd":"./git"}}}}"#;
        assert_eq!(one(git_id, dir.path()), AcpStatus::NotInstalled);
        // The user's own override still reaches any id.
        let custom = exe(dir.path(), "my-foo");
        let reg = parse(&format!(r#"{{"agents":[{agent}]}}"#)).unwrap();
        let over = HashMap::from([("foo-acp".to_string(), custom.clone())]);
        let s = detect_in(&reg, &over, None, dir.path().as_os_str(), "linux-x86_64").remove(0);
        assert!(matches!(s.status, AcpStatus::Installed { command, .. } if command == custom));
        // A generic word is never probed as an agent.
        exe(dir.path(), "kilo");
        let kilo = r#"{"id":"kilo","distribution":{"npx":{"package":"kilo@1"}}}"#;
        assert!(!matches!(one(kilo, dir.path()), AcpStatus::Installed { .. }));
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
            update: None,
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
        // `pi` is a generic word: never run as the interface.
        assert_eq!(m["pi-acp"].status, inst(a, &[], vec![], None));
    }

    #[test]
    fn an_unknown_uvx_agent_runs_only_through_uvx() {
        let dir = tempfile::tempdir().unwrap();
        exe(dir.path(), "fast-agent");
        let m = run(dir.path(), "linux-x86_64");
        assert_eq!(m["fast-agent"].status, AcpStatus::NotInstalled);
        let uvx = exe(dir.path(), "uvx");
        let m = run(dir.path(), "linux-x86_64");
        assert!(matches!(&m["fast-agent"].status, AcpStatus::ViaRunner { runner, .. } if *runner == uvx));
    }

    #[test]
    fn override_wins() {
        let dir = tempfile::tempdir().unwrap();
        let custom = exe(dir.path(), "my-gemini");
        let reg = parse(FIXTURE).unwrap();
        let over = HashMap::from([("gemini".to_string(), custom.clone())]);
        let empty = tempfile::tempdir().unwrap();
        let out = detect_in(&reg, &over, None, empty.path().as_os_str(), "linux-x86_64");
        let g = out.iter().find(|s| s.id == "gemini").unwrap();
        assert_eq!(g.status, inst(custom.clone(), &["--acp"], vec![], Some(custom)));
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

    #[test]
    fn cmd_paths_stay_relative() {
        assert_eq!(rel_cmd("./bin/devin"), Some(PathBuf::from("bin/devin")));
        assert_eq!(rel_cmd("./dist-package\\cursor-agent.cmd"), Some(PathBuf::from("dist-package/cursor-agent.cmd")));
        for bad in ["", ".", "../x", "a/../../x", "/bin/sh", "\\x", "C:\\x", "a/.."] {
            assert_eq!(rel_cmd(bad), None, "{bad}");
        }
    }

    #[cfg(feature = "remote")]
    mod install {
        use super::*;

        fn agent(url: &str, cmd: &str, sha256: Option<&str>) -> RegistryAgent {
            let sha = sha256.map_or("null".to_string(), |s| format!("{s:?}"));
            let json = format!(
                r#"{{"agents":[{{"id":"fix","version":"1.0.0","distribution":{{"binary":{{
                  "test-x":{{"archive":{url:?},"cmd":{cmd:?},"sha256":{sha}}}}}}}}}]}}"#
            );
            parse(&json).unwrap().agents.remove(0)
        }

        /// A tarball (or zip) holding `bin/fix`, built with the system tools.
        fn fixture(kind: &str) -> Option<(tempfile::TempDir, PathBuf)> {
            let dir = tempfile::tempdir().unwrap();
            let src = dir.path().join("src");
            std::fs::create_dir_all(src.join("bin")).unwrap();
            exe(&src.join("bin"), "fix");
            let out = dir.path().join(format!("fix.{kind}"));
            let status = match kind {
                "tar.gz" => std::process::Command::new("tar")
                    .arg("-czf")
                    .arg(&out)
                    .arg("-C")
                    .arg(&src)
                    .arg("bin")
                    .status(),
                _ => std::process::Command::new("zip")
                    .current_dir(&src)
                    .arg("-qr")
                    .arg(&out)
                    .arg("bin")
                    .status(),
            };
            status.ok().filter(|s| s.success())?;
            Some((dir, out))
        }

        #[test]
        fn a_tarball_installs_and_is_then_detected() {
            let (_d, archive) = fixture("tar.gz").expect("tar");
            let sha = sha256_file(&archive).unwrap();
            let a = agent("https://h/fix.tar.gz", "./bin/fix", Some(&sha));
            let installs = tempfile::tempdir().unwrap();
            let cmd = install_archive(installs.path(), &a, "test-x", &archive).unwrap();
            assert_eq!(cmd, installs.path().join("fix/1.0.0/bin/fix"));
            assert!(is_executable(&cmd));
            // Reinstalling replaces it in place, and leaves no temporary behind.
            install_archive(installs.path(), &a, "test-x", &archive).unwrap();
            let names: Vec<_> = std::fs::read_dir(installs.path().join("fix"))
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec![OsString::from("1.0.0")]);
            let reg = Registry { version: String::new(), agents: vec![a] };
            let empty = tempfile::tempdir().unwrap();
            let s = detect_in(&reg, &HashMap::new(), Some(installs.path()), empty.path().as_os_str(), "test-x");
            assert!(matches!(&s[0].status, AcpStatus::Installed { command, .. } if *command == cmd));
        }

        #[test]
        fn concurrent_installs_in_one_process_do_not_collide() {
            assert_ne!(unique(), unique());
            let (_d, archive) = fixture("tar.gz").expect("tar");
            let a = agent("https://h/fix.tar.gz", "./bin/fix", None);
            let installs = tempfile::tempdir().unwrap();
            // A stale move-aside from an interrupted install is no obstacle.
            std::fs::create_dir_all(installs.path().join(format!("fix/1.0.0.old-{}-0", std::process::id()))).unwrap();
            std::thread::scope(|s| {
                let runs: Vec<_> = (0..4)
                    .map(|_| s.spawn(|| install_archive(installs.path(), &a, "test-x", &archive)))
                    .collect();
                for r in runs {
                    let cmd = r.join().unwrap().unwrap();
                    assert_eq!(cmd, installs.path().join("fix/1.0.0/bin/fix"));
                }
            });
            assert!(is_executable(&installs.path().join("fix/1.0.0/bin/fix")));
            let left: Vec<_> = std::fs::read_dir(installs.path().join("fix"))
                .unwrap()
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .filter(|n| n.contains(".tmp-"))
                .collect();
            assert!(left.is_empty(), "{left:?}");
        }

        #[test]
        fn a_zip_installs() {
            let Some((_d, archive)) = fixture("zip") else {
                eprintln!("no zip tool; skipped");
                return;
            };
            let a = agent("https://h/fix.zip?x=1", "./bin/fix", None);
            let installs = tempfile::tempdir().unwrap();
            let cmd = install_archive(installs.path(), &a, "test-x", &archive).unwrap();
            assert!(is_executable(&cmd));
        }

        #[test]
        fn a_raw_executable_is_copied_and_made_executable() {
            let dir = tempfile::tempdir().unwrap();
            let raw = dir.path().join("download");
            std::fs::write(&raw, "#!/bin/sh\n").unwrap();
            let a = agent("https://h/fix-linux-amd64", "./fix-linux-amd64", None);
            let installs = tempfile::tempdir().unwrap();
            let cmd = install_archive(installs.path(), &a, "test-x", &raw).unwrap();
            assert_eq!(cmd, installs.path().join("fix/1.0.0/fix-linux-amd64"));
            assert!(is_executable(&cmd));
        }

        #[test]
        fn a_checksum_mismatch_fails_and_installs_nothing() {
            let (_d, archive) = fixture("tar.gz").expect("tar");
            let a = agent("https://h/fix.tar.gz", "./bin/fix", Some(&"0".repeat(64)));
            let installs = tempfile::tempdir().unwrap();
            let e = install_archive(installs.path(), &a, "test-x", &archive).unwrap_err();
            assert!(e.to_string().contains("checksum"), "{e:#}");
            assert!(!installs.path().join("fix/1.0.0").exists());
        }

        #[test]
        fn climbing_or_absolute_cmds_and_plain_http_are_refused() {
            let (_d, archive) = fixture("tar.gz").expect("tar");
            let installs = tempfile::tempdir().unwrap();
            for (url, cmd) in [
                ("https://h/fix.tar.gz", "../bin/fix"),
                ("https://h/fix.tar.gz", "bin/../../fix"),
                ("https://h/fix.tar.gz", "/bin/fix"),
                ("http://h/fix.tar.gz", "./bin/fix"),
            ] {
                let a = agent(url, cmd, None);
                assert!(install_archive(installs.path(), &a, "test-x", &archive).is_err(), "{url} {cmd}");
            }
            // A cmd the archive does not contain fails, and cleans its temporary up.
            let a = agent("https://h/fix.tar.gz", "./bin/missing", None);
            assert!(install_archive(installs.path(), &a, "test-x", &archive).is_err());
            assert_eq!(std::fs::read_dir(installs.path().join("fix")).unwrap().count(), 0);
        }
    }
}
