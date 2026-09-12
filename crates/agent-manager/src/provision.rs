//! Turns a [`RunSpec`] + a chosen [`Harness`] into a populated ephemeral
//! config dir + a [`Launch`].
//!
//! This module owns directory creation and location. It does NOT launch the
//! harness (that's `run`, a later stage) and does NOT clean up the directory
//! afterwards (the runner owns that lifecycle) — see
//! `_docs/architecture.md` §"The provisioner and the custom config
//! folder bridge".

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;
use crate::harness::{Harness, Launch, TemplateStore};
use crate::source::Source;
use crate::spec::{ConfigStrategy, RunSpec};

/// The result of provisioning: where the config was written and how to launch.
///
/// `Clone` is only available when the `inproc-mcp` feature is off: with it
/// on, a `Provisioned` may own live [`crate::mcp::server::InProcessServer`]
/// handles (real OS resources — a bound socket + a serving thread), which
/// have single-owner shutdown-on-drop semantics and so aren't cloneable.
#[derive(Debug)]
pub struct Provisioned {
    /// The (created, populated) ephemeral config dir.
    pub dir: PathBuf,
    /// How to launch the harness against `dir`.
    pub launch: Launch,
    /// True if `dir` is a throwaway the runner should delete on exit
    /// (`Ephemeral`); false if the user pinned it (`Fixed`).
    pub ephemeral: bool,
    /// Where this run's login was seeded *from*, when one was — the origin
    /// [`crate::harness::harvest_login`] writes a refreshed credential back to
    /// before `dir` is discarded. `None` when nothing was seeded (no login, or
    /// a profile overlay placed it and there is no single origin to name).
    pub login_origin: Option<Source>,
    /// The conversation id this run resumes, when it resumes one — copied from
    /// [`crate::spec::RunSpec::resume`]. Most harnesses put a resume in argv and
    /// never read this; an ACP harness resumes over the wire (`session/load`), so
    /// its bridge needs the id after provisioning has already happened.
    pub resume: Option<String>,
    /// The model this run asked for, when one was named — copied from
    /// [`crate::spec::RunSpec::model`]. Most harnesses put the model in argv and
    /// never read this; an ACP harness sets the model over the wire after
    /// `session/new`, so its bridge needs the value after provisioning has
    /// already happened.
    pub model: Option<String>,
    /// In-process MCP servers started for this run. Kept alive for the
    /// run's lifetime; dropping a `Provisioned` shuts them down. Only
    /// present when the `inproc-mcp` feature is enabled.
    #[cfg(feature = "inproc-mcp")]
    pub inproc_servers: Vec<crate::mcp::server::InProcessServer>,
}

#[cfg(not(feature = "inproc-mcp"))]
impl Clone for Provisioned {
    fn clone(&self) -> Self {
        Provisioned {
            dir: self.dir.clone(),
            launch: self.launch.clone(),
            ephemeral: self.ephemeral,
            login_origin: self.login_origin.clone(),
            resume: self.resume.clone(),
            model: self.model.clone(),
        }
    }
}

/// Provision `spec` for `harness` into a fresh (or pinned) config dir.
///
/// When the `inproc-mcp` feature is enabled, any `McpRef::InProcess` entries
/// in `spec.mcps` are hosted on a loopback HTTP MCP server *before* the
/// harness provisions, and replaced with an `McpRef::Inline` http server
/// pointed at that loopback URL — so every harness sees a normal remote MCP
/// server, with no per-harness change needed. The started servers are kept
/// alive in the returned `Provisioned` and shut down when it is dropped.
/// When the feature is off, `spec` is passed through unchanged and each
/// harness's provisioner `bail!`s on `McpRef::InProcess`, as before.
///
/// `templates` is the [`TemplateStore`] the harness's preference templates are
/// read from (the CLI passes an [`crate::harness::FsTemplateStore`]; an embedder
/// may pass its own) — the injection point that lets templates live somewhere
/// other than `~/.config/agent-manager/templates`.
pub fn provision(
    harness: &dyn Harness,
    spec: &RunSpec,
    templates: &dyn TemplateStore,
) -> Result<Provisioned> {
    let (dir, ephemeral) = match &spec.config {
        ConfigStrategy::Fixed(path) => (path.clone(), false),
        ConfigStrategy::Ephemeral => (new_run_dir()?, true),
    };

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config dir {}", dir.display()))?;

    #[cfg(feature = "inproc-mcp")]
    {
        let (effective_spec, inproc_servers) = host_inproc_mcps(spec)?;
        let launch = harness.provision(&effective_spec, &dir)?;
        // Layer the profile config overlay on top of the harness-written config.
        crate::overlay::materialize(&dir, &spec.config_bases)?;
        let account_origin = account_login_origin(harness, spec, &dir);
        let login_origin = account_origin.or(seed_zero_config_login(harness, spec, &dir)?);
        crate::harness::apply_templates(&dir, &harness.id(), &harness.templates(), templates)?;
        harness.post_seed(&effective_spec, &dir)?;
        Ok(Provisioned {
            dir,
            launch,
            ephemeral,
            login_origin,
            resume: spec.resume.clone(),
            model: spec.model.clone(),
            inproc_servers,
        })
    }
    #[cfg(not(feature = "inproc-mcp"))]
    {
        let launch = harness.provision(spec, &dir)?;
        // Layer the profile config overlay on top of the harness-written config.
        crate::overlay::materialize(&dir, &spec.config_bases)?;
        let account_origin = account_login_origin(harness, spec, &dir);
        let login_origin = account_origin.or(seed_zero_config_login(harness, spec, &dir)?);
        crate::harness::apply_templates(&dir, &harness.id(), &harness.templates(), templates)?;
        harness.post_seed(spec, &dir)?;
        Ok(Provisioned {
            dir,
            launch,
            ephemeral,
            login_origin,
            resume: spec.resume.clone(),
            model: spec.model.clone(),
        })
    }
}

/// Start a loopback HTTP MCP server for each `McpRef::InProcess` in
/// `spec.mcps`, and return a copy of `spec` with those entries replaced by
/// `McpRef::Inline` http servers pointed at the new loopback URLs, plus the
/// started servers (to be kept alive for the run).
#[cfg(feature = "inproc-mcp")]
fn host_inproc_mcps(spec: &RunSpec) -> Result<(RunSpec, Vec<crate::mcp::server::InProcessServer>)> {
    use crate::config::{McpServer, McpTransport};
    use crate::spec::McpRef;

    let mut servers = Vec::new();
    let mut mcps = Vec::with_capacity(spec.mcps.len());
    for mcp in &spec.mcps {
        match mcp {
            McpRef::InProcess(handle) => {
                let server = crate::mcp::server::start(handle.service.clone())?;
                mcps.push(McpRef::Inline(McpServer {
                    id: handle.name.clone(),
                    transport: McpTransport::Http,
                    command: None,
                    args: Vec::new(),
                    env: Default::default(),
                    url: Some(server.url()),
                    headers: Default::default(),
                }));
                servers.push(server);
            }
            other => mcps.push(other.clone()),
        }
    }

    let mut effective = spec.clone();
    effective.mcps = mcps;
    Ok((effective, servers))
}

/// Zero-config login reuse (tier A "just works"): when a bare `am <harness>`
/// run got no login from an account home or a profile overlay, seed the
/// harness's captured login so it reuses the existing session instead of
/// onboarding. Two tiers, tried in order:
///
/// 1. Copy [`crate::harness::ConfigAnchor::login_seed`] out of the user's
///    **real** `HOME` — correct for every harness that keeps its credential
///    as a plain file, since that's the same file the harness itself would
///    read.
/// 2. If that copy placed nothing, fall back to [`Harness::ambient_login`] —
///    the harness's own account of its live login, for the harnesses (Claude
///    Code, via the OS Keychain) whose credential isn't a `HOME`-relative
///    file at all, so tier 1 can never find it.
///
/// No-op when: the harness declares no `login_seed`; a login was already placed
/// (an account home or overlay seeded it — that wins over both tiers below);
/// or the account supplies env/key/helper credentials (those manage their own
/// auth). Missing source files are skipped (see [`crate::harness::seed_login`]),
/// so this only ever *adds* an existing login and never fails a run for the lack
/// of one. Never overrides `HOME`.
///
/// Returns the [`Source`] it seeded from, so the run can write a refreshed
/// credential back to it at teardown ([`crate::harness::harvest_login`]).
fn seed_zero_config_login(
    harness: &dyn Harness,
    spec: &RunSpec,
    dir: &Path,
) -> Result<Option<Source>> {
    let anchor = harness.config_anchor();
    if anchor.login_seed.is_empty() {
        return Ok(None);
    }
    // A login was already materialized (account home or overlay) — respect it.
    if anchor.login_seed.iter().any(|s| dir.join(&s.dst).exists()) {
        return Ok(None);
    }
    // Env/key/helper accounts manage their own auth; don't seed a stale OAuth login.
    if let Some(acct) = &spec.account
        && (acct.api_key_env.is_some() || acct.auth_token_env.is_some() || acct.helper.is_some())
    {
        return Ok(None);
    }
    // Tier 1: a real file under the real HOME wins — it's what the harness
    // itself would read.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let home = Source::Dir(home);
        crate::harness::seed_login(dir, &home, &anchor.login_seed)?;
        if anchor.login_seed.iter().any(|s| dir.join(&s.dst).exists()) {
            return Ok(Some(home));
        }
    }
    // Tier 2: no file landed — ask the harness for its own account of the
    // live login (e.g. Claude Code's OS-Keychain session).
    if let Some(ambient) = harness.ambient_login() {
        crate::harness::seed_login(dir, &ambient, &anchor.login_seed)?;
        return Ok(Some(ambient));
    }
    Ok(None)
}

/// Which [`Source`] an *account's* login was seeded from, when one was.
///
/// That seeding happens inside each harness's own `provision` (from
/// `spec.account_login`, else the account's `home`), so it is recognised here
/// by its result: the login files are already in `dir` before the zero-config
/// fallback runs. A profile overlay can place the same files and names no
/// origin — those yield `None` rather than a guess at the account's.
fn account_login_origin(harness: &dyn Harness, spec: &RunSpec, dir: &Path) -> Option<Source> {
    let anchor = harness.config_anchor();
    if !anchor.login_seed.iter().any(|s| dir.join(&s.dst).exists()) {
        return None;
    }
    spec.account_login
        .clone()
        .or_else(|| Some(Source::Dir(spec.account.as_ref()?.home.clone()?)))
}

/// Generate a fresh `<runs-root>/<run-id>/` path for an ephemeral run.
///
/// `<runs-root>` is the `AM_RUNS` env var if set, else
/// `~/.config/agent-manager/runs` ([`crate::settings::default_config_dir`]) —
/// the same base dir as every other agent-manager store. `<run-id>` is
/// `<unix-millis>-<pid>`, which is unique enough for a single-host tool
/// without pulling in a UUID dependency.
fn new_run_dir() -> Result<PathBuf> {
    let base = std::env::var("AM_RUNS")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::settings::default_config_dir().map(|d| d.join("runs")))
        .context("could not determine a runs directory for this OS")?;

    // Opportunistic GC: sweep run dirs older than the TTL. Best-effort — never
    // fails a run (an unreadable/locked runs dir just leaves stale dirs).
    let _ = crate::overlay::sweep_old_runs(&base);

    let run_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        std::process::id()
    );

    Ok(base.join(run_id))
}

/// A harness stand-in for [`seed_zero_config_login`]'s tier-2 (ambient
/// login) tests: a single `login_seed` file whose `src` name is unique
/// enough that no real `$HOME` on the test machine could ever contain it, so
/// tier 1's file copy always finds nothing and the test doesn't depend on —
/// or need to mock — the process's real `HOME`.
#[cfg(test)]
#[derive(Debug, Clone)]
struct AmbientDummyHarness {
    ambient: Option<Source>,
}

#[cfg(test)]
impl Harness for AmbientDummyHarness {
    fn id(&self) -> crate::spec::HarnessId {
        "ambient-dummy".to_string()
    }
    fn display_name(&self) -> &str {
        "ambient dummy"
    }
    fn command(&self) -> &str {
        "ambient-dummy"
    }
    fn aliases(&self) -> &[&str] {
        &[]
    }
    fn io_support(&self) -> crate::harness::IoSupport {
        crate::harness::IoSupport {
            passthrough: false,
            structured: false,
            multi_turn: false,
            acp: false,
            quota: Default::default(),
        }
    }
    fn config_anchor(&self) -> crate::harness::ConfigAnchor {
        crate::harness::ConfigAnchor {
            levers: Vec::new(),
            login_seed: vec![crate::harness::SeedFile::new(
                "am-test-ambient-login-src-2f0c1e6a.json",
                "ambient-login-dst.json",
            )],
            requires_home_relocation: false,
        }
    }
    fn ambient_login(&self) -> Option<Source> {
        self.ambient.clone()
    }
    fn provision(&self, _spec: &RunSpec, _dir: &Path) -> Result<Launch> {
        anyhow::bail!("ambient-dummy harness provision not implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Claude;
    use crate::spec::RunSpec;
    use std::path::PathBuf;

    /// Tier 2: no file landed from `$HOME` (the seed `src` name is unique to
    /// this test, so tier 1 finds nothing on any real machine) — the
    /// harness's own `ambient_login()` gets seeded instead.
    #[test]
    fn seed_zero_config_login_falls_back_to_ambient_login_when_home_has_no_file() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("ambient-dummy".to_string(), PathBuf::from("."));
        let harness = AmbientDummyHarness {
            ambient: Some(Source::Files(vec![(
                PathBuf::from("am-test-ambient-login-src-2f0c1e6a.json"),
                b"AMBIENT-LOGIN".to_vec(),
            )])),
        };

        seed_zero_config_login(&harness, &spec, config_dir.path()).unwrap();

        let dst = config_dir.path().join("ambient-login-dst.json");
        assert!(
            dst.exists(),
            "ambient login should be seeded when HOME has no file"
        );
        assert_eq!(std::fs::read(&dst).unwrap(), b"AMBIENT-LOGIN");
    }

    /// A login already materialized into `dir` (by an account home or a
    /// profile overlay, run before this function) wins outright: the
    /// harness's `ambient_login()` is never consulted, let alone allowed to
    /// overwrite it.
    #[test]
    fn seed_zero_config_login_does_not_call_ambient_login_when_a_file_already_landed() {
        let config_dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            config_dir.path().join("ambient-login-dst.json"),
            b"REAL-LOGIN",
        )
        .unwrap();
        let spec = RunSpec::new("ambient-dummy".to_string(), PathBuf::from("."));
        let harness = AmbientDummyHarness {
            ambient: Some(Source::Files(vec![(
                PathBuf::from("am-test-ambient-login-src-2f0c1e6a.json"),
                b"AMBIENT-LOGIN".to_vec(),
            )])),
        };

        seed_zero_config_login(&harness, &spec, config_dir.path()).unwrap();

        let dst = config_dir.path().join("ambient-login-dst.json");
        assert_eq!(
            std::fs::read(&dst).unwrap(),
            b"REAL-LOGIN",
            "a pre-existing file must not be overwritten by ambient_login"
        );
    }

    #[test]
    fn fixed_strategy_uses_the_given_dir_and_is_not_ephemeral() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(temp.path().to_path_buf());

        let claude = Claude::new();
        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        let provisioned = provision(&claude, &spec, &templates).unwrap();

        assert_eq!(provisioned.dir, temp.path());
        assert!(!provisioned.ephemeral);
        assert!(provisioned.dir.exists());
    }

    #[test]
    fn ephemeral_strategy_creates_a_fresh_dir_under_state() {
        let spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        let claude = Claude::new();
        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        let provisioned = provision(&claude, &spec, &templates).unwrap();

        assert!(provisioned.ephemeral);
        assert!(provisioned.dir.exists());
        assert!(provisioned.dir.to_string_lossy().contains("runs"));

        // Cleanup: this test writes to the real state dir since it exercises
        // the ephemeral path; remove what we created.
        let _ = std::fs::remove_dir_all(&provisioned.dir);
    }

    /// Injection proof: a skill whose content comes from `Source::Files`
    /// (bytes, as a database-backed catalog would yield — no filesystem skill
    /// folder anywhere) still materializes into the run dir. This is the
    /// end-to-end evidence the storage abstraction works without the FS.
    #[test]
    fn provision_materializes_a_files_backed_skill_without_the_filesystem() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(crate::spec::SkillRef {
            id: "db-skill".to_string(),
            source: crate::source::Source::Files(vec![(
                PathBuf::from("SKILL.md"),
                b"---\nname: db-skill\n---\nfrom the database".to_vec(),
            )]),
        });

        let claude = Claude::new();
        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        provision(&claude, &spec, &templates).unwrap();

        let skill_md = config_dir.path().join("skills/db-skill/SKILL.md");
        assert!(
            skill_md.exists(),
            "a Source::Files skill must materialize into the run dir"
        );
        assert!(
            std::fs::read_to_string(&skill_md)
                .unwrap()
                .contains("from the database")
        );
    }

    /// Proves the `McpRef::InProcess` -> `McpRef::Inline` http injection
    /// works end to end without a real agent: provision Claude Code with an
    /// in-process MCP entry and check the generated `mcp.json` now names a
    /// `type: "http"` server pointed at the loopback server's URL.
    #[cfg(feature = "inproc-mcp")]
    #[test]
    fn provision_hosts_inprocess_mcp_and_rewrites_it_to_http() {
        use crate::mcp::{McpService, ToolDef};
        use crate::spec::{InProcessMcpHandle, McpRef};
        use std::sync::Arc;

        struct StubService;
        impl McpService for StubService {
            fn tools(&self) -> Vec<ToolDef> {
                vec![ToolDef {
                    name: "stub".to_string(),
                    description: "stub tool".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }]
            }
            fn call(
                &self,
                _name: &str,
                arguments: serde_json::Value,
            ) -> crate::Result<serde_json::Value> {
                Ok(arguments)
            }
        }

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.mcps.push(McpRef::InProcess(InProcessMcpHandle {
            name: "stub-tool".to_string(),
            service: Arc::new(StubService),
        }));

        let claude = Claude::new();
        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        let provisioned = provision(&claude, &spec, &templates).unwrap();
        assert_eq!(provisioned.inproc_servers.len(), 1);

        let mcp_json_path = provisioned.dir.join("mcp.json");
        let mcp_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        let entry = &mcp_json["mcpServers"]["stub-tool"];
        assert_eq!(entry["type"].as_str(), Some("http"));
        let url = entry["url"].as_str().expect("url present");
        assert!(url.starts_with("http://127.0.0.1:"), "url was: {url}");
        assert!(url.ends_with("/mcp"), "url was: {url}");
    }
}
