//! `am account` subcommands: `ls`, `use`, `login`, `logout`, `check`.
//!
//! An account is a set of credential *references* (env-var names, a base URL, a key helper) and,
//! since `D194`, the key of a harness's persistent config home: `am account login <id> --harness
//! <h>` signs that account's home in, and every run that names the account — from any profile,
//! from any project — reads the login there. Nothing is captured or copied; the harness owns the
//! login and its refresh.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};

use crate::account::{self, Account, AccountStore, EmptyAccountStore, FsAccountStore};
use crate::home::{self, HomeStore};

mod manage;

use manage::*;

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
    /// Sign this account's harness config home in, with the harness's own login (`D194`).
    Login {
        /// Account id. Every run naming it shares the home this signs in.
        id: String,
        /// Harness to sign in.
        #[arg(long, default_value = "claude-code")]
        harness: String,
    },
    /// Forget this account's login for one harness by removing its config home (`D194`).
    Logout {
        /// Account id.
        id: String,
        /// Harness to sign out.
        #[arg(long, default_value = "claude-code")]
        harness: String,
    },
    /// Report whether this account's harness home holds a login, and until when.
    Check {
        /// Account id.
        id: String,
        /// Harness to check.
        #[arg(long, default_value = "claude-code")]
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
        AccountCommand::Login { id, harness } => cmd_login(&id, &harness),
        AccountCommand::Logout { id, harness } => cmd_logout(&id, &harness),
        AccountCommand::Check { id, harness } => cmd_check(&id, &harness),
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

/// Build the account-keyed config home store from the homes root (`AM_HOMES` / the default
/// `<config dir>/harness-homes`).
fn build_homes() -> Result<HomeStore> {
    home::resolve_homes_root(None)
        .map(HomeStore::new)
        .ok_or_else(|| anyhow!("could not determine a config directory for this OS"))
}

/// Path to the global settings file that `[defaults]` lives in.
fn global_config_path() -> Result<PathBuf> {
    crate::settings::global_config_write_path()
}
