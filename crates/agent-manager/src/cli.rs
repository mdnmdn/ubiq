//! Command-line interface: argument parsing and command dispatch.

use anyhow::Result;
use clap::{Parser, Subcommand};

/// Top-level CLI arguments.
#[derive(Debug, Parser)]
#[command(
    name = "agent-manager",
    version,
    about = "Unified configuration manager for AI agent harnesses.",
    long_about = None,
)]
pub struct Args {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print detected harnesses and their on-disk state.
    Status,
    /// Synchronize the project config into every enabled harness.
    Sync,
    /// Launch the interactive TUI.
    Tui,
    /// Show the resolved path / config for a given harness.
    Inspect {
        /// Harness id (e.g. `claude-code`, `codex`, `copilot`, `opencode`).
        #[arg(value_name = "HARNESS")]
        harness: String,
    },
}

impl Command {
    /// Run the selected subcommand.
    pub fn run(self) -> Result<()> {
        match self {
            Command::Status => {
                eprintln!("status: not yet implemented");
                Ok(())
            }
            Command::Sync => {
                eprintln!("sync: not yet implemented");
                Ok(())
            }
            Command::Tui => {
                eprintln!("tui: not yet implemented");
                Ok(())
            }
            Command::Inspect { harness } => {
                eprintln!("inspect({harness}): not yet implemented");
                Ok(())
            }
        }
    }
}
