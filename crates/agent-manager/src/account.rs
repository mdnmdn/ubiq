//! Accounts: credential *references* for a harness run, never secret material
//! in the account index.
//!
//! **The sharpest invariant in this module: `am`'s account *index* on disk
//! (`accounts.toml` / per-id toml) holds only env-var NAMES, a base URL and/or
//! a helper-command string — never a secret value.** Secret values are read
//! transiently from the environment at launch and placed into the child
//! process's env in memory. A harness login is not one of those references: it lives in
//! the harness's own config home, one per `(account, harness)` — [`crate::home::HomeStore`]
//! keys it — and nothing here reads, copies or captures it (`D194`).
//!
//! This mirrors the shape of [`crate::registry`]: a trait ([`AccountStore`]) so
//! embedders can back it with whatever they like, and a filesystem-backed
//! implementation ([`FsAccountStore`]) for the CLI.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use tracing::info;

use crate::Result;

/// A named credential *reference* for a harness run.
///
/// Holds only references, never secrets: env-var NAMES (whose values are read
/// transiently at launch time and passed through to the child process), a
/// provider base URL, a helper-command string (never run by `am` itself —
/// only wired into the harness's native key-helper slot). No field here can
/// hold a secret value. A record written before `D193` may name a `home` or
/// `captured` metadata; both are ignored on read.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct Account {
    /// Stable account identifier (the store key). Required for entries inline
    /// in `accounts.toml`; defaults to the file stem for per-file entries.
    #[serde(default)]
    pub id: String,
    /// NAME of an env var whose value is passed through to the harness's
    /// native API-key env var (e.g. `ANTHROPIC_API_KEY`) at launch. The value
    /// itself is read transiently from `am`'s environment at launch time and
    /// is never written to disk by `am`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// NAME of an env var whose value is passed through to the harness's
    /// native auth-token env var (e.g. `ANTHROPIC_AUTH_TOKEN`) at launch.
    /// Same never-written-to-disk rule as [`Self::api_key_env`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token_env: Option<String>,
    /// Provider base URL (e.g. a gateway/proxy endpoint), passed through to
    /// the harness's native base-URL env var.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// A command whose stdout yields the key, wired into the harness's native
    /// key-helper slot (e.g. Claude Code's `apiKeyHelper` setting). `am`
    /// never runs this command or sees its output — the harness does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
}

/// A source of [`Account`]s, resolved by id.
///
/// A trait so an embedder can back accounts with a database while the CLI
/// uses [`FsAccountStore`]. The write methods default to a read-only error, so
/// an in-memory or reference-only store implements only what it needs. See
/// `_docs/am-as-library.md`.
pub trait AccountStore {
    /// All accounts, sorted by id.
    fn accounts(&self) -> Result<Vec<Account>>;
    /// One account by exact id.
    fn account(&self, id: &str) -> Result<Option<Account>> {
        Ok(self.accounts()?.into_iter().find(|a| a.id == id))
    }

    /// Rename an account: move its on-disk record so future lookups address
    /// it as `to`. Default: a read-only error.
    fn rename_account(&self, _from: &str, _to: &str) -> Result<()> {
        bail!("this account store does not support renaming accounts")
    }

    /// Delete an account's on-disk record. Default: a read-only error.
    fn delete_account(&self, _id: &str) -> Result<()> {
        bail!("this account store does not support deleting accounts")
    }
}

/// An [`AccountStore`] with no accounts — the default for lib-mode embedders
/// and for the CLI when no accounts root is configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyAccountStore;

impl AccountStore for EmptyAccountStore {
    fn accounts(&self) -> Result<Vec<Account>> {
        Ok(Vec::new())
    }
}

/// A filesystem-backed account store rooted at an accounts directory.
///
/// Two layers, both optional, combined:
/// - `accounts.toml` with inline `[[account]]` entries (each requires `id`).
/// - Per-file `<id>.toml` (the `id` field defaults to the file stem if absent).
///
/// An id appearing in both layers (or twice within a layer) is a load-time
/// error, mirroring [`crate::registry::FsRegistry`]'s MCP-id collision rule.
#[derive(Debug, Clone)]
pub struct FsAccountStore {
    root: PathBuf,
}

impl FsAccountStore {
    /// Create a store rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsAccountStore { root: root.into() }
    }

    /// Persist `account` as a per-file `<id>.toml` under the store root
    /// (creating the root). Overwrites an existing per-file entry. Holds only
    /// references/metadata — never a secret value (same invariant as the rest
    /// of this module).
    pub fn save(&self, account: &Account) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("creating {}", self.root.display()))?;
        let path = self.root.join(format!("{}.toml", account.id));
        let body = toml::to_string_pretty(account)
            .with_context(|| format!("serializing account '{}'", account.id))?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }
}

/// Parsed structure for `accounts.toml`.
#[derive(Debug, serde::Deserialize, Default)]
struct AccountsToml {
    /// Inline account definitions.
    #[serde(default)]
    account: Vec<Account>,
}

impl AccountStore for FsAccountStore {
    fn accounts(&self) -> Result<Vec<Account>> {
        let mut entries = Vec::new();
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();

        // Inline entries from accounts.toml.
        let toml_path = self.root.join("accounts.toml");
        if toml_path.exists() {
            let content = std::fs::read_to_string(&toml_path)
                .with_context(|| format!("reading {}", toml_path.display()))?;
            let parsed: AccountsToml = toml::from_str(&content)
                .with_context(|| format!("parsing {}", toml_path.display()))?;

            for acct in parsed.account {
                if acct.id.is_empty() {
                    bail!("account entry in {} is missing 'id'", toml_path.display());
                }
                if !seen_ids.insert(acct.id.clone()) {
                    bail!(
                        "account id collision: '{}' appears more than once in {}",
                        acct.id,
                        toml_path.display()
                    );
                }
                entries.push(acct);
            }
        }

        // Per-file entries: <id>.toml (excluding accounts.toml itself).
        if self.root.is_dir() {
            for entry in std::fs::read_dir(&self.root)
                .with_context(|| format!("reading directory {}", self.root.display()))?
            {
                let entry = entry?;
                let path = entry.path();

                if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                    continue;
                }
                if path.file_name().and_then(|n| n.to_str()) == Some("accounts.toml") {
                    continue;
                }

                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| anyhow!("invalid account file name: {}", path.display()))?;

                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let mut acct: Account = toml::from_str(&content)
                    .with_context(|| format!("parsing {}", path.display()))?;
                if acct.id.is_empty() {
                    acct.id = stem;
                }

                if !seen_ids.insert(acct.id.clone()) {
                    bail!(
                        "account id collision: '{}' appears in both accounts.toml and a per-file entry ({})",
                        acct.id,
                        path.display()
                    );
                }
                entries.push(acct);
            }
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }

    fn rename_account(&self, from: &str, to: &str) -> Result<()> {
        validate_account_name(to)?;

        let acct = self
            .account(from)?
            .ok_or_else(|| anyhow!("no account named '{from}'"))?;

        let from_record = self.root.join(format!("{from}.toml"));
        if !from_record.is_file() {
            bail!(
                "account '{from}' is declared inline in accounts.toml; edit it there \
                 directly instead of renaming"
            );
        }

        let to_home = self.root.join(to);
        let to_record = self.root.join(format!("{to}.toml"));
        if to_home.exists() || to_record.exists() || self.account(to)?.is_some() {
            bail!("an account named '{to}' already exists");
        }

        // A directory left by a login captured before `D193` moves with the record.
        let from_home = self.root.join(from);
        if from_home.is_dir() {
            std::fs::rename(&from_home, &to_home).with_context(|| {
                format!("renaming {} -> {}", from_home.display(), to_home.display())
            })?;
        }

        let mut new_acct = acct;
        new_acct.id = to.to_string();
        self.save(&new_acct)?;
        std::fs::remove_file(&from_record)
            .with_context(|| format!("removing {}", from_record.display()))?;
        Ok(())
    }

    fn delete_account(&self, id: &str) -> Result<()> {
        self.account(id)?
            .ok_or_else(|| anyhow!("no account named '{id}'"))?;

        let record = self.root.join(format!("{id}.toml"));
        if !record.is_file() {
            bail!(
                "account '{id}' is declared inline in accounts.toml; edit it there \
                 directly instead of deleting"
            );
        }

        // A directory left by a login captured before `D193` goes with the record.
        let home = self.root.join(id);
        if home.is_dir() {
            info!(dir = %home.display(), account = %id, "delete_account: removing account home");
            std::fs::remove_dir_all(&home)
                .with_context(|| format!("removing {}", home.display()))?;
        }
        info!(file = %record.display(), account = %id, "delete_account: removing account record");
        std::fs::remove_file(&record).with_context(|| format!("removing {}", record.display()))?;
        Ok(())
    }
}

/// Reject a name unfit to become a filesystem entry under the accounts root:
/// empty/whitespace-only, `.`/`..`, or containing a path separator or a nul
/// byte. A trust boundary — a name arriving from an embedder's UI must not be
/// able to escape the accounts root.
fn validate_account_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("account name cannot be empty or whitespace-only");
    }
    if name == "." || name == ".." {
        bail!("account name cannot be '{name}'");
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        bail!("account name cannot contain a path separator or a null byte, got '{name}'");
    }
    Ok(())
}

/// The default accounts root: `~/.config/agent-manager/accounts` on all
/// platforms — the same base dir as the config file
/// ([`crate::settings::default_config_dir`]), so `config.toml` and `accounts/`
/// live together. Overridable by `AM_ACCOUNTS` (see [`resolve_accounts_root`]).
pub fn default_accounts_root() -> Option<PathBuf> {
    crate::settings::default_config_dir().map(|d| d.join("accounts"))
}

/// Resolve the accounts root from (highest first): an explicit path, the
/// `AM_ACCOUNTS` env var, then the default. Returns `None` if none apply.
pub fn resolve_accounts_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit
        .or_else(|| std::env::var("AM_ACCOUNTS").ok().map(PathBuf::from))
        .or_else(default_accounts_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fs_account_store_parses_inline_and_per_file_entries() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        fs::write(
            root.join("accounts.toml"),
            r#"
[[account]]
id = "personal"
api_key_env = "PERSONAL_ANTHROPIC_KEY"
base_url = "https://api.anthropic.com"

[[account]]
id = "work"
auth_token_env = "WORK_ANTHROPIC_TOKEN"
helper = "work-key-helper"
"#,
        )?;

        // A record from before `D193`, naming a captured-login home: still read.
        fs::write(
            root.join("sandbox.toml"),
            r#"
home = "/private/sandbox-home"
"#,
        )?;

        let store = FsAccountStore::new(root);
        let accounts = store.accounts()?;

        let ids: Vec<&str> = accounts.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["personal", "sandbox", "work"]);

        let personal = accounts.iter().find(|a| a.id == "personal").unwrap();
        assert_eq!(
            personal.api_key_env.as_deref(),
            Some("PERSONAL_ANTHROPIC_KEY")
        );
        assert_eq!(
            personal.base_url.as_deref(),
            Some("https://api.anthropic.com")
        );
        assert!(personal.auth_token_env.is_none());
        assert!(personal.helper.is_none());

        let work = accounts.iter().find(|a| a.id == "work").unwrap();
        assert_eq!(work.auth_token_env.as_deref(), Some("WORK_ANTHROPIC_TOKEN"));
        assert_eq!(work.helper.as_deref(), Some("work-key-helper"));

        let sandbox = accounts.iter().find(|a| a.id == "sandbox").unwrap();
        assert_eq!(sandbox, &acct_with_id("sandbox"));

        temp.close()?;
        Ok(())
    }

    #[test]
    fn fs_account_store_collision_between_inline_and_per_file_is_an_error() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        fs::write(
            root.join("accounts.toml"),
            r#"
[[account]]
id = "work"
api_key_env = "WORK_KEY"
"#,
        )?;
        fs::write(root.join("work.toml"), "api_key_env = \"OTHER_KEY\"\n")?;

        let store = FsAccountStore::new(root);
        let err = store.accounts().expect_err("should error on collision");
        assert!(err.to_string().contains("collision"), "message was: {err}");

        temp.close()?;
        Ok(())
    }

    #[test]
    fn fs_account_store_missing_id_returns_none() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        fs::write(
            root.join("accounts.toml"),
            r#"
[[account]]
id = "work"
api_key_env = "WORK_KEY"
"#,
        )?;

        let store = FsAccountStore::new(root);
        assert!(store.account("missing")?.is_none());
        assert!(store.account("work")?.is_some());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn empty_account_store_has_no_accounts() {
        let store = EmptyAccountStore;
        assert!(store.accounts().unwrap().is_empty());
        assert!(store.account("anything").unwrap().is_none());
    }

    #[test]
    fn fs_account_store_save_round_trips_id_and_references() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path().join("accounts");

        let account = Account {
            id: "cap".to_string(),
            api_key_env: Some("CAP_KEY".to_string()),
            ..Default::default()
        };

        let store = FsAccountStore::new(&root);
        let path = store.save(&account)?;
        assert!(path.exists());

        let loaded = FsAccountStore::new(&root)
            .account("cap")?
            .expect("saved account should be found");
        assert_eq!(loaded, account);

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_moves_home_and_record_and_reloads_under_new_id() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        let store = FsAccountStore::new(root);
        store.save(&acct_with_id("work"))?;
        fs::create_dir_all(root.join("work"))?;
        fs::write(root.join("work/token.json"), "secret")?;

        store.rename_account("work", "personal")?;

        assert!(store.account("work")?.is_none());
        let renamed = store
            .account("personal")?
            .expect("renamed account should be found");
        assert_eq!(renamed.id, "personal");
        assert!(root.join("personal/token.json").is_file());
        assert!(!root.join("work").exists());
        assert!(!root.join("work.toml").exists());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_to_existing_name_errors() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let store = FsAccountStore::new(temp.path());
        store.save(&acct_with_id("work"))?;
        store.save(&acct_with_id("personal"))?;

        let err = store
            .rename_account("work", "personal")
            .expect_err("should error renaming onto an existing account");
        assert!(err.to_string().contains("already exists"), "{err}");

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_rejects_separator_or_dotdot_and_touches_nothing() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        let store = FsAccountStore::new(root);
        store.save(&acct_with_id("work"))?;
        fs::create_dir_all(root.join("work"))?;

        assert!(store.rename_account("work", "../escaped").is_err());
        assert!(store.rename_account("work", "sub/dir").is_err());
        assert!(store.rename_account("work", "..").is_err());
        assert!(store.rename_account("work", "  ").is_err());

        // Nothing was touched: the original account is unchanged.
        assert!(root.join("work").is_dir());
        assert!(root.join("work.toml").is_file());
        assert!(store.account("work")?.is_some());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn rename_account_of_missing_errors() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = FsAccountStore::new(temp.path());
        assert!(store.rename_account("ghost", "new-name").is_err());
    }

    #[test]
    fn delete_account_removes_home_and_record() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        let store = FsAccountStore::new(root);
        store.save(&acct_with_id("work"))?;
        fs::create_dir_all(root.join("work"))?;
        fs::write(root.join("work/token.json"), "secret")?;

        store.delete_account("work")?;

        assert!(!root.join("work").exists());
        assert!(!root.join("work.toml").exists());
        assert!(store.account("work")?.is_none());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn delete_account_of_missing_errors() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = FsAccountStore::new(temp.path());
        assert!(store.delete_account("ghost").is_err());
    }

    fn acct_with_id(id: &str) -> Account {
        Account {
            id: id.to_string(),
            ..Default::default()
        }
    }
}
