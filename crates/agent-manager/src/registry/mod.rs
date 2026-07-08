//! Catalog (registry) of skills and MCP servers.
//!
//! The catalog is defined as a trait so that embedders can back it with
//! whatever they like (a database, remote service, in-memory map), and the CLI
//! gets a filesystem-backed implementation.
//!
//! Two layers compose: **global** (from `--catalog` / `AM_CATALOG` / the default)
//! and **project** (optional, discovered under `.agent-manager/catalog`). The
//! project layer wins on id collision; otherwise entries fall through to global.

use std::path::PathBuf;
use crate::config::McpServer;
use crate::Result;

/// A resolved skill in the catalog: its id, folder, and parsed metadata.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// Stable skill identifier (directory name).
    pub id: String,
    /// Path to the skill folder (contains `SKILL.md`).
    pub path: PathBuf,
    /// Parsed metadata from `SKILL.md` frontmatter.
    pub meta: SkillMeta,
}

/// Skill metadata parsed from `SKILL.md` YAML frontmatter (lenient; all optional).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SkillMeta {
    /// `name:` from frontmatter (defaults to the folder name if absent).
    #[serde(default)]
    pub name: Option<String>,
    /// `description:` one-line summary.
    #[serde(default)]
    pub description: Option<String>,
}

/// A resolved MCP server in the catalog.
#[derive(Debug, Clone)]
pub struct McpEntry {
    /// Stable MCP identifier.
    pub id: String,
    /// MCP server definition.
    pub def: McpServer,
}

/// A source of injectable skills and MCP servers, resolved by id.
pub trait Registry {
    /// All skills, sorted by id.
    fn skills(&self) -> Result<Vec<SkillEntry>>;
    /// All MCP servers, sorted by id.
    fn mcps(&self) -> Result<Vec<McpEntry>>;
    /// One skill by exact id.
    fn skill(&self, id: &str) -> Result<Option<SkillEntry>> {
        self.skills()?
            .into_iter()
            .find(|e| e.id == id)
            .map(Some)
            .map(Ok)
            .unwrap_or(Ok(None))
    }
    /// One MCP server by exact id.
    fn mcp(&self, id: &str) -> Result<Option<McpEntry>> {
        self.mcps()?
            .into_iter()
            .find(|e| e.id == id)
            .map(Some)
            .map(Ok)
            .unwrap_or(Ok(None))
    }
}

/// The default catalog root: `<config_dir>/agent-manager/catalog`
/// (e.g. `~/.config/agent-manager/catalog` on Linux, or platform-specific).
/// Uses the `directories` crate to locate the platform's standard config directory.
pub fn default_catalog_root() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "agent-manager")
        .map(|dirs| dirs.config_dir().join("catalog"))
}

/// Resolve the catalog root from (highest first): an explicit path,
/// the `AM_CATALOG` env var, then the default. Returns `None` if none apply.
pub fn resolve_catalog_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit
        .or_else(|| std::env::var("AM_CATALOG").ok().map(PathBuf::from))
        .or_else(default_catalog_root)
}

/// Two catalog layers composed: the project layer wins on id collision,
/// otherwise entries fall through to the global layer.
#[derive(Debug, Clone)]
pub struct OverlayRegistry<G: Registry, P: Registry> {
    /// Global catalog (always present).
    pub global: G,
    /// Project catalog (optional overlay).
    pub project: Option<P>,
}

impl<G: Registry, P: Registry> OverlayRegistry<G, P> {
    /// Create an overlay registry from a global and optional project layer.
    pub fn new(global: G, project: Option<P>) -> Self {
        OverlayRegistry { global, project }
    }
}

impl<G: Registry, P: Registry> Registry for OverlayRegistry<G, P> {
    fn skills(&self) -> Result<Vec<SkillEntry>> {
        let mut result = self.global.skills()?;

        if let Some(ref project) = self.project {
            let project_skills = project.skills()?;
            let mut ids: std::collections::BTreeSet<String> =
                result.iter().map(|e| e.id.clone()).collect();

            for skill in project_skills {
                if !ids.contains(&skill.id) {
                    result.push(skill.clone());
                    ids.insert(skill.id.clone());
                } else {
                    // Replace global with project version
                    result.retain(|e| e.id != skill.id);
                    result.push(skill);
                }
            }
        }

        result.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }

    fn mcps(&self) -> Result<Vec<McpEntry>> {
        let mut result = self.global.mcps()?;

        if let Some(ref project) = self.project {
            let project_mcps = project.mcps()?;
            let mut ids: std::collections::BTreeSet<String> =
                result.iter().map(|e| e.id.clone()).collect();

            for mcp in project_mcps {
                if !ids.contains(&mcp.id) {
                    result.push(mcp.clone());
                    ids.insert(mcp.id.clone());
                } else {
                    // Replace global with project version
                    result.retain(|e| e.id != mcp.id);
                    result.push(mcp);
                }
            }
        }

        result.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(result)
    }

    fn skill(&self, id: &str) -> Result<Option<SkillEntry>> {
        if let Some(ref project) = self.project
            && let Some(skill) = project.skill(id)?
        {
            return Ok(Some(skill));
        }
        self.global.skill(id)
    }

    fn mcp(&self, id: &str) -> Result<Option<McpEntry>> {
        if let Some(ref project) = self.project
            && let Some(mcp) = project.mcp(id)?
        {
            return Ok(Some(mcp));
        }
        self.global.mcp(id)
    }
}

mod fs;
pub use fs::FsRegistry;

pub mod import;
pub use import::{import, Action, ImportItem, ImportOptions, ImportPlan, ItemKind};
