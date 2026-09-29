//! Filesystem-backed registry implementation.
//!
//! Scans a catalog root directory for skills and MCP servers. Skills come from
//! three places, in precedence order: copies under `skills/<id>/`, `[[skill]]`
//! links in `catalog.toml` (a folder referenced in place), and `[[skill_dir]]`
//! folders scanned for `<sub>/SKILL.md`. MCP servers come from `catalog.toml`
//! inline `[[mcp]]` tables and `mcp/*.json` files.

use crate::Result;
use crate::config::McpServer;
use crate::registry::{
    McpEntry, McpExpose, Registry, RemoteOrigin, SkillEntry, SkillMeta, SkillOrigin, SkillSource,
    SkillSourceKind,
};
use crate::source::Source;
use anyhow::{Context, anyhow};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// File beside an installed skill's `SKILL.md` recording where it came from.
pub(crate) const ORIGIN_FILE: &str = ".am-origin.toml";

/// A filesystem-backed registry rooted at a catalog directory.
#[derive(Debug, Clone)]
pub struct FsRegistry {
    root: PathBuf,
}

impl FsRegistry {
    /// Create a registry rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsRegistry { root: root.into() }
    }

    /// The catalog root this registry reads and writes.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Registry for FsRegistry {
    fn skills(&self) -> Result<Vec<SkillEntry>> {
        let mut entries: Vec<SkillEntry> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut push = |entry: SkillEntry| {
            if seen.insert(entry.id.clone()) {
                entries.push(entry);
            }
        };

        // 1. Installed copies under skills/. Dot-prefixed folders are install scratch space.
        let skills_dir = self.root.join("skills");
        if skills_dir.is_dir() {
            for entry in std::fs::read_dir(&skills_dir)? {
                let path = entry?.path();
                let Some(id) = folder_name(&path) else {
                    continue;
                };
                if id.starts_with('.') || !path.join("SKILL.md").is_file() {
                    continue;
                }
                let meta = read_meta(&path);
                let remote = read_origin(&path);
                push(SkillEntry {
                    id,
                    source: Source::Dir(path),
                    meta,
                    origin: SkillOrigin::Installed { remote },
                });
            }
        }

        let catalog = self.catalog_toml()?;

        // 2. Linked folders (`[[skill]]`); a dangling link is skipped, not an error.
        for link in catalog.skill {
            let path = PathBuf::from(&link.path);
            let Some(id) = link.id.or_else(|| folder_name(&path)) else {
                continue;
            };
            if !path.join("SKILL.md").is_file() {
                continue;
            }
            let meta = read_meta(&path);
            push(SkillEntry {
                id,
                origin: SkillOrigin::Linked(path.clone()),
                source: Source::Dir(path),
                meta,
            });
        }

        // 3. Scanned folders (`[[skill_dir]]`); the first folder wins on a clash.
        for dir in catalog.skill_dir {
            let dir = PathBuf::from(dir.path);
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut subs: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
            subs.sort();
            for path in subs {
                let Some(id) = folder_name(&path) else {
                    continue;
                };
                if id.starts_with('.') || !path.join("SKILL.md").is_file() {
                    continue;
                }
                let meta = read_meta(&path);
                push(SkillEntry {
                    id,
                    origin: SkillOrigin::Dir(dir.clone()),
                    source: Source::Dir(path),
                    meta,
                });
            }
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }

    fn mcps(&self) -> Result<Vec<McpEntry>> {
        let mut entries = Vec::new();
        let mut seen_ids = BTreeSet::new();

        // First, load inline MCPs from catalog.toml
        for entry in self.catalog_toml()?.mcp {
            let McpToml {
                server,
                expose,
                summary,
                description,
            } = entry;
            if seen_ids.contains(&server.id) {
                return Err(anyhow!(
                    "MCP id collision: '{}' appears in both catalog.toml and mcp/*.json",
                    server.id
                ));
            }
            seen_ids.insert(server.id.clone());
            entries.push(McpEntry {
                id: server.id.clone(),
                def: server,
                expose,
                summary,
                description,
            });
        }

        // Then, load single-file MCPs from mcp/*.json
        let mcp_dir = self.root.join("mcp");
        if mcp_dir.exists() {
            for entry in std::fs::read_dir(&mcp_dir)? {
                let entry = entry?;
                let path = entry.path();

                // Only process .json files
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }

                let id = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow!("Invalid MCP file name"))?;

                if seen_ids.contains(&id) {
                    return Err(anyhow!(
                        "MCP id collision: '{}' appears in both catalog.toml and mcp/*.json",
                        id
                    ));
                }
                seen_ids.insert(id.clone());

                let content = std::fs::read_to_string(&path)?;
                let McpToml {
                    mut server,
                    expose,
                    summary,
                    description,
                } = serde_json::from_str(&content)
                    .with_context(|| format!("parsing {}", path.display()))?;
                server.id = id.clone();

                entries.push(McpEntry {
                    id,
                    def: server,
                    expose,
                    summary,
                    description,
                });
            }
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }
}

impl FsRegistry {
    /// Parse `catalog.toml` (missing file = empty catalog).
    pub(super) fn catalog_toml(&self) -> Result<CatalogToml> {
        let path = self.root.join("catalog.toml");
        if !path.exists() {
            return Ok(CatalogToml::default());
        }
        let content = std::fs::read_to_string(&path)?;
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))
    }
}

/// Parsed structure for catalog.toml.
#[derive(Debug, serde::Deserialize, Default)]
pub(super) struct CatalogToml {
    /// Inline MCP definitions.
    #[serde(default)]
    pub(super) mcp: Vec<McpToml>,
    /// Skill folders referenced in place.
    #[serde(default)]
    pub(super) skill: Vec<SkillLinkToml>,
    /// Folders scanned for `<sub>/SKILL.md`.
    #[serde(default)]
    pub(super) skill_dir: Vec<SkillDirToml>,
    /// Declared skill sources (empty = the defaults).
    #[serde(default)]
    pub(super) skill_source: Vec<SkillSourceToml>,
}

/// One `[[skill]]` entry: a skill folder referenced in place.
#[derive(Debug, serde::Deserialize)]
pub(super) struct SkillLinkToml {
    /// Defaults to the folder name.
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) path: String,
}

/// One `[[skill_dir]]` entry.
#[derive(Debug, serde::Deserialize)]
pub(super) struct SkillDirToml {
    pub(super) path: String,
}

/// One `[[skill_source]]` entry (flat form of [`SkillSource`]).
#[derive(Debug, serde::Deserialize)]
pub(super) struct SkillSourceToml {
    id: String,
    #[serde(default)]
    label: Option<String>,
    kind: String,
    url: String,
    #[serde(default)]
    rev: Option<String>,
    #[serde(default)]
    subpath: Option<String>,
}

impl SkillSourceToml {
    /// Convert to the public type; an unknown `kind` is an error.
    pub(super) fn into_source(self) -> Result<SkillSource> {
        let kind = match self.kind.as_str() {
            "git" => SkillSourceKind::Git {
                url: self.url,
                rev: self.rev.filter(|s| !s.is_empty()),
                subpath: self.subpath.filter(|s| !s.is_empty()),
            },
            "index" => SkillSourceKind::Index { url: self.url },
            other => {
                return Err(anyhow!(
                    "skill_source '{}': unknown kind '{other}' (expected 'git' or 'index')",
                    self.id
                ));
            }
        };
        Ok(SkillSource {
            id: self.id,
            label: self.label,
            kind,
        })
    }
}

/// One `[[mcp]]` entry in `catalog.toml` (or an `mcp/<id>.json` file): the raw
/// [`McpServer`] fields (flattened) plus catalog-only metadata.
#[derive(Debug, serde::Deserialize)]
pub(super) struct McpToml {
    #[serde(flatten)]
    pub(super) server: McpServer,
    /// `expose = "tools"` (default) | `"skill"`. See [`McpExpose`].
    #[serde(default)]
    expose: McpExpose,
    /// Seeds the generated skill's `description:` when `expose = "skill"`.
    #[serde(default)]
    summary: Option<String>,
    /// What the server is for.
    #[serde(default)]
    description: Option<String>,
}

/// Last path component as a `String`.
fn folder_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

/// Frontmatter of `<dir>/SKILL.md` (default when unreadable).
pub(super) fn read_meta(dir: &Path) -> SkillMeta {
    std::fs::read_to_string(dir.join("SKILL.md"))
        .map(|c| parse_skill_frontmatter(&c))
        .unwrap_or_default()
}

/// `.am-origin.toml` of an installed skill, when present and well-formed.
fn read_origin(dir: &Path) -> Option<RemoteOrigin> {
    let text = std::fs::read_to_string(dir.join(ORIGIN_FILE)).ok()?;
    toml::from_str(&text).ok()
}

/// Parse YAML frontmatter from a Markdown file.
///
/// Frontmatter is the block between the first `---` and the next `---`.
/// If no frontmatter exists, returns `SkillMeta::default()`.
pub(super) fn parse_skill_frontmatter(content: &str) -> SkillMeta {
    let lines: Vec<&str> = content.lines().collect();

    // Check if the file starts with ---
    if lines.is_empty() || !lines[0].trim().starts_with("---") {
        return SkillMeta::default();
    }

    // Find the closing --- after the opening ---
    let closing_idx = lines[1..].iter().position(|l| l.trim().starts_with("---"));
    let Some(closing_idx) = closing_idx else {
        return SkillMeta::default();
    };

    // Extract the frontmatter block (between the two --- lines)
    let frontmatter_lines = &lines[1..closing_idx + 1];
    let frontmatter = frontmatter_lines.join("\n");

    // Parse as YAML, lenient (missing fields are OK)
    serde_yaml::from_str(&frontmatter).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_fs_registry_skills() -> Result<()> {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog");

        let registry = FsRegistry::new(&fixture_root);
        let skills = registry.skills()?;

        let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["reviewer", "web-designer"]);

        // Check that web-designer has metadata
        let web_designer = skills
            .iter()
            .find(|s| s.id == "web-designer")
            .expect("web-designer should exist");
        assert_eq!(
            web_designer.meta.description,
            Some("Designs responsive web UIs.".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_fs_registry_mcps() -> Result<()> {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog");

        let registry = FsRegistry::new(&fixture_root);
        let mcps = registry.mcps()?;

        let ids: Vec<_> = mcps.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["docs", "figma", "postgres"]);

        // Check postgres
        let postgres = registry.mcp("postgres")?;
        assert!(postgres.is_some());
        let postgres = postgres.unwrap();
        assert_eq!(postgres.def.transport, crate::config::McpTransport::Stdio);

        // Check docs (HTTP)
        let docs = registry.mcp("docs")?;
        assert!(docs.is_some());
        let docs = docs.unwrap();
        assert_eq!(docs.def.transport, crate::config::McpTransport::Http);
        assert_eq!(docs.def.url, Some("https://example.com/mcp/".to_string()));

        Ok(())
    }

    #[test]
    fn test_fs_registry_skill_single() -> Result<()> {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog");

        let registry = FsRegistry::new(&fixture_root);
        let skill = registry.skill("web-designer")?;

        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert_eq!(skill.id, "web-designer");
        assert_eq!(
            skill.meta.description,
            Some("Designs responsive web UIs.".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_fs_registry_mcp_single() -> Result<()> {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog");

        let registry = FsRegistry::new(&fixture_root);
        let mcp = registry.mcp("postgres")?;

        assert!(mcp.is_some());
        let mcp = mcp.unwrap();
        assert_eq!(mcp.id, "postgres");
        assert_eq!(mcp.def.command, Some("postgres-mcp".to_string()));

        Ok(())
    }

    #[test]
    fn test_fs_registry_missing_skill() -> Result<()> {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog");

        let registry = FsRegistry::new(&fixture_root);
        let skill = registry.skill("nonexistent")?;

        assert!(skill.is_none());

        Ok(())
    }

    #[test]
    fn test_fs_registry_mcp_expose_skill_and_summary_parse() -> Result<()> {
        let temp_dir = tempfile::TempDir::new()?;
        let root = temp_dir.path();

        fs::create_dir_all(root)?;
        fs::write(
            root.join("catalog.toml"),
            r#"
[[mcp]]
id = "postgres"
transport = "stdio"
command = "postgres-mcp"
expose = "skill"
summary = "Query and inspect a Postgres database."

[[mcp]]
id = "figma"
transport = "stdio"
command = "figma-mcp"
"#,
        )?;

        let registry = FsRegistry::new(root);
        let mcps = registry.mcps()?;

        let postgres = mcps.iter().find(|m| m.id == "postgres").expect("postgres");
        assert_eq!(postgres.expose, McpExpose::Skill);
        assert_eq!(
            postgres.summary.as_deref(),
            Some("Query and inspect a Postgres database.")
        );

        // `expose`/`summary` are optional: an entry that omits them defaults
        // to `tools`/`None`.
        let figma = mcps.iter().find(|m| m.id == "figma").expect("figma");
        assert_eq!(figma.expose, McpExpose::Tools);
        assert!(figma.summary.is_none());

        temp_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_fs_registry_collision_error() -> Result<()> {
        let temp_dir = tempfile::TempDir::new()?;
        let root = temp_dir.path();

        // Create catalog.toml with an MCP
        fs::create_dir_all(root)?;
        fs::write(
            root.join("catalog.toml"),
            r#"
[[mcp]]
id = "duplicate"
transport = "stdio"
command = "cmd1"
"#,
        )?;

        // Create mcp/duplicate.json with the same id
        fs::create_dir_all(root.join("mcp"))?;
        fs::write(root.join("mcp/duplicate.json"), r#"{ "command": "cmd2" }"#)?;

        let registry = FsRegistry::new(root);
        let result = registry.mcps();

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("collision"));

        temp_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_overlay_registry() -> Result<()> {
        let global_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog");
        let project_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog-project");

        let global = FsRegistry::new(&global_root);
        let project = FsRegistry::new(&project_root);
        let overlay = crate::registry::OverlayRegistry::new(global, Some(project));

        // Project override: postgres should resolve to project version (HTTP)
        let postgres = overlay.mcp("postgres")?;
        assert!(postgres.is_some());
        let postgres = postgres.unwrap();
        assert_eq!(postgres.def.transport, crate::config::McpTransport::Http);
        assert_eq!(postgres.def.url, Some("https://project/pg/".to_string()));

        // Project-only skill: deploy should exist
        let deploy = overlay.skill("deploy")?;
        assert!(deploy.is_some());
        assert_eq!(deploy.unwrap().id, "deploy");

        // Global fallthrough: web-designer should exist from global
        let web_designer = overlay.skill("web-designer")?;
        assert!(web_designer.is_some());
        assert_eq!(web_designer.unwrap().id, "web-designer");

        // Union listing should include all unique ids
        let mcps = overlay.mcps()?;
        let ids: Vec<_> = mcps.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["docs", "figma", "postgres"]);

        Ok(())
    }
}
