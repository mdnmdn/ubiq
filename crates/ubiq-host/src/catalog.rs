//! The skill and MCP catalog: the library's `FsRegistry` at the config root, one layer per project,
//! overlaid at launch (`D196`).
//!
//! What a layer *holds* and how it is stored — `catalog.toml`, `mcp/<id>.json`, `skills/<id>/` —
//! is the library's business (`agent_manager::registry`). This module is the seam: it picks which
//! directory a [`ubiq_proto::messages::Message`] scope names, turns the library's records into the
//! wire's and back, and owns the two things the library cannot know — the remote-skill cache
//! under the config root, and that an MCP id may not shadow one of Ubiq's own built-in servers.
//!
//! Nothing here renders anything or names a harness. Everything that can touch the network
//! ([`Catalog::install_remote`], [`Catalog::search_skills`], [`Catalog::search_mcp_registry`]) is a
//! plain blocking call: the coordinator runs those on a thread of their own.

use std::path::{Path, PathBuf};

use agent_manager::config::{McpServer, McpTransport};
use agent_manager::registry::remote::{self, RemoteSkill};
use agent_manager::registry::{
    CatalogStore, FsRegistry, McpEntry, McpExpose, OverlayRegistry, Registry, SkillOrigin,
    SkillSource, SkillSourceKind, parse_mcp_config, valid_id,
};
use anyhow::{Context, Result, anyhow, bail};
use ubiq_proto::catalog::{
    CatalogMcp, McpDraftInfo, McpKind, McpParamHint, McpParamKind, RegistryMcpInfo,
    RemoteSkillInfo, SkillAdd, SkillInfo, SkillOriginInfo, SkillSourceInfo, SkillSourceKindInfo,
};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;

use crate::store::project_dir::ProjectData;

/// A catalog layer's directory: under the config root for the application's own layer, under a
/// project's data directory for that project's.
pub const CATALOG_DIR: &str = "catalog";

/// Where the remote-skill sources are cloned, relative to the config root. Derived data: deleting
/// it costs a fetch.
pub const CACHE_DIR: &str = "cache/skill-sources";

/// How many servers one registry search asks for.
const REGISTRY_PAGE: usize = 30;

/// The catalog under one config root. A path and nothing else, so it is cheap to clone into a
/// worker thread.
#[derive(Clone, Debug)]
pub struct Catalog {
    root: PathBuf,
}

impl Catalog {
    /// The catalog under the config `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// One layer: the application's own for `None`, that project's for `Some`.
    ///
    /// A project's layer lives under `<root>/projects/<id>/`, beside its definitions, so Forget
    /// removes it without knowing it is there.
    pub fn layer(&self, scope: Option<ProjectId>) -> FsRegistry {
        match scope {
            None => FsRegistry::new(self.root.join(CATALOG_DIR)),
            Some(project) => {
                FsRegistry::new(ProjectData::under_config(&self.root, project).catalog())
            }
        }
    }

    /// What a run in `project` resolves ids against: that project's layer over the application's.
    pub fn registry_for(
        &self,
        project: Option<ProjectId>,
    ) -> OverlayRegistry<FsRegistry, FsRegistry> {
        OverlayRegistry::new(self.layer(None), project.map(|it| self.layer(Some(it))))
    }

    /// Where the skill sources are cloned.
    pub fn cache(&self) -> PathBuf {
        self.root.join(CACHE_DIR)
    }

    /// One layer, whole, as [`Message::Catalog`]. `sources` is the application's list and empty
    /// for a project's layer.
    pub fn snapshot(&self, scope: Option<ProjectId>) -> Result<Message> {
        let layer = self.layer(scope);
        let skills = layer
            .skills()
            .context("reading the skills")?
            .into_iter()
            .map(|entry| SkillInfo {
                id: entry.id,
                name: entry.meta.name,
                description: entry.meta.description,
                origin: match entry.origin {
                    SkillOrigin::Installed { remote } => SkillOriginInfo::Installed {
                        url: remote.map(|origin| origin.url),
                    },
                    SkillOrigin::Linked(path) => SkillOriginInfo::Linked { path: text(&path) },
                    SkillOrigin::Dir(path) => SkillOriginInfo::Folder { path: text(&path) },
                },
            })
            .collect();
        let mcps = layer
            .mcps()
            .context("reading the MCP servers")?
            .into_iter()
            .map(|entry| mcp_info(&entry))
            .collect();
        let skill_folders = layer
            .skill_dirs()
            .context("reading the skill folders")?
            .iter()
            .map(|path| text(path))
            .collect();
        let sources = match scope {
            None => layer
                .skill_sources()
                .context("reading the skill sources")?
                .iter()
                .map(source_info)
                .collect(),
            Some(_) => Vec::new(),
        };
        Ok(Message::Catalog {
            scope,
            skills,
            mcps,
            skill_folders,
            sources,
        })
    }

    /// Link a skill folder or register a folder to scan. A source install is
    /// [`Self::install_remote`], which fetches and so belongs on a thread of its own.
    pub fn add_skill(&self, scope: Option<ProjectId>, from: &SkillAdd) -> Result<()> {
        let layer = self.layer(scope);
        match from {
            SkillAdd::Link { path, id } => {
                layer.link_skill(Path::new(path), id.as_deref().filter(|id| !id.is_empty()))?;
            }
            SkillAdd::Folder { path } => layer.add_skill_dir(Path::new(path))?,
            SkillAdd::Remote { .. } => bail!("a remote skill is installed with `install_remote`"),
        }
        Ok(())
    }

    /// Install the skill at `path` in source `source` into a layer. Blocking: clones or fetches.
    ///
    /// The sources are the application's whichever layer is written; a project has none of its own.
    pub fn install_remote(
        &self,
        scope: Option<ProjectId>,
        source: &str,
        path: &str,
        id: Option<&str>,
    ) -> Result<()> {
        let src = self.source(source)?;
        let cache = self.cache();
        let skill = remote::list_source(&src, &cache)
            .with_context(|| format!("reading source '{source}'"))?
            .into_iter()
            .find(|skill| skill.path == path)
            .ok_or_else(|| anyhow!("source '{source}' offers no skill at '{path}'"))?;
        remote::install_remote(
            &self.layer(scope),
            &src,
            &cache,
            &skill,
            id.filter(|id| !id.is_empty()),
        )?;
        Ok(())
    }

    /// Take a skill out of a layer. A skill found by scanning a folder is refused by the library.
    pub fn remove_skill(&self, scope: Option<ProjectId>, id: &str) -> Result<()> {
        self.layer(scope).remove_skill(id)
    }

    /// Stop scanning `path` for skills.
    pub fn remove_skill_folder(&self, scope: Option<ProjectId>, path: &str) -> Result<()> {
        self.layer(scope).remove_skill_dir(Path::new(path))
    }

    /// Replace the application's list of skill sources.
    pub fn save_sources(&self, sources: &[SkillSourceInfo]) -> Result<()> {
        let sources: Vec<SkillSource> = sources.iter().map(source_of).collect();
        self.layer(None).set_skill_sources(&sources)
    }

    /// Write one MCP server into a layer, dropping `previous_id`'s entry when this is a rename.
    ///
    /// The id must be a name a file can carry and must not be one of Ubiq's own built-in servers:
    /// both share one `mcps` list on a definition, and a catalog entry named like a built-in would
    /// never be reached, because the built-in answers first (`compose_run`).
    pub fn save_mcp(
        &self,
        scope: Option<ProjectId>,
        mcp: &CatalogMcp,
        previous_id: Option<&str>,
    ) -> Result<()> {
        if mcp.id.is_empty() {
            bail!("an MCP server needs an id");
        }
        if !valid_id(&mcp.id) {
            bail!(
                "'{}' is not a usable id: use letters, digits, '.', '_' and '-'",
                mcp.id
            );
        }
        if crate::mcp::knows(&mcp.id) {
            bail!("'{}' is the name of a built-in Ubiq server", mcp.id);
        }
        let layer = self.layer(scope);
        // What the form does not edit — how the server is exposed, its skill summary — is kept.
        let kept = layer.mcp(previous_id.unwrap_or(&mcp.id))?;
        let entry = McpEntry {
            id: mcp.id.clone(),
            def: McpServer {
                id: mcp.id.clone(),
                transport: match mcp.kind {
                    McpKind::Stdio => McpTransport::Stdio,
                    McpKind::Http => McpTransport::Http,
                    McpKind::Sse => McpTransport::Sse,
                },
                command: mcp.command.clone().filter(|it| !it.is_empty()),
                args: mcp.args.clone(),
                env: mcp.env.clone(),
                url: mcp.url.clone().filter(|it| !it.is_empty()),
                headers: mcp.headers.clone(),
            },
            expose: kept.as_ref().map_or(McpExpose::Tools, |it| it.expose),
            summary: kept.and_then(|it| it.summary),
            description: mcp.description.clone().filter(|it| !it.is_empty()),
        };
        layer.put_mcp(&entry)?;
        if let Some(previous) = previous_id
            && previous != mcp.id
        {
            layer.remove_mcp(previous)?;
        }
        Ok(())
    }

    /// Delete one MCP server from a layer.
    pub fn remove_mcp(&self, scope: Option<ProjectId>, id: &str) -> Result<()> {
        self.layer(scope).remove_mcp(id)
    }

    /// Search the configured sources, or just `source`. Blocking: clones or fetches.
    pub fn search_skills(&self, query: &str, source: Option<&str>) -> Message {
        let mut problems = Vec::new();
        let sources = match self.layer(None).skill_sources() {
            Ok(all) => all
                .into_iter()
                .filter(|it| source.is_none_or(|wanted| it.id == wanted))
                .collect::<Vec<_>>(),
            Err(error) => {
                problems.push(format!("{error:#}"));
                Vec::new()
            }
        };
        if let Some(wanted) = source
            && sources.is_empty()
            && problems.is_empty()
        {
            problems.push(format!("no source named '{wanted}'"));
        }
        let (found, failed) = remote::search(&sources, &self.cache(), query);
        problems.extend(failed);
        Message::SkillSearchResults {
            query: query.to_string(),
            results: found.into_iter().map(remote_skill_info).collect(),
            problems,
        }
    }

    /// Read a pasted MCP configuration. Nothing is stored.
    pub fn parse_config(text: &str) -> Message {
        match parse_mcp_config(text) {
            Ok(servers) => Message::McpConfigParsed {
                servers: servers.iter().map(server_info).collect(),
                error: None,
            },
            Err(error) => Message::McpConfigParsed {
                servers: Vec::new(),
                error: Some(format!("{error:#}")),
            },
        }
    }

    /// Search the public MCP registry. Blocking: one HTTP request.
    pub fn search_mcp_registry(query: &str, cursor: Option<&str>) -> Message {
        let client = agent_manager::registry::McpRegistryClient::default();
        match client.search(query, REGISTRY_PAGE, cursor.filter(|it| !it.is_empty())) {
            Ok((servers, next_cursor)) => Message::McpRegistryResults {
                query: query.to_string(),
                servers: servers.into_iter().map(registry_info).collect(),
                next_cursor,
                error: None,
            },
            Err(error) => Message::McpRegistryResults {
                query: query.to_string(),
                servers: Vec::new(),
                next_cursor: None,
                error: Some(format!("{error:#}")),
            },
        }
    }

    fn source(&self, id: &str) -> Result<SkillSource> {
        self.layer(None)
            .skill_sources()?
            .into_iter()
            .find(|it| it.id == id)
            .ok_or_else(|| anyhow!("no skill source named '{id}'"))
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn kind_info(transport: McpTransport) -> McpKind {
    match transport {
        McpTransport::Stdio => McpKind::Stdio,
        McpTransport::Http => McpKind::Http,
        McpTransport::Sse => McpKind::Sse,
    }
}

fn server_info(server: &McpServer) -> CatalogMcp {
    CatalogMcp {
        id: server.id.clone(),
        kind: kind_info(server.transport),
        command: server.command.clone(),
        args: server.args.clone(),
        env: server.env.clone(),
        url: server.url.clone(),
        headers: server.headers.clone(),
        description: None,
    }
}

fn mcp_info(entry: &McpEntry) -> CatalogMcp {
    CatalogMcp {
        description: entry.description.clone(),
        ..server_info(&entry.def)
    }
}

fn source_info(source: &SkillSource) -> SkillSourceInfo {
    SkillSourceInfo {
        id: source.id.clone(),
        label: source.label.clone(),
        kind: match &source.kind {
            SkillSourceKind::Git { url, rev, subpath } => SkillSourceKindInfo::Git {
                url: url.clone(),
                rev: rev.clone(),
                subpath: subpath.clone(),
            },
            SkillSourceKind::Index { url } => SkillSourceKindInfo::Index { url: url.clone() },
        },
    }
}

fn source_of(info: &SkillSourceInfo) -> SkillSource {
    let blank = |it: &Option<String>| it.clone().filter(|it| !it.trim().is_empty());
    SkillSource {
        id: info.id.clone(),
        label: blank(&info.label),
        kind: match &info.kind {
            SkillSourceKindInfo::Git { url, rev, subpath } => SkillSourceKind::Git {
                url: url.clone(),
                rev: blank(rev),
                subpath: blank(subpath),
            },
            SkillSourceKindInfo::Index { url } => SkillSourceKind::Index { url: url.clone() },
        },
    }
}

fn remote_skill_info(skill: RemoteSkill) -> RemoteSkillInfo {
    RemoteSkillInfo {
        source: skill.source,
        id: skill.id,
        name: skill.name,
        description: skill.description,
        path: skill.path,
    }
}

fn registry_info(server: agent_manager::registry::RegistryServer) -> RegistryMcpInfo {
    use agent_manager::registry::ParamKind;
    RegistryMcpInfo {
        name: server.name,
        title: server.title,
        description: server.description,
        version: server.version,
        repository: server.repository,
        options: server
            .options
            .into_iter()
            .map(|draft| McpDraftInfo {
                label: draft.label,
                server: server_info(&draft.server),
                params: draft
                    .params
                    .into_iter()
                    .map(|hint| McpParamHint {
                        name: hint.name,
                        kind: match hint.kind {
                            ParamKind::Env => McpParamKind::Env,
                            ParamKind::Header => McpParamKind::Header,
                            ParamKind::Arg => McpParamKind::Arg,
                        },
                        description: hint.description,
                        required: hint.required,
                        secret: hint.secret,
                        default: hint.default,
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn skill_folder(parent: &Path, name: &str, description: &str) -> PathBuf {
        let dir = parent.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\nbody\n"),
        )
        .unwrap();
        dir
    }

    fn server(id: &str) -> CatalogMcp {
        CatalogMcp {
            id: id.to_string(),
            kind: McpKind::Stdio,
            command: Some("npx".into()),
            args: vec!["-y".into(), "thing".into()],
            env: BTreeMap::from([("TOKEN".to_string(), "x".to_string())]),
            url: None,
            headers: BTreeMap::new(),
            description: Some("does a thing".into()),
        }
    }

    fn layer_of(message: Message) -> (Vec<SkillInfo>, Vec<CatalogMcp>, Vec<String>, usize) {
        let Message::Catalog {
            skills,
            mcps,
            skill_folders,
            sources,
            ..
        } = message
        else {
            panic!("not a catalog: {message:?}");
        };
        (skills, mcps, skill_folders, sources.len())
    }

    #[test]
    fn a_layer_round_trips_skills_folders_and_mcps() {
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let catalog = Catalog::new(root.path());
        let linked = skill_folder(elsewhere.path(), "pdf", "reads pdfs");
        let scanned = elsewhere.path().join("many");
        skill_folder(&scanned, "docx", "writes docs");

        catalog
            .add_skill(
                None,
                &SkillAdd::Link {
                    path: text(&linked),
                    id: None,
                },
            )
            .unwrap();
        catalog
            .add_skill(
                None,
                &SkillAdd::Folder {
                    path: text(&scanned),
                },
            )
            .unwrap();
        catalog.save_mcp(None, &server("things"), None).unwrap();

        let (skills, mcps, folders, sources) = layer_of(catalog.snapshot(None).unwrap());
        let by_id = |id: &str| skills.iter().find(|it| it.id == id).unwrap();
        assert_eq!(by_id("pdf").description.as_deref(), Some("reads pdfs"));
        assert!(matches!(
            by_id("pdf").origin,
            SkillOriginInfo::Linked { .. }
        ));
        assert!(matches!(
            by_id("docx").origin,
            SkillOriginInfo::Folder { .. }
        ));
        assert_eq!(
            mcps,
            vec![server("things")],
            "the server came back as saved"
        );
        assert_eq!(folders, vec![text(&scanned)]);
        assert!(
            sources > 0,
            "the application's layer carries the default sources"
        );

        // A scanned skill cannot be removed one by one; the folder goes instead.
        assert!(catalog.remove_skill(None, "docx").is_err());
        catalog.remove_skill_folder(None, &text(&scanned)).unwrap();
        catalog.remove_skill(None, "pdf").unwrap();
        catalog.remove_mcp(None, "things").unwrap();
        let (skills, mcps, folders, _) = layer_of(catalog.snapshot(None).unwrap());
        assert!(skills.is_empty() && mcps.is_empty() && folders.is_empty());
    }

    #[test]
    fn a_project_layer_is_its_own_and_carries_no_sources() {
        let root = TempDir::new().unwrap();
        let catalog = Catalog::new(root.path());
        let project = ProjectId::generate();
        catalog
            .save_mcp(Some(project), &server("only-here"), None)
            .unwrap();

        let (_, mcps, _, sources) = layer_of(catalog.snapshot(Some(project)).unwrap());
        assert_eq!(mcps.len(), 1);
        assert_eq!(sources, 0, "sources are the application's alone");
        let (_, global, _, _) = layer_of(catalog.snapshot(None).unwrap());
        assert!(
            global.is_empty(),
            "the project's server is not the application's"
        );
        assert!(
            ProjectData::under_config(root.path(), project)
                .catalog()
                .join("mcp/only-here.json")
                .is_file()
        );
    }

    #[test]
    fn a_built_in_slug_a_blank_id_and_a_bad_name_are_refused() {
        let root = TempDir::new().unwrap();
        let catalog = Catalog::new(root.path());
        let built_in = crate::mcp::catalogue()
            .into_iter()
            .next()
            .expect("the build offers a built-in server")
            .name;
        for id in [built_in.as_str(), "", "has space", "../up"] {
            assert!(
                catalog.save_mcp(None, &server(id), None).is_err(),
                "'{id}' should be refused"
            );
        }
        let (_, mcps, _, _) = layer_of(catalog.snapshot(None).unwrap());
        assert!(mcps.is_empty(), "nothing was written");
    }

    #[test]
    fn a_rename_moves_the_entry_and_keeps_what_the_form_does_not_edit() {
        let root = TempDir::new().unwrap();
        let catalog = Catalog::new(root.path());
        let layer = catalog.layer(None);
        layer
            .put_mcp(&McpEntry {
                id: "old".into(),
                def: McpServer {
                    id: "old".into(),
                    transport: McpTransport::Stdio,
                    command: Some("x".into()),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    url: None,
                    headers: BTreeMap::new(),
                },
                expose: McpExpose::Skill,
                summary: Some("a summary".into()),
                description: None,
            })
            .unwrap();

        catalog.save_mcp(None, &server("new"), Some("old")).unwrap();

        assert!(layer.mcp("old").unwrap().is_none());
        let moved = layer.mcp("new").unwrap().unwrap();
        assert_eq!(moved.expose, McpExpose::Skill);
        assert_eq!(moved.summary.as_deref(), Some("a summary"));
    }

    #[test]
    fn a_project_skill_shadows_the_application_one_in_the_launch_registry() {
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let catalog = Catalog::new(root.path());
        let project = ProjectId::generate();
        let global = skill_folder(&elsewhere.path().join("g"), "review", "the global one");
        let local = skill_folder(&elsewhere.path().join("p"), "review", "the project one");
        catalog
            .add_skill(
                None,
                &SkillAdd::Link {
                    path: text(&global),
                    id: None,
                },
            )
            .unwrap();
        catalog
            .add_skill(
                Some(project),
                &SkillAdd::Link {
                    path: text(&local),
                    id: None,
                },
            )
            .unwrap();

        let described = |scope: Option<ProjectId>| {
            catalog
                .registry_for(scope)
                .skill("review")
                .unwrap()
                .unwrap()
                .meta
                .description
        };
        assert_eq!(described(Some(project)).as_deref(), Some("the project one"));
        assert_eq!(described(None).as_deref(), Some("the global one"));
        assert_eq!(
            described(Some(ProjectId::generate())).as_deref(),
            Some("the global one"),
            "another project sees the application's"
        );
    }

    #[test]
    fn sources_round_trip_and_a_blank_field_is_dropped() {
        let root = TempDir::new().unwrap();
        let catalog = Catalog::new(root.path());
        catalog
            .save_sources(&[SkillSourceInfo {
                id: "mine".into(),
                label: Some(" ".into()),
                kind: SkillSourceKindInfo::Git {
                    url: "https://example.invalid/skills".into(),
                    rev: Some(String::new()),
                    subpath: Some("skills".into()),
                },
            }])
            .unwrap();
        let (_, _, _, count) = layer_of(catalog.snapshot(None).unwrap());
        assert_eq!(count, 1, "the declared list replaces the defaults");
        let Message::Catalog { sources, .. } = catalog.snapshot(None).unwrap() else {
            unreachable!()
        };
        assert_eq!(sources[0].label, None);
        assert_eq!(
            sources[0].kind,
            SkillSourceKindInfo::Git {
                url: "https://example.invalid/skills".into(),
                rev: None,
                subpath: Some("skills".into()),
            }
        );
        assert!(
            catalog
                .save_sources(&[SkillSourceInfo {
                    id: "bad id".into(),
                    label: None,
                    kind: SkillSourceKindInfo::Index { url: "x".into() },
                }])
                .is_err()
        );
    }

    #[test]
    fn a_pasted_config_answers_with_servers_or_an_error() {
        let Message::McpConfigParsed { servers, error } = Catalog::parse_config(
            r#"{"mcpServers": {"fs": {"command": "npx", "args": ["-y", "fs"]}}}"#,
        ) else {
            panic!()
        };
        assert_eq!(error, None);
        assert_eq!(servers[0].id, "fs");
        assert_eq!(servers[0].kind, McpKind::Stdio);

        let Message::McpConfigParsed { servers, error } = Catalog::parse_config("nonsense") else {
            panic!()
        };
        assert!(servers.is_empty() && error.is_some());
    }

    #[test]
    fn a_search_of_an_unknown_source_says_so_without_touching_the_network() {
        let root = TempDir::new().unwrap();
        let catalog = Catalog::new(root.path());
        let Message::SkillSearchResults {
            results, problems, ..
        } = catalog.search_skills("", Some("nowhere"))
        else {
            panic!()
        };
        assert!(results.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(catalog.install_remote(None, "nowhere", "x", None).is_err());
    }
}
