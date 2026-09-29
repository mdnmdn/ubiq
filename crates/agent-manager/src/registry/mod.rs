//! Catalog (registry) of skills and MCP servers.
//!
//! The catalog is defined as a trait so that embedders can back it with
//! whatever they like (a database, remote service, in-memory map), and the CLI
//! gets a filesystem-backed implementation.
//!
//! Two layers compose: **global** (from `--catalog` / `AM_CATALOG` / the default)
//! and **project** (optional, discovered under `.agent-manager/catalog`). The
//! project layer wins on id collision; otherwise entries fall through to global.

use crate::Result;
use crate::config::McpServer;
use crate::source::Source;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A resolved skill in the catalog: its id, content, and parsed metadata.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// Stable skill identifier (directory name).
    pub id: String,
    /// The skill folder's content (a [`Source::Dir`] for the filesystem
    /// catalog, [`Source::Files`] for a database-backed one). Materialized into
    /// the run's `skills/<id>/` dir by the provisioner.
    pub source: Source,
    /// Parsed metadata from `SKILL.md` frontmatter.
    pub meta: SkillMeta,
    /// Where the skill lives relative to the catalog: copied in, linked, or found in a scanned folder.
    pub origin: SkillOrigin,
}

/// How a skill got into a catalog layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillOrigin {
    /// A copy under `skills/<id>/`; `remote` says where it was fetched from, when it was.
    Installed {
        /// The repository it was installed from (`None` for a hand-made or imported copy).
        remote: Option<RemoteOrigin>,
    },
    /// A `[[skill]]` entry: the folder at this path is referenced in place.
    Linked(PathBuf),
    /// Found by scanning the `[[skill_dir]]` folder at this path.
    Dir(PathBuf),
}

/// Where an installed skill was fetched from (`.am-origin.toml` beside its `SKILL.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteOrigin {
    /// Git URL of the repository.
    pub url: String,
    /// Branch or tag it was cloned at, when one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    /// Folder of the skill inside the repository (`""` for the root).
    pub path: String,
    /// Commit that was installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

/// A place to search and install skills from (`[[skill_source]]` in `catalog.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSource {
    /// Stable id (also the cache folder name); `[A-Za-z0-9._-]+`.
    pub id: String,
    /// Human-readable name.
    #[serde(default)]
    pub label: Option<String>,
    /// What kind of place it is.
    pub kind: SkillSourceKind,
}

/// The kinds of [`SkillSource`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSourceKind {
    /// A git repository scanned for `SKILL.md` files.
    Git {
        /// Clone URL.
        url: String,
        /// Branch or tag (default branch when absent).
        rev: Option<String>,
        /// Only scan under this path of the repository.
        subpath: Option<String>,
    },
    /// An HTTP JSON listing: `[{ "name", "description", "url" (git url), "path", "rev"? }]`
    /// or `{ "skills": [...] }`. Needs the `remote` feature.
    Index {
        /// URL of the listing.
        url: String,
    },
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

/// How a catalog MCP is exposed to the harness.
///
/// `Tools` (the default) is today's behavior: the MCP is injected as a
/// normal, always-on tool set. `Skill` marks intent to *also* generate a
/// latent `SKILL.md` pointer for the MCP (see `_docs/mcp-as-skill.md`)
/// — as of this pass that is a stepping stone only: the MCP still stays
/// injected as normal (the SKILL.md is a documented pointer, not a context
/// savings mechanism yet).
///
/// `#[serde(rename_all = "snake_case")]` so this round-trips as `"tools"` /
/// `"skill"` in `catalog.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpExpose {
    /// Injected as a normal, always-on MCP tool set (today's behavior).
    #[default]
    Tools,
    /// Also generate a latent `SKILL.md` pointer for this MCP.
    Skill,
}

/// A resolved MCP server in the catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEntry {
    /// Stable MCP identifier.
    pub id: String,
    /// MCP server definition.
    pub def: McpServer,
    /// How this MCP is exposed to the harness (`tools` default, or `skill`).
    pub expose: McpExpose,
    /// One-line summary seeding the generated skill's `description:`, when
    /// `expose = "skill"` (`summary` key of a `catalog.toml` entry or an `mcp/*.json` file).
    pub summary: Option<String>,
    /// What the server is for, shown to people browsing the catalog (`description` key).
    pub description: Option<String>,
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

/// The default catalog root: `~/.config/agent-manager/catalog` on all
/// platforms — the same base dir as the config file
/// ([`crate::settings::default_config_dir`]), so the config-like stores live
/// together. Overridable by `AM_CATALOG` (see [`resolve_catalog_root`]).
pub fn default_catalog_root() -> Option<PathBuf> {
    crate::settings::default_config_dir().map(|d| d.join("catalog"))
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
pub(crate) use fs::ORIGIN_FILE;

mod manage;
pub use manage::CatalogStore;

pub mod mcp_parse;
pub use mcp_parse::parse_mcp_config;

#[cfg(feature = "remote")]
pub mod mcp_registry;
#[cfg(feature = "remote")]
pub use mcp_registry::{McpDraft, McpRegistryClient, ParamHint, ParamKind, RegistryServer};

pub mod remote;
pub use remote::{RemoteSkill, default_skill_sources};

/// Whether `id` is a valid catalog id: non-empty, `[A-Za-z0-9._-]+`, not starting with a dot.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A process-wide lock for one path: two writers of the same cache folder or skill id take turns.
pub(crate) fn path_lock(path: &std::path::Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.entry(path.to_path_buf()).or_default().clone()
}

pub mod import;
pub use import::{Action, ImportItem, ImportOptions, ImportPlan, ItemKind, import};
