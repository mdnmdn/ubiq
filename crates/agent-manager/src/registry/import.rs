//! Read-only ingest of well-known agent config directories into the catalog.
//!
//! `import` scans source roots (either explicit or well-known defaults) for
//! skills and MCP servers, plans the additions/overwrites, and (unless dry-run)
//! copies them into the catalog root. Never modifies the source directories.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::McpServer;
use crate::Result;

/// What an import would do to one catalog entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Item does not exist in catalog; will be added.
    Add,
    /// Item exists in catalog; will be overwritten (only if `force`).
    Overwrite,
    /// Item exists in catalog; skipped (when `force` is false).
    SkipCollision,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::Add => write!(f, "add"),
            Action::Overwrite => write!(f, "overwrite"),
            Action::SkipCollision => write!(f, "skip"),
        }
    }
}

/// Kind of catalog item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// A skill (folder with SKILL.md).
    Skill,
    /// An MCP server.
    Mcp,
}

/// One planned (or performed) import.
#[derive(Debug, Clone)]
pub struct ImportItem {
    /// What kind of item.
    pub kind: ItemKind,
    /// Stable identifier in the catalog.
    pub id: String,
    /// Where it came from (source path on disk).
    pub source: PathBuf,
    /// What action will be taken.
    pub action: Action,
}

/// Options controlling an import run.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Catalog root to write into.
    pub catalog_root: PathBuf,
    /// A specific source root to scan; if None, scan well-known roots.
    pub from: Option<PathBuf>,
    /// Compute the plan but write nothing.
    pub dry_run: bool,
    /// Overwrite existing catalog entries on id collision (else SkipCollision).
    pub force: bool,
}

/// The result: list of items and whether it was a dry run.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    /// The planned imports.
    pub items: Vec<ImportItem>,
    /// Whether this was a dry-run (no writes happened).
    pub dry_run: bool,
}

/// Scan sources, compute the plan, and (unless dry_run) perform Add/Overwrite copies.
///
/// Well-known sources scanned (if `from` is None):
/// - `~/.claude/skills/*/SKILL.md` (skills), `~/.claude.json` (mcpServers)
/// - `~/.agent/skills/*/SKILL.md` (skills), `~/.agent/mcp/*.json` (mcps)
/// - `./.claude/skills/*/SKILL.md` (skills), `./.mcp.json` (mcpServers)
///
/// Be lenient: missing source files/dirs are not errors, malformed JSON
/// in a source is skipped without hard failure.
pub fn import(opts: &ImportOptions) -> Result<ImportPlan> {
    // Determine source roots
    let source_roots = if let Some(from) = &opts.from {
        vec![from.clone()]
    } else {
        determine_well_known_roots()?
    };

    // Scan all sources for skills and MCPs (using maps for easy lookup later)
    let mut skills: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut mcps: BTreeMap<String, (PathBuf, McpServer)> = BTreeMap::new();

    for root in source_roots {
        collect_from_root(&root, &mut skills, &mut mcps)?;
    }

    // Build the plan: compute actions for each item
    let mut plan = Vec::new();

    for (id, source) in skills {
        let action = compute_action(&opts.catalog_root, ItemKind::Skill, &id, opts.force)?;
        plan.push(ImportItem {
            kind: ItemKind::Skill,
            id,
            source,
            action,
        });
    }

    for (id, (source, _)) in &mcps {
        let action = compute_action(&opts.catalog_root, ItemKind::Mcp, id, opts.force)?;
        plan.push(ImportItem {
            kind: ItemKind::Mcp,
            id: id.clone(),
            source: source.clone(),
            action,
        });
    }

    // Sort by kind then id for consistent output
    plan.sort_by(|a, b| {
        let kind_cmp = (a.kind as u8).cmp(&(b.kind as u8));
        if kind_cmp == std::cmp::Ordering::Equal {
            a.id.cmp(&b.id)
        } else {
            kind_cmp
        }
    });

    // Perform writes (unless dry_run)
    if !opts.dry_run {
        for item in &plan {
            match item.action {
                Action::SkipCollision => {
                    // Do nothing; destination already exists and force is false
                }
                Action::Add | Action::Overwrite => {
                    // Fetch the item data from our maps
                    if item.kind == ItemKind::Skill {
                        let dest = opts.catalog_root.join("skills").join(&item.id);
                        copy_dir_recursive(&item.source, &dest)?;
                    } else {
                        // For MCPs, we need to look up the data in our mcps map
                        if let Some((_, server)) = mcps.get(&item.id) {
                            let dest = opts.catalog_root.join("mcp").join(format!("{}.json", &item.id));
                            std::fs::create_dir_all(dest.parent().unwrap())?;
                            let json = serde_json::to_string_pretty(server)?;
                            std::fs::write(&dest, json)?;
                        }
                    }
                }
            }
        }
    }

    Ok(ImportPlan {
        items: plan,
        dry_run: opts.dry_run,
    })
}

/// Determine well-known source roots to scan (those that exist).
fn determine_well_known_roots() -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();

    // ~/.claude
    if let Some(home) = directories::UserDirs::new()
        .and_then(|d| d.home_dir().to_str().map(PathBuf::from))
    {
        let claude_home = home.join(".claude");
        if claude_home.exists() {
            roots.push(claude_home);
        }
    }

    // ~/.agent
    if let Some(home) = directories::UserDirs::new()
        .and_then(|d| d.home_dir().to_str().map(PathBuf::from))
    {
        let agent_home = home.join(".agent");
        if agent_home.exists() {
            roots.push(agent_home);
        }
    }

    // ./.claude (current directory's .claude)
    let cwd = std::env::current_dir()?;
    let project_claude = cwd.join(".claude");
    if project_claude.exists() {
        roots.push(project_claude);
    }

    // ./.mcp.json (standalone, handled separately below)
    let project_mcp_json = cwd.join(".mcp.json");
    if project_mcp_json.exists() {
        roots.push(project_mcp_json.parent().unwrap().to_path_buf());
    }

    Ok(roots)
}

/// Collect skills and MCPs from a single source root.
/// Updates the maps: skills (id -> source_path) and mcps (id -> (source_path, McpServer)).
fn collect_from_root(
    root: &std::path::Path,
    skills: &mut BTreeMap<String, PathBuf>,
    mcps: &mut BTreeMap<String, (PathBuf, McpServer)>,
) -> Result<()> {
    // Collect skills from various locations
    collect_skills_from_path(root.join("skills"), skills)?;
    collect_skills_from_path(root.join(".claude").join("skills"), skills)?;

    // Collect MCPs from various locations
    // Try <root>.json (e.g., .mcp.json)
    if let Some(stem) = root.file_stem().and_then(|s| s.to_str())
        && (stem == ".mcp" || stem == ".claude")
    {
        let json_path = root.with_extension("json");
        if json_path.exists() {
            collect_mcps_from_json_config(&json_path, mcps)?;
        }
    }

    // Try <root>/.claude.json (e.g., ~/.claude.json)
    let claude_json = root.join(".claude.json");
    if claude_json.exists() {
        collect_mcps_from_json_config(&claude_json, mcps)?;
    }

    // Try <root>/mcp/*.json (single-file MCPs)
    let mcp_dir = root.join("mcp");
    if mcp_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&mcp_dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Some(id) = path.file_stem().and_then(|s| s.to_str())
                && let Ok(content) = std::fs::read_to_string(&path)
                && let Ok(server) = serde_json::from_str::<McpServer>(&content)
            {
                mcps.insert(id.to_string(), (path, server));
            }
        }
    }

    Ok(())
}

/// Collect skills from a skills directory.
fn collect_skills_from_path(skills_path: PathBuf, skills: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    if !skills_path.exists() {
        return Ok(());
    }

    if let Ok(entries) = std::fs::read_dir(&skills_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.exists()
                    && let Some(id) = path.file_name().and_then(|n| n.to_str())
                {
                    skills.insert(id.to_string(), path);
                }
            }
        }
    }

    Ok(())
}

/// Collect MCPs from a JSON config file (e.g., ~/.claude.json, .mcp.json).
/// Expects structure: { "mcpServers": { "id": {...}, ... } }
fn collect_mcps_from_json_config(
    json_path: &std::path::Path,
    mcps: &mut BTreeMap<String, (PathBuf, McpServer)>,
) -> Result<()> {
    let content = match std::fs::read_to_string(json_path) {
        Ok(c) => c,
        Err(_) => return Ok(()), // Lenient: skip malformed files
    };

    // Parse as a generic JSON object
    let root: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Lenient: skip malformed JSON
    };

    // Look for the "mcpServers" key
    if let Some(mcp_servers) = root.get("mcpServers").and_then(|v| v.as_object()) {
        for (id, server_value) in mcp_servers {
            if let Ok(server) = serde_json::from_value::<McpServer>(server_value.clone()) {
                mcps.insert(
                    id.clone(),
                    (json_path.to_path_buf(), server),
                );
            }
        }
    }

    Ok(())
}

/// Compute the action for an item based on whether it exists in the catalog.
fn compute_action(
    catalog_root: &std::path::Path,
    kind: ItemKind,
    id: &str,
    force: bool,
) -> Result<Action> {
    let exists = match kind {
        ItemKind::Skill => {
            let dest = catalog_root.join("skills").join(id);
            dest.exists()
        }
        ItemKind::Mcp => {
            let dest = catalog_root.join("mcp").join(format!("{}.json", id));
            dest.exists()
        }
    };

    if !exists {
        Ok(Action::Add)
    } else if force {
        Ok(Action::Overwrite)
    } else {
        Ok(Action::SkipCollision)
    }
}

/// Recursively copy a directory from `src` to `dst`.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_import_dry_run_with_skills_and_mcps() -> Result<()> {
        let source_dir = tempfile::TempDir::new()?;
        let catalog_dir = tempfile::TempDir::new()?;

        // Create a fake source tree
        let skills_dir = source_dir.path().join(".claude").join("skills").join("demo");
        fs::create_dir_all(&skills_dir)?;
        fs::write(skills_dir.join("SKILL.md"), "# Demo Skill\n\ndemo\n")?;

        let claude_json = source_dir.path().join(".claude.json");
        fs::write(
            &claude_json,
            r#"{ "mcpServers": { "pg": { "command": "pg-mcp", "args": [] } } }"#,
        )?;

        // Run import with dry_run = true
        let opts = ImportOptions {
            catalog_root: catalog_dir.path().to_path_buf(),
            from: Some(source_dir.path().to_path_buf()),
            dry_run: true,
            force: false,
        };

        let plan = import(&opts)?;

        // Verify the plan has both items as "Add"
        assert_eq!(plan.items.len(), 2);
        assert!(plan.dry_run);

        let skill_item = plan
            .items
            .iter()
            .find(|i| i.kind == ItemKind::Skill)
            .unwrap();
        assert_eq!(skill_item.id, "demo");
        assert_eq!(skill_item.action, Action::Add);

        let mcp_item = plan
            .items
            .iter()
            .find(|i| i.kind == ItemKind::Mcp)
            .unwrap();
        assert_eq!(mcp_item.id, "pg");
        assert_eq!(mcp_item.action, Action::Add);

        // Verify nothing was written to the catalog
        assert!(!catalog_dir.path().join("skills/demo").exists());
        assert!(!catalog_dir.path().join("mcp/pg.json").exists());

        // Verify source is unchanged
        assert!(skills_dir.join("SKILL.md").exists());
        assert!(claude_json.exists());

        source_dir.close()?;
        catalog_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_import_actual_write() -> Result<()> {
        let source_dir = tempfile::TempDir::new()?;
        let catalog_dir = tempfile::TempDir::new()?;

        // Create a fake source tree
        let skills_dir = source_dir.path().join(".claude").join("skills").join("demo");
        fs::create_dir_all(&skills_dir)?;
        fs::write(skills_dir.join("SKILL.md"), "# Demo\n\ntest\n")?;
        fs::write(skills_dir.join("extra.md"), "extra content")?;

        let claude_json = source_dir.path().join(".claude.json");
        fs::write(
            &claude_json,
            r#"{ "mcpServers": { "pg": { "command": "pg-mcp", "args": ["--flag"] } } }"#,
        )?;

        // Run import with dry_run = false
        let opts = ImportOptions {
            catalog_root: catalog_dir.path().to_path_buf(),
            from: Some(source_dir.path().to_path_buf()),
            dry_run: false,
            force: false,
        };

        let plan = import(&opts)?;

        // Verify the plan
        assert_eq!(plan.items.len(), 2);
        assert!(!plan.dry_run);

        // Verify files were written to the catalog
        let skill_dest = catalog_dir.path().join("skills/demo");
        assert!(skill_dest.exists());
        assert!(skill_dest.join("SKILL.md").exists());
        assert!(skill_dest.join("extra.md").exists());

        let mcp_dest = catalog_dir.path().join("mcp/pg.json");
        assert!(mcp_dest.exists());
        let mcp_content = fs::read_to_string(&mcp_dest)?;
        assert!(mcp_content.contains("pg-mcp"));

        // Verify source is unchanged
        assert!(skills_dir.join("SKILL.md").exists());
        assert!(claude_json.exists());
        // Source .claude dir should only have the skills subdirectory we created
        assert_eq!(
            fs::read_dir(source_dir.path().join(".claude"))?
                .count(),
            1
        );

        source_dir.close()?;
        catalog_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_import_collision_skip_without_force() -> Result<()> {
        let source_dir = tempfile::TempDir::new()?;
        let catalog_dir = tempfile::TempDir::new()?;

        // Create source
        let skills_dir = source_dir.path().join(".claude").join("skills").join("demo");
        fs::create_dir_all(&skills_dir)?;
        fs::write(skills_dir.join("SKILL.md"), "v1")?;

        // Pre-create the catalog entry (simulating existing entry)
        let catalog_skill = catalog_dir.path().join("skills/demo");
        fs::create_dir_all(&catalog_skill)?;
        fs::write(catalog_skill.join("SKILL.md"), "v0")?;

        let opts = ImportOptions {
            catalog_root: catalog_dir.path().to_path_buf(),
            from: Some(source_dir.path().to_path_buf()),
            dry_run: false,
            force: false,
        };

        let plan = import(&opts)?;

        // Should be SkipCollision
        let item = &plan.items[0];
        assert_eq!(item.action, Action::SkipCollision);

        // Verify the old version is still there
        let content = fs::read_to_string(catalog_skill.join("SKILL.md"))?;
        assert_eq!(content, "v0");

        source_dir.close()?;
        catalog_dir.close()?;
        Ok(())
    }

    #[test]
    fn test_import_collision_overwrite_with_force() -> Result<()> {
        let source_dir = tempfile::TempDir::new()?;
        let catalog_dir = tempfile::TempDir::new()?;

        // Create source
        let skills_dir = source_dir.path().join(".claude").join("skills").join("demo");
        fs::create_dir_all(&skills_dir)?;
        fs::write(skills_dir.join("SKILL.md"), "v1")?;

        // Pre-create the catalog entry
        let catalog_skill = catalog_dir.path().join("skills/demo");
        fs::create_dir_all(&catalog_skill)?;
        fs::write(catalog_skill.join("SKILL.md"), "v0")?;

        let opts = ImportOptions {
            catalog_root: catalog_dir.path().to_path_buf(),
            from: Some(source_dir.path().to_path_buf()),
            dry_run: false,
            force: true,
        };

        let plan = import(&opts)?;

        // Should be Overwrite
        let item = &plan.items[0];
        assert_eq!(item.action, Action::Overwrite);

        // Verify the new version is there
        let content = fs::read_to_string(catalog_skill.join("SKILL.md"))?;
        assert_eq!(content, "v1");

        source_dir.close()?;
        catalog_dir.close()?;
        Ok(())
    }
}
