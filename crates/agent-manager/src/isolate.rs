//! Composes an isolated run: a provisioned [`Launch`] plus a [`RunSpec`]
//! become an isol8 [`Spec`](isol8::Spec) — the policy the harness runs under.
//!
//! This module owns the *translation*, not the enforcement: it decides which
//! directories a run may touch, which environment reaches it and where its
//! `$HOME` is, then hands an isol8 `Spec` and a [`Context`](isol8::Context) to
//! whoever spawns the child. Nothing here spawns, and nothing here reads the
//! process environment or discovers a config file — an embedder passes
//! [`IsolateOptions`] and gets the same answer the CLI does, which is the
//! front-end-agnostic invariant the rest of the core keeps.
//!
//! Four facts drive everything below:
//!
//! - **The run dir is writable, the sources behind it are readable.** The
//!   provisioner symlinks-else-copies a profile's overlay and a skill's folder
//!   into the ephemeral config dir ([`crate::overlay`],
//!   [`crate::source::LinkMode::LinkElseCopy`]), so a deny-by-default policy
//!   that granted only the run dir would break on the first symlink followed
//!   out of it.
//! - **The harness's own environment is injected, not inherited.** isol8
//!   sanitizes the environment down to an allowlist, so every variable the
//!   provisioner computed — `CLAUDE_CONFIG_DIR` and its siblings — is passed
//!   as an explicit `set_env` or the harness reads its real config instead of
//!   the throwaway one.
//! - **A harness draws a screen.** `TERM` and `COLORTERM` are not in isol8's
//!   allowlist, and a harness with neither cannot decide what it may draw.
//! - **A development agent is a build tool's parent.** `cargo`, `npm`,
//!   `dotnet` and `git` are never the confined command — they are children of
//!   it — and isol8 auto-selects a layer only from `cmd[0]`. So every
//!   toolchain layer is *named* here ([`DEV_LAYERS`]), and the run keeps the
//!   real `$HOME` those layers' `~/.cargo`-shaped grants resolve against.
//!
//! See `_docs/architecture.md` for where the isolate stage sits, `_docs/cli.md`
//! for the `--isolate[=profile]` / `--no-isolate` / `[isolate]` surface, and
//! `refs/isol8/_docs/integration.md` for the host-integration contract this
//! module implements.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tracing::debug;
use tracing::info;

use crate::Result;
use crate::harness::Launch;
use crate::source::Source;
use crate::spec::{ConfigStrategy, Isolation, RunSpec};

/// NT device paths a confined process needs open before it can use a socket.
///
/// isol8's Windows hook checks every `NtCreateFile` against the policy, and it
/// checks the object name it is handed without exempting anything that is not a
/// file. Winsock creates a socket by opening the `Afd` device, so a policy that
/// names only directories denies the socket itself: a confined harness cannot
/// reach the network at all, and the symptom is whatever that harness says when
/// a bind fails — `claude auth login` reports its OAuth callback server refusing
/// to start.
///
/// Empty on every other platform. On Windows these are a capability grant rather
/// than a path one: they say a confined run may use the network, which is what
/// an agent is for. Filed against isol8 in `_docs/inbox/isol8-upstream.md`.
#[cfg(target_os = "windows")]
const WINDOWS_DEVICE_RW: &[&str] = &[r"\Device\Afd", r"\Device\Nsi"];
#[cfg(not(target_os = "windows"))]
const WINDOWS_DEVICE_RW: &[&str] = &[];

/// Where a confined run's `$HOME` comes from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HomeMode {
    /// The real home, the way an unconfined run has it — isol8's own default
    /// posture, and the only one under which a toolchain works.
    ///
    /// A layer's `~`-relative grant expands against the *effective* home, so
    /// under a replaced home `toolchains/rust`'s `~/.cargo` names an empty
    /// scratch directory: the agent holds `cargo` in its `PATH` and still
    /// cannot build. Inheriting opens nothing on its own — only the paths a
    /// layer names are reachable inside the home, and `refs/isol8`'s own
    /// `profiles/base.toml` says the same ("HOME is NOT replaced by default").
    #[default]
    Inherit,
    /// A scratch home for this run alone, discarded with it.
    Ephemeral,
    /// A persistent home under the state dir, named by the caller, so a
    /// harness's caches and logins survive from one run to the next.
    Managed(String),
}

impl HomeMode {
    /// Parse the `[isolate] home` setting: `"inherit"`, `"ephemeral"` or
    /// `"managed"`.
    ///
    /// `"managed"` needs a name to key the home by, which is the caller's to
    /// supply — the settings file only chooses the mode.
    pub fn parse(value: &str, managed_id: &str) -> Result<Self> {
        match value {
            "inherit" => Ok(Self::Inherit),
            "ephemeral" => Ok(Self::Ephemeral),
            "managed" => Ok(Self::Managed(managed_id.to_string())),
            other => Err(anyhow!(
                "unknown [isolate] home {other:?}: expected \"inherit\", \"ephemeral\" or \"managed\""
            )),
        }
    }
}

/// What the caller decides about isolation, independent of any run.
///
/// The state dir is the caller's: isol8 reads its profile layers from it and
/// writes managed homes under it, and nothing outside it is touched. The CLI
/// roots it at `<am config dir>/isol8`; an embedder roots it wherever it keeps
/// its own state.
#[derive(Debug, Clone)]
pub struct IsolateOptions {
    /// Root for generated layers (`<state_dir>/profiles`) and managed homes
    /// (`<state_dir>/homes`).
    pub state_dir: PathBuf,
    /// Where the run's `$HOME` comes from.
    pub home: HomeMode,
    /// Extra read-only grants, for anything the caller knows the harness
    /// needs and the run cannot discover — a shared toolchain, a cache.
    pub extra_ro: Vec<PathBuf>,
    /// Extra read-write grants, for a toolchain root the defaults here cannot
    /// know: a relocated `CARGO_HOME`, a shared package store on another
    /// volume, an SDK installed outside `$HOME`.
    ///
    /// The shipped layers grant a toolchain's *default* location, so a
    /// non-standard install needs its root named here — passing the matching
    /// variable through [`ENV_PASS`] tells a tool where to look but grants it
    /// nothing. Passed through as given: an absent path renders as an inert
    /// grant, so a caller never has to pre-check one.
    pub extra_rw: Vec<PathBuf>,
}

impl IsolateOptions {
    /// Options rooted at `state_dir`, with the real home and no extra grants.
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            home: HomeMode::Inherit,
            extra_ro: Vec::new(),
            extra_rw: Vec::new(),
        }
    }

    /// Grant the toolchain roots this host has relocated, read from the
    /// environment.
    ///
    /// The shipped layers grant a toolchain's *default* location — `~/.cargo`,
    /// `~/.rustup`, `~/go` — so a host that installed one elsewhere gets a
    /// policy that names the wrong directory and a run that cannot build.
    /// [`ENV_PASS`] already forwards the variables that say where a root
    /// really is; this grants what those variables name, because forwarding
    /// `CARGO_HOME` tells `cargo` where to look and opens nothing.
    ///
    /// **The environment read is the caller's, deliberately.** [`plan`] must
    /// answer identically for a given `IsolateOptions` — that is the
    /// front-end-agnostic invariant this module keeps — so a host calls this
    /// where it decides to consult its own environment, and `plan` still only
    /// consumes options. Call it before appending a user's own grants, so an
    /// explicit grant is the last word on a path the environment also named.
    pub fn grant_toolchains_from_env(&mut self) {
        self.grant_toolchains(|name| std::env::var_os(name));
    }

    /// Grant a run the config home it runs from (`D194`), read-write: the account's home under
    /// [`ConfigStrategy::Home`], the harness's own [`Harness::default_homes`] under
    /// [`ConfigStrategy::Native`]. The policy grants the run's scratch dir by itself, never the
    /// home, which is shared by every run as the account — or is the user's own. A per-run dir
    /// needs nothing more.
    ///
    /// [`Harness::default_homes`]: crate::harness::Harness::default_homes
    pub fn grant_config_home(
        &mut self,
        harness: &dyn crate::harness::Harness,
        config: &ConfigStrategy,
    ) {
        let homes = match config {
            ConfigStrategy::Home { home, .. } => vec![home.clone()],
            ConfigStrategy::Native { .. } => harness.default_homes(),
            ConfigStrategy::Ephemeral | ConfigStrategy::Fixed(_) => Vec::new(),
        };
        for home in homes {
            if !self.extra_rw.contains(&home) {
                self.extra_rw.push(home);
            }
        }
    }

    /// [`grant_toolchains_from_env`](Self::grant_toolchains_from_env) over a
    /// caller-supplied lookup, which is what makes it testable: a test that
    /// mutated the process environment would race every other test in the
    /// binary.
    ///
    /// Public because a host's answer is not always the process's. A GUI
    /// application never ran a login shell, so `CARGO_HOME` and its siblings
    /// are simply absent from its environment; an embedder that keeps its own
    /// record of where this machine put its toolchains passes that here.
    pub fn grant_toolchains<F>(&mut self, lookup: F)
    where
        F: Fn(&str) -> Option<std::ffi::OsString>,
    {
        for (name, writable) in TOOLCHAIN_ROOT_VARS {
            let Some(value) = lookup(name) else { continue };
            let path = PathBuf::from(value);
            // A relative value would resolve against nothing — isol8 grants
            // absolute paths, so naming one relatively grants the wrong tree
            // rather than failing. Not existence-filtered, for the same reason
            // `DEV_RW_HOME_ROOTS` is not: a root the tool creates on first use
            // needs the grant in order to be created.
            if !path.is_absolute() {
                continue;
            }
            let grants = match writable {
                true => &mut self.extra_rw,
                false => &mut self.extra_ro,
            };
            if !grants.contains(&path) {
                grants.push(path);
            }
        }
    }
}

/// Toolchain roots a host may have relocated, and the access a build needs at
/// each.
///
/// These names are the relocation half of [`ENV_PASS`], and the two must stay
/// in step: passing a variable without granting what it names leaves a tool
/// pointed at a path it cannot read, which is the failure that looks like a
/// broken toolchain rather than a policy.
///
/// An SDK tree is read-only — a build writes to its cache, not to `GOROOT` or
/// `JAVA_HOME` — and everything else is a cache, a registry or an install root
/// a build genuinely writes to. `SDKMAN_DIR` is read-only to match the choice
/// a real tuned setup already made for it.
///
/// A writable root can hold a credential: `CARGO_HOME` carries
/// `credentials.toml`, a crates.io token, and the same is true of `~/.npmrc`
/// under a relocated npm prefix. That is not a posture this table invents —
/// `toolchains/rust` already grants `~/.cargo` read-write on a default
/// install, so relocating the root changes where the exposure is and not
/// whether it exists. Narrowing it means a deny on the credential file
/// specifically, which needs a generated layer rather than a path list; until
/// something asks for one, a confined agent can read the tokens its own
/// toolchain uses.
const TOOLCHAIN_ROOT_VARS: &[(&str, bool)] = &[
    // (variable, writable)
    ("CARGO_HOME", true),  // registry, git checkouts, .package-cache
    ("RUSTUP_HOME", true), // toolchains, downloads, tmp
    ("GOROOT", false),
    ("GOPATH", true),
    ("GOMODCACHE", true),
    ("GOCACHE", true),
    ("JAVA_HOME", false),
    ("SDKMAN_DIR", false),
    ("PYENV_ROOT", true),
    ("MISE_DATA_DIR", true),
    ("NPM_CONFIG_PREFIX", true),
    ("PNPM_HOME", true),
    ("VOLTA_HOME", true),
    ("BUN_INSTALL", true),
    ("DOTNET_ROOT", true),
    ("NUGET_PACKAGES", true),
    ("PUB_CACHE", true),
];

/// The user's real home, whatever a run's own `$HOME` was set to.
///
/// Exposed because an embedder has to resolve a `~`-prefixed grant *before* handing it over:
/// inside a layer `~` means the run's effective home, so passing the token through would grant
/// the replacement directory under [`HomeMode::Ephemeral`] or [`HomeMode::Managed`] rather than
/// the directory the person meant. Nothing else in an embedder should need to name a home.
pub fn real_home() -> PathBuf {
    isol8::home::real_home()
}

/// Whether this process can confine anything at all.
///
/// A sandbox cannot nest, so a process that is itself confined can compose a
/// policy but never apply one. An embedder asks once, at startup, and says so
/// — the alternative is the same failure arriving as every run's error.
pub fn ensure_can_confine() -> Result<()> {
    isol8::sandbox::ensure_not_nested().context("a sandbox cannot nest")
}

/// A run resolved into the policy that will confine it.
///
/// Both halves are needed to spawn: the `Spec` says what is allowed, the
/// `Context` says what the paths in it mean. Holding them together keeps a
/// caller from resolving one against an ambient environment and the other
/// against host state.
#[derive(Debug, Clone)]
pub struct Confined {
    /// The merged policy, command included.
    pub spec: isol8::Spec,
    /// The ambient state the policy resolves against — built from the
    /// caller's own directories, never from the process environment.
    pub ctx: isol8::Context,
}

/// The layers that make a confined run a *development* environment rather
/// than a chat window.
///
/// isol8 auto-selects a layer only when it declares `filter.executables`, and
/// only the `agents/*` layers do — so the harness's own layer arrives on its
/// own and every `toolchains/*` layer would sit unreachable in the registry
/// forever. Naming them is the whole of the fix: a build tool is a child of
/// the confined command, never `cmd[0]`, so nothing about the command can ever
/// reveal it.
///
/// Naming a layer whose `filter.os` does not match this host is free — it is
/// kept as an empty shell rather than dropped — so one list is correct on
/// every platform. Naming one that does not *exist* is a hard error, and not
/// one this function raises: `plan` never loads the layer registry, so a bad
/// name here surfaces on the first real spawn rather than in a `plan` test.
/// That is what `dev_layers_all_resolve` guards.
///
/// Two of these cannot be inferred from isol8's documentation; both were found
/// by debugging a real confined agent, and both are named here because only
/// *some* `agents/*` layers happen to require them:
///
/// - `integrations/keychain` — rustls-based tools (`mise`, `cargo`) validate
///   TLS through the trust daemon rather than a CA file, so without it they
///   cannot fetch at all, and `SSL_CERT_FILE` does not fix them.
/// - `integrations/macos-gui` — a harness whose TUI calls
///   `CGSEventSourceForID` at startup deadlocks forever on a WindowServer
///   mutex when the `com.apple.windowserver.active` lookup is denied. It
///   presents as a hang on the splash screen: no log line, no permission
///   error.
///
/// `toolchains/apple-toolchain-core` is not optional on macOS for two
/// reasons: `cargo build` links through `cc`/`ld` under
/// `/Library/Developer/CommandLineTools`, which the system-runtime layer does
/// not grant, and `/usr/bin/git` is an xcode-select shim that cannot resolve
/// the active developer dir inside a sandbox.
///
/// One name here is not isol8's: `toolchains/dotnet` is generated by this
/// crate ([`write_dotnet_layer`]) because isol8 ships none, and it resolves
/// through the profile path `plan` adds for it.
///
/// See [`BROKEN_LAYERS`] before adding a layer here.
pub const DEV_LAYERS: &[&str] = &[
    // Not optional: see the note above.
    "integrations/keychain",
    "integrations/macos-gui",
    // `~/.gitconfig` — without it a commit has no author, which fails as
    // "please tell me who you are" rather than as a denial. Brings
    // `shared/agent-common` with it via `requires`, so that is not named here.
    "integrations/git",
    // Version managers first: they own the shims every other toolchain
    // resolves through.
    "toolchains/runtime-managers",
    "toolchains/apple-toolchain-core",
    "toolchains/rust",
    "toolchains/node",
    "toolchains/python",
    "toolchains/go",
    "toolchains/java",
    // Generated here, not shipped by isol8 — see [`write_dotnet_layer`].
    "toolchains/dotnet",
    "toolchains/bun",
    "toolchains/deno",
    "toolchains/ruby",
    "toolchains/php",
    "toolchains/perl",
    "toolchains/elixir",
];

/// The .NET layer this crate generates, because isol8 (v0.4.0) ships no
/// `toolchains/*` layer for it.
///
/// Named like a built-in and written into its own profile path, so the rest of
/// the code treats it exactly like the layers it sits beside in [`DEV_LAYERS`].
/// When isol8 grows one, delete [`write_dotnet_layer`] and its profile path:
/// the name in `DEV_LAYERS` keeps working and starts resolving to the built-in.
const DOTNET_LAYER: &str = "toolchains/dotnet";

/// What a confined `dotnet` needs beyond the SDK tree itself.
///
/// The install tree is already readable — `base` grants `/usr` (so
/// `/usr/local/share/dotnet` and `/usr/share/dotnet`), `macos/system-runtime`
/// grants `/opt` for a Homebrew install, and `windows/system-runtime` grants
/// `%PROGRAMFILES%`. What no shipped layer grants is the *home* side, and that
/// is the first thing every `dotnet` command writes: `~/.dotnet` holds the
/// first-use sentinel it creates before doing anything else. Denied, the CLI
/// reports that it cannot determine a home directory rather than a permission
/// error, and the reaction it invites is pointing `DOTNET_CLI_HOME` at the
/// working tree — a build that writes its SDK state into the repository.
///
/// [`DEV_RW_HOME_ROOTS`] already grants most of these, and it is the wrong
/// half of the answer on its own: those grants are joined absolutely against
/// the **real** home, and nothing grants a replacement home wholesale — isol8's
/// override layer contributes the cwd and the caller's own dirs, and no layer
/// names `~` itself. So under [`HomeMode::Ephemeral`] or [`HomeMode::Managed`]
/// the SDK was denied `$HOME/.dotnet` while `~/.dotnet` in the real home stayed
/// writable, which is exactly the failure this layer was written for. A layer's
/// `~` expands against the *effective* home, so the two halves together cover a
/// run under either home mode.
///
/// The Windows half is a `filter = { os = ["windows"] }` policy rather than a
/// second layer, so one name covers both platforms: a non-matching policy is
/// dropped at resolve time, and a `~`-relative grant expands on Windows too.
const DOTNET_LAYER_BODY: &str = r#"# toolchains/dotnet — generated by agent-manager, do not edit.
# isol8 v0.4.0 ships no .NET layer; this is the one DEV_LAYERS names.
# See DOTNET_LAYER_BODY in crates/agent-manager/src/isolate.rs.

paths = [
  # CLI home: first-use sentinel, telemetry state, global tools (~/.dotnet/tools).
  { path = "~/.dotnet", access = "rw" },
  # NuGet: config (~/.nuget/NuGet/NuGet.Config), the package root
  # (~/.nuget/packages, what NUGET_PACKAGES relocates) and plugins.
  { path = "~/.nuget", access = "rw" },
  { path = "~/.config/NuGet", access = "rw" },
  { path = "~/.local/share/NuGet", access = "rw" },
  # `dotnet new` template engine state.
  { path = "~/.templateengine", access = "rw" },
  # ASP.NET Core: `dotnet dev-certs` output and data-protection keys.
  { path = "~/.aspnet", access = "rw" },
  # `dotnet user-secrets` (~/.microsoft/usersecrets).
  { path = "~/.microsoft", access = "rw" },
  # The install-location marker the muxer reads to find the SDK root.
  { path = "/etc/dotnet", access = "ro" },
  { path = "/private/etc/dotnet", access = "ro" },
]

[[policies]]
filter = { os = ["windows"] }
paths = [
  { path = "%USERPROFILE%\\.dotnet", access = "rw" },
  { path = "%USERPROFILE%\\.nuget", access = "rw" },
  { path = "%USERPROFILE%\\.templateengine", access = "rw" },
  { path = "%USERPROFILE%\\.aspnet", access = "rw" },
  { path = "%USERPROFILE%\\.microsoft", access = "rw" },
  # NuGet.Config lives under roaming AppData, its caches under local.
  { path = "%APPDATA%\\NuGet", access = "rw" },
  { path = "%LOCALAPPDATA%\\NuGet", access = "rw" },
  # A per-user SDK install (dotnet-install.ps1, winget) and MSBuild node state.
  { path = "%LOCALAPPDATA%\\Microsoft\\dotnet", access = "rw" },
  { path = "%LOCALAPPDATA%\\Microsoft\\MSBuild", access = "rw" },
  { path = "%LOCALAPPDATA%\\Microsoft", access = "metadata", match = "literal" },
  # Machine-wide SDK tree and NuGet fallback folders.
  { path = "%PROGRAMFILES%\\dotnet", access = "ro" },
  { path = "%ALLUSERSPROFILE%\\NuGet", access = "ro" },
  { path = "%ALLUSERSPROFILE%\\Microsoft\\NuGet", access = "ro" },
]
"#;

/// Write the [`DOTNET_LAYER`] layer into its own directory under `state_dir`,
/// and return that directory for [`isol8::Config::profile_paths`].
///
/// Its own directory, ahead of the caller's profile root in the path list, so
/// that someone who writes their own `toolchains/dotnet` under that root wins:
/// a later profile path replaces a same-named layer from an earlier one.
fn write_dotnet_layer(options: &IsolateOptions) -> Result<PathBuf> {
    let root = options.state_dir.join("profiles-dotnet");
    // A layer's name *is* its path under the profile root, so both derive from
    // the constant.
    let (group, name) = DOTNET_LAYER
        .split_once('/')
        .context("the dotnet layer name must be <group>/<layer>")?;
    let dir = root.join(group);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating the isolation profile dir {}", dir.display()))?;
    let path = dir.join(format!("{name}.toml"));
    // Rewritten whenever it differs, so an older version's copy cannot outlive
    // the code that expects it.
    if !std::fs::read_to_string(&path).is_ok_and(|held| held == DOTNET_LAYER_BODY) {
        std::fs::write(&path, DOTNET_LAYER_BODY)
            .with_context(|| format!("writing the isolation profile {}", path.display()))?;
    }
    Ok(root)
}

/// Layers that must never be named: each one breaks *every* confined run, not
/// just its own feature.
///
/// Nine of them carry `[macos] raw` blocks naming a symbol isol8's macOS
/// backend never defines — `home-literal` and `home-subpath` as macros, and
/// `HOME_DIR` as a bare variable inside a `string-append`. The rendered policy
/// opens `(version 1)\n(deny default)\n` and carries no `(define ...)` prelude
/// at all, so all three are unbound. There is no partial failure:
/// `sandbox-exec` rejects the whole policy with `unbound variable` and nothing
/// starts, however unrelated the layer is to what the run was doing. Granting
/// `toolchains/apple-toolchain-core` directly is the standing workaround for
/// the `integrations/xcode` case.
///
/// `integrations/ssh` is excluded twice over. Beyond the macro bug it denies
/// `~/.ssh` as a *subpath*, and Seatbelt is last-match-wins with denies
/// rendered after every allow — so it swallows the `~/.ssh/known_hosts` read
/// that `integrations/git` grants. Adding it takes a capability away.
///
/// `integrations/shell-init` is milder: it opens every `/Applications/*.app`
/// bundle's completion scripts to buy a `PATH` that [`ENV_PASS`] already
/// carries through.
///
/// None of this stops a user naming one explicitly through
/// `Isolation::Sandboxed`; it stops *this* module choosing one for them.
pub const BROKEN_LAYERS: &[&str] = &[
    // `home-literal` / `home-subpath`
    "integrations/xcode",
    "integrations/kubectl",
    "integrations/chromium-headless",
    "integrations/chromium-full",
    // both families
    "integrations/ssh",
    "integrations/1password",
    "integrations/container-runtime-default-deny",
    // `HOME_DIR`
    "integrations/docker",
    "integrations/ssh-agent-default-deny",
    // Not broken, just a bad default — see above.
    "integrations/shell-init",
];

/// Developer read-write roots under the real home that isol8 ships no layer
/// for, so nothing else can reach them.
///
/// The `toolchains/*` layers cover cargo, npm, pip, gradle and their siblings;
/// there is no dotnet or nuget layer anywhere in isol8, and the SDK writes
/// before it will do anything else — a first-run sentinel under `~/.dotnet`,
/// then the package store under `~/.nuget`.
///
/// [`DOTNET_LAYER_BODY`] is the other half, not a replacement: a layer's `~`
/// expands against the *effective* home, while these are joined against the
/// real one. Under [`HomeMode::Ephemeral`] or [`HomeMode::Managed`] that is the
/// difference between a run that re-downloads the whole package store and one
/// that reads the machine's.
///
/// Joined absolutely against the real home rather than passed as a `~` token,
/// so these stay correct under [`HomeMode::Managed`] too. Deliberately *not*
/// existence-filtered, unlike [`login_runtime_grants`]: `~/.dotnet` does not
/// exist until the first `dotnet` run creates it, and the grant is what makes
/// that creation legal — filtering it would break exactly the machine it is
/// for. A root nothing ever touches costs two lines of rendered policy, since
/// the backend resolves the longest existing prefix of a grant and leaves the
/// rest alone.
pub const DEV_RW_HOME_ROOTS: &[&str] = &[
    ".dotnet",
    ".nuget",
    ".templateengine",
    ".aspnet",
    ".microsoft",
    ".local/share/NuGet",
];

/// The Apple SDK trees a Swift or Xcode build reads, granted on macOS only.
///
/// `toolchains/apple-toolchain-core` grants `/Library/Developer/CommandLineTools/usr/bin`
/// as a *literal* — the directory itself, plus one entry per binary it names —
/// and `swift`, `swiftc` and the twenty `swift-*` tools beside them are not in
/// that list. So `swift --version` denies inside the sandbox on a machine whose
/// toolchain is complete, which reads as a broken Swift install rather than as
/// a policy. `integrations/xcode` is the layer that would cover the rest, and
/// it is in [`BROKEN_LAYERS`]: its `[macos] raw` block calls `home-literal`,
/// which fails the *whole* policy. This const is that layer's path half,
/// replicated the way `refs/dev-home/dev-home.toml` already replicates it.
///
/// Both Xcode bundles are named because either can be the active developer dir,
/// and `DEVELOPER_DIR` (in [`ENV_PASS`]) is what says which — an absent bundle
/// renders as an inert grant. The plist is the license-acceptance record every
/// `xcodebuild` reads before it does anything.
///
/// Not the whole of Xcode: a simulator or a device run also needs
/// `com.apple.CoreSimulator*` mach lookups, which no [`isol8::Spec`] field can
/// express — a grant list cannot carry SBPL. `swift build` needs one thing more
/// still, and it is not a grant either: SwiftPM shells out to `sandbox-exec`
/// itself, a sandbox cannot nest, and only `--disable-sandbox` gets past it.
/// See `G91`.
pub const APPLE_SDK_RO_ROOTS: &[&str] = &[
    "/Library/Developer/CommandLineTools",
    "/Library/Developer/Toolchains",
    "/Applications/Xcode.app",
    "/Applications/Xcode-beta.app",
    "/Library/Preferences/com.apple.dt.Xcode.plist",
];

/// The Apple developer caches a Swift or Xcode build writes, under the real
/// home, granted on macOS only.
///
/// The read-only half above lets the toolchain run; these let it finish. SwiftPM
/// resolves and caches packages under both `org.swift.swiftpm` roots, and
/// `xcodebuild` writes derived data, simulator state and its own build cache
/// under `~/Library/Developer` and `~/Library/Caches`. `swiftc`/`clang` write
/// their module cache under `~/.cache/clang`, and SwiftPM's `configuration`,
/// `security` and `cache` entries are symlinks from `~/.swiftpm` into the
/// `org.swift.swiftpm` roots — so the on-device assist backend's Swift bridge
/// (`foundation-models`, built behind the `assist-apple` feature) dies on both
/// without a grant, surfacing as `framework 'FoundationModels' not found`. Same
/// rules as [`DEV_RW_HOME_ROOTS`]: joined absolutely against the real home so
/// they survive a replaced one, and not existence-filtered, since the grant is
/// what makes a cache's first-run creation legal.
pub const APPLE_RW_HOME_ROOTS: &[&str] = &[
    "Library/Developer/Xcode",
    "Library/Developer/CoreSimulator",
    "Library/Developer/XCTestDevices",
    "Library/Developer/CoreDevice",
    "Library/Caches/com.apple.dt.Xcode",
    "Library/Caches/org.swift.swiftpm",
    "Library/org.swift.swiftpm",
    ".swiftpm",
    ".cache/clang",
];

/// Whether this host is the one [`APPLE_SDK_RO_ROOTS`] and
/// [`APPLE_RW_HOME_ROOTS`] describe. Elsewhere those paths name nothing, and a
/// `~/Library` grant on Linux is a statement about the wrong tree.
fn on_macos() -> bool {
    isol8::Platform::current() == isol8::Platform::Macos
}

/// Whether this host is the one [`windows_ro_home_dirs`] describes.
fn on_windows() -> bool {
    isol8::Platform::current() == isol8::Platform::Windows
}

/// The per-user PowerShell configuration directories, under `Documents`.
///
/// PowerShell 7 reads `powershell.config.json` from `Documents\PowerShell`
/// before it parses its own command line, and treats a file it can see but
/// cannot open as fatal — it throws `PSInvalidOperationException: PowerShell has
/// stopped working because of a security issue` and dies with no line run. A
/// deny-by-default policy that names nothing under `Documents` produces exactly
/// that, so every agent whose shell is `pwsh` loses its shell, which is how the
/// bug is reported: "my PowerShell is broken", with no denial to point at.
///
/// Read-only, and only these two names: the grant is for a config file, not for
/// the user's documents. Windows PowerShell 5.1 uses the `WindowsPowerShell`
/// sibling and survives without it — which is why the ConPTY probes, both of
/// which run `powershell`, never saw this.
const WINDOWS_PWSH_CONFIG_DIRS: &[&str] = &["PowerShell", "WindowsPowerShell"];

/// Where [`WINDOWS_PWSH_CONFIG_DIRS`] live, for every `Documents` this host has.
///
/// Two bases, because `Documents` is a known folder rather than a path: OneDrive's
/// folder redirection moves it under `%OneDrive%` and leaves the one beside the
/// profile in place, and PowerShell resolves the *redirected* one through the
/// registry, which isol8 does not filter. Naming both costs one inert grant on a
/// machine that redirects nothing and is the difference between working and not
/// on one that does.
///
/// Not existence-filtered, for the same reason [`DEV_RW_HOME_ROOTS`] is not: a
/// grant on a directory that does not exist yet is what makes its first-run
/// creation legal.
fn windows_ro_home_dirs(real_home: &Path) -> Vec<PathBuf> {
    let mut bases = vec![real_home.to_path_buf()];
    if let Some(onedrive) = std::env::var_os("OneDrive") {
        let onedrive = PathBuf::from(onedrive);
        if onedrive != *real_home {
            bases.push(onedrive);
        }
    }
    bases
        .iter()
        .flat_map(|base| {
            WINDOWS_PWSH_CONFIG_DIRS
                .iter()
                .map(move |dir| base.join("Documents").join(dir))
        })
        .collect()
}

/// What a harness needs to draw a screen, and what the toolchains it shells
/// out to need to work at all.
///
/// isol8's environment is deny-by-default — `HOME`, `PATH`, `SHELL`,
/// `TMPDIR`, `USER`, `LOGNAME`, `PWD` and nothing else — so every name here is
/// one a real build fails without. Each is read from the host only if set, so
/// an entry costs nothing on a machine that does not use it, and a
/// provisioner's own `set_env` still outranks all of them: `CLAUDE_CONFIG_DIR`
/// cannot be shadowed from here.
///
/// The relocation roots at the end matter only because `$HOME` is the real
/// home: a layer grants a toolchain's default location, so a host that moved
/// one has to say where — and grant it, via [`IsolateOptions::extra_rw`].
/// Naming the variable alone opens no path.
///
/// Absent on purpose. `GIT_DIR` and its siblings are inherited from whatever
/// launched this process and would retarget the agent's own `git` at the wrong
/// repository. `XDG_*` points every lookup away from the `~/.config` paths the
/// layers actually grant. `SSH_AUTH_SOCK` is inert while `integrations/ssh`
/// stays in [`BROKEN_LAYERS`], since the agent socket is denied without it.
pub const ENV_PASS: &[&str] = &[
    // A harness asks the environment what it may draw, and isol8's allowlist
    // holds none of these.
    "TERM",
    "COLORTERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    // Without a UTF-8 locale, python, perl and dotnet fall back to ASCII and
    // mangle every non-ASCII path and diagnostic they touch.
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    // The network is open, but a machine that only egresses through a proxy
    // needs these to find it — and the toolchains disagree about case, so both
    // spellings go through.
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    // …and these to trust it. Belt and braces for the non-rustls tools; a
    // rustls one needs `integrations/keychain` instead, which is why that
    // layer is not optional.
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "CARGO_HTTP_CAINFO",
    // Relocation roots — see the note above.
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GOPATH",
    "GOROOT",
    "GOMODCACHE",
    "GOCACHE",
    "JAVA_HOME",
    "SDKMAN_DIR",
    "NVM_DIR",
    "PNPM_HOME",
    "VOLTA_HOME",
    "PYENV_ROOT",
    "MISE_DATA_DIR",
    "NPM_CONFIG_PREFIX",
    "BUN_INSTALL",
    "DOTNET_ROOT",
    "NUGET_PACKAGES",
    "DEVELOPER_DIR",
    "PUB_CACHE",
];

/// Resolve `launch` into the policy it runs under, or `None` when the run is
/// not isolated.
///
/// `dir` is the ephemeral config dir the provisioner populated; it is granted
/// read-write, along with the run's working directory. Everything the run
/// reaches *through* that dir — a skill's catalog folder, a profile's overlay —
/// is granted read-only, because the
/// provisioner links rather than copies wherever it can.
pub fn plan(
    launch: &Launch,
    run: &RunSpec,
    dir: &Path,
    options: &IsolateOptions,
) -> Result<Option<Confined>> {
    let Isolation::Sandboxed(layer) = &run.isolation else {
        return Ok(None);
    };

    let cmd = command_of(launch);
    let ctx = context(run, options);

    let mut base = isol8::Spec::new(cmd.clone());
    base.add_dirs_rw = read_write_grants(dir, run, options);
    base.add_dirs_ro = read_only_grants(run, options);
    base.set_env = launch.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    base.env_pass = ENV_PASS.iter().map(|name| (*name).to_string()).collect();
    match &options.home {
        // isol8 spells "the real home" as neither field set: `ephemeral_home
        // = false` is the absence of a statement, not a statement.
        HomeMode::Inherit => {}
        HomeMode::Ephemeral => base.ephemeral_home = true,
        HomeMode::Managed(id) => base.home = Some(format!("@managed/{id}")),
    }

    // A profile root that is not there is an error rather than an empty layer
    // set, and this root is the caller's own, so it is created here — the same
    // first step isol8's host-integration checklist prescribes.
    let profiles = options.state_dir.join("profiles");
    std::fs::create_dir_all(&profiles)
        .with_context(|| format!("creating the isolation profile root {}", profiles.display()))?;

    // The config fills a `Spec` field only when that field is empty, and it
    // does so whole: assigning `base.profiles` here would drop `base` and this
    // OS's system-runtime layer with it, so a named layer *joins* the defaults.
    let mut cfg = isol8::Config::builtin_defaults();
    cfg.auto_profiles = true;
    // Ours first, the caller's root second: a later path replaces a same-named
    // layer from an earlier one, so a `toolchains/dotnet` written under the
    // caller's root overrides the generated one. See [`write_dotnet_layer`].
    cfg.profile_paths = vec![
        path_string(&write_dotnet_layer(options)?),
        path_string(&profiles),
    ];
    // The toolchain layers, which nothing else can select: isol8 auto-matches
    // on `cmd[0]` alone, and a build tool is this command's child.
    cfg.default_profiles
        .extend(DEV_LAYERS.iter().map(|name| (*name).to_string()));
    if !layer.is_empty() {
        cfg.default_profiles.push(layer.clone());
    }

    let spec = isol8::resolve::spec_from_config(&cfg, base, cmd, &ctx)
        .context("resolving the isolation policy for this run")?;

    info!(
        layers = ?cfg.default_profiles,
        rw = ?spec.add_dirs_rw,
        ro = ?spec.add_dirs_ro,
        home = ?options.home,
        "resolved sandboxed run policy"
    );

    Ok(Some(Confined { spec, ctx }))
}

/// The effective policy for `confined`, resolved and rendered but not applied
/// — the layer stack, the grants, the environment, the home plan and the
/// OS-native policy text. Writes nothing and spawns nothing, so it is what
/// `--print-config` and an embedder's audit log report.
pub fn describe(confined: &Confined) -> Result<isol8::DryRun> {
    isol8::sandbox::dry_run_in(&confined.spec, &confined.ctx)
        .context("rendering the isolation policy for this run")
}

/// The [`Launch`] that runs `confined`'s command under the OS policy, for any
/// caller that spawns argv itself — a host that owns a pseudo-terminal, or a
/// structured bridge that owns pipes.
///
/// isol8 spawns with inherited stdio and keeps every `SandboxChild`
/// constructor private, so a caller cannot hand it descriptors of its own. On
/// macOS that costs nothing: the policy renders to text and `sandbox-exec`
/// `execve`s in place — the same thing isol8 does internally — so the harness
/// is still one process, whatever is on its descriptors. Linux has no
/// equivalent: Landlock is applied inside the target process between `fork`
/// and `exec`, and no rendered form of it exists to hand anyone. Windows is
/// confined the same way it is spawned — suspended `CreateProcessW` plus the
/// hook DLL — so a caller-owned ConPTY is equally out of reach: isol8
/// (v0.4.0) offers inherited stdio only, no ConPTY seam. Both error below
/// until isol8 grows the seam (`refs/isol8-pty-seam-update.md` for unix;
/// ConPTY is separate work).
pub fn confined_launch(confined: &Confined) -> Result<Launch> {
    // Rendering the policy ourselves bypasses the guard `isol8::Sandbox::spawn`
    // applies, so it is applied here: a sandbox cannot nest, and the honest
    // answer is that this process cannot confine anything — not a
    // `sandbox_apply` failure the caller has to interpret.
    isol8::sandbox::ensure_not_nested()
        .context("this process is already confined, and a sandbox cannot nest")?;

    let mut effective = isol8::resolve::effective_policy_in(&confined.spec, &confined.ctx)
        .context("resolving the isolation policy for this run")?;
    isol8::home::materialize(&effective.home).context("materializing the confined run's home")?;
    isol8::resolve::confine_executable(&mut effective.profile, &mut effective.cmd)
        .context("granting the harness binary to the policy that confines it")?;

    let env: Vec<(String, String)> = effective.env.into_iter().collect();

    #[cfg(target_os = "macos")]
    {
        let policy = isol8::backends::select().render_policy(&effective.profile);
        debug!(program = %effective.cmd.first().cloned().unwrap_or_default(), via_sandbox_exec = true, "confined launch");
        let mut args = vec!["-p".to_string(), policy];
        args.extend(effective.cmd);
        return Ok(Launch {
            program: "/usr/bin/sandbox-exec".to_string(),
            args,
            env,
            env_remove: Vec::new(),
            env_clear: true,
        });
    }

    #[cfg(target_os = "windows")]
    {
        // isol8 confines only a process it creates itself, and offers no ConPTY
        // seam — so nothing can be rendered to an argv the caller execs. What it
        // does do is spawn without any console creation flag, which means the
        // confined child attaches to its caller's console. So put *this binary*
        // in the pane, invoked again under `CONFINE_ARG`: isol8 creates the
        // harness from in there, and it lands on the very ConPTY the caller
        // opened. Nothing extra ships, and the process applying the policy is by
        // construction the build that resolved it.
        debug!(program = %effective.cmd.first().cloned().unwrap_or_default(), via_confine_shim = true, "confined launch");
        let payload = ConfinePayload {
            profile: effective.profile,
            env,
            cmd: effective.cmd,
            cwd: confined.ctx.cwd.clone(),
        };
        let exe = std::env::current_exe().context("locating the running executable")?;
        return Ok(Launch {
            program: path_string(&exe),
            args: vec![
                CONFINE_ARG.to_string(),
                path_string(&payload.write(&confined.ctx.config_dir)?),
            ],
            env: Vec::new(),
            env_remove: Vec::new(),
            env_clear: false,
        });
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (env, effective.profile, effective.cmd);
    }

    #[allow(unreachable_code)]
    Err(anyhow!(
        "isolating a run whose descriptors the caller owns needs isol8's stdio seam, \
         which this platform has no substitute for. Run without --isolate, or see \
         refs/isol8-pty-seam-update.md"
    ))
}

/// The resolved policy a confine run applies, as it crosses from this process to
/// the shim.
///
/// Resolution stays here — the shim never loads a layer, reads a config file or
/// consults a [`Context`](isol8::Context). It receives the same three values
/// [`isol8::backends::Backend::spawn`] takes and hands them straight over, so
/// the policy a pane runs under is the one this module computed and nothing
/// else.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ConfinePayload {
    /// The merged, deny-first profile, with every path grant already resolved.
    pub profile: isol8::Profile,
    /// The sanitized environment for the confined process.
    pub env: Vec<(String, String)>,
    /// The command to run, program first, after profile rewrite rules.
    pub cmd: Vec<String>,
    /// The directory the policy was resolved against, which the shim must make
    /// its own before spawning.
    ///
    /// Not a convenience. isol8 grants the resolving process's working directory
    /// read-write (`overrides_layer`, from `Context::cwd`), and the confined
    /// child inherits its parent's — so if the shim runs anywhere else, the
    /// harness lands in a directory its own policy does not grant. A pane
    /// spawned with no explicit directory starts in the user's home, which is
    /// exactly that mismatch: the harness dies on startup with no denial to read,
    /// because isol8's Windows hook writes no denial log.
    pub cwd: PathBuf,
}

impl ConfinePayload {
    /// Write the payload under `state_dir` and return the file the shim reads.
    ///
    /// One file per launch, named from the pid and a counter so two panes
    /// starting at once cannot collide. The shim deletes it on read: it carries
    /// a run's grants and environment, and nothing needs it twice.
    pub fn write(&self, state_dir: &Path) -> Result<PathBuf> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);

        let dir = state_dir.join("confine");
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating the confine payload dir {}", dir.display()))?;
        let path = dir.join(format!(
            "{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let json = serde_json::to_vec(self).context("serializing the confine payload")?;
        std::fs::write(&path, json)
            .with_context(|| format!("writing the confine payload {}", path.display()))?;
        Ok(path)
    }

    /// Read a payload written by [`ConfinePayload::write`] and delete the file.
    pub fn take(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading the confine payload {}", path.display()))?;
        let payload =
            serde_json::from_slice(&bytes).context("parsing the confine payload as JSON")?;
        let _ = std::fs::remove_file(path);
        Ok(payload)
    }
}

/// The argv word that turns a run of this binary into a confine run.
///
/// Windows confinement needs a process inside the pane, and that process is this
/// same executable invoked again rather than a second one shipped beside it —
/// the shape Chrome's `--type=renderer` and busybox's argv dispatch use. An
/// embedder calls [`confine_entrypoint`] before it starts anything; there is
/// nothing to package, nothing to find on disk, and nothing to get out of step
/// with the build that resolved the policy.
pub const CONFINE_ARG: &str = "--am-confine";

/// Handle a confine run and return its exit code, or [`None`] if this is an
/// ordinary run of the binary.
///
/// Call it first, before a window, a host, a log or a config read: this process
/// is either the interface or a pane's confined harness, and which one it is
/// decides everything after. It reads one argument, spawns, waits, and never
/// returns to its caller's own work.
///
/// ```no_run
/// if let Some(code) = agent_manager::isolate::confine_entrypoint() {
///     std::process::exit(code);
/// }
/// ```
pub fn confine_entrypoint() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != CONFINE_ARG {
        return None;
    }
    let Some(payload) = args.next() else {
        eprintln!("{CONFINE_ARG} wants the path of a confine payload");
        return Some(2);
    };
    match confine_run(Path::new(&payload)) {
        Ok(code) => Some(code),
        Err(error) => {
            eprintln!("ubiq: {error:#}");
            Some(1)
        }
    }
}

/// Apply `payload` and run the harness under it, returning its exit code.
#[cfg(target_os = "windows")]
fn confine_run(payload: &Path) -> Result<i32> {
    let payload = ConfinePayload::take(payload)?;

    // Both before the spawn: from here on, whatever this process creates
    // inherits the directory the policy grants, and belongs to the job.
    std::env::set_current_dir(&payload.cwd).with_context(|| {
        format!(
            "entering {} — the directory this run's policy was resolved against",
            payload.cwd.display()
        )
    })?;
    kill_on_close_job().context("putting the confined run in a kill-on-close job")?;

    let env: std::collections::HashMap<String, String> = payload.env.into_iter().collect();
    let mut child = isol8::backends::select()
        .spawn(&payload.profile, &env, &payload.cmd)
        .map_err(|e| anyhow!("spawning the confined harness: {e}"))?;
    child
        .wait()
        .map_err(|e| anyhow!("waiting on the confined harness: {e}"))
}

#[cfg(not(target_os = "windows"))]
fn confine_run(_payload: &Path) -> Result<i32> {
    Err(anyhow!(
        "a confine run exists for Windows, where isol8 has no ConPTY seam. Everywhere \
         else `confined_launch` renders a policy the caller execs itself."
    ))
}

/// Make every process this one starts die with it.
///
/// Windows has no process tree: a harness, a confine shim or anything either of
/// them spawns outlives the process that started it, so an interface that is
/// closed — or that crashes — leaves its agents running with no window left to
/// reach them through. This is the one call that fixes that for everything at
/// once, and an embedder makes it during boot, after
/// [`confine_entrypoint`] (a confine run wants its own job, not this one) and
/// before any pane or conversation exists.
///
/// A no-op everywhere else: Unix has process groups and a parent that reaps.
pub fn kill_descendants_on_exit() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        kill_on_close_job()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

/// Put this process in an unnamed job object that kills on close, keeping the
/// handle for the life of the process.
///
/// The host kills a pane by terminating the process it spawned — this one — and
/// the harness underneath would outlive it. Every descendant joins the job by
/// inheritance, the handle closes when this process ends, and closing the last
/// handle to such a job terminates everything still in it. Nested jobs are fine:
/// a host that is itself in a job does not stop this one being created.
#[cfg(target_os = "windows")]
fn kill_on_close_job() -> Result<()> {
    let job = win32job::Job::create().map_err(|e| anyhow!("creating a job object: {e}"))?;
    let mut info = job
        .query_extended_limit_info()
        .map_err(|e| anyhow!("reading the job's limits: {e}"))?;
    info.limit_kill_on_job_close();
    job.set_extended_limit_info(&info)
        .map_err(|e| anyhow!("setting kill-on-close on the job: {e}"))?;
    job.assign_current_process()
        .map_err(|e| anyhow!("joining the job object: {e}"))?;

    // Deliberately leaked: the job must outlive this function, and the kernel
    // closes the handle when the process ends — which is exactly the moment the
    // job should take the harness with it. Dropping it here would close the last
    // handle immediately and kill the run before it started.
    std::mem::forget(job);
    Ok(())
}

/// The command `launch` wants exec'd, as isol8 wants it: argv, program first.
fn command_of(launch: &Launch) -> Vec<String> {
    let mut cmd = Vec::with_capacity(launch.args.len() + 1);
    cmd.push(launch.program.clone());
    cmd.extend(launch.args.iter().cloned());
    cmd
}

/// The ambient state the policy resolves against, built from host state alone.
///
/// `cwd` is the run's working directory rather than the process's, so the
/// automatic grant follows the run and an embedder's own directory never
/// leaks into it.
fn context(run: &RunSpec, options: &IsolateOptions) -> isol8::Context {
    isol8::Context {
        real_home: isol8::home::real_home(),
        cwd: run.cwd.clone(),
        platform: isol8::Platform::current(),
        config_dir: options.state_dir.clone(),
        managed_root: options.state_dir.join("homes"),
    }
}

/// Every directory the run may write: its own ephemeral config dir, its
/// working directory, the developer roots isol8 ships no layer for, and
/// whatever the caller declared.
///
/// The run's own two come first, so a duplicate in [`DEV_RW_HOME_ROOTS`],
/// [`APPLE_RW_HOME_ROOTS`] or [`IsolateOptions::extra_rw`] collapses into them
/// rather than the reverse.
fn read_write_grants(dir: &Path, run: &RunSpec, options: &IsolateOptions) -> Vec<String> {
    let mut grants = vec![path_string(dir), path_string(&run.cwd)];

    // The same value `Context.real_home` carries, so a grant and a `~`
    // expansion in a layer cannot disagree about where the home is.
    let real_home = isol8::home::real_home();
    let apple = if on_macos() { APPLE_RW_HOME_ROOTS } else { &[] };
    let extra = DEV_RW_HOME_ROOTS
        .iter()
        .chain(apple)
        .map(|rel| real_home.join(rel))
        .chain(options.extra_rw.iter().cloned());

    for path in extra {
        let grant = path_string(&path);
        if !grants.contains(&grant) {
            grants.push(grant);
        }
    }

    // A run reaches the network for the same reason a login does, and on Windows
    // that is a path grant like any other.
    for device in WINDOWS_DEVICE_RW {
        let grant = (*device).to_string();
        if !grants.contains(&grant) {
            grants.push(grant);
        }
    }

    grants
}

/// Every directory the run reads through the config dir rather than inside it,
/// plus the Apple SDK trees on macOS.
///
/// A `Source::Files` store hands over bytes, which the provisioner writes into
/// the run dir as real files — already covered by that dir's own grant — so
/// only a `Source::Dir` contributes here.
fn read_only_grants(run: &RunSpec, options: &IsolateOptions) -> Vec<String> {
    let mut grants: Vec<String> = Vec::new();

    let mut push = |source: &Source| {
        if let Source::Dir(path) = source {
            let grant = path_string(path);
            if !grants.contains(&grant) {
                grants.push(grant);
            }
        }
    };

    for skill in &run.skills {
        push(&skill.source);
    }
    for base in &run.config_bases {
        push(base);
    }
    // The Apple SDK trees no layer this module may name reaches — see
    // [`APPLE_SDK_RO_ROOTS`].
    if on_macos() {
        for root in APPLE_SDK_RO_ROOTS {
            let grant = (*root).to_string();
            if !grants.contains(&grant) {
                grants.push(grant);
            }
        }
    }
    // What `pwsh` reads before it runs a line — see [`WINDOWS_PWSH_CONFIG_DIRS`].
    if on_windows() {
        for dir in windows_ro_home_dirs(&isol8::home::real_home()) {
            let grant = path_string(&dir);
            if !grants.contains(&grant) {
                grants.push(grant);
            }
        }
    }
    for extra in &options.extra_ro {
        let grant = path_string(extra);
        if !grants.contains(&grant) {
            grants.push(grant);
        }
    }

    grants
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::SkillRef;
    use tempfile::TempDir;

    fn sandboxed_run(cwd: &Path, layer: &str) -> RunSpec {
        let mut run = RunSpec::new("claude-code".to_string(), cwd.to_path_buf());
        run.isolation = Isolation::Sandboxed(layer.to_string());
        run
    }

    // 1. An unsandboxed run must get no policy at all, or a run that never
    // asked for isolation is silently confined anyway.
    #[test]
    fn plan_returns_none_when_isolation_is_none() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let run = RunSpec::new("claude-code".to_string(), cwd.path().to_path_buf());
        let launch = Launch::default();
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&launch, &run, cwd.path(), &options).expect("plan");
        assert!(confined.is_none());
    }

    // 2. A sandboxed run must grant its ephemeral config dir and its cwd
    // read-write, or the harness can neither write its own config nor its
    // working files and the run fails on first touch.
    #[test]
    fn sandboxed_run_grants_config_dir_and_cwd_read_write() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let launch = Launch::default();
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&launch, &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        let rw = confined.spec.add_dirs_rw;
        assert!(rw.contains(&cfg_dir.path().display().to_string()));
        assert!(rw.contains(&cwd.path().display().to_string()));
    }

    // 3. Every Source::Dir a run's skills and profile overlays
    // point at must become a read-only grant, or the provisioner's
    // symlink-else-copy walks out of the granted config dir on the very first
    // launch. A Source::Files (materialized as real bytes, already inside the
    // granted dir) must add nothing, and a directory reused across sources
    // must not be granted twice. Asserted directly on the pure grant-building
    // function rather than through `plan`, so this test needs no isol8 config
    // resolution and cannot flake on the developer's machine.
    #[test]
    fn read_only_grants_cover_every_dir_source_once_and_skip_files() {
        let skill_dir = PathBuf::from("/skills/writer");
        let profile_dir = PathBuf::from("/profiles/base");

        let mut run = RunSpec::new("claude-code".to_string(), PathBuf::from("/work"));
        run.skills = vec![
            SkillRef {
                id: "writer".to_string(),
                source: Source::Dir(skill_dir.clone()),
            },
            SkillRef {
                id: "inline".to_string(),
                source: Source::Files(vec![(PathBuf::from("SKILL.md"), b"hi".to_vec())]),
            },
        ];
        // The same directory reached two ways (e.g. a profile chain that
        // extends itself into a shared base) must be granted once.
        run.config_bases = vec![
            Source::Dir(profile_dir.clone()),
            Source::Dir(profile_dir.clone()),
        ];

        let options = IsolateOptions::new(PathBuf::from("/state"));
        let grants = read_only_grants(&run, &options);

        assert!(grants.contains(&skill_dir.display().to_string()));
        assert!(grants.contains(&profile_dir.display().to_string()));
        let apple = if on_macos() {
            APPLE_SDK_RO_ROOTS.len()
        } else {
            0
        };
        let windows = if on_windows() {
            windows_ro_home_dirs(&real_home()).len()
        } else {
            0
        };
        assert_eq!(
            grants.len(),
            2 + apple + windows,
            "a Source::Files entry and a duplicate Source::Dir must not add grants: {grants:?}"
        );
    }

    // 3b. `pwsh` reads `Documents\PowerShell\powershell.config.json` before it
    // parses its command line and dies on a file it can see but cannot open, so
    // a policy that names nothing there costs the agent its shell — silently,
    // since isol8's Windows hook writes no denial log. Read-only and narrow: the
    // two config directories, never `Documents` itself.
    #[test]
    #[cfg(target_os = "windows")]
    fn the_powershell_config_dirs_are_granted_read_only() {
        let run = RunSpec::new("claude-code".to_string(), PathBuf::from("/work"));
        let options = IsolateOptions::new(PathBuf::from("/state"));
        let grants = read_only_grants(&run, &options);

        let home = real_home();
        for dir in WINDOWS_PWSH_CONFIG_DIRS {
            let want = home.join("Documents").join(dir).display().to_string();
            assert!(
                grants.contains(&want),
                "pwsh cannot start without {want}: {grants:?}"
            );
        }
        let documents = home.join("Documents").display().to_string();
        assert!(
            !grants.contains(&documents),
            "the grant is for a config file, not for the user's documents: {grants:?}"
        );
    }

    // 8b. `swift` and `swiftc` sit in a directory `toolchains/apple-toolchain-core`
    // grants as a literal without naming them, and `integrations/xcode` — the layer
    // that would cover the SDK bundles — is in BROKEN_LAYERS. So the Apple trees are
    // this module's to grant, or a confined `swift --version` is denied on a machine
    // whose toolchain is complete.
    #[test]
    #[cfg(target_os = "macos")]
    fn apple_sdk_roots_are_granted_read_only_and_their_caches_read_write() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&Launch::default(), &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        let ro = &confined.spec.add_dirs_ro;
        for root in APPLE_SDK_RO_ROOTS {
            assert!(
                ro.contains(&(*root).to_string()),
                "{root} missing from {ro:?}"
            );
        }
        let rw = &confined.spec.add_dirs_rw;
        for rel in APPLE_RW_HOME_ROOTS {
            let want = real_home().join(rel).display().to_string();
            assert!(rw.contains(&want), "{want} missing from {rw:?}");
        }
    }

    // 8. extra_ro entries on IsolateOptions must reach the policy as read-only
    // grants, or a caller-declared toolchain/cache the harness needs (which
    // the run itself has no source for) becomes unreachable inside the
    // sandbox. Duplicates must still collapse to one grant.
    #[test]
    fn extra_ro_options_become_read_only_grants() {
        let toolchain = PathBuf::from("/opt/toolchain");
        let mut options = IsolateOptions::new(PathBuf::from("/state"));
        options.extra_ro = vec![toolchain.clone(), toolchain.clone()];
        let run = RunSpec::new("claude-code".to_string(), PathBuf::from("/work"));

        let grants = read_only_grants(&run, &options);

        let mut want: Vec<String> = if on_macos() {
            APPLE_SDK_RO_ROOTS
                .iter()
                .map(|r| (*r).to_string())
                .collect()
        } else {
            Vec::new()
        };
        if on_windows() {
            want.extend(
                windows_ro_home_dirs(&real_home())
                    .iter()
                    .map(|dir| dir.display().to_string()),
            );
        }
        want.push(toolchain.display().to_string());
        assert_eq!(grants, want);
    }

    // 4. launch.env must reach the policy as explicit `K=V` set_env entries —
    // isol8 sanitizes the environment down to an allowlist, so a provisioner
    // variable like CLAUDE_CONFIG_DIR that is not passed this way is silently
    // dropped and the harness reads its real config instead of the throwaway
    // one. TERM/COLORTERM must always be passed through, or a confined
    // harness cannot decide what it may draw.
    #[test]
    fn launch_env_becomes_set_env_and_term_is_passed_through() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let launch = Launch {
            program: "claude".to_string(),
            args: Vec::new(),
            env: vec![(
                "CLAUDE_CONFIG_DIR".to_string(),
                cfg_dir.path().display().to_string(),
            )],
            env_remove: Vec::new(),
            env_clear: false,
        };
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&launch, &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        assert!(
            confined
                .spec
                .set_env
                .contains(&format!("CLAUDE_CONFIG_DIR={}", cfg_dir.path().display()))
        );
        let passed = &confined.spec.env_pass;
        assert!(passed.contains(&"TERM".to_string()));
        assert!(passed.contains(&"COLORTERM".to_string()));
        // A toolchain the agent shells out to needs more than a terminal: with
        // no UTF-8 locale python and dotnet mangle every non-ASCII path, and a
        // machine that egresses through a proxy cannot fetch a crate without
        // being told where the proxy is.
        assert!(passed.contains(&"LANG".to_string()));
        assert!(passed.contains(&"HTTPS_PROXY".to_string()));
        // …and never the inherited git state, which would retarget the agent's
        // own `git` at whatever repository launched this process.
        assert!(
            !passed.iter().any(|name| name.starts_with("GIT_")),
            "inherited GIT_* must not reach a confined run: {passed:?}"
        );
    }

    // 5. HomeMode::Ephemeral must set ephemeral_home (and leave `home` unset),
    // or a run that asked for a scratch home silently gets no home plan at
    // all.
    #[test]
    fn home_mode_ephemeral_sets_ephemeral_home_flag() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let launch = Launch::default();
        let mut options = IsolateOptions::new(state.path().to_path_buf());
        options.home = HomeMode::Ephemeral;

        let confined = plan(&launch, &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        assert!(confined.spec.ephemeral_home);
        assert!(confined.spec.home.is_none());
    }

    // 5. HomeMode::Managed(id) must set `home` to "@managed/<id>" and must
    // NOT also set ephemeral_home, or a persistent home (a harness's caches
    // and logins meant to survive between runs) gets torn down as if it were
    // a throwaway.
    #[test]
    fn home_mode_managed_sets_home_without_ephemeral_flag() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let launch = Launch::default();
        let mut options = IsolateOptions::new(state.path().to_path_buf());
        options.home = HomeMode::Managed("marco".to_string());

        let confined = plan(&launch, &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        assert_eq!(confined.spec.home.as_deref(), Some("@managed/marco"));
        assert!(!confined.spec.ephemeral_home);
    }

    // 6. HomeMode::parse must accept exactly "ephemeral"/"managed" and must
    // name the offending value in its error, or a typo in `[isolate] home`
    // silently falls back to the wrong home mode instead of failing loudly at
    // config load.
    #[test]
    fn home_mode_parse_accepts_known_values() {
        assert_eq!(
            HomeMode::parse("inherit", "unused").expect("inherit"),
            HomeMode::Inherit
        );
        assert_eq!(
            HomeMode::parse("ephemeral", "unused").expect("ephemeral"),
            HomeMode::Ephemeral
        );
        assert_eq!(
            HomeMode::parse("managed", "marco").expect("managed"),
            HomeMode::Managed("marco".to_string())
        );
    }

    #[test]
    fn home_mode_parse_rejects_unknown_value() {
        let err = HomeMode::parse("sometimes", "unused").expect_err("bad value must error");
        assert!(err.to_string().contains("sometimes"));
    }

    // 7. A named Sandboxed layer must JOIN the builtin `base` + this OS's
    // system-runtime layer in the resolved spec's profiles, never replace
    // them: isol8's config fill only fills a field that is still empty, so
    // assigning `base.profiles = vec![layer]` here would silently drop the OS
    // layer for every confined run. This is the guard against that
    // regression.
    #[test]
    fn named_layer_joins_builtin_default_profiles() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "no-network");
        let launch = Launch::default();
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&launch, &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        let system_layer = if cfg!(target_os = "macos") {
            "macos/system-runtime"
        } else if cfg!(target_os = "linux") {
            "linux/system-runtime"
        } else if cfg!(target_os = "windows") {
            "windows/system-runtime"
        } else {
            "base"
        };

        assert!(confined.spec.profiles.contains(&"base".to_string()));
        assert!(confined.spec.profiles.contains(&system_layer.to_string()));
        assert!(confined.spec.profiles.contains(&"no-network".to_string()));
    }

    // A relocated toolchain root is granted, split by what a build needs
    // there: a cache is written, an SDK tree is only read. Without this a
    // policy names `~/.cargo` on a machine whose cargo lives elsewhere, and
    // `cargo` fails in a way that looks like a broken toolchain — the
    // variable reaches the run through ENV_PASS and opens no path by itself.
    //
    // The roots are absolute on every platform under test (`/opt` is not
    // absolute on Windows, so the suite would assert on paths the code
    // correctly refuses).
    #[cfg(windows)]
    const TOOLCHAIN_TEST_ROOT: &str = "C:/opt";
    #[cfg(not(windows))]
    const TOOLCHAIN_TEST_ROOT: &str = "/opt";
    #[test]
    fn toolchain_env_roots_become_grants() {
        let cargo = format!("{TOOLCHAIN_TEST_ROOT}/rust/sdk/cargo");
        let rustup = format!("{TOOLCHAIN_TEST_ROOT}/rust/sdk/rustup");
        let goroot = format!("{TOOLCHAIN_TEST_ROOT}/go/sdk/1.26.1");
        let modcache = format!("{TOOLCHAIN_TEST_ROOT}/go/pkg/mod");
        let mut options = IsolateOptions::new("/state");
        options.grant_toolchains(|name| match name {
            "CARGO_HOME" => Some(cargo.clone().into()),
            "RUSTUP_HOME" => Some(rustup.clone().into()),
            "GOROOT" => Some(goroot.clone().into()),
            "GOMODCACHE" => Some(modcache.clone().into()),
            _ => None,
        });

        assert!(options.extra_rw.contains(&PathBuf::from(&cargo)));
        assert!(
            options.extra_rw.contains(&PathBuf::from(&rustup)),
            "rustup writes its toolchains and downloads: {:?}",
            options.extra_rw
        );
        assert!(options.extra_rw.contains(&PathBuf::from(&modcache)));
        // An SDK tree a build reads and never writes.
        assert!(options.extra_ro.contains(&PathBuf::from(&goroot)));
        assert!(!options.extra_rw.contains(&PathBuf::from(&goroot)));
    }

    // A relative or empty value grants nothing: isol8 resolves a grant against
    // no working directory, so honouring one would grant some other tree
    // rather than fail.
    #[test]
    fn toolchain_env_roots_skip_relative_and_empty() {
        let mut options = IsolateOptions::new("/state");
        options.grant_toolchains(|name| match name {
            "CARGO_HOME" => Some("relative/cargo".into()),
            "GOROOT" => Some("".into()),
            _ => None,
        });

        assert!(options.extra_rw.is_empty(), "{:?}", options.extra_rw);
        assert!(options.extra_ro.is_empty(), "{:?}", options.extra_ro);
    }

    // Not existence-filtered, for the same reason DEV_RW_HOME_ROOTS is not: a
    // root the tool creates on first use needs the grant to be created at all,
    // and a grant on an absent path is inert.
    #[test]
    fn toolchain_env_roots_need_not_exist() {
        let nowhere = format!("{TOOLCHAIN_TEST_ROOT}/nowhere/nuget/packages");
        let mut options = IsolateOptions::new("/state");
        options.grant_toolchains(|name| (name == "NUGET_PACKAGES").then(|| nowhere.clone().into()));

        assert_eq!(options.extra_rw, vec![PathBuf::from(&nowhere)]);
    }

    // Every variable this grants a path for must also be forwarded into the
    // run: a granted path the tool is never told about is dead policy, and a
    // forwarded variable with no grant is the bug this pair exists to prevent.
    #[test]
    fn every_toolchain_root_var_is_also_passed_through() {
        for (name, _) in TOOLCHAIN_ROOT_VARS {
            assert!(
                ENV_PASS.contains(name),
                "{name} is granted but not in ENV_PASS"
            );
        }
    }

    // THE test for DEV_LAYERS. Every name must exist in the pinned isol8
    // revision, and `plan` cannot catch a typo: it only fills a `Spec` and
    // never loads the layer registry, so an unknown name is a hard error
    // raised at *spawn* time, on every confined run, after this module has
    // already reported success. Resolving the policy here is the only place
    // that fails on the same commit that breaks it.
    //
    // Assert on the RESOLVED layer list, never the named one — isol8's own
    // `--show-policies` prints only explicitly named layers, which is exactly
    // the trap that makes a missing `requires` look like a working policy.
    #[test]
    fn dev_layers_all_resolve_and_join_the_builtin_stack() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let mut run = sandboxed_run(cwd.path(), "");
        run.harness = "codex".to_string();
        let launch = Launch {
            program: "/bin/sh".to_string(),
            ..Launch::default()
        };
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&launch, &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");
        // Resolving renders without materializing, and under Inherit no
        // scratch home is created, so this writes nothing outside the temps.
        let resolved = describe(&confined).expect("every named layer must resolve");

        let layers: Vec<&str> = resolved
            .layer_names
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        for want in DEV_LAYERS {
            assert!(layers.contains(want), "{want} missing from {layers:?}");
        }
        assert!(
            layers.contains(&"base"),
            "the toolchain layers must join the builtins, not displace them: {layers:?}"
        );
        // A layer whose raw SBPL calls an undefined macro makes sandbox-exec
        // reject the whole policy, so one of these in the resolved set breaks
        // every confined run rather than just its own feature.
        for broken in BROKEN_LAYERS {
            assert!(
                !layers.contains(broken),
                "{broken} is in BROKEN_LAYERS and must not resolve: {layers:?}"
            );
        }
    }

    // The home a run runs from is granted read-write: the account's under `Home`, the harness's own
    // default under `Native`, and nothing more for a per-run dir the policy grants already.
    #[test]
    fn grant_config_home_grants_the_home_the_run_runs_from() {
        use crate::harness::Harness as _;
        let claude = crate::harness::Claude::new();
        let scratch = PathBuf::from("/tmp/scratch");
        let granted = |config: ConfigStrategy| {
            let mut options = IsolateOptions::new("/tmp/state");
            options.grant_config_home(&claude, &config);
            options.extra_rw
        };

        let home = PathBuf::from("/tmp/profile-home");
        assert_eq!(
            granted(ConfigStrategy::Home {
                home: home.clone(),
                scratch: scratch.clone(),
            }),
            vec![home]
        );
        let native = granted(ConfigStrategy::Native {
            scratch: scratch.clone(),
        });
        assert!(!native.is_empty(), "Claude Code names its own config home");
        assert_eq!(native, claude.default_homes());
        assert!(granted(ConfigStrategy::Fixed(scratch)).is_empty());
        assert!(granted(ConfigStrategy::Ephemeral).is_empty());
    }

    // The real home is the entire point of Inherit: a `~`-relative layer grant
    // expands against the *effective* home, so a replaced one aims
    // `toolchains/rust`'s `~/.cargo` at an empty scratch directory. isol8
    // spells "the real home" as neither field set, so assert both — a future
    // `ephemeral_home = true` default would otherwise quietly undo this.
    #[test]
    fn home_mode_inherit_sets_neither_home_field() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let options = IsolateOptions::new(state.path().to_path_buf());
        assert_eq!(options.home, HomeMode::Inherit, "Inherit is the default");

        let confined = plan(&Launch::default(), &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        assert!(!confined.spec.ephemeral_home);
        assert_eq!(confined.spec.home, None);
    }

    // The .NET SDK writes before it will do anything else, and isol8 ships no
    // layer for it. The grant must be absolute — so it survives a replaced
    // home — and must be there whether or not the directory exists yet, since
    // the grant is what makes the SDK's own first-run creation legal.
    #[test]
    fn dotnet_roots_are_granted_read_write_without_existing() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&Launch::default(), &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        let rw = &confined.spec.add_dirs_rw;
        for rel in DEV_RW_HOME_ROOTS {
            let want = real_home().join(rel).display().to_string();
            assert!(rw.contains(&want), "{want} missing from {rw:?}");
        }
    }

    // `extra_rw` is the caller's escape hatch for a toolchain the shipped
    // layers cannot place — a relocated CARGO_HOME, an SDK on another volume.
    // It must dedupe against the run's own directories the way `extra_ro`
    // does, and must not need the path to exist.
    #[test]
    fn extra_rw_options_become_read_write_grants() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let mut options = IsolateOptions::new(state.path().to_path_buf());
        options.extra_rw = vec![
            PathBuf::from("/opt/toolchain/cargo"),
            // The cwd again: a duplicate must collapse rather than grant twice.
            cwd.path().to_path_buf(),
        ];

        let confined = plan(&Launch::default(), &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        let rw = &confined.spec.add_dirs_rw;
        assert!(rw.contains(&"/opt/toolchain/cargo".to_string()));
        let cwd_grant = cwd.path().display().to_string();
        assert_eq!(
            rw.iter().filter(|g| **g == cwd_grant).count(),
            1,
            "the cwd must be granted once: {rw:?}"
        );
    }

    // An empty layer name (Isolation::Sandboxed(String::new()), the "just
    // isolate me, no named profile" case) must add nothing beyond the
    // builtin defaults, or every plain `--isolate` run silently grows a
    // profile named "".
    #[test]
    fn empty_layer_name_adds_no_extra_profile() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let cfg_dir = TempDir::new().expect("config dir");
        let run = sandboxed_run(cwd.path(), "");
        let launch = Launch::default();
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = plan(&launch, &run, cfg_dir.path(), &options)
            .expect("plan")
            .expect("sandboxed run must produce a policy");

        // The builtins are `base` + this OS's system-runtime, plus the
        // toolchain layers `plan` always names.
        assert_eq!(confined.spec.profiles.len(), 2 + DEV_LAYERS.len());
        assert!(!confined.spec.profiles.contains(&String::new()));
    }

    // Linux has no form to render — Landlock applies between `fork` and `exec`
    // — so `confined_launch` must fail there loudly, naming what it waits on,
    // rather than pretend to confine the run. macOS and Windows both serve it
    // and are covered elsewhere: macOS by `tests/confined_bridge.rs`, Windows
    // by `examples/conpty_confine_probe.rs`, neither of which can run here
    // (one would exec `sandbox-exec`, the other wants a real pseudoconsole).
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn confined_launch_errors_on_platforms_without_a_native_seam() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        // The test binary itself: guaranteed to resolve on every platform
        // (`true` has no Windows equivalent, and an unresolvable command
        // fails earlier, inside `confine_executable`).
        let exe = std::env::current_exe().expect("test binary path");
        let spec = isol8::Spec::new(vec![exe.display().to_string()]);
        let ctx = isol8::Context {
            real_home: cwd.path().to_path_buf(),
            cwd: cwd.path().to_path_buf(),
            platform: isol8::Platform::current(),
            config_dir: state.path().to_path_buf(),
            managed_root: state.path().join("homes"),
        };
        let confined = Confined { spec, ctx };

        let err = confined_launch(&confined).expect_err("Linux must not confine");
        assert!(
            err.to_string().contains("stdio seam"),
            "a Linux refusal must name the missing stdio seam: {err}"
        );
    }

    // The Windows branch renders a re-invocation of this very binary, not a
    // second executable beside it — nothing to package, and the process that
    // applies the policy is by construction the build that resolved it.
    #[cfg(target_os = "windows")]
    #[test]
    fn confined_launch_renders_a_re_invocation_of_this_binary() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let exe = std::env::current_exe().expect("test binary path");
        let spec = isol8::Spec::new(vec![exe.display().to_string()]);
        let ctx = isol8::Context {
            real_home: cwd.path().to_path_buf(),
            cwd: cwd.path().to_path_buf(),
            platform: isol8::Platform::current(),
            config_dir: state.path().to_path_buf(),
            managed_root: state.path().join("homes"),
        };
        let confined = Confined { spec, ctx };

        let launch = confined_launch(&confined).expect("Windows confines a pane");
        assert_eq!(
            PathBuf::from(&launch.program),
            exe,
            "the confine run is this binary again"
        );
        assert_eq!(launch.args.first().map(String::as_str), Some(CONFINE_ARG));

        // The payload is real, carries the directory the policy was resolved
        // against, and is the shim's to delete.
        let payload =
            ConfinePayload::take(Path::new(&launch.args[1])).expect("payload round-trips");
        assert_eq!(payload.cwd, cwd.path());
        assert!(!Path::new(&launch.args[1]).exists(), "taking it removes it");
    }
}
