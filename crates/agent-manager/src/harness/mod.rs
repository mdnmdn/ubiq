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
use crate::quota::QuotaSource;
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
    /// How — if at all — this harness can be asked how much of the plan is left. Read *before*
    /// anything is spawned, so a caller can offer the question, disable it, or say the provider
    /// states nothing, rather than discovering that from a failed call.
    pub quota: QuotaSource,
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

/// A declarative description of how a harness relocates its config/credentials,
/// which the isolation model reads. See `_docs/profiles.md` §5.1.
#[derive(Debug, Clone)]
pub struct ConfigAnchor {
    /// Env vars (with their relocation semantics) that point the harness's
    /// config/data at a dir `am` controls while leaving `HOME` real. Empty for
    /// Class-C harnesses that have no lever (see `requires_home_relocation`).
    pub levers: Vec<(String, Relocate)>,
    /// True only for Class-C harnesses (no config lever): the credential store
    /// is reachable only by relocating `HOME`, which strips the toolchain — so
    /// these should be paired with isol8. False for Class A/B.
    pub requires_home_relocation: bool,
}

/// `$var` when the environment sets it, else `~/<default>` under the user's home: how a harness
/// finds its own config when nothing relocates it ([`Harness::default_homes`]).
pub(crate) fn env_dir_or_home(var: &str, default: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(default)))
}

/// Take an exclusive lock on `<file>.am-lock`, the sibling lock file every read-modify-write
/// of a file in a shared config home goes through. Blocks until the lock is free; released when
/// the returned handle drops.
pub(crate) fn lock_beside(file: &Path) -> Result<std::fs::File> {
    let mut name = file.as_os_str().to_owned();
    name.push(".am-lock");
    let lock_path = PathBuf::from(name);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("locking {}", lock_path.display()))?;
    Ok(lock)
}

/// Place `spec`'s skills (and MCP-as-skill pointers) into `skills_dir` inside a shared home,
/// as profile-owned content for a harness with no per-run skill route (`D193`). The set is
/// built in `.am-stage/` beside them and each `<id>/` renamed over its old copy, so a running
/// harness reads one skill's old files or its new ones, never half of each; all of it under a
/// lock beside `skills_dir`, since every run of the profile writes here. A skill the run does
/// not name is left alone.
pub(crate) fn write_profile_skills(spec: &RunSpec, skills_dir: &Path) -> Result<()> {
    if spec.skills.is_empty() && spec.mcp_as_skill.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(skills_dir)
        .with_context(|| format!("creating {}", skills_dir.display()))?;
    // Released when `_lock` drops, at the end of this function.
    let _lock = lock_beside(skills_dir)?;
    let stage = skills_dir.join(".am-stage");
    let old = skills_dir.join(".am-old");
    for leftover in [&stage, &old] {
        if leftover.exists() {
            std::fs::remove_dir_all(leftover)
                .with_context(|| format!("removing {}", leftover.display()))?;
        }
    }
    for skill in &spec.skills {
        let dest = stage.join(&skill.id);
        skill
            .source
            .materialize(&dest, crate::source::LinkMode::Copy, true)
            .with_context(|| format!("copying skill '{}' into {}", skill.id, dest.display()))?;
    }
    write_mcp_as_skill_pointers(spec, &stage)?;
    for entry in
        std::fs::read_dir(&stage).with_context(|| format!("reading {}", stage.display()))?
    {
        let staged = entry?.path();
        let dest = skills_dir.join(staged.file_name().unwrap_or_default());
        if dest.exists() {
            std::fs::rename(&dest, &old)
                .with_context(|| format!("moving aside {}", dest.display()))?;
        }
        std::fs::rename(&staged, &dest).with_context(|| format!("placing {}", dest.display()))?;
        if old.exists() {
            std::fs::remove_dir_all(&old).with_context(|| format!("removing {}", old.display()))?;
        }
    }
    std::fs::remove_dir(&stage).with_context(|| format!("removing {}", stage.display()))
}

/// Write a credential blob to `path`, creating parents, `0600` on unix.
pub(crate) fn write_credential(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // Written beside the target and renamed onto it, rather than truncated in
    // place: the file may sit in a home a running harness reads, and a
    // truncate-then-write leaves a window in which it reads half a file. The
    // temporary name carries the pid so two processes cannot collide on it.
    let staged = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&staged, bytes).with_context(|| format!("writing {}", staged.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 600 {}", staged.display()))?;
    }
    std::fs::rename(&staged, path).with_context(|| format!("writing {}", path.display()))?;
    tracing::debug!(path = %path.display(), bytes = bytes.len(), "wrote a credential file");
    Ok(())
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
    /// Whether this harness can run from a profile's shared config home with every per-run file
    /// kept outside it, passed by flag ([`crate::spec::ConfigStrategy::Home`], `D193`). Default
    /// `false`: [`crate::provision::provision`] then provisions into the run's scratch dir as it
    /// would a [`crate::spec::ConfigStrategy::Fixed`] one.
    fn shares_home(&self) -> bool {
        false
    }
    /// Where this harness keeps its config and login when nothing relocates it — what a
    /// [`crate::spec::ConfigStrategy::Native`] run reads and writes, and so what a confined one
    /// has to be granted read-write (`D193`). Every path, since one directory is not always all
    /// of it (Claude Code's `~/.claude.json` sits beside `~/.claude`). Computed, never created;
    /// read from this process's environment. Default: none.
    fn default_homes(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    /// Compose a run against the shared config `home`, writing nothing per-run into it: every
    /// per-run file goes into `scratch` and reaches the harness by flag. `None` is
    /// [`crate::spec::ConfigStrategy::Native`]: the launch names no config home, so the harness
    /// reads its own default. Called only when [`Self::shares_home`] answers `true`. Default:
    /// an error naming this harness.
    fn provision_home(
        &self,
        _spec: &RunSpec,
        _home: Option<&Path>,
        _scratch: &Path,
    ) -> Result<Launch> {
        anyhow::bail!(
            "harness '{}' cannot run from a shared config home",
            self.id()
        )
    }
    /// An interactive login performed directly into `home`, a profile's shared config home
    /// (`D193`). The harness keeps the login there and owns its refresh; nothing is captured
    /// or read back. Default: an error naming this harness.
    fn login_home(&self, _home: &Path) -> Result<Launch> {
        anyhow::bail!(
            "harness '{}' cannot log in into a shared config home",
            self.id()
        )
    }
    /// How this harness relocates its config/credentials, for the isolation
    /// model — see `_docs/profiles.md` §5.
    ///
    /// The default is a conservative empty anchor (no levers, no HOME
    /// relocation); **every real harness overrides it**. It exists only so the
    /// crate keeps compiling while harness ports land incrementally.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: Vec::new(),
            requires_home_relocation: false,
        }
    }
    /// Files the harness wrote as its own record of the conversation, inside a
    /// relocated config dir. Empty = this harness's record is not portable yet.
    fn transcripts(&self, _config_dir: &Path) -> Vec<PathBuf> {
        Vec::new()
    }
    /// The files the harness wrote for ONE of its sessions, `session_id`, run in `cwd` — the
    /// accessor for a config home many runs share ([`crate::spec::ConfigStrategy::Home`]) or the
    /// user's own ([`crate::spec::ConfigStrategy::Native`], `config_home` `None`), where
    /// [`Self::transcripts`] would answer every run's. Only paths that exist are returned.
    /// Default `None`: this harness cannot name one session's record.
    fn session_transcripts(
        &self,
        _config_home: Option<&Path>,
        _cwd: &Path,
        _session_id: &str,
    ) -> Option<Vec<PathBuf>> {
        None
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
        let mut cmd = std::process::Command::new(self.command());
        cmd.arg("--version");
        cmd.current_dir(shared::probe_cwd());
        #[cfg(windows)]
        shared::no_window(&mut cmd);
        let output = cmd
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
    /// Whether this harness would understand `mode` as a permission mode — the question
    /// [`crate::resolve::resolve`] asks before letting a profile's (or a caller's) mode reach
    /// `spec.policy`, so a stale spelling is dropped with a warning instead of failing inside
    /// the harness.
    ///
    /// The default answers from [`Self::modes`], case-insensitively, and answers **true** for a
    /// harness whose `modes()` is empty: an empty list means "this harness has no permission-mode
    /// concept", not "it accepts nothing", and dropping a value against an unstated list would be
    /// worse than passing it through. Override only to accept a spelling that is real but is not
    /// a picker entry — [`Codex`]'s documented `restricted` alias is the
    /// one case in the tree.
    fn accepts_mode(&self, mode: &str) -> bool {
        let modes = self.modes();
        modes.is_empty() || modes.iter().any(|m| m.id.eq_ignore_ascii_case(mode))
    }
    /// Fix up `dir` once the run's config has been written. Default: no-op.
    /// Overridden by Claude Code, whose `.claude.json` gates both the
    /// onboarding wizard and the per-project trust dialog, which every run
    /// should preempt for `spec.cwd`.
    fn post_seed(&self, _spec: &RunSpec, _dir: &Path) -> Result<()> {
        Ok(())
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

    /// How much of `account`'s plan is left, asked of the provider now.
    ///
    /// `home` is the config home the harness itself keeps its login in for this account's runs —
    /// a profile's shared home, or `None` for the harness's own default. The implementation reads
    /// the credential there and copies or writes nothing (`G380`).
    ///
    /// Default: an error naming this harness, exactly as [`Self::discover_models`] and
    /// [`Self::structured_bridge`] do — and, for the three harnesses whose providers publish no
    /// queryable limit at all, the permanent and correct answer. Callers check
    /// [`IoSupport::quota`] first if they want to tell "cannot be asked" from "asked and failed".
    ///
    /// **The credential never comes back out.** An implementation reads the token, spends it on
    /// one request and returns percentages; nothing token-shaped reaches a [`QuotaSnapshot`], a
    /// log, or the caller.
    fn quota(&self, _account: &str, _home: Option<&Path>) -> Result<crate::quota::QuotaSnapshot> {
        Err(crate::quota::unreported(&self.id()))
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
    fn every_harness_accepts_every_mode_it_lists_and_rejects_a_typo() {
        for h in all() {
            for m in h.modes() {
                assert!(
                    h.accepts_mode(&m.id),
                    "{} lists mode '{}' but does not accept it",
                    h.id(),
                    m.id
                );
            }
            // A harness that states no modes accepts anything (see `accepts_mode`); one that
            // states some rejects a spelling that is in neither its list nor its aliases.
            assert_eq!(
                h.accepts_mode("definitely-not-a-mode"),
                h.modes().is_empty(),
                "{} answered the wrong way for an invented mode",
                h.id()
            );
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
                    quota: Default::default(),
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
            home: None,
            resume: None,
            model: None,
            mcp_servers: Vec::new(),
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

    /// A stand-in ACP agent: answers `initialize` (advertising `loadSession` and http MCP) and
    /// the session opener, writing the opener's frame to `capture` first.
    #[cfg(unix)]
    fn fake_acp_agent(capture: &Path) -> Launch {
        let script = r#"read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"mcpCapabilities":{"http":true}}}}'
read -r line
printf '%s\n' "$line" > "$1"
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}'
cat > /dev/null"#;
        Launch {
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                script.to_string(),
                "sh".to_string(),
                capture.display().to_string(),
            ],
            env: Vec::new(),
            env_remove: Vec::new(),
            env_clear: false,
        }
    }

    /// `claude-code-acp` and `codex-acp` send the run's servers in `session/new` and
    /// `session/load`; a harness that injects MCP itself (opencode) sends `[]` whatever the
    /// `Provisioned` holds.
    #[cfg(unix)]
    #[test]
    fn only_an_opted_in_acp_harness_sends_the_runs_mcp_servers() {
        let dir = tempfile::TempDir::new().unwrap();
        let server = crate::config::McpServer {
            id: "ubiq".to_string(),
            transport: crate::config::McpTransport::Http,
            command: None,
            args: Vec::new(),
            env: Default::default(),
            url: Some("http://127.0.0.1:1/run".to_string()),
            headers: Default::default(),
        };
        let cases: [(Box<dyn Harness>, Option<&str>, usize); 4] = [
            (Box::new(Claude::new_acp()), None, 1),
            (Box::new(Claude::new_acp()), Some("s0"), 1),
            (Box::new(Codex::new_acp()), Some("s0"), 1),
            (Box::new(Opencode::new()), None, 0),
        ];
        for (i, (harness, resume, sent)) in cases.into_iter().enumerate() {
            let capture = dir.path().join(format!("frame-{i}.json"));
            let provisioned = crate::provision::Provisioned {
                dir: dir.path().to_path_buf(),
                launch: fake_acp_agent(&capture),
                ephemeral: false,
                home: None,
                resume: resume.map(str::to_string),
                model: None,
                mcp_servers: vec![server.clone()],
                #[cfg(feature = "inproc-mcp")]
                inproc_servers: Vec::new(),
            };
            let Ok(bridge) = harness.structured_bridge(&provisioned, dir.path()) else {
                panic!("{} bridge failed", harness.id());
            };
            drop(bridge);
            let frame: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&capture).unwrap()).unwrap();
            let method = if resume.is_some() {
                "session/load"
            } else {
                "session/new"
            };
            assert_eq!(frame["method"], method, "{}", harness.id());
            let servers = frame["params"]["mcpServers"].as_array().unwrap();
            assert_eq!(servers.len(), sent, "{}: {frame}", harness.id());
            if sent > 0 {
                assert_eq!(servers[0]["url"], "http://127.0.0.1:1/run");
            }
        }
    }
}
