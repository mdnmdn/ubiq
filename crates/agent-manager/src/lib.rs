//! agent-manager library root.
//!
//! `agent-manager` is a CLI + TUI that lets you describe a single, harness-agnostic
//! configuration for an AI coding agent (rules, policies, skills, MCP servers,
//! sub-agents) and then *project* that configuration onto every supported harness
//! (Claude Code, Codex, GitHub Copilot, opencode, ...).
//!
//! The library exposes the building blocks used by the binary:
//!
//! - [`spec`]      — [`RunSpec`], the fully-resolved, harness-agnostic plan.
//! - [`resolve`]   — merge CLI flags + settings + catalog into a [`RunSpec`].
//! - [`settings`]  — load / discover the settings file (layered defaults).
//! - [`registry`]  — the catalog (trait + filesystem-backed implementation).
//! - [`harness`]   — the [`harness::Harness`] trait + per-harness impls
//!   (how to identify, provision, and launch each supported harness).
//! - [`provision`] — turn a [`RunSpec`] into an ephemeral config dir + launch
//!   argv/env for a chosen harness.
//! - [`config`]    — the unified, harness-agnostic config model.
//! - [`io`]        — I/O bridging between the local terminal and the running
//!   harness (Phase 1: raw-tty passthrough only). (`pty` feature)
//! - [`run`]       — spawns + supervises the harness child through a PTY:
//!   tty forwarding, `SIGWINCH` resize, exit-code propagation, ephemeral-dir
//!   cleanup. (`pty` feature)
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
#[cfg(feature = "pty")]
pub mod io;
pub mod provision;
pub mod registry;
pub mod resolve;
#[cfg(feature = "pty")]
pub mod run;
pub mod settings;
pub mod spec;
#[cfg(feature = "tui")]
pub mod tui;

pub use anyhow::Result;
pub use spec::RunSpec;
