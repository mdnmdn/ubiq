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

use super::{parse_io_mode, split_passthrough, RunArgs};

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
        safe: run_args.safe,
        instructions,
        prompt: run_args.prompt.clone(),
        passthrough_args,
        cwd: cwd.clone(),
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

    let provisioned = provision::provision(harness, &spec)?;

    if run_args.print_config {
        print_config(&provisioned.dir, &provisioned.launch, run_args.keep_config);
        return Ok(());
    }

    if spec.io == crate::spec::IoModes::Structured {
        return run_structured(harness, &spec, &provisioned, &cwd);
    }

    let code = crate::run::run(&provisioned, &cwd, run_args.keep_config)?;
    std::process::exit(code);
}

/// The `--io structured` path: build a structured bridge, optionally send
/// the seeded initial prompt, then drain events as NDJSON on stdout.
///
/// This is a framework stub — real per-harness bridges land in C2/C3/C4, so
/// today every harness's `structured_bridge` bails with a clear "not
/// supported yet" error via [`Harness::structured_bridge`]'s default impl.
fn run_structured(
    harness: &dyn Harness,
    spec: &crate::spec::RunSpec,
    provisioned: &provision::Provisioned,
    cwd: &Path,
) -> Result<()> {
    if !harness.io_support().structured {
        anyhow::bail!(
            "harness '{}' does not support --io structured (yet)",
            harness.id()
        );
    }

    let mut bridge = harness.structured_bridge(provisioned, cwd)?;

    if let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
        bridge.send(crate::io::AgentInput::Prompt {
            text: prompt.clone(),
        })?;
    }

    while let Some(ev) = bridge.next_event()? {
        println!("{}", serde_json::to_string(&ev)?);
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
