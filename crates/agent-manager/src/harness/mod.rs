//! Knowledge about each supported harness.
//!
//! Where the old design had a `Harness` *struct* of static facts, the target
//! design needs a `Harness` *trait* with behavior: each harness differs in how
//! it is provisioned, launched, and (later) spoken to. See
//! `_docs/architecture.md` §"The `Harness` trait" and
//! `_docs/harness/<id>.md` for the curated per-harness notes each impl
//! transcribes.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;
use crate::source::Source;
use crate::spec::{HarnessId, McpAsSkill, RunSpec};

mod claude;
mod codex;
mod copilot;
mod grok;
mod opencode;
mod shared;
pub use claude::Claude;
pub use codex::Codex;
pub use copilot::Copilot;
pub use grok::Grok;
pub use opencode::Opencode;

/// Render the `SKILL.md` body for one [`McpAsSkill`] pointer.
///
/// Shared by every provisioner so the generated content is byte-identical
/// across harnesses (each provisioner just picks its own skills dir to write
/// it into via [`write_mcp_as_skill_pointers`]).
///
/// **Honest about scope** (see `_docs/mcp-as-skill.md`): this is the
/// schema + pointer stepping stone only. The MCP named by `entry.id` stays
/// injected as a normal, always-on tool set — generating this file does
/// **not** yet save any context. The body says so explicitly rather than
/// implying a context-saving mechanism that isn't built yet. The "expand on
/// demand" mechanism (deferred-load / proxy-tool) that would actually defer
/// the MCP's context cost is explicitly deferred to a later step.
pub(crate) fn mcp_as_skill_markdown(entry: &McpAsSkill) -> String {
    let description = entry
        .summary
        .clone()
        .unwrap_or_else(|| format!("Access the {} MCP server's tools.", entry.id));
    format!(
        "---\nname: {id}\ndescription: {description}\n---\n\n\
The `{id}` MCP server's tools are available for this run — use them when \
relevant to the task.\n\n\
Note: this MCP is currently loaded as normal, always-on tools (not deferred \
on demand); this skill is a documented pointer to it, not a context-saving \
mechanism yet.\n",
        id = entry.id,
        description = description,
    )
}

/// Write one `SKILL.md` pointer per `spec.mcp_as_skill` entry into
/// `<skills_dir>/<id>/SKILL.md`.
///
/// No-op when `spec.mcp_as_skill` is empty (the common case today) — runs
/// that don't use `--mcp-as-skill` / a catalog `expose = "skill"` entry
/// produce byte-identical config to before this existed, since `skills_dir`
/// itself is never touched.
pub(crate) fn write_mcp_as_skill_pointers(spec: &RunSpec, skills_dir: &Path) -> Result<()> {
    for entry in &spec.mcp_as_skill {
        let dir = skills_dir.join(&entry.id);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let skill_md_path = dir.join("SKILL.md");
        std::fs::write(&skill_md_path, mcp_as_skill_markdown(entry))
            .with_context(|| format!("writing {}", skill_md_path.display()))?;
    }
    Ok(())
}

/// How to launch the real harness binary after provisioning.
#[derive(Debug, Clone, Default)]
pub struct Launch {
    /// Program to exec, e.g. `"claude"`.
    pub program: String,
    /// Arguments (injected flags first, then the user's passthrough args).
    pub args: Vec<String>,
    /// Environment variables to SET for the child.
    pub env: Vec<(String, String)>,
    /// Environment variables to REMOVE from the inherited env (hygiene).
    pub env_remove: Vec<String>,
    /// Start from an empty environment rather than inheriting the parent's.
    ///
    /// A confined launch sets this: the sandbox computes the whole environment
    /// it wants, so inheriting the host's would defeat it. `env_remove` is
    /// meaningless when this is set, and `env` is then the complete environment.
    pub env_clear: bool,
}

/// One model a harness can run, as surfaced by `am <harness> --list-models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// Harness-native model id to pass to `--model` (e.g. `sonnet`,
    /// `gpt-5-codex`, `anthropic/claude-sonnet-4-5`).
    pub id: String,
    /// Optional one-line human note (tier, aliases, context window, …).
    pub description: Option<String>,
    /// True if this is the harness's default when no model is selected.
    pub default: bool,
}

impl ModelInfo {
    /// A model entry with just an id (no description, not the default).
    pub fn new(id: impl Into<String>) -> Self {
        ModelInfo {
            id: id.into(),
            description: None,
            default: false,
        }
    }

    /// Builder: attach a human description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Builder: mark this entry as the harness default.
    pub fn as_default(mut self) -> Self {
        self.default = true;
        self
    }
}

/// One reasoning-effort level a model accepts, in the harness's own vocabulary.
///
/// Never flattened across harnesses: Claude's `low|medium|high|xhigh|max` and Codex's
/// `none|minimal|low|medium|high|xhigh` are different vocabularies, and what the user picks
/// has to round-trip exactly through the one the harness reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingLevel {
    /// The harness-native value to pass through (e.g. `--effort <value>`).
    pub value: String,
    /// Human-readable label for a picker UI (e.g. `"Extra high"`).
    pub label: String,
    /// Optional one-line note about what this level trades off.
    pub description: Option<String>,
}

/// What one model accepts. An empty `levels` means this model has no reasoning knob — which
/// is the honest answer for most of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelThinking {
    /// The levels this model accepts, in the harness's own wire order.
    pub levels: Vec<ThinkingLevel>,
    /// The level the harness uses when none is specified, if it names one.
    pub default_level: Option<String>,
}

/// Title-case a reasoning-effort id for display, e.g. `"medium"` -> `"Medium"`. `"xhigh"` is
/// special-cased to `"Extra high"` (bare title-casing would render it as `"Xhigh"`).
pub(crate) fn effort_label(effort: &str) -> String {
    if effort == "xhigh" {
        return "Extra high".to_string();
    }
    let mut chars = effort.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// One permission/sandbox mode this harness's CLI accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeInfo {
    /// The harness-native value to pass through (e.g. `--permission-mode <id>`).
    pub id: String,
    /// Human-readable label for a picker UI.
    pub label: String,
    /// Optional one-line note about what this mode does.
    pub description: Option<String>,
}

/// A plan for an interactive credential login into a relocated home dir,
/// produced by [`Harness::login`]. The launch runs in passthrough (the user
/// completes the harness's native login); afterwards the caller verifies
/// `credential_files[0]` appeared under the home dir and records the account.
#[derive(Debug, Clone)]
pub struct LoginPlan {
    /// Interactive login launch. Its env points the harness's credential store
    /// at the capture home and forces file-based storage where supported.
    pub launch: Launch,
    /// Credential file paths RELATIVE TO the capture home dir. `[0]` is required
    /// (absent after login = capture failed); any others are optional metadata.
    pub credential_files: Vec<std::path::PathBuf>,
}

/// Which I/O modes a harness can support. Only passthrough is used in P1.
#[derive(Debug, Clone, Copy, Default)]
pub struct IoSupport {
    /// Raw tty passthrough (always true).
    pub passthrough: bool,
    /// A structured [`crate::io::IoBridge`] is available via
    /// [`Harness::structured_bridge`] (its wire protocol — NDJSON,
    /// JSON-RPC `app-server`, etc. — is a per-harness implementation
    /// detail). False until each harness's bridge lands (P2, C2/C3/C4).
    pub structured: bool,
    /// The structured bridge stays open for a second turn: the process
    /// survives its first answer and takes further prompts, cancellations and
    /// permission answers over its own stream (Claude Code, codex).
    ///
    /// False for a **one-shot** harness (Copilot, opencode), which takes its
    /// prompt in argv, answers once and exits. A one-shot harness is not
    /// "unsupported" — it converses one turn per process, and a caller
    /// continues it by launching again with [`crate::spec::RunSpec::resume`]
    /// set to the id [`crate::io::AgentEvent::SessionStarted`] reported.
    ///
    /// Meaningless when `structured` is false. Mirrors what
    /// [`crate::io::IoBridge::input`] answers once a bridge exists; this is
    /// the same fact available *before* one is spawned.
    pub multi_turn: bool,
    /// The structured bridge speaks ACP (`crate::io::AcpBridge`) rather than a
    /// harness-specific wire. Meaningless when `structured` is false. This is a
    /// *capability declaration*, not a launch instruction: a caller that wants to
    /// know "can I speak the standard protocol at this harness" asks here instead
    /// of inferring it from the harness id.
    pub acp: bool,
}

/// How a harness's native env lever relocates its config/credentials into a
/// dir `am` controls (so the real `HOME` — and the user's toolchain — is left
/// intact). See `_docs/profiles.md` §5 for the A/B/C taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relocate {
    /// Relocates the entire config tree, credentials included (e.g. Claude's
    /// `CLAUDE_CONFIG_DIR`, Codex's `CODEX_HOME`). Class A.
    All,
    /// Relocates only the config tier, not the credential store (e.g. opencode's
    /// `OPENCODE_CONFIG_DIR`). Class B — pair with a `Data` lever.
    Config,
    /// Relocates the data/credential tier (e.g. `XDG_DATA_HOME`).
    Data,
}

/// One captured-login file to copy from an account's persistent home into a
/// harness's relocated config dir. `src`/`dst` are the two ends of that copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedFile {
    /// Source path RELATIVE TO the account home (as written by
    /// [`Harness::login`], e.g. `.claude/.credentials.json`).
    pub src: PathBuf,
    /// Destination path RELATIVE TO the relocated dir (e.g. `.credentials.json`).
    pub dst: PathBuf,
    /// This file *is* the credential, so a harness that rewrites it mid-run
    /// (an OAuth refresh rotates the refresh token and revokes the old one)
    /// has produced the only live copy — [`harvest_login`] writes it back to
    /// where it was seeded from before the run dir is thrown away.
    ///
    /// False for the identity/onboarding companions a login also needs
    /// (Claude's `.claude.json`): the harness rewrites those too, with a run's
    /// worth of project history and onboarding state that must never land back
    /// on the user's real file.
    pub credential: bool,
}

impl SeedFile {
    /// A seed-file mapping from an account-home-relative `src` to a
    /// relocated-dir-relative `dst`. Not a credential — never written back.
    pub fn new(src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> Self {
        SeedFile {
            src: src.into(),
            dst: dst.into(),
            credential: false,
        }
    }

    /// Same mapping, for the file that *is* the login: [`harvest_login`] writes
    /// a refreshed one back to its origin.
    pub fn credential(src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> Self {
        SeedFile {
            credential: true,
            ..SeedFile::new(src, dst)
        }
    }
}

/// A declarative description of how a harness relocates its config/credentials
/// and which files constitute a captured login. This is what makes credential
/// seeding generic across harnesses (rather than bespoke per provisioner) and
/// what lazy default-profile capture and the isolation model read. See
/// `_docs/profiles.md` §5.1.
#[derive(Debug, Clone)]
pub struct ConfigAnchor {
    /// Env vars (with their relocation semantics) that point the harness's
    /// config/data at a dir `am` controls while leaving `HOME` real. Empty for
    /// Class-C harnesses that have no lever (see `requires_home_relocation`).
    pub levers: Vec<(String, Relocate)>,
    /// The files that make a session "logged in", seeded from an account home
    /// into the relocated dir by [`seed_login`].
    pub login_seed: Vec<SeedFile>,
    /// True only for Class-C harnesses (no config lever): the credential store
    /// is reachable only by relocating `HOME`, which strips the toolchain — so
    /// these should be paired with isol8. False for Class A/B.
    pub requires_home_relocation: bool,
}

/// Seed captured-login files from an account's login [`Source`] into a
/// harness's relocated config `dir`, per a [`ConfigAnchor::login_seed`].
///
/// Writes (never symlinks — a run is ephemeral and the harness rewrites some of
/// these in place), creating parent dirs, and **skips any source file that
/// doesn't exist** so a reference-only or partially-captured account still
/// launches. Works over a [`Source`] rather than a raw path, so a
/// database-backed account store seeds its credential *bytes* exactly as the
/// filesystem store seeds files from the account `home`. Deliberately leaves
/// `HOME` untouched (see `_docs/profiles.md` §3).
pub(crate) fn seed_login(dir: &Path, login: &Source, seed: &[SeedFile]) -> Result<()> {
    for file in seed {
        let Some(bytes) = login.read(&file.src)? else {
            continue;
        };
        let dst = dir.join(&file.dst);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&dst, &bytes)
            .with_context(|| format!("seeding login to {}", dst.display()))?;
    }
    Ok(())
}

/// The mirror of [`seed_login`]: write a login the run *refreshed* back to the
/// [`Source`] it was seeded from, before the run dir is discarded.
///
/// A harness that refreshes an OAuth token mid-run rewrites the seeded copy —
/// and the refresh **rotates** the refresh token, so the original the copy came
/// from is now revoked. Throwing the run dir away therefore logs the user out
/// everywhere; harvesting is what keeps the origin the live credential.
///
/// Only [`SeedFile::credential`] files are considered, and only when their
/// bytes actually changed. A [`Source::Dir`] origin is written in place (mode
/// `0600` on unix); anything else came from somewhere that is not a directory
/// (Claude Code's macOS Keychain) and is handed to [`Harness::adopt_login`].
///
/// Never fails the caller: this runs on teardown paths, and a write-back that
/// could not happen is a warning, not a reason to break a run that is over.
pub fn harvest_login(harness: &dyn Harness, dir: &Path, origin: &Source) -> Result<()> {
    for file in harness
        .config_anchor()
        .login_seed
        .iter()
        .filter(|f| f.credential)
    {
        let path = dir.join(&file.dst);
        let Ok(fresh) = std::fs::read(&path) else {
            continue;
        };
        if origin.read(&file.src)?.as_deref() == Some(&fresh[..]) {
            continue;
        }
        let outcome = match origin {
            Source::Dir(home) => write_credential(&home.join(&file.src), &fresh),
            Source::Files(_) => harness.adopt_login(&file.src, &fresh),
        };
        if let Err(error) = outcome {
            tracing::warn!(
                harness = %harness.id(),
                file = %file.src.display(),
                "a refreshed login could not be written back, so the original may now be \
                 revoked: {error:#}"
            );
        }
    }
    Ok(())
}

/// Write a credential blob to `path`, creating parents, `0600` on unix.
fn write_credential(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 600 {}", path.display()))?;
    }
    Ok(())
}

/// The default [`Harness::renew_credentials`] path: seed `creds` into a temp
/// config dir, launch the harness with its [`Harness::credential_renew_command`]
/// (which is expected to refresh and rewrite the stored token), then read the
/// seed files back out as the renewed blobs.
///
/// Errors (without launching) when the harness declares no
/// `credential_renew_command`. The temp dir is removed on the way out. Uses
/// the harness's [`ConfigAnchor::levers`] to relocate config into the temp dir
/// (never touching the real `HOME`), exactly as provisioning does; a Class-C
/// harness (no levers) relocates `HOME` to the temp dir instead.
pub fn default_renew_via_launch<H: Harness + ?Sized>(
    harness: &H,
    creds: &[crate::credentials::CredentialBlob],
) -> Result<Vec<crate::credentials::CredentialBlob>> {
    let Some(renew_args) = harness.credential_renew_command() else {
        anyhow::bail!(
            "credential renewal for harness '{}' is not implemented",
            harness.id()
        );
    };
    let anchor = harness.config_anchor();

    // A private temp dir (tempfile is dev-only, so build the path by hand).
    let dir = std::env::temp_dir().join(format!(
        "am-renew-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating renew temp dir {}", dir.display()))?;

    let result = (|| -> Result<Vec<crate::credentials::CredentialBlob>> {
        let login = crate::credentials::source_from_blobs(creds);
        seed_login(&dir, &login, &anchor.login_seed)?;

        let mut cmd = std::process::Command::new(harness.command());
        cmd.args(&renew_args);
        if anchor.levers.is_empty() {
            // Class C: no config lever — relocate HOME (toolchain caveat).
            cmd.env("HOME", &dir);
        } else {
            for (var, _relocate) in &anchor.levers {
                cmd.env(var, &dir);
            }
        }
        let status = cmd
            .status()
            .with_context(|| format!("running `{} {}`", harness.command(), renew_args.join(" ")))?;
        if !status.success() {
            anyhow::bail!(
                "harness '{}' renew command exited with {status}",
                harness.id()
            );
        }

        // Read the (possibly refreshed) seed files back out, keyed by the
        // source-relative path so the blobs round-trip back into the store.
        let mut blobs = Vec::new();
        for seed in &anchor.login_seed {
            let path = dir.join(&seed.dst);
            if let Ok(bytes) = std::fs::read(&path) {
                blobs.push(crate::credentials::CredentialBlob {
                    name: seed
                        .dst
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    rel_path: seed.src.clone(),
                    bytes,
                });
            }
        }
        Ok(blobs)
    })();

    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// A JSON file, relative to a harness's relocated config root, whose content
/// is merged into the run's config from a per-harness, user-editable
/// **template store** rather than being computed as a Rust literal. See
/// [`apply_templates`].
#[derive(Clone, Copy)]
pub struct TemplateFile {
    /// Path relative to the config root, e.g. `"settings.json"`.
    pub name: &'static str,
    /// Content written to the template store the first time it's created, so
    /// there's always something on disk to read and edit.
    pub default: fn() -> serde_json::Value,
}

/// The template store root: `~/.config/agent-manager/templates` — sibling to
/// `accounts/`. Overridable by `AM_TEMPLATES`.
pub fn default_templates_root() -> Option<PathBuf> {
    crate::settings::default_config_dir().map(|d| d.join("templates"))
}

/// Resolve the template store root from (highest first): an explicit path,
/// `AM_TEMPLATES`, then the default.
pub fn resolve_templates_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit
        .or_else(|| std::env::var("AM_TEMPLATES").ok().map(PathBuf::from))
        .or_else(default_templates_root)
}

/// A store of per-harness preference **templates** — the editable JSON
/// defaults ([`TemplateFile`]) merged into a run's config. Abstracted as a
/// trait (like [`crate::registry::Registry`] / [`crate::account::AccountStore`])
/// so an embedder can back templates with a database while the CLI uses
/// [`FsTemplateStore`].
pub trait TemplateStore {
    /// The stored template value for `(harness_id, name)`, seeding it from
    /// `default` the first time it's requested so there's always a real,
    /// editable entry rather than a value hidden in Rust. Returns the value to
    /// gap-fill into the run's config (see [`apply_templates`]).
    fn template(
        &self,
        harness_id: &str,
        name: &str,
        default: fn() -> serde_json::Value,
    ) -> Result<serde_json::Value>;
}

/// Filesystem-backed [`TemplateStore`]: `<root>/<harness_id>/<name>`, created
/// with `default()` content on first read so a user always has a file to edit.
#[derive(Debug, Clone)]
pub struct FsTemplateStore {
    root: PathBuf,
}

impl FsTemplateStore {
    /// Create a template store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsTemplateStore { root: root.into() }
    }

    /// Build from the resolved default templates root (`AM_TEMPLATES` / the
    /// default location); falls back to an unset sentinel path when no root is
    /// resolvable, mirroring how the CLI handles an unset catalog root.
    pub fn from_default() -> Self {
        FsTemplateStore::new(
            resolve_templates_root(None)
                .unwrap_or_else(|| PathBuf::from(".agent-manager-templates-unset")),
        )
    }
}

impl TemplateStore for FsTemplateStore {
    fn template(
        &self,
        harness_id: &str,
        name: &str,
        default: fn() -> serde_json::Value,
    ) -> Result<serde_json::Value> {
        let harness_root = self.root.join(harness_id);
        let tpl_path = harness_root.join(name);
        if !tpl_path.exists() {
            std::fs::create_dir_all(&harness_root)
                .with_context(|| format!("creating {}", harness_root.display()))?;
            std::fs::write(&tpl_path, serde_json::to_string_pretty(&default())?)
                .with_context(|| format!("writing {}", tpl_path.display()))?;
        }
        let raw = std::fs::read_to_string(&tpl_path)
            .with_context(|| format!("reading {}", tpl_path.display()))?;
        Ok(serde_json::from_str(&raw)?)
    }
}

/// Merge each of `harness_id`'s [`TemplateFile`]s into `dir` on every run,
/// reading each template's value from `store`.
///
/// The merge is shallow and gap-filling: any key the run itself already
/// generated (credentials, onboarding flags, policy, hooks, …) wins; the
/// template only supplies keys nothing else set. A malformed template or
/// destination file is left untouched rather than failing the run — templates
/// are a convenience layer, not load-bearing.
pub(crate) fn apply_templates(
    dir: &Path,
    harness_id: &str,
    templates: &[TemplateFile],
    store: &dyn TemplateStore,
) -> Result<()> {
    for tpl in templates {
        let Ok(serde_json::Value::Object(tpl_map)) =
            store.template(harness_id, tpl.name, tpl.default)
        else {
            continue;
        };

        let dest_path = dir.join(tpl.name);
        let mut dest: serde_json::Value = match std::fs::read_to_string(&dest_path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({})),
            Err(_) => serde_json::json!({}),
        };
        let serde_json::Value::Object(dest_map) = &mut dest else {
            continue;
        };
        for (k, v) in tpl_map {
            dest_map.entry(k).or_insert(v);
        }

        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&dest_path, serde_json::to_string_pretty(&dest)?)
            .with_context(|| format!("writing {}", dest_path.display()))?;
    }
    Ok(())
}

/// A wrappable agent harness: how to identify, provision, and launch it.
pub trait Harness {
    /// Canonical stable id (e.g. `claude-code`).
    fn id(&self) -> HarnessId;
    /// Human-readable name.
    fn display_name(&self) -> &str;
    /// Launch binary name (e.g. `claude`).
    fn command(&self) -> &str;
    /// Alternate ids/commands that also resolve to this harness.
    fn aliases(&self) -> &[&str];
    /// Populate `dir` (the ephemeral config dir) from `spec`; return how to launch.
    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch>;
    /// How this harness relocates its config/credentials and which files make up
    /// a captured login. Backs generic credential seeding ([`seed_login`]), lazy
    /// default-profile capture, and the isolation model — see
    /// `_docs/profiles.md` §5.
    ///
    /// The default is a conservative empty anchor (no levers, no seed, no HOME
    /// relocation); **every real harness overrides it**. It exists only so the
    /// crate keeps compiling while harness ports land incrementally.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: Vec::new(),
            login_seed: Vec::new(),
            requires_home_relocation: false,
        }
    }
    /// Files the harness wrote as its own record of the conversation, inside a
    /// relocated config dir. Empty = this harness's record is not portable yet.
    fn transcripts(&self, _config_dir: &Path) -> Vec<PathBuf> {
        Vec::new()
    }
    /// User-editable JSON template files merged into `dir` on every run —
    /// see [`apply_templates`]. Default: none. Overridden by harnesses with
    /// preference-style defaults that should live in an editable file under
    /// `~/.config/agent-manager/templates/<harness-id>/` instead of a Rust
    /// literal — e.g. Claude Code's theme/tui/Claude-in-Chrome defaults.
    fn templates(&self) -> Vec<TemplateFile> {
        Vec::new()
    }
    /// Which I/O modes this harness supports.
    fn io_support(&self) -> IoSupport;
    /// Discover the models available for this harness, for
    /// `am <harness> --list-models`.
    ///
    /// Implementations either shell out to the harness's own list command
    /// (e.g. `codex debug models`, `opencode models`) or return a curated
    /// static list when the harness exposes no discovery command. May consult
    /// the ambient login/network. Default: an error naming this harness, so a
    /// harness that hasn't wired discovery yet fails clearly rather than
    /// silently returning nothing.
    fn discover_models(&self) -> Result<Vec<ModelInfo>> {
        anyhow::bail!(
            "model discovery for harness '{}' is not implemented",
            self.id()
        )
    }
    /// The installed binary's own version string, as a cache key. `<command> --version`, first
    /// line, trimmed — the shape every harness in the table answers.
    /// Verified: claude → "2.1.261 (Claude Code)", codex → "codex-cli 0.142.5".
    fn version(&self) -> Result<String> {
        let output = std::process::Command::new(self.command())
            .arg("--version")
            .output()
            .with_context(|| format!("running `{} --version` (is it on PATH?)", self.command()))?;
        if !output.status.success() {
            anyhow::bail!(
                "`{} --version` failed ({}): {}",
                self.command(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().next().unwrap_or_default().trim().to_string())
    }
    /// Per-model reasoning catalogs, keyed by the same ids `discover_models` answers.
    ///
    /// Default empty — a harness with no reasoning concept. That is the truthful answer for
    /// Copilot CLI, and it is why it is not overridden: its `help config` block carries model
    /// ids and nothing about effort (verified: no reasoning-effort flag exists in the CLI).
    /// opencode *does* have one and overrides this: `opencode models --verbose` lists each
    /// model's `variants` map, and `run --variant <name>` is the flag that selects one — see
    /// `Opencode::discover_thinking`. So does Grok, whose four efforts are a fixed enum rather
    /// than a probe — see `Grok::discover_thinking`.
    fn discover_thinking(&self) -> Result<std::collections::BTreeMap<String, ModelThinking>> {
        Ok(std::collections::BTreeMap::new())
    }
    /// The permission/sandbox modes this harness's CLI accepts, as a fixed list — these are
    /// enum values baked into the binary, not something a harness advertises, so there is
    /// nothing to probe. Default empty: opencode, Copilot and Grok offer no choice — their
    /// bridges run `--dangerously-skip-permissions` / `--allow-all` and there is no second
    /// answer, so one doc line here is cheaper than three empty overrides there.
    fn modes(&self) -> Vec<ModeInfo> {
        Vec::new()
    }
    /// Which of this harness's own [`Self::modes`] means "ask nothing" — the mode a caller
    /// picks when something outside the harness already contains the run (a sandbox), so a
    /// permission prompt buys nothing and only stalls an unattended agent.
    ///
    /// The returned id must be one of the ids [`Self::modes`] lists; the harness's own
    /// spelling is the only spelling, and naming it here is what keeps callers from
    /// hard-coding one. Default `None`: the harness has no such mode, or it already asks
    /// nothing — the truthful answer for opencode, Copilot and Grok, whose bridges run
    /// `--dangerously-skip-permissions` / `--allow-all` unconditionally (see
    /// [`Self::modes`]), so there is no mode left to ask for.
    fn unattended_mode(&self) -> Option<&'static str> {
        None
    }
    /// Build a [`LoginPlan`] to interactively log this harness into `home` (a
    /// persistent per-account dir) and capture the resulting credential file(s).
    /// Implementations may write force-file-storage config into `home` before
    /// returning. Default: an error — no login-capture support for this harness.
    fn login(&self, _home: &Path) -> Result<LoginPlan> {
        anyhow::bail!(
            "credential login-capture for harness '{}' is not implemented",
            self.id()
        )
    }
    /// The user's live login as this harness itself would find it, when that
    /// login is **not a file under `$HOME`** that
    /// [`crate::provision`]'s zero-config fallback could copy.
    ///
    /// The fallback exists so a direct run — no account, no profile — reuses
    /// the session the user already has. It copies
    /// [`ConfigAnchor::login_seed`] out of the real `HOME`, which is enough
    /// for every harness that keeps a plaintext credential on disk. It is not
    /// enough for one that keeps it in the OS keychain: there is no file to
    /// copy, the run directory stays empty, and the harness reports itself
    /// logged out from inside the transcript.
    ///
    /// A harness in that position overrides this and returns its credential
    /// as a [`Source::Files`] keyed by the same `login_seed` `src` paths, so
    /// [`seed_login`] places it exactly as an account's captured login would
    /// be. Reading must be cheap and side-effect free — this runs on every
    /// launch, unlike [`Self::renew_credentials`], whose default *spawns the
    /// harness*. Errors are the same as no login: return `None` and let the
    /// run start logged out rather than fail.
    ///
    /// Default: `None` — the file copy is the whole story.
    fn ambient_login(&self) -> Option<Source> {
        None
    }
    /// Store a login this run *refreshed* back where [`Self::ambient_login`]
    /// found it — the write side of a credential that is not a file under
    /// `$HOME`, called by [`harvest_login`] for a non-[`Source::Dir`] origin.
    ///
    /// `src` is the [`ConfigAnchor::login_seed`] source path that names which
    /// credential this is; `bytes` are what the run left behind. Default: an
    /// error, which [`harvest_login`] logs — a harness whose login came from
    /// bytes rather than a directory and that cannot put them back is exactly
    /// the case worth a warning.
    fn adopt_login(&self, src: &Path, _bytes: &[u8]) -> Result<()> {
        anyhow::bail!(
            "harness '{}' has no way to store a refreshed login ({})",
            self.id(),
            src.display()
        )
    }
    /// Fix up `dir` after all login seeding (account-based and zero-config)
    /// has landed. Default: no-op. Overridden by harnesses whose captured
    /// login needs a tweak beyond a byte-for-byte file copy — e.g. Claude
    /// Code, whose non-interactive `claude auth login` capture never runs
    /// the interactive onboarding wizard, so the seeded `.claude.json` is
    /// missing `hasCompletedOnboarding`, and the harness still shows its
    /// onboarding UI on an otherwise fully-authenticated run; the same file
    /// also gates the per-project trust dialog, which every run — seeded
    /// login or not — should preempt for `spec.cwd`.
    fn post_seed(&self, _spec: &RunSpec, _dir: &Path) -> Result<()> {
        Ok(())
    }
    /// Argv fragment for the default credential-renew path
    /// ([`default_renew_via_launch`]): a short, non-billing subcommand that
    /// forces the harness to refresh and rewrite its stored token (e.g. an
    /// `auth status`/`whoami`-style probe). `None` (the default) means this
    /// harness has no headless refresh, so [`Self::renew_credentials`]'s
    /// default errors rather than launching anything.
    fn credential_renew_command(&self) -> Option<Vec<String>> {
        None
    }
    /// Renew a captured login and return the updated blobs (for
    /// [`crate::credentials::SecretStore::set`]).
    ///
    /// Default: [`default_renew_via_launch`] — seed `creds` into a temp config
    /// dir, run the harness with [`Self::credential_renew_command`], and read
    /// the (possibly refreshed) seed files back out. Harnesses whose token
    /// refresh isn't a headless command (e.g. Claude Code, whose live session
    /// is in the OS Keychain) override this with their own path.
    fn renew_credentials(
        &self,
        creds: &[crate::credentials::CredentialBlob],
    ) -> Result<Vec<crate::credentials::CredentialBlob>> {
        default_renew_via_launch(self, creds)
    }
    /// Build a structured-I/O bridge for a provisioned run.
    ///
    /// Default: unsupported (an error naming this harness). Overridden by
    /// harnesses whose bridge has landed (tracked by
    /// [`IoSupport::structured`]); until then this is the behavior every
    /// harness gets for free, and callers should check `io_support().structured`
    /// before invoking it if they want a nicer error message.
    fn structured_bridge(
        &self,
        _provisioned: &crate::provision::Provisioned,
        _cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        anyhow::bail!("harness '{}' does not support structured I/O", self.id())
    }
}

/// Every harness `am` knows how to wrap. (P1: Claude Code only; more are
/// added by writing more `Harness` impls.)
pub fn all() -> Vec<Box<dyn Harness>> {
    vec![
        Box::new(Claude::new()),
        // Same harness, second wire: `claude-code-acp` provisions Claude Code exactly as
        // `claude-code` does and speaks ACP for a structured run. One provisioner, two ids.
        Box::new(Claude::new_acp()),
        Box::new(Codex::new()),
        // Same harness, second wire: `codex-acp` provisions Codex exactly as `codex` does and
        // speaks ACP for a structured run. One provisioner, two ids.
        Box::new(Codex::new_acp()),
        Box::new(Copilot::new()),
        Box::new(Grok::new()),
        Box::new(Opencode::new()),
    ]
}

/// Resolve a harness by id, alias, or launch command (lenient — the CLI boundary).
pub fn resolve(key: &str) -> Option<Box<dyn Harness>> {
    all()
        .into_iter()
        .find(|h| h.id() == key || h.command() == key || h.aliases().contains(&key))
}

/// The list of known harness ids (for error messages).
pub fn known_ids() -> Vec<String> {
    all().iter().map(|h| h.id()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_matches_id_alias_and_command() {
        assert_eq!(resolve("claude-code").unwrap().id(), "claude-code");
        assert_eq!(resolve("claude").unwrap().id(), "claude-code");
        assert!(resolve("nope").is_none());
    }

    #[test]
    fn every_harness_has_a_command() {
        for h in all() {
            assert!(!h.command().is_empty(), "{} missing command", h.id());
        }
    }

    #[test]
    fn unattended_mode_is_one_of_the_harnesss_own_modes() {
        for h in all() {
            if let Some(mode) = h.unattended_mode() {
                assert!(
                    h.modes().iter().any(|m| m.id == mode),
                    "{}'s unattended mode '{mode}' is not one of its modes()",
                    h.id()
                );
            }
        }
    }

    #[test]
    fn every_harness_supports_passthrough() {
        // Passthrough (raw-tty pump) is the universal baseline every wrapped
        // harness must support.
        for h in all() {
            assert!(
                h.io_support().passthrough,
                "{} should support passthrough",
                h.id()
            );
        }
    }

    #[test]
    fn structured_io_support_matches_landed_bridges() {
        // Every harness in the table has landed a structured bridge: Claude
        // Code, Codex, opencode and Copilot on their own wires, Grok on ACP
        // (`grok agent stdio`, driven by `crate::io::AcpBridge`). This test
        // pins that so a new harness claiming `structured` without a bridge
        // behind it — or a bridge landing without the flag — is a deliberate,
        // visible change.
        for h in all() {
            let expected_structured = matches!(
                h.id().as_str(),
                "claude-code"
                    | "claude-code-acp"
                    | "codex"
                    | "codex-acp"
                    | "opencode"
                    | "copilot"
                    | "grok"
            );
            assert_eq!(
                h.io_support().structured,
                expected_structured,
                "{} structured support mismatch",
                h.id()
            );
        }
    }

    #[test]
    fn multi_turn_io_support_matches_the_one_shot_split() {
        // The one-shot split is now empty: copilot and opencode used to be
        // one-shot (prompt in argv, one answer, exit), but both now speak ACP
        // like claude-code, codex and grok, and ACP is inherently multi-turn.
        // So every structured harness today is multi_turn.
        //
        // Pinned here because a consumer reads this to decide whether a turn
        // is written to a running process or becomes the next launch's argv;
        // getting it wrong is a prompt that silently reaches nothing.
        for h in all() {
            let expected_multi_turn = h.io_support().structured;
            assert_eq!(
                h.io_support().multi_turn,
                expected_multi_turn,
                "{} multi-turn support mismatch",
                h.id()
            );
            assert!(
                !h.io_support().multi_turn || h.io_support().structured,
                "{} claims multi-turn without a structured bridge to take the turn",
                h.id()
            );
        }
    }

    #[test]
    fn acp_io_support_matches_the_harnesses_whose_bridge_is_acp() {
        // `acp` says the structured bridge is `crate::io::AcpBridge` rather
        // than a harness-specific wire. Grok, copilot, opencode and the ACP
        // variants of Claude Code and Codex speak it today — and
        // `claude-code-acp`/`codex-acp` are why this is read off `io_support()`
        // and never off the id: the same provisioner answers it both ways.
        // Pinned both ways: a harness that answers `acp` must have a
        // structured bridge at all, since an ACP harness without one is
        // incoherent.
        for h in all() {
            let expected_acp = matches!(
                h.id().as_str(),
                "grok" | "copilot" | "opencode" | "claude-code-acp" | "codex-acp"
            );
            assert_eq!(
                h.io_support().acp,
                expected_acp,
                "{} ACP support mismatch",
                h.id()
            );
            assert!(
                !h.io_support().acp || h.io_support().structured,
                "{} claims ACP without a structured bridge to speak it",
                h.id()
            );
        }
    }

    #[test]
    fn harness_without_structured_bridge_override_errors_mentioning_structured() {
        // A test-only harness that doesn't override `structured_bridge`
        // inherits the trait's default "unsupported" error. All real harnesses
        // override it, but this documents the behavior for a hypothetical
        // future harness.
        #[derive(Clone)]
        struct DummyHarness;

        impl Harness for DummyHarness {
            fn id(&self) -> crate::spec::HarnessId {
                "dummy".to_string()
            }
            fn display_name(&self) -> &str {
                "dummy"
            }
            fn command(&self) -> &str {
                "dummy"
            }
            fn aliases(&self) -> &[&str] {
                &[]
            }
            fn io_support(&self) -> IoSupport {
                IoSupport {
                    passthrough: false,
                    structured: false,
                    multi_turn: false,
                    acp: false,
                }
            }
            fn provision(&self, _spec: &crate::spec::RunSpec, _dir: &Path) -> Result<Launch> {
                anyhow::bail!("dummy harness provision not implemented")
            }
        }

        let dummy = DummyHarness;
        let provisioned = crate::provision::Provisioned {
            dir: std::path::PathBuf::from("/tmp/does-not-matter"),
            launch: Launch {
                program: "dummy".to_string(),
                args: vec![],
                env: vec![],
                env_remove: vec![],
                env_clear: false,
            },
            ephemeral: true,
            login_origin: None,
            resume: None,
            model: None,
            #[cfg(feature = "inproc-mcp")]
            inproc_servers: Vec::new(),
        };
        // `.unwrap_err()` needs `T: Debug`, but `Box<dyn IoBridge>` isn't
        // `Debug`; match the `Err` arm directly instead.
        let result = dummy.structured_bridge(&provisioned, std::path::Path::new("."));
        match result {
            Ok(_) => panic!("expected an error"),
            Err(err) => assert!(err.to_string().contains("structured"), "error was: {err}"),
        }
    }

    /// A harness whose login is one credential file plus one companion that is
    /// deliberately not a credential — the shape `harvest_login` has to tell
    /// apart. No `adopt_login`, so a non-directory origin takes the trait's
    /// default error.
    struct SeedHarness;

    impl Harness for SeedHarness {
        fn id(&self) -> crate::spec::HarnessId {
            "seedy".to_string()
        }
        fn display_name(&self) -> &str {
            "seedy"
        }
        fn command(&self) -> &str {
            "seedy"
        }
        fn aliases(&self) -> &[&str] {
            &[]
        }
        fn io_support(&self) -> IoSupport {
            IoSupport {
                passthrough: true,
                structured: false,
                multi_turn: false,
                acp: false,
            }
        }
        fn config_anchor(&self) -> ConfigAnchor {
            ConfigAnchor {
                levers: Vec::new(),
                login_seed: vec![
                    SeedFile::credential(".creds/token.json", "token.json"),
                    SeedFile::new(".claude.json", ".claude.json"),
                ],
                requires_home_relocation: false,
            }
        }
        fn provision(&self, _spec: &crate::spec::RunSpec, _dir: &Path) -> Result<Launch> {
            anyhow::bail!("seedy provision not implemented")
        }
    }

    /// Seed both files from `home` into a fresh run dir, and hand back the two.
    fn seeded() -> (tempfile::TempDir, tempfile::TempDir) {
        let home = tempfile::TempDir::new().unwrap();
        let run = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".creds")).unwrap();
        std::fs::write(home.path().join(".creds/token.json"), "OLD-TOKEN").unwrap();
        std::fs::write(home.path().join(".claude.json"), "OLD-IDENTITY").unwrap();
        seed_login(
            run.path(),
            &Source::Dir(home.path().to_path_buf()),
            &SeedHarness.config_anchor().login_seed,
        )
        .unwrap();
        (home, run)
    }

    #[test]
    fn harvest_leaves_an_unchanged_credential_alone() {
        let (home, run) = seeded();
        let origin = home.path().join(".creds/token.json");
        // A write-back would also chmod to 0600, so the mode is what tells
        // "left alone" apart from "written with identical bytes".
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&origin, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        harvest_login(
            &SeedHarness,
            run.path(),
            &Source::Dir(home.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(&origin).unwrap(), "OLD-TOKEN");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&origin).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o644, "an untouched credential was rewritten");
        }
    }

    #[test]
    fn harvest_writes_a_refreshed_credential_back_to_its_origin() {
        let (home, run) = seeded();
        std::fs::write(run.path().join("token.json"), "NEW-TOKEN").unwrap();

        harvest_login(
            &SeedHarness,
            run.path(),
            &Source::Dir(home.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(home.path().join(".creds/token.json")).unwrap(),
            "NEW-TOKEN"
        );
    }

    #[test]
    fn harvest_never_writes_back_a_non_credential_seed_file() {
        let (home, run) = seeded();
        // What Claude Code's `.claude.json` picks up during a run: project
        // history and onboarding state that must not reach the user's own file.
        std::fs::write(run.path().join(".claude.json"), "RUN-IDENTITY").unwrap();

        harvest_login(
            &SeedHarness,
            run.path(),
            &Source::Dir(home.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            "OLD-IDENTITY"
        );
    }

    #[test]
    fn harvest_logs_rather_than_fails_when_the_origin_is_not_a_directory() {
        let run = tempfile::TempDir::new().unwrap();
        let origin = Source::Files(vec![(
            PathBuf::from(".creds/token.json"),
            b"OLD-TOKEN".to_vec(),
        )]);
        seed_login(run.path(), &origin, &SeedHarness.config_anchor().login_seed).unwrap();
        std::fs::write(run.path().join("token.json"), "NEW-TOKEN").unwrap();

        // `SeedHarness` has no `adopt_login`, so the write-back errors — and
        // harvesting still succeeds, because a teardown must not break.
        harvest_login(&SeedHarness, run.path(), &origin).unwrap();
    }
}
