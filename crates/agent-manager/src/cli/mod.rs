//! Command-line interface: argument parsing and command dispatch.
//!
//! `am <harness> [flags] [-- passthrough…]` wraps and runs a harness;
//! `am catalog|account|session|help` are reserved subcommands for managing
//! the tool itself (see `_docs/target/cli.md`). This module implements the
//! full `resolve → provision → run` spine, including `--print-config` for
//! inspecting a provisioned run without launching it; `session ls|show` list
//! and inspect recorded session history, and `session resume` is stubbed
//! until its own step (F2) lands.

mod run;
mod catalog;
mod account;
mod profile;
mod agent;
mod session;

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::harness;

/// Reserved first-positional words that are never harness ids.
const RESERVED: &[&str] = &["catalog", "account", "profile", "agent", "session"];

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
        Some("account") => account::run(&args[1..]),
        Some("profile") => profile::run(&args[1..]),
        Some("agent") => agent::run(&args[1..]),
        Some("session") => session::run(&args[1..]),
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
    println!("    am profile   <ls|show|use|create> …           manage profiles");
    println!("    am agent     <name> [-- <harness-args>…]       run a profile as a frozen agent");
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
    /// Account/credential id to use.
    #[arg(long)]
    account: Option<String>,
    /// Profile to resolve: composition defaults + account + isolation, with
    /// `extends` inheritance. Explicit flags override its fields. Absent = the
    /// implicit `default` profile (if one exists).
    #[arg(long)]
    profile: Option<String>,
    /// Model id to launch with (harness-native id). Discover valid ids with
    /// `--list-models`.
    #[arg(long)]
    model: Option<String>,
    /// List the models available for this harness and exit (don't launch).
    #[arg(long)]
    list_models: bool,
    /// Named hooks (defined in the settings file) to enable for this run.
    #[arg(long, value_delimiter = ',')]
    hooks: Option<Vec<String>>,
    /// Shorthand restricted-policy preset.
    #[arg(long)]
    safe: bool,
    /// Seed always-on instructions from a file (written to the harness memory file).
    #[arg(long)]
    instructions: Option<PathBuf>,
    /// Seed an initial prompt for the run.
    #[arg(long)]
    prompt: Option<String>,
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
    /// I/O mode: `passthrough` (default) or `structured` (alias: `jsonl`).
    #[arg(long)]
    io: Option<String>,
    /// Output projection for `--io structured` events: `events` (default,
    /// the raw neutral `AgentEvent` NDJSON), `acp`, or `agui` (alias:
    /// `ag-ui`).
    #[arg(long)]
    output: Option<String>,
    /// Run inside an isol8 sandbox. Bare `--isolate` uses no named profile;
    /// `--isolate=<profile>` selects a profile. Absent by default (no
    /// isolation).
    #[arg(long, num_args = 0..=1)]
    isolate: Option<Option<String>>,
    /// Resume a prior harness-native session by id (raw harness id, not an
    /// `am` session id — for resuming from `am`'s own session history use
    /// `am session resume <id>` instead).
    #[arg(long)]
    resume: Option<String>,
    /// Additionally expose these already-injected catalog mcp ids as a
    /// latent skill pointer for this run (merged with any catalog entries
    /// marked `expose = "skill"`; see `_docs/target/mcp-as-skill.md`).
    #[arg(long = "mcp-as-skill", value_delimiter = ',')]
    mcp_as_skill: Option<Vec<String>>,
}

/// Parse the `--io` flag's string value into an [`crate::spec::IoModes`].
///
/// Accepts `"passthrough"`, `"structured"`, and `"jsonl"` (an alias for
/// `structured`, since that's the wire shape most structured bridges will
/// actually speak); anything else is an error naming the value and the
/// accepted set. `None` (flag not given) defaults to
/// [`crate::spec::IoModes::Passthrough`].
fn parse_io_mode(raw: Option<&str>) -> anyhow::Result<crate::spec::IoModes> {
    match raw {
        None => Ok(crate::spec::IoModes::Passthrough),
        Some("passthrough") => Ok(crate::spec::IoModes::Passthrough),
        Some("structured") | Some("jsonl") => Ok(crate::spec::IoModes::Structured),
        Some(other) => bail!(
            "unknown --io value '{other}'; expected 'passthrough' or 'structured' (alias: 'jsonl')"
        ),
    }
}

/// Which projection `--io structured` prints events through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    /// The raw neutral `AgentEvent` NDJSON (today's only behavior).
    Events,
    /// Project each event through [`crate::io::to_acp`].
    Acp,
    /// Project each event through [`crate::io::to_agui`].
    AgUi,
}

/// Parse the `--output` flag's string value into an [`OutputMode`].
///
/// Accepts `"events"`, `"acp"`, and `"agui"`/`"ag-ui"` (aliases for the same
/// AG-UI projection); anything else is an error naming the value and the
/// accepted set. `None` (flag not given) defaults to [`OutputMode::Events`].
fn parse_output_mode(raw: Option<&str>) -> anyhow::Result<OutputMode> {
    match raw {
        None => Ok(OutputMode::Events),
        Some("events") => Ok(OutputMode::Events),
        Some("acp") => Ok(OutputMode::Acp),
        Some("agui") | Some("ag-ui") => Ok(OutputMode::AgUi),
        Some(other) => bail!(
            "unknown --output value '{other}'; expected 'events', 'acp', or 'agui' (alias: 'ag-ui')"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

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
        assert!(dispatch(&["profile".to_string()]).is_ok());
        assert!(dispatch(&["session".to_string()]).is_ok());
    }

    #[test]
    fn dispatch_help_and_empty_do_not_error() {
        assert!(dispatch(&[]).is_ok());
        assert!(dispatch(&["--help".to_string()]).is_ok());
        assert!(dispatch(&["help".to_string()]).is_ok());
    }

    #[test]
    fn parse_io_mode_defaults_to_passthrough() {
        assert_eq!(
            parse_io_mode(None).unwrap(),
            crate::spec::IoModes::Passthrough
        );
    }

    #[test]
    fn parse_io_mode_accepts_passthrough_and_structured_and_jsonl_alias() {
        assert_eq!(
            parse_io_mode(Some("passthrough")).unwrap(),
            crate::spec::IoModes::Passthrough
        );
        assert_eq!(
            parse_io_mode(Some("structured")).unwrap(),
            crate::spec::IoModes::Structured
        );
        assert_eq!(
            parse_io_mode(Some("jsonl")).unwrap(),
            crate::spec::IoModes::Structured
        );
    }

    #[test]
    fn parse_io_mode_bogus_value_is_an_error() {
        let err = parse_io_mode(Some("bogus")).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn parse_output_mode_defaults_to_events() {
        assert_eq!(parse_output_mode(None).unwrap(), OutputMode::Events);
    }

    #[test]
    fn parse_output_mode_accepts_events_acp_agui_and_ag_ui_alias() {
        assert_eq!(
            parse_output_mode(Some("events")).unwrap(),
            OutputMode::Events
        );
        assert_eq!(parse_output_mode(Some("acp")).unwrap(), OutputMode::Acp);
        assert_eq!(parse_output_mode(Some("agui")).unwrap(), OutputMode::AgUi);
        assert_eq!(parse_output_mode(Some("ag-ui")).unwrap(), OutputMode::AgUi);
    }

    #[test]
    fn parse_output_mode_bogus_value_is_an_error() {
        let err = parse_output_mode(Some("bogus")).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn isolate_flag_absent_is_none() {
        let args = RunArgs::try_parse_from(["am-run"]).unwrap();
        assert_eq!(args.isolate, None);
    }

    #[test]
    fn isolate_flag_bare_is_some_none() {
        let args = RunArgs::try_parse_from(["am-run", "--isolate"]).unwrap();
        assert_eq!(args.isolate, Some(None));
    }

    #[test]
    fn isolate_flag_with_value_is_some_some() {
        let args = RunArgs::try_parse_from(["am-run", "--isolate=dev"]).unwrap();
        assert_eq!(args.isolate, Some(Some("dev".to_string())));
    }

    #[test]
    fn resume_flag_is_parsed() {
        let args = RunArgs::try_parse_from(["am-run", "--resume", "abc"]).unwrap();
        assert_eq!(args.resume, Some("abc".to_string()));
    }

    #[test]
    fn resume_flag_absent_is_none() {
        let args = RunArgs::try_parse_from(["am-run"]).unwrap();
        assert_eq!(args.resume, None);
    }
}
