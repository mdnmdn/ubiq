//! `am profile` subcommands: `ls`, `show`, `use`, `create`.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};

use crate::profile::{
    self, EmptyProfileStore, FsProfileStore, Profile, ProfileDefaults, ProfileIsolate,
    ProfileStore,
};

/// `am profile` subcommand dispatcher.
#[derive(Debug, Parser)]
#[command(name = "am-profile", disable_help_flag = false)]
struct ProfileArgs {
    #[command(subcommand)]
    command: ProfileCommand,
}

/// Subcommands for `am profile`.
#[derive(Debug, Subcommand)]
enum ProfileCommand {
    /// List configured profiles and a short summary of what each carries.
    #[command(name = "ls")]
    List,
    /// Print a profile's flattened (inheritance-resolved) form.
    Show {
        /// Profile name (must exist in the profile store).
        name: String,
    },
    /// Set the default profile (`[defaults].profile`) in the global settings file.
    Use {
        /// Profile name (must exist in the profile store).
        name: String,
    },
    /// Create (and persist) a new profile from the given flags.
    Create {
        /// Profile name (the store key / directory name).
        name: String,
        /// Parent profile to inherit from (must exist).
        #[arg(long)]
        extends: Option<String>,
        /// Account id this profile logs in as.
        #[arg(long)]
        account: Option<String>,
        /// Harness pin (e.g. `claude`).
        #[arg(long)]
        harness: Option<String>,
        /// Default model id (harness-native).
        #[arg(long)]
        model: Option<String>,
        /// Catalog MCP ids (comma-separated or repeatable).
        #[arg(long, value_delimiter = ',')]
        mcps: Option<Vec<String>>,
        /// Catalog skill ids (comma-separated or repeatable).
        #[arg(long, value_delimiter = ',')]
        skills: Option<Vec<String>>,
        /// Overwrite an existing profile of the same name.
        #[arg(long)]
        force: bool,
    },
}

/// Run a profile subcommand, given argv AFTER the `profile` word.
pub(super) fn run(args: &[String]) -> Result<()> {
    // If no args, default to 'ls'.
    let args = if args.is_empty() {
        vec!["ls".to_string()]
    } else {
        args.to_vec()
    };

    let args = ProfileArgs::try_parse_from(
        std::iter::once("am-profile".to_string()).chain(args.iter().cloned()),
    )?;

    match args.command {
        ProfileCommand::List => cmd_list(),
        ProfileCommand::Show { name } => cmd_show(&name),
        ProfileCommand::Use { name } => cmd_use(&name),
        ProfileCommand::Create {
            name,
            extends,
            account,
            harness,
            model,
            mcps,
            skills,
            force,
        } => cmd_create(CreateOpts {
            name,
            extends,
            account,
            harness,
            model,
            mcps,
            skills,
            force,
        }),
    }
}

/// Build the profile store from the default profiles root. Falls back to an
/// empty store when no profiles root exists, so `ls` on a fresh machine prints
/// a friendly "no profiles" line rather than erroring.
fn build_store() -> Box<dyn ProfileStore> {
    match profile::resolve_profiles_root(None) {
        Some(root) if root.is_dir() => Box::new(FsProfileStore::new(root)),
        _ => Box::new(EmptyProfileStore),
    }
}

/// Render a profile's isolation policy for display.
fn describe_isolate(iso: &ProfileIsolate) -> String {
    match iso {
        ProfileIsolate::Off => "isolate=off".to_string(),
        ProfileIsolate::Sandboxed(p) => format!("isolate={p}"),
    }
}

/// One-line summary of what a profile carries — for `ls`.
fn describe_profile(p: &Profile) -> String {
    let mut parts = Vec::new();
    if let Some(parent) = &p.extends {
        parts.push(format!("extends={parent}"));
    }
    if let Some(account) = &p.account {
        parts.push(format!("account={account}"));
    }
    if let Some(harness) = &p.harness {
        parts.push(format!("harness={harness}"));
    }
    if let Some(model) = &p.defaults.model {
        parts.push(format!("model={model}"));
    }
    if let Some(mcps) = &p.defaults.mcps {
        parts.push(format!("mcps={}", mcps.len()));
    }
    if let Some(skills) = &p.defaults.skills {
        parts.push(format!("skills={}", skills.len()));
    }
    if let Some(iso) = &p.isolate {
        parts.push(describe_isolate(iso));
    }
    if parts.is_empty() {
        "(no fields set)".to_string()
    } else {
        format!("({})", parts.join(", "))
    }
}

/// `am profile ls`
fn cmd_list() -> Result<()> {
    let store = build_store();
    let profiles = store.profiles()?;

    if profiles.is_empty() {
        println!("no profiles configured");
        return Ok(());
    }

    for p in profiles {
        println!("{}  {}", p.id, describe_profile(&p));
    }

    Ok(())
}

/// `am profile show <name>`: print the flattened (inheritance-resolved) profile.
fn cmd_show(name: &str) -> Result<()> {
    let store = build_store();
    if store.profile(name)?.is_none() {
        let available: Vec<String> = store.profiles()?.into_iter().map(|p| p.id).collect();
        let listing = if available.is_empty() {
            "(none configured)".to_string()
        } else {
            available.join(", ")
        };
        bail!("unknown profile '{name}'; available: {listing}");
    }

    let flat = profile::resolve_flattened(store.as_ref(), name)?;

    println!("profile: {}", flat.id);
    if let Some(parent) = &flat.extends {
        println!("  extends:  {parent}");
    }
    println!("  account:  {}", opt(&flat.account));
    println!("  harness:  {}", opt(&flat.harness));
    println!(
        "  isolate:  {}",
        flat.isolate
            .as_ref()
            .map(describe_isolate)
            .unwrap_or_else(|| "(unset)".to_string())
    );
    println!("  defaults:");
    println!("    model:        {}", opt(&flat.defaults.model));
    println!("    mcps:         {}", opt_list(&flat.defaults.mcps));
    println!("    skills:       {}", opt_list(&flat.defaults.skills));
    println!("    hooks:        {}", opt_list(&flat.defaults.hooks));
    println!(
        "    instructions: {}",
        flat.defaults
            .instructions
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(unset)".to_string())
    );

    Ok(())
}

/// Render an `Option<String>` for display.
fn opt(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "(unset)".to_string())
}

/// Render an `Option<Vec<String>>` for display.
fn opt_list(v: &Option<Vec<String>>) -> String {
    match v {
        None => "(unset)".to_string(),
        Some(items) if items.is_empty() => "(none)".to_string(),
        Some(items) => items.join(", "),
    }
}

/// `am profile use <name>`: set `[defaults].profile` in the global settings file.
fn cmd_use(name: &str) -> Result<()> {
    let store = build_store();
    if store.profile(name)?.is_none() {
        let available: Vec<String> = store.profiles()?.into_iter().map(|p| p.id).collect();
        let listing = if available.is_empty() {
            "(none configured)".to_string()
        } else {
            available.join(", ")
        };
        bail!("unknown profile '{name}'; available: {listing}");
    }

    let config_path = global_config_path()?;
    let mut table: toml::Table = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| anyhow!("reading {}: {e}", config_path.display()))?;
        toml::from_str(&content)
            .map_err(|e| anyhow!("parsing {}: {e}", config_path.display()))?
    } else {
        toml::Table::new()
    };

    let defaults = table
        .entry("defaults")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let defaults_table = defaults
        .as_table_mut()
        .ok_or_else(|| anyhow!("'defaults' in {} is not a table", config_path.display()))?;
    defaults_table.insert("profile".to_string(), toml::Value::String(name.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, toml::to_string_pretty(&table)?)
        .map_err(|e| anyhow!("writing {}: {e}", config_path.display()))?;

    println!("default profile set to '{name}' ({})", config_path.display());
    Ok(())
}

/// Path to the global settings file that `[defaults]` lives in.
fn global_config_path() -> Result<PathBuf> {
    crate::settings::global_config_write_path()
}

/// Options for `cmd_create` (grouped to avoid a wide argument list).
struct CreateOpts {
    name: String,
    extends: Option<String>,
    account: Option<String>,
    harness: Option<String>,
    model: Option<String>,
    mcps: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    force: bool,
}

/// `am profile create <name> [flags]`: build a [`Profile`] and persist it.
fn cmd_create(opts: CreateOpts) -> Result<()> {
    let root = profile::resolve_profiles_root(None)
        .ok_or_else(|| anyhow!("could not determine the profiles root for this OS"))?;
    let store = FsProfileStore::new(&root);

    // Refuse to clobber an existing profile unless --force.
    if !opts.force && store.profile(&opts.name)?.is_some() {
        bail!(
            "profile '{}' already exists; pass --force to overwrite",
            opts.name
        );
    }

    // If a parent is named, it must exist.
    if let Some(parent) = &opts.extends
        && store.profile(parent)?.is_none()
    {
        let available: Vec<String> = store.profiles()?.into_iter().map(|p| p.id).collect();
        let listing = if available.is_empty() {
            "(none configured)".to_string()
        } else {
            available.join(", ")
        };
        bail!("unknown parent profile '{parent}'; available: {listing}");
    }

    let profile = Profile {
        id: opts.name.clone(),
        extends: opts.extends,
        account: opts.account,
        harness: opts.harness,
        defaults: ProfileDefaults {
            mcps: opts.mcps,
            skills: opts.skills,
            model: opts.model,
            ..Default::default()
        },
        isolate: None,
    };

    let path = store.save(&profile)?;
    println!("profile '{}' written to {}", opts.name, path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatcher_defaults_to_ls_when_empty() {
        // An empty argv parses to the List subcommand (no panic / error at parse).
        let parsed = ProfileArgs::try_parse_from(["am-profile", "ls"]).unwrap();
        assert!(matches!(parsed.command, ProfileCommand::List));
    }

    #[test]
    fn create_round_trips_via_save() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path().join("profiles");
        let store = FsProfileStore::new(&root);

        let profile = Profile {
            id: "work".to_string(),
            account: Some("work-acct".to_string()),
            harness: Some("claude".to_string()),
            defaults: ProfileDefaults {
                mcps: Some(vec!["github".to_string(), "postgres".to_string()]),
                model: Some("haiku".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let path = store.save(&profile)?;
        assert_eq!(path, root.join("work").join("profile.toml"));

        let loaded = FsProfileStore::new(&root)
            .profile("work")?
            .expect("saved profile should be found");
        assert_eq!(loaded, profile);

        temp.close()?;
        Ok(())
    }

    #[test]
    fn use_writes_defaults_profile_key() -> Result<()> {
        // Mirror account::cmd_use's write approach on an explicit config path
        // (do NOT touch env or the real global config).
        let temp = tempfile::TempDir::new()?;
        let config_path = temp.path().join("config.toml");

        let mut table = toml::Table::new();
        let defaults = table
            .entry("defaults")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let defaults_table = defaults.as_table_mut().unwrap();
        defaults_table.insert(
            "profile".to_string(),
            toml::Value::String("work".to_string()),
        );
        std::fs::write(&config_path, toml::to_string_pretty(&table)?)?;

        let content = std::fs::read_to_string(&config_path)?;
        let reparsed: toml::Table = toml::from_str(&content)?;
        assert_eq!(
            reparsed["defaults"]["profile"].as_str(),
            Some("work"),
            "config was: {content}"
        );

        temp.close()?;
        Ok(())
    }

    #[test]
    fn describe_profile_summarizes_fields() {
        let p = Profile {
            id: "x".to_string(),
            extends: Some("base".to_string()),
            account: Some("acct".to_string()),
            defaults: ProfileDefaults {
                mcps: Some(vec!["a".to_string(), "b".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let s = describe_profile(&p);
        assert!(s.contains("extends=base"), "{s}");
        assert!(s.contains("account=acct"), "{s}");
        assert!(s.contains("mcps=2"), "{s}");
    }
}
