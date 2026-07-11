//! Knowledge about each supported harness.
//!
//! Where the old design had a `Harness` *struct* of static facts, the target
//! design needs a `Harness` *trait* with behavior: each harness differs in how
//! it is provisioned, launched, and (later) spoken to. See
//! `_docs/target/architecture.md` §"The `Harness` trait" and
//! `_docs/harness/<id>.md` for the curated per-harness notes each impl
//! transcribes.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::spec::{HarnessId, McpAsSkill, RunSpec};
use crate::Result;

mod claude;
mod codex;
mod grok;
mod opencode;
pub use claude::Claude;
pub use codex::Codex;
pub use grok::Grok;
pub use opencode::Opencode;

/// Recursively copy `src` into `dst`, creating directories as needed.
///
/// Shared by harness provisioners that copy skill folders into an ephemeral
/// config dir (e.g. [`claude::Claude`], [`codex::Codex`]).
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Render the `SKILL.md` body for one [`McpAsSkill`] pointer.
///
/// Shared by every provisioner so the generated content is byte-identical
/// across harnesses (each provisioner just picks its own skills dir to write
/// it into via [`write_mcp_as_skill_pointers`]).
///
/// **Honest about scope** (see `_docs/target/mcp-as-skill.md`): this is the
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
#[derive(Debug, Clone)]
pub struct Launch {
    /// Program to exec, e.g. `"claude"`.
    pub program: String,
    /// Arguments (injected flags first, then the user's passthrough args).
    pub args: Vec<String>,
    /// Environment variables to SET for the child.
    pub env: Vec<(String, String)>,
    /// Environment variables to REMOVE from the inherited env (hygiene).
    pub env_remove: Vec<String>,
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
}

/// How a harness's native env lever relocates its config/credentials into a
/// dir `am` controls (so the real `HOME` — and the user's toolchain — is left
/// intact). See `_docs/target/profiles.md` §5 for the A/B/C taxonomy.
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
}

impl SeedFile {
    /// A seed-file mapping from an account-home-relative `src` to a
    /// relocated-dir-relative `dst`.
    pub fn new(src: impl Into<PathBuf>, dst: impl Into<PathBuf>) -> Self {
        SeedFile {
            src: src.into(),
            dst: dst.into(),
        }
    }
}

/// A declarative description of how a harness relocates its config/credentials
/// and which files constitute a captured login. This is what makes credential
/// seeding generic across harnesses (rather than bespoke per provisioner) and
/// what lazy default-profile capture and the isolation model read. See
/// `_docs/target/profiles.md` §5.1.
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

/// Seed captured-login files from an account's persistent `home` into a
/// harness's relocated config `dir`, per a [`ConfigAnchor::login_seed`].
///
/// Copies (never symlinks — a run is ephemeral and the harness rewrites some of
/// these in place), creating parent dirs, and **skips any source that doesn't
/// exist** so a reference-only or partially-captured account still launches.
/// Deliberately leaves `HOME` untouched (see `_docs/target/profiles.md` §3).
pub(crate) fn seed_login(dir: &Path, home: &Path, seed: &[SeedFile]) -> Result<()> {
    for file in seed {
        let src = home.join(&file.src);
        if !src.exists() {
            continue;
        }
        let dst = dir.join(&file.dst);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::copy(&src, &dst)
            .with_context(|| format!("seeding login from {}", src.display()))?;
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
    /// `_docs/target/profiles.md` §5.
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
    /// Build a [`LoginPlan`] to interactively log this harness into `home` (a
    /// persistent per-account dir) and capture the resulting credential file(s).
    /// Implementations may write force-file-storage config into `home` before
    /// returning. Default: an error — no login-capture support for this harness.
    fn login(&self, _home: &Path) -> Result<LoginPlan> {
        anyhow::bail!("credential login-capture for harness '{}' is not implemented", self.id())
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
        Box::new(Codex::new()),
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
        // Claude Code, Codex, and opencode have landed their structured
        // bridges; Grok is passthrough-only for now (its `--format json`
        // event field shapes aren't documented enough to build a faithful
        // bridge yet — see `_docs/harness/grok.md`). This test pins that
        // split so adding a bridge (or a new passthrough-only harness) is a
        // deliberate, visible change.
        for h in all() {
            let expected_structured = matches!(h.id().as_str(), "claude-code" | "codex" | "opencode");
            assert_eq!(
                h.io_support().structured,
                expected_structured,
                "{} structured support mismatch",
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
            },
            ephemeral: true,
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
}
