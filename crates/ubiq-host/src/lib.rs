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
//! - `atomic`: writing a file so a crash never leaves half of one
//! - `store`: the catalogue, a project's tasks, the interface's view state and settings, behind four traits
//! - `health`: what is actually at a project's path
//! - `repos`: cloning a repository into a project, and the listings that find one
//! - `projects`: the catalogue as the host runs it
//! - `gc`: collecting the directories of projects no record names
//! - `files`: a project's tree and its files, read and written off the coordinator's thread
//! - `git`: a project's repository, observed off the coordinator's thread
//! - `work`: the tasks a project has written down, and the sessions and agents doing them
//! - `reply`: what a service wants said, before the coordinator addresses it
//! - `coordinator`: the run loop that starts harnesses, supervises them, and answers the bus
//! - `pty`: pseudo-terminal streams, the one place a descriptor or a process is held
//! - `shells`: which shells this machine has, and how one is started
//! - `agent`: agent-type definitions and the registry over them
//! - `conversation`: one live agent, its pump thread, and the one mapping onto the bus
//! - `watch`: what changed on disk in an open project, said without being asked
//! - `mcp_server`: the MCP surface Ubiq exposes to the agents it hosts

pub mod agent;
pub mod atomic;
pub mod cli_shortcut;
pub mod config;
pub mod connectors;
pub mod conversation;
pub mod coordinator;
pub mod files;
pub mod gc;
pub mod git;
pub mod health;
pub mod links;
pub mod mcp_server;
pub mod projects;
pub mod pty;
pub mod reply;
pub mod repos;
pub mod search;
pub mod settings;
pub mod shells;
pub mod store;
pub mod watch;
pub mod work;
