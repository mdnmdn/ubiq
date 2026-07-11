//! agent-manager library root.
//!
//! `agent-manager` is a CLI + TUI that lets you describe a single, harness-agnostic
//! configuration for an AI coding agent (rules, policies, skills, MCP servers,
//! sub-agents) and then *project* that configuration onto every supported harness
//! (Claude Code, Codex, GitHub Copilot, opencode, ...).
//!
//! The library exposes the building blocks used by the binary:
//!
//! - [`account`]   — accounts: credential *references* (never secrets) that
//!   resolve to an [`Account`](account::Account), injected at launch time.
//! - [`spec`]      — [`RunSpec`], the fully-resolved, harness-agnostic plan.
//! - [`resolve`]   — merge CLI flags + settings + catalog into a [`RunSpec`].
//! - [`settings`]  — load / discover the settings file (layered defaults).
//! - [`registry`]  — the catalog (trait + filesystem-backed implementation).
//! - [`harness`]   — the [`harness::Harness`] trait + per-harness impls
//!   (how to identify, provision, and launch each supported harness).
//! - [`provision`] — turn a [`RunSpec`] into an ephemeral config dir + launch
//!   argv/env for a chosen harness.
//! - [`config`]    — the unified, harness-agnostic config model.
//! - [`io`]        — I/O bridging between `am` and the running harness: the
//!   harness-neutral model + structured bridges are core; raw-tty
//!   passthrough is gated behind the `pty` feature.
//! - [`mcp`]       — in-process MCP for lib mode: the embedder-facing
//!   `McpService` trait is core; the loopback HTTP server that hosts it
//!   (`mcp::server`) is gated behind the `inproc-mcp` feature.
//! - [`isolate`]   — wraps a provisioned [`harness::Launch`] in an isol8
//!   sandbox invocation via a configurable command template. Core (no
//!   feature gate) — a pure transform over `Launch`.
//! - [`run`]       — spawns + supervises the harness child through a PTY:
//!   tty forwarding, `SIGWINCH` resize, exit-code propagation, ephemeral-dir
//!   cleanup. (`pty` feature)
//! - [`session`]   — session history: metadata + transcript for each
//!   `am`-launched run, recorded under `am`'s own state dir. Core (no
//!   feature gate).
//! - [`tui`]       — the ratatui-based interactive front end.
//! - [`cli`]       — the clap-based command-line front end.
//!
//! The binary in `src/main.rs` is intentionally thin: it parses CLI args,
//! dispatches to a command, and delegates all real work to the library.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod account;
#[cfg(feature = "cli")]
pub mod cli;
pub mod config;
pub mod harness;
pub mod io;
pub mod isolate;
pub mod mcp;
pub mod overlay;
pub mod profile;
pub mod provision;
pub mod registry;
pub mod resolve;
#[cfg(feature = "pty")]
pub mod run;
pub mod session;
pub mod settings;
pub mod spec;
#[cfg(feature = "tui")]
pub mod tui;

pub use anyhow::Result;
pub use spec::RunSpec;
