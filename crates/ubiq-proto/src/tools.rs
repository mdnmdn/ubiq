//! Runnable tools: named commands a window runs in a terminal pane, defined for the machine
//! or for one project.
//!
//! A tool is a shell row the user wrote themselves — a build, a watcher, a specific shell with
//! flags. It travels the road a shell does: the interface lists it behind the titlebar's run
//! control and runs it with [`Message::RunTool`], the host spawns it in the project's folder — or
//! in [`ToolDef::starting_folder`] instead, when the row names one — and answers
//! [`Message::WorkspaceSpawned`] with the tool's name as the tab title's seed and the
//! [`ToolRun`] that started it.
//!
//! [`Message::RunTool`]: crate::messages::Message::RunTool
//! [`Message::WorkspaceSpawned`]: crate::messages::Message::WorkspaceSpawned

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ToolId;
use crate::projects::Scope;

/// The platform strings a tool may name. Compared against `std::env::consts::OS` on the host,
/// which spells them exactly this way.
pub const TOOL_PLATFORMS: [&str; 3] = ["macos", "windows", "linux"];

/// One runnable tool: what the tab says, what runs, and where it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Stable id, minted UI-side when the row is added. The name is what the tab says and
    /// changes; this is what a run names.
    pub id: ToolId,
    /// What the tab says — the title seed `pane_title` numbers.
    pub name: String,
    /// The program textbox: a bare name (`cargo`), a path, or a program with its first
    /// arguments. The host splits it the way it splits a harness command override, uses the
    /// first word as the program and puts the rest in front of [`ToolDef::args`].
    pub command: String,
    /// The parameters textbox, split the same way and appended after the command's own.
    #[serde(default)]
    pub args: String,
    /// Extra environment for the run, on top of what the pane inherits.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Platforms this runs on, as [`TOOL_PLATFORMS`] spells them. Empty runs everywhere; the
    /// settings panels offer the three as a multiple choice.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Keep the pane open after the command ends instead of closing it with the process, so
    /// its output stays readable until the tab is closed.
    #[serde(default)]
    pub wait_on_exit: bool,
    /// Keep the pane open when the command ends with a non-zero status, and close it normally
    /// on a clean exit. Redundant, and disabled in the editor, while [`Self::wait_on_exit`] is
    /// set — that already keeps the pane open on every exit.
    #[serde(default)]
    pub wait_on_error: bool,
    /// Restrict this tool to one run at a time. A second [`Message::RunTool`] while a pane
    /// started by this tool is still held is refused with [`Message::ToolError`] rather than
    /// given a pane — a watcher on a port, a dev server, anything that cannot share a resource
    /// with a copy of itself.
    ///
    /// The host is what enforces it, because the host is what knows every pane that exists:
    /// a window can only see its own.
    ///
    /// [`Message::RunTool`]: crate::messages::Message::RunTool
    /// [`Message::ToolError`]: crate::messages::Message::ToolError
    #[serde(default)]
    pub single_instance: bool,
    /// Where this tool starts instead of the project's own folder — an absolute path, chosen
    /// through Ubiq's own folder picker rather than typed. `None` is today's behaviour exactly:
    /// the host resolves the project's folder the way every other pane does. Set, it overrides
    /// that resolution outright and may name a folder outside the project entirely — a sibling
    /// checkout, a vendored dependency, anywhere the tool needs to run from.
    #[serde(default)]
    pub starting_folder: Option<String>,
}

impl ToolDef {
    /// Whether this runs on `os` — `std::env::consts::OS` on the host. An empty list is not
    /// "nowhere" but "everywhere": a tool that never named a platform runs on all of them.
    pub fn applies(&self, os: &str) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|platform| platform == os)
    }
}

/// Which tool a pane was started by: the list it came from and its id — the pair
/// [`Message::RunTool`] is addressed with.
///
/// Carried back on [`WorkspaceInfo`] so a pane remembers what started it, which is what lets a
/// stopped tool pane offer Restart without the interface guessing from a tab's title.
///
/// [`Message::RunTool`]: crate::messages::Message::RunTool
/// [`WorkspaceInfo`]: crate::messages::WorkspaceInfo
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRun {
    pub scope: Scope,
    pub id: ToolId,
}

/// One tool as a menu offers it: the definition, whose list it came from, and whether the host
/// answering runs it on its own platform.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedTool {
    /// [`Scope::Interface`] is the machine-wide list, [`Scope::Project`] the project's own.
    pub scope: Scope,
    pub tool: ToolDef,
    pub applicable: bool,
}

/// Parse the env textbox: one `KEY=VALUE` per line. Returns the pairs and the lines that were
/// not pairs, so the panel can say what it ignored rather than dropping it silently.
pub fn parse_env(text: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let mut env = BTreeMap::new();
    let mut ignored = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once('=') {
            Some((key, value)) => {
                let key = key.trim();
                if key.is_empty() {
                    ignored.push(line.to_string());
                } else {
                    env.insert(key.to_string(), value.trim().to_string());
                }
            }
            None => ignored.push(line.to_string()),
        }
    }
    (env, ignored)
}

/// The env textbox as a tool stores it, one `KEY=VALUE` per line.
pub fn format_env(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ToolDef {
        ToolDef {
            id: ToolId::generate(),
            name: "Build".to_string(),
            command: "cargo build".to_string(),
            args: "--release".to_string(),
            env: BTreeMap::new(),
            platforms: Vec::new(),
            wait_on_exit: false,
            wait_on_error: false,
            single_instance: false,
            starting_folder: None,
        }
    }

    #[test]
    fn a_tool_with_no_platforms_runs_everywhere() {
        let tool = tool();
        assert!(tool.applies("macos"));
        assert!(tool.applies("windows"));
        assert!(tool.applies("linux"));
    }

    #[test]
    fn a_platform_list_narrows_where_it_runs() {
        let mut tool = tool();
        tool.platforms = vec!["windows".to_string()];
        assert!(!tool.applies("macos"));
        assert!(tool.applies("windows"));
    }

    #[test]
    fn the_env_textbox_parses_pairs_and_reports_the_rest() {
        let (env, ignored) = parse_env("RUST_LOG=debug\nEMPTY=\nno-equals-here\n# comment\n");
        assert_eq!(env.get("RUST_LOG").map(String::as_str), Some("debug"));
        assert_eq!(env.get("EMPTY").map(String::as_str), Some(""));
        assert_eq!(ignored, vec!["no-equals-here".to_string()]);
        assert_eq!(
            format_env(&env),
            "EMPTY=\nRUST_LOG=debug".to_string(),
            "a map reads back sorted"
        );
    }
}
