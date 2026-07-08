//! `am <harness> [flags] [-- passthrough…]` — the run path.
//!
//! Wires the whole `resolve → RunSpec → provision → run` spine together for
//! one CLI invocation: parse flags, build settings + a catalog, resolve a
//! [`RunSpec`], provision it for the chosen harness, and either print the
//! result (`--print-config`) or hand it to [`crate::run::run`] and exit with
//! the child's own exit code.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;

use crate::harness::Harness;
use crate::provision;
use crate::registry::{resolve_catalog_root, FsRegistry, OverlayRegistry};
use crate::resolve::{resolve, RunFlags};
use crate::settings::{self, Settings};

use super::{split_passthrough, RunArgs};

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

    let flags = RunFlags {
        harness: harness.id(),
        mcps: run_args.mcps.clone(),
        skills: run_args.skills.clone(),
        mcp_json: run_args.mcp_json.clone(),
        account: run_args.account.clone(),
        safe: run_args.safe,
        passthrough_args,
        cwd: cwd.clone(),
    };

    let spec = match find_project_catalog(&cwd) {
        Some(project_root) => {
            let project = FsRegistry::new(project_root);
            let overlay = OverlayRegistry::new(global, Some(project));
            resolve(&flags, &settings, &overlay)?
        }
        None => resolve(&flags, &settings, &global)?,
    };

    let provisioned = provision::provision(harness, &spec)?;

    if run_args.print_config {
        print_config(&provisioned.dir, &provisioned.launch, run_args.keep_config);
        return Ok(());
    }

    let code = crate::run::run(&provisioned, &cwd, run_args.keep_config)?;
    std::process::exit(code);
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
