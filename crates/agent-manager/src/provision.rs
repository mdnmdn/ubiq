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
use tracing::{debug, info, warn};

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
    /// The (created, populated) ephemeral config dir — under
    /// [`ConfigStrategy::Home`], the run's scratch dir.
    pub dir: PathBuf,
    /// How to launch the harness against `dir`.
    pub launch: Launch,
    /// True if `dir` is a throwaway the runner should delete on exit
    /// (`Ephemeral`); false if the user pinned it (`Fixed`, `Home`).
    pub ephemeral: bool,
    /// The profile's shared config home the harness runs from, when it runs
    /// from one ([`ConfigStrategy::Home`] with a harness that
    /// [`Harness::shares_home`]). Never removed by the run. `None` elsewhere.
    pub home: Option<PathBuf>,
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
            home: self.home.clone(),
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
///
/// [`ConfigStrategy::Home`] runs a harness that [`Harness::shares_home`] from
/// the profile's shared home (see `provision_shared_home`); any other harness
/// is provisioned into the run's scratch dir as under [`ConfigStrategy::Fixed`].
pub fn provision(
    harness: &dyn Harness,
    spec: &RunSpec,
    templates: &dyn TemplateStore,
) -> Result<Provisioned> {
    if let ConfigStrategy::Home { home, scratch } = &spec.config {
        if harness.shares_home() {
            return provision_shared_home(harness, spec, templates, home, scratch);
        }
        debug!(
            harness = %harness.id(),
            scratch = %scratch.display(),
            "harness cannot share a home; provisioning into the run's scratch dir"
        );
    }
    let (dir, ephemeral) = match &spec.config {
        ConfigStrategy::Fixed(path) => (path.clone(), false),
        ConfigStrategy::Home { scratch, .. } => (scratch.clone(), false),
        ConfigStrategy::Ephemeral => (new_run_dir()?, true),
    };

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating config dir {}", dir.display()))?;

    info!(
        dir = %dir.display(),
        harness = %harness.id(),
        strategy = if ephemeral { "ephemeral" } else { "fixed" },
        "provisioned run dir"
    );

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
            home: None,
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
            home: None,
            login_origin,
            resume: spec.resume.clone(),
            model: spec.model.clone(),
        })
    }
}

/// Provision a run against a profile's shared config `home` (`D193`).
///
/// The harness writes every per-run file into `scratch` and passes it by flag
/// ([`Harness::provision_home`]); `home` takes only the preference templates,
/// once, when this run is the one that creates it, and whatever
/// [`Harness::post_seed`] adds when missing. No login is seeded — the login
/// lives in the home and the harness owns it — so `login_origin` is `None`.
/// Neither dir is ephemeral: the scratch is kept for a resume, and the home is
/// never removed by a run.
fn provision_shared_home(
    harness: &dyn Harness,
    spec: &RunSpec,
    templates: &dyn TemplateStore,
    home: &Path,
    scratch: &Path,
) -> Result<Provisioned> {
    // Decided before anything creates it: a home already in use keeps what its
    // runs and its login left there.
    let new_home = std::fs::read_dir(home).map_or(true, |mut it| it.next().is_none());
    std::fs::create_dir_all(home)
        .with_context(|| format!("creating config home {}", home.display()))?;
    std::fs::create_dir_all(scratch)
        .with_context(|| format!("creating scratch dir {}", scratch.display()))?;
    info!(
        home = %home.display(),
        scratch = %scratch.display(),
        harness = %harness.id(),
        new_home,
        "provisioned run against a shared home"
    );
    if !spec.config_bases.is_empty() {
        // ponytail: the overlay links into a config dir, and a shared home is no
        // run's to link into. Carried per run only once it is folded into the
        // flag-passed files (`G377`).
        warn!(
            bases = spec.config_bases.len(),
            "profile config overlay is not applied under a shared home"
        );
    }

    #[cfg(feature = "inproc-mcp")]
    let (owned_spec, inproc_servers) = host_inproc_mcps(spec)?;
    #[cfg(feature = "inproc-mcp")]
    let effective_spec = &owned_spec;
    #[cfg(not(feature = "inproc-mcp"))]
    let effective_spec = spec;

    let launch = harness.provision_home(effective_spec, home, scratch)?;
    if new_home {
        crate::harness::apply_templates(home, &harness.id(), &harness.templates(), templates)?;
    }
    harness.post_seed(effective_spec, home)?;
    Ok(Provisioned {
        dir: scratch.to_path_buf(),
        launch,
        ephemeral: false,
        home: Some(home.to_path_buf()),
        login_origin: None,
        resume: spec.resume.clone(),
        model: spec.model.clone(),
        #[cfg(feature = "inproc-mcp")]
        inproc_servers,
    })
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
    // Only a *credential* counts: the identity companions (Claude's
    // `.claude.json`) are onboarding state, and letting one of those stand in
    // for a login is how a keychain-only machine ends up with an authenticated
    // identity and no token.
    if anchor
        .login_seed
        .iter()
        .any(|s| s.credential && dir.join(&s.dst).exists())
    {
        info!("zero-config login skipped: a credential already landed in the run dir");
        return Ok(None);
    }
    // Env/key/helper accounts manage their own auth; don't seed a stale OAuth login.
    if let Some(acct) = &spec.account
        && (acct.api_key_env.is_some() || acct.auth_token_env.is_some() || acct.helper.is_some())
    {
        info!("zero-config login skipped: account supplies env/key/helper credentials");
        return Ok(None);
    }
    // Tier 1: a real file under the real HOME wins — it's what the harness
    // itself would read.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let home_src = Source::Dir(home.clone());
        crate::harness::seed_login(dir, &home_src, &anchor.login_seed)?;
        // Again, only the credential settles it. On macOS Claude Code keeps its
        // session in the Keychain, so `~/.claude/.credentials.json` is absent
        // while `~/.claude.json` is not: counting the companion here would
        // return before tier 2 ever reads the Keychain.
        if let Some(seed) = anchor
            .login_seed
            .iter()
            .find(|s| s.credential && dir.join(&s.dst).exists())
        {
            let digest = std::fs::read(dir.join(&seed.dst))
                .map(|b| crate::credentials::login_digest(&b))
                .unwrap_or_default();
            info!(
                tier = "A",
                source = %home.display(),
                digest = %digest,
                "zero-config login seeded from real HOME"
            );
            return Ok(Some(home_src));
        }
    }
    // Tier 2: no credential landed — ask the harness for its own account of the
    // live login (e.g. Claude Code's OS-Keychain session).
    if let Some(ambient) = harness.ambient_login() {
        crate::harness::seed_login(dir, &ambient, &anchor.login_seed)?;
        let digest = anchor
            .login_seed
            .iter()
            .find(|s| s.credential)
            .and_then(|seed| std::fs::read(dir.join(&seed.dst)).ok())
            .map(|b| crate::credentials::login_digest(&b))
            .unwrap_or_default();
        info!(
            tier = "B",
            digest = %digest,
            "zero-config login seeded from harness ambient_login (e.g. macOS Keychain)"
        );
        return Ok(Some(ambient));
    }
    info!("zero-config login produced no login for this run");
    Ok(None)
}

/// Which [`Source`] an *account's* login was seeded from, when one was.
///
/// That seeding happens inside each harness's own `provision` (from
/// `spec.account_login`, else the account's `home`), so it is recognised here
/// by its result: the login files are already in `dir` before the zero-config
/// fallback runs. A profile overlay can place the same files and names no
/// origin — those yield `None` rather than a guess at the account's. Only a
/// [`SeedFile::credential`] counts, since the credential is the only thing
/// [`crate::harness::harvest_login`] ever writes back.
fn account_login_origin(harness: &dyn Harness, spec: &RunSpec, dir: &Path) -> Option<Source> {
    let anchor = harness.config_anchor();
    if !anchor
        .login_seed
        .iter()
        .any(|s| s.credential && dir.join(&s.dst).exists())
    {
        return None;
    }
    let origin = spec
        .account_login
        .clone()
        .or_else(|| Some(Source::Dir(spec.account.as_ref()?.home.clone()?)));
    if let Some(origin) = &origin {
        let account = spec.account.as_ref().map(|a| a.id.as_str()).unwrap_or("");
        info!(account = %account, origin = ?origin, "account login origin resolved");
    }
    origin
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

    let run_dir = base.join(run_id);
    debug!(dir = %run_dir.display(), "new ephemeral run dir");
    Ok(run_dir)
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
            login_seed: vec![
                crate::harness::SeedFile::credential(
                    "am-test-ambient-login-src-2f0c1e6a.json",
                    "ambient-login-dst.json",
                ),
                // The identity companion, mirroring Claude's `.claude.json`:
                // seeded alongside the credential but never a login by itself.
                crate::harness::SeedFile::new(
                    "am-test-ambient-identity-src-2f0c1e6a.json",
                    "ambient-identity-dst.json",
                ),
            ],
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

    /// The regression: only the login's *identity companion* landed (tier 1
    /// copied `~/.claude.json`, because the credential itself lives in the
    /// macOS Keychain and no `~/.claude/.credentials.json` exists). That is
    /// not a login, so tier 2 must still be consulted.
    #[test]
    fn seed_zero_config_login_still_asks_ambient_when_only_the_identity_landed() {
        let config_dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            config_dir.path().join("ambient-identity-dst.json"),
            b"IDENTITY-ONLY",
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
            b"AMBIENT-LOGIN",
            "an identity companion must not stand in for the credential"
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

    fn http_mcp(id: &str, url: &str) -> crate::spec::McpRef {
        crate::spec::McpRef::Inline(crate::config::McpServer {
            id: id.to_string(),
            transport: crate::config::McpTransport::Http,
            command: None,
            args: Vec::new(),
            env: Default::default(),
            url: Some(url.to_string()),
            headers: Default::default(),
        })
    }

    fn home_spec(home: &Path, scratch: &Path, cwd: &str) -> RunSpec {
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from(cwd));
        spec.config = ConfigStrategy::Home {
            home: home.to_path_buf(),
            scratch: scratch.to_path_buf(),
        };
        spec
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// `D193`: a shared home takes the templates and the trust entry and nothing else — no
    /// per-run file, and no login seeded from an account, whatever the spec names.
    #[test]
    fn home_strategy_writes_only_templates_and_trust_into_a_new_home() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        let scratch = root.path().join("scratch");
        let account_home = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(account_home.path().join(".claude")).unwrap();
        std::fs::write(
            account_home.path().join(".claude/.credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"tok"}}"#,
        )
        .unwrap();

        let mut spec = home_spec(&home, &scratch, "/tmp/project");
        spec.mcps.push(http_mcp("docs", "https://example.com/mcp/"));
        spec.policy = Some(crate::spec::Policy {
            permission_mode: Some("plan".to_string()),
            ..Default::default()
        });
        spec.initial = Some(crate::spec::Instructions {
            instructions: Some("REMEMBER ME".to_string()),
            prompt: None,
        });
        spec.account = Some(crate::account::Account {
            id: "acct".to_string(),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });

        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        let provisioned = provision(&Claude::new(), &spec, &templates).unwrap();

        assert_eq!(provisioned.dir, scratch);
        assert_eq!(provisioned.home.as_deref(), Some(home.as_path()));
        assert!(!provisioned.ephemeral, "no cleanup path may reach the home");
        assert!(provisioned.login_origin.is_none());
        assert_eq!(
            names_in(&home),
            vec![".claude.json", ".claude.json.am-lock", "settings.json"]
        );
        let home_settings: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("settings.json")).unwrap())
                .unwrap();
        assert!(home_settings.get("theme").is_some());
        assert!(home_settings.get("permissions").is_none());
        let claude_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap())
                .unwrap();
        assert_eq!(
            claude_json["projects"]["/tmp/project"]["hasTrustDialogAccepted"].as_bool(),
            Some(true)
        );
        assert_eq!(names_in(&scratch), vec!["mcp.json", "settings.json"]);
    }

    /// A home somebody already logged into or ran from is not "new": its settings are its
    /// user's, and the templates stay out of it.
    #[test]
    fn home_strategy_leaves_the_templates_out_of_a_home_in_use() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        let scratch = root.path().join("scratch");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"hasCompletedOnboarding":true}"#,
        )
        .unwrap();

        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        provision(
            &Claude::new(),
            &home_spec(&home, &scratch, "/tmp/project"),
            &templates,
        )
        .unwrap();

        assert!(!home.join("settings.json").exists());
        let claude_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap())
                .unwrap();
        assert!(claude_json.get("claudeInChromeDefaultEnabled").is_none());
    }

    /// Two runs of one profile share its home but not their MCP servers: each run's URL (Ubiq's
    /// embed the run key) stays in its own scratch dir.
    #[test]
    fn two_runs_in_one_home_keep_their_mcps_apart() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        let (a, b) = (root.path().join("run-a"), root.path().join("run-b"));
        let mut spec_a = home_spec(&home, &a, "/tmp/a");
        spec_a
            .mcps
            .push(http_mcp("ubiq", "http://127.0.0.1:1/run-a"));
        let mut spec_b = home_spec(&home, &b, "/tmp/b");
        spec_b
            .mcps
            .push(http_mcp("ubiq", "http://127.0.0.1:1/run-b"));

        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        let first = provision(&Claude::new(), &spec_a, &templates).unwrap();
        let second = provision(&Claude::new(), &spec_b, &templates).unwrap();

        for (dir, own, other) in [(&a, "run-a", "run-b"), (&b, "run-b", "run-a")] {
            let mcp = std::fs::read_to_string(dir.join("mcp.json")).unwrap();
            assert!(mcp.contains(own) && !mcp.contains(other), "{mcp}");
        }
        assert!(!home.join("mcp.json").exists());
        for launch in [&first.launch, &second.launch] {
            assert!(
                launch
                    .env
                    .iter()
                    .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &home.display().to_string())
            );
        }
        let claude_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap())
                .unwrap();
        assert!(claude_json["projects"]["/tmp/a"].is_object());
        assert!(claude_json["projects"]["/tmp/b"].is_object());
    }

    /// `claude-code-acp` takes no argv, so it cannot share a home: it is provisioned into the
    /// run's scratch dir exactly as a fixed dir would be, and the home is never touched.
    #[test]
    fn home_strategy_falls_back_to_scratch_for_a_harness_that_cannot_share() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        let scratch = root.path().join("scratch");
        let mut spec = home_spec(&home, &scratch, ".");
        spec.harness = "claude-code-acp".to_string();

        let tmpl_dir = tempfile::TempDir::new().unwrap();
        let templates = crate::harness::FsTemplateStore::new(tmpl_dir.path());
        let provisioned = provision(&Claude::new_acp(), &spec, &templates).unwrap();

        assert_eq!(provisioned.dir, scratch);
        assert!(provisioned.home.is_none());
        assert!(!provisioned.ephemeral);
        assert!(scratch.join("mcp.json").is_file());
        assert!(!home.exists());
        assert!(
            provisioned
                .launch
                .env
                .iter()
                .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &scratch.display().to_string())
        );
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
