//! Filesystem-backed registry implementation.
//!
//! Scans a catalog root directory for skills (folders with `SKILL.md` files)
//! and MCP servers (from `catalog.toml` inline definitions and `mcp/*.json` files).

use std::collections::BTreeSet;
use std::path::PathBuf;
use crate::config::McpServer;
use crate::registry::{McpEntry, McpExpose, Registry, SkillEntry, SkillMeta};
use crate::Result;
use anyhow::anyhow;

/// A filesystem-backed registry rooted at a catalog directory.
#[derive(Debug, Clone)]
pub struct FsRegistry {
    root: PathBuf,
}

impl FsRegistry {
    /// Create a registry rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsRegistry {
            root: root.into(),
        }
    }
}

impl Registry for FsRegistry {
    fn skills(&self) -> Result<Vec<SkillEntry>> {
        let skills_dir = self.root.join("skills");

        // Missing skills/ dir is not an error; just return empty.
        if !skills_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();

        for entry in std::fs::read_dir(&skills_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Only process directories
            if !path.is_dir() {
                continue;
            }

            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }

            let id = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Invalid skill directory name"))?;

            let content = std::fs::read_to_string(&skill_md)?;
            let meta = parse_skill_frontmatter(&content);

            entries.push(SkillEntry {
                id,
                path,
                meta,
            });
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }

    fn mcps(&self) -> Result<Vec<McpEntry>> {
        let mut entries = Vec::new();
        let mut seen_ids = BTreeSet::new();

        // First, load inline MCPs from catalog.toml
        let catalog_toml_path = self.root.join("catalog.toml");
        if catalog_toml_path.exists() {
            let content = std::fs::read_to_string(&catalog_toml_path)?;
            let catalog: CatalogToml = toml::from_str(&content)?;

            for entry in catalog.mcp {
                let McpToml {
                    server,
                    expose,
                    summary,
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
                });
            }
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
                let mut server: McpServer = serde_json::from_str(&content)?;
                server.id = id.clone();

                entries.push(McpEntry {
                    id,
                    def: server,
                    // Raw `mcp/*.json` server defs carry no `expose`/`summary`
                    // catalog metadata (that's a `catalog.toml` `[[mcp]]`-only
                    // concept); they always default to `tools`/`None`.
                    expose: McpExpose::Tools,
                    summary: None,
                });
            }
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }
}

/// Parsed structure for catalog.toml; only the `[[mcp]]` sections are relevant.
#[derive(Debug, serde::Deserialize, Default)]
struct CatalogToml {
    /// Inline MCP definitions.
    #[serde(default)]
    mcp: Vec<McpToml>,
}

/// One `[[mcp]]` entry in `catalog.toml`: the raw [`McpServer`] fields
/// (flattened) plus catalog-only metadata (`expose`, `summary`) that only
/// `catalog.toml`-declared MCPs can carry (single-file `mcp/*.json` entries
/// have no room for it).
#[derive(Debug, serde::Deserialize)]
struct McpToml {
    #[serde(flatten)]
    server: McpServer,
    /// `expose = "tools"` (default) | `"skill"`. See [`McpExpose`].
    #[serde(default)]
    expose: McpExpose,
    /// Seeds the generated skill's `description:` when `expose = "skill"`.
    #[serde(default)]
    summary: Option<String>,
}

/// Parse YAML frontmatter from a Markdown file.
///
/// Frontmatter is the block between the first `---` and the next `---`.
/// If no frontmatter exists, returns `SkillMeta::default()`.
fn parse_skill_frontmatter(content: &str) -> SkillMeta {
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
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/catalog");

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
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/catalog");

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
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/catalog");

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
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/catalog");

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
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/catalog");

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
        fs::write(
            root.join("mcp/duplicate.json"),
            r#"{ "command": "cmd2" }"#,
        )?;

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
        let global_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/catalog");
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/catalog-project");

        let global = FsRegistry::new(&global_root);
        let project = FsRegistry::new(&project_root);
        let overlay = crate::registry::OverlayRegistry::new(global, Some(project));

        // Project override: postgres should resolve to project version (HTTP)
        let postgres = overlay.mcp("postgres")?;
        assert!(postgres.is_some());
        let postgres = postgres.unwrap();
        assert_eq!(postgres.def.transport, crate::config::McpTransport::Http);
        assert_eq!(
            postgres.def.url,
            Some("https://project/pg/".to_string())
        );

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
