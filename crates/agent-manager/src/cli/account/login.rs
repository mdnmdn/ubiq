use super::*;

/// `am account login <id> --harness <h>`: interactively log `harness_key`
/// into a persistent per-account home dir, verify the resulting credential
/// file landed on disk, and record `home` on the account so
/// `am <h> --account <id>` reuses it.
///
/// The stored account id is the bare `id` the caller typed — `am account
/// login mdn --harness claude` and `am account login mdn --harness copilot`
/// share **one** home dir (`accounts/mdn/`) and **one** account-store entry.
/// This is safe, not a collision: each harness's [`crate::harness::Harness::
/// config_anchor`] seeds from its own harness-specific relative subpath
/// under `home` (e.g. Claude's `.claude/.credentials.json` vs Copilot's
/// `config.json`), so two harnesses' captured logins coexist in the same
/// home without overwriting each other — exactly like a real `$HOME` holds
/// `.claude/`, `.copilot/`, `.codex/` side by side today. Which harnesses a
/// given account actually has an effective login for is never stored
/// separately (nothing to let drift out of sync); it's derived at display
/// time by [`effective_harnesses`], which checks each harness's primary
/// credential file for real on disk. `am` never parses or copies the
/// credential file's contents — it only points the harness's own credential
/// store at the capture home and checks that the harness wrote *something*
/// there.
///
/// Isolation decision (`isolate` = the `--isolate[=profile]` flag, `no_isolate`
/// = `--no-isolate`):
/// - `--no-isolate` always runs the plain (non-isolated) capture: HOME is
///   relocated to `home` and the harness is launched through the ordinary PTY
///   runner ([`crate::run::run`]), relying on the harness falling back to a
///   plaintext credential file when it can't reach the OS keychain.
/// - `--isolate` / `--isolate=<profile>` always run the capture inside an isol8
///   sandbox that denies keychain access at the sandbox layer (see
///   [`run_login_isolated`]).
/// - Neither flag: **macOS defaults to the sandbox** (bare-`--isolate`
///   behavior), because as of Claude Code 2.1.218 a merely-relocated HOME no
///   longer forces the plaintext fallback there; every other OS defaults to the
///   plain path, where the keychain-denial problem doesn't arise.
pub(super) fn cmd_login(
    id: &str,
    harness_key: &str,
    isolate: Option<Option<String>>,
    no_isolate: bool,
) -> Result<()> {
    // Fold the two flags + the per-OS default into the existing
    // `Option<Option<String>>` shape the launch branch below already handles
    // (`Some(_)` = sandbox, `None` = plain PTY path).
    let isolate: Option<Option<String>> = if no_isolate {
        None
    } else {
        match isolate {
            Some(profile) => Some(profile),
            None if cfg!(target_os = "macos") => Some(None),
            None => None,
        }
    };
    let root = account::resolve_accounts_root(None)
        .ok_or_else(|| anyhow!("no accounts root; set AM_ACCOUNTS"))?;
    // Route the capture through the store trait: `login_home` gives a real dir
    // to log into (the persistent per-account home for the filesystem store),
    // and `capture_login` below persists the result — so a database-backed
    // store captures the same way without any CLI change.
    let store = FsAccountStore::new(&root);
    let home = store.login_home(id)?;

    let harness = crate::harness::resolve(harness_key).ok_or_else(|| {
        anyhow!(
            "unknown harness '{harness_key}'; known: {}",
            crate::harness::known_ids().join(", ")
        )
    })?;

    let plan = harness.login(&home)?;

    // Record the primary credential file's mtime *before* launching login, so
    // a harness that exits 0 without actually writing fresh credentials (e.g.
    // Claude Code aborting the persist step after a keychain-unreachable
    // error, but still completing the rest of the OAuth flow) can't leave a
    // stale pre-existing file behind and be reported as a success. This
    // verification is shared by both the isolated and non-isolated launch
    // paths below — only the launch mechanism differs.
    let primary = home.join(&plan.credential_files[0]);
    let mtime_before = std::fs::metadata(&primary).and_then(|m| m.modified()).ok();

    let code = if let Some(profile) = isolate {
        // Sandboxed path: keychain access is denied cleanly at the sandbox
        // layer (no relocated-HOME keychain-lookup error to explain), so skip
        // the non-isolated path's "expect a keychain error" note below.
        run_login_isolated(&home, &plan, profile)?
    } else {
        let provisioned = crate::provision::Provisioned {
            dir: home.clone(),
            launch: plan.launch.clone(),
            ephemeral: false, // persistent home — never auto-deleted
            #[cfg(feature = "inproc-mcp")]
            inproc_servers: Vec::new(),
        };
        // Login capture relocates HOME to `home` (a bare dir with no
        // ~/Library/Keychains) precisely so the harness can't reach the OS
        // keychain and falls back to writing a portable credential file instead
        // — see the `login()` docs on each harness. On macOS that fallback is
        // preceded by a keychain-lookup error printed straight to the terminal;
        // it's expected and harmless, so flag it before it appears rather than
        // let it read as a failure.
        //
        // NOTE: as of Claude Code 2.1.218 this fallback is broken — with a
        // relocated HOME, macOS `securityd` is reachable but finds no
        // default/login keychain, Claude Code reports "A keychain cannot be
        // found to store '<user>'" and does NOT fall back to a file (OAuth
        // still completes, nothing is persisted). Pass `--isolate` to use the
        // sandboxed path instead, which does trigger the clean file fallback.
        #[cfg(target_os = "macos")]
        println!(
            "note: macOS may print \"A keychain cannot be found to store '{}'\" below — that's \
             expected, am relocates HOME during capture so credentials land in a portable file \
             instead of your system keychain (if nothing gets captured despite exit 0, retry \
             with --isolate)",
            std::env::var("USER").unwrap_or_else(|_| "you".to_string())
        );
        let cwd = std::env::current_dir()?;
        // A login capture runs unconfined here; `--isolate` on `am account
        // login` takes the `isol8::Sandbox` path below instead.
        crate::run::run(&provisioned, &cwd, true, None)? // keep_config: persistent
    };
    if code != 0 {
        bail!("harness login exited with code {code}; no account recorded");
    }

    let mtime_after = std::fs::metadata(&primary).and_then(|m| m.modified()).ok();
    match (mtime_before, mtime_after) {
        (_, None) => bail!(
            "login did not produce a credential file at {}",
            primary.display()
        ),
        (Some(before), Some(after)) if after <= before => bail!(
            "login exited successfully but did not refresh the credential file at {} \
             (mtime unchanged since before this run) — a stale credential was left in \
             place. On macOS this usually means the OS keychain was unreachable (relocated \
             HOME has no ~/Library/Keychains) and Claude Code aborted persisting the new \
             token instead of falling back to plaintext; rerun and check for a keychain \
             error, or delete {} and try again",
            primary.display(),
            home.display()
        ),
        _ => {}
    }

    store.capture_login(id, &home, &plan.credential_files)?;

    println!("captured credential file(s):");
    for rel in &plan.credential_files {
        let full = home.join(rel);
        if full.exists() {
            println!("  {}", full.display());
        }
    }
    println!("account '{id}' captured ({})", home.display());
    let captured = effective_harnesses(&home);
    if captured.len() > 1 {
        println!(
            "note: '{id}' now has effective logins for multiple harnesses ({}) — they share \
             this home dir but don't share credentials, each harness only reads its own \
             subpath",
            captured.join(", ")
        );
    }
    println!("reuse with: am {harness_key} --account {id}");

    Ok(())
}

/// Run `plan.launch` (program `claude`, args `["auth", "login"]`) inside an
/// isol8 deny-by-default sandbox, blocking until it exits, and return its
/// exit code. Used by [`cmd_login`] when `--isolate` is passed.
///
/// This is the fix for a Claude Code 2.1.218+ regression: the non-isolated
/// capture path relocates `HOME` to a bare directory (no
/// `~/Library/Keychains`) so the harness's OS-keychain write fails and it
/// falls back to writing a plaintext credential file — but 2.1.218 changed
/// that failure mode. With a relocated HOME, macOS `securityd` is still
/// reachable, just finds no default/login keychain there; Claude Code
/// reports "A keychain cannot be found to store '<user>'" and does NOT fall
/// back to a file (OAuth still completes with exit 0, but nothing is
/// persisted). Empirically, when keychain access is instead *denied at the
/// sandbox layer* (rather than the keychain being merely *missing*), Claude
/// Code DOES take its clean file-fallback path. Hence: sandbox the process
/// instead of relocating HOME.
///
/// Profile selection: `Some(name)` (`--isolate=<profile>`) uses that single
/// named isol8 profile verbatim (an explicit override). `None` (bare
/// `--isolate`) composes the layers the harness needs to run *normally* with
/// only the OS keychain denied — the point isn't to lock the process down but
/// to make Claude Code's keychain write fail cleanly (access-denied, not the
/// "no default keychain" hard error a relocated HOME produces) so it takes its
/// plaintext-file fallback. On macOS that composition is `macos/system-runtime`
/// (process-exec/fork, tty, open network) + `integrations/launch-services` and
/// `integrations/browser-native-messaging` (open the OAuth browser) — exactly
/// the `agents/claude-code` layer set WITHOUT `integrations/keychain`, which
/// that agent profile `requires` and so can't simply be subtracted from.
///
/// Grants: `home` is replaced (`.home`) and granted read-write (`.grant_rw`)
/// so `claude` writes `<home>/.claude/.credentials.json`; isol8's
/// `confine_executable` auto-grants read+exec on the resolved `claude` binary,
/// but the `~/.local/bin/claude` launcher symlinks into the real home's
/// `~/.local/share/claude/versions/<v>` (a self-contained native binary), which
/// HOME replacement doesn't relocate — so that real runtime tree is granted
/// read-only by absolute path below.
///
/// CAVEAT (not yet exercised against a live `sandbox-exec`): the grant/layer
/// set is a considered starting point, not an interactively-validated one. If
/// the OAuth browser fails to open or `claude` can't start, diagnose the
/// missing grant with `isol8 @diag claude` and either widen the composition
/// here or pass an explicit `--isolate=<profile>`.
fn run_login_isolated(
    home: &Path,
    plan: &crate::harness::LoginPlan,
    profile: Option<String>,
) -> Result<i32> {
    let home_str = home.to_string_lossy().into_owned();

    let mut sandbox = isol8::Sandbox::new()
        .home(home_str.clone())
        .grant_rw(home_str);

    match profile {
        // Explicit override: a single named isol8 profile.
        Some(name) => {
            sandbox = sandbox.profile(name);
        }
        // Default: the harness's normal layer set minus the keychain layer.
        None => {
            #[cfg(target_os = "macos")]
            {
                sandbox = sandbox
                    .profile("macos/system-runtime")
                    .profile("integrations/launch-services")
                    .profile("integrations/browser-native-messaging");
                // Deliberately NOT `integrations/keychain` — its absence is
                // what forces the plaintext-file fallback this path captures.
            }
            #[cfg(not(target_os = "macos"))]
            {
                // The keychain-denial scenario is macOS-specific; elsewhere a
                // bare `--isolate` just runs under the minimal base layer.
                sandbox = sandbox.profile("base");
            }
        }
    }

    if let Some(base_dirs) = directories::BaseDirs::new() {
        let versions_dir = base_dirs.home_dir().join(".local/share/claude");
        if versions_dir.is_dir() {
            sandbox = sandbox.grant_ro(versions_dir.to_string_lossy().into_owned());
        }
    }

    let mut argv = vec![plan.launch.program.clone()];
    argv.extend(plan.launch.args.iter().cloned());

    sandbox
        .run(argv)
        .map_err(|e| anyhow!("isol8 sandbox run failed: {e}"))
}
