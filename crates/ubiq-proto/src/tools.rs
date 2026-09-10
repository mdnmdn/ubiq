//! Runnable tools: named commands a window runs in a terminal pane, defined for the machine
//! or for one project.
//!
//! A tool is a shell row the user wrote themselves — a build, a watcher, a specific shell with
//! flags. It travels the road a shell does: the interface lists it in the new-pane menu and runs
//! it with [`Message::RunTool`], the host spawns it in the project's folder and answers
//! [`Message::WorkspaceSpawned`] with the tool's name as the tab title's seed.
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
}

impl ToolDef {
    /// Whether this runs on `os` — `std::env::consts::OS` on the host. An empty list is not
    /// "nowhere" but "everywhere": a tool that never named a platform runs on all of them.
    pub fn applies(&self, os: &str) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|platform| platform == os)
    }
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
