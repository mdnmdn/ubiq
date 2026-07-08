//! Loads and represents the settings file: the layered defaults that CLI
//! flags override.
//!
//! A settings file (`am.toml` / `agent-manager.yaml` / …) can set a catalog
//! root, defaults applied to every run, per-harness overrides, and named
//! `--safe`-style presets. [`discover`] walks up from a starting directory to
//! find the nearest project-level file (mirroring the way harnesses discover
//! `CLAUDE.md`), falling back to a global config file if none is found.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::spec::Policy;

/// Candidate settings file basenames, in the order they are tried within a
/// single directory. The first one that exists wins.
const CANDIDATE_BASENAMES: &[&str] = &[
    "am.toml",
    "am.yaml",
    "am.yml",
    "agent-manager.toml",
    "agent-manager.yaml",
    "agent-manager.yml",
    ".am.toml",
    ".am.yaml",
    ".am.yml",
    ".agent-manager.toml",
    ".agent-manager.yaml",
    ".agent-manager.yml",
];

/// The parsed settings file: catalog override, layered defaults, and presets.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Settings {
    /// Catalog root override (`catalog =` key). Applied when building the registry.
    #[serde(default)]
    pub catalog: Option<String>,
    /// Defaults applied to every `am <harness>` run.
    #[serde(default)]
    pub defaults: HarnessDefaults,
    /// Per-harness defaults, keyed by harness id (`[harness.claude]`).
    #[serde(default)]
    pub harness: BTreeMap<String, HarnessDefaults>,
    /// Named presets (`[presets.safe]`) that flags like `--safe` expand to.
    #[serde(default)]
    pub presets: BTreeMap<String, Policy>,
}

/// One layer of defaults (either `[defaults]` or a `[harness.<id>]` table).
///
/// Each field is `Option<Vec<_>>` / `Option<String>` rather than a bare
/// collection: `None` means "this layer didn't mention the key" (fall
/// through to a lower-precedence layer), while `Some(vec![])` means "this
/// layer explicitly sets it to empty" (replaces lower layers with nothing).
/// This distinction is load-bearing for the replace-by-default merge in
/// [`crate::resolve`].
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HarnessDefaults {
    /// Catalog MCP ids. `None` = "not mentioned at this layer" (distinct from `Some(vec![])`).
    #[serde(default)]
    pub mcps: Option<Vec<String>>,
    /// Catalog skill ids. Same None-vs-empty distinction.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    /// Account/credential profile id.
    #[serde(default)]
    pub account: Option<String>,
}

/// Load a settings file from an explicit path.
///
/// The format is chosen by extension: `.toml` parses as TOML, anything else
/// (`.yaml`, `.yml`, or no/unknown extension) parses as YAML.
pub fn load(path: &Path) -> Result<Settings> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading settings file {}", path.display()))?;

    let is_toml = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("toml"))
        .unwrap_or(false);

    if is_toml {
        toml::from_str(&raw).with_context(|| format!("parsing settings file {}", path.display()))
    } else {
        serde_yaml::from_str(&raw)
            .with_context(|| format!("parsing settings file {}", path.display()))
    }
}

/// Discover the nearest settings file by walking up from `cwd`.
///
/// In each directory, the [`CANDIDATE_BASENAMES`] are tried in order; the
/// first one that exists is loaded and returned along with its path.
/// Ascent stops after checking a directory that contains a `.git` entry
/// (that directory is still checked before stopping). If nothing is found
/// while walking, falls back to the global config file at
/// `<config_dir>/agent-manager/config.{toml,yaml,yml}`. Returns `Ok(None)`
/// if nothing exists anywhere.
pub fn discover(cwd: &Path) -> Result<Option<(Settings, PathBuf)>> {
    let mut current = Some(cwd.to_path_buf());

    while let Some(dir) = current {
        if let Some(found) = find_in_dir(&dir)? {
            return Ok(Some(found));
        }

        if dir.join(".git").exists() {
            break;
        }

        current = dir.parent().map(|p| p.to_path_buf());
    }

    // Fall back to the global config file.
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "agent-manager") {
        let config_dir = proj_dirs.config_dir();
        for ext in ["toml", "yaml", "yml"] {
            let candidate = config_dir.join(format!("config.{ext}"));
            if candidate.is_file() {
                let settings = load(&candidate)?;
                return Ok(Some((settings, candidate)));
            }
        }
    }

    Ok(None)
}

/// Try each candidate basename in `dir`, in order; load and return the first
/// one that exists.
fn find_in_dir(dir: &Path) -> Result<Option<(Settings, PathBuf)>> {
    for basename in CANDIDATE_BASENAMES {
        let candidate = dir.join(basename);
        if candidate.is_file() {
            let settings = load(&candidate)?;
            return Ok(Some((settings, candidate)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_discover_finds_am_toml_and_stops_at_git_root() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        // A settings file that lives *outside* (above) the git repo; it must
        // never be picked up once discovery has stopped at the git root.
        fs::write(temp.path().join("am.toml"), "catalog = \"/outer\"\n")?;

        let root = temp.path().join("repo");
        fs::create_dir_all(&root)?;

        // Mark the repo root as a git repo.
        fs::create_dir_all(root.join(".git"))?;

        // Settings file lives at the git root.
        fs::write(
            root.join("am.toml"),
            "catalog = \"/somewhere\"\n[defaults]\nmcps = [\"github\"]\n",
        )?;

        // Start discovery from a nested subdirectory.
        let nested = root.join("a").join("b");
        fs::create_dir_all(&nested)?;

        let (settings, path) = discover(&nested)?.expect("settings should be found");
        assert_eq!(path, root.join("am.toml"));
        assert_eq!(settings.catalog.as_deref(), Some("/somewhere"));
        assert_eq!(settings.defaults.mcps, Some(vec!["github".to_string()]));

        Ok(())
    }

    #[test]
    fn test_load_toml() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("am.toml");
        fs::write(
            &path,
            r#"
catalog = "~/.agent-manager/catalog"

[defaults]
mcps = ["github"]
skills = []

[harness.claude]
account = "work"
mcps = ["postgres"]

[presets.safe]
permission_mode = "restricted"
deny = ["Bash(rm *)", "WebFetch"]
"#,
        )?;

        let settings = load(&path)?;
        assert_eq!(
            settings.catalog.as_deref(),
            Some("~/.agent-manager/catalog")
        );
        assert_eq!(settings.defaults.mcps, Some(vec!["github".to_string()]));
        assert_eq!(settings.defaults.skills, Some(vec![]));

        let claude = settings.harness.get("claude").expect("harness.claude");
        assert_eq!(claude.account.as_deref(), Some("work"));
        assert_eq!(claude.mcps, Some(vec!["postgres".to_string()]));

        let safe = settings.presets.get("safe").expect("presets.safe");
        assert_eq!(safe.permission_mode.as_deref(), Some("restricted"));
        assert_eq!(
            safe.deny,
            vec!["Bash(rm *)".to_string(), "WebFetch".to_string()]
        );

        Ok(())
    }

    #[test]
    fn test_load_yaml() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("am.yaml");
        fs::write(
            &path,
            r#"
catalog: "~/.agent-manager/catalog"
defaults:
  mcps: ["github"]
harness:
  claude:
    account: "work"
    mcps: ["postgres"]
presets:
  safe:
    permission_mode: "restricted"
    deny: ["Bash(rm *)"]
"#,
        )?;

        let settings = load(&path)?;
        assert_eq!(
            settings.catalog.as_deref(),
            Some("~/.agent-manager/catalog")
        );
        assert_eq!(settings.defaults.mcps, Some(vec!["github".to_string()]));
        let claude = settings.harness.get("claude").expect("harness.claude");
        assert_eq!(claude.account.as_deref(), Some("work"));

        Ok(())
    }

    #[test]
    fn test_candidate_ordering_am_toml_wins() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let dir = temp.path();

        // Both present in the same directory; am.toml must win.
        fs::write(dir.join("am.toml"), "catalog = \"/from-am-toml\"\n")?;
        fs::write(
            dir.join(".agent-manager.toml"),
            "catalog = \"/from-dotfile\"\n",
        )?;
        // Mark as git root so discovery stops here.
        fs::create_dir_all(dir.join(".git"))?;

        let (settings, path) = discover(dir)?.expect("settings should be found");
        assert_eq!(path, dir.join("am.toml"));
        assert_eq!(settings.catalog.as_deref(), Some("/from-am-toml"));

        Ok(())
    }
}
