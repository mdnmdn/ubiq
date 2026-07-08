//! Command-line interface: argument parsing and command dispatch.
//!
//! `am <harness> [flags] [-- passthrough…]` wraps and runs a harness;
//! `am catalog|account|session|help` are reserved subcommands for managing
//! the tool itself (see `_docs/target/cli.md`). This module implements the
//! full `resolve → provision → run` spine, including `--print-config` for
//! inspecting a provisioned run without launching it; `catalog`/`account`/
//! `session` are stubbed until their own steps land.

mod run;
mod catalog;

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::harness;

/// Reserved first-positional words that are never harness ids.
const RESERVED: &[&str] = &["catalog", "account", "session"];

/// Entry point called by `main.rs`. Parses `std::env::args`, dispatches, and
/// returns the process-level result (errors become a non-zero exit via
/// `main`'s `anyhow::Result` propagation).
pub fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    dispatch(&args)
}

/// Dispatch on the already-collected argv (excludes the binary name).
/// Split out from [`run`] so it can be exercised without `std::env::args`.
fn dispatch(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            print_usage();
            Ok(())
        }
        Some("--version") | Some("-V") => {
            println!("agent-manager {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("catalog") => catalog::run(&args[1..]),
        Some(word) if RESERVED.contains(&word) => {
            println!("{word}: not yet implemented");
            Ok(())
        }
        Some(key) => {
            let Some(h) = harness::resolve(key) else {
                bail!(
                    "unknown harness '{key}'; known: {}",
                    harness::known_ids().join(", ")
                );
            };
            run::run_harness(h.as_ref(), &args[1..])
        }
    }
}

/// Print a short usage summary: reserved subcommands + known harness ids.
fn print_usage() {
    println!("agent-manager {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("USAGE:");
    println!("    am <harness> [flags] [-- <harness-args>…]   wrap & run a harness");
    println!("    am catalog   <ls|import|show|path> …         manage the catalog");
    println!("    am account   <ls|use|import> …                manage accounts");
    println!("    am session   <ls|show|resume> …               manage session history");
    println!("    am help | am --version");
    println!();
    println!("KNOWN HARNESSES:");
    for id in harness::known_ids() {
        println!("    {id}");
    }
}

/// Split `args` at the first standalone `"--"`: everything before is `am`'s
/// own flags, everything after is forwarded verbatim to the harness binary.
fn split_passthrough(args: &[String]) -> (Vec<String>, Vec<String>) {
    match args.iter().position(|a| a == "--") {
        Some(idx) => (args[..idx].to_vec(), args[idx + 1..].to_vec()),
        None => (args.to_vec(), Vec::new()),
    }
}

/// Parsed `am <harness>` run flags (the part before `--`).
#[derive(Debug, Clone, Default, clap::Parser)]
#[command(name = "am-run", disable_help_flag = false)]
struct RunArgs {
    /// Catalog MCP ids to inject (comma-separated or repeatable).
    #[arg(long, value_delimiter = ',')]
    mcps: Option<Vec<String>>,
    /// Catalog skill ids to inject (comma-separated or repeatable).
    #[arg(long, value_delimiter = ',')]
    skills: Option<Vec<String>>,
    /// Inline MCP definition file (bypasses the catalog).
    #[arg(long)]
    mcp_json: Option<PathBuf>,
    /// Account/credential profile to use. (P2)
    #[arg(long)]
    account: Option<String>,
    /// Shorthand restricted-policy preset.
    #[arg(long)]
    safe: bool,
    /// Settings file to merge (toml/yaml). Default: discovered.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Catalog root override.
    #[arg(long)]
    catalog: Option<PathBuf>,
    /// Don't delete the ephemeral config dir on exit (debugging).
    #[arg(long)]
    keep_config: bool,
    /// Provision only; print the generated dir + argv + env; don't launch.
    #[arg(long)]
    print_config: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_passthrough_splits_at_first_double_dash() {
        let args = vec![
            "--mcps".to_string(),
            "a,b".to_string(),
            "--".to_string(),
            "--model".to_string(),
            "opus".to_string(),
        ];
        let (before, after) = split_passthrough(&args);
        assert_eq!(before, vec!["--mcps".to_string(), "a,b".to_string()]);
        assert_eq!(after, vec!["--model".to_string(), "opus".to_string()]);
    }

    #[test]
    fn split_passthrough_no_dashdash_is_all_before() {
        let args = vec!["--print-config".to_string()];
        let (before, after) = split_passthrough(&args);
        assert_eq!(before, args);
        assert!(after.is_empty());
    }

    #[test]
    fn dispatch_unknown_harness_errors_with_known_ids() {
        let err = dispatch(&["nope".to_string()]).unwrap_err();
        assert!(err.to_string().contains("unknown harness"));
        assert!(err.to_string().contains("claude-code"));
    }

    #[test]
    fn dispatch_reserved_word_does_not_error() {
        assert!(dispatch(&["catalog".to_string()]).is_ok());
        assert!(dispatch(&["account".to_string()]).is_ok());
        assert!(dispatch(&["session".to_string()]).is_ok());
    }

    #[test]
    fn dispatch_help_and_empty_do_not_error() {
        assert!(dispatch(&[]).is_ok());
        assert!(dispatch(&["--help".to_string()]).is_ok());
        assert!(dispatch(&["help".to_string()]).is_ok());
    }
}
