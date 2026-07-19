//! `am account` subcommands: `ls`, `use`, `import`.

use std::collections::BTreeSet;
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
    /// Log into a harness inside a persistent per-account home and capture its
    /// credential file for reuse via `--account <id>`.
    Login {
        /// Identity to capture (e.g. `mdn`). Logging in a second harness
        /// under the same id reuses the same home dir — each harness only
        /// reads its own subpath there, so captures for different harnesses
        /// coexist without colliding (`am account ls` shows which harnesses
        /// an id has an effective login for).
        id: String,
        /// Harness to log into (e.g. `claude`, `codex`).
        #[arg(long)]
        harness: String,
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
        AccountCommand::Login { id, harness } => cmd_login(&id, &harness),
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

/// Which harnesses have an *effective* captured login under `home`: for each
/// harness `am` knows about, its primary credential file
/// ([`crate::harness::ConfigAnchor::login_seed`]'s first entry, `src`
/// relative to `home`) exists on disk. Sorted by harness id.
///
/// Derived from the filesystem at call time, never stored — a shared home
/// dir (one account, multiple harnesses each captured separately via
/// `am account login <id> --harness <h>`) can never drift out of sync with
/// what's actually captured there, because there's no separate bookkeeping
/// to drift.
fn effective_harnesses(home: &std::path::Path) -> Vec<String> {
    let mut ids: Vec<String> = crate::harness::all()
        .into_iter()
        .filter(|h| {
            h.config_anchor()
                .login_seed
                .first()
                .is_some_and(|seed| home.join(&seed.src).exists())
        })
        .map(|h| h.id())
        .collect();
    ids.sort();
    ids
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
        let mut line = format!("{}  {}", acct.id, describe_refs(&acct));
        if let Some(home) = &acct.home {
            let captured = effective_harnesses(home);
            if !captured.is_empty() {
                line.push_str(&format!("  [captured: {}]", captured.join(", ")));
            }
        }
        println!("{line}");
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
    crate::settings::global_config_write_path()
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
        let candidates: [(&str, PathBuf); 5] = [
            ("claude-credentials", home.join(".claude/credentials.json")),
            ("claude-json", home.join(".claude.json")),
            ("codex-auth", home.join(".codex/auth.json")),
            ("opencode-auth", home.join(".local/share/opencode/auth.json")),
            ("copilot-config", home.join(".copilot/config.json")),
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

    // Idempotency: never suggest or append an id that already exists in the
    // store — whether from a prior `import --write` or an `account login`
    // per-file entry. Re-running `import [--write]` is therefore a no-op once
    // everything is already present.
    let root = account::resolve_accounts_root(None)
        .ok_or_else(|| anyhow!("could not determine the accounts root for this OS"))?;
    let existing_ids: BTreeSet<String> = if root.is_dir() {
        FsAccountStore::new(&root)
            .accounts()?
            .into_iter()
            .map(|a| a.id)
            .collect()
    } else {
        BTreeSet::new()
    };

    let (to_add, skipped) = partition_new(&existing_ids, suggestions);
    for id in &skipped {
        println!("skip (already present): {id}");
    }

    if to_add.is_empty() {
        println!();
        println!("all suggested accounts already present — nothing to add (idempotent).");
        return Ok(());
    }

    println!();
    println!("new suggested account(s) (references only; edit ids/env names as needed):");
    for acct in &to_add {
        print!("{}", account_toml_snippet(acct));
    }

    if write {
        std::fs::create_dir_all(&root)?;
        let toml_path = root.join("accounts.toml");
        let mut existing = if toml_path.exists() {
            std::fs::read_to_string(&toml_path)?
        } else {
            String::new()
        };
        for acct in &to_add {
            // Keep array-of-tables well-separated even if the prior content
            // didn't end in a newline.
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push('\n');
            existing.push_str(&account_toml_snippet(acct));
        }
        std::fs::write(&toml_path, existing)?;
        println!();
        println!(
            "appended {} new account(s) to {}",
            to_add.len(),
            toml_path.display()
        );
    } else {
        println!();
        println!("(dry run — nothing written; pass --write to append to accounts.toml)");
    }

    Ok(())
}

/// Split `suggestions` into `(to_add, skipped_ids)`: entries whose id is not
/// already in `existing_ids`, de-duplicated by id within the batch (order
/// preserved), plus the ids skipped as already-present or intra-batch
/// duplicates. Pure — the idempotency core of `am account import`, unit-tested
/// without touching the filesystem.
fn partition_new(
    existing_ids: &BTreeSet<String>,
    suggestions: Vec<Account>,
) -> (Vec<Account>, Vec<String>) {
    let mut to_add = Vec::new();
    let mut skipped = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for acct in suggestions {
        if existing_ids.contains(&acct.id) || !seen.insert(acct.id.clone()) {
            skipped.push(acct.id);
            continue;
        }
        to_add.push(acct);
    }
    (to_add, skipped)
}

/// `am account login <id> --harness <h>`: interactively log `harness_key`
/// into a persistent per-account home dir, verify the resulting credential
/// file landed on disk, and record `home` on the account so
/// `am <h> --account <id>` reuses it.
///
/// The stored account id is the bare `id` the caller typed — `am account
/// login mdn --harness claude` and `am account login mdn --harness copilot`
/// share **one** home dir (`accounts/mdn/`) and **one** account-store entry.
/// This is safe, not a collision: each harness's [`crate::harness::Harness::
/// config_anchor`] seeds from its own harness-specific relative subpath
/// under `home` (e.g. Claude's `.claude/.credentials.json` vs Copilot's
/// `config.json`), so two harnesses' captured logins coexist in the same
/// home without overwriting each other — exactly like a real `$HOME` holds
/// `.claude/`, `.copilot/`, `.codex/` side by side today. Which harnesses a
/// given account actually has an effective login for is never stored
/// separately (nothing to let drift out of sync); it's derived at display
/// time by [`effective_harnesses`], which checks each harness's primary
/// credential file for real on disk. `am` never parses or copies the
/// credential file's contents — it only points the harness's own credential
/// store at the capture home and checks that the harness wrote *something*
/// there.
fn cmd_login(id: &str, harness_key: &str) -> Result<()> {
    let root = account::resolve_accounts_root(None)
        .ok_or_else(|| anyhow!("no accounts root; set AM_ACCOUNTS"))?;
    // Route the capture through the store trait: `login_home` gives a real dir
    // to log into (the persistent per-account home for the filesystem store),
    // and `capture_login` below persists the result — so a database-backed
    // store captures the same way without any CLI change.
    let store = FsAccountStore::new(&root);
    let home = store.login_home(id)?;

    let harness = crate::harness::resolve(harness_key).ok_or_else(|| {
        anyhow!(
            "unknown harness '{harness_key}'; known: {}",
            crate::harness::known_ids().join(", ")
        )
    })?;

    let plan = harness.login(&home)?;

    // Record the primary credential file's mtime *before* launching login, so
    // a harness that exits 0 without actually writing fresh credentials (e.g.
    // Claude Code aborting the persist step after a keychain-unreachable
    // error, but still completing the rest of the OAuth flow) can't leave a
    // stale pre-existing file behind and be reported as a success.
    let primary = home.join(&plan.credential_files[0]);
    let mtime_before = std::fs::metadata(&primary).and_then(|m| m.modified()).ok();

    let provisioned = crate::provision::Provisioned {
        dir: home.clone(),
        launch: plan.launch,
        ephemeral: false, // persistent home — never auto-deleted
        #[cfg(feature = "inproc-mcp")]
        inproc_servers: Vec::new(),
    };
    // Login capture relocates HOME to `home` (a bare dir with no
    // ~/Library/Keychains) precisely so the harness can't reach the OS
    // keychain and falls back to writing a portable credential file instead
    // — see the `login()` docs on each harness. On macOS that fallback is
    // preceded by a keychain-lookup error printed straight to the terminal;
    // it's expected and harmless, so flag it before it appears rather than
    // let it read as a failure.
    #[cfg(target_os = "macos")]
    println!(
        "note: macOS may print \"A keychain cannot be found to store '{}'\" below — that's \
         expected, am relocates HOME during capture so credentials land in a portable file \
         instead of your system keychain",
        std::env::var("USER").unwrap_or_else(|_| "you".to_string())
    );
    let cwd = std::env::current_dir()?;
    let code = crate::run::run(&provisioned, &cwd, true)?; // keep_config: persistent
    if code != 0 {
        bail!("harness login exited with code {code}; no account recorded");
    }

    let mtime_after = std::fs::metadata(&primary).and_then(|m| m.modified()).ok();
    match (mtime_before, mtime_after) {
        (_, None) => bail!(
            "login did not produce a credential file at {}",
            primary.display()
        ),
        (Some(before), Some(after)) if after <= before => bail!(
            "login exited successfully but did not refresh the credential file at {} \
             (mtime unchanged since before this run) — a stale credential was left in \
             place. On macOS this usually means the OS keychain was unreachable (relocated \
             HOME has no ~/Library/Keychains) and Claude Code aborted persisting the new \
             token instead of falling back to plaintext; rerun and check for a keychain \
             error, or delete {} and try again",
            primary.display(),
            home.display()
        ),
        _ => {}
    }

    store.capture_login(id, &home, &plan.credential_files)?;

    println!("captured credential file(s):");
    for rel in &plan.credential_files {
        let full = home.join(rel);
        if full.exists() {
            println!("  {}", full.display());
        }
    }
    println!("account '{id}' captured ({})", home.display());
    let captured = effective_harnesses(&home);
    if captured.len() > 1 {
        println!(
            "note: '{id}' now has effective logins for multiple harnesses ({}) — they share \
             this home dir but don't share credentials, each harness only reads its own \
             subpath",
            captured.join(", ")
        );
    }
    println!("reuse with: am {harness_key} --account {id}");

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

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(id: &str) -> Account {
        Account {
            id: id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn partition_new_skips_already_present_ids() {
        let existing: BTreeSet<String> = ["anthropic-api-key".to_string()].into_iter().collect();
        let (to_add, skipped) =
            partition_new(&existing, vec![acct("anthropic-api-key"), acct("codex-auth-home")]);
        assert_eq!(
            to_add.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec!["codex-auth-home"]
        );
        assert_eq!(skipped, vec!["anthropic-api-key"]);
    }

    #[test]
    fn partition_new_dedupes_within_batch() {
        let existing = BTreeSet::new();
        let (to_add, skipped) = partition_new(&existing, vec![acct("dup"), acct("dup")]);
        assert_eq!(to_add.len(), 1);
        assert_eq!(skipped, vec!["dup"]);
    }

    #[test]
    fn partition_new_all_present_is_empty_add() {
        // Idempotency: a second import when everything already exists adds nothing.
        let existing: BTreeSet<String> = ["a".to_string(), "b".to_string()].into_iter().collect();
        let (to_add, skipped) = partition_new(&existing, vec![acct("a"), acct("b")]);
        assert!(to_add.is_empty());
        assert_eq!(skipped, vec!["a", "b"]);
    }

    #[test]
    fn effective_harnesses_empty_home_is_empty() {
        let home = tempfile::TempDir::new().unwrap();
        assert!(effective_harnesses(home.path()).is_empty());
    }

    #[test]
    fn effective_harnesses_detects_multiple_harnesses_sharing_one_home() {
        // A shared home dir (one account id, captured for two harnesses):
        // each harness's primary credential file laid out at its own
        // harness-specific relative subpath, exactly as `login()` writes it —
        // no collision, both detected.
        let home = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(home.path().join(".claude/.credentials.json"), "{}").unwrap();
        std::fs::write(home.path().join("config.json"), "{}").unwrap();

        let captured = effective_harnesses(home.path());
        assert_eq!(captured, vec!["claude-code".to_string(), "copilot".to_string()]);
    }

    #[test]
    fn effective_harnesses_only_lists_harnesses_actually_captured() {
        let home = tempfile::TempDir::new().unwrap();
        std::fs::write(home.path().join("config.json"), "{}").unwrap();

        let captured = effective_harnesses(home.path());
        assert_eq!(captured, vec!["copilot".to_string()]);
    }
}
