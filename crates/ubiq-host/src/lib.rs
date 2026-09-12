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
//! - `connectors`: the identities Ubiq holds at external services, and the flows that obtain them
//!   (behind `listener`)
//! - `assist`: whether a suggestion can be asked for, and the backend that writes one (behind
//!   `harness`)
//! - `atomic`: writing a file so a crash never leaves half of one
//! - `carrier`: a session over one duplex byte stream — the pump every transport shares
//! - `store`: the catalogue, a project's tasks, the interface's view state and settings, behind four traits
//! - `health`: what is actually at a project's path
//! - `repos`: cloning a repository into a project, and the listings that find one (behind `git`)
//! - `projects`: the catalogue as the host runs it
//! - `gc`: collecting the directories of projects no record names (behind `harness`)
//! - `files`: a project's tree and its files, read and written off the coordinator's thread
//! - `git`: a project's repository, observed off the coordinator's thread (behind `git`)
//! - `work`: the tasks a project has written down, and the sessions and agents doing them (behind
//!   `harness`)
//! - `reply`: what a service wants said, before the coordinator addresses it
//! - `coordinator`: the run loop that starts harnesses, supervises them, and answers the bus
//!   (behind `full`)
//! - `pty`: pseudo-terminal streams, the one place a descriptor or a process is held
//! - `quota`: how much of an account's plan is left — the in-memory cache, and the thread that
//!   asks (behind `harness`)
//! - `shells`: which shells this machine has, and how one is started
//! - `agent`: agent-type definitions and the registry over them (behind `harness`)
//! - `conversation`: one live agent, its pump thread, and the one mapping onto the bus (behind
//!   `harness`)
//! - `conversation_record`: a conversation's launch recipe on disk, so a marked one outlives the
//!   process (behind `harness`)
//! - `watch`: what changed on disk in an open project, said without being asked
//! - `mcp`: the MCP surface Ubiq exposes to the agents it hosts — one loopback listener, the
//!   built-in servers behind it, and the registry that says which agent is calling (behind
//!   `harness`)
//! - `web_assets`: the vendor bundles a web panel needs, fetched once into the shared workarea,
//!   verified against a manifest in source, and served by the interface off its own origin (behind
//!   `listener`)
//! - `index`: the full-text index that speeds up content search (behind `index`)
//! - `notifications`: telling the desktop about a notification (behind `desktop`)
//! - `remote`, `links`: the rest of what `listener` gates
//! - `cli_shortcut`: behind `harness`
//!
//! `git`, `index`, `harness`, `listener` and `desktop` are the features a lean embedder (a
//! headless drone with no window, no account, no network surface) can each leave off; `full`,
//! which `default` turns on, is all five together and is what `coordinator` needs.

#[cfg(feature = "harness")]
pub mod agent;
#[cfg(feature = "harness")]
pub mod assist;
pub mod atomic;
pub mod carrier;
#[cfg(feature = "harness")]
pub mod cli_shortcut;
pub mod config;
#[cfg(feature = "listener")]
pub mod connectors;
#[cfg(feature = "harness")]
pub mod conversation;
#[cfg(feature = "harness")]
pub mod conversation_record;
#[cfg(feature = "full")]
pub mod coordinator;
pub mod environment;
pub mod files;
#[cfg(feature = "harness")]
pub mod gc;
#[cfg(feature = "git")]
pub mod git;
pub mod health;
pub mod host_meta;
pub mod host_path;
#[cfg(feature = "index")]
pub mod index;
pub mod links;
#[cfg(feature = "harness")]
pub mod mcp;
#[cfg(feature = "desktop")]
pub mod notifications;
pub mod projects;
pub mod pty;
#[cfg(feature = "harness")]
pub mod quota;
#[cfg(feature = "listener")]
pub mod remote;
pub mod reply;
#[cfg(feature = "git")]
pub mod repos;
pub mod search;
pub mod settings;
pub mod shells;
pub mod store;
pub mod watch;
#[cfg(feature = "listener")]
pub mod web_assets;
#[cfg(feature = "harness")]
pub mod work;
