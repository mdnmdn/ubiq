//! `am catalog` subcommands: `ls`, `show`, `path`, `import`.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};

use crate::registry::{self, import::ImportOptions, FsRegistry, OverlayRegistry, Registry};

/// `am catalog` subcommand dispatcher.
#[derive(Debug, Parser)]
#[command(name = "am-catalog", disable_help_flag = false)]
struct CatalogArgs {
    #[command(subcommand)]
    command: CatalogCommand,
}

/// Subcommands for `am catalog`.
#[derive(Debug, Subcommand)]
enum CatalogCommand {
    /// List available skills and MCP servers.
    #[command(name = "ls")]
    List {
        /// Show only MCPs.
        #[arg(long)]
        mcps: bool,
        /// Show only skills.
        #[arg(long)]
        skills: bool,
    },
    /// Print one entry's resolved definition.
    Show {
        /// The id to look up (skill or MCP).
        id: String,
    },
    /// Print the active catalog root.
    Path,
    /// Ingest ~/.claude, ~/.agent, etc. into the catalog.
    Import {
        /// A specific source root to scan; if not given, scans well-known roots.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Compute the plan but write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Overwrite existing catalog entries on id collision.
        #[arg(long)]
        force: bool,
    },
}

/// Run a catalog subcommand, given argv AFTER the `catalog` word.
pub(super) fn run(args: &[String]) -> Result<()> {
    // If no args, default to 'ls'
    let args = if args.is_empty() {
        vec!["ls".to_string()]
    } else {
        args.to_vec()
    };

    let args = CatalogArgs::try_parse_from(
        std::iter::once("am-catalog".to_string()).chain(args.iter().cloned()),
    )?;

    match args.command {
        CatalogCommand::List { mcps, skills } => cmd_list(mcps, skills),
        CatalogCommand::Show { id } => cmd_show(&id),
        CatalogCommand::Path => cmd_path(),
        CatalogCommand::Import {
            from,
            dry_run,
            force,
        } => cmd_import(from, dry_run, force),
    }
}

/// `am catalog ls [--mcps] [--skills]`
fn cmd_list(show_mcps_only: bool, show_skills_only: bool) -> Result<()> {
    let registry = build_registry()?;

    let show_both = !show_mcps_only && !show_skills_only;

    if show_both || show_skills_only {
        let skills = registry.skills()?;
        if !skills.is_empty() && (show_both || show_skills_only) {
            if show_both {
                println!("Skills:");
            }
            for skill in skills {
                let desc = skill
                    .meta
                    .description
                    .as_deref()
                    .unwrap_or("(no description)");
                let name = skill
                    .meta
                    .name
                    .as_ref()
                    .map(|n| format!(" ({})", n))
                    .unwrap_or_default();
                println!("  {}{}  — {}", skill.id, name, desc);
            }
        }
    }

    if show_both || show_mcps_only {
        let mcps = registry.mcps()?;
        if !mcps.is_empty() && (show_both || show_mcps_only) {
            if show_both {
                println!("MCPs:");
            }
            for mcp in mcps {
                let transport = format!("{:?}", mcp.def.transport).to_lowercase();
                let endpoint = match (&mcp.def.command, &mcp.def.url) {
                    (Some(cmd), _) => cmd.clone(),
                    (_, Some(url)) => url.clone(),
                    _ => "(no command or url)".to_string(),
                };
                println!("  {}  [{}] {}", mcp.id, transport, endpoint);
            }
        }
    }

    if !show_both && (show_mcps_only || show_skills_only) {
        // If filtering and found nothing, that's ok
    }

    Ok(())
}

/// `am catalog show <id>`
fn cmd_show(id: &str) -> Result<()> {
    let registry = build_registry()?;

    // Try to find as skill first
    if let Some(skill) = registry.skill(id)? {
        println!("Skill: {}", skill.id);
        println!("  Path: {}", skill.path.display());
        if let Some(name) = &skill.meta.name {
            println!("  Name: {}", name);
        }
        if let Some(desc) = &skill.meta.description {
            println!("  Description: {}", desc);
        }
        return Ok(());
    }

    // Try to find as MCP
    if let Some(mcp) = registry.mcp(id)? {
        println!("MCP: {}", mcp.id);
        println!("{}", serde_json::to_string_pretty(&mcp.def)?);
        return Ok(());
    }

    bail!("not found: '{}' (neither a skill nor an MCP)", id);
}

/// `am catalog path`
fn cmd_path() -> Result<()> {
    match registry::resolve_catalog_root(None) {
        Some(root) => {
            println!("{}", root.display());
            Ok(())
        }
        None => {
            println!("No catalog root configured.");
            println!("Set --catalog, AM_CATALOG env var, or check the default location.");
            Ok(())
        }
    }
}

/// `am catalog import [--from <path>] [--dry-run] [--force]`
fn cmd_import(from: Option<PathBuf>, dry_run: bool, force: bool) -> Result<()> {
    let catalog_root = registry::resolve_catalog_root(None)
        .ok_or_else(|| anyhow!("No catalog root configured. Set --catalog, AM_CATALOG env var, or check the default location."))?;

    let opts = ImportOptions {
        catalog_root,
        from,
        dry_run,
        force,
    };

    let plan = registry::import::import(&opts)?;

    // Print the plan
    let mut add_count = 0;
    let mut overwrite_count = 0;
    let mut skip_count = 0;

    for item in &plan.items {
        let action_str = match item.action {
            registry::Action::Add => {
                add_count += 1;
                "add"
            }
            registry::Action::Overwrite => {
                overwrite_count += 1;
                "overwrite"
            }
            registry::Action::SkipCollision => {
                skip_count += 1;
                "skip"
            }
        };

        let kind_str = match item.kind {
            registry::ItemKind::Skill => "skill",
            registry::ItemKind::Mcp => "mcp",
        };

        if item.action == registry::Action::SkipCollision {
            println!(
                "[{}] {} {}  (collision; use --force to overwrite)",
                action_str, kind_str, item.id
            );
        } else {
            println!(
                "[{}] {} {}  (from {})",
                action_str,
                kind_str,
                item.id,
                item.source.display()
            );
        }
    }

    if plan.items.is_empty() {
        println!("(no items to import)");
    }

    // Print summary
    println!();
    println!(
        "Summary: {} added, {} overwritten, {} skipped",
        add_count, overwrite_count, skip_count
    );

    if plan.dry_run {
        println!("(dry run — nothing written)");
    }

    Ok(())
}

/// Build a registry (global + optional project overlay) from the current environment.
fn build_registry() -> Result<Box<dyn Registry>> {
    let cwd = std::env::current_dir()?;

    let catalog_root = registry::resolve_catalog_root(None)
        .unwrap_or_else(|| cwd.join(".agent-manager-catalog-unset"));

    let global = FsRegistry::new(&catalog_root);

    // Look for project overlay
    match find_project_catalog(&cwd) {
        Some(project_root) => {
            let project = FsRegistry::new(project_root);
            Ok(Box::new(OverlayRegistry::new(global, Some(project))) as Box<dyn Registry>)
        }
        None => {
            let empty: Option<FsRegistry> = None;
            Ok(Box::new(OverlayRegistry::new(global, empty)) as Box<dyn Registry>)
        }
    }
}

/// Look for a project-local catalog overlay (`<dir>/.agent-manager/catalog`),
/// walking up from `cwd` to the git root (mirrors settings discovery).
fn find_project_catalog(cwd: &std::path::Path) -> Option<PathBuf> {
    let mut current = Some(cwd.to_path_buf());
    while let Some(dir) = current {
        let candidate = dir.join(".agent-manager").join("catalog");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if dir.join(".git").exists() {
            break;
        }
        current = dir.parent().map(|p| p.to_path_buf());
    }
    None
}
