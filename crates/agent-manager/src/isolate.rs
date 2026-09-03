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
//! Three facts drive everything below:
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
//!
//! See `_docs/architecture.md` for where the isolate stage sits, `_docs/cli.md`
//! for the `--isolate[=profile]` / `--no-isolate` / `[isolate]` surface, and
//! `refs/isol8/_docs/integration.md` for the host-integration contract this
//! module implements.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow};

use crate::Result;
use crate::harness::Launch;
use crate::source::Source;
use crate::spec::{Isolation, RunSpec};

/// Where a confined run's `$HOME` comes from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HomeMode {
    /// A scratch home for this run alone, discarded with it.
    #[default]
    Ephemeral,
    /// A persistent home under the state dir, named by the caller, so a
    /// harness's caches and logins survive from one run to the next.
    Managed(String),
}

impl HomeMode {
    /// Parse the `[isolate] home` setting: `"ephemeral"` or `"managed"`.
    ///
    /// `"managed"` needs a name to key the home by, which is the caller's to
    /// supply — the settings file only chooses the mode.
    pub fn parse(value: &str, managed_id: &str) -> Result<Self> {
        match value {
            "ephemeral" => Ok(Self::Ephemeral),
            "managed" => Ok(Self::Managed(managed_id.to_string())),
            other => Err(anyhow!(
                "unknown [isolate] home {other:?}: expected \"ephemeral\" or \"managed\""
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
}

impl IsolateOptions {
    /// Options rooted at `state_dir`, with an ephemeral home and no extra
    /// grants.
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            home: HomeMode::Ephemeral,
            extra_ro: Vec::new(),
        }
    }
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

/// Resolve `launch` into the policy it runs under, or `None` when the run is
/// not isolated.
///
/// `dir` is the ephemeral config dir the provisioner populated; it is granted
/// read-write, along with the run's working directory. Everything the run
/// reaches *through* that dir — a skill's catalog folder, a profile's overlay,
/// an account's captured login — is granted read-only, because the
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
    base.add_dirs_rw = vec![path_string(dir), path_string(&run.cwd)];
    base.add_dirs_ro = read_only_grants(run, options);
    base.set_env = launch.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    // A harness asks the environment what it may draw, and isol8's allowlist
    // holds neither of these.
    base.env_pass = vec!["TERM".to_string(), "COLORTERM".to_string()];
    match &options.home {
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
    cfg.profile_paths = vec![path_string(&profiles)];
    if !layer.is_empty() {
        cfg.default_profiles.push(layer.clone());
    }

    let spec = isol8::resolve::spec_from_config(&cfg, base, cmd, &ctx)
        .context("resolving the isolation policy for this run")?;

    Ok(Some(Confined { spec, ctx }))
}

/// The policy an interactive **login capture** runs under.
///
/// A login is confined for the opposite reason a run is. A run is confined to keep the
/// harness away from the machine; a login is confined to keep it away from the *operating
/// system's keychain*, so that it writes the plaintext credential a capture can collect.
///
/// That indirection is not a preference. For Claude Code 2.1.218+ a relocated `$HOME` alone
/// leaves the keychain merely *unreachable* — no `~/Library/Keychains` — which that version
/// reports as an error rather than falling back to a file, so the capture gets nothing.
/// Denying the keychain at the policy layer does still take the clean file-fallback path.
///
/// So the layer set is chosen rather than discovered: `auto_profiles` is **off**, because
/// isol8 would otherwise match the harness's own layer on the command name and that layer
/// requires `integrations/keychain` — the one thing this policy exists to withhold. A
/// `layer` given by the caller joins the defaults, as it does in [`plan`].
///
/// `home` is the account's capture home: the login's `$HOME`, its only writable directory,
/// and where [`crate::account::AccountStore::capture_login`] reads the result from.
pub fn login_confined(
    home: &Path,
    plan: &crate::harness::LoginPlan,
    layer: Option<&str>,
    options: &IsolateOptions,
) -> Result<Confined> {
    let cmd = command_of(&plan.launch);
    let ctx = isol8::Context {
        real_home: isol8::home::real_home(),
        // A login belongs to an account rather than to a project, so the automatic
        // cwd grant is the capture home and no project folder is reachable at all.
        cwd: home.to_path_buf(),
        platform: isol8::Platform::current(),
        config_dir: options.state_dir.clone(),
        managed_root: options.state_dir.join("homes"),
    };

    let mut base = isol8::Spec::new(cmd.clone());
    base.add_dirs_rw = vec![path_string(home)];
    base.home = Some(path_string(home));
    base.set_env = plan
        .launch
        .env
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    base.env_pass = vec!["TERM".to_string(), "COLORTERM".to_string()];

    // A harness installed as a versioned payload execs out of its own store, which is
    // outside the capture home and so denied by default. Guarded by its own existence, so
    // this costs a harness that keeps no such directory nothing.
    if let Some(base_dirs) = directories::BaseDirs::new() {
        let versions = base_dirs.home_dir().join(".local/share/claude");
        if versions.is_dir() {
            base.add_dirs_ro.push(path_string(&versions));
        }
    }

    let profiles = options.state_dir.join("profiles");
    std::fs::create_dir_all(&profiles)
        .with_context(|| format!("creating the isolation profile root {}", profiles.display()))?;

    let mut cfg = isol8::Config::builtin_defaults();
    // Never the harness's own layer: it pulls in the keychain this policy withholds.
    cfg.auto_profiles = false;
    cfg.profile_paths = vec![path_string(&profiles)];
    match layer {
        Some(name) if !name.is_empty() => cfg.default_profiles.push(name.to_string()),
        _ => {
            if isol8::Platform::current() == isol8::Platform::Macos {
                // What the harness needs to open a browser and finish an OAuth flow, and
                // deliberately not `integrations/keychain`.
                cfg.default_profiles
                    .push("integrations/launch-services".to_string());
                cfg.default_profiles
                    .push("integrations/browser-native-messaging".to_string());
            }
        }
    }

    let spec = isol8::resolve::spec_from_config(&cfg, base, cmd, &ctx)
        .context("resolving the isolation policy for this login")?;

    Ok(Confined { spec, ctx })
}

/// The effective policy for `confined`, resolved and rendered but not applied
/// — the layer stack, the grants, the environment, the home plan and the
/// OS-native policy text. Writes nothing and spawns nothing, so it is what
/// `--print-config` and an embedder's audit log report.
pub fn describe(confined: &Confined) -> Result<isol8::DryRun> {
    isol8::sandbox::dry_run_in(&confined.spec, &confined.ctx)
        .context("rendering the isolation policy for this run")
}

/// The [`Launch`] that runs `confined`'s command under the OS policy, for a
/// caller that spawns argv itself — a host that owns a pseudo-terminal.
///
/// **This is a stopgap.** isol8 spawns with inherited stdio and keeps every
/// `SandboxChild` constructor private, so a host cannot hand it a terminal;
/// until it grows that seam (`refs/isol8-pty-seam-update.md`), macOS is served
/// by rendering the policy here and exec'ing `sandbox-exec` — which is what
/// isol8 itself does, and which `execve`s in place, so the harness is still one
/// process. Linux has no equivalent: Landlock is applied inside the target
/// process between `fork` and `exec`, and no rendered form of it exists to
/// hand anyone.
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

    if cfg!(target_os = "macos") {
        let policy = isol8::backends::select().render_policy(&effective.profile);
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

    Err(anyhow!(
        "isolating a run in a caller-owned terminal needs isol8's pseudo-terminal seam, \
         which this platform has no substitute for. Run without --isolate, or see \
         refs/isol8-pty-seam-update.md"
    ))
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

/// Every directory the run reads through the config dir rather than inside it.
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
    if let Some(login) = &run.account_login {
        push(login);
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

    // 1b. A login capture exists to make the harness write a plaintext credential, and it
    // only does that when the OS keychain is *denied* rather than merely absent. So the
    // resolved layer stack must not contain `integrations/keychain` — and the way it would
    // sneak back in is isol8 matching the harness's own layer on the command name, since
    // `agents/claude-code` requires it. Asserting on the resolved stack is what catches
    // both the direct and the transitive route.
    #[test]
    fn a_login_policy_never_resolves_the_keychain_layer() {
        let state = TempDir::new().expect("state dir");
        let home = TempDir::new().expect("capture home");
        let plan = crate::harness::LoginPlan {
            launch: Launch {
                // The name isol8 would match `agents/claude-code` on, which is exactly the
                // layer that would drag the keychain in.
                program: "claude".to_string(),
                args: vec!["auth".to_string(), "login".to_string()],
                env: vec![("HOME".to_string(), home.path().display().to_string())],
                env_remove: Vec::new(),
                env_clear: false,
            },
            credential_files: vec![PathBuf::from(".claude/.credentials.json")],
        };
        let options = IsolateOptions::new(state.path().to_path_buf());

        let confined = login_confined(home.path(), &plan, None, &options).expect("login policy");
        let resolved = describe(&confined).expect("rendering the login policy");

        let layers: Vec<&str> = resolved
            .layer_names
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(
            !layers.contains(&"integrations/keychain"),
            "a login policy must deny the keychain, got layers {layers:?}"
        );
        assert!(
            !layers.contains(&"agents/claude-code"),
            "auto-selection must be off, or the agent layer brings the keychain back: {layers:?}"
        );
        // The capture home is the login's own $HOME and its only writable directory.
        assert_eq!(
            confined.spec.home.as_deref(),
            Some(home.path().display().to_string().as_str())
        );
        assert_eq!(
            confined.spec.add_dirs_rw,
            vec![home.path().display().to_string()]
        );
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

    // 3. Every Source::Dir a run's skills, profile overlays and captured login
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
        let login_dir = PathBuf::from("/accounts/marco/home");

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
        run.account_login = Some(Source::Dir(login_dir.clone()));

        let options = IsolateOptions::new(PathBuf::from("/state"));
        let grants = read_only_grants(&run, &options);

        assert!(grants.contains(&skill_dir.display().to_string()));
        assert!(grants.contains(&profile_dir.display().to_string()));
        assert!(grants.contains(&login_dir.display().to_string()));
        assert_eq!(
            grants.len(),
            3,
            "a Source::Files entry and a duplicate Source::Dir must not add grants: {grants:?}"
        );
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

        assert_eq!(grants, vec![toolchain.display().to_string()]);
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
        assert_eq!(
            confined.spec.env_pass,
            vec!["TERM".to_string(), "COLORTERM".to_string()]
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

        assert_eq!(confined.spec.profiles.len(), 2);
        assert!(!confined.spec.profiles.contains(&String::new()));
    }

    // Off the one platform `confined_launch` actually serves, it must fail
    // loudly naming the pty seam it is waiting on rather than pretend to
    // confine the run. Not run end-to-end on macOS (it would exec
    // sandbox-exec); this only exercises the non-macOS early return, and is
    // unverified on this development machine since it is compiled out here.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn confined_launch_errors_on_platforms_without_a_native_seam() {
        let state = TempDir::new().expect("state dir");
        let cwd = TempDir::new().expect("cwd");
        let spec = isol8::Spec::new(vec!["true".to_string()]);
        let ctx = isol8::Context {
            real_home: cwd.path().to_path_buf(),
            cwd: cwd.path().to_path_buf(),
            platform: isol8::Platform::current(),
            config_dir: state.path().to_path_buf(),
            managed_root: state.path().join("homes"),
        };
        let confined = Confined { spec, ctx };

        let err = confined_launch(&confined).expect_err("non-macOS must not confine");
        assert!(err.to_string().contains("pseudo-terminal seam"));
    }
}
