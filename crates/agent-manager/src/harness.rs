//! Knowledge about each supported harness.
//!
//! Each [`Harness`] is the portable description of a concrete agent harness:
//! how to *launch* it (command + default args) and where its configuration
//! files *live* on disk. This is the single source of truth that both the
//! `agent-manager` sync engine and embedding applications (e.g. the Ubiq
//! harness multiplexer) program against, instead of hard-coding per-tool
//! knowledge in many places.
//!
//! See `_docs/harness/<id>.md` for the curated per-harness notes that this
//! module codifies.

use std::path::PathBuf;

/// Stable, lowercase identifier for a harness (e.g. `claude-code`).
pub type HarnessId = String;

/// Description of a supported harness: how to launch it and where its
/// configuration lives.
#[derive(Debug, Clone)]
pub struct Harness {
    /// Stable id, used in config files and CLI flags (e.g. `claude-code`).
    pub id: HarnessId,
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Executable to launch the harness in a terminal (e.g. `claude`).
    pub command: &'static str,
    /// Default arguments passed to [`command`](Self::command) on launch.
    pub default_args: Vec<String>,
    /// Alternative ids that also resolve to this harness (e.g. `claude`).
    pub aliases: Vec<&'static str>,
    /// Root directory of the harness config (e.g. `~/.claude`).
    pub config_root: PathBuf,
}

impl Harness {
    /// Enumerate every harness known at compile time.
    pub fn all() -> Vec<Harness> {
        let home = directories::UserDirs::new()
            .and_then(|d| d.home_dir().to_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let home = PathBuf::from(&home);

        vec![
            Harness {
                id: "claude-code".into(),
                display_name: "Claude Code",
                command: "claude",
                default_args: Vec::new(),
                aliases: vec!["claude"],
                config_root: home.join(".claude"),
            },
            Harness {
                id: "codex".into(),
                display_name: "Codex",
                command: "codex",
                default_args: Vec::new(),
                aliases: vec!["openai-codex"],
                config_root: home.join(".codex"),
            },
            Harness {
                id: "copilot".into(),
                display_name: "GitHub Copilot",
                command: "copilot",
                default_args: Vec::new(),
                aliases: vec!["github-copilot"],
                config_root: home.join(".copilot"),
            },
            Harness {
                id: "gemini".into(),
                display_name: "Gemini CLI",
                command: "gemini",
                default_args: Vec::new(),
                aliases: vec!["gemini-cli"],
                config_root: home.join(".gemini"),
            },
            Harness {
                id: "opencode".into(),
                display_name: "opencode",
                command: "opencode",
                default_args: Vec::new(),
                aliases: vec!["open-code"],
                config_root: home.join(".config").join("opencode"),
            },
        ]
    }

    /// Look up a harness by its exact stable id (returns `None` if unknown).
    pub fn by_id(id: &str) -> Option<Harness> {
        Self::all().into_iter().find(|h| h.id == id)
    }

    /// Resolve a harness by id, alias, or launch command.
    ///
    /// This is the lenient lookup used at the boundary with embedding apps,
    /// where a harness may be referred to by its short command (`claude`),
    /// an alias, or its canonical id (`claude-code`).
    pub fn resolve(key: &str) -> Option<Harness> {
        Self::all().into_iter().find(|h| {
            h.id == key || h.command == key || h.aliases.iter().any(|a| *a == key)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_matches_id_alias_and_command() {
        assert_eq!(Harness::resolve("claude-code").unwrap().id, "claude-code");
        assert_eq!(Harness::resolve("claude").unwrap().id, "claude-code");
        assert_eq!(Harness::resolve("gemini-cli").unwrap().id, "gemini");
        assert!(Harness::resolve("does-not-exist").is_none());
    }

    #[test]
    fn every_harness_has_a_command() {
        for h in Harness::all() {
            assert!(!h.command.is_empty(), "{} missing command", h.id);
        }
    }
}
