//! ratatui-based interactive front end.
//!
//! The TUI is intentionally a thin layer over the library: it never owns any
//! state, it only renders [`crate::config::UnifiedConfig`] and dispatches user
//! actions back to the rest of the crate.

use anyhow::Result;

/// Launch the TUI on the current terminal.
pub fn run() -> Result<()> {
    eprintln!("tui: not yet implemented");
    Ok(())
}
