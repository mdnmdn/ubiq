//! `am account` subcommands: `ls`, `use`, `import`.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use clap::{Parser, Subcommand};

use crate::account::{self, Account, AccountStore, EmptyAccountStore, FsAccountStore};

/// `am account` subcommand dispatcher.
#[derive(Debug, Parser)]
#[command(name = "am-account", disable_help_flag = false)]
struct AccountArgs {
    #[command(subcommand)]
    command: AccountCommand,
}

/// Subcommands for `am account`.
#[derive(Debug, Subcommand)]
enum AccountCommand {
    /// List configured accounts and which references each carries.
    #[command(name = "ls")]
    List,
    /// Set the default account (`[defaults].account`) in the global settings file.
    Use {
        /// Account id (must exist in the account store).
        id: String,
    },
    /// Read-only discovery of existing credential locations (never their contents).
    Import {
        /// Append the suggested reference-only account(s) to accounts.toml.
        #[arg(long)]
        write: bool,
    },
}

/// Run an account subcommand, given argv AFTER the `account` word.
pub(super) fn run(args: &[String]) -> Result<()> {
    // If no args, default to 'ls'.
    let args = if args.is_empty() {
        vec!["ls".to_string()]
    } else {
        args.to_vec()
    };

    let args = AccountArgs::try_parse_from(
        std::iter::once("am-account".to_string()).chain(args.iter().cloned()),
    )?;

    match args.command {
        AccountCommand::List => cmd_list(),
        AccountCommand::Use { id } => cmd_use(&id),
        AccountCommand::Import { write } => cmd_import(write),
    }
}

/// Build the account store from the default accounts root. Falls back to an
/// empty store when no accounts root exists, so `ls` on a fresh machine
/// prints a friendly "no accounts" line rather than erroring.
fn build_store() -> Box<dyn AccountStore> {
    match account::resolve_accounts_root(None) {
        Some(root) if root.is_dir() => Box::new(FsAccountStore::new(root)),
        _ => Box::new(EmptyAccountStore),
    }
}

/// Describe which reference fields an account carries, e.g. `(api_key_env, base_url)`.
fn describe_refs(acct: &Account) -> String {
    let mut parts = Vec::new();
    if acct.api_key_env.is_some() {
        parts.push("api_key_env");
    }
    if acct.auth_token_env.is_some() {
        parts.push("auth_token_env");
    }
    if acct.base_url.is_some() {
        parts.push("base_url");
    }
    if acct.helper.is_some() {
        parts.push("helper");
    }
    if acct.home.is_some() {
        parts.push("home");
    }
    if parts.is_empty() {
        "(no references set)".to_string()
    } else {
        format!("({})", parts.join(", "))
    }
}

/// `am account ls`
fn cmd_list() -> Result<()> {
    let store = build_store();
    let accounts = store.accounts()?;

    if accounts.is_empty() {
        println!("no accounts configured");
        return Ok(());
    }

    for acct in accounts {
        println!("{}  {}", acct.id, describe_refs(&acct));
    }

    Ok(())
}

/// `am account use <id>`: set `[defaults].account` in the global settings file.
fn cmd_use(id: &str) -> Result<()> {
    let store = build_store();
    if store.account(id)?.is_none() {
        let available: Vec<String> = store.accounts()?.into_iter().map(|a| a.id).collect();
        let listing = if available.is_empty() {
            "(none configured)".to_string()
        } else {
            available.join(", ")
        };
        bail!("unknown account id '{id}'; available: {listing}");
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
    let defaults_table = defaults.as_table_mut().ok_or_else(|| {
        anyhow!("'defaults' in {} is not a table", config_path.display())
    })?;
    defaults_table.insert("account".to_string(), toml::Value::String(id.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, toml::to_string_pretty(&table)?)
        .map_err(|e| anyhow!("writing {}: {e}", config_path.display()))?;

    println!("default account set to '{id}' ({})", config_path.display());
    Ok(())
}

/// Path to the global settings file that `[defaults]` lives in.
fn global_config_path() -> Result<PathBuf> {
    directories::ProjectDirs::from("", "", "agent-manager")
        .map(|dirs| dirs.config_dir().join("config.toml"))
        .ok_or_else(|| anyhow!("could not determine the global config directory for this OS"))
}

/// `am account import [--write]`: read-only discovery of credential
/// *locations* — env var NAMES and credential file PATHS only, never
/// contents or values. Prints suggested reference-only [`Account`]s; with
/// `--write`, appends them to `accounts.toml`.
fn cmd_import(write: bool) -> Result<()> {
    let mut suggestions: Vec<Account> = Vec::new();

    for env_name in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"] {
        if std::env::var(env_name).is_ok() {
            println!("found env var: {env_name}");
            suggestions.push(Account {
                id: env_name.to_lowercase().replace('_', "-"),
                api_key_env: Some(env_name.to_string()),
                ..Default::default()
            });
        }
    }

    if let Some(base_dirs) = directories::BaseDirs::new() {
        let home = base_dirs.home_dir();
        let candidates: [(&str, PathBuf); 4] = [
            ("claude-credentials", home.join(".claude/credentials.json")),
            ("claude-json", home.join(".claude.json")),
            ("codex-auth", home.join(".codex/auth.json")),
            ("opencode-auth", home.join(".local/share/opencode/auth.json")),
        ];

        for (label, path) in candidates {
            // NEVER read the contents of a credential file — only check
            // whether it exists, and report its path.
            if path.exists() {
                println!("found credential file ({label}): {}", path.display());
                let home_dir = path.parent().unwrap_or(&path).to_path_buf();
                suggestions.push(Account {
                    id: format!("{label}-home"),
                    home: Some(home_dir),
                    ..Default::default()
                });
            }
        }
    }

    if suggestions.is_empty() {
        println!("no known credential locations found");
        return Ok(());
    }

    println!();
    println!("suggested account(s) (references only; edit ids/env names as needed):");
    for acct in &suggestions {
        print!("{}", account_toml_snippet(acct));
    }

    if write {
        let root = account::resolve_accounts_root(None)
            .ok_or_else(|| anyhow!("could not determine the accounts root for this OS"))?;
        std::fs::create_dir_all(&root)?;
        let toml_path = root.join("accounts.toml");
        let mut existing = if toml_path.exists() {
            std::fs::read_to_string(&toml_path)?
        } else {
            String::new()
        };
        for acct in &suggestions {
            existing.push('\n');
            existing.push_str(&account_toml_snippet(acct));
        }
        std::fs::write(&toml_path, existing)?;
        println!();
        println!("appended to {}", toml_path.display());
    } else {
        println!();
        println!("(dry run — nothing written; pass --write to append to accounts.toml)");
    }

    Ok(())
}

/// Render an [`Account`] as an inline `[[account]]` TOML snippet.
fn account_toml_snippet(acct: &Account) -> String {
    let mut s = String::new();
    s.push_str("[[account]]\n");
    s.push_str(&format!("id = \"{}\"\n", acct.id));
    if let Some(v) = &acct.api_key_env {
        s.push_str(&format!("api_key_env = \"{v}\"\n"));
    }
    if let Some(v) = &acct.auth_token_env {
        s.push_str(&format!("auth_token_env = \"{v}\"\n"));
    }
    if let Some(v) = &acct.base_url {
        s.push_str(&format!("base_url = \"{v}\"\n"));
    }
    if let Some(v) = &acct.helper {
        s.push_str(&format!("helper = \"{v}\"\n"));
    }
    if let Some(v) = &acct.home {
        s.push_str(&format!("home = \"{}\"\n", v.display()));
    }
    s
}
