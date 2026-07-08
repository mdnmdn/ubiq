//! Executable entry point for `agent-manager`.
//!
//! Real logic lives in the library. This binary just wires up logging, parses
//! CLI args, and dispatches to the appropriate command.

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    init_tracing();

    agent_manager::cli::run()
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();
}
