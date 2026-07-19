//! `am account` subcommands: `ls`, `use`, `import`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

use crate::account::{self, Account, AccountStore, EmptyAccountStore, FsAccountStore};
use crate::credentials::{blobs_from_seed, CredentialBlob, CredentialId, SecretStore};

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
        AccountCommand::Login { id, harness } => cmd_login(&id, &harness),
        AccountCommand::Dump {
            name,
            harness,
            json,
            show_secrets,
            path,
        } => cmd_dump(&name, &harness, json, show_secrets, path.as_deref()),
        AccountCommand::Delete { name, harness, yes } => cmd_delete(&name, &harness, yes),
        AccountCommand::Check {
            name,
            harness,
            all,
        } => cmd_check(name.as_deref(), harness.as_deref(), all),
        AccountCommand::Renew {
            name,
            harness,
            all,
        } => cmd_renew(name.as_deref(), harness.as_deref(), all),
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

/// `am account dump <name> --harness <id> [--json] [--show-secrets] [--path <p>]`
fn cmd_dump(
    name: &str,
    harness: &str,
    json: bool,
    show_secrets: bool,
    path_filter: Option<&Path>,
) -> Result<()> {
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    let store = build_secret_store()?;
    let all_blobs = store
        .get(&id)?
        .ok_or_else(|| anyhow!("no stored credential ({}, {})", id.harness, id.name))?;

    let blobs: Vec<CredentialBlob> = all_blobs
        .into_iter()
        .filter(|b| path_filter.is_none_or(|p| b.rel_path == p))
        .collect();
    if blobs.is_empty() {
        if let Some(p) = path_filter {
            bail!(
                "no blob at path {} for ({}, {})",
                p.display(),
                id.harness,
                id.name
            );
        }
        println!("(no blobs)");
        return Ok(());
    }

    if show_secrets {
        let is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
        let env_allow = std::env::var("AM_ALLOW_SECRET_DUMP").as_deref() == Ok("1");
        if !secret_dump_allowed(show_secrets, is_tty, env_allow) {
            bail!(
                "refusing to print secrets outside a TTY; re-run interactively or set \
                 AM_ALLOW_SECRET_DUMP=1"
            );
        }
    }

    if json {
        let entries: Vec<serde_json::Value> = blobs
            .iter()
            .map(|b| {
                serde_json::json!({
                    "rel_path": b.rel_path.to_string_lossy(),
                    "bytes_utf8": dump_display_text(b, show_secrets),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    for blob in &blobs {
        println!("{}", blob.rel_path.display());
        println!("{}", dump_display_text(blob, show_secrets));
    }
    Ok(())
}

/// Render one blob's displayable text for `am account dump`: raw UTF-8
/// (lossy) when `show_secrets`; otherwise a `<present, N bytes>` placeholder,
/// followed by a redacted pretty-JSON rendering when the bytes parse as JSON.
fn dump_display_text(blob: &CredentialBlob, show_secrets: bool) -> String {
    if show_secrets {
        return String::from_utf8_lossy(&blob.bytes).into_owned();
    }
    let mut out = format!("<present, {} bytes>", blob.bytes.len());
    if let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(&blob.bytes) {
        redact_json(&mut v);
        if let Ok(pretty) = serde_json::to_string_pretty(&v) {
            out.push('\n');
            out.push_str(&pretty);
        }
    }
    out
}

/// Recursively replace object string values whose key case-insensitively
/// contains `token`, `secret`, `key`, `password`, or `auth` with
/// `"<redacted:LEN>"` (`LEN` = the original string's byte length). Non-string
/// values under a matching key, and every value under a non-matching key, are
/// walked/left as-is. Pure — the redaction core of `am account dump`, unit
/// tested without any store or I/O.
fn redact_json(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            let secret_like = ["token", "secret", "key", "password", "auth"];
            for (k, val) in map.iter_mut() {
                let key_matches = secret_like
                    .iter()
                    .any(|pat| k.to_lowercase().contains(pat));
                if key_matches && let serde_json::Value::String(s) = val {
                    *val = serde_json::Value::String(format!("<redacted:{}>", s.len()));
                } else {
                    redact_json(val);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_json(item);
            }
        }
        _ => {}
    }
}

/// Gate for `am account dump --show-secrets`: only allowed on a real TTY (so
/// secrets can't land silently in a captured log/pipe) or when the caller
/// explicitly opts in via `AM_ALLOW_SECRET_DUMP=1`. Pure — factored out of
/// [`cmd_dump`] so the gating logic is unit-testable without a real terminal.
fn secret_dump_allowed(show_secrets: bool, is_tty: bool, env_allow: bool) -> bool {
    !show_secrets || is_tty || env_allow
}

/// `am account delete <name> --harness <id> [--yes]`
fn cmd_delete(name: &str, harness: &str, yes: bool) -> Result<()> {
    let h = resolve_harness(harness)?;
    let default_account = configured_default_account()?;
    if default_account.as_deref() == Some(name) && !yes {
        bail!("refusing to delete the default account '{name}' without --yes");
    }

    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    let store = build_secret_store()?;
    store.delete(&id)?;
    println!(
        "deleted stored credential ({}, {}) — the account index entry, if any, is untouched",
        id.harness, id.name
    );
    Ok(())
}

/// The validity of a stored credential, as computed by [`credential_validity`]
/// from any embedded expiry field. Harness-agnostic (it just looks for a
/// numeric `*expire*` key anywhere in the parsed JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Validity {
    /// An expiry was found and is still in the future (or no expiry field was
    /// found but blobs are present — see [`credential_validity`], which uses
    /// [`Validity::Unknown`] for that case, so `expires_at_ms` here is always
    /// `Some`). Kept as `Option` to leave room for future "valid, no expiry".
    Valid {
        /// Epoch-millis expiry, if one was found.
        expires_at_ms: Option<i64>,
    },
    /// An expiry was found and is at/before `now_ms`.
    Expired {
        /// Epoch-millis expiry that has passed.
        expires_at_ms: i64,
    },
    /// Blobs are present but carry no recognizable expiry field.
    Unknown,
    /// No blobs are stored for this credential.
    Empty,
}

/// Compute a stored credential's [`Validity`] at `now_ms` (epoch millis).
///
/// Harness-agnostic but Claude-aware: for each blob, parse the bytes as JSON
/// and recursively search for a numeric field whose key case-insensitively
/// contains `"expire"` (covers Claude's `claudeAiOauth.expiresAt`, which is
/// epoch **millis**). Each value is normalized — numbers below `10^12` are
/// treated as **seconds** and scaled to millis — and the **maximum** expiry
/// across all blobs wins. Pure (takes `now_ms`) so it's unit-testable without
/// a clock. Returns [`Validity::Empty`] for no blobs, [`Validity::Unknown`]
/// for blobs with no expiry field, else `Valid`/`Expired`.
fn credential_validity(blobs: &[CredentialBlob], now_ms: i64) -> Validity {
    if blobs.is_empty() {
        return Validity::Empty;
    }
    let mut max_expiry: Option<i64> = None;
    for blob in blobs {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&blob.bytes)
            && let Some(ms) = max_expiry_ms(&v)
        {
            max_expiry = Some(max_expiry.map_or(ms, |cur| cur.max(ms)));
        }
    }
    match max_expiry {
        Some(ms) if ms > now_ms => Validity::Valid {
            expires_at_ms: Some(ms),
        },
        Some(ms) => Validity::Expired { expires_at_ms: ms },
        None => Validity::Unknown,
    }
}

/// Recursively find the maximum numeric `*expire*` value in `v`, normalized to
/// epoch millis (values below `10^12` are treated as seconds and ×1000).
fn max_expiry_ms(v: &serde_json::Value) -> Option<i64> {
    fn normalize(n: i64) -> i64 {
        if n < 1_000_000_000_000 {
            n.saturating_mul(1000)
        } else {
            n
        }
    }
    match v {
        serde_json::Value::Object(map) => {
            let mut best: Option<i64> = None;
            for (k, val) in map {
                if k.to_lowercase().contains("expire")
                    && let Some(n) = val.as_i64().or_else(|| val.as_f64().map(|f| f as i64))
                {
                    let ms = normalize(n);
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
                if let Some(ms) = max_expiry_ms(val) {
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
            }
            best
        }
        serde_json::Value::Array(items) => {
            let mut best: Option<i64> = None;
            for item in items {
                if let Some(ms) = max_expiry_ms(item) {
                    best = Some(best.map_or(ms, |cur| cur.max(ms)));
                }
            }
            best
        }
        _ => None,
    }
}

/// Current wall-clock time in epoch millis, for [`credential_validity`].
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Render a validity as a human-readable status suffix, e.g.
/// `valid (expires in 3d 4h)`, `expired`, `present (no expiry info)`,
/// `empty (no blobs stored)`.
fn describe_validity(v: Validity, now: i64) -> String {
    match v {
        Validity::Valid {
            expires_at_ms: Some(ms),
        } => format!("valid (expires in {})", human_duration_ms(ms - now)),
        Validity::Valid {
            expires_at_ms: None,
        } => "valid".to_string(),
        Validity::Expired { expires_at_ms: ms } => {
            format!("expired ({} ago)", human_duration_ms(now - ms))
        }
        Validity::Unknown => "present (no expiry info)".to_string(),
        Validity::Empty => "empty (no blobs stored)".to_string(),
    }
}

/// Format a non-negative millisecond span compactly as `Nd Nh` / `Nh Nm` /
/// `Nm` / `Ns`. Clamps negatives to `0s`.
fn human_duration_ms(ms: i64) -> String {
    let secs = ms.max(0) / 1000;
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins}m")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        format!("{secs}s")
    }
}

/// `am account check <name> --harness <id>` / `am account check --all`
fn cmd_check(name: Option<&str>, harness: Option<&str>, all: bool) -> Result<()> {
    let store = build_secret_store()?;
    let now = now_ms();

    if all {
        if name.is_some() {
            bail!("give either a <name> --harness <id> or --all, not both");
        }
        let mut valid = 0usize;
        let mut expired = 0usize;
        let mut unknown = 0usize;
        for meta in store.list()? {
            let blobs = store.get(&meta.id)?.unwrap_or_default();
            let v = credential_validity(&blobs, now);
            match v {
                Validity::Valid { .. } => valid += 1,
                Validity::Expired { .. } => expired += 1,
                Validity::Unknown | Validity::Empty => unknown += 1,
            }
            println!(
                "({}, {}): {}",
                meta.id.harness,
                meta.id.name,
                describe_validity(v, now)
            );
        }
        println!("{valid} valid, {expired} expired, {unknown} unknown");
        return Ok(());
    }

    let name = name.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let harness = harness.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    match store.get(&id)? {
        None => println!("({}, {}): missing", id.harness, id.name),
        Some(blobs) => {
            let v = credential_validity(&blobs, now);
            println!(
                "({}, {}): {}",
                id.harness,
                id.name,
                describe_validity(v, now)
            );
        }
    }
    Ok(())
}

/// `am account renew <name> --harness <id>` / `am account renew --all`
fn cmd_renew(name: Option<&str>, harness: Option<&str>, all: bool) -> Result<()> {
    let store = build_secret_store()?;

    if all {
        if name.is_some() {
            bail!("give either a <name> --harness <id> or --all, not both");
        }
        let mut renewed_ok = 0usize;
        let mut failed = 0usize;
        for meta in store.list()? {
            let id = &meta.id;
            let Some(h) = crate::harness::resolve(&id.harness) else {
                println!(
                    "({}, {}): skipped (unknown harness)",
                    id.harness, id.name
                );
                failed += 1;
                continue;
            };
            let creds = store.get(id)?.unwrap_or_default();
            match h.renew_credentials(&creds).and_then(|renewed| {
                let count = renewed.len();
                store.set(id, &renewed).map(|()| count)
            }) {
                Ok(count) => {
                    renewed_ok += 1;
                    println!("({}, {}): renewed {count} blob(s)", id.harness, id.name);
                }
                Err(e) => {
                    failed += 1;
                    println!("({}, {}): failed: {e:#}", id.harness, id.name);
                }
            }
        }
        println!("{renewed_ok} renewed, {failed} failed");
        return Ok(());
    }

    let name = name.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let harness = harness.ok_or_else(|| anyhow!("give a <name> --harness <id> or --all"))?;
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: name.to_string(),
    };
    let creds = store.get(&id)?.unwrap_or_default();
    let renewed = h
        .renew_credentials(&creds)
        .with_context(|| format!("renewing credentials for ({}, {})", id.harness, id.name))?;
    let count = renewed.len();
    store.set(&id, &renewed)?;
    println!("renewed {count} blob(s) for ({}, {})", id.harness, id.name);
    Ok(())
}

/// `am account rename <old> <new> --harness <id>`
fn cmd_rename(old: &str, new: &str, harness: &str) -> Result<()> {
    let h = resolve_harness(harness)?;
    let id = CredentialId {
        harness: h.id(),
        name: old.to_string(),
    };
    let store = build_secret_store()?;
    store.rename(&id, new)?;
    println!("renamed credential ({}, {old}) -> {new}", id.harness);

    if configured_default_account()?.as_deref() == Some(old) {
        set_defaults_account(new, true)?;
    }
    Ok(())
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

/// `am account import [--write]`: discover credential *locations* (env names
/// / file paths — never values) and, on macOS, optionally import the live
/// Claude Keychain OAuth session as account [`account::DEFAULT_ACCOUNT_ID`].
fn cmd_import(write: bool) -> Result<()> {
    let root = account::resolve_accounts_root(None)
        .ok_or_else(|| anyhow!("could not determine the accounts root for this OS"))?;

    // --- macOS Keychain → accounts/default (primary path for bare `am claude`) ---
    let mut keychain_imported = false;
    match try_claude_keychain_import(&root, write) {
        Ok(KeychainImport::Written(home)) => {
            keychain_imported = true;
            println!(
                "imported Claude Keychain → account '{}' (home {})",
                account::DEFAULT_ACCOUNT_ID,
                home.display()
            );
            // Always wire bare `am claude` to this account id (`default`).
            set_defaults_account(account::DEFAULT_ACCOUNT_ID, /*force*/ true)?;
        }
        Ok(KeychainImport::WouldWrite(home)) => {
            println!(
                "found macOS Keychain: {} — would materialize account id '{}' \
                 (home {}) and set [defaults].account = '{}' (pass --write)",
                account::CLAUDE_KEYCHAIN_SERVICE,
                account::DEFAULT_ACCOUNT_ID,
                home.display(),
                account::DEFAULT_ACCOUNT_ID,
            );
        }
        Ok(KeychainImport::Unavailable(reason)) => {
            println!("Claude Keychain import skipped: {reason}");
        }
        Err(e) => {
            // Don't fail the whole import if Keychain is missing; continue
            // with path/env discovery. Print the error so the user can act.
            println!("Claude Keychain import failed: {e:#}");
        }
    }

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
        // Path existence only — never read credential file contents here.
        // (Claude's usable macOS tokens live in Keychain; the on-disk
        // `.credentials.json` is often an empty stub — handled above.)
        let candidates: [(&str, PathBuf); 5] = [
            ("claude-credentials", home.join(".claude/.credentials.json")),
            ("claude-json", home.join(".claude.json")),
            ("codex-auth", home.join(".codex/auth.json")),
            ("opencode-auth", home.join(".local/share/opencode/auth.json")),
            ("copilot-config", home.join(".copilot/config.json")),
        ];

        for (label, path) in candidates {
            if path.exists() {
                println!("found credential file ({label}): {}", path.display());
                // Skip suggesting a claude file-home when Keychain import
                // already owns `default` — pointing home at ~/.claude would
                // re-introduce the empty-stub problem under CLAUDE_CONFIG_DIR.
                if label.starts_with("claude") && keychain_imported {
                    println!(
                        "  (not suggesting a separate account — Keychain `default` covers Claude)"
                    );
                    continue;
                }
                let home_dir = path.parent().unwrap_or(&path).to_path_buf();
                suggestions.push(Account {
                    id: format!("{label}-home"),
                    home: Some(home_dir),
                    ..Default::default()
                });
            }
        }
    }

    if suggestions.is_empty() && !keychain_imported {
        // Keychain dry-run already printed; only claim "nothing found" when
        // we truly have no leads.
        if !cfg!(target_os = "macos") {
            println!("no known credential locations found");
        }
        if !write {
            println!();
            println!("(dry run — nothing written; pass --write to apply)");
        }
        return Ok(());
    }

    // Idempotency for reference-only suggestions: never append an id that
    // already exists. Keychain `default` is handled separately (refreshable).
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
        if keychain_imported {
            println!("no additional reference-only accounts to add.");
        } else {
            println!("all suggested accounts already present — nothing to add (idempotent).");
        }
        if !write && !keychain_imported {
            println!("(dry run — nothing written; pass --write to append to accounts.toml)");
        }
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

/// Outcome of the optional Claude Keychain import step.
enum KeychainImport {
    /// `--write`: files + account record materialized.
    Written(PathBuf),
    /// Dry-run: Keychain is readable and would be written to this home.
    WouldWrite(PathBuf),
    /// Not applicable (non-macOS) or no entry — not an error.
    Unavailable(String),
}

/// Probe / materialize the ambient Claude Keychain session as account `default`.
fn try_claude_keychain_import(accounts_root: &Path, write: bool) -> Result<KeychainImport> {
    if !cfg!(target_os = "macos") {
        return Ok(KeychainImport::Unavailable(
            "only supported on macOS".into(),
        ));
    }
    // Probe first so dry-run and --write share the same validity checks
    // (empty token stub → Unavailable with a clear reason).
    match account::read_claude_keychain_credentials() {
        Ok(_creds) => {
            let home = accounts_root.join(account::DEFAULT_ACCOUNT_ID);
            if write {
                let acct = account::import_default_claude_from_keychain(accounts_root)?;
                let home = acct.home.unwrap_or(home);

                // Phase D: dual-write the same captured login into the
                // SecretStore alongside the file-home write above. Best
                // effort only for the store write itself — the file home
                // remains the primary/authoritative copy during migration,
                // so a SecretStore build/write failure here must not fail
                // the whole import.
                let claude = crate::harness::resolve("claude").expect("claude harness exists");
                let seed = claude.config_anchor().login_seed;
                let home_source = crate::source::Source::Dir(home.clone());
                let blobs = blobs_from_seed(&home_source, &seed)?;
                if !blobs.is_empty()
                    && let Ok(store) = build_secret_store()
                {
                    let id = CredentialId {
                        harness: claude.id(),
                        name: account::DEFAULT_ACCOUNT_ID.to_string(),
                    };
                    let _ = store.set(&id, &blobs);
                }

                Ok(KeychainImport::Written(home))
            } else {
                Ok(KeychainImport::WouldWrite(home))
            }
        }
        Err(e) => Ok(KeychainImport::Unavailable(format!("{e:#}"))),
    }
}

/// Set `[defaults].account = id` in the global settings file.
///
/// When `force` is false, only fills the key if unset (prints a note if
/// another id is already configured). When `force` is true (Keychain
/// import of the ambient session as account [`account::DEFAULT_ACCOUNT_ID`]),
/// always writes `id` so bare `am claude` uses the imported credentials.
fn set_defaults_account(id: &str, force: bool) -> Result<()> {
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

    let existing = defaults_table
        .get("account")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    match (existing.as_deref(), force) {
        (Some(e), false) if e == id => {
            println!("[defaults].account already '{id}'");
            return Ok(());
        }
        (Some(e), false) => {
            println!(
                "note: [defaults].account is '{e}' (not overwritten); \
                 run `am account use {id}` to make bare `am claude` use account '{id}'"
            );
            return Ok(());
        }
        (Some(e), true) if e == id => {
            println!("[defaults].account already '{id}'");
            return Ok(());
        }
        (Some(e), true) => {
            println!(
                "updating [defaults].account '{e}' → '{id}' (Keychain import is the ambient default)"
            );
        }
        (None, _) => {}
    }

    defaults_table.insert("account".to_string(), toml::Value::String(id.to_string()));
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&config_path, toml::to_string_pretty(&table)?)
        .map_err(|e| anyhow!("writing {}: {e}", config_path.display()))?;
    println!(
        "set [defaults].account = '{id}' ({})",
        config_path.display()
    );
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
        assert_eq!(v["sessions"][0]["apiKey"], serde_json::json!("<redacted:9>"));
        assert_eq!(v["sessions"][1]["apiKey"], serde_json::json!("<redacted:10>"));
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
        assert_eq!(v, Validity::Expired { expires_at_ms: past });
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
}
