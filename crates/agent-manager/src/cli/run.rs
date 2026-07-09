//! `am <harness> [flags] [-- passthrough…]` — the run path.
//!
//! Wires the whole `resolve → RunSpec → provision → run` spine together for
//! one CLI invocation: parse flags, build settings + a catalog, resolve a
//! [`RunSpec`], provision it for the chosen harness, and either print the
//! result (`--print-config`) or hand it to [`crate::run::run`] and exit with
//! the child's own exit code.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;

use crate::account::{resolve_accounts_root, AccountStore, EmptyAccountStore, FsAccountStore};
use crate::harness::Harness;
use crate::provision;
use crate::registry::{resolve_catalog_root, FsRegistry, OverlayRegistry};
use crate::resolve::{resolve, RunFlags};
use crate::settings::{self, Settings};

use super::{parse_io_mode, parse_output_mode, split_passthrough, OutputMode, RunArgs};

/// Run `harness` against the remaining argv (everything after the harness
/// name on the command line).
pub(super) fn run_harness(harness: &dyn Harness, args: &[String]) -> Result<()> {
    let (before, passthrough_args) = split_passthrough(args);

    let run_args = RunArgs::try_parse_from(std::iter::once("am-run".to_string()).chain(before))?;

    let cwd = std::env::current_dir()?;

    let settings = load_settings(&run_args, &cwd)?;

    let catalog_root = resolve_catalog_root(
        run_args
            .catalog
            .clone()
            .or_else(|| settings.catalog.clone().map(PathBuf::from)),
    )
    .unwrap_or_else(|| cwd.join(".agent-manager-catalog-unset"));
    let global = FsRegistry::new(&catalog_root);

    let instructions = match &run_args.instructions {
        Some(p) => Some(std::fs::read_to_string(p).with_context(|| format!("reading instructions file {}", p.display()))?),
        None => None,
    };

    let flags = RunFlags {
        harness: harness.id(),
        mcps: run_args.mcps.clone(),
        skills: run_args.skills.clone(),
        mcp_json: run_args.mcp_json.clone(),
        account: run_args.account.clone(),
        hooks: run_args.hooks.clone(),
        safe: run_args.safe,
        instructions,
        prompt: run_args.prompt.clone(),
        passthrough_args,
        cwd: cwd.clone(),
        isolate: run_args.isolate.clone(),
        resume: run_args.resume.clone(),
        mcp_as_skill: run_args.mcp_as_skill.clone(),
    };

    let accounts = build_account_store();

    let mut spec = match find_project_catalog(&cwd) {
        Some(project_root) => {
            let project = FsRegistry::new(project_root);
            let overlay = OverlayRegistry::new(global, Some(project));
            resolve(&flags, &settings, &overlay, accounts.as_ref())?
        }
        None => resolve(&flags, &settings, &global, accounts.as_ref())?,
    };

    spec.io = parse_io_mode(run_args.io.as_deref())?;

    let mut provisioned = provision::provision(harness, &spec)?;

    if !matches!(spec.isolation, crate::spec::Isolation::None) {
        let template = crate::isolate::IsolateTemplate {
            command: settings
                .isolate
                .command
                .clone()
                .unwrap_or_else(|| crate::isolate::IsolateTemplate::default().command),
        };
        provisioned.launch =
            crate::isolate::wrap_launch(&provisioned.launch, &spec.isolation, &template);
    }

    if run_args.print_config {
        print_config(&provisioned.dir, &provisioned.launch, run_args.keep_config);
        return Ok(());
    }

    let sessions_root = crate::session::sessions_root(None);

    // `--output` only matters for `--io structured`; keep it ungated from
    // parsing (and any bogus-value error) in passthrough mode, matching the
    // pre-refactor behavior where `parse_output_mode` was only ever called
    // inside the structured branch.
    let output = if spec.io == crate::spec::IoModes::Structured {
        parse_output_mode(run_args.output.as_deref())?
    } else {
        OutputMode::Events
    };

    run_provisioned(
        harness,
        &spec,
        &provisioned,
        &cwd,
        output,
        sessions_root,
        run_args.keep_config,
    )
}

/// The "provision is done → run it" tail shared by [`run_harness`] and
/// `crate::cli::session::resume`: build a [`crate::session::SessionMeta`],
/// dispatch to the structured or passthrough path based on `spec.io`, and
/// (passthrough only) exit the process with the child's own exit code.
///
/// `output` is only consulted for `--io structured`. `keep_config` is only
/// consulted for passthrough (structured runs never delete their dir); when
/// a session recorder actually starts, the config dir is retained regardless
/// of `keep_config` — a resume needs the retained dir to point the harness
/// back at.
pub(super) fn run_provisioned(
    harness: &dyn Harness,
    spec: &crate::spec::RunSpec,
    provisioned: &provision::Provisioned,
    cwd: &Path,
    output: OutputMode,
    sessions_root: Option<PathBuf>,
    keep_config: bool,
) -> Result<()> {
    let io_label = if spec.io == crate::spec::IoModes::Structured {
        "structured"
    } else {
        "passthrough"
    };
    let meta = crate::session::SessionMeta::new(
        spec.harness.clone(),
        cwd.to_path_buf(),
        launch_argv(&provisioned.launch),
        spec.account.as_ref().map(|a| a.id.clone()),
        io_label.to_string(),
        provisioned.dir.clone(),
    );

    if spec.io == crate::spec::IoModes::Structured {
        return run_structured(harness, spec, provisioned, cwd, output, sessions_root, meta);
    }

    // Passthrough: record metadata-only (no transcript events), and finish
    // the recorder BEFORE `std::process::exit` — that call skips destructors,
    // so `finish`'s write of `finished_at`/`exit_code` must happen explicitly
    // first. Recording is best-effort: a missing/unwritable sessions root
    // must never fail or alter the run's own outcome.
    let recorder = sessions_root.and_then(|root| crate::session::start(&root, meta).ok());

    // Resume needs the retained config dir to point the harness back at, so
    // once a session is actually being recorded, keep the dir even if
    // `--keep-config` wasn't passed. (Fixed dirs — e.g. a resume's own
    // re-provision — are never deleted regardless; see `run::cleanup`'s
    // `ephemeral` check.)
    let keep_config = keep_config || recorder.is_some();

    let code = crate::run::run(provisioned, cwd, keep_config)?;

    if let Some(recorder) = recorder {
        let _ = recorder.finish(Some(code));
    }

    std::process::exit(code);
}

/// The launch program + args actually run, as recorded in [`crate::session::SessionMeta::argv`].
fn launch_argv(launch: &crate::harness::Launch) -> Vec<String> {
    std::iter::once(launch.program.clone())
        .chain(launch.args.iter().cloned())
        .collect()
}

/// The `--io structured` path: build a structured bridge, optionally send
/// the seeded initial prompt, then drain events on stdout as NDJSON,
/// projected through `output` (see [`OutputMode`]).
///
/// This is a framework stub — real per-harness bridges land in C2/C3/C4, so
/// today every harness's `structured_bridge` bails with a clear "not
/// supported yet" error via [`Harness::structured_bridge`]'s default impl.
///
/// When `sessions_root` is available, records every drained event to
/// `meta`'s session transcript and finishes the recorder with `Some(0)` once
/// the stream drains cleanly (structured runs have no child exit code
/// surfaced today; an error propagating out of this function simply leaves
/// the recorder unfinished rather than failing the run over recording).
fn run_structured(
    harness: &dyn Harness,
    spec: &crate::spec::RunSpec,
    provisioned: &provision::Provisioned,
    cwd: &Path,
    output: OutputMode,
    sessions_root: Option<PathBuf>,
    meta: crate::session::SessionMeta,
) -> Result<()> {
    if !harness.io_support().structured {
        anyhow::bail!(
            "harness '{}' does not support --io structured (yet)",
            harness.id()
        );
    }

    let mut recorder = sessions_root.and_then(|root| crate::session::start(&root, meta).ok());

    let mut bridge = harness.structured_bridge(provisioned, cwd)?;

    if let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
        bridge.send(crate::io::AgentInput::Prompt {
            text: prompt.clone(),
        })?;
    }

    while let Some(ev) = bridge.next_event()? {
        if let Some(recorder) = recorder.as_mut() {
            let _ = recorder.record_event(&ev);
        }

        let line = match output {
            OutputMode::Events => Some(serde_json::to_string(&ev)?),
            OutputMode::Acp => crate::io::to_acp(&ev)
                .map(|v| serde_json::to_string(&v))
                .transpose()?,
            OutputMode::AgUi => crate::io::to_agui(&ev)
                .map(|v| serde_json::to_string(&v))
                .transpose()?,
        };
        if let Some(line) = line {
            println!("{line}");
        }
    }

    if let Some(recorder) = recorder {
        let _ = recorder.finish(Some(0));
    }

    Ok(())
}

/// Build the account store from the default accounts root (`--accounts` has
/// no CLI flag yet; this honors `AM_ACCOUNTS` / the default location only).
/// Falls back to an empty store when no accounts root exists, so accountless
/// runs are unaffected.
fn build_account_store() -> Box<dyn AccountStore> {
    match resolve_accounts_root(None) {
        Some(root) if root.is_dir() => Box::new(FsAccountStore::new(root)),
        _ => Box::new(EmptyAccountStore),
    }
}

/// Load settings from `--config`, else discover from `cwd`, else defaults.
fn load_settings(run_args: &RunArgs, cwd: &Path) -> Result<Settings> {
    if let Some(path) = &run_args.config {
        settings::load(path)
    } else {
        Ok(settings::discover(cwd)?
            .map(|(s, _)| s)
            .unwrap_or_default())
    }
}

/// Look for a project-local catalog overlay (`<dir>/.agent-manager/catalog`),
/// walking up from `cwd` to the git root (mirrors settings discovery).
fn find_project_catalog(cwd: &Path) -> Option<PathBuf> {
    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        let candidate = dir.join(".agent-manager").join("catalog");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if dir.join(".git").exists() {
            break;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    None
}

/// Print the provisioned config dir, launch argv, and env — the
/// `--print-config` output.
fn print_config(dir: &Path, launch: &crate::harness::Launch, keep_config: bool) {
    println!("config dir: {}", dir.display());
    println!(
        "argv: {} {}",
        launch.program,
        launch.args.join(" ")
    );
    println!("env:");
    for (k, v) in &launch.env {
        println!("  {k}={v}");
    }
    println!("env_remove: {}", launch.env_remove.join(", "));
    println!("keep_config: {keep_config}");
}
