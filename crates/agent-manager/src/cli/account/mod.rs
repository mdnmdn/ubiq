//! `am account` subcommands: `ls`, `use`, `import`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};

use crate::account::{self, Account, AccountStore, EmptyAccountStore, FsAccountStore};
use crate::credentials::{CredentialBlob, CredentialId, SecretStore, blobs_from_seed};

mod check;
mod dump;
mod import;
mod login;
mod manage;

use check::*;
use dump::*;
use import::*;
use login::*;
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
    /// Discover credential locations and (on macOS) Claude Keychain OAuth.
    ///
    /// Dry-run reports env names, credential *paths*, and whether a Keychain
    /// Claude session can be imported as account id `default`. With `--write`,
    /// appends reference-only accounts to `accounts.toml` and materializes
    /// the Keychain blob into `accounts/default/` (file layout Claude seeds
    /// from), then sets `[defaults].account = "default"` so bare `am claude`
    /// reuses those credentials.
    Import {
        /// Write suggestions / materialize Keychain `default` account.
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
        /// Run the interactive login capture inside an isol8 sandbox instead
        /// of the plain PTY runner. **Default on macOS** (where a relocated
        /// HOME alone no longer forces the plaintext fallback — see below); use
        /// `--no-isolate` to opt out. Bare `--isolate` composes the layers the
        /// harness needs to run normally *minus* the OS-keychain layer (on
        /// macOS: `macos/system-runtime` + browser/launch-services, i.e. the
        /// `agents/claude-code` set without `integrations/keychain`);
        /// `--isolate=<profile>` overrides that with a single named profile.
        /// Needed for Claude Code 2.1.218+: with a relocated `HOME` alone the
        /// OS keychain is *unreachable* (no `~/Library/Keychains`), which
        /// that version reports as an error and does NOT fall back to a
        /// plaintext credential file — whereas denying keychain access at
        /// the sandbox layer does still take the clean file-fallback path.
        #[arg(long, num_args = 0..=1)]
        isolate: Option<Option<String>>,
        /// Force the plain (non-isolated) PTY capture, overriding the macOS
        /// default of running the capture in an isol8 keychain-denying sandbox.
        #[arg(long, conflicts_with = "isolate")]
        no_isolate: bool,
    },
    /// Show a stored credential's metadata (and, with `--show-secrets`, raw
    /// bytes) from the [`crate::credentials::SecretStore`].
    Dump {
        /// Credential name (e.g. `default`).
        name: String,
        /// Harness id or alias (e.g. `claude`, `codex`).
        #[arg(long)]
        harness: String,
        /// Emit the blob listing as a JSON array instead of human-readable text.
        #[arg(long)]
        json: bool,
        /// Print raw secret bytes instead of redacted metadata. Requires a
        /// TTY (or `AM_ALLOW_SECRET_DUMP=1`) so secrets can't land silently
        /// in a captured log/pipe.
        #[arg(long)]
        show_secrets: bool,
        /// Only show the blob whose `rel_path` matches this path (default:
        /// show every blob).
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Delete a stored credential from the [`crate::credentials::SecretStore`].
    /// The account-index entry (if any), from `am account login`/`import`, is
    /// left untouched — only the secret material is removed.
    Delete {
        /// Credential name (e.g. `default`).
        name: String,
        /// Harness id or alias.
        #[arg(long)]
        harness: String,
        /// Required to delete the credential currently set as
        /// `[defaults].account`, so a stray `am account delete default`
        /// can't silently break bare `am claude`.
        #[arg(long)]
        yes: bool,
    },
    /// Report whether stored credential(s) are still valid (unexpired), by
    /// inspecting any embedded expiry field — see [`credential_validity`].
    /// A report, not a gate: exit status stays 0 even for expired creds.
    Check {
        /// Credential name (e.g. `default`). Omit with `--all`.
        name: Option<String>,
        /// Harness id or alias. Required unless `--all`; ignored with `--all`.
        #[arg(long)]
        harness: Option<String>,
        /// Check every stored credential across all harnesses.
        #[arg(long)]
        all: bool,
    },
    /// Refresh a stored credential's token(s) in place via
    /// [`crate::harness::Harness::renew_credentials`].
    Renew {
        /// Credential name (e.g. `default`). Omit with `--all`.
        name: Option<String>,
        /// Harness id or alias. Required unless `--all`.
        #[arg(long)]
        harness: Option<String>,
        /// Renew every stored credential across all harnesses.
        #[arg(long)]
        all: bool,
    },
    /// Rename a stored credential within the same harness. Also updates
    /// `[defaults].account` when it pointed at the old name.
    Rename {
        /// Existing credential name.
        old: String,
        /// New credential name.
        new: String,
        /// Harness id or alias.
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
        AccountCommand::Login {
            id,
            harness,
            isolate,
            no_isolate,
        } => cmd_login(&id, &harness, isolate, no_isolate),
        AccountCommand::Dump {
            name,
            harness,
            json,
            show_secrets,
            path,
        } => cmd_dump(&name, &harness, json, show_secrets, path.as_deref()),
        AccountCommand::Delete { name, harness, yes } => cmd_delete(&name, &harness, yes),
        AccountCommand::Check { name, harness, all } => {
            cmd_check(name.as_deref(), harness.as_deref(), all)
        }
        AccountCommand::Renew { name, harness, all } => {
            cmd_renew(name.as_deref(), harness.as_deref(), all)
        }
        AccountCommand::Rename { old, new, harness } => cmd_rename(&old, &new, &harness),
    }
}

/// Build the [`crate::credentials::SecretStore`] the CLI should use, from the
/// effective settings for the current directory (see
/// [`crate::credentials::build_secret_store`] for engine resolution).
fn build_secret_store() -> Result<Box<dyn SecretStore>> {
    let cwd = std::env::current_dir()?;
    let settings = crate::settings::resolve(&cwd)?
        .map(|(s, _)| s)
        .unwrap_or_default();
    crate::credentials::build_secret_store(&settings)
}

/// Resolve `key` (harness id, alias, or launch command) to a
/// [`crate::harness::Harness`], erroring with the known-id list on no match.
fn resolve_harness(key: &str) -> Result<Box<dyn crate::harness::Harness>> {
    crate::harness::resolve(key).ok_or_else(|| {
        anyhow!(
            "unknown harness '{key}'; known: {}",
            crate::harness::known_ids().join(", ")
        )
    })
}

/// `[defaults].account` from the effective settings for the current
/// directory, if set. Used by `delete`/`rename` to guard/mirror the default.
fn configured_default_account() -> Result<Option<String>> {
    let cwd = std::env::current_dir()?;
    Ok(crate::settings::resolve(&cwd)?.and_then(|(s, _)| s.defaults.account))
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

/// Path to the global settings file that `[defaults]` lives in.
fn global_config_path() -> Result<PathBuf> {
    crate::settings::global_config_write_path()
}

/// Effective settings for the current directory (empty default if none).
fn effective_settings() -> crate::settings::Settings {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| crate::settings::resolve(&cwd).ok().flatten())
        .map(|(s, _)| s)
        .unwrap_or_default()
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
        let (to_add, skipped) = partition_new(
            &existing,
            vec![acct("anthropic-api-key"), acct("codex-auth-home")],
        );
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
        assert_eq!(
            captured,
            vec!["claude-code".to_string(), "copilot".to_string()]
        );
    }

    #[test]
    fn effective_harnesses_only_lists_harnesses_actually_captured() {
        let home = tempfile::TempDir::new().unwrap();
        std::fs::write(home.path().join("config.json"), "{}").unwrap();

        let captured = effective_harnesses(home.path());
        assert_eq!(captured, vec!["copilot".to_string()]);
    }

    #[test]
    fn redact_json_redacts_secret_like_keys_but_not_others() {
        let mut v = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "abcdef1234",
                "refreshToken": "zzzz",
                "subscriptionType": "pro"
            }
        });
        redact_json(&mut v);
        let inner = &v["claudeAiOauth"];
        assert_eq!(inner["accessToken"], serde_json::json!("<redacted:10>"));
        assert_eq!(inner["refreshToken"], serde_json::json!("<redacted:4>"));
        // Non-secret field survives untouched.
        assert_eq!(inner["subscriptionType"], serde_json::json!("pro"));
    }

    #[test]
    fn redact_json_walks_arrays_and_nested_objects() {
        let mut v = serde_json::json!({
            "sessions": [
                {"apiKey": "sk-live-1"},
                {"apiKey": "sk-live-22"}
            ]
        });
        redact_json(&mut v);
        assert_eq!(
            v["sessions"][0]["apiKey"],
            serde_json::json!("<redacted:9>")
        );
        assert_eq!(
            v["sessions"][1]["apiKey"],
            serde_json::json!("<redacted:10>")
        );
    }

    fn blob(bytes: &[u8]) -> CredentialBlob {
        CredentialBlob {
            name: "x".to_string(),
            rel_path: PathBuf::from(".claude/.credentials.json"),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn credential_validity_claude_future_expiry_is_valid() {
        // Claude shape: expiresAt in epoch MILLIS, far in the future.
        let future = 5_000_000_000_000i64; // year ~2128
        let bytes = format!("{{\"claudeAiOauth\":{{\"expiresAt\":{future}}}}}");
        let v = credential_validity(&[blob(bytes.as_bytes())], 1_000_000_000_000);
        assert_eq!(
            v,
            Validity::Valid {
                expires_at_ms: Some(future)
            }
        );
    }

    #[test]
    fn credential_validity_past_expiry_is_expired() {
        let past = 1_000_000_000_000i64;
        let bytes = format!("{{\"claudeAiOauth\":{{\"expiresAt\":{past}}}}}");
        let v = credential_validity(&[blob(bytes.as_bytes())], 2_000_000_000_000);
        assert_eq!(
            v,
            Validity::Expired {
                expires_at_ms: past
            }
        );
    }

    #[test]
    fn credential_validity_no_expiry_field_is_unknown() {
        let v = credential_validity(&[blob(b"{\"apiKey\":\"sk-1\"}")], 1_000);
        assert_eq!(v, Validity::Unknown);
    }

    #[test]
    fn credential_validity_empty_blobs_is_empty() {
        assert_eq!(credential_validity(&[], 1_000), Validity::Empty);
    }

    #[test]
    fn credential_validity_normalizes_seconds_to_millis() {
        // A bare epoch-SECONDS expiry (< 10^12) must be scaled by 1000 so it
        // compares correctly against a millis `now`. 2_000_000_000s = year
        // 2033, well ahead of a `now` of 1_500_000_000_000ms (~2017).
        let secs = 2_000_000_000i64;
        let bytes = format!("{{\"expires_at\":{secs}}}");
        let v = credential_validity(&[blob(bytes.as_bytes())], 1_500_000_000_000);
        assert_eq!(
            v,
            Validity::Valid {
                expires_at_ms: Some(secs * 1000)
            }
        );
    }

    #[test]
    fn credential_validity_takes_max_expiry_across_blobs() {
        let near = 1_600_000_000_000i64;
        let far = 4_000_000_000_000i64;
        let b1 = format!("{{\"expiresAt\":{near}}}");
        let b2 = format!("{{\"expiresAt\":{far}}}");
        let v = credential_validity(
            &[blob(b1.as_bytes()), blob(b2.as_bytes())],
            1_000_000_000_000,
        );
        assert_eq!(
            v,
            Validity::Valid {
                expires_at_ms: Some(far)
            }
        );
    }

    #[test]
    fn secret_dump_allowed_requires_tty_or_env_opt_in() {
        // Not requesting secrets at all: always allowed regardless of tty/env.
        assert!(secret_dump_allowed(false, false, false));
        // Requesting secrets with neither a tty nor the env opt-in: denied.
        assert!(!secret_dump_allowed(true, false, false));
        // A real tty is sufficient.
        assert!(secret_dump_allowed(true, true, false));
        // The env opt-in is sufficient even without a tty (e.g. CI/pipe).
        assert!(secret_dump_allowed(true, false, true));
    }

    /// `--isolate` on `login` follows the same `Option<Option<String>>` idiom
    /// as the top-level run flag (`src/cli/mod.rs`'s `RunArgs::isolate`):
    /// absent => `None`, bare `--isolate` => `Some(None)`, `--isolate=<p>` =>
    /// `Some(Some(p))`. Parsed directly through `AccountArgs` so a
    /// regression in the flag plumbing (e.g. an accidental `--isolate
    /// <profile>` two-token form) is caught without needing a real login.
    #[test]
    fn login_isolate_flag_parses_absent_bare_and_named() {
        let absent =
            AccountArgs::try_parse_from(["am-account", "login", "mdn", "--harness", "claude"])
                .unwrap();
        let AccountCommand::Login { isolate, .. } = absent.command else {
            panic!("expected Login");
        };
        assert_eq!(isolate, None);

        let bare = AccountArgs::try_parse_from([
            "am-account",
            "login",
            "mdn",
            "--harness",
            "claude",
            "--isolate",
        ])
        .unwrap();
        let AccountCommand::Login { isolate, .. } = bare.command else {
            panic!("expected Login");
        };
        assert_eq!(isolate, Some(None));

        let named = AccountArgs::try_parse_from([
            "am-account",
            "login",
            "mdn",
            "--harness",
            "claude",
            "--isolate=custom",
        ])
        .unwrap();
        let AccountCommand::Login { isolate, .. } = named.command else {
            panic!("expected Login");
        };
        assert_eq!(isolate, Some(Some("custom".to_string())));
    }
}
