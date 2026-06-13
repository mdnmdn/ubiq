//! agent-manager library root.
//!
//! `agent-manager` is a CLI + TUI that lets you describe a single, harness-agnostic
//! configuration for an AI coding agent (rules, policies, skills, MCP servers,
//! sub-agents) and then *project* that configuration onto every supported harness
//! (Claude Code, Codex, GitHub Copilot, opencode, ...).
//!
//! The library exposes the building blocks used by the binary:
//!
//! - [`harness`]   — knowledge about each supported harness (paths, formats).
//! - [`config`]    — the unified, harness-agnostic config model.
//! - [`project`]   — discover / load a project root and its config.
//! - [`sync`]      — reconcile a unified config against the on-disk state of
//!                   one or more harnesses.
//! - [`tui`]       — the ratatui-based interactive front end.
//! - [`cli`]       — the clap-based command-line front end.
//!
//! The binary in `src/main.rs` is intentionally thin: it parses CLI args,
//! dispatches to a command, and delegates all real work to the library.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "cli")]
pub mod cli;
pub mod config;
pub mod harness;
pub mod project;
pub mod sync;
#[cfg(feature = "tui")]
pub mod tui;

pub use anyhow::Result;
