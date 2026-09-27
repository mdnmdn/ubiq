//! `am account` subcommands: `ls`, `use`.
//!
//! An account is a set of credential *references* (env-var names, a base URL, a key helper). A
//! harness login is not one: it lives in the harness's own config home, one per profile, signed
//! in with `am profile login` (`D193`).

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};

use crate::account::{self, Account, AccountStore, EmptyAccountStore, FsAccountStore};

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

/// Path to the global settings file that `[defaults]` lives in.
fn global_config_path() -> Result<PathBuf> {
    crate::settings::global_config_write_path()
}
