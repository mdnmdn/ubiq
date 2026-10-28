//! The remote-tool family's records: what an agent bound to a drone asks of it, and what comes back.
//!
//! An agent started with [`crate::messages::Message::StartConversation`]'s `remote` set is given the
//! `ubiq-remote` MCP server. Its tool calls leave the host as a
//! [`crate::messages::Message::RemoteToolCall`], travel to the window — which holds the drone's link
//! (`D116`) — and the window relays them to the drone unread. The drone answers with a
//! [`crate::messages::Message::RemoteToolResult`], which the window relays back the same way
//! (`D226`).
//!
//! **Paths are root-relative** (`D136`): every `path` and `cwd` here names a place under the root
//! the call's `root` names — one of [`RemoteInfo::roots`], the first when `None` — never an absolute
//! path on the far machine, and confinement and a root's read-only mode are the drone's to enforce
//! rather than the tool layer's.
//!
//! **A thin façade.** The drone answers each op by delegating to the handlers it already has — the
//! file worker for list, read, write and edit, the search tool for glob and grep — so path safety
//! and refusals are theirs, not a second implementation's.

use serde::{Deserialize, Serialize};

use crate::files::EntryKind;

/// The most a [`RemoteOp::Read`] returns in one answer. Past it the content is cut and
/// [`RemoteOutput::Read`]'s `truncated` is set; the model pages on with `offset`.
pub const REMOTE_READ_CAP: usize = 128 * 1024;

/// The most of each of stdout and stderr a [`RemoteOp::Run`] keeps — the **tail**, because the end
/// of a build log is where its error is. Past it [`RemoteRun::truncated`] is set.
pub const REMOTE_RUN_TAIL_CAP: usize = 64 * 1024;

/// One thing an agent asks a drone to do. Internally tagged by `kind`, the
/// [`crate::browser::BrowserCmd`] shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteOp {
    /// The machine and its roots — answered with [`RemoteOutput::Info`].
    Info,
    /// The entries under `path`, `depth` levels down; `None` is one level.
    List { path: String, depth: Option<u32> },
    /// A text file's lines: `offset` lines skipped, at most `limit` returned (`0` is "up to the
    /// cap"), never more than [`REMOTE_READ_CAP`] bytes.
    Read {
        path: String,
        #[serde(default)]
        offset: u64,
        #[serde(default)]
        limit: u64,
    },
    /// Write `content` to `path`. A file that exists is refused unless `overwrite` — a model tracks
    /// no content hash between turns, so the explicit flag stands where the file family's
    /// `expected` does (`G268`). A read-only root refuses it at the drone.
    Write {
        path: String,
        content: String,
        #[serde(default)]
        overwrite: bool,
    },
    /// Replace `old` with `new` in `path`. Without `replace_all`, an `old` that occurs other than
    /// exactly once is refused, so an edit never lands somewhere the model did not mean.
    Edit {
        path: String,
        old: String,
        new: String,
        #[serde(default)]
        replace_all: bool,
    },
    /// Paths matching `pattern`, under `path` or the root.
    Glob {
        pattern: String,
        path: Option<String>,
    },
    /// Lines matching `pattern`, under `path` or the root, restricted to files matching `glob`.
    Grep {
        pattern: String,
        path: Option<String>,
        glob: Option<String>,
        #[serde(default)]
        case_insensitive: bool,
        max_results: u32,
    },
    /// Run `cmd` through `shell` (the drone's own when `None`) in `cwd` (the root when `None`),
    /// killed after `timeout_ms`. A process, never a pane: there is no pseudo-terminal to scrape
    /// (`D137`).
    Run {
        cmd: String,
        cwd: Option<String>,
        timeout_ms: u64,
        shell: Option<String>,
    },
}

impl RemoteOp {
    /// The op's name, for a log line.
    pub fn name(&self) -> &'static str {
        match self {
            RemoteOp::Info => "info",
            RemoteOp::List { .. } => "list",
            RemoteOp::Read { .. } => "read",
            RemoteOp::Write { .. } => "write",
            RemoteOp::Edit { .. } => "edit",
            RemoteOp::Glob { .. } => "glob",
            RemoteOp::Grep { .. } => "grep",
            RemoteOp::Run { .. } => "run",
        }
    }
}

/// What a [`RemoteOp`] produced — one variant per op, same tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoteOutput {
    /// Boxed: the widest answer, and the rarest.
    Info(Box<RemoteInfo>),
    List {
        entries: Vec<RemoteEntry>,
        truncated: bool,
    },
    Read {
        content: String,
        truncated: bool,
    },
    Write {
        bytes: u64,
        created: bool,
    },
    Edit {
        replacements: u32,
    },
    Glob {
        paths: Vec<String>,
        truncated: bool,
    },
    Grep {
        matches: Vec<RemoteMatch>,
        truncated: bool,
    },
    Run(Box<RemoteRun>),
}

/// The far machine, as [`RemoteOp::Info`] reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteInfo {
    pub os: String,
    pub arch: String,
    pub hostname: String,
    /// The shell [`RemoteOp::Run`] uses when it names none.
    pub shell: Option<String>,
    /// Exactly the places a path may name (`D136`).
    pub roots: Vec<RemoteRoot>,
    /// The tools found on the drone's `PATH`, as `(name, version)` — `rg`, `git`, `node`, …
    pub tools: Vec<(String, String)>,
}

/// One root a drone serves, and whether a write under it is refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRoot {
    /// What a call's `root` names it by.
    pub name: String,
    /// The root's folder on the far machine — informational; paths in a call are relative to it.
    pub path: String,
    #[serde(default)]
    pub read_only: bool,
}

/// One entry of a [`RemoteOutput::List`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteEntry {
    /// Root-relative.
    pub path: String,
    pub kind: EntryKind,
    /// Bytes, for a file.
    pub size: Option<u64>,
}

/// One matching line of a [`RemoteOutput::Grep`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMatch {
    /// Root-relative.
    pub path: String,
    /// One-based.
    pub line: u64,
    pub text: String,
}

/// What a [`RemoteOp::Run`] produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRun {
    /// `None` when the process was killed by a signal — including by the timeout.
    pub exit_code: Option<i32>,
    /// The last [`REMOTE_RUN_TAIL_CAP`] bytes, lossily decoded.
    pub stdout: String,
    /// The last [`REMOTE_RUN_TAIL_CAP`] bytes, lossily decoded.
    pub stderr: String,
    /// Either stream lost its head to the cap.
    pub truncated: bool,
    pub timed_out: bool,
}
