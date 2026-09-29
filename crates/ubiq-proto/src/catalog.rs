//! The skills and MCP servers a user keeps in a catalog — the records the interface lists, edits
//! and picks from, as opposed to the built-in servers [`crate::mcp`] describes.
//!
//! A catalog has two layers: the application's own, and one per project that shadows it by id.
//! What a layer *holds* and how it is stored is the harness library's business; this module only
//! names what crosses the bus. Every path here is a string on the **host's** filesystem — the
//! interface never assumes it can open one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One skill a layer offers, as the interface is told about it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillInfo {
    /// The id a definition's `skills` entry stores.
    pub id: String,
    /// The skill's own display name, from its `SKILL.md` frontmatter.
    #[serde(default)]
    pub name: Option<String>,
    /// One line saying when to use it, from the same frontmatter.
    #[serde(default)]
    pub description: Option<String>,
    /// Where the skill lives, which decides what removing it means.
    pub origin: SkillOriginInfo,
}

/// Where a catalog skill came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillOriginInfo {
    /// A copy in the catalog. `url` is the repository it was installed from, when it was installed
    /// rather than written in place.
    Installed {
        #[serde(default)]
        url: Option<String>,
    },
    /// One skill folder the catalog points at, in place.
    Linked { path: String },
    /// Found inside a folder the catalog scans; removing the folder entry is the only way out.
    Folder { path: String },
}

/// How a catalog MCP server is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpKind {
    /// A local process the harness starts.
    Stdio,
    /// A remote server over streamable HTTP.
    Http,
    /// A remote server over server-sent events.
    Sse,
}

/// One MCP server in a catalog layer. Header and environment values may be credentials the user
/// typed in; they cross the bus as the catalog file holds them, like the file itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMcp {
    /// The id a definition's `mcps` entry stores, alongside the built-in slugs.
    pub id: String,
    pub kind: McpKind,
    /// The program to start, for [`McpKind::Stdio`].
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// The endpoint, for [`McpKind::Http`] and [`McpKind::Sse`].
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// What the server is for, in a line or two.
    #[serde(default)]
    pub description: Option<String>,
}

/// A place skills can be searched for and installed from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSourceInfo {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    pub kind: SkillSourceKindInfo,
}

/// What a [`SkillSourceInfo`] is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSourceKindInfo {
    /// A git repository scanned for `SKILL.md` files.
    Git {
        url: String,
        #[serde(default)]
        rev: Option<String>,
        /// Only scan under this path in the repository.
        #[serde(default)]
        subpath: Option<String>,
    },
    /// An HTTP JSON listing of skills.
    Index { url: String },
}

/// One skill a source offers, not yet installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSkillInfo {
    /// The [`SkillSourceInfo::id`] it was found in.
    pub source: String,
    /// The id it installs under by default.
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Where it sits in the source; [`SkillAdd::Remote::path`] names it back.
    pub path: String,
}

/// What kind of value a [`McpParamHint`] asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpParamKind {
    Env,
    Header,
    Arg,
}

/// A value a registry entry says the user has to supply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpParamHint {
    pub name: String,
    pub kind: McpParamKind,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: bool,
    /// The value is a credential; a form should not echo it.
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub default: Option<String>,
}

/// One way to run a registry server: a draft the form fills from, with what still needs a value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpDraftInfo {
    /// What this option is, e.g. `npx some-server` or `remote: streamable-http`.
    pub label: String,
    pub server: CatalogMcp,
    #[serde(default)]
    pub params: Vec<McpParamHint>,
}

/// One server the public MCP registry lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryMcpInfo {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub options: Vec<McpDraftInfo>,
}

/// How a skill is added to a layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillAdd {
    /// Point at one skill folder on the host, in place.
    Link {
        path: String,
        #[serde(default)]
        id: Option<String>,
    },
    /// Scan a folder on the host: every subfolder with a `SKILL.md` is a skill.
    Folder { path: String },
    /// Install a skill a source offers.
    Remote {
        /// The [`SkillSourceInfo::id`].
        source: String,
        /// The [`RemoteSkillInfo::path`].
        path: String,
        #[serde(default)]
        id: Option<String>,
    },
}
