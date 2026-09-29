//! `am skill` subcommands: manage the skills of a catalog layer and install them from remote sources.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};

use crate::registry::{
    CatalogStore, FsRegistry, Registry, SkillOrigin, SkillSource, SkillSourceKind, remote,
};

/// `am skill` argument parser.
#[derive(Debug, Parser)]
#[command(name = "am-skill", disable_help_flag = false)]
struct SkillArgs {
    #[command(flatten)]
    layer: LayerArgs,
    #[command(subcommand)]
    command: SkillCommand,
}

/// Which catalog layer a command targets: the global one, or the project's.
#[derive(Debug, Args)]
pub(super) struct LayerArgs {
    /// Target the project catalog (`<cwd>/.agent-manager/catalog`) instead of the global one.
    #[arg(long, global = true)]
    pub(super) project: bool,
    /// Catalog root override (else `AM_CATALOG`, else the default).
    #[arg(long, global = true)]
    pub(super) catalog: Option<PathBuf>,
}

impl LayerArgs {
    /// The registry of the chosen layer.
    pub(super) fn store(&self) -> Result<FsRegistry> {
        if self.project {
            return Ok(FsRegistry::new(
                std::env::current_dir()?
                    .join(".agent-manager")
                    .join("catalog"),
            ));
        }
        Ok(FsRegistry::new(self.global_root()?))
    }

    fn global_root(&self) -> Result<PathBuf> {
        crate::registry::resolve_catalog_root(self.catalog.clone())
            .ok_or_else(|| anyhow!("No catalog root configured. Set --catalog, AM_CATALOG, or check the default location."))
    }

    /// Where git sources are cloned: `cache/skill-sources` beside the global catalog.
    fn cache(&self) -> Result<PathBuf> {
        let root = self.global_root()?;
        Ok(root
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| root.clone())
            .join("cache")
            .join("skill-sources"))
    }
}

/// Subcommands for `am skill`.
#[derive(Debug, Subcommand)]
enum SkillCommand {
    /// List the skills of the layer.
    #[command(name = "ls")]
    List,
    /// Reference a skill folder in place.
    Link {
        /// The skill folder (contains `SKILL.md`).
        path: PathBuf,
        /// Id to use (default: the folder name).
        #[arg(long)]
        id: Option<String>,
    },
    /// Scan a folder: every `<sub>/SKILL.md` in it is a skill.
    AddDir {
        /// The folder to scan.
        path: PathBuf,
    },
    /// Stop scanning a folder.
    RmDir {
        /// The folder.
        path: PathBuf,
    },
    /// Remove a skill (installed copy or link).
    Rm {
        /// The skill id.
        id: String,
    },
    /// List the skill sources.
    Sources,
    /// Search the skill sources.
    Search {
        /// Text to look for in id, name and description (empty = everything).
        query: Option<String>,
        /// Only search this source.
        #[arg(long)]
        source: Option<String>,
    },
    /// Install a skill found by `search`.
    Install {
        /// `<source>/<path or skill id>`, as printed by `search`.
        spec: String,
        /// Id to install as (default: the skill's own).
        #[arg(long)]
        id: Option<String>,
    },
}

/// Run a skill subcommand, given argv AFTER the `skill` word.
pub(super) fn run(args: &[String]) -> Result<()> {
    let args = if args.is_empty() {
        vec!["ls".to_string()]
    } else {
        args.to_vec()
    };
    let args = SkillArgs::try_parse_from(std::iter::once("am-skill".to_string()).chain(args))?;
    let store = args.layer.store()?;
    match args.command {
        SkillCommand::List => cmd_list(&store),
        SkillCommand::Link { path, id } => {
            let id = store.link_skill(&path, id.as_deref())?;
            println!("linked skill '{id}'");
            Ok(())
        }
        SkillCommand::AddDir { path } => {
            store.add_skill_dir(&path)?;
            println!("scanning {}", path.display());
            Ok(())
        }
        SkillCommand::RmDir { path } => {
            store.remove_skill_dir(&path)?;
            println!("no longer scanning {}", path.display());
            Ok(())
        }
        SkillCommand::Rm { id } => {
            store.remove_skill(&id)?;
            println!("removed skill '{id}'");
            Ok(())
        }
        SkillCommand::Sources => cmd_sources(&store),
        SkillCommand::Search { query, source } => cmd_search(
            &store,
            &args.layer.cache()?,
            query.as_deref().unwrap_or(""),
            source.as_deref(),
        ),
        SkillCommand::Install { spec, id } => {
            cmd_install(&store, &args.layer.cache()?, &spec, id.as_deref())
        }
    }
}

fn cmd_list(store: &FsRegistry) -> Result<()> {
    let skills = store.skills()?;
    if skills.is_empty() {
        println!("(no skills)");
    }
    for s in skills {
        let origin = match &s.origin {
            SkillOrigin::Installed { remote: Some(r) } => format!("installed from {}", r.url),
            SkillOrigin::Installed { remote: None } => "installed".to_string(),
            SkillOrigin::Linked(p) => format!("linked {}", p.display()),
            SkillOrigin::Dir(p) => format!("folder {}", p.display()),
        };
        let desc = short(s.meta.description.as_deref().unwrap_or("(no description)"));
        println!("  {}  [{origin}]  — {desc}", s.id);
    }
    for dir in store.skill_dirs()? {
        println!("  scanning: {}", dir.display());
    }
    Ok(())
}

/// `s` cut to one line of at most 100 characters.
pub(super) fn short(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    match line.char_indices().nth(100) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

fn describe(src: &SkillSource) -> String {
    match &src.kind {
        SkillSourceKind::Git { url, rev, subpath } => {
            let mut s = format!("git {url}");
            if let Some(rev) = rev {
                s.push_str(&format!(" @{rev}"));
            }
            if let Some(sub) = subpath {
                s.push_str(&format!(" under {sub}"));
            }
            s
        }
        SkillSourceKind::Index { url } => format!("index {url}"),
    }
}

fn cmd_sources(store: &FsRegistry) -> Result<()> {
    for src in store.skill_sources()? {
        println!(
            "  {}  {}  ({})",
            src.id,
            src.label.as_deref().unwrap_or(""),
            describe(&src)
        );
    }
    Ok(())
}

fn pick_sources(store: &FsRegistry, only: Option<&str>) -> Result<Vec<SkillSource>> {
    let all = store.skill_sources()?;
    match only {
        None => Ok(all),
        Some(id) => {
            let found: Vec<_> = all.into_iter().filter(|s| s.id == id).collect();
            if found.is_empty() {
                bail!("unknown skill source '{id}' (see `am skill sources`)");
            }
            Ok(found)
        }
    }
}

fn cmd_search(
    store: &FsRegistry,
    cache: &std::path::Path,
    query: &str,
    source: Option<&str>,
) -> Result<()> {
    let sources = pick_sources(store, source)?;
    let (found, problems) = remote::search(&sources, cache, query);
    for s in &found {
        let desc = short(s.description.as_deref().unwrap_or("(no description)"));
        println!("  {}/{}  — {desc}", s.source, s.path);
    }
    if found.is_empty() {
        println!("(no matches)");
    }
    for p in problems {
        eprintln!("warning: {p}");
    }
    Ok(())
}

fn cmd_install(
    store: &FsRegistry,
    cache: &std::path::Path,
    spec: &str,
    id: Option<&str>,
) -> Result<()> {
    let (source_id, what) = spec
        .split_once('/')
        .ok_or_else(|| anyhow!("expected <source>/<path>, e.g. anthropics/skills/pdf"))?;
    let src = pick_sources(store, Some(source_id))?.remove(0);
    let listed = remote::list_source(&src, cache)?;
    let matches: Vec<_> = listed
        .iter()
        .filter(|s| s.path == what || (s.id == what && !what.contains('/')))
        .collect();
    let skill = match matches.as_slice() {
        [one] => *one,
        [] => bail!("no skill '{what}' in source '{source_id}' (try `am skill search`)"),
        many => bail!(
            "'{what}' is ambiguous in '{source_id}': {}",
            many.iter()
                .map(|s| s.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let id = remote::install_remote(store, &src, cache, skill, id)?;
    println!("installed skill '{id}' from {source_id}/{}", skill.path);
    Ok(())
}
