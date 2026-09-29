//! The skills and MCP servers catalog, as the window holds it.
//!
//! The host keeps one layer for the application and one per project; each `Catalog` message is
//! one layer, whole, and replaces what was held for that scope. The layers are **not** merged
//! here on arrival: a page for one layer draws exactly that layer, and a surface that offers
//! "what a start in this project may use" asks [`CatalogState::skills_in`] /
//! [`CatalogState::mcps_in`], which is the interface's copy of the rule the host launches by
//! (the project's entry shadows the application's of the same id).
//!
//! Also here: the small state of the four modals the pages raise, and the pure text-to-record
//! conversions the MCP form needs, so they can be tested without a window.

use std::collections::{BTreeMap, HashMap};

use ubiq_proto::catalog::{
    CatalogMcp, McpKind, McpParamHint, RegistryMcpInfo, RemoteSkillInfo, SkillInfo, SkillSourceInfo,
};
use ubiq_proto::ids::ProjectId;

/// One catalog layer, as the host last said it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogLayer {
    pub skills: Vec<SkillInfo>,
    pub mcps: Vec<CatalogMcp>,
    /// The folders this layer scans for skills, as the host spells them.
    pub skill_folders: Vec<String>,
    /// Where skills can be searched for. Empty on a project layer.
    pub sources: Vec<SkillSourceInfo>,
}

/// Everything the catalog pages and their modals hold.
#[derive(Clone, Debug, Default)]
pub struct CatalogState {
    /// The application's own layer.
    pub global: CatalogLayer,
    /// One layer per project, for the projects the window has asked about.
    pub projects: HashMap<ProjectId, CatalogLayer>,
    /// What the host last refused, in its words. Cleared by the user, or by the next request.
    pub error: Option<String>,
    /// The repository browser, while it is up.
    pub browse: Option<SkillBrowse>,
    /// Whether the "Add source" question is up.
    pub add_source: bool,
    /// The add-or-edit MCP server form, while it is up.
    pub form: Option<McpForm>,
    /// The MCP registry search, while it is up.
    pub registry: Option<RegistryBrowse>,
}

impl CatalogState {
    /// The layer a scope names, when the window has one. The application's always exists.
    pub fn layer(&self, scope: Option<ProjectId>) -> Option<&CatalogLayer> {
        match scope {
            None => Some(&self.global),
            Some(project) => self.projects.get(&project),
        }
    }

    /// Replace one layer whole.
    pub fn replace(&mut self, scope: Option<ProjectId>, layer: CatalogLayer) {
        match scope {
            None => self.global = layer,
            Some(project) => {
                self.projects.insert(project, layer);
            }
        }
    }

    /// The skills a start in `project` may name: that project's own first, then every
    /// application skill it does not shadow by id. `None` is the application's alone.
    pub fn skills_in(&self, project: Option<ProjectId>) -> Vec<SkillInfo> {
        let mut offered = project
            .and_then(|it| self.projects.get(&it))
            .map(|layer| layer.skills.clone())
            .unwrap_or_default();
        let inherited: Vec<SkillInfo> = self
            .global
            .skills
            .iter()
            .filter(|skill| !offered.iter().any(|it| it.id == skill.id))
            .cloned()
            .collect();
        offered.extend(inherited);
        offered
    }

    /// The MCP servers a start in `project` may name, on [`Self::skills_in`]'s terms.
    pub fn mcps_in(&self, project: Option<ProjectId>) -> Vec<CatalogMcp> {
        let mut offered = project
            .and_then(|it| self.projects.get(&it))
            .map(|layer| layer.mcps.clone())
            .unwrap_or_default();
        let inherited: Vec<CatalogMcp> = self
            .global
            .mcps
            .iter()
            .filter(|mcp| !offered.iter().any(|it| it.id == mcp.id))
            .cloned()
            .collect();
        offered.extend(inherited);
        offered
    }
}

/// The repository browser: search the configured sources, install a result into one layer.
#[derive(Clone, Debug)]
pub struct SkillBrowse {
    /// The layer an install lands in.
    pub scope: Option<ProjectId>,
    /// One source's id, or `None` for all of them.
    pub source: Option<String>,
    /// The query the current results answer. A result for any other query is stale and dropped.
    pub asked: String,
    pub results: Vec<RemoteSkillInfo>,
    /// One line per source the host could not read.
    pub problems: Vec<String>,
    /// A search is in flight.
    pub loading: bool,
    /// The `source/path` of the install in flight, if any.
    pub installing: Option<String>,
}

impl SkillBrowse {
    pub fn new(scope: Option<ProjectId>) -> Self {
        Self {
            scope,
            source: None,
            asked: String::new(),
            results: Vec::new(),
            problems: Vec::new(),
            loading: false,
            installing: None,
        }
    }

    /// The key an install in flight is held under, and a result row is compared against.
    pub fn key(result: &RemoteSkillInfo) -> String {
        format!("{}/{}", result.source, result.path)
    }
}

/// The add-or-edit MCP server form. The typed fields live in inputs on the window; this holds
/// the choices and what came back from the host.
#[derive(Clone, Debug, Default)]
pub struct McpForm {
    /// The layer it is written to.
    pub scope: Option<ProjectId>,
    /// The id it was listed under, when this is an edit — so a rename removes the old entry.
    pub previous_id: Option<String>,
    /// Remote (a URL) rather than local (a command).
    pub remote: bool,
    /// For a remote one: server-sent events rather than streamable HTTP.
    pub sse: bool,
    /// A save is in flight.
    pub saving: bool,
    /// A paste is being read by the host.
    pub parsing: bool,
    /// What a registry pick still needs a value for.
    pub hints: Vec<McpParamHint>,
    /// Several servers found in a paste: pick one to fill the form, or add them all.
    pub picks: Vec<CatalogMcp>,
    /// Why a paste could not be read, or the host's refusal of the save.
    pub error: Option<String>,
    /// Text waiting to be written into the form's boxes — what a fill from a paste, a registry
    /// pick or an edit's record leaves for the next frame, which has a window to write with.
    pub fill: Option<McpFields>,
}

impl McpForm {
    pub fn new(scope: Option<ProjectId>) -> Self {
        Self {
            scope,
            ..Self::default()
        }
    }

    /// The kind the choice pills say.
    pub fn kind(&self) -> McpKind {
        match (self.remote, self.sse) {
            (false, _) => McpKind::Stdio,
            (true, false) => McpKind::Http,
            (true, true) => McpKind::Sse,
        }
    }

    /// Point the pills at a record's kind.
    pub fn set_kind(&mut self, kind: McpKind) {
        self.remote = kind != McpKind::Stdio;
        self.sse = kind == McpKind::Sse;
    }
}

/// The MCP registry search.
#[derive(Clone, Debug, Default)]
pub struct RegistryBrowse {
    /// The layer a pick is written to.
    pub scope: Option<ProjectId>,
    /// The query the current results answer.
    pub asked: String,
    pub results: Vec<RegistryMcpInfo>,
    pub next_cursor: Option<String>,
    pub error: Option<String>,
    pub loading: bool,
    /// The request in flight continues a listing rather than starting one.
    pub appending: bool,
}

/// The text a form's boxes hold, for one record — what filling the form from a record writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpFields {
    pub id: String,
    pub command: String,
    pub args: String,
    pub env: String,
    pub url: String,
    pub headers: String,
    pub description: String,
}

/// A record as the boxes spell it: arguments one per line, `KEY=VALUE` and `Name: value` lines.
pub fn mcp_fields(mcp: &CatalogMcp) -> McpFields {
    McpFields {
        id: mcp.id.clone(),
        command: mcp.command.clone().unwrap_or_default(),
        args: mcp.args.join("\n"),
        env: mcp
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n"),
        url: mcp.url.clone().unwrap_or_default(),
        headers: mcp
            .headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect::<Vec<_>>()
            .join("\n"),
        description: mcp.description.clone().unwrap_or_default(),
    }
}

/// The record the boxes spell, of the kind the pills say. The fields the kind does not use are
/// left empty rather than carried along unseen.
pub fn compose_mcp(kind: McpKind, fields: &McpFields) -> CatalogMcp {
    let local = kind == McpKind::Stdio;
    let some = |text: &str| Some(text.trim().to_string()).filter(|it| !it.is_empty());
    CatalogMcp {
        id: fields.id.trim().to_string(),
        kind,
        command: if local { some(&fields.command) } else { None },
        args: if local {
            fields
                .args
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect()
        } else {
            Vec::new()
        },
        env: if local {
            pairs(&fields.env, '=')
        } else {
            BTreeMap::new()
        },
        url: if local { None } else { some(&fields.url) },
        headers: if local {
            BTreeMap::new()
        } else {
            pairs(&fields.headers, ':')
        },
        description: some(&fields.description),
    }
}

/// `KEY=VALUE` or `Name: value` per line. A line with no separator or an empty key is dropped;
/// the value keeps everything after the first separator.
fn pairs(text: &str, separator: char) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(separator)?;
            let key = key.trim();
            (!key.is_empty()).then(|| (key.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Whether a record has what its kind needs to run at all.
pub fn mcp_ready(mcp: &CatalogMcp) -> bool {
    !mcp.id.is_empty()
        && match mcp.kind {
            McpKind::Stdio => mcp.command.is_some(),
            McpKind::Http | McpKind::Sse => mcp.url.is_some(),
        }
}

/// The one line a list row says a server is: the command with its arguments, or the URL.
pub fn mcp_summary(mcp: &CatalogMcp) -> String {
    match mcp.kind {
        McpKind::Stdio => std::iter::once(mcp.command.as_deref().unwrap_or_default())
            .chain(mcp.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        McpKind::Http | McpKind::Sse => mcp.url.clone().unwrap_or_default(),
    }
}

/// The word a kind badge carries.
pub fn kind_label(kind: McpKind) -> &'static str {
    match kind {
        McpKind::Stdio => "Local",
        McpKind::Http => "HTTP",
        McpKind::Sse => "SSE",
    }
}

/// An id for a registry server named `io.github.org/some-server`: the part after the last slash,
/// with anything a file name should not carry turned into `-`.
pub fn registry_id(name: &str) -> String {
    let tail = name.rsplit('/').next().unwrap_or(name);
    tail.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// A source id for a repository URL: the last two path segments, lower-cased and joined with `-`
/// (`https://github.com/org/skills.git` becomes `org-skills`). Not a name anyone typed, so it
/// only has to be stable and unique enough; the caller de-duplicates.
pub fn source_id(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let tail: Vec<&str> = trimmed
        .rsplit(['/', ':'])
        .filter(|it| !it.is_empty())
        .take(2)
        .collect();
    let id = tail
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    if id.is_empty() {
        "source".to_string()
    } else {
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::catalog::SkillOriginInfo;

    fn skill(id: &str) -> SkillInfo {
        SkillInfo {
            id: id.to_string(),
            name: None,
            description: None,
            origin: SkillOriginInfo::Installed { url: None },
        }
    }

    fn server(id: &str) -> CatalogMcp {
        CatalogMcp {
            id: id.to_string(),
            kind: McpKind::Stdio,
            command: Some("npx".to_string()),
            args: vec!["-y".to_string(), "pkg".to_string()],
            env: BTreeMap::from([("KEY".to_string(), "v=1".to_string())]),
            url: None,
            headers: BTreeMap::new(),
            description: None,
        }
    }

    #[test]
    fn a_project_layer_shadows_the_application_one_by_id() {
        let project = ProjectId::generate();
        let mut catalog = CatalogState::default();
        catalog.replace(
            None,
            CatalogLayer {
                skills: vec![skill("a"), skill("b")],
                mcps: vec![server("x")],
                ..CatalogLayer::default()
            },
        );
        catalog.replace(
            Some(project),
            CatalogLayer {
                skills: vec![skill("b"), skill("c")],
                mcps: vec![server("x"), server("y")],
                ..CatalogLayer::default()
            },
        );
        let ids = |skills: Vec<SkillInfo>| skills.into_iter().map(|it| it.id).collect::<Vec<_>>();
        assert_eq!(ids(catalog.skills_in(None)), ["a", "b"]);
        assert_eq!(ids(catalog.skills_in(Some(project))), ["b", "c", "a"]);
        assert_eq!(catalog.mcps_in(Some(project)).len(), 2, "x is not doubled");
        // A project nothing was heard about offers the application's.
        assert_eq!(
            ids(catalog.skills_in(Some(ProjectId::generate()))),
            ["a", "b"]
        );
    }

    #[test]
    fn a_catalog_message_replaces_its_layer_whole() {
        let mut catalog = CatalogState::default();
        catalog.replace(
            None,
            CatalogLayer {
                skills: vec![skill("a"), skill("b")],
                ..CatalogLayer::default()
            },
        );
        catalog.replace(
            None,
            CatalogLayer {
                skills: vec![skill("b")],
                ..CatalogLayer::default()
            },
        );
        assert_eq!(catalog.layer(None).unwrap().skills.len(), 1);
        assert!(catalog.layer(Some(ProjectId::generate())).is_none());
    }

    #[test]
    fn a_record_round_trips_through_the_boxes() {
        let mcp = server("files");
        let fields = mcp_fields(&mcp);
        assert_eq!(fields.args, "-y\npkg");
        assert_eq!(fields.env, "KEY=v=1", "the value keeps its own =");
        assert_eq!(compose_mcp(McpKind::Stdio, &fields), mcp);
    }

    #[test]
    fn a_remote_record_leaves_the_local_fields_out() {
        let fields = McpFields {
            id: " docs ".to_string(),
            command: "ignored".to_string(),
            args: "ignored".to_string(),
            env: "A=b".to_string(),
            url: " https://x.test/mcp ".to_string(),
            headers: "Authorization: Bearer t\nbad line\n: no key".to_string(),
            description: "  ".to_string(),
        };
        let mcp = compose_mcp(McpKind::Sse, &fields);
        assert_eq!(mcp.id, "docs");
        assert_eq!(mcp.command, None);
        assert!(mcp.args.is_empty() && mcp.env.is_empty());
        assert_eq!(mcp.url.as_deref(), Some("https://x.test/mcp"));
        assert_eq!(mcp.headers.len(), 1);
        assert_eq!(mcp.headers["Authorization"], "Bearer t");
        assert_eq!(mcp.description, None);
        assert!(mcp_ready(&mcp));
        let bare = McpFields {
            id: "x".to_string(),
            ..McpFields::default()
        };
        assert!(
            !mcp_ready(&compose_mcp(McpKind::Stdio, &bare)),
            "no command"
        );
        assert!(!mcp_ready(&compose_mcp(McpKind::Http, &bare)), "no URL");
    }

    #[test]
    fn the_pills_and_the_kind_agree() {
        let mut form = McpForm::new(None);
        assert_eq!(form.kind(), McpKind::Stdio);
        form.set_kind(McpKind::Sse);
        assert!(form.remote && form.sse);
        assert_eq!(form.kind(), McpKind::Sse);
        form.set_kind(McpKind::Http);
        assert_eq!(form.kind(), McpKind::Http);
    }

    #[test]
    fn a_source_id_comes_from_the_repository_url() {
        assert_eq!(source_id("https://github.com/Org/Skills.git"), "org-skills");
        assert_eq!(source_id("git@github.com:org/skills/"), "org-skills");
        assert_eq!(source_id(""), "source");
    }

    #[test]
    fn a_registry_name_becomes_a_file_safe_id() {
        assert_eq!(registry_id("io.github.org/Some Server"), "Some-Server");
        assert_eq!(registry_id("plain"), "plain");
    }
}
