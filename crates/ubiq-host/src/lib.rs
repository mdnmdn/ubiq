//! The host: everything that is not drawing.
//!
//! Processes, pseudo-terminals, and the harnesses running under them. It has no window, no
//! palette, no layout and no dependency that draws — `just host` checks that mechanically rather
//! than trusting the rule.
//!
//! Everything it says leaves as a [`ubiq_proto::messages::Message`] addressed to a client. It
//! renders nothing and has no opinion about layout or colour.
//!
//! - `config`: where Ubiq's config root is, and how it is found
//! - `atomic`: writing a file so a crash never leaves half of one
//! - `store`: the catalogue and the interface's view state, behind two traits
//! - `health`: what is actually at a project's path
//! - `projects`: the catalogue as the host runs it
//! - `gc`: collecting the directories of projects no record names
//! - `files`: a project's tree and its files, read and written off the coordinator's thread
//! - `coordinator`: the run loop that starts harnesses, supervises them, and answers the bus
//! - `pty`: pseudo-terminal streams, the one place a descriptor or a process is held
//! - `agent`: agent-type definitions and the registry over them
//! - `mcp_server`: the MCP surface Ubiq exposes to the agents it hosts

pub mod agent;
pub mod atomic;
pub mod config;
pub mod coordinator;
pub mod files;
pub mod gc;
pub mod health;
pub mod mcp_server;
pub mod projects;
pub mod pty;
pub mod store;
